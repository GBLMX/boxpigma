//! TOML configuration: the runtime `Config` plus the border/cache/column/
//! navigation/playerbar/theme registries.

mod audio;
mod border;
mod cache;
mod column;
pub mod keymap;
pub mod lyrics;
mod navigation;
mod notify;
mod panes;
mod playerbar;
mod symbols;
pub mod theme;
mod titles;
mod update;

use std::{collections::HashMap, fs, path::Path};

pub use audio::*;
pub use border::*;
pub use cache::*;
pub use column::*;
pub use lyrics::*;
pub use navigation::*;
pub use notify::*;
pub use panes::*;
pub use playerbar::*;
use serde::{Deserialize, Serialize};
pub use symbols::*;
pub use theme::{ColorSpec, RANDOM_THEME, Theme, ThemeRegistry, UserTheme, theme_fallback};
pub use titles::*;
pub use update::*;

use crate::{
    logger::Logger,
    utils::{
        self, GradientPreset,
        terminal::{BackgroundFill, BackgroundMode},
    },
};

/// `#[serde(default)]` for a field whose natural default is "on".
fn default_true() -> bool {
    true
}

/// Schema version of `config.toml` written by this build.
///
/// Bump it whenever a field is renamed, removed, or changes meaning, and add the
/// matching step in [`migrate_document`].
pub const CONFIG_VERSION: u32 = 2;

/// `config_version` is absent from files written before versioning existed, and such a
/// file must be treated as v0 rather than as "current" — otherwise a migration could
/// never trigger.
fn unversioned_config_version() -> u32 {
    0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Whether this config may be written back to the user's file.
    ///
    /// The TUI owns the terminal and the user's config; a test, the CLI and the headless paths
    /// do not — and they *do* call the same setters and commands, every one of which saves. A
    /// test that resized a pane used to write its own default config straight over the real one
    /// (measured: a `cargo test` run replaced a user's theme, gradient, player-bar layout and
    /// navigation position with defaults). Never the user's file from a process that does not
    /// own it.
    #[serde(skip)]
    pub persist: bool,
    /// `[keys]`: which key runs which command, by the command's name — the command table's `key`
    /// column is the default, and this is what a user rebinds. Empty means "the table's own keys".
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub keys: crate::config::keymap::KeyBindings,
    #[serde(default = "unversioned_config_version")]
    pub config_version: u32,
    pub default_theme: String,
    /// Theme used when the terminal background is light; `default_theme` is the dark slot.
    #[serde(default)]
    pub light_theme: Option<String>,
    /// The panes inside the frame: their sizes, and which are collapsed.
    #[serde(default)]
    pub panes: PanesConfig,
    /// Whether the app paints the theme's background over the whole frame: `auto` (only when it
    /// differs from the terminal's, which keeps a translucent or acrylic terminal background),
    /// `always`, or `never`.
    #[serde(default)]
    pub paint_background: BackgroundFill,
    /// Which background the theme slots are chosen for: `auto` (follow the terminal),
    /// `dark`, or `light`.
    #[serde(default)]
    pub background: BackgroundMode,
    /// Glyph preset and per-key overrides for terminals without a Nerd Font.
    #[serde(default)]
    pub symbols: SymbolsConfig,
    pub border: BorderConfig,
    pub seek_interval_secs: u32,
    /// User themes, `[themes.<name>]` in the config: a base plus the colours to override.
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub themes: HashMap<String, UserTheme>,
    pub logger: Logger,
    pub navigation: NavConfig,
    #[serde(default)]
    pub cache: CacheConfig,
    /// `[audio]`: what the player does to the sound on its way to the device — sample-rate
    /// conversion, the parametric EQ and loudness normalization.
    #[serde(default)]
    pub audio: AudioConfig,
    #[serde(default)]
    pub playerbar: PlayerbarConfig,
    #[serde(default)]
    pub titles: TitlesConfig,
    #[serde(default)]
    pub columns: ColumnsConfig,
    /// Lyrics highlight gradient style: warm / cubehelix / rainbow / spectral / viridis / turbo.
    #[serde(default)]
    pub lyric_gradient: GradientPreset,
    /// Colour of the sung part in the `ktv` lyrics style: a theme field (`accent`, `text`, …)
    /// or a colour of its own (`blue`, `lightblue`, `#4da6ff`, an ANSI index).
    #[serde(default = "default_lyric_ktv_color")]
    pub lyric_ktv_color: String,
    /// Draw the translated lyric lines under the original ones (`y`, `:translation on|off`).
    /// Only songs whose lyrics come with a translation have any to draw.
    #[serde(default = "default_true")]
    pub lyric_translation: bool,
    /// How the lyrics page draws: `window`, `one_line`, `ktv`, `flow` or `plain`.
    #[serde(default)]
    pub lyric_style: LyricStyle,
    /// Capture the mouse: clicks on the player bar, the tabs and the lists. Turning it off
    /// keeps the terminal's own selection and scrolling (holding `Shift` does the same in
    /// most terminals, without the config).
    #[serde(default = "default_true")]
    pub mouse: bool,
    /// Shape of the cursor in the input fields: `default` (whatever the terminal is set to),
    /// `block`, `underline` or `bar`.
    #[serde(default)]
    pub cursor_style: crate::utils::terminal::CursorStyle,
    /// Desktop notifications, off by default.
    #[serde(default)]
    pub notify: NotifyConfig,
    /// Update feed: `[update] channel = "stable" | "beta" | "alpha"`.
    #[serde(default)]
    pub update: UpdateConfig,
    /// Proxy address for every network request — the NetEase Cloud API, cover
    /// downloads and audio streams. Absent means a direct connection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy: Option<String>,
    /// Maximum number of search results.
    #[serde(default = "default_search_limit")]
    pub search_limit: u16,
    /// Navigation bar position: left (default), right, top, or bottom.
    #[serde(default)]
    pub navigation_position: NavPosition,
    /// Minimum splash screen display time (seconds); auto-transition waits for this even if boot finishes instantly.
    #[serde(default = "default_splash_duration")]
    pub splash_duration_secs: f64,
    /// Default template for `boxpigma status` (plain format).
    #[serde(default = "default_cli_status_template")]
    pub cli_status_template: String,
    /// Default format for `boxpigma status`: `plain` or `json`.
    #[serde(default = "default_cli_status_format")]
    pub cli_status_format: String,
}

/// The karaoke blue: what a KTV screen paints over the words once they have been sung.
fn default_lyric_ktv_color() -> String {
    "#4da6ff".into()
}

/// Navigation bar position: left (default), right, top, or bottom.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum NavPosition {
    #[default]
    Left,
    Right,
    Top,
    Bottom,
}

impl NavPosition {
    /// The next position in the left → right → top → bottom cycle.
    pub fn cycle(self) -> Self {
        match self {
            NavPosition::Left => NavPosition::Right,
            NavPosition::Right => NavPosition::Top,
            NavPosition::Top => NavPosition::Bottom,
            NavPosition::Bottom => NavPosition::Left,
        }
    }

    /// Human-readable Chinese label used for toasts.
    pub fn label(self) -> &'static str {
        match self {
            NavPosition::Left => "左侧",
            NavPosition::Right => "右侧",
            NavPosition::Top => "顶部",
            NavPosition::Bottom => "底部",
        }
    }
}

fn default_search_limit() -> u16 {
    100
}

fn default_splash_duration() -> f64 {
    2.0
}

fn default_cli_status_template() -> String {
    "{name}  {artist}  {current}/{duration}  {status}  vol {volume}%".into()
}

fn default_cli_status_format() -> String {
    "plain".into()
}

impl Default for Config {
    fn default() -> Self {
        Self {
            config_version: CONFIG_VERSION,
            keys: crate::config::keymap::KeyBindings::new(),
            persist: true,
            default_theme: Theme::default().name,
            light_theme: None,
            panes: PanesConfig::default(),
            paint_background: BackgroundFill::default(),
            background: BackgroundMode::default(),
            symbols: SymbolsConfig::default(),
            border: BorderConfig::default(),
            seek_interval_secs: 15,
            lyric_gradient: GradientPreset::default(),
            lyric_style: LyricStyle::default(),
            lyric_ktv_color: default_lyric_ktv_color(),
            lyric_translation: true,
            mouse: true,
            cursor_style: crate::utils::terminal::CursorStyle::default(),
            notify: NotifyConfig::default(),
            update: UpdateConfig::default(),
            proxy: None,
            search_limit: default_search_limit(),
            navigation_position: NavPosition::default(),
            splash_duration_secs: default_splash_duration(),
            logger: Logger::default(),
            cache: CacheConfig::default(),
            audio: AudioConfig::default(),
            playerbar: PlayerbarConfig::default(),
            titles: TitlesConfig::default(),
            themes: HashMap::new(),
            navigation: NavConfig::default(),
            columns: ColumnsConfig::default(),
            cli_status_template: default_cli_status_template(),
            cli_status_format: default_cli_status_format(),
        }
    }
}

impl Config {
    /// The lyrics page's options, resolved against the theme in force.
    ///
    /// The page is handed this rather than the whole config: it draws five things, and resolving
    /// the `ktv` colour here means once a frame rather than once per line, which is also what
    /// keeps an unknown colour to a single warning per name.
    pub fn lyrics_config(&self, theme: &Theme) -> crate::config::lyrics::LyricsConfig<'_> {
        crate::config::lyrics::LyricsConfig {
            style: self.lyric_style,
            gradient: self.lyric_gradient,
            ktv_color: theme.resolve_color(&self.lyric_ktv_color),
            show_translation: self.lyric_translation,
            title: &self.titles.lyrics,
        }
    }

    pub fn load() -> Self {
        Self::load_from(&utils::boxpigma_config_dir().join("config.toml"))
    }

    /// Load a config from an explicit path, upgrading a file written by an older release.
    ///
    /// Split out of [`Config::load`] so the migration path is testable against a scratch
    /// file instead of the user's real config.
    pub fn load_from(config_path: &Path) -> Self {
        let (mut config, migrated) = if config_path.exists() {
            match fs::read_to_string(config_path) {
                Ok(text) => match toml_edit::de::from_str(&text) {
                    Ok(cfg) => (cfg, migrate_document(&text, config_path)),
                    Err(e) => {
                        log::warn!("Failed to parse config.toml: {e}, using defaults");
                        (Config::default(), None)
                    }
                },
                Err(e) => {
                    log::warn!("Failed to read config.toml: {e}, using defaults");
                    (Config::default(), None)
                }
            }
        } else {
            (Config::default(), None)
        };

        // The migration edited the *file*, so the config it produced is read back from the text
        // that is about to be written: one implementation of the rules, and the app runs with what
        // the file now says rather than with a struct migrated on the side.
        if let Some(text) = migrated.as_deref() {
            match toml_edit::de::from_str(text) {
                Ok(cfg) => config = cfg,
                Err(e) => log::warn!("Failed to read the migrated config.toml back: {e}"),
            }
        }

        // A config read from the user's file is the user's: the app that loaded it may write it
        // back. (`Config::deserialize` cannot know this — `persist` is skipped on the way in —
        // so it is said here, and `App::new` narrows it again to "the process owns the
        // terminal".)
        config.persist = true;

        // Write the file back when this call changed it: there was no file (a fresh default),
        // or `migrate_document` just upgraded one (after the backup it took). Doing it here
        // rather than at the next `save()` is what makes a migration visible at once — the keys
        // the new schema dropped stop sitting in the user's file — and it is why the write
        // targets the path that was loaded rather than `save()`'s own directory.
        //
        // A file that is already current is left byte-for-byte alone, which is what keeps a
        // user's own comments, and a file that exists but failed to parse is left alone too:
        // overwriting it with defaults would silently discard what they wrote.
        let fresh = !config_path.exists();
        if fresh || migrated.is_some() {
            if fresh && let Some(dir) = config_path.parent() {
                let _ = fs::create_dir_all(dir);
            }
            // A migration hands back the user's own document with only the schema's changes in
            // it; a fresh file has nothing of theirs in it and is written from the struct.
            let content = migrated.unwrap_or_else(|| config.to_toml());
            if content.is_empty() {
                log::error!("Refusing to overwrite config.toml with an empty document");
            } else if let Err(e) = utils::fs::write_atomic(config_path, content.as_bytes()) {
                log::warn!("Failed to write config.toml: {e}");
            }
        }
        config
    }

    pub fn save(&self) {
        if !self.persist {
            return;
        }

        let dir = utils::boxpigma_config_dir();
        if let Err(e) = fs::create_dir_all(&dir) {
            log::error!("Failed to create config directory: {e}");
            return;
        }
        let content = self.to_toml();
        if content.is_empty() {
            log::error!("Refusing to overwrite config.toml with an empty document");
            return;
        }
        if let Err(e) = utils::fs::write_atomic(&dir.join("config.toml"), content.as_bytes()) {
            log::error!("Failed to write config.toml: {e}");
        }
    }

    pub(crate) fn to_toml(&self) -> String {
        let Ok(pretty) = toml_edit::ser::to_string_pretty(self) else {
            log::error!("failed to serialize config to TOML");
            return String::new();
        };
        let Ok(mut doc) = pretty.parse::<toml_edit::DocumentMut>() else {
            return pretty;
        };

        // Every step below is best-effort: `serde` may omit these tables (or serialize them
        // as plain arrays instead of arrays-of-tables, e.g. when the user clears
        // `navigation.sections` or `columns.songs`), which must not panic.
        if let Some(nav) = doc.get_mut("navigation").and_then(|v| v.as_table_mut()) {
            nav.set_implicit(true);
            if let Some(sections) = nav
                .get_mut("sections")
                .and_then(|v| v.as_array_of_tables_mut())
            {
                for section in sections.iter_mut() {
                    utils::format::convert_aot_to_inline(section, "items", "\n  ");
                }
            }
        }

        if let Some(columns) = doc.get_mut("columns").and_then(|v| v.as_table_mut()) {
            columns.set_implicit(true);
            if let Some(overrides) = columns.get_mut("overrides").and_then(|v| v.as_table_mut()) {
                overrides.set_implicit(true);
                utils::format::convert_all_aot_to_inline(overrides, "\n  ");
            }
            utils::format::convert_aot_to_inline(columns, "songs", "\n  ");
            utils::format::convert_aot_to_inline(columns, "songlist", "\n  ");
        }

        doc.to_string()
    }
}

/// Where a key sits in the user's file: the names — and the array elements — to walk down from the
/// root, as [`stale_keys`] walked them.
#[derive(Debug, Clone)]
enum Step {
    Key(String),
    Element(usize),
}

/// A place a key can be found in — a table (`[a]`, `a.b`, an inline table, the root of the file) or
/// an element of an array — plus the item that *is* a key's value.
enum Node<'a> {
    Item(&'a mut toml_edit::Item),
    Table(&'a mut toml_edit::Table),
    Value(&'a mut toml_edit::Value),
}

impl<'a> Node<'a> {
    /// The entries this node holds, when it holds keys at all.
    fn entries(self) -> Option<&'a mut dyn toml_edit::TableLike> {
        match self {
            Node::Item(item) => item.as_table_like_mut(),
            Node::Table(table) => Some(table),
            Node::Value(toml_edit::Value::InlineTable(table)) => Some(table),
            Node::Value(_) => None,
        }
    }

    /// The element `index` of this node, when it is an array — an array of tables, or one of the
    /// arrays a table writes tables in.
    fn element(self, index: usize) -> Option<Node<'a>> {
        match self {
            Node::Item(toml_edit::Item::ArrayOfTables(arrays)) => {
                Some(Node::Table(arrays.get_mut(index)?))
            }
            Node::Item(toml_edit::Item::Value(toml_edit::Value::Array(array)))
            | Node::Value(toml_edit::Value::Array(array)) => {
                Some(Node::Value(array.get_mut(index)?))
            }
            _ => None,
        }
    }
}

/// Walk `path` down from `node`. `None` cannot happen for a path this module built — the sweep only
/// walks keys it has just looked at — so both callers read it as "nothing there to touch".
fn node_at<'a>(mut node: Node<'a>, path: &[Step]) -> Option<Node<'a>> {
    for step in path {
        node = match step {
            Step::Key(name) => Node::Item(node.entries()?.get_mut(name)?),
            Step::Element(index) => node.element(*index)?,
        };
    }
    Some(node)
}

/// The shapes a key is asked about, in the order they are tried: a number, a string, an array, a
/// table.
const PROBE_SHAPES: [&str; 4] = ["0", "\"\"", "[]", "{}"];

/// Whether [`Config`] still has the key at `path`.
///
/// `Config`'s own `Deserialize` is the one list of keys that exists, so the question goes to it
/// rather than to a list written here: the key is handed a value of every shape a TOML value has
/// (see [`PROBE_SHAPES`]) and the file is read back. A key serde does not know, it *ignores*, and
/// it ignores all four of them — that is what "the new schema dropped this key" means. A key the
/// schema still has rejects at least one: the four shapes are a number, a string, an array and a
/// table, and every field of the config graph is one of those things or has one inside it (an
/// `Option`, a `Vec`, a `HashMap`, an untagged `ColorSpec` — each refuses at least one shape).
/// Nothing in the graph is a catch-all that would swallow all four, which is what makes "all four
/// were ignored" an answer about the schema rather than about the value.
fn schema_ignores(doc: &toml_edit::DocumentMut, path: &[Step]) -> bool {
    PROBE_SHAPES
        .iter()
        .all(|shape| schema_takes(doc, path, shape))
}

/// Read the file back with the key at `path` holding what `literal` spells: whether the schema took
/// it. The file is asked on a copy — the user's own document is never touched by a question.
fn schema_takes(doc: &toml_edit::DocumentMut, path: &[Step], literal: &str) -> bool {
    let Ok(value) = literal.parse::<toml_edit::Value>() else {
        // One of ours that does not parse would make every key look unknown; "this shape was not
        // taken" is the reading that keeps the key.
        return false;
    };
    let mut probe = doc.clone();
    let Some(Node::Item(entry)) = node_at(Node::Item(probe.as_item_mut()), path) else {
        return false;
    };
    *entry = toml_edit::Item::Value(value);
    toml_edit::de::from_str::<Config>(&probe.to_string()).is_ok()
}

/// Every key of the user's own file that [`Config`] no longer has, at every level of it.
fn stale_keys(doc: &toml_edit::DocumentMut) -> Vec<Vec<Step>> {
    let mut stale = Vec::new();
    let mut path = Vec::new();
    visit_keys(doc, doc.as_table(), &mut path, &mut stale);
    stale
}

/// One level of the sweep: every key a table holds, and — for a key the schema still has — what is
/// under it. A key that is gone is not looked into: dropping it takes the subtree with it.
fn visit_keys(
    doc: &toml_edit::DocumentMut,
    at: &dyn toml_edit::TableLike,
    path: &mut Vec<Step>,
    stale: &mut Vec<Vec<Step>>,
) {
    for (name, item) in at.iter() {
        path.push(Step::Key(name.to_string()));
        if schema_ignores(doc, path) {
            stale.push(path.clone());
        } else {
            visit_item(doc, item, path, stale);
        }
        path.pop();
    }
}

/// What is under a key the schema has: a table, the elements of an array of tables, or an array —
/// which is another place a table can hide.
fn visit_item(
    doc: &toml_edit::DocumentMut,
    at: &toml_edit::Item,
    path: &mut Vec<Step>,
    stale: &mut Vec<Vec<Step>>,
) {
    match at {
        toml_edit::Item::Table(table) => visit_keys(doc, table, path, stale),
        toml_edit::Item::ArrayOfTables(arrays) => {
            for (index, table) in arrays.iter().enumerate() {
                path.push(Step::Element(index));
                visit_keys(doc, table, path, stale);
                path.pop();
            }
        }
        toml_edit::Item::Value(value) => visit_value(doc, value, path, stale),
        toml_edit::Item::None => {}
    }
}

/// What is under a key the schema has and that is written as a value: an inline table holds keys
/// like any other table, and an array holds them in its elements.
fn visit_value(
    doc: &toml_edit::DocumentMut,
    at: &toml_edit::Value,
    path: &mut Vec<Step>,
    stale: &mut Vec<Vec<Step>>,
) {
    match at {
        toml_edit::Value::InlineTable(table) => visit_keys(doc, table, path, stale),
        toml_edit::Value::Array(array) => {
            for (index, value) in array.iter().enumerate() {
                path.push(Step::Element(index));
                visit_value(doc, value, path, stale);
                path.pop();
            }
        }
        _ => {}
    }
}

/// Take the key at `path` out of the user's document — its own comment goes with it.
fn remove_key(doc: &mut toml_edit::DocumentMut, path: &[Step]) {
    let Some((Step::Key(name), above)) = path.split_last() else {
        return;
    };
    let Some(entries) =
        node_at(Node::Item(doc.as_item_mut()), above).and_then(|node| node.entries())
    else {
        return;
    };
    entries.remove(name);
}

/// The key a path names, the way the user reads it in their file: `cache.quality`,
/// `navigation.sections[0].items`.
fn path_name(path: &[Step]) -> String {
    let mut name = String::new();
    for step in path {
        match step {
            Step::Key(key) => {
                if !name.is_empty() {
                    name.push('.');
                }
                name.push_str(key);
            }
            Step::Element(index) => name.push_str(&format!("[{index}]")),
        }
    }
    name
}

/// The schema version the user's file declares.
///
/// Read off the document rather than off a parsed [`Config`], because it is the file that is being
/// upgraded. `config_version` first appeared in v1, so a file that says nothing — or says something
/// that is not a version — is v0, the reading [`unversioned_config_version`] gives it.
fn document_version(doc: &toml_edit::DocumentMut) -> u32 {
    doc.get("config_version")
        .and_then(toml_edit::Item::as_integer)
        .and_then(|version| u32::try_from(version).ok())
        .unwrap_or_else(unversioned_config_version)
}

/// Write the version this build writes, in place: whatever the user has on that line — their own
/// spacing, a comment at the end of it — is the value's decor, and the new value is handed it back.
fn set_version(doc: &mut toml_edit::DocumentMut) {
    let mut version = toml_edit::Value::from(i64::from(CONFIG_VERSION));
    match doc
        .get_mut("config_version")
        .and_then(toml_edit::Item::as_value_mut)
    {
        Some(existing) => {
            *version.decor_mut() = existing.decor().clone();
            *existing = version;
        }
        // A file older than versioning has no such line, and gets one.
        None => {
            doc.insert("config_version", toml_edit::Item::Value(version));
        }
    }
}

/// Upgrade the user's own file to [`CONFIG_VERSION`] by editing their document instead of writing a
/// new one: their keys keep their values, their order, their spacing and their comments, and only
/// what the schema requires moves. Returns the text to write, or `None` when the file needs nothing
/// — it is already at this schema, or it comes from a newer build and is left as it is (its unknown
/// fields are ignored rather than downgraded).
///
/// The previous file is copied to `config.toml.bak-v{old}` first, so a user can always roll back.
/// The caller writes the result out at once: a migration the user cannot see on disk has not
/// happened as far as the next reader of that file is concerned.
fn migrate_document(text: &str, config_path: &Path) -> Option<String> {
    let mut doc = text.parse::<toml_edit::DocumentMut>().ok()?;
    let from = document_version(&doc);
    if from == CONFIG_VERSION {
        return None;
    }
    if from > CONFIG_VERSION {
        log::warn!(
            "config.toml is schema v{from}, this build understands v{CONFIG_VERSION}: \
             newer fields are ignored"
        );
        return None;
    }

    let backup = config_path.with_extension(format!("toml.bak-v{from}"));
    match fs::copy(config_path, &backup) {
        Ok(_) => log::info!(
            "config.toml schema v{from} → v{CONFIG_VERSION}（已备份到 {}）",
            backup.display()
        ),
        Err(e) => log::warn!(
            "config.toml schema v{from} → v{CONFIG_VERSION}（备份到 {} 失败: {e}）",
            backup.display()
        ),
    }

    // Steps run in order, each rewriting whatever the previous schema got wrong.

    // v1 → v2: `proxy_target` is gone along with the multi-source fallback it scoped, and `proxy`
    // now covers every network source. `normal` (the default) only ever proxied one non-NetEase
    // source — the source that no longer exists — so that address is dropped and the user ends up
    // on a direct connection; `reversed`/`both` meant "proxy NetEase traffic", which is exactly
    // what `proxy` means now, so it is kept. The key itself is one the new schema no longer has, so
    // the sweep below is what takes it out of the file.
    if from < 2 {
        match doc.get("proxy_target").and_then(toml_edit::Item::as_str) {
            None | Some("normal") => {
                doc.remove("proxy");
                log::info!(
                    "config.toml schema v1 → v2: proxy_target 为默认值，不再代理（清理 proxy）"
                );
            }
            Some(target) => {
                log::info!("config.toml schema v1 → v2: 保留 proxy（原 proxy_target = {target}）");
            }
        }
    }

    // Everything the steps left in the file that the schema no longer has: keys a file written by
    // this build cannot carry, and which nothing would ever take out again.
    let stale = stale_keys(&doc);
    for path in &stale {
        remove_key(&mut doc, path);
    }
    if !stale.is_empty() {
        let names: Vec<String> = stale.iter().map(|path| path_name(path)).collect();
        log::info!(
            "config.toml schema v{from} → v{CONFIG_VERSION}: 删除新 schema 没有的键 {}",
            names.join("、")
        );
    }

    set_version(&mut doc);
    Some(doc.to_string())
}

#[cfg(test)]
mod tests {
    /// A config the process does not own is never written: `App::new(_, false)` — tests, the
    /// CLI, the headless paths — leaves the user's file exactly as it found it.
    ///
    /// This is the regression test for a `cargo test` run replacing a real config with defaults
    /// (theme, gradient, player-bar layout, navigation position — all of it), because a test
    /// resized a pane and the setter that follows a drag saved.
    #[test]
    fn a_config_that_is_not_the_users_is_never_written() {
        let path = utils::boxpigma_config_dir().join("config.toml");
        let before = fs::read(&path).ok();
        let config = Config {
            persist: false,
            ..Config::default()
        };

        config.save();

        assert_eq!(
            fs::read(&path).ok(),
            before,
            "a config the process does not own was written over the user's"
        );
    }

    /// Mouse capture is what makes the player bar clickable and also what stops the terminal
    /// from selecting text, so the default is on and the way out has to actually parse.
    #[test]
    fn mouse_defaults_to_on_and_can_be_turned_off() {
        let parsed: Config = toml_edit::de::from_str("mouse = false\n").expect("parse");
        assert!(!parsed.mouse);
        assert!(
            Config::default().mouse,
            "capture is on unless asked otherwise"
        );
    }

    use super::*;

    #[test]
    fn default_config_serializes_save_on_play() {
        let cfg = Config::default();
        let toml = cfg.to_toml();
        assert!(
            toml.contains("save_on_play = true"),
            "missing save_on_play in default config:\n{toml}"
        );
    }

    /// A cleared `navigation.sections` / `columns.songs` is serialized as an empty array
    /// rather than an array of tables, which used to panic inside `to_toml`.
    #[test]
    fn to_toml_survives_empty_sections_and_columns() {
        let mut cfg = Config::default();
        cfg.navigation.sections.clear();
        cfg.columns.songs.clear();

        let toml = cfg.to_toml();
        assert!(!toml.is_empty(), "config serialization produced nothing");
    }

    /// Scratch directory for the file-backed tests: they run in parallel, so the name
    /// carries the test's own label.
    fn scratch_dir(label: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "boxpigma-config-test-{}-{label}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create scratch dir");
        dir
    }

    /// A file written before versioning existed (no `config_version`) is upgraded in
    /// memory, keeps its values, and is backed up so the user can roll back.
    #[test]
    fn legacy_config_is_migrated_and_backed_up() {
        let dir = scratch_dir("legacy");
        let path = dir.join("config.toml");
        fs::write(&path, "default_theme = \"dracula\"\nsearch_limit = 42\n")
            .expect("write legacy config");

        let cfg = Config::load_from(&path);

        assert_eq!(cfg.config_version, CONFIG_VERSION);
        assert_eq!(cfg.default_theme, "dracula", "user values must survive");
        assert_eq!(cfg.search_limit, 42, "user values must survive");
        assert!(
            dir.join("config.toml.bak-v0").exists(),
            "the pre-versioning file must be backed up"
        );
        let rewritten = fs::read_to_string(&path).expect("read migrated config");
        assert!(
            rewritten.contains(&format!("config_version = {CONFIG_VERSION}")),
            "the migrated file must be written back at startup, not at the next save:\n{rewritten}"
        );
        assert!(
            fs::read_to_string(dir.join("config.toml.bak-v0"))
                .expect("read the backup")
                .contains("search_limit = 42"),
            "the backup has to hold the file as it was, or it is not a rollback"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// The v1 → v2 rule: the old `proxy_target` decides whether the configured `proxy`
    /// survives. `normal` only ever proxied a source that no longer exists — never
    /// NetEase — so the address is dropped and the user ends up direct;
    /// `reversed`/`both` meant NetEase traffic goes through the proxy, which is what
    /// `proxy` means now, so it is kept. Neither the stale key nor the (removed)
    /// `[source_fallback]` section may be written back.
    #[test]
    fn v1_proxy_target_decides_whether_the_proxy_survives() {
        for (target, expected) in [
            ("normal", None),
            ("both", Some("http://127.0.0.1:7890")),
            ("reversed", Some("http://127.0.0.1:7890")),
        ] {
            let dir = scratch_dir(&format!("proxy-target-{target}"));
            let path = dir.join("config.toml");
            fs::write(
                &path,
                format!(
                    "config_version = 1\n\
                     proxy = \"http://127.0.0.1:7890\"\n\
                     proxy_target = \"{target}\"\n\
                     [source_fallback]\n\
                     enabled = true\n"
                ),
            )
            .expect("write v1 config");

            let cfg = Config::load_from(&path);

            assert_eq!(cfg.config_version, CONFIG_VERSION);
            assert_eq!(cfg.proxy.as_deref(), expected, "proxy_target = {target}");
            assert!(
                dir.join("config.toml.bak-v1").exists(),
                "the v1 file must be backed up"
            );
            let on_disk = fs::read_to_string(&path).expect("read the migrated config");
            assert!(
                on_disk.contains(&format!("config_version = {CONFIG_VERSION}")),
                "the file must be brought to the new schema at startup:\n{on_disk}"
            );
            assert!(
                !on_disk.contains("proxy_target") && !on_disk.contains("source_fallback"),
                "the keys the new schema dropped must be gone from the user's file:\n{on_disk}"
            );
            let backup = fs::read_to_string(dir.join("config.toml.bak-v1")).expect("read backup");
            assert!(
                backup.contains("proxy_target") && backup.contains("[source_fallback]"),
                "the backup is the rollback path and must keep the v1 file verbatim:\n{backup}"
            );
            let _ = fs::remove_dir_all(&dir);
        }
    }

    /// The startup path end to end: `save`/first-run output carries the current version and
    /// is read back without triggering another migration.
    #[test]
    fn saved_config_round_trips_without_migrating_again() {
        let dir = scratch_dir("roundtrip");
        let path = dir.join("config.toml");
        let written = Config {
            default_theme: "dracula".into(),
            search_limit: 42,
            ..Config::default()
        };
        fs::write(&path, written.to_toml()).expect("write config");

        let cfg = Config::load_from(&path);

        assert_eq!(
            cfg.config_version, CONFIG_VERSION,
            "the written file must carry the current version"
        );
        assert_eq!(cfg.default_theme, "dracula", "values must survive");
        assert_eq!(cfg.search_limit, 42, "values must survive");
        assert!(
            !dir.join(format!("config.toml.bak-v{CONFIG_VERSION}"))
                .exists(),
            "a config at the current version must not be migrated"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// A file already at the current schema is not rewritten: it is the user's file, and what
    /// they wrote in it — comments included — is part of it. Only a migration overwrites it,
    /// and only after taking a backup.
    #[test]
    fn a_current_config_is_left_byte_for_byte_alone() {
        let dir = scratch_dir("current-untouched");
        let path = dir.join("config.toml");
        let authored = format!(
            "# 手写的一行注释：重写就会丢\n\
             config_version = {CONFIG_VERSION}\n\
             default_theme = \"dracula\"\n\
             search_limit = 7\n"
        );
        fs::write(&path, &authored).expect("write current config");

        let cfg = Config::load_from(&path);

        assert_eq!(cfg.search_limit, 7, "the value must still be read");
        assert_eq!(
            fs::read_to_string(&path).expect("read config"),
            authored,
            "a config that was not migrated must not be rewritten"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// A file from a newer build is used as it is and never written back: this build does not
    /// understand its fields, and rewriting it would drop them.
    #[test]
    fn a_newer_config_is_read_but_never_rewritten() {
        let dir = scratch_dir("newer");
        let path = dir.join("config.toml");
        let authored = format!(
            "config_version = {}\nsearch_limit = 9\nfuture_key = \"kept\"\n",
            CONFIG_VERSION + 1
        );
        fs::write(&path, &authored).expect("write newer config");

        let cfg = Config::load_from(&path);

        assert_eq!(cfg.search_limit, 9, "the fields this build knows are read");
        assert_eq!(
            fs::read_to_string(&path).expect("read config"),
            authored,
            "a file from a newer schema must be left alone, not downgraded"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// The sweep's answer for a document, as the paths read in a failure message.
    fn sweep(doc: &toml_edit::DocumentMut) -> Vec<String> {
        stale_keys(doc).iter().map(|path| path_name(path)).collect()
    }

    /// A migration edits the user's own document instead of writing a new one: what they wrote
    /// around the change — their comments, their spacing, their order — is still there character
    /// for character, and only the line the schema owns moves.
    #[test]
    fn a_migration_keeps_what_the_user_wrote() {
        let dir = scratch_dir("comments");
        let path = dir.join("config.toml");
        let authored = "# 手写的说明，迁移不能碰它\n\
                        config_version = 1\n\
                        default_theme = \"dracula\"  # 行尾注释也在\n\
                        \n\
                        # 缓存一节\n\
                        [cache]\n\
                        quality = \"flac\"\n";
        fs::write(&path, authored).expect("write the user's config");

        let cfg = Config::load_from(&path);

        assert_eq!(cfg.default_theme, "dracula", "the value must still be read");
        assert_eq!(cfg.cache.quality, "flac", "and the section's");
        assert_eq!(
            fs::read_to_string(&path).expect("read the migrated config"),
            authored.replace(
                "config_version = 1",
                &format!("config_version = {CONFIG_VERSION}")
            ),
            "a migration may only move what the schema requires; the rest is the user's file"
        );
        assert_eq!(
            fs::read_to_string(dir.join("config.toml.bak-v1")).expect("read the backup"),
            authored,
            "the backup is the rollback path and holds the file exactly as it was"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// The keys the new schema dropped leave the user's file at every level they sit at: the v1
    /// `proxy_target`, the section it scoped (no key of this build at all), one inside a table the
    /// schema still has, and one inside a table inside an array.
    #[test]
    fn a_migration_drops_the_keys_the_schema_no_longer_has() {
        let dir = scratch_dir("stale");
        let path = dir.join("config.toml");
        fs::write(
            &path,
            "config_version = 1\n\
             search_limit = 42\n\
             proxy = \"http://127.0.0.1:7890\"\n\
             proxy_target = \"both\"\n\
             [source_fallback]\n\
             enabled = true\n\
             [cache]\n\
             quality = \"flac\"\n\
             old_ttl = 3\n\
             [[navigation.sections]]\n\
             title = \"我的\"\n\
             icon = \"star\"\n\
             items = [{ name = \"我喜欢的音乐\", api = \"liked\", badge = \"x\" }]\n",
        )
        .expect("write the file");

        let cfg = Config::load_from(&path);

        assert_eq!(cfg.config_version, CONFIG_VERSION);
        assert_eq!(cfg.search_limit, 42, "the keys the schema has are kept");
        assert_eq!(
            cfg.proxy.as_deref(),
            Some("http://127.0.0.1:7890"),
            "proxy_target = both means NetEase traffic went through the proxy, which is what `proxy` means now"
        );
        assert_eq!(
            cfg.navigation.sections[0].items[0].name, "我喜欢的音乐",
            "and what is under a section it keeps is still read"
        );

        let on_disk = fs::read_to_string(&path).expect("read the migrated config");
        for gone in [
            "proxy_target",
            "source_fallback",
            "old_ttl",
            "icon",
            "badge",
        ] {
            assert!(
                !on_disk.contains(gone),
                "`{gone}` is not a key of the new schema and must not stay in the user's file:\n{on_disk}"
            );
        }
        assert!(
            on_disk.contains("quality = \"flac\"")
                && on_disk.contains("title = \"我的\"")
                && on_disk.contains("proxy = \"http://127.0.0.1:7890\""),
            "the keys the schema has keep their values:\n{on_disk}"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// The sweep is only allowed to take away keys the schema does not have. A file holding one key
    /// of every kind the config writes — scalars, a map, nested tables, a table inside an array, a
    /// theme with its own sections — comes through with all of them, while a key the schema never
    /// had on the same file is found, so the first half cannot pass for the wrong reason.
    #[test]
    fn the_sweep_keeps_every_key_the_config_has() {
        let text = "config_version = 2\n\
                    default_theme = \"dracula\"\n\
                    mouse = true\n\
                    search_limit = 100\n\
                    seek_interval_secs = 5\n\
                    proxy = \"http://127.0.0.1:7890\"\n\
                    [keys]\n\
                    volume_up = \"volume +5\"\n\
                    [cache]\n\
                    content_cache_ttl = 1\n\
                    quality = \"flac\"\n\
                    [themes.mine]\n\
                    base = \"dracula\"\n\
                    accent = \"#4da6ff\"\n\
                    [themes.mine.table]\n\
                    row = { fg = \"text\", bg = \"surface\" }\n\
                    [themes.\"my.theme\"]\n\
                    base = \"dracula\"\n\
                    [[navigation.sections]]\n\
                    title = \"我\"\n\
                    items = [{ name = \"x\", api = \"y\", title_template = \"{name}\" }]\n\
                    [columns.overrides]\n\
                    default = [{ header = \"H\", field = \"f\", width = 3, ratio = [1, 2] }]\n\
                    [symbols]\n\
                    preset = \"nerd\"\n\
                    selected = \">\"\n";
        let doc: toml_edit::DocumentMut = text.parse().expect("the sample parses");
        toml_edit::de::from_str::<Config>(text).expect("the sample is a config this build reads");

        assert_eq!(
            sweep(&doc),
            Vec::<String>::new(),
            "the sweep would take away keys the schema has"
        );

        // The same document with keys the schema never had — at the top level and inside a table it
        // does have — so the assertion above cannot pass for the wrong reason (a schema that
        // rejected everything would take all of them away).
        let mut with_stale = doc.clone();
        let replaced = with_stale.insert("no_such_key", toml_edit::value(1));
        assert!(replaced.is_none(), "the sample never had such a key");
        let theme = with_stale
            .get_mut("themes")
            .and_then(|item| item.as_table_like_mut())
            .and_then(|themes| themes.get_mut("mine"))
            .and_then(|item| item.as_table_like_mut())
            .expect("the sample has that theme");
        theme.insert("old_colour", toml_edit::value("red"));
        let mut stale = sweep(&with_stale);
        stale.sort();
        assert_eq!(
            stale,
            vec![
                "no_such_key".to_string(),
                "themes.mine.old_colour".to_string()
            ],
            "the keys the schema never had, and only those"
        );

        // A key the user wrote as a dotted key goes whole: the table it was folded into is not a
        // key of the schema either, and an empty one left behind would be one more thing to look at.
        let mut dotted: toml_edit::DocumentMut = "config_version = 2\nold_theme.section = true\n"
            .parse()
            .expect("the sample parses");
        for path in stale_keys(&dotted) {
            remove_key(&mut dotted, &path);
        }
        assert!(
            !dotted.to_string().contains("old_theme"),
            "a stale dotted key is removed with the table it made:\n{dotted}"
        );
    }

    /// A 329-line example with a comment on nearly every key is what a user copies, so it is the
    /// corpus the migration is checked against: migrating it from v1 keeps every line that the
    /// schema has, keeps the comments that explain them, and leaves the settings the file means
    /// exactly as they were.
    #[test]
    fn the_example_config_migrates_without_rewriting_what_it_keeps() {
        let example = include_str!("../config.example.toml");
        let v1 = example.replace("config_version = 2", "config_version = 1");
        assert_ne!(v1, example, "the example declares the current version");

        let dir = scratch_dir("example");
        let path = dir.join("config.toml");
        fs::write(&path, &v1).expect("write the example as a version-1 file");

        let migrated = migrate_document(&v1, &path).expect("a version-1 file is migrated");

        // The migration only takes lines out; it never rewrites one. (The version line is the
        // example's own line, with the number the schema writes.)
        let version_line = format!("config_version = {CONFIG_VERSION}");
        for line in migrated.lines().filter(|line| !line.is_empty()) {
            assert!(
                example.contains(line) || line == version_line,
                "a migration may not rewrite `{line}`"
            );
        }

        // What the app reads out of the file must not change: the migration takes away only what
        // serde ignores. The two readings are compared in their canonical form — the form the
        // app's own `save` writes — with the lines sorted, because one map in it
        // (`columns.overrides`) is a `HashMap` and writes its entries in a different order every
        // run. The version is the one field the migration is there to change. (The example keeps a
        // handful of lyrics keys inside `[notify]`, which the schema does not have there and serde
        // ignores — the sweep takes those out of a user's file, the same way the rewrite this
        // replaced did.)
        let mut before: Config = toml_edit::de::from_str(&v1).expect("the example reads");
        before.config_version = CONFIG_VERSION;
        let after =
            toml_edit::de::from_str::<Config>(&migrated).expect("the migrated example reads");
        let (wants_text, got_text) = (before.to_toml(), after.to_toml());
        let (mut wants, mut got): (Vec<&str>, Vec<&str>) =
            (wants_text.lines().collect(), got_text.lines().collect());
        wants.sort();
        got.sort();
        assert_eq!(
            got, wants,
            "a migration may not change the settings the file means"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// A pre-versioning file is brought to this schema once, and what is written is a file this
    /// build reads back as current: `config_version` lands where the schema looks for it — with the
    /// user's other top-level keys, not inside the last section of the file.
    #[test]
    fn an_unversioned_file_is_migrated_once_and_keeps_its_sections() {
        let dir = scratch_dir("unversioned");
        let path = dir.join("config.toml");
        fs::write(&path, "search_limit = 7\n[cache]\nquality = \"flac\"\n").expect("write config");

        assert_eq!(Config::load_from(&path).config_version, CONFIG_VERSION);

        let written = fs::read_to_string(&path).expect("read the migrated config");
        let again = Config::load_from(&path);
        assert_eq!(
            again.config_version, CONFIG_VERSION,
            "the version has to sit where the schema reads it:\n{written}"
        );
        assert_eq!(again.search_limit, 7, "values must survive");
        assert_eq!(again.cache.quality, "flac", "sections must survive");
        assert_eq!(
            fs::read_to_string(&path).expect("read it again"),
            written,
            "a file at the current version must not be migrated a second time"
        );
        let _ = fs::remove_dir_all(&dir);
    }
}
