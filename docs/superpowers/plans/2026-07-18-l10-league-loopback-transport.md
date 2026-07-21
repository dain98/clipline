# L-10 League Loopback Transport Plan

> **Finding:** L-10 — League loopback trust is not enforced across names, redirects, and proxies.

## Goal

Keep every League Live Client request on an explicit numeric loopback endpoint from URL validation
through transport, with redirects and environment/system proxies disabled before accepting Riot's
self-signed certificate.

## TDD sequence

- [ ] Add base-normalization tests proving numeric IPv4/IPv6 loopback endpoints are retained,
  `localhost` is rewritten to `127.0.0.1`, and remote or malformed hosts are rejected.
- [ ] Add an HTTP test whose loopback response redirects to a second server; prove the second
  server is never contacted.
- [ ] Add a deterministic construction contract for proxy bypass and redirect refusal; run the
  focused League suite red.
- [ ] Store a parsed, normalized base URL instead of renderer/configuration text and join fixed Live
  Client endpoint paths through URL semantics.
- [ ] Build the request client with no proxy and no redirects before enabling invalid certificates
  for the already-normalized numeric loopback destination.
- [ ] Run focused League tests, fresh-cache League Clippy, CI-mode workspace tests, and workspace
  Clippy with warnings denied.
- [ ] Rebuild/open Clipline and verify normal startup/Library behavior; retain real-match endpoint
  continuity in the final manual acceptance checklist.
- [ ] Update `handoff.md` and the combined audit ledger.

## Invariants

- [ ] Certificate verification is disabled only for an HTTP(S) numeric loopback base URL.
- [ ] `localhost` never reaches DNS or hosts-file resolution.
- [ ] League requests never use configured environment/system proxies.
- [ ] League requests never follow redirects, including redirects to another loopback listener.
- [ ] Fixed endpoint joins cannot inherit base credentials, query strings, fragments, or path tricks.
- [ ] Existing bounded-body and timeout protections remain unchanged.

## Commits

- `docs(plan): define L-10 League transport boundary`
- `fix(security): pin League requests to loopback`
- `docs(audit): close League loopback transport finding`
