//! Browser key codes (`KeyboardEvent.code`, which names the physical key whatever the
//! layout) to macOS virtual key codes (`kVK_*` in Carbon's Events.h). The Mac then applies
//! its own keyboard layout, as for a real keyboard.

pub fn keycode(code: &str) -> Option<u16> {
    Some(match code {
        "KeyA" => 0,
        "KeyS" => 1,
        "KeyD" => 2,
        "KeyF" => 3,
        "KeyH" => 4,
        "KeyG" => 5,
        "KeyZ" => 6,
        "KeyX" => 7,
        "KeyC" => 8,
        "KeyV" => 9,
        "IntlBackslash" => 10,
        "KeyB" => 11,
        "KeyQ" => 12,
        "KeyW" => 13,
        "KeyE" => 14,
        "KeyR" => 15,
        "KeyY" => 16,
        "KeyT" => 17,
        "Digit1" => 18,
        "Digit2" => 19,
        "Digit3" => 20,
        "Digit4" => 21,
        "Digit6" => 22,
        "Digit5" => 23,
        "Equal" => 24,
        "Digit9" => 25,
        "Digit7" => 26,
        "Minus" => 27,
        "Digit8" => 28,
        "Digit0" => 29,
        "BracketRight" => 30,
        "KeyO" => 31,
        "KeyU" => 32,
        "BracketLeft" => 33,
        "KeyI" => 34,
        "KeyP" => 35,
        "Enter" => 36,
        "KeyL" => 37,
        "KeyJ" => 38,
        "Quote" => 39,
        "KeyK" => 40,
        "Semicolon" => 41,
        "Backslash" => 42,
        "Comma" => 43,
        "Slash" => 44,
        "KeyN" => 45,
        "KeyM" => 46,
        "Period" => 47,
        "Tab" => 48,
        "Space" => 49,
        "Backquote" => 50,
        "Backspace" => 51,
        "Escape" => 53,
        "MetaRight" | "OSRight" => 54,
        "MetaLeft" | "OSLeft" => 55,
        "ShiftLeft" => 56,
        "CapsLock" => 57,
        "AltLeft" => 58,
        "ControlLeft" => 59,
        "ShiftRight" => 60,
        "AltRight" => 61,
        "ControlRight" => 62,
        "Fn" => 63,
        "F17" => 64,
        "NumpadDecimal" => 65,
        "NumpadMultiply" => 67,
        "NumpadAdd" => 69,
        "NumLock" | "NumpadClear" => 71,
        "AudioVolumeUp" | "VolumeUp" => 72,
        "AudioVolumeDown" | "VolumeDown" => 73,
        "AudioVolumeMute" | "VolumeMute" => 74,
        "NumpadDivide" => 75,
        "NumpadEnter" => 76,
        "NumpadSubtract" => 78,
        "F18" => 79,
        "F19" => 80,
        "NumpadEqual" => 81,
        "Numpad0" => 82,
        "Numpad1" => 83,
        "Numpad2" => 84,
        "Numpad3" => 85,
        "Numpad4" => 86,
        "Numpad5" => 87,
        "Numpad6" => 88,
        "Numpad7" => 89,
        "F20" => 90,
        "Numpad8" => 91,
        "Numpad9" => 92,
        "IntlYen" => 93,
        "IntlRo" => 94,
        "NumpadComma" => 95,
        "F5" => 96,
        "F6" => 97,
        "F7" => 98,
        "F3" => 99,
        "F8" => 100,
        "F9" => 101,
        "Lang2" => 102,
        "F11" => 103,
        "Lang1" => 104,
        "F13" => 105,
        "F16" => 106,
        "F14" => 107,
        "F10" => 109,
        "ContextMenu" => 110,
        "F12" => 111,
        "F15" => 113,
        "Help" | "Insert" => 114,
        "Home" => 115,
        "PageUp" => 116,
        "Delete" => 117,
        "F4" => 118,
        "End" => 119,
        "F2" => 120,
        "PageDown" => 121,
        "F1" => 122,
        "ArrowLeft" => 123,
        "ArrowRight" => 124,
        "ArrowDown" => 125,
        "ArrowUp" => 126,
        _ => return None,
    })
}

/// The modifier flag a key holds down (`CGEventFlags` bits), if it is a modifier.
pub fn modifier_flag(keycode: u16) -> Option<u64> {
    Some(match keycode {
        54 | 55 => 0x0010_0000, // Command
        56 | 60 => 0x0002_0000, // Shift
        58 | 61 => 0x0008_0000, // Option
        59 | 62 => 0x0004_0000, // Control
        63 => 0x0080_0000,      // Fn
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letters_follow_the_physical_layout() {
        assert_eq!(keycode("KeyA"), Some(0));
        assert_eq!(keycode("KeyQ"), Some(12));
        assert_eq!(keycode("Enter"), Some(36));
        assert_eq!(keycode("ArrowUp"), Some(126));
        assert_eq!(keycode("Nonsense"), None);
    }

    #[test]
    fn modifiers_have_flags() {
        assert_eq!(modifier_flag(55), Some(0x0010_0000));
        assert_eq!(modifier_flag(0), None);
    }
}
