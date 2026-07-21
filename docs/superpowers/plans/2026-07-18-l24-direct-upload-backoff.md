# L-24 Direct Upload Backoff Plan

**Goal:** Prevent burst retries for direct object-storage PUT failures while keeping recovery
bounded, respecting server throttling guidance, and remaining cancellation-responsive.

**Finding:** `CODEBASE_AUDIT_COMBINED.md` L-24.

## Design boundary

- [ ] Wait only between retryable failed PUT attempts; successful, fallback, terminal, presign, and
      acknowledgement paths keep their current behavior.
- [ ] Use bounded exponential delays with deterministic per-upload/part/attempt jitter so clients do
      not synchronize retry bursts and tests remain reproducible.
- [ ] Parse `Retry-After` delta seconds and HTTP dates, use it as a minimum when larger than local
      backoff, and cap hostile/unreasonable values at a documented foreground limit.
- [ ] Classify request-builder/invalid-URL errors as immediate provider fallback while retaining
      timeout/connect/request/body transport failures as retryable.
- [ ] Await Tokio timers directly; dropping/aborting the upload future cancels the wait immediately.

## TDD sequence

- [ ] Add pure delay tests for exponential growth, deterministic jitter, Retry-After minimums, and
      the maximum bound.
- [ ] Add parser tests for delta seconds, a future HTTP date, expired dates, and malformed values.
- [ ] Preserve existing direct PUT expiry/retry and provider-fallback integration tests.
- [ ] Implement structured retry metadata, transport classification, and the inter-attempt wait.

## Verification

- [ ] Run focused Cloud upload retry tests.
- [ ] Clean `clipline-app`, then run warning-denied app Clippy.
- [ ] Run CI-mode workspace tests and warning-denied workspace Clippy.
- [ ] Rebuild and open the exact workspace app; verify the ordinary local Library because real Cloud
      credentials/object storage are outside deterministic scope.
- [ ] Update `handoff.md`, the combined ledger, and extend the existing real Cloud upload acceptance
      scenario with throttled/retry timing rather than adding a duplicate manual item.
