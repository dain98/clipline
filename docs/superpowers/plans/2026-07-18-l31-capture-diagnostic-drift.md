# L-31 Capture Diagnostic Drift Plan

**Goal:** Make Windows capture identity names, unsafe ownership comments, and runtime diagnostics
match their actual behavior without changing process matching or audio recovery semantics.

**Finding:** `CODEBASE_AUDIT_COMBINED.md` L-31.

## Design boundary

- [ ] Rename the ToolHelp snapshot's fallback executable field to `image_name`; keep
      `AudioProcessInfo.process_path` as the full path returned by `QueryFullProcessImageNameW`.
- [ ] Rename internal lookup helpers so a bare image name is never described as a full path, while
      retaining the existing case-insensitive basename/path comparison behavior.
- [ ] Add a typed capture-diagnostic event and one process-wide handler installed by the desktop
      application, routing capture events into the existing bounded diagnostic log.
- [ ] Rate-limit WASAPI data-discontinuity events per capture instance, retain a suppressed-event
      count, and do not change gap-fill or packet-release behavior.
- [ ] Correct every activation-blob safety comment to the actual `CoTaskMemAlloc` plus
      `PROPVARIANT`/`PropVariantClear` ownership path.
- [ ] Confirm the cited FFmpeg reader path no longer performs ad-hoc runtime printing; keep device
      test output outside the production-source contract.

## TDD sequence

- [ ] Add pure rate-limiter fixtures covering first emission, suppression inside the interval,
      accumulated count, and emission after the interval.
- [ ] Add typed diagnostic formatting/handler tests and an application contract proving the handler
      is installed before capture services can start.
- [ ] Add repository contracts that reject `process_path` in `ProcessSnapshotEntry`, stale
      `InitPropVariantFromBuffer` comments, and production `eprintln!` calls in WASAPI/FFmpeg paths.
- [ ] Implement the naming migration, diagnostic route, limiter, and safety-comment corrections.

## Verification

- [ ] Run focused capture diagnostic, WASAPI, application contract, and FFmpeg reader tests.
- [ ] Clean changed crates, then run warning-denied Clippy for all targets.
- [ ] Run CI-mode workspace tests and warning-denied workspace Clippy.
- [ ] Rebuild/open the native app and confirm the Library remains healthy. Real audio-device
      discontinuities are nondeterministic and need no new manual item because routing/limiting is
      covered by deterministic tests and the existing Windows capture lifecycle scenario.
- [ ] Update `handoff.md` and the combined remediation ledger.
