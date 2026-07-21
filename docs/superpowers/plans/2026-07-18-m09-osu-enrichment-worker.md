# M-09 osu! Enrichment Worker Plan

**Goal:** Make enrichment retries one crash-safe, rate-limited queue per media root so overlapping UI/startup/save triggers cannot race and one damaged record cannot stop healthy work.

## Single-flight scheduling

- [ ] Normalize every trigger to the configured media root and acquire a process-wide per-root lease before discovery or network work.
- [ ] Coalesce overlapping passes for the same canonical root without blocking unrelated media roots.
- [ ] Filter jobs by persisted status, attempt count, and sidecar modification time before fetching scores: new jobs run immediately, pending retries use capped exponential backoff, and failed legacy jobs periodically re-enter with a longer capped backoff.
- [ ] Fetch the recent-score window only for eligible jobs and keep completion refresh events deduplicated at the pass boundary.

## Durable queue files

- [ ] Publish new, retry, failed, and marker JSON via unique same-directory temporary files, file sync, and replace-existing rename; clean every owned temporary on failure.
- [ ] Never discover temporary/quarantine names as queue jobs.
- [ ] Isolate each unreadable, malformed, mismatched, or otherwise invalid pending sidecar: log its exact path and reason, move it to a unique quarantine sibling when possible, and continue discovering valid jobs.

## Tests and verification

- [ ] Add deterministic tests for per-root lease coalescing/release, independent roots, retry/failed due times and caps, and trigger root normalization.
- [ ] Add mixed valid/malformed discovery tests proving healthy work continues and quarantine is not rediscovered.
- [ ] Add atomic replacement tests proving complete JSON publication and owned-temp cleanup.
- [ ] Run focused app tests, fresh-cache app Clippy, CI-mode workspace tests, and workspace Clippy with warnings denied.
- [ ] Rebuild/open Clipline and update `handoff.md` and the master audit ledger; no manual-only acceptance should remain for the deterministic worker guarantees.
