use std::fs;
use std::path::Path;

fn manifest() -> String {
    fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"))
        .expect("read Cargo.toml")
}

fn main_rs() -> String {
    fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/main.rs"))
        .expect("read src/main.rs")
}

#[test]
fn app_runtime_dependencies_are_available_on_macos() {
    let manifest = manifest();
    assert!(
        manifest.contains("[target.'cfg(any(windows, target_os = \"macos\"))'.dependencies]"),
        "Tauri and shared app dependencies must be available to both Windows and macOS"
    );
    assert!(
        manifest.contains("[target.'cfg(windows)'.dependencies]\nwindows-sys"),
        "windows-sys should remain Windows-only"
    );
}

#[test]
fn real_app_entrypoint_runs_on_windows_and_macos() {
    let main_rs = main_rs();
    assert!(
        main_rs.contains("#[cfg(any(windows, target_os = \"macos\"))]\nfn main()"),
        "Windows and macOS should run app::run()"
    );
    assert!(
        main_rs.contains("#[cfg(not(any(windows, target_os = \"macos\")))]\nfn main()"),
        "other targets should keep a stub main"
    );
    assert!(
        !main_rs.contains("clipline-app is Windows-only"),
        "macOS should no longer use the old Windows-only runtime message"
    );
}

#[test]
fn real_modules_are_declared_for_macos() {
    let main_rs = main_rs();
    for required in [
        "#[cfg(any(windows, target_os = \"macos\"))]\nmod app;",
        "#[cfg(any(windows, target_os = \"macos\"))]\nmod settings;",
        "#[cfg(any(windows, target_os = \"macos\"))]\nmod platform;",
        "#[cfg(target_os = \"macos\")]\n#[path = \"service_macos.rs\"]\nmod service;",
    ] {
        assert!(
            main_rs.contains(required),
            "missing macOS module declaration: {required}"
        );
    }
}

#[test]
fn platform_facade_exposes_macos_capability_model() {
    let types =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/platform/types.rs"))
            .expect("read platform/types.rs");

    for required in [
        "pub struct PlatformCapabilities",
        "pub in_game_hotkey_fallback: CapabilityStatus",
        "pub hardware_encode: CapabilityStatus",
        "pub hdr_capture: CapabilityStatus",
        "pub player_decode: CapabilityStatus",
        "pub struct CapturableWindow",
    ] {
        assert!(
            types.contains(required),
            "missing platform type: {required}"
        );
    }
}

#[test]
fn game_detection_uses_platform_window_type() {
    let games = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/games.rs"))
        .expect("read games.rs");
    let plugins =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/game_plugins.rs"))
            .expect("read game_plugins.rs");

    assert!(games.contains("use crate::platform::CapturableWindow;"));
    assert!(plugins.contains("use crate::platform::CapturableWindow;"));
    assert!(
        !games.contains("clipline_capture::windows::CapturableWindow"),
        "game detection should not import Windows window types directly"
    );
    assert!(
        !plugins.contains("clipline_capture::windows::CapturableWindow"),
        "game plugins should not import Windows window types directly"
    );
}

#[test]
fn macos_service_stub_exposes_app_facing_contract() {
    let service =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/service_macos.rs"))
            .expect("read service_macos.rs");

    for required in [
        "pub enum Cmd",
        "pub enum Event",
        "pub struct ServiceOptions",
        "pub enum CaptureBackend",
        "pub enum VideoEncoder",
        "pub fn spawn(opts: ServiceOptions) -> (Sender<Cmd>, Receiver<Event>)",
        "pub fn default_clips_dir() -> PathBuf",
        "pub fn clips_dir(root: &Path) -> Result<PathBuf, String>",
        "pub fn available_encoder_options() -> Vec<EncoderOption>",
    ] {
        assert!(
            service.contains(required),
            "missing service contract: {required}"
        );
    }

    for required in [
        "Vec::new()",
        ".name(\"clipline-recorder-stub\".into())",
        "encoder: \"Unavailable on macOS M1\".into(),",
        "message: \"macOS recording is not implemented in Milestone 1\".into(),",
        "Cmd::Stop { announce } => {",
        "if announce {",
    ] {
        assert!(
            service.contains(required),
            "missing macOS stub behavior: {required}"
        );
    }
}
