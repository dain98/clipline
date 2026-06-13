Macroscope should review Clipline changes against the repository source of truth:

- `ddoc.md` defines product and architecture decisions.
- `handoff.md` describes current implementation state, conventions, sharp edges, and next work.
- `AGENTS.md` contains agent workflow rules.

Prioritize correctness, data loss, crashes, privacy leaks, anti-cheat risk, media corruption, A/V desync, resource leaks, race conditions, and CI breakage. Do not raise style-only comments unless the style issue hides a concrete bug.

Core constraints:

- Never introduce DLL injection, process memory reading, kernel drivers, hidden telemetry, ads, account requirements, or cloud-only behavior.
- Keep first-party code compatible with `MIT OR Apache-2.0`; flag GPL or nonfree dependencies unless explicitly justified.
- Neutral Rust logic should stay cross-platform and testable on Ubuntu and Windows CI.
- Windows-only code must stay behind `#[cfg(windows)]` or equivalent build gates.
- Behavior-changing PRs should update `handoff.md` or the relevant `docs/superpowers/plans` entry when project state changes.
