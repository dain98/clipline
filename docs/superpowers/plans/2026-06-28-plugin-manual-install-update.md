# Plugin Manual Install And Update

## Goal

Add a manual first-party install/update path for the League package without opening arbitrary third-party plugin execution.

## Scope

- [ ] Add known first-party package actions: check/update/reinstall/reset-to-seed.
- [ ] Reject arbitrary URL installs.
- [ ] Verify package authenticity with an app-pinned public key or expected digest.
- [ ] Treat GitHub-side checksums as corruption-only checks.
- [ ] Reject unsupported schema versions and unknown required capability names.
- [ ] Extract package zips through a staging directory with zip-slip protection and atomic activation.

## TDD Steps

- [ ] Test bad signature, wrong key, and corrupt zip paths.
- [ ] Test zip-slip entries are rejected before extraction.
- [ ] Test malformed manifest, unsupported schema version, and unknown event-source capability rejection.
- [ ] Test failed install rollback leaves active package unchanged.
- [ ] Test manual update/reinstall/reset UI states for the first-party League package.

## Verification

- [ ] `cargo test -p clipline-app`
- [ ] `cargo test --workspace`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`

