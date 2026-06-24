#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

#[cfg(not(any(windows, target_os = "macos")))]
fn main() {
    eprintln!("clipline-app has a desktop runtime on Windows and macOS only");
}

#[cfg(any(windows, target_os = "macos"))]
fn main() {
    app::run();
}

#[cfg(any(windows, target_os = "macos"))]
mod app;
#[cfg(any(windows, target_os = "macos"))]
mod cloud;
#[cfg(any(windows, target_os = "macos"))]
mod cloud_upload;
#[cfg(any(windows, target_os = "macos"))]
mod game_icon;
#[cfg(windows)]
mod game_plugins;
#[cfg(target_os = "macos")]
#[path = "game_plugins_macos.rs"]
mod game_plugins;
#[cfg(windows)]
mod games;
#[cfg(target_os = "macos")]
#[path = "games_macos.rs"]
mod games;
#[cfg(windows)]
mod hotkeys;
#[cfg(target_os = "macos")]
#[path = "hotkeys_macos.rs"]
mod hotkeys;
#[cfg(any(windows, target_os = "macos"))]
mod library;
#[cfg(windows)]
mod markers;
#[cfg(windows)]
mod memory;
#[cfg(target_os = "macos")]
#[path = "memory_macos.rs"]
mod memory;
#[cfg(any(windows, target_os = "macos"))]
mod platform;
#[cfg(windows)]
mod poster;
#[cfg(target_os = "macos")]
#[path = "poster_macos.rs"]
mod poster;
#[cfg(windows)]
mod service;
#[cfg(target_os = "macos")]
#[path = "service_macos.rs"]
mod service;
#[cfg(any(windows, target_os = "macos"))]
mod settings;
#[cfg(any(windows, target_os = "macos"))]
mod sound;
#[cfg(any(windows, target_os = "macos"))]
mod updates;
#[cfg(any(windows, target_os = "macos"))]
mod util;
