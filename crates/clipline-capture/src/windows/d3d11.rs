//! Thin safe wrappers over D3D11 device/texture creation, shared by WGC
//! capture now and the Media Foundation encoder milestone later.

use windows::core::{Interface, Result as WinResult};
use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE, D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP,
};
use windows::Win32::Graphics::Direct3D10::ID3D10Multithread;
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Resource, ID3D11Texture2D,
    ID3D11VideoContext, ID3D11VideoContext1, ID3D11VideoDevice, ID3D11VideoProcessor,
    ID3D11VideoProcessorEnumerator, D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE,
    D3D11_BOX, D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAPPED_SUBRESOURCE,
    D3D11_MAP_FLAG_DO_NOT_WAIT, D3D11_MAP_READ, D3D11_SDK_VERSION, D3D11_TEX2D_VPIV,
    D3D11_TEX2D_VPOV, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT, D3D11_USAGE_STAGING,
    D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE, D3D11_VIDEO_PROCESSOR_COLOR_SPACE,
    D3D11_VIDEO_PROCESSOR_CONTENT_DESC, D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC,
    D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC_0, D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC,
    D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0, D3D11_VIDEO_PROCESSOR_STREAM,
    D3D11_VIDEO_USAGE_PLAYBACK_NORMAL, D3D11_VPIV_DIMENSION_TEXTURE2D,
    D3D11_VPOV_DIMENSION_TEXTURE2D,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_COLOR_SPACE_RGB_FULL_G22_NONE_P709, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_NV12,
    DXGI_RATIONAL, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::DXGI_ERROR_WAS_STILL_DRAWING;

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
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            None,
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut context),
        )?;
    }
    let device = device.expect("device out-param set on Ok");
    // MF's DXGI device manager shares this device across threads —
    // multithread protection is required for D3D-aware MFTs.
    let mt: ID3D10Multithread = device.cast()?;
    // SAFETY: trivial setter on a valid interface (returns the previous
    // value, which we don't need).
    let _ = unsafe { mt.SetMultithreadProtected(true) };
    Ok((device, context.expect("context out-param set on Ok")))
}

/// Default-usage BGRA texture, e.g. the destination for a capture-frame
/// copy. RENDER_TARGET is included so the texture can feed video-processor
/// views (drivers reject input views on SHADER_RESOURCE-only textures).
pub fn create_bgra_texture(
    device: &ID3D11Device,
    width: u32,
    height: u32,
) -> WinResult<ID3D11Texture2D> {
    let desc = D3D11_TEXTURE2D_DESC {
        Width: width,
        Height: height,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: (D3D11_BIND_SHADER_RESOURCE.0 | D3D11_BIND_RENDER_TARGET.0) as u32,
        CPUAccessFlags: 0,
        MiscFlags: 0,
    };
    let mut texture = None;
    // SAFETY: desc is fully initialized; out-param receives a valid pointer
    // on success.
    unsafe { device.CreateTexture2D(&desc, None, Some(&mut texture))? };
    Ok(texture.expect("texture out-param set on Ok"))
}

/// NV12 render-target texture (video processor output / encoder input).
pub fn create_nv12_texture(
    device: &ID3D11Device,
    width: u32,
    height: u32,
) -> WinResult<ID3D11Texture2D> {
    let desc = D3D11_TEXTURE2D_DESC {
        Width: width,
        Height: height,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_NV12,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32,
        CPUAccessFlags: 0,
        MiscFlags: 0,
    };
    let mut texture = None;
    // SAFETY: desc is fully initialized; out-param receives a valid pointer
    // on success.
    unsafe { device.CreateTexture2D(&desc, None, Some(&mut texture))? };
    Ok(texture.expect("texture out-param set on Ok"))
}

/// CPU-readable NV12 staging texture: the FFmpeg encoder copies a
/// GPU-converted NV12 frame here and maps it to pipe contiguous bytes.
pub fn create_nv12_staging(
    device: &ID3D11Device,
    width: u32,
    height: u32,
) -> WinResult<ID3D11Texture2D> {
    let desc = D3D11_TEXTURE2D_DESC {
        Width: width,
        Height: height,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_NV12,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_STAGING,
        BindFlags: 0,
        CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
        MiscFlags: 0,
    };
    let mut texture = None;
    // SAFETY: desc is fully initialized; out-param receives a valid pointer
    // on success (checked by `?`).
    unsafe { device.CreateTexture2D(&desc, None, Some(&mut texture))? };
    Ok(texture.expect("CreateTexture2D succeeded but returned null"))
}

fn create_bgra_staging(
    device: &ID3D11Device,
    width: u32,
    height: u32,
) -> WinResult<ID3D11Texture2D> {
    let desc = D3D11_TEXTURE2D_DESC {
        Width: width,
        Height: height,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_STAGING,
        BindFlags: 0,
        CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
        MiscFlags: 0,
    };
    let mut texture = None;
    // SAFETY: desc is fully initialized; out-param receives a valid pointer
    // on success (checked by `?`).
    unsafe { device.CreateTexture2D(&desc, None, Some(&mut texture))? };
    Ok(texture.expect("CreateTexture2D succeeded but returned null"))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RgbaImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// Stateful live-preview readback. It first scales the capture texture on the
/// GPU, then maps a previously queued small staging texture with DO_NOT_WAIT so
/// the recorder thread never blocks on a CPU/GPU sync for the preview.
pub struct PreviewReadback {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    max_width: u32,
    max_height: u32,
    pipeline: Option<PreviewPipeline>,
}

impl PreviewReadback {
    pub fn new(device: &ID3D11Device, max_width: u32, max_height: u32) -> WinResult<Self> {
        // SAFETY: trivial getter; the device owns a single immediate context.
        let context = unsafe { device.GetImmediateContext()? };
        Ok(Self {
            device: device.clone(),
            context,
            max_width: max_width.max(1),
            max_height: max_height.max(1),
            pipeline: None,
        })
    }

    pub fn reset(&mut self) {
        if let Some(pipeline) = &mut self.pipeline {
            pipeline.pending = None;
        }
    }

    pub fn try_read(&mut self, src: &ID3D11Texture2D) -> WinResult<Option<RgbaImage>> {
        let (src_width, src_height) = texture_size(src);
        self.ensure_pipeline(src_width, src_height)?;
        let pipeline = self
            .pipeline
            .as_mut()
            .expect("preview pipeline initialized above");

        let image = if let Some(pending) = pipeline.pending {
            let resource: ID3D11Resource = pipeline.staging[pending].cast()?;
            match map_bgra_staging_nonblocking(
                &self.context,
                &resource,
                pipeline.out_width,
                pipeline.out_height,
            )? {
                Some(image) => {
                    pipeline.pending = None;
                    Some(image)
                }
                None => return Ok(None),
            }
        } else {
            None
        };

        let next = pipeline.next_staging;
        pipeline.next_staging = 1 - pipeline.next_staging;
        pipeline.scale_to_output(src)?;
        let dst: ID3D11Resource = pipeline.staging[next].cast()?;
        let source: ID3D11Resource = pipeline.scaled.cast()?;
        // SAFETY: both resources are BGRA textures with identical small output
        // dimensions. The copy is queued and read on a later call.
        unsafe {
            self.context.CopyResource(&dst, &source);
        }
        pipeline.pending = Some(next);
        Ok(image)
    }

    fn ensure_pipeline(&mut self, src_width: u32, src_height: u32) -> WinResult<()> {
        let (out_width, out_height) =
            fit_within(src_width, src_height, self.max_width, self.max_height);
        if self.pipeline.as_ref().is_some_and(|pipeline| {
            pipeline.in_width == src_width
                && pipeline.in_height == src_height
                && pipeline.out_width == out_width
                && pipeline.out_height == out_height
        }) {
            return Ok(());
        }
        self.pipeline = Some(PreviewPipeline::new(
            &self.device,
            src_width,
            src_height,
            out_width,
            out_height,
        )?);
        Ok(())
    }
}

struct PreviewPipeline {
    video_context: ID3D11VideoContext,
    video_device: ID3D11VideoDevice,
    processor: ID3D11VideoProcessor,
    enumerator: ID3D11VideoProcessorEnumerator,
    scaled: ID3D11Texture2D,
    staging: [ID3D11Texture2D; 2],
    pending: Option<usize>,
    next_staging: usize,
    in_width: u32,
    in_height: u32,
    out_width: u32,
    out_height: u32,
}

impl PreviewPipeline {
    fn new(
        device: &ID3D11Device,
        in_width: u32,
        in_height: u32,
        out_width: u32,
        out_height: u32,
    ) -> WinResult<Self> {
        let video_device: ID3D11VideoDevice = device.cast()?;
        // SAFETY: trivial getter on a valid device.
        let video_context: ID3D11VideoContext = unsafe { device.GetImmediateContext()? }.cast()?;
        let (enumerator, processor) =
            create_video_processor(&video_device, in_width, in_height, out_width, out_height)?;
        configure_bgra_video_processor_color_spaces(&video_context, &processor);
        Ok(Self {
            video_context,
            video_device,
            processor,
            enumerator,
            scaled: create_bgra_texture(device, out_width, out_height)?,
            staging: [
                create_bgra_staging(device, out_width, out_height)?,
                create_bgra_staging(device, out_width, out_height)?,
            ],
            pending: None,
            next_staging: 0,
            in_width,
            in_height,
            out_width,
            out_height,
        })
    }

    fn scale_to_output(&self, bgra: &ID3D11Texture2D) -> WinResult<()> {
        let in_desc = D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC {
            FourCC: 0,
            ViewDimension: D3D11_VPIV_DIMENSION_TEXTURE2D,
            Anonymous: D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC_0 {
                Texture2D: D3D11_TEX2D_VPIV {
                    MipSlice: 0,
                    ArraySlice: 0,
                },
            },
        };
        let mut in_view = None;
        // SAFETY: bgra is a live texture on the same device; desc initialized.
        unsafe {
            self.video_device.CreateVideoProcessorInputView(
                bgra,
                &self.enumerator,
                &in_desc,
                Some(&mut in_view),
            )?;
        }

        let out_desc = D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC {
            ViewDimension: D3D11_VPOV_DIMENSION_TEXTURE2D,
            Anonymous: D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0 {
                Texture2D: D3D11_TEX2D_VPOV { MipSlice: 0 },
            },
        };
        let mut out_view = None;
        // SAFETY: scaled is a render-target BGRA texture on the same device.
        unsafe {
            self.video_device.CreateVideoProcessorOutputView(
                &self.scaled,
                &self.enumerator,
                &out_desc,
                Some(&mut out_view),
            )?;
        }

        let stream = D3D11_VIDEO_PROCESSOR_STREAM {
            Enable: true.into(),
            pInputSurface: std::mem::ManuallyDrop::new(in_view),
            ..Default::default()
        };
        // SAFETY: processor/views are live; one enabled stream, no past or
        // future frames. ManuallyDrop field is reclaimed immediately after.
        let result = unsafe {
            self.video_context.VideoProcessorBlt(
                &self.processor,
                out_view.as_ref().expect("out view set on Ok"),
                0,
                std::slice::from_ref(&stream),
            )
        };
        drop(std::mem::ManuallyDrop::into_inner(stream.pInputSurface));
        result
    }
}

fn create_video_processor(
    video_device: &ID3D11VideoDevice,
    in_w: u32,
    in_h: u32,
    out_w: u32,
    out_h: u32,
) -> WinResult<(ID3D11VideoProcessorEnumerator, ID3D11VideoProcessor)> {
    let desc = D3D11_VIDEO_PROCESSOR_CONTENT_DESC {
        InputFrameFormat: D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
        InputFrameRate: DXGI_RATIONAL {
            Numerator: 60,
            Denominator: 1,
        },
        InputWidth: in_w,
        InputHeight: in_h,
        OutputFrameRate: DXGI_RATIONAL {
            Numerator: 60,
            Denominator: 1,
        },
        OutputWidth: out_w,
        OutputHeight: out_h,
        Usage: D3D11_VIDEO_USAGE_PLAYBACK_NORMAL,
    };
    // SAFETY: desc is fully initialized; out-params are valid.
    let enumerator = unsafe { video_device.CreateVideoProcessorEnumerator(&desc)? };
    // SAFETY: enumerator is valid; rate-conversion caps index 0 always exists.
    let processor = unsafe { video_device.CreateVideoProcessor(&enumerator, 0)? };
    Ok((enumerator, processor))
}

fn configure_bgra_video_processor_color_spaces(
    video_context: &ID3D11VideoContext,
    processor: &ID3D11VideoProcessor,
) {
    if let Ok(ctx1) = video_context.cast::<ID3D11VideoContext1>() {
        unsafe {
            ctx1.VideoProcessorSetStreamColorSpace1(
                processor,
                0,
                DXGI_COLOR_SPACE_RGB_FULL_G22_NONE_P709,
            );
            ctx1.VideoProcessorSetOutputColorSpace1(
                processor,
                DXGI_COLOR_SPACE_RGB_FULL_G22_NONE_P709,
            );
        }
        return;
    }

    let rgb_full_709 = D3D11_VIDEO_PROCESSOR_COLOR_SPACE {
        _bitfield: (2 << 4),
    };
    unsafe {
        video_context.VideoProcessorSetStreamColorSpace(processor, 0, &rgb_full_709);
        video_context.VideoProcessorSetOutputColorSpace(processor, &rgb_full_709);
    }
}

fn map_bgra_staging_nonblocking(
    ctx: &ID3D11DeviceContext,
    resource: &ID3D11Resource,
    width: u32,
    height: u32,
) -> WinResult<Option<RgbaImage>> {
    let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
    let map = unsafe {
        ctx.Map(
            resource,
            0,
            D3D11_MAP_READ,
            D3D11_MAP_FLAG_DO_NOT_WAIT.0 as u32,
            Some(&mut mapped),
        )
    };
    match map {
        Ok(()) => {}
        Err(e) if e.code() == DXGI_ERROR_WAS_STILL_DRAWING => return Ok(None),
        Err(e) => return Err(e),
    }

    let image = unsafe { mapped_bgra_to_rgba(&mapped, width, height) };
    // SAFETY: the resource was successfully mapped above.
    unsafe {
        ctx.Unmap(resource, 0);
    }
    Ok(Some(image))
}

unsafe fn mapped_bgra_to_rgba(
    mapped: &D3D11_MAPPED_SUBRESOURCE,
    width: u32,
    height: u32,
) -> RgbaImage {
    let width = width as usize;
    let height = height as usize;
    let pitch = mapped.RowPitch as usize;
    let base = mapped.pData as *const u8;
    let mut out = vec![0u8; width * height * 4];
    for y in 0..height {
        let row = unsafe { base.add(y * pitch) };
        for x in 0..width {
            let px = unsafe { std::slice::from_raw_parts(row.add(x * 4), 4) };
            let i = (y * width + x) * 4;
            out[i] = px[2];
            out[i + 1] = px[1];
            out[i + 2] = px[0];
            out[i + 3] = 255;
        }
    }
    RgbaImage {
        width: width as u32,
        height: height as u32,
        pixels: out,
    }
}

fn fit_within(width: u32, height: u32, max_width: u32, max_height: u32) -> (u32, u32) {
    let width = width.max(1);
    let height = height.max(1);
    let max_width = max_width.max(1);
    let max_height = max_height.max(1);
    let scale_num = max_width
        .saturating_mul(height)
        .min(max_height.saturating_mul(width));
    let scale_den = width.saturating_mul(height).max(1);
    if scale_num >= scale_den {
        return (width, height);
    }
    let out_width = ((width as u64 * scale_num as u64) / scale_den as u64)
        .max(1)
        .min(width as u64) as u32;
    let out_height = ((height as u64 * scale_num as u64) / scale_den as u64)
        .max(1)
        .min(height as u64) as u32;
    (out_width, out_height)
}

pub fn texture_desc(texture: &ID3D11Texture2D) -> D3D11_TEXTURE2D_DESC {
    let mut desc = D3D11_TEXTURE2D_DESC::default();
    // SAFETY: GetDesc writes to a valid out-pointer.
    unsafe { texture.GetDesc(&mut desc) };
    desc
}

pub fn texture_size(texture: &ID3D11Texture2D) -> (u32, u32) {
    let mut desc = D3D11_TEXTURE2D_DESC::default();
    // SAFETY: GetDesc writes to a valid out-pointer.
    unsafe { texture.GetDesc(&mut desc) };
    (desc.Width, desc.Height)
}

pub fn copy_texture_region(
    context: &ID3D11DeviceContext,
    dst: &ID3D11Texture2D,
    src: &ID3D11Texture2D,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
) {
    let src_box = D3D11_BOX {
        left: x,
        top: y,
        front: 0,
        right: x + width,
        bottom: y + height,
        back: 1,
    };
    unsafe {
        context.CopySubresourceRegion(dst, 0, 0, 0, 0, src, 0, Some(&src_box));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_within_preserves_aspect_and_does_not_upscale() {
        assert_eq!(fit_within(1920, 1080, 1280, 720), (1280, 720));
        assert_eq!(fit_within(5120, 1440, 1280, 720), (1280, 360));
        assert_eq!(fit_within(640, 360, 1280, 720), (640, 360));
    }

    #[test]
    fn preview_readback_scales_before_cpu_mapping() {
        let (device, _) = match create_device() {
            Ok(device) => device,
            Err(e) => {
                eprintln!("SKIP: no hardware D3D11 device: {e}");
                return;
            }
        };
        let mut readback = match PreviewReadback::new(&device, 8, 8) {
            Ok(readback) => readback,
            Err(e) => {
                eprintln!("SKIP: preview readback unavailable: {e}");
                return;
            }
        };
        let texture = create_bgra_texture(&device, 16, 8).expect("texture");
        match readback.try_read(&texture) {
            Ok(frame) => assert!(frame.is_none()),
            Err(e) => {
                eprintln!("SKIP: preview readback unavailable: {e}");
                return;
            }
        }

        let mut image = None;
        for _ in 0..20 {
            match readback.try_read(&texture) {
                Ok(Some(frame)) => {
                    image = Some(frame);
                    break;
                }
                Ok(None) => std::thread::sleep(std::time::Duration::from_millis(5)),
                Err(e) => {
                    eprintln!("SKIP: preview readback unavailable after queue: {e}");
                    return;
                }
            }
        }
        let image = image.expect("preview frame");

        assert_eq!((image.width, image.height), (8, 4));
        assert_eq!(image.pixels.len(), 8 * 4 * 4);
    }
}
