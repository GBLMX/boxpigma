//! Main screen area layout: splits the frame into topbar, navigation, content and
//! player-bar regions (plus the splash layout).

use ratatui::layout::{Constraint, Flex, Layout, Rect};

use crate::{
    config::{NavPosition, Pane, PanesConfig},
    utils::Named,
};

/// Which coordinate a divider moves along.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    Rows,
    Columns,
}

/// A draggable edge between two panes.
///
/// The frame's own border is not one of these: it never moves, or it would not be a frame. What
/// moves is the edge *inside* it. `sign` is which way the mouse has to travel to make the pane
/// bigger — the player bar's top edge grows the bar when dragged up, so its sign is negative.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Divider {
    pub pane: Pane,
    pub axis: Axis,
    sign: i32,
    /// The single row or column the mouse has to land on to grab it.
    pub rect: Rect,
}

impl Divider {
    /// A draggable edge: the pane behind it, the axis the mouse moves along, and which way that
    /// movement grows the pane (`-1` when the edge is on the far side, like the player bar's top
    /// row — dragging it up makes the bar taller).
    pub fn new(pane: Pane, axis: Axis, sign: i32, rect: Rect) -> Self {
        Self {
            pane,
            axis,
            sign,
            rect,
        }
    }

    /// The size the pane should take when the mouse has moved `delta` cells from where the drag
    /// started, before clamping (see [`clamp`]).
    pub fn size_for(&self, start: u16, delta: i32) -> u16 {
        (i32::from(start) + self.sign * delta).max(0) as u16
    }
}

/// The dividers a layout produced this frame, one per resizable pane at most.
///
/// A fixed-size list rather than a `Vec`: the layout runs every frame, and there are four panes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Dividers {
    items: [Option<Divider>; Pane::ALL.len()],
    len: usize,
}

impl Dividers {
    pub fn clear(&mut self) {
        self.items = [None; Pane::ALL.len()];
        self.len = 0;
    }

    pub fn push(&mut self, divider: Divider) {
        if self.len < self.items.len() {
            self.items[self.len] = Some(divider);
            self.len += 1;
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = &Divider> {
        self.items[..self.len].iter().filter_map(Option::as_ref)
    }

    /// The divider under a cell, if the mouse is on one.
    pub fn at(&self, col: u16, row: u16) -> Option<&Divider> {
        self.iter().find(|divider| {
            let rect = divider.rect;
            col >= rect.x && col < rect.right() && row >= rect.y && row < rect.bottom()
        })
    }
}

/// Rows the navigation takes when it is a band along the top or bottom: one row per item, drawn
/// short whatever [`PanesConfig::navigation`] says, since a band is a strip of items rather than
/// a column of them.
const NAV_BAND_ROWS: u16 = 3;

/// The narrowest the navigation column may be dragged to.
const MIN_NAV_COLUMNS: u16 = 12;

/// Cells the topbar and the player bar may be dragged to.
const MIN_TOPBAR: u16 = 1;
const MAX_TOPBAR: u16 = 6;
const MIN_PLAYERBAR: u16 = 3;
const MAX_PLAYERBAR: u16 = 12;

/// Width the MV column may be dragged to.
const MIN_MV_COLUMNS: u16 = 8;

/// What a pane's size is clamped to, given the axis it lands on and the room there is.
///
/// Dragging is bounded rather than free: a pane dragged past what the content needs would leave
/// the app with nowhere to put the music, and clamped sizes are also what keeps a config written
/// on a wide terminal usable on a narrow one.
pub fn clamp(pane: Pane, axis: Axis, size: u16, available: u16) -> u16 {
    /// Clamp between a floor and a ceiling, keeping the two in order: on a terminal too narrow
    /// for the pane *and* the content, the floor wins rather than `clamp` panicking — a narrow
    /// terminal drops the pane (see `main`) instead of taking the app down.
    fn between(size: u16, min: u16, max: u16) -> u16 {
        size.clamp(min, max.max(min))
    }

    match (pane, axis) {
        (Pane::Topbar, _) => between(size, MIN_TOPBAR, MAX_TOPBAR),
        (Pane::Playerbar, _) => between(size, MIN_PLAYERBAR, MAX_PLAYERBAR),
        // The content area keeps at least 40 columns beside a sidebar.
        (Pane::Navigation, Axis::Columns) => {
            between(size, MIN_NAV_COLUMNS, available.saturating_sub(41))
        }
        // A band along the top or the bottom is one item tall whatever the config says.
        (Pane::Navigation, Axis::Rows) => NAV_BAND_ROWS,
        (Pane::Mv, _) => between(size, MIN_MV_COLUMNS, available.saturating_sub(32)),
    }
}

pub struct SplashLayout {
    pub logo: Rect,
    pub progress: Rect,
    pub logs: Rect,
    pub tag: Rect,
}

/// `logo_rows` is the height of the ASCII logo: the art is the only place its height is
/// written down, and the splash must not have a second opinion about it.
pub fn splash(area: Rect, logo_rows: u16) -> SplashLayout {
    let [logo_area, progress_area, logs_area, tag_area] = Layout::vertical([
        Constraint::Length(logo_rows),
        Constraint::Length(2),
        Constraint::Length(5),
        Constraint::Length(1),
    ])
    .flex(Flex::SpaceAround)
    .areas(area);

    SplashLayout {
        logo: logo_area,
        progress: progress_area,
        logs: logs_area,
        tag: tag_area,
    }
}

pub struct LoginLayout {
    pub status: Rect,
    pub logo: Rect,
    pub login_box: Rect,
}

pub fn login(area: Rect) -> LoginLayout {
    let [status_area, body] = Layout::vertical([Constraint::Length(1), Constraint::Min(26)])
        .flex(Flex::Center)
        .spacing(1)
        .areas(area);

    let [logo_area, box_area] =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).areas(body);

    LoginLayout {
        status: status_area,
        logo: logo_area,
        login_box: box_area,
    }
}

pub struct LayoutAreas {
    pub topbar: Rect,
    pub sidebar: Rect,
    pub breadcrumb: Rect,
    pub nav: Rect,
    pub content: Rect,
    pub playerbar: Rect,
    /// The edges the mouse can drag, rebuilt with the frame.
    pub dividers: Dividers,
}

/// The actor TUI's shell: the same pane geometry as [`main`], but the content
/// owns the old breadcrumb rows because this frontend draws no breadcrumb.
pub struct ShellLayout {
    pub topbar: Rect,
    pub sidebar: Rect,
    pub nav: Rect,
    pub content: Rect,
    pub playerbar: Rect,
    pub dividers: Dividers,
}

pub fn shell(area: Rect, panes: &PanesConfig, nav_position: NavPosition) -> ShellLayout {
    let layout = main(area, panes, nav_position);
    let content = if layout.breadcrumb.is_empty() {
        layout.content
    } else {
        Rect {
            x: layout.content.x,
            y: layout.breadcrumb.y,
            width: layout.content.width,
            height: layout
                .breadcrumb
                .height
                .saturating_add(layout.content.height),
        }
    };
    ShellLayout {
        topbar: layout.topbar,
        sidebar: layout.sidebar,
        nav: layout.nav,
        content,
        playerbar: layout.playerbar,
        dividers: layout.dividers,
    }
}

/// The main page: topbar, navigation, content and player bar.
///
/// Every size comes from [`PanesConfig`] and a collapsed pane is dropped to zero, so the content
/// takes what is left. The frame around all of it never moves: what the mouse drags are the edges
/// listed in `dividers`.
pub fn main(area: Rect, panes: &PanesConfig, nav_position: NavPosition) -> LayoutAreas {
    let mut dividers = Dividers::default();
    let topbar_rows = rows(panes, Pane::Topbar, panes.topbar);
    let playerbar_rows = rows(panes, Pane::Playerbar, panes.playerbar);

    match nav_position {
        NavPosition::Top | NavPosition::Bottom => {
            let nav_rows = rows(panes, Pane::Navigation, NAV_BAND_ROWS);
            let (topbar, nav, middle, playerbar) = if nav_position == NavPosition::Top {
                let [topbar, nav, middle, playerbar] = Layout::vertical([
                    Constraint::Length(topbar_rows),
                    Constraint::Length(nav_rows),
                    Constraint::Min(10),
                    Constraint::Length(playerbar_rows),
                ])
                .areas(area);
                (topbar, nav, middle, playerbar)
            } else {
                let [topbar, middle, nav, playerbar] = Layout::vertical([
                    Constraint::Length(topbar_rows),
                    Constraint::Min(10),
                    Constraint::Length(nav_rows),
                    Constraint::Length(playerbar_rows),
                ])
                .areas(area);
                (topbar, middle, nav, playerbar)
            };
            push_row_dividers(
                &mut dividers,
                area,
                topbar,
                playerbar,
                topbar_rows,
                playerbar_rows,
            );

            LayoutAreas {
                topbar,
                sidebar: Rect::default(),
                breadcrumb: Rect::default(),
                nav,
                content: middle,
                playerbar,
                dividers,
            }
        }
        NavPosition::Left | NavPosition::Right => {
            let [topbar, middle, playerbar] = Layout::vertical([
                Constraint::Length(topbar_rows),
                Constraint::Min(10),
                Constraint::Length(playerbar_rows),
            ])
            .areas(area);
            push_row_dividers(
                &mut dividers,
                area,
                topbar,
                playerbar,
                topbar_rows,
                playerbar_rows,
            );

            // The sidebar is dropped when the terminal is too narrow for both it and a readable
            // table, and when it is collapsed; either way the content fills the whole area — so
            // its width is only worked out when there is a sidebar to give it to.
            let (sidebar, right) = if area.width < 60 || !panes.visible(Pane::Navigation) {
                (Rect::default(), middle)
            } else {
                let nav_columns = clamp(
                    Pane::Navigation,
                    Axis::Columns,
                    panes.navigation,
                    area.width,
                );
                match nav_position {
                    NavPosition::Left => {
                        let [sidebar, right] = Layout::horizontal([
                            Constraint::Length(nav_columns),
                            Constraint::Min(40),
                        ])
                        .areas(middle);
                        (sidebar, right)
                    }
                    NavPosition::Right => {
                        let [right, sidebar] = Layout::horizontal([
                            Constraint::Min(40),
                            Constraint::Length(nav_columns),
                        ])
                        .areas(middle);
                        (sidebar, right)
                    }
                    _ => unreachable!(),
                }
            };

            // The sidebar's inner edge is the divider: on the left the mouse drags it rightwards
            // to widen the sidebar, on the right it drags leftwards.
            if !sidebar.is_empty() {
                let (x, sign) = match nav_position {
                    NavPosition::Left => (sidebar.right().saturating_sub(1), 1),
                    _ => (sidebar.x, -1),
                };
                dividers.push(Divider::new(
                    Pane::Navigation,
                    Axis::Columns,
                    sign,
                    Rect::new(x, sidebar.y, 1, sidebar.height),
                ));
            }

            let [breadcrumb, content] =
                Layout::vertical([Constraint::Length(3), Constraint::Min(1)]).areas(right);

            LayoutAreas {
                topbar,
                sidebar,
                breadcrumb,
                nav: Rect::default(),
                content,
                playerbar,
                dividers,
            }
        }
    }
}

/// The rows a pane takes: its size, or none of them when it is collapsed.
fn rows(panes: &PanesConfig, pane: Pane, size: u16) -> u16 {
    if panes.visible(pane) { size } else { 0 }
}

/// The dividers the topbar and the player bar contribute: the topbar's bottom row and the player
/// bar's top row. Each is a single row across the frame, which is what the mouse grabs.
fn push_row_dividers(
    dividers: &mut Dividers,
    area: Rect,
    topbar: Rect,
    playerbar: Rect,
    topbar_rows: u16,
    playerbar_rows: u16,
) {
    if topbar_rows > 0 {
        dividers.push(Divider::new(
            Pane::Topbar,
            Axis::Rows,
            1,
            Rect::new(area.x, topbar.bottom().saturating_sub(1), area.width, 1),
        ));
    }
    if playerbar_rows > 0 {
        dividers.push(Divider::new(
            Pane::Playerbar,
            Axis::Rows,
            -1,
            Rect::new(area.x, playerbar.y, area.width, 1),
        ));
    }
}

/// A page that is one content area between the topbar and the player bar: the lyrics and the
/// queue. It has no navigation area, so the navigation position is taken and ignored — every
/// shell page's layout has the same shape for the table.
pub fn content(area: Rect, panes: &PanesConfig, _nav_position: NavPosition) -> LayoutAreas {
    let mut dividers = Dividers::default();
    let topbar_rows = rows(panes, Pane::Topbar, panes.topbar);
    let playerbar_rows = rows(panes, Pane::Playerbar, panes.playerbar);

    let [topbar, middle, playerbar] = Layout::vertical([
        Constraint::Length(topbar_rows),
        Constraint::Min(10),
        Constraint::Length(playerbar_rows),
    ])
    .areas(area);
    push_row_dividers(
        &mut dividers,
        area,
        topbar,
        playerbar,
        topbar_rows,
        playerbar_rows,
    );

    LayoutAreas {
        topbar,
        sidebar: Rect::default(),
        breadcrumb: Rect::default(),
        nav: Rect::default(),
        content: middle,
        playerbar,
        dividers,
    }
}

/// The artist page inside the shell's content area: the profile band on top, then the hot
/// songs, the albums and the similar artists.
pub struct ArtistLayout {
    pub profile: Rect,
    pub songs: Rect,
    pub albums: Rect,
    pub similar: Rect,
}

/// Height of the profile band: a portrait, the name and sizes lines, and a few lines of
/// biography. It is fixed rather than proportional so the lists below it stay where they were
/// when a biography is long — the band clips its text instead of growing.
const PROFILE_HEIGHT: u16 = 9;

/// Width from which the songs keep a column of their own instead of every list stacking.
/// Below it, three tables would each be too narrow to read.
const SIDE_BY_SIDE_WIDTH: u16 = 100;

/// The artist page. The hot songs — what a reader comes for — keep a column of their own, and
/// the two secondary lists share the other one, stacked: on a wide terminal that is two columns
/// side by side, and on a narrow one all three stack. The songs are the wider half either way.
pub fn artist(area: Rect) -> ArtistLayout {
    let [profile, lists] =
        Layout::vertical([Constraint::Length(PROFILE_HEIGHT), Constraint::Min(2)]).areas(area);

    let (songs, albums, similar) = if area.width >= SIDE_BY_SIDE_WIDTH {
        let [songs, side] =
            Layout::horizontal([Constraint::Percentage(60), Constraint::Percentage(40)])
                .areas(lists);
        let [albums, similar] =
            Layout::vertical([Constraint::Percentage(50), Constraint::Percentage(50)]).areas(side);
        (songs, albums, similar)
    } else {
        let [songs, albums, similar] = Layout::vertical([
            Constraint::Percentage(50),
            Constraint::Percentage(25),
            Constraint::Percentage(25),
        ])
        .areas(lists);
        (songs, albums, similar)
    };

    ArtistLayout {
        profile,
        songs,
        albums,
        similar,
    }
}
