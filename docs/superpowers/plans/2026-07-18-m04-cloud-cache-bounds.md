# M-04 Cloud Cache Bounds Plan

**Goal:** Keep downloaded cloud media bounded on disk without racing active downloads or evicting media that WebView playback is currently using.

## Location and migration

- [ ] Add a LocalAppData cache base distinct from roaming settings storage and move new cloud-cache data there.
- [ ] On first use, migrate only the known legacy `cloud-cache` tree. Never recurse through directory links/reparse points and never delete unrelated roaming data.
- [ ] Keep asset-protocol scope validation anchored to the new canonical root.

## Ownership, leases, and concurrency

- [ ] Give every in-flight download a unique owned temporary path and RAII cleanup behavior.
- [ ] Prune only Clipline-named temporary files older than a stale threshold; never delete every `.tmp` file and never traverse linked directories.
- [ ] Serialize cache accounting/publication and maintain process-local leases for in-flight and recently returned playback paths so eviction cannot invalidate an active response.
- [ ] Refresh access recency on cache hits and completed downloads.

## Resource bounds

- [ ] Clamp untrusted server size hints to a documented hard per-file ceiling.
- [ ] Enforce an aggregate LRU quota and a minimum free-space reserve before and after download publication.
- [ ] Count marker sidecars in accounting and remove them with their media entry.
- [ ] If enough unleased data cannot be evicted, fail the download clearly while preserving completed cache entries and the active temp owner.

## Tests and verification

- [ ] Add deterministic tests for the hard per-file cap, LRU ordering, aggregate quota, free-space pressure, active leases, stale-vs-active temps, marker pairing, linked-directory refusal, and legacy migration boundaries.
- [ ] Run focused cloud tests, fresh-cache app Clippy, CI-mode workspace tests, and workspace Clippy with warnings denied.
- [ ] Rebuild/open Clipline and verify local/cloud library rendering; do not download user media during automation.
- [ ] Update `handoff.md`, the master audit ledger, and the accumulated manual acceptance checklist.

## Manual acceptance additions

- With a real cloud account, play several large remote clips until the cache crosses its quota. Confirm the oldest unplayed item is evicted, the clip currently playing never disappears, cache storage is under LocalAppData, and available disk space never drops below the reserve solely because of caching.
