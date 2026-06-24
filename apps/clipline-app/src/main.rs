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
#[cfg(windows)]
mod cloud;
#[cfg(windows)]
mod cloud_upload;
#[cfg(windows)]
mod game_icon;
#[cfg(windows)]
mod game_plugins;
#[cfg(windows)]
mod games;
#[cfg(windows)]
mod hotkeys;
#[cfg(target_os = "macos")]
#[path = "hotkeys_macos.rs"]
mod hotkeys;
#[cfg(windows)]
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
#[cfg(windows)]
mod service;
#[cfg(target_os = "macos")]
#[path = "service_macos.rs"]
mod service;
#[cfg(any(windows, target_os = "macos"))]
mod settings;
#[cfg(windows)]
mod sound;
#[cfg(any(windows, target_os = "macos"))]
mod updates;
#[cfg(windows)]
mod util;
