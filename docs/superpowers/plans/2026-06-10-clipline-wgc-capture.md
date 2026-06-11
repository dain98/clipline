# Clipline WGC Capture (Windows Platform Layer, Part 1) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The first real implementation behind the platform traits — Windows.Graphics.Capture
(WGC) monitor + window capture as a `CaptureEngine` (ddoc §3/§8: DWM-level, **no injection**),
frames staying GPU-side as D3D11 textures, every frame stamped with `pts_s` derived from WGC
`SystemRelativeTime` against a QPC capture-start origin (ddoc §6 "Clocking & A/V sync").
Verified by a smoke example run manually on the dev machine (CI runners have no desktop
session for WGC) reporting resolution and measured fps.

**Architecture:** Pure timing math (`RelativeClock`: 100 ns ticks → `pts_s`, QPC → 100 ns
conversion) lives platform-neutral in `clock.rs` so ubuntu CI tests it. All COM/WinRT unsafe
is confined to a new `#[cfg(windows)]` `windows/` module: `wgc.rs` builds D3D11 device →
`GraphicsCaptureItem` (`CreateForMonitor` primary / `CreateForWindow`) → free-threaded
`Direct3D11CaptureFramePool`; the `FrameArrived` handler copies each frame's texture (pool
surfaces are recycled) and sends `(texture, ticks)` over an mpsc channel; `next_frame()`
receives with a timeout, satisfying the pull-model `CaptureEngine` contract. `FrameData`
gains a `Gpu(ID3D11Texture2D)` variant behind `#[cfg(windows)]`; `CaptureError` gains
`Init`/`Timeout` variants (neutral-side change, additive). Border suppression
(`IsBorderRequired`) is best-effort — Win11/20348+ only (ddoc Caveats).

**Tech Stack:** `windows` crate 0.62 (windows-rs) under `[target.'cfg(windows)'.dependencies]`.
No FFmpeg yet. robmikh's windows-rs capture samples are the reference pattern.

**Environment notes:** Windows 11 dev machine with GPU and desktop session — device tests and
the smoke example run for real here. CI (ubuntu + windows) compiles everything; device tests
self-skip when capture is unsupported (skip-if-absent pattern, like the ffprobe e2e tests).
Commits end with `Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>`.

> **windows-rs drift note:** the code below targets `windows = "0.62"`. Exact signatures
> (e.g. `Ref<'_, T>` event-handler params, `BindFlags` as `u32`) sometimes shift between
> releases — adjust at compile time; the structure and safety boundaries are the contract.

---

### Task 1: `RelativeClock` — tick→pts math (platform-neutral)

WGC stamps frames with `SystemRelativeTime` (a QPC-derived `TimeSpan`, 100 ns units); WASAPI
positions are also QPC-based 100 ns. Both clocks will diff against one capture-start origin
(ddoc §6 calls this M0 core). The math is pure — keep it neutral so ubuntu CI runs it.

**Files:**
- Create: `crates/clipline-capture/src/clock.rs`
- Modify: `crates/clipline-capture/src/lib.rs`
- Test: inline `#[cfg(test)]` in `clock.rs`

- [ ] **Step 1: Write the failing tests** (`clock.rs`)

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origin_maps_to_zero() {
        let clock = RelativeClock::new(5_000_000);
        assert_eq!(clock.pts_s(5_000_000), 0.0);
    }

    #[test]
    fn ticks_after_origin_convert_at_100ns_per_tick() {
        let clock = RelativeClock::new(1_000);
        // 15_000_000 ticks of 100 ns = 1.5 s.
        assert!((clock.pts_s(1_000 + 15_000_000) - 1.5).abs() < 1e-12);
    }

    #[test]
    fn pre_origin_ticks_clamp_to_zero() {
        // A frame stamped before capture start (in-flight at session start)
        // must not produce a negative pts.
        let clock = RelativeClock::new(10_000_000);
        assert_eq!(clock.pts_s(9_999_999), 0.0);
    }

    #[test]
    fn qpc_converts_to_100ns_ticks() {
        // 10 MHz QPC frequency (the common modern value): 1 count = 100 ns.
        assert_eq!(qpc_to_ticks_100ns(123_456_789, 10_000_000), 123_456_789);
        // 3 MHz: 3_000_000 counts = 1 s = 10_000_000 ticks.
        assert_eq!(qpc_to_ticks_100ns(3_000_000, 3_000_000), 10_000_000);
    }

    #[test]
    fn qpc_conversion_does_not_overflow_large_uptimes() {
        // ~30 days of uptime at 10 MHz: counter * 10^7 overflows i64 — the
        // conversion must widen internally.
        let counter = 30 * 24 * 3600 * 10_000_000_i64;
        assert_eq!(qpc_to_ticks_100ns(counter, 10_000_000), counter);
    }
}
```

Add to `lib.rs`:
```rust
pub mod clock;

pub use clock::{qpc_to_ticks_100ns, RelativeClock};
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p clipline-capture`
Expected: COMPILE ERROR (`RelativeClock` not defined).

- [ ] **Step 3: Write the implementation** (top of `clock.rs`)

```rust
//! Capture timing (ddoc §6 "Clocking & A/V sync"): all capture clocks are
//! expressed as 100 ns ticks on the QPC timebase (WGC `SystemRelativeTime`
//! and WASAPI QPC positions both arrive in those units) and diffed against
//! one capture-start origin to produce `pts_s`.

/// Maps absolute 100 ns ticks to seconds since a fixed origin.
#[derive(Debug, Clone, Copy)]
pub struct RelativeClock {
    origin_ticks_100ns: i64,
}

impl RelativeClock {
    pub fn new(origin_ticks_100ns: i64) -> Self {
        Self { origin_ticks_100ns }
    }

    /// Seconds since the origin; ticks before the origin clamp to 0.0
    /// (a frame already in flight when capture started).
    pub fn pts_s(&self, ticks_100ns: i64) -> f64 {
        (ticks_100ns - self.origin_ticks_100ns).max(0) as f64 / 1e7
    }
}

/// Convert a raw QPC counter reading to 100 ns ticks. Widens to i128 so
/// `counter * 10^7` cannot overflow at large uptimes.
pub fn qpc_to_ticks_100ns(counter: i64, frequency: i64) -> i64 {
    (counter as i128 * 10_000_000 / frequency as i128) as i64
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p clipline-capture`
Expected: prior tests + 5 new pass.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "$(cat <<'EOF'
feat(capture): RelativeClock — QPC/100ns tick to pts math

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 2: `windows` dependency, `FrameData::Gpu`, new error variants

Neutral-side trait changes first (handoff rule), then the Windows-only payload variant.

**Files:**
- Modify: `crates/clipline-capture/Cargo.toml`
- Modify: `crates/clipline-capture/src/traits.rs`
- Modify: `crates/clipline-capture/src/mock.rs` (drop the now-refutable `let FrameData::Cpu` destructure)
- Create: `crates/clipline-capture/src/windows/mod.rs`
- Test: new `#[cfg(all(test, windows))]` module in `windows/mod.rs` (WARP device — runs headless on the Windows CI runner)

- [ ] **Step 1: Add the dependency**

`crates/clipline-capture/Cargo.toml` gains:

```toml
[target.'cfg(windows)'.dependencies]
windows = { version = "0.62", features = [
    "Foundation",
    "Graphics_Capture",
    "Graphics_DirectX",
    "Graphics_DirectX_Direct3D11",
    "Win32_Foundation",
    "Win32_Graphics_Direct3D",
    "Win32_Graphics_Direct3D11",
    "Win32_Graphics_Dxgi",
    "Win32_Graphics_Dxgi_Common",
    "Win32_Graphics_Gdi",
    "Win32_System_Performance",
    "Win32_System_WinRT",
    "Win32_System_WinRT_Direct3D11",
    "Win32_System_WinRT_Graphics_Capture",
    "Win32_UI_WindowsAndMessaging",
] }
```

- [ ] **Step 2: Write the failing test** (`windows/mod.rs`)

```rust
#[cfg(all(test, windows))]
mod tests {
    use crate::traits::{Frame, FrameData};

    /// A GPU frame must round-trip through the platform-neutral `Frame`
    /// struct (Debug + Clone are derived; windows-rs COM wrappers provide
    /// both). WARP renders headless, so this runs on the CI runner too.
    #[test]
    fn gpu_frame_data_wraps_a_d3d11_texture() {
        let (device, _context) = super::d3d11::create_device_for_tests()
            .expect("WARP D3D11 device");
        let texture = super::d3d11::create_bgra_texture(&device, 16, 16)
            .expect("16x16 texture");
        let frame = Frame { pts_s: 0.25, data: FrameData::Gpu(texture) };
        let cloned = frame.clone();
        let FrameData::Gpu(tex) = cloned.data else {
            panic!("expected Gpu variant");
        };
        let (w, h) = super::d3d11::texture_size(&tex);
        assert_eq!((w, h), (16, 16));
        assert!(!format!("{frame:?}").is_empty());
    }
}
```

Add to `lib.rs`:
```rust
#[cfg(windows)]
pub mod windows;
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p clipline-capture`
Expected: COMPILE ERROR (no `windows` module / no `Gpu` variant).

- [ ] **Step 4: Write the implementation**

`traits.rs` — `FrameData` and `CaptureError` become:

```rust
/// Frame payload. `Cpu` serves mocks/tests/software paths; `Gpu` keeps
/// pixels on the GPU as ddoc §3 requires (no CPU round-trips).
#[derive(Debug, Clone)]
pub enum FrameData {
    Cpu(Vec<u8>),
    #[cfg(windows)]
    Gpu(::windows::Win32::Graphics::Direct3D11::ID3D11Texture2D),
}

#[derive(Debug, thiserror::Error)]
pub enum CaptureError {
    #[error("capture init failed: {0}")]
    Init(String),
    #[error("capture device lost: {0}")]
    DeviceLost(String),
    #[error("no frame arrived within {0:?}")]
    Timeout(std::time::Duration),
}
```

`mock.rs` — delete the line `let FrameData::Cpu(_) = &frame.data;` from
`MockEncoder::encode` (with a second variant it is refutable; it asserted nothing).
If `FrameData` is then unused in mock.rs imports, drop it from the `use`.

`windows/mod.rs`:

```rust
//! Windows platform layer. All COM/WinRT `unsafe` lives in this module
//! tree; everything exported is a safe wrapper honoring the platform
//! traits' contracts (see `crate::mock` for the reference behavior).

pub mod d3d11;
```

`windows/d3d11.rs` — device/texture plumbing shared by WGC now and the MFT
encoder milestone later:

```rust
//! Thin safe wrappers over D3D11 device/texture creation.

use windows::core::Result as WinResult;
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE, D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP};
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D,
    D3D11_BIND_SHADER_RESOURCE, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION,
    D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};

/// Create a hardware D3D11 device with BGRA support (required by WGC).
pub fn create_device() -> WinResult<(ID3D11Device, ID3D11DeviceContext)> {
    create_device_with(D3D_DRIVER_TYPE_HARDWARE)
}

/// WARP (software) device — headless-safe, used by tests.
pub fn create_device_for_tests() -> WinResult<(ID3D11Device, ID3D11DeviceContext)> {
    create_device_with(D3D_DRIVER_TYPE_WARP)
}

fn create_device_with(driver: D3D_DRIVER_TYPE) -> WinResult<(ID3D11Device, ID3D11DeviceContext)> {
    let mut device = None;
    let mut context = None;
    // SAFETY: out-params receive valid COM pointers on success; we pass no
    // adapter (driver type selects it) and no software rasterizer module.
    unsafe {
        D3D11CreateDevice(
            None,
            driver,
            None,
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            None,
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut context),
        )?;
    }
    Ok((device.expect("device out-param set on Ok"), context.expect("context out-param set on Ok")))
}

/// Default-usage BGRA texture, e.g. the destination for a capture-frame copy.
pub fn create_bgra_texture(device: &ID3D11Device, width: u32, height: u32) -> WinResult<ID3D11Texture2D> {
    let desc = D3D11_TEXTURE2D_DESC {
        Width: width,
        Height: height,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
        CPUAccessFlags: 0,
        MiscFlags: 0,
    };
    let mut texture = None;
    // SAFETY: desc is fully initialized; out-param receives a valid pointer on success.
    unsafe { device.CreateTexture2D(&desc, None, Some(&mut texture))? };
    Ok(texture.expect("texture out-param set on Ok"))
}

pub fn texture_size(texture: &ID3D11Texture2D) -> (u32, u32) {
    let mut desc = D3D11_TEXTURE2D_DESC::default();
    // SAFETY: GetDesc writes to a valid out-pointer.
    unsafe { texture.GetDesc(&mut desc) };
    (desc.Width, desc.Height)
}
```

(`pub mod d3d11;` referenced from `windows/mod.rs` above.)

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p clipline-capture` — all prior tests plus
`gpu_frame_data_wraps_a_d3d11_texture` pass on Windows.
Also: `cargo test --workspace` — the platform-neutral suite is untouched.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "$(cat <<'EOF'
feat(capture): FrameData::Gpu(ID3D11Texture2D) behind cfg(windows)

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 3: WGC monitor capture engine

**Files:**
- Create: `crates/clipline-capture/src/windows/wgc.rs`
- Modify: `crates/clipline-capture/src/windows/mod.rs`
- Test: `#[cfg(all(test, windows))]` in `wgc.rs`, self-skipping without a desktop session

- [ ] **Step 1: Write the failing test**

```rust
#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use crate::traits::{CaptureEngine, FrameData};
    use std::time::Duration;

    /// Real WGC against the primary monitor. Self-skips when capture is
    /// unavailable (CI runners: no GPU / no interactive desktop) — the
    /// skip-if-absent pattern the ffprobe e2e tests use.
    #[test]
    fn captures_monotonic_gpu_frames_from_primary_monitor() {
        let mut cap = match WgcCapture::primary_monitor() {
            Ok(cap) => cap,
            Err(e) => {
                eprintln!("SKIP: WGC unavailable: {e}");
                return;
            }
        };
        let mut last_pts = -1.0;
        for _ in 0..3 {
            let frame = cap
                .next_frame_timeout(Duration::from_secs(5))
                .expect("frame within 5s on a live desktop")
                .expect("session still open");
            assert!(frame.pts_s >= last_pts, "pts must be monotonic");
            assert!(frame.pts_s < 60.0, "pts is relative to capture start");
            last_pts = frame.pts_s;
            let FrameData::Gpu(tex) = &frame.data else {
                panic!("WGC frames are GPU textures");
            };
            let (w, h) = crate::windows::d3d11::texture_size(tex);
            assert!(w > 0 && h > 0);
        }
    }
}
```

`windows/mod.rs` gains:
```rust
pub mod wgc;

pub use wgc::WgcCapture;
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p clipline-capture`
Expected: COMPILE ERROR (`WgcCapture` not defined).

- [ ] **Step 3: Write the implementation** (`wgc.rs`)

```rust
//! Windows.Graphics.Capture engine (ddoc §3/§8): DWM-level capture, no
//! injection. Free-threaded frame pool; the FrameArrived handler copies
//! each frame's texture (pool surfaces are recycled) and queues it; the
//! pull-model `next_frame` drains the queue.

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::Duration;

use windows::core::{Interface, Result as WinResult};
use windows::Foundation::TypedEventHandler;
use windows::Graphics::Capture::{
    Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession,
};
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Win32::Foundation::{HWND, POINT};
use windows::Win32::Graphics::Direct3D11::{ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D};
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::Graphics::Gdi::{MonitorFromPoint, MONITOR_DEFAULTTOPRIMARY};
use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
use windows::Win32::System::WinRT::Direct3D11::{
    CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess,
};
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};

use crate::clock::{qpc_to_ticks_100ns, RelativeClock};
use crate::traits::{CaptureEngine, CaptureError, Frame, FrameData};
use crate::windows::d3d11;

/// Default `next_frame` wait. WGC only delivers on screen updates, so an
/// idle desktop can legitimately go quiet; recorders that need cadence
/// repeat the previous frame (encoder-side concern, milestone 4).
const DEFAULT_FRAME_TIMEOUT: Duration = Duration::from_secs(5);

struct QueuedFrame {
    texture: ID3D11Texture2D,
    ticks_100ns: i64,
}

pub struct WgcCapture {
    session: GraphicsCaptureSession,
    frame_pool: Direct3D11CaptureFramePool,
    rx: Receiver<QueuedFrame>,
    clock: RelativeClock,
}

impl WgcCapture {
    /// Capture the primary monitor.
    pub fn primary_monitor() -> Result<Self, CaptureError> {
        // SAFETY: plain Win32 call; a primary monitor always exists in an
        // interactive session (HMONITOR is null otherwise, failing item
        // creation below with a descriptive error).
        let hmon = unsafe { MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY) };
        let item = create_item(|interop| unsafe { interop.CreateForMonitor(hmon) })?;
        Self::new(item)
    }

    /// Capture one window (must be visible; ddoc §3: per-window preferred,
    /// borderless fullscreen recommended for games).
    pub fn for_window(hwnd: HWND) -> Result<Self, CaptureError> {
        let item = create_item(|interop| unsafe { interop.CreateForWindow(hwnd) })?;
        Self::new(item)
    }

    fn new(item: GraphicsCaptureItem) -> Result<Self, CaptureError> {
        init_winrt()?;
        let init = |e: windows::core::Error| CaptureError::Init(e.to_string());

        let (device, context) = d3d11::create_device().map_err(init)?;
        let winrt_device = winrt_device(&device).map_err(init)?;

        let size = item.Size().map_err(init)?;
        let frame_pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
            &winrt_device,
            DirectXPixelFormat::B8G8R8A8UIntNormalized,
            2,
            size,
        )
        .map_err(init)?;
        let session = frame_pool.CreateCaptureSession(&item).map_err(init)?;
        // Best-effort: needs Win10 20348+/Win11 (ddoc Caveats) — older
        // builds show the yellow border.
        let _ = session.SetIsBorderRequired(false);

        let origin = qpc_now_ticks_100ns().map_err(init)?;
        let (tx, rx) = mpsc::channel();
        frame_pool
            .FrameArrived(&TypedEventHandler::new(on_frame_arrived(device, context, tx)))
            .map_err(init)?;
        session.StartCapture().map_err(init)?;

        Ok(Self { session, frame_pool, rx, clock: RelativeClock::new(origin) })
    }

    /// `next_frame` with an explicit wait bound.
    pub fn next_frame_timeout(&mut self, timeout: Duration) -> Result<Option<Frame>, CaptureError> {
        match self.rx.recv_timeout(timeout) {
            Ok(q) => Ok(Some(Frame {
                pts_s: self.clock.pts_s(q.ticks_100ns),
                data: FrameData::Gpu(q.texture),
            })),
            Err(RecvTimeoutError::Disconnected) => Ok(None), // session closed
            Err(RecvTimeoutError::Timeout) => Err(CaptureError::Timeout(timeout)),
        }
    }
}

impl CaptureEngine for WgcCapture {
    fn next_frame(&mut self) -> Result<Option<Frame>, CaptureError> {
        self.next_frame_timeout(DEFAULT_FRAME_TIMEOUT)
    }
}

impl Drop for WgcCapture {
    fn drop(&mut self) {
        let _ = self.session.Close();
        let _ = self.frame_pool.Close();
    }
}

/// FrameArrived handler: copy the (recycled) pool surface into a fresh
/// texture and queue it. Only this handler thread touches the immediate
/// context, respecting D3D11's single-threaded context rule.
fn on_frame_arrived(
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    tx: Sender<QueuedFrame>,
) -> impl Fn(
    windows::core::Ref<'_, Direct3D11CaptureFramePool>,
    windows::core::Ref<'_, windows::core::IInspectable>,
) -> WinResult<()> {
    move |pool, _| {
        let Some(pool) = pool.as_ref() else { return Ok(()) };
        let Ok(frame) = pool.TryGetNextFrame() else { return Ok(()) };
        let ticks_100ns = frame.SystemRelativeTime()?.Duration;
        let access: IDirect3DDxgiInterfaceAccess = frame.Surface()?.cast()?;
        // SAFETY: the surface is a live IDirect3D surface backed by an
        // ID3D11Texture2D; GetInterface AddRefs it.
        let source: ID3D11Texture2D = unsafe { access.GetInterface()? };
        let (w, h) = d3d11::texture_size(&source);
        let copy = d3d11::create_bgra_texture(&device, w, h)?;
        // SAFETY: both resources belong to `device`; CopyResource is valid
        // for same-format, same-size textures.
        unsafe { context.CopyResource(&copy, &source) };
        // Receiver gone (engine dropped) → stop forwarding, not an error.
        let _ = tx.send(QueuedFrame { texture: copy, ticks_100ns });
        Ok(())
    }
}

fn create_item(
    create: impl FnOnce(&IGraphicsCaptureItemInterop) -> WinResult<GraphicsCaptureItem>,
) -> Result<GraphicsCaptureItem, CaptureError> {
    init_winrt()?;
    let interop = windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()
        .map_err(|e| CaptureError::Init(format!("WGC interop factory: {e}")))?;
    create(&interop).map_err(|e| CaptureError::Init(format!("create capture item: {e}")))
}

/// Wrap the DXGI device for WinRT consumption.
fn winrt_device(device: &ID3D11Device) -> WinResult<IDirect3DDevice> {
    let dxgi: IDXGIDevice = device.cast()?;
    // SAFETY: dxgi is a valid device; the call returns an IInspectable we cast.
    let inspectable = unsafe { CreateDirect3D11DeviceFromDXGIDevice(&dxgi)? };
    inspectable.cast()
}

fn qpc_now_ticks_100ns() -> WinResult<i64> {
    let (mut counter, mut freq) = (0i64, 0i64);
    // SAFETY: out-pointers are valid; these calls cannot fail on XP+.
    unsafe {
        QueryPerformanceCounter(&mut counter)?;
        QueryPerformanceFrequency(&mut freq)?;
    }
    Ok(qpc_to_ticks_100ns(counter, freq))
}

/// Idempotent WinRT init; an already-initialized thread (RPC_E_CHANGED_MODE
/// under an STA host) is fine — WGC's free-threaded pool works either way.
fn init_winrt() -> Result<(), CaptureError> {
    // SAFETY: RoInitialize is safe to call repeatedly per thread.
    match unsafe { RoInitialize(RO_INIT_MULTITHREADED) } {
        Ok(()) => Ok(()),
        Err(e) if e.code() == windows::Win32::Foundation::RPC_E_CHANGED_MODE => Ok(()),
        Err(e) => Err(CaptureError::Init(format!("RoInitialize: {e}"))),
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p clipline-capture`
Expected: the device test captures 3 real frames on the dev machine (and
self-skips on CI). All neutral tests still green.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "$(cat <<'EOF'
feat(capture): WGC monitor/window capture engine behind CaptureEngine

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 4: window lookup helper (for window capture by title)

`WgcCapture::for_window` exists; the smoke example (and later the UI) needs an HWND from a
title substring. `FindWindowW` is exact-match only, so enumerate.

**Files:**
- Create: `crates/clipline-capture/src/windows/window.rs`
- Modify: `crates/clipline-capture/src/windows/mod.rs`
- Test: `#[cfg(all(test, windows))]` in `window.rs`

- [ ] **Step 1: Write the failing test**

```rust
#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn no_match_returns_none() {
        assert!(find_window_by_title("no window is named this 5f2c9a").is_none());
    }

    #[test]
    fn match_is_case_insensitive_substring() {
        // The test process has no windows of its own; just exercise the
        // enumeration path — any visible-titled window on a desktop session
        // matches "" ... so use a needle that matches nothing vs. the
        // empty needle which matches the first titled window, if any.
        let _ = find_window_by_title(""); // must not crash; result depends on session
    }
}
```

`windows/mod.rs` gains:
```rust
pub mod window;

pub use window::find_window_by_title;
```

- [ ] **Step 2: Run test to verify it fails** — COMPILE ERROR.

- [ ] **Step 3: Write the implementation** (`window.rs`)

```rust
//! Find a visible top-level window by case-insensitive title substring.

use windows::Win32::Foundation::{BOOL, HWND, LPARAM};
use windows::Win32::UI::WindowsAndMessaging::{EnumWindows, GetWindowTextW, IsWindowVisible};

struct Search {
    needle_lower: String,
    found: Option<HWND>,
}

pub fn find_window_by_title(needle: &str) -> Option<HWND> {
    let mut search = Search { needle_lower: needle.to_lowercase(), found: None };
    // SAFETY: callback only runs during this call; lparam points at `search`,
    // which outlives it. EnumWindows returns Err when the callback stops
    // enumeration early — that's our found case, not an error.
    unsafe {
        let _ = EnumWindows(Some(enum_proc), LPARAM(&mut search as *mut Search as isize));
    }
    search.found
}

unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let search = &mut *(lparam.0 as *mut Search);
    if !IsWindowVisible(hwnd).as_bool() {
        return BOOL(1);
    }
    let mut buf = [0u16; 512];
    let len = GetWindowTextW(hwnd, &mut buf);
    if len == 0 {
        return BOOL(1);
    }
    let title = String::from_utf16_lossy(&buf[..len as usize]).to_lowercase();
    if title.contains(&search.needle_lower) {
        search.found = Some(hwnd);
        return BOOL(0); // stop enumeration
    }
    BOOL(1)
}
```

- [ ] **Step 4: Run tests to verify they pass** — `cargo test -p clipline-capture`.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "$(cat <<'EOF'
feat(capture): find_window_by_title for window-capture targeting

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 5: smoke example + manual verification (milestone exit)

**Files:**
- Create: `crates/clipline-capture/examples/wgc_smoke.rs`

- [ ] **Step 1: Write the example**

```rust
//! WGC capture smoke test (run manually — needs a desktop session):
//!   cargo run -p clipline-capture --example wgc_smoke -- --frames 120
//!   cargo run -p clipline-capture --example wgc_smoke -- --window "notepad"
//! Reports resolution and measured fps from frame pts deltas.

#[cfg(not(windows))]
fn main() {
    eprintln!("wgc_smoke is Windows-only");
}

#[cfg(windows)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use clipline_capture::traits::FrameData;
    use clipline_capture::windows::{find_window_by_title, WgcCapture};
    use clipline_capture::CaptureEngine;

    let args: Vec<String> = std::env::args().collect();
    let arg = |flag: &str| {
        args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned()
    };
    let frames: u32 = arg("--frames").map(|v| v.parse()).transpose()?.unwrap_or(120);

    let mut cap = match arg("--window") {
        Some(needle) => {
            let hwnd = find_window_by_title(&needle)
                .ok_or_else(|| format!("no visible window matching {needle:?}"))?;
            println!("capturing window matching {needle:?}");
            WgcCapture::for_window(hwnd)?
        }
        None => {
            println!("capturing primary monitor");
            WgcCapture::primary_monitor()?
        }
    };

    let mut pts = Vec::with_capacity(frames as usize);
    let mut resolution = (0, 0);
    while pts.len() < frames as usize {
        let Some(frame) = cap.next_frame()? else {
            println!("capture ended early after {} frames", pts.len());
            break;
        };
        if let FrameData::Gpu(tex) = &frame.data {
            resolution = clipline_capture::windows::d3d11::texture_size(tex);
        }
        pts.push(frame.pts_s);
        if pts.len() % 30 == 0 {
            println!("  {} frames…", pts.len());
        }
    }

    let span = pts.last().unwrap_or(&0.0) - pts.first().unwrap_or(&0.0);
    let fps = if span > 0.0 { (pts.len() as f64 - 1.0) / span } else { 0.0 };
    let deltas: Vec<f64> = pts.windows(2).map(|w| w[1] - w[0]).collect();
    let (min_d, max_d) = deltas.iter().fold((f64::MAX, 0.0f64), |(lo, hi), d| (lo.min(*d), hi.max(*d)));

    println!("captured {} frames @ {}x{}", pts.len(), resolution.0, resolution.1);
    println!("pts span {span:.3}s → {fps:.1} fps (frame gap {:.1}–{:.1} ms)", min_d * 1e3, max_d * 1e3);
    println!("first pts {:.4}s, monotonic: {}", pts.first().unwrap_or(&0.0),
        pts.windows(2).all(|w| w[1] >= w[0]));
    Ok(())
}
```

Note: `d3d11` is already `pub mod`; ensure `windows/mod.rs` re-exports cover what the
example uses (`find_window_by_title`, `WgcCapture`, `d3d11`).

- [ ] **Step 2: Verify it compiles on both target shapes**

Run: `cargo build -p clipline-capture --example wgc_smoke`
(CI's ubuntu job compiles the `not(windows)` stub via `cargo test --workspace`.)

- [ ] **Step 3: Run it for real — the milestone verification**

Run: `cargo run -p clipline-capture --example wgc_smoke -- --frames 120`
Expected: monitor resolution (e.g. 2560x1440), ~display-refresh fps under screen activity,
monotonic pts starting near 0. Move a window during capture so WGC has dirty frames.
Then: `cargo run -p clipline-capture --example wgc_smoke -- --window "<some open window>"`.

Record the observed output in the commit message body.

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "$(cat <<'EOF'
feat(capture): wgc_smoke example — manual WGC verification harness

<observed output summary here>

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 6: quality gates

- [ ] `cargo test --workspace` — green (device tests run for real on this machine).
- [ ] `cargo clippy --workspace --all-targets` — zero warnings (CI uses `-D warnings`).
- [ ] Push; confirm GitHub Actions green on **ubuntu-latest and windows-latest**.

---

## Out of scope (follow-ups)

- Media Foundation H.264 encoder behind `Encoder` + real `probe::enumerate()` (milestone 2).
- WASAPI loopback `AudioSource` (milestone 3); A/V sync hardening on the shared QPC origin (milestone 4).
- Frame-cadence handling for idle screens (repeat-last-frame at encoder), capture-item resize
  (`FramePool::Recreate` on size change), DXGI Desktop Duplication fallback, HDR pixel formats.
- Borderless-fullscreen guidance + display-capture privacy warning in the UI layer (ddoc §8/§9).
