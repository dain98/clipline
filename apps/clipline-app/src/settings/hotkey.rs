//! Hotkey parsing: Ctrl/Alt/Shift + F1-F11/F13-F24. F12 is reserved by
//! Windows for debuggers.

use tauri_plugin_global_shortcut::Shortcut;

pub fn parse_hotkey(raw: &str) -> Result<Shortcut, String> {
    normalize_hotkey(raw)?
        .parse::<Shortcut>()
        .map_err(|e| format!("hotkey: {e}"))
}

pub fn normalize_hotkey(raw: &str) -> Result<String, String> {
    let mut ctrl = false;
    let mut alt = false;
    let mut shift = false;
    let mut key = None::<u8>;

    for part in raw.split('+') {
        let token = part.trim();
        if token.is_empty() {
            return Err("hotkey has an empty part".into());
        }
        match token.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => set_once(&mut ctrl, "Ctrl")?,
            "alt" => set_once(&mut alt, "Alt")?,
            "shift" => set_once(&mut shift, "Shift")?,
            other if other.starts_with('f') => {
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
                key = Some(n);
            }
            _ => {
                return Err(
                    "hotkey must use optional Ctrl, Alt, Shift, and F1-F11 or F13-F24".into(),
                )
            }
        }
    }

    let key = key.ok_or("hotkey needs an F-key")?;

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
    parts.push(format!("F{key}"));
    Ok(parts.join("+"))
}

fn set_once(slot: &mut bool, name: &str) -> Result<(), String> {
    if *slot {
        return Err(format!("hotkey repeats {name}"));
    }
    *slot = true;
    Ok(())
}