use std::time::Duration;

use crossterm::event::{Event as CrosstermEvent, MouseEventKind};
use tokio::time::sleep;

use super::App;
use crate::{
    event::{
        AppEvent, AuthEvent, CommandEvent, CommandPanelAction, Event, NavigationEvent,
        PlaybackEvent, SplashEvent,
    },
    input,
    state::{ContentState, PanelAction},
};

/// How much of a song the cloud counts as a listen; the official client reports around
/// this point, so there is no reason to sit through the whole track.
const LISTEN_THRESHOLD: Duration = Duration::from_secs(30);

/// A tick that advances more than this did not play — the position was seeked.
const SEEK_JUMP: Duration = Duration::from_secs(1);

/// How many already-queued events one frame may consume. Bounded so a producer that never
/// stops (a mouse that keeps moving, a paste storm) cannot starve the draw: whatever is
/// left waits for the next frame, which is a millisecond away.
const MAX_EVENTS_PER_FRAME: usize = 256;

/// What one progress tick adds to the played total. Progress reports the player position,
/// so a jump means the bar was dragged: seeking past the threshold is not listening to it.
fn played_advance(previous: Duration, position: Duration) -> Duration {
    let advanced = position.saturating_sub(previous);
    if advanced <= SEEK_JUMP {
        advanced
    } else {
        Duration::ZERO
    }
}

/// Whether the played total means the cloud should be told, once per play.
fn listen_due(played: Duration, song_id: u64, reported: Option<u64>) -> bool {
    played >= LISTEN_THRESHOLD && reported != Some(song_id)
}

impl App {
    /// Report the listen the cloud counts, the way the official client does: about thirty
    /// seconds in rather than at the end, and once per play.
    fn report_listen(&mut self, position: Duration) {
        // Progress ticks the player position, so accumulate what actually played: seeking
        // only moves the position and must not count towards the threshold.
        self.played_in_track += played_advance(self.last_position, position);
        self.last_position = position;

        let Some(song_id) = self
            .playback
            .state
            .current_song
            .as_ref()
            .map(|song| song.id)
        else {
            return;
        };
        if !listen_due(self.played_in_track, song_id, self.reported_listen) {
            return;
        }
        self.reported_listen = Some(song_id);

        let service = self.service.clone();
        let played_ms = self.played_in_track.as_millis() as u64;
        tokio::spawn(async move {
            if let Err(error) = service.report_play(song_id, played_ms).await {
                log::debug!("report_play({song_id}): {error}");
            }
        });
    }

    /// The loop's event step: what a frame's worth of queued events does to the app. `pub(crate)`
    /// because driving it is how a test presses a key and sees what the keyboard did — the same
    /// entry point `App::run` uses.
    pub(crate) async fn handle_events(&mut self) -> color_eyre::Result<()> {
        if self.playback.state.seeking {
            tokio::select! {
                biased;
                result = self.state.events.next() => {
                    self.dispatch_event(result?).await?;
                }
                _ = sleep(Duration::from_millis(32)) => {}
            }
        } else {
            let event = self.state.events.next().await?;
            self.dispatch_event(event).await?;
        }

        // The caller draws exactly one frame per iteration, so an event that is already
        // queued would buy another whole frame of its own. That matters most for the one
        // event a terminal produces fastest: with `?1003h` in effect every mouse movement
        // is reported, hundreds per second, and each one used to cost a full redraw of a
        // frame that could not have changed. Handle what has already arrived, then let the
        // caller draw once for all of it.
        for _ in 0..MAX_EVENTS_PER_FRAME {
            let Some(event) = self.state.events.try_next() else {
                break;
            };
            self.dispatch_event(event).await?;
        }
        Ok(())
    }

    async fn dispatch_event(&mut self, event: Event) -> color_eyre::Result<()> {
        match event {
            Event::Crossterm(event) => match event {
                CrosstermEvent::Key(key) if key.kind == crossterm::event::KeyEventKind::Press => {
                    input::handle_key_events(self, key.into())?
                }
                // Bracketed paste (`enable_terminal_modes`): the block is a paste, not
                // typing, so its newlines must not press Enter.
                CrosstermEvent::Paste(text) => input::handle_paste(self, &text),
                // `?1003h` reports every movement of the mouse, and nothing in the UI reacts
                // to one (there is no hover state to update) — while the frame after it is
                // redrawn in full regardless. Motion therefore falls through to the arm below.
                // Dragging is different: that is how the seek bar and the volume are scrubbed.
                CrosstermEvent::Mouse(mouse) if mouse.kind != MouseEventKind::Moved => {
                    input::handle_mouse_event(self, mouse.kind, mouse.column, mouse.row);
                }
                _ => {}
            },
            Event::App(app_event) => match app_event {
                AppEvent::Quit => self.quit(),
                AppEvent::Splash(e) => self.handle_splash_event(e),
                AppEvent::Auth(e) => self.handle_auth_event(e),
                AppEvent::Playback(e) => self.handle_playback_event(e),
                AppEvent::Navigation(e) => self.handle_navigation_event(e),
                AppEvent::Command(e) => self.handle_command_event(e),
                AppEvent::Toast(msg) => self.toast(msg),
                AppEvent::Ipc(e) => self.handle_ipc_event(e).await,
                // Nothing to do: waking the loop so it redraws is the whole point.
                AppEvent::Repaint => {}
            },
        }
        Ok(())
    }

    fn handle_splash_event(&mut self, event: SplashEvent) {
        match event {
            SplashEvent::Tick { progress, log } => self.handle_splash_tick(progress, log),
            SplashEvent::SetOffline => self.state.offline = true,
        }
    }

    fn handle_auth_event(&mut self, event: AuthEvent) {
        match event {
            AuthEvent::Login => self.handle_login(),
            AuthEvent::Success(info) => self.handle_login_success(info),
            AuthEvent::LoggedOut => self.handle_logout_done(),
            AuthEvent::Error(e) => self.handle_login_error(e),
            AuthEvent::QRCreated { url, key } => self.handle_qr_created(url, key),
            AuthEvent::QRStatus(text) => self.handle_qr_status(text),
            AuthEvent::Submit(method) => self.handle_login_submit(method),
            AuthEvent::SmsCodeSent(phone) => self.handle_sms_code_sent(phone),
            AuthEvent::ActionResult(result) => self.handle_action_result(result),
        }
    }

    fn handle_playback_event(&mut self, event: PlaybackEvent) {
        match event {
            PlaybackEvent::SongPlay(id) => self.handle_song_play(id),
            PlaybackEvent::Started => {
                self.reported_listen = None;
                self.played_in_track = Duration::ZERO;
                self.last_position = Duration::ZERO;
                self.handle_playback_started();
                self.notify_song_change();
            }
            PlaybackEvent::Progress { position, total } => {
                self.playback.on_playback_progress(position, total);
                self.report_listen(position);
            }
            PlaybackEvent::Finished => {
                // The cloud only counts a listen when the client reports it, and that record
                // is what feeds 最近播放 and the recommendations. Long songs were already
                // reported thirty seconds in; this catches the ones too short to reach it,
                // where a completed play is the whole song.
                let finished = self.playback.finish_and_snapshot();
                if let Some((song_id, duration_ms, progress)) = finished
                    && progress >= 0.9
                    && Duration::from_millis(duration_ms) < LISTEN_THRESHOLD
                {
                    let service = self.service.clone();
                    tokio::spawn(async move {
                        let _ = service.report_play(song_id, duration_ms).await;
                    });
                }
            }
            PlaybackEvent::Error(e) => {
                if self.config.notify.errors {
                    let _ = crate::utils::terminal::notify(&mut std::io::stdout(), "", &e);
                }
                self.playback.on_playback_error(e);
            }
            PlaybackEvent::LyricsLoaded {
                song_id,
                lyrics,
                translated_lyrics,
            } => self
                .playback
                .on_lyrics_loaded(song_id, lyrics, translated_lyrics),
            PlaybackEvent::HeartbeatSong(song) => {
                self.playback.play_heartbeat_song(song);
            }
            PlaybackEvent::HeartbeatFallback => {
                self.playback.on_heartbeat_fallback();
            }
            PlaybackEvent::SetPlaylistId(id) => {
                // After the content (re)loads, the previous "全量已入队" marker is stale.
                self.queued_playlists.remove(&id);
                self.playback.set_playlist_id(id);
            }
            PlaybackEvent::LikeSong(id, like) => {
                // Update the local set immediately and refresh the icon (regardless of the cloud result, consistent with existing behavior).
                if let Ok(mut guard) = self.liked_ids.lock() {
                    if like {
                        guard.insert(id);
                    } else {
                        guard.remove(&id);
                    }
                }
                if self
                    .playback
                    .state
                    .current_song
                    .as_ref()
                    .is_some_and(|s| s.id == id)
                {
                    self.playback.update_liked_status();
                }
                let is_liked_root = self.state.navigation.nav.selected_api() == Some("liked")
                    && self.state.navigation.history.is_empty();
                if !like && is_liked_root {
                    // The active list and its cache are snapshots; keep them in sync with the
                    // optimistic liked-state update instead of showing the removed song until reload.
                    self.service.cache().remove_content_cache("liked");
                    let playlist_id = self
                        .state
                        .navigation
                        .pagination
                        .as_ref()
                        .and_then(|p| p.api.strip_prefix("playlist:"))
                        .and_then(|id| id.parse::<u64>().ok());
                    if self.state.navigation.remove_song(id)
                        && let Some(playlist_id) = playlist_id
                    {
                        self.service.remove_cached_playlist_track(playlist_id, id);
                    }
                } else if is_liked_root {
                    // A like can target the current playing song (`S`) without it being the
                    // selected row. Insert the newly liked song at the top of the open liked
                    // list so the visible list reflects the change immediately.
                    let already_present = matches!(
                        self.state.navigation.content.as_ref(),
                        ContentState::Songs(songs) if songs.iter().any(|song| song.id == id)
                    );
                    if !already_present
                        && let Some(song) =
                            self.playback.current_song().filter(|song| song.id == id)
                    {
                        let playlist_id = self
                            .state
                            .navigation
                            .pagination
                            .as_ref()
                            .and_then(|p| p.api.strip_prefix("playlist:"))
                            .and_then(|id| id.parse::<u64>().ok());
                        if self.state.navigation.insert_song_at_top(song)
                            && let Some(playlist_id) = playlist_id
                        {
                            self.service.insert_cached_playlist_track(playlist_id, id);
                        }
                    }
                }
                let service = self.service.clone();
                tokio::spawn(async move {
                    if let Err(e) = service.like_song(id, like).await {
                        log::warn!("Failed to update liked state for song {id}: {e}");
                    }
                });
            }
            PlaybackEvent::LikedUpdated => {
                self.playback.update_liked_status();
            }
            PlaybackEvent::DislikeSong(id) => {
                let service = self.service.clone();
                tokio::spawn(async move {
                    match service.dislike_song(id).await {
                        Ok(_) => {}
                        Err(e) => log::warn!("Dislike failed: {e}"),
                    }
                });
            }
            PlaybackEvent::Cached(song_id) => {
                if self
                    .playback
                    .state
                    .current_song
                    .as_ref()
                    .is_some_and(|s| s.id == song_id)
                {
                    self.playback.state.cached = true;
                }
            }
            PlaybackEvent::PlaySongs { key, songs, index } => {
                self.playback.play_songs(&key, songs, index);
            }
            PlaybackEvent::QueueAppend { key, songs } => {
                self.playback.append_songs_to_key(&key, songs);
            }
            PlaybackEvent::QueueLoadDone { playlist_id } => {
                self.queued_playlists.insert(playlist_id);
            }
        }
    }

    fn handle_navigation_event(&mut self, event: NavigationEvent) {
        match event {
            NavigationEvent::NavSelect(api_str) => {
                // The work starts here: a navigation request is what the app asks for, and the
                // content that arrives is what ends it (see the match arms below).
                let label = self
                    .config
                    .navigation
                    .name_for_api(&api_str)
                    .map(|name| format!("加载 {name}"))
                    .unwrap_or_else(|| format!("加载 {api_str}"));
                self.state.tasks.begin(label);

                if let Err(e) = self.handle_nav_select(api_str, false) {
                    log::error!("NavSelect error: {e}");
                }
            }
            NavigationEvent::ContentLoaded(content) => {
                self.state
                    .tasks
                    .finish_running(crate::state::tasks::TaskState::Done);
                self.handle_content_loaded(content)
            }
            NavigationEvent::ContentLoadedPaged {
                content,
                pagination,
                generation,
            } => {
                self.handle_content_loaded_paged(content, pagination, generation);
            }
            NavigationEvent::PlaylistSelect { id, name } => self.handle_playlist_select(id, name),
            NavigationEvent::BreadcrumbSet(name) => self.handle_breadcrumb(name),
            NavigationEvent::SearchSong(keyword) => self.handle_search_song(keyword),
            NavigationEvent::Navigate(page) => self.state.navigation.page = page,
            NavigationEvent::SearchActivated => self.handle_search_activate(),
            NavigationEvent::SearchDeactivated => self.handle_search_deactivate(),
            NavigationEvent::ContentRestore => self.handle_content_restore(),
            NavigationEvent::OpenAlbum { id, name } => self.open_album(id, name),
            NavigationEvent::OpenSongContext { song_id, similar } => {
                self.open_song_context(song_id, similar)
            }
            NavigationEvent::CellAction(row, col) => {
                if let Err(e) = self.handle_cell_action(row, col) {
                    log::error!("CellAction error: {e}");
                }
            }
            NavigationEvent::LoadMore => self.handle_load_more(),
            NavigationEvent::LoadMoreFailed => {
                if let Some(ref mut pg) = self.state.navigation.pagination {
                    pg.loading = false;
                }
            }
            NavigationEvent::UploadCachedSong(row) => self.handle_upload_cached_song(row),
        }
    }

    fn handle_command_event(&mut self, event: CommandEvent) {
        match event {
            CommandEvent::Panel(action) => self.handle_command_panel(action),
        }
    }

    fn handle_command_panel(&mut self, action: CommandPanelAction) {
        let panel = &mut self.state.command_panel;
        match action {
            CommandPanelAction::Open => {
                panel.open = true;
                panel.selected = 0;
            }
            CommandPanelAction::Close => panel.back(),
            CommandPanelAction::Previous => {
                if let Some(items) = panel.current_items() {
                    let len = items.len();
                    panel.selected = (panel.selected + len - 1) % len;
                }
            }
            CommandPanelAction::Next => {
                if let Some(items) = panel.current_items() {
                    let len = items.len();
                    panel.selected = (panel.selected + 1) % len;
                }
            }
            CommandPanelAction::Select => {
                let command = panel.enter();
                if command.is_some() {
                    panel.open = false;
                }
                match command {
                    // The palette runs exactly what `:` would: one vocabulary, one executor, so
                    // the two cannot drift apart again.
                    Some(PanelAction::Run(line)) => {
                        match crate::input::ex::ExCommand::parse(&line) {
                            Ok(command) => {
                                if let Err(error) = crate::input::ex::execute(self, command) {
                                    self.notice(
                                        crate::state::notices::Level::Error,
                                        format!("E: {error}"),
                                    );
                                }
                            }
                            Err(error) => self
                                .notice(crate::state::notices::Level::Error, format!("E: {error}")),
                        }
                    }
                    // One that needs an argument opens the command line ready for it, rather than
                    // running and reporting a missing value.
                    Some(PanelAction::Prefill(text)) => {
                        crate::input::ex::open(self);
                        crate::input::handle_paste(self, &text);
                    }
                    None => {}
                }
            }
        }
    }

    /// Tell the terminal what started playing. Off unless the config asks for it, and the
    /// sequence is out-of-band: it changes no cells, so it cannot disturb the frame being
    /// drawn around it.
    fn notify_song_change(&self) {
        if !self.config.notify.song_change {
            return;
        }
        let Some(song) = &self.playback.state.current_song else {
            return;
        };
        let _ = crate::utils::terminal::notify(&mut std::io::stdout(), &song.name, &song.singer);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The listen rule the cloud cares about: a listen is counted about thirty seconds in,
    /// once per play, and a different song is a different listen.
    #[test]
    fn a_listen_is_reported_once_past_the_threshold() {
        let below = LISTEN_THRESHOLD - Duration::from_secs(1);
        assert!(!listen_due(below, 42, None), "not a listen yet");
        assert!(listen_due(LISTEN_THRESHOLD, 42, None), "at the threshold");
        assert!(
            !listen_due(LISTEN_THRESHOLD, 42, Some(42)),
            "the same play is only reported once"
        );
        assert!(
            listen_due(LISTEN_THRESHOLD, 7, Some(42)),
            "the next song counts again"
        );
    }

    /// Listening is measured in playback, not in position: dragging the bar past the
    /// threshold must not fake a listen, which is what reporting the raw position did.
    #[test]
    fn seeking_past_the_threshold_is_not_a_listen() {
        let tick = Duration::from_millis(80);

        assert_eq!(
            played_advance(Duration::from_secs(5), Duration::from_secs(5) + tick),
            tick,
            "a normal tick counts"
        );
        assert_eq!(
            played_advance(Duration::from_secs(5), Duration::from_secs(90)),
            Duration::ZERO,
            "a forward jump is a seek"
        );
        assert_eq!(
            played_advance(Duration::from_secs(90), Duration::from_secs(5)),
            Duration::ZERO,
            "seeking back plays nothing"
        );

        // 375 ticks of 80ms is exactly the threshold.
        let mut total = Duration::ZERO;
        let mut previous = Duration::ZERO;
        for _ in 0..375 {
            let position = previous + tick;
            total += played_advance(previous, position);
            previous = position;
        }
        assert_eq!(total, LISTEN_THRESHOLD);
        assert!(listen_due(total, 42, None));
    }
}
