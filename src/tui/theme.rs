//! The terminal palette derived from the backend's resolved theme snapshot.
//!
//! The backend resolves named theme fields (including terminal-only colours) into
//! RGB; `None` means "this field only had an ANSI name", and the terminal keeps a
//! readable default instead.

use ferriso_tui::ratatui::style::{Color, Modifier, Style};

use crate::backend::View;

#[derive(Clone, Copy, Debug)]
pub struct Palette {
    pub background: Color,
    pub surface: Color,
    pub text: Color,
    pub muted: Color,
    pub accent: Color,
    pub border: Color,
    pub error: Color,
    pub warning: Color,
}

impl Palette {
    pub fn from_view(view: &View) -> Self {
        let theme = &view.theme;
        let background = rgb(theme.background, (18, 19, 23));
        let surface = rgb(theme.surface, blend(background, (58, 62, 74), 0.28));
        Self {
            background,
            surface,
            text: rgb(theme.text, (229, 232, 239)),
            muted: rgb(theme.muted, (139, 145, 158)),
            accent: rgb(theme.accent, (238, 92, 92)),
            border: rgb(theme.border, (74, 79, 93)),
            error: rgb(theme.error, (242, 89, 89)),
            warning: rgb(theme.warning, (232, 179, 77)),
        }
    }

    pub fn panel(&self) -> Style {
        Style::default().bg(self.background).fg(self.text)
    }

    pub fn title(&self) -> Style {
        Style::default()
            .fg(self.accent)
            .add_modifier(Modifier::BOLD)
    }

    pub fn header(&self) -> Style {
        Style::default().fg(self.muted).add_modifier(Modifier::BOLD)
    }

    pub fn selected(&self) -> Style {
        Style::default()
            .bg(self.surface)
            .fg(self.text)
            .add_modifier(Modifier::BOLD)
    }

    pub fn subtle_selected(&self) -> Style {
        Style::default().bg(self.surface).fg(self.accent)
    }

    pub fn hint(&self) -> Style {
        Style::default().fg(self.muted)
    }
}

pub fn rgb(value: Option<(u8, u8, u8)>, fallback: (u8, u8, u8)) -> Color {
    let (r, g, b) = value.unwrap_or(fallback);
    Color::Rgb(r, g, b)
}

/// Blend `amount` of `other` into `base`, used to invent a surface when the
/// theme did not name one.
pub fn blend(base: Color, other: (u8, u8, u8), amount: f32) -> (u8, u8, u8) {
    let (br, bg, bb) = match base {
        Color::Rgb(r, g, b) => (r, g, b),
        _ => (18, 19, 23),
    };
    let mix = |a: u8, b: u8| -> u8 {
        (f32::from(a) * (1.0 - amount) + f32::from(b) * amount)
            .round()
            .clamp(0.0, 255.0) as u8
    };
    (mix(br, other.0), mix(bg, other.1), mix(bb, other.2))
}
