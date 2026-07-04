# Advanced Recording Settings Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add an Advanced toggle in Settings > Recording for exact resolution bounds, bitrate, and FPS while keeping preset controls as the normal path.

**Architecture:** Persist a small `advanced_recording` settings object. When disabled, service options keep using the existing preset resolution, video quality, and FPS. When enabled, service options use custom max width/height bounds that preserve aspect ratio, plus exact bitrate/FPS values. The vanilla Settings UI shows/hides numeric fields and keeps dirty indicators working through `data-settings-key`.

**Tech Stack:** Rust settings/service code, serde JSON persistence, vanilla HTML/CSS/JS Settings UI, Rust-driven UI contract and Boa JavaScript tests.

---

### Task 1: Settings Model and Service Values

**Files:**
- Modify: `apps/clipline-app/src/settings/types.rs`
- Modify: `apps/clipline-app/src/settings/mod.rs`
- Modify: `apps/clipline-app/src/settings/persistence.rs`
- Modify: `apps/clipline-app/src/settings/validation.rs`
- Modify: `apps/clipline-app/src/settings/tests.rs`
- Modify: `apps/clipline-app/src/service.rs`

- [ ] **Step 1: Write failing Rust settings tests**

Add tests proving advanced settings persist, sanitize, and drive service options:

```rust
#[test]
fn advanced_recording_overrides_preset_service_values() {
    let settings = AppSettings {
        advanced_recording: AdvancedRecordingSettings {
            enabled: true,
            output_width: 1600,
            output_height: 900,
            bitrate_mbps: 13.5,
            fps: 75,
        },
        output_resolution: OutputResolution::P720,
        video_quality: VideoQuality::Compact,
        bitrate_mbps: 2.5,
        fps: 30,
        ..AppSettings::default()
    };

    let opts = settings.to_service_options(None).unwrap();

    assert_eq!(opts.output_resolution, OutputResolution::P720);
    assert_eq!(opts.output_resolution_bounds.unwrap().width, 1600);
    assert_eq!(opts.output_resolution_bounds.unwrap().height, 900);
    assert_eq!(opts.bitrate_bps, 13_500_000);
    assert_eq!(opts.fps, 75);
}

#[test]
fn advanced_recording_load_repairs_numeric_values() {
    let value = serde_json::json!({
        "advanced_recording": {
            "enabled": true,
            "output_width": 1919,
            "output_height": 1079,
            "bitrate_mbps": 17.25,
            "fps": 75
        }
    });

    let settings = AppSettings::load_from_object(value.as_object().unwrap());

    assert!(settings.advanced_recording.enabled);
    assert_eq!(settings.advanced_recording.output_width, 1920);
    assert_eq!(settings.advanced_recording.output_height, 1080);
    assert_eq!(settings.advanced_recording.bitrate_mbps, 17.25);
    assert_eq!(settings.advanced_recording.fps, 75);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p clipline-app settings::tests::advanced_recording -- --nocapture`

Expected: FAIL because `AdvancedRecordingSettings` and `output_resolution_bounds` do not exist yet.

- [ ] **Step 3: Implement minimal Rust model and service support**

Add `AdvancedRecordingSettings` with defaults, repair helpers for width/height, bitrate, and FPS, `effective_fps()`, `effective_bitrate_mbps()`, and `effective_output_resolution_bounds()`. Add `OutputResolutionBounds` to `service.rs`, use it in `ServiceOptions`, and make `output_dimensions` prefer custom bounds when present.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p clipline-app settings::tests::advanced_recording -- --nocapture`

Expected: PASS.

### Task 2: Recording UI Contract and Core Labels

**Files:**
- Modify: `apps/clipline-app/tests/ui_contract.rs`
- Modify: `apps/clipline-app/tests/player_core.rs`
- Modify: `apps/clipline-app/ui/player-core.js`
- Modify: `apps/clipline-app/ui/index.html`
- Modify: `apps/clipline-app/ui/settings.js`
- Modify: `apps/clipline-app/ui/main.js`
- Modify: `apps/clipline-app/ui/styles.css`

- [ ] **Step 1: Write failing UI and JS tests**

Update UI contract tests to require `set-recording-advanced`, `advanced-recording-fields`, `set-output-width`, `set-output-height`, `set-custom-bitrate`, and `set-custom-fps`. Update the player core quality-label test to expect a bitrate label helper such as `Sharp quality - more detail. 24 Mbps.`

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p clipline-app --test ui_contract recording`

Run: `cargo test -p clipline-app --test player_core recording_quality_labels_hide_bitrate_jargon`

Expected: FAIL because the controls and label helper do not exist yet.

- [ ] **Step 3: Implement minimal UI**

Add the Advanced checkbox and numeric fields below the preset recording controls. Wire fill/read/sync logic so Advanced values are read from and written to `advanced_recording`, preset controls remain visible, and custom fields are disabled/hidden when Advanced is off. Update `quality-summary` to include the current preset bitrate.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p clipline-app --test ui_contract recording`

Run: `cargo test -p clipline-app --test player_core recording_quality_labels_hide_bitrate_jargon`

Expected: PASS.

### Task 3: Full Verification and Commit

**Files:**
- Verify all modified files.

- [ ] **Step 1: Run focused tests**

Run:

```powershell
cargo test -p clipline-app settings::tests::advanced_recording
cargo test -p clipline-app --test ui_contract
cargo test -p clipline-app --test player_core
```

Expected: PASS.

- [ ] **Step 2: Run workspace gates**

Run:

```powershell
cargo test --workspace
cargo clean -p clipline-app
cargo clippy --workspace --all-targets -- -D warnings
```

Expected: PASS.

- [ ] **Step 3: Commit implementation**

Commit with:

```powershell
git add apps/clipline-app docs/superpowers/plans/2026-07-04-advanced-recording-settings.md
git commit -m "feat(settings): add advanced recording controls"
```
