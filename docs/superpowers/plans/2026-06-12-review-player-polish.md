# Review Player Polish (Milestone 12) Implementation Plan

> **For agentic workers:** Follow the repo's plan-driven TDD convention. Execute task-by-task,
> write failing tests or repeatable checks before implementation, and leave checkboxes unticked.

**Goal:** Close the intuitiveness gap against Outplayed's layout (user-supplied comparison,
2026-06-12) without their ad rail or mode-switching. **Exit criterion:** typed marker ticks,
a labeled time ruler, transport in the conventional position under the stage, human-first
library labels, and a focus mode — tests, clippy, live user check, push, CI green.

**Scope guard:** frontend-only (markup/styles/player-core/main + tests). No backend changes.
The two-band zoomable timeline and click-to-edit timecodes are the **next** milestone, not this
one.

## Design

1. **Typed marker ticks.** Marker `kind` arrives as the `EventKind` variant name. Map kinds to
   glyph + color category in pure logic (`markerStyle`):
   - kill (`ChampionKill`, `FirstBlood`) → red `✕`
   - spree (`Multikill`, `Ace`) → orange `★`
   - objective (`DragonKill`, `HeraldKill`, `BaronKill`) → purple `◆`
   - structure (`TurretKilled`, `InhibKilled`, `FirstBrick`) → amber `▣`
   - info (everything else / unknown) → muted `•`
   Ticks become small kind-colored chips with the glyph; tooltip unchanged.
2. **Labeled time ruler** under the timeline: `rulerMarks(duration, maxMarks)` picks a "nice"
   step (1/2/5/10/15/30/60/120/300/600 s…) and returns `{t, label}` marks rendered as a slim
   gradation row. Pure, tested.
3. **Transport reorder.** Deck order becomes transport → timeline+ruler → export row, matching
   twenty years of video-player muscle memory (controls glued to the stage). Contract test
   asserts the order.
4. **Human-first library labels.** Row title becomes `Jun 11 · 10:25 PM`
   (`formatClipTitle(month0, day, hours, minutes)` — pure, 12-hour conversion tested); info
   line `0:22 · 63.2 MB · 2h ago` plus a marker digest (`markerDigest`: "1 kill",
   "2 kills · 1 objective"). Filename moves to the row tooltip; the review header keeps full
   filename + path.
5. **Focus mode.** `F` (new `keyIntent`) or a header button collapses the sidebar while
   reviewing — the stage gets the full window. Esc still closes the clip (and leaves focus
   mode with it).

### Task 1: failing tests first

- [ ] `player_core.rs`: `markerStyle` category/glyph mapping incl. unknown kinds;
  `rulerMarks` nice-step selection and labels (e.g. 22 s → 5 s steps, 1200 s → 300 s steps);
  `formatClipTitle` 12-hour edges (midnight, noon, PM); `markerDigest` pluralization and
  category collapsing; `keyIntent('KeyF')` → `toggle-focus`.
- [ ] `ui_contract.rs`: `id="ruler"` and `id="focus-toggle"` exist; transport (`play-toggle`)
  precedes `id="timeline"` in the document; existing contract intact.

### Task 2: implement

- [ ] `player-core.js`: the four pure functions + `KeyF` intent.
- [ ] `index.html`/`styles.css`/`main.js`: chip ticks, ruler row, deck reorder, library labels,
  focus toggle (class on `.app`, sidebar collapses, grid stays bounded).
- [ ] All Task-1 tests green.

### Task 3: gates and handoff

- [ ] `cargo test --workspace`; `cargo clean -p clipline-app` + clippy zero warnings.
- [ ] Launch the app and hand the user a test checklist (user-driven verification this
  session — synthesized input races a live operator; see handoff sharp edge).
- [ ] Update `handoff.md` milestone list.
- [ ] Commit, push to the open PR branch, CI green.

## Out of scope

- Timeline zoom / overview band, click-to-edit timecodes (next milestone).
- Export naming (needs a backend parameter — bundle with a future export milestone).
- Thumbnails, session grouping, search (need decode/metadata work).
