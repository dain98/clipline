# M-06 Replay Cache Lifecycle Plan

**Goal:** Make every recorder exit finalize consistently and ensure disk replay-cache ownership, cleanup, and quota accounting survive crashes and filesystem failures.

## Disk ring transactions

- [ ] Add failure-injection tests proving partial segment `.tmp` files are removed after write/flush/rename errors.
- [ ] Stage a new segment under an owned path guard and do not commit ring bookkeeping until required eviction succeeds.
- [ ] On partial eviction failure, update bookkeeping for every file already removed, discard the uncommitted new segment, and leave the ring at or below its prior budget.
- [ ] Make run-directory teardown remove all Clipline-owned contents, including orphaned temporary files and the ownership record.

## Run ownership and abandoned cleanup

- [ ] Create a run ownership record containing process-instance identity and creation time.
- [ ] Sweep only correctly named Clipline run directories. Preserve another live process instance, skip linked/reparse directories, and require a stale age before deleting missing/unqueryable ownership records.
- [ ] Count preserved live/unsweepable run bytes against the configured cache quota before assigning the new ring budget.
- [ ] Clean a newly reserved run if recorder construction fails before ownership transfers to the disk ring.

## Recorder shutdown funnel

- [ ] Add a deterministic lifecycle test for a low-space status failure after full-session recording has begun.
- [ ] Route low-space, capture failure, Stop, channel disconnect, and natural capture end through one shutdown/finalization funnel.
- [ ] Always emit the stopped state after teardown and preserve the primary failure while surfacing any finish/finalize failure.

## Verification

- [ ] Run focused buffer/service tests and fresh-cache Clippy for both changed crates.
- [ ] Run CI-mode workspace tests and workspace Clippy with warnings denied.
- [ ] Rebuild/open Clipline for a native smoke test.
- [ ] Update `handoff.md`, the master audit ledger, and the manual acceptance checklist.

## Manual acceptance additions

- Start disk replay buffering and full-session recording, then make the replay-cache drive cross the 2 GiB reserve. Confirm recording stops visibly, the full-session file finalizes or is explicitly recoverable, and restarting Clipline removes only stale owned cache runs while preserving any live instance.
