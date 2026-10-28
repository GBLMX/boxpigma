//! The terminal frontend for the shared boxpigma backend.
//!
//! The actor owns the API, playback engine, queue, navigation and login state.
//! This module owns only terminal-local state: focus, list cursors, the search
//! edit, image protocols, overlays, and the frame's mouse hit map. Every input
//! becomes a [`CoreMessage`]; no terminal code touches the backend directly.

mod draw;
mod input;
mod theme;

use std::time::{Duration, Instant};

use ferriso_tui::{Outcome, Renderer, crossterm::event::Event, ratatui::layout::Rect};
use qrcode::{QrCode, render::unicode::Dense1x2};
use ratatui_image::{StatefulImage, picker::Picker, protocol::StatefulProtocol};

use crate::{
    backend::{self, Backend, ContentSource, CoreMessage, Page, View},
    config::{
        Config, Pane, Symbols,
        keymap::{Chord, Keymap},
    },
    layout::Divider,
    state::{LoginField, LoginMethod},
};

use theme::Palette;

/// Run the interactive TUI on the Ferriso runtime and actor.
///
/// `Config` is moved into the backend; the renderer only keeps the two values it
/// needs for terminal behaviour, so a settings change made by the actor cannot
/// race the local copy.
pub fn run(config: Config) -> std::io::Result<()> {
    let options = ferriso_tui::Options {
        mouse: config.mouse,
        poll_interval: Duration::from_millis(40),
    };
    let tui = Tui::new(&config);
    ferriso_tui::run_with_app(
        move || {
            Backend::new_with_bridge(config, true)
                .unwrap_or_else(|error| panic!("boxpigma: failed to build the backend: {error}"))
        },
        tui,
        options,
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Focus {
    Nav,
    Content,
    Search,
}

impl Focus {
    fn next(self, forward: bool) -> Self {
        let order = [Self::Nav, Self::Content, Self::Search];
        let at = order.iter().position(|focus| *focus == self).unwrap_or(1);
        let delta = if forward { 1 } else { order.len() - 1 };
        order[(at + delta) % order.len()]
    }
}

#[derive(Clone, Debug)]
pub(crate) struct CommandLine {
    pub text: String,
    pub cursor: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Hit {
    Nav { section: usize, item: usize },
    Page(Page),
    Content(usize),
    Artist { tab: usize, index: usize },
    Queue(usize),
    Settings(usize),
    LoginMethod(usize),
    Gauge,
    Prev,
    Play,
    Next,
    Volume,
}

#[derive(Default)]
pub(crate) struct Areas {
    /// Everything the pointer can hit, in draw order. Input walks this backwards,
    /// so later (topmost) targets win.
    pub hits: Vec<(Rect, Hit)>,
    /// The rectangle rows were drawn into, and the first row's content index.
    pub content: Rect,
    pub content_offset: usize,
}

struct Last {
    page: Page,
    source: Option<ContentSource>,
    title: String,
    artist: Option<u64>,
    queue_key: String,
    login_method: Option<LoginMethod>,
    qr_url: String,
    banner: String,
}

impl Default for Last {
    fn default() -> Self {
        Self {
            page: Page::Browse,
            source: None,
            title: String::new(),
            artist: None,
            queue_key: String::new(),
            login_method: None,
            qr_url: String::new(),
            banner: String::new(),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct PaneDrag {
    pub(crate) divider: Divider,
    pub(crate) start_size: u16,
    pub(crate) column: u16,
    pub(crate) row: u16,
}

pub struct Tui {
    pub(crate) focus: Focus,
    pub(crate) content_selected: usize,
    pub(crate) content_offset: usize,
    pub(crate) content_rows: usize,
    pub(crate) artist_tab: usize,
    pub(crate) artist_selected: [usize; 3],
    pub(crate) artist_offset: [usize; 3],
    pub(crate) artist_rows: usize,
    pub(crate) queue_selected: usize,
    pub(crate) queue_offset: usize,
    pub(crate) queue_rows: usize,
    pub(crate) settings_selected: usize,
    pub(crate) login_field: usize,
    pub(crate) nav_offset: usize,
    pub(crate) nav_rows: usize,
    pub(crate) search: String,
    pub(crate) search_cursor: usize,
    pub(crate) search_editing: bool,
    pub(crate) command: Option<CommandLine>,
    pub(crate) help: bool,
    pub(crate) notice: Option<(String, Instant)>,
    pub(crate) banner: String,
    banner_error: bool,
    banner_at: Instant,
    pub(crate) tick: u64,
    last_tick: Instant,
    last: Last,
    pub(crate) areas: Areas,
    last_click: Option<(Instant, Hit)>,
    pub(crate) pane_dividers: crate::layout::Dividers,
    pub(crate) pane_preview: Option<(Pane, u16)>,
    pub(crate) pane_drag: Option<PaneDrag>,
    pub(crate) last_pane_click: Option<(Divider, Instant)>,
    pub(crate) shell_area: Rect,
    pub(crate) picker: Option<Picker>,
    picker_attempted: bool,
    pub(crate) cover: Option<(u64, StatefulProtocol)>,
    pub(crate) qr: Option<Vec<String>>,
    seek_step: f64,
    mouse: bool,
    keymap: Keymap,
    pending_keys: Chord,
    symbols: Symbols,
}

impl Tui {
    pub fn new(config: &Config) -> Self {
        Self {
            focus: Focus::Content,
            content_selected: 0,
            content_offset: 0,
            content_rows: 10,
            artist_tab: 0,
            artist_selected: [0; 3],
            artist_offset: [0; 3],
            artist_rows: 6,
            queue_selected: 0,
            queue_offset: 0,
            queue_rows: 10,
            settings_selected: 0,
            login_field: 0,
            nav_offset: 0,
            nav_rows: 10,
            search: String::new(),
            search_cursor: 0,
            search_editing: false,
            command: None,
            help: false,
            notice: None,
            banner: String::new(),
            banner_error: false,
            banner_at: Instant::now(),
            tick: 0,
            last_tick: Instant::now(),
            last: Last::default(),
            areas: Areas::default(),
            pane_dividers: crate::layout::Dividers::default(),
            pane_preview: None,
            pane_drag: None,
            last_pane_click: None,
            shell_area: Rect::default(),
            last_click: None,
            picker: None,
            picker_attempted: false,
            cover: None,
            qr: None,
            seek_step: f64::from(config.seek_interval_secs.max(1)),
            mouse: config.mouse,
            keymap: Keymap::from_config(config),
            pending_keys: Vec::new(),
            symbols: Symbols::resolve(&config.symbols),
        }
    }

    pub(crate) fn mouse_enabled(&self) -> bool {
        self.mouse
    }

    pub(crate) fn seek_step(&self) -> f64 {
        self.seek_step
    }

    pub(crate) fn notice(&mut self, text: impl Into<String>) {
        self.notice = Some((text.into(), Instant::now()));
    }

    pub(crate) fn visible_banner(&self) -> Option<(&str, bool)> {
        if self.banner.is_empty() || self.banner_at.elapsed() >= Duration::from_secs(5) {
            return None;
        }
        Some((self.banner.as_str(), self.banner_error))
    }

    pub(crate) fn visible_notice(&self) -> Option<&str> {
        self.notice
            .as_ref()
            .filter(|(_, at)| at.elapsed() < Duration::from_secs(4))
            .map(|(text, _)| text.as_str())
    }

    /// Fold the newest snapshot into terminal-local cursors.
    fn sync(&mut self, view: &View) {
        let now = Instant::now();
        let steps = (now.duration_since(self.last_tick).as_millis() / 80).max(1) as u64;
        self.last_tick = now;
        self.tick = self.tick.wrapping_add(steps);

        let page_changed = view.page != self.last.page;
        let source_changed = view.source != self.last.source;
        let title_changed = view.title != self.last.title;
        if page_changed || source_changed || title_changed {
            self.content_selected = 0;
            self.content_offset = 0;
        }

        let artist = view.artist.as_ref().map(|artist| artist.detail.id);
        if artist != self.last.artist {
            self.artist_selected = [0; 3];
            self.artist_offset = [0; 3];
            self.artist_tab = 0;
        }
        let artist_len = artist_count(view, self.artist_tab);
        self.artist_selected[self.artist_tab] =
            self.artist_selected[self.artist_tab].min(artist_len.saturating_sub(1));

        let len = content_rows(view);
        self.content_selected = self.content_selected.min(len.saturating_sub(1));

        if view.player.queue_key != self.last.queue_key {
            self.queue_selected = view.player.queue_index.unwrap_or(0);
            self.queue_offset = 0;
        }
        self.queue_selected = self
            .queue_selected
            .min(view.player.queue.len().saturating_sub(1));

        let method = view.login.method;
        if self.last.login_method != Some(method) {
            self.login_field = 0;
        }
        let fields = method.fields().len();
        if fields > 0 {
            self.login_field = self.login_field.min(fields - 1);
        } else {
            self.login_field = 0;
        }
        self.settings_selected = self.settings_selected.min(SETTINGS.len() - 1);

        if view.banner.text != self.last.banner {
            self.banner = view.banner.text.clone();
            self.banner_error = view.banner.error;
            self.banner_at = now;
        }

        if view.login.qr_url != self.last.qr_url {
            self.qr = make_qr(&view.login.qr_url);
        }

        self.last.page = view.page;
        self.last.source = view.source.clone();
        self.last.title = view.title.clone();
        self.last.artist = artist;
        self.last.queue_key = view.player.queue_key.clone();
        self.last.login_method = Some(method);
        self.last.qr_url = view.login.qr_url.clone();
        self.last.banner = view.banner.text.clone();
    }

    fn prepare_cover(&mut self, view: &View) {
        let Some(bytes) = view.player.cover.as_deref() else {
            self.cover = None;
            return;
        };
        let Some(song) = view.player.song.as_ref() else {
            self.cover = None;
            return;
        };
        if self.cover.as_ref().map(|(id, _)| *id) == Some(song.id) {
            return;
        }
        if self.picker.is_none() && !self.picker_attempted {
            self.picker_attempted = true;
            self.picker = query_picker();
        }
        let Some(picker) = self.picker.as_ref() else {
            self.cover = None;
            return;
        };
        match image::load_from_memory(bytes) {
            Ok(image) => {
                self.cover = Some((song.id, picker.new_resize_protocol(image)));
            }
            Err(error) => {
                log::debug!("cover decode: {error}");
                self.cover = None;
            }
        }
    }

    fn draw_cover(&mut self, area: Rect, frame: &mut ferriso_tui::ratatui::Frame) {
        let Some((_, protocol)) = self.cover.as_mut() else {
            return;
        };
        let image = StatefulImage::new().resize(ratatui_image::Resize::Crop(None));
        frame.render_stateful_widget(image, area, protocol);
    }
}

impl Renderer<View, CoreMessage> for Tui {
    fn draw(&mut self, view: &View, frame: &mut ferriso_tui::ratatui::Frame) {
        self.sync(view);
        self.prepare_cover(view);
        draw::draw(self, view, frame);
    }

    fn handle(&mut self, view: &View, event: Event) -> Outcome<CoreMessage> {
        self.sync(view);
        input::handle(self, view, event)
    }
}

pub(crate) const SETTINGS: &[&str] = &[
    "主题",
    "音质",
    "边听边存",
    "切歌通知",
    "错误通知",
    "歌词翻译",
    "频谱",
    "音高",
    "每日签到",
    "退出登录",
];

pub(crate) fn content_rows(view: &View) -> usize {
    let more = view.pagination.as_ref().is_some_and(|p| p.has_more);
    view.content.len() + usize::from(more)
}

pub(crate) fn artist_count(view: &View, tab: usize) -> usize {
    let Some(artist) = view.artist.as_ref() else {
        return 0;
    };
    match tab {
        0 => artist.detail.hot_songs.len(),
        1 => artist.albums.len(),
        _ => artist.similar.len(),
    }
}

pub(crate) fn ensure_visible(selected: usize, offset: &mut usize, rows: usize) {
    if rows == 0 {
        return;
    }
    if selected < *offset {
        *offset = selected;
    } else if selected >= *offset + rows {
        *offset = selected + 1 - rows;
    }
}

pub(crate) fn clamp_selection(selected: usize, len: usize) -> usize {
    selected.min(len.saturating_sub(1))
}

/// Ask the terminal what it can draw, with a deadline.
///
/// `Picker::from_query_stdio` restarts its own read timeout after every byte and
/// can park forever on a stdin that never answers (a terminal multiplexer, or a
/// pipe). The query thread is left parked when the deadline expires; the UI
/// continues with the text cover and never asks again.
fn query_picker() -> Option<Picker> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("boxpigma-picker".into())
        .spawn(move || {
            let picker = Picker::from_query_stdio().unwrap_or_else(|_| Picker::halfblocks());
            let _ = tx.send(Some(picker));
        })
        .ok()?;
    match rx.recv_timeout(Duration::from_secs(2)) {
        Ok(picker) => picker,
        Err(_) => {
            log::warn!("covers: the terminal did not answer the graphics query; using half blocks");
            Some(Picker::halfblocks())
        }
    }
}

pub(crate) fn make_qr(url: &str) -> Option<Vec<String>> {
    if url.is_empty() {
        return None;
    }
    let code = QrCode::new(url.as_bytes()).ok()?;
    Some(
        code.render::<Dense1x2>()
            .module_dimensions(1, 1)
            .build()
            .lines()
            .map(str::to_string)
            .collect(),
    )
}

pub(crate) fn field_value(view: &View, field: LoginField) -> &str {
    match field {
        LoginField::Account => &view.login.account,
        LoginField::Password => &view.login.password,
        LoginField::Phone => &view.login.phone,
        LoginField::Code => &view.login.code,
    }
}

pub(crate) fn toggle_play() -> CoreMessage {
    backend::Message::TogglePlay
}

pub(crate) fn seek_to(view: &View, fraction: f64) -> CoreMessage {
    let _ = view;
    backend::Message::Seek(fraction.clamp(0.0, 1.0))
}

pub(crate) fn seek_by(view: &View, seconds: f64) -> CoreMessage {
    let duration = view.player.duration_secs.max(0.001);
    let position = (view.player.position_secs + seconds).clamp(0.0, duration);
    backend::Message::Seek(position / duration)
}

pub(crate) fn volume_by(view: &View, delta: f64) -> CoreMessage {
    backend::Message::SetVolume((view.player.volume + delta).clamp(0.0, 1.0))
}

pub(crate) fn palette(view: &View) -> Palette {
    Palette::from_view(view)
}

/// The selected content row as an activation message, including the synthetic
/// "load more" row.
pub(crate) fn activate_content(view: &View, index: usize) -> Option<CoreMessage> {
    match view.page {
        Page::Queue => {
            (index < view.player.queue.len()).then_some(backend::Message::PlayQueueRow(index))
        }
        Page::Artist => activate_artist(view, 0, index),
        _ => {
            if index < view.content.len() {
                Some(backend::Message::ActivateRow(index))
            } else if view.pagination.as_ref().is_some_and(|p| p.has_more) {
                Some(backend::Message::LoadMore)
            } else {
                None
            }
        }
    }
}

pub(crate) fn activate_artist(view: &View, tab: usize, index: usize) -> Option<CoreMessage> {
    let artist = view.artist.as_ref()?;
    match tab {
        0 => (index < artist.detail.hot_songs.len())
            .then_some(backend::Message::PlayArtistSong(index)),
        1 => (index < artist.albums.len()).then_some(backend::Message::OpenArtistAlbum(index)),
        _ => (index < artist.similar.len()).then_some(backend::Message::OpenSimilarArtist(index)),
    }
}

pub(crate) fn page_for_key(key: char) -> Option<Page> {
    match key {
        'f' => Some(Page::Queue),
        'l' => Some(Page::Lyrics),
        ',' => Some(Page::Settings),
        'L' => Some(Page::Login),
        'g' => Some(Page::Browse),
        _ => None,
    }
}

pub(crate) fn login_fields(view: &View) -> &'static [LoginField] {
    view.login.method.fields()
}

pub(crate) fn hit_at(areas: &Areas, column: u16, row: u16) -> Option<Hit> {
    areas
        .hits
        .iter()
        .rev()
        .find(|(rect, _)| {
            column >= rect.x
                && column < rect.x.saturating_add(rect.width)
                && row >= rect.y
                && row < rect.y.saturating_add(rect.height)
        })
        .map(|(_, hit)| *hit)
}

impl Tui {
    pub(crate) fn remember_click(&mut self, hit: Hit) -> bool {
        let now = Instant::now();
        let double = self.last_click.is_some_and(|(at, previous)| {
            previous == hit && now.duration_since(at) < Duration::from_millis(400)
        });
        self.last_click = Some((now, hit));
        double
    }
}
