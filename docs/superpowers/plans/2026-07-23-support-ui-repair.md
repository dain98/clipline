# Support UI Repair

## Goal

Repair the Support tab so it presents one coherent diagnostic-report state at a time, behaves
honestly when private upload is unavailable, and no longer exposes irrelevant settings actions.

## Implementation

- [ ] Add UI contracts for the Support grid, explicit `[hidden]` overrides, workflow-state
  rendering, development upload availability, and Support-aware settings footer.
- [ ] Move Support visibility and enabled-state decisions into a small DOM-free phase model with
  `idle`, `preparing`, `prepared`, `uploading`, and `success` states.
- [ ] Lock the exact description while a prepared report exists; return upload failures and
  cancellations to the prepared state for retry, save, or discard.
- [ ] Expose only whether the compile-time support endpoint is available. Keep release endpoint
  enforcement unchanged and keep Clipline Cloud outside the diagnostic path.
- [ ] Fix the Support layout and ensure preview, progress, and success panels honor `hidden`.
- [ ] Hide the generic settings save action on Support unless another tab has unsaved settings.

## Verification

- [ ] Run the focused Support core, UI contract, and native support tests.
- [ ] Run `cargo test --workspace`.
- [ ] Run `cargo clippy --workspace --all-targets -- -D warnings`.
- [ ] Rebuild and visually verify every Support state at the normal Clipline window size.
- [ ] Record the regression cause and `[hidden]` CSS invariant in `handoff.md`.
