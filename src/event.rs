//! Event model and the `EventHandler` channel that bridges async workers and the
//! main render loop (`Event`, `AppEvent`, `AuthEvent`, `PlaybackEvent`,
//! `NavigationEvent`, ...).

use std::{sync::Arc, time::Duration};

use color_eyre::eyre::OptionExt;
use crossterm::event::Event as CrosstermEvent;
use futures::{FutureExt, StreamExt};
use ncm_api::{LoginInfo, SongInfo};
use tokio::sync::mpsc;

use crate::{
    ipc::IpcEvent,
    playback::LyricLine,
    state::{ContentState, LoginMethod, Page, PaginationInfo, SplashLogEntry},
};

#[derive(Clone, Debug)]
pub enum Event {
    Crossterm(CrosstermEvent),
    App(AppEvent),
}

#[derive(Clone, Debug)]
pub enum AppEvent {
    Quit,
    Splash(SplashEvent),
    Auth(AuthEvent),
    Playback(PlaybackEvent),
    Navigation(NavigationEvent),
    Command(CommandEvent),
    Toast(String),
    /// Control request received over the IPC socket (`boxpigma msg`).
    Ipc(IpcEvent),
    /// The frame on screen is stale: application state did not change, but something the app
    /// draws did — a background task filled in a channel that nobody owns until the next draw.
    /// Handling it does nothing; its whole job is to wake the main loop so it redraws.
    Repaint,
}

#[derive(Clone, Debug)]
pub enum SplashEvent {
    Tick {
        progress: f64,
        log: Option<SplashLogEntry>,
    },
    SetOffline,
}

#[derive(Clone, Debug)]
pub enum AuthEvent {
    Login,
    /// The session is gone; the UI has to forget the user it was showing.
    LoggedOut,
    Success(LoginInfo),
    Error(String),
    QRCreated {
        url: String,
        key: String,
    },
    QRStatus(String),
    /// A submit from the login page's form, carrying the method the page was showing. The
    /// handler reads the values out of `LoginState` rather than out of the event, so what goes
    /// to the server is what is on screen at the moment `Enter` is pressed.
    Submit(LoginMethod),
    /// The SMS code went out to that number: the second step has to use the same one, so the
    /// page keeps it and moves the caret to the code box.
    SmsCodeSent(String),
    /// The result of something the page itself did, which belongs in the page's own line and
    /// not in a toast: `Ok` is drawn in accent, `Err` in the error colour.
    ActionResult(Result<String, String>),
}

#[derive(Clone, Debug)]
pub enum PlaybackEvent {
    SongPlay(u64),
    Started,
    Progress {
        position: Duration,
        total: Option<Duration>,
    },
    Finished,
    Error(String),
    LyricsLoaded {
        song_id: u64,
        lyrics: Vec<LyricLine>,
        translated_lyrics: Vec<LyricLine>,
    },
    HeartbeatSong(SongInfo),
    HeartbeatFallback,
    SetPlaylistId(u64),
    LikeSong(u64, bool),
    DislikeSong(u64),
    Cached(u64),
    /// The cloud "我喜欢的音乐" list has been fetched/updated; the main thread refreshes `PlaybackState.liked` from it.
    LikedUpdated,
    /// Append lazily-paged songs to the queue identified by `key` (background
    /// full-list load after Enter on a playlist page). Songs are shared `Arc`s
    /// so appending never deep-clones the page.
    QueueAppend {
        key: String,
        songs: Vec<Arc<SongInfo>>,
    },
    /// Play these songs as the queue and start the one at `index`. A list that was fetched
    /// somewhere other than the content table — the private FM — arrives this way, so the
    /// fetch can happen off the main thread and the playing still does not.
    PlaySongs {
        key: String,
        songs: Vec<Arc<SongInfo>>,
        index: usize,
    },
    /// The full song list for `playlist_id` has been queued; subsequent Enter
    /// presses on the same playlist can jump within the queue without reloading.
    QueueLoadDone {
        playlist_id: u64,
    },
}

#[derive(Clone, Debug)]
pub enum NavigationEvent {
    NavSelect(String),
    ContentLoaded(ContentState),
    ContentLoadedPaged {
        content: ContentState,
        pagination: PaginationInfo,
        generation: u64,
    },
    PlaylistSelect {
        id: u64,
        name: Option<String>,
    },
    BreadcrumbSet(String),
    SearchSong(String),
    Navigate(Page),
    SearchActivated,
    SearchDeactivated,
    ContentRestore,
    /// Open the neighbourhood of one song as content: the songs similar to it, or the playlists
    /// that contain it. It is opened *from* wherever the song is playing, so the page that asked
    /// is the way back.
    OpenSongContext {
        song_id: u64,
        similar: bool,
    },
    /// Open one album as table content. The artist page is the only sender: an album row is
    /// not a row of the main table, so opening it is a load this app has to start itself.
    OpenAlbum {
        id: u64,
        name: String,
    },
    CellAction(usize, usize),
    LoadMore,
    /// A paged load failed; clears the in-flight flag so it can be retried.
    LoadMoreFailed,
    UploadCachedSong(usize),
}

#[derive(Clone, Debug)]
pub enum CommandEvent {
    Panel(CommandPanelAction),
}

#[derive(Clone, Debug)]
pub enum CommandPanelAction {
    Open,
    Close,
    Previous,
    Next,
    Select,
}

/// Sub-event enums convert directly into `Event` (callers need this path when using `.into()`).
/// `From<$sub> for AppEvent` is no longer generated because all callers go straight
/// `.into()` → `Event` without the `AppEvent` intermediate step. `From<AppEvent> for Event`
/// is kept separately for cases that construct an `AppEvent` variant directly, such as `AppEvent::Quit`.
macro_rules! impl_from_sub_event {
    ($variant:ident, $sub:ty) => {
        impl From<$sub> for Event {
            fn from(e: $sub) -> Self {
                Event::App(AppEvent::$variant(e))
            }
        }
    };
}

impl_from_sub_event!(Splash, SplashEvent);
impl_from_sub_event!(Auth, AuthEvent);
impl_from_sub_event!(Playback, PlaybackEvent);
impl_from_sub_event!(Navigation, NavigationEvent);
impl_from_sub_event!(Command, CommandEvent);

impl From<AppEvent> for Event {
    fn from(e: AppEvent) -> Self {
        Event::App(e)
    }
}

/// Bridges crossterm terminal events into an internal mpsc event channel.
pub struct EventHandler {
    sender: mpsc::UnboundedSender<Event>,
    receiver: mpsc::UnboundedReceiver<Event>,
}

impl Default for EventHandler {
    fn default() -> Self {
        Self::new(true)
    }
}

impl EventHandler {
    /// Create an event channel. With `with_terminal = true` a task reading
    /// crossterm terminal events is spawned; pass `false` for headless daemon
    /// mode (no terminal available).
    pub fn new(with_terminal: bool) -> Self {
        // Unbounded on purpose. The consumer is the main loop, which takes one event per
        // frame (a draw measured at ~230 µs) and can therefore drain thousands per second,
        // while every producer is bounded by real work: input, HTTP calls that carry their own
        // timeouts, and the analysis stream at ~30 Hz. A bounded channel would add a
        // back-pressure path nothing needs; when the receiver is gone (the app is quitting)
        // a send simply fails, which every sender here already ignores.
        let (sender, receiver) = mpsc::unbounded_channel();
        if with_terminal {
            let actor = EventTask::new(sender.clone());
            tokio::spawn(async { actor.run().await });
        }
        Self { sender, receiver }
    }

    pub async fn next(&mut self) -> color_eyre::Result<Event> {
        self.receiver
            .recv()
            .await
            .ok_or_eyre("Failed to receive event")
    }

    /// Take the next event if one is already queued, without waiting for one.
    ///
    /// The main loop draws one frame per iteration, so every event that is already waiting
    /// would otherwise buy another whole frame — see [`App::handle_events`] for the case
    /// that made this necessary.
    ///
    /// [`App::handle_events`]: crate::app::App
    pub fn try_next(&mut self) -> Option<Event> {
        self.receiver.try_recv().ok()
    }

    pub fn send<E: Into<Event>>(&mut self, event: E) {
        if let Err(e) = self.sender.send(event.into()) {
            log::error!("Failed to send event: {}", e);
        }
    }

    pub fn sender(&self) -> mpsc::UnboundedSender<Event> {
        self.sender.clone()
    }
}

struct EventTask {
    sender: mpsc::UnboundedSender<Event>,
}

impl EventTask {
    fn new(sender: mpsc::UnboundedSender<Event>) -> Self {
        Self { sender }
    }

    async fn run(self) -> color_eyre::Result<()> {
        let mut reader = crossterm::event::EventStream::new();
        loop {
            let crossterm_event = reader.next().fuse();

            tokio::select! {
              _ = self.sender.closed() => {
                break;
              }
              Some(Ok(evt)) = crossterm_event => {
                self.send(Event::Crossterm(evt));
              }
            };
        }
        Ok(())
    }

    fn send(&self, event: Event) {
        if self.sender.send(event).is_err() {
            log::error!("EventTask: failed to send event: receiver dropped");
        }
    }
}
