//! boxpigma library crate: the netease cloud music TUI and its CLI helpers.
//!
//! The modules are exposed so the binary entry point (`src/main.rs`) and the
//! CLI subcommands (`src/cli.rs`) can share the app logic without duplicating
//! it. `main.rs` keeps only process-level side effects (terminal init, argument
//! dispatch); everything else lives here.

/// Timing helper for the `#[ignore]`d benchmarks — see any `bench_*` test.
///
/// `cargo test --release --lib -- --ignored --nocapture` prints them. Criterion would be
/// a dependency tree for a handful of numbers, and the question here is narrow: does the
/// work that runs while music plays stay far below its own frame budget?
#[cfg(test)]
pub(crate) mod bench_util {
    use std::time::Instant;

    /// Run `f` `iterations` times and print the mean cost in microseconds.
    pub fn time(label: &str, iterations: u32, mut f: impl FnMut()) -> f64 {
        for _ in 0..iterations.div_ceil(10) {
            f();
        }

        let start = Instant::now();
        for _ in 0..iterations {
            f();
        }
        let per_call = start.elapsed().as_secs_f64() / f64::from(iterations);
        println!("  {label:<38} {:>8.2} µs/次", per_call * 1e6);
        per_call
    }

    /// Share of one core that `calls_per_second` calls of `per_call` seconds would use.
    pub fn core_share(per_call: f64, calls_per_second: f64) -> f64 {
        per_call * calls_per_second * 100.0
    }
}

pub mod app;
pub mod cache;
pub mod cli;
pub mod config;
pub mod event;
pub mod input;
pub mod ipc;
pub mod key;
pub mod layout;
pub mod logger;
pub mod mpris;
pub mod playback;
pub mod service;
pub mod state;
pub mod text_input;
pub mod ui;
pub mod update;
pub mod utils;
