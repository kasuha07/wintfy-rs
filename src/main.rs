#![cfg_attr(
    all(not(debug_assertions), not(feature = "debug-console")),
    windows_subsystem = "windows"
)]

mod app;
mod args;
mod config;
mod logging;
mod ntfy;
mod platform;
mod toast;
mod tray;
mod util;
mod worker;

use std::process::ExitCode;

fn main() -> ExitCode {
    match app::run() {
        Ok(code) => code,
        Err(err) => {
            eprintln!("wintfy-rs: {err}");
            ExitCode::FAILURE
        }
    }
}
