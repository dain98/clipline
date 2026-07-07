# Focus-Follow Capture Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add an optional Game detection mode that follows the foreground enabled game window, records a privacy slate with muted audio when focus leaves enabled games, and preserves one continuous replay buffer.

**Architecture:** Keep one recorder, one encoder, one replay ring, and one full-session writer state. Add foreground-aware game detection in the app layer, send focus changes to the running service through a new `Cmd::SwitchCapture`, and keep source mutability inside a concrete service-layer `CaptureEngine` controlled by a small switch handle.

**Tech Stack:** Rust/Tauri 2, vanilla HTML/CSS/JS, Windows Graphics Capture, D3D11 video processor, Opus audio via `clipline_capture::OpusFrameEncoder`, Rust unit tests, UI contract tests, workspace Cargo tests and clippy.

## Global Constraints

- `games.follow_focused_windows` defaults to `false`; existing automatic game detection behavior stays unchanged until the user enables it.
- Focus-follow mode records only enabled saved game windows. Non-game foreground windows record a neutral slate and muted audio.
- Focus-only target changes do not restart the recorder service and do not clear replay storage.
- The encoder canvas is chosen once at recorder start and MP4 track parameters do not change mid-stream.
- No monitor/display fallback is recorded when focus leaves enabled games.
- No multi-window compositor, pre-opened WGC pool, per-process audio following, or new game-plugin system.
- Windows-only code remains behind the existing Windows app/capture modules; new `unsafe` stays inside `crates/clipline-capture/src/windows/`.
- Keep plan checkboxes unticked in commits.

---

## File Structure

- Modify `apps/clipline-app/src/settings/games.rs`
  - Persist `GameSettings::follow_focused_windows` with serde default `false`.
- Modify `apps/clipline-app/src/settings/tests.rs`
  - Cover defaults, serialization, and loading old settings without the new field.
- Modify `apps/clipline-app/ui/index.html`
  - Add the `Follow focused game windows` checkbox under Game detection.
- Modify `apps/clipline-app/ui/settings.js`
  - Read, fill, default, and status-copy support for `follow_focused_windows`.
- Modify `apps/clipline-app/ui/main.js`
  - Wire the new checkbox into settings dirty-state/status refresh.
- Modify `apps/clipline-app/tests/ui_contract.rs`
  - Guard the new DOM id and JS settings wiring.
- Modify `crates/clipline-capture/src/windows/window.rs`
  - Add `foreground_capturable_window()` and refactor shared HWND-to-`CapturableWindow` metadata.
- Modify `crates/clipline-capture/src/windows/mod.rs`
  - Re-export the foreground helper.
- Modify `apps/clipline-app/src/games.rs`
  - Add focused-window matching that ignores background games.
- Modify `apps/clipline-app/src/app.rs`
  - Branch the detector loop for focus-follow mode, dedupe targets, and send switch commands instead of recorder restarts.
- Modify `apps/clipline-app/src/service.rs`
  - Add switch command types, mutable run attribution, switchable video source, slate source, marker gating, full-session transitions, and status fields.
- Modify `crates/clipline-events/src/markers.rs`
  - Store focus-follow source switch metadata in the existing clip sidecar schema.
- Modify `crates/clipline-capture/src/audio_gate.rs`
  - Add reusable focus/privacy audio gating around `AudioSource`.
- Modify `crates/clipline-capture/src/lib.rs`
  - Export the audio gate types.
- Modify `crates/clipline-capture/src/windows/d3d11.rs`
  - Add CPU-filled BGRA texture creation for privacy slate frames.
- Modify `crates/clipline-capture/src/windows/nv12.rs`
  - Add aspect-preserving fit mode and pure fit-rectangle tests.
- Modify `crates/clipline-capture/src/windows/mft.rs`
  - Let the MFT encoder choose stretch or fit conversion mode.
- Modify `crates/clipline-capture/src/ffmpeg_encoder.rs`
  - Let the FFmpeg encoder choose stretch or fit conversion mode.
- Modify `apps/clipline-app/ui/app-core.js`
  - Track service capture privacy/status state for UI status copy.
- Modify `handoff.md`
  - Add a concise implementation note after the feature is working.

---

### Task 1: Persist And Surface The Focus-Follow Setting

**Files:**
- Modify: `apps/clipline-app/src/settings/games.rs`
- Modify: `apps/clipline-app/src/settings/tests.rs`
- Modify: `apps/clipline-app/ui/index.html`
- Modify: `apps/clipline-app/ui/settings.js`
- Modify: `apps/clipline-app/ui/main.js`
- Modify: `apps/clipline-app/tests/ui_contract.rs`

**Interfaces:**
- Produces: `GameSettings { follow_focused_windows: bool }`
- Produces: DOM control `#set-games-follow-focused`
- Consumes: existing `defaultGameSettings()`, `fillSettings()`, `readSettings()`, `updateGameDetectionStatus()`

- [ ] **Step 1: Write failing settings tests**

In `apps/clipline-app/src/settings/tests.rs`, extend `defaults_match_current_recorder_behavior()`:

```rust
assert!(!settings.games.follow_focused_windows);
assert_eq!(serialized["games"]["follow_focused_windows"], false);
```

Add this test near the existing game settings migration tests:

```rust
#[test]
fn legacy_games_default_focus_follow_off() {
    let json = r#"{
            "games": {
                "auto_detect": true,
                "custom_games": []
            }
        }"#;

    let settings: AppSettings = serde_json::from_str(json).unwrap();

    assert!(settings.games.auto_detect);
    assert!(!settings.games.follow_focused_windows);
    let saved = serde_json::to_value(&settings).unwrap();
    assert_eq!(saved["games"]["follow_focused_windows"], false);
}
```

- [ ] **Step 2: Write failing UI contract checks**

In `apps/clipline-app/tests/ui_contract.rs`, add these assertions to the app-shell/settings contract that already checks `set-games-auto-detect`:

```rust
assert!(
    html.contains("id=\"set-games-follow-focused\"")
        && html.contains("Follow focused game windows"),
    "Settings > Games must expose the focus-follow toggle"
);
assert!(
    main_js().contains("follow_focused_windows: false")
        && main_js().contains("$(\"set-games-follow-focused\").checked = !!games.follow_focused_windows")
        && main_js().contains("follow_focused_windows: $(\"set-games-follow-focused\").checked")
        && main_js().contains("$(\"set-games-follow-focused\").addEventListener(\"change\", updateGameDetectionStatus)"),
    "Settings JS must default, fill, read, and wire focus-follow"
);
```

- [ ] **Step 3: Run tests to verify they fail**

Run:

```powershell
cargo test -p clipline-app settings::tests::legacy_games_default_focus_follow_off
cargo test -p clipline-app --test ui_contract app_shell_contract
```

Expected: FAIL because `follow_focused_windows` and `set-games-follow-focused` do not exist.

- [ ] **Step 4: Implement the settings field**

In `apps/clipline-app/src/settings/games.rs`, update both settings structs and default construction:

```rust
fn default_disabled() -> bool {
    false
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct GameSettings {
    #[serde(default = "default_enabled")]
    pub auto_detect: bool,
    #[serde(default = "default_disabled")]
    pub follow_focused_windows: bool,
    #[serde(default)]
    pub plugins: BTreeMap<String, GamePluginSettings>,
    #[serde(default)]
    pub custom_games: Vec<CustomGameSettings>,
}

#[derive(Deserialize)]
struct GameSettingsWire {
    #[serde(default = "default_enabled")]
    auto_detect: bool,
    #[serde(default = "default_disabled")]
    follow_focused_windows: bool,
    #[serde(default)]
    plugins: BTreeMap<String, GamePluginSettings>,
    #[serde(default, rename = "recording_mode")]
    legacy_recording_mode: Option<GameRecordingMode>,
    #[serde(default)]
    custom_games: Vec<CustomGameSettings>,
}

impl Default for GameSettings {
    fn default() -> Self {
        Self {
            auto_detect: true,
            follow_focused_windows: false,
            plugins: BTreeMap::new(),
            custom_games: Vec::new(),
        }
    }
}
```

In the manual `Deserialize` implementation, copy the wire field:

```rust
Ok(Self {
    auto_detect: wire.auto_detect,
    follow_focused_windows: wire.follow_focused_windows,
    plugins: wire.plugins,
    custom_games,
})
```

- [ ] **Step 5: Implement UI markup and JavaScript wiring**

In `apps/clipline-app/ui/index.html`, add this row directly after the existing `games.auto_detect` row:

```html
<div class="setting-row" data-settings-key="games.follow_focused_windows">
  <div class="setting-info">
    <strong>Focused game follow</strong>
    <span>When two saved games are open, record the one in front and use a privacy slate outside saved games.</span>
  </div>
  <label class="check-line">
    <input id="set-games-follow-focused" type="checkbox" />
    <span>Follow focused game windows</span>
  </label>
</div>
```

In `apps/clipline-app/ui/settings.js`, update `defaultGameSettings()`:

```js
function defaultGameSettings() {
  return {
    auto_detect: true,
    follow_focused_windows: false,
    plugins: {},
    custom_games: [],
  };
}
```

In `fillSettings(s)`, set the checkbox after `set-games-auto-detect`:

```js
$("set-games-auto-detect").checked = !!games.auto_detect;
$("set-games-follow-focused").checked = !!games.follow_focused_windows;
```

In `readSettings()`, include the field:

```js
games: {
  auto_detect: $("set-games-auto-detect").checked,
  follow_focused_windows: $("set-games-follow-focused").checked,
  plugins: readGamePluginSettings(),
  custom_games: customGames.map((game) => Object.assign({}, game)),
},
```

In `updateGameDetectionStatus()`, add focus-follow copy when detection is on and no game is active:

```js
if ($("set-games-follow-focused").checked) {
  $("game-detection-status").textContent = "Recording the focused saved game; private windows use a slate.";
  return;
}
```

Place that block after the `set-games-auto-detect` off branch and before the enabled-plugin list branch.

In `apps/clipline-app/ui/main.js`, add the listener next to the auto-detect listener:

```js
$("set-games-follow-focused").addEventListener("change", updateGameDetectionStatus);
```

- [ ] **Step 6: Run focused tests**

Run:

```powershell
cargo test -p clipline-app settings::tests::legacy_games_default_focus_follow_off
cargo test -p clipline-app --test ui_contract app_shell_contract
```

Expected: PASS.

- [ ] **Step 7: Commit**

```powershell
git add apps/clipline-app/src/settings/games.rs apps/clipline-app/src/settings/tests.rs apps/clipline-app/ui/index.html apps/clipline-app/ui/settings.js apps/clipline-app/ui/main.js apps/clipline-app/tests/ui_contract.rs
git commit -m "feat(games): add focus-follow setting"
```

---

### Task 2: Detect Only The Focused Enabled Game

**Files:**
- Modify: `crates/clipline-capture/src/windows/window.rs`
- Modify: `crates/clipline-capture/src/windows/mod.rs`
- Modify: `apps/clipline-app/src/games.rs`

**Interfaces:**
- Produces: `clipline_capture::windows::foreground_capturable_window() -> Option<CapturableWindow>`
- Produces: `detect_focused_game(settings: &GameSettings) -> Option<DetectedGame>`
- Produces: `detect_focused_game_from_window(settings: &GameSettings, foreground: Option<CapturableWindow>) -> Option<DetectedGame>`

- [ ] **Step 1: Write failing focused detection tests**

In `apps/clipline-app/src/games.rs`, add these tests inside the existing test module:

```rust
#[test]
fn focused_detection_ignores_background_saved_games() {
    let settings = GameSettings {
        auto_detect: true,
        custom_games: vec![game()],
        ..GameSettings::default()
    };

    let detected = detect_focused_game_from_window(
        &settings,
        Some(window(
            100,
            "Notepad",
            "notepad.exe",
            Some(r"C:\Windows\System32\notepad.exe"),
        )),
    );

    assert!(detected.is_none());
}

#[test]
fn focused_detection_matches_custom_foreground_game() {
    let settings = GameSettings {
        auto_detect: true,
        custom_games: vec![game()],
        ..GameSettings::default()
    };

    let detected = detect_focused_game_from_window(
        &settings,
        Some(window(42, "Test Game", "game.exe", Some(r"C:\Games\Test\game.exe"))),
    )
    .expect("focused custom game should match");

    assert_eq!(detected.hwnd, 42);
    assert_eq!(detected.id, "custom-test");
}

#[test]
fn focused_detection_matches_built_in_before_custom_rules() {
    let settings = GameSettings {
        auto_detect: true,
        custom_games: vec![CustomGameSettings {
            id: "custom-league".into(),
            name: "Custom League".into(),
            exe_name: "League of Legends.exe".into(),
            process_path: None,
            window_title: String::new(),
            ..game()
        }],
        ..GameSettings::default()
    };

    let detected = detect_focused_game_from_window(
        &settings,
        Some(window(
            7,
            "League of Legends (TM) Client",
            "League of Legends.exe",
            Some(r"C:\Riot Games\League of Legends\Game\League of Legends.exe"),
        )),
    )
    .expect("focused League game should match");

    assert_eq!(detected.id, crate::game_plugins::LEAGUE_OF_LEGENDS_ID);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run:

```powershell
cargo test -p clipline-app games::tests::focused_detection_ignores_background_saved_games games::tests::focused_detection_matches_custom_foreground_game games::tests::focused_detection_matches_built_in_before_custom_rules
```

Expected: FAIL because `detect_focused_game_from_window` does not exist.

- [ ] **Step 3: Add the foreground HWND helper**

In `crates/clipline-capture/src/windows/window.rs`, add `GetForegroundWindow` to the import list:

```rust
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClientRect, GetForegroundWindow, GetWindowRect, GetWindowTextW,
    GetWindowThreadProcessId, IsWindow, IsWindowVisible,
};
```

Add this public helper below `enumerate_capturable_windows()`:

```rust
pub fn foreground_capturable_window() -> Option<CapturableWindow> {
    let hwnd = unsafe { GetForegroundWindow() };
    capturable_window_from_hwnd(hwnd)
}
```

Refactor `enum_capturable_proc` to call this shared helper:

```rust
unsafe extern "system" fn enum_capturable_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let windows = unsafe { &mut *(lparam.0 as *mut Vec<CapturableWindow>) };
    if let Some(window) = capturable_window_from_hwnd(hwnd) {
        windows.push(window);
    }
    BOOL(1)
}

fn capturable_window_from_hwnd(hwnd: HWND) -> Option<CapturableWindow> {
    unsafe {
        if !IsWindowVisible(hwnd).as_bool() {
            return None;
        }
        let title = window_title(hwnd)?;
        if title.trim().is_empty() {
            return None;
        }
        let mut process_id = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut process_id));
        let exe_path = process_path(process_id);
        let exe_name = exe_path
            .as_deref()
            .and_then(exe_name_from_path)
            .unwrap_or_default();
        Some(CapturableWindow {
            handle: hwnd.0 as isize,
            title,
            process_id,
            exe_name,
            exe_path,
        })
    }
}
```

In `crates/clipline-capture/src/windows/mod.rs`, re-export the helper:

```rust
pub use window::{
    enumerate_capturable_windows, find_window_by_title, foreground_capturable_window,
    window_client_crop, window_from_raw_handle, CapturableWindow,
};
```

- [ ] **Step 4: Add focused game matching**

In `apps/clipline-app/src/games.rs`, update the import:

```rust
use clipline_capture::windows::{
    enumerate_capturable_windows, foreground_capturable_window, CapturableWindow,
};
```

Add these functions below `detect_active_game()`:

```rust
pub fn detect_focused_game(settings: &GameSettings) -> Option<DetectedGame> {
    detect_focused_game_from_window(settings, foreground_capturable_window())
}

pub fn detect_focused_game_from_window(
    settings: &GameSettings,
    foreground: Option<CapturableWindow>,
) -> Option<DetectedGame> {
    if !settings.auto_detect || !has_enabled_games(settings) {
        return None;
    }
    let window = foreground?;
    if window.process_id == std::process::id() {
        return None;
    }
    detect_built_in_game_from_windows(settings, std::slice::from_ref(&window)).or_else(|| {
        settings
            .custom_games
            .iter()
            .filter(|game| game.enabled)
            .find(|game| match_score(game, &window).is_some())
            .map(|game| DetectedGame {
                id: game.id.clone(),
                name: game.name.clone(),
                hwnd: window.handle,
                window_title: window.title.clone(),
                process_id: window.process_id,
                exe_name: window.exe_name.clone(),
                recording_mode: game.recording_mode,
            })
    })
}
```

- [ ] **Step 5: Run focused tests**

Run:

```powershell
cargo test -p clipline-app games::tests::focused_detection_ignores_background_saved_games games::tests::focused_detection_matches_custom_foreground_game games::tests::focused_detection_matches_built_in_before_custom_rules
```

Expected: PASS.

- [ ] **Step 6: Commit**

```powershell
git add crates/clipline-capture/src/windows/window.rs crates/clipline-capture/src/windows/mod.rs apps/clipline-app/src/games.rs
git commit -m "feat(games): detect focused game window"
```

---

### Task 3: Route Focus Changes As Recorder Commands

**Files:**
- Modify: `apps/clipline-app/src/service.rs`
- Modify: `apps/clipline-app/src/app.rs`

**Interfaces:**
- Produces: `Cmd::SwitchCapture(SwitchCaptureTarget)`
- Produces: `SwitchCaptureTarget::{Window, Slate}`
- Produces: `SlateReason::{NoEnabledForegroundGame, WindowUnavailable, SwitchFailed}`
- Produces: `ServiceOptions::focus_follow_enabled`

- [ ] **Step 1: Write failing app command tests**

In `apps/clipline-app/src/app.rs`, add this helper in the test module:

```rust
fn runtime_inner_with_sender(tx: Sender<Cmd>, settings: AppSettings) -> RuntimeInner {
    RuntimeInner {
        tx: Some(tx),
        recording_generation: 1,
        settings,
        lol_url: None,
        active_game: None,
        focus_follow_target: None,
        osu_title_events: Vec::new(),
        last_save_request: None,
        decodable_codecs: vec![service::Codec::H264],
    }
}
```

Add these tests:

```rust
#[test]
fn focus_follow_update_sends_switch_command_without_restart() {
    let (tx, rx) = mpsc::channel();
    let mut settings = AppSettings::default();
    settings.games.follow_focused_windows = true;
    let mut inner = runtime_inner_with_sender(tx, settings);
    let detected = DetectedGame {
        id: "custom-game".into(),
        name: "Game".into(),
        hwnd: 42,
        window_title: "Game Window".into(),
        process_id: 7,
        exe_name: "game.exe".into(),
        recording_mode: GameRecordingMode::ReplaysOnly,
    };

    let emit = RuntimeState::prepare_focus_follow_update(&mut inner, Some(detected)).unwrap();

    assert!(emit);
    assert!(inner.tx.is_some(), "recorder sender stays installed");
    assert!(matches!(
        rx.try_recv(),
        Ok(Cmd::SwitchCapture(service::SwitchCaptureTarget::Window { hwnd: 42, .. }))
    ));
}

#[test]
fn focus_follow_duplicate_target_sends_no_command() {
    let (tx, rx) = mpsc::channel();
    let mut settings = AppSettings::default();
    settings.games.follow_focused_windows = true;
    let mut inner = runtime_inner_with_sender(tx, settings);
    let detected = DetectedGame {
        id: "custom-game".into(),
        name: "Game".into(),
        hwnd: 42,
        window_title: "Game Window".into(),
        process_id: 7,
        exe_name: "game.exe".into(),
        recording_mode: GameRecordingMode::ReplaysOnly,
    };

    RuntimeState::prepare_focus_follow_update(&mut inner, Some(detected.clone())).unwrap();
    let emit = RuntimeState::prepare_focus_follow_update(&mut inner, Some(detected)).unwrap();

    assert!(!emit);
    assert!(rx.try_recv().is_ok(), "first command exists");
    assert!(rx.try_recv().is_err(), "duplicate command suppressed");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run:

```powershell
cargo test -p clipline-app app::tests::focus_follow_update_sends_switch_command_without_restart app::tests::focus_follow_duplicate_target_sends_no_command
```

Expected: FAIL because the command model and helper state do not exist.

- [ ] **Step 3: Add service command data**

In `apps/clipline-app/src/service.rs`, replace the current `Cmd` enum with:

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Cmd {
    Save,
    SwitchCapture(SwitchCaptureTarget),
    Stop { announce: bool },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SwitchCaptureTarget {
    Window {
        hwnd: isize,
        title: String,
        active_game: Option<ActiveGame>,
        active_game_plugin_id: Option<String>,
        recording_mode: RecordingMode,
    },
    Slate {
        reason: SlateReason,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SlateReason {
    NoEnabledForegroundGame,
    WindowUnavailable,
    SwitchFailed,
}
```

Update derives for `RecordingMode` and `ActiveGame`:

```rust
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RecordingMode {
    FullSession,
    #[default]
    ReplaysOnly,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActiveGame {
    pub id: String,
    pub name: String,
}
```

Add the service option:

```rust
pub focus_follow_enabled: bool,
```

Add it directly after `capture_source` in `ServiceOptions`. Default it to `false`.

- [ ] **Step 4: Add app-side target keys and command preparation**

In `apps/clipline-app/src/app.rs`, add a field to `RuntimeInner`:

```rust
focus_follow_target: Option<FocusFollowTargetKey>,
```

Add the key type near `GameDetectionEvent`:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
enum FocusFollowTargetKey {
    Window {
        hwnd: isize,
        recording_mode: GameRecordingMode,
    },
    Slate,
}
```

In `RuntimeState::options`, pass the setting through:

```rust
opts.focus_follow_enabled = inner.settings.games.follow_focused_windows;
```

Add helpers inside `impl RuntimeState`:

```rust
fn focus_follow_key(detected: Option<&DetectedGame>) -> FocusFollowTargetKey {
    match detected {
        Some(game) => FocusFollowTargetKey::Window {
            hwnd: game.hwnd,
            recording_mode: game.recording_mode,
        },
        None => FocusFollowTargetKey::Slate,
    }
}

fn focus_follow_command(detected: Option<&DetectedGame>) -> Cmd {
    match detected {
        Some(game) => Cmd::SwitchCapture(service::SwitchCaptureTarget::Window {
            hwnd: game.hwnd,
            title: game.window_title.clone(),
            active_game: Some(service::ActiveGame {
                id: game.id.clone(),
                name: game.name.clone(),
            }),
            active_game_plugin_id: crate::game_plugins::contains(&game.id)
                .then(|| game.id.clone()),
            recording_mode: game.recording_mode.into(),
        }),
        None => Cmd::SwitchCapture(service::SwitchCaptureTarget::Slate {
            reason: service::SlateReason::NoEnabledForegroundGame,
        }),
    }
}

fn prepare_focus_follow_update(
    inner: &mut RuntimeInner,
    detected: Option<DetectedGame>,
) -> Result<bool, String> {
    record_osu_title_event(inner, detected.as_ref(), unix_now());
    let next_key = Self::focus_follow_key(detected.as_ref());
    if inner.focus_follow_target.as_ref() == Some(&next_key) {
        return Ok(false);
    }
    let command = Self::focus_follow_command(detected.as_ref());
    inner.active_game = detected;
    inner.focus_follow_target = Some(next_key);
    if let Some(tx) = &inner.tx {
        let _ = tx.send(command);
    }
    Ok(true)
}
```

- [ ] **Step 5: Branch `set_detected_game` and detector loop**

At the start of `set_detected_game`, branch when the current settings enable focus-follow:

```rust
{
    let mut inner = self.0.lock().map_err(|_| "runtime state lock poisoned")?;
    if inner.settings.games.follow_focused_windows {
        let emit_event = Self::prepare_focus_follow_update(&mut inner, detected)?;
        drop(inner);
        if emit_event {
            let _ = app.emit("game-detection", event);
        }
        return Ok(());
    }
}
```

In `prepare_settings_restart`, clear the key when settings change:

```rust
inner.focus_follow_target = None;
```

In `spawn_game_detector`, select the detector:

```rust
let detected = if settings.games.follow_focused_windows {
    crate::games::detect_focused_game(&settings.games)
} else {
    crate::games::detect_active_game(&settings.games)
};
```

- [ ] **Step 6: Run focused tests**

Run:

```powershell
cargo test -p clipline-app app::tests::focus_follow_update_sends_switch_command_without_restart app::tests::focus_follow_duplicate_target_sends_no_command app::tests::active_full_session_game_sets_service_recording_mode
```

Expected: PASS.

- [ ] **Step 7: Commit**

```powershell
git add apps/clipline-app/src/service.rs apps/clipline-app/src/app.rs
git commit -m "feat(games): route focused target switches"
```

---

### Task 4: Add Aspect-Fit Video Conversion And Slate Texture Support

**Files:**
- Modify: `crates/clipline-capture/src/windows/nv12.rs`
- Modify: `crates/clipline-capture/src/windows/d3d11.rs`
- Modify: `crates/clipline-capture/src/windows/mft.rs`
- Modify: `crates/clipline-capture/src/ffmpeg_encoder.rs`
- Modify: `apps/clipline-app/src/service.rs`

**Interfaces:**
- Produces: `ResizeMode::{Stretch, Fit}`
- Produces: `VideoConverter::new_with_crop_and_resize(device, in_w, in_h, out_w, out_h, crop, resize_mode)`
- Produces: `aspect_fit_rect(in_w, in_h, out_w, out_h) -> RECT`
- Produces: `d3d11::create_bgra_texture_from_pixels(device, width, height, bgra)`
- Produces: focus-follow encoder construction passes `ResizeMode::Fit`; normal recording keeps `ResizeMode::Stretch`

- [ ] **Step 1: Write failing fit-rectangle tests**

In `crates/clipline-capture/src/windows/nv12.rs`, add tests:

```rust
#[test]
fn aspect_fit_rect_keeps_same_aspect_full_frame() {
    let rect = aspect_fit_rect(1920, 1080, 1280, 720);
    assert_eq!((rect.left, rect.top, rect.right, rect.bottom), (0, 0, 1280, 720));
}

#[test]
fn aspect_fit_rect_pillarboxes_wide_output() {
    let rect = aspect_fit_rect(4, 3, 1920, 1080);
    assert_eq!((rect.left, rect.top, rect.right, rect.bottom), (240, 0, 1680, 1080));
}

#[test]
fn aspect_fit_rect_letterboxes_tall_output() {
    let rect = aspect_fit_rect(16, 9, 1000, 1000);
    assert_eq!((rect.left, rect.top, rect.right, rect.bottom), (0, 218, 1000, 781));
}
```

In `crates/clipline-capture/src/windows/d3d11.rs`, add a WARP-safe test:

```rust
#[test]
fn creates_bgra_texture_from_cpu_pixels() {
    let (device, _ctx) = create_device_for_tests().expect("WARP D3D11 device");
    let pixels = vec![0x33; 4 * 4 * 4];
    let texture = create_bgra_texture_from_pixels(&device, 4, 4, &pixels).expect("texture");
    assert_eq!(texture_size(&texture), (4, 4));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run:

```powershell
cargo test -p clipline-capture windows::nv12::tests::aspect_fit_rect_keeps_same_aspect_full_frame windows::nv12::tests::aspect_fit_rect_pillarboxes_wide_output windows::nv12::tests::aspect_fit_rect_letterboxes_tall_output windows::d3d11::tests::creates_bgra_texture_from_cpu_pixels
```

Expected: FAIL because the helpers do not exist.

- [ ] **Step 3: Add the D3D texture upload helper**

In `crates/clipline-capture/src/windows/d3d11.rs`, import `D3D11_SUBRESOURCE_DATA` and add:

```rust
pub fn create_bgra_texture_from_pixels(
    device: &ID3D11Device,
    width: u32,
    height: u32,
    bgra: &[u8],
) -> WinResult<ID3D11Texture2D> {
    assert_eq!(bgra.len(), width as usize * height as usize * 4);
    let desc = D3D11_TEXTURE2D_DESC {
        Width: width,
        Height: height,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: (D3D11_BIND_SHADER_RESOURCE.0 | D3D11_BIND_RENDER_TARGET.0) as u32,
        CPUAccessFlags: 0,
        MiscFlags: 0,
    };
    let data = D3D11_SUBRESOURCE_DATA {
        pSysMem: bgra.as_ptr().cast(),
        SysMemPitch: width * 4,
        SysMemSlicePitch: 0,
    };
    let mut texture = None;
    unsafe { device.CreateTexture2D(&desc, Some(&data), Some(&mut texture))? };
    Ok(texture.expect("texture out-param set on Ok"))
}
```

- [ ] **Step 4: Add aspect-fit mode**

In `crates/clipline-capture/src/windows/nv12.rs`, add:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResizeMode {
    Stretch,
    Fit,
}
```

Add `resize_mode: ResizeMode` to `VideoConverter`. Make existing constructors call a new constructor with `ResizeMode::Stretch`:

```rust
pub fn new_with_crop_and_resize(
    device: &ID3D11Device,
    in_w: u32,
    in_h: u32,
    out_w: u32,
    out_h: u32,
    crop: Option<CropRect>,
    resize_mode: ResizeMode,
) -> WinResult<Self> {
    let video_device: ID3D11VideoDevice = device.cast()?;
    let video_context: ID3D11VideoContext = unsafe { device.GetImmediateContext()? }.cast()?;
    let (enumerator, processor) =
        create_video_processor(&video_device, in_w, in_h, out_w, out_h)?;
    configure_video_processor_color_spaces(&video_context, &processor);
    Ok(Self {
        device: device.clone(),
        video_context,
        video_device,
        processor,
        enumerator,
        in_width: in_w,
        in_height: in_h,
        out_width: out_w,
        out_height: out_h,
        source_rect: crop.map(CropRect::to_rect),
        resize_mode,
    })
}
```

Add the pure helper:

```rust
fn aspect_fit_rect(in_w: u32, in_h: u32, out_w: u32, out_h: u32) -> RECT {
    let in_w = in_w.max(1) as f64;
    let in_h = in_h.max(1) as f64;
    let out_w_f = out_w.max(1) as f64;
    let out_h_f = out_h.max(1) as f64;
    let scale = (out_w_f / in_w).min(out_h_f / in_h);
    let fit_w = (in_w * scale).round().clamp(1.0, out_w_f) as i32;
    let fit_h = (in_h * scale).round().clamp(1.0, out_h_f) as i32;
    let left = (out_w as i32 - fit_w) / 2;
    let top = (out_h as i32 - fit_h) / 2;
    RECT {
        left,
        top,
        right: left + fit_w,
        bottom: top + fit_h,
    }
}
```

In `convert`, before `VideoProcessorBlt`, set the destination rect for fit mode:

```rust
if self.resize_mode == ResizeMode::Fit {
    let rect = aspect_fit_rect(in_width, in_height, self.out_width, self.out_height);
    unsafe {
        self.video_context.VideoProcessorSetOutputTargetRect(
            &self.processor,
            true,
            Some(&rect),
        );
    }
} else {
    unsafe {
        self.video_context.VideoProcessorSetOutputTargetRect(&self.processor, false, None);
    }
}
```

- [ ] **Step 5: Let encoder constructors select fit mode**

In `crates/clipline-capture/src/windows/mft.rs`, import `ResizeMode` and extend `MftConfig`:

```rust
use crate::windows::nv12::{CropRect, ResizeMode, VideoConverter};

pub struct MftConfig {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub bitrate_bps: u32,
    pub encoder_backend: Option<EncoderBackend>,
    pub resize_mode: ResizeMode,
}
```

Where `MftH264Encoder::new` builds the converter, use:

```rust
VideoConverter::new_with_crop_and_resize(
    device,
    in_w,
    in_h,
    cfg.width,
    cfg.height,
    crop,
    cfg.resize_mode,
)
```

Update existing `MftConfig` literals in tests and probes with:

```rust
resize_mode: ResizeMode::Stretch,
```

In `crates/clipline-capture/src/ffmpeg_encoder.rs`, import `ResizeMode` and add a constructor that takes a resize mode:

```rust
pub fn new_on_with_resize(
    device: &ID3D11Device,
    ffmpeg: &Path,
    backend: EncoderBackend,
    codec: Codec,
    in_w: u32,
    in_h: u32,
    crop: Option<CropRect>,
    out_w: u32,
    out_h: u32,
    fps: u32,
    bitrate_bps: u32,
    resize_mode: ResizeMode,
) -> Result<Self, EncodeError> {
    let mut encoder = Self::new_on(
        device,
        ffmpeg,
        backend,
        codec,
        in_w,
        in_h,
        crop,
        out_w,
        out_h,
        fps,
        bitrate_bps,
    )?;
    encoder.converter = Some(
        VideoConverter::new_with_crop_and_resize(
            device,
            in_w,
            in_h,
            out_w,
            out_h,
            crop,
            resize_mode,
        )
        .map_err(|e| EncodeError::Backend(format!("nv12 converter: {e}")))?,
    );
    Ok(encoder)
}
```

Keep `new_on` as the normal stretch path by making it call `new_on_with_resize` with its existing parameters plus `ResizeMode::Stretch`, or by leaving its existing converter construction on `ResizeMode::Stretch`.

In `apps/clipline-app/src/service.rs`, import `ResizeMode`:

```rust
use clipline_capture::windows::nv12::{CropRect, ResizeMode};
```

In `open_candidate`, select the conversion mode once:

```rust
let resize_mode = if opts.focus_follow_enabled {
    ResizeMode::Fit
} else {
    ResizeMode::Stretch
};
```

Set it on `MftConfig`:

```rust
let cfg = MftConfig {
    width: enc_w,
    height: enc_h,
    fps: opts.fps,
    bitrate_bps: opts.bitrate_bps,
    encoder_backend: Some(candidate.backend),
    resize_mode,
};
```

Use it for FFmpeg:

```rust
FfmpegVideoEncoder::new_on_with_resize(
    device,
    ffmpeg,
    candidate.backend,
    candidate.codec,
    in_w,
    in_h,
    None,
    enc_w,
    enc_h,
    opts.fps,
    opts.bitrate_bps,
    resize_mode,
)
```

- [ ] **Step 6: Run focused tests**

Run:

```powershell
cargo test -p clipline-capture windows::nv12::tests::aspect_fit_rect_keeps_same_aspect_full_frame windows::nv12::tests::aspect_fit_rect_pillarboxes_wide_output windows::nv12::tests::aspect_fit_rect_letterboxes_tall_output windows::d3d11::tests::creates_bgra_texture_from_cpu_pixels
```

Expected: PASS.

- [ ] **Step 7: Commit**

```powershell
git add crates/clipline-capture/src/windows/nv12.rs crates/clipline-capture/src/windows/d3d11.rs crates/clipline-capture/src/windows/mft.rs crates/clipline-capture/src/ffmpeg_encoder.rs apps/clipline-app/src/service.rs
git commit -m "feat(capture): add focus-follow video primitives"
```

---

### Task 5: Add Privacy Audio Gate

**Files:**
- Create: `crates/clipline-capture/src/audio_gate.rs`
- Modify: `crates/clipline-capture/src/lib.rs`

**Interfaces:**
- Produces: `AudioPrivacyState`
- Produces: `PrivacyAudioGate::new(inner: Box<dyn AudioSource>, state: AudioPrivacyState) -> Result<Self, CaptureError>`

- [ ] **Step 1: Write failing audio gate tests**

Create `crates/clipline-capture/src/audio_gate.rs` with tests first:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::MockAudioSource;

    #[test]
    fn game_mode_passes_inner_packets_through() {
        let state = AudioPrivacyState::new_game();
        let mut gate = PrivacyAudioGate::new(Box::new(MockAudioSource::new(48_000, 20)), state)
            .expect("gate");

        let packets = gate.poll_packets(0.04).unwrap();

        assert_eq!(packets.len(), 2);
        assert!(packets[0].data.starts_with(b"P00000"));
        assert!(packets[1].data.starts_with(b"P00001"));
    }

    #[test]
    fn slate_mode_drains_inner_and_emits_silence() {
        let state = AudioPrivacyState::new_game();
        let mut gate = PrivacyAudioGate::new(Box::new(MockAudioSource::new(48_000, 20)), state.clone())
            .expect("gate");
        state.set_slate(true);

        let packets = gate.poll_packets(0.06).unwrap();

        assert_eq!(packets.len(), 3);
        assert_eq!(packets[0].pts_s, 0.0);
        assert_eq!(packets[1].pts_s, 0.02);
        assert_eq!(packets[2].pts_s, 0.04);
        assert!(packets.iter().all(|packet| !packet.data.starts_with(b"P")));

        state.set_slate(false);
        let resumed = gate.poll_packets(0.08).unwrap();
        assert_eq!(resumed.len(), 1);
        assert!(resumed[0].pts_s >= 0.06);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run:

```powershell
cargo test -p clipline-capture audio_gate::tests::game_mode_passes_inner_packets_through audio_gate::tests::slate_mode_drains_inner_and_emits_silence
```

Expected: FAIL because the module is not exported and production types do not exist.

- [ ] **Step 3: Implement the audio gate**

In `crates/clipline-capture/src/lib.rs`, add:

```rust
pub mod audio_gate;
pub use audio_gate::{AudioPrivacyState, PrivacyAudioGate};
```

In `crates/clipline-capture/src/audio_gate.rs`, add:

```rust
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use clipline_mp4::AudioTrackConfig;

use crate::opus::{OpusFrameEncoder, FRAME_DURATION_S, FRAME_LEN};
use crate::traits::{AudioPacket, AudioSource, CaptureError};

#[derive(Clone, Debug)]
pub struct AudioPrivacyState {
    slate: Arc<AtomicBool>,
}

impl AudioPrivacyState {
    pub fn new_game() -> Self {
        Self { slate: Arc::new(AtomicBool::new(false)) }
    }

    pub fn set_slate(&self, slate: bool) {
        self.slate.store(slate, Ordering::Release);
    }

    pub fn is_slate(&self) -> bool {
        self.slate.load(Ordering::Acquire)
    }
}

pub struct PrivacyAudioGate {
    inner: Box<dyn AudioSource>,
    state: AudioPrivacyState,
    opus: OpusFrameEncoder,
    silence_frame: Vec<f32>,
    next_silence_pts_s: f64,
}

impl PrivacyAudioGate {
    pub fn new(
        inner: Box<dyn AudioSource>,
        state: AudioPrivacyState,
    ) -> Result<Self, CaptureError> {
        Ok(Self {
            inner,
            state,
            opus: OpusFrameEncoder::new()
                .map_err(|e| CaptureError::Init(format!("opus silence: {e}")))?,
            silence_frame: vec![0.0; FRAME_LEN],
            next_silence_pts_s: 0.0,
        })
    }
}

impl AudioSource for PrivacyAudioGate {
    fn poll_packets(&mut self, until_pts_s: f64) -> Result<Vec<AudioPacket>, CaptureError> {
        let inner_packets = self.inner.poll_packets(until_pts_s)?;
        if !self.state.is_slate() {
            if let Some(last) = inner_packets.last() {
                self.next_silence_pts_s = last.pts_s + last.duration_s;
            }
            return Ok(inner_packets);
        }

        if let Some(first) = inner_packets.first() {
            self.next_silence_pts_s = self.next_silence_pts_s.max(first.pts_s);
        }
        let mut out = Vec::new();
        while self.next_silence_pts_s + FRAME_DURATION_S <= until_pts_s + 1e-9 {
            let data = self
                .opus
                .encode_frame(&self.silence_frame)
                .map_err(|e| CaptureError::DeviceLost(format!("opus silence encode: {e}")))?;
            out.push(AudioPacket {
                data,
                pts_s: self.next_silence_pts_s,
                duration_s: FRAME_DURATION_S,
            });
            self.next_silence_pts_s += FRAME_DURATION_S;
        }
        Ok(out)
    }

    fn track_config(&self) -> AudioTrackConfig {
        self.inner.track_config()
    }
}
```

- [ ] **Step 4: Run focused tests**

Run:

```powershell
cargo test -p clipline-capture audio_gate::tests::game_mode_passes_inner_packets_through audio_gate::tests::slate_mode_drains_inner_and_emits_silence
```

Expected: PASS.

- [ ] **Step 5: Commit**

```powershell
git add crates/clipline-capture/src/audio_gate.rs crates/clipline-capture/src/lib.rs
git commit -m "feat(capture): add privacy audio gate"
```

---

### Task 6: Add Switchable Live Capture And Privacy Slate

**Files:**
- Modify: `apps/clipline-app/src/service.rs`

**Interfaces:**
- Produces: `SwitchableLiveCapture`
- Produces: `SwitchableCaptureController::switch_to(target: SwitchCaptureTarget) -> Result<SwitchCaptureResult, String>`
- Produces: `SlateCapture`
- Produces: `initial_canvas_dimensions(opts, device, events) -> Result<(u32, u32), String>`

- [ ] **Step 1: Write failing service tests for pure source state**

In `apps/clipline-app/src/service.rs`, add tests:

```rust
#[test]
fn initial_canvas_uses_display_region_without_opening_capture() {
    let region = CaptureRegion {
        display_id: None,
        x: 0,
        y: 0,
        width: 1280,
        height: 720,
    };

    assert_eq!(
        canvas_dimensions_from_capture_source(&CaptureSource::DisplayRegion(region)).unwrap(),
        (1280, 720)
    );
}

#[test]
fn switch_target_identity_dedupes_repeated_slate_and_window() {
    let slate = SwitchCaptureTarget::Slate {
        reason: SlateReason::NoEnabledForegroundGame,
    };
    let window = SwitchCaptureTarget::Window {
        hwnd: 42,
        title: "Game".into(),
        active_game: Some(ActiveGame { id: "g".into(), name: "Game".into() }),
        active_game_plugin_id: None,
        recording_mode: RecordingMode::ReplaysOnly,
    };

    assert_eq!(switch_target_identity(&slate), switch_target_identity(&slate));
    assert_eq!(switch_target_identity(&window), switch_target_identity(&window));
    assert_ne!(switch_target_identity(&slate), switch_target_identity(&window));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run:

```powershell
cargo test -p clipline-app service::tests::initial_canvas_uses_display_region_without_opening_capture service::tests::switch_target_identity_dedupes_repeated_slate_and_window
```

Expected: FAIL because the helpers do not exist.

- [ ] **Step 3: Add slate and switchable source types**

In `apps/clipline-app/src/service.rs`, replace:

```rust
type LiveCapture = CadencedCapture<LiveBackend>;
type LiveRecorder = Recorder<LiveCapture, Box<dyn Encoder>>;
```

with:

```rust
type LiveCapture = CadencedCapture<SwitchableLiveCapture>;
type LiveRecorder = Recorder<LiveCapture, Box<dyn Encoder>>;
```

Add source identity and controller state:

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
enum SwitchTargetIdentity {
    Window(isize),
    Slate,
}

fn switch_target_identity(target: &SwitchCaptureTarget) -> SwitchTargetIdentity {
    match target {
        SwitchCaptureTarget::Window { hwnd, .. } => SwitchTargetIdentity::Window(*hwnd),
        SwitchCaptureTarget::Slate { .. } => SwitchTargetIdentity::Slate,
    }
}

enum LiveBackend {
    Wgc(WgcCapture),
    Dxgi(DxgiDuplicationCapture),
    Slate(SlateCapture),
}

struct SlateCapture {
    frame: FrameData,
    next_pts_s: f64,
    frame_interval_s: f64,
}

impl SlateCapture {
    fn new(device: &ID3D11Device, width: u32, height: u32, fps: u32) -> Result<Self, String> {
        let pixels = privacy_slate_bgra(width, height);
        let texture = d3d11::create_bgra_texture_from_pixels(device, width, height, &pixels)
            .map_err(|e| format!("slate texture: {e}"))?;
        Ok(Self {
            frame: FrameData::Gpu(texture),
            next_pts_s: 0.0,
            frame_interval_s: 1.0 / fps.max(1) as f64,
        })
    }
}
```

Add a deterministic slate bitmap generator. This uses simple local drawing, not a webview or external asset:

```rust
fn privacy_slate_bgra(width: u32, height: u32) -> Vec<u8> {
    let mut pixels = vec![0x12; width as usize * height as usize * 4];
    for px in pixels.chunks_exact_mut(4) {
        px[0] = 0x1b;
        px[1] = 0x18;
        px[2] = 0x14;
        px[3] = 0xff;
    }
    let banner_w = (width * 3 / 5).max(12).min(width);
    let banner_h = (height / 7).max(8).min(height);
    let left = (width - banner_w) / 2;
    let top = (height - banner_h) / 2;
    fill_bgra_rect(&mut pixels, width, left, top, banner_w, banner_h, [0x28, 0x2f, 0x39, 0xff]);
    fill_bgra_rect(
        &mut pixels,
        width,
        left + banner_w / 12,
        top + banner_h / 3,
        banner_w * 5 / 6,
        (banner_h / 6).max(2),
        [0x80, 0x88, 0x94, 0xff],
    );
    pixels
}

fn fill_bgra_rect(
    pixels: &mut [u8],
    stride_width: u32,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    bgra: [u8; 4],
) {
    for row in y..y.saturating_add(h) {
        for col in x..x.saturating_add(w) {
            let idx = ((row * stride_width + col) * 4) as usize;
            if idx + 4 <= pixels.len() {
                pixels[idx..idx + 4].copy_from_slice(&bgra);
            }
        }
    }
}
```

- [ ] **Step 4: Add controller-based switching**

Add:

```rust
struct SwitchableLiveCapture {
    state: std::sync::Arc<std::sync::Mutex<SwitchableLiveCaptureState>>,
}

struct SwitchableCaptureController {
    state: std::sync::Arc<std::sync::Mutex<SwitchableLiveCaptureState>>,
}

struct SwitchableLiveCaptureState {
    device: ID3D11Device,
    clock: RelativeClock,
    fps: u32,
    canvas: (u32, u32),
    active: LiveBackend,
    identity: SwitchTargetIdentity,
}

impl SwitchableLiveCapture {
    fn new(
        device: ID3D11Device,
        clock: RelativeClock,
        fps: u32,
        canvas: (u32, u32),
        active: LiveBackend,
        identity: SwitchTargetIdentity,
    ) -> (Self, SwitchableCaptureController) {
        let state = std::sync::Arc::new(std::sync::Mutex::new(SwitchableLiveCaptureState {
            device,
            clock,
            fps,
            canvas,
            active,
            identity,
        }));
        (
            Self { state: state.clone() },
            SwitchableCaptureController { state },
        )
    }
}

impl TimedFrameSource for SwitchableLiveCapture {
    fn next_frame_timeout(&mut self, timeout: Duration) -> Result<Option<Frame>, CaptureError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| CaptureError::DeviceLost("switchable capture lock poisoned".into()))?;
        state.active.next_frame_timeout(timeout)
    }
}
```

Move the existing `TimedFrameSource for LiveBackend` implementation after the `Slate` variant and add:

```rust
LiveBackend::Slate(cap) => cap.next_frame_timeout(timeout),
```

Implement `SlateCapture` as `TimedFrameSource`:

```rust
impl TimedFrameSource for SlateCapture {
    fn next_frame_timeout(&mut self, _timeout: Duration) -> Result<Option<Frame>, CaptureError> {
        let pts_s = self.next_pts_s;
        self.next_pts_s += self.frame_interval_s;
        Ok(Some(Frame {
            pts_s,
            data: self.frame.clone(),
        }))
    }
}
```

Add `SwitchableCaptureController::switch_to`:

```rust
impl SwitchableCaptureController {
    fn switch_to(&self, target: SwitchCaptureTarget) -> Result<(), String> {
        let next_identity = switch_target_identity(&target);
        let mut state = self
            .state
            .lock()
            .map_err(|_| "switchable capture lock poisoned".to_string())?;
        if state.identity == next_identity {
            return Ok(());
        }
        let slate = LiveBackend::Slate(SlateCapture::new(
            &state.device,
            state.canvas.0,
            state.canvas.1,
            state.fps,
        )?);
        let old = std::mem::replace(&mut state.active, slate);
        state.identity = SwitchTargetIdentity::Slate;
        drop(old);
        let next = match target {
            SwitchCaptureTarget::Window { hwnd, title, .. } => {
                let hwnd = window_from_raw_handle(hwnd)
                    .ok_or_else(|| format!("game window {title:?} is no longer available"))?;
                let cap = WgcCapture::for_window_client_on(
                    state.device.clone(),
                    hwnd,
                    state.clock,
                )
                .map_err(|e| e.to_string())?;
                LiveBackend::Wgc(cap)
            }
            SwitchCaptureTarget::Slate { .. } => return Ok(()),
        };
        state.active = next;
        state.identity = next_identity;
        Ok(())
    }
}
```

This intentionally drops the old WGC source before opening the new one. A failed game-to-game WGC switch leaves the already-installed slate active; the service warning path records that as `SlateReason::SwitchFailed`.

- [ ] **Step 5: Add startup canvas helpers**

Add:

```rust
fn canvas_dimensions_from_capture_source(source: &CaptureSource) -> Result<(u32, u32), String> {
    match source {
        CaptureSource::DisplayRegion(region) => Ok((region.width, region.height)),
        CaptureSource::PrimaryMonitor => {
            let (display, _) =
                clipline_capture::windows::display::display_handle_by_id_or_primary(None)
                    .map_err(|e| e.to_string())?;
            Ok((display.info.width, display.info.height))
        }
        CaptureSource::WindowTitle(_) | CaptureSource::WindowHandle { .. } => {
            Ok((1920, 1080))
        }
    }
}
```

When `opts.focus_follow_enabled && opts.active_game.is_none()`, use this helper to derive input dimensions, apply `output_dimensions_with_bounds`, create a slate `Frame`, and avoid `open_screen_capture`.

- [ ] **Step 6: Run focused service tests**

Run:

```powershell
cargo test -p clipline-app service::tests::initial_canvas_uses_display_region_without_opening_capture service::tests::switch_target_identity_dedupes_repeated_slate_and_window
```

Expected: PASS.

- [ ] **Step 7: Commit**

```powershell
git add apps/clipline-app/src/service.rs
git commit -m "feat(capture): add switchable live source"
```

---

### Task 7: Wire Service Switching, Audio Muting, Markers, And Full Sessions

**Files:**
- Modify: `crates/clipline-events/src/markers.rs`
- Modify: `apps/clipline-app/src/service.rs`
- Modify: `apps/clipline-app/ui/app-core.js`
- Modify: `apps/clipline-app/ui/main.js`
- Modify: `apps/clipline-app/ui/settings.js`
- Modify: `apps/clipline-app/tests/ui_contract.rs`

**Interfaces:**
- Produces: `FocusRunState`
- Produces: `ClipSourceSwitch` sidecar entries in `markers.json`
- Produces: `FullSessionTransition::{None, Start, Finish, FinishThenStart}`
- Produces: status fields `capture_kind`, `capture_label`, `slate_reason`

- [ ] **Step 1: Write failing pure service-state tests**

In `apps/clipline-app/src/service.rs`, add tests:

```rust
#[test]
fn focus_run_state_uses_latest_active_game_for_save_meta() {
    let mut state = FocusRunState::from_options(&ServiceOptions::default());
    state.apply_target(&SwitchCaptureTarget::Window {
        hwnd: 9,
        title: "Game B".into(),
        active_game: Some(ActiveGame { id: "game-b".into(), name: "Game B".into() }),
        active_game_plugin_id: None,
        recording_mode: RecordingMode::ReplaysOnly,
    });

    assert_eq!(
        state.active_game.as_ref().map(|game| game.id.as_str()),
        Some("game-b")
    );
    assert_eq!(state.recording_mode, RecordingMode::ReplaysOnly);
}

#[test]
fn focus_run_state_gates_plugin_markers_to_current_plugin() {
    let mut state = FocusRunState::from_options(&ServiceOptions {
        active_game_plugin_id: Some(crate::game_plugins::LEAGUE_OF_LEGENDS_ID.into()),
        ..ServiceOptions::default()
    });
    assert!(state.accepts_plugin_markers(crate::game_plugins::LEAGUE_OF_LEGENDS_ID));

    state.apply_target(&SwitchCaptureTarget::Slate {
        reason: SlateReason::NoEnabledForegroundGame,
    });
    assert!(!state.accepts_plugin_markers(crate::game_plugins::LEAGUE_OF_LEGENDS_ID));
}

#[test]
fn full_session_transition_splits_between_different_full_session_games() {
    let old = FocusRunState {
        capture_kind: CaptureKind::Game,
        active_game: Some(ActiveGame { id: "a".into(), name: "A".into() }),
        active_game_plugin_id: None,
        recording_mode: RecordingMode::FullSession,
        slate_reason: None,
    };
    let next = FocusRunState {
        capture_kind: CaptureKind::Game,
        active_game: Some(ActiveGame { id: "b".into(), name: "B".into() }),
        active_game_plugin_id: None,
        recording_mode: RecordingMode::FullSession,
        slate_reason: None,
    };

    assert_eq!(
        full_session_transition(Some("a"), &old, &next),
        FullSessionTransition::FinishThenStart
    );
}

#[test]
fn capture_switch_log_filters_to_saved_window() {
    let mut log = CaptureSwitchLog::default();
    log.push(
        0.5,
        &FocusRunState {
            capture_kind: CaptureKind::Game,
            active_game: Some(ActiveGame { id: "a".into(), name: "A".into() }),
            active_game_plugin_id: None,
            recording_mode: RecordingMode::ReplaysOnly,
            slate_reason: None,
        },
    );
    log.push(
        1.5,
        &FocusRunState {
            capture_kind: CaptureKind::Slate,
            active_game: None,
            active_game_plugin_id: None,
            recording_mode: RecordingMode::ReplaysOnly,
            slate_reason: Some(SlateReason::NoEnabledForegroundGame),
        },
    );

    let switches = log.clip_switches(1.0, 2.0);

    assert_eq!(switches.len(), 1);
    assert_eq!(switches[0].t_s, 0.5);
    assert_eq!(switches[0].kind, "slate");
    assert_eq!(
        switches[0].slate_reason.as_deref(),
        Some("no_enabled_foreground_game")
    );
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run:

```powershell
cargo test -p clipline-app service::tests::focus_run_state_uses_latest_active_game_for_save_meta service::tests::focus_run_state_gates_plugin_markers_to_current_plugin service::tests::full_session_transition_splits_between_different_full_session_games
```

Expected: FAIL because the state helpers and switch log do not exist.

- [ ] **Step 3: Extend the markers sidecar schema**

In `crates/clipline-events/src/markers.rs`, add:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ClipSourceSwitch {
    pub t_s: f64,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slate_reason: Option<String>,
}
```

Add it to `ClipMarkers`:

```rust
#[serde(default, skip_serializing_if = "Vec::is_empty")]
pub source_switches: Vec<ClipSourceSwitch>,
```

In `MarkerLog::clip_markers`, initialize the field:

```rust
source_switches: Vec::new(),
```

- [ ] **Step 4: Implement run state, switch log, and transition helpers**

In `apps/clipline-app/src/service.rs`, add:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
enum CaptureKind {
    Game,
    Slate,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct FocusRunState {
    capture_kind: CaptureKind,
    active_game: Option<ActiveGame>,
    active_game_plugin_id: Option<String>,
    recording_mode: RecordingMode,
    slate_reason: Option<SlateReason>,
}

impl FocusRunState {
    fn from_options(opts: &ServiceOptions) -> Self {
        Self {
            capture_kind: if opts.active_game.is_some() {
                CaptureKind::Game
            } else {
                CaptureKind::Slate
            },
            active_game: opts.active_game.clone(),
            active_game_plugin_id: opts.active_game_plugin_id.clone(),
            recording_mode: opts.recording_mode,
            slate_reason: opts.active_game.is_none().then_some(SlateReason::NoEnabledForegroundGame),
        }
    }

    fn apply_target(&mut self, target: &SwitchCaptureTarget) {
        match target {
            SwitchCaptureTarget::Window {
                active_game,
                active_game_plugin_id,
                recording_mode,
                ..
            } => {
                self.capture_kind = CaptureKind::Game;
                self.active_game = active_game.clone();
                self.active_game_plugin_id = active_game_plugin_id.clone();
                self.recording_mode = *recording_mode;
                self.slate_reason = None;
            }
            SwitchCaptureTarget::Slate { reason } => {
                self.capture_kind = CaptureKind::Slate;
                self.active_game = None;
                self.active_game_plugin_id = None;
                self.recording_mode = RecordingMode::ReplaysOnly;
                self.slate_reason = Some(*reason);
            }
        }
    }

    fn accepts_plugin_markers(&self, plugin_id: &str) -> bool {
        self.capture_kind == CaptureKind::Game
            && self.active_game_plugin_id.as_deref() == Some(plugin_id)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FullSessionTransition {
    None,
    Start,
    Finish,
    FinishThenStart,
}
```

Add switch log support:

```rust
#[derive(Default)]
struct CaptureSwitchLog {
    entries: Vec<CaptureSwitchEntry>,
}

struct CaptureSwitchEntry {
    pts_s: f64,
    state: FocusRunState,
}

impl CaptureSwitchLog {
    fn push(&mut self, pts_s: f64, state: &FocusRunState) {
        self.entries.push(CaptureSwitchEntry {
            pts_s,
            state: state.clone(),
        });
    }

    fn clip_switches(&self, start_s: f64, end_s: f64) -> Vec<clipline_events::ClipSourceSwitch> {
        self.entries
            .iter()
            .filter(|entry| entry.pts_s >= start_s && entry.pts_s < end_s)
            .map(|entry| clipline_events::ClipSourceSwitch {
                t_s: entry.pts_s - start_s,
                kind: match entry.state.capture_kind {
                    CaptureKind::Game => "game".into(),
                    CaptureKind::Slate => "slate".into(),
                },
                game_id: entry.state.active_game.as_ref().map(|game| game.id.clone()),
                game_name: entry.state.active_game.as_ref().map(|game| game.name.clone()),
                slate_reason: entry.state.slate_reason.map(|reason| match reason {
                    SlateReason::NoEnabledForegroundGame => "no_enabled_foreground_game".into(),
                    SlateReason::WindowUnavailable => "window_unavailable".into(),
                    SlateReason::SwitchFailed => "switch_failed".into(),
                }),
            })
            .collect()
    }
}
```

Add:

```rust
fn full_session_transition(
    active_recording_game_id: Option<&str>,
    old_state: &FocusRunState,
    next_state: &FocusRunState,
) -> FullSessionTransition {
    let next_full = next_state.capture_kind == CaptureKind::Game
        && next_state.recording_mode == RecordingMode::FullSession;
    let next_id = next_state.active_game.as_ref().map(|game| game.id.as_str());
    match (active_recording_game_id, next_full, next_id) {
        (None, true, Some(_)) => FullSessionTransition::Start,
        (Some(_), false, _) if next_state.capture_kind == CaptureKind::Game => FullSessionTransition::Finish,
        (Some(current), true, Some(next)) if current != next => FullSessionTransition::FinishThenStart,
        _ => {
            let _ = old_state;
            FullSessionTransition::None
        }
    }
}
```

- [ ] **Step 5: Wrap audio sources with privacy gates**

In `run`, create the state:

```rust
let audio_privacy = clipline_capture::AudioPrivacyState::new_game();
let mut focus_state = FocusRunState::from_options(&opts);
audio_privacy.set_slate(focus_state.capture_kind == CaptureKind::Slate);
```

When adding audio tracks:

```rust
for (audio, _) in audio_tracks {
    let gated = clipline_capture::PrivacyAudioGate::new(audio, audio_privacy.clone())
        .map_err(|e| format!("audio privacy: {e}"))?;
    rec = rec.with_audio(Box::new(gated));
}
```

- [ ] **Step 6: Track frame position and handle `Cmd::SwitchCapture`**

Before the main loop, initialize:

```rust
let mut switch_log = CaptureSwitchLog::default();
let mut last_frame_pts_s = 0.0;
switch_log.push(0.0, &focus_state);
```

Change the recorder step call:

```rust
match rec.step_with_frame(|frame| {
    last_frame_pts_s = frame.pts_s;
}) {
```

In the command loop, add a branch before `Cmd::Stop`:

```rust
Ok(Cmd::SwitchCapture(target)) => {
    let old_state = focus_state.clone();
    match capture_controller.switch_to(target.clone()) {
        Ok(()) => {
            focus_state.apply_target(&target);
            audio_privacy.set_slate(focus_state.capture_kind == CaptureKind::Slate);
            switch_log.push(last_frame_pts_s, &focus_state);
            reconcile_full_session_transition(
                &mut rec,
                &clips_dir,
                session.current(),
                &mut full_session,
                &old_state,
                &focus_state,
                &marker_log,
                player_summary.full_session_summary(),
                &audio_track_metadata,
                events,
                &opts,
            );
            send_recording_status(events, &rec, &full_session, &encoder_status, &focus_state);
        }
        Err(e) => {
            warn_user(events, format!("switch capture target: {e}; using privacy slate"));
            let fallback = SwitchCaptureTarget::Slate { reason: SlateReason::SwitchFailed };
            let _ = capture_controller.switch_to(fallback.clone());
            focus_state.apply_target(&fallback);
            audio_privacy.set_slate(true);
            switch_log.push(last_frame_pts_s, &focus_state);
        }
    }
}
```

Update `send_recording_status` signature and `Event::Status` fields:

```rust
Status {
    recording: bool,
    segments: usize,
    buffered_s: f64,
    buffered_mb: f64,
    full_session: bool,
    encoder: String,
    capture_kind: CaptureKind,
    capture_label: Option<String>,
    slate_reason: Option<SlateReason>,
}
```

In `send_recording_status`, set:

```rust
capture_kind: focus_state.capture_kind,
capture_label: focus_state.active_game.as_ref().map(|game| game.name.clone()),
slate_reason: focus_state.slate_reason,
```

- [ ] **Step 7: Gate markers, write switch metadata, and use mutable attribution**

In the marker loop, wrap event updates:

```rust
let marker_allowed = focus_state
    .active_game_plugin_id
    .as_deref()
    .is_some_and(|plugin_id| focus_state.accepts_plugin_markers(plugin_id));
```

Only push review markers and player summaries when `marker_allowed` is true:

```rust
PollerMsg::Event(event) => {
    if marker_allowed {
        if event.kind == EventKind::GameEnd {
            player_summary.match_ended();
            session.match_ended();
        }
        if is_review_event(&event) {
            marker_log.push(event);
        }
    }
}
PollerMsg::PlayerSummary(summary) if marker_allowed => player_summary.update(summary),
PollerMsg::MatchStarted if marker_allowed => {
    player_summary.match_started();
    session.match_started(local_session_label(true));
}
PollerMsg::MatchEnded if marker_allowed => {
    player_summary.match_ended();
    session.match_ended();
}
_ => {}
```

In `Cmd::Save`, replace:

```rust
write_session_game_meta(&session_dir, opts.active_game.as_ref());
```

with:

```rust
write_session_game_meta(&session_dir, focus_state.active_game.as_ref());
```

In `begin_full_session_recording`, call sites now pass `focus_state.recording_mode` and `focus_state.active_game.as_ref()`.

Update `write_marker_sidecar` to accept the switch log:

```rust
fn write_marker_sidecar(
    events: &Sender<Event>,
    marker_log: &MarkerLog,
    switch_log: &CaptureSwitchLog,
    path: &Path,
    start_s: f64,
    end_s: f64,
    player_summary: Option<&PlayerSummary>,
    audio_tracks: &[ClipAudioTrack],
) -> usize {
    let mut clip = marker_log.clip_markers(start_s, end_s);
    clip.markers.retain(|m| is_review_event(&m.event));
    clip.player_summary = player_summary.cloned();
    clip.audio_tracks = audio_tracks.to_vec();
    clip.source_switches = switch_log.clip_switches(start_s, end_s);
    let markers = clip.markers.len();
    if markers == 0
        && clip.player_summary.is_none()
        && clip.audio_tracks.is_empty()
        && clip.plays.is_empty()
        && clip.source_switches.is_empty()
    {
        return 0;
    }
    match serde_json::to_string_pretty(&clip) {
        Ok(json) => {
            if let Err(e) = std::fs::write(path.with_extension("markers.json"), json) {
                warn_user(events, format!("write marker sidecar for {path:?}: {e}"));
            }
        }
        Err(e) => warn_user(
            events,
            format!("serialize marker sidecar for {path:?}: {e}"),
        ),
    }
    markers
}
```

Update every `write_marker_sidecar` call to pass `switch_log`.

- [ ] **Step 8: Reconcile full-session transitions**

Add helper:

```rust
fn active_full_session_game_id(recording: &Option<FullSessionRecording>) -> Option<&str> {
    recording.as_ref().and_then(|recording| recording.game_id.as_deref())
}
```

Extend `FullSessionRecording`:

```rust
struct FullSessionRecording {
    final_path: PathBuf,
    temp_path: PathBuf,
    wall_start_unix: i64,
    min_duration_s: f64,
    game_id: Option<String>,
}
```

Set `game_id` in `begin_full_session_recording`:

```rust
game_id: active_game.map(|game| game.id.clone()),
```

Add `reconcile_full_session_transition`:

```rust
#[allow(clippy::too_many_arguments)]
fn reconcile_full_session_transition(
    rec: &mut LiveRecorder,
    clips_dir: &Path,
    session_label: &str,
    full_session: &mut Option<FullSessionRecording>,
    old_state: &FocusRunState,
    next_state: &FocusRunState,
    marker_log: &MarkerLog,
    player_summary: Option<&PlayerSummary>,
    audio_tracks: &[ClipAudioTrack],
    events: &Sender<Event>,
    opts: &ServiceOptions,
) {
    let transition =
        full_session_transition(active_full_session_game_id(full_session), old_state, next_state);
    let ctx = RecorderFinishContext {
        marker_log,
        player_summary,
        audio_tracks,
        clips_dir,
        opts,
        events,
    };
    match transition {
        FullSessionTransition::None => {}
        FullSessionTransition::Finish => {
            finish_full_session_recording(rec, full_session, &ctx);
        }
        FullSessionTransition::Start => {
            *full_session = begin_full_session_recording(
                rec,
                clips_dir,
                session_label,
                next_state.recording_mode,
                next_state.active_game.as_ref(),
                events,
            );
        }
        FullSessionTransition::FinishThenStart => {
            finish_full_session_recording(rec, full_session, &ctx);
            *full_session = begin_full_session_recording(
                rec,
                clips_dir,
                session_label,
                next_state.recording_mode,
                next_state.active_game.as_ref(),
                events,
            );
        }
    }
}
```

- [ ] **Step 9: Update frontend status handling**

In `apps/clipline-app/ui/app-core.js`, add:

```js
var capturePrivacyState = { kind: "game", label: null, slate_reason: null };
```

In `apps/clipline-app/ui/main.js`, update the status listener:

```js
capturePrivacyState = {
  kind: s.capture_kind || "game",
  label: s.capture_label || null,
  slate_reason: s.slate_reason || null,
};
```

In `apps/clipline-app/ui/settings.js`, at the start of `updateGameDetectionStatus()`, add:

```js
if (capturePrivacyState.kind === "slate" && $("set-games-follow-focused").checked) {
  $("game-detection-status").textContent = "Privacy slate active. Focus a saved game to resume capture.";
  return;
}
```

Add UI contract strings for these fields in `apps/clipline-app/tests/ui_contract.rs`:

```rust
for required in [
    "var capturePrivacyState = { kind: \"game\", label: null, slate_reason: null }",
    "s.capture_kind || \"game\"",
    "Privacy slate active. Focus a saved game to resume capture.",
] {
    assert!(main_js().contains(required), "focus-follow status wiring must include {required}");
}
```

- [ ] **Step 10: Run focused tests**

Run:

```powershell
cargo test -p clipline-app service::tests::focus_run_state_uses_latest_active_game_for_save_meta service::tests::focus_run_state_gates_plugin_markers_to_current_plugin service::tests::full_session_transition_splits_between_different_full_session_games service::tests::capture_switch_log_filters_to_saved_window
cargo test -p clipline-app --test ui_contract app_shell_contract
```

Expected: PASS.

- [ ] **Step 11: Commit**

```powershell
git add crates/clipline-events/src/markers.rs apps/clipline-app/src/service.rs apps/clipline-app/ui/app-core.js apps/clipline-app/ui/main.js apps/clipline-app/ui/settings.js apps/clipline-app/tests/ui_contract.rs
git commit -m "feat(capture): switch focused sources in recorder"
```

---

### Task 8: Final Verification, Handoff, And Live Test

**Files:**
- Modify: `handoff.md`

**Interfaces:**
- Consumes: all feature tasks above.
- Produces: verified local app run for manual testing.

- [ ] **Step 1: Run workspace tests**

Run:

```powershell
cargo test --workspace
```

Expected: PASS.

- [ ] **Step 2: Run clippy**

Run:

```powershell
cargo clippy --workspace --all-targets -- -D warnings
```

Expected: PASS with zero warnings.

- [ ] **Step 3: Update handoff**

Add this bullet to `handoff.md` in the current/completed state area:

```markdown
- Settings > Games now has an optional Follow focused game windows mode. It records the foreground enabled saved game, switches to a privacy slate with muted audio outside saved games, and keeps the replay buffer alive across focus changes.
```

- [ ] **Step 4: Commit handoff**

```powershell
git add handoff.md
git commit -m "docs: update handoff for focus-follow capture"
```

- [ ] **Step 5: Stop existing app processes**

Run:

```powershell
Get-Process clipline-app -ErrorAction SilentlyContinue | Stop-Process
```

Expected: No `clipline-app.exe` process remains.

- [ ] **Step 6: Launch the app**

Run:

```powershell
cargo run -p clipline-app
```

Expected: Clipline opens.

- [ ] **Step 7: Manual verification script**

Verify these user-visible behaviors:

- Settings > Games shows `Follow focused game windows`, off by default.
- With the option off, existing automatic game detection restarts behavior remains unchanged.
- With the option on and two saved games open, focusing game A then game B changes capture without clearing buffered seconds.
- Focusing a browser, chat, password manager, launcher, or Clipline shows the privacy slate and the saved replay contains silence for that interval.
- Focusing a saved game again resumes game capture.
- Save Replay across game -> slate -> game produces one playable MP4.
- League markers appear only while the focused target is League.
- Switching from one full-session game to another finalizes the first session file and starts the next.

- [ ] **Step 8: Final status**

Run:

```powershell
git status --short --branch
```

Expected: working tree contains only unrelated pre-existing files, or is clean if the implementer started from a clean tree.
