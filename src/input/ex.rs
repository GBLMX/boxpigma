//! Vim-style `:` command line: parsing, `Tab` completion and execution.
//!
//! The prompt mirrors the search field's editing behaviour (`TextInput`, `Esc`, `Enter`)
//! but executes ex commands instead of searching. Commands are named after what they do
//! rather than after a key, so `:volume +5` and the `+` key reach the same code, and the
//! volume grammar is the one `boxpigma msg volume` already accepts.
//!
//! A few commands change a setting rather than doing something once (`:notify song_change on`,
//! `:lyricgradient spectral`). Those are live because of where the config is read: the
//! notify switches when an event fires, the lyrics ones on every frame, the save-on-play one
//! when the next track resolves — so the field *is* the switch. The two that are terminal modes
//! rather than values the app draws from (`:mouse`, `:cursor`) send the escape sequence that
//! switches the mode there and then, which is exactly what a restart would send.

use std::io::{self, Write};

use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
};

use crate::{
    app::App,
    cli::parse_volume,
    config::{Config, LyricStyle, NotifyConfig, Pane, ProgressStyle},
    event::{AppEvent, AuthEvent, NavigationEvent, PlaybackEvent},
    ipc::MsgAction,
    key::{KeyCode, KeyPress},
    playback::{SeekTarget, parse_seek},
    state::{LoginMethod, Page},
    text_input::TextInput,
    utils::{GradientPreset, Named, terminal::CursorStyle},
};

/// What a command line asks for.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ExCommand {
    Quit,
    Help,
    Login,
    Save,
    /// `None` cycles to the next theme.
    Theme(Option<String>),
    /// `None` cycles to the next style, like every other value-taking command.
    LyricStyle(Option<String>),
    Volume(String),
    Seek(String),
    /// `None` toggles, like the `v` key.
    Visualizer(Option<bool>),
    /// Toggle the border mode (the `b` key).
    Border,
    /// Cycle the navigation bar's position (the `z` key).
    NavPos,
    /// Draw the translated lyric lines under the originals; `None` toggles, like the `y` key.
    Translation(Option<bool>),
    /// The progress bar's look; `None` cycles, like the bare `:lyrics`.
    ProgressStyle(Option<String>),
    /// Open the settings page: the switches as a list rather than one command per line.
    Settings,
    /// Show what the app has said this session.
    Messages,
    /// Show what the app is doing: the loads behind the navigation.
    Tasks,
    /// Throw the playback queue away (the `w` key).
    ClearQueue,
    /// Turn the record on the player bar; `None` toggles, like the `t` key.
    Spin(Option<bool>),
    /// Write to the download cache while playing; `None` toggles. The engine copied the
    /// setting when it was built, so the switch has to reach it too.
    SaveOnPlay(Option<bool>),
    /// One `[notify]` switch; `None` toggles it.
    Notify {
        which: NotifySwitch,
        on: Option<bool>,
    },
    /// Capture the mouse; `None` toggles. The terminal is told at once, the same way the
    /// startup path tells it.
    Mouse(Option<bool>),
    /// Shape of the cursor in the input fields; `None` cycles through the four shapes.
    Cursor(Option<CursorStyle>),
    /// Preset of the lyrics highlight gradient; `None` cycles through the presets.
    LyricGradient(Option<GradientPreset>),
    /// Show, hide or resize a pane: `:pane navigation off`, `:pane playerbar toggle`.
    Pane {
        name: String,
        action: Option<String>,
    },
    /// `None` toggles, like the `V` key.
    Pitch(Option<bool>),
    /// `None` cycles.
    Layout(Option<String>),
    /// Email (or phone number) plus password.
    Signin {
        account: String,
        password: String,
    },
    /// Send the SMS login code to a phone number.
    Sms(String),
    /// Log in with a phone number and the code it received.
    SmsLogin {
        phone: String,
        code: String,
    },
    Logout,
    /// The daily 云贝 check-in.
    Sign,
    /// Open the songs similar to what is playing: the queue's own continuation.
    Simi,
    /// Open the playlists that contain what is playing.
    SimiPlaylists,
    /// Play the private FM: radio seeded by the listener, not by a song.
    Fm,
    /// Keep the current FM song out of the FM from now on.
    FmTrash,
}

impl ExCommand {
    /// Parse one command line, without the leading `:`.
    pub(crate) fn parse(line: &str) -> Result<Self, String> {
        let mut parts = line.split_whitespace();
        let Some(name) = parts.next() else {
            return Err("缺少命令".to_string());
        };
        let args: Vec<&str> = parts.collect();

        let required = |what: &str| -> Result<String, String> {
            match args.as_slice() {
                [value] => Ok((*value).to_string()),
                [] => Err(format!("`{name}` 需要一个{what}")),
                _ => Err(format!("`{name}` 只接受一个参数")),
            }
        };
        let required_pair = |first: &str, second: &str| -> Result<(String, String), String> {
            match args.as_slice() {
                [a, b] => Ok(((*a).to_string(), (*b).to_string())),
                [] => Err(format!("`{name}` 需要{first}与{second}")),
                [_] => Err(format!("`{name}` 还需要{second}")),
                _ => Err(format!("`{name}` 只接受两个参数")),
            }
        };
        let no_args = || -> Result<(), String> {
            if args.is_empty() {
                Ok(())
            } else {
                Err(format!("`{name}` 不接受参数"))
            }
        };

        match name {
            "q" | "quit" => {
                no_args()?;
                Ok(Self::Quit)
            }
            "help" => {
                no_args()?;
                Ok(Self::Help)
            }
            "login" => {
                no_args()?;
                Ok(Self::Login)
            }
            "logout" => {
                no_args()?;
                Ok(Self::Logout)
            }
            "sign" => {
                no_args()?;
                Ok(Self::Sign)
            }
            "save" => {
                no_args()?;
                Ok(Self::Save)
            }
            "signin" => {
                let (account, password) = required_pair("账号", "密码")?;
                Ok(Self::Signin { account, password })
            }
            "sms" => Ok(Self::Sms(required("手机号")?)),
            "smslogin" => {
                let (phone, code) = required_pair("手机号", "验证码")?;
                Ok(Self::SmsLogin { phone, code })
            }
            "simi" | "similar" => Ok(Self::Simi),
            "simiplaylist" | "songlists" => Ok(Self::SimiPlaylists),
            "fm" => Ok(Self::Fm),
            "fmtrash" | "fm-trash" => Ok(Self::FmTrash),
            "theme" => Ok(Self::Theme(optional_argument(&args)?)),
            "lyrics" => Ok(Self::LyricStyle(optional_argument(&args)?)),
            "progress" => Ok(Self::ProgressStyle(optional_argument(&args)?)),
            "volume" => Ok(Self::Volume(required("音量")?)),
            "seek" => Ok(Self::Seek(required("跳转位置")?)),
            "visualizer" => Ok(Self::Visualizer(optional_on_off(args.first().copied())?)),
            "border" => Ok(Self::Border),
            "navpos" => Ok(Self::NavPos),
            "translation" | "trans" => {
                Ok(Self::Translation(optional_on_off(args.first().copied())?))
            }
            "spin" => Ok(Self::Spin(optional_on_off(args.first().copied())?)),
            "saveonplay" => Ok(Self::SaveOnPlay(optional_on_off(args.first().copied())?)),
            "notify" => match args.as_slice() {
                [] => Err("`notify` 需要一个开关（song_change / errors）".to_string()),
                [which, rest @ ..] => {
                    let which = NotifySwitch::parse(which)?;
                    match rest {
                        [] => Ok(Self::Notify { which, on: None }),
                        [value] => Ok(Self::Notify {
                            which,
                            on: Some(parse_on_off(Some(value))?),
                        }),
                        _ => Err("`notify` 只接受一个开关与 on/off".to_string()),
                    }
                }
            },
            "mouse" => Ok(Self::Mouse(optional_on_off(args.first().copied())?)),
            "cursor" => Ok(Self::Cursor(optional_from(
                &args,
                &CURSOR_STYLES,
                "光标形状",
            )?)),
            "lyricgradient" => Ok(Self::LyricGradient(optional_from(
                &args,
                &GRADIENTS,
                "歌词渐变",
            )?)),
            "pitch" => Ok(Self::Pitch(optional_on_off(args.first().copied())?)),
            "settings" => Ok(Self::Settings),
            "messages" => Ok(Self::Messages),
            "tasks" => Ok(Self::Tasks),
            "clear" => Ok(Self::ClearQueue),
            "pane" => match args.as_slice() {
                [] => Err(format!(
                    "`pane` 需要一个面板（{}）",
                    Pane::names().join(" / ")
                )),
                [name, rest @ ..] => match rest {
                    [] => Ok(Self::Pane {
                        name: (*name).to_string(),
                        action: None,
                    }),
                    [action] => Ok(Self::Pane {
                        name: (*name).to_string(),
                        action: Some((*action).to_string()),
                    }),
                    _ => Err("`pane` 只接受一个面板与 on/off/toggle".to_string()),
                },
            },
            "layout" => Ok(Self::Layout(optional_argument(&args)?)),
            other => Err(format!("未知命令: {other}")),
        }
    }
}

/// The single optional argument of a value-taking command.
///
/// No argument means the bare form, which cycles to the next value; more than one is an error
/// rather than a silently ignored tail.
fn optional_argument(args: &[&str]) -> Result<Option<String>, String> {
    match args {
        [] => Ok(None),
        [one] => Ok(Some((*one).to_string())),
        _ => Err("只接受一个参数".to_string()),
    }
}

/// Like [`parse_on_off`], but no argument means "toggle" rather than an error.
fn optional_on_off(argument: Option<&str>) -> Result<Option<bool>, String> {
    match argument {
        Some(_) => parse_on_off(argument).map(Some),
        None => Ok(None),
    }
}

fn parse_on_off(argument: Option<&str>) -> Result<bool, String> {
    match argument {
        Some("on" | "true" | "1") => Ok(true),
        Some("off" | "false" | "0") => Ok(false),
        Some(other) => Err(format!("需要 on/off，得到 `{other}`")),
        None => Err("需要 on/off".to_string()),
    }
}

/// The argument of a command whose values are a fixed list: `None` for the bare form, which
/// cycles; one name; and an error that lists the alternatives, since the command line is the
/// only place a user can find them out.
fn optional_from<T: Copy>(
    args: &[&str],
    pool: &[(&'static str, T)],
    what: &str,
) -> Result<Option<T>, String> {
    match args {
        [] => Ok(None),
        [given] => pool
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(given))
            .map(|(_, value)| *value)
            .map(Some)
            .ok_or_else(|| {
                let known: Vec<&str> = pool.iter().map(|(name, _)| *name).collect();
                format!("未知{what}: {given}（可用: {}）", known.join(" / "))
            }),
        _ => Err(format!("`{what}` 只接受一个参数")),
    }
}

/// One switch of `[notify]`: which events raise a terminal notification.
///
/// It is a type rather than two commands because the two switches share everything but the
/// field they write — `:notify song_change on` and `:notify errors on` are one arm of the
/// match, and a third event would be one more variant plus one more table entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NotifySwitch {
    /// Announce the song that starts playing.
    SongChange,
    /// Announce playback failures.
    Errors,
}

impl NotifySwitch {
    /// Every switch, in the order the command line offers them.
    const ALL: [Self; 2] = [Self::SongChange, Self::Errors];

    /// The config key, which is also the word on the command line.
    fn name(self) -> &'static str {
        match self {
            Self::SongChange => "song_change",
            Self::Errors => "errors",
        }
    }

    /// What the toast calls this switch.
    fn label(self) -> &'static str {
        match self {
            Self::SongChange => "切歌通知",
            Self::Errors => "出错通知",
        }
    }

    /// Read the word after `:notify`. The underscore is optional, so the config's own key and
    /// the squashed spelling command names use both work.
    fn parse(word: &str) -> Result<Self, String> {
        let squashed = |name: &str| name.replace('_', "").to_ascii_lowercase();
        let given = squashed(word);
        Self::ALL
            .into_iter()
            .find(|switch| squashed(switch.name()) == given)
            .ok_or_else(|| {
                let known: Vec<&str> = Self::ALL.iter().map(|switch| switch.name()).collect();
                format!("未知通知开关: {word}（可用: {}）", known.join(" / "))
            })
    }

    /// What the config holds for this switch now.
    fn get(self, notify: &NotifyConfig) -> bool {
        match self {
            Self::SongChange => notify.song_change,
            Self::Errors => notify.errors,
        }
    }

    /// Flip it.
    fn set(self, notify: &mut NotifyConfig, on: bool) {
        match self {
            Self::SongChange => notify.song_change = on,
            Self::Errors => notify.errors = on,
        }
    }
}

/// The cursor shapes `:cursor` switches between, under the names the config writes them as.
///
/// [`CursorStyle`] knows the escape sequence it becomes but neither its name nor how to parse
/// one, so this table carries both directions; a test checks the names against serde, which is
/// what reads them out of the config file.
const CURSOR_STYLES: [(&str, CursorStyle); 4] = [
    ("default", CursorStyle::Default),
    ("block", CursorStyle::Block),
    ("underline", CursorStyle::Underline),
    ("bar", CursorStyle::Bar),
];

/// The lyric gradient presets `:lyricgradient` switches between, under the names the config
/// writes them as, in the order the bare command cycles them.
///
/// The same reason as [`CURSOR_STYLES`]: [`GradientPreset`] parses a name but cannot say its
/// own.
const GRADIENTS: [(&str, GradientPreset); 6] = [
    ("rainbow", GradientPreset::Rainbow),
    ("warm", GradientPreset::Warm),
    ("cubehelix", GradientPreset::Cubehelix),
    ("turbo", GradientPreset::Turbo),
    ("spectral", GradientPreset::Spectral),
    ("viridis", GradientPreset::Viridis),
];

/// The name of a value from a fixed list, for the toast. Every value on offer is in the table,
/// which is what the `expect` rests on.
fn pool_name<T: PartialEq>(pool: &[(&'static str, T)], value: T) -> &'static str {
    pool.iter()
        .find(|(_, candidate)| *candidate == value)
        .map(|(name, _)| *name)
        .expect("every value of a command line list is in its table")
}

/// The next value of a fixed list, wrapping — what the bare form of a cycling command does.
fn next_in<T: Copy + PartialEq>(pool: &[(&'static str, T)], current: T) -> T {
    let index = pool
        .iter()
        .position(|(_, candidate)| *candidate == current)
        .unwrap_or(0);
    pool[(index + 1) % pool.len()].1
}

/// Turn save-on-play on or off in `config`; `None` toggles it, like the other switches.
///
/// The engine keeps its own copy of this one — see the `:saveonplay` arm, which pushes the new
/// value into it — so the two halves are deliberately separate functions.
fn set_save_on_play(config: &mut Config, on: Option<bool>) -> bool {
    let on = on.unwrap_or(!config.cache.save_on_play);
    config.cache.save_on_play = on;
    on
}

/// Turn one `[notify]` switch on or off in `config`, answering with the state it now holds.
///
/// Reading the current value here is what makes the bare form a toggle, the way `:visualizer`
/// is one — the caller passes `None` for it.
fn set_notify_switch(config: &mut Config, which: NotifySwitch, on: Option<bool>) -> bool {
    let on = on.unwrap_or(!which.get(&config.notify));
    which.set(&mut config.notify, on);
    on
}

/// Capture or release the mouse, in `config` and in the terminal.
///
/// Both halves are needed and neither is enough: the config is what the next start reads, and
/// the escape sequence is what gives the terminal's own text selection back *now* — which is
/// the whole point of the switch. The sequence is the one the startup path sends, so switching
/// here and restarting reach the terminal as the same bytes.
fn set_mouse_capture<W: Write>(
    config: &mut Config,
    out: &mut W,
    on: Option<bool>,
) -> Result<bool, String> {
    let on = on.unwrap_or(!config.mouse);
    config.mouse = on;

    let sent = if on {
        execute!(out, EnableMouseCapture)
    } else {
        execute!(out, DisableMouseCapture)
    };
    sent.map_err(|error| format!("切换鼠标捕获失败: {error}"))?;

    Ok(on)
}

/// Ask the terminal for a cursor shape, and remember it.
///
/// The shape only matters while an input field has focus and needs no new frame to appear, so
/// it is written out rather than queued into the frame the way a widget's cells are.
fn set_cursor_style<W: Write>(
    config: &mut Config,
    out: &mut W,
    shape: Option<CursorStyle>,
) -> Result<CursorStyle, String> {
    let shape = shape.unwrap_or_else(|| next_in(&CURSOR_STYLES, config.cursor_style));
    config.cursor_style = shape;

    execute!(out, shape.command()).map_err(|error| format!("切换光标形状失败: {error}"))?;

    Ok(shape)
}

/// The lyrics gradient to use: the one asked for, or the next preset for the bare form.
fn set_lyric_gradient(config: &mut Config, preset: Option<GradientPreset>) -> GradientPreset {
    let preset = preset.unwrap_or_else(|| next_in(&GRADIENTS, config.lyric_gradient));
    config.lyric_gradient = preset;
    preset
}

/// Split a command line into the part already fixed plus the word being typed:
/// `("theme ", "dra")` for `theme dra`, `("", "th")` while the name is still open.
fn split_head(line: &str) -> (String, &str) {
    match line.split_once(char::is_whitespace) {
        Some((name, rest)) => (format!("{name} "), rest.trim_start()),
        None => (String::new(), line.trim_start()),
    }
}

/// Completions for the word being typed, from the command table: the command names, and then, for
/// a command that takes values, the pool its own row carries ([`crate::state::command::ArgsPool`]).
///
/// Those pools used to be a second table here, so a command's options lived in two places and a new
/// preset had to be added to both. They live on the row now, the way Helix keeps each command's
/// completer beside the command.
fn candidate_names(head: &str, typed: &str, themes: &[&str]) -> Vec<String> {
    let name = head.trim();
    let pool: Vec<String> = if name.is_empty() {
        crate::state::command_names()
            .iter()
            .map(|name| (*name).to_string())
            .collect()
    } else {
        let themes: Vec<String> = themes.iter().map(|theme| (*theme).to_string()).collect();
        crate::state::command::args_for(name, &themes)
    };

    pool.into_iter()
        .filter(|candidate| candidate.starts_with(typed))
        .collect()
}

/// Completed command line for `Tab`, or `None` when there is nothing to add.
fn completion(line: &str, themes: &[&str]) -> Option<String> {
    let (head, typed) = split_head(line);
    let candidates = candidate_names(&head, typed, themes);

    match candidates.as_slice() {
        [] => None,
        [only] => Some(format!("{head}{only} ")),
        many => {
            // Ambiguous: extend to the longest common prefix, as a shell would.
            let mut prefix = many[0].clone();
            for candidate in many {
                while !candidate.starts_with(&prefix) {
                    prefix.pop();
                }
            }
            (prefix.len() > typed.len()).then_some(format!("{head}{prefix}"))
        }
    }
}

/// Open the prompt on an empty line.
pub(crate) fn open(app: &mut App) {
    app.state.prompt.active = true;
    app.state.prompt.input = TextInput::new();
    app.state.prompt.history_index = None;
}

/// Handle a key while the prompt is open; returns whether the key was consumed.
pub(super) fn handle_ex_key(app: &mut App, key_event: KeyPress) -> bool {
    if !app.state.prompt.active {
        return false;
    }

    match key_event.code {
        KeyCode::Esc => close(app),
        KeyCode::Enter => {
            let line = app.state.prompt.input.value.trim().to_string();
            app.state.prompt.remember(&line);
            close(app);
            if !line.is_empty() {
                run(app, &line);
            }
        }
        KeyCode::Up => app.state.prompt.recall_previous(),
        KeyCode::Down => app.state.prompt.recall_next(),
        KeyCode::Backspace => app.state.prompt.input.delete_char(),
        KeyCode::Left => app.state.prompt.input.move_left(),
        KeyCode::Right => app.state.prompt.input.move_right(),
        KeyCode::Tab => {
            // Theme names come from the live registry, so `:theme <Tab>` offers exactly the
            // themes this build can load, custom ones included, plus `random`.
            let themes = app.theme_registry.choosable_names();

            match completion(&app.state.prompt.input.value, &themes) {
                Some(completed) => {
                    let mut input = TextInput::new();
                    for ch in completed.chars() {
                        input.enter_char(ch);
                    }
                    app.state.prompt.input = input;
                }
                None => {
                    let (head, typed) = split_head(&app.state.prompt.input.value);
                    let matches = candidate_names(&head, typed, &themes);
                    if !matches.is_empty() {
                        app.toast(matches.join(" "));
                    }
                }
            }
        }
        KeyCode::Char(ch) => {
            app.state.prompt.leave_history();
            app.state.prompt.input.enter_char(ch);
        }
        _ => {}
    }

    true
}

fn close(app: &mut App) {
    app.state.prompt.active = false;
    app.state.prompt.input = TextInput::new();
}

/// Run a command line, reporting the outcome the way vim reports errors.
/// `:simi` and `:simiplaylist`: the neighbourhood of what is playing. Both are about a song,
/// so with nothing playing there is nothing to ask about — a message, not a silent no-op.
fn open_song_neighbourhood(app: &mut App, similar: bool) {
    let Some(song) = app.playback.current_song() else {
        app.notice(
            crate::state::notices::Level::Warn,
            "没有正在播放的歌曲".to_string(),
        );
        return;
    };
    app.state.events.send(NavigationEvent::OpenSongContext {
        song_id: song.id,
        similar,
    });
}

/// `:fm`: fetch a page of the private FM and play it. The fetch is off the main thread and the
/// playing is not, which is why the songs travel back as an event.
fn play_private_fm(app: &mut App) {
    let service = app.service.clone();
    let sender = app.state.events.sender();
    tokio::spawn(async move {
        match service.personal_fm().await {
            Ok(songs) if !songs.is_empty() => {
                let _ = sender.send(
                    PlaybackEvent::PlaySongs {
                        key: "私人FM".to_string(),
                        songs,
                        index: 0,
                    }
                    .into(),
                );
            }
            // An empty FM is the service's answer, not a failure: nothing to say about it.
            Ok(_) => {}
            Err(error) => {
                let _ = sender.send(AppEvent::Toast(format!("私人FM: {error}")).into());
            }
        }
    });
}

/// `:fmtrash`: stop the FM from offering the song it just played again.
fn trash_current_fm_song(app: &mut App) {
    let Some(song) = app.playback.current_song() else {
        app.notice(
            crate::state::notices::Level::Warn,
            "没有正在播放的歌曲".to_string(),
        );
        return;
    };
    let service = app.service.clone();
    let sender = app.state.events.sender();
    let name = song.name.clone();
    tokio::spawn(async move {
        match service.fm_trash(song.id).await {
            Ok(()) => {
                let _ = sender.send(AppEvent::Toast(format!("已从私人FM移除: {name}")).into());
            }
            Err(error) => {
                let _ = sender.send(AppEvent::Toast(format!("私人FM 移除失败: {error}")).into());
            }
        }
    });
}

fn run(app: &mut App, line: &str) {
    match ExCommand::parse(line) {
        Err(error) => app.notice(crate::state::notices::Level::Error, format!("E: {error}")),
        Ok(command) => {
            if let Err(error) = execute(app, command) {
                app.notice(crate::state::notices::Level::Error, format!("E: {error}"));
            }
        }
    }
}

pub(crate) fn execute(app: &mut App, command: ExCommand) -> Result<(), String> {
    match command {
        ExCommand::Simi => open_song_neighbourhood(app, true),
        ExCommand::SimiPlaylists => open_song_neighbourhood(app, false),
        ExCommand::Fm => play_private_fm(app),
        ExCommand::FmTrash => trash_current_fm_song(app),
        ExCommand::Quit => {
            app.state.events.send(AppEvent::Quit);
        }
        ExCommand::Help => app.state.help.toggle(),
        ExCommand::Login => {
            // The page's own entry: no prefill, and the tab back on the QR code.
            app.state.login.open(LoginMethod::Qr);
            app.state
                .events
                .send(NavigationEvent::Navigate(Page::Login));
        }
        ExCommand::Save => {
            app.config.save();
            app.toast("已保存配置".to_string());
        }
        ExCommand::Border => {
            app.state.border.enabled = !app.state.border.enabled;
            app.toast(format!(
                "边框模式: {}",
                if app.state.border.enabled {
                    "ON"
                } else {
                    "OFF"
                }
            ));
        }
        ExCommand::NavPos => app.cycle_nav_position(),
        ExCommand::SaveOnPlay(on) => {
            let on = set_save_on_play(&mut app.config, on);
            // The engine read this setting when it was built, so it holds its own copy: without
            // this the toast would announce a change that only a restart makes real.
            app.playback.set_save_on_play(on);
            app.config.save();
            app.toast(format!("边听边存: {}", if on { "ON" } else { "OFF" }));
        }
        ExCommand::Notify { which, on } => {
            let on = set_notify_switch(&mut app.config, which, on);
            app.config.save();
            app.toast(format!(
                "{}: {}",
                which.label(),
                if on { "ON" } else { "OFF" }
            ));
        }
        ExCommand::Mouse(on) => {
            let on = set_mouse_capture(&mut app.config, &mut io::stdout(), on)?;
            app.config.save();
            app.toast(format!("鼠标捕获: {}", if on { "ON" } else { "OFF" }));
        }
        ExCommand::Cursor(shape) => {
            let shape = set_cursor_style(&mut app.config, &mut io::stdout(), shape)?;
            app.config.save();
            app.toast(format!("光标形状: {}", pool_name(&CURSOR_STYLES, shape)));
        }
        ExCommand::LyricGradient(preset) => {
            let preset = set_lyric_gradient(&mut app.config, preset);
            app.config.save();
            app.toast(format!("歌词渐变: {}", pool_name(&GRADIENTS, preset)));
        }
        ExCommand::Theme(None) => {
            let names = app.theme_registry.choosable_names();
            let current = names
                .iter()
                .position(|name| *name == app.config.default_theme)
                .unwrap_or(0);
            let requested = names[(current + 1) % names.len().max(1)];
            let next = app.theme_registry.concrete_name(requested);
            app.config.default_theme = next.clone();
            app.config.save();
            app.toast(format!("主题: {next}"));
        }
        ExCommand::Theme(Some(name)) => {
            // `random` is rolled now and the roll is what gets written: a *standing* random
            // theme is `random` in the config file, and typing it again re-rolls.
            let name = app.theme_registry.concrete_name(&name);
            app.config.default_theme = name.clone();
            app.config.save();
            app.toast(format!("主题: {name}"));
        }
        ExCommand::LyricStyle(None) => {
            // Bare, it cycles — the same shape as `:visualizer`, `:pitch` and the keys that do
            // the same thing. The order is `LyricStyle::ALL`'s, so a new style cannot be added
            // and then silently skipped here.
            let next = app.config.lyric_style.next();
            app.config.lyric_style = next;
            app.config.save();
            app.toast(format!("歌词样式: {} — {}", next.name(), next.describe()));
        }
        ExCommand::LyricStyle(Some(name)) => {
            let Some(style) = LyricStyle::parse(&name) else {
                let known: Vec<&str> = LyricStyle::ALL.iter().map(|s| s.name()).collect();
                return Err(format!(
                    "未知歌词样式: {name}（可用: {}）",
                    known.join(" / ")
                ));
            };
            app.config.lyric_style = style;
            app.config.save();
            app.toast(format!("歌词样式: {} — {}", style.name(), style.describe()));
        }
        ExCommand::Settings => {
            app.state.navigation.page = Page::Settings;
        }
        ExCommand::Messages => app.state.messages.toggle(),
        ExCommand::Tasks => app.state.tasks_popup.toggle(),
        ExCommand::ClearQueue => {
            app.playback.clear_queue();
            app.toast(format!(
                " {}  已清空播放队列",
                crate::config::symbols().queue_clear
            ));

            // The queue page is now showing tabs the queue no longer has: focus the one that is
            // left rather than a tab that has gone.
            if app.state.navigation.page == Page::Playlist {
                if let Some(key) = app.playback.switch_queue(false) {
                    app.state.navigation.playlist_selected =
                        app.playback.queue_current_index().unwrap_or(0);
                    app.toast(format!("▣ 队列: {key}"));
                } else if let Some(key) = app.playback.queue_keys().last().cloned() {
                    app.playback.activate_queue(&key);
                    app.state.navigation.playlist_selected =
                        app.playback.queue_current_index().unwrap_or(0);
                    app.toast(format!("▣ 队列: {key}"));
                }
            }
        }
        ExCommand::ProgressStyle(None) => {
            // Bare, it cycles — the same shape as the bare `:lyrics`.
            let next = app.config.playerbar.progress_style.next();
            app.config.playerbar.progress_style = next;
            app.config.save();
            app.toast(format!("进度条样式: {} — {}", next.name(), next.describe()));
        }
        ExCommand::ProgressStyle(Some(name)) => {
            let Some(style) = ProgressStyle::parse(&name) else {
                let known: Vec<&str> = ProgressStyle::ALL.iter().map(|s| s.name()).collect();
                return Err(format!(
                    "未知进度条样式: {name}（可用: {}）",
                    known.join(" / ")
                ));
            };
            app.config.playerbar.progress_style = style;
            app.config.save();
            app.toast(format!(
                "进度条样式: {} — {}",
                style.name(),
                style.describe()
            ));
        }
        ExCommand::Volume(value) => match parse_volume(&value) {
            Err(error) => return Err(error.to_string()),
            Ok(MsgAction::Volume { absolute, delta }) => {
                let volume = match (absolute, delta) {
                    (Some(absolute), _) => absolute,
                    (None, Some(delta)) => app.playback.state.volume + delta,
                    (None, None) => return Err("音量参数无效".to_string()),
                };
                app.playback.set_volume(volume.clamp(0.0, 1.0));
            }
            Ok(_) => return Err("音量参数无效".to_string()),
        },
        ExCommand::Seek(value) => seek(app, &value)?,
        ExCommand::Visualizer(on) => {
            let on = on.unwrap_or(!app.config.playerbar.visible.visualizer);
            app.set_visualizer(on);
        }
        ExCommand::Translation(on) => {
            let on = on.unwrap_or(!app.config.lyric_translation);
            app.set_lyric_translation(on);
        }
        ExCommand::Spin(on) => {
            let on = on.unwrap_or(!app.config.playerbar.spinning_cover);
            app.set_spinning_cover(on);
        }
        ExCommand::Pitch(on) => {
            let on = on.unwrap_or(!app.config.playerbar.visible.pitch);
            app.set_pitch(on);
        }
        ExCommand::Pane { name, action } => {
            let Some(pane) = Pane::parse(&name) else {
                let known: Vec<&str> = Pane::names();
                return Err(format!("未知面板: {name}（可用: {}）", known.join(" / ")));
            };
            match action.as_deref() {
                None | Some("toggle") => app.config.panes.toggle(pane),
                Some("on") => app.config.panes.restore(pane),
                Some("off") => app.config.panes.collapse(pane),
                Some(other) => {
                    return Err(format!("未知动作: {other}（可用: on / off / toggle）"));
                }
            }
            app.config.save();

            let size = app.config.panes.size(pane);
            let state = if app.config.panes.visible(pane) {
                match pane {
                    Pane::Navigation | Pane::Mv => format!("{size} 列"),
                    _ => format!("{size} 行"),
                }
            } else {
                "关".to_string()
            };
            app.toast(format!("{}: {}", pane.describe(), state));
        }
        ExCommand::Layout(None) => {
            let next = match app.config.playerbar.layout {
                crate::config::LayoutType::Default => "modern",
                crate::config::LayoutType::Modern => "minimal",
                crate::config::LayoutType::Minimal => "default",
            };
            app.set_playerbar_layout(next)?;
        }
        ExCommand::Layout(Some(name)) => app.set_playerbar_layout(&name)?,
        ExCommand::Logout => {
            let service = app.service.clone();
            let sender = app.state.events.sender();
            app.toast("正在退出登录…".to_string());
            tokio::spawn(async move {
                if let Err(error) = service.logout().await {
                    log::warn!("logout: {error}");
                }
                let _ = sender.send(AuthEvent::LoggedOut.into());
            });
        }
        ExCommand::Sign => {
            app.state.login.open(LoginMethod::Sign);
            app.state
                .events
                .send(NavigationEvent::Navigate(Page::Login));
        }
        ExCommand::Signin { account, password } => {
            app.state.login.open_password(&account, &password);
            app.state
                .events
                .send(NavigationEvent::Navigate(Page::Login));
        }
        ExCommand::Sms(phone) => {
            app.state.login.open_sms(&phone);
            app.state
                .events
                .send(NavigationEvent::Navigate(Page::Login));
        }
        ExCommand::SmsLogin { phone, code } => {
            app.state.login.open_sms_login(&phone, &code);
            app.state
                .events
                .send(NavigationEvent::Navigate(Page::Login));
        }
    }
    Ok(())
}

/// `:seek +15` / `:seek -30` move relative to the position, `:seek 90` jumps to a second,
/// and `:seek 50%` to a fraction of the track.
fn seek(app: &mut App, value: &str) -> Result<(), String> {
    match parse_seek(value)? {
        SeekTarget::Fraction(fraction) => {
            app.playback.seek_to_fraction(fraction);
            Ok(())
        }
        SeekTarget::Relative(delta) => {
            app.playback.seek_relative(delta);
            Ok(())
        }
        SeekTarget::Absolute(seconds) => {
            let total_secs = app
                .playback
                .state
                .current_song
                .as_ref()
                .map(|song| song.duration as f64 / 1000.0)
                .filter(|total| *total > 0.0)
                .ok_or_else(|| "当前没有可跳转的歌曲".to_string())?;
            app.playback
                .seek_to_fraction((seconds / total_secs).clamp(0.0, 1.0));
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_parse_into_their_arguments() {
        assert_eq!(ExCommand::parse("simi"), Ok(ExCommand::Simi));
        assert_eq!(ExCommand::parse("similar"), Ok(ExCommand::Simi));
        assert_eq!(ExCommand::parse("songlists"), Ok(ExCommand::SimiPlaylists));
        assert_eq!(ExCommand::parse("fm"), Ok(ExCommand::Fm));
        assert_eq!(ExCommand::parse("fm-trash"), Ok(ExCommand::FmTrash));
        assert_eq!(ExCommand::parse("q"), Ok(ExCommand::Quit));
        assert_eq!(ExCommand::parse("quit"), Ok(ExCommand::Quit));
        assert_eq!(ExCommand::parse("save"), Ok(ExCommand::Save));
        assert_eq!(
            ExCommand::parse("theme dracula"),
            Ok(ExCommand::Theme(Some("dracula".to_string())))
        );
        assert_eq!(
            ExCommand::parse("volume +5"),
            Ok(ExCommand::Volume("+5".to_string()))
        );
        assert_eq!(
            ExCommand::parse("seek 50%"),
            Ok(ExCommand::Seek("50%".to_string()))
        );
        assert_eq!(
            ExCommand::parse("visualizer on"),
            Ok(ExCommand::Visualizer(Some(true)))
        );
        assert_eq!(
            ExCommand::parse("pitch 0"),
            Ok(ExCommand::Pitch(Some(false)))
        );
        // Bare, it toggles — the same thing the `v` and `V` keys do.
        assert_eq!(
            ExCommand::parse("visualizer"),
            Ok(ExCommand::Visualizer(None))
        );
        assert_eq!(
            ExCommand::parse("spin off"),
            Ok(ExCommand::Spin(Some(false)))
        );
        assert_eq!(ExCommand::parse("spin on"), Ok(ExCommand::Spin(Some(true))));
        // Bare, it toggles — the same thing the `t` key does.
        assert_eq!(ExCommand::parse("spin"), Ok(ExCommand::Spin(None)));
        assert_eq!(
            ExCommand::parse("progress"),
            Ok(ExCommand::ProgressStyle(None))
        );
        assert_eq!(
            ExCommand::parse("progress segment"),
            Ok(ExCommand::ProgressStyle(Some("segment".to_string())))
        );
        assert_eq!(
            ExCommand::parse("translation"),
            Ok(ExCommand::Translation(None))
        );
        assert_eq!(
            ExCommand::parse("translation on"),
            Ok(ExCommand::Translation(Some(true)))
        );
        assert_eq!(
            ExCommand::parse("trans off"),
            Ok(ExCommand::Translation(Some(false)))
        );
        assert_eq!(
            ExCommand::parse("translation 也许"),
            Err("需要 on/off，得到 `也许`".to_string())
        );
        assert_eq!(ExCommand::parse("pitch"), Ok(ExCommand::Pitch(None)));
        assert_eq!(
            ExCommand::parse("layout default"),
            Ok(ExCommand::Layout(Some("default".to_string())))
        );
        assert_eq!(
            ExCommand::parse("signin someone@example.com hunter2"),
            Ok(ExCommand::Signin {
                account: "someone@example.com".to_string(),
                password: "hunter2".to_string(),
            })
        );
        assert_eq!(
            ExCommand::parse("sms 13800000000"),
            Ok(ExCommand::Sms("13800000000".to_string()))
        );
        assert_eq!(
            ExCommand::parse("smslogin 13800000000 246810"),
            Ok(ExCommand::SmsLogin {
                phone: "13800000000".to_string(),
                code: "246810".to_string(),
            })
        );
        assert_eq!(ExCommand::parse("logout"), Ok(ExCommand::Logout));
        assert_eq!(ExCommand::parse("sign"), Ok(ExCommand::Sign));
        // `:login` keeps opening the QR page; the password form is its own command
        assert_eq!(ExCommand::parse("login"), Ok(ExCommand::Login));
        // surrounding whitespace is the user's business, not an error
        assert_eq!(ExCommand::parse("  q  "), Ok(ExCommand::Quit));
    }

    /// The settings switches parse, each carrying what it was told; `None` is what the bare
    /// form leaves behind, and it is what makes the command toggle or cycle.
    #[test]
    fn settings_parse_into_their_arguments() {
        assert_eq!(
            ExCommand::parse("notify song_change on"),
            Ok(ExCommand::Notify {
                which: NotifySwitch::SongChange,
                on: Some(true),
            })
        );
        assert_eq!(
            ExCommand::parse("notify errors off"),
            Ok(ExCommand::Notify {
                which: NotifySwitch::Errors,
                on: Some(false),
            })
        );
        // the config's own key and the squashed spelling are the same switch
        assert_eq!(
            ExCommand::parse("notify songchange"),
            Ok(ExCommand::Notify {
                which: NotifySwitch::SongChange,
                on: None,
            })
        );
        assert_eq!(
            ExCommand::parse("mouse on"),
            Ok(ExCommand::Mouse(Some(true)))
        );
        assert_eq!(ExCommand::parse("mouse"), Ok(ExCommand::Mouse(None)));
        assert_eq!(
            ExCommand::parse("mouse 0"),
            Ok(ExCommand::Mouse(Some(false)))
        );
        assert_eq!(
            ExCommand::parse("cursor underline"),
            Ok(ExCommand::Cursor(Some(CursorStyle::Underline)))
        );
        assert_eq!(ExCommand::parse("cursor"), Ok(ExCommand::Cursor(None)));
        assert_eq!(
            ExCommand::parse("lyricgradient turbo"),
            Ok(ExCommand::LyricGradient(Some(GradientPreset::Turbo)))
        );
        assert_eq!(
            ExCommand::parse("lyricgradient"),
            Ok(ExCommand::LyricGradient(None))
        );
        assert_eq!(
            ExCommand::parse("saveonplay off"),
            Ok(ExCommand::SaveOnPlay(Some(false)))
        );
        assert_eq!(
            ExCommand::parse("saveonplay"),
            Ok(ExCommand::SaveOnPlay(None)),
            "bare, it toggles"
        );
    }

    /// The lists a command line offers have to be the lists the config accepts: these names are
    /// read out of `config.toml` by serde, so a rename there would otherwise leave a command
    /// that no longer writes a name the config can read back.
    #[test]
    fn the_fixed_lists_are_the_names_the_config_uses() {
        #[derive(serde::Serialize, serde::Deserialize)]
        struct Holder {
            cursor_style: CursorStyle,
            lyric_gradient: GradientPreset,
        }

        for (name, style) in CURSOR_STYLES {
            let text = toml_edit::ser::to_string(&Holder {
                cursor_style: style,
                lyric_gradient: GradientPreset::default(),
            })
            .expect("serialize");
            let back: Holder = toml_edit::de::from_str(&text).expect("deserialize");
            assert_eq!(back.cursor_style, style, "{name}: {text}");
            assert!(
                text.contains(&format!("cursor_style = \"{name}\"")),
                "{text}"
            );
        }

        for (name, preset) in GRADIENTS {
            let text = toml_edit::ser::to_string(&Holder {
                cursor_style: CursorStyle::default(),
                lyric_gradient: preset,
            })
            .expect("serialize");
            let back: Holder = toml_edit::de::from_str(&text).expect("deserialize");
            assert_eq!(back.lyric_gradient, preset, "{name}: {text}");
            assert!(
                text.contains(&format!("lyric_gradient = \"{name}\"")),
                "{text}"
            );
        }
    }

    /// The bytes crossterm sends for the two terminal modes, spelled out rather than asked of
    /// crossterm: what has to hold is that the *terminal* was told, so the assertion is on the
    /// sequence a terminal reads.
    const ENABLE_MOUSE: &[u8] = b"\x1b[?1000h\x1b[?1002h\x1b[?1003h\x1b[?1015h\x1b[?1006h";
    const DISABLE_MOUSE: &[u8] = b"\x1b[?1006l\x1b[?1015l\x1b[?1003l\x1b[?1002l\x1b[?1000l";

    /// The mouse switch is the one that has to act on the terminal, not only on the config: it
    /// is what gives the terminal's own text selection back while the app runs.
    #[test]
    fn the_mouse_switch_reaches_the_config_and_the_terminal() {
        let mut config = Config::default();
        assert!(config.mouse, "the default captures the mouse");

        let mut out = Vec::new();
        assert!(
            !set_mouse_capture(&mut config, &mut out, Some(false)).expect("write"),
            "off means off"
        );
        assert!(!config.mouse);
        assert_eq!(out, DISABLE_MOUSE);

        out.clear();
        assert!(
            set_mouse_capture(&mut config, &mut out, None).expect("write"),
            "bare, it toggles"
        );
        assert!(config.mouse);
        assert_eq!(out, ENABLE_MOUSE);
    }

    /// Same for the cursor: the config keeps the shape, and the terminal is asked for it now.
    #[test]
    fn the_cursor_switch_reaches_the_config_and_the_terminal() {
        let mut config = Config::default();
        let mut out = Vec::new();

        assert_eq!(
            set_cursor_style(&mut config, &mut out, None).expect("write"),
            CursorStyle::Block,
            "bare, it cycles from the default"
        );
        assert_eq!(config.cursor_style, CursorStyle::Block);
        assert_eq!(out, b"\x1b[2 q");

        out.clear();
        assert_eq!(
            set_cursor_style(&mut config, &mut out, Some(CursorStyle::Bar)).expect("write"),
            CursorStyle::Bar
        );
        assert_eq!(config.cursor_style, CursorStyle::Bar);
        assert_eq!(out, b"\x1b[6 q");
    }

    /// Every switch a consumer observes without a terminal: the config field the rest of the app
    /// reads (the lyrics ones on each frame, the notify ones when an event fires, save-on-play
    /// when the next track resolves).
    #[test]
    fn the_setting_switches_reach_the_config() {
        let mut config = Config::default();

        assert!(set_notify_switch(
            &mut config,
            NotifySwitch::SongChange,
            Some(true)
        ));
        assert!(config.notify.song_change);
        assert!(
            !set_notify_switch(&mut config, NotifySwitch::SongChange, None),
            "bare, it toggles"
        );
        assert!(!config.notify.song_change);

        assert!(set_notify_switch(
            &mut config,
            NotifySwitch::Errors,
            Some(true)
        ));
        assert!(
            config.notify.errors && !config.notify.song_change,
            "one switch does not move the other"
        );

        assert!(config.cache.save_on_play, "the default saves while playing");
        assert!(!set_save_on_play(&mut config, Some(false)));
        assert!(!config.cache.save_on_play);
        assert!(set_save_on_play(&mut config, None), "bare, it toggles");
        assert!(config.cache.save_on_play);

        assert_eq!(
            set_lyric_gradient(&mut config, Some(GradientPreset::Spectral)),
            GradientPreset::Spectral
        );
        assert_eq!(config.lyric_gradient, GradientPreset::Spectral);
        assert_eq!(
            set_lyric_gradient(&mut config, None),
            GradientPreset::Viridis,
            "bare, it cycles"
        );
        assert_eq!(config.lyric_gradient, GradientPreset::Viridis);
    }

    /// Each failure must explain itself: the toast is the only feedback there is.
    #[test]
    fn bad_lines_report_why() {
        let unknown = ExCommand::parse("frobnicate").unwrap_err();
        assert!(unknown.contains("frobnicate"), "got {unknown}");

        assert_eq!(
            ExCommand::parse("theme"),
            Ok(ExCommand::Theme(None)),
            "bare, it cycles to the next theme"
        );

        let extra = ExCommand::parse("theme a b").unwrap_err();
        assert!(extra.contains("只接受一个参数"), "got {extra}");

        let bad_flag = ExCommand::parse("pitch maybe").unwrap_err();
        assert!(bad_flag.contains("on/off"), "got {bad_flag}");

        // Bare `:layout` cycles now; too many arguments is the error left to catch.
        let bad_layout = ExCommand::parse("layout a b").unwrap_err();
        assert!(bad_layout.contains("只接受一个参数"), "got {bad_layout}");

        for (line, missing) in [
            ("signin someone@example.com", "密码"),
            ("signin a b c", "只接受两个参数"),
            ("smslogin 13800000000", "验证码"),
            ("sms", "手机号"),
            ("logout now", "不接受参数"),
            // the settings: an unknown value has to list the ones that exist
            ("notify", "song_change"),
            ("notify nope on", "未知通知开关"),
            ("notify song_change maybe", "on/off"),
            ("notify song_change on extra", "只接受一个开关"),
            ("mouse maybe", "on/off"),
            ("cursor sideways", "未知光标形状"),
            ("cursor block bar", "只接受一个参数"),
            ("lyricgradient neon", "未知歌词渐变"),
            ("saveonplay maybe", "on/off"),
        ] {
            let error = ExCommand::parse(line).unwrap_err();
            assert!(error.contains(missing), "{line}: got {error}");
        }

        assert!(ExCommand::parse("   ").is_err());
    }

    /// Theme names the completion pool is fed from in these tests.
    const THEMES: [&str; 3] = ["catppuccin_mocha", "default", "dracula"];

    #[test]
    fn tab_completes_unambiguous_names() {
        assert_eq!(completion("h", &THEMES), Some("help ".to_string()));
        assert_eq!(completion("vol", &THEMES), Some("volume ".to_string()));
        // nothing to add once the name is complete
        assert_eq!(completion("help ", &THEMES), None);
        // an unknown name has no candidates
        assert_eq!(completion("zzz", &THEMES), None);
    }

    #[test]
    fn tab_extends_to_the_common_prefix_when_ambiguous() {
        // `sig` could be sign or signin, so it extends to what the two share
        assert_eq!(candidate_names("", "sig", &THEMES), vec!["sign", "signin"]);
        assert_eq!(completion("sig", &THEMES), Some("sign".to_string()));
        // `s` is the start of several commands, so there is nothing to add
        assert_eq!(completion("s", &THEMES), None);
        assert_eq!(
            candidate_names("", "s", &THEMES),
            vec![
                "sign",
                "simi",
                "simiplaylist",
                "spin",
                "save",
                "seek",
                "signin",
                "sms",
                "smslogin",
                "settings",
                "saveonplay"
            ]
        );
        // an empty line offers every command
        assert_eq!(
            candidate_names("", "", &THEMES).len(),
            crate::state::command_names().len()
        );
        assert_eq!(completion("", &THEMES), None);
    }

    /// `:theme` completes the names the registry actually has, and the toggles complete
    /// their own two keywords.
    #[test]
    fn arguments_complete_from_the_pool_of_their_command() {
        assert_eq!(
            completion("theme dra", &THEMES),
            Some("theme dracula ".to_string())
        );
        assert_eq!(
            completion("theme cat", &THEMES),
            Some("theme catppuccin_mocha ".to_string())
        );
        assert_eq!(completion("theme zzz", &THEMES), None);

        // every theme matches an empty argument: Tab lists them instead of completing
        assert_eq!(completion("theme ", &THEMES), None);
        assert_eq!(
            candidate_names("theme ", "", &THEMES),
            vec!["catppuccin_mocha", "default", "dracula"]
        );

        assert_eq!(
            completion("pitch of", &THEMES),
            Some("pitch off ".to_string())
        );
        // `on` shares its first letter with `off`, so prefix completion cannot reach it —
        // the same limit a shell has — but the already-complete form stays put
        assert_eq!(completion("pitch n", &THEMES), None);
        assert_eq!(
            completion("pitch on", &THEMES),
            Some("pitch on ".to_string())
        );
        assert_eq!(
            candidate_names("visualizer ", "n", &THEMES),
            Vec::<String>::new()
        );
        assert_eq!(
            candidate_names("visualizer ", "", &THEMES),
            vec!["off", "on"]
        );
        assert_eq!(
            candidate_names("saveonplay ", "", &THEMES),
            vec!["off", "on"]
        );
        assert_eq!(
            completion("mouse of", &THEMES),
            Some("mouse off ".to_string())
        );
        assert_eq!(
            candidate_names("notify ", "", &THEMES),
            vec!["song_change", "errors"]
        );
        // `notify song` is the switch being typed, not the value: it completes to the key
        assert_eq!(
            completion("notify song", &THEMES),
            Some("notify song_change ".to_string())
        );
        assert_eq!(
            candidate_names("cursor ", "", &THEMES),
            vec!["default", "block", "underline", "bar"]
        );
        assert_eq!(
            completion("cursor under", &THEMES),
            Some("cursor underline ".to_string())
        );
        assert_eq!(
            candidate_names("lyricgradient ", "", &THEMES),
            vec![
                "rainbow",
                "warm",
                "cubehelix",
                "turbo",
                "spectral",
                "viridis"
            ]
        );
        assert_eq!(
            candidate_names("layout ", "", &THEMES),
            vec!["default", "minimal", "modern"]
        );
        assert_eq!(
            completion("layout mod", &THEMES),
            Some("layout modern ".to_string())
        );

        // a command without an argument pool has nothing to offer
        assert_eq!(completion("seek 1", &THEMES), None);
        assert!(candidate_names("seek ", "1", &THEMES).is_empty());
    }
}
