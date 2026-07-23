fn main() {
    // The Tauri context only exists for Windows builds (see Cargo.toml).
    if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
        println!("cargo:rerun-if-env-changed=CLIPLINE_BUG_REPORT_ENDPOINT");
        let endpoint = std::env::var("CLIPLINE_BUG_REPORT_ENDPOINT").ok();
        if std::env::var("PROFILE").as_deref() == Ok("release") {
            let endpoint = endpoint
                .as_deref()
                .expect("release builds require CLIPLINE_BUG_REPORT_ENDPOINT");
            assert!(
                endpoint.starts_with("https://"),
                "release CLIPLINE_BUG_REPORT_ENDPOINT must use https"
            );
        }
        if let Some(endpoint) = endpoint {
            println!("cargo:rustc-env=CLIPLINE_BUG_REPORT_ENDPOINT={endpoint}");
        }
        tauri_build::build();
    }
}
