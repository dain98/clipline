# L-32 Cloud Progress Rendering Plan

**Goal:** Stop multipart byte-progress events from rebuilding the complete Library while preserving
immediate, correctly sorted rendering for every meaningful upload-state transition.

**Finding:** `CODEBASE_AUDIT_COMBINED.md` L-32.

## Design boundary

- [ ] Keep the deck's byte/percentage status live on every event; that update is constant-size and
      does not touch gallery cards or poster observers.
- [ ] Move progress-record reconciliation into DOM-free `CloudCore`, returning both the next upload
      record and whether card membership, presentation, filtering, or ordering can have changed.
- [ ] Treat local/path/remote identity, URL, status, and error transitions as render-worthy and
      render those transitions immediately, including processing, failure, and uploaded states.
- [ ] Ignore byte-only progress for persisted frontend record shape and preserve `updated_at_unix`
      while the meaningful state is unchanged, preventing sort-order churn.
- [ ] Continue updating current settings on every event so commands and review actions read the
      latest reconciled record; do not alter native upload persistence or event cadence.

## TDD sequence

- [ ] Add Boa fixtures proving a first event and identity/status/error transitions request a render.
- [ ] Add a burst fixture proving hundreds of byte-only uploading events request no renders and do
      not change the record timestamp.
- [ ] Add UI contracts proving the event handler renders conditionally while the percentage deck
      update remains outside that condition.
- [ ] Implement the pure reconciler and migrate `upsertCloudProgress`/the event handler.

## Verification

- [ ] Run CloudCore and UI contract tests plus JavaScript syntax checks.
- [ ] Clean the app crate and run warning-denied Clippy for all app targets.
- [ ] Run CI-mode workspace tests and warning-denied workspace Clippy.
- [ ] Rebuild/open the native app and verify the Local/Cloud source controls remain healthy. The
      existing large real-account upload scenario will confirm live progress and terminal state,
      so extend that item with the gallery-stability observation instead of adding a duplicate.
- [ ] Update `handoff.md` and the combined remediation ledger.
