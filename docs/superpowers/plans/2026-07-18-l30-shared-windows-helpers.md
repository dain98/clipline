# L-30 Shared Windows Helpers Plan

**Goal:** Reduce duplicated unsafe Win32 ownership and conversion code to one reviewable set of safe
application wrappers without changing credential targets or user-visible behavior.

**Finding:** `CODEBASE_AUDIT_COMBINED.md` L-30.

## Design boundary

- [ ] Convert the existing application `windows` module into a directory module and keep every new
      Win32 call plus RAII owner below that boundary.
- [ ] Add one `CredentialStore` wrapper for generic credential write/read/delete, with one
      `CredFree` owner, null/blob-length validation, UTF-8 validation, missing-delete semantics,
      and caller-specific labels that preserve existing diagnostics.
- [ ] Add one shell-open wrapper and one disk-free-space wrapper; Cloud, osu!, and recorder modules
      supply only targets and user-facing context.
- [ ] Move shared wide-string and Win32 last-error formatting under the Windows boundary.
- [ ] Keep one neutral unsigned Unix-seconds clock plus an explicitly checked/saturating signed
      adapter in `util`; remove app, service, osu-enrichment, and media-name duplicates.
- [ ] Preserve the exact existing Cloud and osu! credential-target formats to avoid orphaning
      installed secrets.

## TDD sequence

- [ ] Add repository contracts proving credential FFI/`CredFree`, `ShellExecuteW`, and
      `GetDiskFreeSpaceExW` appear only below `src/windows/`.
- [ ] Add credential decoding fixtures for empty blobs, nonempty null blobs, valid UTF-8, and
      invalid UTF-8, plus diagnostic-label preservation.
- [ ] Add shell-result classification, wide-string termination, and Unix conversion boundary tests.
- [ ] Implement the shared wrappers, migrate both integrations and the recorder, then delete every
      duplicated unsafe helper and clock function.

## Verification

- [ ] Run focused Windows-helper, repository-security, Cloud, osu!, recorder, and utility tests.
- [ ] Clean `clipline-app`, then run warning-denied Clippy for all app targets.
- [ ] Run CI-mode workspace tests and warning-denied workspace Clippy.
- [ ] Rebuild/open the native app; use Computer Use to open the generated osu! setup guide and a
      configured Cloud page only if the installed state provides a safe non-authenticated target.
- [ ] Update `handoff.md` and the combined remediation ledger. Existing real credential transaction
      acceptance remains the manual coverage for Windows Credential Manager; do not duplicate it.
