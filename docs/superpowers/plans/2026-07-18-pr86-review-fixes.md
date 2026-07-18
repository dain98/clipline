# PR #86 Review Fixes

## Goal

Address the actionable review findings on PR #86 without regressing cadence
downsampling or the mixed-output safety default.

## Plan

- [ ] Add a failing cadence regression test proving one stale queued frame yields
      control back to the service loop instead of draining the queue internally.
- [ ] Change `CadencedCapture` to retain the newest stale texture and surface a
      bounded timeout result, allowing command processing between dropped frames.
- [ ] Add failing player-core assertions proving split-output defaults survive
      selection normalization and render as selected.
- [ ] Preserve an explicitly selected mixed-output track unless process tracks
      replace it, while keeping the existing master-toggle behavior.
- [ ] Run the focused tests, workspace tests, formatting check, and clippy.
- [ ] Re-review the complete branch diff for remaining actionable defects.
