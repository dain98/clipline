# Simple Trim Timeline Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the default review deck timeline with an Outplayed-style simple browse/trim flow while preserving the current editor behind a legacy setting.

**Architecture:** Add a persisted `legacy_timeline_editor` setting, keep the existing trim/export backend, and layer a simple timeline UI mode on top of the current deck. The current navigator/zoom/snap surface remains in the DOM and is shown only when the legacy setting is enabled.

**Tech Stack:** Rust settings persistence and validation, vanilla HTML/CSS/JS Tauri UI, Boa-backed `player-core.js` tests, Rust UI contract tests.

---

### Task 1: Persist The Legacy Timeline Preference

**Files:**
- Modify: `apps/clipline-app/src/settings/mod.rs`
- Modify: `apps/clipline-app/src/settings/persistence.rs`
- Modify: `apps/clipline-app/src/settings/tests.rs`

- [ ] **Step 1: Write the failing settings tests**

Add assertions to `defaults_match_current_recorder_behavior`:

```rust
assert!(!settings.legacy_timeline_editor);
assert_eq!(serialized["legacy_timeline_editor"], false);
```

Add a load test:

```rust
#[test]
fn load_preserves_legacy_timeline_editor_preference() {
    let settings = AppSettings::load_from_object(
        serde_json::from_str::<Value>(
            r#"{
                "capture_mode": "primary_monitor",
                "window_title": "",
                "buffer_seconds": 75.0,
                "replay_window_s": 60.0,
                "bitrate_mbps": 12.0,
                "fps": 60,
                "disk_quota_gb": 10.0,
                "hotkey": "Alt+F10",
                "legacy_timeline_editor": true
            }"#,
        )
        .unwrap()
        .as_object()
        .unwrap(),
    );

    assert!(settings.legacy_timeline_editor);
}
```

- [ ] **Step 2: Run the failing settings test**

Run:

```powershell
cargo test -p clipline-app settings::tests::load_preserves_legacy_timeline_editor_preference -- --nocapture
```

Expected: FAIL because `AppSettings` has no `legacy_timeline_editor` field.

- [ ] **Step 3: Implement the setting**

Add `#[serde(default)] pub legacy_timeline_editor: bool` to `AppSettings`, set it to `false` in `Default`, and load it in `load_from_object` with:

```rust
legacy_timeline_editor: bool_field(object, "legacy_timeline_editor")
    .unwrap_or(defaults.legacy_timeline_editor),
```

- [ ] **Step 4: Run the settings test again**

Run:

```powershell
cargo test -p clipline-app settings::tests::load_preserves_legacy_timeline_editor_preference -- --nocapture
```

Expected: PASS.

### Task 2: Add Pure Simple-Trim Math

**Files:**
- Modify: `apps/clipline-app/ui/player-core.js`
- Modify: `apps/clipline-app/tests/player_core.rs`

- [ ] **Step 1: Write the failing player-core test**

Add:

```rust
#[test]
fn quick_trim_range_centers_on_playhead_and_clamps_to_clip() {
    let mut ctx = player_core_context();

    assert_eq!(
        eval_json(&mut ctx, "PlayerCore.quickTrimRange(50, 120)"),
        r#"{"start":35,"end":65}"#
    );
    assert_eq!(
        eval_json(&mut ctx, "PlayerCore.quickTrimRange(4, 120)"),
        r#"{"start":0,"end":30}"#
    );
    assert_eq!(
        eval_json(&mut ctx, "PlayerCore.quickTrimRange(118, 120)"),
        r#"{"start":90,"end":120}"#
    );
    assert_eq!(
        eval_json(&mut ctx, "PlayerCore.quickTrimRange(8, 12)"),
        r#"{"start":0,"end":12}"#
    );
}
```

- [ ] **Step 2: Run the failing player-core test**

Run:

```powershell
cargo test -p clipline-app quick_trim_range_centers_on_playhead_and_clamps_to_clip -- --nocapture
```

Expected: FAIL because `PlayerCore.quickTrimRange` is missing.

- [ ] **Step 3: Implement `quickTrimRange`**

Add a pure helper near `resolveTrim`:

```javascript
const QUICK_TRIM_WINDOW_S = 30;

const quickTrimRange = (playhead, duration, windowS = QUICK_TRIM_WINDOW_S) => {
  if (!(duration > 0)) return { start: 0, end: 0 };
  const span = Math.min(duration, Math.max(MIN_TRIM_GAP_S, windowS));
  const center = clampTime(Number.isFinite(playhead) ? playhead : 0, duration);
  const start = Math.max(0, Math.min(duration - span, center - span / 2));
  return { start, end: start + span };
};
```

Export `QUICK_TRIM_WINDOW_S` and `quickTrimRange`.

- [ ] **Step 4: Run the player-core test again**

Run:

```powershell
cargo test -p clipline-app quick_trim_range_centers_on_playhead_and_clamps_to_clip -- --nocapture
```

Expected: PASS.

### Task 3: Wire Simple And Legacy Timeline Controls

**Files:**
- Modify: `apps/clipline-app/ui/index.html`
- Modify: `apps/clipline-app/ui/main.js`
- Modify: `apps/clipline-app/ui/styles.css`
- Modify: `apps/clipline-app/tests/ui_contract.rs`

- [ ] **Step 1: Write the failing UI contract test**

Extend `review_player_owns_all_controls` so it requires:

```rust
"id=\"trim-mode-toggle\"",
"id=\"set-legacy-timeline-editor\"",
```

Add assertions that `main.js` contains:

```rust
main_js().contains("function setSimpleTrimMode(active)")
main_js().contains("function applyTimelineEditorPreference()")
main_js().contains("legacy_timeline_editor")
main_js().contains("quickTrimRange(")
```

Add assertions that `styles.css` contains:

```rust
styles_css().contains(".deck.simple-timeline")
styles_css().contains(".deck.simple-trim-active")
styles_css().contains(".deck.legacy-timeline")
```

- [ ] **Step 2: Run the failing UI contract test**

Run:

```powershell
cargo test -p clipline-app --test ui_contract review_player_owns_all_controls -- --nocapture
```

Expected: FAIL because the new controls and mode wiring are missing.

- [ ] **Step 3: Update markup**

In `index.html`, add a General settings row for `Legacy timeline editor` with checkbox `set-legacy-timeline-editor`.

In `.timeline-tools`, add the scissors button:

```html
<button id="trim-mode-toggle" title="Trim clip">
  <svg viewBox="0 0 24 24"><path d="M9.64 7.64c.23-.5.36-1.05.36-1.64 0-2.21-1.79-4-4-4S2 3.79 2 6s1.79 4 4 4c.59 0 1.14-.13 1.64-.36L10 12l-2.36 2.36C7.14 14.13 6.59 14 6 14c-2.21 0-4 1.79-4 4s1.79 4 4 4 4-1.79 4-4c0-.59-.13-1.14-.36-1.64L12 14l7 7h3v-1L9.64 7.64zM6 8c-1.1 0-2-.9-2-2s.9-2 2-2 2 .9 2 2-0.9 2-2 2zm0 12c-1.1 0-2-.9-2-2s.9-2 2-2 2 .9 2 2-0.9 2-2 2zM19 3l-6 6 2 2 7-7V3h-3z"/></svg>
</button>
```

- [ ] **Step 4: Update JS mode wiring**

Add `let simpleTrimMode = false;`.

Add `legacyTimelineEnabled`, `applyTimelineEditorPreference`, and `setSimpleTrimMode` helpers. `setSimpleTrimMode(true)` should call `quickTrimRange(video.currentTime || 0, clipDuration())`, `setTrim`, `applyView(viewForRange(...))`, and change the export button label to `Create Clip`. `setSimpleTrimMode(false)` should call `zoomFit()` and restore browse styling.

Wire `trim-mode-toggle` click to toggle simple trim mode. Keep `zoom-in`, `zoom-out`, `zoom-fit`, `snap-toggle`, and navigator behavior available in legacy mode.

- [ ] **Step 5: Update CSS**

Add deck classes:

```css
.deck.simple-timeline #overview,
.deck.simple-timeline #zoom-out,
.deck.simple-timeline #zoom-fit,
.deck.simple-timeline #zoom-in,
.deck.simple-timeline #snap-toggle {
  display: none;
}

.deck.simple-timeline:not(.simple-trim-active) #dim-in,
.deck.simple-timeline:not(.simple-trim-active) #dim-out,
.deck.simple-timeline:not(.simple-trim-active) #handle-in,
.deck.simple-timeline:not(.simple-trim-active) #handle-out,
.deck.simple-timeline:not(.simple-trim-active) #trim-band {
  display: none !important;
}

.deck.simple-trim-active #trim-mode-toggle,
.deck.legacy-timeline #trim-mode-toggle {
  color: var(--accent);
}
```

- [ ] **Step 6: Run the UI contract test again**

Run:

```powershell
cargo test -p clipline-app --test ui_contract review_player_owns_all_controls -- --nocapture
```

Expected: PASS.

### Task 4: Verify And Launch

**Files:**
- Modify: `handoff.md`

- [ ] **Step 1: Syntax-check JavaScript**

Run:

```powershell
node --check apps/clipline-app/ui/player-core.js
node --check apps/clipline-app/ui/main.js
```

Expected: both commands exit 0.

- [ ] **Step 2: Run targeted tests**

Run:

```powershell
cargo test -p clipline-app quick_trim_range_centers_on_playhead_and_clamps_to_clip -- --nocapture
cargo test -p clipline-app settings::tests::load_preserves_legacy_timeline_editor_preference -- --nocapture
cargo test -p clipline-app --test ui_contract review_player_owns_all_controls -- --nocapture
```

Expected: all targeted tests pass.

- [ ] **Step 3: Run workspace tests**

Run:

```powershell
cargo test --workspace
```

Expected: PASS.

- [ ] **Step 4: Run clippy**

Run:

```powershell
cargo clippy --workspace --all-targets -- -D warnings
```

Expected: PASS with no warnings.

- [ ] **Step 5: Stop existing app and launch**

Run:

```powershell
Get-Process clipline-app -ErrorAction SilentlyContinue | Stop-Process
cargo run -p clipline-app
```

Expected: the app opens with the simple timeline by default. The settings checkbox restores the legacy navigator/zoom/snap editor.

- [ ] **Step 6: Update handoff**

Add a short note to `handoff.md` describing the simple timeline default, the legacy settings toggle, and the manual verification focus.
