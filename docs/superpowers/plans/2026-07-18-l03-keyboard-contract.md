# L-03 Keyboard Contract Plan

> **Finding:** L-03 — settings parsing, the Windows hook, player intents, and dialog guards disagree on keyboard behavior.

## Goal

Give every recorder hotkey consumer one typed parse result, remove the unimplemented review-player
`F` shortcut, and make the global player-key guard follow the actual set of open modal dialogs.

## TDD sequence

- [ ] Add a Windows-hook regression proving `Ctrl+Shift+F` is parsed as the literal `F` key while
  `F1` and `F24` remain function keys and mouse hotkeys retain their virtual-key mappings.
- [ ] Change the player-core contract so the removed focus-mode key is not consumed.
- [ ] Add a UI structural contract requiring the player guard to query the open-dialog state instead
  of maintaining a drifting list of dialog ids.
- [ ] Run the focused tests and record the expected failures.
- [ ] Expose a crate-private typed hotkey specification from settings parsing and map it directly to
  the low-level hook without reparsing its normalized display string.
- [ ] Remove the orphaned `KeyF` intent and derive modal suppression from `dialog[open]`.
- [ ] Run focused tests, fresh-cache app Clippy, workspace tests, and workspace Clippy with warnings
  denied.
- [ ] Rebuild and open Clipline for a native settings/dialog smoke check.
- [ ] Update `handoff.md`, the combined audit ledger, and the final manual checklist only for
  behavior that cannot be covered deterministically.

## Invariants

- [ ] Literal `F` and function keys `F1` through `F24` are distinct typed values at every consumer.
- [ ] Modifier state and mouse-button identity are parsed once and cannot drift between settings and
  the Windows hook.
- [ ] Every intent returned by `PlayerCore.keyIntent` has an implemented consumer; removed features
  do not consume browser key events.
- [ ] Any native `<dialog>` currently open owns the keyboard automatically, including future dialogs.
- [ ] Settings-page and form-field keyboard protections remain unchanged.

## Commits

- `docs(plan): define L-03 keyboard contract remediation`
- `fix(app): unify keyboard contracts`
- `docs(audit): close keyboard contract finding`
