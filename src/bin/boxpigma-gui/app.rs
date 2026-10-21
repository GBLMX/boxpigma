//! The iced renderer for the shared backend actor.
//!
//! Ferriso owns the window, the Tokio runtime and the latest snapshot. This
//! type owns only widget-local state — image handles, the QR code, the seek
//! preview — and translates native input into semantic [`CoreMessage`]s.

use std::sync::Arc;

use boxpigma::backend::{self, CoreMessage, View};
use ferriso_gui::{Message as GuiMessage, Renderer};
use iced::{
    Element, Subscription, Task, Theme, keyboard,
    widget::{image, qr_code},
};

/// A message the window handles without waking the backend actor.
#[derive(Clone, Debug)]
pub enum Local {
    Key(keyboard::Event),
    SeekPreview(f64),
    SeekCommit,
}

pub type UiMessage = GuiMessage<View, CoreMessage, Local>;

pub struct Gui {
    seek_preview: Option<f64>,
    last_song: Option<u64>,
    last_banner: String,
    cover: Option<(u64, Arc<Vec<u8>>, image::Handle)>,
    mv: Option<(u64, image::Handle)>,
    qr: Option<(String, qr_code::Data)>,
}

impl Gui {
    pub fn new() -> Self {
        Self {
            seek_preview: None,
            last_song: None,
            last_banner: String::new(),
            cover: None,
            mv: None,
            qr: None,
        }
    }

    pub fn cover(&self) -> Option<&image::Handle> {
        self.cover.as_ref().map(|(_, _, handle)| handle)
    }

    pub fn mv(&self) -> Option<&image::Handle> {
        self.mv.as_ref().map(|(_, handle)| handle)
    }

    pub fn qr(&self) -> Option<&qr_code::Data> {
        self.qr.as_ref().map(|(_, data)| data)
    }

    pub fn seek_preview(&self) -> Option<f64> {
        self.seek_preview
    }

    /// Desktop notifications are the GUI's replacement for the TUI's OSC
    /// notifications. Both toggles live in `[notify]`; a failure to start
    /// `notify-send` is silent, exactly like a terminal without OSC support.
    fn notify_changes(&mut self, view: &View) {
        let song = view.player.song.as_deref();
        let song_id = song.map(|song| song.id);
        if view.settings.notify_song_change
            && self.last_song.is_some()
            && song_id != self.last_song
            && let Some(song) = song
        {
            notification(&format!("{} · {}", song.name, song.singer), "正在播放");
        }
        let banner = &view.banner;
        if banner.error && banner.text != self.last_banner && view.settings.notify_errors {
            notification("boxpigma", &banner.text);
        }
        self.last_banner = banner.text.clone();
    }

    fn sync_caches(&mut self, view: &View) {
        let song_id = view.player.song.as_ref().map(|song| song.id);
        if song_id != self.last_song {
            self.last_song = song_id;
            self.seek_preview = None;
        }

        match (&view.player.cover, song_id) {
            (Some(bytes), Some(id)) => {
                let stale = self.cover.as_ref().is_none_or(|(cached_id, cached, _)| {
                    *cached_id != id || !Arc::ptr_eq(cached, bytes)
                });
                if stale {
                    self.cover = Some((
                        id,
                        Arc::clone(bytes),
                        image::Handle::from_bytes(bytes.as_ref().clone()),
                    ));
                }
            }
            _ => self.cover = None,
        }

        match (&view.player.mv, &view.player.mv_poster) {
            (Some(info), Some(bytes)) => {
                if self.mv.as_ref().map(|(id, _)| *id) != Some(info.id) {
                    self.mv = Some((info.id, image::Handle::from_bytes(bytes.as_ref().clone())));
                }
            }
            _ => self.mv = None,
        }

        let url = &view.login.qr_url;
        if url.is_empty() {
            self.qr = None;
        } else if self.qr.as_ref().map(|(cached, _)| cached) != Some(url) {
            match qr_code::Data::new(url.as_bytes()) {
                Ok(data) => self.qr = Some((url.clone(), data)),
                Err(error) => {
                    log::warn!("QR code: {error}");
                    self.qr = None;
                }
            }
        }
    }
}

impl Default for Gui {
    fn default() -> Self {
        Self::new()
    }
}

impl Renderer<View, CoreMessage, Local> for Gui {
    fn view<'a>(&'a self, view: &'a View) -> Element<'a, UiMessage> {
        self.render(view)
    }

    fn subscription(&self, _view: &View) -> Subscription<UiMessage> {
        keyboard::listen().map(|event| UiMessage::Local(Local::Key(event)))
    }

    fn update(&mut self, local: Local, view: &View) -> Task<UiMessage> {
        match local {
            Local::Key(event) => key_message(&event)
                .map(|message| Task::done(UiMessage::User(message)))
                .unwrap_or_else(Task::none),
            Local::SeekPreview(seconds) => {
                self.seek_preview = Some(seconds);
                Task::none()
            }
            Local::SeekCommit => {
                let Some(seconds) = self.seek_preview.take() else {
                    return Task::none();
                };
                let total = view.player.duration_secs.max(0.001);
                Task::done(UiMessage::User(CoreMessage::Seek(
                    (seconds / total).clamp(0.0, 1.0),
                )))
            }
        }
    }

    fn on_snapshot(&mut self, view: &View) {
        self.notify_changes(view);
        self.sync_caches(view);
    }

    fn theme(&self, view: &View) -> Theme {
        theme(view)
    }
}

fn key_message(event: &keyboard::Event) -> Option<CoreMessage> {
    use iced::keyboard::key::Named;
    let keyboard::Event::KeyPressed {
        key,
        modifiers,
        repeat: false,
        ..
    } = event
    else {
        return None;
    };
    if !modifiers.command() {
        return matches!(key, keyboard::Key::Named(Named::Escape)).then_some(CoreMessage::Back);
    }
    match key {
        keyboard::Key::Named(Named::Space) => Some(CoreMessage::TogglePlay),
        keyboard::Key::Named(Named::ArrowRight) => Some(CoreMessage::Next),
        keyboard::Key::Named(Named::ArrowLeft) => Some(CoreMessage::Prev),
        keyboard::Key::Character(c) => match c.as_str() {
            "l" | "L" => Some(CoreMessage::ToggleLike),
            "m" | "M" => Some(CoreMessage::CycleMode),
            "f" | "F" => Some(CoreMessage::Navigate(backend::Page::Queue)),
            "y" | "Y" => Some(CoreMessage::ToggleTranslation),
            "v" | "V" => Some(CoreMessage::ToggleVisualizer),
            "p" | "P" => Some(CoreMessage::TogglePitch),
            _ => None,
        },
        _ => None,
    }
}

fn notification(summary: &str, body: &str) {
    let _ = std::process::Command::new("notify-send")
        .arg("boxpigma")
        .arg(summary)
        .arg(body)
        .spawn();
}

pub fn theme(view: &View) -> Theme {
    let view = &view.theme;
    let background = color(view.background, (19, 20, 23));
    let text = color(view.text, (230, 232, 237));
    let primary = color(view.accent, (236, 65, 65));
    let success = color(view.accent, (82, 199, 133));
    let warning = color(view.warning, (242, 179, 77));
    let danger = color(view.error, (242, 89, 89));
    Theme::custom(
        "boxpigma".to_string(),
        iced::theme::Palette {
            background,
            text,
            primary,
            success,
            warning,
            danger,
        },
    )
}

fn color(value: Option<(u8, u8, u8)>, fallback: (u8, u8, u8)) -> iced::Color {
    let (r, g, b) = value.unwrap_or(fallback);
    iced::Color::from_rgb8(r, g, b)
}
