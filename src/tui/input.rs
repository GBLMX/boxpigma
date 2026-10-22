//! Keyboard and mouse translation for the terminal frontend.
//!
//! Every branch returns either a semantic backend message, a local overlay
//! change, or `Quit`. It never mutates the backend directly.

use ferriso_tui::{
    Outcome,
    crossterm::event::{
        Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent,
        MouseEventKind,
    },
    ratatui::layout::Rect,
};

use crate::{
    backend::{self, CoreMessage, Page, SearchKind, View},
    state::LoginMethod,
};

use super::{
    CommandLine, Focus, Hit, SETTINGS, Tui, activate_artist, activate_content, artist_count,
    clamp_selection, content_rows, ensure_visible, hit_at, login_fields, page_for_key, seek_by,
    seek_to, toggle_play, volume_by,
};

pub(super) fn handle(tui: &mut Tui, view: &View, event: Event) -> Outcome<CoreMessage> {
    match event {
        Event::Key(event) => key(tui, view, event),
        Event::Mouse(event) => mouse(tui, view, event),
        _ => Outcome::Ignored,
    }
}

fn key(tui: &mut Tui, view: &View, key: KeyEvent) -> Outcome<CoreMessage> {
    if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
        return Outcome::Ignored;
    }

    if view.page == Page::Login {
        let method = match key.code {
            KeyCode::F(1) => Some(LoginMethod::Qr),
            KeyCode::F(2) => Some(LoginMethod::Password),
            KeyCode::F(3) => Some(LoginMethod::Sms),
            KeyCode::F(4) => Some(LoginMethod::Sign),
            KeyCode::Char('1'..='4') if key.modifiers.contains(KeyModifiers::ALT) => {
                let index = match key.code {
                    KeyCode::Char(ch) => ch as usize - '1' as usize,
                    _ => 0,
                };
                LoginMethod::ALL.get(index).copied()
            }
            _ => None,
        };
        if let Some(method) = method {
            tui.login_field = 0;
            return Outcome::Message(backend::Message::SetLoginMethod(method));
        }
    }

    // The adapter catches Ctrl+C/Ctrl+Q itself; handling them here too keeps
    // behaviour identical when a test drives the renderer directly.
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        match key.code {
            KeyCode::Char('c' | 'q') => return Outcome::Quit,
            KeyCode::Char('s') if view.page == Page::Login => {
                return Outcome::Message(backend::Message::SendSmsCode);
            }
            KeyCode::Char('1'..='4') if view.page == Page::Login => {
                if let KeyCode::Char(ch) = key.code {
                    let index = ch as usize - '1' as usize;
                    if let Some(method) = LoginMethod::ALL.get(index).copied() {
                        tui.login_field = 0;
                        return Outcome::Message(backend::Message::SetLoginMethod(method));
                    }
                }
            }
            KeyCode::Left => {
                return Outcome::Message(seek_by(view, -tui.seek_step()));
            }
            KeyCode::Right => {
                return Outcome::Message(seek_by(view, tui.seek_step()));
            }
            KeyCode::Up => return Outcome::Message(volume_by(view, 0.05)),
            KeyCode::Down => return Outcome::Message(volume_by(view, -0.05)),
            _ => {}
        }
    }

    if tui.help {
        tui.help = false;
        return Outcome::Ignored;
    }
    if tui.command.is_some() {
        return command_key(tui, view, key);
    }
    if tui.search_editing {
        return search_key(tui, view, key);
    }

    // Login fields consume printable keys before the global page shortcuts;
    // otherwise typing a password containing `q` would quit the app.
    if view.page == Page::Login
        && tui.focus == Focus::Content
        && let Some(outcome) = login_key(tui, view, key)
    {
        return outcome;
    }
    if view.page == Page::Settings
        && tui.focus == Focus::Content
        && let Some(outcome) = settings_key(tui, view, key)
    {
        return outcome;
    }

    match key.code {
        KeyCode::Char('?') => {
            tui.help = true;
            return Outcome::Ignored;
        }
        KeyCode::Char(':') => {
            tui.command = Some(CommandLine {
                text: String::new(),
                cursor: 0,
            });
            return Outcome::Ignored;
        }
        KeyCode::Char('/') => {
            begin_search(tui, view);
            return Outcome::Ignored;
        }
        KeyCode::Char('q') => return Outcome::Quit,
        KeyCode::Tab => {
            tui.focus = tui.focus.next(true);
            tui.search_editing = false;
            if tui.focus == Focus::Search {
                begin_search(tui, view);
            }
            return Outcome::Ignored;
        }
        KeyCode::BackTab => {
            tui.focus = tui.focus.next(false);
            tui.search_editing = false;
            if tui.focus == Focus::Search {
                begin_search(tui, view);
            }
            return Outcome::Ignored;
        }
        KeyCode::Esc => {
            if view.can_back || view.page != Page::Browse {
                return Outcome::Message(backend::Message::Back);
            }
            return Outcome::Ignored;
        }
        _ => {}
    }

    // Player-wide keys. They stay available from the navigation and content
    // panes, but not while a login field is taking text.
    match key.code {
        KeyCode::Char(' ') => return Outcome::Message(toggle_play()),
        KeyCode::Char('n') => return Outcome::Message(backend::Message::Next),
        KeyCode::Char('p') => return Outcome::Message(backend::Message::Prev),
        KeyCode::Char('m') => return Outcome::Message(backend::Message::CycleMode),
        KeyCode::Char('S') => return Outcome::Message(backend::Message::ToggleLike),
        KeyCode::Char('y') => return Outcome::Message(backend::Message::ToggleTranslation),
        KeyCode::Char('v') => return Outcome::Message(backend::Message::ToggleVisualizer),
        KeyCode::Char('V') => return Outcome::Message(backend::Message::TogglePitch),
        KeyCode::Char('w') => return Outcome::Message(backend::Message::ClearQueue),
        KeyCode::Char('-') => return Outcome::Message(volume_by(view, -0.05)),
        KeyCode::Char('=') => return Outcome::Message(volume_by(view, 0.05)),
        KeyCode::Char('r') => return Outcome::Message(backend::Message::Refresh),
        KeyCode::Char('[') if view.page == Page::Queue => {
            return Outcome::Message(backend::Message::SwitchQueue(false));
        }
        KeyCode::Char(']') if view.page == Page::Queue => {
            return Outcome::Message(backend::Message::SwitchQueue(true));
        }
        KeyCode::Char('[') => return Outcome::Message(seek_by(view, -tui.seek_step())),
        KeyCode::Char(']') => return Outcome::Message(seek_by(view, tui.seek_step())),
        _ => {}
    }

    if let KeyCode::Char(ch) = key.code
        && let Some(page) = page_for_key(ch)
    {
        if page == Page::Queue {
            tui.queue_selected = view.player.queue_index.unwrap_or(0);
        }
        tui.focus = Focus::Content;
        return Outcome::Message(backend::Message::Navigate(page));
    }

    match tui.focus {
        Focus::Nav => nav_key(tui, view, key),
        Focus::Content => content_key(tui, view, key),
        Focus::Search => {
            begin_search(tui, view);
            Outcome::Ignored
        }
    }
}

fn begin_search(tui: &mut Tui, view: &View) {
    tui.search = view.search.query.clone();
    tui.search_cursor = tui.search.chars().count();
    tui.search_editing = true;
    tui.focus = Focus::Search;
}

fn search_key(tui: &mut Tui, view: &View, key: KeyEvent) -> Outcome<CoreMessage> {
    match key.code {
        KeyCode::Esc => {
            tui.search_editing = false;
            tui.focus = Focus::Content;
            Outcome::Ignored
        }
        KeyCode::Enter => {
            tui.search_editing = false;
            tui.focus = Focus::Content;
            Outcome::Message(backend::Message::SearchSubmit)
        }
        KeyCode::Tab | KeyCode::BackTab => {
            let current = SearchKind::ALL
                .iter()
                .position(|kind| *kind == view.search.kind)
                .unwrap_or(0);
            let delta = if key.code == KeyCode::Tab {
                1
            } else {
                SearchKind::ALL.len() - 1
            };
            let kind = SearchKind::ALL[(current + delta) % SearchKind::ALL.len()];
            Outcome::Message(backend::Message::SetSearchKind(kind))
        }
        KeyCode::Backspace => {
            if let Some(cursor) = tui.search_cursor.checked_sub(1) {
                let mut chars: Vec<char> = tui.search.chars().collect();
                chars.remove(cursor);
                tui.search = chars.into_iter().collect();
                tui.search_cursor = cursor;
            }
            Outcome::Message(backend::Message::SearchInput(tui.search.clone()))
        }
        KeyCode::Delete => {
            let mut chars: Vec<char> = tui.search.chars().collect();
            if tui.search_cursor < chars.len() {
                chars.remove(tui.search_cursor);
                tui.search = chars.into_iter().collect();
            }
            Outcome::Message(backend::Message::SearchInput(tui.search.clone()))
        }
        KeyCode::Left => {
            tui.search_cursor = tui.search_cursor.saturating_sub(1);
            Outcome::Ignored
        }
        KeyCode::Right => {
            tui.search_cursor = (tui.search_cursor + 1).min(tui.search.chars().count());
            Outcome::Ignored
        }
        KeyCode::Home => {
            tui.search_cursor = 0;
            Outcome::Ignored
        }
        KeyCode::End => {
            tui.search_cursor = tui.search.chars().count();
            Outcome::Ignored
        }
        KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            let mut chars: Vec<char> = tui.search.chars().collect();
            let at = tui.search_cursor.min(chars.len());
            chars.insert(at, ch);
            tui.search = chars.into_iter().collect();
            tui.search_cursor = at + 1;
            Outcome::Message(backend::Message::SearchInput(tui.search.clone()))
        }
        _ => Outcome::Ignored,
    }
}

fn command_key(tui: &mut Tui, view: &View, key: KeyEvent) -> Outcome<CoreMessage> {
    let Some(command) = tui.command.as_mut() else {
        return Outcome::Ignored;
    };
    match key.code {
        KeyCode::Esc => {
            tui.command = None;
            Outcome::Ignored
        }
        KeyCode::Enter => {
            let line = command.text.trim().to_string();
            tui.command = None;
            if line.is_empty() {
                Outcome::Ignored
            } else {
                execute_command(tui, view, &line)
            }
        }
        KeyCode::Backspace => {
            if let Some(cursor) = command.cursor.checked_sub(1) {
                let mut chars: Vec<char> = command.text.chars().collect();
                chars.remove(cursor);
                command.text = chars.into_iter().collect();
                command.cursor = cursor;
            }
            Outcome::Ignored
        }
        KeyCode::Delete => {
            let mut chars: Vec<char> = command.text.chars().collect();
            if command.cursor < chars.len() {
                chars.remove(command.cursor);
                command.text = chars.into_iter().collect();
            }
            Outcome::Ignored
        }
        KeyCode::Left => {
            command.cursor = command.cursor.saturating_sub(1);
            Outcome::Ignored
        }
        KeyCode::Right => {
            command.cursor = (command.cursor + 1).min(command.text.chars().count());
            Outcome::Ignored
        }
        KeyCode::Home => {
            command.cursor = 0;
            Outcome::Ignored
        }
        KeyCode::End => {
            command.cursor = command.text.chars().count();
            Outcome::Ignored
        }
        KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            let mut chars: Vec<char> = command.text.chars().collect();
            let at = command.cursor.min(chars.len());
            chars.insert(at, ch);
            command.text = chars.into_iter().collect();
            command.cursor = at + 1;
            Outcome::Ignored
        }
        _ => Outcome::Ignored,
    }
}

fn execute_command(tui: &mut Tui, view: &View, line: &str) -> Outcome<CoreMessage> {
    let mut parts = line.split_whitespace();
    let name = parts.next().unwrap_or("").to_ascii_lowercase();
    let arg = parts.collect::<Vec<_>>().join(" ");

    match name.as_str() {
        "q" | "quit" | "exit" => Outcome::Quit,
        "help" | "h" => {
            tui.help = true;
            Outcome::Ignored
        }
        "login" => Outcome::Message(backend::Message::Navigate(Page::Login)),
        "logout" => Outcome::Message(backend::Message::Logout),
        "sign" | "signin" => Outcome::Message(backend::Message::DailySign),
        "browse" | "main" => Outcome::Message(backend::Message::Navigate(Page::Browse)),
        "queue" | "playlist" => Outcome::Message(backend::Message::Navigate(Page::Queue)),
        "lyrics" | "lyric" => Outcome::Message(backend::Message::Navigate(Page::Lyrics)),
        "settings" | "set" => Outcome::Message(backend::Message::Navigate(Page::Settings)),
        "theme" if arg.is_empty() => Outcome::Message(backend::Message::CycleTheme),
        "theme" => Outcome::Message(backend::Message::SetTheme(arg)),
        "quality" if arg.is_empty() => Outcome::Message(backend::Message::CycleQuality),
        "quality" => Outcome::Message(backend::Message::SetQuality(arg)),
        "clear" => Outcome::Message(backend::Message::ClearQueue),
        "play" | "pause" => Outcome::Message(toggle_play()),
        "next" => Outcome::Message(backend::Message::Next),
        "prev" | "previous" => Outcome::Message(backend::Message::Prev),
        "like" => Outcome::Message(backend::Message::ToggleLike),
        "refresh" => Outcome::Message(backend::Message::Refresh),
        "translation" => switch(
            tui,
            &arg,
            view.settings.lyric_translation,
            backend::Message::ToggleTranslation,
        ),
        "visualizer" => switch(
            tui,
            &arg,
            view.settings.visualizer,
            backend::Message::ToggleVisualizer,
        ),
        "pitch" => switch(
            tui,
            &arg,
            view.settings.pitch,
            backend::Message::TogglePitch,
        ),
        "notify" if arg == "song" => switch(
            tui,
            "",
            view.settings.notify_song_change,
            backend::Message::ToggleNotifySongChange,
        ),
        "notify" if arg == "errors" => switch(
            tui,
            "",
            view.settings.notify_errors,
            backend::Message::ToggleNotifyErrors,
        ),
        "save" => {
            tui.notice("配置在每次更改时自动保存");
            Outcome::Ignored
        }
        "search" if arg.is_empty() => {
            begin_search(tui, view);
            Outcome::Ignored
        }
        "search" => Outcome::Message(backend::Message::Search(arg)),
        "volume" => match arg.parse::<f64>() {
            Ok(value) => {
                Outcome::Message(backend::Message::SetVolume((value / 100.0).clamp(0.0, 1.0)))
            }
            Err(_) => {
                tui.notice("volume 需要 0-100 的数字");
                Outcome::Ignored
            }
        },
        "seek" => match seek_argument(view, &arg) {
            Some(message) => Outcome::Message(message),
            None => {
                tui.notice("seek 需要秒数、+15、-30 或 50%");
                Outcome::Ignored
            }
        },
        _ => {
            tui.notice(format!("未知命令：{line}"));
            Outcome::Ignored
        }
    }
}

/// `on`/`off` around a boolean switch. An empty argument toggles, as the
/// command table did; an argument that already matches the state does nothing.
fn switch(tui: &mut Tui, arg: &str, current: bool, message: CoreMessage) -> Outcome<CoreMessage> {
    match arg {
        "" => Outcome::Message(message),
        "on" | "true" if !current => Outcome::Message(message),
        "off" | "false" if current => Outcome::Message(message),
        "on" | "off" | "true" | "false" => Outcome::Ignored,
        _ => {
            tui.notice("开关只接受 on / off");
            Outcome::Ignored
        }
    }
}

fn seek_argument(view: &View, arg: &str) -> Option<CoreMessage> {
    let arg = arg.trim();
    if arg.is_empty() {
        return None;
    }
    if let Some(percent) = arg.strip_suffix('%') {
        let value = percent.parse::<f64>().ok()?;
        return Some(seek_to(view, value / 100.0));
    }
    if let Some(rest) = arg.strip_prefix('+') {
        return Some(seek_by(view, rest.parse::<f64>().ok()?));
    }
    if let Some(rest) = arg.strip_prefix('-') {
        return Some(seek_by(view, -rest.parse::<f64>().ok()?));
    }
    let seconds = arg.parse::<f64>().ok()?;
    let duration = view.player.duration_secs.max(0.001);
    Some(seek_to(view, seconds / duration))
}

fn login_key(tui: &mut Tui, view: &View, key: KeyEvent) -> Option<Outcome<CoreMessage>> {
    let method = view.login.method;
    let fields = login_fields(view);

    match key.code {
        KeyCode::Esc => Some(Outcome::Message(backend::Message::Back)),
        KeyCode::Tab | KeyCode::BackTab => {
            if fields.is_empty() {
                return None;
            }
            let delta = if key.code == KeyCode::Tab {
                1
            } else {
                fields.len() - 1
            };
            tui.login_field = (tui.login_field + delta) % fields.len();
            Some(Outcome::Ignored)
        }
        KeyCode::Up | KeyCode::Down => {
            if fields.is_empty() {
                return None;
            }
            let delta = if key.code == KeyCode::Down {
                1
            } else {
                fields.len() - 1
            };
            tui.login_field = (tui.login_field + delta) % fields.len();
            Some(Outcome::Ignored)
        }
        KeyCode::Enter => Some(match method {
            LoginMethod::Qr => Outcome::Message(backend::Message::RefreshQr),
            LoginMethod::Sign => Outcome::Message(backend::Message::DailySign),
            _ => Outcome::Message(backend::Message::SubmitLogin),
        }),
        KeyCode::Char('r') if method == LoginMethod::Qr => {
            Some(Outcome::Message(backend::Message::RefreshQr))
        }
        KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            let field = fields.get(tui.login_field).copied()?;
            Some(Outcome::Message(backend::Message::AppendLoginField(
                field, ch,
            )))
        }
        KeyCode::Backspace => {
            let field = fields.get(tui.login_field).copied()?;
            Some(Outcome::Message(backend::Message::BackspaceLoginField(
                field,
            )))
        }
        KeyCode::Delete => {
            let field = fields.get(tui.login_field).copied()?;
            Some(Outcome::Message(backend::Message::ClearLoginField(field)))
        }
        _ => None,
    }
}

fn settings_key(tui: &mut Tui, view: &View, key: KeyEvent) -> Option<Outcome<CoreMessage>> {
    match key.code {
        KeyCode::Up | KeyCode::Char('k') => {
            tui.settings_selected = tui.settings_selected.saturating_sub(1);
            Some(Outcome::Ignored)
        }
        KeyCode::Down | KeyCode::Char('j') => {
            tui.settings_selected = (tui.settings_selected + 1).min(SETTINGS.len() - 1);
            Some(Outcome::Ignored)
        }
        KeyCode::Home => {
            tui.settings_selected = 0;
            Some(Outcome::Ignored)
        }
        KeyCode::End => {
            tui.settings_selected = SETTINGS.len() - 1;
            Some(Outcome::Ignored)
        }
        KeyCode::Enter | KeyCode::Char(' ') => Some(Outcome::Message(settings_message(
            tui.settings_selected,
            view,
        ))),
        KeyCode::Esc => Some(Outcome::Message(backend::Message::Back)),
        _ => None,
    }
}

fn settings_message(index: usize, view: &View) -> CoreMessage {
    match index {
        0 => backend::Message::CycleTheme,
        1 => backend::Message::CycleQuality,
        2 => backend::Message::ToggleSaveOnPlay,
        3 => backend::Message::ToggleNotifySongChange,
        4 => backend::Message::ToggleNotifyErrors,
        5 => backend::Message::ToggleTranslation,
        6 => backend::Message::ToggleVisualizer,
        7 => backend::Message::TogglePitch,
        8 => backend::Message::DailySign,
        _ => {
            if view.user.is_some() {
                backend::Message::Logout
            } else {
                backend::Message::Navigate(Page::Login)
            }
        }
    }
}

fn nav_key(tui: &mut Tui, view: &View, key: KeyEvent) -> Outcome<CoreMessage> {
    let flat: Vec<(usize, usize)> = view
        .nav
        .sections
        .iter()
        .enumerate()
        .flat_map(|(section, items)| (0..items.items.len()).map(move |item| (section, item)))
        .collect();
    if flat.is_empty() {
        return Outcome::Ignored;
    }
    let current = flat
        .iter()
        .position(|entry| *entry == (view.nav.section, view.nav.item))
        .unwrap_or(0);
    let target = match key.code {
        KeyCode::Up | KeyCode::Char('k') => current.saturating_sub(1),
        KeyCode::Down | KeyCode::Char('j') => (current + 1).min(flat.len() - 1),
        KeyCode::Home => 0,
        KeyCode::End => flat.len() - 1,
        KeyCode::Enter => {
            tui.focus = Focus::Content;
            return Outcome::Message(backend::Message::ActivateNav {
                section: view.nav.section,
                item: view.nav.item,
            });
        }
        _ => return Outcome::Ignored,
    };
    let (section, item) = flat[target];
    tui.nav_offset = theme_nav_offset(tui, target, tui.nav_rows);
    Outcome::Message(backend::Message::ActivateNav { section, item })
}

fn theme_nav_offset(_tui: &Tui, selected: usize, rows: usize) -> usize {
    if rows == 0 {
        return 0;
    }
    selected.saturating_sub(rows.saturating_sub(1))
}

fn content_key(tui: &mut Tui, view: &View, key: KeyEvent) -> Outcome<CoreMessage> {
    if view.page == Page::Artist
        && let KeyCode::Char(ch @ '1'..='3') = key.code
    {
        tui.artist_tab = ch as usize - '1' as usize;
        return Outcome::Ignored;
    }

    match key.code {
        KeyCode::Up | KeyCode::Char('k') => move_selection(tui, view, -1),
        KeyCode::Down | KeyCode::Char('j') => move_selection(tui, view, 1),
        KeyCode::PageUp => move_selection(tui, view, -10),
        KeyCode::PageDown => move_selection(tui, view, 10),
        KeyCode::Home => jump_selection(tui, view, false),
        KeyCode::End => jump_selection(tui, view, true),
        KeyCode::Enter => activate_current(tui, view),
        KeyCode::Char('a') => add_next(tui, view),
        KeyCode::Char('s') => like_row(tui, view),
        KeyCode::Char('u') => {
            Outcome::Message(backend::Message::UploadRow(selected_index(tui, view)))
        }
        KeyCode::Char('A') => {
            Outcome::Message(backend::Message::OpenArtistRow(selected_index(tui, view)))
        }
        KeyCode::Char('b') => {
            Outcome::Message(backend::Message::OpenAlbumRow(selected_index(tui, view)))
        }
        KeyCode::Char('o') => Outcome::Message(backend::Message::LoadMore),
        KeyCode::Char('c') => Outcome::Message(backend::Message::OpenSongContext(false)),
        KeyCode::Char('C') => Outcome::Message(backend::Message::OpenSongContext(true)),
        KeyCode::Char('w') if view.page == Page::Queue => {
            Outcome::Message(backend::Message::ClearQueue)
        }
        _ => Outcome::Ignored,
    }
}

fn selected_index(tui: &Tui, view: &View) -> usize {
    match view.page {
        Page::Artist => tui.artist_selected[tui.artist_tab],
        Page::Queue => tui.queue_selected,
        _ => tui.content_selected,
    }
}

fn selection_len(tui: &Tui, view: &View) -> usize {
    match view.page {
        Page::Artist => artist_count(view, tui.artist_tab),
        Page::Queue => view.player.queue.len(),
        Page::Settings => SETTINGS.len(),
        Page::Login | Page::Lyrics => 0,
        Page::Browse => content_rows(view),
    }
}

fn selection_rows(tui: &Tui, view: &View) -> usize {
    match view.page {
        Page::Artist => tui.artist_rows,
        Page::Queue => tui.queue_rows,
        _ => tui.content_rows,
    }
}

fn move_selection(tui: &mut Tui, view: &View, delta: isize) -> Outcome<CoreMessage> {
    let len = selection_len(tui, view);
    if len == 0 {
        return Outcome::Ignored;
    }
    let current = selected_index(tui, view) as isize;
    let selected = clamp_selection((current + delta).max(0) as usize, len);
    set_selection(tui, view, selected);
    let rows = selection_rows(tui, view);
    match view.page {
        Page::Artist => ensure_visible(selected, &mut tui.artist_offset[tui.artist_tab], rows),
        Page::Queue => ensure_visible(selected, &mut tui.queue_offset, rows),
        _ => ensure_visible(selected, &mut tui.content_offset, rows),
    }
    Outcome::Ignored
}

fn jump_selection(tui: &mut Tui, view: &View, last: bool) -> Outcome<CoreMessage> {
    let len = selection_len(tui, view);
    if len == 0 {
        return Outcome::Ignored;
    }
    set_selection(tui, view, if last { len - 1 } else { 0 });
    Outcome::Ignored
}

fn set_selection(tui: &mut Tui, view: &View, selected: usize) {
    match view.page {
        Page::Artist => tui.artist_selected[tui.artist_tab] = selected,
        Page::Queue => tui.queue_selected = selected,
        Page::Settings => tui.settings_selected = selected.min(SETTINGS.len() - 1),
        _ => tui.content_selected = selected,
    }
}

fn activate_current(tui: &mut Tui, view: &View) -> Outcome<CoreMessage> {
    match view.page {
        Page::Artist => activate_artist(view, tui.artist_tab, tui.artist_selected[tui.artist_tab])
            .map(Outcome::Message)
            .unwrap_or(Outcome::Ignored),
        Page::Queue => {
            if tui.queue_selected < view.player.queue.len() {
                Outcome::Message(backend::Message::PlayQueueRow(tui.queue_selected))
            } else {
                Outcome::Ignored
            }
        }
        _ => activate_content(view, tui.content_selected)
            .map(Outcome::Message)
            .unwrap_or(Outcome::Ignored),
    }
}

fn add_next(tui: &Tui, view: &View) -> Outcome<CoreMessage> {
    match view.page {
        Page::Artist => {
            Outcome::Message(backend::Message::AddNextArtistSong(tui.artist_selected[0]))
        }
        _ => Outcome::Message(backend::Message::AddNextRow(selected_index(tui, view))),
    }
}

fn like_row(tui: &Tui, view: &View) -> Outcome<CoreMessage> {
    match view.page {
        Page::Artist => Outcome::Message(backend::Message::LikeArtistSong(tui.artist_selected[0])),
        _ => Outcome::Message(backend::Message::LikeRow(selected_index(tui, view))),
    }
}

fn mouse(tui: &mut Tui, view: &View, mouse: MouseEvent) -> Outcome<CoreMessage> {
    if !tui.mouse_enabled() {
        return Outcome::Ignored;
    }
    match mouse.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            let Some((rect, hit)) = hit_rect_at(tui, mouse.column, mouse.row) else {
                return Outcome::Ignored;
            };
            let double = tui.remember_click(hit);
            match hit {
                Hit::Nav { section, item } => {
                    tui.focus = Focus::Nav;
                    Outcome::Message(backend::Message::ActivateNav { section, item })
                }
                Hit::Page(page) => {
                    tui.focus = Focus::Content;
                    Outcome::Message(backend::Message::Navigate(page))
                }
                Hit::Content(index) => {
                    tui.focus = Focus::Content;
                    tui.content_selected = index;
                    if double {
                        activate_content(view, index)
                            .map(Outcome::Message)
                            .unwrap_or(Outcome::Ignored)
                    } else {
                        Outcome::Ignored
                    }
                }
                Hit::Artist { tab, index } => {
                    tui.focus = Focus::Content;
                    tui.artist_tab = tab;
                    tui.artist_selected[tab] = index;
                    if double {
                        activate_artist(view, tab, index)
                            .map(Outcome::Message)
                            .unwrap_or(Outcome::Ignored)
                    } else {
                        Outcome::Ignored
                    }
                }
                Hit::Queue(index) => {
                    tui.focus = Focus::Content;
                    tui.queue_selected = index;
                    if double {
                        Outcome::Message(backend::Message::PlayQueueRow(index))
                    } else {
                        Outcome::Ignored
                    }
                }
                Hit::Settings(index) => {
                    tui.focus = Focus::Content;
                    tui.settings_selected = index;
                    if double {
                        Outcome::Message(settings_message(index, view))
                    } else {
                        Outcome::Ignored
                    }
                }
                Hit::LoginMethod(index) => {
                    if let Some(method) = LoginMethod::ALL.get(index).copied() {
                        tui.login_field = 0;
                        Outcome::Message(backend::Message::SetLoginMethod(method))
                    } else {
                        Outcome::Ignored
                    }
                }
                Hit::Gauge => {
                    let fraction = fraction_in(rect, mouse.column);
                    Outcome::Message(seek_to(view, fraction))
                }
                Hit::Prev => Outcome::Message(backend::Message::Prev),
                Hit::Next => Outcome::Message(backend::Message::Next),
                Hit::Volume => {
                    let fraction = fraction_in(rect, mouse.column);
                    Outcome::Message(backend::Message::SetVolume(fraction))
                }
                Hit::Play => Outcome::Message(toggle_play()),
            }
        }
        MouseEventKind::ScrollUp => scroll(tui, view, mouse.column, mouse.row, -3),
        MouseEventKind::ScrollDown => scroll(tui, view, mouse.column, mouse.row, 3),
        _ => Outcome::Ignored,
    }
}

fn scroll(tui: &mut Tui, view: &View, column: u16, row: u16, delta: isize) -> Outcome<CoreMessage> {
    let area = tui.areas.content;
    if area.height == 0
        || column < area.x
        || column >= area.x.saturating_add(area.width)
        || row < area.y
        || row >= area.y.saturating_add(area.height)
    {
        return Outcome::Ignored;
    }
    move_selection(tui, view, delta)
}

fn hit_rect_at(tui: &Tui, column: u16, row: u16) -> Option<(Rect, Hit)> {
    let hit = hit_at(&tui.areas, column, row)?;
    tui.areas
        .hits
        .iter()
        .rev()
        .find(|(rect, candidate)| {
            *candidate == hit
                && column >= rect.x
                && column < rect.x.saturating_add(rect.width)
                && row >= rect.y
                && row < rect.y.saturating_add(rect.height)
        })
        .map(|(rect, hit)| (*rect, *hit))
}

fn fraction_in(rect: Rect, column: u16) -> f64 {
    if rect.width == 0 {
        return 0.0;
    }
    f64::from(column.saturating_sub(rect.x)) / f64::from(rect.width)
}

#[cfg(test)]
mod tests {
    use ferriso_tui::crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};

    use super::*;
    use crate::{
        backend::{Banner, LoginView, NavView, PlayerView, SearchView, SettingsView, ThemeView},
        config::Config,
        state::{ContentState, LoginField},
    };

    fn view() -> View {
        View {
            page: Page::Browse,
            nav: NavView {
                sections: Vec::new(),
                section: 0,
                item: 0,
            },
            content: ContentState::Empty,
            title: String::new(),
            subtitle: None,
            source: None,
            pagination: None,
            loading: false,
            banner: Banner::default(),
            user: None,
            can_back: false,
            artist: None,
            player: PlayerView {
                song: None,
                playing: false,
                paused: false,
                progress: 0.0,
                position_secs: 0.0,
                duration_secs: 0.0,
                volume: 0.5,
                mode: crate::playback::PlayMode::Sequential,
                liked: false,
                cached: false,
                cover: None,
                lyrics: Vec::new(),
                translated_lyrics: Vec::new(),
                visualizer: Vec::new(),
                pitch: None,
                mv: None,
                mv_poster: None,
                queue: Vec::new(),
                queue_keys: Vec::new(),
                queue_key: String::new(),
                queue_index: None,
            },
            search: SearchView {
                kind: SearchKind::Songs,
                query: String::new(),
                active: false,
            },
            login: LoginView {
                method: LoginMethod::Qr,
                account: String::new(),
                password: String::new(),
                phone: String::new(),
                code: String::new(),
                qr_url: String::new(),
                qr_status: String::new(),
                loading: false,
                error: None,
                notice: None,
            },
            settings: SettingsView {
                theme: "default".into(),
                themes: vec!["default".into()],
                quality: "standard".into(),
                save_on_play: false,
                notify_song_change: false,
                notify_errors: false,
                lyric_translation: true,
                visualizer: false,
                pitch: false,
            },
            theme: ThemeView::default(),
        }
    }

    fn press(tui: &mut Tui, view: &View, code: KeyCode) -> Outcome<CoreMessage> {
        handle(
            tui,
            view,
            Event::Key(KeyEvent::new(code, KeyModifiers::NONE)),
        )
    }

    #[test]
    fn quit_and_page_shortcuts_are_semantic_messages() {
        let view = view();
        let mut tui = Tui::new(&Config::default());
        assert!(matches!(
            press(&mut tui, &view, KeyCode::Char('q')),
            Outcome::Quit
        ));
        assert!(matches!(
            press(&mut tui, &view, KeyCode::Char('f')),
            Outcome::Message(backend::Message::Navigate(Page::Queue))
        ));
    }

    #[test]
    fn slash_opens_the_search_editor() {
        let view = view();
        let mut tui = Tui::new(&Config::default());
        press(&mut tui, &view, KeyCode::Char('/'));
        assert!(tui.search_editing);
        assert_eq!(tui.focus, Focus::Search);
    }

    #[test]
    fn login_text_is_not_a_page_shortcut() {
        let mut view = view();
        view.page = Page::Login;
        view.login.method = LoginMethod::Password;
        let mut tui = Tui::new(&Config::default());

        let outcome = press(&mut tui, &view, KeyCode::Char('q'));
        assert!(matches!(
            outcome,
            Outcome::Message(backend::Message::AppendLoginField(LoginField::Account, 'q'))
        ));
    }

    #[test]
    fn tab_cycles_focus_without_loading_anything() {
        let view = view();
        let mut tui = Tui::new(&Config::default());
        assert!(matches!(
            press(&mut tui, &view, KeyCode::Tab),
            Outcome::Ignored
        ));
        assert_eq!(tui.focus, Focus::Search);
        assert!(tui.search_editing);
    }
}
