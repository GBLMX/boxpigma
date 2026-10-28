//! The panes inside the frame: how much room each one takes, and which are collapsed.
//!
//! The frame itself does not move — it is the app's border, and a border you can drag is not a
//! border. What moves inside it are the panes: the topbar, the navigation, the player bar, and
//! the MV column on the lyrics page. Each has a size, and each can be collapsed to nothing and
//! put back the way it was, which is what a drag on its edge and a double click on that edge do.
//!
//! Sizes and collapsed panes are decisions the user made about their screen, so they live in the
//! config file rather than in memory. The model follows Herdr's: a size per pane, and a list of
//! collapsed panes (`collapsed_space_keys`) whose sizes are *kept*, so restoring a pane returns
//! it exactly as it was.

use serde::{Deserialize, Serialize};

use crate::utils::Named;

/// A pane that can be resized and collapsed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Pane {
    /// The title bar across the top.
    Topbar,
    /// The navigation: a column on the left or right, a row on the top or bottom.
    Navigation,
    /// The player bar along the bottom.
    Playerbar,
    /// The MV column on the lyrics page.
    Mv,
}

impl Named for Pane {
    const ALL: &'static [Self] = &[Self::Topbar, Self::Navigation, Self::Playerbar, Self::Mv];

    fn name(self) -> &'static str {
        match self {
            Self::Topbar => "topbar",
            Self::Navigation => "navigation",
            Self::Playerbar => "playerbar",
            Self::Mv => "mv",
        }
    }

    fn describe(self) -> &'static str {
        match self {
            Self::Topbar => "顶栏",
            Self::Navigation => "导航栏",
            Self::Playerbar => "播放条",
            Self::Mv => "MV 海报栏",
        }
    }
}

/// Rows the topbar takes by default — enough for the title, the search box and the portrait.
pub const DEFAULT_TOPBAR: u16 = 3;
/// Cells the navigation takes by default: its width as a column.
pub const DEFAULT_NAVIGATION: u16 = 26;
/// Rows the player bar takes by default.
pub const DEFAULT_PLAYERBAR: u16 = 6;

/// Per-pane sizes and the collapsed list.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PanesConfig {
    /// Rows the topbar takes.
    pub topbar: u16,
    /// Cells the navigation takes: its width as a column, its rows as a band. A band is drawn
    /// one item tall per row, so a row navigation is short whatever this says — the size is
    /// clamped to the axis it lands on rather than stored per position.
    pub navigation: u16,
    /// Rows the player bar takes.
    pub playerbar: u16,
    /// Cells the MV column takes. `0` — the default — lets the poster decide.
    pub mv: u16,
    /// Panes collapsed to nothing, in the order they were collapsed. Their sizes are kept, so
    /// [`PanesConfig::restore`] puts a pane back exactly as it was.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub collapsed: Vec<Pane>,
}

impl Default for PanesConfig {
    fn default() -> Self {
        Self {
            topbar: DEFAULT_TOPBAR,
            navigation: DEFAULT_NAVIGATION,
            playerbar: DEFAULT_PLAYERBAR,
            mv: 0,
            collapsed: Vec::new(),
        }
    }
}

impl PanesConfig {
    /// Whether the pane is drawn at all.
    pub fn visible(&self, pane: Pane) -> bool {
        !self.collapsed.contains(&pane)
    }

    /// The size the pane was left at, collapsed or not.
    pub fn size(&self, pane: Pane) -> u16 {
        match pane {
            Pane::Topbar => self.topbar,
            Pane::Navigation => self.navigation,
            Pane::Playerbar => self.playerbar,
            Pane::Mv => self.mv,
        }
    }

    /// Set the pane's size, collapsed or not: a pane can be resized while it is hidden, and the
    /// size it takes when it comes back is the one left behind.
    pub fn set_size(&mut self, pane: Pane, size: u16) {
        match pane {
            Pane::Topbar => self.topbar = size,
            Pane::Navigation => self.navigation = size,
            Pane::Playerbar => self.playerbar = size,
            Pane::Mv => self.mv = size,
        }
    }

    /// Hide the pane, keeping its size.
    pub fn collapse(&mut self, pane: Pane) {
        if !self.collapsed.contains(&pane) {
            self.collapsed.push(pane);
        }
    }

    /// Show the pane again, at the size it had.
    pub fn restore(&mut self, pane: Pane) {
        self.collapsed.retain(|collapsed| *collapsed != pane);
    }

    /// Collapse it if it is up, restore it if it is down.
    pub fn toggle(&mut self, pane: Pane) {
        if self.visible(pane) {
            self.collapse(pane);
        } else {
            self.restore(pane);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::named::assert_named_contract;

    #[test]
    fn every_pane_names_itself() {
        assert_named_contract::<Pane>();
    }

    /// Collapsing keeps the size: a pane that comes back comes back the way it was, which is what
    /// makes the toggle safe to press — nothing has to be set up again afterwards.
    #[test]
    fn collapsing_keeps_the_size_for_the_way_back() {
        let mut panes = PanesConfig {
            navigation: 34,
            ..PanesConfig::default()
        };
        assert!(panes.visible(Pane::Navigation));

        panes.toggle(Pane::Navigation);
        assert!(!panes.visible(Pane::Navigation));
        assert_eq!(panes.size(Pane::Navigation), 34);

        panes.toggle(Pane::Navigation);
        assert!(panes.visible(Pane::Navigation));
        assert_eq!(panes.size(Pane::Navigation), 34);

        // Resizing a hidden pane is allowed and sticks: it is the size it will come back at.
        panes.collapse(Pane::Navigation);
        panes.set_size(Pane::Navigation, 40);
        panes.restore(Pane::Navigation);
        assert_eq!(panes.size(Pane::Navigation), 40);
    }

    /// The list of collapsed panes survives the config file, and a config that never collapsed
    /// anything does not grow the key.
    #[test]
    fn the_collapsed_list_round_trips_through_the_file() {
        let panes = PanesConfig {
            collapsed: vec![Pane::Mv, Pane::Topbar],
            ..PanesConfig::default()
        };
        let written = toml_edit::ser::to_string(&panes).expect("ser");
        assert!(
            written.contains("collapsed = [\"mv\", \"topbar\"]"),
            "{written}"
        );
        let back: PanesConfig = toml_edit::de::from_str(&written).expect("de");
        assert_eq!(back, panes);

        let plain = toml_edit::ser::to_string(&PanesConfig::default()).expect("ser");
        assert!(!plain.contains("collapsed"), "{plain}");
    }
}
