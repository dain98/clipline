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
- [ ] M-17 — exact per-track fragment/edit-list timing, integer trim/remux boundaries, complete AVC/HEVC parameter arrays, and capture PTS retention (`ec6f373`)
- [ ] L-02 — bounded MP4 scalar parsing plus release-mode public writer/config validation (`ec6f373`)
- [ ] L-27 — reserved HEVC temporal-layer counts rejected before hvcC serialization (`ec6f373`)
- [ ] L-28 — malformed public segment sample metadata returns `InvalidData` instead of panicking (`ec6f373`)
- [ ] M-18 — real Clipline HWND ownership, bounded busy retry, `EmptyClipboard`, and exact `CF_HDROP` allocation transfer (`68bbc82`)
- [ ] M-20 — reserved custom-ID namespace, deterministic legacy collision aliases, and typed built-in/custom runtime identity (`2d0a33f`)
- [ ] M-21 — durable unique-file media-root probe, verified fallback, and resolved Library/playback scope (`410a7da`)
- [ ] M-22 — latest-generation local snapshots, mutation invalidation, and caught event refresh failures (`9cebaf5`)
- [ ] M-23 — verified multithread protection at every caller-provided shared D3D11 boundary (`fe55590`)
- [ ] L-01 — borrowed/COM wave-format ownership and all-branch MFT output cleanup (`3c5d059`); event-handle overlap already closed by M-14
- [ ] L-03 — shared typed recorder-hotkey parsing, released orphan `KeyF` intent, and data-driven open-dialog keyboard ownership (`cc836fa`)
- [ ] L-04 — case-insensitive recording-suffix recovery, already closed by the stronger H-02 ownership batch (`234f6af`)
- [ ] L-05 — zero, duplicate, and file-range validation for proxy/direct multipart work lists (`b353966`)
- [ ] L-06 — unique owned poster temps, all-branch cleanup, and atomic stale-poster publication (`509e5cd`)
- [ ] L-07 — backend-only Cloud auth merge that preserves unrelated settings drafts and preferences (`4ad75ac`)
- [ ] L-08 — explicit normalized-origin consent before any plain-HTTP password request (`962ba5e`)
- [ ] L-09 — native-authorized media roots, exact-file asset scopes, and backend-enumerated local icon paths (`f80117b`)
- [ ] L-10 — numeric loopback normalization with proxy bypass and redirect refusal for League requests (`a49813e`)
- [ ] L-11 — fixed advisory versions, zero-ignore RustSec CI, immutable action pins, and automated update proposals (`a1b3e20`)
- [ ] L-12 — maintained Opus 1.6.1 binding, owned dependency exceptions, and expiring standalone WebView2 preflight (`706d329`)
- [ ] L-13 — immutable hashed FFmpeg archive, exact runtime allowlist, verified LGPL configuration, and bundled provenance (`2890d0a`)
- [ ] L-17 — opaque remote clip identity with native configured-origin page construction (`bdff7aa`)
- [ ] L-18 — validated own-property marker presentation and allowlisted CSS marker images (`bdff7aa`)
- [ ] L-19 — alignment-safe WASAPI decoding and validated, lifecycle-safe D3D readback (`bd2d617`)
- [ ] L-20 — generation-serialized microphone tests with disconnect shutdown and stale-event gating (`065c9a7`)
- [ ] L-21 — partial child-session Library scans with request-gated visible warnings (`5e69249`)
- [ ] L-22 — immutable catalog base with live extracted-icon overlays and detection refresh (`ea11121`)
- [ ] L-24 — bounded jittered direct-PUT backoff with Retry-After and terminal config fallback (`dd896dc`)
- [ ] L-25 — typed pre-recording clock-anchor rejection before event fetch/watermark mutation (`a4d2ad7`)
- [ ] L-26 — exact-first normalized Riot ID matching with missing-tagline compatibility (`2c40f15`)
- [ ] L-29 — in-process bounded diagnostic rotation with noisy window-event filtering (`d95568f`)
- [ ] L-33 — renderer capability manifest reduced to observed window/autostart operations (`bdff7aa`)

- [ ] L-30 — centralized credential, shell, disk-space, atomic-replace, UTF-16, and wall-clock helpers (`b26b88e`)

- [ ] L-31 — truthful process-image naming, corrected ownership comments, and typed rate-limited capture diagnostics (`e5c51c2`)

- [ ] L-32 — transition-only Cloud gallery rendering with live constant-size byte progress (`255a8a6`)

- [ ] L-16 — coalesced cached memory sampling on the blocking pool with hidden-window pause (`fb30ca0`)

- [ ] L-15 — shared replay planning/storage dispatch, typed capture timeouts, centralized MP4 parsing/writing, and single native folder-picker ownership (`621c6dc`)

- [ ] L-14 — named Rust domain owners, shared renderer presentation policy, and explicit ES-module bootstrap/core surface (`6c86a72`)

Recently hardened and requiring reconciliation against the combined labels before closure:

- [ ] None currently.

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

Acceptance run on 2026-07-19 completed every safely runnable scenario. Large trim exposed an
immediate-playback asset-scope regression and now needs one focused retest after
`2026-07-19-exported-clip-asset-scope.md`. Clipboard export passed for one selected track and a
multi-track mix with bounded memory and clean temporary artifacts; paste/audio and contention still
need designated helper applications. Cloud-page origin passed with two distinct opaque IDs and the
correct selected media. Media-root direct entry, drive-root, and profile-root rejection passed;
writable/unwritable fallback still needs disposable storage. A real Cloud account loaded all 17
cards and retained usable context menus; upload and cache-pressure scenarios remain.

A follow-up full-session stop exposed an audio packet straddling the first-video timeline origin.
The file-preservation path passed, and the deterministic regression is fixed under
`2026-07-19-full-session-audio-origin.md`. Retest one full-session start/stop with audio enabled:
the warning must not recur, the result must publish as `.mp4`, and it must play from the beginning.
Leave the previously preserved `.mp4.recording` untouched pending explicit recovery inspection.

A subsequent replay save exposed the equivalent straddling-packet condition at an arbitrary
selected GOP boundary. The deterministic regression and replay-only prefix filtering are recorded
under `2026-07-20-replay-audio-origin.md` (plan `47cd9cc`, implementation `c91d805`). Retest Save
Replay after capture has run longer than one GOP with audio enabled: the negative-origin warning
must not recur, the clip must appear in Library, and its audio must begin cleanly and stay synced.

A 989-second real full-session recording then showed both output and microphone stuttering into
permanent encoded silence despite structurally complete Opus tracks. The delayed-buffer lockout,
persistent recovery correction, and rate-limited diagnostic are recorded under
`2026-07-20-late-wasapi-audio-recovery.md` (plan `71e9977`, implementation `65f45ff`). Retest a
five-minute full session with both sources active near the beginning, middle, and end, save one
replay during the run, and confirm continuous synchronized audio. If
`wasapi_late_audio_reanchored` appears, confirm capture remains audible after it.

The 74-second follow-up session retained continuous Opus packets and decoded samples at its reported
two-second stutter, isolating that symptom to multi-track review playback rather than capture or
muxing. Smooth sidecar rate correction with hard seeks reserved for discontinuities is recorded
under `2026-07-20-review-audio-sidecar-drift.md` (plan `3abaf7c`, implementation `e7ca91e`). Retest
that session from the beginning with Output Audio and Microphone selected, play for at least one
minute, then seek and toggle track selections; no periodic skip or repeated fragment should occur.

The next 188-second session encoded a broadband Output Audio transient immediately after its first
packet began at 11.687 ms, coincident with the recorder's startup `wasapi_data_discontinuity` event.
A 40 ms PCM fade armed at startup and every WASAPI discontinuity is recorded under
`2026-07-20-wasapi-discontinuity-fade.md` (plan `475a5eb`, implementation `7920ad0`). Retest a newly
recorded full session from 0:00 several times with both sources selected; startup must be clean and
normal volume must follow immediately after the brief ramp.

The following 30-second replay began cleanly but contained repeated crackle. Continuous Opus packet
timelines and isolated deep 10 ms decoded-PCM holes, together with recurring
`wasapi_late_audio_reanchored` diagnostics, isolated a hard synthetic-silence-to-live recovery edge.
Live experiments with 30 ms and 60 ms normal-poll allowances left the recovery cadence unchanged,
because quiescent endpoints can stop delivering longer than any sensible fixed timeout. The final
path keeps 30 ms of active-delivery headroom, applies a five-millisecond fade only to recovered live
samples, and uses a separate three-frame terminal drain without synthetic tail silence. It is
recorded under `2026-07-20-wasapi-delivery-headroom.md` (initial plan/implementation
`1b13651`/`58109ac`; final plan/implementation `565954e`/`b029b80`). Retest a fresh replay of at least
30 seconds with both sources active throughout, then listen with both tracks selected and each track
alone; no periodic crackle, hard recovery edge, missing tail audio, or added tail silence should
occur.

That fresh replay's source tracks and generated review sidecars then produced identical encoded
packet SHA-256 hashes, with no packet gaps or GOP-boundary impulses, ruling out replay buffering and
sidecar extraction. The remaining throughout-the-clip artifact was isolated to WebView repeatedly
time-stretching two sidecars between 0.95x, 1.00x, and 1.05x on the 500 ms drift timer. Stable
requested-rate playback with the existing greater-than-500 ms emergency seek is recorded under
`2026-07-20-review-sidecar-rate-artifacts.md` (plan `814e4ee`, implementation `a85ceae`). Retest the
same replay with both tracks and each track alone, then seek and change playback speed; no continuous
crackle, repeated fragment, or gross desynchronization should occur.

- Elevated-game boundary: run a game as administrator while Clipline remains normal. Confirm the warning appears once for that process, recommends running the game without administrator privileges, contains no restart/UAC action, and ordinary Clipline recording remains unaffected after dismissal.
- Large trim: export a range from a multi-gigabyte/full-session clip. Confirm Clipline memory stays broadly flat, the source remains playable, no partial clip appears during export, and the completed trim plays through its end.
- Clipboard audio selection and contention: copy one clip with a single selected audio track and again with multiple tracks mixed. Paste each into another app; confirm video is intact, only the selected/mixed audio is audible, memory stays broadly flat, and no `.clipline-*-tmp` files remain after completion. Repeat once while a clipboard manager or another app holds the clipboard briefly and confirm Clipline retries then succeeds. Hold it longer than the retry window and confirm Clipline reports failure without claiming success; after releasing it, retry and paste the expected file normally.
- Large cloud upload: upload a large original clip and a selected-audio variant using a real account. While byte progress is active, scroll the Local and Cloud galleries and open a card context menu; confirm the deck percentage keeps advancing without cards flickering, scroll position jumping, the menu closing, or posters reloading, then confirm processing/failure/completion transitions appear immediately. Interrupt and retry a resumable upload; confirm memory remains bounded, the remote file plays, progress resumes correctly, and local media is deleted only when the configured policy and ready-media verification both permit it. If the test deployment can throttle direct object PUTs, return 429 or 503 with a short `Retry-After`; confirm attempts are spaced (not burst), honor at least that delay, remain cancelable during the wait, and succeed or report failure within the 30-second per-wait cap.
- Cloud cache pressure: with a real account, play several large remote clips until cache pressure triggers. Confirm cache data is under LocalAppData, the oldest unplayed entry is evicted, the clip currently playing remains available, total cache use returns under 10 GiB, and caching does not consume the final 2 GiB of free space.
- Replay-cache low space and restart: use disk replay buffering with full-session recording on a disposable/test volume, then reduce free space below the 2 GiB reserve. Confirm recording visibly stops, the full-session file finalizes or remains explicitly recoverable, no partial segment remains in the active run, and restarting Clipline removes stale owned runs without deleting another live Clipline instance's cache.
- Installed settings transaction: in an installed release build, change both recording hotkeys and `Open on startup`, save, restart/sign in, and confirm the hotkeys, tray label, and autostart behavior all match. Then make the settings folder temporarily unwritable and repeat a change; confirm the save reports failure and the prior hotkeys, tray label, autostart registration, and persisted settings all remain active. Restore folder permissions afterward.
- Credential transaction: using real Clipline Cloud and osu! credentials, connect, reconnect to a different account/user, disconnect, and restart. Before the Cloud reconnect, make an unsaved change on another Settings tab and to a Cloud preference; confirm both edits remain marked unsaved and retain their values after reconnect and disconnect, then save or discard them deliberately. Confirm `settings.json` never contains a secret, Windows Credential Manager retains only the current referenced targets, and any obsolete target that could not be removed is cleaned by the next status check/startup.
- Remote integration continuity: with real accounts and a League match, load the Cloud library/profile/avatar, run the osu! connection test and recent-play enrichment, and record League markers. Briefly interrupt networking or the local League endpoint during each flow; confirm commands fail within their documented timeout instead of hanging, retry succeeds after recovery, large uploads/downloads continue while making progress, and the Cloud library reports truncation if the server exposes more than 10,000 unique clips. Keep the League interruption under twenty seconds and confirm it neither splits the match/session nor duplicates earlier markers, then start another match after `GameEnd` and confirm its low event IDs are accepted into a new session.
- Cloud page origin: connect to a real deployment whose public frontend URL differs from its API host (and, if available, a private deployment that uses the host URL directly). From a Cloud card's context menu choose Open cloud page. Confirm the browser opens that configured frontend/host with the selected opaque clip ID as one path segment; repeat after selecting another card and confirm no stale or renderer-supplied origin is used.
- Windows capture lifecycle: configure split per-process output audio for a windowed app/game that can keep playing sound while its image is static. Record at least one minute of that idle visual, save a replay, and confirm the selected process audio is continuous with no dropouts. Start another recording of that window, close the target, and confirm recording stops promptly instead of extending a frozen last frame; then reopen the target and confirm a fresh recording starts normally.
- Delayed/gapped audio export: record four or more seconds from an app whose sound starts after the video, stops for at least one second, then resumes. Save a replay, export it with that audio track selected, and trim across both silent intervals. In each result, confirm playback begins silently, sound starts at the same visual moment as the source, the middle silence remains the same length, resumed sound stays synchronized, and no click or early packet is pulled to the trim boundary.
- Media-root authority and fallback: first type or paste a different absolute path into the media-folder field without using Choose Folder and confirm Save Settings rejects it without changing the prior root. Confirm choosing a drive root or your Windows profile root is also rejected. Then select a disposable folder, network share, or removable volume through the native picker while it is writable, save it, and revoke write permission or disconnect it before recording. Confirm Clipline warns and writes to the default `Videos\Clipline` folder, the saved clip immediately appears and plays in Library, and neither root contains a `.clipline-write-probe-*` file. Restore the configured root and confirm a new recording uses it again. Also select a currently unwritable folder in Settings and confirm Save Settings fails without changing the prior root. Restore permissions/connectivity afterward.
- Standalone browser-runtime release: on a release checkout, review the current official WebView2 Fixed Version runtime, update the runtime manifest and both Tauri paths if needed, stage the matching x64 payload, and run `.\scripts\verify-webview2-runtime.ps1 -RequirePayload`. Build/install the standalone variant and confirm it uses the bundled runtime, plays an H.264/Opus clip with system and microphone audio through its end, and enables HEVC/AV1 encoders only when its capability probes can play them. Publish to a disposable/test release channel and confirm its updater selects `latest-standalone.json` and remains on the standalone installer variant. Remove the test release afterward.
- FFmpeg release integrity: on the same release checkout, download the exact archive named by `apps/clipline-app/ffmpeg-runtime.json`, run `.\scripts\stage-ffmpeg-resource.ps1 -ArchivePath <archive>`, and retain the logged provenance. Build and install both regular and standalone variants. Confirm their FFmpeg resource contains only `README.md`, `LICENSE.txt`, `PROVENANCE.json`, `ffmpeg.exe`, and the seven manifest DLLs; compare the receipt's archive/version/configuration/file hashes with the committed manifest. In each variant, open Library so posters generate, probe the available FFmpeg GPU/SVT encoder tier, record a short clip with one available tier, and play it through. Finally, in a disposable install copy, replace the complete FFmpeg executable/DLL set with a compatible LGPL build and confirm Clipline discovers it; restore or uninstall the test copy afterward.
