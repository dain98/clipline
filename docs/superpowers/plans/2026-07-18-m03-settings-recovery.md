# M-03 Settings Recovery Plan

**Goal:** Never silently replace a recoverable `settings.json` with whole-app defaults after a read, parse, or validation failure.

## Recovery classification

- [ ] Add a typed internal load failure that distinguishes missing files, unreadable files, and invalid JSON/settings while preserving the existing `load_from` API.
- [ ] Add tests proving a first run defaults only when both primary and backup are absent.
- [ ] Add tests proving a missing or invalid primary recovers a valid last-known-good backup and reports the recovery.
- [ ] Quarantine invalid primary/backup files under unique Clipline-owned sibling names; never move an unreadable path whose ownership/content could not be established.
- [ ] If neither copy is usable, start with safe defaults but return an explicit diagnostic containing the preserved/quarantined path and root cause.

## Durable last-known-good copy

- [ ] Before replacing a valid existing primary, atomically publish its exact bytes as `settings.json.bak`.
- [ ] Refuse to overwrite an existing primary that cannot first be read and validated, so a transient permission/sharing failure cannot turn a later save into data loss.
- [ ] Preserve an existing valid backup when the primary is invalid, unreadable, or a new write fails.
- [ ] Keep field-level legacy repair on the normal valid-file path without unnecessary quarantine.

## User-visible startup diagnostic

- [ ] Carry startup recovery messages into managed app state rather than emitting before WebView listeners exist.
- [ ] Return and drain those messages from `frontend_ready`, then render them in the existing persistent error area.
- [ ] Add Rust and UI-contract coverage proving the diagnostic is delivered once and is not silently discarded.

## Verification

- [ ] Run focused settings/app/UI-contract tests and fresh-cache app Clippy.
- [ ] Run CI-mode workspace tests and workspace Clippy with warnings denied.
- [ ] Rebuild/open Clipline and verify a normal settings load/save remains healthy.
- [ ] Update `handoff.md`, the master audit ledger, and the final manual acceptance list if an OS-only recovery scenario remains.
