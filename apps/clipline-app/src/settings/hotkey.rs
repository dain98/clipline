//! Hotkey parsing: Ctrl/Alt/Shift + F1-F11/F13-F24 or selected mouse buttons.
//! F12 is reserved by Windows for debuggers.

use tauri_plugin_global_shortcut::Shortcut;

pub fn parse_hotkey(raw: &str) -> Result<Shortcut, String> {
    normalize_hotkey(raw)?
        .parse::<Shortcut>()
        .map_err(|e| format!("hotkey: {e}"))
}

pub fn is_global_shortcut_hotkey(raw: &str) -> Result<bool, String> {
    Ok(matches!(
        parse_hotkey_key(&normalize_hotkey(raw)?)?,
        HotkeyKey::Function(_)
    ))
}

pub fn normalize_hotkey(raw: &str) -> Result<String, String> {
    let mut ctrl = false;
    let mut alt = false;
    let mut shift = false;
    let mut key = None::<HotkeyKey>;

    for part in raw.split('+') {
        let token = part.trim();
        if token.is_empty() {
            return Err("hotkey has an empty part".into());
        }
        match token.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => set_once(&mut ctrl, "Ctrl")?,
            "alt" => set_once(&mut alt, "Alt")?,
            "shift" => set_once(&mut shift, "Shift")?,
            other if other.starts_with('f') && other[1..].chars().all(|c| c.is_ascii_digit()) => {
                if key.is_some() {
                    return Err("hotkey has more than one key".into());
                }
                let n = other[1..]
                    .parse::<u8>()
                    .map_err(|_| "hotkey key must be F1-F11 or F13-F24")?;
                if !(1..=24).contains(&n) {
                    return Err("hotkey key must be F1-F11 or F13-F24".into());
                }
                if n == 12 {
                    return Err("F12 is reserved by Windows for debuggers".into());
                }
                key = Some(HotkeyKey::Function(n));
            }
            other => {
                if key.is_some() {
                    return Err("hotkey has more than one key".into());
                }
                if let Some(mouse) = mouse_key_from_token(other) {
                    key = Some(mouse);
                } else {
                    return Err(
                        "hotkey must use optional Ctrl, Alt, Shift, and F1-F11, F13-F24, Middle, Mouse4, or Mouse5".into(),
                    );
                }
            }
        }
    }

    let key = key.ok_or("hotkey needs a key")?;

    let mut parts = Vec::new();
    if ctrl {
        parts.push("Ctrl".to_string());
    }
    if alt {
        parts.push("Alt".to_string());
    }
    if shift {
        parts.push("Shift".to_string());
    }
    parts.push(key.label().to_string());
    Ok(parts.join("+"))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HotkeyKey {
    Function(u8),
    Middle,
    Mouse4,
    Mouse5,
}

impl HotkeyKey {
    fn label(self) -> &'static str {
        match self {
            Self::Function(1) => "F1",
            Self::Function(2) => "F2",
            Self::Function(3) => "F3",
            Self::Function(4) => "F4",
            Self::Function(5) => "F5",
            Self::Function(6) => "F6",
            Self::Function(7) => "F7",
            Self::Function(8) => "F8",
            Self::Function(9) => "F9",
            Self::Function(10) => "F10",
            Self::Function(11) => "F11",
            Self::Function(13) => "F13",
            Self::Function(14) => "F14",
            Self::Function(15) => "F15",
            Self::Function(16) => "F16",
            Self::Function(17) => "F17",
            Self::Function(18) => "F18",
            Self::Function(19) => "F19",
            Self::Function(20) => "F20",
            Self::Function(21) => "F21",
            Self::Function(22) => "F22",
            Self::Function(23) => "F23",
            Self::Function(24) => "F24",
            Self::Function(_) => "F?",
            Self::Middle => "Middle",
            Self::Mouse4 => "Mouse4",
            Self::Mouse5 => "Mouse5",
        }
    }
}

fn parse_hotkey_key(normalized: &str) -> Result<HotkeyKey, String> {
    normalized
        .split('+')
        .find_map(|part| match part {
            "Ctrl" | "Alt" | "Shift" => None,
            key => Some(key),
        })
        .and_then(mouse_key_from_token)
        .or_else(|| {
            normalized.split('+').find_map(|part| {
                part.strip_prefix('F')
                    .and_then(|number| number.parse::<u8>().ok())
                    .map(HotkeyKey::Function)
            })
        })
        .ok_or_else(|| "hotkey needs a key".to_string())
}

fn mouse_key_from_token(token: &str) -> Option<HotkeyKey> {
    match token.to_ascii_lowercase().as_str() {
        "middle" | "mouse3" | "mbutton" | "middlemouse" | "mousemiddle" => Some(HotkeyKey::Middle),
        "mouse4" | "xbutton1" | "x1" | "back" | "browserback" => Some(HotkeyKey::Mouse4),
        "mouse5" | "xbutton2" | "x2" | "forward" | "browserforward" => Some(HotkeyKey::Mouse5),
        _ => None,
    }
}

fn set_once(slot: &mut bool, name: &str) -> Result<(), String> {
    if *slot {
        return Err(format!("hotkey repeats {name}"));
    }
    *slot = true;
    Ok(())
}
