# Clipline Library View + Marker Timeline (Milestone 7) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The markers become visible (ddoc §3/§5 UI layer, first slice): the app gains a
library of saved clips and an in-app player whose timeline renders marker ticks —
click a tick, land on the dragon. **Exit criterion:** the running app lists the existing
clips with durations/marker counts, plays one in the webview (H.264+Opus `<video>` — ddoc §4
explicitly blesses H.264 for in-webview playback; the native-decode path is for AV1/HEVC
later), and the marked clip shows its marker on the scrubber.

**Architecture:** One neutral addition: `clipline-mp4::walker::movie_duration_s` parses
`moov/mvhd` (v0 + v1) so the library reads durations from our own files — dogfooding the box
walker, TDD'd against `HybridMp4Writer` output. App side: `#[tauri::command] list_clips`
scans `Videos\Clipline` (size, mtime, mvhd duration, sidecar markers when present) and
`delete_clip` removes a clip + sidecar (path-validated: must resolve inside the clips dir —
the webview never passes raw paths to `fs`). Playback uses Tauri's **asset protocol**
(`assetProtocol.enable` + scope on the clips dir, `convertFileSrc` in JS). UI: the status
page grows a library section (newest-first cards: name, duration, size, marker count, play +
delete) and a player overlay (`<video>` + a custom timeline bar under it: marker ticks
positioned at `t_s/duration`, hover = event kind, click = seek).

**Tech Stack:** no new dependencies. `tauri.conf.json` gains the asset-protocol scope;
`capabilities/default.json` stays `core:default` (asset protocol is config-side).

**Environment notes:** the dev machine has real clips already (one with a markers sidecar
from the mock-server e2e: a Hextech DragonKill at t≈2.15 s). Verification is live with
screenshots. Commits end with `Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>`.

---

### Task 1: `movie_duration_s` (clipline-mp4)

**Files:** `crates/clipline-mp4/src/walker.rs`, `src/lib.rs` (re-export).

- [ ] **Step 1: failing tests** (walker tests; build a finalized file via the existing
writer test helpers — same pattern the e2e tests use):

```rust
    #[test]
    fn movie_duration_reads_mvhd() {
        // Finalized writer output: known sample count/durations.
        let buf = finalized_file_with(60, 90_000, 1_500); // 60 samples × 1500/90k = 1.0 s
        let d = movie_duration_s(&buf).expect("mvhd present");
        assert!((d - 1.0).abs() < 1e-6);
    }

    #[test]
    fn movie_duration_none_without_moov() {
        assert!(movie_duration_s(b"not an mp4").is_none());
        let frag_only = mp4_box(*b"ftyp", vec![0; 4]);
        assert!(movie_duration_s(&frag_only).is_none());
    }
```
(`finalized_file_with` = small local helper writing N fragments through `HybridMp4Writer`
and finalizing — crib from the existing writer tests.)

- [ ] **Step 2: verify failure → Step 3: implement**

```rust
/// Movie duration from moov/mvhd (version 0 or 1). None when the buffer
/// has no finalized moov (still-fragmented or foreign data).
pub fn movie_duration_s(buf: &[u8]) -> Option<f64> {
    let moov = find(&walk(buf), b"moov")?.clone();
    let mvhd = find(&children(buf, &moov), b"mvhd")?.clone();
    let p = mvhd.payload_offset as usize;
    let version = *buf.get(p)?;
    // v0: ver/flags(4) ctime(4) mtime(4) timescale(4) duration(4)
    // v1: ver/flags(4) ctime(8) mtime(8) timescale(4) duration(8)
    let (ts_off, dur_off, dur_is_64) = match version {
        0 => (p + 12, p + 16, false),
        1 => (p + 20, p + 24, true),
        _ => return None,
    };
    let timescale = u32::from_be_bytes(buf.get(ts_off..ts_off + 4)?.try_into().ok()?) as f64;
    let duration = if dur_is_64 {
        u64::from_be_bytes(buf.get(dur_off..dur_off + 8)?.try_into().ok()?) as f64
    } else {
        u32::from_be_bytes(buf.get(dur_off..dur_off + 4)?.try_into().ok()?) as f64
    };
    (timescale > 0.0).then(|| duration / timescale)
}
```

- [ ] **Step 4: pass → Step 5: commit** `feat(mp4): movie_duration_s from mvhd`.

---

### Task 2: clip commands + asset protocol

**Files:** `apps/clipline-app/src/app.rs` (or new `library.rs`), `tauri.conf.json`.

`tauri.conf.json` → `app.security.assetProtocol`:
```json
"assetProtocol": { "enable": true, "scope": ["$VIDEO/Clipline/**", "**/Videos/Clipline/**"] }
```
(use whichever scope variable resolves on Windows; verify live.)

`library.rs`:
```rust
#[derive(serde::Serialize)]
pub struct ClipInfo {
    pub path: String,
    pub name: String,
    pub size_mb: f64,
    pub modified_unix: u64,
    pub duration_s: Option<f64>,   // movie_duration_s on the file head+tail read
    pub markers: Option<clipline_events::ClipMarkers>, // sidecar if present
}

#[tauri::command]
pub fn list_clips() -> Vec<ClipInfo> { /* scan clips_dir, newest first */ }

#[tauri::command]
pub fn delete_clip(path: String) -> Result<(), String> {
    // canonicalize; require parent == clips_dir; remove mp4 + sidecar
}
```
Reading duration: `movie_duration_s(&std::fs::read(path))` is fine at these sizes for v1
(clips are ≤ a few hundred MB and listed once; optimize later by reading just the moov tail).
Hmm — actually read at most the LAST 1 MiB and scan from the first box boundary… moov is at
the END of finalized hybrid files only after the soft-remux overwrote the head placeholder.
Simplest correct v1: full read. Revisit if listing feels slow.

Register both commands; service `clips_dir()` becomes `pub(crate)` and shared.

- [ ] Compile + clippy. Commit `feat(app): clip library commands + asset protocol`.

---

### Task 3: library UI + player

**Files:** `apps/clipline-app/ui/index.html`, `src/app.rs` (window height bump in config).

- Library section under the status block: cards via `invoke("list_clips")` on load and on
  every `saved` event. Each card: name, `duration_s`, `size_mb`, marker count badge, ▶ play,
  🗑 delete (`invoke("delete_clip", {path})` then refresh).
- Player overlay: full-window dark layer; `<video controls>` with
  `src = convertFileSrc(path)`; beneath it a custom marker bar (relative div): for each
  marker a tick at `left = t_s / duration * 100%`, `title` = kind/actor/victim; click →
  `video.currentTime = t_s`. Close button.
- Window grows (e.g. 520x640, resizable true).

- [ ] Compile. Commit `feat(app): library view + marker timeline player`.

---

### Task 4: live e2e + gates

- [ ] Relaunch the app; screenshot: library lists the existing clips with durations and the
marked clip shows its badge. Open the marked clip: screenshot the player with the marker tick
rendered; click it (seek) — confirm `<video>` actually plays H.264+Opus in WebView2 (the one
genuine unknown; if Opus-in-MP4 refuses, fall back: `muted` video plays regardless — note it
and decide).
- [ ] Delete one of the old oversized test clips through the UI (proves delete + refresh).
- [ ] `cargo test --workspace`, clippy, push, CI green both OSes.
- [ ] `handoff.md`: milestone 7 done; next frontier (settings/disk-GC, auto-clip, FFmpeg
matrix, installer).

---

## Out of scope (follow-ups)

- Frame-accurate scrubbing / native FFmpeg decode preview (ddoc §11 — needed for AV1/HEVC).
- Trim/export editor; thumbnails; disk quota + GC; auto-clip on importance.
- Marker overlays burned into exports.
