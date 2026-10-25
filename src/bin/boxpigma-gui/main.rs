//! `boxpigma-gui`: the desktop frontend.
//!
//! Ferriso owns the Tokio runtime, the backend actor and the iced window. This
//! binary only builds the backend and hands it the native renderer.

mod app;
mod view;

use boxpigma::{backend::Backend, config::Config};

fn main() -> iced::Result {
    // `reqwest` is built with `rustls-no-provider`; the TUI installs `ring` in `main.rs` before
    // the first request and the GUI must do the same, or building an HTTP client panics.
    let _ = rustls::crypto::ring::default_provider().install_default();

    ferriso_gui::run_with_app(
        || {
            let config = Config::load();
            let _ = boxpigma::logger::init_logger(&config);
            Backend::new(config, true).expect("boxpigma-gui: backend")
        },
        app::Gui::new(),
        ferriso_gui::Settings {
            title: "boxpigma".into(),
            size: (1180.0, 780.0),
            exit_on_close_request: true,
            application_id: "io.github.gblmx.boxpigma".into(),
        },
    )
}
