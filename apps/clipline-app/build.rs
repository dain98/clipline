use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os == "macos" {
        build_macos_helper();
        tauri_build::build();
    } else if target_os == "windows" {
        tauri_build::build();
    }
}

fn build_macos_helper() {
    println!("cargo:rerun-if-changed=macos/ScreenCaptureKitHelper.swift");
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let helper_src = manifest_dir.join("macos/ScreenCaptureKitHelper.swift");
    let out_dir = repo_root(&manifest_dir).join("target/clipline-sidecars");
    std::fs::create_dir_all(&out_dir).expect("create target/clipline-sidecars");
    let helper_out = out_dir.join("clipline-sck-helper");
    let status = Command::new("xcrun")
        .args([
            "swiftc",
            "-O",
            "-target",
            "arm64-apple-macosx13.0",
            "-framework",
            "Foundation",
            "-o",
        ])
        .arg(&helper_out)
        .arg(&helper_src)
        .status()
        .expect("spawn xcrun swiftc for ScreenCaptureKit helper");
    assert!(
        status.success(),
        "swiftc failed for ScreenCaptureKit helper"
    );
}

fn repo_root(manifest_dir: &Path) -> PathBuf {
    manifest_dir
        .parent()
        .and_then(Path::parent)
        .expect("apps/clipline-app lives two levels below repo root")
        .to_path_buf()
}
