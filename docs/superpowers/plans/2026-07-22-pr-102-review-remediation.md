# PR 102 Review Remediation

> **For agentic workers:** Execute this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for
> tracking and remain unticked by repository convention.

**Goal:** Close the remaining cross-GOP decode-time drift and make pathological finite video
timestamps degrade without terminating capture or discarding pending media.

## Task 1: Reproduce the review findings

- [ ] Add a deterministic multi-segment regression where repeated one-tick rounding ties would
      leave the MP4 writer frontier ahead of later absolute segment starts.
- [ ] Assert a tolerated boundary uses the writer's actual frontier when quantizing the fragment.
- [ ] Add coverage proving a failed seal leaves pending video and audio untouched.
- [ ] Add cases for crowded, slightly backward, and repeatedly jittering finite timestamps.

## Task 2: Anchor fragments and preserve pending state

- [ ] Return the writer's effective per-track decode frontier after applying absolute timestamps.
- [ ] Quantize each fragment from those effective frontiers so a tolerated rounding tie cannot
      accumulate across GOPs.
- [ ] Compute sealed video durations before taking pending packets or draining pending audio.
- [ ] Keep the MP4 writer strict for genuine backward decode-time movement.

## Task 3: Degrade pathological finite timestamps safely

- [ ] Validate the video timescale once and remove the contradictory zero-timescale fallback.
- [ ] For a finite boundary too short to represent every encoded dependency with a positive MP4
      duration, retain every packet and minimally extend the effective span instead of failing the
      session.
- [ ] Treat local sub-tick PTS regressions as packet-selection pressure rather than accumulating
      them into a session-fatal error.
- [ ] Consolidate the duplicate PTS-remapping encoder fixtures.

## Task 4: Verify and update the draft PR

- [ ] Run focused capture regressions, the workspace tests, and fresh-cache warning-denied Clippy.
- [ ] Obtain an independent review of the final diff.
- [ ] Update `handoff.md`, commit the remediation, and push PR #102.
- [ ] Rebuild and launch `clipline-app` for a full-session VRR/manual retest.
