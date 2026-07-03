# Settings Dirty Indicators Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add visible row glows and tab pips for Settings controls with unsaved draft changes.

**Architecture:** Mark settings surfaces with `data-settings-key`, compare keyed draft values against a normalized baseline captured after `fillSettings()`, and render row/tab indicator classes from that comparison. Keep save/discard behavior unchanged.

**Tech Stack:** Vanilla HTML/CSS/JS in `apps/clipline-app/ui`, Rust static UI contract tests in `apps/clipline-app/tests/ui_contract.rs`, workspace Cargo tests and clippy.

---

## File Structure

- Modify `apps/clipline-app/tests/ui_contract.rs`
  - Add one contract test that locks keyed markup, indicator CSS, and indicator JS helpers.
- Modify `apps/clipline-app/ui/index.html`
  - Add `data-settings-key` attributes to static Settings surfaces.
- Modify `apps/clipline-app/ui/settings.js`
  - Add the normalized indicator baseline and syncing helpers.
  - Add dynamic keys for rendered supported/custom game rows.
  - Sync indicators whenever dirty state changes or dynamic game rows are rendered.
- Modify `apps/clipline-app/ui/styles.css`
  - Add row glow and tab pip styling.
- Modify `handoff.md`
  - Note that Settings now marks changed rows and tabs.

---

### Task 1: Failing UI Contract Test

**Files:**
- Modify: `apps/clipline-app/tests/ui_contract.rs`
- Test: `apps/clipline-app/tests/ui_contract.rs`

- [ ] **Step 1: Add the failing test**

Add this test near `settings_opens_as_popup_and_guards_unsaved_discard`:

```rust
#[test]
fn settings_marks_changed_rows_and_tabs() {
    let html = index_html();
    let js = settings_js();
    let css = styles_css();

    for required in [
        "data-settings-key=\"open_on_startup\"",
        "data-settings-key=\"capture_mode capture_region window_title\"",
        "data-settings-key=\"audio.output_enabled audio.output_device_id audio.output_volume audio.split_output_by_process\"",
        "data-settings-key=\"games.plugins\"",
        "data-settings-key=\"games.custom_games\"",
        "data-settings-key=\"cloud.default_visibility\"",
        "data-settings-key=\"hotkey\"",
    ] {
        assert!(
            html.contains(required),
            "settings dirty indicator markup must include `{required}`"
        );
    }

    for required in [
        ".setting-changed",
        ".settings-tabs .tab.settings-tab-changed::after",
    ] {
        assert!(
            css.contains(required),
            "settings dirty indicator CSS must include `{required}`"
        );
    }

    for required in [
        "var settingsIndicatorBaseline = null;",
        "function settingsValueAtPath(source, path)",
        "function settingKeyChanged(path, draft, baseline)",
        "function syncSettingsChangeIndicators()",
        "node.classList.toggle(\"setting-changed\", changed)",
        "tab.classList.toggle(\"settings-tab-changed\", changed)",
        "settingsIndicatorBaseline = readSettings();",
        "row.dataset.settingsKey = `games.plugins.${plugin.id}`;",
        "row.dataset.settingsKey = \"games.custom_games\";",
    ] {
        assert!(
            js.contains(required),
            "settings dirty indicator JS must include `{required}`"
        );
    }
}
```

- [ ] **Step 2: Run the focused test and verify RED**

Run:

```powershell
cargo test -p clipline-app --test ui_contract settings_marks_changed_rows_and_tabs -- --nocapture
```

Expected: FAIL because the markup keys, CSS classes, and JS helpers do not exist yet.

---

### Task 2: Static Markup Keys

**Files:**
- Modify: `apps/clipline-app/ui/index.html`
- Test: `apps/clipline-app/tests/ui_contract.rs`

- [ ] **Step 1: Add keys to static settings rows**

Add `data-settings-key` attributes to the existing rows and grouped surfaces:

```html
<div class="setting-row" data-settings-key="open_on_startup">
<div class="setting-row" data-settings-key="close_to_tray">
<div class="setting-row" data-settings-key="minimize_to_tray">
<div class="setting-row" data-settings-key="legacy_timeline_editor">
<div class="setting-row" data-settings-key="update_channel">
<div class="setting-row" data-settings-key="capture_mode capture_region window_title">
<div class="setting-row" data-settings-key="capture_backend">
<div id="capture-region-editor" class="capture-region-editor" hidden data-settings-key="capture_region">
<div class="setting-row" data-settings-key="audio.output_enabled audio.output_device_id audio.output_volume audio.split_output_by_process">
<div class="setting-row" data-settings-key="audio.mic_enabled audio.mic_device_id audio.mic_volume audio.mic_channels">
<div class="setting-row" data-settings-key="games.auto_detect">
<div class="setting-row" data-settings-key="games.plugins">
<div class="games-panel" data-settings-key="games.custom_games">
<div class="setting-row" data-settings-key="video_encoder">
<div class="setting-row" data-settings-key="output_resolution">
<div class="setting-row" data-settings-key="replay_window_s buffer_seconds">
<div class="setting-row" data-settings-key="video_quality">
<div class="setting-row" data-settings-key="fps">
<div class="setting-row" data-settings-key="media_dir">
<div class="setting-row" data-settings-key="disk_quota_gb">
<div class="advanced-box" data-settings-key="replay_storage.mode replay_storage.disk_dir replay_storage.disk_quota_gb replay_storage.disk_acknowledged">
<div class="setting-row compact" data-settings-key="replay_storage.disk_dir">
<div class="setting-row compact" data-settings-key="replay_storage.disk_quota_gb">
<div class="setting-row" data-settings-key="cloud.credential_target cloud.connected_user_id">
<div class="setting-row" data-settings-key="cloud.default_visibility">
<div class="setting-row" data-settings-key="cloud.delete_local_after_upload">
<div class="setting-row" data-settings-key="cloud.auto_upload_rules">
<div class="setting-row" data-settings-key="hotkey">
```

- [ ] **Step 2: Run the focused test and verify partial RED**

Run:

```powershell
cargo test -p clipline-app --test ui_contract settings_marks_changed_rows_and_tabs -- --nocapture
```

Expected: FAIL only on missing CSS/JS behavior.

---

### Task 3: Indicator Helpers and Dynamic Keys

**Files:**
- Modify: `apps/clipline-app/ui/settings.js`
- Test: `apps/clipline-app/tests/ui_contract.rs`

- [ ] **Step 1: Add indicator state and helpers**

Add near the existing settings dirty-state helpers:

```javascript
var settingsIndicatorBaseline = null;

function settingsValueAtPath(source, path) {
  return String(path || "")
    .split(".")
    .filter(Boolean)
    .reduce((value, key) => (value == null ? undefined : value[key]), source);
}

function settingKeyChanged(path, draft, baseline) {
  return settingsSnapshot(settingsValueAtPath(draft, path))
    !== settingsSnapshot(settingsValueAtPath(baseline, path));
}

function settingsNodeKeys(node) {
  return String(node.dataset.settingsKey || "")
    .split(/\s+/)
    .filter(Boolean);
}

function syncSettingsChangeIndicators() {
  const draft = settingsDraft || {};
  const baseline = settingsIndicatorBaseline || currentSettings || {};
  const dirtyTabs = new Set();
  document.querySelectorAll("#settings-page [data-settings-key]").forEach((node) => {
    const changed = settingsNodeKeys(node).some((key) => settingKeyChanged(key, draft, baseline));
    node.classList.toggle("setting-changed", changed);
    const section = node.closest(".settings-section");
    if (changed && section && section.dataset.section) dirtyTabs.add(section.dataset.section);
  });
  document.querySelectorAll("#settings-tabs .tab").forEach((tab) => {
    const changed = dirtyTabs.has(tab.dataset.tab);
    tab.classList.toggle("settings-tab-changed", changed);
    if (changed) tab.setAttribute("aria-label", `${tab.textContent.trim()} has unsaved changes`);
    else tab.removeAttribute("aria-label");
  });
}
```

Update `syncSettingsDirtyState()` so it calls `syncSettingsChangeIndicators()` before returning.

- [ ] **Step 2: Capture the normalized baseline**

At the end of `fillSettings(s)`, after rendering dynamic settings UI and before `syncSettingsDirtyState({ resetDiscard: true })`, set:

```javascript
settingsIndicatorBaseline = readSettings();
```

- [ ] **Step 3: Add dynamic row keys**

In `renderGamePlugins()`, after setting `row.dataset.gamePluginId`, add:

```javascript
row.dataset.settingsKey = `games.plugins.${plugin.id}`;
```

In `renderCustomGames()`, after `row.className = "custom-game";`, add:

```javascript
row.dataset.settingsKey = "games.custom_games";
```

At the end of `renderGamePlugins()` and `renderCustomGames()`, call:

```javascript
syncSettingsChangeIndicators();
```

- [ ] **Step 4: Run the focused test and verify partial RED**

Run:

```powershell
cargo test -p clipline-app --test ui_contract settings_marks_changed_rows_and_tabs -- --nocapture
```

Expected: FAIL only on missing CSS classes.

---

### Task 4: Indicator Styling

**Files:**
- Modify: `apps/clipline-app/ui/styles.css`
- Test: `apps/clipline-app/tests/ui_contract.rs`

- [ ] **Step 1: Add row glow and tab pip CSS**

Add near the settings tab and row styles:

```css
.settings-tabs .tab {
  position: relative;
}

.settings-tabs .tab.settings-tab-changed::after {
  content: "";
  position: absolute;
  top: 7px;
  right: -9px;
  width: 6px;
  height: 6px;
  border-radius: 999px;
  background: #78a8ff;
  box-shadow: 0 0 10px rgba(61, 123, 253, 0.95);
}

.setting-changed {
  border-color: rgba(122, 184, 255, 0.38);
  border-radius: 8px;
  margin-inline: -10px;
  padding-inline: 10px;
  background: rgba(61, 123, 253, 0.08);
  box-shadow: 0 0 0 1px rgba(122, 184, 255, 0.24), 0 0 18px rgba(61, 123, 253, 0.2);
}
```

- [ ] **Step 2: Run the focused test and verify GREEN**

Run:

```powershell
cargo test -p clipline-app --test ui_contract settings_marks_changed_rows_and_tabs -- --nocapture
```

Expected: PASS.

---

### Task 5: Broader Verification, Handoff, Commit, Relaunch

**Files:**
- Modify: `handoff.md`

- [ ] **Step 1: Run UI contract tests**

Run:

```powershell
cargo test -p clipline-app --test ui_contract
```

Expected: PASS.

- [ ] **Step 2: Run workspace tests**

Run:

```powershell
cargo test --workspace
```

Expected: PASS.

- [ ] **Step 3: Run clippy with a fresh app cache**

Run:

```powershell
cargo clean -p clipline-app
cargo clippy --workspace --all-targets -- -D warnings
```

Expected: PASS with zero warnings.

- [ ] **Step 4: Update handoff**

Add a short note to `handoff.md`:

```markdown
- Settings now highlights rows with unsaved changes and shows a pip on any Settings tab containing changed rows; indicators clear when edits are saved, discarded, or reverted.
```

- [ ] **Step 5: Commit implementation**

Run:

```powershell
git add apps/clipline-app/tests/ui_contract.rs apps/clipline-app/ui/index.html apps/clipline-app/ui/settings.js apps/clipline-app/ui/styles.css handoff.md
git commit -m "feat(settings): mark changed settings"
```

- [ ] **Step 6: Relaunch Clipline for manual testing**

Run:

```powershell
Get-Process -Name 'clipline-app' -ErrorAction SilentlyContinue | Stop-Process -Force
cargo run -p clipline-app
```

Expected: Clipline opens for manual testing.
