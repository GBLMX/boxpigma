//! CLI entry: `boxpigma status` / `boxpigma msg` subcommands plus the argument
//! parser. The TUI itself runs when no subcommand is given.

use std::path::PathBuf;

use clap::{
    Parser, Subcommand, ValueHint,
    builder::{PossibleValue, PossibleValuesParser, Styles, styling::AnsiColor},
};

use crate::{
    app::App,
    cli,
    config::Config,
    ipc::{self, ActionKind, ControlAction, MsgAction, QueryAction, StatusSnapshot},
    logger::init_logger,
    playback::parse_seek,
    update,
    utils::format_duration,
};

const STYLES: Styles = Styles::styled()
    .header(AnsiColor::Yellow.on_default().bold())
    .usage(AnsiColor::Yellow.on_default().bold())
    .literal(AnsiColor::Cyan.on_default().bold())
    .placeholder(AnsiColor::Cyan.on_default());

/// The accepted `boxpigma msg` actions. Names, aliases and help text all come
/// from [`ipc::ACTIONS`], so the CLI, the shell completions and the
/// `capabilities` payload cannot disagree about what exists.
fn msg_action_parser() -> PossibleValuesParser {
    PossibleValuesParser::new(ipc::ACTIONS.iter().map(|spec| {
        spec.aliases.iter().fold(
            PossibleValue::new(spec.name).help(spec.summary),
            |value, alias| value.alias(*alias),
        )
    }))
}

#[derive(Debug, Parser)]
#[command(
    name = "boxpigma",
    version,
    disable_version_flag = true,
    about = "A netease cloud music client",
    long_about = "A netease cloud music client.\n\nNo subcommand launches the TUI; `status` / `msg` query or control a running instance.",
    args_conflicts_with_subcommands = true,
    styles = STYLES
)]
pub struct Cli {
    /// Print version and exit.
    #[arg(short = 'v', long = "version", action = clap::ArgAction::Version)]
    pub version: Option<bool>,
    /// Run headless, loading an endpoint (default `liked`). Suffix `:N` picks
    /// the N-th playlist of a list endpoint, e.g. `toplist:3`.
    #[arg(
        long,
        short = 'd',
        value_name = "ENDPOINT[:N]",
        num_args = 0..=1,
        default_missing_value = "liked"
    )]
    pub daemon: Option<String>,
    /// Load the N-th playlist of the endpoint (1-based).
    #[arg(long, value_name = "INDEX", requires = "daemon", hide = true, value_hint = ValueHint::Other)]
    pub playlist: Option<usize>,
    /// IPC socket path.
    #[arg(long, value_name = "SOCKET")]
    pub socket: Option<PathBuf>,
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Show the running instance's status.
    Status {
        /// Output template: {name} {artist} {album} {duration} {position}
        /// {volume} {status} {mode} {id} {liked}. Defaults to config.
        #[arg(long, default_value = "", value_hint = ValueHint::Other)]
        template: String,
        /// Output as JSON.
        #[arg(long)]
        json: bool,
        /// List the playback queue (`>` marks the current song).
        #[arg(short = 'L', long)]
        list: bool,
        /// IPC socket path.
        #[arg(long, value_name = "SOCKET")]
        socket: Option<PathBuf>,
    },
    /// Control the running instance.
    Msg {
        /// Playback action. Possible values are listed below.
        #[arg(value_parser = msg_action_parser(), value_hint = ValueHint::Other)]
        action: String,
        /// Value for `play` (a song id), `search` (a keyword), `volume`
        /// (`75`/`+5`/`-5`), the endpoint for `switch-list`/`list`, or omitted.
        #[arg(allow_hyphen_values = true, value_hint = ValueHint::Other)]
        value: Option<String>,
        /// Playlist index for `switch-list` (1-based).
        #[arg(long, value_name = "INDEX", value_hint = ValueHint::Other)]
        playlist: Option<usize>,
        /// Output the queue as JSON (`list` with no value).
        #[arg(long)]
        json: bool,
        /// IPC socket path.
        #[arg(long, value_name = "SOCKET")]
        socket: Option<PathBuf>,
    },
    /// Generate a shell completion script and print it to stdout.
    Completions {
        /// Target shell: bash | zsh | fish | elvish | powershell.
        #[arg(value_parser = ["bash", "zsh", "fish", "elvish", "powershell"])]
        shell: String,
    },
    /// Check for and install the latest release in place (self-update).
    Update {
        /// Only report the installed version and the latest release.
        #[arg(long, conflicts_with_all = ["rollback", "force"])]
        check: bool,
        /// Release tag to install (`v1.6.0` or `1.6.0`); default `latest`.
        #[arg(long, value_name = "TAG", default_value = "latest", value_hint = ValueHint::Other)]
        version: String,
        /// Install root: the directory holding `releases/` and `current`.
        #[arg(long, value_name = "DIR", value_hint = ValueHint::AnyPath)]
        dir: Option<PathBuf>,
        /// Replace the `https://github.com` prefix (mirrors/proxies).
        #[arg(long, value_name = "URL", value_hint = ValueHint::Url)]
        mirror: Option<String>,
        /// `SHA256SUMS` URL or local file; default the one beside the asset.
        #[arg(long, value_name = "URL|FILE", value_hint = ValueHint::AnyPath)]
        checksums: Option<String>,
        /// Reinstall even when this version is already installed.
        #[arg(long)]
        force: bool,
        /// Install even when `SHA256SUMS` cannot be fetched (no integrity check).
        #[arg(long)]
        insecure: bool,
        /// Print the plan; download nothing, write nothing.
        #[arg(long)]
        dry_run: bool,
        /// Switch `current` back to the previous version.
        #[arg(long)]
        rollback: bool,
    },
}

/* -------------------------------------------------------------------------- */
/*                               command actions                              */
/* -------------------------------------------------------------------------- */

/// `boxpigma status` handler.
pub async fn status(template: &str, json: bool, list: bool) -> color_eyre::Result<()> {
    let config = Config::load();
    if list {
        let queue = ipc::fetch_queue().await?;
        if json {
            println!("{}", serde_json::to_string_pretty(&queue)?);
        } else {
            let current = queue.current_index;
            for (i, song) in queue.songs.iter().enumerate() {
                let marker = if Some(i) == current { ">" } else { " " };
                println!(
                    "{marker} {:<3} {:<32} {:<24} {}",
                    i + 1,
                    song.name,
                    song.singer,
                    format_duration(song.duration_ms)
                );
            }
        }
        return Ok(());
    }
    let snapshot = ipc::fetch_status().await?;
    if json || config.cli_status_format == "json" {
        println!("{}", serde_json::to_string_pretty(&snapshot)?);
    } else {
        let template = if template.is_empty() {
            &config.cli_status_template
        } else {
            template
        };
        println!("{}", format_status(template, &snapshot));
    }
    Ok(())
}

/// Render the playback queue the way the TUI's queue table does: `▶` marks the
/// currently playing song, rows show a 1-based index, title, artist and duration.
fn render_queue(queue: &ipc::QueueSnapshot) -> String {
    let current = queue.current_index;
    let mut out = String::new();
    for (i, song) in queue.songs.iter().enumerate() {
        let marker = if Some(i) == current { "▶" } else { " " };
        use std::fmt::Write;
        let _ = writeln!(
            out,
            "{marker}{:02}  {:<24}  {:<20}  {}",
            i + 1,
            song.name,
            song.singer,
            format_duration(song.duration_ms)
        );
    }
    out
}

fn print_queue(queue: &ipc::QueueSnapshot) {
    print!("{}", render_queue(queue));
}

/// `boxpigma msg` handler. `capabilities`, `list` (no value) and `search` are
/// request/response commands answered by the daemon; everything else is a
/// fire-and-forget control action. The action name is resolved through
/// [`ipc::ACTIONS`], so the CLI accepts exactly what `capabilities` advertises.
pub async fn msg(
    action: &str,
    value: Option<&str>,
    playlist: Option<usize>,
    json: bool,
) -> color_eyre::Result<()> {
    let spec = ipc::action_spec(action).ok_or_else(|| {
        color_eyre::eyre::eyre!("unknown action `{action}` (see `boxpigma msg capabilities`)")
    })?;
    if value.is_some() && !spec.takes_value {
        color_eyre::eyre::bail!(
            "`{}` takes no value (see `boxpigma msg capabilities`)",
            spec.name
        );
    }

    match spec.kind {
        ActionKind::Query(QueryAction::Capabilities) => {
            let capabilities = ipc::fetch_capabilities().await?;
            print_capabilities(&capabilities, json);
            Ok(())
        }
        ActionKind::Query(QueryAction::List) => {
            // `boxpigma msg list <endpoint>` keeps the old switch-list alias;
            // `boxpigma msg list` with no value prints the live playback queue.
            if let Some(endpoint) = value {
                let action = MsgAction::SwitchList {
                    endpoint: endpoint.to_string(),
                    playlist,
                };
                ipc::send_msg(action).await?;
                return Ok(());
            }
            let queue = ipc::fetch_queue().await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&queue)?);
            } else {
                print_queue(&queue);
            }
            Ok(())
        }
        ActionKind::Query(QueryAction::Search) => {
            let keyword = value.ok_or_else(|| {
                color_eyre::eyre::eyre!(
                    "search requires a keyword (e.g. `boxpigma msg search 周杰伦`)"
                )
            })?;
            let results = ipc::search_songs(keyword).await?;
            if results.is_empty() {
                println!("没有找到与「{keyword}」相关的歌曲");
                return Ok(());
            }
            for entry in &results {
                println!(
                    "{:<10} {:<20} {} - {}",
                    entry.source, entry.id, entry.name, entry.singer
                );
            }
            Ok(())
        }
        ActionKind::Control(action) => {
            let action = parse_msg_action(action, value, playlist)?;
            ipc::send_msg(action).await?;
            Ok(())
        }
    }
}

/// Print the `capabilities` reply. `--json` pretty-prints the reply the daemon
/// sent (same fields, same values) for scripts; the plain form is for humans.
fn print_capabilities(capabilities: &serde_json::Value, json: bool) {
    if json {
        match serde_json::to_string_pretty(capabilities) {
            Ok(text) => println!("{text}"),
            Err(e) => eprintln!("failed to render capabilities: {e}"),
        }
        return;
    }
    let text = |value: &serde_json::Value| value.as_str().unwrap_or_default().to_string();
    println!(
        "api {} · boxpigma {} · {}",
        capabilities["api"],
        text(&capabilities["version"]),
        text(&capabilities["socket"])
    );
    for action in capabilities["actions"].as_array().into_iter().flatten() {
        let aliases: Vec<String> = action["aliases"]
            .as_array()
            .into_iter()
            .flatten()
            .map(text)
            .collect();
        let name = match aliases.is_empty() {
            true => text(&action["name"]),
            false => format!("{} ({})", text(&action["name"]), aliases.join(", ")),
        };
        println!(
            "  {name:<28} {:<5} {}",
            if action["takes_value"].as_bool().unwrap_or(false) {
                "VALUE"
            } else {
                ""
            },
            text(&action["summary"])
        );
    }
}

/// Turn a control action plus the command-line value into the wire
/// [`MsgAction`].
fn parse_msg_action(
    action: ControlAction,
    value: Option<&str>,
    playlist: Option<usize>,
) -> color_eyre::Result<MsgAction> {
    Ok(match action {
        ControlAction::Previous => MsgAction::Previous,
        ControlAction::Next => MsgAction::Next,
        ControlAction::Pause => MsgAction::Pause,
        ControlAction::Play => {
            let song_id = match value {
                None => None,
                Some(v) => Some(v.parse().map_err(|_| {
                    color_eyre::eyre::eyre!(
                        "invalid song id `{v}` (expected a number, or omit to play/resume)"
                    )
                })?),
            };
            MsgAction::Play { song_id }
        }
        ControlAction::TogglePlay => MsgAction::TogglePlay,
        ControlAction::SwitchList => {
            let endpoint = value.ok_or_else(|| {
                color_eyre::eyre::eyre!(
                    "switch-list requires an endpoint (e.g. `boxpigma msg switch-list toplist`)"
                )
            })?;
            MsgAction::SwitchList {
                endpoint: endpoint.to_string(),
                playlist,
            }
        }
        ControlAction::Volume => {
            let value = value.ok_or_else(|| {
                color_eyre::eyre::eyre!("volume requires a value like `75`, `+5` or `-5`")
            })?;
            parse_volume(value)?
        }
        ControlAction::Seek => {
            // The shapes `:seek` accepts come from the same parser: a value the command would
            // refuse must not reach the daemon, where the only feedback is a toast the caller
            // never sees (and the action's reply says `ok` either way).
            let usage = || {
                color_eyre::eyre::eyre!("seek requires a value like `+15`, `-30`, `50%` or `90`")
            };
            let value = value.ok_or_else(usage)?;
            parse_seek(value).map_err(|_| usage())?;
            MsgAction::Seek {
                value: value.to_string(),
            }
        }
        ControlAction::Clear => MsgAction::Clear,
        ControlAction::Mode => MsgAction::Mode,
        ControlAction::Like => MsgAction::Like,
        ControlAction::Dislike => MsgAction::Dislike,
        ControlAction::ToggleLike => MsgAction::ToggleLike,
    })
}

/// Parse a volume value. A leading `+`/`-` is a delta (percent) applied via
/// `adjust_volume`, matching the TUI's `+`/`-` keys; a bare number is an
/// absolute volume percent.
pub(crate) fn parse_volume(value: &str) -> color_eyre::Result<MsgAction> {
    let err = || color_eyre::eyre::eyre!("invalid volume `{value}`");
    if let Some(delta) = value.strip_prefix('+').or_else(|| value.strip_prefix('-')) {
        let number: f64 = delta.parse().map_err(|_| err())?;
        let signed = if value.starts_with('-') {
            -number
        } else {
            number
        };
        Ok(MsgAction::Volume {
            delta: Some(signed / 100.0),
            absolute: None,
        })
    } else {
        let number: f64 = value.parse().map_err(|_| err())?;
        if !(0.0..=100.0).contains(&number) {
            return Err(err());
        }
        Ok(MsgAction::Volume {
            delta: None,
            absolute: Some(number / 100.0),
        })
    }
}

/// Replace `{token}` placeholders in a plain-status template.
fn format_status(template: &str, s: &StatusSnapshot) -> String {
    let duration = format_duration(s.duration_ms);
    let current = format_duration(s.position_ms);
    let volume = (s.volume * 100.0).round() as u64;
    let status = if !s.playing {
        "stopped"
    } else if s.paused {
        "paused"
    } else {
        "playing"
    };
    template
        .replace("{current}", &current)
        .replace("{position}", &current)
        .replace("{duration}", &duration)
        .replace("{artist}", &s.artist)
        .replace("{name}", &s.name)
        .replace("{album}", &s.album)
        .replace("{volume}", &volume.to_string())
        .replace("{status}", status)
        .replace("{mode}", &s.mode)
        .replace("{id}", &s.id.to_string())
        .replace("{liked}", if s.liked { "true" } else { "false" })
}

/// `boxpigma completions <shell>` handler: print a shell completion script for the
/// `boxpigma` CLI to stdout.
fn completions(shell: &str) -> color_eyre::Result<()> {
    let mut cmd = <Cli as clap::CommandFactory>::command();
    let mut out = std::io::stdout();
    match shell {
        "bash" => {
            clap_complete::generate(clap_complete::shells::Bash, &mut cmd, "boxpigma", &mut out)
        }
        "zsh" => {
            clap_complete::generate(clap_complete::shells::Zsh, &mut cmd, "boxpigma", &mut out)
        }
        "fish" => {
            clap_complete::generate(clap_complete::shells::Fish, &mut cmd, "boxpigma", &mut out)
        }
        "elvish" => clap_complete::generate(
            clap_complete::shells::Elvish,
            &mut cmd,
            "boxpigma",
            &mut out,
        ),
        "powershell" => clap_complete::generate(
            clap_complete::shells::PowerShell,
            &mut cmd,
            "boxpigma",
            &mut out,
        ),
        other => {
            color_eyre::eyre::bail!(
                "unsupported shell `{other}` (expected bash/zsh/fish/elvish/powershell)"
            )
        }
    }
    Ok(())
}

/// Split a `-d` value into its endpoint and optional 1-based playlist index.
/// Accepts both `endpoint` and `endpoint:N` (e.g. `toplist:3`). The legacy
/// global `--playlist` fallback stays for backward compatibility.
fn parse_daemon_endpoint(
    value: &str,
    playlist: Option<usize>,
) -> color_eyre::Result<(String, Option<usize>)> {
    if let Some((ep, idx)) = value.rsplit_once(':')
        && !ep.is_empty()
        && let Ok(n) = idx.parse::<usize>()
        && n > 0
    {
        return Ok((ep.to_string(), Some(n)));
    }
    if playlist.is_some() {
        return Ok((value.to_string(), playlist));
    }
    // A trailing `:N` that failed to parse is a user error, not an endpoint.
    if value.contains(':') {
        color_eyre::eyre::bail!(
            "invalid playlist index in `{value}` (expected `ENDPOINT:N` with N > 0)"
        );
    }
    Ok((value.to_string(), None))
}

/// Restore the default `SIGPIPE` disposition for the CLI subcommands.
///
/// Rust ignores `SIGPIPE` at startup, so writing to a closed pipe fails instead of ending the
/// process — and `println!` turns a failed stdout write into a panic. Under the release profile's
/// `panic = "abort"` that panic is `SIGABRT` plus a core dump: `boxpigma msg list | head` dumped
/// core where every other Unix tool would have exited quietly. The TUI is deliberately not
/// covered: it must keep running so it can restore the terminal.
fn exit_on_broken_pipe() {
    // SAFETY: setting a signal disposition to the default has no preconditions to uphold.
    unsafe { libc::signal(libc::SIGPIPE, libc::SIG_DFL) };
}

pub async fn run_cli(mut cli: Cli) -> color_eyre::Result<Option<Config>> {
    // A subcommand prints and exits, so it must not inherit the ignored `SIGPIPE` (see
    // `exit_on_broken_pipe`). The daemon path below owns this runtime; the interactive
    // path returns the config and lets `main` hand the terminal to `ferriso-tui`.
    if cli.command.is_some() {
        exit_on_broken_pipe();
    }

    let socket = match &cli.command {
        Some(Command::Status {
            socket: cmd_socket, ..
        })
        | Some(Command::Msg {
            socket: cmd_socket, ..
        }) => match cmd_socket {
            Some(s) => Some(s.clone()),
            None => cli.socket.take(),
        },
        _ => cli.socket.take(),
    };
    ipc::set_socket_path(socket);

    match &cli.command {
        Some(Command::Status {
            template,
            json,
            list,
            ..
        }) => {
            cli::status(template, *json, *list).await?;
            return Ok(None);
        }
        Some(Command::Msg {
            action,
            value,
            playlist,
            json,
            ..
        }) => {
            cli::msg(action, value.as_deref(), *playlist, *json).await?;
            return Ok(None);
        }
        Some(Command::Completions { shell }) => {
            cli::completions(shell)?;
            return Ok(None);
        }
        Some(Command::Update {
            check,
            version,
            dir,
            mirror,
            checksums,
            force,
            insecure,
            dry_run,
            rollback,
        }) => {
            update::run(update::Options {
                check: *check,
                version: version.clone(),
                dir: dir.clone(),
                mirror: mirror.clone(),
                checksums: checksums.clone(),
                force: *force,
                insecure: *insecure,
                dry_run: *dry_run,
                rollback: *rollback,
            })
            .await?;
            return Ok(None);
        }
        None => {}
    }

    let config = Config::load();
    init_logger(&config)?;

    if let Some(endpoint) = cli.daemon {
        let (endpoint, playlist) = parse_daemon_endpoint(&endpoint, cli.playlist)?;
        App::new(config, false)?
            .run_headless(&endpoint, playlist)
            .await?;
        return Ok(None);
    }

    Ok(Some(config))
}

/* -------------------------------------------------------------------------- */
/*                                   Testing                                  */
/* -------------------------------------------------------------------------- */

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot() -> StatusSnapshot {
        StatusSnapshot {
            id: 123,
            name: "Example Song".into(),
            artist: "Example Artist".into(),
            album: "Example Album".into(),
            duration_ms: 125000,
            position_ms: 65000,
            volume: 0.75,
            playing: true,
            paused: false,
            mode: "sequential".into(),
            liked: true,
        }
    }

    #[test]
    fn format_status_plain() {
        let out = format_status(
            "{current}/{duration} {artist} {name} {volume}% {status}",
            &snapshot(),
        );
        assert_eq!(out, "01:05/02:05 Example Artist Example Song 75% playing");
    }

    #[test]
    fn format_status_unknown_tokens_left_alone() {
        let out = format_status("{name} {bogus}", &snapshot());
        assert_eq!(out, "Example Song {bogus}");
    }

    #[test]
    fn parse_volume_absolute() {
        let action = parse_volume("75").unwrap();
        assert_eq!(
            action,
            MsgAction::Volume {
                delta: None,
                absolute: Some(0.75),
            }
        );
    }

    #[test]
    fn parse_volume_delta() {
        let plus = parse_volume("+5").unwrap();
        assert_eq!(
            plus,
            MsgAction::Volume {
                delta: Some(0.05),
                absolute: None,
            }
        );
        let minus = parse_volume("-10").unwrap();
        assert_eq!(
            minus,
            MsgAction::Volume {
                delta: Some(-0.10),
                absolute: None,
            }
        );
    }

    #[test]
    fn parse_volume_out_of_range() {
        assert!(parse_volume("150").is_err());
        assert!(parse_volume("abc").is_err());
    }

    #[test]
    fn parse_msg_play_with_optional_id() {
        let plain = parse_msg_action(ControlAction::Play, None, None).unwrap();
        assert_eq!(plain, MsgAction::Play { song_id: None });

        let with_id = parse_msg_action(ControlAction::Play, Some("187186"), None).unwrap();
        assert_eq!(
            with_id,
            MsgAction::Play {
                song_id: Some(187186)
            }
        );

        assert!(parse_msg_action(ControlAction::Play, Some("abc"), None).is_err());
        assert_eq!(
            parse_msg_action(ControlAction::TogglePlay, None, None).unwrap(),
            MsgAction::TogglePlay
        );
    }

    /// The two actions the Plan's "more `msg` actions" asked for: `seek` (whose value is
    /// required — without one there is nothing to jump to) and `clear` (which takes none).
    #[test]
    fn msg_seek_needs_a_value_and_clear_takes_none() {
        assert_eq!(
            parse_msg_action(ControlAction::Seek, Some("+15"), None).unwrap(),
            MsgAction::Seek {
                value: "+15".into()
            }
        );
        assert!(
            parse_msg_action(ControlAction::Seek, None, None).is_err(),
            "a seek without a value has nothing to jump to"
        );
        assert!(
            parse_msg_action(ControlAction::Seek, Some("abc"), None).is_err(),
            "the CLI rejects a seek value `:seek` would refuse, instead of mailing it to the daemon"
        );
        assert_eq!(
            parse_msg_action(ControlAction::Clear, None, None).unwrap(),
            MsgAction::Clear
        );
    }

    #[test]
    fn msg_switch_list_and_aliases() {
        assert_eq!(
            ipc::action_spec("switch").map(|s| s.name),
            Some("switch-list")
        );
        assert_eq!(ipc::action_spec("prev").map(|s| s.name), Some("previous"));
        assert_eq!(
            parse_msg_action(ControlAction::SwitchList, Some("toplist"), None).unwrap(),
            MsgAction::SwitchList {
                endpoint: "toplist".into(),
                playlist: None,
            }
        );
        assert_eq!(
            parse_msg_action(ControlAction::ToggleLike, None, None).unwrap(),
            MsgAction::ToggleLike
        );
        assert_eq!(
            parse_msg_action(ControlAction::Previous, None, None).unwrap(),
            MsgAction::Previous
        );
    }

    /// The CLI accepts every name `capabilities` publishes — canonical names and
    /// aliases alike — and resolves it to the action the catalogue names.
    #[test]
    fn every_published_action_parses() {
        for spec in ipc::ACTIONS {
            for name in std::iter::once(spec.name).chain(spec.aliases.iter().copied()) {
                let cli = Cli::try_parse_from(["boxpigma", "msg", name])
                    .unwrap_or_else(|e| panic!("`msg {name}` was rejected: {e}"));
                let Some(Command::Msg { action, .. }) = cli.command else {
                    panic!("`msg {name}` did not parse as the msg subcommand");
                };
                assert_eq!(action, name);
                assert_eq!(
                    ipc::action_spec(&action).map(|s| s.name),
                    Some(spec.name),
                    "`{name}` resolved to the wrong action"
                );
            }
        }
    }

    /// A control action that takes no value must not silently swallow one.
    #[tokio::test]
    async fn msg_rejects_a_value_for_an_action_that_takes_none() {
        let err = msg("next", Some("oops"), None, false)
            .await
            .expect_err("`next` takes no value");
        assert!(err.to_string().contains("takes no value"), "{err}");
    }

    #[test]
    fn print_queue_marks_current_song() {
        let queue = ipc::QueueSnapshot {
            current_index: Some(0),
            songs: vec![
                ipc::QueueEntry {
                    id: 1,
                    name: "Song A".into(),
                    singer: "Artist A".into(),
                    album: String::new(),
                    duration_ms: 125_000,
                },
                ipc::QueueEntry {
                    id: 2,
                    name: "Song B".into(),
                    singer: "Artist B".into(),
                    album: String::new(),
                    duration_ms: 65_000,
                },
            ],
        };
        let out = render_queue(&queue);
        let expected = format!(
            "▶01  {:<24}  {:<20}  {}\n {:02}  {:<24}  {:<20}  {}\n",
            "Song A",
            "Artist A",
            format_duration(125_000),
            2,
            "Song B",
            "Artist B",
            format_duration(65_000),
        );
        assert_eq!(out, expected);
    }

    #[test]
    fn completions_prints_script() {
        // Capturing stdout is awkward; instead verify the subcommand parses and
        // that each supported shell name is accepted by the parser.
        let cmd = Cli::try_parse_from(["boxpigma", "completions", "bash"]).unwrap();
        assert!(matches!(
            cmd.command,
            Some(Command::Completions { shell }) if shell == "bash"
        ));
        assert!(Cli::try_parse_from(["boxpigma", "completions", "zsh"]).is_ok());
        assert!(Cli::try_parse_from(["boxpigma", "completions", "fish"]).is_ok());
        assert!(Cli::try_parse_from(["boxpigma", "completions", "powershell"]).is_ok());
        assert!(Cli::try_parse_from(["boxpigma", "completions", "nushell"]).is_err());
    }
}
