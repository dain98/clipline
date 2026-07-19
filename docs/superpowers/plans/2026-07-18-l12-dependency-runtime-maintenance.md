# L-12 Dependency and Fixed-Runtime Maintenance Plan

> **Finding:** L-12 — the unmaintained Opus sys binding, duplicated HTTP stacks, and a pinned
> standalone browser runtime lack a deliberate update policy.

## Goal

Move Clipline off `audiopus_sys` 0.1, keep native Opus encode/decode compatibility covered, make
the presently unavoidable `reqwest` major split an owned and expiring exception, and prevent a
standalone release from silently shipping a stale or mismatched WebView2 Fixed Version runtime.

## TDD sequence

- [ ] Extend the repository-security contract to reject `audiopus_sys` 0.1, require every allowed
  duplicate major to name its owner, rationale, review date, and removal condition, and require a
  WebView2 runtime manifest whose version matches both standalone Tauri paths; run it red.
- [ ] Upgrade all first-party Opus users together to a maintained binding and retain the
  existing Opus encode/decode, MP4 mux, mix, trim, and capture regression coverage.
- [ ] Record the two `reqwest` majors as a narrow temporary exception: first-party and the pinned
  cloud API stay on 0.12 while Tauri's updater owns 0.13, with a quarterly review and a concrete
  removal trigger when those upstreams converge.
- [ ] Add a machine-readable WebView2 Fixed Version manifest and a verification script that rejects
  config/version drift, an overdue review, and a missing staged payload when release mode requests
  payload verification.
- [ ] Document a refresh check for every standalone release and at least every 30 days, including
  the official source, review procedure, staged-directory verification, and media regression gate.
- [ ] Run the focused structural contract and script tests, Opus/MP4/app tests, a local RustSec audit,
  fresh-cache Clippy for changed crates, then CI-mode workspace tests and workspace Clippy.
- [ ] Rebuild and open Clipline because the Opus binding is runtime media code, smoke the native app,
  and update `handoff.md`, the master ledger, and the final manual acceptance checklist.

## Invariants

- [ ] No selected package depends on unmaintained `audiopus_sys` 0.1.x.
- [ ] One reviewed Opus binding is selected everywhere and its encode/decode API remains
  compatible with Clipline's 48 kHz stereo Opus-in-MP4 pipeline.
- [ ] Duplicate dependency majors are never silent: each has an owner, rationale, review deadline,
  upstream/removal condition, and only the exact allowed major versions.
- [ ] The standalone resource glob, `webviewInstallMode.path`, and runtime manifest name the same
  architecture and exact WebView2 runtime version.
- [ ] A standalone release cannot pass its documented preflight with an overdue runtime review or
  without the expected staged Fixed Version runtime directory.
- [ ] Every runtime refresh includes H.264/Opus playback plus HEVC/AV1 capability-probe regression
  testing before publication.

## Commits

- `docs(plan): define L-12 dependency maintenance gates`
- `fix(deps): retire unmaintained Opus binding`
- `docs(audit): close dependency runtime policy finding`
