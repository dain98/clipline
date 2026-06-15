# Compatibility & tested configurations

Clipline is **nightly, testing‑grade software**, and its hardware/OS matrix is
still small and honest. This page tracks what has actually been exercised, what
is implemented but not yet verified on real hardware, and what the community has
reported. It is a living document — see [Help us fill this in](#help-us-fill-this-in).

> **How to read this:** "Verified live" means a maintainer ran the real path on
> real hardware. "Implemented, not hardware‑verified" means the code path exists
> and is unit/mock‑tested, but no one has confirmed it on a physical device yet.
> Neither is a guarantee for your exact setup — capture/encode behavior depends
> on GPU model, driver version, and Windows build.

## GPUs / encoders

| GPU / encoder | Status | Notes |
|---|---|---|
| **AMD RX 6700 XT (RDNA2)** — AMF H.264, SVT‑AV1 | ✅ Verified live | Primary dev/test rig (5120×1440 primary display). AMF H.264 via the Media Foundation Transform path and software SVT‑AV1 (FFmpeg subprocess) are exercised here. |
| **NVIDIA — NVENC (H.264 / HEVC / AV1)** | 🟡 Implemented, not hardware‑verified | Probe + arg paths exist and are mock‑tested; not yet confirmed on a physical NVIDIA GPU. NVENC arg tuning is unvalidated. |
| **Intel — QuickSync (QSV)** | 🟡 Implemented, not hardware‑verified | Same as NVENC: code path present, not yet run on real Intel hardware. |
| **Software H.264 / HEVC** | ❌ Not shipped | The bundled FFmpeg is an **LGPL** build with no GPL `libx264`/`libx265`. The only software tier is SVT‑AV1. (Microsoft's software H.264 MFT exists as a not‑yet‑wired last resort.) |

Encoder selection probes hardware at startup (H.264 first for compatibility,
then NVENC → AMF → QuickSync → software AV1) and test‑encodes each backend, so a
compiled‑but‑unusable encoder is dropped rather than offered.

> ⚠️ **About performance figures.** Widely circulated "zero FPS impact" numbers
> trace to anecdotal, methodology‑free posts. Methodical testing measures a
> small‑but‑real cost — budget **~3–6%** for NVENC/QSV capture and slightly more
> for AMD AMF. Treat all figures as directional and re‑benchmark on your hardware.

## Windows versions

| Version | Status | Notes |
|---|---|---|
| **Windows 11** | ✅ Supported | WebView2 preinstalled; the WGC yellow capture border is suppressible. |
| **Windows 10, build 20348+** | ✅ Supported | WGC border suppression and per‑application audio loopback are available here. |
| **Windows 10, 1803 – pre‑20348** | 🟡 Minimum supported | WGC requires 1803+. Older builds show a non‑suppressible **yellow capture border** (a visible difference from ShadowPlay) and fall back to full‑system audio loopback. |
| **Windows 10 < 1803, any 32‑bit, Linux, macOS** | ❌ Unsupported | Clipline is Windows 10/11, x64 only. |

## Games

| Game | Capture / recording | Automatic event markers |
|---|---|---|
| **League of Legends** | ✅ Verified | ✅ Built‑in plugin — polls the official local *Live Client Data API* (`127.0.0.1:2999`) for kills, multikills, aces, dragons, barons, towers, inhibitors, herald, first blood, and more. Enabled by default. |
| **Any other Win32 game** | 🟡 Architecturally supported | Register the game once (or it's auto‑detected from window/process metadata) and Clipline records it — but with **no automatic markers** yet. |
| **VALORANT, CS2, others** | 🟡 Records, no markers | Event adapters are [planned](../README.md#-roadmap) (VALORANT kill‑feed OCR, CS2 Game State Integration). |

Game detection uses only Win32 window/process metadata — no injection, no memory
reading — so any game that renders to a normal window is capturable.

## Help us fill this in

If you run Clipline on hardware or a Windows build not listed above, a quick
[bug report](https://github.com/dain98/clipline/issues/new?template=bug_report.yml)
(even just "it works") helps grow this matrix. The most useful reports include:

- **GPU** model and **driver version**
- **Encoder** Clipline selected (shown in Settings)
- **Windows** edition + build number (`winver`)
- **Game** and **capture mode** (replay buffer vs. full session, per‑window vs. monitor)
- Relevant **logs**

Verified by maintainers; community reports are tracked in issues until confirmed.
