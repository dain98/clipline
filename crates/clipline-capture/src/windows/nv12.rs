//! GPU BGRA→NV12 conversion + scaling via the D3D11 video processor —
//! WGC delivers BGRA, H.264 encoder MFTs consume NV12; ddoc §3/§7 require
//! the path to stay on the GPU.

use windows::core::{Interface, Result as WinResult};
use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_VIDEO_COLOR_YCbCrA, ID3D11Device, ID3D11Resource, ID3D11Texture2D, ID3D11VideoContext,
    ID3D11VideoContext1, ID3D11VideoDevice, ID3D11VideoProcessor, ID3D11VideoProcessorEnumerator,
    D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_READ, D3D11_TEX2D_VPIV, D3D11_TEX2D_VPOV,
    D3D11_VIDEO_COLOR, D3D11_VIDEO_COLOR_0, D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
    D3D11_VIDEO_PROCESSOR_COLOR_SPACE, D3D11_VIDEO_PROCESSOR_CONTENT_DESC,
    D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC, D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC_0,
    D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC, D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0,
    D3D11_VIDEO_PROCESSOR_STREAM, D3D11_VIDEO_USAGE_PLAYBACK_NORMAL,
    D3D11_VPIV_DIMENSION_TEXTURE2D, D3D11_VPOV_DIMENSION_TEXTURE2D,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_COLOR_SPACE_RGB_FULL_G22_NONE_P709, DXGI_COLOR_SPACE_YCBCR_STUDIO_G22_LEFT_P709,
    DXGI_RATIONAL,
};

use crate::windows::d3d11;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CropRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl CropRect {
    pub fn in_frame(self, frame_width: u32, frame_height: u32) -> Option<Self> {
        let right = self.x.checked_add(self.width)?;
        let bottom = self.y.checked_add(self.height)?;
        if self.width < 2 || self.height < 2 || right > frame_width || bottom > frame_height {
            return None;
        }
        Some(self)
    }

    pub fn from_i32_in_frame(
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        frame_width: i32,
        frame_height: i32,
    ) -> Option<Self> {
        if x < 0 || y < 0 {
            return None;
        }
        let frame_width = u32::try_from(frame_width).ok()?;
        let frame_height = u32::try_from(frame_height).ok()?;
        Self {
            x: u32::try_from(x).ok()?,
            y: u32::try_from(y).ok()?,
            width: u32::try_from(width).ok()?,
            height: u32::try_from(height).ok()?,
        }
        .in_frame(frame_width, frame_height)
    }

    pub fn is_full_frame(self, frame_width: u32, frame_height: u32) -> bool {
        self.x == 0 && self.y == 0 && self.width == frame_width && self.height == frame_height
    }

    fn to_rect(self) -> RECT {
        RECT {
            left: self.x as i32,
            top: self.y as i32,
            right: (self.x + self.width) as i32,
            bottom: (self.y + self.height) as i32,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResizeMode {
    Stretch,
    Fit,
}

/// One converter per recording. The MP4 output size stays fixed, but window
/// captures can resize, so the video processor is rebuilt when input changes.
pub struct VideoConverter {
    device: ID3D11Device,
    video_context: ID3D11VideoContext,
    video_device: ID3D11VideoDevice,
    processor: ID3D11VideoProcessor,
    enumerator: ID3D11VideoProcessorEnumerator,
    in_width: u32,
    in_height: u32,
    out_width: u32,
    out_height: u32,
    source_rect: Option<RECT>,
    resize_mode: ResizeMode,
    black_frame: Option<Vec<u8>>,
}

impl VideoConverter {
    pub fn new(
        device: &ID3D11Device,
        in_w: u32,
        in_h: u32,
        out_w: u32,
        out_h: u32,
    ) -> WinResult<Self> {
        Self::new_with_crop_and_resize(device, in_w, in_h, out_w, out_h, None, ResizeMode::Stretch)
    }

    pub fn new_with_crop(
        device: &ID3D11Device,
        in_w: u32,
        in_h: u32,
        out_w: u32,
        out_h: u32,
        crop: Option<CropRect>,
    ) -> WinResult<Self> {
        Self::new_with_crop_and_resize(device, in_w, in_h, out_w, out_h, crop, ResizeMode::Stretch)
    }

    pub fn new_with_crop_and_resize(
        device: &ID3D11Device,
        in_w: u32,
        in_h: u32,
        out_w: u32,
        out_h: u32,
        crop: Option<CropRect>,
        resize_mode: ResizeMode,
    ) -> WinResult<Self> {
        let video_device: ID3D11VideoDevice = device.cast()?;
        // SAFETY: trivial getter on a valid device.
        let video_context: ID3D11VideoContext = unsafe { device.GetImmediateContext()? }.cast()?;
        let (enumerator, processor) =
            create_video_processor(&video_device, in_w, in_h, out_w, out_h)?;
        configure_video_processor_color_spaces(&video_context, &processor);
        Ok(Self {
            device: device.clone(),
            video_context,
            video_device,
            processor,
            enumerator,
            in_width: in_w,
            in_height: in_h,
            out_width: out_w,
            out_height: out_h,
            source_rect: crop.map(CropRect::to_rect),
            resize_mode,
            black_frame: (resize_mode == ResizeMode::Fit).then(|| black_nv12_frame(out_w, out_h)),
        })
    }

    /// Convert one BGRA texture into a freshly allocated NV12 texture
    /// (the encoder holds frames asynchronously; pooling is a follow-up).
    pub fn convert(&mut self, bgra: &ID3D11Texture2D) -> WinResult<ID3D11Texture2D> {
        let (in_width, in_height) = d3d11::texture_size(bgra);
        if (in_width, in_height) != (self.in_width, self.in_height) {
            let (enumerator, processor) = create_video_processor(
                &self.video_device,
                in_width,
                in_height,
                self.out_width,
                self.out_height,
            )?;
            configure_video_processor_color_spaces(&self.video_context, &processor);
            self.enumerator = enumerator;
            self.processor = processor;
            self.in_width = in_width;
            self.in_height = in_height;
        }
        let out = if let Some(black_frame) = &self.black_frame {
            d3d11::create_nv12_texture_from_bytes(
                &self.device,
                self.out_width,
                self.out_height,
                black_frame,
            )?
        } else {
            d3d11::create_nv12_texture(&self.device, self.out_width, self.out_height)?
        };

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
        // SAFETY: bgra is a live texture on self.device; desc initialized.
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
        // SAFETY: out is a render-target NV12 texture on self.device.
        unsafe {
            self.video_device.CreateVideoProcessorOutputView(
                &out,
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
        set_output_background_black(&self.video_context, &self.processor);
        if let Some(rect) = &self.source_rect {
            // SAFETY: processor is live and `rect` is a valid source rectangle
            // for stream 0. The caller validates the crop against the input.
            unsafe {
                self.video_context.VideoProcessorSetStreamSourceRect(
                    &self.processor,
                    0,
                    true,
                    Some(rect),
                );
            }
        }
        if self.resize_mode == ResizeMode::Fit {
            let rect = aspect_fit_rect(in_width, in_height, self.out_width, self.out_height);
            unsafe {
                self.video_context.VideoProcessorSetOutputTargetRect(
                    &self.processor,
                    true,
                    Some(&rect),
                );
            }
        } else {
            unsafe {
                self.video_context
                    .VideoProcessorSetOutputTargetRect(&self.processor, false, None);
            }
        }
        // SAFETY: processor/views are live; one enabled stream, no past or
        // future frames. ManuallyDrop field: we drop the view ourselves after.
        let result = unsafe {
            self.video_context.VideoProcessorBlt(
                &self.processor,
                out_view.as_ref().expect("out view set on Ok"),
                0,
                std::slice::from_ref(&stream),
            )
        };
        // Reclaim the input view reference wrapped in ManuallyDrop.
        drop(std::mem::ManuallyDrop::into_inner(stream.pInputSurface));
        result?;
        Ok(out)
    }
}

fn set_output_background_black(
    video_context: &ID3D11VideoContext,
    processor: &ID3D11VideoProcessor,
) {
    let color = limited_ycbcr_black();
    // SAFETY: `processor` belongs to this live video context, and `color`
    // points to a valid YCbCr background for the duration of the call.
    unsafe {
        video_context.VideoProcessorSetOutputBackgroundColor(processor, true, &color);
    }
}

fn limited_ycbcr_black() -> D3D11_VIDEO_COLOR {
    D3D11_VIDEO_COLOR {
        Anonymous: D3D11_VIDEO_COLOR_0 {
            YCbCr: D3D11_VIDEO_COLOR_YCbCrA {
                Y: 16.0 / 255.0,
                Cb: 0.5,
                Cr: 0.5,
                A: 1.0,
            },
        },
    }
}

fn black_nv12_frame(width: u32, height: u32) -> Vec<u8> {
    let y_len = width as usize * height as usize;
    let mut frame = vec![16u8; y_len + y_len / 2];
    frame[y_len..].fill(128);
    frame
}

fn aspect_fit_rect(in_w: u32, in_h: u32, out_w: u32, out_h: u32) -> RECT {
    let in_w = in_w.max(1) as f64;
    let in_h = in_h.max(1) as f64;
    let out_w_f = out_w.max(1) as f64;
    let out_h_f = out_h.max(1) as f64;
    let scale = (out_w_f / in_w).min(out_h_f / in_h);
    let fit_w = (in_w * scale).round().clamp(1.0, out_w_f) as i32;
    let fit_h = (in_h * scale).round().clamp(1.0, out_h_f) as i32;
    let left = (out_w as i32 - fit_w) / 2;
    let top = (out_h as i32 - fit_h) / 2;
    RECT {
        left,
        top,
        right: left + fit_w,
        bottom: top + fit_h,
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

fn configure_video_processor_color_spaces(
    video_context: &ID3D11VideoContext,
    processor: &ID3D11VideoProcessor,
) {
    // WGC gives us SDR BGRA in full-range RGB. Encoders receive NV12, which
    // standard players expect as limited-range Rec.709 for screen recordings.
    // Letting drivers infer this caused dark/saturated clips on some systems.
    if let Ok(ctx1) = video_context.cast::<ID3D11VideoContext1>() {
        unsafe {
            ctx1.VideoProcessorSetStreamColorSpace1(
                processor,
                0,
                DXGI_COLOR_SPACE_RGB_FULL_G22_NONE_P709,
            );
            ctx1.VideoProcessorSetOutputColorSpace1(
                processor,
                DXGI_COLOR_SPACE_YCBCR_STUDIO_G22_LEFT_P709,
            );
        }
        return;
    }

    // Older D3D11.0 fallback: Usage=playback, RGB input full-range, YCbCr
    // output with BT.709 matrix and 16-235 nominal range.
    let rgb_full_709 = D3D11_VIDEO_PROCESSOR_COLOR_SPACE {
        _bitfield: (2 << 4),
    };
    let ycbcr_limited_709 = D3D11_VIDEO_PROCESSOR_COLOR_SPACE {
        _bitfield: (1 << 2) | (1 << 4),
    };
    unsafe {
        video_context.VideoProcessorSetStreamColorSpace(processor, 0, &rgb_full_709);
        video_context.VideoProcessorSetOutputColorSpace(processor, &ycbcr_limited_709);
    }
}

/// Copy a GPU NV12 texture to a staging texture and pack it into contiguous
/// NV12 bytes (Y plane `width*height`, then interleaved UV `width*height/2`)
/// for piping to FFmpeg. Maps with the texture's row pitch and reads the UV
/// plane immediately after the Y plane — the conventional D3D11 NV12 layout.
/// Dimensions come from `src` itself so the packed size can't drift from the
/// texture the caller actually produced.
pub fn read_nv12(device: &ID3D11Device, src: &ID3D11Texture2D) -> WinResult<Vec<u8>> {
    let (width, height) = d3d11::texture_size(src);
    let staging = d3d11::create_nv12_staging(device, width, height)?;
    let dst: ID3D11Resource = staging.cast()?;
    let source: ID3D11Resource = src.cast()?;
    // SAFETY: trivial getter; the device owns a single immediate context.
    let ctx = unsafe { device.GetImmediateContext()? };
    let (w, h) = (width as usize, height as usize);
    let mut out = vec![0u8; w * h * 3 / 2];
    // SAFETY: dst/source are valid resources of identical NV12 descs; the
    // staging texture is CPU-readable and mapped for read below. The mapped
    // pointer is valid until Unmap, which we always call before returning.
    unsafe {
        ctx.CopyResource(&dst, &source);
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        ctx.Map(&dst, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;
        let pitch = mapped.RowPitch as usize;
        let base = mapped.pData as *const u8;
        for row in 0..h {
            let src_row = std::slice::from_raw_parts(base.add(row * pitch), w);
            out[row * w..(row + 1) * w].copy_from_slice(src_row);
        }
        let uv_base = base.add(pitch * h);
        let uv_out = w * h;
        for row in 0..h / 2 {
            let src_row = std::slice::from_raw_parts(uv_base.add(row * pitch), w);
            out[uv_out + row * w..uv_out + (row + 1) * w].copy_from_slice(src_row);
        }
        ctx.Unmap(&dst, 0);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aspect_fit_rect_keeps_same_aspect_full_frame() {
        let rect = aspect_fit_rect(1920, 1080, 1280, 720);
        assert_eq!(
            (rect.left, rect.top, rect.right, rect.bottom),
            (0, 0, 1280, 720)
        );
    }

    #[test]
    fn aspect_fit_rect_pillarboxes_wide_output() {
        let rect = aspect_fit_rect(4, 3, 1920, 1080);
        assert_eq!(
            (rect.left, rect.top, rect.right, rect.bottom),
            (240, 0, 1680, 1080)
        );
    }

    #[test]
    fn aspect_fit_rect_letterboxes_tall_output() {
        let rect = aspect_fit_rect(16, 9, 1000, 1000);
        assert_eq!(
            (rect.left, rect.top, rect.right, rect.bottom),
            (0, 218, 1000, 781)
        );
    }

    #[test]
    fn fit_background_black_uses_limited_range_ycbcr() {
        let color = limited_ycbcr_black();
        // SAFETY: limited_ycbcr_black initializes the YCbCr union arm.
        let ycbcr = unsafe { color.Anonymous.YCbCr };

        assert!((ycbcr.Y - 16.0 / 255.0).abs() < f32::EPSILON);
        assert_eq!(ycbcr.Cb, 0.5);
        assert_eq!(ycbcr.Cr, 0.5);
        assert_eq!(ycbcr.A, 1.0);
    }

    #[test]
    fn reads_back_converted_nv12_as_contiguous_bytes() {
        // WARP has no ID3D11VideoDevice — needs real hardware; skips on CI.
        let device = match crate::windows::d3d11::create_device() {
            Ok((device, _ctx)) => device,
            Err(e) => {
                eprintln!("SKIP: no hardware D3D11 device: {e}");
                return;
            }
        };
        let mut conv = match VideoConverter::new(&device, 64, 64, 64, 64) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("SKIP: video processor unavailable: {e}");
                return;
            }
        };
        let src = crate::windows::d3d11::create_bgra_texture(&device, 64, 64).expect("src");
        let nv12 = conv.convert(&src).expect("convert");
        let bytes = read_nv12(&device, &nv12).expect("readback");
        assert_eq!(bytes.len(), 64 * 64 * 3 / 2, "tightly packed NV12");
    }

    #[test]
    fn converts_bgra_texture_to_nv12_with_scaling() {
        // WARP has no ID3D11VideoDevice — needs real hardware; skips on CI.
        let device = match crate::windows::d3d11::create_device() {
            Ok((device, _ctx)) => device,
            Err(e) => {
                eprintln!("SKIP: no hardware D3D11 device: {e}");
                return;
            }
        };
        let mut conv = match VideoConverter::new(&device, 64, 64, 32, 32) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("SKIP: video processor unavailable: {e}");
                return;
            }
        };
        let src = crate::windows::d3d11::create_bgra_texture(&device, 64, 64).expect("src");
        let nv12 = conv.convert(&src).expect("convert");
        let desc = crate::windows::d3d11::texture_desc(&nv12);
        assert_eq!((desc.Width, desc.Height), (32, 32));
        assert_eq!(
            desc.Format,
            windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_NV12
        );
    }

    #[test]
    fn fit_letterbox_background_initializes_chroma_black() {
        // WARP has no ID3D11VideoDevice — needs real hardware; skips on CI.
        let device = match crate::windows::d3d11::create_device() {
            Ok((device, _ctx)) => device,
            Err(e) => {
                eprintln!("SKIP: no hardware D3D11 device: {e}");
                return;
            }
        };
        let mut conv = match VideoConverter::new_with_crop_and_resize(
            &device,
            64,
            32,
            64,
            64,
            None,
            ResizeMode::Fit,
        ) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("SKIP: video processor unavailable: {e}");
                return;
            }
        };
        let pixels = vec![0x80; 64 * 32 * 4];
        let src = crate::windows::d3d11::create_bgra_texture_from_pixels(&device, 64, 32, &pixels)
            .expect("src");
        let nv12 = conv.convert(&src).expect("convert");
        let bytes = read_nv12(&device, &nv12).expect("readback");
        let y_plane_len = 64 * 64;
        let top_y = &bytes[..64];
        let top_uv = &bytes[y_plane_len..y_plane_len + 64];

        assert!(
            top_y.iter().all(|&y| (12..=20).contains(&y)),
            "top letterbox Y should be limited black, got {top_y:?}"
        );
        assert!(
            top_uv.iter().all(|&uv| (120..=136).contains(&uv)),
            "top letterbox UV should be neutral chroma, got {top_uv:?}"
        );
    }

    #[test]
    fn converts_bgra_texture_with_source_crop() {
        // WARP has no ID3D11VideoDevice — needs real hardware; skips on CI.
        let device = match crate::windows::d3d11::create_device() {
            Ok((device, _ctx)) => device,
            Err(e) => {
                eprintln!("SKIP: no hardware D3D11 device: {e}");
                return;
            }
        };
        let crop = CropRect {
            x: 16,
            y: 8,
            width: 32,
            height: 24,
        };
        let mut conv = match VideoConverter::new_with_crop(&device, 96, 64, 48, 36, Some(crop)) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("SKIP: video processor unavailable: {e}");
                return;
            }
        };
        let src = crate::windows::d3d11::create_bgra_texture(&device, 96, 64).expect("src");
        let nv12 = conv.convert(&src).expect("convert");
        let desc = crate::windows::d3d11::texture_desc(&nv12);
        assert_eq!((desc.Width, desc.Height), (48, 36));
        assert_eq!(
            desc.Format,
            windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_NV12
        );
    }
}
