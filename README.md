<div align="center">

# 🎬 Clipline

**A lightweight, ad‑free, open‑source game recorder for Windows — with automatic in‑game event markers and zero anti‑cheat risk.**

[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#-license)
[![Platform: Windows 10/11](https://img.shields.io/badge/platform-Windows%2010%20%7C%2011-0078D6.svg)](#requirements)
[![Built with Rust](https://img.shields.io/badge/core-Rust-CE412B.svg)](https://www.rust-lang.org/)
[![UI: Tauri 2](https://img.shields.io/badge/ui-Tauri%202-24C8DB.svg)](https://tauri.app/)
[![No telemetry](https://img.shields.io/badge/telemetry-none-success.svg)](#-privacy--anti-cheat)

</div>

---

Clipline is what you get when you build a game recorder around three commitments most tools won't make all at once: **never inject code into your games** (so it's safe with Vanguard, EAC, and BattlEye), **never run ads, telemetry, or accounts** (so your clips and your machine stay yours), and **automatically mark the moments that matter** on the timeline — using official, local game APIs instead of the Overwolf platform.

It pairs a native **Rust** capture/encode core with a tiny **Tauri (WebView2)** UI, defaulting to **Windows.Graphics.Capture** with hardware encoders (**NVENC / AMF / QuickSync**, plus software **AV1**). The result is a ShadowPlay‑style replay buffer with near‑zero gameplay impact — but cross‑GPU, open source, and free of the baggage that makes Outplayed, Medal, and OBS frustrating to live with.

> **Status:** `v0.1.0` — a working tray recorder with a first‑party review/trim player, 25 development milestones deep. Windows‑only. Built from source today; signed installer and auto‑update are on the roadmap.

---

## ✨ Highlights

- 🛡️ **Anti‑cheat safe by design** — no DLL injection, no kernel driver, no memory reading. Capture happens at the desktop‑compositor level via Windows.Graphics.Capture (WGC). This is the single biggest architectural bet, and the only one that works reliably with **Riot Vanguard** (VALORANT), **Easy Anti‑Cheat**, and **BattlEye** — the exact place OBS's injection‑based Game Capture breaks.
- 🏷️ **Automatic timeline event markers** — Clipline polls **League of Legends'** official local *Live Client Data API* (`127.0.0.1:2999`) and drops markers on your clips for kills, multikills, dragons, barons, towers, aces, and more. No Overwolf, no injection, no account — just the data the game already exposes to you locally.
- ⚡ **Lightweight, hardware‑accelerated** — a thin custom pipeline (capture → encode → mux) instead of embedding heavyweight libobs. Hardware encoding (NVENC/AMF/QuickSync) keeps gameplay impact in the low single‑digit percent, and the Tauri UI sips RAM compared to Electron‑based tools. A live RAM readout sits right in the app so you can watch it.
- 🎞️ **Instant replay buffer + full‑session recording** — retroactively save the last N seconds with a hotkey (**Alt+F10** by default), ShadowPlay‑style, *and* optionally record full sessions per game — both fed by a single encoder, so older GPUs don't pay for two encode passes.
- ✂️ **Built‑in review player & lossless trimmer** — open any clip in a keyboard‑first review workspace, scrub the timeline with event markers, set in/out points, and export a trimmed clip **instantly and losslessly** via keyframe‑aligned stream copy (no re‑encode, no quality loss).
- 🎚️ **Multi‑source audio** — system/output loopback **and** microphone, with per‑source device selection, 0–200% gain, mono mixdown, and a live mic test monitor with a level meter. Mic is opt‑in for privacy.
- 🎯 **Custom game auto‑detection** — register a game once; Clipline watches for its window and automatically switches capture to it (and back) — using only Win32 window/process metadata, still zero injection.
- 🧱 **Crash‑safe Hybrid MP4** — records as a fragmented MP4 (each fragment independently decodable, so a BSOD or power loss doesn't nuke the recording) and finalizes to a standard, seekable MP4 on save. AV‑sync is QPC‑anchored across video and audio so clips stay in sync even under VRR/G‑Sync.
- 🔒 **No ads. No telemetry. No account. No watermark — ever.** Sustained by donations and a permissive license, not by your attention or your data.

---

## 🥊 How Clipline compares

|                         | **Clipline** | OBS Studio | Outplayed | Medal.tv | NVIDIA ShadowPlay | Steam Recording |
|-------------------------|:---:|:---:|:---:|:---:|:---:|:---:|
| **Platform**            | Win 10/11 | Cross‑platform | Win (Overwolf) | Win / mobile | NVIDIA GPUs only | Win (Steam games) |
| **Overhead**            | 🟢 Lowest tier | 🔴 High / complex | 🟠 Medium–high | 🟡 Low–med | 🟢 Lowest | 🟢 Low |
| **Anti‑cheat safe**     | ✅ No injection | ⚠️ Game Capture injects | ⚠️ Mixed | ✅ Mostly | ✅ Driver‑level | ✅ No injection |
| **Replay buffer**       | ✅ RAM + disk | ✅ RAM only | ✅ | ✅ | ✅ Instant Replay | ✅ |
| **Event markers**       | ✅ **Official local APIs (LoL)** | ❌ | ✅ Overwolf GEP | ✅ ~15 games | ❌ | ✅ Steam Timeline (not LoL/VAL) |
| **Built‑in editor**     | ✅ Lossless trim | ❌ | ✅ | ✅ (browser) | ▫️ Minimal | ▫️ Trim only |
| **Ads / model**         | 🟢 Donations | 🟢 Free | 🔴 Ads + freemium | 🟡 Cloud / social | 🟢 Free | 🟢 Free |
| **Telemetry / account** | 🟢 None | 🟢 None | 🔴 Yes | 🔴 Yes | 🟡 NVIDIA account | 🟡 Steam |
| **Open source**         | ✅ MIT/Apache | ✅ GPLv2 | ❌ | ❌ | ❌ | ❌ |
| **Vendor lock‑in**      | 🟢 Any GPU | 🟢 Any GPU | 🟢 Any GPU | 🟢 Any GPU | 🔴 NVIDIA only | 🟢 Any GPU |

**Where Clipline is genuinely different:** it's the only tool that combines *no‑injection anti‑cheat safety*, *automatic event markers for games that don't integrate the Steam Timeline API (League, and VALORANT on the roadmap)*, a *local‑first privacy stance*, and a *permissive open‑source license* — all in a footprint that targets ShadowPlay's class without locking you to one GPU vendor or an ad‑driven platform.

---

## 🚀 Features in detail

### Capture
- **Windows.Graphics.Capture (WGC)** as the primary engine — DWM‑level, cross‑GPU, HDR‑capable, requires no injection. Works with anti‑cheat titles where injection‑based capture is blocked.
- **Per‑monitor, per‑window, or display‑region capture.** Pick a display, capture a specific game window (excluding title bar/borders via the client rect), or draw a precise pixel region on a virtual‑desktop map with drag/resize handles, numeric fields, and align/snap actions.
- **GPU‑side frames** — captured textures stay on the GPU and are converted (BGRA→NV12) and scaled in the D3D11 video processor before encode, avoiding costly CPU round‑trips.
- **Adaptive to window resizes** — the frame pool tracks per‑frame content size and the converter rescales into the fixed output track instead of artifacting.

### Encoding
- **Hardware‑first encoder matrix** probed at startup and ranked by merit: **NVENC → AMF → QuickSync → software SVT‑AV1**, with Microsoft's hardware H.264 MFT on the zero‑copy path.
- **Codec choice:** H.264 (maximum compatibility), **HEVC**, and **AV1** (40%‑ish bitrate savings for the same quality on supported silicon). Clipline writes the correct codec boxes (`avc1`/`hvc1`/`av01` with `avcC`/`hvcC`/`av1C`) and parses parameter sets straight from the bitstream.
- **Two encoder backends, one abstraction:** a zero‑copy Media Foundation Transform path for H.264, and a bundled **FFmpeg subprocess** (`ffmpeg.exe`, fed raw NV12 over a pipe) for NVENC/AMF/QSV HEVC/AV1 and software AV1. The subprocess approach was a deliberate choice over linking libavcodec — zero unsafe FFI, version‑robust, and the cleanest LGPL boundary.
- **Smart auto‑selection** never picks a codec your in‑app player can't decode; explicit HEVC/AV1 picks carry a clear "limited playback" caveat.

### Replay buffer & recording
- **Instant replay:** a rolling, GOP‑aligned ring buffer of *encoded* video + audio in RAM. Hit the save hotkey to flush the trailing window to disk from the last clean keyframe — clips always start cleanly.
- **Smart no‑overlap saves** so back‑to‑back saves don't re‑clip the same footage.
- **Full‑session recording per game:** mark a game as "full session" and Clipline opens a second, shared‑encoder MP4 sink that keeps footage even after the replay ring evicts it — while Save Replay keeps working off the same ring. In‑progress sessions use a `.mp4.recording` suffix so the library never opens a half‑written file.

### Audio
- **System/output loopback** via WASAPI plus optional **microphone** capture, each with selectable default or explicit endpoints.
- **Per‑source gain (0–200%), mono mixdown,** and automatic resampling to Opus's 48 kHz timeline.
- When both output and mic are on, they're **mixed into one Opus track** so every player (in‑app or external) hears both.
- **Live mic test monitor** — play your selected mic back through the app with a real‑time level meter before you record.

### Review player & editor
- A **two‑pane review workspace** with no native video chrome: a dimmed‑outside‑trim timeline with draggable in/out edges, **kind‑colored event marker chips** (kill ✕ / spree ★ / objective ◆ / structure ▣ / info •), a labeled time ruler, and an overlay transport that fades while playing and pins while paused.
- **Keyboard‑first review:** `Space`/`K` play‑pause, `←`/`→` (or `J`/`L`) jump 5 s (`Shift` = 1 s), `,`/`.` nudge 0.1 s, `I`/`O` set trim at the playhead, `M`/`Shift+M` add markers, `F` toggles the sidebar, `Esc` closes.
- **Lossless trim/export:** keyframe‑aligned stream copy writes a fresh, finalized MP4 as a sibling of the source — instant, no quality loss, marker sidecars cropped to match. (The in point snaps backward and the out point forward to keyframes, so the kept range may be slightly wider than requested.)

### Library & storage
- **Session‑foldered library:** saves land in `Videos\Clipline\<session>\` — one folder per recorder run, plus a dedicated folder per detected League match. The library groups by session, labels clips human‑first ("Jun 11 · 10:25 PM" + a marker digest), and tags full‑session recordings.
- **Configurable media folder, disk quota, and oldest‑first auto‑GC** that protects the clip you just saved.
- Reveal a clip in Explorer, open its folder, or delete with an in‑app confirmation dialog.

### Event markers (the differentiator)
- **League of Legends adapter** polls the documented Live Client Data API (~1 Hz, monotonic event de‑dup, quiet retries outside matches) and normalizes 11+ official event types into a common schema.
- **Timeline anchoring** maps each event's game‑clock time onto the recording timeline, re‑sampled every poll so pauses/remakes self‑correct. Markers are written as `<clip>.markers.json` sidecars, re‑based to clip time on save.
- *Design principle:* only ever use official, locally‑exposed data the player can already see — never injection, never memory reading, never Overwolf.

---

## 🧰 Tech stack

| Layer | Technology |
|---|---|
| **Core language** | Rust (memory‑safe for a 24/7 background recorder) |
| **UI shell** | Tauri 2 + WebView2 (tiny footprint vs Electron), vanilla HTML/CSS/JS — no npm/bundler |
| **Screen capture** | Windows.Graphics.Capture (WGC) via `windows-rs`, D3D11 texture path |
| **Audio capture** | WASAPI loopback + mic capture; Opus encode via `audiopus` |
| **Video encode** | Media Foundation Transform (H.264, zero‑copy) **+** bundled FFmpeg subprocess (NVENC/AMF/QSV, SVT‑AV1) |
| **Container** | Custom Hybrid MP4 muxer (fragmented → finalized), codec‑aware, multi‑track, keyframe‑aligned trim — hand‑rolled, no external mux dependency |
| **Event source** | League Live Client Data API over `reqwest` + `tokio` |
| **Global hotkey & tray** | `tauri-plugin-global-shortcut`, Tauri tray icon |
| **Tests** | Rust unit/integration tests, `httpmock` for the LoL adapter, `boa_engine` to unit‑test the pure JS player logic from Rust, a DOM‑contract guard test, and `ffprobe` for real‑demuxer MP4 validation |

> **Encoders are subprocesses, not links:** Clipline drives a bundled **LGPL** FFmpeg build (no GPL `libx264`/`libx265`) as a separate process. This keeps the first‑party code permissively licensed and the binary tiny. See [`THIRD-PARTY-NOTICES.md`](THIRD-PARTY-NOTICES.md).

---

## 🏗️ Architecture

```
                   ┌─────────────────────────────────────────┐
                   │           UI Layer (Tauri / WebView2)     │
                   │   Library · Timeline+Markers · Review     │
                   │   Player/Trim · Settings · Hotkeys · Tray │
                   └───────────────▲───────────────────────────┘
                                   │ Tauri IPC (typed commands/events)
                   ┌───────────────┴───────────────────────────┐
                   │            Core Service (Rust)             │
   ┌────────────┐  │  ┌──────────┐   ┌──────────────────────┐  │
   │  Capture   │──┼─▶│  Encode  │──▶│  Replay Ring (RAM)    │  │
   │  WGC/DXGI  │  │  │ NVENC/AMF│   │  + Full‑session sink  │  │
   └────────────┘  │  │ QSV/AV1  │   └──────────┬───────────┘  │
   ┌────────────┐  │  └──────────┘              │              │
   │   Audio    │──┼──────────────▶  ┌──────────▼───────────┐  │
   │ WASAPI +   │  │                 │  Hybrid MP4 Storage   │  │
   │   mic      │  │                 │  quota · GC · folders │  │
   └────────────┘  │  ┌──────────────────────────────────┐  │  │
                   │  │  Event Ingestion (LoL :2999 → …)  │──┼──┘
                   │  └──────────────────────────────────┘  │
                   │     Normalized events → timeline sync   │
                   └─────────────────────────────────────────┘
```

The workspace is split into focused crates so the platform‑agnostic logic stays testable on any OS and all `unsafe` Windows code is confined behind safe wrappers:

| Crate | Responsibility |
|---|---|
| [`clipline-capture`](crates/clipline-capture) | The capture + encode pipeline. WGC capture, WASAPI audio, the MFT H.264 encoder, the FFmpeg subprocess encoder, NV12 conversion, the encoder probe/ranking, codec bitstream parsing, QPC clocking, and AV‑sync validation. |
| [`clipline-mp4`](crates/clipline-mp4) | The Hybrid MP4 muxer: fragmented‑during‑capture → finalized‑on‑save, codec‑aware (H.264/HEVC/AV1), multi‑track, plus codec‑agnostic keyframe‑aligned stream‑copy trim. |
| [`clipline-buffer`](crates/clipline-buffer) | The replay ring: byte‑budgeted, GOP‑aligned segments with oldest‑first eviction and smart save‑window extraction; optional disk spill. |
| [`clipline-storage`](crates/clipline-storage) | Saved‑clip inventory, sidecar‑aware size accounting, and oldest‑first quota GC that protects fresh saves. |
| [`clipline-events`](crates/clipline-events) | The normalized event schema, game‑clock→recording anchor math, and marker sidecar models. |
| [`clipline-lol`](crates/clipline-lol) | The League of Legends Live Client adapter: HTTP client, polling, de‑dup, and normalization to the common event schema. |
| [`apps/clipline-app`](apps/clipline-app) | The Tauri 2 desktop shell: recorder service thread, global hotkey, tray, settings, library, game detection, memory metering, and the first‑party review player. |

---

## 🛠️ Building from source

### Requirements
- **Windows 10 (1803+) or Windows 11**
- **[Rust](https://rustup.rs/) stable** toolchain (with `clippy`)
- **[WebView2 runtime](https://developer.microsoft.com/microsoft-edge/webview2/)** — preinstalled on Windows 11; Windows 10 may need the Evergreen runtime
- **FFmpeg** *(optional)* — only needed for **HEVC/AV1** recording and to run the full test suite. H.264 works with no extra dependencies via the OS Media Foundation encoder.

### Build & run

```powershell
git clone https://github.com/dain98/clipline.git
cd clipline
cargo run -p clipline-app
```

That launches the tray app. Settings persist to `%APPDATA%\Clipline\settings.json`; clips land in `Videos\Clipline\` by default (configurable in Settings → Storage).

### Optional CLI flags

| Flag | Effect |
|---|---|
| `--window <title substring>` | Capture a single window instead of the primary monitor |
| `--disk-quota-gb <n>` | Override the saved storage quota for this launch (`0` disables GC) |
| `--lol-url <url>` | Point the League marker poller at a mock server (for testing) |

### HEVC / AV1 encoding

Clipline looks for an **LGPL‑shared** FFmpeg build (e.g. from [BtbN/FFmpeg‑Builds](https://github.com/BtbN/FFmpeg-Builds) — it ships SVT‑AV1 and the GPU vendor encoders, but no GPL `libx264`/`libx265`). Search order: the `CLIPLINE_FFMPEG` env override → the executable's directory → `%APPDATA%\Clipline\ffmpeg` → `PATH`. H.264 recording needs none of this.

### Tests

```powershell
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Real device tests (WGC/MFT/WASAPI) self‑skip on CI runners with no GPU/audio hardware and run live on a real Windows machine. MP4 tests self‑skip without `ffprobe` on `PATH`.

---

## ⌨️ Default shortcuts

| Action | Shortcut |
|---|---|
| **Save replay** (global) | `Alt+F10` *(rebindable to F‑keys + Ctrl/Alt/Shift in Settings → Hotkeys)* |
| Play / pause | `Space` or `K` |
| Jump ±5 s | `←` / `→` (or `J` / `L`) — hold `Shift` for ±1 s |
| Nudge ±0.1 s | `,` / `.` |
| Set trim in / out | `I` / `O` |
| Add marker | `M` (`Shift+M` for the other team/variant) |
| Toggle sidebar / focus | `F` |
| Close clip / settings | `Esc` |

---

## 🔐 Privacy & anti‑cheat

Clipline is built on a hard line: **it never injects DLLs, never loads a kernel driver, and never reads game memory.** Capture is done at the desktop‑compositor level (WGC), and event data is fetched only from local `127.0.0.1` endpoints. Nothing leaves your machine without an explicit action from you.

- **No telemetry, no analytics, no phone‑home.** Any future diagnostics will be strictly opt‑in and local.
- **No account required.** (Riot RSO would only ever be involved if you opt into a future VALORANT post‑match enrichment feature, bring‑your‑own‑key.)
- **Capture hygiene matters.** Because display capture records the whole monitor, Clipline prefers per‑window/per‑game capture and treats accidentally recording a password manager or a DM popup as a privacy bug, not a cosmetic one.
- **Fully open source**, so every one of these claims is auditable — the structural opposite of closed, ad‑driven recorders.

---

## 🗺️ Roadmap

Implemented today: WGC capture, hardware + AV1 encoding, replay buffer, full‑session recording, multi‑track audio, the review/trim player, custom‑game detection, disk quota/GC, and League event markers.

Planned (each gets its own design + TDD plan):

- **Auto‑clip on importance** — automatically save when a high‑importance event fires (marker importance is already tracked).
- **Frame‑accurate trim** — re‑encode only the boundary GOPs, keeping the instant lossless path as the default.
- **In‑app HEVC/AV1 playback** — a native FFmpeg decode path so the review player can preview codecs WebView2 can't decode on its own.
- **VALORANT support** — kill‑feed OCR over Clipline's own captured frames (no key, no injection), with optional opt‑in post‑match enrichment.
- **More event adapters** — CS2 Game State Integration and other log/OCR‑based sources.
- **Per‑process audio loopback**, display‑capture privacy warnings, a signed installer, and auto‑update.

---

## 🤝 Contributing

Contributions are welcome. The project follows a **plan‑driven, test‑first** workflow — each milestone has a design doc under [`docs/superpowers/plans/`](docs/superpowers/plans) and is executed strictly failing‑test‑first. Conventions worth knowing before you start:

- Workspace tests green and `cargo clippy --workspace --all-targets -- -D warnings` clean on both Ubuntu and Windows CI.
- Platform‑neutral logic stays neutral and testable on both OSes; Windows‑only code lives behind `#[cfg(windows)]`, and all `unsafe` is confined to the `windows/` modules behind safe wrappers.
- Conventional commits (`feat(capture): …`), one logical change per commit.

Read [`ddoc.md`](ddoc.md) for the product/architecture source of truth and [`handoff.md`](handoff.md) for the current development state, sharp edges, and what's next.

---

## 📄 License

Clipline's first‑party code is dual‑licensed under **MIT OR Apache‑2.0** — pick whichever you prefer. This is deliberate: a permissive, contributor‑friendly license, the opposite of OBS's GPL copyleft, and a conscious choice to avoid libobs (and its injection‑based capture) entirely.

Clipline additionally relies on a dynamically‑loaded **LGPL** build of FFmpeg, invoked as a separate process. See [`THIRD-PARTY-NOTICES.md`](THIRD-PARTY-NOTICES.md) for attribution, source‑code pointers, and the codec/patent notes.

---

<div align="center">

**Clipline** — record the game, mark the moment. No ads, no injection, no nonsense.

</div>
