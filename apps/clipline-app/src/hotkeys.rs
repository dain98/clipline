//! Low-level Windows keyboard hook used as a fallback for games that do not
//! reliably deliver registered global shortcuts while focused.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;

use windows_sys::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetMessageW, SetWindowsHookExW, TranslateMessage, HC_ACTION,
    KBDLLHOOKSTRUCT, LLKHF_ALTDOWN, MSG, WH_KEYBOARD_LL, WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN,
    WM_SYSKEYUP,
};

const VK_SHIFT_CODE: i32 = 0x10;
const VK_CONTROL_CODE: i32 = 0x11;
const VK_ALT_CODE: i32 = 0x12;
const VK_F1_CODE: u32 = 0x70;
const VK_F24_CODE: u32 = 0x87;

static SAVE_HOOK: OnceLock<Arc<HookState>> = OnceLock::new();

#[derive(Clone, Debug, PartialEq, Eq)]
struct HookHotkey {
    ctrl: bool,
    alt: bool,
    shift: bool,
    key_vk: u32,
}

struct HookState {
    hotkey: Mutex<HookHotkey>,
    down_keys: Mutex<BTreeSet<u32>>,
    on_trigger: Box<dyn Fn() + Send + Sync>,
}

impl HookState {
    fn set_hotkey(&self, raw: &str) -> Result<(), String> {
        let parsed = parse_hook_hotkey(raw)?;
        let mut hotkey = self
            .hotkey
            .lock()
            .map_err(|_| "save hotkey lock poisoned".to_string())?;
        *hotkey = parsed;
        Ok(())
    }

    fn on_key_down(&self, vk_code: u32, ctrl: bool, alt: bool, shift: bool) -> bool {
        let mut down_keys = match self.down_keys.lock() {
            Ok(keys) => keys,
            Err(_) => return false,
        };
        if !down_keys.insert(vk_code) {
            return false;
        }
        drop(down_keys);

        let hotkey = match self.hotkey.lock() {
            Ok(hotkey) => hotkey.clone(),
            Err(_) => return false,
        };
        if hotkey.matches(vk_code, ctrl, alt, shift) {
            (self.on_trigger)();
            return true;
        }
        false
    }

    fn on_key_up(&self, vk_code: u32) {
        if let Ok(mut down_keys) = self.down_keys.lock() {
            down_keys.remove(&vk_code);
        }
    }
}

impl HookHotkey {
    fn matches(&self, vk_code: u32, ctrl: bool, alt: bool, shift: bool) -> bool {
        self.key_vk == vk_code && self.ctrl == ctrl && self.alt == alt && self.shift == shift
    }
}

pub fn install_save_hook<F>(hotkey: &str, on_trigger: F) -> Result<(), String>
where
    F: Fn() + Send + Sync + 'static,
{
    if let Some(state) = SAVE_HOOK.get() {
        return state.set_hotkey(hotkey);
    }

    let state = Arc::new(HookState {
        hotkey: Mutex::new(parse_hook_hotkey(hotkey)?),
        down_keys: Mutex::new(BTreeSet::new()),
        on_trigger: Box::new(on_trigger),
    });
    SAVE_HOOK
        .set(state)
        .map_err(|_| "save hotkey hook was already installed".to_string())?;

    thread::Builder::new()
        .name("clipline-save-hotkey-hook".into())
        .spawn(run_keyboard_hook)
        .map_err(|e| format!("spawn save hotkey hook: {e}"))?;
    Ok(())
}

pub fn set_save_hotkey(hotkey: &str) -> Result<(), String> {
    if let Some(state) = SAVE_HOOK.get() {
        state.set_hotkey(hotkey)?;
    }
    Ok(())
}

fn parse_hook_hotkey(raw: &str) -> Result<HookHotkey, String> {
    let normalized = crate::settings::normalize_hotkey(raw)?;
    let mut ctrl = false;
    let mut alt = false;
    let mut shift = false;
    let mut key_vk = None;

    for part in normalized.split('+') {
        match part {
            "Ctrl" => ctrl = true,
            "Alt" => alt = true,
            "Shift" => shift = true,
            key if key.starts_with('F') => {
                let number = key[1..]
                    .parse::<u32>()
                    .map_err(|_| "hotkey key must be an F-key".to_string())?;
                key_vk = Some(VK_F1_CODE + number - 1);
            }
            _ => return Err("unsupported hotkey part".into()),
        }
    }

    let key_vk = key_vk.ok_or_else(|| "hotkey needs an F-key".to_string())?;
    if !(VK_F1_CODE..=VK_F24_CODE).contains(&key_vk) {
        return Err("hotkey key must be F1-F11 or F13-F24".into());
    }
    Ok(HookHotkey {
        ctrl,
        alt,
        shift,
        key_vk,
    })
}

fn run_keyboard_hook() {
    let hook =
        unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_proc), std::ptr::null_mut(), 0) };
    if hook.is_null() {
        eprintln!("low-level save hotkey hook could not be installed");
        return;
    }

    let mut msg = unsafe { std::mem::zeroed::<MSG>() };
    while unsafe { GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) } > 0 {
        unsafe {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

unsafe extern "system" fn keyboard_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let message = wparam as u32;
        let keyboard = unsafe { &*(lparam as *const KBDLLHOOKSTRUCT) };
        match message {
            WM_KEYDOWN | WM_SYSKEYDOWN => {
                if (VK_F1_CODE..=VK_F24_CODE).contains(&keyboard.vkCode) {
                    let ctrl = key_is_down(VK_CONTROL_CODE);
                    let shift = key_is_down(VK_SHIFT_CODE);
                    let alt = (keyboard.flags & LLKHF_ALTDOWN) != 0 || key_is_down(VK_ALT_CODE);
                    if let Some(state) = SAVE_HOOK.get() {
                        state.on_key_down(keyboard.vkCode, ctrl, alt, shift);
                    }
                }
            }
            WM_KEYUP | WM_SYSKEYUP => {
                if let Some(state) = SAVE_HOOK.get() {
                    state.on_key_up(keyboard.vkCode);
                }
            }
            _ => {}
        }
    }
    unsafe { CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam) }
}

fn key_is_down(vk: i32) -> bool {
    unsafe { (GetAsyncKeyState(vk) & i16::MIN) != 0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_function_key_hotkeys_for_hook_matching() {
        let hotkey = parse_hook_hotkey("Ctrl+Alt+F9").unwrap();

        assert!(hotkey.matches(VK_F1_CODE + 8, true, true, false));
        assert!(!hotkey.matches(VK_F1_CODE + 8, true, true, true));
        assert!(!hotkey.matches(VK_F1_CODE + 9, true, true, false));
    }

    #[test]
    fn rejects_reserved_f12_through_shared_normalizer() {
        assert!(parse_hook_hotkey("F12").is_err());
    }
}
