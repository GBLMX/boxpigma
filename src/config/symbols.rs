//! Glyph presets: the same interface drawn with Nerd Font, plain Unicode or ASCII
//! characters, plus per-key overrides.
//!
//! Every private-use glyph in the UI — the powerline capsule separators, the three volume
//! icons, the queue-cleared icon — used to be hardcoded, which is why the README has to
//! insist on a Nerd Font: without a patched font they render as tofu boxes. A preset now
//! switches all of them at once, individual keys can be overridden, and the spinner frames
//! come from the same place.

use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

/// Which family of glyphs the UI draws with.
///
/// `nerd` is the default because it is what the UI drew before presets existed; use
/// `unicode` or `ascii` on a terminal without a patched font.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SymbolPreset {
    #[default]
    Nerd,
    Unicode,
    Ascii,
}

/// Per-key overrides, all optional so a config only names what it changes.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SymbolsConfig {
    pub preset: SymbolPreset,
    /// Left end of the selected navigation capsule (Nerd: the powerline separator).
    pub nav_capsule_left: Option<String>,
    /// Right end of the selected navigation capsule.
    pub nav_capsule_right: Option<String>,
    pub volume_low: Option<String>,
    pub volume_mid: Option<String>,
    pub volume_high: Option<String>,
    /// Shown when the playback queue is emptied.
    pub queue_clear: Option<String>,
    /// Written before a translated lyric line, so it reads as the translation of the line above
    /// it and not as another lyric.
    ///
    /// ASCII by default: the arrows and box-drawing glyphs that would read better are
    /// ambiguous-width, and a terminal in a CJK locale draws those two cells wide, which the
    /// lyrics layout does not budget for.
    pub translation: Option<String>,
    /// Spectrum bar characters, lowest first (at least two).
    pub visualizer_bars: Option<String>,
    /// Icon written before the spectrum bars.
    pub visualizer_icon: Option<String>,
    /// Spinner frames, one per animation step; an empty list is ignored.
    pub spinner_activity: Option<Vec<String>>,
    /// Main-loop ticks per spinner frame (the UI has always advanced every 3 ticks).
    pub spinner_ticks_per_frame: Option<u64>,
    /// The arrows a titled surface writes around its title (`► HELP ◄`).
    pub title_open: Option<String>,
    pub title_close: Option<String>,
    /// The chevron of a menu entry that opens another menu.
    pub submenu: Option<String>,
    /// The cursor of a selected line.
    pub selected: Option<String>,
    /// The marker of a notice, by level.
    pub notice_info: Option<String>,
    pub notice_warn: Option<String>,
    pub notice_error: Option<String>,
    /// The marker of a task, by state.
    pub task_running: Option<String>,
    pub task_done: Option<String>,
    pub task_failed: Option<String>,
}

/// The resolved glyph set the UI reads through [`symbols`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Symbols {
    pub nav_capsule_left: String,
    pub nav_capsule_right: String,
    pub volume_low: String,
    pub volume_mid: String,
    pub volume_high: String,
    pub queue_clear: String,
    /// Written before a translated lyric line.
    pub translation: String,
    /// Spectrum bar characters from shortest to tallest.
    pub visualizer_bars: String,
    /// Icon written before the spectrum bars.
    pub visualizer_icon: String,
    pub spinner_activity: Vec<String>,
    pub spinner_ticks_per_frame: u64,
    pub title_open: String,
    pub title_close: String,
    pub submenu: String,
    pub selected: String,
    pub notice_info: String,
    pub notice_warn: String,
    pub notice_error: String,
    pub task_running: String,
    pub task_done: String,
    pub task_failed: String,
}

/// Spinner frames before presets existed (`ui::spinner`), kept as the default so the
/// animation looks the same until someone picks another preset or overrides the frames.
const BLOCKS_ACTIVITY: [&str; 8] = [
    "▰▱▱▱▱▱▱",
    "▰▰▱▱▱▱▱",
    "▰▰▰▱▱▱▱",
    "▰▰▰▰▱▱▱",
    "▰▰▰▰▰▱▱",
    "▰▰▰▰▰▰▱",
    "▰▰▰▰▰▰▰",
    "▰▱▱▱▱▱▱",
];

/// Same shape and width as [`BLOCKS_ACTIVITY`], drawn with ASCII only.
const ASCII_ACTIVITY: [&str; 8] = [
    "=      ", "==     ", "===    ", "====   ", "=====  ", "====== ", "=======", "=      ",
];

const DEFAULT_TICKS_PER_FRAME: u64 = 3;

impl SymbolPreset {
    fn glyph(self, key: &str) -> &'static str {
        match (self, key) {
            // Nerd Font: what the UI drew before presets existed.
            (SymbolPreset::Nerd, "nav_capsule_left") => "\u{e0b2}",
            (SymbolPreset::Nerd, "nav_capsule_right") => "\u{e0b0}",
            (SymbolPreset::Nerd, "volume_low") => "\u{f026}",
            (SymbolPreset::Nerd, "volume_mid") => "\u{f027}",
            (SymbolPreset::Nerd, "volume_high") => "\u{f028}",
            (SymbolPreset::Nerd, "queue_clear") => "\u{f48e}",
            (SymbolPreset::Nerd, "visualizer_icon") => "\u{f001}",
            // Half blocks read like the capsule ends without a patched font.
            (SymbolPreset::Unicode, "nav_capsule_left") => "▐",
            (SymbolPreset::Unicode, "nav_capsule_right") => "▌",
            (SymbolPreset::Unicode, "volume_low") => "♩",
            (SymbolPreset::Unicode, "volume_mid") => "♫",
            (SymbolPreset::Unicode, "volume_high") => "♬",
            (SymbolPreset::Unicode, "queue_clear") => "∅",
            (SymbolPreset::Unicode, "visualizer_icon") => "♫",
            (SymbolPreset::Ascii, "nav_capsule_left") => "[",
            (SymbolPreset::Ascii, "nav_capsule_right") => "]",
            (SymbolPreset::Ascii, "volume_low") => "-",
            (SymbolPreset::Ascii, "volume_mid") => "=",
            (SymbolPreset::Ascii, "volume_high") => "#",
            (SymbolPreset::Ascii, "queue_clear") => "x",
            (SymbolPreset::Ascii, "visualizer_icon") => "~",
            // The arrows a title is written between, and the markers of a notice or a task: these
            // are the ones the UI drew as literals, which is what made them impossible to switch
            // off on a terminal that cannot draw them.
            (SymbolPreset::Nerd | SymbolPreset::Unicode, "title_open") => "►",
            (SymbolPreset::Nerd | SymbolPreset::Unicode, "title_close") => "◄",
            (SymbolPreset::Nerd | SymbolPreset::Unicode, "submenu") => "▸",
            (SymbolPreset::Nerd | SymbolPreset::Unicode, "selected") => "▶",
            (SymbolPreset::Ascii, "title_open") => ">",
            (SymbolPreset::Ascii, "title_close") => "<",
            (SymbolPreset::Ascii, "submenu") => ">",
            (SymbolPreset::Ascii, "selected") => ">",
            (SymbolPreset::Nerd | SymbolPreset::Unicode, "notice_info") => "·",
            (SymbolPreset::Nerd | SymbolPreset::Unicode, "notice_warn") => "!",
            (SymbolPreset::Nerd | SymbolPreset::Unicode, "notice_error") => "✗",
            (SymbolPreset::Ascii, "notice_info") => ".",
            (SymbolPreset::Ascii, "notice_warn") => "!",
            (SymbolPreset::Ascii, "notice_error") => "x",
            (SymbolPreset::Nerd | SymbolPreset::Unicode, "task_running") => "…",
            (SymbolPreset::Nerd | SymbolPreset::Unicode, "task_done") => "✓",
            (SymbolPreset::Nerd | SymbolPreset::Unicode, "task_failed") => "✗",
            (SymbolPreset::Ascii, "task_running") => "~",
            (SymbolPreset::Ascii, "task_done") => "+",
            (SymbolPreset::Ascii, "task_failed") => "x",
            (SymbolPreset::Nerd | SymbolPreset::Unicode, "visualizer_bars") => "▁▂▃▄▅▆▇█",
            (SymbolPreset::Ascii, "visualizer_bars") => " .:-=+*#",
            // Every preset of the marker is the same glyph: it is punctuation rather than an
            // icon, and the safe ones are the ASCII ones (see `SymbolsConfig::translation`).
            (_, "translation") => ">",
            _ => "?",
        }
    }

    fn spinner_frames(self) -> Vec<String> {
        match self {
            SymbolPreset::Ascii => ASCII_ACTIVITY.iter().map(|f| (*f).to_string()).collect(),
            SymbolPreset::Nerd | SymbolPreset::Unicode => {
                BLOCKS_ACTIVITY.iter().map(|f| (*f).to_string()).collect()
            }
        }
    }
}

impl Symbols {
    /// Resolve the configured preset plus overrides into the glyph set the UI reads.
    ///
    /// Invalid overrides cannot break rendering: an empty frame list and a zero
    /// ticks-per-frame both fall back to the preset value with a warning.
    pub fn resolve(config: &SymbolsConfig) -> Self {
        let preset = config.preset;
        let frames = match &config.spinner_activity {
            Some(frames) if frames.is_empty() => {
                log::warn!("symbols.spinner_activity is empty, using the preset frames");
                preset.spinner_frames()
            }
            Some(frames) => frames.clone(),
            None => preset.spinner_frames(),
        };
        let ticks = match config.spinner_ticks_per_frame {
            Some(0) => {
                log::warn!("symbols.spinner_ticks_per_frame must be >= 1, using the default");
                DEFAULT_TICKS_PER_FRAME
            }
            Some(ticks) => ticks,
            None => DEFAULT_TICKS_PER_FRAME,
        };

        let pick = |over: &Option<String>, key: &str| {
            over.clone()
                .unwrap_or_else(|| preset.glyph(key).to_string())
        };

        Self {
            nav_capsule_left: pick(&config.nav_capsule_left, "nav_capsule_left"),
            nav_capsule_right: pick(&config.nav_capsule_right, "nav_capsule_right"),
            volume_low: pick(&config.volume_low, "volume_low"),
            volume_mid: pick(&config.volume_mid, "volume_mid"),
            volume_high: pick(&config.volume_high, "volume_high"),
            queue_clear: pick(&config.queue_clear, "queue_clear"),
            translation: pick(&config.translation, "translation"),
            visualizer_icon: pick(&config.visualizer_icon, "visualizer_icon"),
            title_open: pick(&config.title_open, "title_open"),
            title_close: pick(&config.title_close, "title_close"),
            submenu: pick(&config.submenu, "submenu"),
            selected: pick(&config.selected, "selected"),
            notice_info: pick(&config.notice_info, "notice_info"),
            notice_warn: pick(&config.notice_warn, "notice_warn"),
            notice_error: pick(&config.notice_error, "notice_error"),
            task_running: pick(&config.task_running, "task_running"),
            task_done: pick(&config.task_done, "task_done"),
            task_failed: pick(&config.task_failed, "task_failed"),
            visualizer_bars: {
                let bars = pick(&config.visualizer_bars, "visualizer_bars");
                if bars.chars().count() < 2 {
                    log::warn!(
                        "symbols.visualizer_bars needs at least two characters, using the preset"
                    );
                    preset.glyph("visualizer_bars").to_string()
                } else {
                    bars
                }
            },
            spinner_activity: frames,
            spinner_ticks_per_frame: ticks,
        }
    }

    /// Frame for a main-loop tick, cycling through the animation.
    pub fn activity_frame(&self, tick: u64) -> &str {
        let len = self.spinner_activity.len();
        if len == 0 {
            // `resolve` guarantees a non-empty list; keep this total anyway so a
            // hand-built `Symbols` can never panic mid-render.
            return "";
        }
        let index = (tick / self.spinner_ticks_per_frame) as usize % len;
        &self.spinner_activity[index]
    }

    /// Glyph for a volume level, matching the playerbar's three-step icon.
    pub fn volume_icon(&self, volume: f64) -> &str {
        if volume <= 0.30 {
            &self.volume_low
        } else if volume <= 0.60 {
            &self.volume_mid
        } else {
            &self.volume_high
        }
    }
}

static SYMBOLS: OnceLock<Symbols> = OnceLock::new();

/// Resolve the glyph set once, from the loaded config.
///
/// Called during app construction; anything that renders before that (or a path that
/// never loads a config, such as the CLI subcommands) keeps the default set.
pub fn init_symbols(config: &SymbolsConfig) {
    let _ = SYMBOLS.set(Symbols::resolve(config));
}

/// The process-wide glyph set, defaulting to the resolved default config.
pub fn symbols() -> &'static Symbols {
    SYMBOLS.get_or_init(|| Symbols::resolve(&SymbolsConfig::default()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The default preset must keep drawing exactly what the UI drew before presets
    /// existed, otherwise existing users lose the look they chose the font for.
    #[test]
    fn default_preset_keeps_the_nerd_glyphs() {
        let symbols = Symbols::resolve(&SymbolsConfig::default());
        assert_eq!(symbols.nav_capsule_left, "\u{e0b2}");
        assert_eq!(symbols.nav_capsule_right, "\u{e0b0}");
        assert_eq!(symbols.volume_high, "\u{f028}");
        assert_eq!(symbols.queue_clear, "\u{f48e}");
        assert_eq!(symbols.spinner_activity[0], BLOCKS_ACTIVITY[0]);
        assert_eq!(symbols.spinner_ticks_per_frame, 3);
    }

    /// The point of the ascii preset: nothing outside ASCII, so a terminal without a
    /// patched font (and without a font that has the Unicode fallbacks) can still draw.
    #[test]
    fn ascii_preset_is_ascii_only() {
        let config = SymbolsConfig {
            preset: SymbolPreset::Ascii,
            ..SymbolsConfig::default()
        };
        let symbols = Symbols::resolve(&config);

        for glyph in [
            &symbols.nav_capsule_left,
            &symbols.nav_capsule_right,
            &symbols.volume_low,
            &symbols.volume_mid,
            &symbols.volume_high,
            &symbols.queue_clear,
        ] {
            assert!(
                glyph.is_ascii(),
                "ascii preset produced non-ascii {glyph:?}"
            );
        }
        for frame in &symbols.spinner_activity {
            assert!(
                frame.is_ascii(),
                "ascii spinner frame {frame:?} is not ascii"
            );
        }
    }

    /// Unicode fallbacks must stay one column wide: the playerbar computes padding from
    /// the glyph widths, and a double-width glyph would push the layout out of place.
    #[test]
    fn unicode_preset_glyphs_are_single_width() {
        use unicode_width::UnicodeWidthStr;

        let config = SymbolsConfig {
            preset: SymbolPreset::Unicode,
            ..SymbolsConfig::default()
        };
        let symbols = Symbols::resolve(&config);

        for glyph in [
            &symbols.nav_capsule_left,
            &symbols.nav_capsule_right,
            &symbols.volume_low,
            &symbols.volume_mid,
            &symbols.volume_high,
            &symbols.queue_clear,
        ] {
            assert_eq!(
                UnicodeWidthStr::width(glyph.as_str()),
                1,
                "{glyph:?} must occupy a single column"
            );
        }
    }

    #[test]
    fn overrides_win_over_the_preset() {
        let config = SymbolsConfig {
            preset: SymbolPreset::Unicode,
            volume_high: Some("VOL".to_string()),
            spinner_ticks_per_frame: Some(1),
            ..SymbolsConfig::default()
        };
        let symbols = Symbols::resolve(&config);
        assert_eq!(symbols.volume_high, "VOL");
        assert_eq!(symbols.spinner_ticks_per_frame, 1);
        // untouched keys keep the preset glyph
        assert_eq!(symbols.volume_low, "♩");
    }

    /// A degenerate override must not panic the render loop later.
    #[test]
    fn empty_frames_and_zero_ticks_fall_back() {
        let config = SymbolsConfig {
            spinner_activity: Some(Vec::new()),
            spinner_ticks_per_frame: Some(0),
            ..SymbolsConfig::default()
        };
        let symbols = Symbols::resolve(&config);
        assert_eq!(symbols.spinner_activity.len(), BLOCKS_ACTIVITY.len());
        assert_eq!(symbols.spinner_ticks_per_frame, DEFAULT_TICKS_PER_FRAME);
    }

    #[test]
    fn activity_frame_cycles_within_bounds() {
        let symbols = Symbols::resolve(&SymbolsConfig::default());
        for tick in 0..64 {
            let frame = symbols.activity_frame(tick);
            assert!(
                symbols.spinner_activity.iter().any(|f| f == frame),
                "tick {tick} produced an unknown frame {frame:?}"
            );
        }
        assert_eq!(symbols.activity_frame(0), symbols.activity_frame(24));
        assert_ne!(symbols.activity_frame(0), symbols.activity_frame(3));
    }

    #[test]
    fn visualizer_bars_keep_at_least_two_levels() {
        let ascii = Symbols::resolve(&SymbolsConfig {
            preset: SymbolPreset::Ascii,
            ..SymbolsConfig::default()
        });
        assert!(
            ascii.visualizer_bars.is_ascii(),
            "ascii preset must draw ascii bars"
        );

        let single = Symbols::resolve(&SymbolsConfig {
            visualizer_bars: Some("x".to_string()),
            ..SymbolsConfig::default()
        });
        assert_ne!(
            single.visualizer_bars, "x",
            "a one-character bar set cannot express a level and must fall back"
        );
    }

    #[test]
    fn volume_icon_follows_the_three_steps() {
        let symbols = Symbols::resolve(&SymbolsConfig::default());
        assert_eq!(symbols.volume_icon(0.0), symbols.volume_low);
        assert_eq!(symbols.volume_icon(0.5), symbols.volume_mid);
        assert_eq!(symbols.volume_icon(1.0), symbols.volume_high);
    }
}

#[cfg(test)]
mod glyph_tests {
    use super::*;

    /// Every glyph the UI reads is non-empty, and the ones the layout counts on are one cell
    /// wide: an empty or wide glyph moves everything after it, which is the class of bug this
    /// table exists to make impossible.
    #[test]
    fn every_glyph_is_drawable() {
        for preset in [
            SymbolPreset::Nerd,
            SymbolPreset::Unicode,
            SymbolPreset::Ascii,
        ] {
            let symbols = Symbols::resolve(&SymbolsConfig {
                preset,
                ..SymbolsConfig::default()
            });

            for (key, glyph) in [
                ("title_open", &symbols.title_open),
                ("title_close", &symbols.title_close),
                ("submenu", &symbols.submenu),
                ("selected", &symbols.selected),
                ("notice_info", &symbols.notice_info),
                ("notice_warn", &symbols.notice_warn),
                ("notice_error", &symbols.notice_error),
                ("task_running", &symbols.task_running),
                ("task_done", &symbols.task_done),
                ("task_failed", &symbols.task_failed),
                ("volume_low", &symbols.volume_low),
                ("volume_mid", &symbols.volume_mid),
                ("volume_high", &symbols.volume_high),
                ("queue_clear", &symbols.queue_clear),
                ("translation", &symbols.translation),
                ("visualizer_icon", &symbols.visualizer_icon),
            ] {
                assert!(!glyph.is_empty(), "{preset:?}: {key} is empty");
                assert_eq!(
                    unicode_width::UnicodeWidthStr::width(glyph.as_str()),
                    1,
                    "{preset:?}: {key} is not one cell wide"
                );
            }
        }
    }

    /// A preset that is not the default still draws a complete set: every key has a value in it,
    /// which is what makes `preset = "ascii"` a thing a terminal without a patched font can use.
    #[test]
    fn every_preset_answers_every_key() {
        for preset in [
            SymbolPreset::Nerd,
            SymbolPreset::Unicode,
            SymbolPreset::Ascii,
        ] {
            for key in [
                "title_open",
                "title_close",
                "submenu",
                "selected",
                "notice_info",
                "notice_warn",
                "notice_error",
                "task_running",
                "task_done",
                "task_failed",
                "visualizer_icon",
            ] {
                assert_ne!(preset.glyph(key), "?", "{preset:?} has no {key}");
            }
        }
    }
}
