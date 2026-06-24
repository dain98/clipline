#[cfg(windows)]
#[path = "app_windows.rs"]
mod imp;

#[cfg(windows)]
pub(crate) use imp::run;

#[cfg(target_os = "macos")]
pub(crate) fn run() {
    eprintln!("clipline-app macOS shell stubs are active");
}
