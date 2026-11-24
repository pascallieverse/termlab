//! Mouse and keyboard through `SendInput`. Keys go as scan codes, so Windows applies its own
//! keyboard layout; text that has no key (phone keyboards, paste) goes as Unicode.

use std::collections::HashSet;

use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBD_EVENT_FLAGS, KEYBDINPUT,
    KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, KEYEVENTF_SCANCODE, KEYEVENTF_UNICODE,
    MOUSE_EVENT_FLAGS, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_HWHEEL, MOUSEEVENTF_LEFTDOWN,
    MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_MOVE,
    MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_VIRTUALDESK, MOUSEEVENTF_WHEEL,
    MOUSEINPUT, SendInput, VIRTUAL_KEY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
};

use super::super::pc_keys::{PcKey, pc_key};
use crate::proto::InputEvent;

/// Browser pixels per wheel notch (120 units): Chrome and Edge scroll 100 px a notch.
const PIXELS_PER_NOTCH: f64 = 100.0;

pub struct Input {
    keys: HashSet<String>,
    buttons: [bool; 3],
    last: (f64, f64),
    wheel: (f64, f64),
}

impl Default for Input {
    fn default() -> Self {
        Self::new()
    }
}

fn send(inputs: &[INPUT]) {
    // SAFETY: well-formed INPUT structs of the right size.
    let sent = unsafe { SendInput(inputs, std::mem::size_of::<INPUT>() as i32) };
    if (sent as usize) < inputs.len() {
        tracing::debug!(
            sent,
            "SendInput blocked (an administrator window, or the secure desktop)"
        );
    }
}

fn mouse(dx: i32, dy: i32, data: i32, flags: MOUSE_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx,
                dy,
                mouseData: data as u32,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn key(vk: u16, scan: u16, flags: KEYBD_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(vk),
                wScan: scan,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn key_input(k: PcKey, down: bool) -> INPUT {
    let up = if down {
        KEYBD_EVENT_FLAGS(0)
    } else {
        KEYEVENTF_KEYUP
    };
    if k.scan == 0 {
        return key(k.vk, 0, up);
    }
    let mut flags = KEYEVENTF_SCANCODE | up;
    if k.scan & 0xff00 == 0xe000 {
        flags |= KEYEVENTF_EXTENDEDKEY;
    }
    key(0, k.scan & 0xff, flags)
}

/// A point on the desktop (physical pixels) as `SendInput`'s 0..65535 over all monitors.
fn absolute(x: f64, y: f64) -> (i32, i32) {
    // SAFETY: plain metric queries.
    let (vx, vy, vw, vh) = unsafe {
        (
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
            GetSystemMetrics(SM_CXVIRTUALSCREEN).max(2),
            GetSystemMetrics(SM_CYVIRTUALSCREEN).max(2),
        )
    };
    let nx = ((x - f64::from(vx)) * 65535.0 / f64::from(vw - 1)).round();
    let ny = ((y - f64::from(vy)) * 65535.0 / f64::from(vh - 1)).round();
    (nx.clamp(0.0, 65535.0) as i32, ny.clamp(0.0, 65535.0) as i32)
}

const MOVE: MOUSE_EVENT_FLAGS =
    MOUSE_EVENT_FLAGS(MOUSEEVENTF_MOVE.0 | MOUSEEVENTF_ABSOLUTE.0 | MOUSEEVENTF_VIRTUALDESK.0);

impl Input {
    pub fn new() -> Self {
        super::dpi_aware();
        Input {
            keys: HashSet::new(),
            buttons: [false; 3],
            last: (0.0, 0.0),
            wheel: (0.0, 0.0),
        }
    }

    pub fn apply(&mut self, ev: InputEvent) {
        match ev {
            InputEvent::Move { x, y } => {
                self.last = (x, y);
                let (ax, ay) = absolute(x, y);
                send(&[mouse(ax, ay, 0, MOVE)]);
            }
            InputEvent::Button {
                x, y, button, down, ..
            } => {
                // Windows counts double clicks itself, from the timing.
                self.last = (x, y);
                let (ax, ay) = absolute(x, y);
                let Some(flag) = (match (button, down) {
                    (0, true) => Some(MOUSEEVENTF_LEFTDOWN),
                    (0, false) => Some(MOUSEEVENTF_LEFTUP),
                    (1, true) => Some(MOUSEEVENTF_RIGHTDOWN),
                    (1, false) => Some(MOUSEEVENTF_RIGHTUP),
                    (2, true) => Some(MOUSEEVENTF_MIDDLEDOWN),
                    (2, false) => Some(MOUSEEVENTF_MIDDLEUP),
                    _ => None,
                }) else {
                    return;
                };
                if let Some(b) = self.buttons.get_mut(button) {
                    *b = down;
                }
                send(&[mouse(ax, ay, 0, MOUSE_EVENT_FLAGS(MOVE.0 | flag.0))]);
            }
            InputEvent::Wheel { dx, dy } => {
                // Whole wheel units (120 a notch); the rest waits for the next event.
                self.wheel.0 += dx * 120.0 / PIXELS_PER_NOTCH;
                self.wheel.1 += dy * 120.0 / PIXELS_PER_NOTCH;
                let (h, v) = (self.wheel.0.trunc(), self.wheel.1.trunc());
                self.wheel.0 -= h;
                self.wheel.1 -= v;
                let mut events = Vec::new();
                if v != 0.0 {
                    // Positive is away from the user (up); the browser's positive y is down.
                    events.push(mouse(0, 0, -v as i32, MOUSEEVENTF_WHEEL));
                }
                if h != 0.0 {
                    events.push(mouse(0, 0, h as i32, MOUSEEVENTF_HWHEEL));
                }
                if !events.is_empty() {
                    send(&events);
                }
            }
            InputEvent::Key { code, down, .. } => {
                let Some(k) = pc_key(&code) else { return };
                if down {
                    self.keys.insert(code);
                } else {
                    self.keys.remove(&code);
                }
                send(&[key_input(k, down)]);
            }
            InputEvent::Text { text } => {
                let mut events = Vec::with_capacity(text.len() * 2);
                for unit in text.encode_utf16() {
                    if unit == u16::from(b'\n') {
                        let enter = pc_key("Enter").expect("Enter is mapped");
                        events.push(key_input(enter, true));
                        events.push(key_input(enter, false));
                        continue;
                    }
                    events.push(key(0, unit, KEYEVENTF_UNICODE));
                    events.push(key(0, unit, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP));
                }
                for chunk in events.chunks(64) {
                    send(chunk);
                }
            }
            InputEvent::ReleaseAll => self.release_all(),
        }
    }

    pub fn release_all(&mut self) {
        let keys: Vec<String> = self.keys.drain().collect();
        for code in keys {
            if let Some(k) = pc_key(&code) {
                send(&[key_input(k, false)]);
            }
        }
        let (x, y) = self.last;
        for button in 0..3 {
            if self.buttons[button] {
                self.apply(InputEvent::Button {
                    x,
                    y,
                    button,
                    down: false,
                    clicks: 1,
                });
            }
        }
    }
}

impl Drop for Input {
    fn drop(&mut self) {
        self.release_all();
    }
}
