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
