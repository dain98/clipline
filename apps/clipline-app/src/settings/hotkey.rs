//! Hotkey parsing: optional Ctrl/Alt/Shift + F1-F11/F13-F24 or selected mouse buttons.
//! F12 is reserved by Windows for debuggers.

use tauri_plugin_global_shortcut::Shortcut;

pub fn parse_hotkey(raw: &str) -> Result<Shortcut, String> {
    normalize_hotkey(raw)?
        .parse::<Shortcut>()
        .map_err(|e| format!("hotkey: {e}"))
}

pub fn is_global_shortcut_hotkey(raw: &str) -> Result<bool, String> {
    Ok(matches!(
        normalize_hotkey_parts(raw)?.1,
        HotkeyKey::Function(_)
    ))
}

pub fn normalize_hotkey(raw: &str) -> Result<String, String> {
    normalize_hotkey_parts(raw).map(|(normalized, _)| normalized)
}

fn normalize_hotkey_parts(raw: &str) -> Result<(String, HotkeyKey), String> {
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
    parts.push(key.label());
    Ok((parts.join("+"), key))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HotkeyKey {
    Function(u8),
    Middle,
    Mouse4,
    Mouse5,
}

impl HotkeyKey {
    fn label(self) -> String {
        match self {
            Self::Function(n) => format!("F{n}"),
            Self::Middle => "Middle".to_string(),
            Self::Mouse4 => "Mouse4".to_string(),
            Self::Mouse5 => "Mouse5".to_string(),
        }
    }

}

fn mouse_key_from_token(token: &str) -> Option<HotkeyKey> {
    match token.to_ascii_lowercase().as_str() {
        "middle" => Some(HotkeyKey::Middle),
        "mouse4" => Some(HotkeyKey::Mouse4),
        "mouse5" => Some(HotkeyKey::Mouse5),
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
