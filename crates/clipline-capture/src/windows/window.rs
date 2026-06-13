//! Find a visible top-level window by case-insensitive title substring
//! (`FindWindowW` is exact-match only, so enumerate).

use windows::core::BOOL;
use windows::Win32::Foundation::{HWND, LPARAM};
use windows::Win32::UI::WindowsAndMessaging::{EnumWindows, GetWindowTextW, IsWindowVisible};

struct Search {
    needle_lower: String,
    found: Option<HWND>,
}

pub fn find_window_by_title(needle: &str) -> Option<HWND> {
    let mut search = Search {
        needle_lower: needle.to_lowercase(),
        found: None,
    };
    // SAFETY: the callback only runs during this call; lparam points at
    // `search`, which outlives it. EnumWindows returns Err when the
    // callback stops enumeration early — our found case, not an error.
    unsafe {
        let _ = EnumWindows(Some(enum_proc), LPARAM(&mut search as *mut Search as isize));
    }
    search.found
}

unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    // SAFETY: lparam is the `Search` pointer passed by find_window_by_title
    // on this same thread, alive for the whole enumeration.
    let search = unsafe { &mut *(lparam.0 as *mut Search) };
    // SAFETY: hwnd comes from the enumeration; these are read-only queries.
    unsafe {
        if !IsWindowVisible(hwnd).as_bool() {
            return BOOL(1);
        }
        let mut buf = [0u16; 512];
        let len = GetWindowTextW(hwnd, &mut buf);
        if len == 0 {
            return BOOL(1);
        }
        let title = String::from_utf16_lossy(&buf[..len as usize]).to_lowercase();
        if title.contains(&search.needle_lower) {
            search.found = Some(hwnd);
            return BOOL(0); // stop enumeration
        }
    }
    BOOL(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_match_returns_none() {
        assert!(find_window_by_title("no window is named this 5f2c9a").is_none());
    }

    #[test]
    fn enumeration_path_does_not_crash() {
        // Result depends on the session (CI may have no titled windows);
        // this just exercises EnumWindows + the callback end to end.
        let _ = find_window_by_title("");
    }
}
