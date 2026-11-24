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
    /// WebCodecs codec string, e.g. `avc1.640034` or `hvc1.1.6.L183.B0`.
    pub codec: String,
    /// The `avcC` or `hvcC` record (decoder configuration).
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
    /// 4K.
    High,
    /// The display at its full pixel size (6K on a 6K display): beyond 4K needs HEVC, and
    /// the encoder manages fewer frames there.
    Native,
}

/// The video format: H.264 works in every browser with WebCodecs; HEVC (H.265) gives a
/// sharper picture for the same bits and goes beyond 4K, where the browser can decode it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Codec {
    #[default]
    H264,
    Hevc,
}

/// How a stream is encoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Encoding {
    pub codec: Codec,
    /// The level it comes from (the software encoder sets its sharpness by it).
    pub quality: Quality,
    /// Largest width in pixels (the display's own size when smaller).
    pub max_width: u32,
    pub fps: u32,
    /// Average bits per second.
    pub bitrate: u32,
    /// Full colour detail (YUV 4:4:4, HEVC only): coloured text stays crisp instead of
    /// getting colour fringes from colour stored at half resolution.
    pub chroma444: bool,
    /// Draw the pointer into the picture (a browser that draws it itself says so).
    pub draw_cursor: bool,
}

/// What the browser said it can do (its `codecs` message).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Browser {
    /// The widest HEVC picture it decodes, 0 for none.
    pub hevc: u32,
    /// The widest HEVC 4:4:4 picture it decodes, 0 for none.
    pub hevc444: u32,
    /// It draws the pointer itself (from `cursor` messages).
    pub cursor: bool,
}

/// Widest H.264 picture: the hardware encoders stop at 4096, and 4K keeps the level common.
const H264_MAX_WIDTH: u32 = 3840;

impl Quality {
    /// How to encode at this level for `browser`.
    pub fn encoding(self, browser: Browser) -> Encoding {
        let (max_width, fps, bitrate) = match self {
            Quality::Low => (1280, 30, 1_500_000),
            Quality::Balanced => (1920, 60, 6_000_000),
            Quality::High | Quality::Native => (H264_MAX_WIDTH, 60, 16_000_000),
        };
        let draw_cursor = !browser.cursor;
        let sharp = matches!(self, Quality::High | Quality::Native);
        match (self, browser.hevc) {
            (_, 0) => Encoding {
                codec: Codec::H264,
                quality: self,
                max_width,
                fps,
                bitrate,
                chroma444: false,
                draw_cursor,
            },
            // An M4 encodes 6016×3384 HEVC at about 24 fps (4K: over 50).
            (Quality::Native, w) => Encoding {
                codec: Codec::Hevc,
                quality: self,
                max_width: w,
                fps: 30,
                bitrate: 24_000_000,
                chroma444: browser.hevc444 >= w,
                draw_cursor,
            },
            (_, w) => {
                let max_width = max_width.min(w);
                Encoding {
                    codec: Codec::Hevc,
                    quality: self,
                    max_width,
                    fps,
                    bitrate,
                    // Only for Sharp: it costs encoder time, and smaller pictures are scaled
                    // down anyway, which blurs more than half-resolution colour does.
                    chroma444: sharp && browser.hevc444 >= max_width,
                    draw_cursor,
                }
            }
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
    /// What the browser can do: the widest HEVC picture it decodes (0 for none: H.264
    /// only), the same for HEVC 4:4:4, and whether it draws the pointer itself.
    Codecs {
        #[serde(default)]
        hevc: u32,
        #[serde(default)]
        hevc444: u32,
        #[serde(default)]
        cursor: bool,
    },
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
// Click counts and key repeats only matter to macOS (Windows and X11 work them out).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
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

/// The WebCodecs codec string for an `hvcC` record (ISO/IEC 14496-15, annex E):
/// `hvc1.<space><profile>.<compatibility, bit-reversed>.<tier><level>.<constraints>`.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn hevc_codec(hvcc: &[u8]) -> Option<String> {
    if hvcc.len() < 13 || hvcc[0] != 1 {
        return None;
    }
    let space = ["", "A", "B", "C"][(hvcc[1] >> 6) as usize];
    let tier = if hvcc[1] & 0x20 != 0 { 'H' } else { 'L' };
    let profile = hvcc[1] & 0x1f;
    let compat = u32::from_be_bytes([hvcc[2], hvcc[3], hvcc[4], hvcc[5]]).reverse_bits();
    let level = hvcc[12];
    let mut constraints = &hvcc[6..12];
    while let [rest @ .., 0] = constraints {
        constraints = rest;
    }
    let mut out = format!("hvc1.{space}{profile}.{compat:X}.{tier}{level}");
    for b in constraints {
        out.push_str(&format!(".{b:X}"));
    }
    Some(out)
}

/// The parameter sets (SPS, PPS; for HEVC also VPS) in a decoder configuration record.
pub fn parameter_sets(codec: Codec, record: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    let nal = |i: &mut usize| -> Option<&[u8]> {
        let len = record.get(*i..*i + 2)?;
        let len = u16::from_be_bytes([len[0], len[1]]) as usize;
        let n = record.get(*i + 2..*i + 2 + len)?;
        *i += 2 + len;
        Some(n)
    };
    match codec {
        Codec::H264 => {
            let mut i = 5;
            for count_mask in [0x1f, 0xff] {
                let Some(&count) = record.get(i) else { break };
                i += 1;
                for _ in 0..(count & count_mask) {
                    let Some(n) = nal(&mut i) else { return out };
                    out.push(n);
                }
            }
        }
        Codec::Hevc => {
            let mut i = 22;
            let Some(&arrays) = record.get(i) else {
                return out;
            };
            i += 1;
            for _ in 0..arrays {
                let Some(count) = record.get(i + 1..i + 3) else {
                    return out;
                };
                let count = u16::from_be_bytes([count[0], count[1]]);
                i += 3;
                for _ in 0..count {
                    let Some(n) = nal(&mut i) else { return out };
                    out.push(n);
                }
            }
        }
    }
    out
}

/// Length-prefixed NAL units to Annex B (start codes), with the parameter sets of `record`
/// first on key frames. For the `probe` file only.
pub fn annex_b(codec: Codec, record: &[u8], frame: &[u8], key: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(frame.len() + 64);
    let start = [0u8, 0, 0, 1];
    if key {
        for n in parameter_sets(codec, record) {
            out.extend_from_slice(&start);
            out.extend_from_slice(n);
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
        assert_eq!(
            annex_b(Codec::H264, &[], &frame, false),
            vec![0, 0, 0, 1, 0x65, 0x88]
        );
    }

    #[test]
    fn hevc_codec_string() {
        // Main profile, compatibility flags for Main and Main 10, level 5.1 (153), only the
        // first constraint byte set: the example from the spec, `hvc1.1.6.L153.B0`.
        let mut r = vec![1, 0x01, 0x60, 0, 0, 0, 0xb0, 0, 0, 0, 0, 0, 153];
        r.extend_from_slice(&[0; 10]);
        assert_eq!(hevc_codec(&r).as_deref(), Some("hvc1.1.6.L153.B0"));
        r[1] = 0x21; // high tier
        assert_eq!(hevc_codec(&r).as_deref(), Some("hvc1.1.6.H153.B0"));
        assert_eq!(hevc_codec(&[0; 5]), None);
    }

    #[test]
    fn parameter_sets_from_records() {
        // avcC: one SPS [0x67, 1], one PPS [0x68].
        let avcc = [1, 0x64, 0, 0x34, 0xff, 0xe1, 0, 2, 0x67, 1, 1, 0, 1, 0x68];
        assert_eq!(
            parameter_sets(Codec::H264, &avcc),
            vec![&[0x67, 1][..], &[0x68][..]]
        );
        // hvcC: 22 header bytes, then 2 arrays (VPS, SPS) of one NAL each.
        let mut hvcc = vec![1; 22];
        hvcc.extend_from_slice(&[2, 0x20, 0, 1, 0, 1, 0x40, 0x21, 0, 1, 0, 2, 0x42, 7]);
        assert_eq!(
            parameter_sets(Codec::Hevc, &hvcc),
            vec![&[0x40][..], &[0x42, 7][..]]
        );
    }

    fn hevc(w: u32) -> Browser {
        Browser {
            hevc: w,
            ..Browser::default()
        }
    }

    #[test]
    fn hevc_when_the_browser_decodes_it() {
        assert_eq!(Quality::High.encoding(hevc(0)).codec, Codec::H264);
        assert_eq!(Quality::Native.encoding(hevc(0)).max_width, 3840);
        let e = Quality::High.encoding(hevc(7680));
        assert_eq!((e.codec, e.max_width), (Codec::Hevc, 3840));
        let e = Quality::Native.encoding(hevc(7680));
        assert_eq!((e.codec, e.max_width), (Codec::Hevc, 7680));
        let e = Quality::Balanced.encoding(hevc(3840));
        assert_eq!((e.codec, e.max_width), (Codec::Hevc, 1920));
        assert!(e.draw_cursor && !e.chroma444);
    }

    #[test]
    fn full_colour_for_sharp_where_decoded() {
        let b = Browser {
            hevc: 7680,
            hevc444: 3840,
            cursor: true,
        };
        let e = Quality::High.encoding(b);
        assert!(e.chroma444 && !e.draw_cursor);
        assert!(!Quality::Balanced.encoding(b).chroma444);
        // 4:4:4 only up to 4K here, so Native (wider) stays 4:2:0.
        assert!(!Quality::Native.encoding(b).chroma444);
        assert!(!Quality::High.encoding(Browser { hevc444: 0, ..b }).chroma444);
    }
}
