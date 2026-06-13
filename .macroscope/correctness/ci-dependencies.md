---
include:
  - ".github/workflows/**"
  - "Cargo.toml"
  - "**/Cargo.toml"
  - "Cargo.lock"
---

Review CI and dependency changes for:

- CI still runs `cargo test --workspace` and `cargo clippy --workspace --all-targets -- -D warnings` on Ubuntu and Windows.
- Hosted runners do not require real capture devices, hardware encoders, audio endpoints, or WebView2 UI interaction.
- MP4 tests keep `ffprobe` available where needed.
- New dependencies have compatible licenses and do not undermine the no-injection, no-telemetry, LGPL FFmpeg, or permissive first-party licensing constraints.
- `Cargo.lock` changes do not introduce suspicious major-version jumps, duplicate dependency trees, security-sensitive crates, or license risk.
