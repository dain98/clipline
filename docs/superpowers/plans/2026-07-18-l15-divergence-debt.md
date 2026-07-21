# L-15 Divergence Debt Plan

**Goal:** Replace the audit's remaining duplicated/stringly maintenance paths with shared typed
helpers while preserving replay eviction, byte-for-byte MP4 output, and picker behavior.

**Finding:** `CODEBASE_AUDIT_COMBINED.md` L-15. Whole-file trim/resource concerns were already
closed by H-05/M-16; parser boundary hardening was already closed by M-17/L-02. This batch closes
the independent quality root causes that remain in current code.

## Design boundary

- [ ] Remove the module-wide `dead_code` allowance from game discovery and delete or narrowly mark
      only genuinely platform/test-specific items identified by warning-denied builds.
- [ ] Share one blocking native folder-picker helper; media authorization/normalization remains a
      caller-specific postcondition and replay-cache selection remains non-authorizing.
- [ ] Make process-loopback activation timeout a typed `CaptureError::Timeout` and remove service
      message parsing without changing mixed-output fallback behavior.
- [ ] Centralize `ReplayStorage` length/bytes/span/window/load/push dispatch, and share memory/disk
      window selection plus eviction-count planning in `clipline-buffer`.
- [ ] Share checked MP4 header decoding between in-memory walking and streaming trim readers while
      retaining walk's best-effort stop and trim's precise corruption errors.
- [ ] Build one checked fragment metadata plan and one state-commit path for owned, borrowed,
      single-source, and per-track-source writer transports; transport closures perform only payload
      I/O. Preserve existing serialized bytes for every variant.
- [ ] Remove the FFmpeg codec no-op and add structural contracts against every drift signal.

## TDD sequence

- [ ] Add typed activation-timeout and service-fallback fixtures that reject string classification.
- [ ] Add shared replay selection/eviction fixtures exercised through both memory and disk rings.
- [ ] Add cross-parser normal/large/zero/truncated/overflow header parity fixtures.
- [ ] Add byte-for-byte fragment parity fixtures for all four transports, including gaps, empty
      tracks, oversized/zero duration errors, and multiple sources.
- [ ] Add repository contracts rejecting blanket dead-code, duplicated picker construction,
      timeout substring matching, duplicated box size decoding, and `let _ = codec`.
- [ ] Implement helpers incrementally, running focused tests after each boundary.

## Verification

- [ ] Run game discovery/app, service/audio, buffer, capture, and MP4 focused suites.
- [ ] Clean every changed crate and run warning-denied Clippy for all targets.
- [ ] Run CI-mode workspace tests and warning-denied workspace Clippy.
- [ ] Rebuild/open the app and verify Library/recording health. Existing media-root and Windows
      capture lifecycle acceptance scenarios cover the two hardware/native picker boundaries, so
      no duplicate manual-only item is needed.
- [ ] Update `handoff.md` and the combined remediation ledger.
