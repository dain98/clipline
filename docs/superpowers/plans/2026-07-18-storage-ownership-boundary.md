# Storage Ownership Boundary Hardening Plan

**Goal:** Quota garbage collection and abandoned-recording recovery must never modify unrelated MP4 files merely because the user selected their containing directory as Clipline's media folder.

**Architecture:** Treat the existing `<clip>.clipline.json` sidecar as Clipline's per-file ownership marker. New replay and full-session paths receive a valid empty metadata document before recording starts. `clipline-storage` inventories or recovers only files with that marker, with an existing Clipline markers or osu! enrichment sidecar accepted as a conservative legacy signal for older saved clips. The currently saved clip remains explicitly included and protected during its first quota pass. Ambiguous unmarked MP4s fail safe: they remain visible to the library for compatibility, but storage accounting and destructive cleanup ignore them.

**Compatibility:** Existing clips that already have Clipline-specific metadata remain quota-managed. Older video-only clips with no sidecar are retained but no longer count toward the quota; editing their title/file through Clipline creates metadata and opts them back in. New recordings are always marked. No directory name or generic filename pattern is considered proof of ownership.

## Task 1: Lock the destructive ownership boundary with failing tests

- [ ] Add storage tests proving status and quota ignore an unmarked root MP4 and an unmarked MP4 in a direct child directory.
- [ ] Add a quota test proving an explicitly protected new clip is counted even before its ownership marker is visible to a concurrent inventory pass.
- [ ] Add recovery tests proving unmarked `.mp4.recording` files are untouched while marked recordings are recovered or cleaned up.
- [ ] Add coverage for mixed-case `.MP4.RECORDING` recovery so suffix handling remains internally consistent.
- [ ] Run `cargo test -p clipline-storage` and confirm the new assertions fail for the expected reason.

## Task 2: Implement managed-file discovery in `clipline-storage`

- [ ] Add public helpers to create a valid empty ownership metadata sidecar without overwriting existing metadata and to remove a marker created for a failed recording.
- [ ] Normalize recording paths to their intended final MP4 before deriving the ownership sidecar.
- [ ] Require a Clipline ownership/legacy sidecar in normal inventory and abandoned-recording recovery.
- [ ] Allow `enforce_quota` to include its explicitly protected path even if no sidecar is observable yet, while never allowing that path to be deleted.
- [ ] Remove mixed-case recording suffixes without corrupting the original filename.
- [ ] Run `cargo test -p clipline-storage`.

## Task 3: Mark every newly authored clip

- [ ] Create ownership metadata before replay serialization and remove it if the save fails.
- [ ] Create ownership metadata when reserving a full-session file; refuse to start if the marker cannot be created.
- [ ] Remove the newly created marker whenever an empty, too-short, failed-start, or deliberately discarded full session is removed.
- [ ] Keep the marker beside successful and recoverable recordings so startup recovery can prove ownership.
- [ ] Add service tests for marker creation and failure cleanup at the narrow helper boundaries.

## Task 4: Verify and document

- [ ] Run `cargo test --workspace`.
- [ ] Run `cargo clean -p clipline-storage` followed by `cargo clippy --workspace --all-targets -- -D warnings`.
- [ ] Update `handoff.md` with the new destructive-operation ownership rule and the conservative legacy behavior.
- [ ] Stop any running `clipline-app.exe`, launch `cargo run -p clipline-app`, and verify a new replay/full session creates its ownership metadata while an unrelated MP4 remains untouched.

