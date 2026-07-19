# L-01 Windows Resource Ownership Plan

> **Finding:** L-01 — unsafe Windows resource ownership is not guarded consistently on error paths.

## Reconciliation

The WASAPI event-handle portion was already removed by M-14 when process loopback moved from an
unused event-callback contract to pull mode. No `CreateEvent`/`SetEventHandle` ownership remains in
the capture path. The borrowed-versus-COM-allocated mix format and Media Foundation output-field
cleanup findings remain live.

## Goal

Encode both remaining ownership distinctions in RAII so every early return frees only owned memory
and releases every COM output field exactly once.

## TDD sequence

- [ ] Add tests that require an explicit borrowed/COM-owned wave-format wrapper and verify the
  borrowed variant never reports ownership.
- [ ] Add a generic `ManuallyDrop<Option<T>>` take-and-clear test with drop spies proving moved
  values are not double-dropped and untouched values are released once.
- [ ] Run the focused tests and record the expected red compile failure.
- [ ] Replace the WASAPI boolean/raw-pointer pair with a wave-format RAII wrapper that frees only
  `GetMixFormat` allocations on every parse/initialize return path.
- [ ] Wrap `MFT_OUTPUT_DATA_BUFFER` so `pSample` and `pEvents` release on success, stream change,
  missing-sample, and arbitrary error returns; taking a sample must clear its field first.
- [ ] Run focused capture tests, fresh-cache capture Clippy, CI-mode workspace tests, and workspace
  Clippy with warnings denied.
- [ ] Rebuild and open Clipline for a native audio/capture startup smoke check.
- [ ] Update `handoff.md`, the audit ledger, and manual acceptance only if hardware-only behavior is
  not already covered by the Windows capture lifecycle item.

## Invariants

- [ ] Stack-owned fixed formats are never passed to `CoTaskMemFree`.
- [ ] COM-allocated mix formats are freed exactly once after `Initialize` or any earlier failure.
- [ ] Every `ProcessOutput` branch releases returned `pEvents` and any unconsumed `pSample`.
- [ ] A consumed MFT sample is cleared from the output field before the guard drops.
- [ ] No removed event-callback handle lifecycle is reintroduced.

## Commits

- `docs(plan): define L-01 Windows ownership remediation`
- `fix(capture): guard Windows resource ownership`
- `docs(audit): close Windows ownership finding`
