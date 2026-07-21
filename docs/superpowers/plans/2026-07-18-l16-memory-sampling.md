# L-16 Memory Sampling Plan

**Goal:** Keep the exact private-resident process-tree metric without running its expensive Windows
address-space walk synchronously or redundantly on the Tauri command thread.

**Finding:** `CODEBASE_AUDIT_COMBINED.md` L-16.

## Design boundary

- [ ] Preserve `private_working_set_bytes` semantics and the existing child-process/conhost rules.
- [ ] Add an application-managed sampler with one async mutex and a short monotonic cache window;
      concurrent callers coalesce behind the same in-flight blocking measurement.
- [ ] Make `memory_status` async and run the Windows walk through Tauri's blocking pool.
- [ ] Return cloned success or failure results from the cache so a failing query cannot create a
      two-second retry storm.
- [ ] Skip periodic renderer invokes while the document is hidden and refresh immediately when it
      becomes visible again.

## TDD sequence

- [ ] Add cache fixtures for empty, fresh success, fresh failure, and expired samples.
- [ ] Add a concurrent sampler fixture proving multiple simultaneous callers execute one injected
      measurement and all receive the same result.
- [ ] Update the UI contract to require an async blocking command, managed coalescer, and
      visibility-aware polling.
- [ ] Implement the sampler, command migration, state registration, and renderer guard.

## Verification

- [ ] Run focused memory/app/UI tests and JavaScript syntax checks.
- [ ] Clean the app crate and run warning-denied Clippy for all targets.
- [ ] Run CI-mode workspace tests and warning-denied workspace Clippy.
- [ ] Rebuild/open the native app, observe the RAM rail value, hide/show the window, and confirm the
      value resumes. Deterministic contracts cover the performance boundary, so no manual-only item
      is needed.
- [ ] Update `handoff.md` and the combined remediation ledger.
