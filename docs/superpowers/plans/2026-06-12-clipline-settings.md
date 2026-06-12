# Clipline Settings (Milestone 9) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans or
> superpowers:subagent-driven-development to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the current launch-flag-only tuning with persisted, in-app settings for the
recorder basics. **Exit criterion:** the app loads settings from disk, lets the user change capture
target, replay/buffer length, bitrate, FPS, disk quota, and hotkey, saves them to
`%APPDATA%\Clipline\settings.json`, restarts the recorder service with the new recording settings,
rebinds the save hotkey, and keeps the storage row consistent with the configured quota.

**Architecture:** Add `apps/clipline-app/src/settings.rs` as the Windows app's persisted config
model. Keep it Tauri-free where possible so validation and option mapping have ordinary unit tests.
`AppSettings` is serde JSON with conservative defaults:

- primary monitor capture by default; optional window title substring target
- 120 s buffer, 60 s replay save window
- 12 Mbps H.264, 60 fps
- 10 GiB disk quota (`0` disables GC)
- `Alt+F10` save hotkey

`AppSettings::to_service_options(lol_override)` computes the existing `ServiceOptions`, including a
byte buffer estimate from buffer seconds and bitrate. Runtime app state moves from a one-shot
`CmdChannel` to a small `RuntimeState` that can swap the service command sender after
`save_settings`. Service events are pumped from every spawned service into the same webview. The
global shortcut handler reads the current runtime state at press time, so a restarted service still
receives Save. Hotkey parsing is deliberately narrow and testable: modifier-plus-F1..F24 strings
such as `Alt+F10`, `Ctrl+Alt+F10`, and `Ctrl+Shift+F9`.

**Tech Stack:** no new runtime dependencies. Use `serde_json` already present in the app.

---

### Task 1: persisted settings model

**Files:** `apps/clipline-app/src/settings.rs`, `apps/clipline-app/src/main.rs`.

- [ ] **Step 1: failing tests**

```rust
#[test]
fn defaults_match_current_recorder_behavior() { /* 60s replay, 12 Mbps, 60 fps, 10 GiB */ }

#[test]
fn validation_rejects_replay_longer_than_buffer() { /* replay > buffer */ }

#[test]
fn service_options_include_estimated_buffer_bytes() { /* buffer_seconds -> byte budget */ }

#[test]
fn settings_round_trip_json() { /* save_to/load_from temp file */ }
```

- [ ] **Step 2: implement**

```rust
#[derive(Clone, Serialize, Deserialize)]
pub struct AppSettings { ... }

impl AppSettings {
    pub fn validate(&self) -> Result<(), String>;
    pub fn to_service_options(&self, lol_url: Option<String>) -> Result<ServiceOptions, String>;
    pub fn load_or_default() -> Self;
    pub fn save(&self) -> Result<(), String>;
}
```

### Task 2: hotkey parsing and rebinding

**Files:** `apps/clipline-app/src/settings.rs`, `apps/clipline-app/src/app.rs`.

- [ ] **Step 1: failing tests**

```rust
#[test]
fn parses_alt_f10_hotkey() { ... }

#[test]
fn rejects_non_function_key_hotkeys() { ... }
```

- [ ] **Step 2: implement** `parse_hotkey(&str) -> Result<Shortcut, String>` and a normalized
string helper. Keep support intentionally to modifiers plus `F1`..`F24`.
- [ ] Rework the global shortcut handler to send `Cmd::Save` through runtime state, not a captured
startup sender.
- [ ] On `save_settings`, unregister the old hotkey, register the new one, and only persist the new
settings after the new hotkey registers successfully.

### Task 3: runtime restart + Tauri commands

**Files:** `apps/clipline-app/src/app.rs`, `apps/clipline-app/src/library.rs`.

- [ ] Replace `CmdChannel` with runtime state that can swap the active sender.
- [ ] Add commands:

```rust
#[tauri::command]
fn get_settings(...) -> AppSettings;

#[tauri::command]
fn save_settings(settings: AppSettings, ...) -> Result<AppSettings, String>;
```

- [ ] `save_settings` validates, saves JSON, stops the old service, spawns a new service with the
new `ServiceOptions`, starts a new event pump, updates the storage quota state, and returns the
normalized settings.
- [ ] Preserve CLI overrides for `--window`, `--lol-url`, and `--disk-quota-gb` at startup.

### Task 4: settings UI

**Files:** `apps/clipline-app/ui/index.html`.

- [ ] Add a compact Settings section below the status block and above Library.
- [ ] Controls:
  - capture mode segmented/select: primary monitor or window title
  - window title input
  - buffer seconds input
  - replay seconds input
  - bitrate Mbps input
  - FPS select
  - disk quota GiB input (`0` disables GC)
  - hotkey text input
- [ ] On load, call `get_settings`; on Save, call `save_settings`, refresh storage, and show a short
status notice.
- [ ] Keep the tray-app visual density; no landing-page treatment and no explanatory feature text.

### Task 5: gates and handoff

- [ ] `cargo test --workspace`
- [ ] `cargo clippy --workspace --all-targets`
- [ ] Optional live check: `cargo run -p clipline-app -- --disk-quota-gb 0`
- [ ] Push and verify CI on Ubuntu + Windows.
- [ ] Update `handoff.md` with milestone 9 status and the next frontier.

---

## Out of scope

- Frame-accurate trim/export.
- Multi-monitor picker with thumbnails.
- Arbitrary keyboard hotkeys beyond modifier-plus-function-key combinations.
- Live bitrate/fps changes without restarting the capture pipeline.
