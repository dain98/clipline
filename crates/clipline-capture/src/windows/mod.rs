//! Windows platform layer. All COM/WinRT `unsafe` lives in this module
//! tree; everything exported is a safe wrapper honoring the platform
//! traits' contracts (see `crate::mock` for the reference behavior).

pub mod d3d11;
pub mod wgc;

pub use wgc::WgcCapture;

#[cfg(test)]
mod tests {
    use crate::traits::{Frame, FrameData};

    /// A GPU frame must round-trip through the platform-neutral `Frame`
    /// struct (Debug + Clone are derived; windows-rs COM wrappers provide
    /// both). WARP renders headless, so this runs on the CI runner too.
    #[test]
    fn gpu_frame_data_wraps_a_d3d11_texture() {
        let (device, _context) =
            super::d3d11::create_device_for_tests().expect("WARP D3D11 device");
        let texture =
            super::d3d11::create_bgra_texture(&device, 16, 16).expect("16x16 texture");
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
