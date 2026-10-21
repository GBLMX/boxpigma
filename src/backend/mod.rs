//! Shared backend actor, based on the Ferriso architecture.
//!
//! The actor owns the domain state and GUI renders only immutable snapshots.
//! This is the only place that talks to `ApiService` and `PlaybackEngine` for
//! that session. The TUI still runs its legacy `App`; moving it onto the same
//! actor means replacing its terminal renderer/input loop, not adding a second
//! backend state.

mod app;
mod view;

pub use app::{Backend, Internal, Message, Message as CoreMessage};
pub use ferriso_core::{App as ActorApp, Command, CoreHandle, CoreTask, Snapshots, spawn};
pub use view::{
    ArtistView, Banner, ContentSource, LoginView, NavItem, NavSection, NavView, Page, PlayerView,
    SearchKind, SearchView, SettingsView, ThemeView, View, current_lyric_index, translation_at,
};
