# Combined Codebase Audit Remediation Program

**Goal:** Resolve every still-relevant finding in `CODEBASE_AUDIT_COMBINED.md`, preserve regression coverage for each root cause, and finish with one practical manual acceptance checklist.

**Execution model:** Work proceeds in independently reviewable TDD batches. Every batch gets its own detailed plan, failing regression tests, one logical implementation commit, full proportional verification, and a handoff checkpoint. Findings that overlap or have already been fixed are verified against current code and recorded as satisfied rather than reimplemented. Security and irreversible data-loss boundaries come first, followed by resource/lifecycle safety, media correctness, configuration/UI correctness, and low-severity hardening.

## Current ledger

Already completed and verified on this branch:

- [ ] H-02 — destructive storage ownership boundary (`234f6af`)
- [ ] H-03 — cloud upload durability and delete-local boundary (`5323174`)
- [ ] H-04 — recorder desired-state/generation races (`820c68f`)
- [ ] M-10 — bounded full-session writer backlog (`5c3b810`)
- [ ] M-19 — keyboard-hook readiness and teardown (`820c68f`)
- [ ] H-01 / L-23 — full-app elevation and privileged relaunch removed (`5d06c21`)

Additional completed findings:

- [ ] M-01 — same-origin bearer controls, no authenticated redirects, separate token-free object PUT transport (`716b3d3`)
- [ ] M-02 — transactional settings/runtime side effects and durable credential cleanup (`99d5e7d`, `fc647fb`)
- [ ] M-03 — last-known-good backup, invalid-file quarantine, overwrite guard, and visible recovery diagnostic (`63dca68`)
- [ ] M-04 — LocalAppData cache, hard/aggregate/free-space bounds, LRU leases, safe temps/migration (`d54426b`)
- [ ] M-05 — bounded/reused HTTP clients, body caps, size-aware transfer deadlines, and finite deduplicated pagination (`3a51d1b`)
- [ ] M-06 — owned run recovery, cross-run quota accounting, transactional segments, and low-space finalization (`52eb9f4`)
- [ ] M-07 — persistent League match/event continuity, failure grace/backoff, reliable new-match reset, and idle receiver liveness (`905d976`)
- [ ] M-08 — canonical sidecar-derived osu! enrichment jobs with path/depth/mismatch/reparse validation (`d143dbc`)
- [ ] M-09 — per-root single-flight osu! worker, persisted backoff, atomic JSON publication, and per-record quarantine (`16b20f1`)
- [ ] M-11 — pre-append Annex-B cap, incremental start-code cursor, and generation-safe malformed reset (`725a310`)
- [ ] M-12 — first-slice/AUD picture framing, encoded AV1 frame types, and strict input/output timestamp cardinality (`68c6606`)
- [ ] M-13 — combined pending video/audio budget, maximum GOP duration, and monotonic PCM discontinuity anchors (`05152fd`)
- [ ] M-14 — pull-mode process loopback buffering, explicit WGC target closure, event-token teardown, and cadence end propagation (`e3190a0`)
- [ ] M-15 — concurrent bounded probe drainage, shared child deadlines, finite encoder flush/drop, and kill-before-reader-join teardown (`8ff611e`)
- [ ] H-05 — bounded file transforms, hashing, upload, and temporary ownership (`db86efe`)
- [ ] M-16 — hard-link identity checks and atomic MP4 publication (`db86efe`)

Recently hardened and requiring reconciliation against the combined labels before closure:

- [ ] L-02 — MP4 scalar/configuration boundaries
- [ ] L-27 — HEVC layer-count representation
- [ ] L-28 — public segment sample slicing

## Phase 1: remaining high severity

- [ ] H-01 — completed by removing the full-application elevation boundary (`5d06c21`).
- [ ] H-05 — completed with file-backed trim/remux/mix/clipboard paths and bounded file upload (`db86efe`).

## Phase 2: medium security, persistence, and lifecycle

- [ ] M-01 through M-09, excluding findings already subsumed by a stronger completed fix.
- [ ] M-11 through M-18, excluding M-16 completed by the H-05 file-transform batch.
- [ ] M-20 through M-23.

## Phase 3: low-severity hardening and debt

- [ ] L-01 through L-33, closing overlaps by evidence and implementing every independent remaining root cause.
- [ ] Run dependency/advisory and release-staging checks where the finding concerns CI or supply chain rather than runtime code.

## Verification contract for every batch

- [ ] Commit the batch plan before behavior changes.
- [ ] Add a failing test or a deterministic structural contract for every changed invariant.
- [ ] Run focused tests and fresh-cache clippy for changed crates.
- [ ] Run `cargo test --workspace` and workspace clippy with warnings denied.
- [ ] Rebuild and open the native app when the batch affects app/runtime behavior.
- [ ] Update `handoff.md` and this ledger with the finding ids and commit evidence.

## Final manual acceptance checklist

Accumulate only tests that require a real account, hardware, elevated game, slow/failing device, installer, or release environment and therefore cannot be safely completed with deterministic automated fixtures. The final handoff will group them by risk and provide expected results, setup, and cleanup.

- Elevated-game boundary: run a game as administrator while Clipline remains normal. Confirm the warning appears once for that process, recommends running the game without administrator privileges, contains no restart/UAC action, and ordinary Clipline recording remains unaffected after dismissal.
- Large trim: export a range from a multi-gigabyte/full-session clip. Confirm Clipline memory stays broadly flat, the source remains playable, no partial clip appears during export, and the completed trim plays through its end.
- Clipboard audio selection: copy one clip with a single selected audio track and again with multiple tracks mixed. Paste each into another app; confirm video is intact, only the selected/mixed audio is audible, memory stays broadly flat, and no `.clipline-*-tmp` files remain after completion.
- Large cloud upload: upload a large original clip and a selected-audio variant using a real account. Interrupt and retry a resumable upload; confirm memory remains bounded, the remote file plays, progress resumes correctly, and local media is deleted only when the configured policy and ready-media verification both permit it.
- Cloud cache pressure: with a real account, play several large remote clips until cache pressure triggers. Confirm cache data is under LocalAppData, the oldest unplayed entry is evicted, the clip currently playing remains available, total cache use returns under 10 GiB, and caching does not consume the final 2 GiB of free space.
- Replay-cache low space and restart: use disk replay buffering with full-session recording on a disposable/test volume, then reduce free space below the 2 GiB reserve. Confirm recording visibly stops, the full-session file finalizes or remains explicitly recoverable, no partial segment remains in the active run, and restarting Clipline removes stale owned runs without deleting another live Clipline instance's cache.
- Installed settings transaction: in an installed release build, change both recording hotkeys and `Open on startup`, save, restart/sign in, and confirm the hotkeys, tray label, and autostart behavior all match. Then make the settings folder temporarily unwritable and repeat a change; confirm the save reports failure and the prior hotkeys, tray label, autostart registration, and persisted settings all remain active. Restore folder permissions afterward.
- Credential transaction: using real Clipline Cloud and osu! credentials, connect, reconnect to a different account/user, disconnect, and restart. Confirm `settings.json` never contains a secret, Windows Credential Manager retains only the current referenced targets, and any obsolete target that could not be removed is cleaned by the next status check/startup.
- Remote integration continuity: with real accounts and a League match, load the Cloud library/profile/avatar, run the osu! connection test and recent-play enrichment, and record League markers. Briefly interrupt networking or the local League endpoint during each flow; confirm commands fail within their documented timeout instead of hanging, retry succeeds after recovery, large uploads/downloads continue while making progress, and the Cloud library reports truncation if the server exposes more than 10,000 unique clips. Keep the League interruption under twenty seconds and confirm it neither splits the match/session nor duplicates earlier markers, then start another match after `GameEnd` and confirm its low event IDs are accepted into a new session.
- Windows capture lifecycle: configure split per-process output audio for a windowed app/game that can keep playing sound while its image is static. Record at least one minute of that idle visual, save a replay, and confirm the selected process audio is continuous with no dropouts. Start another recording of that window, close the target, and confirm recording stops promptly instead of extending a frozen last frame; then reopen the target and confirm a fresh recording starts normally.
