# M-01 Upload Control Origin Plan

**Goal:** Ensure the Clipline Cloud bearer token is sent only to the configured cloud origin and never follows a server-directed redirect.

## Authenticated URL boundary

- [ ] Add tests for relative and same-origin absolute upload URLs.
- [ ] Add tests rejecting cross-origin, port-changing, and HTTPS-to-HTTP upload-control URLs before a request is sent.
- [ ] Apply the same-origin rule to direct presign/ack templates and authenticated single-PUT URLs, while leaving token-free presigned object-storage PUT URLs cross-origin capable.

## Redirect boundary

- [ ] Build the authenticated upload HTTP client with redirects disabled.
- [ ] Add an integration test proving a create-upload redirect is not followed and the bearer-authenticated request never reaches the redirect target.

## Verification

- [ ] Run focused cloud-upload tests and fresh-cache app Clippy.
- [ ] Run CI-mode workspace tests and workspace Clippy with warnings denied.
- [ ] Rebuild/open Clipline for a native smoke test.
- [ ] Update `handoff.md`, the master audit ledger, and the manual acceptance checklist if deployment-specific verification remains.
