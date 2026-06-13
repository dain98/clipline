---
conclusion: neutral
waitsFor:
  - "*"
waitsForTimeout: 30
---

Use Macroscope auto-approval only for low-risk maintenance changes.

Do NOT auto-approve a PR when it changes any of these areas:

- `crates/clipline-capture/**`
- `crates/clipline-mp4/**`
- `crates/clipline-buffer/**`
- `crates/clipline-events/**`
- `crates/clipline-lol/**`
- `crates/clipline-storage/**`
- `apps/clipline-app/src/**`
- `apps/clipline-app/ui/**`
- `.github/workflows/**`
- `Cargo.toml`
- `Cargo.lock`
- `ddoc.md`
- `handoff.md`

Do NOT auto-approve if the PR introduces or changes behavior related to capture, encoding, muxing, replay saving, trimming/export, League markers, storage deletion or quota GC, settings persistence, hotkeys, file path validation, privacy, anti-cheat posture, CI, release packaging, or dependencies.

Auto-approval can be considered only for small documentation updates, comments, or tests that do not change production behavior and have no unresolved review or CI findings.
