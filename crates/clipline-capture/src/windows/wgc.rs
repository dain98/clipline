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
use windows::Win32::Foundation::{HWND, POINT, RPC_E_CHANGED_MODE};
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
        // SAFETY: plain Win32 call returning a handle (null in headless
        // sessions — checked below, since CreateForMonitor access-violates
        // on an invalid HMONITOR instead of returning an error).
        let hmon = unsafe { MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY) };
        if hmon.is_invalid() {
            return Err(CaptureError::Init("no monitor in this session (headless?)".into()));
        }
        let item = create_item(|interop| unsafe { interop.CreateForMonitor(hmon) })?;
        Self::new(item)
    }

    /// Capture one window (must be visible; ddoc §3: per-window preferred,
    /// borderless fullscreen recommended for games).
    pub fn for_window(hwnd: HWND) -> Result<Self, CaptureError> {
        if hwnd.is_invalid() {
            return Err(CaptureError::Init("invalid window handle".into()));
        }
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
    match GraphicsCaptureSession::IsSupported() {
        Ok(true) => {}
        Ok(false) => return Err(CaptureError::Init("WGC not supported in this session".into())),
        Err(e) => return Err(CaptureError::Init(format!("WGC support query: {e}"))),
    }
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
        Err(e) if e.code() == RPC_E_CHANGED_MODE => Ok(()),
        Err(e) => Err(CaptureError::Init(format!("RoInitialize: {e}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traits::FrameData;
    use std::time::Duration;

    /// Real WGC against the primary monitor. Self-skips on CI and when
    /// capture init fails — the skip-if-absent pattern the ffprobe e2e
    /// tests use. The CI gate is unconditional: Windows Server runners
    /// report `IsSupported() == true` and expose a virtual display, then
    /// access-violate inside the capture component (observed on
    /// windows-2025); WGC verification is manual on a real desktop
    /// (see the handoff/plan).
    #[test]
    fn captures_monotonic_gpu_frames_from_primary_monitor() {
        if std::env::var_os("CI").is_some() {
            eprintln!("SKIP: WGC device test needs a real interactive desktop");
            return;
        }
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
