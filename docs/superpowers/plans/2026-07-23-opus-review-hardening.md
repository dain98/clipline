# Opus Review Hardening

## Goal

Resolve the actionable findings from the first independent Opus 4.8 review of
PR #104 without broadening the private diagnostic-reporting design.

## Test-Driven Steps

1. Add failing exporter tests for Bearer authorization values and quoted JSON
   credential values.
2. Add a deterministic cancellation test that requests cancellation before the
   upload waiter is registered.
3. Add bundle tests proving every JSON entry passes through the generic
   redactor and failed bundle construction removes staging artifacts.
4. Add UI contract tests for settings-tab semantics, selected state, panel
   relationships, and phase-change focus.
5. Implement persistent upload cancellation, broader secret redaction,
   allowlisted JSON redaction, failure cleanup, and accessible settings tabs.
6. Replace duplicated logger-size arithmetic with diagnostics constants exposed
   through a neutral helper.
7. Run focused tests, `cargo test --workspace`, and warning-denied workspace
   Clippy.
8. Commit and push the fixes, then ask the same Opus 4.8 session to re-review
   the updated diff. Repeat until it reports no actionable findings or three
   review rounds have completed.

## Constraints

- Keep report submission anonymous, explicit, private, and directed only to the
  immutable official endpoint.
- Do not include recordings, raw settings, credentials, identities, filenames,
  or directory listings.
- Preserve the lossy/non-blocking logging path and current bundle allowlist.
- Keep the unrelated untracked `paseo.json` out of every commit.
