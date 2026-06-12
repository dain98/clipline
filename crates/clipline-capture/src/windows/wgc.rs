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
use windows::Win32::Graphics::Gdi::{MonitorFromPoint, HMONITOR, MONITOR_DEFAULTTOPRIMARY};
use windows::Win32::System::WinRT::Direct3D11::{
    CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess,
};
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};

use crate::clock::RelativeClock;
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
    /// Capture the primary monitor on a freshly created device and clock.
    pub fn primary_monitor() -> Result<Self, CaptureError> {
        let (device, _) = d3d11::create_device().map_err(|e| CaptureError::Init(e.to_string()))?;
        Self::primary_monitor_on(device, Self::new_clock()?)
    }

    /// A capture clock anchored at "now" — create one and share it across
    /// every engine of a recording (ddoc §6: one QPC timebase).
    pub fn new_clock() -> Result<RelativeClock, CaptureError> {
        let origin =
            crate::windows::qpc_now_ticks_100ns().map_err(|e| CaptureError::Init(e.to_string()))?;
        Ok(RelativeClock::new(origin))
    }

    /// Capture the primary monitor on a caller-provided device and clock
    /// (one device must be shared with the encoder — textures don't cross
    /// devices; one clock must be shared with audio — ddoc §6).
    pub fn primary_monitor_on(
        device: ID3D11Device,
        clock: RelativeClock,
    ) -> Result<Self, CaptureError> {
        // SAFETY: plain Win32 call returning a handle (null in headless
        // sessions — checked below, since CreateForMonitor access-violates
        // on an invalid HMONITOR instead of returning an error).
        let hmon = unsafe { MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY) };
        if hmon.is_invalid() {
            return Err(CaptureError::Init(
                "no monitor in this session (headless?)".into(),
            ));
        }
        Self::for_monitor_on(device, hmon, clock)
    }

    /// Capture a specific monitor on a caller-provided device and clock.
    pub fn for_monitor_on(
        device: ID3D11Device,
        monitor: HMONITOR,
        clock: RelativeClock,
    ) -> Result<Self, CaptureError> {
        if monitor.is_invalid() {
            return Err(CaptureError::Init("invalid monitor handle".into()));
        }
        let item = create_item(|interop| unsafe { interop.CreateForMonitor(monitor) })?;
        Self::new(item, device, clock)
    }

    /// Capture one window (must be visible; ddoc §3: per-window preferred,
    /// borderless fullscreen recommended for games).
    pub fn for_window(hwnd: HWND) -> Result<Self, CaptureError> {
        let (device, _) = d3d11::create_device().map_err(|e| CaptureError::Init(e.to_string()))?;
        Self::for_window_on(device, hwnd, Self::new_clock()?)
    }

    /// Window capture on a caller-provided device and clock.
    pub fn for_window_on(
        device: ID3D11Device,
        hwnd: HWND,
        clock: RelativeClock,
    ) -> Result<Self, CaptureError> {
        if hwnd.is_invalid() {
            return Err(CaptureError::Init("invalid window handle".into()));
        }
        let item = create_item(|interop| unsafe { interop.CreateForWindow(hwnd) })?;
        Self::new(item, device, clock)
    }

    fn new(
        item: GraphicsCaptureItem,
        device: ID3D11Device,
        clock: RelativeClock,
    ) -> Result<Self, CaptureError> {
        init_winrt()?;
        let init = |e: windows::core::Error| CaptureError::Init(e.to_string());

        // SAFETY: trivial getter on a valid device.
        let context = unsafe { device.GetImmediateContext() }.map_err(init)?;
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

        let (tx, rx) = mpsc::channel();
        frame_pool
            .FrameArrived(&TypedEventHandler::new(on_frame_arrived(
                device, context, tx,
            )))
            .map_err(init)?;
        session.StartCapture().map_err(init)?;

        Ok(Self {
            session,
            frame_pool,
            rx,
            clock,
        })
    }

    /// The capture-start clock — share it with audio sources so all pts
    /// live on one timeline (ddoc §6).
    pub fn clock(&self) -> RelativeClock {
        self.clock
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
        let Some(pool) = pool.as_ref() else {
            return Ok(());
        };
        let Ok(frame) = pool.TryGetNextFrame() else {
            return Ok(());
        };
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
        let _ = tx.send(QueuedFrame {
            texture: copy,
            ticks_100ns,
        });
        Ok(())
    }
}

fn create_item(
    create: impl FnOnce(&IGraphicsCaptureItemInterop) -> WinResult<GraphicsCaptureItem>,
) -> Result<GraphicsCaptureItem, CaptureError> {
    init_winrt()?;
    match GraphicsCaptureSession::IsSupported() {
        Ok(true) => {}
        Ok(false) => {
            return Err(CaptureError::Init(
                "WGC not supported in this session".into(),
            ))
        }
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

    #[test]
    fn capture_runs_on_a_caller_provided_device() {
        if std::env::var_os("CI").is_some() {
            eprintln!("SKIP: WGC device test needs a real interactive desktop");
            return;
        }
        let (device, _ctx) = crate::windows::d3d11::create_device().expect("device");
        let clock = WgcCapture::new_clock().expect("clock");
        let mut cap = match WgcCapture::primary_monitor_on(device.clone(), clock) {
            Ok(cap) => cap,
            Err(e) => {
                eprintln!("SKIP: WGC unavailable: {e}");
                return;
            }
        };
        let frame = cap
            .next_frame_timeout(Duration::from_secs(5))
            .expect("frame")
            .expect("session open");
        // The provided clock is the frame timebase: pts is near zero.
        assert!(
            frame.pts_s >= 0.0 && frame.pts_s < 5.0,
            "pts {}",
            frame.pts_s
        );
        let FrameData::Gpu(tex) = &frame.data else {
            panic!("gpu frame")
        };
        // The texture must live on the device we provided.
        use windows::core::Interface;
        // SAFETY: trivial getter on a valid texture.
        let owner = unsafe { tex.GetDevice() }.expect("owner device");
        assert_eq!(owner.as_raw(), device.as_raw());
    }

    /// The milestone 4 exit test: video + audio on ONE clock, recorded by
    /// the real pipeline, validated by the tolerance-based avsync checks —
    /// the mock-pinned GOP discipline reproduced on real hardware.
    #[test]
    fn real_engines_on_one_clock_produce_a_synced_timeline() {
        if std::env::var_os("CI").is_some() {
            eprintln!("SKIP: device test needs desktop + audio endpoint");
            return;
        }
        let (device, _ctx) = crate::windows::d3d11::create_device().expect("device");
        let clock = WgcCapture::new_clock().expect("clock");
        let cap = match WgcCapture::primary_monitor_on(device.clone(), clock) {
            Ok(cap) => cap,
            Err(e) => {
                eprintln!("SKIP: WGC unavailable: {e}");
                return;
            }
        };
        let audio = match crate::windows::WasapiLoopback::start(clock) {
            Ok(a) => a,
            Err(e) => {
                eprintln!("SKIP: loopback unavailable: {e}");
                return;
            }
        };
        let cfg = crate::windows::MftConfig {
            width: 640,
            height: 360,
            fps: 60,
            bitrate_bps: 2_000_000,
        };
        // Pull one frame to learn the capture size, then hand the engine on.
        let mut cap = cap;
        let first = cap
            .next_frame_timeout(std::time::Duration::from_secs(5))
            .expect("frame")
            .expect("open");
        let crate::traits::FrameData::Gpu(tex) = &first.data else {
            panic!("gpu")
        };
        let (in_w, in_h) = crate::windows::d3d11::texture_size(tex);
        let enc = match crate::windows::MftH264Encoder::new(&device, in_w, in_h, cfg) {
            Ok(e) => e,
            Err(e) => {
                eprintln!("SKIP: no hardware encoder: {e}");
                return;
            }
        };
        let mut rec = crate::pipeline::Recorder::new(
            crate::mock::LimitedCapture::new(cap, 60),
            enc,
            usize::MAX,
        )
        .with_audio(Box::new(audio));
        rec.run_to_end().expect("record");
        let segs: Vec<&clipline_buffer::Segment> = rec.ring().segments().collect();
        let report =
            crate::avsync::validate_timeline(&segs, &crate::avsync::SyncTolerances::default())
                .expect("real-clock timeline within tolerances");
        eprintln!("sync report: {report:?}");
        assert!(
            report.video_duration_s > 0.5,
            "recorded something substantial"
        );
    }
}
