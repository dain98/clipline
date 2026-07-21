# L-08 Explicit Plain HTTP Consent Plan

> **Finding:** L-08 — the Cloud connection request infers password-transmission consent from an HTTP URL instead of obtaining it explicitly.

## Goal

Require an affirmative acknowledgment before sending Cloud credentials over an allowed plain-HTTP
origin, bind that acknowledgment to the normalized origin, and invalidate it when the origin changes.

## TDD sequence

- [ ] Add pure CloudCore regressions for normalized HTTP origins and exact-origin consent matching.
- [ ] Replace the existing UI contract that permits automatic confirmation with one requiring an
  explicit checkbox, a pre-request guard, and an origin-bound backend flag.
- [ ] Run focused tests and record the expected failures.
- [ ] Add an accessible plain-HTTP acknowledgment next to the credential fields.
- [ ] Reset the acknowledgment when the normalized origin changes, while allowing path-only edits
  on the same origin to retain the current acknowledgment.
- [ ] Block `cloud_connect` until the active HTTP origin is acknowledged and send `false` for HTTPS.
- [ ] Run focused tests, fresh-cache app Clippy, CI-mode workspace tests, and workspace Clippy with
  warnings denied.
- [ ] Rebuild/open Clipline and smoke the consent/reset behavior in the Cloud settings pane.
- [ ] Update `handoff.md`, the combined audit ledger, and the final manual acceptance checklist.

## Invariants

- [ ] Merely entering an `http://` URL never sets `plain_http_confirmed`.
- [ ] No password-bearing request is invoked before explicit acknowledgment.
- [ ] Consent is valid only for the normalized HTTP origin shown to the user.
- [ ] Changing scheme, host, or effective port clears consent; changing only the path does not.
- [ ] HTTPS connections remain frictionless and report no plain-HTTP confirmation.
- [ ] Backend host validation remains authoritative for which plain-HTTP hosts are allowed.

## Commits

- `docs(plan): define L-08 HTTP consent`
- `fix(cloud): require explicit plain HTTP consent`
- `docs(audit): close inferred HTTP consent finding`
