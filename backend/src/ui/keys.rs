use eframe::egui::{Key, Modifiers};
use serde_json::Value;

pub fn vk(key: Key) -> Option<u64> {
    use Key::*;
    Some(match key {
        ArrowLeft => 0x25,
        ArrowUp => 0x26,
        ArrowRight => 0x27,
        ArrowDown => 0x28,
        Escape => 0x1b,
        Tab => 9,
        Backspace => 8,
        Enter => 13,
        Space => 32,
        Insert => 0x2d,
        Delete => 0x2e,
        Home => 0x24,
        End => 0x23,
        PageUp => 0x21,
        PageDown => 0x22,
        A => 65,
        B => 66,
        C => 67,
        D => 68,
        E => 69,
        F => 70,
        G => 71,
        H => 72,
        I => 73,
        J => 74,
        K => 75,
        L => 76,
        M => 77,
        N => 78,
        O => 79,
        P => 80,
        Q => 81,
        R => 82,
        S => 83,
        T => 84,
        U => 85,
        V => 86,
        W => 87,
        X => 88,
        Y => 89,
        Z => 90,
        Num0 => 48,
        Num1 => 49,
        Num2 => 50,
        Num3 => 51,
        Num4 => 52,
        Num5 => 53,
        Num6 => 54,
        Num7 => 55,
        Num8 => 56,
        Num9 => 57,
        F1 => 0x70,
        F2 => 0x71,
        F3 => 0x72,
        F4 => 0x73,
        F5 => 0x74,
        F6 => 0x75,
        F7 => 0x76,
        F8 => 0x77,
        F9 => 0x78,
        F10 => 0x79,
        F11 => 0x7a,
        F12 => 0x7b,
        F13 => 0x7c,
        F14 => 0x7d,
        F15 => 0x7e,
        F16 => 0x7f,
        F17 => 0x80,
        F18 => 0x81,
        F19 => 0x82,
        F20 => 0x83,
        F21 => 0x84,
        F22 => 0x85,
        F23 => 0x86,
        F24 => 0x87,
        Backtick => 0xc0,
        Minus => 0xbd,
        Equals | Plus => 0xbb,
        OpenBracket => 0xdb,
        CloseBracket => 0xdd,
        Backslash | Pipe => 0xdc,
        Semicolon | Colon => 0xba,
        Quote => 0xde,
        Comma => 0xbc,
        Period => 0xbe,
        Slash | Questionmark => 0xbf,
        _ => return None,
    })
}
pub fn modifiers(m: Modifiers, platform: &str) -> u64 {
    u64::from(m.ctrl)
        | (u64::from(m.alt) << 1)
        | (u64::from(m.shift) << 2)
        | (u64::from(platform == "mac" && m.mac_cmd) << 3)
}
fn trigger_vk(value: &Value) -> Option<u64> {
    if value == "capslock" {
        Some(20)
    } else {
        value["vk"].as_u64()
    }
}
pub fn conflicts(key: &str, vk: u64, settings: &Value) -> bool {
    ["trigger", "emoteTrigger", "toggleHotkey"]
        .iter()
        .any(|other| *other != key && trigger_vk(&settings[*other]) == Some(vk))
}
pub fn label(vk: u64) -> String {
    if (65..=90).contains(&vk) || (48..=57).contains(&vk) {
        return char::from_u32(vk as u32).unwrap().to_string();
    }
    if (0x70..=0x87).contains(&vk) {
        return format!("F{}", vk - 0x6f);
    }
    if (0x60..=0x69).contains(&vk) {
        return format!("Num {}", vk - 0x60);
    }
    match vk {
        8 => "Backspace",
        9 => "Tab",
        13 => "Enter",
        20 => "Caps Lock",
        27 => "Esc",
        32 => "Space",
        0x21 => "Page Up",
        0x22 => "Page Down",
        0x23 => "End",
        0x24 => "Home",
        0x25 => "Left",
        0x26 => "Up",
        0x27 => "Right",
        0x28 => "Down",
        0x2d => "Insert",
        0x2e => "Delete",
        0xc0 => "`",
        0xbd => "-",
        0xbb => "=",
        0xdb => "[",
        0xdd => "]",
        0xdc => "\\",
        0xba => ";",
        0xde => "'",
        0xbc => ",",
        0xbe => ".",
        0xbf => "/",
        _ => return format!("Key 0x{vk:02X}"),
    }
    .into()
}
pub fn shortcut(value: &Value, platform: &str) -> String {
    let mods = value["mods"].as_u64().unwrap_or(3);
    let mut parts = Vec::new();
    for (bit, name) in [
        (1, "Ctrl"),
        (2, if platform == "mac" { "Option" } else { "Alt" }),
        (4, "Shift"),
        (
            8,
            if platform == "mac" {
                "Command"
            } else if platform == "linux" {
                "Super"
            } else {
                "Win"
            },
        ),
    ] {
        if mods & bit != 0 {
            parts.push(name.into());
        }
    }
    parts.push(label(value["vk"].as_u64().unwrap_or(80)));
    parts.join(" + ")
}
pub fn trigger_label(value: &Value, platform: &str, zh: bool) -> String {
    if let Some(vk) = value["vk"].as_u64() {
        return label(vk);
    }
    match value.as_str().unwrap_or("alt") {
        "alt" => {
            if platform == "mac" {
                "Option"
            } else {
                "Alt"
            }
        }
        "ctrl" => "Ctrl",
        "shift" => "Shift",
        "win" => {
            if platform == "mac" {
                "Command"
            } else if platform == "linux" {
                "Super"
            } else {
                "Win"
            }
        }
        "capslock" => "Caps Lock",
        "mouse4" => {
            if zh {
                "鼠标侧键 4"
            } else {
                "Mouse 4"
            }
        }
        "mouse5" => {
            if zh {
                "鼠标侧键 5"
            } else {
                "Mouse 5"
            }
        }
        "off" => {
            if zh {
                "关闭"
            } else {
                "Off"
            }
        }
        _ => "Alt",
    }
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn physical_keys_use_helper_virtual_codes() {
        assert_eq!(vk(Key::P), Some(80));
        assert_eq!(vk(Key::F12), Some(123));
        assert_eq!(vk(Key::OpenBracket), Some(219));
        assert_eq!(vk(Key::F25), None);
    }
    #[test]
    fn modifier_and_conflict_rules_preserve_helper_contract() {
        let m = Modifiers {
            ctrl: true,
            alt: true,
            shift: false,
            mac_cmd: true,
            command: true,
        };
        assert_eq!(modifiers(m, "win"), 3);
        assert_eq!(modifiers(m, "mac"), 11);
        let s =
            json!({"trigger":{"vk":65},"emoteTrigger":"ctrl","toggleHotkey":{"mods":3,"vk":80}});
        assert!(conflicts("trigger", 80, &s));
        assert!(!conflicts("trigger", 65, &s));
        assert!(conflicts("toggleHotkey", 65, &s));
    }
}
