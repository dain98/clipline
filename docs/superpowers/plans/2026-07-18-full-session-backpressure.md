# Full-Session Writer Backpressure Plan

**Goal:** A stalled full-session output must not grow capture-process memory without limit or stop replay capture.

**Architecture:** Sealed segments become shared immutable values so the memory replay ring and full-session writer can reference one payload instead of deep-cloning it. The disk replay ring writes the same value by reference. Full-session delivery uses both a bounded channel and an atomic byte reservation; capture attempts a non-blocking enqueue and disables only the full-session sink when either limit is reached. Already accepted segments are finalized when recording stops, while the queue error is returned to the app. Writer-thread spawn errors propagate from `start_full_session` instead of panicking.

## Task 1: Backpressure TDD

- [ ] Add a deterministic test with a deliberately tiny byte budget that proves full-session enqueue failure does not abort capture or stop replay buffering.
- [ ] Add queue tests proving rejected messages release their byte reservation and the queued payload never exceeds its configured budget.
- [ ] Confirm the new tests fail against the unbounded channel implementation.

## Task 2: Shared segment ownership

- [ ] Store immutable shared segments inside the memory replay ring while preserving its borrowed public iterator and save-window API.
- [ ] Add a borrowed disk-ring insertion path so full-session recording does not require a payload clone with disk replay storage.
- [ ] Pass shared segments through the full-session writer and keep muxing APIs borrowed.

## Task 3: Bounded writer delivery

- [ ] Replace the unbounded writer channel with a bounded synchronous channel and exact payload-byte accounting.
- [ ] Use non-blocking segment enqueue so slow output cannot stall capture.
- [ ] On queue saturation, stop accepting full-session segments, retain replay capture, finalize accepted output, and return a clear error.
- [ ] Return writer-thread spawn failures from `start_full_session`.

## Task 4: Verify and document

- [ ] Run focused buffer and capture tests.
- [ ] Run `cargo test --workspace`.
- [ ] Clean changed crates and run `cargo clippy --workspace --all-targets -- -D warnings`.
- [ ] Update `handoff.md` with the queue bound, ownership model, and failure policy.
- [ ] Stop any running `clipline-app.exe`, launch `cargo run -p clipline-app`, and verify replay/full-session controls initialize normally.
