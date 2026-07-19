# M-05 Bounded HTTP Operations Plan

**Goal:** Ensure every Cloud, osu!, upload, download, and local League HTTP operation has explicit connection/progress/body/pagination bounds without imposing a short fixed deadline on large media transfers.

## Shared desktop HTTP boundaries

- [ ] Add configured control and streaming clients with connect and read-idle deadlines; authenticated Cloud clients must retain redirects-disabled behavior.
- [ ] Add a bounded streaming body reader that rejects oversized `Content-Length` values before reading and enforces the same limit for chunked/deceptive responses.
- [ ] Parse JSON and error bodies only through the bounded reader (4 MiB control JSON, 64 KiB diagnostic text) and cover exact-limit, chunked-over-limit, and advertised-over-limit cases.
- [ ] Reuse configured clients per operation/session rather than creating fresh defaults at each request.

## Cloud control, assets, and pagination

- [ ] Replace unbounded Cloud API reads used by connect, identity, clip status/listing, visibility, and upload control with bounded same-origin requests.
- [ ] Stream avatar bytes through the existing 2 MiB definitive limit and retain the stricter image content-type checks.
- [ ] Keep media/thumbnail streaming size limits, add connect/read-idle deadlines, and bound error bodies.
- [ ] Cap cloud library enumeration at 100 pages / 10,000 unique clips, deduplicate remote ids, and return a visible truncation indicator rather than looping forever.

## Upload transport

- [ ] Apply configured clients to authenticated proxy/direct-control and token-free object PUT requests.
- [ ] Bound every control response/error body and JSON decode.
- [ ] Give media requests a size-aware minimum-throughput deadline so multi-gigabyte uploads remain practical while a stalled body cannot remain pending forever.
- [ ] Keep cancellation/drop behavior and existing direct-S3 same-origin/token separation intact.

## osu! and League

- [ ] Reuse one configured osu! client across token, identity, and recent-score requests; bound all success/error JSON bodies and retain the existing score-count ceiling.
- [ ] Add connect/read timeouts and a bounded JSON reader to the loopback League client so an arbitrary local listener cannot return an unbounded body.
- [ ] Add local mock tests for stalled/deceptive/oversized bodies and preserve normal endpoint fixtures.

## Verification

- [ ] Run focused Cloud/upload/osu!/League tests and fresh-cache Clippy for both changed crates.
- [ ] Run CI-mode workspace tests and workspace Clippy with warnings denied.
- [ ] Rebuild/open Clipline for a native smoke test.
- [ ] Update `handoff.md`, the master audit ledger, and the real-account upload/download manual acceptance checks.
