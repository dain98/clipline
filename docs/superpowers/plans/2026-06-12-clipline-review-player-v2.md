# Clipline Review Player v2 (Milestone 11, redone) Implementation Plan

> **For agentic workers:** Follow the repo's plan-driven TDD convention. Execute task-by-task,
> write failing tests or repeatable checks before implementation, and leave checkboxes unticked.

**Goal:** Replace the prototype overlay player (native `<video controls>`) with a first-party
review player. **Exit criterion:** opening a clip presents a Clipline-owned player with an
integrated trim timeline, marker navigation, keyboard-first review controls, export/delete/copy
actions, and no browser media chrome. Frontend logic is unit-tested from Rust; tests, clippy,
live smoke, push, and CI must pass.

**History note:** a prior custom workspace (commit `bd1c84f`) was reverted in this branch; this
plan rebuilds the milestone from the pre-overlay base with a different design and a tested
frontend architecture.

## Design

**Editing model:** position the playhead precisely, then mark. There are no in/out number
inputs and no Set In/Set Out buttons — the timeline plus `I`/`O` at the playhead is the whole
trim interface, with `,`/`.` stepping 0.1 s for precision. One way to do each thing.

**Shell:** two panes. Left sidebar (280 px): recorder status + Save Replay on top, the library
as the main scrollable content, settings collapsed in a `<details>` at the bottom (settings are
visited rarely; clips constantly). Right: the review pane — header (clip name/meta, Copy Path,
Delete, Close), video stage, control deck.

**Control deck:**

- Timeline (taller, ~40 px): excluded regions outside the trim are dimmed — "what you keep" is
  bright; in/out edges are draggable brackets; amber marker ticks (hover tooltip, click seeks);
  thin playhead; click/drag anywhere else scrubs.
- Transport row: `⟨` prev / `⟩` next marker + marker count, `-5` `Play/Pause` `+5`,
  `current / duration` readout (tenths, tabular), playback-rate select, mute + volume.
- Export row: primary `Export` button next to a live trim readout
  (`keeps 0:02.0 – 0:18.4 · 16.4 s · snaps to keyframes`) and a status line that reports the
  exported file and its aligned range. Backend stays authoritative for validation.

**Keyboard (while a clip is open, never when typing in a field):** `Space`/`K` play-pause,
`←`/`→`/`J`/`L` seek 5 s (`Shift` = 1 s), `,`/`.` step 0.1 s, `I`/`O` set trim in/out at the
playhead, `M` next / `Shift+M` previous marker, `Esc` close.

**Architecture:** `ui/index.html` (markup only) + `ui/styles.css` + `ui/player-core.js` (pure,
DOM-free, Tauri-free logic exposed as `globalThis.PlayerCore`) + `ui/main.js` (DOM/Tauri
wiring). Pure logic is tested from Rust via `boa_engine` (dev-dependency; no Node/npm — runs on
both CI OSes). The `<video>` element stays the decode engine (H.264/Opus in WebView2);
`export_clip`/`delete_clip`/`list_clips`/settings invokes are unchanged.

**Layout discipline (sharp edge from the reverted attempt):** every grid that must bound its
content needs explicit `minmax(0, 1fr)` rows/columns and `min-height: 0` down the chain — a
content-sized grid row lets the video's intrinsic height push the deck below the window.

**Window:** default 1200x760, minimum 960x640 (body min sizes match, so below-minimum
degrades to scrollbars, never to clipped controls).

---

### Task 1: failing tests first

**Files:** `apps/clipline-app/Cargo.toml`, `apps/clipline-app/tests/player_core.rs`,
`apps/clipline-app/tests/ui_contract.rs`.

- [ ] `boa_engine` dev-dependency (neutral; dev-deps skip the windows gate).
- [ ] `player_core.rs`: evaluate `ui/player-core.js` in Boa and assert formatting
  (`fmtDur` carry incl. the `0:60` bug, `fmtTenths`, `fmtBytes`, pure `fmtAgo`), trim
  clamping (`resolveTrim`, `trimDrag` minimum gap), `clampTime`/`percentFor`/`timelineTime`,
  marker navigation (epsilon skip, wraparound, empty), `trimSummary` text, and `keyIntent`
  for every shortcut above incl. `,`/`.` steps and `Shift+M`.
- [ ] `ui_contract.rs`: the `<video>` tag must not carry `controls`; required control ids
  (play toggle, seek back/forward, marker prev/next + summary, timeline, trim handles, export,
  delete, copy path, close, rate, mute, volume) exist; `index.html` references `styles.css`,
  `player-core.js`, `main.js` and contains no inline `<style>` or script bodies.
- [ ] Run: new assertions fail.

### Task 2: implement

**Files:** `apps/clipline-app/ui/{index.html,styles.css,player-core.js,main.js}`,
`apps/clipline-app/tauri.conf.json`.

- [ ] `player-core.js` per the API above.
- [ ] Markup/styles/wiring per the design; clip rows built with `textContent` (filenames never
  meet `innerHTML`); delete confirms; mute restores volume when unmuting from zero; autoplay on
  open; volume/rate persist across clips within the session.
- [ ] Window sizing in `tauri.conf.json`.
- [ ] All Task-1 tests green.

### Task 3: gates and handoff

- [ ] `cargo test --workspace`
- [ ] `cargo clean -p clipline-app && cargo clippy --workspace --all-targets` (zero warnings)
- [ ] Live smoke: open the marked test clip, exercise transport/trim/markers/shortcuts at
  default and minimum window sizes, confirm no native chrome, shut down cleanly.
- [ ] Update `handoff.md` (milestone 11 entry rewritten for this design, frontend test seam,
  layout sharp edge).
- [ ] Commit, push branch, open PR; CI green on ubuntu + windows.

---

## Out of scope

- Native FFmpeg/D3D11 preview decode; frame-accurate boundary re-encode.
- Thumbnails/filmstrip, waveform, multi-clip montage, GIF/WebM export.
- Shell reveal (copy-path remains the stand-in), cloud/social anything.
- A JS bundler, npm, or Node toolchain.
