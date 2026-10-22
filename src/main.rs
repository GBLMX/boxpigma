//! Application entry point: parses CLI arguments, dispatches `status`/`msg`
//! subcommands, runs the headless daemon, or launches the actor-backed TUI.
//!
//! The interactive terminal is owned by `ferriso-tui`; the backend actor is
//! built inside that adapter's runtime. The CLI and daemon paths stay on their
//! one-shot Tokio runtime so they can exit without touching the terminal.

use boxpigma::cli::{Cli, run_cli};
use clap::{Parser, error::ErrorKind};

fn main() -> color_eyre::Result<()> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let cli = Cli::parse();

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let config = match runtime.block_on(run_cli(cli)) {
        Ok(config) => config,
        Err(err) => {
            let msg = err
                .chain()
                .next()
                .map(ToString::to_string)
                .unwrap_or_default();
            clap::Error::raw(ErrorKind::Io, msg)
                .print()
                .expect("failed to write error");
            eprintln!();
            std::process::exit(1);
        }
    };
    drop(runtime);

    let Some(config) = config else {
        return Ok(());
    };
    color_eyre::install()?;
    boxpigma::tui::run(config)?;
    Ok(())
}
