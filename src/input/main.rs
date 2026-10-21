use crossterm::event::{MouseButton, MouseEventKind};
use ncm_api::SongInfo;
use ratatui::layout::Rect;
use std::sync::Arc;

use super::{
    content::{
        cell_enter_action, check_load_more, content_item_count, content_select_first,
        content_select_last, content_select_next, content_select_prev, playlist_play_selected,
        playlist_select_next, playlist_select_prev, row_enter_action,
    },
    hit,
    navigation::{emit_nav_select, navigate_nav_down, navigate_nav_up},
    table::{cell_select_next_column, cell_select_prev_column, toggle_table_mode},
};
use crate::{
    app::App,
    config::symbols,
    event::{NavigationEvent, PlaybackEvent},
    key::{KeyCode, KeyPress},
    playback::mode_icon,
    state::{ArtistIo, ArtistPane, ContentState, Page, TableMode},
    ui::playerbar,
};

/// The key a key event is, for the key map: the keys a binding can name are characters, with
/// Ctrl/Alt optionally held (Shift is part of the character). Anything else — the arrows, `Tab`,
/// `Enter` — is not a binding and is left to the key map's own arms.
fn key_of(key_event: KeyPress) -> Option<crate::config::keymap::Key> {
    let KeyCode::Char(code) = key_event.code else {
        return None;
    };
    let alt = key_event.mods.alt;

    Some(crate::config::keymap::Key {
        code,
        ctrl: key_event.mods.ctrl,
        alt,
    })
}

pub(super) fn handle_main_key(app: &mut App, key_event: KeyPress) -> color_eyre::Result<()> {
    // `Ctrl` + an arrow moves a pane edge, the way dragging one does; nothing else uses it.
    // The page's own keys come first, the way a layered keymap works: what `↑`/`↓`/`←`/`→` mean
    // depends on the page that is up, and each page writes its own layer (see `PageSpec::keys`).
    if let Some(keys) = app.state.navigation.page.spec().keys
        && keys(app, key_event)
    {
        return Ok(());
    }

    if super::panes::handle_key(app, key_event) {
        return Ok(());
    }

    // Then the command table's keys: the table *is* the key map (see `config::keymap`), so a key
    // the user rebound runs its command here, before the hand-written arms below — those are for
    // the keys no command owns (navigation, playback, the row keys).
    //
    // Built per key press rather than cached: a `[keys]` edit has to take effect the moment it is
    // saved, and a dozen bindings are nothing next to a key press.
    if let Some(key) = key_of(key_event) {
        let keymap = crate::config::keymap::Keymap::from_config(&app.config);
        match keymap.advance(&mut app.state.pending_keys, key) {
            crate::config::keymap::Pressed::Run(name) => {
                if let Ok(command) = super::ex::ExCommand::parse(name)
                    && let Err(error) = super::ex::execute(app, command)
                {
                    app.notice(
                        crate::state::notices::Level::Error,
                        format!("{name}: {error}"),
                    );
                }
                return Ok(());
            }
            // The keys so far are the start of a binding: hold them and let the next key decide.
            // `Esc` gives up on them (see the `Esc` arm below).
            crate::config::keymap::Pressed::Wait => return Ok(()),
            // Not a binding: the keys are forgotten and the press carries on to the map below.
            crate::config::keymap::Pressed::FallThrough => {}
        }
    }

    match key_event.code {
        KeyCode::Esc => {
            // `Esc` also gives up on a half-typed key sequence: a modal keymap needs a way out of
            // one, and this is the key that means "forget that".
            app.state.pending_keys.clear();
            // Pages that are not in the breadcrumb stack handle `Esc` in their own layer, before
            // this map runs; what is left here is the restore every other page shares.
            app.state.events.send(NavigationEvent::ContentRestore);
        }
        KeyCode::Tab => navigate_nav_down(app),
        KeyCode::BackTab => navigate_nav_up(app),
        // The keys a page owns are handled by that page's own layer before this map runs (see
        // `input::pages` and `PageSpec::keys`), so what is left here is what every page shares.
        KeyCode::Up | KeyCode::Char('k' | 'K') => content_select_prev(app),
        KeyCode::Down | KeyCode::Char('j' | 'J') => content_select_next(app),
        KeyCode::Char('g') => content_select_first(app),
        KeyCode::Char('G') => content_select_last(app),
        KeyCode::Enter => {
            if !open_artist_from_table(app) {
                // Not an artist row: the table's own Enter, cell mode or row mode.
                if app.state.navigation.table_mode == TableMode::Cell {
                    cell_enter_action(app);
                } else {
                    row_enter_action(app);
                }
            }
        }
        KeyCode::Left => {
            if app.state.navigation.table_mode == TableMode::Cell
                && matches!(app.state.navigation.page, Page::Main | Page::Lyrics)
            {
                cell_select_prev_column(app);
            } else if app.playback.current_song().is_some() {
                let interval = app.config.seek_interval_secs as f64;
                app.playback.seek_relative(-interval);
            }
        }
        KeyCode::Right => {
            if app.state.navigation.table_mode == TableMode::Cell
                && matches!(app.state.navigation.page, Page::Main | Page::Lyrics)
            {
                cell_select_next_column(app);
            } else if app.playback.current_song().is_some() {
                let interval = app.config.seek_interval_secs as f64;
                app.playback.seek_relative(interval);
            }
        }
        KeyCode::Char('l') => open_page_key(app, 'l'),
        // `,` is the settings page, the way it is in most of these TUIs; `:settings` says the
        // same thing for anyone who would rather type it.
        KeyCode::Char(',') => open_page_key(app, ','),
        KeyCode::Char('p' | 'P') => {
            app.playback.prev();
        }
        KeyCode::Char('n' | 'N') => {
            app.playback.next();
        }
        KeyCode::Char('c' | 'C') => {
            toggle_table_mode(app);
        }
        KeyCode::Char('f' | 'F') => {
            // The queue opens on the song that is playing.
            if app.state.navigation.page == Page::Main {
                app.state.navigation.playlist_selected =
                    app.playback.queue_current_index().unwrap_or(0);
            }
            open_page_key(app, 'f');
        }
        KeyCode::Char('/') => {
            if app.state.navigation.page == Page::Lyrics {
                return Ok(());
            } else {
                app.state.events.send(NavigationEvent::SearchActivated);
            }
        }
        KeyCode::Char(' ') => toggle_play_pause(app),
        KeyCode::Char('m') => cycle_play_mode(app),
        KeyCode::Char('S') => {
            if let Some(song) = app.playback.current_song() {
                app.state
                    .events
                    .send(PlaybackEvent::LikeSong(song.id, true));
                app.toast(format!("♥  {}", song.name));
            }
        }
        KeyCode::Char('s') => {
            // The artist page's rows are songs too, so `s` likes the one its cursor is on: that
            // page is not table content, and its own list is what the cursor walks.
            let song = match app.state.navigation.page {
                Page::Artist => app.state.navigation.artist.selected_song(),
                _ => match app.state.navigation.content.as_ref() {
                    ContentState::Songs(songs) => songs
                        .get(app.state.navigation.content_selected)
                        .map(|song| &**song),
                    _ => None,
                },
            }
            .map(|song| (song.id, song.name.clone()));
            if let Some((id, name)) = song {
                app.state.events.send(PlaybackEvent::LikeSong(id, true));
                app.toast(format!("♥  {name}"));
            }
        }
        KeyCode::Char('a' | 'A') => {
            // Same rows as `s`: the artist page's own list, or the table's current content.
            let song = if app.state.navigation.page == Page::Artist {
                app.state.navigation.artist.selected_song().cloned()
            } else if let ContentState::Songs(songs) = app.state.navigation.content.as_ref() {
                songs
                    .get(app.state.navigation.content_selected)
                    .map(|song| (**song).clone())
            } else {
                None
            };
            if let Some(song) = song {
                app.playback.add_next(Arc::new(song.clone()));
                app.toast(format!("⏭  下一首: {}", song.name));
            }
        }
        KeyCode::Char('d') => {
            if is_daily_recommend(app)
                && let ContentState::Songs(songs) = app.state.navigation.content.as_ref()
            {
                let sel = app.state.navigation.content_selected;
                if let Some(song) = songs.get(sel) {
                    app.state.events.send(PlaybackEvent::DislikeSong(song.id));
                    app.toast(format!("✕  {}", song.name));
                }
            } else if let ContentState::Songs(songs) = app.state.navigation.content.as_ref() {
                let sel = app.state.navigation.content_selected;
                if let Some(song) = songs.get(sel) {
                    app.state
                        .events
                        .send(PlaybackEvent::LikeSong(song.id, false));
                    app.toast(format!("♡ 已取消喜欢: {}", song.name));
                }
            }
        }
        KeyCode::Char('D') => {
            if let Some(song) = app.playback.current_song() {
                app.state
                    .events
                    .send(PlaybackEvent::LikeSong(song.id, false));
                app.toast(format!("♡ 已取消喜欢: {}", song.name));
            }
        }
        KeyCode::Char('r' | 'R') => {
            if matches!(app.state.navigation.page, Page::Main | Page::Lyrics) {
                app.reload_current_nav();
            }
        }
        KeyCode::Char('u' | 'U') if is_download_view(app) || is_local_music_view(app) => {
            let sel = app.state.navigation.content_selected;
            app.state
                .events
                .send(NavigationEvent::UploadCachedSong(sel));
        }
        KeyCode::Char('+' | '=') => {
            app.adjust_volume(0.05);
        }
        KeyCode::Char('-' | '_') => {
            app.adjust_volume(-0.05);
        }
        _ => {}
    }
    Ok(())
}

/// A page key — `l` for the lyrics, `f` for the queue: it opens the page whose table entry
/// names it, and that page's own key comes back to the main page. See [`Page::on_key`].
fn open_page_key(app: &mut App, key: char) {
    if let Some(next) = app.state.navigation.page.on_key(key) {
        app.state.events.send(NavigationEvent::Navigate(next));
    }
}

pub(super) fn handle_main_mouse(app: &mut App, kind: MouseEventKind, col: u16, row: u16) {
    // The pane edges first: they are the frame's own furniture, so a drag on one is not a click
    // on whatever the pane is showing.
    if super::panes::handle_mouse(app, kind, col, row) {
        return;
    }

    if kind == MouseEventKind::Down(MouseButton::Left) {
        handle_click(app, col, row);
        return;
    }

    // Volume scroll: if mouse is over playerbar area
    let area = app.state.playerbar_area;
    if row >= area.y && row < area.y + area.height && col >= area.x && col < area.x + area.width {
        let vol = app.playback.state.volume;
        match kind {
            MouseEventKind::ScrollUp => {
                let new = (vol + 0.05).clamp(0.0, 1.0);
                app.playback.set_volume(new);
                app.toast(format!(" {}  {:.0}%", symbols().volume_high, new * 100.0));
            }
            MouseEventKind::ScrollDown => {
                let new = (vol - 0.05).clamp(0.0, 1.0);
                app.playback.set_volume(new);
                app.toast(format!(" {}  {:.0}%", symbols().volume_high, new * 100.0));
            }
            _ => {}
        }
        return;
    }

    // The wheel over the navigation moves its cursor, exactly as a wheel over the content
    // moves the content cursor: an item the layout cannot show — scrolled out of the sidebar,
    // or past the end of the row — is otherwise unreachable with the mouse, because the
    // navigation only ever scrolls to follow the keyboard.
    if hit::contains(app.state.nav_area, col, row) {
        match kind {
            MouseEventKind::ScrollUp => navigate_nav_up(app),
            MouseEventKind::ScrollDown => navigate_nav_down(app),
            _ => {}
        }
        return;
    }

    match app.state.navigation.page {
        Page::Lyrics => {
            if kind == MouseEventKind::ScrollUp {
                app.playback.seek_relative(-5.0);
            } else if kind == MouseEventKind::ScrollDown {
                app.playback.seek_relative(5.0);
            }
        }
        Page::Main => {
            if kind == MouseEventKind::ScrollUp {
                content_select_prev(app);
            } else if kind == MouseEventKind::ScrollDown {
                content_select_next(app);
            }
        }
        Page::Playlist => {
            if kind == MouseEventKind::ScrollUp {
                playlist_select_prev(app);
            } else if kind == MouseEventKind::ScrollDown {
                playlist_select_next(app);
            }
        }
        Page::Settings => {
            if kind == MouseEventKind::ScrollUp {
                crate::ui::settings::handle_settings_scroll(app, true);
            } else if kind == MouseEventKind::ScrollDown {
                crate::ui::settings::handle_settings_scroll(app, false);
            }
        }
        Page::Artist => {
            // The wheel walks the list it is over, and that list takes the cursor: the pointer
            // says which pane the reader is working in. Over neither pane the wheel keeps
            // walking whichever list the cursor is already in.
            if let Some(pane) = artist_pane_at(app, col, row) {
                app.state.navigation.artist.pane = pane;
            }
            if kind == MouseEventKind::ScrollUp {
                app.state.navigation.artist.select_prev();
            } else if kind == MouseEventKind::ScrollDown {
                app.state.navigation.artist.select_next();
            }
        }
        _ => {}
    }
}

/// A player bar button the mouse can land on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlayerbarTarget {
    Transport(playerbar::ControlButton),
    Mode,
    Like,
}

/// Which button a click lands on, if any: the transport buttons, the mode icon and the
/// like hearts are hit-tested against the rects the last frame published, so they follow
/// whatever the active layout drew.
fn playerbar_target(
    transport: &[(playerbar::ControlButton, Rect); 3],
    mode: Rect,
    likes: &[Rect; 2],
    col: u16,
    row: u16,
) -> Option<PlayerbarTarget> {
    if let Some((button, _)) = transport
        .iter()
        .find(|(_, rect)| hit::contains(*rect, col, row))
    {
        return Some(PlayerbarTarget::Transport(*button));
    }
    if hit::contains(mode, col, row) {
        return Some(PlayerbarTarget::Mode);
    }
    if likes.iter().any(|rect| hit::contains(*rect, col, row)) {
        return Some(PlayerbarTarget::Like);
    }
    None
}

/// Start or pause playback, the way `Space` and the play button both mean it.
fn toggle_play_pause(app: &mut App) {
    let was_paused = app.playback.state.paused;
    app.playback.toggle_pause();
    if let Some(song) = app.playback.current_song() {
        if was_paused {
            app.toast(format!("\u{f03e4}  {}", song.name));
        } else {
            app.toast(format!("\u{f040a}  {}", song.name));
        }
    }
}

/// Move to the next play mode, the way `m` and the mode icon both mean it.
fn cycle_play_mode(app: &mut App) {
    let mode = app.playback.cycle_mode();
    let (icon, label) = mode_icon(&mode);
    app.toast(format!("{icon} 循环: {label}"));
}

/// Like or unlike the current song, the way the heart in the player bar means it.
fn toggle_like(app: &mut App) {
    let Some(song) = app.playback.current_song() else {
        return;
    };
    let like = !app.playback.state.liked;
    app.state
        .events
        .send(PlaybackEvent::LikeSong(song.id, like));
    if like {
        app.toast(format!("♥  {}", song.name));
    } else {
        app.toast(format!("♡  {}", song.name));
    }
}

/// Left click: seek on the progress bar, toggle what the player bar shows, switch to a
/// navigation item, and select rows — clicking the row that is already selected opens it,
/// which is the mouse's Enter.
fn handle_click(app: &mut App, col: u16, row: u16) {
    if let Some(fraction) = hit::progress_fraction(app.state.gauge_area, col, row) {
        app.playback.seek_to_fraction(fraction);
        return;
    }

    if click_playerbar(app, col, row) {
        return;
    }

    if let Some((section, item)) = hit::nav_item(&app.state.navigation.nav.nav_hits, col, row)
        && app
            .state
            .navigation
            .nav
            .sections
            .get(section)
            .is_some_and(|s| item < s.items.len())
    {
        let nav = &mut app.state.navigation.nav;
        nav.focus_section = section;
        nav.section_states[section].select(Some(item));
        emit_nav_select(app);
        return;
    }

    match app.state.navigation.page {
        Page::Playlist => click_queue_page(app, col, row),
        Page::Main => click_content(app, col, row),
        Page::Artist => click_artist(app, col, row),
        Page::Settings => {
            crate::ui::settings::handle_settings_click(app, col, row);
        }
        _ => {}
    }
}

/// The player bar's own click targets: the readouts, the transport buttons, the mode
/// icon, the like button, the volume icon and the cover. Returns whether the click was one
/// of them.
fn click_playerbar(app: &mut App, col: u16, row: u16) -> bool {
    if app.state.spectrum_row_area.width > 0 || app.state.pitch_area.width > 0 {
        // Clicking a readout hides it; `:visualizer on` / `:pitch on` bring it back. The
        // pitch cell is its own in layouts that have one, and part of the spectrum row
        // otherwise.
        let (bars, shared) =
            playerbar::spectrum_row(app.state.spectrum_row_area, &app.config.playerbar.visible);
        let pitch = if app.state.pitch_area.width > 0 {
            app.state.pitch_area
        } else {
            shared
        };

        if app.config.playerbar.visible.pitch && hit::contains(pitch, col, row) {
            app.config.playerbar.visible.pitch = false;
            app.toast("音高已隐藏（:pitch on 恢复）".to_string());
            return true;
        }
        if app.config.playerbar.visible.visualizer && hit::contains(bars, col, row) {
            app.config.playerbar.visible.visualizer = false;
            app.toast("频谱已隐藏（:visualizer on 恢复）".to_string());
            return true;
        }
    }

    let buttons = playerbar_target(
        &app.state.transport,
        app.state.mode_area,
        &app.state.like_areas,
        col,
        row,
    );
    if let Some(target) = buttons {
        match target {
            PlayerbarTarget::Transport(playerbar::ControlButton::Prev) => app.playback.prev(),
            PlayerbarTarget::Transport(playerbar::ControlButton::PlayPause) => {
                toggle_play_pause(app);
            }
            PlayerbarTarget::Transport(playerbar::ControlButton::Next) => app.playback.next(),
            PlayerbarTarget::Mode => cycle_play_mode(app),
            PlayerbarTarget::Like => toggle_like(app),
        }
        return true;
    }

    if hit::contains(app.state.volume_area, col, row) {
        if app.playback.state.volume > 0.0 {
            app.state.volume_before_mute = Some(app.playback.state.volume);
            app.playback.set_volume(0.0);
            app.toast(" 静音".to_string());
        } else {
            let restored = app.state.volume_before_mute.take().unwrap_or(0.65);
            app.playback.set_volume(restored);
            app.toast(format!(
                " {}  {:.0}%",
                symbols().volume_high,
                restored * 100.0
            ));
        }
        return true;
    }

    if hit::contains(app.state.cover_area, col, row) {
        // The cover is the "now playing" surface, so clicking it flips to the lyrics and
        // back, exactly like the `l` key.
        let next = match app.state.navigation.page {
            Page::Main => Page::Lyrics,
            Page::Lyrics => Page::Main,
            other => other,
        };
        app.state.events.send(NavigationEvent::Navigate(next));
        return true;
    }

    false
}

/// Queue page: tabs switch queues, and rows select, then play on the second click.
fn click_queue_page(app: &mut App, col: u16, row: u16) {
    let key = hit::queue_tab(&app.state.queue_hits.tabs, col, row).map(str::to_string);
    if let Some(key) = key {
        app.playback.activate_queue(&key);
        app.state.navigation.playlist_selected = app.playback.queue_current_index().unwrap_or(0);
        return;
    }

    let (table, offset) = (app.state.queue_hits.table, app.state.queue_hits.offset);
    let total = app.playback.queue_len();
    let Some(index) = hit::table_row(table, 1, offset, total, col, row) else {
        return;
    };

    let repeat = app.state.navigation.playlist_selected == index;
    app.state.navigation.playlist_selected = index;
    if repeat {
        playlist_play_selected(app);
    }
}

/// Content table of the main page: select, then open on the second click.
fn click_content(app: &mut App, col: u16, row: u16) {
    let total = content_item_count(app);
    let Some(index) = hit::table_row(
        app.state.content_inner,
        1,
        app.state.content_offset,
        total,
        col,
        row,
    ) else {
        return;
    };

    let repeat = app.state.navigation.content_selected == index;
    app.state.navigation.content_selected = index;
    app.state.navigation.table_state.select(Some(index));
    check_load_more(app, total);
    // The second click opens the row, which for an artist row is the artist page — the same
    // thing Enter does.
    if repeat && !open_artist_from_table(app) {
        if app.state.navigation.table_mode == TableMode::Cell {
            cell_enter_action(app);
        } else {
            row_enter_action(app);
        }
    }
}

/// The artist row under the cursor, if the table is showing artists: the id, and the name and
/// portrait the row already carries.
fn selected_artist(app: &App) -> Option<(u64, String, String)> {
    let ContentState::Singers(singers) = app.state.navigation.content.as_ref() else {
        return None;
    };
    let singer = singers.get(app.state.navigation.content_selected)?;
    // The API sends id 0 for rows it has no artist for; such a row has nothing to open.
    (singer.id != 0).then(|| (singer.id, singer.name.clone(), singer.pic_url.clone()))
}

/// Open the artist page on the row under the cursor, and say whether the table had one.
///
/// The hot-artists table is the only way into the page, so this is where the table's rows stop
/// being rows and start being artists: the row's name and portrait go along, because they are
/// what the page can draw before its own request lands. Everything else about Enter is
/// unchanged — an artist row opens the page, any other row (or cell) keeps its old meaning.
fn open_artist_from_table(app: &mut App) -> bool {
    let Some((id, name, pic_url)) = selected_artist(app) else {
        return false;
    };
    open_artist(app, id, name, pic_url);
    true
}

/// Load the artist the page is showing again (`r`). The page keeps its header, so this is the
/// retry a failed page offers.
pub(super) fn reload_artist(app: &mut App) {
    let io = artist_io(app);
    app.state.navigation.artist.reload(io);
}

/// Where the artist page gets its data: the API client, the proxy-aware image client, and the
/// terminal's image protocol for the portrait.
fn artist_io(app: &App) -> ArtistIo {
    ArtistIo {
        client: app.service.client().clone(),
        http: app.cover_http.clone(),
        picker: app.picker.clone(),
        repaint: app.state.events.sender(),
    }
}

/// Play the hot song under the artist page's cursor.
///
/// The page's songs live in the page rather than in the content table, and
/// `PlaybackEvent::SongPlay` resolves a song against the open table — it would find nothing
/// here — so the queue is built from the page's own list.
pub(super) fn artist_play_selected(app: &mut App) {
    let (key, index) = {
        let artist = &app.state.navigation.artist;
        let songs = artist.hot_songs();
        if songs.is_empty() {
            return;
        }
        // The same context string the artist's songs use as their breadcrumb on the main page,
        // so playing from here lands in one queue per artist rather than a new one each time.
        (
            format!("歌手: {}", artist.name),
            artist.song_selected.min(songs.len() - 1),
        )
    };

    let songs: Vec<Arc<SongInfo>> = app
        .state
        .navigation
        .artist
        .hot_songs()
        .iter()
        .cloned()
        .map(Arc::new)
        .collect();
    app.playback.play_songs(&key, songs, index);
}

/// The pane of the artist page a screen position is over, if it is over one.
fn artist_pane_at(app: &App, col: u16, row: u16) -> Option<ArtistPane> {
    artist_row_at(app, col, row).map(|(pane, _)| pane)
}

/// The pane and the row of the artist page a screen position is over. The draw pass publishes
/// both tables' bodies and their first visible row, so this is the same arithmetic the main
/// table's clicks use — rather than a second layout pass, which would have to guess how tall
/// the profile band came out.
fn artist_row_at(app: &App, col: u16, row: u16) -> Option<(ArtistPane, usize)> {
    let hits = app.state.artist_hits;
    let artist = &app.state.navigation.artist;
    if let Some(index) = hit::table_row(
        hits.songs,
        1,
        hits.songs_offset,
        artist.hot_songs().len(),
        col,
        row,
    ) {
        return Some((ArtistPane::Songs, index));
    }
    if let Some(index) = hit::table_row(
        hits.albums,
        1,
        hits.albums_offset,
        artist.albums().len(),
        col,
        row,
    ) {
        return Some((ArtistPane::Albums, index));
    }
    hit::table_row(
        hits.similar,
        1,
        hits.similar_offset,
        artist.similar().len(),
        col,
        row,
    )
    .map(|index| (ArtistPane::Similar, index))
}

/// Open the artist page on one artist. Both ways in land here: a row of the hot-artist table,
/// and a row of the page's own similar-artist pane — which is why the page is a page and not a
/// step of the table's walk.
fn open_artist(app: &mut App, id: u64, name: String, pic_url: String) {
    let io = artist_io(app);
    app.state.navigation.artist.open(id, name, pic_url, io);
    // The artist page is opened by name, so it is a step of its own: content opened from it
    // returns here, but nothing returns to it.
    app.state.navigation.return_page = None;
    app.state
        .events
        .send(NavigationEvent::Navigate(Page::Artist));
}

/// A click on the artist page puts the cursor on the row it landed on, in the pane it landed
/// in: the list that was clicked is the list being worked in, so the click moves the focus too.
/// Clicking the row that was already selected opens it, which is what `Enter` does there.
fn click_artist(app: &mut App, col: u16, row: u16) {
    let Some((pane, index)) = artist_row_at(app, col, row) else {
        return;
    };
    let artist = &mut app.state.navigation.artist;
    let repeat = artist.pane == pane && artist.cursor_in(pane) == index;
    artist.select_in(pane, index);
    if repeat {
        artist_activate(app);
    }
}

/// `Enter` on the artist page. The pane the cursor is in decides what the row under it means:
/// a hot song is played, an album is opened as content.
pub(super) fn artist_activate(app: &mut App) {
    match app.state.navigation.artist.pane {
        ArtistPane::Songs => artist_play_selected(app),
        ArtistPane::Similar => {
            let Some((id, name, pic_url)) = app
                .state
                .navigation
                .artist
                .selected_similar()
                .map(|singer| (singer.id, singer.name.clone(), singer.pic_url.clone()))
            else {
                return;
            };
            open_artist(app, id, name, pic_url);
        }
        ArtistPane::Albums => {
            let Some((id, name)) = app
                .state
                .navigation
                .artist
                .selected_album()
                .map(|album| (album.id, album.name.clone()))
            else {
                return;
            };
            app.state
                .events
                .send(NavigationEvent::OpenAlbum { id, name });
        }
    }
}

fn current_api(app: &App) -> Option<&str> {
    app.state.navigation.nav.selected_api()
}

fn is_daily_recommend(app: &App) -> bool {
    current_api(app) == Some("recommend_songs")
}

fn is_download_view(app: &App) -> bool {
    current_api(app) == Some("download")
}

fn is_local_music_view(app: &App) -> bool {
    current_api(app) == Some("local_music")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::key::Modifiers;

    /// One key press, as the app receives it.
    fn press(app: &mut App, key: char) {
        press_key(app, KeyCode::Char(key));
    }

    /// The same, for the keys that are not characters.
    ///
    /// Through the app's own entry point, not this module's: a popup, the `:` line and the page
    /// layers are all consulted before the map below, and a test that skipped them would be
    /// testing a keyboard the user does not have.
    fn press_key(app: &mut App, key: KeyCode) {
        crate::input::handle_key_events(app, KeyPress::new(key, Modifiers::NONE)).expect("key");
    }

    /// A headless app: it owns no terminal, so — since `Config::persist` — nothing a key press
    /// does here can reach the user's own `config.toml`.
    fn app() -> App {
        // Both are what `src/main.rs` does at startup and what the other tests do before building
        // one: the crypto provider for the HTTP clients the app brings up.
        let _ = rustls::crypto::ring::default_provider().install_default();
        let mut app = App::new(crate::config::Config::default(), false).expect("app");
        // Past the splash: it is the one page that ignores keys, so a session that presses one is
        // a session that has booted.
        app.state.navigation.page = Page::Main;

        app
    }

    /// The keyboard is the command table, end to end: a key runs the command its row names, a
    /// rebind moves that command, and an empty binding leaves it to the `:` line.
    ///
    /// This is the chain a unit test cannot see — the key map is built from the table, the command
    /// is parsed out of the name and executed, and the config the command writes is what the
    /// assertion reads.
    #[tokio::test]
    async fn a_key_runs_its_command_and_a_rebind_moves_it() {
        let mut app = app();
        assert!(!app.config.playerbar.visible.visualizer, "a fresh config");

        press(&mut app, 'v');
        assert!(
            app.config.playerbar.visible.visualizer,
            "`v` ran `:visualizer`"
        );

        app.config
            .keys
            .insert("visualizer".to_string(), "w".to_string());
        press(&mut app, 'v');
        assert!(
            app.config.playerbar.visible.visualizer,
            "`v` was moved off `:visualizer`, so it runs nothing"
        );
        press(&mut app, 'w');
        assert!(!app.config.playerbar.visible.visualizer, "`w` runs it now");

        app.config
            .keys
            .insert("visualizer".to_string(), String::new());
        press(&mut app, 'w');
        assert!(
            !app.config.playerbar.visible.visualizer,
            "an empty binding leaves the command to the `:` line"
        );
    }

    /// The song-scoped pages (`:simi`, `:simiplaylist`) are opened *from* wherever the song is,
    /// so the way back is that page and not the main table — which is where the content table's
    /// own walk ends.
    #[tokio::test]
    async fn a_song_context_remembers_the_page_it_was_opened_from() {
        let mut app = app();
        app.state.navigation.page = Page::Playlist;

        app.state.events.send(NavigationEvent::OpenSongContext {
            song_id: 5,
            similar: true,
        });
        app.handle_events().await.expect("events");

        assert_eq!(
            app.state.navigation.page,
            Page::Main,
            "content is drawn on the table's page"
        );
        assert_eq!(
            app.state.navigation.return_page,
            Some(Page::Playlist),
            "the page that asked is the way back"
        );
        assert_eq!(
            app.state.navigation.nav.subtitle.as_deref(),
            Some("相似歌曲"),
            "and the page says which list it is"
        );

        press_key(&mut app, KeyCode::Esc);
        app.handle_events().await.expect("events");
        assert_eq!(
            app.state.navigation.page,
            Page::Playlist,
            "`Esc` went back to the queue, not to the table"
        );
    }

    /// The settings page has three ways in — keys, `Tab`, and the mouse — and they agree about
    /// where the cursor is. The `Tab` pane switch and the click handling are the two the page grew
    /// after it was first drawn.
    #[tokio::test]
    async fn the_settings_page_answers_keys_tab_and_mouse() {
        use crate::ui::settings::{Focus, SETTINGS};
        use crossterm::event::{MouseButton, MouseEventKind};

        let mut app = app();
        press(&mut app, ',');
        app.handle_events().await.expect("events");
        assert_eq!(app.state.navigation.page, Page::Settings);
        assert_eq!(
            app.state.settings.focus,
            Focus::Rows,
            "the rows are what a page is for"
        );

        // `Tab` moves to the sections, and `↑↓` walks them there.
        press_key(&mut app, KeyCode::Tab);
        assert_eq!(app.state.settings.focus, Focus::Sections);
        let before = SETTINGS[app.state.settings.selected].group;
        press(&mut app, 'j');
        let after = SETTINGS[app.state.settings.selected].group;
        assert_ne!(before, after, "`j` walked a section, not a row");
        assert_eq!(
            app.state.settings.selected,
            SETTINGS
                .iter()
                .position(|setting| setting.group == after)
                .expect("the section has a row"),
            "the page follows to the section's first row"
        );

        // `Tab` back: the rows walk again.
        press_key(&mut app, KeyCode::Tab);
        assert_eq!(app.state.settings.focus, Focus::Rows);

        // The hit areas come from a frame, so draw one the way the page does — and the page only
        // draws the rows of the section the cursor is in, so put the cursor in that section first.
        let target = SETTINGS
            .iter()
            .position(|setting| setting.label == "边听边存")
            .expect("the cache switch is a row");
        app.state.settings.select_row(target);

        let config = app.config.clone();
        let theme = app.current_theme().clone();
        let style = crate::ui::block::BlockStyle {
            colors: &theme,
            base: theme.bg,
            border: &config.border,
            tick: 0,
        };
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(90, 24)).expect("backend");
        terminal
            .draw(|f| {
                crate::ui::settings::draw(f, &config, &mut app.state.settings, &style, f.area())
            })
            .expect("draw");

        // A click picks the row it lands on — the row whose hit area is under the cell — and the
        // second click on that row changes what it names.
        let area = app
            .state
            .settings
            .row_hits
            .last()
            .copied()
            .expect("the frame drew the section's rows");
        assert_eq!(
            app.state.settings.row_hits.len(),
            1,
            "one row in this section"
        );
        crate::input::handle_mouse_event(
            &mut app,
            MouseEventKind::Down(MouseButton::Left),
            area.x + 1,
            area.y,
        );
        assert_eq!(
            app.state.settings.selected, target,
            "the click picked the row"
        );

        let before = app.config.cache.save_on_play;
        crate::input::handle_mouse_event(
            &mut app,
            MouseEventKind::Down(MouseButton::Left),
            area.x + 1,
            area.y,
        );
        assert_ne!(
            app.config.cache.save_on_play, before,
            "clicking the picked row changed it"
        );

        // The wheel walks the rows — through the whole table, wrapping at the end, which is what
        // the cursor does on every other list in the app.
        let before = app.state.settings.selected;
        crate::input::handle_mouse_event(&mut app, MouseEventKind::ScrollDown, area.x + 1, area.y);
        assert_ne!(
            app.state.settings.selected, before,
            "the wheel moved the cursor"
        );
    }

    /// A key sequence, typed through the real entry point: the first key is held, the second runs
    /// the command, and `Esc` gives up on a half-typed one instead of running it later.
    #[tokio::test]
    async fn a_key_sequence_runs_when_it_is_complete() {
        let mut app = app();
        app.config
            .keys
            .insert("spin".to_string(), "e e".to_string());
        assert!(!app.config.playerbar.spinning_cover, "a fresh config");

        press(&mut app, 'e');
        assert!(
            !app.config.playerbar.spinning_cover,
            "one key of two runs nothing"
        );
        press(&mut app, 'e');
        assert!(app.config.playerbar.spinning_cover, "the sequence ran");

        // Half a sequence, then `Esc`: the held key is forgotten rather than joined by the next
        // one, so the command does not run.
        press(&mut app, 'e');
        press_key(&mut app, KeyCode::Esc);
        press(&mut app, 'e');
        assert!(
            app.config.playerbar.spinning_cover,
            "`Esc` gave up on the half-typed sequence"
        );
    }

    /// The translation switch has one trigger path: `y` is the table's key for `translation`,
    /// so the key and the `:` command run the same handler rather than two copies.
    #[tokio::test]
    async fn the_translation_key_and_command_share_one_trigger() {
        let mut app = app();
        assert!(app.config.lyric_translation, "the default is on");

        press(&mut app, 'y');
        assert!(!app.config.lyric_translation, "`y` toggled it off");

        crate::input::ex::execute(
            &mut app,
            crate::input::ex::ExCommand::parse("translation on").expect("a command"),
        )
        .expect("toggles it back");
        assert!(app.config.lyric_translation, "the command toggled it on");

        // The settings page reads the same row and runs the same command.
        let target = crate::ui::settings::SETTINGS
            .iter()
            .position(|setting| setting.label == "译文")
            .expect("the translation row");
        app.state.settings.select_row(target);
        crate::ui::settings::change(&mut app, &crate::ui::settings::SETTINGS[target], true)
            .expect("the row changes it");
        assert!(!app.config.lyric_translation, "the row toggled it off");
    }

    /// `:tasks` opens the list, `Esc` closes it, and the keys while it is up are its own — the
    /// same shape `:messages` and the help popup have, because they are the same list.
    #[tokio::test]
    async fn the_task_list_opens_with_its_command_and_closes_with_esc() {
        let mut app = app();
        assert!(!app.state.tasks_popup.open);

        crate::input::ex::execute(
            &mut app,
            crate::input::ex::ExCommand::parse("tasks").expect("a command"),
        )
        .expect("opens");
        assert!(app.state.tasks_popup.open);

        press_key(&mut app, KeyCode::Esc);
        assert!(!app.state.tasks_popup.open, "`Esc` closed it");
    }

    /// What the app is doing is visible: a navigation request registers a task, and the content
    /// that arrives finishes it — the two ends of the wire the load travels along.
    #[tokio::test]
    async fn the_load_behind_a_navigation_shows_up_and_finishes() {
        use crate::state::{ContentState, tasks::TaskState};

        let mut app = app();
        // The registration half is the navigation request's, which would go to the network if it
        // were sent from a test; the task it registers is the same one.
        app.state.tasks.begin("加载 每日推荐");
        assert_eq!(app.state.tasks.running(), 1);

        let arrived: crate::event::Event =
            crate::event::NavigationEvent::ContentLoaded(ContentState::Empty).into();
        app.state.events.send(arrived);
        app.handle_events().await.expect("events");

        assert_eq!(
            app.state.tasks.running(),
            0,
            "the arrival finished the load"
        );
        assert!(
            app.state
                .tasks
                .all()
                .all(|task| task.state == TaskState::Done),
            "and it is in the history as done"
        );
    }

    /// The settings page end to end: its key opens it, the page's own key layer walks its rows,
    /// and the row's change lands in the config key the row declares.
    #[tokio::test]
    async fn the_settings_page_changes_what_its_rows_name() {
        use crate::ui::settings::SETTINGS;

        let mut app = app();
        // Past the splash: it is the one page that ignores keys, so a session that presses `,` is
        // one that has already booted.
        app.state.navigation.page = Page::Main;

        // The page keys travel as navigation events, so the app has to take the loop's event step
        // for the page to actually change — which is exactly what the real loop does.
        press(&mut app, ',');
        app.handle_events().await.expect("events");
        assert_eq!(app.state.navigation.page, Page::Settings);

        // Walk to the last row of the table, which is a switch the page draws as 开/关.
        let Some(target) = SETTINGS
            .iter()
            .position(|setting| setting.label == "边听边存")
        else {
            panic!("the cache switch is a row of the settings page");
        };
        while app.state.settings.selected != target {
            press(&mut app, 'j');
        }

        let before = app.config.cache.save_on_play;
        press(&mut app, ' ');
        assert_eq!(
            app.config.cache.save_on_play, !before,
            "space on the row flips the key the row names"
        );

        // `Esc` leaves the page. It is the page's own key: the settings page is not in the content
        // breadcrumb stack, so the restore the main map sends has no breadcrumb to pop here — the
        // page used to stay up.
        press_key(&mut app, KeyCode::Esc);
        app.handle_events().await.expect("events");
        assert_eq!(
            app.state.navigation.page,
            Page::Main,
            "`Esc` closed the settings page"
        );
    }

    /// Enter on the hot-artists table reads the row as an artist: the id is what the page is
    /// opened for, and the name and portrait travel with it so the page has a header before
    /// its own request lands. Nothing else in the app's tables is an artist.
    #[tokio::test]
    async fn only_artist_rows_offer_an_artist_to_open() {
        use crate::{config::Config, state::ContentState};
        use ncm_api::SingerInfo;

        // Building the app builds its HTTP clients, which need the process-wide crypto
        // provider first — the same line `src/main.rs` runs at startup.
        let _ = rustls::crypto::ring::default_provider().install_default();
        let mut app = App::new(Config::default(), false).expect("app");
        app.state.navigation.content = ContentState::Singers(vec![
            SingerInfo {
                id: 6452,
                name: "周杰伦".into(),
                pic_url: "https://p3.music.126.net/portrait.jpg".into(),
            },
            // The API sends id 0 for a row it has no artist for.
            SingerInfo {
                id: 0,
                name: "未知歌手".into(),
                pic_url: String::new(),
            },
        ])
        .into();

        app.state.navigation.content_selected = 0;
        assert_eq!(
            selected_artist(&app),
            Some((
                6452,
                "周杰伦".to_string(),
                "https://p3.music.126.net/portrait.jpg".to_string()
            ))
        );

        app.state.navigation.content_selected = 1;
        assert_eq!(selected_artist(&app), None);

        // A song table's rows are not artists, however they are indexed.
        app.state.navigation.content = ContentState::Songs(Vec::new()).into();
        app.state.navigation.content_selected = 0;
        assert_eq!(selected_artist(&app), None);
    }

    /// A click has to map to the button that is drawn under it, for both controls row
    /// alignments, for the mode cell and for the hearts — the mapping is what the mouse
    /// handlers act on, so it is worth pinning down without a terminal.
    #[test]
    fn clicks_map_to_the_button_under_the_cursor() {
        let area = Rect::new(0, 4, 60, 1);
        let mode = Rect::new(58, 4, 1, 1);
        let likes = [Rect::new(2, 6, 1, 1), Rect::new(2, 7, 1, 1)];

        for centered in [true, false] {
            let transport = playerbar::control_rects(area, centered);
            let target = |rect: Rect| playerbar_target(&transport, mode, &likes, rect.x, rect.y);

            assert_eq!(
                target(transport[0].1),
                Some(PlayerbarTarget::Transport(playerbar::ControlButton::Prev)),
                "centered={centered}"
            );
            assert_eq!(
                target(transport[1].1),
                Some(PlayerbarTarget::Transport(
                    playerbar::ControlButton::PlayPause
                )),
                "centered={centered}"
            );
            assert_eq!(
                target(transport[2].1),
                Some(PlayerbarTarget::Transport(playerbar::ControlButton::Next)),
                "centered={centered}"
            );

            // The cell just past the last icon, and the gap between two of them, are dead.
            let last = transport[2].1;
            assert_eq!(target(Rect::new(last.right(), last.y, 1, 1)), None);
            let first = transport[0].1;
            assert_eq!(
                target(Rect::new(first.right(), first.y, 1, 1)),
                None,
                "the gap"
            );
        }

        // No transport buttons on screen, but the mode cell and both hearts still are.
        let none = [(playerbar::ControlButton::Prev, Rect::default()); 3];
        assert_eq!(
            playerbar_target(&none, mode, &likes, 58, 4),
            Some(PlayerbarTarget::Mode)
        );
        for heart in likes {
            assert_eq!(
                playerbar_target(&none, Rect::default(), &likes, heart.x, heart.y),
                Some(PlayerbarTarget::Like)
            );
        }
        assert_eq!(
            playerbar_target(&none, Rect::default(), &[Rect::default(); 2], 30, 20),
            None,
            "empty space is not a button"
        );
    }
}

#[cfg(test)]
mod nav_hit_area_lifetime_tests {
    //! Hit areas belong to the frame that drew them.
    use ratatui::{Terminal, backend::TestBackend};

    use crate::{
        config::{Config, NavPosition},
        state::{ContentState, Page},
    };

    fn singer(name: &str) -> ncm_api::SingerInfo {
        ncm_api::SingerInfo {
            id: 1,
            name: name.into(),
            pic_url: String::new(),
        }
    }

    fn left_click(col: u16, row: u16) -> crossterm::event::MouseEventKind {
        let _ = (col, row);
        crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left)
    }

    /// A terminal too narrow for the sidebar hides the navigation (`layout::main`), but the
    /// previous frame's areas used to survive it. `handle_click` asks the navigation before
    /// anything else, so the invisible items kept taking clicks that were meant for whatever
    /// is drawn there now — and switching the navigation position moves those stale areas to
    /// places the user would never connect to a navigation item.
    #[tokio::test]
    async fn a_navigation_that_is_not_drawn_takes_no_clicks() {
        let _ = rustls::crypto::ring::default_provider().install_default();

        let config = Config {
            navigation_position: NavPosition::Left,
            ..Config::default()
        };
        let mut app = crate::app::App::new(config, false).expect("app");
        app.state.navigation.page = Page::Main;
        app.state.navigation.content = ContentState::Singers(vec![singer("A"), singer("B")]).into();

        // Wide enough for the sidebar: it is drawn, and its second item has an area.
        let mut wide = Terminal::new(TestBackend::new(120, 40)).expect("backend");
        wide.draw(|f| crate::ui::draw(f, &mut app)).expect("draw");
        let second = app
            .state
            .navigation
            .nav
            .nav_hits
            .get(1)
            .map(|(_, _, rect)| *rect)
            .expect("the sidebar draws its items");
        let before = app
            .state
            .navigation
            .nav
            .selected_item()
            .map(|item| item.name.clone());

        // Narrow enough that the sidebar is not drawn at all.
        let mut narrow = Terminal::new(TestBackend::new(50, 20)).expect("backend");
        narrow.draw(|f| crate::ui::draw(f, &mut app)).expect("draw");
        assert!(
            app.state.navigation.nav.nav_hits.is_empty(),
            "the frame that does not draw the navigation must not leave its areas behind"
        );

        // Click exactly where that navigation item used to be.
        crate::input::handle_mouse_event(
            &mut app,
            left_click(second.x + 1, second.y),
            second.x + 1,
            second.y,
        );
        assert_eq!(
            app.state
                .navigation
                .nav
                .selected_item()
                .map(|item| item.name.clone()),
            before,
            "the click must not reach a navigation item that is no longer on screen"
        );
    }

    /// The wheel over the navigation moves its cursor, so every item stays reachable with the
    /// mouse even when the layout cannot show all of them — the sidebar scrolls only to
    /// follow the keyboard, and the row's offset is only ever changed by the selection.
    #[tokio::test]
    async fn the_wheel_over_the_navigation_reaches_every_item() {
        let _ = rustls::crypto::ring::default_provider().install_default();

        let config = Config {
            navigation_position: NavPosition::Top,
            ..Config::default()
        };
        let mut app = crate::app::App::new(config, false).expect("app");
        app.state.navigation.page = Page::Main;
        app.state.navigation.content = ContentState::Singers(vec![singer("A")]).into();

        let mut terminal = Terminal::new(TestBackend::new(120, 40)).expect("backend");
        terminal
            .draw(|f| crate::ui::draw(f, &mut app))
            .expect("draw");

        let total: usize = app
            .state
            .navigation
            .nav
            .sections
            .iter()
            .map(|section| section.items.len())
            .sum();
        let visible = app.state.navigation.nav.nav_hits.len();
        let area = app.state.nav_area;
        assert!(area.width > 0, "the row mode draws its navigation");
        assert!(
            visible < total,
            "this test needs a nav the row cannot fit: {visible} of {total}"
        );

        let mut reached = std::collections::HashSet::new();
        for _ in 0..total {
            crate::input::handle_mouse_event(
                &mut app,
                crossterm::event::MouseEventKind::ScrollDown,
                area.x + 1,
                area.y,
            );
            // One event, one frame: the navigation's scroll offset follows the draw.
            terminal
                .draw(|f| crate::ui::draw(f, &mut app))
                .expect("draw");
            let nav = &app.state.navigation.nav;
            let selected = nav.section_states[nav.focus_section]
                .selected()
                .unwrap_or(0);
            reached.insert((nav.focus_section, selected));
        }

        assert_eq!(
            reached.len(),
            total,
            "the wheel must reach every item, including the ones the row had no room for"
        );
    }
}
