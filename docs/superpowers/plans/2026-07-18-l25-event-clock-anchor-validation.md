# L-25 Event Clock Anchor Validation Plan

**Goal:** Reject an event clock anchor sampled before recording start instead of silently mapping
it to a saturated zero wall offset and shifting markers.

**Finding:** `CODEBASE_AUDIT_COMBINED.md` L-25.

## Design boundary

- [ ] Make `recording_offset_s` fallible with a small typed clock-sync error for the violated
      `sampled_at >= recording_t0` invariant.
- [ ] Preserve intentionally negative offsets for events that occurred before recording; only the
      invalid wall-clock anchor relation is rejected.
- [ ] Validate the fresh League anchor before fetching/mutating the cumulative event tracker so an
      invalid anchor cannot consume the event watermark.
- [ ] Map the neutral clock error into the existing Live Client invalid-response boundary with a
      diagnostic message.

## TDD sequence

- [ ] Add a neutral sync test proving an earlier anchor returns the typed error rather than zero-
      based marker math.
- [ ] Update valid mapping tests to unwrap the fallible result while retaining exact values.
- [ ] Add a League poll integration test with a deliberately future recording start and verify the
      poll fails before event-data fetch/tracker mutation.
- [ ] Implement checked duration mapping and caller propagation.

## Verification

- [ ] Run focused events and League poll tests.
- [ ] Clean both changed crates, then run warning-denied Clippy for them.
- [ ] Run CI-mode workspace tests and warning-denied workspace Clippy.
- [ ] No app rebuild is required for a neutral latent invariant unless downstream relinking occurs
      during the workspace gates; keep the current tested app open afterward.
- [ ] Update `handoff.md` and the combined remediation ledger; no manual item is expected.
