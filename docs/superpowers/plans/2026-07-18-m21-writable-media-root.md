# M-21 Writable Media Root and Fallback Consistency Plan

> **Finding:** M-21 — existing but unwritable media directories bypass fallback.

## Goal

Prove the configured media root can actually reserve output before recording uses it, fall back with
a visible warning when it cannot, and keep the runtime library/playback root aligned with the
directory where clips are really written.

## Design

- Replace directory-existence/create-only acceptance with a bounded unique `create_new` probe file
  in the candidate directory. Close and remove the probe before accepting the root.
- Keep probing synchronous and minimal: no directory scan and no network retry loop. The existing
  recorder startup thread absorbs removable/network filesystem latency without blocking the UI.
- Refactor resolution around an injectable probe so permission-denied behavior is deterministic on
  CI even when the test process can write to ordinary temporary directories.
- Probe the fallback too; return a clear combined error if neither root can accept an output.
- Emit one internal resolved-media-root event before recording begins. The app updates
  `StorageSettings` and the asset-protocol scope to the actual root before any Saved event, while
  retaining the user's configured path in persisted settings for later recovery/retry.
- Use the same writability check when saving a newly selected media folder so an unwritable choice
  fails before settings/runtime side effects are committed.

## TDD sequence

1. Add a failure-injected resolver test where `create_dir_all` succeeds for an existing configured
   root but the write probe returns `PermissionDenied`; assert fallback is selected and probed.
2. Add real-filesystem tests proving successful probes leave no file behind and fallback-probe
   failure reports both candidate paths.
3. Add app/service structural or unit coverage proving the resolved root event updates the library
   root and playback scope before normal service events are forwarded.
4. Implement the probe, resolver, event plumbing, and settings-save preflight.
5. Run focused tests, fresh-cache app Clippy, CI-mode workspace tests, and workspace Clippy.
6. Launch Clipline and smoke library/settings startup. Add a final manual acceptance item for an
   actually unwritable/removable/network media root because desktop ACL/share behavior cannot be
   reproduced faithfully by an injected unit test.
7. Update the audit ledger and `handoff.md` with commit and verification evidence.

## Expected commits

- `docs(plan): define M-21 media-root fallback remediation`
- `fix(storage): verify and publish writable media roots`
- `docs(audit): close media-root fallback finding`
