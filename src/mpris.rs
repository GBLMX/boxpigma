//! Desktop media session: the MPRIS service (`org.mpris.MediaPlayer2`).
//!
//! Linux desktops drive players over D-Bus rather than over a private socket: `playerctl`,
//! the media widgets of KDE / GNOME / waybar, and the media keys on a keyboard or a
//! Bluetooth headset all speak this interface. Commands land in the same event queue the IPC
//! socket feeds (`Event::App(AppEvent::Ipc(..))`), so `playerctl pause` and `boxpigma msg
//! pause` are one implementation rather than two, and the properties are published from the
//! same [`StatusSnapshot`] `boxpigma status` serves.
//!
//! Where MPRIS has no exact counterpart, the mapping is deliberate:
//!
//! - `Stop` is `Pause` plus a jump to the start of the track — the queue has no "stopped"
//!   state of its own, and stopping playback is what a client means by it.
//! - `LoopStatus` and `Shuffle` are two bits of a five-valued mode. `heartbeat` reports as
//!   `Playlist` + `Shuffle`; writing either property composes the mode those two bits name,
//!   so a client that touches them leaves heartbeat (the one mode MPRIS cannot express).
//!
//! Nothing here touches the bus until [`MprisService::run`] is called from a run loop:
//! `App::new` also runs in tests, where there is no tokio runtime and no session bus.

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use tokio::sync::mpsc;
use zbus::{
    Connection,
    object_server::InterfaceRef,
    zvariant::{ObjectPath, OwnedValue, Value},
};

use crate::{
    event::{AppEvent, Event},
    ipc::{IpcEvent, StatusSnapshot},
    playback::PlayMode,
};

/// Object path both MPRIS interfaces are served on.
const PATH: &str = "/org/mpris/MediaPlayer2";
/// The well-known name `playerctl` and the desktop media widgets look for.
const BUS_NAME: &str = "org.mpris.MediaPlayer2.boxpigma";
/// The player interface, for the one signal that is not a property change.
const PLAYER_IFACE: &str = "org.mpris.MediaPlayer2.Player";
/// Where `mpris:trackid` object paths live: one per song.
const TRACK_PATH: &str = "/org/mpris/MediaPlayer2/Track";

/// What the interfaces report: the snapshot the IPC socket serves, the play mode (MPRIS
/// splits it into two properties) and the cached cover path (clients show it; the socket has
/// no use for it).
struct State {
    snapshot: StatusSnapshot,
    mode: PlayMode,
    art_url: Option<String>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            snapshot: StatusSnapshot::default(),
            mode: PlayMode::Sequential,
            art_url: None,
        }
    }
}

/// The app-side half of the service: where a snapshot is published, and the means to wake the
/// task that turns one into `PropertiesChanged` signals.
#[derive(Clone)]
pub struct MprisHandle {
    state: Arc<Mutex<State>>,
    updated: mpsc::UnboundedSender<()>,
}

/// The D-Bus half, owned by the task a run loop spawns.
pub struct MprisService {
    state: Arc<Mutex<State>>,
    updated: mpsc::UnboundedReceiver<()>,
}

impl MprisHandle {
    /// The pair: a handle for the app, a service for the run loop to spawn.
    pub fn new() -> (Self, MprisService) {
        let state = Arc::new(Mutex::new(State::default()));
        let (updated, receiver) = mpsc::unbounded_channel();
        (
            Self {
                state: Arc::clone(&state),
                updated,
            },
            MprisService {
                state,
                updated: receiver,
            },
        )
    }

    /// Publish the current status. Called from `App::update_status_snapshot` every loop
    /// iteration, so this stays cheap: one lock, one send, no bus traffic — the service task
    /// diffs and announces only what moved.
    pub fn update(&self, snapshot: &StatusSnapshot, mode: PlayMode, art_url: Option<PathBuf>) {
        let art_url = art_url
            .and_then(|path| url::Url::from_file_path(&path).ok())
            .map(|url| url.to_string());
        if let Ok(mut state) = self.state.lock() {
            state.snapshot = snapshot.clone();
            state.mode = mode;
            state.art_url = art_url;
        }
        let _ = self.updated.send(());
    }
}

/// What MPRIS can see of [`State`], gathered so a change can be diffed and only the
/// properties that moved are announced.
#[derive(Clone, PartialEq)]
struct Props {
    playback_status: &'static str,
    loop_status: &'static str,
    shuffle: bool,
    volume: f64,
    track: Track,
}

/// The metadata of what is playing, in the MPRIS vocabulary.
#[derive(Clone, PartialEq)]
struct Track {
    id: u64,
    title: String,
    artist: String,
    album: String,
    length_us: i64,
    art_url: Option<String>,
}

impl Props {
    fn of(state: &State) -> Self {
        let snapshot = &state.snapshot;
        Self {
            playback_status: playback_status(snapshot),
            loop_status: loop_status(state.mode),
            shuffle: shuffle(state.mode),
            volume: snapshot.volume,
            track: Track {
                id: snapshot.id,
                title: snapshot.name.clone(),
                artist: snapshot.artist.clone(),
                album: snapshot.album.clone(),
                length_us: snapshot.duration_ms as i64 * 1000,
                art_url: state.art_url.clone(),
            },
        }
    }
}

/// `Playing` / `Paused` / `Stopped`, from the two flags the snapshot carries.
fn playback_status(snapshot: &StatusSnapshot) -> &'static str {
    match (snapshot.playing, snapshot.paused) {
        (true, false) => "Playing",
        (true, true) => "Paused",
        _ => "Stopped",
    }
}

/// The MPRIS view of the mode: `heartbeat` is a playlist shuffled by the player itself.
fn loop_status(mode: PlayMode) -> &'static str {
    match mode {
        PlayMode::RepeatOne => "Track",
        PlayMode::RepeatAll => "Playlist",
        _ if shuffle(mode) => "Playlist",
        _ => "None",
    }
}

fn shuffle(mode: PlayMode) -> bool {
    matches!(mode, PlayMode::Shuffle | PlayMode::Heartbeat { .. })
}

/// The mode the two MPRIS bits name. `heartbeat` has no representation here, so writing
/// either property is what leaves it.
fn mode_for(loop_status: &str, shuffle: bool) -> PlayMode {
    match (loop_status, shuffle) {
        // A repeat of one track wins over the shuffle bit: it is what makes the two bits
        // name exactly one of the four modes a client can express.
        ("Track", _) => PlayMode::RepeatOne,
        ("Playlist", false) => PlayMode::RepeatAll,
        (_, true) => PlayMode::Shuffle,
        _ => PlayMode::Sequential,
    }
}

/// The `mpris:trackid` object path for a song.
fn track_path(song_id: u64) -> String {
    format!("{TRACK_PATH}/{song_id}")
}

/* -------------------------------------------------------------------------- */
/*                                the interfaces                              */
/* -------------------------------------------------------------------------- */

#[derive(Clone)]
enum EventTarget {
    Legacy(mpsc::UnboundedSender<Event>),
    Actor(mpsc::UnboundedSender<IpcEvent>),
}

impl EventTarget {
    fn send(&self, event: IpcEvent) {
        match self {
            Self::Legacy(sender) => {
                let _ = sender.send(AppEvent::Ipc(event).into());
            }
            Self::Actor(sender) => {
                let _ = sender.send(event);
            }
        }
    }

    fn quit(&self) {
        if let Self::Legacy(sender) = self {
            let _ = sender.send(AppEvent::Quit.into());
        }
    }
}

/// `org.mpris.MediaPlayer2`: who the player is, and how to quit it.
struct MediaPlayer2 {
    target: EventTarget,
}

#[zbus::interface(name = "org.mpris.MediaPlayer2")]
impl MediaPlayer2 {
    /// Quit, through the same `AppEvent::Quit` the `q` key and SIGTERM take (session save
    /// and cookie flush included).
    fn quit(&self) {
        self.target.quit();
    }

    fn raise(&self) {}

    #[zbus(property(emits_changed_signal = "false"))]
    fn can_quit(&self) -> bool {
        true
    }

    #[zbus(property(emits_changed_signal = "false"))]
    fn can_raise(&self) -> bool {
        false
    }

    #[zbus(property(emits_changed_signal = "false"))]
    fn has_track_list(&self) -> bool {
        false
    }

    #[zbus(property(emits_changed_signal = "false"))]
    fn identity(&self) -> String {
        "boxpigma".to_string()
    }

    #[zbus(property(emits_changed_signal = "false"))]
    fn desktop_entry(&self) -> String {
        "boxpigma".to_string()
    }

    #[zbus(property(emits_changed_signal = "false"))]
    fn supported_uri_schemes(&self) -> Vec<String> {
        Vec::new()
    }

    #[zbus(property(emits_changed_signal = "false"))]
    fn supported_mime_types(&self) -> Vec<String> {
        Vec::new()
    }
}

/// `org.mpris.MediaPlayer2.Player`: transport, level, mode and metadata.
struct Player {
    state: Arc<Mutex<State>>,
    target: EventTarget,
    /// Told about a seek so the service task can emit `Seeked`: the interface methods have no
    /// connection of their own.
    seeked: mpsc::UnboundedSender<i64>,
}

impl Player {
    /// Send a control request into the app loop — the same path the IPC socket uses.
    fn send(&self, event: IpcEvent) {
        self.target.send(event);
    }

    /// The snapshot the properties read from; a poisoned lock is not worth taking the media
    /// session down for, so it reports the empty state instead.
    fn snapshot(&self) -> StatusSnapshot {
        self.state
            .lock()
            .map(|state| state.snapshot.clone())
            .unwrap_or_default()
    }

    fn mode(&self) -> PlayMode {
        self.state
            .lock()
            .map(|state| state.mode)
            .unwrap_or(PlayMode::Sequential)
    }
}

#[zbus::interface(name = "org.mpris.MediaPlayer2.Player")]
impl Player {
    fn next(&self) {
        self.send(IpcEvent::Next);
    }

    fn previous(&self) {
        self.send(IpcEvent::Previous);
    }

    fn pause(&self) {
        self.send(IpcEvent::Pause);
    }

    fn play_pause(&self) {
        self.send(IpcEvent::TogglePlay);
    }

    fn play(&self) {
        self.send(IpcEvent::Play { song_id: None });
    }

    /// Rewind, then pause: MPRIS `Stop` means the position is reset and playback stops, and
    /// the two events go through the same ordered queue the IPC socket feeds. The order
    /// matters — a seek restarts the background player at the new position, so pausing first
    /// would be undone by the seek that follows it.
    fn stop(&self) {
        self.send(IpcEvent::Seek {
            value: "0".to_string(),
        });
        self.send(IpcEvent::Pause);
    }

    /// A relative jump, in microseconds: what the `:seek` grammar calls `+15` / `-30`.
    fn seek(&self, offset: i64) {
        if offset == 0 {
            return;
        }
        let seconds = offset as f64 / 1_000_000.0;
        self.send(IpcEvent::Seek {
            value: format!("{seconds:+}"),
        });
        let _ = self.seeked.send(self.snapshot().position_ms as i64 * 1000);
    }

    /// An absolute jump, in the same microseconds; a track id that is not the one playing is
    /// ignored, as the spec asks.
    fn set_position(&self, track_id: ObjectPath<'_>, position: i64) {
        if track_id.as_str() != track_path(self.snapshot().id) {
            return;
        }
        let seconds = position as f64 / 1_000_000.0;
        self.send(IpcEvent::Seek {
            value: seconds.to_string(),
        });
        let _ = self.seeked.send(position);
    }

    fn open_uri(&self, _uri: String) {}

    #[zbus(property)]
    fn playback_status(&self) -> String {
        playback_status(&self.snapshot()).to_string()
    }

    #[zbus(property)]
    fn loop_status(&self) -> String {
        loop_status(self.mode()).to_string()
    }

    #[zbus(property)]
    fn set_loop_status(&mut self, value: String) {
        self.send(IpcEvent::SetMode(mode_for(&value, shuffle(self.mode()))));
    }

    #[zbus(property)]
    fn shuffle(&self) -> bool {
        shuffle(self.mode())
    }

    #[zbus(property)]
    fn set_shuffle(&mut self, value: bool) {
        self.send(IpcEvent::SetMode(mode_for(loop_status(self.mode()), value)));
    }

    #[zbus(property)]
    fn rate(&self) -> f64 {
        1.0
    }

    #[zbus(property)]
    fn minimum_rate(&self) -> f64 {
        1.0
    }

    #[zbus(property)]
    fn maximum_rate(&self) -> f64 {
        1.0
    }

    #[zbus(property)]
    fn volume(&self) -> f64 {
        self.snapshot().volume
    }

    #[zbus(property)]
    fn set_volume(&mut self, value: f64) {
        self.send(IpcEvent::Volume {
            delta: None,
            absolute: Some(value.clamp(0.0, 1.0)),
        });
    }

    /// Polled by clients rather than signalled, as the spec asks: playback advances a little
    /// every second and announcing that would be noise on the bus.
    #[zbus(property(emits_changed_signal = "false"))]
    fn position(&self) -> i64 {
        self.snapshot().position_ms as i64 * 1000
    }

    #[zbus(property)]
    fn metadata(&self) -> HashMap<String, OwnedValue> {
        let snapshot = self.snapshot();
        let art_url = self
            .state
            .lock()
            .ok()
            .and_then(|state| state.art_url.clone());
        metadata(&snapshot, art_url.as_deref())
    }

    #[zbus(property)]
    fn can_play(&self) -> bool {
        self.snapshot().id != 0
    }

    #[zbus(property)]
    fn can_pause(&self) -> bool {
        let snapshot = self.snapshot();
        snapshot.playing && !snapshot.paused
    }

    #[zbus(property)]
    fn can_go_next(&self) -> bool {
        self.snapshot().id != 0
    }

    #[zbus(property)]
    fn can_go_previous(&self) -> bool {
        self.snapshot().id != 0
    }

    #[zbus(property)]
    fn can_seek(&self) -> bool {
        self.snapshot().duration_ms > 0
    }

    #[zbus(property)]
    fn can_control(&self) -> bool {
        true
    }
}

/* -------------------------------------------------------------------------- */
/*                               the bus plumbing                             */
/* -------------------------------------------------------------------------- */

impl MprisService {
    /// Serve on the session bus until the app exits.
    ///
    /// A missing or private session bus, or a bus that refuses us the name, is a warning and a
    /// return: the player works without a media session, and refusing to start over one would be
    /// worse than losing the media keys. A *second* instance is not a refusal — `Builder::name`
    /// requests the name with `ReplaceExisting`, so the newest instance takes the session over and
    /// the older one is told it lost the name (see SKILLS.md).
    pub async fn run(self, events: mpsc::UnboundedSender<Event>) {
        self.run_target(EventTarget::Legacy(events)).await;
    }

    /// Serve the actor frontend: controls land in the same `IpcEvent` queue the IPC
    /// socket does, without borrowing the legacy app loop.
    pub async fn run_actor(self, events: mpsc::UnboundedSender<IpcEvent>) {
        self.run_target(EventTarget::Actor(events)).await;
    }

    async fn run_target(self, target: EventTarget) {
        let (seeked_tx, mut seeked_rx) = mpsc::unbounded_channel();
        let connection = match serve(Arc::clone(&self.state), target, seeked_tx).await {
            Ok(connection) => connection,
            Err(error) => {
                log::warn!("MPRIS 媒体会话不可用（{error}）：桌面媒体键与 playerctl 无法控制播放");
                return;
            }
        };
        log::info!("MPRIS: 已在 {BUS_NAME} 上提供媒体会话");

        let player = match connection
            .object_server()
            .interface::<_, Player>(PATH)
            .await
        {
            Ok(iface) => iface,
            Err(error) => {
                log::warn!("MPRIS: 取不到播放器接口（{error}）");
                return;
            }
        };

        let mut announced: Option<Props> = None;
        let mut updated = self.updated;
        loop {
            tokio::select! {
                update = updated.recv() => {
                    if update.is_none() {
                        return;
                    }
                    // Read under the lock in its own scope: the guard must not live across
                    // the `announce` await below (it is neither `Send` nor worth holding).
                    let props = self.state.lock().map(|state| Props::of(&state)).ok();
                    let Some(props) = props else { continue };
                    let moved = announced.as_ref() != Some(&props);
                    if !moved {
                        continue;
                    }
                    if let Err(error) = announce(&player, announced.as_ref(), &props).await {
                        log::debug!("MPRIS: 属性变更广播失败: {error}");
                    }
                    announced = Some(props);
                }
                seeked = seeked_rx.recv() => {
                    let Some(position) = seeked else { return };
                    if let Err(error) = connection
                        .emit_signal(None::<&str>, PATH, PLAYER_IFACE, "Seeked", &position)
                        .await
                    {
                        log::debug!("MPRIS: Seeked 广播失败: {error}");
                    }
                }
            }
        }
    }
}

/// Serve both interfaces and ask for the well-known name; the returned connection is what the
/// `Seeked` signal needs.
async fn serve(
    state: Arc<Mutex<State>>,
    target: EventTarget,
    seeked: mpsc::UnboundedSender<i64>,
) -> zbus::Result<Connection> {
    zbus::connection::Builder::session()?
        .name(BUS_NAME)?
        .serve_at(
            PATH,
            MediaPlayer2 {
                target: target.clone(),
            },
        )?
        .serve_at(
            PATH,
            Player {
                state,
                target,
                seeked,
            },
        )?
        .build()
        .await
}

/// Announce the properties that moved since `previous` — all of them when there is no
/// previous snapshot, because before the first one clients are looking at the empty defaults.
///
/// The per-property `*_changed` helpers the `zbus::interface` macro generates rebuild the
/// value from the getter and send one `PropertiesChanged`, which is what keeps the announced
/// value and the served value from drifting apart.
async fn announce(
    player: &InterfaceRef<Player>,
    previous: Option<&Props>,
    current: &Props,
) -> zbus::Result<()> {
    let interface = player.get().await;
    let emitter = player.signal_emitter();

    let status_moved = previous.is_none_or(|old| old.playback_status != current.playback_status);
    if status_moved {
        interface.playback_status_changed(emitter).await?;
        // CanPause is the other half of "playing": it moves with the same flag.
        interface.can_pause_changed(emitter).await?;
    }
    if previous.is_none_or(|old| old.loop_status != current.loop_status) {
        interface.loop_status_changed(emitter).await?;
    }
    if previous.is_none_or(|old| old.shuffle != current.shuffle) {
        interface.shuffle_changed(emitter).await?;
    }
    if previous.is_none_or(|old| old.volume != current.volume) {
        interface.volume_changed(emitter).await?;
    }
    if previous.is_none_or(|old| old.track != current.track) {
        interface.metadata_changed(emitter).await?;
        interface.can_seek_changed(emitter).await?;
    }
    if previous.is_none_or(|old| old.track.id != current.track.id) {
        interface.can_play_changed(emitter).await?;
        interface.can_go_next_changed(emitter).await?;
        interface.can_go_previous_changed(emitter).await?;
    }
    Ok(())
}

/// What MPRIS clients show for the current track.
fn metadata(snapshot: &StatusSnapshot, art_url: Option<&str>) -> HashMap<String, OwnedValue> {
    let mut map = HashMap::new();
    if snapshot.id == 0 {
        return map;
    }
    let track = track_path(snapshot.id);
    let track = ObjectPath::try_from(track.as_str()).expect("track path is a valid object path");
    map.insert("mpris:trackid".to_string(), owned(track));
    map.insert(
        "mpris:length".to_string(),
        owned(snapshot.duration_ms as i64 * 1000),
    );
    map.insert("xesam:title".to_string(), owned(snapshot.name.clone()));
    map.insert(
        "xesam:artist".to_string(),
        owned(vec![snapshot.artist.clone()]),
    );
    map.insert("xesam:album".to_string(), owned(snapshot.album.clone()));
    if let Some(art_url) = art_url {
        map.insert("mpris:artUrl".to_string(), owned(art_url.to_string()));
    }
    map
}

/// An owned variant value, built from anything that serializes as one.
fn owned<'a>(value: impl Into<Value<'a>>) -> OwnedValue {
    OwnedValue::try_from(value.into()).expect("value has an owned representation")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The two MPRIS bits and the five modes have to agree: what is reported is what a
    /// client writes back.
    #[test]
    fn the_mode_round_trips_through_loop_status_and_shuffle() {
        for mode in [
            PlayMode::Sequential,
            PlayMode::RepeatOne,
            PlayMode::RepeatAll,
            PlayMode::Shuffle,
        ] {
            assert_eq!(
                mode_for(loop_status(mode), shuffle(mode)),
                mode,
                "{mode:?} did not survive the MPRIS view"
            );
        }
    }

    /// Heartbeat is the one mode MPRIS cannot name; it reports as a shuffled playlist.
    #[test]
    fn heartbeat_reports_as_a_shuffled_playlist() {
        let heartbeat = PlayMode::Heartbeat { playlist_id: 42 };
        assert_eq!(loop_status(heartbeat), "Playlist");
        assert!(shuffle(heartbeat));
    }

    /// What clients show: title, artist, length and the track id, or nothing at all when no
    /// song is loaded.
    #[test]
    fn metadata_follows_the_snapshot() {
        let empty = metadata(&StatusSnapshot::default(), None);
        assert!(empty.is_empty(), "no track is no metadata");

        let snapshot = StatusSnapshot {
            id: 7,
            name: "歌".into(),
            artist: "歌手".into(),
            album: "专辑".into(),
            duration_ms: 185_000,
            ..StatusSnapshot::default()
        };
        let map = metadata(&snapshot, Some("file:///tmp/7.jpg"));
        assert_eq!(map["mpris:length"], owned(185_000_000i64));
        assert_eq!(map["xesam:title"], owned("歌".to_string()));
        assert_eq!(map["xesam:artist"], owned(vec!["歌手".to_string()]));
        assert_eq!(map["mpris:artUrl"], owned("file:///tmp/7.jpg".to_string()));
    }

    /// The empty status is "Stopped", not "Paused": a player with nothing loaded has not
    /// paused.
    #[test]
    fn the_status_matches_the_flags() {
        let playing = StatusSnapshot {
            playing: true,
            ..StatusSnapshot::default()
        };
        let paused = StatusSnapshot {
            playing: true,
            paused: true,
            ..StatusSnapshot::default()
        };
        assert_eq!(playback_status(&playing), "Playing");
        assert_eq!(playback_status(&paused), "Paused");
        assert_eq!(playback_status(&StatusSnapshot::default()), "Stopped");
    }
}
