//! The boxpigma backend: one actor owns the NetEase service, cache, playback
//! queue, navigation, login and settings. Frontends render [`View`] snapshots
//! and send [`Message`]s; no frontend calls the API or the engine directly.

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use ncm_api::{LoginInfo, MvInfo, SongInfo};
use reqwest::Client;

use super::view::{
    ArtistView, Banner, ContentSource, LoginView, NavItem, NavSection, NavView, Page, PlayerView,
    SearchKind, SearchView, SettingsView, ThemeView, View,
};
use crate::{
    cache::CacheManager,
    config::{Config, Pane, ThemeRegistry, theme_fallback},
    event::{AppEvent, Event, EventHandler, PlaybackEvent},
    ipc::{
        BridgeRequest, IpcEvent, IpcServerGuard, QueueEntry, QueueSnapshot, SearchEntry,
        StatusSnapshot, start_actor_server,
    },
    mpris::MprisHandle,
    playback::{
        LyricLine, NCM_SEARCH_QUEUE_KEY, PlaybackEngine, SeekTarget, parse_lyric_lines, parse_seek,
    },
    service::{ApiEndpoint, ApiService},
    state::{ContentState, LoginField, LoginMethod, PaginationInfo},
    utils::{Named, boxpigma_cache_dir, boxpigma_config_dir, path::expand_tilde},
};
use ferriso_core::{App, Command, Subscription};
use tokio::sync::{Mutex as AsyncMutex, broadcast, mpsc};

const TICK: Duration = Duration::from_millis(100);
const LISTEN_THRESHOLD: Duration = Duration::from_secs(30);
const SEEK_JUMP: Duration = Duration::from_secs(1);
const MAX_EVENTS_PER_TICK: usize = 128;
const QR_POLL_INTERVAL: Duration = Duration::from_secs(2);
const QR_POLLS: u32 = 150;

/// What a frontend can ask the backend to do.
#[derive(Debug, Clone)]
pub enum Message {
    Navigate(Page),
    ActivateNav { section: usize, item: usize },
    Refresh,
    Back,
    LoadMore,
    ActivateRow(usize),
    AddNextRow(usize),
    LikeRow(usize),
    UploadRow(usize),
    OpenArtistRow(usize),
    OpenAlbumRow(usize),
    PlayArtistSong(usize),
    AddNextArtistSong(usize),
    LikeArtistSong(usize),
    OpenArtistAlbum(usize),
    OpenSimilarArtist(usize),
    OpenSongContext(bool),
    OpenPersonalFm,
    TrashFm,
    PlayQueueRow(usize),
    SwitchQueue(bool),
    ActivateQueue(String),
    ClearQueue,
    TogglePlay,
    Next,
    Prev,
    Seek(f64),
    SetVolume(f64),
    CycleMode,
    ToggleLike,
    SearchInput(String),
    Search(String),
    SetSearchKind(SearchKind),
    SearchSubmit,
    SetLoginMethod(LoginMethod),
    SetLoginField(LoginField, String),
    AppendLoginField(LoginField, char),
    BackspaceLoginField(LoginField),
    ClearLoginField(LoginField),
    SendSmsCode,
    SignIn { account: String, password: String },
    SendSms { phone: String },
    SmsLogin { phone: String, code: String },
    SubmitLogin,
    StartQr,
    RefreshQr,
    DailySign,
    Logout,
    ToggleSaveOnPlay,
    CycleQuality,
    ToggleNotifySongChange,
    ToggleNotifyErrors,
    ToggleTranslation,
    ToggleVisualizer,
    TogglePitch,
    SetPaneSize(Pane, u16),
    CycleLyricStyle,
    SetLyricStyle(String),
    CycleNavPosition,
    TogglePane(Pane),
    CycleTheme,
    SetTheme(String),
    SetQuality(String),
    Internal(Internal),
}

/// Results of asynchronous work, and playback events already normalized by the
/// engine. Frontends never construct these.
#[derive(Debug, Clone)]
pub enum Internal {
    UserLoaded(Option<LoginInfo>),
    LikedLoaded(Result<HashSet<u64>, String>),
    ContentLoaded {
        generation: u64,
        source: ContentSource,
        content: ContentState,
        pagination: Option<PaginationInfo>,
        playlist_id: Option<u64>,
        title: Option<String>,
        append: bool,
    },
    ArtistLoaded {
        generation: u64,
        detail: ncm_api::ArtistDetail,
        albums: Vec<ncm_api::ArtistAlbum>,
        similar: Vec<ncm_api::SingerInfo>,
        error: Option<String>,
    },
    QrCreated {
        generation: u64,
        result: Result<(String, String), String>,
    },
    QrChecked {
        generation: u64,
        result: Result<ncm_api::Msg, String>,
    },
    QrLoginStatus(Result<LoginInfo, String>),
    LoginFinished(Result<LoginInfo, String>),
    SmsSent(Result<(), String>),
    ActionResult(Result<String, String>),
    LoggedOut(Result<(), String>),
    LikeDone(Result<(), String>),
    CoverLoaded {
        song_id: u64,
        bytes: Option<Vec<u8>>,
    },
    MvLoaded {
        mv_id: u64,
        result: Result<(Box<MvInfo>, Option<Vec<u8>>), String>,
    },
    LyricsLoaded {
        song_id: u64,
        lyrics: Vec<LyricLine>,
        translated: Vec<LyricLine>,
    },
    QueueAppended {
        key: String,
        songs: Vec<Arc<SongInfo>>,
        playlist_id: u64,
    },
    SwitchListLoaded {
        key: String,
        result: Result<Vec<Arc<SongInfo>>, String>,
    },
    Bridge(BridgeRequest),
    PersonalFmLoaded {
        generation: u64,
        result: Result<Vec<Arc<SongInfo>>, String>,
    },
    Tick,
    Noop,
}

struct Bridge {
    requests: Arc<AsyncMutex<mpsc::UnboundedReceiver<BridgeRequest>>>,
    status: Arc<Mutex<StatusSnapshot>>,
    queue: Arc<Mutex<QueueSnapshot>>,
    status_tx: broadcast::Sender<StatusSnapshot>,
    mpris: MprisHandle,
    _guard: IpcServerGuard,
}

impl Bridge {
    fn start() -> Option<Self> {
        if tokio::runtime::Handle::try_current().is_err() {
            return None;
        }
        let status = Arc::new(Mutex::new(StatusSnapshot::default()));
        let queue = Arc::new(Mutex::new(QueueSnapshot::default()));
        let (status_tx, _) = broadcast::channel(16);
        let (request_tx, requests) = mpsc::unbounded_channel();
        let guard = start_actor_server(
            Arc::clone(&status),
            Arc::clone(&queue),
            status_tx.clone(),
            request_tx.clone(),
        );
        let (mpris, service) = MprisHandle::new();
        let (mpris_tx, mut mpris_rx) = mpsc::unbounded_channel::<IpcEvent>();
        tokio::spawn(service.run_actor(mpris_tx));
        tokio::spawn(async move {
            while let Some(event) = mpris_rx.recv().await {
                if request_tx.send(BridgeRequest::Control(event)).is_err() {
                    break;
                }
            }
        });
        Some(Self {
            requests: Arc::new(AsyncMutex::new(requests)),
            status,
            queue,
            status_tx,
            mpris,
            _guard: guard,
        })
    }

    fn publish(&self, view: &View, cache: &CacheManager) {
        let song = view.player.song.as_ref();
        let snapshot = StatusSnapshot {
            id: song.map(|song| song.id).unwrap_or(0),
            name: song.map(|song| song.name.clone()).unwrap_or_default(),
            artist: song.map(|song| song.singer.clone()).unwrap_or_default(),
            album: song.map(|song| song.album.clone()).unwrap_or_default(),
            duration_ms: song.map(|song| song.duration).unwrap_or(0),
            position_ms: (view.player.position_secs * 1000.0) as u64,
            volume: view.player.volume,
            playing: view.player.playing,
            paused: view.player.paused,
            mode: match view.player.mode {
                crate::playback::PlayMode::Sequential => "sequential",
                crate::playback::PlayMode::RepeatOne => "repeat_one",
                crate::playback::PlayMode::RepeatAll => "repeat_all",
                crate::playback::PlayMode::Shuffle => "shuffle",
                crate::playback::PlayMode::Heartbeat { .. } => "heartbeat",
            }
            .to_string(),
            liked: view.player.liked,
        };
        if let Ok(mut stored) = self.status.lock() {
            *stored = snapshot.clone();
        }
        if let Ok(mut stored) = self.queue.lock() {
            *stored = QueueSnapshot {
                key: view.player.queue_key.clone(),
                current_index: view.player.queue_index,
                songs: view
                    .player
                    .queue
                    .iter()
                    .map(|song| QueueEntry::from_song(song))
                    .collect(),
            };
        }
        let _ = self.status_tx.send(snapshot.clone());
        self.mpris
            .update(&snapshot, view.player.mode, cache.cached_cover(snapshot.id));
    }
}

struct HistoryEntry {
    page: Page,
    content: Arc<ContentState>,
    title: String,
    subtitle: Option<String>,
    source: Option<ContentSource>,
    pagination: Option<PaginationInfo>,
    artist: Option<ArtistView>,
    search: SearchView,
    nav_section: usize,
    nav_item: usize,
}

pub struct Backend {
    config: Config,
    service: ApiService,
    cache: Arc<CacheManager>,
    http: Client,
    engine: PlaybackEngine,
    events: EventHandler,
    liked_ids: Arc<Mutex<HashSet<u64>>>,
    search_results: Arc<Mutex<HashMap<u64, Arc<SongInfo>>>>,
    theme_registry: ThemeRegistry,

    page: Page,
    nav_sections: Vec<NavSection>,
    nav_section: usize,
    nav_item: usize,
    content: Arc<ContentState>,
    title: String,
    subtitle: Option<String>,
    source: Option<ContentSource>,
    pagination: Option<PaginationInfo>,
    history: Vec<HistoryEntry>,
    artist: Option<ArtistView>,
    search: SearchView,
    user: Option<LoginInfo>,
    loading: bool,
    banner: Banner,
    content_generation: u64,
    queued_playlists: HashSet<u64>,

    cover: Option<Arc<Vec<u8>>>,
    mv: Option<MvInfo>,
    mv_poster: Option<Arc<Vec<u8>>>,
    reported_listen: Option<u64>,
    last_position: Duration,
    played_in_track: Duration,

    login_method: LoginMethod,
    login_account: String,
    login_password: String,
    login_phone: String,
    login_code: String,
    login_loading: bool,
    login_error: Option<String>,
    login_notice: Option<String>,
    qr_url: String,
    qr_key: Option<String>,
    qr_status: String,
    qr_generation: u64,
    qr_polls: u32,
    qr_poll_due: Option<Instant>,
    qr_in_flight: bool,
    qr_scanned: bool,
    bridge: Option<Bridge>,
}

impl Backend {
    /// Build the backend. `persist` decides whether settings changes are written
    /// to the user's config file; tests pass `false`.
    pub fn new(config: Config, persist: bool) -> color_eyre::Result<Self> {
        Self::build(config, persist, false)
    }

    /// Build the interactive backend and bridge IPC/MPRIS to its snapshots.
    pub fn new_with_bridge(config: Config, persist: bool) -> color_eyre::Result<Self> {
        Self::build(config, persist, true)
    }

    fn build(config: Config, persist: bool, bridge: bool) -> color_eyre::Result<Self> {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let mut config = config;
        config.persist = persist;

        let proxy = config.proxy.clone().unwrap_or_default();
        let cookie_path = boxpigma_config_dir().join("cookies.json");
        let mut builder = ncm_api::NcmClient::builder().cookie_path(cookie_path);
        if !proxy.is_empty() {
            builder = builder.proxy(&proxy);
        }
        let api = Arc::new(builder.build()?);

        let cache_dir = {
            let expanded = expand_tilde(&config.cache.cache_dir);
            if expanded.is_absolute() {
                expanded
            } else {
                boxpigma_cache_dir().join(&config.cache.cache_dir)
            }
        };
        let base_dir = boxpigma_cache_dir();
        let cache = Arc::new(CacheManager::new(
            cache_dir,
            base_dir.clone(),
            config.cache.cache_template.clone(),
        ));
        let service = ApiService::new(api, cache.clone());

        let quality = ncm_api::SongQuality::from_level(&config.cache.quality)
            .unwrap_or(ncm_api::SongQuality::Higher);
        let stream_client = http_client(&proxy, None);
        let http = http_client(&proxy, Some(Duration::from_secs(30)));
        let events = EventHandler::new(false);
        let liked_ids: Arc<Mutex<HashSet<u64>>> = Arc::new(Mutex::new(HashSet::new()));
        let search_results = Arc::new(Mutex::new(HashMap::new()));
        let engine = PlaybackEngine::new(
            events.sender(),
            service.clone(),
            cache.clone(),
            base_dir,
            quality,
            config.audio.clone(),
            config.cache.save_on_play,
            stream_client,
            Arc::clone(&liked_ids),
        );

        let sections = if config.navigation.sections.is_empty() {
            crate::config::NavConfig::default().sections
        } else {
            config.navigation.sections.clone()
        };
        let nav_sections = sections
            .into_iter()
            .map(|section| NavSection {
                title: plain(&section.title),
                items: section
                    .items
                    .into_iter()
                    .map(|item| NavItem {
                        name: plain(&item.name),
                        api: item.api,
                    })
                    .collect(),
            })
            .collect();

        let theme_registry = ThemeRegistry::new(config.themes.clone());
        use crate::config::RANDOM_THEME;
        if config.default_theme == RANDOM_THEME {
            config.default_theme = theme_registry.concrete_name(RANDOM_THEME);
        }
        if config.light_theme.as_deref() == Some(RANDOM_THEME) {
            config.light_theme = Some(theme_registry.concrete_name(RANDOM_THEME));
        }

        Ok(Self {
            config,
            service,
            cache,
            http,
            engine,
            events,
            liked_ids,
            search_results,
            theme_registry,
            page: Page::Browse,
            nav_sections,
            nav_section: 0,
            nav_item: 0,
            content: Arc::new(ContentState::Empty),
            title: "boxpigma".into(),
            subtitle: None,
            source: None,
            pagination: None,
            history: Vec::new(),
            artist: None,
            search: SearchView {
                kind: SearchKind::Songs,
                query: String::new(),
                active: false,
            },
            user: None,
            loading: true,
            banner: Banner {
                text: "正在读取登录状态…".into(),
                error: false,
            },
            content_generation: 0,
            queued_playlists: HashSet::new(),
            cover: None,
            mv: None,
            mv_poster: None,
            reported_listen: None,
            last_position: Duration::ZERO,
            played_in_track: Duration::ZERO,
            login_method: LoginMethod::Qr,
            login_account: String::new(),
            login_password: String::new(),
            login_phone: String::new(),
            login_code: String::new(),
            login_loading: false,
            login_error: None,
            login_notice: None,
            qr_url: String::new(),
            qr_key: None,
            qr_status: String::new(),
            qr_generation: 0,
            qr_polls: 0,
            qr_poll_due: None,
            qr_in_flight: false,
            qr_scanned: false,
            bridge: if bridge { Bridge::start() } else { None },
        })
    }

    fn can_back(&self) -> bool {
        !self.history.is_empty() || self.page != Page::Browse
    }

    /// Switch to the configured item and load it from scratch.
    fn activate_nav(&mut self, section: usize, item: usize) -> Command<Message> {
        let Some(api) = self
            .nav_sections
            .get(section)
            .and_then(|section| section.items.get(item))
            .and_then(|item| item.api.clone())
        else {
            self.banner = Banner {
                text: "这一项没有配置 API".into(),
                error: true,
            };
            return Command::none();
        };
        let name = self.nav_sections[section].items[item].name.clone();
        self.nav_section = section;
        self.nav_item = item;
        self.history.clear();
        self.artist = None;
        self.page = Page::Browse;
        self.title = name;
        self.subtitle = endpoint_subtitle(&api);
        self.search.active = false;
        self.load_endpoint(api.clone(), false)
    }

    fn load_endpoint(&mut self, api: String, force: bool) -> Command<Message> {
        self.begin_load(ContentSource::Endpoint(api.clone()));
        let source = ContentSource::Endpoint(api.clone());
        self.reload_source(source, force)
    }

    fn begin_load(&mut self, source: ContentSource) {
        self.content_generation = self.content_generation.wrapping_add(1);
        self.source = Some(source);
        self.content = Arc::new(ContentState::Loading);
        self.pagination = None;
        self.loading = true;
    }

    fn reload_source(&mut self, source: ContentSource, force: bool) -> Command<Message> {
        let generation = self.content_generation;
        let service = self.service.clone();
        let cache = self.cache.clone();
        let uid = self.user.as_ref().map(|user| user.uid);
        let limit = self.config.search_limit;
        let ttl = self.config.cache.content_cache_ttl;
        let title = self.title.clone();
        Command::perform(
            async move { load_source(service, cache, uid, limit, ttl, force, source, title).await },
            move |loaded| {
                Message::Internal(Internal::ContentLoaded {
                    generation,
                    source: loaded.source,
                    content: loaded.content,
                    pagination: loaded.pagination,
                    playlist_id: loaded.playlist_id,
                    title: loaded.title,
                    append: false,
                })
            },
        )
    }

    fn reload_current(&mut self, force: bool) -> Command<Message> {
        if self.page == Page::Artist {
            if let Some(artist) = &self.artist {
                let artist_id = artist.detail.id;
                self.content_generation = self.content_generation.wrapping_add(1);
                let generation = self.content_generation;
                self.loading = true;
                let service = self.service.clone();
                return Command::perform(
                    async move {
                        let detail = service.client().artist_detail(artist_id).await;
                        let albums = service.client().artist_albums(artist_id, 0, 100).await;
                        let similar = service.client().simi_artist(artist_id).await;
                        match detail {
                            Ok(detail) => Internal::ArtistLoaded {
                                generation,
                                detail,
                                albums: albums.unwrap_or_default(),
                                similar: similar.unwrap_or_default(),
                                error: None,
                            },
                            Err(error) => Internal::ArtistLoaded {
                                generation,
                                detail: ncm_api::ArtistDetail {
                                    id: artist_id,
                                    name: "未知歌手".into(),
                                    alias: Vec::new(),
                                    brief_desc: String::new(),
                                    pic_url: String::new(),
                                    album_size: 0,
                                    music_size: 0,
                                    hot_songs: Vec::new(),
                                },
                                albums: albums.unwrap_or_default(),
                                similar: similar.unwrap_or_default(),
                                error: Some(error.to_string()),
                            },
                        }
                    },
                    Message::Internal,
                );
            }
            return Command::none();
        }
        let Some(source) = self.source.clone() else {
            return Command::none();
        };
        self.begin_load(source.clone());
        self.reload_source(source, force)
    }

    fn push_history(&mut self) {
        self.history.push(HistoryEntry {
            page: self.page,
            content: Arc::clone(&self.content),
            title: self.title.clone(),
            subtitle: self.subtitle.clone(),
            source: self.source.clone(),
            pagination: self.pagination.clone(),
            artist: self.artist.clone(),
            search: self.search.clone(),
            nav_section: self.nav_section,
            nav_item: self.nav_item,
        });
    }

    fn back(&mut self) -> Command<Message> {
        if self.page != Page::Browse {
            self.page = Page::Browse;
            if let Some(entry) = self.history.pop() {
                self.restore(entry);
            } else if self.artist.is_some() {
                self.artist = None;
                self.content = Arc::new(ContentState::Empty);
                self.title = "boxpigma".into();
                self.subtitle = None;
            } else {
                self.page = Page::Browse;
            }
            return Command::none();
        }
        match self.history.pop() {
            Some(entry) => {
                self.restore(entry);
                Command::none()
            }
            None => Command::none(),
        }
    }

    fn restore(&mut self, entry: HistoryEntry) {
        self.page = entry.page;
        self.content = entry.content;
        self.title = entry.title;
        self.subtitle = entry.subtitle;
        self.source = entry.source;
        self.pagination = entry.pagination;
        self.artist = entry.artist;
        self.search = entry.search;
        self.nav_section = entry.nav_section;
        self.nav_item = entry.nav_item;
        self.loading = false;
    }

    fn open_detail(
        &mut self,
        source: ContentSource,
        title: String,
        subtitle: &str,
    ) -> Command<Message> {
        self.push_history();
        self.page = Page::Browse;
        self.artist = None;
        self.title = title;
        self.subtitle = Some(subtitle.into());
        if let ContentSource::Playlist(id) | ContentSource::Radio(id) = &source {
            self.queued_playlists.remove(id);
            self.engine.set_playlist_id(*id);
        }
        self.begin_load(source.clone());
        self.reload_source(source, false)
    }

    fn open_personal_fm(&mut self) -> Command<Message> {
        self.push_history();
        self.page = Page::Browse;
        self.artist = None;
        self.title = "私人FM".into();
        self.subtitle = Some("根据你的口味推荐".into());
        self.begin_load(ContentSource::PersonalFm);
        let generation = self.content_generation;
        let service = self.service.clone();
        Command::perform(async move { service.personal_fm().await }, move |result| {
            Message::Internal(Internal::PersonalFmLoaded { generation, result })
        })
    }

    fn open_artist(&mut self, artist_id: u64) -> Command<Message> {
        if artist_id == 0 {
            return Command::none();
        }
        self.push_history();
        self.content_generation = self.content_generation.wrapping_add(1);
        let generation = self.content_generation;
        self.page = Page::Artist;
        self.loading = true;
        self.artist = None;
        self.title = "歌手".into();
        self.subtitle = None;
        let service = self.service.clone();
        Command::perform(
            async move {
                let detail = service.client().artist_detail(artist_id).await;
                let albums = service.client().artist_albums(artist_id, 0, 100).await;
                let similar = service.client().simi_artist(artist_id).await;
                match detail {
                    Ok(detail) => Internal::ArtistLoaded {
                        generation,
                        detail,
                        albums: albums.unwrap_or_default(),
                        similar: similar.unwrap_or_default(),
                        error: None,
                    },
                    Err(error) => Internal::ArtistLoaded {
                        generation,
                        detail: ncm_api::ArtistDetail {
                            id: artist_id,
                            name: "未知歌手".into(),
                            alias: Vec::new(),
                            brief_desc: String::new(),
                            pic_url: String::new(),
                            album_size: 0,
                            music_size: 0,
                            hot_songs: Vec::new(),
                        },
                        albums: Vec::new(),
                        similar: Vec::new(),
                        error: Some(error.to_string()),
                    },
                }
            },
            Message::Internal,
        )
    }

    fn play_row(&mut self, row: usize) -> Command<Message> {
        match self.content.as_ref() {
            ContentState::Songs(songs) => {
                let Some(song) = songs.get(row).cloned() else {
                    return Command::none();
                };
                if self.engine.is_currently_playing(song.id) {
                    self.engine.toggle_pause();
                    return Command::none();
                }
                let songs = songs.to_vec();
                let search = matches!(self.source, Some(ContentSource::Search(..)));
                if search {
                    self.engine
                        .append_and_play_key(NCM_SEARCH_QUEUE_KEY, &songs, row);
                } else {
                    let key = self.queue_key();
                    self.engine.play_songs(&key, songs.clone(), row);
                    if let Some(pagination) = self.pagination.clone()
                        && pagination.has_more
                        && pagination.api.starts_with("playlist:")
                    {
                        let Some(playlist_id) = pagination
                            .api
                            .strip_prefix("playlist:")
                            .and_then(|id| id.parse::<u64>().ok())
                        else {
                            return Command::none();
                        };
                        if self.queued_playlists.contains(&playlist_id) {
                            return Command::none();
                        }
                        let key = self.engine.queue_key_for(&key);
                        let service = self.service.clone();
                        let offset = songs.len() as u32;
                        let limit = pagination.limit;
                        let total = pagination.total;
                        let api = pagination.api.clone();
                        return Command::perform(
                            async move {
                                let mut all = Vec::new();
                                let mut offset = offset;
                                while u64::from(offset) < total {
                                    let Some((content, next)) =
                                        service.load_more(&api, offset, limit).await
                                    else {
                                        break;
                                    };
                                    let ContentState::Songs(page) = content else {
                                        break;
                                    };
                                    if page.is_empty() {
                                        break;
                                    }
                                    all.extend(page);
                                    offset = next.offset + next.limit;
                                    if !next.has_more {
                                        break;
                                    }
                                }
                                Internal::QueueAppended {
                                    key,
                                    songs: all,
                                    playlist_id,
                                }
                            },
                            Message::Internal,
                        );
                    }
                }
                self.banner = Banner {
                    text: format!("▶ {}", song.name),
                    error: false,
                };
                Command::none()
            }
            ContentState::SongLists(lists) => {
                let Some(list) = lists.get(row) else {
                    return Command::none();
                };
                let id = list.id;
                let name = list.name.clone();
                match &self.source {
                    Some(ContentSource::Search(SearchKind::Albums, _)) => {
                        self.open_detail(ContentSource::Album(id), name, "专辑")
                    }
                    Some(ContentSource::Endpoint(api)) if api == "album_sublist" => {
                        self.open_detail(ContentSource::Album(id), name, "专辑")
                    }
                    Some(ContentSource::Endpoint(api))
                        if api == "user_radio_sublist" || api == "radio_recommend" =>
                    {
                        self.open_detail(ContentSource::Radio(id), name, "电台")
                    }
                    _ => self.open_detail(ContentSource::Playlist(id), name, "歌单"),
                }
            }
            ContentState::TopLists(lists) => {
                let Some(list) = lists.get(row) else {
                    return Command::none();
                };
                let name = list.name.clone();
                self.open_detail(ContentSource::Toplist(list.id), name, "排行榜")
            }
            ContentState::Singers(singers) => {
                let Some(singer) = singers.get(row) else {
                    return Command::none();
                };
                self.open_artist(singer.id)
            }
            ContentState::HotSearch(keywords) => {
                let Some(keyword) = keywords.0.get(row) else {
                    return Command::none();
                };
                self.search.query = keyword.clone();
                self.search.kind = SearchKind::Songs;
                self.submit_search()
            }
            ContentState::Empty | ContentState::Loading | ContentState::Error(_) => Command::none(),
        }
    }

    fn queue_key(&self) -> String {
        if let Some(subtitle) = self.subtitle.as_deref().filter(|s| !s.trim().is_empty()) {
            return subtitle.to_string();
        }
        if !self.title.trim().is_empty() {
            return self.title.clone();
        }
        "默认队列".into()
    }

    fn submit_search(&mut self) -> Command<Message> {
        let query = self.search.query.trim().to_string();
        if query.is_empty() {
            return Command::none();
        }
        if !matches!(self.source, Some(ContentSource::Search(..))) {
            self.push_history();
        }
        self.search.query = query.clone();
        self.search.active = true;
        self.page = Page::Browse;
        self.artist = None;
        self.title = format!("搜索: {query}");
        self.subtitle = Some(self.search.kind.label().into());
        self.begin_load(ContentSource::Search(self.search.kind, query));
        let source = self.source.clone().expect("source set");
        self.reload_source(source, true)
    }

    fn toggle_like(&mut self, song_id: u64) -> Command<Message> {
        if self.user.is_none() {
            self.banner = Banner {
                text: "登录后才能收藏".into(),
                error: true,
            };
            return Command::none();
        }
        let like = !self
            .liked_ids
            .lock()
            .map(|ids| ids.contains(&song_id))
            .unwrap_or(false);
        if let Ok(mut ids) = self.liked_ids.lock() {
            if like {
                ids.insert(song_id);
            } else {
                ids.remove(&song_id);
            }
        }
        if self.engine.current_song().map(|song| song.id) == Some(song_id) {
            self.engine.update_liked_status();
        }
        let service = self.service.clone();
        Command::perform(
            async move {
                service
                    .like_song(song_id, like)
                    .await
                    .map_err(|error| error.to_string())
            },
            |result| Message::Internal(Internal::LikeDone(result)),
        )
    }

    fn set_volume(&mut self, volume: f64) {
        self.engine.set_volume(volume.clamp(0.0, 1.0));
    }

    fn start_qr(&mut self) -> Command<Message> {
        self.qr_generation = self.qr_generation.wrapping_add(1);
        let generation = self.qr_generation;
        self.qr_url.clear();
        self.qr_key = None;
        self.qr_status = "正在生成二维码…".into();
        self.qr_polls = 0;
        self.qr_scanned = false;
        self.qr_in_flight = false;
        self.qr_poll_due = None;
        self.login_loading = true;
        self.login_error = None;
        self.login_notice = None;
        let service = self.service.clone();
        Command::perform(
            async move {
                service
                    .login_qr_create()
                    .await
                    .map_err(|error| error.to_string())
            },
            move |result| Message::Internal(Internal::QrCreated { generation, result }),
        )
    }

    fn submit_login(&mut self) -> Command<Message> {
        self.login_loading = true;
        self.login_error = None;
        self.login_notice = None;
        match self.login_method {
            LoginMethod::Qr => {
                self.login_loading = false;
                self.start_qr()
            }
            LoginMethod::Sign => {
                self.login_loading = false;
                self.daily_sign()
            }
            LoginMethod::Password => {
                let account = self.login_account.trim().to_string();
                let password = self.login_password.clone();
                if account.is_empty() || password.is_empty() {
                    self.login_loading = false;
                    self.login_error = Some("请填写账号和密码".into());
                    return Command::none();
                }
                let service = self.service.clone();
                Command::perform(
                    async move {
                        service
                            .login_password(&account, &password)
                            .await
                            .map_err(|error| error.to_string())
                    },
                    |result| Message::Internal(Internal::LoginFinished(result)),
                )
            }
            LoginMethod::Sms => {
                let phone = self.login_phone.trim().to_string();
                let code = self.login_code.trim().to_string();
                if phone.is_empty() || code.is_empty() {
                    self.login_loading = false;
                    self.login_error = Some("请填写手机号和验证码".into());
                    return Command::none();
                }
                let service = self.service.clone();
                Command::perform(
                    async move {
                        service
                            .login_sms(&phone, &code)
                            .await
                            .map_err(|error| error.to_string())
                    },
                    |result| Message::Internal(Internal::LoginFinished(result)),
                )
            }
        }
    }

    fn send_sms_code(&mut self) -> Command<Message> {
        let phone = self.login_phone.trim().to_string();
        if phone.is_empty() {
            self.login_error = Some("请填写手机号".into());
            return Command::none();
        }
        self.login_loading = true;
        let service = self.service.clone();
        Command::perform(
            async move {
                service
                    .send_sms_code(&phone)
                    .await
                    .map_err(|error| error.to_string())
            },
            |result| Message::Internal(Internal::SmsSent(result)),
        )
    }

    fn sign_in(&mut self, account: String, password: String) -> Command<Message> {
        self.page = Page::Login;
        self.login_method = LoginMethod::Password;
        self.login_account = account;
        self.login_password = password;
        self.submit_login()
    }

    fn send_sms_to(&mut self, phone: String) -> Command<Message> {
        self.page = Page::Login;
        self.login_method = LoginMethod::Sms;
        self.login_phone = phone;
        self.send_sms_code()
    }

    fn sms_login(&mut self, phone: String, code: String) -> Command<Message> {
        self.page = Page::Login;
        self.login_method = LoginMethod::Sms;
        self.login_phone = phone;
        self.login_code = code;
        self.submit_login()
    }

    fn trash_current_fm_song(&mut self) -> Command<Message> {
        let Some(song) = self.engine.current_song() else {
            self.banner = Banner {
                text: "没有正在播放的歌曲".into(),
                error: true,
            };
            return Command::none();
        };
        let service = self.service.clone();
        let song_id = song.id;
        let name = song.name.clone();
        Command::perform(
            async move { service.fm_trash(song_id).await },
            move |result| {
                Message::Internal(Internal::ActionResult(
                    result.map(|()| format!("已从私人FM移除：{name}")),
                ))
            },
        )
    }

    fn daily_sign(&mut self) -> Command<Message> {
        let service = self.service.clone();
        Command::perform(
            async move {
                service
                    .daily_sign()
                    .await
                    .map(|msg| msg.msg)
                    .map_err(|error| error.to_string())
            },
            |result| Message::Internal(Internal::ActionResult(result)),
        )
    }

    fn logout(&mut self) -> Command<Message> {
        let service = self.service.clone();
        Command::perform(
            async move { service.logout().await.map_err(|error| error.to_string()) },
            |result| Message::Internal(Internal::LoggedOut(result)),
        )
    }

    fn login_succeeded(&mut self, user: LoginInfo) -> Command<Message> {
        self.login_loading = false;
        self.login_error = None;
        self.login_notice = Some(format!("登录成功：{}", user.nickname));
        self.user = Some(user.clone());
        self.service.client().flush_cookies();
        self.page = Page::Browse;
        self.history.clear();
        self.artist = None;
        let liked = self.load_liked(user.uid);
        let target = self.first_nav_for_session(true);
        let content = target.map(|(section, item)| self.activate_nav(section, item));
        Command::batch([liked, content.unwrap_or_else(Command::none)])
    }

    fn load_liked(&mut self, uid: u64) -> Command<Message> {
        let service = self.service.clone();
        let liked = Arc::clone(&self.liked_ids);
        Command::perform(
            async move {
                match service.load_liked_song_ids(uid).await {
                    Ok(ids) => {
                        if let Ok(mut guard) = liked.lock() {
                            *guard = ids.clone();
                        }
                        Ok(ids)
                    }
                    Err(error) => Err(error.to_string()),
                }
            },
            |result| Message::Internal(Internal::LikedLoaded(result)),
        )
    }

    fn first_nav_for_session(&self, logged_in: bool) -> Option<(usize, usize)> {
        const PUBLIC: &[&str] = &["top_song_list", "toplist", "top_singers", "search"];
        for (section_index, section) in self.nav_sections.iter().enumerate() {
            for (item_index, item) in section.items.iter().enumerate() {
                let Some(api) = item.api.as_deref() else {
                    continue;
                };
                if logged_in || PUBLIC.contains(&api) {
                    return Some((section_index, item_index));
                }
            }
        }
        self.nav_sections
            .iter()
            .enumerate()
            .find_map(|(section, section_value)| {
                (!section_value.items.is_empty()).then_some((section, 0))
            })
    }

    fn load_media(&mut self) -> Vec<Command<Message>> {
        let Some(song) = self.engine.current_song() else {
            return Vec::new();
        };
        let song_id = song.id;
        let mut commands = Vec::new();

        let cache = self.cache.clone();
        let http = self.http.clone();
        let pic_url = song.pic_url.clone();
        commands.push(Command::perform(
            async move {
                let bytes = match cache.load_cover_async(song_id).await {
                    Some(bytes) => Some(bytes),
                    None if !pic_url.is_empty() => {
                        let url = if pic_url.contains('?') {
                            format!("{pic_url}&param=400y400")
                        } else {
                            format!("{pic_url}?param=400y400")
                        };
                        match http.get(&url).send().await {
                            Ok(response) => match response.bytes().await {
                                Ok(bytes) => {
                                    let bytes = bytes.to_vec();
                                    cache.save_cover(song_id, &bytes);
                                    Some(bytes)
                                }
                                Err(_) => None,
                            },
                            Err(_) => None,
                        }
                    }
                    None => None,
                };
                Internal::CoverLoaded { song_id, bytes }
            },
            Message::Internal,
        ));

        if song.mv != 0 {
            let service = self.service.clone();
            let http = self.http.clone();
            let mv_id = song.mv;
            commands.push(Command::perform(
                async move {
                    let info = service
                        .client()
                        .mv_detail(mv_id)
                        .await
                        .map_err(|error| error.to_string())?;
                    let bytes = if info.cover.is_empty() {
                        None
                    } else {
                        let separator = if info.cover.contains('?') { '&' } else { '?' };
                        let url = format!("{}{separator}param=480y480", info.cover);
                        match http.get(url).send().await {
                            Ok(response) => response.bytes().await.ok().map(|bytes| bytes.to_vec()),
                            Err(_) => None,
                        }
                    };
                    Ok((Box::new(info), bytes))
                },
                move |result| Message::Internal(Internal::MvLoaded { mv_id, result }),
            ));
        }

        if let Some(audio) = local_audio_path(&song) {
            commands.push(Command::perform(
                async move {
                    let lyrics = tokio::task::spawn_blocking(move || load_local_lyrics(&audio))
                        .await
                        .ok()
                        .flatten();
                    let (lyrics, translated) = lyrics
                        .map(|lyrics| {
                            (
                                parse_lyric_lines(&lyrics.lyric),
                                parse_lyric_lines(&lyrics.tlyric),
                            )
                        })
                        .unwrap_or_default();
                    Internal::LyricsLoaded {
                        song_id,
                        lyrics,
                        translated,
                    }
                },
                Message::Internal,
            ));
        } else {
            let service = self.service.clone();
            commands.push(Command::perform(
                async move {
                    let (lyrics, translated) = match service.load_lyrics(song_id).await {
                        Some(lyrics) => (
                            parse_lyric_lines(&lyrics.lyric),
                            parse_lyric_lines(&lyrics.tlyric),
                        ),
                        None => (Vec::new(), Vec::new()),
                    };
                    Internal::LyricsLoaded {
                        song_id,
                        lyrics,
                        translated,
                    }
                },
                Message::Internal,
            ));
        }
        commands
    }

    fn on_playback(&mut self, event: PlaybackEvent) -> Option<Command<Message>> {
        match event {
            PlaybackEvent::SongPlay(id) => {
                if self.engine.is_currently_playing(id) {
                    self.engine.toggle_pause();
                } else if !self.engine.play_song_by_id(id)
                    && let ContentState::Songs(songs) = self.content.as_ref()
                    && let Some(index) = songs.iter().position(|song| song.id == id)
                {
                    self.engine
                        .play_songs(&self.queue_key(), songs.to_vec(), index);
                }
                None
            }
            PlaybackEvent::Started => {
                self.reported_listen = None;
                self.played_in_track = Duration::ZERO;
                self.last_position = Duration::ZERO;
                self.engine.on_playback_started();
                self.cover = None;
                self.mv = None;
                self.mv_poster = None;
                Some(Command::batch(self.load_media()))
            }
            PlaybackEvent::Progress { position, total } => {
                self.engine.on_playback_progress(position, total);
                self.played_in_track += played_advance(self.last_position, position);
                self.last_position = position;
                let song = self.engine.current_song()?;
                if !listen_due(self.played_in_track, song.id, self.reported_listen) {
                    return None;
                }
                self.reported_listen = Some(song.id);
                let service = self.service.clone();
                let song_id = song.id;
                let played = self.played_in_track.as_millis() as u64;
                Some(Command::perform(
                    async move {
                        let _ = service.report_play(song_id, played).await;
                    },
                    |()| Message::Internal(Internal::Noop),
                ))
            }
            PlaybackEvent::Finished => {
                let finished = self.engine.finish_and_snapshot();
                if let Some((song_id, duration_ms, progress)) = finished
                    && progress >= 0.9
                    && Duration::from_millis(duration_ms) < LISTEN_THRESHOLD
                {
                    let service = self.service.clone();
                    return Some(Command::perform(
                        async move {
                            let _ = service.report_play(song_id, duration_ms).await;
                        },
                        |()| Message::Internal(Internal::Noop),
                    ));
                }
                None
            }
            PlaybackEvent::Error(error) => {
                self.banner = Banner {
                    text: error.clone(),
                    error: true,
                };
                self.engine.on_playback_error(error);
                None
            }
            PlaybackEvent::LyricsLoaded {
                song_id,
                lyrics,
                translated_lyrics,
            } => {
                self.engine
                    .on_lyrics_loaded(song_id, lyrics, translated_lyrics);
                None
            }
            PlaybackEvent::HeartbeatSong(song) => {
                self.engine.play_heartbeat_song(song);
                None
            }
            PlaybackEvent::HeartbeatFallback => {
                self.engine.on_heartbeat_fallback();
                None
            }
            PlaybackEvent::SetPlaylistId(id) => {
                self.queued_playlists.remove(&id);
                self.engine.set_playlist_id(id);
                None
            }
            PlaybackEvent::LikeSong(id, like) => {
                if let Ok(mut ids) = self.liked_ids.lock() {
                    if like {
                        ids.insert(id);
                    } else {
                        ids.remove(&id);
                    }
                }
                if self.engine.current_song().map(|song| song.id) == Some(id) {
                    self.engine.update_liked_status();
                }
                let service = self.service.clone();
                Some(Command::perform(
                    async move {
                        let _ = service.like_song(id, like).await;
                    },
                    |()| Message::Internal(Internal::Noop),
                ))
            }
            PlaybackEvent::DislikeSong(id) => {
                let service = self.service.clone();
                Some(Command::perform(
                    async move {
                        let _ = service.dislike_song(id).await;
                    },
                    |()| Message::Internal(Internal::Noop),
                ))
            }
            PlaybackEvent::LikedUpdated => {
                self.engine.update_liked_status();
                None
            }
            PlaybackEvent::Cached(song_id) => {
                if self.engine.current_song().map(|song| song.id) == Some(song_id) {
                    self.engine.state.cached = true;
                }
                None
            }
            PlaybackEvent::QueueAppend { key, songs } => {
                self.engine.append_songs_to_key(&key, songs);
                None
            }
            PlaybackEvent::PlaySongs { key, songs, index } => {
                self.engine.play_songs(&key, songs, index);
                None
            }
            PlaybackEvent::QueueLoadDone { playlist_id } => {
                self.queued_playlists.insert(playlist_id);
                None
            }
        }
    }

    fn tick_qr(&mut self) -> Option<Command<Message>> {
        let due = self.qr_poll_due?;
        if self.qr_in_flight || Instant::now() < due || self.qr_key.is_none() {
            return None;
        }
        if self.qr_polls >= QR_POLLS {
            self.qr_poll_due = None;
            self.qr_status = "登录超时".into();
            self.login_error = Some("登录超时，请重新生成二维码".into());
            return None;
        }
        let key = self.qr_key.clone()?;
        let generation = self.qr_generation;
        self.qr_polls += 1;
        self.qr_in_flight = true;
        self.qr_poll_due = Some(Instant::now() + QR_POLL_INTERVAL);
        let service = self.service.clone();
        Some(Command::perform(
            async move {
                service
                    .login_qr_check(&key)
                    .await
                    .map_err(|error| error.to_string())
            },
            move |result| Message::Internal(Internal::QrChecked { generation, result }),
        ))
    }

    fn theme_view(&self) -> ThemeView {
        let theme = self
            .theme_registry
            .get(&self.config.default_theme)
            .unwrap_or_else(|| theme_fallback());
        ThemeView {
            background: rgb(theme.bg),
            surface: rgb(theme.surface),
            text: rgb(theme.text),
            accent: rgb(theme.accent),
            muted: rgb(theme.muted),
            border: rgb(theme.border),
            error: rgb(theme.error),
            warning: rgb(theme.warn),
        }
    }
}

impl App for Backend {
    type Message = Message;
    type View = View;

    fn init(&mut self) -> Command<Message> {
        let service = self.service.clone();
        let login = Command::perform(
            async move {
                service
                    .login_status()
                    .await
                    .ok()
                    .filter(|user| user.uid != 0)
            },
            |user| Message::Internal(Internal::UserLoaded(user)),
        );
        Command::batch([login, self.next_bridge()])
    }

    fn update(&mut self, message: Message) -> Command<Message> {
        match message {
            Message::Navigate(page) => {
                self.page = page;
                if page == Page::Login && self.user.is_none() && self.qr_url.is_empty() {
                    return self.start_qr();
                }
                Command::none()
            }
            Message::ActivateNav { section, item } => self.activate_nav(section, item),
            Message::Refresh => self.reload_current(true),
            Message::Back => self.back(),
            Message::LoadMore => {
                let Some(pagination) = self.pagination.clone() else {
                    return Command::none();
                };
                if !pagination.has_more || pagination.loading {
                    return Command::none();
                }
                let generation = self.content_generation;
                let source = self
                    .source
                    .clone()
                    .unwrap_or(ContentSource::Endpoint(String::new()));
                if let Some(pagination) = &mut self.pagination {
                    pagination.loading = true;
                }
                let service = self.service.clone();
                let api = pagination.api.clone();
                let offset = pagination.next_offset();
                let limit = pagination.limit;
                Command::perform(
                    async move {
                        let result = service.load_more(&api, offset, limit).await;
                        (result, source)
                    },
                    move |(result, source)| {
                        let (content, pagination) = match result {
                            Some((content, pagination)) => (content, Some(pagination)),
                            None => (ContentState::Error("加载更多失败".into()), None),
                        };
                        Message::Internal(Internal::ContentLoaded {
                            generation,
                            source,
                            content,
                            pagination,
                            playlist_id: None,
                            title: None,
                            append: true,
                        })
                    },
                )
            }
            Message::ActivateRow(row) => self.play_row(row),
            Message::AddNextRow(row) => {
                if let ContentState::Songs(songs) = self.content.as_ref()
                    && let Some(song) = songs.get(row)
                {
                    self.engine.add_next(Arc::clone(song));
                    self.banner = Banner {
                        text: format!("已添加到下一首：{}", song.name),
                        error: false,
                    };
                }
                Command::none()
            }
            Message::LikeRow(row) => {
                let song_id = match self.content.as_ref() {
                    ContentState::Songs(songs) => songs.get(row).map(|song| song.id),
                    _ => None,
                };
                song_id
                    .map(|id| self.toggle_like(id))
                    .unwrap_or_else(Command::none)
            }
            Message::UploadRow(row) => self.upload_row(row),
            Message::OpenArtistRow(row) => {
                let artist_id = match self.content.as_ref() {
                    ContentState::Songs(songs) => songs.get(row).map(|song| song.artist_id),
                    ContentState::Singers(singers) => singers.get(row).map(|singer| singer.id),
                    _ => None,
                };
                artist_id
                    .map(|id| self.open_artist(id))
                    .unwrap_or_else(Command::none)
            }
            Message::OpenAlbumRow(row) => {
                let album = match self.content.as_ref() {
                    ContentState::Songs(songs) => songs
                        .get(row)
                        .map(|song| (song.album_id, song.album.clone())),
                    _ => None,
                };
                match album {
                    Some((0, _)) | None => Command::none(),
                    Some((id, name)) => self.open_detail(ContentSource::Album(id), name, "专辑"),
                }
            }
            Message::PlayArtistSong(index) => {
                let Some(artist) = &self.artist else {
                    return Command::none();
                };
                let Some(song) = artist.detail.hot_songs.get(index) else {
                    return Command::none();
                };
                let songs: Vec<Arc<SongInfo>> = artist
                    .detail
                    .hot_songs
                    .iter()
                    .cloned()
                    .map(Arc::new)
                    .collect();
                let name = artist.detail.name.clone();
                if self.engine.is_currently_playing(song.id) {
                    self.engine.toggle_pause();
                } else {
                    self.engine
                        .play_songs(&format!("歌手: {name}"), songs, index);
                    self.banner = Banner {
                        text: format!("▶ {}", song.name),
                        error: false,
                    };
                }
                Command::none()
            }
            Message::AddNextArtistSong(index) => {
                if let Some(artist) = &self.artist
                    && let Some(song) = artist.detail.hot_songs.get(index)
                {
                    self.engine.add_next(Arc::new(song.clone()));
                    self.banner = Banner {
                        text: format!("已添加到下一首：{}", song.name),
                        error: false,
                    };
                }
                Command::none()
            }
            Message::LikeArtistSong(index) => {
                let id = self
                    .artist
                    .as_ref()
                    .and_then(|artist| artist.detail.hot_songs.get(index))
                    .map(|song| song.id);
                id.map(|id| self.toggle_like(id))
                    .unwrap_or_else(Command::none)
            }
            Message::OpenArtistAlbum(index) => {
                let album = self
                    .artist
                    .as_ref()
                    .and_then(|artist| artist.albums.get(index))
                    .map(|album| (album.id, album.name.clone()));
                match album {
                    Some((id, name)) => self.open_detail(ContentSource::Album(id), name, "专辑"),
                    None => Command::none(),
                }
            }
            Message::OpenSimilarArtist(index) => {
                let id = self
                    .artist
                    .as_ref()
                    .and_then(|artist| artist.similar.get(index))
                    .map(|singer| singer.id);
                id.map(|id| self.open_artist(id))
                    .unwrap_or_else(Command::none)
            }
            Message::OpenSongContext(similar) => {
                let Some(song) = self.engine.current_song() else {
                    return Command::none();
                };
                let source = if similar {
                    ContentSource::SimilarSongs(song.id)
                } else {
                    ContentSource::SimilarPlaylists(song.id)
                };
                let title = if similar {
                    "相似歌曲".to_string()
                } else {
                    "包含这首歌的歌单".to_string()
                };
                self.open_detail(source, title, "")
            }
            Message::OpenPersonalFm => self.open_personal_fm(),
            Message::TrashFm => self.trash_current_fm_song(),
            Message::PlayQueueRow(index) => {
                self.engine.play_index(index);
                Command::none()
            }
            Message::SwitchQueue(forward) => {
                self.engine.switch_queue(forward);
                Command::none()
            }
            Message::ActivateQueue(key) => {
                self.engine.activate_queue(&key);
                Command::none()
            }
            Message::ClearQueue => {
                self.engine.clear_queue();
                Command::none()
            }
            Message::TogglePlay => {
                self.engine.toggle_pause();
                Command::none()
            }
            Message::Next => {
                self.engine.next();
                Command::none()
            }
            Message::Prev => {
                self.engine.prev();
                Command::none()
            }
            Message::Seek(fraction) => {
                self.engine.seek_to_fraction(fraction);
                Command::none()
            }
            Message::SetVolume(volume) => {
                self.set_volume(volume);
                Command::none()
            }
            Message::CycleMode => {
                let mode = self.engine.cycle_mode();
                self.banner = Banner {
                    text: format!("播放模式：{}", crate::playback::mode_icon(&mode).1),
                    error: false,
                };
                Command::none()
            }
            Message::ToggleLike => match self.engine.current_song() {
                Some(song) => self.toggle_like(song.id),
                None => Command::none(),
            },
            Message::SearchInput(value) => {
                self.search.query = value;
                self.search.active = true;
                Command::none()
            }
            Message::Search(query) => {
                self.search.query = query;
                self.search.active = true;
                self.submit_search()
            }
            Message::SetSearchKind(kind) => {
                self.search.kind = kind;
                if !self.search.query.trim().is_empty() {
                    self.submit_search()
                } else {
                    Command::none()
                }
            }
            Message::SearchSubmit => self.submit_search(),
            Message::SetLoginMethod(method) => {
                self.login_method = method;
                self.login_error = None;
                self.login_notice = None;
                if method == LoginMethod::Qr && self.user.is_none() && self.qr_url.is_empty() {
                    self.start_qr()
                } else {
                    Command::none()
                }
            }
            Message::SetLoginField(field, value) => {
                *self.login_field_mut(field) = value;
                Command::none()
            }
            Message::AppendLoginField(field, ch) => {
                self.login_field_mut(field).push(ch);
                Command::none()
            }
            Message::BackspaceLoginField(field) => {
                self.login_field_mut(field).pop();
                Command::none()
            }
            Message::ClearLoginField(field) => {
                self.login_field_mut(field).clear();
                Command::none()
            }
            Message::SendSmsCode => self.send_sms_code(),
            Message::SignIn { account, password } => self.sign_in(account, password),
            Message::SendSms { phone } => self.send_sms_to(phone),
            Message::SmsLogin { phone, code } => self.sms_login(phone, code),
            Message::SubmitLogin => self.submit_login(),
            Message::StartQr | Message::RefreshQr => self.start_qr(),
            Message::DailySign => self.daily_sign(),
            Message::Logout => self.logout(),
            Message::ToggleSaveOnPlay => {
                self.config.cache.save_on_play = !self.config.cache.save_on_play;
                self.engine.set_save_on_play(self.config.cache.save_on_play);
                self.config.save();
                Command::none()
            }
            Message::CycleQuality => {
                const QUALITIES: &[&str] = &[
                    "standard", "higher", "exhigh", "lossless", "hires", "jyeffect", "sky",
                    "jymaster",
                ];
                let current = QUALITIES
                    .iter()
                    .position(|quality| *quality == self.config.cache.quality)
                    .unwrap_or(1);
                let next = QUALITIES[(current + 1) % QUALITIES.len()].to_string();
                self.config.cache.quality = next.clone();
                if let Some(quality) = ncm_api::SongQuality::from_level(&next) {
                    self.engine.set_quality(quality);
                }
                self.config.save();
                Command::none()
            }
            Message::ToggleNotifySongChange => {
                self.config.notify.song_change = !self.config.notify.song_change;
                self.config.save();
                Command::none()
            }
            Message::ToggleNotifyErrors => {
                self.config.notify.errors = !self.config.notify.errors;
                self.config.save();
                Command::none()
            }
            Message::ToggleVisualizer => {
                self.config.playerbar.visible.visualizer =
                    !self.config.playerbar.visible.visualizer;
                self.config.save();
                Command::none()
            }
            Message::TogglePitch => {
                self.config.playerbar.visible.pitch = !self.config.playerbar.visible.pitch;
                self.config.save();
                Command::none()
            }
            Message::CycleLyricStyle => {
                self.config.lyric_style = self.config.lyric_style.next();
                self.config.save();
                self.banner = Banner {
                    text: format!("歌词样式：{}", self.config.lyric_style.name()),
                    error: false,
                };
                Command::none()
            }
            Message::SetLyricStyle(name) => {
                match crate::config::LyricStyle::parse(&name) {
                    Some(style) => {
                        self.config.lyric_style = style;
                        self.config.save();
                        self.banner = Banner {
                            text: format!("歌词样式：{}", style.name()),
                            error: false,
                        };
                    }
                    None => {
                        self.banner = Banner {
                            text: format!("未知歌词样式：{name}"),
                            error: true,
                        };
                    }
                }
                Command::none()
            }
            Message::CycleNavPosition => {
                self.config.navigation_position = self.config.navigation_position.cycle();
                self.config.save();
                self.banner = Banner {
                    text: format!("导航栏位置：{}", self.config.navigation_position.label()),
                    error: false,
                };
                Command::none()
            }
            Message::SetPaneSize(pane, size) => {
                self.config.panes.set_size(pane, size);
                self.config.save();
                Command::none()
            }
            Message::TogglePane(pane) => {
                self.config.panes.toggle(pane);
                self.config.save();
                Command::none()
            }
            Message::ToggleTranslation => {
                self.config.lyric_translation = !self.config.lyric_translation;
                self.config.save();
                Command::none()
            }
            Message::CycleTheme => {
                let names = self.theme_registry.choosable_names();
                if !names.is_empty() {
                    let current = names
                        .iter()
                        .position(|name| *name == self.config.default_theme)
                        .unwrap_or(0);
                    let next = names[(current + 1) % names.len()].to_string();
                    self.config.default_theme = next;
                    self.config.save();
                }
                Command::none()
            }
            Message::SetTheme(name) => {
                if self
                    .theme_registry
                    .choosable_names()
                    .iter()
                    .any(|theme| *theme == name)
                {
                    self.config.default_theme = name;
                    self.config.save();
                } else {
                    self.banner = Banner {
                        text: format!("未知主题：{name}"),
                        error: true,
                    };
                }
                Command::none()
            }
            Message::SetQuality(level) => {
                match ncm_api::SongQuality::from_level(&level) {
                    Some(quality) => {
                        self.config.cache.quality = level;
                        self.engine.set_quality(quality);
                        self.config.save();
                    }
                    None => {
                        self.banner = Banner {
                            text: format!("未知音质：{level}"),
                            error: true,
                        };
                    }
                }
                Command::none()
            }
            Message::Internal(internal) => self.handle_internal(internal),
        }
    }

    fn view(&self) -> View {
        let song = self.engine.current_song();
        let duration_secs = song
            .as_ref()
            .map(|song| song.duration as f64 / 1000.0)
            .unwrap_or(0.0);
        let player = PlayerView {
            song: song.clone(),
            playing: self.engine.state.playing,
            paused: self.engine.state.paused,
            progress: self.engine.state.progress,
            position_secs: self.engine.state.position_secs,
            duration_secs,
            volume: self.engine.state.volume,
            mode: self.engine.state.mode,
            liked: self.engine.state.liked,
            cached: self.engine.state.cached,
            cover: self.cover.clone(),
            lyrics: self.engine.state.lyrics.clone().unwrap_or_default(),
            translated_lyrics: self
                .engine
                .state
                .translated_lyrics
                .clone()
                .unwrap_or_default(),
            visualizer: self.engine.state.visualizer.clone(),
            pitch: self.engine.state.pitch.as_ref().map(|note| note.label()),
            mv: self.mv.clone(),
            mv_poster: self.mv_poster.clone(),
            queue: self.engine.queue_songs().to_vec(),
            queue_keys: self.engine.queue_keys().to_vec(),
            queue_key: self.engine.queue_key().to_string(),
            queue_index: self.engine.queue_current_index(),
        };
        let view = View {
            page: self.page,
            nav: NavView {
                sections: self.nav_sections.clone(),
                section: self.nav_section,
                item: self.nav_item,
            },
            content: self.content.as_ref().clone(),
            title: self.title.clone(),
            subtitle: self.subtitle.clone(),
            source: self.source.clone(),
            pagination: self.pagination.clone(),
            loading: self.loading,
            banner: self.banner.clone(),
            user: self.user.clone(),
            can_back: self.can_back(),
            artist: self.artist.clone(),
            player,
            search: self.search.clone(),
            panes: self.config.panes.clone(),
            nav_position: self.config.navigation_position,
            login: LoginView {
                method: self.login_method,
                account: self.login_account.clone(),
                password: self.login_password.clone(),
                phone: self.login_phone.clone(),
                code: self.login_code.clone(),
                qr_url: self.qr_url.clone(),
                qr_status: self.qr_status.clone(),
                loading: self.login_loading,
                error: self.login_error.clone(),
                notice: self.login_notice.clone(),
            },
            settings: SettingsView {
                theme: self.config.default_theme.clone(),
                themes: self
                    .theme_registry
                    .choosable_names()
                    .into_iter()
                    .map(str::to_string)
                    .collect(),
                quality: self.config.cache.quality.clone(),
                save_on_play: self.config.cache.save_on_play,
                notify_song_change: self.config.notify.song_change,
                notify_errors: self.config.notify.errors,
                lyric_translation: self.config.lyric_translation,
                visualizer: self.config.playerbar.visible.visualizer,
                pitch: self.config.playerbar.visible.pitch,
                lyric_style: self.config.lyric_style,
            },
            theme: self.theme_view(),
        };
        if let Some(bridge) = &self.bridge {
            bridge.publish(&view, &self.cache);
        }
        view
    }

    fn subscriptions(&self) -> Subscription<Message> {
        Subscription::interval(1, TICK, Message::Internal(Internal::Tick))
    }

    fn shutdown(&mut self) {
        self.engine.save_session();
        self.service.client().flush_cookies();
    }
}

impl Backend {
    fn poll_tick(&mut self) -> Command<Message> {
        let mut commands = Vec::new();
        for _ in 0..MAX_EVENTS_PER_TICK {
            let Some(event) = self.events.try_next() else {
                break;
            };
            if let Event::App(AppEvent::Playback(event)) = event
                && let Some(command) = self.on_playback(event)
            {
                commands.push(command);
            }
        }
        if self.engine.state.playing && !self.engine.state.paused {
            self.engine.update_analysis();
        }
        if let Some(command) = self.tick_qr() {
            commands.push(command);
        }
        Command::batch(commands)
    }

    fn login_field_mut(&mut self, field: LoginField) -> &mut String {
        match field {
            LoginField::Account => &mut self.login_account,
            LoginField::Password => &mut self.login_password,
            LoginField::Phone => &mut self.login_phone,
            LoginField::Code => &mut self.login_code,
        }
    }

    fn next_bridge(&self) -> Command<Message> {
        let Some(bridge) = &self.bridge else {
            return Command::none();
        };
        let requests = Arc::clone(&bridge.requests);
        Command::perform(
            async move { requests.lock().await.recv().await },
            |request| match request {
                Some(request) => Message::Internal(Internal::Bridge(request)),
                None => Message::Internal(Internal::Noop),
            },
        )
    }

    fn handle_bridge(&mut self, request: BridgeRequest) -> Command<Message> {
        match request {
            BridgeRequest::Control(event) => self.handle_control(event),
            BridgeRequest::Search { keyword, reply } => {
                let service = self.service.clone();
                let results = Arc::clone(&self.search_results);
                let limit = self.config.search_limit;
                tokio::spawn(async move {
                    let entries = match service.search_songs(&keyword, limit).await {
                        ContentState::Songs(songs) => {
                            if let Ok(mut cached) = results.lock() {
                                for song in &songs {
                                    cached.insert(song.id, Arc::clone(song));
                                }
                            }
                            songs
                                .iter()
                                .map(|song| SearchEntry::from_song(song, "netease"))
                                .collect()
                        }
                        _ => Vec::new(),
                    };
                    let _ = reply.send(entries);
                });
                Command::none()
            }
        }
    }

    fn handle_control(&mut self, event: IpcEvent) -> Command<Message> {
        match event {
            IpcEvent::Previous => {
                self.engine.prev();
                Command::none()
            }
            IpcEvent::Next => {
                self.engine.next();
                Command::none()
            }
            IpcEvent::Pause => {
                if self.engine.state.playing && !self.engine.state.paused {
                    self.engine.toggle_pause();
                }
                Command::none()
            }
            IpcEvent::Play { song_id } => {
                if let Some(id) = song_id {
                    let mut played = self.engine.play_song_by_id(id);
                    if !played {
                        let song = self
                            .search_results
                            .lock()
                            .ok()
                            .and_then(|results| results.get(&id).cloned());
                        if let Some(song) = song {
                            self.engine.append_and_play_key(
                                NCM_SEARCH_QUEUE_KEY,
                                std::slice::from_ref(&song),
                                0,
                            );
                            played = true;
                        }
                    }
                    if played {
                        let name = self
                            .engine
                            .current_song()
                            .map(|song| song.name.clone())
                            .unwrap_or_default();
                        self.banner = Banner {
                            text: format!("正在播放：{name}"),
                            error: false,
                        };
                    } else {
                        self.banner = Banner {
                            text: format!("找不到 id={id} 的歌曲"),
                            error: true,
                        };
                    }
                } else if self.engine.state.paused || !self.engine.state.playing {
                    self.engine.toggle_pause();
                }
                Command::none()
            }
            IpcEvent::TogglePlay => {
                self.engine.toggle_pause();
                Command::none()
            }
            IpcEvent::Volume { delta, absolute } => {
                let volume = absolute.unwrap_or(self.engine.state.volume + delta.unwrap_or(0.0));
                self.engine.set_volume(volume.clamp(0.0, 1.0));
                Command::none()
            }
            IpcEvent::Seek { value } => {
                match parse_seek(&value) {
                    Ok(SeekTarget::Fraction(fraction)) => self.engine.seek_to_fraction(fraction),
                    Ok(SeekTarget::Relative(seconds)) => self.engine.seek_relative(seconds),
                    Ok(SeekTarget::Absolute(seconds)) => {
                        let duration = self
                            .engine
                            .current_song()
                            .map(|song| song.duration as f64 / 1000.0)
                            .unwrap_or(0.0);
                        self.engine
                            .seek_to_fraction((seconds / duration).clamp(0.0, 1.0));
                    }
                    Err(error) => {
                        self.banner = Banner {
                            text: error,
                            error: true,
                        };
                    }
                }
                Command::none()
            }
            IpcEvent::Clear => {
                self.engine.clear_queue();
                Command::none()
            }
            IpcEvent::Mode => {
                self.engine.cycle_mode();
                Command::none()
            }
            IpcEvent::SetMode(mode) => {
                self.engine.set_mode(mode);
                Command::none()
            }
            IpcEvent::Like => {
                if let Some(song) = self.engine.current_song()
                    && !self.engine.state.liked
                {
                    return self.toggle_like(song.id);
                }
                Command::none()
            }
            IpcEvent::Dislike => {
                if let Some(song) = self.engine.current_song() {
                    let service = self.service.clone();
                    tokio::spawn(async move {
                        let _ = service.dislike_song(song.id).await;
                    });
                }
                Command::none()
            }
            IpcEvent::ToggleLike => self
                .engine
                .current_song()
                .map(|song| self.toggle_like(song.id))
                .unwrap_or_else(Command::none),
            IpcEvent::SwitchList { endpoint, playlist } => {
                let service = self.service.clone();
                let uid = self.user.as_ref().map(|user| user.uid);
                let limit = self.config.search_limit;
                let key = self
                    .config
                    .navigation
                    .name_for_api(&endpoint)
                    .unwrap_or_else(|| endpoint.clone());
                let result_key = key.clone();
                Command::perform(
                    async move {
                        let Some(api) = ApiEndpoint::parse(&endpoint) else {
                            return Err(format!("未知端点：{endpoint}"));
                        };
                        let content = service.resolve_endpoint_content(api, uid, limit).await;
                        let songs = match content {
                            ContentState::Songs(songs) => songs,
                            ContentState::SongLists(lists) => {
                                let index = playlist.unwrap_or(1).saturating_sub(1);
                                let Some(list) = lists.get(index) else {
                                    return Err("歌单索引超出范围".into());
                                };
                                match service.load_playlist_detail(list.id, false, limit).await.0 {
                                    ContentState::Songs(songs) => songs,
                                    ContentState::Error(error) => return Err(error),
                                    _ => return Err("歌单没有歌曲".into()),
                                }
                            }
                            ContentState::TopLists(lists) => {
                                let index = playlist.unwrap_or(1).saturating_sub(1);
                                let Some(list) = lists.get(index) else {
                                    return Err("排行榜索引超出范围".into());
                                };
                                match service.client().top_songs(list.id).await {
                                    Ok(detail) => detail.songs.into_iter().map(Arc::new).collect(),
                                    Err(error) => return Err(error.to_string()),
                                }
                            }
                            ContentState::Error(error) => return Err(error),
                            _ => return Err("该端点没有可播放的歌曲".into()),
                        };
                        if songs.is_empty() {
                            Err("该端点没有歌曲".into())
                        } else {
                            Ok(songs)
                        }
                    },
                    move |result| {
                        Message::Internal(Internal::SwitchListLoaded {
                            key: result_key.clone(),
                            result,
                        })
                    },
                )
            }
        }
    }

    fn handle_internal(&mut self, internal: Internal) -> Command<Message> {
        match internal {
            Internal::Bridge(request) => {
                let next = self.next_bridge();
                let handled = self.handle_bridge(request);
                Command::batch([handled, next])
            }
            Internal::UserLoaded(user) => {
                self.loading = false;
                if let Some(user) = user {
                    self.banner = Banner {
                        text: format!("已登录：{}", user.nickname),
                        error: false,
                    };
                    self.user = Some(user.clone());
                    let liked = self.load_liked(user.uid);
                    let target = self.first_nav_for_session(true);
                    if let Some((section, item)) = target {
                        let content = self.activate_nav(section, item);
                        return Command::batch([liked, content]);
                    }
                    return liked;
                }
                self.banner = Banner {
                    text: "未登录：搜索与公开排行榜可用".into(),
                    error: false,
                };
                if let Some((section, item)) = self.first_nav_for_session(false) {
                    return self.activate_nav(section, item);
                }
                Command::none()
            }
            Internal::LikedLoaded(result) => {
                if let Err(error) = result {
                    log::warn!("load liked ids: {error}");
                }
                self.engine.update_liked_status();
                Command::none()
            }
            Internal::ContentLoaded {
                generation,
                source,
                content,
                pagination,
                playlist_id,
                title,
                append,
            } => {
                if generation != self.content_generation {
                    return Command::none();
                }
                self.loading = false;
                if let Some(playlist_id) = playlist_id {
                    self.engine.set_playlist_id(playlist_id);
                }
                if let Some(title) = title
                    && !matches!(source, ContentSource::Search(..))
                {
                    self.title = title;
                }
                if append {
                    if let ContentState::Songs(new_songs) = content
                        && let ContentState::Songs(existing) = Arc::make_mut(&mut self.content)
                    {
                        let mut ids: HashSet<u64> = existing.iter().map(|song| song.id).collect();
                        existing.extend(new_songs.into_iter().filter(|song| ids.insert(song.id)));
                    }
                    if let Some(pagination) = pagination {
                        let mut pagination = pagination;
                        pagination.loading = false;
                        self.pagination = Some(pagination);
                    } else if let Some(pagination) = &mut self.pagination {
                        pagination.loading = false;
                        self.banner = Banner {
                            text: "加载更多失败".into(),
                            error: true,
                        };
                    }
                } else {
                    self.content = Arc::new(content);
                    self.pagination = pagination;
                }
                self.source = Some(source);
                if let ContentState::Error(error) = self.content.as_ref() {
                    self.banner = Banner {
                        text: error.clone(),
                        error: true,
                    };
                }
                Command::none()
            }
            Internal::ArtistLoaded {
                generation,
                detail,
                albums,
                similar,
                error,
            } => {
                if generation != self.content_generation {
                    return Command::none();
                }
                self.loading = false;
                self.title = detail.name.clone();
                self.artist = Some(ArtistView {
                    detail,
                    albums,
                    similar,
                });
                if let Some(error) = error {
                    self.banner = Banner {
                        text: error,
                        error: true,
                    };
                }
                Command::none()
            }
            Internal::QrCreated { generation, result } => {
                if generation != self.qr_generation {
                    return Command::none();
                }
                self.login_loading = false;
                match result {
                    Ok((url, key)) => {
                        self.qr_url = url;
                        self.qr_key = Some(key);
                        self.qr_status = "等待扫码…".into();
                        self.qr_poll_due = Some(Instant::now() + QR_POLL_INTERVAL);
                        self.qr_in_flight = false;
                    }
                    Err(error) => {
                        self.qr_status = error.clone();
                        self.login_error = Some(error);
                    }
                }
                Command::none()
            }
            Internal::QrChecked { generation, result } => {
                if generation != self.qr_generation {
                    return Command::none();
                }
                self.qr_in_flight = false;
                match result {
                    Ok(msg) => match msg.code {
                        803 => {
                            self.qr_poll_due = None;
                            self.qr_status = "已确认，正在登录…".into();
                            let service = self.service.clone();
                            Command::perform(
                                async move {
                                    service
                                        .login_status()
                                        .await
                                        .map_err(|error| error.to_string())
                                },
                                |result| Message::Internal(Internal::QrLoginStatus(result)),
                            )
                        }
                        800 => {
                            self.qr_poll_due = None;
                            self.qr_status = "二维码已过期".into();
                            self.login_error = Some("二维码已过期，请重新生成".into());
                            Command::none()
                        }
                        802 => {
                            if !self.qr_scanned {
                                self.qr_scanned = true;
                                self.qr_status = "已扫码，请在手机上确认…".into();
                            }
                            self.qr_poll_due = Some(Instant::now() + QR_POLL_INTERVAL);
                            Command::none()
                        }
                        _ => {
                            self.qr_poll_due = Some(Instant::now() + QR_POLL_INTERVAL);
                            Command::none()
                        }
                    },
                    Err(error) => {
                        self.qr_poll_due = None;
                        self.qr_status = error.clone();
                        self.login_error = Some(error);
                        Command::none()
                    }
                }
            }
            Internal::QrLoginStatus(result) => match result {
                Ok(user) => self.login_succeeded(user),
                Err(error) => {
                    self.login_loading = false;
                    self.login_error = Some(error.clone());
                    self.banner = Banner {
                        text: error,
                        error: true,
                    };
                    Command::none()
                }
            },
            Internal::LoginFinished(result) => match result {
                Ok(user) => self.login_succeeded(user),
                Err(error) => {
                    self.login_loading = false;
                    self.login_error = Some(error.clone());
                    self.banner = Banner {
                        text: error,
                        error: true,
                    };
                    Command::none()
                }
            },
            Internal::SmsSent(result) => {
                self.login_loading = false;
                match result {
                    Ok(()) => {
                        self.login_notice = Some("验证码已发送".into());
                        self.login_error = None;
                    }
                    Err(error) => self.login_error = Some(format!("发送验证码失败：{error}")),
                }
                Command::none()
            }
            Internal::ActionResult(result) => {
                self.login_loading = false;
                match result {
                    Ok(text) => {
                        self.login_notice = Some(text.clone());
                        self.banner = Banner { text, error: false };
                    }
                    Err(error) => {
                        self.login_error = Some(error.clone());
                        self.banner = Banner {
                            text: error,
                            error: true,
                        };
                    }
                }
                Command::none()
            }
            Internal::LoggedOut(result) => {
                self.user = None;
                self.qr_generation = self.qr_generation.wrapping_add(1);
                self.qr_url.clear();
                self.qr_key = None;
                self.qr_status.clear();
                self.login_error = None;
                self.login_notice = None;
                if let Ok(mut ids) = self.liked_ids.lock() {
                    ids.clear();
                }
                self.engine.update_liked_status();
                self.page = Page::Login;
                self.history.clear();
                self.artist = None;
                self.content = Arc::new(ContentState::Empty);
                self.pagination = None;
                self.source = None;
                self.title = "登录".into();
                self.subtitle = None;
                match result {
                    Ok(()) => {
                        self.banner = Banner {
                            text: "已退出登录".into(),
                            error: false,
                        };
                    }
                    Err(error) => {
                        self.banner = Banner {
                            text: format!("已退出本地登录；{error}"),
                            error: false,
                        };
                    }
                }
                self.start_qr()
            }
            Internal::LikeDone(result) => {
                if let Err(error) = result {
                    self.banner = Banner {
                        text: format!("收藏操作失败：{error}"),
                        error: true,
                    };
                }
                Command::none()
            }
            Internal::CoverLoaded { song_id, bytes } => {
                if self.engine.current_song().map(|song| song.id) == Some(song_id) {
                    self.cover = bytes.map(Arc::new);
                }
                Command::none()
            }
            Internal::MvLoaded { mv_id, result } => {
                if self.engine.current_song().map(|song| song.mv) == Some(mv_id) {
                    match result {
                        Ok((info, bytes)) => {
                            self.mv = Some(*info);
                            self.mv_poster = bytes.map(Arc::new);
                        }
                        Err(error) => {
                            log::debug!("MV {mv_id}: {error}");
                            self.mv = None;
                            self.mv_poster = None;
                        }
                    }
                }
                Command::none()
            }
            Internal::LyricsLoaded {
                song_id,
                lyrics,
                translated,
            } => {
                self.engine.on_lyrics_loaded(song_id, lyrics, translated);
                Command::none()
            }
            Internal::QueueAppended {
                key,
                songs,
                playlist_id,
            } => {
                if !songs.is_empty() {
                    self.engine.append_songs_to_key(&key, songs);
                }
                self.queued_playlists.insert(playlist_id);
                Command::none()
            }
            Internal::PersonalFmLoaded { generation, result } => {
                if generation != self.content_generation {
                    return Command::none();
                }
                self.loading = false;
                match result {
                    Ok(songs) if songs.is_empty() => {
                        self.content = Arc::new(ContentState::Empty);
                        self.banner = Banner {
                            text: "私人FM 暂无歌曲".into(),
                            error: true,
                        };
                    }
                    Ok(songs) => {
                        self.content = Arc::new(ContentState::Songs(songs.clone()));
                        self.engine.play_songs("私人FM", songs, 0);
                        self.banner = Banner {
                            text: "私人FM 已开始播放".into(),
                            error: false,
                        };
                    }
                    Err(error) => {
                        self.content = Arc::new(ContentState::Error(error.clone()));
                        self.banner = Banner {
                            text: format!("私人FM：{error}"),
                            error: true,
                        };
                    }
                }
                Command::none()
            }
            Internal::SwitchListLoaded { key, result } => {
                match result {
                    Ok(songs) => {
                        let name = self.config.navigation.name_for_api(&key).unwrap_or(key);
                        self.engine.load_songs(&name, songs, 0);
                        self.banner = Banner {
                            text: format!("已切换到：{name}"),
                            error: false,
                        };
                    }
                    Err(error) => {
                        self.banner = Banner {
                            text: format!("切换失败：{error}"),
                            error: true,
                        };
                    }
                }
                Command::none()
            }
            Internal::Tick => self.poll_tick(),
            Internal::Noop => Command::none(),
        }
    }

    fn upload_row(&mut self, row: usize) -> Command<Message> {
        let song = match self.content.as_ref() {
            ContentState::Songs(songs) => songs.get(row).cloned(),
            _ => None,
        };
        let Some(song) = song else {
            return Command::none();
        };
        let local = song.local_path.as_deref().unwrap_or(&song.album);
        let cached = if !local.is_empty() && Path::new(local).is_file() {
            Some(PathBuf::from(local))
        } else {
            const EXTS: &[&str] = &["mp3", "flac", "m4a", "ogg"];
            EXTS.iter().find_map(|ext| {
                let path = self.cache.cache_path(song.id, ext);
                path.is_file().then_some(path)
            })
        };
        let Some(path) = cached else {
            self.banner = Banner {
                text: "未找到可上传的文件".into(),
                error: true,
            };
            return Command::none();
        };
        let service = self.service.clone();
        let cache = self.cache.clone();
        let name = song.name.clone();
        let album = song.album.clone();
        let singer = song.singer.clone();
        let song_id = song.id;
        Command::perform(
            async move {
                let result = service
                    .upload_song_with_meta(&path, &name, &album, &singer)
                    .await
                    .map_err(|error| error.to_string());
                if result.is_ok() {
                    cache.mark_uploaded(song_id);
                }
                result.map(|result| format!("上传成功：{}", result.song_name))
            },
            |result| Message::Internal(Internal::ActionResult(result)),
        )
    }
}

struct Loaded {
    source: ContentSource,
    content: ContentState,
    pagination: Option<PaginationInfo>,
    playlist_id: Option<u64>,
    title: Option<String>,
}

#[allow(clippy::too_many_arguments)]
async fn load_source(
    service: ApiService,
    cache: Arc<CacheManager>,
    uid: Option<u64>,
    limit: u16,
    ttl: u64,
    force: bool,
    source: ContentSource,
    fallback_title: String,
) -> Loaded {
    let mut playlist_id = None;
    let mut title = None;
    let (content, pagination) = match &source {
        ContentSource::Endpoint(api_string) => {
            let Some(api) = ApiEndpoint::parse(api_string) else {
                return Loaded {
                    source: source.clone(),
                    content: ContentState::Error(format!("未知: {api_string}")),
                    pagination: None,
                    playlist_id: None,
                    title: None,
                };
            };
            if api == ApiEndpoint::LocalMusic {
                let songs = tokio::task::spawn_blocking(|| {
                    let dir = dirs::home_dir().unwrap_or_default().join("Music");
                    crate::playback::scan_local_music(&dir)
                })
                .await
                .unwrap_or_default();
                let content = ContentState::Songs(songs.into_iter().map(Arc::new).collect());
                if ttl > 0 {
                    let cache = cache.clone();
                    let content = content.clone();
                    let cache_key = api_string.clone();
                    let _ = tokio::task::spawn_blocking(move || {
                        cache.save_content_cache(&cache_key, &content, None);
                    })
                    .await;
                }
                (content, None)
            } else if api == ApiEndpoint::Download {
                let songs = cache.list_cached_songs_async().await;
                (
                    ContentState::Songs(songs.into_iter().map(Arc::new).collect()),
                    None,
                )
            } else if api == ApiEndpoint::LikedSongs {
                match uid {
                    Some(uid) => {
                        let (content, pagination, id) = service.load_liked_songs(uid, limit).await;
                        playlist_id = id;
                        (content, pagination)
                    }
                    None => (ContentState::Error("未登录".into()), None),
                }
            } else {
                if ttl > 0
                    && !force
                    && !matches!(api, ApiEndpoint::Search)
                    && let Some((content, pagination)) =
                        cache.load_content_cache_async(api_string, ttl).await
                {
                    (content, pagination)
                } else {
                    let (content, pagination) = service.resolve_content(api, uid, limit).await;
                    if ttl > 0 && !matches!(content, ContentState::Error(_)) {
                        let cache = cache.clone();
                        let content = content.clone();
                        let pagination = pagination.clone();
                        let api_string = api_string.clone();
                        let _ = tokio::task::spawn_blocking(move || {
                            cache.save_content_cache(&api_string, &content, pagination.as_ref());
                        })
                        .await;
                    }
                    (content, pagination)
                }
            }
        }
        ContentSource::Playlist(id) => {
            let (content, name, pagination) = service.load_playlist_detail(*id, false, limit).await;
            title = name;
            (content, pagination)
        }
        ContentSource::Album(id) => (service.load_album(*id).await, None),
        ContentSource::Radio(id) => {
            let (content, name, pagination) = service.load_playlist_detail(*id, true, limit).await;
            title = name;
            (content, pagination)
        }
        ContentSource::Toplist(id) => match service.client().top_songs(*id).await {
            Ok(detail) => (
                ContentState::Songs(detail.songs.into_iter().map(Arc::new).collect::<Vec<_>>()),
                None,
            ),
            Err(error) => (ContentState::Error(error.to_string()), None),
        },
        ContentSource::ArtistSongs(id) => (service.load_artist_songs(*id, limit).await, None),
        ContentSource::SimilarSongs(song_id) => (service.simi_song(*song_id).await, None),
        ContentSource::SimilarPlaylists(song_id) => (service.simi_playlists(*song_id).await, None),
        ContentSource::PersonalFm => match service.personal_fm().await {
            Ok(songs) if songs.is_empty() => (ContentState::Empty, None),
            Ok(songs) => (ContentState::Songs(songs), None),
            Err(error) => (ContentState::Error(error), None),
        },
        ContentSource::Search(kind, query) => {
            let content = match kind {
                SearchKind::Songs => service.search_songs(query, limit).await,
                SearchKind::Playlists => {
                    match service.client().search_songlist(query, 0, limit).await {
                        Ok(lists) => ContentState::SongLists(lists),
                        Err(error) => ContentState::Error(error.to_string()),
                    }
                }
                SearchKind::Artists => {
                    match service.client().search_singer(query, 0, limit).await {
                        Ok(singers) => ContentState::Singers(singers),
                        Err(error) => ContentState::Error(error.to_string()),
                    }
                }
                SearchKind::Albums => match service.client().search_album(query, 0, limit).await {
                    Ok(lists) => ContentState::SongLists(lists),
                    Err(error) => ContentState::Error(error.to_string()),
                },
            };
            (content, None)
        }
    };
    if title.is_none() {
        title = Some(fallback_title);
    }
    Loaded {
        source,
        content,
        pagination,
        playlist_id,
        title,
    }
}

fn endpoint_subtitle(api: &str) -> Option<String> {
    match api {
        "search" => Some("热搜榜".into()),
        "record_week" => Some("近一周".into()),
        "record_all" => Some("全部时间".into()),
        _ => None,
    }
}

/// Strip the tiny markup the navigation titles use (`<accent>…</accent>`).
fn plain(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_tag = false;
    for ch in text.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    out.trim().to_string()
}

/// Convert a playback position advance into play time. A large jump is a seek,
/// not listening.
fn played_advance(previous: Duration, position: Duration) -> Duration {
    let advanced = position.saturating_sub(previous);
    if advanced <= SEEK_JUMP {
        advanced
    } else {
        Duration::ZERO
    }
}

fn listen_due(played: Duration, song_id: u64, reported: Option<u64>) -> bool {
    played >= LISTEN_THRESHOLD && reported != Some(song_id)
}

fn local_audio_path(song: &SongInfo) -> Option<PathBuf> {
    let candidate = song.local_path.as_deref().unwrap_or(&song.album);
    let path = Path::new(candidate);
    path.is_file().then(|| path.to_path_buf())
}

fn load_local_lyrics(audio: &Path) -> Option<ncm_api::Lyrics> {
    let sidecar = sidecar_lrc_path(audio)?;
    let text = std::fs::read_to_string(sidecar).ok()?;
    let lines: Vec<String> = text.lines().map(str::to_string).collect();
    Some(ncm_api::Lyrics {
        lyric: lines,
        tlyric: Vec::new(),
    })
}

fn sidecar_lrc_path(audio: &Path) -> Option<PathBuf> {
    let exact = audio.with_extension("lrc");
    if exact.is_file() {
        return Some(exact);
    }
    let directory = audio.parent()?;
    let stem = audio.file_stem()?.to_string_lossy().to_lowercase();
    std::fs::read_dir(directory)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .find(|path| {
            path.extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("lrc"))
                && path
                    .file_stem()
                    .is_some_and(|name| name.to_string_lossy().to_lowercase() == stem)
        })
}

fn http_client(proxy: &str, total: Option<Duration>) -> Client {
    let mut builder = Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .read_timeout(Duration::from_secs(30));
    if let Some(total) = total {
        builder = builder.timeout(total);
    }
    if !proxy.is_empty()
        && let Ok(proxy) = reqwest::Proxy::all(proxy)
    {
        builder = builder.proxy(proxy);
    }
    builder.build().unwrap_or_else(|error| {
        log::warn!("backend HTTP client: {error}; falling back to defaults");
        Client::new()
    })
}

fn rgb(color: ratatui::style::Color) -> Option<(u8, u8, u8)> {
    use ratatui::style::Color;
    match color {
        Color::Rgb(r, g, b) => Some((r, g, b)),
        Color::Black => Some((0, 0, 0)),
        Color::White => Some((255, 255, 255)),
        Color::Red => Some((205, 49, 49)),
        Color::Green => Some((13, 188, 121)),
        Color::Yellow => Some((229, 229, 16)),
        Color::Blue => Some((36, 114, 200)),
        Color::Magenta => Some((188, 63, 188)),
        Color::Cyan => Some((17, 168, 205)),
        Color::Gray => Some((229, 229, 229)),
        Color::DarkGray => Some((102, 102, 102)),
        Color::LightRed => Some((241, 76, 76)),
        Color::LightGreen => Some((35, 209, 139)),
        Color::LightYellow => Some((245, 245, 67)),
        Color::LightBlue => Some((59, 142, 234)),
        Color::LightMagenta => Some((214, 112, 214)),
        Color::LightCyan => Some((41, 184, 219)),
        Color::Reset | Color::Indexed(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_listen_is_reported_once_past_the_threshold() {
        let below = LISTEN_THRESHOLD - Duration::from_secs(1);
        assert!(!listen_due(below, 42, None));
        assert!(listen_due(LISTEN_THRESHOLD, 42, None));
        assert!(!listen_due(LISTEN_THRESHOLD, 42, Some(42)));
        assert!(listen_due(LISTEN_THRESHOLD, 7, Some(42)));
    }

    #[test]
    fn seeking_is_not_listening() {
        let tick = Duration::from_millis(80);
        assert_eq!(
            played_advance(Duration::from_secs(5), Duration::from_secs(5) + tick),
            tick
        );
        assert_eq!(
            played_advance(Duration::from_secs(5), Duration::from_secs(90)),
            Duration::ZERO
        );
        assert_eq!(
            played_advance(Duration::from_secs(90), Duration::from_secs(5)),
            Duration::ZERO
        );
    }

    #[test]
    fn nav_titles_lose_their_markup() {
        assert_eq!(
            plain("  <accent>▎</accent> <accent>DISCOVER</accent>"),
            "▎ DISCOVER"
        );
        assert_eq!(plain(" 每日推荐 "), "每日推荐");
    }
}
