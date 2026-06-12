//! Win32 monitor enumeration for display-region capture.

use windows::core::BOOL;
use windows::Win32::Foundation::{LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO, MONITORINFOEXW,
};

use crate::traits::CaptureError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayInfo {
    /// Win32 device id, e.g. `\\.\DISPLAY1`.
    pub id: String,
    /// Human-friendly fallback label for the UI.
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub is_primary: bool,
}

#[derive(Debug, Clone)]
pub struct DisplayHandle {
    pub handle: HMONITOR,
    pub info: DisplayInfo,
}

pub fn enumerate_displays() -> Result<Vec<DisplayInfo>, CaptureError> {
    Ok(enumerate_display_handles()?
        .into_iter()
        .map(|display| display.info)
        .collect())
}

pub fn display_handle_by_id(id: Option<&str>) -> Result<DisplayHandle, CaptureError> {
    let displays = enumerate_display_handles()?;
    if let Some(id) = id.filter(|id| !id.trim().is_empty()) {
        if let Some(display) = displays.iter().find(|display| display.info.id == id) {
            return Ok(display.clone());
        }
        return Err(CaptureError::Init(format!("display {id:?} was not found")));
    }
    displays
        .iter()
        .find(|display| display.info.is_primary)
        .or_else(|| displays.first())
        .cloned()
        .ok_or_else(|| CaptureError::Init("no displays found".into()))
}

fn enumerate_display_handles() -> Result<Vec<DisplayHandle>, CaptureError> {
    let mut displays = Vec::<DisplayHandle>::new();
    // SAFETY: the callback only runs during this call; lparam points at
    // `displays`, which outlives the enumeration.
    let ok = unsafe {
        EnumDisplayMonitors(
            None,
            None,
            Some(enum_monitor_proc),
            LPARAM(&mut displays as *mut Vec<DisplayHandle> as isize),
        )
    };
    if !ok.as_bool() {
        return Err(CaptureError::Init("EnumDisplayMonitors failed".into()));
    }
    Ok(displays)
}

unsafe extern "system" fn enum_monitor_proc(
    monitor: HMONITOR,
    _hdc: HDC,
    _rect: *mut RECT,
    lparam: LPARAM,
) -> BOOL {
    // SAFETY: lparam is the Vec pointer passed by enumerate_display_handles
    // on this same thread, alive for the whole enumeration.
    let displays = unsafe { &mut *(lparam.0 as *mut Vec<DisplayHandle>) };
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
    // SAFETY: monitor comes from EnumDisplayMonitors; info points to a
    // properly-sized MONITORINFOEXW whose first field is MONITORINFO.
    let ok = unsafe { GetMonitorInfoW(monitor, &mut info as *mut _ as *mut MONITORINFO) };
    if !ok.as_bool() {
        return BOOL(1);
    }
    let id = utf16_z(&info.szDevice);
    let rect = info.monitorInfo.rcMonitor;
    let width = (rect.right - rect.left).max(0) as u32;
    let height = (rect.bottom - rect.top).max(0) as u32;
    if width == 0 || height == 0 {
        return BOOL(1);
    }
    let name = id
        .strip_prefix(r"\\.\")
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| id.clone());
    displays.push(DisplayHandle {
        handle: monitor,
        info: DisplayInfo {
            id,
            name,
            x: rect.left,
            y: rect.top,
            width,
            height,
            is_primary: info.monitorInfo.dwFlags & 1 == 1,
        },
    });
    BOOL(1)
}

fn utf16_z(buf: &[u16]) -> String {
    let len = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..len])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf16_z_stops_at_nul() {
        assert_eq!(utf16_z(&[b'D' as u16, b'1' as u16, 0, b'X' as u16]), "D1");
    }

    #[test]
    fn display_enumeration_is_best_effort() {
        if std::env::var_os("CI").is_some() {
            eprintln!("SKIP: display enumeration needs an interactive desktop");
            return;
        }
        let displays = match enumerate_displays() {
            Ok(displays) => displays,
            Err(e) => {
                eprintln!("SKIP: no displays: {e}");
                return;
            }
        };
        assert!(!displays.is_empty());
        assert!(displays.iter().all(|d| d.width > 0 && d.height > 0));
    }
}
