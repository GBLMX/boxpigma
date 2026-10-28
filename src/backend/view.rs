//! Snapshot data shared by every frontend.
//!
//! Nothing here is a widget tree: each frontend lays these values out with its
//! own native widgets.

use std::sync::Arc;

use ncm_api::{ArtistAlbum, ArtistDetail, LoginInfo, MvInfo, SingerInfo, SongInfo};
use serde::{Deserialize, Serialize};

use crate::{
    config::{NavPosition, PanesConfig},
    playback::{LyricLine, PlayMode},
    state::{ContentState, LoginMethod, PaginationInfo},
};

/// The screen a frontend should be showing. These are semantic destinations,
/// not TUI pages with their own key bindings.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Page {
    Browse,
    Artist,
    Queue,
    Lyrics,
    Login,
    Settings,
}

impl Page {
    pub fn label(self) -> &'static str {
        match self {
            Page::Browse => "浏览",
            Page::Artist => "歌手",
            Page::Queue => "播放队列",
            Page::Lyrics => "歌词",
            Page::Login => "登录",
            Page::Settings => "设置",
        }
    }
}

#[derive(Clone, Debug)]
pub struct NavItem {
    pub name: String,
    pub api: Option<String>,
}

#[derive(Clone, Debug)]
pub struct NavSection {
    pub title: String,
    pub items: Vec<NavItem>,
}

#[derive(Clone, Debug)]
pub struct NavView {
    pub sections: Vec<NavSection>,
    pub section: usize,
    pub item: usize,
}

#[derive(Clone, Debug, Default)]
pub struct Banner {
    pub text: String,
    pub error: bool,
}

/// Where the visible content came from. The frontend never switches on this;
/// it is useful for labels and for the actor's own refresh/open logic.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContentSource {
    /// A configured navigation endpoint, by its config key (`recommend_songs`).
    Endpoint(String),
    Playlist(u64),
    Album(u64),
    Toplist(u64),
    Radio(u64),
    ArtistSongs(u64),
    SimilarSongs(u64),
    SimilarPlaylists(u64),
    PersonalFm,
    Search(SearchKind, String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SearchKind {
    Songs,
    Playlists,
    Artists,
    Albums,
}

impl SearchKind {
    pub const ALL: [Self; 4] = [Self::Songs, Self::Playlists, Self::Artists, Self::Albums];

    pub fn label(self) -> &'static str {
        match self {
            Self::Songs => "单曲",
            Self::Playlists => "歌单",
            Self::Artists => "歌手",
            Self::Albums => "专辑",
        }
    }
}

#[derive(Clone, Debug)]
pub struct SearchView {
    pub kind: SearchKind,
    pub query: String,
    pub active: bool,
}

#[derive(Clone, Debug)]
pub struct PlayerView {
    pub song: Option<Arc<SongInfo>>,
    pub playing: bool,
    pub paused: bool,
    pub progress: f64,
    pub position_secs: f64,
    pub duration_secs: f64,
    pub volume: f64,
    pub mode: PlayMode,
    pub liked: bool,
    pub cached: bool,
    pub cover: Option<Arc<Vec<u8>>>,
    pub lyrics: Vec<LyricLine>,
    pub translated_lyrics: Vec<LyricLine>,
    pub visualizer: Vec<f32>,
    pub pitch: Option<String>,
    /// The song's music video, when it has one; the poster is fetched once per
    /// track and lives only in this snapshot.
    pub mv: Option<MvInfo>,
    pub mv_poster: Option<Arc<Vec<u8>>>,
    pub queue: Vec<Arc<SongInfo>>,
    pub queue_keys: Vec<String>,
    pub queue_key: String,
    pub queue_index: Option<usize>,
}

impl PlayerView {
    pub fn duration(&self) -> std::time::Duration {
        std::time::Duration::from_secs_f64(self.duration_secs.max(0.0))
    }
}

#[derive(Clone, Debug)]
pub struct LoginView {
    pub method: LoginMethod,
    pub account: String,
    pub password: String,
    pub phone: String,
    pub code: String,
    pub qr_url: String,
    pub qr_status: String,
    pub loading: bool,
    pub error: Option<String>,
    pub notice: Option<String>,
}

#[derive(Clone, Debug)]
pub struct SettingsView {
    pub theme: String,
    pub themes: Vec<String>,
    pub quality: String,
    pub save_on_play: bool,
    pub notify_song_change: bool,
    pub notify_errors: bool,
    pub lyric_translation: bool,
    pub visualizer: bool,
    pub pitch: bool,
    pub lyric_style: crate::config::LyricStyle,
}

/// Colours resolved for a native window; `None` means the theme used a
/// terminal-only colour and the frontend should keep its own fallback.
#[derive(Clone, Debug, Default)]
pub struct ThemeView {
    pub background: Option<(u8, u8, u8)>,
    pub surface: Option<(u8, u8, u8)>,
    pub text: Option<(u8, u8, u8)>,
    pub accent: Option<(u8, u8, u8)>,
    pub muted: Option<(u8, u8, u8)>,
    pub border: Option<(u8, u8, u8)>,
    pub error: Option<(u8, u8, u8)>,
    pub warning: Option<(u8, u8, u8)>,
}

#[derive(Clone, Debug)]
pub struct ArtistView {
    pub detail: ArtistDetail,
    pub albums: Vec<ArtistAlbum>,
    pub similar: Vec<SingerInfo>,
}

/// Everything a frontend needs to draw one frame.
#[derive(Clone, Debug)]
pub struct View {
    pub page: Page,
    pub nav: NavView,
    pub content: ContentState,
    pub title: String,
    pub subtitle: Option<String>,
    pub source: Option<ContentSource>,
    pub pagination: Option<PaginationInfo>,
    pub loading: bool,
    pub banner: Banner,
    pub user: Option<LoginInfo>,
    pub can_back: bool,
    pub artist: Option<ArtistView>,
    pub player: PlayerView,
    pub search: SearchView,
    pub panes: PanesConfig,
    pub nav_position: NavPosition,
    pub login: LoginView,
    pub settings: SettingsView,
    pub theme: ThemeView,
}

impl View {
    /// Index of the lyric line in force at `position_secs`.
    pub fn current_lyric(&self) -> Option<usize> {
        current_lyric_index(&self.player.lyrics, self.player.position_secs)
    }
}

/// The last timestamp at or before `position_secs`. `None` during an intro.
pub fn current_lyric_index(lines: &[LyricLine], position_secs: f64) -> Option<usize> {
    if lines.is_empty() {
        return None;
    }
    let position = std::time::Duration::from_secs_f64(position_secs.max(0.0));
    match lines.partition_point(|line| line.time <= position) {
        0 => None,
        index => Some(index - 1),
    }
}

/// The translation that belongs with `line`: the closest translated timestamp,
/// accepted only when it is close enough to be the same utterance.
pub fn translation_at<'a>(translated: &'a [LyricLine], line: &LyricLine) -> Option<&'a str> {
    let (translation, distance) = translated
        .iter()
        .map(|candidate| (candidate, candidate.time.abs_diff(line.time)))
        .min_by_key(|(_, distance)| *distance)?;
    (distance < std::time::Duration::from_millis(1500)).then_some(translation.text.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn lyric(seconds: u64, text: &str) -> LyricLine {
        LyricLine {
            time: Duration::from_secs(seconds),
            text: text.into(),
        }
    }

    #[test]
    fn the_lyric_clock_follows_the_timestamps() {
        let lines = vec![lyric(2, "two"), lyric(5, "five")];
        assert_eq!(current_lyric_index(&lines, 0.0), None);
        assert_eq!(current_lyric_index(&lines, 1.9), None);
        assert_eq!(current_lyric_index(&lines, 2.0), Some(0));
        assert_eq!(current_lyric_index(&lines, 4.9), Some(0));
        assert_eq!(current_lyric_index(&lines, 5.0), Some(1));
        assert_eq!(current_lyric_index(&lines, 99.0), Some(1));
        assert_eq!(current_lyric_index(&[], 3.0), None);
    }

    #[test]
    fn a_translation_must_share_the_utterance() {
        let line = lyric(10, "text");
        assert_eq!(translation_at(&[lyric(10, "译")], &line), Some("译"));
        assert_eq!(translation_at(&[lyric(9, "near")], &line), Some("near"));
        assert_eq!(translation_at(&[lyric(2, "far")], &line), None);
    }
}
