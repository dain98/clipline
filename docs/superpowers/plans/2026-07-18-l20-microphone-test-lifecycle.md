# L-20 Microphone Test Lifecycle Plan

**Goal:** Make microphone-test replacement atomic, ensure every superseded worker observes shutdown,
and prevent an obsolete worker from publishing UI state after a newer test becomes active.

**Finding:** `CODEBASE_AUDIT_COMBINED.md` L-20.

## Design boundary

- [ ] Replace the bare optional stop sender with a mutex-protected session record containing a
      monotonic generation and its stop sender.
- [ ] Make generation allocation, previous-session stop, and new-session installation one locked
      transaction so concurrent starts cannot strand an untracked worker.
- [ ] Treat both an explicit stop message and stop-channel disconnection as terminal in the worker.
- [ ] Gate monitor/error/stopped event publication on the worker still owning the active generation;
      serialize that check with replacement so a superseded worker cannot publish after install.
- [ ] Clear the installed generation conditionally if thread creation fails or the active worker
      terminates with an error, without disturbing a newer generation.

## TDD sequence

- [ ] Add a focused stop-channel test proving Empty continues while sent and disconnected channels
      stop.
- [ ] Add a concurrent replacement test that starts multiple sessions together, proves exactly one
      generation remains active, and proves every superseded receiver observes stop.
- [ ] Add stale-generation callback/finish tests proving obsolete workers cannot mutate active UI
      state or clear the current session.
- [ ] Implement the session transaction and worker integration with fallible named thread creation.

## Verification

- [ ] Run the focused app tests for microphone session lifecycle.
- [ ] Clean `clipline-app`, then run warning-denied app Clippy.
- [ ] Run CI-mode workspace tests and warning-denied workspace Clippy.
- [ ] Rebuild and open the exact workspace app; verify the Library loads and use Computer Use to
      start and stop the microphone monitor from Settings when a microphone is available.
- [ ] Update `handoff.md` and the combined remediation ledger with plan/implementation evidence and
      add any genuinely hardware-only acceptance scenario to the final checklist.
