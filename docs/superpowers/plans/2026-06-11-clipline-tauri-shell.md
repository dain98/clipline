# Clipline Tauri Shell (Milestone 5: hotkey → save_replay) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The first user-facing artifact — a Tauri app that runs the replay buffer continuously
and saves a clip on **Alt+F10** (ShadowPlay's default, ddoc §6) or from the tray/UI.
**Exit criterion:** launch `clipline-app`, see the tray icon + status window, play something,
press Alt+F10 → a timestamped, playable MP4 lands in `Videos\Clipline`, with the smart
no-overlap mode active across repeated saves.

**Architecture:** Two layers. (1) Platform-neutral: `Recorder` becomes steppable —
`step()` processes one frame (audio drain → encode → GOP seal), `finish_stream()` drains the
encoder and seals the tail; `run_to_end()` becomes a thin loop over both. This is what makes
**save-while-recording** possible: a service loop alternates `step()` with command handling,
and `save_replay(&self)` runs between steps. (2) `clipline-app` (new workspace crate,
`apps/clipline-app`): a recorder service thread (WGC primary monitor by default or
`--window <title>`; AMF H.264; WASAPI loopback; one shared clock; ring budget ~120 s) driven
by a `Cmd::{Save, Stop}` channel; Tauri v2 shell with tray (Save Replay / Quit), the
`global-shortcut` plugin binding Alt+F10 → `Cmd::Save`, and a static-HTML status page fed by
`status`/`saved` events. **All Tauri/native deps are `[target.'cfg(windows)'.dependencies]`
with a stub `main` elsewhere** — ubuntu CI must not need webkit2gtk (the repo's established
pattern; capture is Windows-only anyway).

**Tech Stack:** `tauri 2.11` (features `tray-icon`), `tauri-plugin-global-shortcut 2.3`,
`tauri-build 2` (build-dep, gated by `CARGO_CFG_WINDOWS` in build.rs). Frontend: one static
`index.html` (`frontendDist` + `withGlobalTauri` — no node/npm). Tray icon: a procedurally
generated RGBA square via `tauri::image::Image::new_owned` — no icon assets, no bundler
(this milestone ships `cargo run -p clipline-app`, not an installer).

**Environment notes:** Idle desktops starve WGC — the service loop treats
`CaptureError::Timeout` as "no frame, keep serving commands", not an error. Saves only cover
sealed GOPs (≤2 s behind realtime at the default GOP). Monitor capture on the 5120-wide
display scales to ≤2560 (same as record_smoke). Commits end with
`Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>`.

---

### Task 1: steppable `Recorder` (platform-neutral)

**Files:** `crates/clipline-capture/src/pipeline.rs`

- [ ] **Step 1: failing test**

```rust
    #[test]
    fn save_replay_works_between_steps_while_recording() {
        let mut rec = Recorder::new(
            MockCapture::new(90, 30),
            MockEncoder::new(30, 30),
            usize::MAX,
        );
        // Two GOPs in: a save must succeed without ending the recording.
        for _ in 0..60 {
            assert!(rec.step().unwrap());
        }
        let (buf, end) = rec
            .save_replay(std::io::Cursor::new(Vec::new()), 10.0, None)
            .map(|(w, e)| (w.into_inner(), e))
            .expect("mid-recording save");
        assert!(!buf.is_empty());
        assert!((end - 1.0).abs() < 1e-6, "one sealed GOP at save time (second pending)");
        // Recording continues; smart mode skips the already-saved second.
        for _ in 0..30 {
            assert!(rec.step().unwrap());
        }
        assert!(!rec.step().unwrap(), "source exhausted");
        rec.finish_stream().unwrap();
        let (_, end2) = rec
            .save_replay(std::io::Cursor::new(Vec::new()), 10.0, Some(end))
            .expect("post-finish save");
        assert!((end2 - 3.0).abs() < 1e-6, "everything sealed after finish");
        // run_to_end equivalence: same segment layout as the stepped path.
        let mut whole = Recorder::new(MockCapture::new(90, 30), MockEncoder::new(30, 30), usize::MAX);
        whole.run_to_end().unwrap();
        assert_eq!(whole.ring().len(), rec.ring().len());
    }
```

- [ ] **Step 2: verify failure → Step 3: implement** — extract `run_to_end`'s loop body:

```rust
    /// Process one captured frame (audio drain → encode → GOP sealing).
    /// `Ok(false)` = the capture source ended. Errors pass through —
    /// callers running live decide how to treat `CaptureError::Timeout`.
    pub fn step(&mut self) -> Result<bool, PipelineError> { /* body of the while-let */ }

    /// End of stream: drain the encoder (`finish`), final audio drain,
    /// seal the trailing partial GOP.
    pub fn finish_stream(&mut self) -> Result<(), PipelineError> { /* tail of run_to_end */ }

    pub fn run_to_end(&mut self) -> Result<(), PipelineError> {
        while self.step()? {}
        self.finish_stream()
    }
```

- [ ] **Step 4: all green → Step 5: commit**
`refactor(capture): steppable Recorder - save-while-recording`.

---

### Task 2: `clipline-app` scaffold

**Files:**
- Modify: root `Cargo.toml` (member `apps/clipline-app`)
- Create: `apps/clipline-app/Cargo.toml`, `build.rs`, `tauri.conf.json`, `ui/index.html`,
  `src/main.rs` (stub split), `.gitignore` already covers target

Key shapes:

`Cargo.toml`:
```toml
[package]
name = "clipline-app"
version = "0.1.0"
edition.workspace = true
license.workspace = true

[target.'cfg(windows)'.dependencies]
clipline-capture = { path = "../../crates/clipline-capture" }
tauri = { version = "2", features = ["tray-icon"] }
tauri-plugin-global-shortcut = "2"
serde = { workspace = true }
serde_json = { workspace = true }

[build-dependencies]
tauri-build = { version = "2", features = [] }
```

`build.rs` (host-built; gate on the *target*):
```rust
fn main() {
    if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
        tauri_build::build();
    }
}
```

`tauri.conf.json`: identifier `io.clipline.app`, `app.withGlobalTauri: true`,
`build.frontendDist: "ui"`, one 480x360 window titled "Clipline".

`src/main.rs`:
```rust
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

#[cfg(not(windows))]
fn main() {
    eprintln!("clipline-app is Windows-only (capture/encode are platform-bound)");
}

#[cfg(windows)]
fn main() {
    app::run();
}

#[cfg(windows)]
mod app; // service + shell, Task 3
```

- [ ] Verify `cargo build -p clipline-app` compiles on Windows with a do-nothing `app::run()`,
and that the crate is a stub for non-windows targets (CI ubuntu proves it).
- [ ] Commit: `feat(app): clipline-app Tauri scaffold (windows-gated)`.

---

### Task 3: recorder service + shell wiring

**Files:** `apps/clipline-app/src/app.rs` (+ `service.rs`), `ui/index.html`

`service.rs` — the recorder thread (no Tauri types; talks via channels):
```rust
pub enum Cmd { Save, Stop }

pub enum Event {
    Status { recording: bool, segments: usize, buffered_s: f64, buffered_mb: f64 },
    Saved { path: PathBuf, seconds: f64 },
    Error(String),
}

pub fn spawn(opts: ServiceOptions) -> (Sender<Cmd>, Receiver<Event>) { /* thread::spawn(run) */ }

fn run(opts, rx, tx) {
    // device + clock + WgcCapture (monitor scaled ≤2560, or --window title)
    // + MftH264Encoder + WasapiLoopback → Recorder (ring budget from opts)
    // last_save_end: Option<f64> = None (smart no-overlap)
    loop {
        match rec.step() {
            Ok(true) => {}
            Ok(false) => break,
            Err(PipelineError::Capture(CaptureError::Timeout(_))) => {} // idle screen
            Err(e) => { tx.send(Event::Error(..)); break }
        }
        // ~1 Hz: tx.send(Event::Status{..}) (ring len / bytes / span)
        while let Ok(cmd) = rx.try_recv() {
            match cmd {
                Cmd::Stop => { rec.finish_stream(); return; }
                Cmd::Save => {
                    let path = clips_dir().join(format!("clip_{stamp}.mp4"));
                    match rec.save_replay(File::create(&path)?, opts.window_s, last_save_end) {
                        Ok((_, end)) => { last_save_end = Some(end); tx.send(Event::Saved{..}); }
                        Err(e) => tx.send(Event::Error(format!("nothing to save: {e}"))),
                    }
                }
            }
        }
    }
}
```
`clips_dir()` = `%USERPROFILE%\Videos\Clipline` (create_dir_all).

`app.rs` — Tauri shell:
- `tauri::Builder` + global-shortcut plugin; register **Alt+F10** → `cmd_tx.send(Cmd::Save)`.
- Tray: procedural 32x32 icon, menu = Save Replay / Quit; left-click shows the window.
- A forwarder thread pumps service `Event`s into webview events (`app.emit("status"| "saved" | "error", payload)`).
- `#[tauri::command] fn save_replay(state)` for the UI button → `Cmd::Save`.
- On exit (Quit/window close): `Cmd::Stop`, join the service thread.

`ui/index.html`: dark single page — recording dot, buffered seconds/MB, hotkey hint,
"Save Replay" button (`window.__TAURI__.core.invoke('save_replay')`), list of saved clips
(from `saved` events).

- [ ] Compile + clippy clean. Commit:
`feat(app): replay-buffer service, Alt+F10 hotkey, tray, status UI`.

---

### Task 4: manual e2e + gates

- [ ] `cargo run -p clipline-app` on the dev machine: tray appears, status window counts
buffered seconds; play A/V content; **Alt+F10** twice (second within the same buffer →
smart mode trims overlap); confirm files in `Videos\Clipline` play, ffprobe-sane, audio
audible. Record observed output in the commit body.
- [ ] `cargo test --workspace` + clippy zero warnings; push; **CI green on both OSes**
(ubuntu builds the stub — the whole point of the gating).
- [ ] `handoff.md`: milestone 5 done — Clipline is now a usable tray recorder; next per
ddoc §15 (FFmpeg matrix, per-process audio, library/timeline UI, event markers wiring).

---

## Out of scope (follow-ups)

- Installer/bundling, code signing, auto-update (ddoc §4) — `cargo run` is the deliverable.
- Settings UI (buffer length, bitrate, capture target picker), display-capture privacy
  warning (ddoc §9), WebView2-destroyed-when-minimized RAM trick (ddoc §4).
- League event markers wired into saved clips (clipline-events/lol exist; integration is its
  own milestone).
- Pause-on-idle pacing (repeat-last-frame) and disk-spill ring mode.
