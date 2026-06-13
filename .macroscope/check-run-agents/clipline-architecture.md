---
title: Clipline Architecture Guardrails
model: claude-opus-4-6
reasoning: medium
effort: medium
input: full_diff
tools:
  - browse_code
  - git_tools
  - github_api_read_only
include:
  - "crates/**"
  - "apps/clipline-app/**"
  - ".github/workflows/**"
  - "Cargo.toml"
  - "Cargo.lock"
  - "ddoc.md"
  - "handoff.md"
  - "docs/superpowers/plans/**"
exclude:
  - "target/**"
  - ".playwright-mcp/**"
conclusion: neutral
showToolCalls: true
waitsFor:
  - "*"
waitsForTimeout: 30
---

Review this PR for Clipline-specific architectural regressions. Use `ddoc.md` as the product and architecture source of truth, and `handoff.md` as the current implementation state.

Report only actionable issues that a maintainer should fix before merging. If no issues are found, say so briefly.

Check these guardrails:

- No DLL injection, process memory reading, kernel drivers, hidden telemetry, ads, account requirement, or cloud-only dependency.
- Capture remains WGC/DXGI-oriented and anti-cheat-safe.
- Media pipeline preserves one shared clock, stamp-derived PTS, audio gap fill, GOP-aligned replay saves, and finalized MP4 correctness.
- Event marker code uses local/official APIs or captured-frame data only, handles retries and duplicate events, and does not leak user data.
- Storage paths are validated under the configured media root and protect sidecars and the just-saved clip during GC.
- Tauri commands validate paths, keep service state coherent, and preserve non-Windows CI stubs.
- UI changes preserve the review player contract from `handoff.md`.
- New dependencies are compatible with `MIT OR Apache-2.0` first-party code and the FFmpeg LGPL dynamic-linking plan.
- Tests or docs are updated in proportion to the risk of the change.
