# L-07 Cloud Settings Draft Preservation Plan

> **Finding:** L-07 — cloud connect/disconnect reloads the full settings object and discards unrelated unsaved edits.

## Goal

Apply authoritative cloud account/upload state after connect or disconnect without replacing the
user's settings draft, unrelated form controls, or user-editable cloud preferences.

## TDD sequence

- [ ] Add a pure CloudCore regression proving backend account fields and uploads are patched while
  local recording/audio/storage fields and cloud preferences remain unchanged.
- [ ] Add a UI contract that connect/disconnect synchronizes the form draft before awaiting and no
  longer calls full `fillSettings` during account reload.
- [ ] Run focused tests and record the expected failures.
- [ ] Define the explicit backend-owned cloud field set in CloudCore and expose an immutable merge.
- [ ] Patch that state into `currentSettings`, `settingsDraft`, and the comparison baseline, repaint
  only Cloud/account UI, and preserve dirty indicators for unrelated edits.
- [ ] Keep cloud account-key invalidation, gallery/profile refresh, upload records, and authoritative
  credential metadata current.
- [ ] Run focused tests, fresh-cache app Clippy, CI-mode workspace tests, and workspace Clippy with
  warnings denied.
- [ ] Rebuild/open Clipline and smoke the Cloud settings pane without requiring a real account.
- [ ] Update `handoff.md`, the combined audit ledger, and the final manual checklist only if real
  account behavior remains necessary.

## Invariants

- [ ] Connect/disconnect never replaces non-cloud draft fields or form controls.
- [ ] `default_visibility`, delete-local policy, and other user-editable cloud preferences remain the
  user's draft values until Save Settings.
- [ ] Host/account identity, public URL, credential target, and upload records follow backend state.
- [ ] Baseline patching does not falsely mark backend-owned connection transitions as user edits.
- [ ] Account changes still invalidate cloud request/cache generations before new loads.

## Commits

- `docs(plan): define L-07 cloud draft preservation`
- `fix(settings): preserve draft across cloud auth`
- `docs(audit): close cloud draft reset finding`
