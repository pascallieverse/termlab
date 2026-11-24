//! X11: monitors from RandR, the picture through MIT-SHM (plain `GetImage` when the server
//! can't share memory), the pointer drawn in from XFixes, input through XTEST.

use std::collections::HashSet;
use std::sync::Arc;

use anyhow::{Context, bail};
use x11rb::connection::Connection;
use x11rb::protocol::randr::ConnectionExt as _;
use x11rb::protocol::shm::ConnectionExt as _;
use x11rb::protocol::xfixes::ConnectionExt as _;
use x11rb::protocol::xproto::{
    BUTTON_PRESS_EVENT, BUTTON_RELEASE_EVENT, ConnectionExt as _, ImageFormat, ImageOrder,
    KEY_PRESS_EVENT, KEY_RELEASE_EVENT, MOTION_NOTIFY_EVENT, Window,
};
use x11rb::protocol::xtest::ConnectionExt as _;
use x11rb::rust_connection::RustConnection;

use super::super::Bounds;
use super::super::cursor::{Cursor, Shape, Shapes};
use super::super::pc_keys::{keysym, pc_key};
use super::super::soft::{Grab, Image, Opened, Pointer};
use crate::proto::{DisplayInfo, InputEvent};

fn connect() -> anyhow::Result<(RustConnection, Window)> {
    let (conn, screen) = x11rb::connect(None).context("Can't reach the X display")?;
    let root = conn.setup().roots[screen].root;
    Ok((conn, root))
}

struct Monitor {
    name: String,
    primary: bool,
    x: i16,
    y: i16,
    w: u16,
    h: u16,
}

fn monitors(conn: &RustConnection, root: Window) -> Vec<Monitor> {
    let listed = conn
        .randr_get_monitors(root, true)
        .ok()
        .and_then(|c| c.reply().ok())
        .map(|r| r.monitors)
        .unwrap_or_default();
    let mut out: Vec<Monitor> = listed
        .into_iter()
        .map(|m| Monitor {
            name: conn
                .get_atom_name(m.name)
                .ok()
                .and_then(|c| c.reply().ok())
                .map(|r| String::from_utf8_lossy(&r.name).into_owned())
                .unwrap_or_default(),
            primary: m.primary,
            x: m.x,
            y: m.y,
            w: m.width,
            h: m.height,
        })
        .filter(|m| m.w > 0 && m.h > 0)
        .collect();
    if out.is_empty() {
        let s = &conn.setup().roots[0];
        out.push(Monitor {
            name: String::new(),
            primary: true,
            x: 0,
            y: 0,
            w: s.width_in_pixels,
            h: s.height_in_pixels,
        });
    }
    out
}

pub fn displays() -> anyhow::Result<Vec<DisplayInfo>> {
    let (conn, root) = connect()?;
    Ok(monitors(&conn, root)
        .into_iter()
        .enumerate()
        .map(|(i, m)| DisplayInfo {
            id: i as u32 + 1,
            name: if m.name.is_empty() {
                format!("Display {}", i + 1)
            } else {
                m.name
            },
            width: u32::from(m.w),
            height: u32::from(m.h),
            scale: 1.0,
            primary: m.primary,
        })
        .collect())
}

pub fn open(display: u32, draw_cursor: bool) -> anyhow::Result<Opened> {
    let (conn, root) = connect()?;
    let setup = conn.setup();
    let depth = setup.roots[0].root_depth;
    let bpp = setup
        .pixmap_formats
        .iter()
        .find(|f| f.depth == depth)
        .map(|f| f.bits_per_pixel)
        .unwrap_or(0);
    if !(depth == 24 || depth == 32) || bpp != 32 || setup.image_byte_order != ImageOrder::LSB_FIRST
    {
        bail!("This X display's pixel format (depth {depth}, {bpp} bits) isn't supported.");
    }
    let list = monitors(&conn, root);
    let (index, m) = list
        .iter()
        .enumerate()
        .find(|(i, _)| *i as u32 + 1 == display)
        .or_else(|| list.iter().enumerate().find(|(_, m)| m.primary))
        .or_else(|| list.iter().enumerate().next())
        .context("There is no display to share.")?;
    let (x, y, w, h) = (m.x, m.y, m.w, m.h);
    let shm = Shm::new(&conn, usize::from(w) * usize::from(h) * 4);
    if shm.is_none() {
        tracing::info!("X11 shared memory unavailable: copying the screen the slow way");
    }
    let xfixes = conn
        .xfixes_query_version(4, 0)
        .ok()
        .and_then(|c| c.reply().ok())
        .is_some();
    // The pointer on its own connection: it's read from the session's thread.
    let pointer: Option<Box<dyn Pointer>> = if xfixes {
        connect().ok().and_then(|(c, _)| {
            c.xfixes_query_version(4, 0).ok()?.reply().ok()?;
            Some(Box::new(XPointer {
                conn: c,
                shapes: Shapes::default(),
                last: None,
            }) as Box<dyn Pointer>)
        })
    } else {
        None
    };
    let cursor = xfixes && draw_cursor;
    Ok(Opened {
        pointer,
        grab: Box::new(X11Grab {
            conn,
            root,
            x,
            y,
            w,
            h,
            shm,
            cursor,
        }),
        display: index as u32 + 1,
        bounds: Bounds {
            x: f64::from(x),
            y: f64::from(y),
            w: f64::from(w),
            h: f64::from(h),
        },
        width: u32::from(w),
        height: u32::from(h),
    })
}

/// A System V shared memory segment attached to the X server.
struct Shm {
    seg: u32,
    addr: *mut u8,
    size: usize,
}

impl Shm {
    fn new(conn: &RustConnection, size: usize) -> Option<Shm> {
        conn.shm_query_version().ok()?.reply().ok()?;
        // SAFETY: plain System V calls; the segment is marked for removal right after both
        // sides attached, so it goes away with us.
        unsafe {
            let id = libc::shmget(libc::IPC_PRIVATE, size, libc::IPC_CREAT | 0o600);
            if id < 0 {
                return None;
            }
            let addr = libc::shmat(id, std::ptr::null(), 0);
            if addr as isize == -1 {
                libc::shmctl(id, libc::IPC_RMID, std::ptr::null_mut());
                return None;
            }
            let seg = conn.generate_id().ok();
            let attached = seg
                .and_then(|seg| conn.shm_attach(seg, id as u32, false).ok())
                .and_then(|c| c.check().ok())
                .is_some();
            libc::shmctl(id, libc::IPC_RMID, std::ptr::null_mut());
            match (attached, seg) {
                (true, Some(seg)) => Some(Shm {
                    seg,
                    addr: addr.cast(),
                    size,
                }),
                _ => {
                    libc::shmdt(addr);
                    None
                }
            }
        }
    }
}

/// The pointer from XFixes: position and picture together (premultiplied ARGB).
struct XPointer {
    conn: RustConnection,
    shapes: Shapes,
    /// The X server's serial for the last picture, and the shape made from it.
    last: Option<(u32, Arc<Shape>)>,
}

impl Pointer for XPointer {
    fn read(&mut self) -> Option<Cursor> {
        let c = self.conn.xfixes_get_cursor_image().ok()?.reply().ok()?;
        if self.last.as_ref().map(|l| l.0) != Some(c.cursor_serial) {
            let mut rgba = Vec::with_capacity(c.cursor_image.len() * 4);
            for p in &c.cursor_image {
                rgba.extend_from_slice(&[
                    (p >> 16) as u8,
                    (p >> 8) as u8,
                    *p as u8,
                    (p >> 24) as u8,
                ]);
            }
            super::super::cursor::unpremultiply(&mut rgba);
            let shape = self.shapes.intern(
                u32::from(c.width),
                u32::from(c.height),
                u32::from(c.xhot),
                u32::from(c.yhot),
                1.0,
                rgba,
            );
            self.last = shape.map(|s| (c.cursor_serial, s));
        }
        Some(Cursor {
            x: f64::from(c.x),
            y: f64::from(c.y),
            visible: true,
            shape: self.last.as_ref().map(|l| l.1.clone()),
        })
    }
}

struct X11Grab {
    conn: RustConnection,
    root: Window,
    x: i16,
    y: i16,
    w: u16,
    h: u16,
    shm: Option<Shm>,
    cursor: bool,
}

impl X11Grab {
    /// Draws the pointer (premultiplied ARGB from XFixes) over the picture.
    fn draw_cursor(&self, data: &mut [u8]) {
        let Some(c) = self
            .conn
            .xfixes_get_cursor_image()
            .ok()
            .and_then(|c| c.reply().ok())
        else {
            return;
        };
        let (w, h) = (i32::from(self.w), i32::from(self.h));
        let left = i32::from(c.x) - i32::from(c.xhot) - i32::from(self.x);
        let top = i32::from(c.y) - i32::from(c.yhot) - i32::from(self.y);
        for cy in 0..i32::from(c.height) {
            let y = top + cy;
            if y < 0 || y >= h {
                continue;
            }
            for cx in 0..i32::from(c.width) {
                let x = left + cx;
                if x < 0 || x >= w {
                    continue;
                }
                let p = c.cursor_image[(cy * i32::from(c.width) + cx) as usize];
                let a = p >> 24;
                if a == 0 {
                    continue;
                }
                let i = ((y * w + x) * 4) as usize;
                for (ch, shift) in [(0, 0), (1, 8), (2, 16)] {
                    let src = (p >> shift) & 0xff;
                    let dst = u32::from(data[i + ch]);
                    data[i + ch] = (src + dst * (255 - a) / 255).min(255) as u8;
                }
            }
        }
    }
}

impl Grab for X11Grab {
    fn grab(&mut self) -> anyhow::Result<Option<Image>> {
        let len = usize::from(self.w) * usize::from(self.h) * 4;
        let mut data = if let Some(shm) = &self.shm {
            self.conn
                .shm_get_image(
                    self.root,
                    self.x,
                    self.y,
                    self.w,
                    self.h,
                    !0,
                    ImageFormat::Z_PIXMAP.into(),
                    shm.seg,
                    0,
                )?
                .reply()?;
            // SAFETY: the server wrote `len` bytes into our segment and replied.
            unsafe { std::slice::from_raw_parts(shm.addr, len.min(shm.size)) }.to_vec()
        } else {
            self.conn
                .get_image(
                    ImageFormat::Z_PIXMAP,
                    self.root,
                    self.x,
                    self.y,
                    self.w,
                    self.h,
                    !0,
                )?
                .reply()?
                .data
        };
        if data.len() < len {
            return Ok(None);
        }
        if self.cursor {
            self.draw_cursor(&mut data);
        }
        Ok(Some(Image {
            data,
            width: u32::from(self.w),
            height: u32::from(self.h),
            stride: usize::from(self.w) * 4,
        }))
    }
}

impl Drop for X11Grab {
    fn drop(&mut self) {
        if let Some(shm) = self.shm.take() {
            let _ = self.conn.shm_detach(shm.seg);
            let _ = self.conn.flush();
            // SAFETY: attached in Shm::new, detached once.
            unsafe {
                libc::shmdt(shm.addr.cast());
            }
        }
    }
}

// ---- input -------------------------------------------------------------------------------

/// Browser pixels per wheel click (buttons 4–7).
const PIXELS_PER_CLICK: f64 = 50.0;

pub struct Input {
    conn: RustConnection,
    root: Window,
    keys: HashSet<u8>,
    buttons: [bool; 3],
    wheel: (f64, f64),
    /// A keycode with nothing on it, borrowed to type characters the layout lacks.
    spare: Option<u8>,
}

/// X buttons for the browser's left, right and middle.
const BUTTONS: [u8; 3] = [1, 3, 2];

impl Input {
    pub fn new() -> anyhow::Result<Self> {
        let (conn, root) = connect()?;
        conn.xtest_get_version(2, 2)?
            .reply()
            .context("The X display can't take input (XTEST is missing)")?;
        let spare = spare_keycode(&conn);
        Ok(Input {
            conn,
            root,
            keys: HashSet::new(),
            buttons: [false; 3],
            wheel: (0.0, 0.0),
            spare,
        })
    }

    fn fake(&self, kind: u8, detail: u8, x: i16, y: i16) {
        let _ = self
            .conn
            .xtest_fake_input(kind, detail, 0, self.root, x, y, 0);
    }

    fn flush(&self) {
        let _ = self.conn.flush();
    }

    fn click(&self, button: u8, times: u32) {
        for _ in 0..times {
            self.fake(BUTTON_PRESS_EVENT, button, 0, 0);
            self.fake(BUTTON_RELEASE_EVENT, button, 0, 0);
        }
    }

    /// Types `c` by putting its keysym on the spare keycode for a moment.
    fn type_char(&self, c: char) {
        let Some(spare) = self.spare else { return };
        let sym = keysym(c);
        if self
            .conn
            .change_keyboard_mapping(1, spare, 1, &[sym])
            .is_err()
        {
            return;
        }
        // A round trip, so the mapping is in place before the key.
        let _ = self
            .conn
            .get_input_focus()
            .ok()
            .and_then(|c| c.reply().ok());
        self.fake(KEY_PRESS_EVENT, spare, 0, 0);
        self.fake(KEY_RELEASE_EVENT, spare, 0, 0);
        let _ = self
            .conn
            .get_input_focus()
            .ok()
            .and_then(|c| c.reply().ok());
    }

    pub fn apply(&mut self, ev: InputEvent) {
        match ev {
            InputEvent::Move { x, y } => {
                self.fake(MOTION_NOTIFY_EVENT, 0, x.round() as i16, y.round() as i16);
            }
            InputEvent::Button {
                x, y, button, down, ..
            } => {
                let Some(&b) = BUTTONS.get(button) else {
                    return;
                };
                self.fake(MOTION_NOTIFY_EVENT, 0, x.round() as i16, y.round() as i16);
                self.fake(
                    if down {
                        BUTTON_PRESS_EVENT
                    } else {
                        BUTTON_RELEASE_EVENT
                    },
                    b,
                    0,
                    0,
                );
                self.buttons[button] = down;
            }
            InputEvent::Wheel { dx, dy } => {
                self.wheel.0 += dx / PIXELS_PER_CLICK;
                self.wheel.1 += dy / PIXELS_PER_CLICK;
                let (h, v) = (self.wheel.0.trunc(), self.wheel.1.trunc());
                self.wheel.0 -= h;
                self.wheel.1 -= v;
                // 4 up, 5 down, 6 left, 7 right.
                if v != 0.0 {
                    self.click(if v > 0.0 { 5 } else { 4 }, v.abs() as u32);
                }
                if h != 0.0 {
                    self.click(if h > 0.0 { 7 } else { 6 }, h.abs() as u32);
                }
            }
            InputEvent::Key { code, down, .. } => {
                let Some(k) = pc_key(&code) else { return };
                let Ok(keycode) = u8::try_from(k.evdev + 8) else {
                    return;
                };
                if down {
                    self.keys.insert(keycode);
                } else {
                    self.keys.remove(&keycode);
                }
                self.fake(
                    if down {
                        KEY_PRESS_EVENT
                    } else {
                        KEY_RELEASE_EVENT
                    },
                    keycode,
                    0,
                    0,
                );
            }
            InputEvent::Text { text } => {
                for c in text.chars() {
                    self.type_char(c);
                }
                if let Some(spare) = self.spare {
                    let _ = self.conn.change_keyboard_mapping(1, spare, 1, &[0]);
                }
            }
            InputEvent::ReleaseAll => self.release_all(),
        }
        self.flush();
    }

    pub fn release_all(&mut self) {
        for keycode in std::mem::take(&mut self.keys) {
            self.fake(KEY_RELEASE_EVENT, keycode, 0, 0);
        }
        let held = std::mem::take(&mut self.buttons);
        for (down, button) in held.into_iter().zip(BUTTONS) {
            if down {
                self.fake(BUTTON_RELEASE_EVENT, button, 0, 0);
            }
        }
        self.flush();
    }
}

impl Drop for Input {
    fn drop(&mut self) {
        self.release_all();
    }
}

/// The highest keycode with no keysym at all.
fn spare_keycode(conn: &RustConnection) -> Option<u8> {
    let (min, max) = (conn.setup().min_keycode, conn.setup().max_keycode);
    let map = conn
        .get_keyboard_mapping(min, max - min + 1)
        .ok()?
        .reply()
        .ok()?;
    let per = usize::from(map.keysyms_per_keycode).max(1);
    (min..=max).rev().find(|&k| {
        let i = usize::from(k - min) * per;
        map.keysyms
            .get(i..i + per)
            .is_some_and(|syms| syms.iter().all(|&s| s == 0))
    })
}
