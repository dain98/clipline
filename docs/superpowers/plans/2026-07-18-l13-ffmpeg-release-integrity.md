# L-13 FFmpeg Release Integrity Plan

> **Finding:** L-13 — release staging trusts an arbitrary directory and copies every entry into a
> signed application resource.

## Goal

Make the FFmpeg release input immutable and reviewable, stage only the executable/runtime/license
files Clipline needs, reject GPL/nonfree or unexpected builds, and preserve verified provenance in
both release logs and the bundled resource without weakening LGPL replacement rights.

## TDD sequence

- [ ] Add a repository-security contract requiring an immutable non-`latest` FFmpeg release URL,
  exact archive SHA-256, expected version/configuration, an explicit unique file allowlist, a
  retained license, and structural staging-script integrity checks; run it red.
- [ ] Pin the retained BtbN monthly FFmpeg 8.1 x64 LGPL-shared archive selected from its official
  GitHub release, including exact asset name, release timestamp, digest, root, and runtime files.
- [ ] Replace directory staging with archive staging: reject links/non-files, hash before opening,
  select exact zip entries only, verify every staged hash, and validate `ffmpeg -version` against
  the pinned version plus shared/LGPL configuration contract.
- [ ] Build the complete resource in an owned temporary directory, retain the tracked release
  README, emit a deterministic `PROVENANCE.json`, and publish only after all verification passes.
- [ ] Test the real pinned archive, a corrupted/tiny substitute, unexpected destination residue,
  the executable version/configuration, the final exact filename set, and provenance hashes.
- [ ] Update release instructions and third-party notices with the immutable download/rotation
  workflow and source-offer/license obligations.
- [ ] Run focused contracts and script tests, CI-mode workspace tests, and warning-denied workspace
  Clippy; no native app rebuild is needed because only release staging metadata/scripts change.
- [ ] Update `handoff.md`, the master ledger, and the final manual release acceptance checklist.

## Invariants

- [ ] No unverified archive bytes are opened, extracted, executed, or copied into app resources.
- [ ] The release URL contains an immutable retained tag and the expected archive name; neither may
  use BtbN's floating `latest` alias.
- [ ] The zip can contain arbitrary other files, but only exact manifest entries reach staging.
- [ ] `ffplay.exe`, `ffprobe.exe`, headers, import libraries, presets, and unexpected DLLs never
  enter the signed application resource.
- [ ] The selected FFmpeg is x64, shared, version3/LGPL-compatible, and has no GPL/nonfree enable
  flags or GPL-only x264/x265 libraries.
- [ ] License and deterministic provenance files ship beside the independently replaceable FFmpeg
  executable and DLLs.
- [ ] Verification completes before the current staged resource is replaced.

## Commits

- `docs(plan): define L-13 FFmpeg staging boundary`
- `fix(release): verify and allowlist FFmpeg staging`
- `docs(audit): close FFmpeg release integrity finding`
