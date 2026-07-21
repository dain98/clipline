# Cloud Upload Durability Plan

**Goal:** “Delete local after upload” must never remove the only usable copy before Clipline Cloud has explicitly finished processing and can serve the uploaded media.

**Architecture:** Post-upload handling becomes an explicit readiness state machine. Metadata polling continues through `processing` and unknown transient states, returns only on `ready`, treats `failed` as terminal, and classifies poll exhaustion separately. Every post-upload network or visibility error is converted into a persisted reconcilable record rather than escaping with only an IPC error. Before local cleanup, the client performs a bounded authenticated range request against the ready clip’s media asset. Cleanup deletes the MP4 first and only then attempts sidecars, returning and persisting any cleanup failure.

## Task 1: Readiness state-machine TDD

- [ ] Add HTTP-backed tests proving `processing` is not accepted as ready, `failed` terminates without deletion eligibility, and `ready` is returned.
- [ ] Add a timeout/unknown-status test that preserves the remote identity in `uploaded_processing`.
- [ ] Confirm the readiness tests fail against the current first-success behavior.

## Task 2: Durable remote verification

- [ ] Add a bounded, no-redirect, authenticated `Range: bytes=0-0` media probe.
- [ ] Test that a non-empty successful response is accepted and missing, empty, or failed media remains ineligible for local deletion.
- [ ] Run the probe only when delete-local-after-upload is enabled.

## Task 3: Persistent post-upload outcomes and ordered cleanup

- [ ] Persist `uploaded_processing` plus a useful error for polling, visibility, or media-verification failures after upload completion.
- [ ] Persist explicit remote processing failure without deleting the source.
- [ ] Make local cleanup fallible, remove the MP4 before sidecars, and preserve every sidecar when primary deletion fails.
- [ ] Persist cleanup failures on an otherwise ready upload record.

## Task 4: Verify and document

- [ ] Run focused cloud tests.
- [ ] Run `cargo test --workspace`.
- [ ] Clean the changed app crate and run `cargo clippy --workspace --all-targets -- -D warnings`.
- [ ] Update `handoff.md` with the readiness, media-verification, and cleanup ordering contracts.
- [ ] Stop any running workspace `clipline-app.exe`, launch `cargo run -p clipline-app`, and verify the native cloud/library UI initializes normally.
