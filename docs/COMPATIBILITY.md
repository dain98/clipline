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
| **Microsoft software H.264 (`h264_mf`)** | ✅ Verified live | Last-resort FFmpeg path for VMs and software-only display adapters. Reads WGC BGRA textures back, converts them to NV12 on the CPU, and explicitly sets `-hw_encoding 0`. Verified in a Proxmox Windows 11 VM using Microsoft Basic Display Adapter at 1280×800/60 FPS. |
| **Software HEVC** | ❌ Not shipped | The bundled FFmpeg is an **LGPL** build with no GPL `libx264`/`libx265`. Software H.264 comes from Windows Media Foundation; there is no corresponding software HEVC fallback. |

Encoder selection probes at startup (H.264 first for compatibility, then
NVENC → AMF → QuickSync → software AV1 → software H.264) and test‑encodes every
device-dependent backend, including Media Foundation software H.264. A
compiled‑but‑unusable encoder is dropped rather than offered.

The VM fallback needs no PCI passthrough, virtual GPU feature, or IOMMU setting;
WGC and the Windows Media Foundation H.264 encoder are sufficient. It trades GPU
requirements for CPU work, so lower the output resolution or FPS if recording
affects the guest workload. The LGPL FFmpeg runtime must be present because the
fallback is implemented as a separate subprocess.

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

If you run Clipline on hardware or a Windows build not listed above, use
**Settings > Support** to prepare and explicitly send a private diagnostic report
(even just "it works"). General Clipline reports are not posted to a public issue
tracker and are not sent to Clipline Cloud. The most useful descriptions mention:

- **GPU** model and **driver version**
- **Encoder** Clipline selected (shown in Settings)
- **Windows** edition + build number (`winver`)
- **Game** and **capture mode** (replay buffer vs. full session, per‑window vs. monitor)
- What failed and the approximate time; the prepared package already previews the
  relevant sanitized logs and system details before you confirm sending it.

Verified by maintainers; community reports are tracked in issues until confirmed.
