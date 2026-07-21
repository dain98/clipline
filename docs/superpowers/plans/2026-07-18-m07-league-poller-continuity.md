# M-07 League Poller Continuity Plan

**Goal:** Keep one League match and one cumulative-event watermark alive across brief Live Client API failures, while still recognizing explicit endings, sustained endpoint loss, and a genuinely new match.

## Continuity model

- [ ] Keep `EventTracker` for the lifetime of the poller rather than recreating it after an HTTP error.
- [ ] Record the last successful game clock and event watermark; treat a meaningful game-clock rollback or maximum-event-ID rollback as a reliable new-match signal and reset exactly once.
- [ ] Return the new-match signal with each normalized poll batch so the app can order session boundaries before the first event of the new match.
- [ ] Preserve the existing cumulative-event deduplication behavior during ordinary polls and after transient failures.

## Failure and lifecycle policy

- [ ] Extract a deterministic lifecycle policy with tests for initial start, transient failure recovery, sustained absence, explicit `GameEnd`, and recovery/new-match transitions.
- [ ] Retry transient failures with bounded exponential backoff and emit `MatchEnded` only after six consecutive failed polls (roughly twenty seconds with the selected backoff).
- [ ] End immediately on `GameEnd`, but do not create a second session while Riot's endpoint still serves the completed match.
- [ ] Add a non-semantic heartbeat message during waits/backoff so a dropped receiver terminates the poller even when League is not running.

## Tests and verification

- [ ] Add tracker and mock-HTTP tests proving that a transient failure does not replay cumulative events and that clock/event-ID rollback admits the first events of the next match.
- [ ] Add app policy tests proving boundary messages are emitted once and only under the documented conditions.
- [ ] Run focused League/app tests and fresh-cache Clippy for changed crates.
- [ ] Run workspace tests and workspace Clippy with warnings denied.
- [ ] Rebuild/open Clipline and update `handoff.md`, the master audit ledger, and the accumulated manual acceptance checklist.

## Manual acceptance addition

- During a real League match, briefly interrupt access to the Live Client API (less than twenty seconds), then restore it. Confirm Clipline keeps one match/session, does not duplicate earlier markers, and captures later markers. After a real `GameEnd`, start another match and confirm the new session begins and low event IDs are accepted.
