//! Low-level Windows keyboard hook used as a fallback for games that do not
//! reliably deliver registered global shortcuts while focused.

use std::collections::BTreeSet;
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::Duration;

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

static SAVE_HOOK: OnceLock<Mutex<Option<Arc<HookState>>>> = OnceLock::new();

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
            .recv_timeout(Duration::from_secs(2))
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
    if let Some(state) = current_save_hook() {
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
    publish_save_hook(state.clone())?;
    if parsed_hotkey.requires_mouse_hook() {
        if let Err(e) = state.ensure_mouse_hook() {
            clear_save_hook(&state);
            return Err(e);
        }
    }
    let (ready_tx, ready_rx) = mpsc::channel();
    if let Err(e) = thread::Builder::new()
        .name("clipline-save-keyboard-hook".into())
        .spawn(move || run_keyboard_hook(ready_tx))
        .map_err(|e| format!("spawn save hotkey hook: {e}"))
    {
        clear_save_hook(&state);
        return Err(e);
    }
    if let Err(e) = ready_rx
        .recv_timeout(Duration::from_secs(2))
        .map_err(|e| format!("install save hotkey hook: {e}"))
        .and_then(|result| result)
    {
        clear_save_hook(&state);
        return Err(e);
    }
    Ok(())
}

pub fn set_save_hotkey(hotkey: &str) -> Result<(), String> {
    if let Some(state) = current_save_hook() {
        state.set_hotkey(hotkey)?;
    }
    Ok(())
}

fn save_hook_slot() -> &'static Mutex<Option<Arc<HookState>>> {
    SAVE_HOOK.get_or_init(|| Mutex::new(None))
}

fn current_save_hook() -> Option<Arc<HookState>> {
    save_hook_slot().lock().ok().and_then(|state| state.clone())
}

fn publish_save_hook(state: Arc<HookState>) -> Result<(), String> {
    let mut guard = save_hook_slot()
        .lock()
        .map_err(|_| "save hotkey hook lock poisoned".to_string())?;
    if guard.is_some() {
        return Err("save hotkey hook was already installed".to_string());
    }
    *guard = Some(state);
    Ok(())
}

fn clear_save_hook(state: &Arc<HookState>) {
    let Ok(mut guard) = save_hook_slot().lock() else {
        return;
    };
    if guard
        .as_ref()
        .is_some_and(|current| Arc::ptr_eq(current, state))
    {
        *guard = None;
    }
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
            key => {
                key_vk = Some(
                    keyboard_key_vk_code(key)
                        .ok_or_else(|| format!("unsupported hotkey key: {key}"))?,
                );
            }
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

fn run_keyboard_hook(ready_tx: Sender<Result<u32, String>>) {
    let keyboard_hook =
        unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_proc), std::ptr::null_mut(), 0) };
    if keyboard_hook.is_null() {
        let _ = ready_tx.send(Err(
            "low-level save hotkey hook could not be installed".into()
        ));
        return;
    }

    let _ = ready_tx.send(Ok(unsafe { GetCurrentThreadId() }));
    let mut msg = unsafe { std::mem::zeroed::<MSG>() };
    while unsafe { GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) } > 0 {
        unsafe {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    unsafe {
        UnhookWindowsHookEx(keyboard_hook);
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
                if is_supported_keyboard_hook_vk(keyboard.vkCode) {
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
    if let Some(state) = current_save_hook() {
        state.on_key_down(vk_code, ctrl, alt, shift);
    }
}

fn release_mouse_hotkey(vk_code: u32) {
    release_key(vk_code);
}

fn release_key(vk_code: u32) {
    if let Some(state) = current_save_hook() {
        state.on_key_up(vk_code);
    }
}

fn is_supported_keyboard_hook_vk(vk_code: u32) -> bool {
    (VK_F1_CODE..=VK_F24_CODE).contains(&vk_code)
        || (0x30..=0x39).contains(&vk_code)
        || (0x41..=0x5A).contains(&vk_code)
        || matches!(
            vk_code,
            0x08 | 0x09
                | 0x0D
                | 0x20
                | 0x21
                | 0x22
                | 0x23
                | 0x24
                | 0x25
                | 0x26
                | 0x27
                | 0x28
                | 0x2D
                | 0x2E
                | 0xBA
                | 0xBB
                | 0xBC
                | 0xBD
                | 0xBE
                | 0xBF
                | 0xC0
                | 0xDB
                | 0xDC
                | 0xDD
                | 0xDE
        )
}

fn keyboard_key_vk_code(key: &str) -> Option<u32> {
    if key.len() == 1 {
        let c = key.as_bytes()[0];
        if c.is_ascii_uppercase() || c.is_ascii_digit() {
            return Some(c as u32);
        }
    }

    match key {
        "Backspace" => Some(0x08),
        "Tab" => Some(0x09),
        "Enter" => Some(0x0D),
        "Space" => Some(0x20),
        "PageUp" => Some(0x21),
        "PageDown" => Some(0x22),
        "End" => Some(0x23),
        "Home" => Some(0x24),
        "ArrowLeft" => Some(0x25),
        "ArrowUp" => Some(0x26),
        "ArrowRight" => Some(0x27),
        "ArrowDown" => Some(0x28),
        "Insert" => Some(0x2D),
        "Delete" => Some(0x2E),
        "Semicolon" => Some(0xBA),
        "Equal" => Some(0xBB),
        "Comma" => Some(0xBC),
        "Minus" => Some(0xBD),
        "Period" => Some(0xBE),
        "Slash" => Some(0xBF),
        "Backquote" => Some(0xC0),
        "BracketLeft" => Some(0xDB),
        "Backslash" => Some(0xDC),
        "BracketRight" => Some(0xDD),
        "Quote" => Some(0xDE),
        _ => None,
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
    fn parses_modified_keyboard_hotkeys_for_hook_matching() {
        let letter = parse_hook_hotkey("Ctrl+G").unwrap();
        assert!(letter.matches(0x47, true, false, false));
        assert!(!letter.matches(0x47, false, false, false));

        let arrow = parse_hook_hotkey("Alt+Shift+ArrowLeft").unwrap();
        assert!(arrow.matches(0x25, false, true, true));
        assert!(!arrow.matches(0x25, false, true, false));

        let slash = parse_hook_hotkey("Ctrl+Slash").unwrap();
        assert!(slash.matches(0xBF, true, false, false));
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
    fn keyboard_hook_filter_includes_supported_modified_keys() {
        assert!(is_supported_keyboard_hook_vk(0x47));
        assert!(is_supported_keyboard_hook_vk(0x25));
        assert!(is_supported_keyboard_hook_vk(0xBF));
        assert!(is_supported_keyboard_hook_vk(VK_F1_CODE));
        assert!(!is_supported_keyboard_hook_vk(0x1B));
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
