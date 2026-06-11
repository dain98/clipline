//! Thin safe wrappers over D3D11 device/texture creation, shared by WGC
//! capture now and the Media Foundation encoder milestone later.

use windows::core::{Interface, Result as WinResult};
use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct3D10::ID3D10Multithread;
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE, D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP,
};
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
    // SAFETY: trivial setter on a valid interface.
    unsafe { mt.SetMultithreadProtected(true) };
    Ok((device, context.expect("context out-param set on Ok")))
}

/// Default-usage BGRA texture, e.g. the destination for a capture-frame copy.
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
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
        CPUAccessFlags: 0,
        MiscFlags: 0,
    };
    let mut texture = None;
    // SAFETY: desc is fully initialized; out-param receives a valid pointer
    // on success.
    unsafe { device.CreateTexture2D(&desc, None, Some(&mut texture))? };
    Ok(texture.expect("texture out-param set on Ok"))
}

pub fn texture_size(texture: &ID3D11Texture2D) -> (u32, u32) {
    let mut desc = D3D11_TEXTURE2D_DESC::default();
    // SAFETY: GetDesc writes to a valid out-pointer.
    unsafe { texture.GetDesc(&mut desc) };
    (desc.Width, desc.Height)
}
