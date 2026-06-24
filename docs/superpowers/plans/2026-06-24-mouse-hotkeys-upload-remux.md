# Mouse Hotkeys And Upload Remux Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let users bind Save Replay to useful mouse buttons, show the current Save Replay shortcut in the rail, and upload selected audio tracks by remuxing rather than mixing.

**Architecture:** Keep persisted hotkeys as normalized strings. Keyboard F-key hotkeys continue through Tauri global shortcuts plus the low-level fallback hook; mouse hotkeys use the low-level Windows hook only and skip Tauri registration. Cloud upload no longer shells out to FFmpeg for audio selection, and instead always uses the existing MP4 selected-audio remuxer.

**Tech Stack:** Rust/Tauri app shell, Windows low-level keyboard and mouse hooks through `windows-sys`, vanilla HTML/CSS/JS UI, Boa-backed player-core tests, Rust unit and UI contract tests.

---

### Task 1: Normalize Mouse Hotkeys

**Files:**
- Modify: `apps/clipline-app/src/settings/hotkey.rs`
- Modify: `apps/clipline-app/src/settings/tests.rs`

- [ ] **Step 1: Write failing tests**

Add tests proving aliases normalize to canonical mouse buttons and unsupported buttons still fail:

```rust
#[test]
fn parses_mouse_button_hotkeys() {
    assert_eq!(normalize_hotkey("mouse5").unwrap(), "Mouse5");
    assert_eq!(normalize_hotkey("ctrl+mouse4").unwrap(), "Ctrl+Mouse4");
    assert_eq!(normalize_hotkey("alt+forward").unwrap(), "Alt+Mouse5");
    assert_eq!(normalize_hotkey("shift+back").unwrap(), "Shift+Mouse4");
}

#[test]
fn rejects_unsafe_mouse_hotkeys() {
    assert!(parse_hotkey("Mouse1").is_err());
    assert!(parse_hotkey("RightMouse").is_err());
}
```

- [ ] **Step 2: Run tests to verify RED**

Run: `cargo test -p clipline-app settings::tests::parses_mouse_button_hotkeys settings::tests::rejects_unsafe_mouse_hotkeys`

Expected: fail because mouse-button hotkeys are not accepted yet.

- [ ] **Step 3: Implement normalization**

Extend `normalize_hotkey` so the key part can be one of:

- F1-F11 or F13-F24, as today
- `Middle` / `Mouse3` / `MButton`
- `Mouse4` / `XButton1` / `Back`
- `Mouse5` / `XButton2` / `Forward`

Canonical output should be `Middle`, `Mouse4`, or `Mouse5`, with modifiers before the key.

- [ ] **Step 4: Run tests to verify GREEN**

Run: `cargo test -p clipline-app settings::tests::parses_mouse_button_hotkeys settings::tests::rejects_unsafe_mouse_hotkeys`

Expected: pass.

### Task 2: Hook Mouse Hotkeys

**Files:**
- Modify: `apps/clipline-app/src/hotkeys.rs`
- Modify: `apps/clipline-app/src/app.rs`

- [ ] **Step 1: Write failing hook tests**

Add tests proving `parse_hook_hotkey("Mouse5")` maps to the XButton2 virtual key and modifiers are honored.

- [ ] **Step 2: Run tests to verify RED**

Run: `cargo test -p clipline-app hotkeys::tests::parses_mouse_button_hotkeys_for_hook_matching`

Expected: fail because mouse buttons are not handled by the hook.

- [ ] **Step 3: Implement low-level mouse hook support**

Change the hook model from keyboard-only to input-hook:

- Add `WH_MOUSE_LL`, `MSLLHOOKSTRUCT`, `WM_MBUTTONDOWN`, `WM_MBUTTONUP`, `WM_XBUTTONDOWN`, `WM_XBUTTONUP`, `VK_MBUTTON`, `VK_XBUTTON1`, and `VK_XBUTTON2`.
- Track down keys with the same debounce set used by keyboard hotkeys.
- On mouse down, compute `Ctrl`, `Alt`, and `Shift` via `GetAsyncKeyState`, then trigger if the configured key matches.
- Keep `set_save_hotkey` using the same parser so settings changes update the live hook.

In `app.rs`, register Tauri global shortcuts only when the normalized hotkey is an F-key hotkey. Mouse hotkeys should not attempt plugin registration/unregistration because the global-shortcut plugin cannot register mouse buttons.

- [ ] **Step 4: Run tests to verify GREEN**

Run: `cargo test -p clipline-app hotkeys::tests::parses_mouse_button_hotkeys_for_hook_matching`

Expected: pass.

### Task 3: Record Mouse Hotkeys In The UI And Show Current Hotkey In Rail

**Files:**
- Modify: `apps/clipline-app/ui/player-core.js`
- Modify: `apps/clipline-app/tests/player_core.rs`
- Modify: `apps/clipline-app/ui/index.html`
- Modify: `apps/clipline-app/ui/main.js`
- Modify: `apps/clipline-app/ui/styles.css`
- Modify: `apps/clipline-app/tests/ui_contract.rs`

- [ ] **Step 1: Write failing UI tests**

Add Boa tests for mouse event conversion and a UI contract test requiring a rail hotkey element below the RAM readout.

- [ ] **Step 2: Run tests to verify RED**

Run: `cargo test -p clipline-app --test player_core hotkey_recorder_formats_mouse_buttons` and `cargo test -p clipline-app --test ui_contract rail_shows_save_hotkey`

Expected: fail because mouse capture and the rail hotkey element are missing.

- [ ] **Step 3: Implement UI**

- Add `hotkeyFromMouseEvent` to `player-core.js`.
- In the settings hotkey recorder, listen for `mousedown` on the readonly input while capture is active.
- Capture middle, XButton1, and XButton2; ignore left and right with a clear inline error.
- Add `<div id="rail-hotkey" class="rail-hotkey" title="Save Replay hotkey">Alt+F10</div>` directly below `#memory-usage`.
- Update the rail hotkey text, rail save button title, and empty-library copy from the current settings hotkey whenever settings load or save.

- [ ] **Step 4: Run tests to verify GREEN**

Run: `cargo test -p clipline-app --test player_core hotkey_recorder_formats_mouse_buttons` and `cargo test -p clipline-app --test ui_contract rail_shows_save_hotkey`

Expected: pass.

### Task 4: Upload Selected Audio Tracks By Remuxing

**Files:**
- Modify: `apps/clipline-app/src/cloud.rs`

- [ ] **Step 1: Write failing upload tests**

Change the existing tests so selecting multiple audio tracks expects `UploadAudioSelectionPlan::Remux(vec![...])`, not `Mix`, and verifies the selected tracks remain separate in the uploaded MP4 bytes.

- [ ] **Step 2: Run tests to verify RED**

Run: `cargo test -p clipline-app cloud::tests::upload_audio_selection_remuxes_multiple_selected_tracks`

Expected: fail because multi-track upload currently chooses `Mix`.

- [ ] **Step 3: Implement remux-only upload selection**

Remove the upload-only FFmpeg mixing branch and make any explicit selected track set use `clipline_mp4::remux_with_selected_audio_tracks`. `None` selection still uploads the original bytes.

- [ ] **Step 4: Run tests to verify GREEN**

Run: `cargo test -p clipline-app cloud::tests::upload_audio_selection_remuxes_multiple_selected_tracks`

Expected: pass.

### Task 5: Verify And Handoff

**Files:**
- Modify: `handoff.md`
- Optionally modify: `README.md`

- [ ] **Step 1: Run focused app tests**

Run: `cargo test -p clipline-app`

Expected: pass.

- [ ] **Step 2: Run workspace gates**

Run: `cargo test --workspace`

Expected: pass.

Run: `cargo clean -p clipline-app && cargo clippy --workspace --all-targets -- -D warnings`

Expected: pass.

- [ ] **Step 3: Launch app for manual test**

Stop any existing `clipline-app.exe`, then run `cargo run -p clipline-app`.

Manual checks:
- Settings > Hotkeys can record `Mouse5`, `Ctrl+Mouse4`, and `Middle`.
- Left/right mouse buttons are rejected in the recorder.
- The rail shows the current Save Replay shortcut below RAM.
- Upload with multiple selected audio tracks no longer reports `ffmpeg is not available for audio track mixing`.

- [ ] **Step 4: Commit implementation**

Commit with a conventional message after verification, keeping the already-committed plan separate.
