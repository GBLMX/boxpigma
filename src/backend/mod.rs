//! Shared backend actor, based on the Ferriso architecture.
//!
//! The actor owns the domain state; both frontends render only immutable
//! snapshots. This is the only place that talks to `ApiService` and
//! `PlaybackEngine` for that session. The interactive TUI mounts this backend
//! through `ferriso-tui`; the desktop GUI mounts it through `ferriso-gui`. The
//! headless daemon still owns the legacy `App` because it hosts IPC/MPRIS,
//! which are not yet bridged into this actor.

mod app;
mod view;

pub use app::{Backend, Internal, Message, Message as CoreMessage};
pub use ferriso_core::{App as ActorApp, Command, CoreHandle, CoreTask, Snapshots, spawn};
pub use view::{
    ArtistView, Banner, ContentSource, LoginView, NavItem, NavSection, NavView, Page, PlayerView,
    SearchKind, SearchView, SettingsView, ThemeView, View, current_lyric_index, translation_at,
};
