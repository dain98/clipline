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

Recently hardened and requiring reconciliation against the combined labels before closure:

- [ ] L-02 — MP4 scalar/configuration boundaries
- [ ] L-27 — HEVC layer-count representation
- [ ] L-28 — public segment sample slicing

## Phase 1: remaining high severity

- [ ] H-01 — completed by removing the full-application elevation boundary (`5d06c21`).
- [ ] H-05 — eliminate whole-file/multi-copy behavior from upload, remux/mix, clipboard audio export, and trim paths; impose explicit safe limits where streaming cannot land atomically.

## Phase 2: medium security, persistence, and lifecycle

- [ ] M-01 through M-09, excluding findings already subsumed by a stronger completed fix.
- [ ] M-11 through M-18.
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
