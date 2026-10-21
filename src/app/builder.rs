use std::time::{Duration, Instant};

use ratatui_image::{FontSize, picker::Picker};
use reqwest::Client;

use super::App;
use crate::{
    config::ThemeRegistry,
    state::{CommandItem, CommandPanel, palette_items},
    utils::terminal::{ImageProtocol, choose_image_protocol, is_tuios_terminal},
};

/// Deadline for the connection phase (TCP + TLS, through the proxy when one is
/// configured). An unreachable proxy or CDN host must fail, not hang the task.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Deadline for a single read: it restarts every time reqwest hands out a chunk,
/// so it aborts a stalled socket and never caps how long a transfer may run.
const READ_TIMEOUT: Duration = Duration::from_secs(30);

/// Total deadline for one-shot requests (cover downloads). Never applied to the
/// streaming client: `stream_download` holds one response body open for the
/// length of a track, and a wall-clock deadline would cut playback short.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Budget for the terminal's answer to the graphics query. The query writes an escape
/// sequence and reads the reply, so a terminal (or a pipe) that never answers must not be
/// able to keep the app from starting.
const PICKER_QUERY_BUDGET: Duration = Duration::from_secs(2);

/// How long to keep re-reading a tuios pane's pty when it reports no pixels.
///
/// tuios writes the pixel fields when the client lays the pane out, and a pane this
/// process is starting in does not exist before that: `tuios new-window` hands the pty
/// over first, and measured on 0.8.1 the pane reports 13x37 cells with 0x0 px at t=0 and
/// its final size with the client's 10x22 px cell at t=0.34s. A pane that was already
/// there — `boxpigma` typed into a shell — has the pixels on the first read.
const TUIOS_CELL_SIZE_BUDGET: Duration = Duration::from_millis(500);

/// Interval between those reads, one `ioctl` on `/dev/tty` each.
const TUIOS_CELL_SIZE_POLL: Duration = Duration::from_millis(10);

impl App {
    /// Build the command palette from the one command table.
    ///
    /// Every entry comes from [`COMMANDS`] — the plain commands, then one submenu per config
    /// section the table names — so the palette cannot drift from the `:` command line again:
    /// the same text that names a command there is what Enter runs here. Themes are the one
    /// submenu built here instead, because the list of them is only known at runtime.
    pub(super) fn build_command_panel(theme_registry: &ThemeRegistry) -> CommandPanel {
        let theme_children: Vec<CommandItem> = theme_registry
            .choosable_names()
            .into_iter()
            .map(|name| CommandItem::Action {
                name: name.to_string(),
                summary: "",
                key: None,
                ex: format!("theme {name}"),
                needs_argument: false,
            })
            .collect();

        let mut commands: Vec<CommandItem> = vec![CommandItem::SubMenu {
            name: "切换主题".to_string(),
            children: theme_children,
        }];
        commands.extend(palette_items());

        CommandPanel::with_root("COMMANDS", commands)
    }

    /// Build the image picker for the current terminal and settle on a protocol.
    ///
    /// The picker queries the terminal itself — graphics support and the cell size in
    /// pixels — and that answer is what [`choose_image_protocol`] trusts first; the
    /// configuration can override the result. The decision is logged, because "the
    /// cover looks wrong" is otherwise impossible to answer from a bug report.
    ///
    /// `ask_the_terminal` is false when nothing is on the other end (the headless daemon,
    /// the one-shot CLI subcommands, tests, `boxpigma > file`): there is no reply to read,
    /// so asking would only block [`query_picker`] for its whole budget.
    ///
    /// A tuios pane is the one terminal that is asked nothing even though it is attached:
    /// what it answers the query with is not a reply the query can read, and its pty
    /// carries the cell size anyway ([`tuios_picker`]). Which protocol it gets needs no
    /// answer either — `TUIOS_ENV` settles that in [`choose_image_protocol`].
    pub(super) fn build_picker(
        playerbar: &crate::config::PlayerbarConfig,
        ask_the_terminal: bool,
    ) -> Picker {
        use ratatui_image::picker::{Picker, ProtocolType};

        // A terminal that does not answer falls back to half blocks, which is what
        // upstream recommends: the fixed-cell-size constructors are deprecated, and a
        // guessed cell size would scale every cover wrongly anyway.
        let tuios = ask_the_terminal && is_tuios_terminal(&|key| std::env::var(key).ok());
        let (mut picker, queried) = if tuios {
            (tuios_picker(), None)
        } else if ask_the_terminal {
            let picker = query_picker().unwrap_or_else(Picker::halfblocks);
            let queried = match picker.protocol_type() {
                ProtocolType::Kitty => Some(ImageProtocol::Kitty),
                ProtocolType::Iterm2 => Some(ImageProtocol::ITerm2),
                ProtocolType::Sixel => Some(ImageProtocol::Sixel),
                _ => None,
            };
            (picker, queried)
        } else {
            (Picker::halfblocks(), None)
        };

        let tmux = picker.tmux_detected();
        let chosen = choose_image_protocol(playerbar.image_protocol, queried, tmux, |key| {
            std::env::var(key).ok()
        });

        log::debug!(
            "covers: config={:?} queried={queried:?} tmux={tmux} -> {chosen:?}",
            playerbar.image_protocol
        );

        picker.set_protocol_type(match chosen {
            Some(ImageProtocol::Kitty) => ProtocolType::Kitty,
            Some(ImageProtocol::ITerm2) => ProtocolType::Iterm2,
            Some(ImageProtocol::Sixel) => ProtocolType::Sixel,
            None => ProtocolType::Halfblocks,
        });
        picker
    }

    /// Build the client that streams audio; `proxy` is the streaming proxy (empty = direct).
    ///
    /// Only the connect and per-read deadlines apply. A total deadline would abort
    /// playback mid-track: `stream_download` keeps a single response body open and
    /// reads from it for as long as the track plays, so the only legitimate limit is
    /// "no byte within [`READ_TIMEOUT`]" — which a stalled socket trips and a playing
    /// track never does.
    pub(super) fn build_stream_client(proxy: &str) -> color_eyre::Result<Client> {
        Self::http_client_builder(proxy)?
            .build()
            .map_err(color_eyre::Report::msg)
    }

    /// Build the client for one-shot requests (cover downloads); `proxy` is the search/cover proxy (empty = direct).
    ///
    /// The whole body is consumed at once here, so the total deadline of
    /// [`REQUEST_TIMEOUT`] is safe in addition to the shared connect/read deadlines —
    /// a cover download that never completes must not keep its spawned task alive.
    pub(super) fn build_cover_client(proxy: &str) -> color_eyre::Result<Client> {
        Self::http_client_builder(proxy)?
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(color_eyre::Report::msg)
    }

    /// Proxy plus the connect/read deadlines every client shares; the caller decides
    /// whether a total deadline applies (streaming must not have one).
    fn http_client_builder(proxy: &str) -> color_eyre::Result<reqwest::ClientBuilder> {
        let mut builder = Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .read_timeout(READ_TIMEOUT);
        if !proxy.is_empty() {
            builder = builder.proxy(reqwest::Proxy::all(proxy).map_err(color_eyre::Report::msg)?);
        }
        Ok(builder)
    }
}

/// Ask the terminal for its graphics protocol and cell size, but never wait forever.
///
/// `Picker::from_query_stdio()` writes a query and reads the answer from stdin. Its own
/// timeout only covers the gap *between* reads — the reader restarts it after every read —
/// so a stdin at end-of-file, or a terminal that answers nothing at all (a pipe rather than a
/// tty), leaves the loop spinning and the call never returns. That is what hung `App::new`, and
/// with it every test that builds an app.
///
/// So the query runs on its own thread and the caller waits with a deadline. On a timeout
/// the picker is built without asking (half blocks, or whatever the config forces) and the
/// app starts.
///
/// The worker cannot be cancelled: if the terminal never answers, it stays parked in its
/// read until the process exits. That is the price of asking at all — the alternatives are
/// a startup that never finishes, or never asking and losing graphics on the terminals
/// that do answer. Callers that know no terminal is attached must pass
/// `ask_the_terminal = false` instead of paying this.
fn query_picker() -> Option<Picker> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("picker-query".into())
        .spawn(move || {
            let _ = tx.send(Picker::from_query_stdio().ok());
        })
        .ok()?;

    match rx.recv_timeout(PICKER_QUERY_BUDGET) {
        Ok(answer) => answer,
        Err(_) => {
            log::warn!(
                "covers: the terminal did not answer the graphics query within {:?} \
                 (stdin may be at end-of-file); using half blocks",
                PICKER_QUERY_BUDGET
            );
            None
        }
    }
}

/// The picker for a tuios pane, which cannot be asked and does not have to be.
///
/// tuios emulates the pane's terminal and redraws the kitty graphics it parses, so the
/// protocol comes from `TUIOS_ENV` ([`choose_image_protocol`]) — no query involved. What
/// the query would have answered, and what has to come from somewhere else, is the cell
/// size: tuios replies `CSI ? 0 n` to the graphics query, and `ratatui-image`'s reply
/// parser only ends the read on the standard `CSI 0 n`, so `Picker::from_query_stdio`
/// never returns inside a pane — the worker parks in `read()` until the process exits and
/// [`PICKER_QUERY_BUDGET`] buys nothing but a two second delay to a guessed cell size.
///
/// The pane's pty does know it: tuios writes the cell size of the client attached to the
/// pane into `TIOCGWINSZ`'s pixel fields (0.8.1, measured: a pane of 18x6 cells reporting
/// 180x132 px for a client whose cells are 10x22 px, exactly the size the daemon logs for
/// that client). The fields arrive only once that client has laid the pane out, hence the
/// re-reads — and no client at all means no cell size, where the library's guess is all
/// there is.
fn tuios_picker() -> Picker {
    let deadline = Instant::now() + TUIOS_CELL_SIZE_BUDGET;

    loop {
        if let Some((width, height)) = measured_cell_size() {
            log::debug!("covers: the tuios pane reports a {width}x{height} px cell");
            return picker_at(FontSize::new(width, height));
        }

        if Instant::now() >= deadline {
            break;
        }

        std::thread::sleep(TUIOS_CELL_SIZE_POLL);
    }

    log::debug!("covers: the tuios pane reported no cell size; using half blocks");
    Picker::halfblocks()
}

/// The pane's cell size in pixels, if its pty reports one yet.
fn measured_cell_size() -> Option<(u16, u16)> {
    let size = crossterm::terminal::window_size().ok()?;
    pane_cell_size(size.rows, size.columns, size.width, size.height)
}

/// The cell size in pixels out of `TIOCGWINSZ`: `width`/`height` are the pty's pixel
/// dimensions, `columns`/`rows` the cells they are divided into. Zero on either side means
/// the terminal reported no pixels (the fields are optional and most terminals leave them
/// at zero), and anything that rounds down to nothing is not a cell size either.
fn pane_cell_size(rows: u16, columns: u16, width: u16, height: u16) -> Option<(u16, u16)> {
    if rows == 0 || columns == 0 {
        return None;
    }

    let cell = (width / columns, height / rows);
    (cell.0 > 0 && cell.1 > 0).then_some(cell)
}

/// A picker at a measured cell size.
///
/// [`Picker::from_fontsize`] is deprecated in favour of the query — but the query is
/// exactly what a tuios pane cannot answer, and this is the only public constructor that
/// takes a cell size instead of reading one.
#[allow(deprecated)]
fn picker_at(font_size: FontSize) -> Picker {
    Picker::from_fontsize(font_size)
}

#[cfg(test)]
mod command_panel_tests {
    use super::*;
    use crate::state::COMMANDS;

    /// Every entry the panel can reach, root list and submenus alike: the label, the command
    /// line it runs and whether Enter prefills instead of running it.
    fn entries(panel: &CommandPanel) -> Vec<(String, String, bool)> {
        fn walk(items: &[CommandItem], found: &mut Vec<(String, String, bool)>) {
            for item in items {
                match item {
                    CommandItem::Action {
                        name,
                        ex,
                        needs_argument,
                        ..
                    } => found.push((name.clone(), ex.clone(), *needs_argument)),
                    CommandItem::SubMenu { children, .. } => walk(children, found),
                }
            }
        }

        let mut found = Vec::new();
        walk(panel.current_items().expect("a root level"), &mut found);
        found
    }

    /// The palette is built from the table, so it cannot fall behind it: every command the `:`
    /// line knows has an entry somewhere in it, and nothing else is there.
    #[test]
    fn the_palette_lists_every_command() {
        let registry = ThemeRegistry::new(Default::default());
        let panel = App::build_command_panel(&registry);

        let mut listed: Vec<String> = entries(&panel).into_iter().map(|(_, ex, _)| ex).collect();
        // Themes are the one submenu whose children the registry supplies.
        listed.retain(|ex| !ex.starts_with("theme "));

        let mut expected: Vec<String> = COMMANDS
            .iter()
            .filter(|command| command.in_palette && command.name != "theme")
            .map(|command| command.ex.to_string())
            .collect();

        listed.sort_unstable();
        expected.sort_unstable();
        assert_eq!(listed, expected, "the palette and the table disagree");
    }

    /// The settings are reachable where the table files them, and the popup says which section
    /// is open — the title comes from the submenu, not from a list of its own.
    #[test]
    fn the_settings_submenus_come_from_the_table() {
        let registry = ThemeRegistry::new(Default::default());
        let mut panel = App::build_command_panel(&registry);

        for (section, expected) in [
            ("通知", vec!["notify song_change", "notify errors"]),
            ("终端", vec!["mouse", "cursor"]),
            ("歌词", vec!["lyrics", "lyricgradient", "translation"]),
            ("缓存", vec!["saveonplay"]),
        ] {
            let index = panel
                .current_items()
                .expect("root level")
                .iter()
                .position(
                    |item| matches!(item, CommandItem::SubMenu { name, .. } if name == section),
                )
                .unwrap_or_else(|| panic!("no {section} submenu"));
            panel.selected = index;
            panel.enter();

            let opened = entries(&panel);
            assert_eq!(
                opened
                    .iter()
                    .map(|(_, ex, _)| ex.as_str())
                    .collect::<Vec<_>>(),
                expected,
                "{section} lists something else"
            );
            assert_eq!(
                panel.current_title(),
                format!("\u{25BA} {section} \u{25C4}")
            );
            panel.back();
        }
    }

    /// Whatever the palette offers has to be a real command: the ones that take an argument
    /// open the command line instead of running, and everything else runs as it stands.
    #[test]
    fn every_palette_entry_is_a_command() {
        let registry = ThemeRegistry::new(Default::default());
        let panel = App::build_command_panel(&registry);

        for (label, ex, needs_argument) in entries(&panel) {
            match crate::input::ex::ExCommand::parse(&ex) {
                Ok(_) => assert!(
                    !needs_argument,
                    "{label}: {ex} takes an argument but runs without one"
                ),
                Err(error) => {
                    assert!(needs_argument, "{label}: {ex} does not parse: {error}");
                    assert!(
                        !error.contains("未知命令"),
                        "{label}: {ex} is not a command at all: {error}"
                    );
                }
            }
        }
    }

    /// Themes come from the registry at runtime, and picking one has to run `theme <name>`.
    #[test]
    fn the_theme_submenu_runs_the_theme_command() {
        let registry = ThemeRegistry::new(Default::default());
        let panel = App::build_command_panel(&registry);
        let Some(CommandItem::SubMenu { children, .. }) = panel.current_items().unwrap().first()
        else {
            panic!("the first entry is the theme submenu");
        };
        let first = children.first().expect("at least one theme");
        let CommandItem::Action { ex, .. } = first else {
            panic!("theme entries are actions");
        };
        assert!(ex.starts_with("theme "), "{ex}");
        crate::input::ex::ExCommand::parse(ex).expect("the submenu entry parses");
    }
}

#[cfg(test)]
mod picker_tests {
    use super::*;

    /// A pane of a real tuios client (0.8.1, measured): 18x6 cells, the client's cells
    /// 10x22 px, and the pty reporting 180x132 px for them. This division is the whole
    /// cell size a tuios pane gets, so getting it the wrong way round scales every cover
    /// wrongly.
    #[test]
    fn the_cell_size_divides_the_pty_pixels_by_its_cells() {
        assert_eq!(pane_cell_size(6, 18, 180, 132), Some((10, 22)));
    }

    /// Most terminals never fill the pixel fields in — a pty of the same size reporting
    /// nothing, a degenerate one with no cells, and pixels too small to be a cell all
    /// have to come back as "not a cell size" rather than as a division.
    #[test]
    fn a_pty_that_reports_no_pixels_has_no_cell_size() {
        assert_eq!(pane_cell_size(34, 154, 0, 0), None, "no pixels reported");
        assert_eq!(
            pane_cell_size(0, 154, 2156, 1292),
            None,
            "no cells to divide by"
        );
        assert_eq!(pane_cell_size(6, 8, 3, 132), None, "narrower than a pixel");
    }
}
