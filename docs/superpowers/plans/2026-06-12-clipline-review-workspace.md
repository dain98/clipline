# Clipline Review Workspace (Milestone 11) Implementation Plan

> **For agentic workers:** Follow the repo's plan-driven TDD convention. Execute task-by-task,
> write failing tests or repeatable checks before implementation, and leave checkboxes unticked.

**Goal:** Replace the prototype-grade native media controls with a Clipline-owned review workspace
that feels like a real clipping product. **Exit criterion:** opening a clip presents a custom player
surface with first-party transport controls, a useful timeline, marker navigation, keyboard review
shortcuts, visible in/out trim handles, export/delete/reveal actions near the clip, and no browser
default video chrome. Workspace tests, clippy, a live app smoke test, push, and CI must pass.

**Architecture:** Keep the existing H.264/Opus WebView2 `<video>` playback and
`export_clip(path, start_s, end_s)` backend. This milestone only replaces the user-facing review
experience around that video element. The video element remains the decode/playback engine, but its
native controls are hidden and all interaction flows through Clipline's own DOM controls.

The design should stay operational rather than decorative: dense library list on the left, the
current clip as the dominant workspace, fixed-size transport buttons, a stable timeline that cannot
shift as labels update, and direct review actions. No landing-page treatment, no nested cards, no
generic browser media controls.

---

### Task 1: frontend workspace checks

**Files:** `apps/clipline-app/ui/index.html`, optional test helper under `apps/clipline-app/ui/`.

- [ ] **Step 1: failing checks**
  - Add a lightweight repeatable check for the player DOM contract: the video element must not have
    `controls`, and the document must include custom controls for play/pause, skip back/forward,
    timeline, trim handles, marker navigation, export, delete, and reveal.
  - The check should run without a Tauri runtime, so it can execute in CI on Windows and Ubuntu.

- [ ] **Step 2: implement enough markup/classes to satisfy the structural check.**

### Task 2: first-party player chrome

**Files:** `apps/clipline-app/ui/index.html`.

- [ ] Remove visible native video controls.
- [ ] Add a custom transport row:
  - play/pause
  - jump back/forward
  - mute/unmute
  - playback rate selector
  - current time / duration
  - volume slider
- [ ] Add keyboard review controls while the player is open:
  - `Space` toggles play/pause
  - arrow keys seek
  - `I` and `O` set trim in/out
  - `M` jumps to the next marker
  - `Esc` closes the workspace
- [ ] Keep text and controls inside stable dimensions so labels cannot resize the transport row.

### Task 3: product timeline and marker navigation

**Files:** `apps/clipline-app/ui/index.html`.

- [ ] Replace the thin progress strip with a richer timeline:
  - buffered/progress fill
  - draggable or click-seek playhead
  - visible trim selection range
  - draggable trim handles for in/out
  - marker ticks with hover tooltips
- [ ] Add previous/next marker buttons and a marker summary/count near the timeline.
- [ ] Timeline clicks and marker clicks must seek without affecting trim handles unless the user is
  dragging a handle.

### Task 4: review workspace layout and actions

**Files:** `apps/clipline-app/ui/index.html`.

- [ ] Replace the full-screen overlay with a two-pane review workspace:
  - library/status/settings remain available in a left sidebar
  - the selected clip dominates the right workspace
  - empty state still guides toward saving a replay
- [ ] Add clip-local actions near the selected clip:
  - export current trim
  - delete source clip
  - reveal source path by copying it to the clipboard when shell reveal is not yet available
- [ ] Keep backend validation authoritative for delete/export.
- [ ] On successful export, refresh library/storage and keep the source clip open with clear aligned
  range feedback.

### Task 5: gates and handoff

- [ ] Run the new frontend check.
- [ ] `cargo test --workspace`
- [ ] `cargo clippy --workspace --all-targets`
- [ ] Live smoke: launch the app, open a saved clip, confirm the custom workspace renders without
  native controls, and shut the process down.
- [ ] Update `handoff.md` with milestone 11, the new next-step ordering, and any UI sharp edges.
- [ ] Commit implementation, push, and verify CI on Ubuntu + Windows.

---

## Out of scope

- Native FFmpeg/D3D11 preview decode.
- Frame-accurate trim re-encode.
- Montage/joining multiple clips.
- GIF/WebM export.
- Shell-integrated reveal command.
- Cloud sharing, accounts, social feed, or upload flows.
