#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

#[cfg(not(windows))]
fn main() {
    eprintln!("clipline-app is Windows-only (capture/encode are platform-bound)");
}

#[cfg(windows)]
fn main() {
    app::run();
}

#[cfg(windows)]
mod app;
#[cfg(windows)]
mod library;
#[cfg(windows)]
mod markers;
#[cfg(windows)]
mod service;
#[cfg(windows)]
mod settings;
