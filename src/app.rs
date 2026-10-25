//! Main application state (`App`) and the wiring of views, events, navigation,
//! search, login and theming for the boxpigma TUI.

mod builder;
mod content;
mod event;
mod login;
mod navigation;
mod search;
mod search_core;
mod splash;
mod theme;

pub use search_core::{SearchEngine, SearchResults};

use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    io::IsTerminal,
    sync::{Arc, LazyLock, Mutex},
    time::{Duration, Instant},
};

use ncm_api::SongList;
use ratatui::{DefaultTerminal, Frame, layout::Rect, widgets::TableState};
use ratatui_image::picker::Picker;
use reqwest::Client;
use splash::send_event;

use crate::{
    cache::CacheManager,
    config::{Config, ThemeRegistry, init_symbols, symbols},
    event::{AuthEvent, EventHandler},
    input::ex::ExCommand,
    ipc::{IpcEvent, QueueSnapshot, StatusSnapshot},
    playback::{NCM_SEARCH_QUEUE_KEY, PlaybackEngine},
    service::{ApiEndpoint, ApiService},
    state::{
        ContentState, LoginState, NavState, NavigationState, Page, PromptState, QueueHits,
        SearchState, SplashState, State, TableMode,
    },
    ui,
    utils::{
        boxpigma_cache_dir, boxpigma_config_dir,
        path::expand_tilde,
        terminal::{BACKGROUND, begin_synchronized_update, end_synchronized_update},
    },
};

/// Main application state and entry point for the boxpigma TUI.
pub struct App {
    pub config: Config,
    pub state: State,
    pub playback: PlaybackEngine,
    pub theme_registry: ThemeRegistry,
    /// The terminal's own background, probed once at startup: what `background = auto` picks the
    /// theme slot from, and what decides whether the page paints its own background over the
    /// terminal's at all (see
    /// [`BackgroundFill`](crate::utils::terminal::BackgroundFill)).
    pub terminal_background: crate::utils::terminal::Background,
    pub service: ApiService,
    pub picker: Picker,
    /// HTTP client for one-shot cover downloads (honours the proxy config); bounded by
    /// [`Self::build_cover_client`]'s total deadline, since a cover is never streamed.
    pub cover_http: Client,
    /// Registry of recently searched songs keyed by song id,
    /// shared with the IPC `search` engine so `boxpigma msg play <id>` can enqueue
    /// and play a search result that is not in the playback queue.
    pub search_results: SearchResults,
    /// Cross-provider search engine serving `boxpigma msg search <keyword>`.
    pub searcher: Arc<SearchEngine>,
    /// Song ID set of the user's "我喜欢的音乐" playlist, sharing the same `Arc` as `PlaybackEngine`.
    pub liked_ids: Arc<Mutex<HashSet<u64>>>,
    /// Song whose listen has already been reported to the cloud, so one play reports once.
    /// Cleared when a track starts, which is what lets a repeat of the same song count.
    pub(super) reported_listen: Option<u64>,
    /// Last position seen, to tell playback apart from a seek.
    pub(super) last_position: Duration,
    /// Playback actually accumulated for the current song; the threshold is measured on it.
    pub(super) played_in_track: Duration,
    /// Playlists whose full tracks have already been merged into the playback queue for lazy pagination, avoiding repeated Enter presses refetching/truncating the queue.
    queued_playlists: HashSet<u64>,
    /// Live playback snapshot served to `boxpigma status` over the IPC socket.
    pub status: Arc<Mutex<StatusSnapshot>>,
    /// Live playback queue served to `boxpigma status -L` over the IPC socket.
    pub queue: Arc<Mutex<QueueSnapshot>>,
    /// Desktop media session: the properties published to MPRIS alongside the IPC snapshot.
    pub mpris: crate::mpris::MprisHandle,
    /// The D-Bus half of that session; whichever run loop starts takes it and spawns it.
    mpris_service: Option<crate::mpris::MprisService>,
    /// Fan-out channel for snapshot changes, consumed by the IPC `subscribe`
    /// handler so long-running clients get event push.
    status_tx: tokio::sync::broadcast::Sender<StatusSnapshot>,
    /// When the last snapshot was broadcast; discrete changes fire immediately,
    /// position refresh (playing) is throttled to once per second.
    last_status_broadcast: Instant,
    /// Last queue version the `queue` snapshot was built from; rebuilds only on
    /// change instead of cloning the whole queue every event-loop iteration.
    last_queue_version: u64,
    /// Guards the startup splash branch: `login_status` is requested exactly
    /// once, so later events (IPC, mouse...) cannot spawn duplicate requests
    /// that would double-toast and re-fetch liked IDs.
    login_status_requested: bool,
}

impl App {
    /// `with_terminal` selects the interactive TUI event source (crossterm);
    /// pass `false` for headless daemon mode.
    pub fn new(mut config: Config, with_terminal: bool) -> color_eyre::Result<Self> {
        // Only the process that owns the terminal owns the config file: a test or a CLI run
        // calls the same setters and commands, and saving from there used to write over the
        // user's own settings.
        config.persist = with_terminal;

        let border = config.border.clone();

        // Resolve the appearance once, up front: the glyph set comes from the config, and
        // the background probe asks the terminal directly, which has to happen before the
        // event loop starts consuming input.
        init_symbols(&config.symbols);
        if with_terminal {
            LazyLock::force(&BACKGROUND);
        }

        let events = EventHandler::new(with_terminal);
        let tx = events.sender();

        let theme_registry = ThemeRegistry::new(config.themes.clone());
        // Read once here: `with_terminal` is false in tests and in the CLI paths, where nothing
        // probed the terminal, and the probe's own default is what the page gets.
        let terminal_background = *BACKGROUND;

        // `random` is a request, not a theme, so it is settled here — once. `resolve_theme` runs
        // on every frame; rolling there would repaint the interface in a new theme each frame.
        // The two slots roll separately, so `background = auto` with both set to `random` gets
        // two independent picks.
        use crate::config::RANDOM_THEME;
        if config.default_theme == RANDOM_THEME {
            config.default_theme = theme_registry.concrete_name(RANDOM_THEME);
            log::info!("theme: random picked `{}`", config.default_theme);
        }
        if config.light_theme.as_deref() == Some(RANDOM_THEME) {
            let picked = theme_registry.concrete_name(RANDOM_THEME);
            log::info!("theme: random picked `{picked}` for the light slot");
            config.light_theme = Some(picked);
        }

        let command_panel = Self::build_command_panel(&theme_registry);

        // One proxy for everything on the network — the NCM API, covers and the audio
        // streams. `proxy` is absent by default ("直连"); the v1 key that used to scope it
        // to a subset of the sources is gone (see `config::migrate_document`).
        let proxy = config.proxy.clone().unwrap_or_default();

        let cookie_path = boxpigma_config_dir().join("cookies.json");
        let mut api_builder = ncm_api::NcmClient::builder().cookie_path(cookie_path);
        if !proxy.is_empty() {
            api_builder = api_builder.proxy(&proxy);
        }
        let api = Arc::new(api_builder.build()?);

        let quality = ncm_api::SongQuality::from_level(&config.cache.quality)
            .unwrap_or(ncm_api::SongQuality::Higher);
        let save_on_play = config.cache.save_on_play;
        // The audio chain's settings travel with the engine: the player thread owns its copy.
        let audio = config.audio.clone();

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

        let service = ApiService::new(api.clone(), cache.clone());

        // Ask the terminal about its graphics support only when something is attached to
        // answer: the headless daemon, the CLI subcommands, tests and `boxpigma > file`
        // all reach here with stdin at end-of-file, where the query can only spin.
        let picker = Self::build_picker(
            &config.playerbar,
            with_terminal && std::io::stdin().is_terminal(),
        );

        let stream_client = Self::build_stream_client(&proxy)?;
        let cover_http = Self::build_cover_client(&proxy)?;

        let liked_ids: Arc<Mutex<HashSet<u64>>> = Arc::new(Mutex::new(HashSet::new()));
        let state = State {
            running: true,
            events,
            border,
            splash: SplashState::default(),
            login: LoginState::default(),
            navigation: NavigationState {
                page: Page::Splash,
                user: None,
                nav: NavState::from_config(&config.navigation),
                content: Arc::new(ContentState::Empty),
                history: Vec::new(),
                content_selected: 0,
                content_column_selected: 0,
                table_mode: TableMode::Row,
                table_state: TableState::default(),
                playlist_selected: 0,
                queue_tab_scroll_x: 0,
                artist: crate::state::ArtistState::default(),
                search: SearchState::default(),
                pagination: None,
                generation: 0,
                content_is_search: false,
                return_page: None,
                title_cache: RefCell::new(None),
            },
            command_panel,
            help: crate::state::popup::PopupState::default(),
            offline: false,
            tick: 0,
            last_tick: Instant::now(),
            lyrics: crate::state::lyrics::LyricsState::default(),
            settings: crate::ui::settings::SettingsState::default(),
            pending_keys: Vec::new(),
            pane_dividers: crate::layout::Dividers::default(),
            pane_drag: None,
            last_pane_click: None,
            shell_area: ratatui::layout::Rect::default(),
            notices: crate::state::notices::Notices::default(),
            messages: crate::state::popup::PopupState::default(),
            tasks: crate::state::tasks::Tasks::default(),
            tasks_popup: crate::state::popup::PopupState::default(),
            playerbar_area: Rect::default(),
            nav_area: Rect::default(),
            content_inner: Rect::default(),
            content_offset: 0,
            gauge_area: Rect::default(),
            prompt: PromptState::default(),
            queue_hits: QueueHits::default(),
            artist_hits: crate::state::ArtistHits::default(),
            volume_area: Rect::default(),
            cover_area: Rect::default(),
            spectrum_row_area: Rect::default(),
            pitch_area: Rect::default(),
            transport: Default::default(),
            mode_area: Rect::default(),
            like_areas: Default::default(),
            volume_before_mute: None,
        };
        let search_results: SearchResults = Arc::new(Mutex::new(HashMap::new()));
        let searcher = Arc::new(SearchEngine::new(
            service.clone(),
            Arc::clone(&search_results),
            config.search_limit as usize,
        ));
        let (status_tx, _status_rx) = tokio::sync::broadcast::channel(16);
        // Created here, started by a run loop: `App::new` is also called from tests, where
        // there is no tokio runtime and no session bus to connect to.
        let (mpris, mpris_service) = crate::mpris::MprisHandle::new();
        Ok(Self {
            config,
            service: service.clone(),
            playback: PlaybackEngine::new(
                tx,
                service,
                cache,
                base_dir,
                quality,
                // The engine copies the chain's settings: it hands them to the player thread
                // when the first song starts.
                audio.clone(),
                save_on_play,
                stream_client,
                Arc::clone(&liked_ids),
            ),
            state,
            theme_registry,
            terminal_background,
            picker,
            cover_http,
            search_results,
            searcher,
            liked_ids,
            reported_listen: None,
            last_position: Duration::ZERO,
            played_in_track: Duration::ZERO,
            queued_playlists: HashSet::new(),
            status: Arc::new(Mutex::new(StatusSnapshot::default())),
            queue: Arc::new(Mutex::new(QueueSnapshot::default())),
            mpris,
            mpris_service: Some(mpris_service),
            status_tx,
            last_status_broadcast: Instant::now(),
            // Force the first `update_status_snapshot` to populate the queue,
            // e.g. when a session is restored from disk during engine startup.
            last_queue_version: u64::MAX,
            login_status_requested: false,
        })
    }

    /* -------------------------------------------------------------------------- */
    /*                      shared helpers (TUI + headless)                        */
    /* -------------------------------------------------------------------------- */

    pub fn quit(&mut self) {
        self.playback.save_session();
        self.service.client().flush_cookies();
        self.state.running = false;
    }

    /// Start the desktop media session (MPRIS) on the session bus.
    ///
    /// Both run loops call this: a media session that only worked in one of them would make
    /// `playerctl` and the media keys work or not depending on how the player was started.
    /// A session bus that is missing, private, or already has a boxpigma on it is a warning
    /// from the service task, not a reason not to play music.
    fn start_mpris(&mut self) {
        let Some(service) = self.mpris_service.take() else {
            return;
        };
        tokio::spawn(service.run(self.state.events.sender()));
    }

    /// Say something. A confirmation: it lingers for its level's time and the newest wins.
    pub fn toast(&mut self, msg: String) {
        self.notice(crate::state::notices::Level::Info, msg);
    }

    /// Say something, at a level: an error stays on screen longer than a confirmation, and both are
    /// in `:messages` afterwards.
    pub fn notice(&mut self, level: crate::state::notices::Level, msg: String) {
        self.state.notices.push(level, msg);
    }

    /// Adjust playback volume by `delta` (fraction of 0..=1), clamped to bounds
    /// and surfaced as a toast. Keyboard `+`/`-` mirrors the playerbar scroll.
    pub fn adjust_volume(&mut self, delta: f64) {
        let new = (self.playback.state.volume + delta).clamp(0.0, 1.0);
        self.playback.set_volume(new);
        self.toast(format!(" {}  {:.0}%", symbols().volume_high, new * 100.0));
    }

    /// Show or hide the spectrum bars (`v`, `:visualizer on|off`).
    pub fn set_visualizer(&mut self, on: bool) {
        self.config.playerbar.visible.visualizer = on;
        self.config.save();
        self.toast(format!("频谱: {}", if on { "开" } else { "关" }));
    }

    /// Switch the player bar layout (`:layout default|modern|minimal`).
    ///
    /// Worth knowing before switching: the modern layout fills every row with something, so
    /// the spectrum has to share the cover column there and is only eight cells wide; the
    /// other two layouts give it a full-width row.
    pub fn set_playerbar_layout(&mut self, name: &str) -> Result<(), String> {
        let layout = match name.to_ascii_lowercase().as_str() {
            "default" => crate::config::LayoutType::Default,
            "modern" => crate::config::LayoutType::Modern,
            "minimal" => crate::config::LayoutType::Minimal,
            other => {
                return Err(format!("未知布局: {other}（default / modern / minimal）"));
            }
        };
        self.config.playerbar.layout = layout;
        self.config.save();
        self.toast(format!("播放条布局: {}", name.to_ascii_lowercase()));
        Ok(())
    }

    /// Show or hide the pitch readout (`V`, `:pitch on|off`).
    pub fn set_pitch(&mut self, on: bool) {
        self.config.playerbar.visible.pitch = on;
        self.config.save();
        self.toast(format!("音高: {}", if on { "开" } else { "关" }));
    }

    /// `[playerbar] spinning_cover`: the record turns while a track plays. The frame reads this
    /// config every time it draws, so the flip is on screen at once; saving is what makes it
    /// survive the next start.
    /// Draw (or hide) the translated lyric lines. The page reads the flag on every draw, so
    /// nothing else has to be told.
    pub fn set_lyric_translation(&mut self, on: bool) {
        self.config.lyric_translation = on;
        self.config.save();
        self.toast(format!("歌词译文: {}", if on { "开" } else { "关" }));
    }

    pub fn set_spinning_cover(&mut self, on: bool) {
        self.config.playerbar.spinning_cover = on;
        self.config.save();
        self.toast(format!("封面旋转: {}", if on { "开" } else { "关" }));
    }

    /// Refresh the IPC status snapshot from the live playback state. The status
    /// (current song + progress) is cheap and rebuilt each loop; the full queue
    /// listing is only rebuilt when the queue actually changed.
    ///
    /// A snapshot change is broadcast to IPC `subscribe` clients immediately;
    /// while a track is playing the position also advances, but that refresh is
    /// throttled to once per second so the daemon does not spam subscribers at
    /// the event-loop rate.
    fn update_status_snapshot(&mut self) {
        self.playback.update_analysis();
        let snapshot = StatusSnapshot::from_playback(&self.playback.state);
        // The desktop media session reports the same status the socket serves; the cover is
        // only interesting to it, so it is looked up here rather than carried in the snapshot.
        let art_url = (snapshot.id != 0)
            .then(|| self.playback.cached_cover(snapshot.id))
            .flatten();
        self.mpris
            .update(&snapshot, self.playback.state.mode, art_url);
        let mut changed = false;
        if let Ok(mut stored) = self.status.lock() {
            changed = stored.meaningfully_differs(&snapshot);
            *stored = snapshot.clone();
        }
        let elapsed = self.last_status_broadcast.elapsed();
        let position_stale =
            snapshot.playing && !snapshot.paused && elapsed >= Duration::from_secs(1);
        if changed || position_stale {
            self.last_status_broadcast = Instant::now();
            let _ = self.status_tx.send(snapshot);
        }
        let version = self.playback.queue_version();
        if version != self.last_queue_version {
            self.last_queue_version = version;
            if let Ok(mut queue) = self.queue.lock() {
                *queue = QueueSnapshot {
                    key: self.playback.queue_key().to_string(),
                    current_index: self.playback.queue_current_index(),
                    songs: self
                        .playback
                        .queue_songs()
                        .iter()
                        .map(|s| crate::ipc::QueueEntry::from_song(s))
                        .collect(),
                };
            }
        }
    }

    /// Apply a control request received over the IPC socket (`boxpigma msg`).
    async fn handle_ipc_event(&mut self, event: IpcEvent) {
        match event {
            IpcEvent::Previous => self.playback.prev(),
            IpcEvent::Next => self.playback.next(),
            IpcEvent::Pause => {
                // Pause only pauses; when stopped it stays stopped (unlike the
                // TUI spacebar which toggles/start).
                if self.playback.state.playing && !self.playback.state.paused {
                    self.playback.toggle_pause();
                }
            }
            IpcEvent::Play { song_id } => {
                if let Some(id) = song_id {
                    // Jump to a song in the active queue and play it. Songs
                    // returned by `boxpigma msg search` are not queued, so fall
                    // back to the shared search-result registry and enqueue the
                    // result into the NCM search queue.
                    if !self.playback.play_song_by_id(id)
                        && let Some(song) = self
                            .search_results
                            .lock()
                            .ok()
                            .and_then(|m| m.get(&id).cloned())
                    {
                        self.playback
                            .append_and_play_key(NCM_SEARCH_QUEUE_KEY, &[song], 0);
                    }
                    let name = self
                        .playback
                        .current_song()
                        .map(|s| s.name.clone())
                        .unwrap_or_default();
                    if name.is_empty() {
                        self.toast(format!("找不到 id={id} 的歌曲"));
                    } else {
                        self.toast(format!("♪ 正在播放: {name}"));
                    }
                } else if self.playback.state.paused || !self.playback.state.playing {
                    // Resume when paused; start when stopped (if a song is queued).
                    self.playback.toggle_pause();
                }
            }
            IpcEvent::TogglePlay => self.playback.toggle_pause(),
            IpcEvent::Volume { delta, absolute } => {
                if let Some(delta) = delta {
                    self.adjust_volume(delta);
                } else if let Some(volume) = absolute {
                    let volume = volume.clamp(0.0, 1.0);
                    self.playback.set_volume(volume);
                    self.toast(format!(
                        " {}  {:.0}%",
                        symbols().volume_high,
                        volume * 100.0
                    ));
                }
            }
            IpcEvent::Mode => {
                let mode = self.playback.cycle_mode();
                let (_, label) = crate::playback::mode_icon(&mode);
                self.toast(format!("播放模式: {label}"));
            }
            // Not reachable from `boxpigma msg` (which only cycles): the desktop media
            // session sets `LoopStatus`/`Shuffle` outright.
            IpcEvent::SetMode(mode) => {
                self.playback.set_mode(mode);
                let (_, label) = crate::playback::mode_icon(&mode);
                self.toast(format!("播放模式: {label}"));
            }
            IpcEvent::Like => {
                if let Some(song) = self.playback.current_song() {
                    self.state
                        .events
                        .send(crate::event::PlaybackEvent::LikeSong(song.id, true));
                }
            }
            IpcEvent::Dislike => {
                if let Some(song) = self.playback.current_song() {
                    self.state
                        .events
                        .send(crate::event::PlaybackEvent::DislikeSong(song.id));
                }
            }
            IpcEvent::ToggleLike => {
                if let Some(song) = self.playback.current_song() {
                    let like = !self.playback.state.liked;
                    self.state
                        .events
                        .send(crate::event::PlaybackEvent::LikeSong(song.id, like));
                }
            }
            IpcEvent::SwitchList { endpoint, playlist } => {
                let loaded = self.load_endpoint(&endpoint, playlist).await;
                self.toast(if loaded {
                    format!("已切换到: {endpoint}")
                } else {
                    format!("切换失败: {endpoint}")
                });
            }
            // Both go through the command layer the `:` line uses, so `msg seek` and `:seek`
            // cannot drift apart — one parser, one behaviour, one set of error messages.
            IpcEvent::Seek { value } => {
                if let Err(error) = crate::input::ex::execute(self, ExCommand::Seek(value)) {
                    self.toast(format!("seek: {error}"));
                }
            }
            IpcEvent::Clear => {
                if let Err(error) = crate::input::ex::execute(self, ExCommand::ClearQueue) {
                    self.toast(format!("clear: {error}"));
                }
            }
        }
    }

    /* -------------------------------------------------------------------------- */
    /*                                  TUI mode                                   */
    /* -------------------------------------------------------------------------- */

    /// Cycle the navigation bar position (left → right → top → bottom) at
    /// runtime and persist the new value so it survives restarts. Keyboard
    /// `p`/`shift+p` binding.
    pub fn cycle_nav_position(&mut self) {
        self.config.navigation_position = self.config.navigation_position.cycle();
        self.config.save();
        let pos = self.config.navigation_position;
        self.toast(format!("◧ 导航栏位置: {}", pos.label()));
    }

    /// Interactive terminal entry point (`boxpigma` without subcommands). Draws
    /// the UI, serves the IPC socket, and pumps the event loop until quit.
    pub async fn run(mut self, mut terminal: DefaultTerminal) -> color_eyre::Result<()> {
        self.start_splash_boot();
        self.start_mpris();
        let _ipc_guard = crate::ipc::start_server(
            Arc::clone(&self.status),
            Arc::clone(&self.queue),
            self.status_tx.clone(),
            self.state.events.sender(),
            Arc::clone(&self.searcher),
        );
        while self.state.running {
            self.update_status_snapshot();
            // Present one whole frame at a time (`DECSET 2026`) instead of letting the
            // terminal show it while it is still being written — most visible while the
            // spectrum redraws. Terminals without the mode ignore both sequences, and
            // the update is closed before the draw error is propagated: kitty drops a
            // frame whose end never arrives, so it must never be left open.
            let mut out = std::io::stdout();
            begin_synchronized_update(&mut out)?;
            let drawn = terminal.draw(|frame| self.draw(frame));
            end_synchronized_update(&mut out)?;
            drawn?;
            self.handle_events().await?;

            let splash_ready = self.state.splash.shown_at.elapsed().as_secs_f64()
                >= self.config.splash_duration_secs;
            if self.state.splash.boot_complete
                && splash_ready
                && self.state.navigation.page == Page::Splash
            {
                if self.state.offline {
                    self.navigate_to_local();
                } else if self.service.client().is_logged_in() && !self.login_status_requested {
                    // Wait for the authenticated user info before entering the
                    // default tab. Login-gated endpoints need the UID returned by
                    // this request, so loading the tab first can produce a false
                    // "未登录" error during startup.
                    self.login_status_requested = true;
                    let service = self.service.clone();
                    let sender = self.state.events.sender();
                    tokio::spawn(async move {
                        match service.login_status().await {
                            Ok(info) => {
                                if sender.send(AuthEvent::Success(info).into()).is_err() {
                                    log::error!("Failed to send LoginSuccess: receiver dropped");
                                }
                            }
                            Err(e) => {
                                log::error!("Failed to get login status: {e}");
                                if sender.send(AuthEvent::Error(e.to_string()).into()).is_err() {
                                    log::error!("Failed to send LoginError: receiver dropped");
                                }
                            }
                        }
                    });
                } else {
                    self.navigate_to_main();
                }
            }
        }
        Ok(())
    }

    fn draw(&mut self, frame: &mut Frame) {
        // The spin rides the tick that is already here. The angle comes from wall-clock
        // time rather than from a frame count, and the disc is re-encoded only once it
        // reaches a new angle step (every 278 ms of a 20 s turn), so a frame that is
        // not turning the disc adds nothing to the one the app was drawing anyway.
        self.playback.state.cover.advance(
            &self.picker,
            self.config.playerbar.spinning_cover,
            self.playback.state.playing && !self.playback.state.paused,
        );
        ui::draw(frame, self);
    }

    /* -------------------------------------------------------------------------- */
    /*                              CLI / headless daemon                          */
    /* -------------------------------------------------------------------------- */

    /// Headless daemon mode (`boxpigma --daemon <endpoint>`): no terminal is opened.
    /// Loads the endpoint as the initial list, starts playing it, and runs the
    /// IPC socket so `boxpigma status` / `boxpigma msg` can observe and control it.
    /// Stops on SIGINT/SIGTERM (saving the session).
    pub async fn run_headless(
        mut self,
        endpoint: &str,
        playlist_index: Option<usize>,
    ) -> color_eyre::Result<()> {
        self.state.navigation.page = Page::Main;
        self.start_mpris();
        let _ipc_guard = crate::ipc::start_server(
            Arc::clone(&self.status),
            Arc::clone(&self.queue),
            self.status_tx.clone(),
            self.state.events.sender(),
            Arc::clone(&self.searcher),
        );

        // Resolve the user session from cookies so login-gated endpoints like
        // `liked` can obtain the uid even without an interactive QR login.
        if self.state.navigation.user.is_none() {
            match self.service.login_status().await {
                Ok(info) => {
                    let uid = info.uid;
                    self.state.navigation.user = Some(info);
                    // Preload the liked-song id set so player-bar like status and
                    // the waybar heart icon reflect reality in headless mode too.
                    match self.service.load_liked_song_ids(uid).await {
                        Ok(ids) => {
                            if let Ok(mut guard) = self.liked_ids.lock() {
                                *guard = ids;
                            }
                        }
                        Err(e) => log::warn!("headless: failed to load liked song ids: {e}"),
                    }
                }
                Err(e) => log::warn!("headless: failed to resolve user session: {e}"),
            }
        }

        // Forward termination signals to the event loop so shutdown goes through
        // `App::quit` (session save + cookie flush) like a normal quit.
        let tx = self.state.events.sender();
        tokio::spawn(async move {
            wait_shutdown_signal().await;
            let _ = tx.send(crate::event::AppEvent::Quit.into());
        });

        self.bootstrap_headless(endpoint, playlist_index).await;
        while self.state.running {
            self.update_status_snapshot();
            self.handle_events().await?;
        }
        Ok(())
    }

    /// Resolve the `--daemon` endpoint and load its songs into the queue **without
    /// starting playback**; the user starts it via `boxpigma msg play` / toggle.
    async fn bootstrap_headless(&mut self, api_str: &str, playlist_index: Option<usize>) {
        let loaded = self.load_endpoint(api_str, playlist_index).await;
        if loaded {
            let name = self
                .playback
                .current_song()
                .map(|s| s.name.clone())
                .unwrap_or_default();
            log::info!("headless: loaded {} (paused, press play)", name);
        }
    }

    /// Resolve an endpoint string into playable songs and load them into the
    /// queue without starting playback. Shared by the daemon bootstrap
    /// (`--daemon`) and the IPC `boxpigma msg switch-list` action. Returns whether
    /// songs were loaded.
    async fn load_endpoint(&mut self, api_str: &str, playlist_index: Option<usize>) -> bool {
        let api = ApiEndpoint::parse(api_str).unwrap_or(ApiEndpoint::RecommendSongs);
        let uid = self.state.navigation.user.as_ref().map(|u| u.uid);
        let content = self
            .service
            .resolve_endpoint_content(api, uid, self.config.search_limit)
            .await;

        // Playlist/toplist endpoints resolve to a *list* of playlists; pick one
        // (default first, or `--playlist N`) and load its songs.
        let content = match content {
            ContentState::SongLists(lists) => {
                self.load_headless_list_songs(api, lists, playlist_index)
                    .await
            }
            ContentState::TopLists(lists) => {
                let lists: Vec<SongList> = lists
                    .into_iter()
                    .map(|t| SongList {
                        id: t.id,
                        name: t.name,
                        cover_img_url: t.cover,
                        author: String::new(),
                        subscribed: false,
                    })
                    .collect();
                self.load_headless_list_songs(api, lists, playlist_index)
                    .await
            }
            other => other,
        };

        match content {
            ContentState::Songs(songs) if !songs.is_empty() => {
                // Use the nav display name (e.g. " 我喜欢的音乐") as the queue key
                // so the daemon shows the same title the TUI would, falling back
                // to the raw endpoint string (e.g. `liked`) when unknown.
                let key = self
                    .config
                    .navigation
                    .name_for_api(api_str)
                    .unwrap_or_else(|| api_str.to_string());
                self.playback.load_songs(&key, songs, 0);
                true
            }
            ContentState::Error(e) => {
                log::error!("headless: {api_str}: {e}");
                false
            }
            _ => {
                log::warn!("headless: {api_str} resolved to no playable songs");
                false
            }
        }
    }

    /// Resolve a playlist-list (or toplist) to the songs of the selected
    /// playlist: `--playlist N` picks the 1-based `N`-th entry (default 1st).
    async fn load_headless_list_songs(
        &mut self,
        api: ApiEndpoint,
        lists: Vec<SongList>,
        playlist_index: Option<usize>,
    ) -> ContentState {
        if lists.is_empty() {
            return ContentState::Empty;
        }
        let index = playlist_index.unwrap_or(1).max(1) - 1;
        let Some(list) = lists.get(index) else {
            return ContentState::Error(format!(
                "playlist index {} out of range (1..={})",
                index + 1,
                lists.len()
            ));
        };
        self.playback.set_playlist_id(list.id);
        let limit = self.config.search_limit;
        match api {
            ApiEndpoint::SavedAlbums => self.service.load_album(list.id).await,
            ApiEndpoint::UserRadioSublist => {
                let (state, _, _) = self
                    .service
                    .load_playlist_detail(list.id, true, limit)
                    .await;
                state
            }
            _ => {
                let (state, _, _) = self
                    .service
                    .load_playlist_detail(list.id, false, limit)
                    .await;
                state
            }
        }
    }
}

async fn wait_shutdown_signal() {
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .expect("failed to install SIGTERM handler");
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = sigterm.recv() => {}
    }
}

/// The spin's wiring: the frame the app already draws is what turns the cover.
///
/// Everything else about the spin is tested where it lives (`playback::cover`): these
/// are the two links the draw pass owns — the config flag reaching the state, and the
/// player's state at the moment of the frame deciding whether the disc turns.
#[cfg(test)]
mod cover_spin {
    use std::time::Duration;

    use ratatui::{Terminal, backend::TestBackend};

    use super::*;
    use crate::{
        config::LayoutType, playback::CoverState, state::Page, utils::terminal::ImageProtocolChoice,
    };

    /// The width the modern layout gives the cover column.
    const COVER_WIDTH: u16 = 8;

    /// What the player bar draws in the cover area: a kitty protocol carries the image
    /// in the first cell it draws, so a re-encoded disc changes these cells.
    fn drawn_cover(app: &mut App) -> String {
        let mut terminal = Terminal::new(TestBackend::new(120, 30)).expect("backend");
        terminal.draw(|frame| app.draw(frame)).expect("draw frame");
        let area = app.state.cover_area;
        let buffer = terminal.backend().buffer();
        (0..area.height)
            .flat_map(|y| (0..area.width).map(move |x| (x, y)))
            .map(|(x, y)| buffer[(area.x + x, area.y + y)].symbol().to_string())
            .collect()
    }

    /// Two frames, so the returned string is the settled one: the first render of a
    /// freshly encoded protocol is the frame that transmits the image, the ones after
    /// it only place it.
    fn settled(app: &mut App) -> String {
        drawn_cover(app);
        drawn_cover(app)
    }

    /// A playing app whose cover has finished loading, with a graphics protocol so the
    /// spin has an image to turn rather than the halfblocks placeholder.
    fn app_with_a_cover(spinning: bool) -> App {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let mut config = Config::default();
        config.playerbar.spinning_cover = spinning;
        config.playerbar.layout = LayoutType::Modern;
        config.playerbar.image_protocol = ImageProtocolChoice::Kitty;
        let mut app = App::new(config, false).expect("app");
        app.state.navigation.page = Page::Main;
        app.playback.state.current_song = Some(Arc::new(ncm_api::SongInfo {
            id: 7,
            name: "test".into(),
            singer: String::new(),
            artist_id: 0,
            album: String::new(),
            album_id: 0,
            pic_url: String::new(),
            duration: 60_000,
            copyright: ncm_api::SongCopyright::Free,
            local_path: None,
            mv: 0,
        }));
        app.playback.state.playing = true;

        let square = image::RgbaImage::from_fn(64, 64, |x, y| {
            image::Rgba([(x * 4) as u8, (y * 4) as u8, ((x + y) * 2) as u8, 255])
        });
        let protocol = CoverState::encode_disc(&square, 0.0, &app.picker);
        *app.playback.state.cover.song_id.lock().expect("song id") = Some(7);
        app.playback.state.cover.install(7, square, protocol);
        app
    }

    /// One step of the turn is 278 ms, so this is comfortably past the first one — and
    /// a frame that overruns only turns the disc further.
    const PAST_A_STEP: Duration = Duration::from_millis(350);

    #[tokio::test]
    async fn a_playing_frame_turns_the_cover_and_a_paused_one_parks_it() {
        let mut app = app_with_a_cover(true);
        let still = settled(&mut app);
        assert_eq!(
            app.state.cover_area.width, COVER_WIDTH,
            "the modern layout gives the cover a column"
        );

        std::thread::sleep(PAST_A_STEP);
        assert_ne!(settled(&mut app), still, "playing has to turn the disc");

        app.playback.state.paused = true;
        let parked = settled(&mut app);
        std::thread::sleep(PAST_A_STEP);
        assert_eq!(
            settled(&mut app),
            parked,
            "a pause has to leave it where it is"
        );
    }

    #[tokio::test]
    async fn without_the_flag_the_cover_is_the_still_image_it_was() {
        let mut app = app_with_a_cover(false);

        let still = settled(&mut app);
        std::thread::sleep(PAST_A_STEP);
        assert_eq!(settled(&mut app), still, "off is off");
    }
}
