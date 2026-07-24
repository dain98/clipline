# GOP Boundary Quantization Fix

> **For agentic workers:** Execute this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for
> tracking and remain unticked by repository convention.

**Goal:** Prevent multiple sub-timescale video intervals in one GOP from accumulating into a
multi-tick overlap with the next GOP's absolute decode timestamp.

## Task 1: Reproduce the remaining Nightly 0.1.39 failure

- [ ] Add a deterministic full-session encoder fixture containing two positive sub-one-tick PTS
      intervals in one GOP.
- [ ] Assert the session finalizes and the sealed GOP still lands on the following keyframe
      boundary.
- [ ] Run the focused regression first and confirm it fails with a two-tick backward decode-time
      error on the current implementation.

## Task 2: Quantize sealed GOPs against their boundary

- [ ] Quantize finite GOP sample boundaries cumulatively in the configured video timescale.
- [ ] Preserve nonzero MP4 sample durations while reserving enough ticks for every remaining
      sample.
- [ ] Make the final sample land on the sealing keyframe boundary instead of accumulating
      per-interval minimum-duration inflation.
- [ ] Keep the final unbounded seal behavior and the writer's one-tick inter-segment rounding
      tolerance unchanged.
- [ ] Retain the existing seven-tick regression and larger-regression rejection coverage.

## Task 3: Verify and hand off

- [ ] Run the focused capture tests and full workspace test suite.
- [ ] Run fresh-cache warning-denied Clippy for the changed capture crate and workspace.
- [ ] Update `handoff.md` with the remaining 0.1.39 cause and boundary-constrained fix.
- [ ] Commit the implementation as one conventional logical change.
- [ ] Rebuild and launch `clipline-app` for a full-session manual retest.
