//! Low-level Windows keyboard hook used as a fallback for games that do not
//! reliably deliver registered global shortcuts while focused.

use std::collections::BTreeSet;
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;

use windows_sys::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, VK_MBUTTON, VK_XBUTTON1, VK_XBUTTON2,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetMessageW, PeekMessageW, PostThreadMessageW,
    SetWindowsHookExW, TranslateMessage, UnhookWindowsHookEx, HC_ACTION, KBDLLHOOKSTRUCT,
    LLKHF_ALTDOWN, MSG, MSLLHOOKSTRUCT, PM_NOREMOVE, WH_KEYBOARD_LL, WH_MOUSE_LL, WM_KEYDOWN,
    WM_KEYUP, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_QUIT, WM_SYSKEYDOWN, WM_SYSKEYUP, WM_USER,
    WM_XBUTTONDOWN, WM_XBUTTONUP, XBUTTON1, XBUTTON2,
};

const VK_SHIFT_CODE: i32 = 0x10;
const VK_CONTROL_CODE: i32 = 0x11;
const VK_ALT_CODE: i32 = 0x12;
const VK_MBUTTON_CODE: u32 = VK_MBUTTON as u32;
const VK_XBUTTON1_CODE: u32 = VK_XBUTTON1 as u32;
const VK_XBUTTON2_CODE: u32 = VK_XBUTTON2 as u32;
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
    trigger_tx: Sender<()>,
    mouse_hook: Mutex<Option<MouseHookThread>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct MouseHookThread {
    thread_id: u32,
}

impl HookState {
    fn set_hotkey(&self, raw: &str) -> Result<(), String> {
        let parsed = parse_hook_hotkey(raw)?;
        if parsed.requires_mouse_hook() {
            self.ensure_mouse_hook()?;
        }
        let mut hotkey = self
            .hotkey
            .lock()
            .map_err(|_| "save hotkey lock poisoned".to_string())?;
        let requires_mouse_hook = parsed.requires_mouse_hook();
        *hotkey = parsed;
        drop(hotkey);
        if !requires_mouse_hook {
            self.stop_mouse_hook();
        }
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
            Ok(hotkey) => hotkey,
            Err(_) => return false,
        };
        if hotkey.matches(vk_code, ctrl, alt, shift) {
            let _ = self.trigger_tx.send(());
            return true;
        }
        false
    }

    fn on_key_up(&self, vk_code: u32) {
        if let Ok(mut down_keys) = self.down_keys.try_lock() {
            down_keys.remove(&vk_code);
        }
    }

    fn ensure_mouse_hook(&self) -> Result<(), String> {
        let mut mouse_hook = self
            .mouse_hook
            .lock()
            .map_err(|_| "save mouse hook lock poisoned".to_string())?;
        if mouse_hook.is_some() {
            return Ok(());
        }

        let (ready_tx, ready_rx) = mpsc::channel();
        thread::Builder::new()
            .name("clipline-save-mouse-hook".into())
            .spawn(move || run_mouse_hook(ready_tx))
            .map_err(|e| format!("spawn save mouse hotkey hook: {e}"))?;

        let thread_id = ready_rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .map_err(|e| format!("install save mouse hotkey hook: {e}"))??;
        *mouse_hook = Some(MouseHookThread { thread_id });
        Ok(())
    }

    fn stop_mouse_hook(&self) {
        let Ok(mut mouse_hook) = self.mouse_hook.lock() else {
            return;
        };
        if let Some(mouse_hook) = mouse_hook.take() {
            mouse_hook.stop();
        }
    }
}

impl HookHotkey {
    fn matches(&self, vk_code: u32, ctrl: bool, alt: bool, shift: bool) -> bool {
        self.key_vk == vk_code && self.ctrl == ctrl && self.alt == alt && self.shift == shift
    }

    fn requires_mouse_hook(&self) -> bool {
        matches!(
            self.key_vk,
            VK_MBUTTON_CODE | VK_XBUTTON1_CODE | VK_XBUTTON2_CODE
        )
    }
}

impl MouseHookThread {
    fn stop(self) {
        if unsafe { PostThreadMessageW(self.thread_id, WM_QUIT, 0, 0) } == 0 {
            eprintln!("low-level save mouse hotkey hook could not be stopped");
        }
    }
}

pub fn install_save_hook<F>(hotkey: &str, on_trigger: F) -> Result<(), String>
where
    F: Fn() + Send + Sync + 'static,
{
    if let Some(state) = SAVE_HOOK.get() {
        return state.set_hotkey(hotkey);
    }

    let parsed_hotkey = parse_hook_hotkey(hotkey)?;
    let (trigger_tx, trigger_rx) = mpsc::channel();
    thread::Builder::new()
        .name("clipline-save-hotkey-dispatch".into())
        .spawn(move || {
            while trigger_rx.recv().is_ok() {
                on_trigger();
            }
        })
        .map_err(|e| format!("spawn save hotkey dispatcher: {e}"))?;

    let state = Arc::new(HookState {
        hotkey: Mutex::new(parsed_hotkey.clone()),
        down_keys: Mutex::new(BTreeSet::new()),
        trigger_tx,
        mouse_hook: Mutex::new(None),
    });
    if parsed_hotkey.requires_mouse_hook() {
        state.ensure_mouse_hook()?;
    }
    SAVE_HOOK
        .set(state)
        .map_err(|_| "save hotkey hook was already installed".to_string())?;

    thread::Builder::new()
        .name("clipline-save-keyboard-hook".into())
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
            "Middle" => key_vk = Some(VK_MBUTTON_CODE),
            "Mouse4" => key_vk = Some(VK_XBUTTON1_CODE),
            "Mouse5" => key_vk = Some(VK_XBUTTON2_CODE),
            _ => return Err("unsupported hotkey part".into()),
        }
    }

    let key_vk = key_vk.ok_or_else(|| "hotkey needs a key".to_string())?;
    Ok(HookHotkey {
        ctrl,
        alt,
        shift,
        key_vk,
    })
}

fn run_keyboard_hook() {
    let keyboard_hook =
        unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_proc), std::ptr::null_mut(), 0) };
    if keyboard_hook.is_null() {
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

fn run_mouse_hook(ready_tx: Sender<Result<u32, String>>) {
    let mut msg = unsafe { std::mem::zeroed::<MSG>() };
    unsafe {
        PeekMessageW(
            &mut msg,
            std::ptr::null_mut(),
            WM_USER,
            WM_USER,
            PM_NOREMOVE,
        );
    }
    let hook = unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), std::ptr::null_mut(), 0) };
    if hook.is_null() {
        let _ = ready_tx.send(Err(
            "low-level save mouse hotkey hook could not be installed".into(),
        ));
        return;
    }

    let _ = ready_tx.send(Ok(unsafe { GetCurrentThreadId() }));
    while unsafe { GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) } > 0 {
        unsafe {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    unsafe {
        UnhookWindowsHookEx(hook);
    }
}

unsafe extern "system" fn keyboard_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let message = wparam as u32;
        let keyboard = unsafe { &*(lparam as *const KBDLLHOOKSTRUCT) };
        match message {
            WM_KEYDOWN | WM_SYSKEYDOWN => {
                if (VK_F1_CODE..=VK_F24_CODE).contains(&keyboard.vkCode) {
                    dispatch_key_down(keyboard.vkCode, (keyboard.flags & LLKHF_ALTDOWN) != 0);
                }
            }
            WM_KEYUP | WM_SYSKEYUP => {
                release_key(keyboard.vkCode);
            }
            _ => {}
        }
    }
    unsafe { CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam) }
}

unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let message = wparam as u32;
        match message {
            WM_MBUTTONDOWN => trigger_mouse_hotkey(VK_MBUTTON_CODE),
            WM_MBUTTONUP => release_mouse_hotkey(VK_MBUTTON_CODE),
            WM_XBUTTONDOWN | WM_XBUTTONUP => {
                let mouse = unsafe { &*(lparam as *const MSLLHOOKSTRUCT) };
                if let Some(vk_code) = xbutton_vk_code(mouse.mouseData) {
                    if message == WM_XBUTTONDOWN {
                        trigger_mouse_hotkey(vk_code);
                    } else {
                        release_mouse_hotkey(vk_code);
                    }
                }
            }
            _ => {}
        }
    }
    unsafe { CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam) }
}

fn trigger_mouse_hotkey(vk_code: u32) {
    dispatch_key_down(vk_code, false);
}

fn dispatch_key_down(vk_code: u32, alt_flag: bool) {
    let ctrl = key_is_down(VK_CONTROL_CODE);
    let shift = key_is_down(VK_SHIFT_CODE);
    let alt = alt_flag || key_is_down(VK_ALT_CODE);
    if let Some(state) = SAVE_HOOK.get() {
        state.on_key_down(vk_code, ctrl, alt, shift);
    }
}

fn release_mouse_hotkey(vk_code: u32) {
    release_key(vk_code);
}

fn release_key(vk_code: u32) {
    if let Some(state) = SAVE_HOOK.get() {
        state.on_key_up(vk_code);
    }
}

fn xbutton_vk_code(mouse_data: u32) -> Option<u32> {
    match ((mouse_data >> 16) & 0xffff) as u16 {
        XBUTTON1 => Some(VK_XBUTTON1_CODE),
        XBUTTON2 => Some(VK_XBUTTON2_CODE),
        _ => None,
    }
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
    fn parses_mouse_button_hotkeys_for_hook_matching() {
        let hotkey = parse_hook_hotkey("Ctrl+Mouse5").unwrap();

        assert!(hotkey.matches(0x06, true, false, false));
        assert!(!hotkey.matches(0x06, false, false, false));
        assert!(!hotkey.matches(0x05, true, false, false));

        let middle = parse_hook_hotkey("Shift+Middle").unwrap();
        assert!(middle.matches(0x04, false, false, true));

        let bare_mouse = parse_hook_hotkey("Mouse4").unwrap();
        assert!(bare_mouse.matches(0x05, false, false, false));
        assert!(!bare_mouse.matches(0x05, true, false, false));
    }

    #[test]
    fn rejects_reserved_f12_through_shared_normalizer() {
        assert!(parse_hook_hotkey("F12").is_err());
    }

    #[test]
    fn hook_requirement_tracks_mouse_button_hotkeys() {
        assert!(!parse_hook_hotkey("Alt+F10").unwrap().requires_mouse_hook());
        assert!(parse_hook_hotkey("Ctrl+Mouse5")
            .unwrap()
            .requires_mouse_hook());
        assert!(parse_hook_hotkey("Mouse5").unwrap().requires_mouse_hook());
    }

    #[test]
    fn hotkey_lock_contention_waits_instead_of_dropping_trigger() {
        let (trigger_tx, trigger_rx) = mpsc::channel();
        let state = Arc::new(HookState {
            hotkey: Mutex::new(parse_hook_hotkey("Ctrl+Mouse5").unwrap()),
            down_keys: Mutex::new(BTreeSet::new()),
            trigger_tx,
            mouse_hook: Mutex::new(None),
        });
        let guard = state.hotkey.lock().unwrap();
        let worker_state = Arc::clone(&state);
        let worker =
            thread::spawn(move || worker_state.on_key_down(VK_XBUTTON2_CODE, true, false, false));

        thread::sleep(std::time::Duration::from_millis(25));
        assert!(trigger_rx.try_recv().is_err());
        drop(guard);

        assert!(worker.join().unwrap());
        assert!(trigger_rx
            .recv_timeout(std::time::Duration::from_millis(250))
            .is_ok());
    }
}
