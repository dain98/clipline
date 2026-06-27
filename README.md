<div align="center">

# 🎬 Clipline

**A lightweight, ad‑free, open‑source game recorder for Windows — with automatic in‑game event markers.**

[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#-license)
[![Platform: Windows 10/11](https://img.shields.io/badge/platform-Windows%2010%20%7C%2011-0078D6.svg)](#requirements)
[![Built with Rust](https://img.shields.io/badge/core-Rust-CE412B.svg)](https://www.rust-lang.org/)
[![UI: Tauri 2](https://img.shields.io/badge/ui-Tauri%202-24C8DB.svg)](https://tauri.app/)
[![No telemetry](https://img.shields.io/badge/telemetry-none-success.svg)](#-privacy--anti-cheat)

</div>

---

Clipline is a game recorder built around three commitments: **never inject code into your games** (so it avoids the injected‑capture pattern that anti‑cheats like Vanguard, EAC, and BattlEye flag), **never run ads, telemetry, or accounts** (so your clips and your machine stay yours), and **automatically mark the moments that matter** on the timeline — using official, local game APIs.

Under the hood it pairs a native **Rust** capture/encode core with a small **Tauri (WebView2)** UI: capture via **Windows.Graphics.Capture**, hardware encoding on **NVENC / AMF / QuickSync** (plus software **AV1**), and a crash‑safe MP4 writer. The result is a ShadowPlay‑style replay buffer with near‑zero gameplay impact — cross‑GPU, open source, and free.

> **Status:** `v0.1.14`, nightly — testing‑grade, not stable. A working tray recorder with a first‑party review/trim player, 31 development milestones deep. Windows‑only. [Download the installer](#-install) or build from source. The installer binary isn't Authenticode code‑signed yet, so Windows SmartScreen will warn on first run — Authenticode signing is [on the roadmap](#-roadmap). (The auto‑updater *is* signed; the two are different — see [Install](#-install).)

<!--
  SCREENSHOTS — drop a visual-first impression here, above the text/badges.
  For a recorder/editor, gamers want to *see* the flow before reading architecture.
  Recommended (commit images under docs/screenshots/ and uncomment):

  <div align="center">
    <img src="docs/screenshots/review-timeline.png" alt="Review player with event-marker timeline" width="800"><br>
    <em>Review player — scrub past automatic League event markers and trim losslessly.</em>
  </div>

  Four shots worth capturing: tray flow · replay save toast · marker timeline · trim UI.
  A short GIF of "save replay → markers → trim → export" near the top is even better.
-->

---

## 📥 Install

Download the latest **[nightly installer](https://github.com/dain98/clipline/releases)** (`Clipline_<version>_x64-setup.exe`) and run it. It installs per‑user, starts in the tray, and keeps itself up to date on the nightly channel.

On Windows, the standard installer embeds Microsoft's small WebView2 Evergreen bootstrapper. If WebView2 is missing or older than Clipline's minimum supported runtime, the installer may download the current runtime from Microsoft. Offline or Microsoft-blocked machines may need the WebView2 Runtime installed manually first. Windows 11 normally includes WebView2.

If WebView2 is unavailable or broken, Clipline starts a local browser fallback that uses the same first-party UI through a tokenized `127.0.0.1` connection. The fallback is intended for Windows 10 machines where WebView2 cannot be kept installed; it still requires a normal browser for the UI. The native Clipline process keeps running behind that browser UI, so the global Save Replay hotkey is still owned by the native global/low-level hook path and does not depend on the browser window staying focused.

To validate the fallback on a WebView2-removed Windows 10 machine, run `scripts\validate-fallback-client.ps1 -CliplineExe <path-to-Clipline.exe>`. On a dev machine that still has WebView2, add `-UseDebugMissingPreflight` to exercise the same startup fallback path and write an evidence JSON. The evidence verifies the native Save Replay hotkey hook is ready before fallback startup and that the auto-opened browser runs the shared UI JavaScript far enough to call `frontend_ready`. Add `-IncludeGlobalHotkeyProbe` to briefly focus Notepad and prove the configured Save Replay hotkey reaches the native hook while Clipline/browser is unfocused. When clips are present, it also includes a `/media-path` redirect plus a ranged `/media/{id}` playback probe, and verifies the `/events` SSE stream reaches a heartbeat.

**On signing — two different things, only one is done yet:**

- ✅ **The auto‑updater is signed.** Every update bundle carries a cryptographic signature that's verified against a public key committed in the repo before it's applied, so the update channel is tamper‑resistant.
- ⚠️ **The installer binary is *not* yet Authenticode code‑signed.** Authenticode is the separate, paid certificate Windows SmartScreen checks. Until Clipline has one, SmartScreen may warn on first launch — choose **More info → Run anyway**. This is the same warning every unsigned indie installer triggers; it isn't a malware detection. Authenticode signing is [on the roadmap](#-roadmap).

Prefer to build it yourself? See **Building from source** below.

---

## ✨ Features

- 🛡️ **Anti‑cheat safe by design** — capture happens at the desktop‑compositor level via Windows.Graphics.Capture. No DLL injection, no kernel driver, no memory reading — so Clipline avoids the injected‑capture pattern that anti‑cheats like **Riot Vanguard** (VALORANT), **Easy Anti‑Cheat**, and **BattlEye** flag. This is an auditable architectural property, not a vendor‑approval guarantee — see [Known limitations](#-known-limitations).
- 🏷️ **Automatic event markers** — Clipline polls **League of Legends'** official local *Live Client Data API* and drops timeline markers for kills, multikills, dragons, barons, towers, aces, and more — just the data the game already exposes locally, no account or injection.
- ⚡ **Lightweight & hardware‑accelerated** — a thin capture → encode → mux pipeline with hardware encoders (NVENC / AMF / QuickSync, plus software AV1) keeps gameplay impact in the low single digits, with a live RAM readout in the app.
- 🎞️ **Replay buffer + full‑session recording** — retroactively save the last N seconds with a hotkey (**Alt+F10** by default), ShadowPlay‑style, *and* optionally record full sessions per game — both fed by a single encoder.
- 🎮 **Game auto‑detection & auto‑recording** — register a game once (or use the built‑in **League** plugin) and Clipline switches capture to its window automatically, and can auto‑record full matches. It uses only Win32 window/process metadata — still zero injection.
- ✂️ **Built‑in review player & lossless trim** — a keyboard‑first review workspace with a zoomable, snapping timeline and a navigator. Scrub past event markers, set in/out points, and export **instantly and losslessly** via keyframe‑aligned stream copy — no re‑encode, no quality loss.
- 🎚️ **Multi‑source audio** — system/output loopback **and** microphone, with per‑source device selection, 0–200% gain, mono mixdown, and a live mic test monitor with a level meter. Mic is opt‑in.
- 🧱 **Crash‑safe Hybrid MP4** — records as a fragmented MP4 (so a crash or power loss doesn't nuke the recording) and finalizes to a standard, seekable MP4 on save. A/V sync is QPC‑anchored so clips stay in sync even under VRR/G‑Sync.
- 🔒 **No ads, no telemetry, no account, no watermark.** Optionally launches on Windows startup, minimized to the tray. Everything stays on your machine.

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

**Where Clipline is different:** it combines no‑injection anti‑cheat safety, automatic event markers for League, a local‑first privacy stance, and a permissive open‑source license — without locking you to one GPU vendor.

### Why not just use…?

- **OBS?** OBS is the power tool — but its low‑overhead Game Capture works by injecting into your game, the replay buffer is RAM‑only and fiddly to configure, and there's no automatic event tagging or built‑in trim. Clipline trades OBS's breadth for a no‑injection capture path, one‑hotkey replay save, and automatic League markers out of the box.
- **ShadowPlay?** Lowest overhead in the category, but **NVIDIA‑only**, ties you to an NVIDIA account, has a barely‑there editor, and tags no game events. Clipline runs on any GPU, needs no account, and marks the moments on the timeline.
- **Outplayed / Medal?** They have event markers, but via the Overwolf platform with ads, accounts, telemetry, and cloud uploads. Clipline keeps everything local — no ads, no account, no phone‑home — and reads the same data straight from League's official local API.

---

## 🧰 Tech stack

| Layer | Technology |
|---|---|
| **Core language** | Rust |
| **UI shell** | Tauri 2 + WebView2 (small footprint), vanilla HTML/CSS/JS — no npm/bundler |
| **Screen capture** | Windows.Graphics.Capture via `windows-rs`, D3D11 texture path |
| **Audio capture** | WASAPI loopback + mic capture; Opus encode via `audiopus` |
| **Video encode** | Media Foundation Transform (H.264, zero‑copy) **+** bundled FFmpeg subprocess (NVENC/AMF/QSV, SVT‑AV1) |
| **Container** | Custom Hybrid MP4 muxer (fragmented → finalized), codec‑aware, multi‑track, keyframe‑aligned trim |
| **Event source** | League Live Client Data API over `reqwest` + `tokio` |
| **Tests** | Rust unit/integration tests, `httpmock` for the LoL adapter, `boa_engine` for the pure JS player logic, a DOM‑contract guard, and `ffprobe` for real‑demuxer MP4 validation |

> FFmpeg is driven as a separate **LGPL** process (no GPL `libx264`/`libx265`), which keeps the first‑party code permissively licensed and the binary tiny. See [`THIRD-PARTY-NOTICES.md`](THIRD-PARTY-NOTICES.md).

---

## 🗂️ Project layout

The workspace is split into focused crates so platform‑agnostic logic stays testable on any OS and all `unsafe` Windows code is confined behind safe wrappers.

| Crate | Responsibility |
|---|---|
| [`clipline-capture`](crates/clipline-capture) | The capture + encode pipeline: WGC capture, WASAPI audio, the MFT H.264 encoder, the FFmpeg subprocess encoder, NV12 conversion, encoder probe/ranking, codec bitstream parsing, QPC clocking, AV‑sync validation. |
| [`clipline-mp4`](crates/clipline-mp4) | The Hybrid MP4 muxer (fragmented‑during‑capture → finalized‑on‑save), codec‑aware (H.264/HEVC/AV1), multi‑track, plus codec‑agnostic keyframe‑aligned stream‑copy trim. |
| [`clipline-buffer`](crates/clipline-buffer) | The replay ring: byte‑budgeted, GOP‑aligned segments with oldest‑first eviction and smart save‑window extraction; optional disk spill. |
| [`clipline-storage`](crates/clipline-storage) | Saved‑clip inventory, sidecar‑aware size accounting, and oldest‑first quota GC that protects fresh saves. |
| [`clipline-events`](crates/clipline-events) | The normalized event schema, game‑clock→recording anchor math, and marker sidecar models. |
| [`clipline-lol`](crates/clipline-lol) | The League of Legends Live Client adapter: HTTP client, polling, de‑dup, and normalization. |
| [`apps/clipline-app`](apps/clipline-app) | The Tauri 2 desktop shell: recorder service, global hotkey, tray, settings, library, game detection/plugins, and the first‑party review player. |

---

## 🛠️ Building from source

### Requirements
- **Windows 10 (1803+) or Windows 11**
- **[Rust](https://rustup.rs/) stable** toolchain (with `clippy`)
- **[WebView2 runtime](https://developer.microsoft.com/microsoft-edge/webview2/)** — the installer embeds Microsoft's Evergreen bootstrapper and may download/repair the runtime on Windows 10; preinstalled on most Windows 11 machines
- **FFmpeg** *(optional)* — only needed for **HEVC/AV1** recording and to run the full test suite. H.264 works with no extra dependencies via the OS Media Foundation encoder.

See **[Compatibility & tested configurations](docs/COMPATIBILITY.md)** for the GPU/encoder, Windows‑version, and per‑game support matrix.

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

## ⌨️ Shortcuts

| Action | Shortcut |
|---|---|
| **Save replay** (global) | `Alt+F10` *(rebindable to F‑keys or Middle/Mouse4/Mouse5 with Ctrl/Alt/Shift in Settings → Hotkeys)* |
| Play / pause | `Space` or `K` |
| Back / forward 5 s | `J` / `L` (hold `Shift` for 1 s) |
| Step / nudge | `←` `→` (10 frames) · `,` `.` (0.1 s) |
| Set trim in / out | `I` / `O` |
| Zoom in / out | `+` / `−` (scroll to zoom, `Shift`+scroll to pan) |
| Toggle sidebar | `F` |
| Close clip | `Esc` |

The in‑app **Keyboard shortcuts** panel lists the full set (edit points, snapping, fit‑to‑selection, and more).

---

## 🔐 Privacy & anti‑cheat

Clipline is built on a hard line: **it never injects DLLs, never loads a kernel driver, and never reads game memory.** Capture is done at the desktop‑compositor level (WGC), and event data is fetched only from local `127.0.0.1` endpoints. Nothing leaves your machine without an explicit action from you.

- **No telemetry, no analytics, no phone‑home.** Any future diagnostics will be strictly opt‑in and local.
- **No account required** to record or review clips.
- **Capture hygiene matters.** Because display capture records the whole monitor, Clipline prefers per‑window/per‑game capture and treats accidentally recording a password manager or a DM popup as a privacy bug.
- **Fully open source**, so every one of these claims is auditable.

---

## ⚠️ Known limitations

Clipline is **nightly, testing‑grade software**. Going in with clear expectations:

- **Not code‑signed.** The installer isn't Authenticode‑signed yet, so SmartScreen warns on first run (see [Install](#-install)).
- **Anti‑cheat compatibility is a design property, not a promise.** "No injection / no kernel driver / no memory reading" is architecturally true and auditable — that's the part Clipline controls. Whether any given anti‑cheat *permits* desktop‑compositor capture is the vendor's call, and policies can change without notice. Clipline avoids the failure mode most likely to trip a cheat; it can't guarantee any vendor will keep allowing every capture path.
- **Windows 10/11 only**, x64. No Linux/macOS, no 32‑bit, no DirectX‑exclusive‑fullscreen edge cases that WGC can't see.
- **League of Legends is the only built‑in event source today.** Other games record fine, but get no automatic markers yet (VALORANT, CS2, and more are [planned](#-roadmap)).
- **HEVC/AV1 needs FFmpeg and can't preview in‑app yet.** The WebView2 player decodes H.264; HEVC/AV1 clips require an external player until native decode lands.
- **Trim is keyframe‑aligned (lossless), not frame‑accurate.** In/out points snap to the nearest keyframe so export is an instant stream‑copy; frame‑accurate trim is [on the roadmap](#-roadmap).
- **Hardware coverage isn't exhaustively validated.** Encoder probing covers NVENC/AMF/QuickSync plus software AV1, but the matrix of GPU + driver + Windows build combinations is large — and only the AMD/RDNA2 path is verified live so far. See **[Compatibility & tested configurations](docs/COMPATIBILITY.md)** for the current matrix. If something misbehaves, a [bug report](https://github.com/dain98/clipline/issues/new?template=bug_report.yml) with your GPU, encoder, Windows build, game, and capture mode is the fastest path to a fix.

---

## 🗺️ Roadmap

Implemented today: WGC capture, hardware + AV1 encoding, replay buffer, full‑session recording, multi‑track audio, the review/trim player, custom‑game detection, League event markers and auto‑recording, disk quota/GC, startup‑on‑login, and a self‑updating nightly installer.

Planned (each gets its own design + TDD plan):

- **Authenticode code signing** for the installer binary to clear the SmartScreen warning (distinct from the already‑signed updater).
- **Auto‑clip on importance** — automatically save when a high‑importance event fires (marker importance is already tracked).
- **Frame‑accurate trim** — re‑encode only the boundary GOPs, keeping the instant lossless path as the default.
- **In‑app HEVC/AV1 playback** — a native FFmpeg decode path so the review player can preview codecs WebView2 can't decode on its own.
- **VALORANT support** — kill‑feed OCR over Clipline's own captured frames (no key, no injection).
- **More event adapters** — CS2 Game State Integration and other log/OCR‑based sources.
- **Per‑process audio loopback** and a display‑capture privacy warning.

---

## 🤝 Contributing

Contributions are welcome. The project follows a **plan‑driven, test‑first** workflow — each milestone has a design doc under [`docs/superpowers/plans/`](docs/superpowers/plans) and is built strictly failing‑test‑first. Conventions worth knowing:

- Workspace tests green and `cargo clippy --workspace --all-targets -- -D warnings` clean on both Ubuntu and Windows CI.
- Platform‑neutral logic stays neutral and testable on both OSes; Windows‑only code lives behind `#[cfg(windows)]`, and all `unsafe` is confined to the `windows/` modules behind safe wrappers.
- Conventional commits (`feat(capture): …`), one logical change per commit.

Read [`ddoc.md`](ddoc.md) for the product/architecture source of truth and [`handoff.md`](handoff.md) for the current development state, sharp edges, and what's next.

---

## 📄 License

Clipline's first‑party code is dual‑licensed under **MIT OR Apache‑2.0** — pick whichever you prefer. It also loads a dynamically‑linked **LGPL** build of FFmpeg as a separate process for HEVC/AV1 encoding. See [`THIRD-PARTY-NOTICES.md`](THIRD-PARTY-NOTICES.md) for attribution, source‑code pointers, and codec/patent notes.

---

<div align="center">

**Clipline** — record the game, mark the moment. No ads, no injection.

</div>
