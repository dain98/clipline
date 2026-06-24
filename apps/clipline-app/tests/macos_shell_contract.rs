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
        "#[cfg(any(windows, target_os = \"macos\"))]\nmod cloud;",
        "#[cfg(any(windows, target_os = \"macos\"))]\nmod game_icon;",
        "#[cfg(any(windows, target_os = \"macos\"))]\nmod library;",
        "#[cfg(any(windows, target_os = \"macos\"))]\nmod settings;",
        "#[cfg(any(windows, target_os = \"macos\"))]\nmod platform;",
        "#[cfg(any(windows, target_os = \"macos\"))]\nmod video_encoder;",
        "#[cfg(target_os = \"macos\")]\n#[path = \"service_macos.rs\"]\nmod service;",
    ] {
        assert!(
            main_rs.contains(required),
            "missing macOS module declaration: {required}"
        );
    }
}

#[test]
fn tracked_macos_modules_are_declared_or_intentionally_absent() {
    let main_rs = main_rs();
    let src_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    for entry in fs::read_dir(&src_dir).expect("read src dir") {
        let entry = entry.expect("read src entry");
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !name.ends_with("_macos.rs") {
            continue;
        }

        let module = name.trim_end_matches(".rs").trim_end_matches("_macos");
        let expected_path_attr = format!("#[path = \"{name}\"]\nmod {module}");
        assert!(
            main_rs.contains(&expected_path_attr),
            "{name} is tracked but not wired through main.rs"
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
fn macos_capabilities_do_not_offer_permission_actions_for_unimplemented_features() {
    let macos =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/platform/macos.rs"))
            .expect("read platform/macos.rs");

    for feature in [
        "display_capture",
        "window_capture",
        "display_region_capture",
        "microphone",
        "in_game_hotkey_fallback",
    ] {
        let assignment = format!("{feature}: CapabilityStatus::unavailable(");
        assert!(
            macos.contains(&assignment),
            "{feature} should be unavailable, not a permission action, until implemented"
        );
    }
    assert!(
        !macos.contains("CapabilityStatus::needs_permission("),
        "macOS Milestone 1 capabilities should not imply Settings can enable unimplemented features"
    );
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
        "pub use crate::video_encoder::{codec_id, EncoderOption, VideoEncoder};",
        "pub fn spawn(opts: ServiceOptions) -> (Sender<Cmd>, Receiver<Event>)",
        "pub fn ensure_recording_available() -> Result<(), String>",
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
        "encoder: \"Unavailable on macOS Milestone 1\".into(),",
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

#[test]
fn macos_recording_start_fails_before_spawning_stub_service() {
    let app = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/app.rs"))
        .expect("read app.rs");
    let service =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/service_macos.rs"))
            .expect("read service_macos.rs");

    assert!(
        service.contains(
            "pub fn ensure_recording_available() -> Result<(), String> {\n    Err(\"macOS recording is not implemented in Milestone 1\".into())\n}"
        ),
        "macOS must expose recording as unavailable before app start_recording can spawn the stub"
    );
    let guard = app
        .find("service::ensure_recording_available()?;")
        .expect("start_recording should check service availability");
    let spawn = app
        .find("let (tx, rx) = service::spawn(Self::options(&inner)?);")
        .expect("start_recording should spawn the service after the availability check");
    assert!(
        guard < spawn,
        "recording availability must be checked before spawning the service"
    );
}

#[test]
fn app_commands_use_platform_facade() {
    let app = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/app.rs"))
        .expect("read app.rs");

    for forbidden in [
        "clipline_capture::windows::display::enumerate_displays",
        "clipline_capture::windows::wasapi::enumerate_audio_devices",
        "crate::memory::current_process_tree_memory()",
    ] {
        assert!(
            !app.contains(forbidden),
            "app.rs should call platform facade instead of {forbidden}"
        );
    }

    for required in [
        "use crate::platform::{AudioDeviceLists, DisplayInfo};",
        "crate::platform::list_displays()",
        "crate::platform::list_audio_devices()",
        "crate::platform::memory_status()",
        "#[cfg(windows)]\nfn start_microphone_test_windows",
        "#[cfg(target_os = \"macos\")]\n    {\n        let _ = (app, state, device_id, volume, mono);\n        Err(\"macOS microphone test is not implemented in Milestone 1\".into())",
        "#[cfg(not(any(windows, target_os = \"macos\")))]\n    {\n        let _ = (app, state, device_id, volume, mono);\n        Err(\"Microphone test is unsupported on this platform\".into())",
    ] {
        assert!(
            app.contains(required),
            "missing platform app usage: {required}"
        );
    }
}

#[test]
fn macos_hotkey_and_memory_stubs_exist() {
    let hotkeys =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/hotkeys_macos.rs"))
            .expect("read hotkeys_macos.rs");
    let memory =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/memory_macos.rs"))
            .expect("read memory_macos.rs");
    let platform =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/platform/macos.rs"))
            .expect("read platform/macos.rs");

    assert!(hotkeys.contains("pub fn install_save_hook"));
    assert!(hotkeys.contains("focused-game hotkey fallback"));
    assert!(hotkeys.contains("pub fn set_save_hotkey"));
    assert!(memory.contains("const MACOS_PS: &str = \"/bin/ps\";"));
    assert!(memory.contains("const MACOS_PGREP: &str = \"/usr/bin/pgrep\";"));
    assert!(memory.contains("Command::new(MACOS_PS)"));
    assert!(memory.contains("Command::new(MACOS_PGREP)"));
    assert!(memory.contains("parse_ps_rss_kib"));
    assert!(!memory.contains("macOS memory status is not implemented in Milestone 1"));
    assert!(platform.contains("crate::memory::current_process_tree_memory()"));
    assert!(!platform.contains("macOS memory status is not implemented in Milestone 1"));
}

#[test]
fn macos_bundle_and_update_status_are_explicit() {
    let config = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tauri.conf.json"))
        .expect("read tauri.conf.json");
    let app = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/app.rs"))
        .expect("read app.rs");

    assert!(
        config.contains("\"identifier\": \"io.clipline.desktop\""),
        "macOS bundle identifier should not end with .app"
    );
    assert!(config.contains("\"targets\": [\"nsis\", \"dmg\", \"app\"]"));
    assert!(
        !config.contains("for Windows"),
        "bundle product copy should not describe Clipline as Windows-only"
    );
    assert!(
        app.contains("macos_update_artifact_missing_message")
            && app.contains("No macOS update artifact is published yet"),
        "macOS updater artifact gaps should return an actionable status"
    );
}

#[test]
fn macos_screencapturekit_helper_is_built_and_bundled() {
    let build = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("build.rs"))
        .expect("read build.rs");
    let base_config =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tauri.conf.json"))
            .expect("read tauri.conf.json");

    assert!(build.contains("ScreenCaptureKitHelper.swift"));
    assert!(build.contains("xcrun"));
    assert!(build.contains("swiftc"));
    assert!(build.contains("clipline-sidecars"));
    assert!(build.contains("clipline-sck-helper"));
    assert!(build.contains("cargo:rerun-if-changed=macos/ScreenCaptureKitHelper.swift"));
    assert!(
        !base_config.contains("clipline-sck-helper"),
        "base Tauri config should not include the macOS-only helper resource"
    );

    let macos_config =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tauri.macos.conf.json"))
            .expect("read tauri.macos.conf.json");
    assert!(macos_config.contains("\"../../target/clipline-sidecars/clipline-sck-helper\": \"\""));
}

#[test]
fn macos_capture_wrapper_uses_helper_resource_and_protocol_magic() {
    let main_rs = main_rs();
    let capture =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/macos_capture.rs"))
            .expect("read macos_capture.rs");
    assert!(main_rs.contains("#[cfg(target_os = \"macos\")]\nmod macos_capture;"));
    assert!(capture.contains("CLIPLINE_SCK_HELPER"));
    assert!(capture.contains("clipline-sck-helper"));
    assert!(capture.contains("b\"CLNV\""));
    assert!(capture.contains("b\"FRAM\""));
    assert!(capture.contains("impl CaptureEngine for ScreenCaptureKitCapture"));
}

#[test]
fn macos_asset_scope_allows_default_movies_folder() {
    let config = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tauri.conf.json"))
        .expect("read tauri.conf.json");
    assert!(config.contains("\"**/Movies/Clipline/*.mp4\""));
    assert!(config.contains("\"**/Movies/Clipline/**/*.mp4\""));
}

#[test]
fn os_specific_helpers_are_cfg_gated() {
    let persistence = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/settings/persistence.rs"),
    )
    .expect("read persistence.rs");
    let util = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/util.rs"))
        .expect("read util.rs");
    let cloud = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/cloud.rs"))
        .expect("read cloud.rs");
    let library = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/library.rs"))
        .expect("read library.rs");
    let game_icon =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/game_icon.rs"))
            .expect("read game_icon.rs");

    assert!(persistence.contains("#[cfg(windows)]\nfn replace_file"));
    assert!(persistence.contains("#[cfg(not(windows))]\nfn replace_file"));
    assert!(util.contains("#[cfg(windows)]\npub(crate) fn wide_null"));
    assert!(cloud.contains("#[cfg(windows)]\nfn write_credential"));
    assert!(cloud.contains("#[cfg(target_os = \"macos\")]\nfn write_credential"));
    assert!(library.contains("#[cfg(windows)]\nfn copy_file_to_clipboard"));
    assert!(library.contains("#[cfg(target_os = \"macos\")]\nfn copy_file_to_clipboard"));
    assert!(game_icon.contains("#[cfg(target_os = \"macos\")]\npub fn extract_exe_icon_png"));
}

#[test]
fn macos_cloud_credentials_use_keychain_before_network_uploads() {
    let manifest = manifest();
    let cloud = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/cloud.rs"))
        .expect("read cloud.rs");

    assert!(
        manifest.contains("[target.'cfg(target_os = \"macos\")'.dependencies]")
            && manifest.contains("security-framework = \"3.7.0\""),
        "macOS builds should depend directly on security-framework for Keychain access"
    );
    assert!(
        !cloud.contains("macOS cloud connect is unavailable until Keychain storage is implemented"),
        "cloud connect should no longer be blocked on macOS"
    );
    assert!(cloud.contains("const KEYCHAIN_SERVICE: &str = \"Clipline Cloud\";"));
    assert!(cloud.contains("security_framework::os::macos::keychain::SecKeychain"));
    assert!(cloud.contains("security_framework::os::macos::passwords::find_generic_password"));
    assert!(cloud.contains("security_framework::passwords::delete_generic_password"));
    assert!(cloud.contains("security_framework_sys::base::errSecItemNotFound"));
    assert!(cloud.contains("set_generic_password(KEYCHAIN_SERVICE, target, token.as_bytes())"));
    assert!(cloud.contains("find_generic_password(None, KEYCHAIN_SERVICE, target)"));
    assert!(
        cloud.contains("delete_generic_password(KEYCHAIN_SERVICE, target)"),
        "macOS disconnect should delete Keychain items through the Result-returning API"
    );
    assert!(
        cloud.contains("error.code() == errSecItemNotFound"),
        "macOS disconnect should only ignore Keychain item-not-found errors"
    );
    let network = cloud
        .find("clipline_cloud_api::connect_with_device_token")
        .expect("cloud_connect should use real network connect");
    let write = cloud
        .find("write_credential(&target, &result.username, &result.token)?;")
        .expect("cloud_connect should persist the returned token");
    assert!(
        network < write,
        "token should be stored after a successful connect"
    );
}

#[test]
fn macos_file_actions_are_native_and_available() {
    let library = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/library.rs"))
        .expect("read library.rs");
    let macos =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/platform/macos.rs"))
            .expect("read platform/macos.rs");

    assert!(
        library.contains("fn reveal_file_path(path: &Path) -> Result<(), String>"),
        "reveal_clip should reveal the selected clip, not only open its parent folder"
    );
    assert!(
        library.contains("const MACOS_OPEN: &str = \"/usr/bin/open\";")
            && library.contains("Command::new(MACOS_OPEN)")
            && library.contains(".arg(\"-R\")"),
        "macOS reveal should use Finder's open -R behavior"
    );
    assert!(
        library.contains("const MACOS_OSASCRIPT: &str = \"/usr/bin/osascript\";")
            && library.contains("Command::new(MACOS_OSASCRIPT)")
            && library.contains("set the clipboard to POSIX file"),
        "macOS clipboard copy should put a Finder file on the pasteboard"
    );
    assert!(
        library.contains("fn escape_applescript_string(raw: &str) -> String"),
        "AppleScript command text must escape paths before invoking osascript"
    );
    assert!(
        macos.contains("file_clipboard: CapabilityStatus::available()"),
        "Finder clipboard copy should be advertised once implemented"
    );
}
