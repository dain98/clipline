//! GPU BGRA→NV12 conversion + scaling via the D3D11 video processor —
//! WGC delivers BGRA, H.264 encoder MFTs consume NV12; ddoc §3/§7 require
//! the path to stay on the GPU.

use windows::core::{Interface, Result as WinResult};
use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Direct3D11::{
    ID3D11Device, ID3D11Texture2D, ID3D11VideoContext, ID3D11VideoDevice, ID3D11VideoProcessor,
    ID3D11VideoProcessorEnumerator, D3D11_TEX2D_VPIV, D3D11_TEX2D_VPOV,
    D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE, D3D11_VIDEO_PROCESSOR_CONTENT_DESC,
    D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC, D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC_0,
    D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC, D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0,
    D3D11_VIDEO_PROCESSOR_STREAM, D3D11_VIDEO_USAGE_PLAYBACK_NORMAL,
    D3D11_VPIV_DIMENSION_TEXTURE2D, D3D11_VPOV_DIMENSION_TEXTURE2D,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_RATIONAL;

use crate::windows::d3d11;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CropRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl CropRect {
    fn to_rect(self) -> RECT {
        RECT {
            left: self.x as i32,
            top: self.y as i32,
            right: (self.x + self.width) as i32,
            bottom: (self.y + self.height) as i32,
        }
    }
}

/// One converter per recording: input size is fixed by the capture item,
/// output size by the encoder configuration (already even-rounded).
pub struct VideoConverter {
    device: ID3D11Device,
    video_context: ID3D11VideoContext,
    video_device: ID3D11VideoDevice,
    processor: ID3D11VideoProcessor,
    enumerator: ID3D11VideoProcessorEnumerator,
    out_width: u32,
    out_height: u32,
    source_rect: Option<RECT>,
}

impl VideoConverter {
    pub fn new(
        device: &ID3D11Device,
        in_w: u32,
        in_h: u32,
        out_w: u32,
        out_h: u32,
    ) -> WinResult<Self> {
        Self::new_with_crop(device, in_w, in_h, out_w, out_h, None)
    }

    pub fn new_with_crop(
        device: &ID3D11Device,
        in_w: u32,
        in_h: u32,
        out_w: u32,
        out_h: u32,
        crop: Option<CropRect>,
    ) -> WinResult<Self> {
        let video_device: ID3D11VideoDevice = device.cast()?;
        // SAFETY: trivial getter on a valid device.
        let video_context: ID3D11VideoContext = unsafe { device.GetImmediateContext()? }.cast()?;
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
        Ok(Self {
            device: device.clone(),
            video_context,
            video_device,
            processor,
            enumerator,
            out_width: out_w,
            out_height: out_h,
            source_rect: crop.map(CropRect::to_rect),
        })
    }

    /// Convert one BGRA texture into a freshly allocated NV12 texture
    /// (the encoder holds frames asynchronously; pooling is a follow-up).
    pub fn convert(&mut self, bgra: &ID3D11Texture2D) -> WinResult<ID3D11Texture2D> {
        let out = d3d11::create_nv12_texture(&self.device, self.out_width, self.out_height)?;

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

#[cfg(test)]
mod tests {
    use super::*;

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
