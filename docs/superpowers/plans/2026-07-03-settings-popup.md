# Settings Popup Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Convert Settings from a full main-pane page into a popup with a two-step discard warning for unsaved edits.

**Architecture:** Reuse the existing settings form and draft/save flow. Restyle `#settings-page` as a modal overlay containing a `#settings-popup-shell`, add dirty-state helpers in `settings.js`, and route Settings close actions through a guarded close function.

**Tech Stack:** Vanilla HTML/CSS/JS in `apps/clipline-app/ui`, Rust static UI contract tests in `apps/clipline-app/tests/ui_contract.rs`, workspace Cargo tests and clippy.

---

## File Structure

- Modify `apps/clipline-app/tests/ui_contract.rs`
  - Add a focused test that locks the popup markup, CSS affordances, dirty-state helper names, and guarded close wiring.
- Modify `apps/clipline-app/ui/index.html`
  - Wrap existing settings content in `#settings-popup-shell`.
  - Add `role="dialog"`, `aria-modal="true"`, `aria-labelledby="settings-title"`, and `#settings-discard-warning`.
- Modify `apps/clipline-app/ui/styles.css`
  - Convert `.settings-page` from a full-bleed stacked page to an overlay.
  - Add `.settings-popup-shell`, `.settings-discard-warning`, `.settings-shake`, and `.settings-save-glow`.
- Modify `apps/clipline-app/ui/settings.js`
  - Add stable snapshot comparison, dirty-state rendering, discard warning, and discard reset helpers.
  - Ensure programmatic custom-game changes refresh the draft dirty state.
- Modify `apps/clipline-app/ui/review-player.js`
  - Keep Gallery/Review visible behind the settings overlay.
  - Add `requestSettingsClose()` to run the discard guard before closing.
- Modify `apps/clipline-app/ui/main.js`
  - Wire rail Settings, footer Close/Discard, and Escape through `requestSettingsClose()`.
- Modify `handoff.md`
  - Add a short note after implementation because this is a visible Settings workflow change.

---

### Task 1: Failing UI Contract Test

**Files:**
- Modify: `apps/clipline-app/tests/ui_contract.rs`
- Test: `apps/clipline-app/tests/ui_contract.rs`

- [ ] **Step 1: Add the failing test**

Add this test near the other UI contract tests:

```rust
#[test]
fn settings_opens_as_popup_and_guards_unsaved_discard() {
    let html = index_html();
    let js = main_js();
    let css = styles_css();

    for required in [
        "id=\"settings-page\" class=\"settings-page\" hidden role=\"dialog\" aria-modal=\"true\"",
        "id=\"settings-title\"",
        "id=\"settings-popup-shell\"",
        "id=\"settings-discard-warning\"",
        "Careful--your changes aren't saved.",
    ] {
        assert!(
            html.contains(required),
            "settings popup markup must include `{required}`"
        );
    }

    for required in [
        ".settings-popup-shell",
        ".settings-discard-warning",
        ".settings-save-glow",
        ".settings-shake",
        "@keyframes settings-shake",
        "@keyframes settings-save-glow",
    ] {
        assert!(
            css.contains(required),
            "settings popup CSS must include `{required}`"
        );
    }

    for required in [
        "function stableSettingsSnapshot(value)",
        "function settingsHaveUnsavedChanges()",
        "function syncSettingsDirtyState",
        "function showSettingsDiscardWarning()",
        "function resetSettingsDiscardWarning()",
        "function requestSettingsClose()",
        "$(\"settings-close\").textContent = dirty ? \"Discard Changes\" : \"Close\"",
        "$(\"settings-save\").classList.toggle(\"settings-save-glow\"",
        "$(\"settings-discard-warning\").textContent = \"Careful--your changes aren't saved.\"",
        "$(\"rail-settings\").addEventListener(\"click\", () => {",
        "$(\"settings-close\").addEventListener(\"click\", requestSettingsClose)",
        "requestSettingsClose();",
    ] {
        assert!(
            js.contains(required),
            "settings popup JS must include `{required}`"
        );
    }

    assert!(
        js.contains("$(\"review-viewer\").hidden = !currentClip")
            && js.contains("$(\"gallery-view\").hidden = !!currentClip"),
        "settings popup must not hide the underlying review/gallery view"
    );
}
```

- [ ] **Step 2: Run the focused test and verify RED**

Run:

```powershell
cargo test -p clipline-app --test ui_contract settings_opens_as_popup_and_guards_unsaved_discard -- --nocapture
```

Expected: FAIL because the popup shell, warning markup, CSS classes, and guarded close helpers do not exist yet.

---

### Task 2: Popup Markup and Styles

**Files:**
- Modify: `apps/clipline-app/ui/index.html`
- Modify: `apps/clipline-app/ui/styles.css`
- Test: `apps/clipline-app/tests/ui_contract.rs`

- [ ] **Step 1: Wrap the Settings markup**

Change the opening Settings section from:

```html
<section id="settings-page" class="settings-page" hidden>
  <header class="settings-head">
    <h1>Settings</h1>
  </header>
```

to:

```html
<section id="settings-page" class="settings-page" hidden role="dialog" aria-modal="true" aria-labelledby="settings-title">
  <div id="settings-popup-shell" class="settings-popup-shell">
    <header class="settings-head">
      <h1 id="settings-title">Settings</h1>
    </header>
```

Then move the existing `</section>` so the existing Settings content closes `</div>` first, followed by the warning:

```html
    <footer class="settings-actions">
      <button id="settings-save" type="button" class="primary">Save Settings</button>
      <button id="settings-close" type="button">Close</button>
      <span id="settings-status" class="hint"></span>
    </footer>
  </div>
  <span id="settings-discard-warning" class="settings-discard-warning" hidden>Careful--your changes aren't saved.</span>
</section>
```

- [ ] **Step 2: Convert Settings CSS to popup CSS**

Replace the existing `.settings-page` block with:

```css
/* ---- settings popup (tabbed) ---- */
.settings-page {
  grid-area: 1 / 1;
  position: relative;
  z-index: 30;
  display: grid;
  grid-template-columns: minmax(600px, min(760px, calc(100% - 252px))) 220px;
  align-items: center;
  justify-content: center;
  gap: 16px;
  min-height: 0;
  min-width: 0;
  padding: 24px;
  background: rgba(5, 7, 10, 0.62);
}
.settings-page[hidden] { display: none; }

.settings-popup-shell {
  display: grid;
  grid-template-rows: auto auto minmax(0, 1fr) auto;
  width: 100%;
  max-height: min(720px, calc(100vh - var(--titlebar-h) - 48px));
  min-height: 0;
  border: 1px solid var(--line-strong);
  border-radius: 10px;
  background: var(--panel);
  color: var(--text);
  box-shadow: 0 18px 50px rgba(0, 0, 0, 0.55);
}
```

Keep the existing settings head/tabs/body rules, then add:

```css
.settings-discard-warning {
  align-self: center;
  color: #ff7b86;
  font-size: 13px;
  font-weight: 650;
  line-height: 1.35;
  text-shadow: 0 0 14px rgba(229, 72, 77, 0.34);
}
.settings-discard-warning[hidden] { display: none; }

.settings-shake {
  animation: settings-shake 180ms ease-in-out 0s 2;
}

button.primary.settings-save-glow {
  box-shadow: 0 0 0 1px rgba(122, 184, 255, 0.55), 0 0 18px rgba(61, 123, 253, 0.82);
  animation: settings-save-glow 900ms ease-in-out infinite alternate;
}

@keyframes settings-shake {
  0%, 100% { transform: translateX(0); }
  25% { transform: translateX(-7px); }
  75% { transform: translateX(7px); }
}

@keyframes settings-save-glow {
  from { box-shadow: 0 0 0 1px rgba(122, 184, 255, 0.48), 0 0 12px rgba(61, 123, 253, 0.62); }
  to { box-shadow: 0 0 0 1px rgba(122, 184, 255, 0.8), 0 0 24px rgba(61, 123, 253, 0.95); }
}

@media (max-width: 1180px) {
  .settings-page {
    grid-template-columns: minmax(0, 720px);
    align-content: center;
  }

  .settings-discard-warning {
    justify-self: end;
    align-self: start;
  }
}
```

- [ ] **Step 3: Run the focused test and verify partial RED**

Run:

```powershell
cargo test -p clipline-app --test ui_contract settings_opens_as_popup_and_guards_unsaved_discard -- --nocapture
```

Expected: FAIL only on missing JavaScript behavior.

---

### Task 3: Dirty-State and Guarded Close Behavior

**Files:**
- Modify: `apps/clipline-app/ui/settings.js`
- Modify: `apps/clipline-app/ui/review-player.js`
- Modify: `apps/clipline-app/ui/main.js`
- Test: `apps/clipline-app/tests/ui_contract.rs`

- [ ] **Step 1: Add dirty-state helpers in `settings.js`**

Add after `cloneSettings`:

```javascript
var settingsDiscardWarningArmed = false;

function stableSettingsSnapshot(value) {
  if (Array.isArray(value)) {
    return value.map(stableSettingsSnapshot);
  }
  if (value && typeof value === "object") {
    return Object.keys(value)
      .sort()
      .reduce((out, key) => {
        out[key] = stableSettingsSnapshot(value[key]);
        return out;
      }, {});
  }
  return value;
}

function settingsSnapshot(value) {
  return JSON.stringify(stableSettingsSnapshot(value || null));
}

function settingsHaveUnsavedChanges() {
  return settingsSnapshot(settingsDraft) !== settingsSnapshot(currentSettings);
}

function resetSettingsDiscardWarning() {
  settingsDiscardWarningArmed = false;
  $("settings-discard-warning").hidden = true;
  $("settings-save").classList.remove("settings-save-glow");
  $("settings-popup-shell").classList.remove("settings-shake");
}

function syncSettingsDirtyState({ resetDiscard = false } = {}) {
  const dirty = settingsHaveUnsavedChanges();
  if (resetDiscard || !dirty) resetSettingsDiscardWarning();
  $("settings-close").textContent = dirty ? "Discard Changes" : "Close";
  $("settings-close").classList.toggle("settings-discard", dirty);
  $("settings-save").classList.toggle("settings-save-glow", dirty && settingsDiscardWarningArmed);
  return dirty;
}

function showSettingsDiscardWarning() {
  settingsDiscardWarningArmed = true;
  $("settings-discard-warning").textContent = "Careful--your changes aren't saved.";
  $("settings-discard-warning").hidden = false;
  $("settings-save").classList.add("settings-save-glow");
  const shell = $("settings-popup-shell");
  shell.classList.remove("settings-shake");
  void shell.offsetWidth;
  shell.classList.add("settings-shake");
}
```

Update `syncSettingsDraftFromForm()` to:

```javascript
function syncSettingsDraftFromForm() {
  settingsDraft = readSettings();
  syncSettingsDirtyState({ resetDiscard: true });
  return settingsDraft;
}
```

At the end of `fillSettings(s)`, add:

```javascript
  syncSettingsDirtyState({ resetDiscard: true });
```

Update `syncGamePluginSettingsDraft()` to call `syncSettingsDirtyState({ resetDiscard: true })` after assigning `settingsDraft`.

After custom-game add/remove programmatic mutations, call `syncSettingsDraftFromForm()`.

- [ ] **Step 2: Update close/view behavior**

In `review-player.js`, change `updateViews()` to:

```javascript
function updateViews() {
  $("settings-page").hidden = !settingsOpen;
  $("review-viewer").hidden = !currentClip;
  $("gallery-view").hidden = !!currentClip;
}
```

Add before `toggleSettings()`:

```javascript
function requestSettingsClose() {
  if (!settingsOpen) return;
  if (settingsHaveUnsavedChanges()) {
    if (!settingsDiscardWarningArmed) {
      showSettingsDiscardWarning();
      return;
    }
  }
  toggleSettings(false);
}
```

In `toggleSettings()`, call `resetSettingsDiscardWarning()` when opening a fresh popup, and keep the existing close-time `fillSettings(currentSettings)` discard path.

- [ ] **Step 3: Route event handlers through guarded close**

In `main.js`, replace:

```javascript
$("rail-settings").addEventListener("click", () => toggleSettings());
$("settings-close").addEventListener("click", () => toggleSettings(false));
```

with:

```javascript
$("rail-settings").addEventListener("click", () => {
  if (settingsOpen) requestSettingsClose();
  else toggleSettings(true);
});
$("settings-close").addEventListener("click", requestSettingsClose);
```

In the Escape handler, replace `toggleSettings(false);` with `requestSettingsClose();`.

- [ ] **Step 4: Run the focused test and verify GREEN**

Run:

```powershell
cargo test -p clipline-app --test ui_contract settings_opens_as_popup_and_guards_unsaved_discard -- --nocapture
```

Expected: PASS.

---

### Task 4: Broader Verification and Handoff

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

- [ ] **Step 3: Run clippy**

Run:

```powershell
cargo clippy --workspace --all-targets -- -D warnings
```

Expected: PASS with zero warnings.

- [ ] **Step 4: Update handoff**

Add a short note to `handoff.md`:

```markdown
- Settings now opens as a popup over the current Library/Review view. Unsaved edits change Close to Discard Changes; the first discard attempt warns/shakes/glows Save, and the second attempt discards and closes.
```

- [ ] **Step 5: Launch the app for manual testing**

If a previous app process is running, stop it first. Then run:

```powershell
cargo run -p clipline-app
```

Expected: Clipline opens for manual testing.
