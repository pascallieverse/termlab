//! Mouse and keyboard input, posted as CGEvents at the HID level (as if from real devices).
//! Needs the Accessibility permission; without it macOS drops the events silently.

use std::collections::HashSet;

use objc2_core_foundation::{CFRetained, CGPoint};
use objc2_core_graphics::{
    CGEvent, CGEventField, CGEventFlags, CGEventTapLocation, CGEventType, CGMouseButton,
    CGScrollEventUnit,
};

use super::keymap::{keycode, modifier_flag};
use crate::proto::InputEvent;

/// Holds what the browser pressed, so it can all be let go (a closed tab mustn't leave
/// Command held down).
#[derive(Default)]
pub struct Input {
    buttons: [bool; 3],
    keys: HashSet<u16>,
    flags: u64,
    at: (f64, f64),
}

const BUTTONS: [CGMouseButton; 3] = [
    CGMouseButton::Left,
    CGMouseButton::Right,
    CGMouseButton::Center,
];

fn post(ev: Option<CFRetained<CGEvent>>) {
    if let Some(ev) = ev {
        CGEvent::post(CGEventTapLocation::HIDEventTap, Some(&ev));
    }
}

impl Input {
    pub fn new() -> Self {
        Self::default()
    }

    fn flags(&self) -> CGEventFlags {
        CGEventFlags::from_bits_retain(self.flags)
    }

    fn mouse(&self, kind: CGEventType, button: usize, clicks: u8) {
        let at = CGPoint {
            x: self.at.0,
            y: self.at.1,
        };
        let ev = CGEvent::new_mouse_event(None, kind, at, BUTTONS[button]);
        if let Some(ev) = &ev {
            CGEvent::set_flags(Some(ev), self.flags());
            if clicks > 0 {
                CGEvent::set_integer_value_field(
                    Some(ev),
                    CGEventField::MouseEventClickState,
                    i64::from(clicks),
                );
            }
        }
        post(ev);
    }

    pub fn apply(&mut self, ev: InputEvent) {
        match ev {
            InputEvent::Move { x, y } => {
                self.at = (x, y);
                let (kind, button) = if self.buttons[0] {
                    (CGEventType::LeftMouseDragged, 0)
                } else if self.buttons[1] {
                    (CGEventType::RightMouseDragged, 1)
                } else if self.buttons[2] {
                    (CGEventType::OtherMouseDragged, 2)
                } else {
                    (CGEventType::MouseMoved, 0)
                };
                self.mouse(kind, button, 0);
            }
            InputEvent::Button {
                x,
                y,
                button,
                down,
                clicks,
            } => {
                if button > 2 {
                    return;
                }
                self.at = (x, y);
                self.buttons[button] = down;
                let kind = match (button, down) {
                    (0, true) => CGEventType::LeftMouseDown,
                    (0, false) => CGEventType::LeftMouseUp,
                    (1, true) => CGEventType::RightMouseDown,
                    (1, false) => CGEventType::RightMouseUp,
                    (_, true) => CGEventType::OtherMouseDown,
                    (_, false) => CGEventType::OtherMouseUp,
                };
                self.mouse(kind, button, clicks.max(1));
            }
            InputEvent::Wheel { dx, dy } => {
                // The browser's positive y scrolls down; the Mac's wheel turns the other way.
                let ev = CGEvent::new_scroll_wheel_event2(
                    None,
                    CGScrollEventUnit::Pixel,
                    2,
                    (-dy).round().clamp(-10_000.0, 10_000.0) as i32,
                    (-dx).round().clamp(-10_000.0, 10_000.0) as i32,
                    0,
                );
                if let Some(ev) = &ev {
                    CGEvent::set_flags(Some(ev), self.flags());
                }
                post(ev);
            }
            InputEvent::Key { code, down, repeat } => {
                let Some(key) = keycode(&code) else { return };
                if let Some(flag) = modifier_flag(key) {
                    if down {
                        self.flags |= flag;
                    } else {
                        self.flags &= !flag;
                    }
                }
                if down {
                    self.keys.insert(key);
                } else {
                    self.keys.remove(&key);
                }
                self.key(key, down, repeat);
            }
            InputEvent::Text { text } => self.text(&text),
            InputEvent::ReleaseAll => self.release_all(),
        }
    }

    fn key(&self, key: u16, down: bool, repeat: bool) {
        let ev = CGEvent::new_keyboard_event(None, key, down);
        if let Some(ev) = &ev {
            CGEvent::set_flags(Some(ev), self.flags());
            if repeat {
                CGEvent::set_integer_value_field(
                    Some(ev),
                    CGEventField::KeyboardEventAutorepeat,
                    1,
                );
            }
        }
        post(ev);
    }

    /// Types text as Unicode, whatever the Mac's keyboard layout.
    fn text(&self, text: &str) {
        let units: Vec<u16> = text.encode_utf16().collect();
        // CGEventKeyboardSetUnicodeString takes at most 20 UTF-16 units per event.
        for chunk in units.chunks(20) {
            for down in [true, false] {
                let ev = CGEvent::new_keyboard_event(None, 0, down);
                if let Some(ev) = &ev {
                    // SAFETY: `chunk` is a valid buffer of `chunk.len()` UTF-16 units.
                    #[allow(unsafe_code)]
                    unsafe {
                        CGEvent::keyboard_set_unicode_string(
                            Some(ev),
                            chunk.len() as _,
                            chunk.as_ptr(),
                        );
                    }
                }
                post(ev);
            }
        }
    }

    pub fn release_all(&mut self) {
        let keys: Vec<u16> = self.keys.drain().collect();
        for key in keys {
            if let Some(flag) = modifier_flag(key) {
                self.flags &= !flag;
            }
            self.key(key, false, false);
        }
        self.flags = 0;
        for b in 0..3 {
            if self.buttons[b] {
                self.buttons[b] = false;
                let kind = [
                    CGEventType::LeftMouseUp,
                    CGEventType::RightMouseUp,
                    CGEventType::OtherMouseUp,
                ][b];
                self.mouse(kind, b, 1);
            }
        }
    }
}

impl Drop for Input {
    fn drop(&mut self) {
        self.release_all();
    }
}
