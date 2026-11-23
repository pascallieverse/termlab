//! What goes over the screen websocket (docs/server-protocol.md, "Screen sessions").
//!
//! Machine → browser:
//! - binary video frames: `[1u8, flags, seq: u32 BE, timestamp_us: u64 BE, AVCC data…]`
//!   (flags bit 0: key frame);
//! - JSON text: `hello`, `config` (before the first frame and whenever the stream changes),
//!   `status`, `clipboard`, `error`.
//!
//! Browser → machine (JSON text): `ack`, `keyframe`, `display`, `quality`, `pointer`,
//! `wheel`, `key`, `text`, `release`, `clipboard.set`, `clipboard.get`.

use serde::{Deserialize, Serialize};

pub const FRAME_VIDEO: u8 = 1;

/// A display that can be shared.
#[derive(Debug, Clone, Serialize)]
pub struct DisplayInfo {
    pub id: u32,
    pub name: String,
    /// Size in points (logical pixels).
    pub width: u32,
    pub height: u32,
    /// Physical pixels per point.
    pub scale: f64,
    pub primary: bool,
}

/// What the browser needs to set up its decoder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoConfig {
    /// WebCodecs codec string, e.g. `avc1.4d0028`.
    pub codec: String,
    /// The `avcC` record (decoder configuration).
    pub description: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// An encoded frame.
pub struct Encoded {
    pub key: bool,
    pub ts_us: u64,
    pub data: Vec<u8>,
    pub config: Option<VideoConfig>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Quality {
    Low,
    #[default]
    Balanced,
    High,
}

impl Quality {
    /// (largest width in pixels, frames per second, average bits per second)
    pub fn params(self) -> (u32, u32, u32) {
        match self {
            Quality::Low => (1280, 30, 1_500_000),
            Quality::Balanced => (1920, 60, 6_000_000),
            // Up to 4K: a Retina display at its full pixel size, for 1:1 viewing.
            Quality::High => (3840, 60, 16_000_000),
        }
    }
}

/// A mouse button, as the browser numbers them (`MouseEvent.button`).
pub fn button_index(b: u8) -> Option<usize> {
    match b {
        0 => Some(0), // left
        2 => Some(1), // right
        1 => Some(2), // middle
        _ => None,
    }
}

/// Input and control from the browser.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FromBrowser {
    /// The browser decoded frame `seq`: one more may be sent.
    Ack {
        #[allow(dead_code)]
        seq: u32,
    },
    /// Start over from a key frame (the decoder was reset).
    Keyframe,
    Display {
        id: u32,
    },
    Quality {
        quality: Quality,
    },
    /// Pointer at (x, y), each 0..1 across the shown display. `action`: move, down or up.
    Pointer {
        x: f64,
        y: f64,
        action: PointerAction,
        #[serde(default)]
        button: u8,
        /// Click count for down/up (double click = 2).
        #[serde(default)]
        clicks: u8,
    },
    /// Scroll by this many pixels (positive y: down, as in the browser).
    Wheel {
        dx: f64,
        dy: f64,
    },
    /// A key, as `KeyboardEvent.code` (layout independent).
    Key {
        code: String,
        down: bool,
        #[serde(default)]
        repeat: bool,
    },
    /// Text to type (phone keyboards, paste).
    Text {
        text: String,
    },
    /// Let go of every key and button held from the browser.
    Release,
    #[serde(rename = "clipboard.set")]
    ClipboardSet {
        text: String,
    },
    #[serde(rename = "clipboard.get")]
    ClipboardGet,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PointerAction {
    Move,
    Down,
    Up,
}

/// One input event for the platform layer, in global screen coordinates (points).
#[derive(Debug, Clone)]
pub enum InputEvent {
    Move {
        x: f64,
        y: f64,
    },
    Button {
        x: f64,
        y: f64,
        button: usize,
        down: bool,
        clicks: u8,
    },
    Wheel {
        dx: f64,
        dy: f64,
    },
    Key {
        code: String,
        down: bool,
        repeat: bool,
    },
    Text {
        text: String,
    },
    ReleaseAll,
}

pub fn video_frame(seq: u32, e: &Encoded) -> Vec<u8> {
    let mut out = Vec::with_capacity(14 + e.data.len());
    out.push(FRAME_VIDEO);
    out.push(u8::from(e.key));
    out.extend_from_slice(&seq.to_be_bytes());
    out.extend_from_slice(&e.ts_us.to_be_bytes());
    out.extend_from_slice(&e.data);
    out
}

/// The WebCodecs codec string for an `avcC` record: `avc1.PPCCLL`.
pub fn avc_codec(avcc: &[u8]) -> Option<String> {
    (avcc.len() >= 4 && avcc[0] == 1)
        .then(|| format!("avc1.{:02x}{:02x}{:02x}", avcc[1], avcc[2], avcc[3]))
}

/// AVCC (length-prefixed NAL units) to Annex B (start codes), with the parameter sets of
/// `avcc` first on key frames. For the `probe` file only.
pub fn annex_b(avcc_record: &[u8], frame: &[u8], key: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(frame.len() + 64);
    let start = [0u8, 0, 0, 1];
    if key && avcc_record.len() > 6 {
        let mut i = 5;
        let sps = (avcc_record[i] & 0x1f) as usize;
        i += 1;
        for _ in 0..sps {
            let Some(len) = avcc_record.get(i..i + 2) else {
                break;
            };
            let len = u16::from_be_bytes([len[0], len[1]]) as usize;
            i += 2;
            out.extend_from_slice(&start);
            out.extend_from_slice(avcc_record.get(i..i + len).unwrap_or_default());
            i += len;
        }
        let pps = avcc_record.get(i).copied().unwrap_or(0) as usize;
        i += 1;
        for _ in 0..pps {
            let Some(len) = avcc_record.get(i..i + 2) else {
                break;
            };
            let len = u16::from_be_bytes([len[0], len[1]]) as usize;
            i += 2;
            out.extend_from_slice(&start);
            out.extend_from_slice(avcc_record.get(i..i + len).unwrap_or_default());
            i += len;
        }
    }
    let mut i = 0;
    while i + 4 <= frame.len() {
        let len = u32::from_be_bytes([frame[i], frame[i + 1], frame[i + 2], frame[i + 3]]) as usize;
        i += 4;
        let Some(nal) = frame.get(i..i + len) else {
            break;
        };
        out.extend_from_slice(&start);
        out.extend_from_slice(nal);
        i += len;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codec_string_comes_from_the_avcc_header() {
        assert_eq!(
            avc_codec(&[1, 0x4d, 0x00, 0x28, 0xff]).as_deref(),
            Some("avc1.4d0028")
        );
        assert_eq!(avc_codec(&[0, 1, 2]), None);
    }

    #[test]
    fn frames_carry_seq_timestamp_and_key_flag() {
        let e = Encoded {
            key: true,
            ts_us: 5,
            data: vec![9, 9],
            config: None,
        };
        let f = video_frame(7, &e);
        assert_eq!(f[0], FRAME_VIDEO);
        assert_eq!(f[1], 1);
        assert_eq!(&f[2..6], &7u32.to_be_bytes());
        assert_eq!(&f[6..14], &5u64.to_be_bytes());
        assert_eq!(&f[14..], &[9, 9]);
    }

    #[test]
    fn browser_messages_parse() {
        let m: FromBrowser = serde_json::from_str(
            r#"{"type":"pointer","x":0.5,"y":0.25,"action":"down","button":0,"clicks":2}"#,
        )
        .unwrap();
        assert!(matches!(
            m,
            FromBrowser::Pointer {
                action: PointerAction::Down,
                clicks: 2,
                ..
            }
        ));
        let m: FromBrowser = serde_json::from_str(r#"{"type":"clipboard.get"}"#).unwrap();
        assert!(matches!(m, FromBrowser::ClipboardGet));
        let m: FromBrowser =
            serde_json::from_str(r#"{"type":"quality","quality":"high"}"#).unwrap();
        assert!(matches!(
            m,
            FromBrowser::Quality {
                quality: Quality::High
            }
        ));
    }

    #[test]
    fn annex_b_replaces_length_prefixes() {
        let frame = [0, 0, 0, 2, 0x65, 0x88];
        assert_eq!(annex_b(&[], &frame, false), vec![0, 0, 0, 1, 0x65, 0x88]);
    }
}
