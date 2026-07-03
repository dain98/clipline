# Settings Popup Design

## Goal

Change Clipline Settings from a full main-pane page into a modal popup, and add a two-step discard guard for unsaved settings edits.

## User-Facing Behavior

- Pressing the rail Settings button opens Settings as a centered popup over the current Library or Review view.
- The underlying view remains mounted behind a dim backdrop. Playback pauses when Settings opens, matching the existing full-page behavior.
- A clean settings form shows footer buttons `Save Settings` and `Close`.
- After any settings edit, `Close` changes to `Discard Changes`.
- The first `Discard Changes` press does not close the popup. It shakes the popup, shows the red warning `Careful--your changes aren't saved.` to the right of the popup, and adds a glow to `Save Settings`.
- A second `Discard Changes` press, with no intervening edit, discards the draft by repainting the form from `currentSettings` and closes the popup.
- Any new edit after the warning resets the discard confirmation, hides the warning, and requires another first warning press.
- Saving clears dirty/warning/glow state and keeps the popup open with the existing `saved` status.

## Architecture

Reuse the existing settings DOM, draft model, and save command. The main markup changes from a stacked `#settings-page` view into a modal overlay containing a `#settings-popup-shell` panel and a sibling warning element. This avoids duplicating settings tabs or changing persisted settings logic.

Dirty-state logic lives in `ui/settings.js` beside the existing form draft helpers:

- `syncSettingsDraftFromForm()` continues to read the whole form.
- A stable JSON snapshot comparison detects whether `settingsDraft` differs from `currentSettings`.
- A small discard-warning state tracks whether the current dirty draft has already received its first warning click.

View ownership remains in `ui/review-player.js`, but `updateViews()` stops hiding the Gallery/Review view while Settings is open. The settings overlay is drawn over whichever view is current.

## Components

- `apps/clipline-app/ui/index.html`
  - Wrap existing Settings content in `#settings-popup-shell`.
  - Add `role="dialog"` and `aria-modal="true"` on `#settings-page`.
  - Add `#settings-discard-warning` with the exact warning copy.

- `apps/clipline-app/ui/styles.css`
  - Restyle `.settings-page` as an overlay/backdrop.
  - Style `.settings-popup-shell` as the popup panel.
  - Add red warning placement, popup shake animation, and Save button glow.
  - Keep explicit `[hidden] { display: none }` rules.

- `apps/clipline-app/ui/settings.js`
  - Add stable settings snapshot comparison.
  - Update footer button labels/classes from dirty state.
  - Add first-click discard warning and reset helpers.
  - Ensure non-input settings changes, such as adding/removing custom games, update the dirty state.

- `apps/clipline-app/ui/review-player.js`
  - Keep the underlying Gallery/Review view visible while the settings popup is open.
  - Route settings close requests through the discard guard.

- `apps/clipline-app/ui/main.js`
  - Wire Settings rail, Close/Discard, and Escape through the guarded close path.
  - Keep Save using the existing `save_settings` command.

## Testing

- Extend `apps/clipline-app/tests/ui_contract.rs` with static contract coverage for:
  - Settings popup shell and warning markup.
  - Settings overlay/popup/warning/glow/shake CSS.
  - Dirty-state functions and guarded close wiring in JS.
  - Underlying Library/Review visibility no longer depending on `settingsOpen`.
- Run the focused UI contract test red first, then green after implementation.
- Run `cargo test --workspace` and `cargo clippy --workspace --all-targets -- -D warnings`.

## Out of Scope

- Persisted settings schema changes.
- Reorganizing settings tabs or controls.
- Replacing the settings UI with a native `<dialog>`.
- Adding a backdrop-click close behavior.
