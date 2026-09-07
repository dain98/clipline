# Experimental DWM window capture

This standalone probe tests border-free window capture using an undocumented DWM surface
export. It does not change Clipline, inject into the target, run WGC, or fall back to display
capture. Use it only on a window you intend to record. Snapshots and window titles stay local.

From an extracted probe ZIP, open PowerShell in that directory:

```powershell
.\dwm_probe.exe --list
.\dwm_probe.exe --hwnd 123456 --seconds 30 --fps 60 --out league-test
```

Replace `123456` with the first-column HWND for the actual game window. Alternatively use
`--window "unique title"`; ambiguous matches fail instead of choosing an arbitrary window.
Output directories must be new. `--help` describes limits and defaults. Stop with Ctrl+C if
needed (a completed summary is not guaranteed after interruption).

For each Windows 10 22H2 game test:

1. Keep the game animating. Check there is no yellow capture border from this probe.
2. Cover part of the game with another window around halfway through the run. Inspect
   `middle.bmp`: it should show the game, not the overlapping window, with current motion.
3. Repeat for windowed and borderless fullscreen, then resize, minimize/restore, and move to
   another monitor. Test exclusive fullscreen separately; a minimized game may stop rendering.

Inspect `first.bmp`, `middle.bmp`, and `last.bmp` for correct colors, fresh content, and
partial/black frames. `samples.csv` records read time, dimensions, format, opaque DWM update
IDs, resource changes, pixel hashes, and errors. `summary.txt` aggregates results. Record
Windows edition/build, GPU/driver, game, render mode, and actual on-screen behavior alongside
the files. Review snapshots before sharing them.

The probe supports 32-bit SDR surfaces only; HDR, unusual layouts, protected windows, failed
capture-affinity checks, and keyed-mutex surfaces are rejected. Full compositor-surface
snapshots can include window padding/non-client pixels; game-client cropping is not implemented.
Matching adapter selection is implemented but hybrid-GPU behavior still needs live validation.

**Read rate is not game FPS.** Polling can return duplicate/stale frames. Changed hashes include
dimension changes and do not prove tear-free capture. `read_ms` includes surface acquisition,
GPU-to-CPU copying, and BMP packing; CPU hashing and disk writes also affect total sampling rate.
This intentionally uses CPU readback to inspect pixels. No encoder integration or producer
synchronization guarantee is claimed. Map waits for the probe's copy, not a documented DWM
frame boundary. Do not promote this to the recording path on screenshot evidence alone.

Build and run from the repository:

```powershell
cargo test -p clipline-capture --example dwm_probe
cargo build -p clipline-capture --release --example dwm_probe
.\target\release\examples\dwm_probe.exe --list
```

For a controlled local fixture, run `powershell -NoProfile -File scripts/dwm-probe-target.ps1`
in another terminal, then capture `--window "Clipline DWM Probe Target"`. The fixture animates
a green square with red/blue reference blocks, covers it with a magenta window at 4 seconds,
resizes at 12 seconds, minimizes at 15, restores at 18, and closes at 28. An 8-second probe
started promptly exercises overlap; a 22-second run exercises resize/minimize/recovery.

Research and API limitations: [Windows 10 report](research/2026-09-07-windows-10-support.md).
