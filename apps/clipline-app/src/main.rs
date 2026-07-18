#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

#[cfg(not(windows))]
fn main() {
    eprintln!("clipline-app is Windows-only (capture/encode are platform-bound)");
}

#[cfg(windows)]
fn main() {
    if let Err(error) = windows::wait_for_elevation_parent_from_args() {
        eprintln!("administrator restart handoff: {error}");
        return;
    }
    app::run();
}

#[cfg(windows)]
mod app;
#[cfg(windows)]
mod cloud;
#[cfg(windows)]
mod cloud_upload;
#[cfg(windows)]
mod game_discovery;
#[cfg(windows)]
mod game_icon;
#[cfg(windows)]
mod game_plugins;
#[cfg(windows)]
mod games;
#[cfg(windows)]
mod hotkeys;
#[cfg(windows)]
mod library;
#[cfg(windows)]
mod markers;
#[cfg(windows)]
mod memory;
#[cfg(windows)]
mod osu_api;
#[cfg(windows)]
mod osu_enrichment;
#[cfg(windows)]
mod poster;
#[cfg(windows)]
mod service;
#[cfg(windows)]
mod settings;
#[cfg(windows)]
mod sound;
#[cfg(windows)]
mod updates;
#[cfg(windows)]
mod util;
#[cfg(windows)]
mod windows;
