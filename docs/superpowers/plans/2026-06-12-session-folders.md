# Session Folders (Milestone 13) Implementation Plan

> **For agentic workers:** Follow the repo's plan-driven TDD convention. Execute task-by-task,
> write failing tests or repeatable checks before implementation, and leave checkboxes unticked.

**Goal:** Group saved clips into session folders the way Outplayed does, with both session
definitions the user asked for (2026-06-12): one folder per recorder run, plus a dedicated
folder per detected LoL match. **Exit criterion:** new saves land in
`Videos\Clipline\<session>\`, the library groups clips by session, an Open Folder button
reveals the clip in Explorer, GC/quota work across folders, and legacy root clips keep
working. Tests, clippy, user-driven live check, push, CI green.

## Design

**Session semantics.** The recorder service owns a current session label:

- On service start: run label `YYYY-MM-DD HH-MM` (local time at service start).
- When the LoL poller connects to a live match: switch to `YYYY-MM-DD HH-MM league`
  (stamp at match start). When the match ends: revert to the run label.
- Folders are created **lazily at save time** — idle runs leave no empty folders.
- Settings restarts spawn a new service → a new run label. That is the "per app/recorder
  run" definition working as intended.

**Layout.** One level deep: `Videos\Clipline\<session>\clip_<unix>.mp4` (+ sidecars).
Pre-existing clips at the root remain valid ("legacy"); exports stay siblings of their
source, so they inherit the session folder.

**Plumbing changes:**

1. `clipline-storage` (neutral, TDD on both CI OSes):
   - `inventory` scans the root **plus one level** of subdirectories; status/GC totals
     include nested clips; GC deletes oldest-first across folders and removes
     **now-empty session directories** afterwards.
   - New `sessions` module: `session_label(y, mo, d, h, mi, league)` formatting and a
     `SessionTracker` (run label, `match_started(label)` / `match_ended()` / `current()`).
2. `apps/clipline-app`:
   - `markers::spawn` channel becomes `PollerMsg { Event(GameEvent), MatchStarted, MatchEnded }`
     — MatchStarted when `active_player_name` first succeeds, MatchEnded when the poll loop
     breaks. Service feeds the tracker and saves into `clips_dir/<tracker.current()>/`.
   - `chrono` (windows-gated dependency) supplies local wall-clock fields for labels.
   - `list_clips` walks root + one level, `ClipInfo` gains `session: Option<String>`.
   - `validate_clip_path` accepts clips whose parent **or grandparent** is the clips dir.
   - New `reveal_clip(path)` command: validate, then `explorer /select,<path>`.
   - assetProtocol scope gains `**/Videos/Clipline/**/*.mp4` (subfolder playback).
3. UI:
   - Library renders **session groups**: pure `sessionGroups(clips)` in `player-core.js`
     (group by `session`, legacy root clips under "Earlier", groups sorted by newest clip,
     clips newest-first within) — Boa-tested.
   - `Open Folder` button (`id="open-folder"`) in the review header → `reveal_clip`.

### Task 1: failing tests first

- [ ] `clipline-storage`: nested inventory counted in status; GC deletes oldest-first across
  folders; sidecars in folders deleted with their clip; emptied session folders removed;
  root legacy clips still inventoried; `session_label` formatting (zero-padding, league
  suffix); `SessionTracker` start → match → revert transitions.
- [ ] `player_core.rs`: `sessionGroups` grouping, ordering, legacy bucket, empty input.
- [ ] `ui_contract.rs`: `id="open-folder"` required.

### Task 2: implement

- [ ] Storage recursion + empty-dir cleanup + sessions module.
- [ ] Poller messages, service session state, save path, chrono labels.
- [ ] `list_clips` sessions, path validation, `reveal_clip`, asset scope.
- [ ] `sessionGroups` + grouped library render + Open Folder button.
- [ ] All Task-1 tests green.

### Task 3: gates and handoff

- [ ] `cargo test --workspace`; clean clippy.
- [ ] Launch the app, hand the user a checklist (save a replay → lands in a new session
  folder and group; Open Folder reveals it; legacy clips under "Earlier"; delete/export/GC
  still work on foldered clips). Match folders are exercised with the `--lol-url` mock or
  noted for the next real match.
- [ ] Update `handoff.md` (milestone 13, layout, sharp edges: asset scope glob, legacy root
  clips, lazy folder creation).
- [ ] Commit, push, CI green.

## Out of scope

- Migrating legacy root clips into dated folders (manual move works; revisit if wanted).
- Per-game session detection beyond LoL.
- Session renaming/metadata files; the folder name is the session identity.
- Timeline zoom (next milestone, unchanged).
