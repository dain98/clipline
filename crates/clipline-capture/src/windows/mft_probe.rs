//! Real encoder probing (ddoc §3) via MFTEnumEx: which hardware vendors
//! offer encoder MFTs for which codecs, plus the Microsoft software H.264
//! MFT as the last-resort tier.

use windows::core::GUID;
use windows::Win32::Media::MediaFoundation::{
    IMFActivate, MFStartup, MFTEnumEx, MFMediaType_Video, MFSTARTUP_FULL,
    MFT_CATEGORY_VIDEO_ENCODER, MFT_ENUM_FLAG_HARDWARE, MFT_ENUM_FLAG_SORTANDFILTER,
    MFT_ENUM_FLAG_SYNCMFT, MFT_ENUM_HARDWARE_VENDOR_ID_Attribute, MFT_REGISTER_TYPE_INFO,
    MFVideoFormat_AV1, MFVideoFormat_H264, MFVideoFormat_HEVC, MF_VERSION,
};
use windows::Win32::System::Com::CoTaskMemFree;

use crate::probe::{Codec, EncoderBackend, EncoderCapability};

/// PCI vendor id (as MFT_ENUM_HARDWARE_VENDOR_ID reports it) → backend.
pub fn backend_for_vendor(vendor: &str) -> Option<EncoderBackend> {
    match vendor.trim_end_matches('\0').trim() {
        "VEN_10DE" => Some(EncoderBackend::Nvenc),
        "VEN_1002" => Some(EncoderBackend::Amf),
        "VEN_8086" => Some(EncoderBackend::QuickSync),
        _ => None,
    }
}

/// Make sure Media Foundation is up. Refcounted by the OS; never paired
/// with MFShutdown — the process uses MF for its whole lifetime.
pub fn ensure_mf_started() -> windows::core::Result<()> {
    // SAFETY: MFStartup is safe to call repeatedly.
    unsafe { MFStartup(MF_VERSION, MFSTARTUP_FULL) }
}

/// Enumerate activates for one codec/flag combination. The returned
/// activates are released on drop; the CoTaskMem array is freed here.
pub(crate) fn enum_activates(
    subtype: GUID,
    flags: windows::Win32::Media::MediaFoundation::MFT_ENUM_FLAG,
) -> windows::core::Result<Vec<IMFActivate>> {
    let out_info =
        MFT_REGISTER_TYPE_INFO { guidMajorType: MFMediaType_Video, guidSubtype: subtype };
    let mut activates: *mut Option<IMFActivate> = std::ptr::null_mut();
    let mut count = 0u32;
    // SAFETY: out-params receive a CoTaskMem array of COM pointers; we take
    // ownership of each element and free the array afterwards.
    unsafe {
        MFTEnumEx(
            MFT_CATEGORY_VIDEO_ENCODER,
            flags,
            None,
            Some(&out_info),
            &mut activates,
            &mut count,
        )?;
        // No matches → the out array may stay null (e.g. AV1 on pre-RDNA3).
        if count == 0 || activates.is_null() {
            return Ok(Vec::new());
        }
        let slice = std::slice::from_raw_parts_mut(activates, count as usize);
        let owned: Vec<IMFActivate> = slice.iter_mut().filter_map(|a| a.take()).collect();
        CoTaskMemFree(Some(activates as *const _));
        Ok(owned)
    }
}

fn vendor_of(activate: &IMFActivate) -> Option<String> {
    let mut buf = [0u16; 64];
    let mut len = 0u32;
    // SAFETY: fixed-size out buffer; GetString writes at most buf.len() chars.
    unsafe {
        activate
            .GetString(&MFT_ENUM_HARDWARE_VENDOR_ID_Attribute, &mut buf, Some(&mut len))
            .ok()?;
    }
    Some(String::from_utf16_lossy(&buf[..len as usize]))
}

/// MF-backed implementation of the ddoc §3 probe.
pub fn enumerate() -> windows::core::Result<Vec<EncoderCapability>> {
    ensure_mf_started()?;
    let mut by_backend: Vec<(EncoderBackend, Vec<Codec>)> = Vec::new();
    for (subtype, codec) in [
        (MFVideoFormat_AV1, Codec::Av1),
        (MFVideoFormat_HEVC, Codec::Hevc),
        (MFVideoFormat_H264, Codec::H264),
    ] {
        for activate in
            enum_activates(subtype, MFT_ENUM_FLAG_HARDWARE | MFT_ENUM_FLAG_SORTANDFILTER)?
        {
            let Some(backend) = vendor_of(&activate).and_then(|v| backend_for_vendor(&v)) else {
                continue;
            };
            match by_backend.iter_mut().find(|(b, _)| *b == backend) {
                Some((_, codecs)) if !codecs.contains(&codec) => codecs.push(codec),
                Some(_) => {}
                None => by_backend.push((backend, vec![codec])),
            }
        }
    }
    // Software H.264 (sync MFT — Microsoft's encoder) as the last resort.
    if !enum_activates(MFVideoFormat_H264, MFT_ENUM_FLAG_SYNCMFT | MFT_ENUM_FLAG_SORTANDFILTER)?
        .is_empty()
    {
        by_backend.push((EncoderBackend::MfSoftware, vec![Codec::H264]));
    }
    Ok(by_backend
        .into_iter()
        .map(|(backend, codecs)| EncoderCapability { backend, codecs })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::probe::EncoderBackend;

    /// Enumeration itself must work anywhere Media Foundation exists;
    /// result contents depend on hardware. Self-skips if MF is absent
    /// (Server without the Media Foundation feature).
    #[test]
    fn enumerate_returns_without_error() {
        let caps = match enumerate() {
            Ok(caps) => caps,
            Err(e) => {
                eprintln!("SKIP: Media Foundation unavailable: {e}");
                return;
            }
        };
        for c in &caps {
            assert!(!c.codecs.is_empty(), "empty-codec entries are filtered");
        }
        eprintln!("encoders found: {caps:?}");
    }

    #[test]
    fn vendor_ids_map_to_backends() {
        assert_eq!(backend_for_vendor("VEN_10DE"), Some(EncoderBackend::Nvenc));
        assert_eq!(backend_for_vendor("VEN_1002"), Some(EncoderBackend::Amf));
        assert_eq!(backend_for_vendor("VEN_8086"), Some(EncoderBackend::QuickSync));
        assert_eq!(backend_for_vendor("VEN_FFFF"), None);
    }
}
