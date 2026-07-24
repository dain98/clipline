# Official Support Endpoint

## Goal

Configure every Clipline desktop build and release to submit private diagnostic reports only to
`https://support.dain.cafe/api/v1/reports`.

## Implementation

- [ ] Add failing tests proving the configured value is the complete report URL and is never
  extended with another `/api/v1/reports` suffix.
- [ ] Make the build script inject the exact official HTTPS URL for debug and release profiles.
- [ ] Reject build-time attempts to substitute a different diagnostic destination.
- [ ] Parse and use the injected URL directly in the desktop client.
- [ ] Update release workflow documentation to use the literal official URL instead of a mutable
  repository variable.

## Verification

- [ ] Verify the deployed health/readiness endpoints and POST-only intake route.
- [ ] Run focused endpoint and Support UI tests.
- [ ] Run `cargo test --workspace`.
- [ ] Run `cargo clippy --workspace --all-targets -- -D warnings`.
- [ ] Rebuild Clipline and confirm the Support UI reports private upload as available.
