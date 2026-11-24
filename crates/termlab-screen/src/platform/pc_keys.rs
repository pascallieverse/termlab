//! Browser key codes (`KeyboardEvent.code`, the physical key whatever the layout) to PC keys:
//! Linux evdev codes (X11 adds 8; Wayland's remote desktop portal takes them as they are) and
//! Windows scan codes (set 1). The computer then applies its own keyboard layout, as for a
//! real keyboard.

/// One physical key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PcKey {
    /// Linux `KEY_*` (input-event-codes.h).
    pub evdev: u16,
    /// Windows scan code; `0xE0xx` for extended keys. 0: send `vk` instead.
    pub scan: u16,
    /// Windows virtual key, for the keys without a usable scan code (Pause, Num Lock).
    pub vk: u16,
}

const fn k(evdev: u16, scan: u16) -> PcKey {
    PcKey { evdev, scan, vk: 0 }
}

pub fn pc_key(code: &str) -> Option<PcKey> {
    Some(match code {
        "Escape" => k(1, 0x01),
        "Digit1" => k(2, 0x02),
        "Digit2" => k(3, 0x03),
        "Digit3" => k(4, 0x04),
        "Digit4" => k(5, 0x05),
        "Digit5" => k(6, 0x06),
        "Digit6" => k(7, 0x07),
        "Digit7" => k(8, 0x08),
        "Digit8" => k(9, 0x09),
        "Digit9" => k(10, 0x0A),
        "Digit0" => k(11, 0x0B),
        "Minus" => k(12, 0x0C),
        "Equal" => k(13, 0x0D),
        "Backspace" => k(14, 0x0E),
        "Tab" => k(15, 0x0F),
        "KeyQ" => k(16, 0x10),
        "KeyW" => k(17, 0x11),
        "KeyE" => k(18, 0x12),
        "KeyR" => k(19, 0x13),
        "KeyT" => k(20, 0x14),
        "KeyY" => k(21, 0x15),
        "KeyU" => k(22, 0x16),
        "KeyI" => k(23, 0x17),
        "KeyO" => k(24, 0x18),
        "KeyP" => k(25, 0x19),
        "BracketLeft" => k(26, 0x1A),
        "BracketRight" => k(27, 0x1B),
        "Enter" => k(28, 0x1C),
        "ControlLeft" => k(29, 0x1D),
        "KeyA" => k(30, 0x1E),
        "KeyS" => k(31, 0x1F),
        "KeyD" => k(32, 0x20),
        "KeyF" => k(33, 0x21),
        "KeyG" => k(34, 0x22),
        "KeyH" => k(35, 0x23),
        "KeyJ" => k(36, 0x24),
        "KeyK" => k(37, 0x25),
        "KeyL" => k(38, 0x26),
        "Semicolon" => k(39, 0x27),
        "Quote" => k(40, 0x28),
        "Backquote" => k(41, 0x29),
        "ShiftLeft" => k(42, 0x2A),
        "Backslash" => k(43, 0x2B),
        "KeyZ" => k(44, 0x2C),
        "KeyX" => k(45, 0x2D),
        "KeyC" => k(46, 0x2E),
        "KeyV" => k(47, 0x2F),
        "KeyB" => k(48, 0x30),
        "KeyN" => k(49, 0x31),
        "KeyM" => k(50, 0x32),
        "Comma" => k(51, 0x33),
        "Period" => k(52, 0x34),
        "Slash" => k(53, 0x35),
        "ShiftRight" => k(54, 0x36),
        "NumpadMultiply" => k(55, 0x37),
        "AltLeft" => k(56, 0x38),
        "Space" => k(57, 0x39),
        "CapsLock" => k(58, 0x3A),
        "F1" => k(59, 0x3B),
        "F2" => k(60, 0x3C),
        "F3" => k(61, 0x3D),
        "F4" => k(62, 0x3E),
        "F5" => k(63, 0x3F),
        "F6" => k(64, 0x40),
        "F7" => k(65, 0x41),
        "F8" => k(66, 0x42),
        "F9" => k(67, 0x43),
        "F10" => k(68, 0x44),
        "NumLock" => PcKey {
            evdev: 69,
            scan: 0,
            vk: 0x90,
        },
        "ScrollLock" => k(70, 0x46),
        "Numpad7" => k(71, 0x47),
        "Numpad8" => k(72, 0x48),
        "Numpad9" => k(73, 0x49),
        "NumpadSubtract" => k(74, 0x4A),
        "Numpad4" => k(75, 0x4B),
        "Numpad5" => k(76, 0x4C),
        "Numpad6" => k(77, 0x4D),
        "NumpadAdd" => k(78, 0x4E),
        "Numpad1" => k(79, 0x4F),
        "Numpad2" => k(80, 0x50),
        "Numpad3" => k(81, 0x51),
        "Numpad0" => k(82, 0x52),
        "NumpadDecimal" => k(83, 0x53),
        "IntlBackslash" => k(86, 0x56),
        "F11" => k(87, 0x57),
        "F12" => k(88, 0x58),
        "IntlRo" => k(89, 0x73),
        "Convert" => k(92, 0x79),
        "KanaMode" => k(93, 0x70),
        "NonConvert" => k(94, 0x7B),
        "NumpadEnter" => k(96, 0xE01C),
        "ControlRight" => k(97, 0xE01D),
        "NumpadDivide" => k(98, 0xE035),
        "PrintScreen" => k(99, 0xE037),
        "AltRight" => k(100, 0xE038),
        "Home" => k(102, 0xE047),
        "ArrowUp" => k(103, 0xE048),
        "PageUp" => k(104, 0xE049),
        "ArrowLeft" => k(105, 0xE04B),
        "ArrowRight" => k(106, 0xE04D),
        "End" => k(107, 0xE04F),
        "ArrowDown" => k(108, 0xE050),
        "PageDown" => k(109, 0xE051),
        "Insert" => k(110, 0xE052),
        "Delete" => k(111, 0xE053),
        "AudioVolumeMute" => k(113, 0xE020),
        "AudioVolumeDown" => k(114, 0xE02E),
        "AudioVolumeUp" => k(115, 0xE030),
        "NumpadEqual" => k(117, 0x59),
        "Pause" => PcKey {
            evdev: 119,
            scan: 0,
            vk: 0x13,
        },
        "NumpadComma" => k(121, 0x7E),
        "IntlYen" => k(124, 0x7D),
        "MetaLeft" | "OSLeft" => k(125, 0xE05B),
        "MetaRight" | "OSRight" => k(126, 0xE05C),
        "ContextMenu" => k(127, 0xE05D),
        "MediaTrackNext" => k(163, 0xE019),
        "MediaPlayPause" => k(164, 0xE022),
        "MediaTrackPrevious" => k(165, 0xE010),
        "MediaStop" => k(166, 0xE024),
        "F13" => k(183, 0x64),
        "F14" => k(184, 0x65),
        "F15" => k(185, 0x66),
        "F16" => k(186, 0x67),
        "F17" => k(187, 0x68),
        "F18" => k(188, 0x69),
        "F19" => k(189, 0x6A),
        "F20" => k(190, 0x6B),
        "F21" => k(191, 0x6C),
        "F22" => k(192, 0x6D),
        "F23" => k(193, 0x6E),
        "F24" => k(194, 0x76),
        _ => return None,
    })
}

/// The X11 keysym that types `c` (for text that has no key on the layout).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn keysym(c: char) -> u32 {
    match c {
        '\n' | '\r' => 0xff0d,
        '\t' => 0xff09,
        '\u{8}' => 0xff08,
        c if (' '..='~').contains(&c) || ('\u{a0}'..='\u{ff}').contains(&c) => c as u32,
        c => 0x0100_0000 | c as u32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn main_block_matches_between_evdev_and_scan_codes() {
        // evdev numbers the main block like PC scan codes.
        for code in [
            "KeyA",
            "Digit1",
            "Space",
            "Enter",
            "F1",
            "ShiftLeft",
            "IntlBackslash",
        ] {
            let key = pc_key(code).unwrap();
            assert_eq!(key.evdev, key.scan, "{code}");
        }
        assert_eq!(pc_key("ArrowUp").unwrap().scan, 0xE048);
        assert_eq!(pc_key("Pause").unwrap().vk, 0x13);
        assert_eq!(pc_key("Nope"), None);
    }

    #[test]
    fn keysyms() {
        assert_eq!(keysym('a'), 0x61);
        assert_eq!(keysym('é'), 0xe9);
        assert_eq!(keysym('€'), 0x0100_20ac);
        assert_eq!(keysym('\n'), 0xff0d);
    }
}
