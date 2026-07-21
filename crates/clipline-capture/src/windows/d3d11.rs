//! Thin safe wrappers over D3D11 device/texture creation, shared by WGC
//! capture now and the Media Foundation encoder milestone later.

use windows::core::{Error as WinError, Interface, Result as WinResult};
use windows::Win32::Foundation::{E_FAIL, HMODULE};
#[cfg(test)]
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_WARP;
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE, D3D_DRIVER_TYPE_HARDWARE};
use windows::Win32::Graphics::Direct3D10::ID3D10Multithread;
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D,
    D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE, D3D11_BOX, D3D11_CPU_ACCESS_READ,
    D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT,
    D3D11_USAGE_STAGING,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_NV12, DXGI_SAMPLE_DESC,
};

/// Create a hardware D3D11 device with BGRA support (required by WGC).
pub fn create_device() -> WinResult<(ID3D11Device, ID3D11DeviceContext)> {
    create_device_with(D3D_DRIVER_TYPE_HARDWARE)
}

/// WARP (software) device — headless-safe, used by tests.
#[cfg(test)]
pub(super) fn create_device_for_tests() -> WinResult<(ID3D11Device, ID3D11DeviceContext)> {
    create_device_with(D3D_DRIVER_TYPE_WARP)
}

#[cfg(test)]
pub(super) fn create_unprotected_device_for_tests() -> WinResult<(ID3D11Device, ID3D11DeviceContext)>
{
    create_device_unprotected_with(D3D_DRIVER_TYPE_WARP)
}

fn create_device_with(driver: D3D_DRIVER_TYPE) -> WinResult<(ID3D11Device, ID3D11DeviceContext)> {
    let (device, context) = create_device_unprotected_with(driver)?;
    ensure_multithread_protected(&device)?;
    Ok((device, context))
}

fn create_device_unprotected_with(
    driver: D3D_DRIVER_TYPE,
) -> WinResult<(ID3D11Device, ID3D11DeviceContext)> {
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
    Ok((device, context.expect("context out-param set on Ok")))
}

/// Establish the immediate-context serialization required whenever capture,
/// conversion, readback, and encoding share a caller-provided device.
pub(crate) fn ensure_multithread_protected(device: &ID3D11Device) -> WinResult<()> {
    let mt: ID3D10Multithread = device.cast()?;
    // SAFETY: these are trivial accessors on a live COM interface. The setter
    // returns the previous value, so query again to verify the invariant.
    if !unsafe { mt.GetMultithreadProtected() }.as_bool() {
        let _ = unsafe { mt.SetMultithreadProtected(true) };
    }
    if !unsafe { mt.GetMultithreadProtected() }.as_bool() {
        return Err(WinError::new(
            E_FAIL,
            "D3D11 device did not enable multithread protection",
        ));
    }
    Ok(())
}

/// Default-usage BGRA texture, e.g. the destination for a capture-frame
/// copy. RENDER_TARGET is included so the texture can feed video-processor
/// views (drivers reject input views on SHADER_RESOURCE-only textures).
pub(super) fn create_bgra_texture(
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
pub(super) fn create_nv12_texture(
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
pub(super) fn create_nv12_staging(
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

/// CPU-readable BGRA staging texture for the software encoder fallback.
pub(super) fn create_bgra_staging(
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
    unsafe { device.CreateTexture2D(&desc, None, Some(&mut texture))? };
    Ok(texture.expect("CreateTexture2D succeeded but returned null"))
}

pub(super) fn texture_desc(texture: &ID3D11Texture2D) -> D3D11_TEXTURE2D_DESC {
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

pub(super) fn copy_texture_region(
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
    fn multithread_guard_enables_an_unprotected_caller_device_idempotently() {
        let (device, _context) =
            create_unprotected_device_for_tests().expect("unprotected WARP device");
        let multithread: ID3D10Multithread = device.cast().expect("ID3D10Multithread");
        assert!(!unsafe { multithread.GetMultithreadProtected() }.as_bool());

        ensure_multithread_protected(&device).expect("enable multithread protection");
        assert!(unsafe { multithread.GetMultithreadProtected() }.as_bool());

        ensure_multithread_protected(&device).expect("idempotent protection check");
        assert!(unsafe { multithread.GetMultithreadProtected() }.as_bool());
    }
}
