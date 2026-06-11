fn main() {
    // The Tauri context only exists for Windows builds (see Cargo.toml).
    if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
        tauri_build::build();
    }
}
