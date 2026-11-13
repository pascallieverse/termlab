//! A capped in-memory byte buffer of a session's raw terminal output, plus helpers to
//! flush it to (and load it from) a per-session file.
//!
//! The buffer stores the raw pty bytes (escape sequences included), so replaying it into
//! a fresh terminal emulator reproduces the screen.

use std::collections::VecDeque;
use std::fs;
use std::io::Write;
use std::path::Path;

/// Default cap: 5 MiB per session.
pub const DEFAULT_SCROLLBACK_BYTES: usize = 5 * 1024 * 1024;

/// When trimming, look this far past the cut for a line break so replay starts on a
/// clean line instead of in the middle of an escape sequence or UTF-8 character.
const TRIM_SEARCH_WINDOW: usize = 4096;

#[derive(Debug)]
pub struct Scrollback {
    buf: VecDeque<u8>,
    cap: usize,
    total_written: u64,
}

impl Scrollback {
    pub fn new(cap: usize) -> Self {
        assert!(cap > 0, "scrollback cap must be positive");
        Self {
            buf: VecDeque::with_capacity(cap.min(64 * 1024)),
            cap,
            total_written: 0,
        }
    }

    /// Creates a buffer pre-filled with earlier output (e.g. loaded from disk).
    pub fn with_contents(cap: usize, data: &[u8]) -> Self {
        let mut s = Self::new(cap);
        s.push(data);
        s
    }

    pub fn push(&mut self, data: &[u8]) {
        self.total_written += data.len() as u64;
        // Skip bytes that would be trimmed anyway, but keep enough slack for the
        // line-boundary search below.
        let keep_from = data.len().saturating_sub(self.cap + TRIM_SEARCH_WINDOW);
        if keep_from > 0 {
            self.buf.clear();
        }
        self.buf.extend(&data[keep_from..]);
        if self.buf.len() > self.cap {
            let mut cut = self.buf.len() - self.cap;
            let limit = (cut + TRIM_SEARCH_WINDOW).min(self.buf.len());
            if let Some(nl) = (cut..limit).find(|&i| self.buf[i] == b'\n') {
                cut = nl + 1;
            }
            self.buf.drain(..cut);
        }
    }

    pub fn len(&self) -> usize {
        self.buf.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    pub fn cap(&self) -> usize {
        self.cap
    }

    /// Bytes ever pushed, including those since trimmed away.
    pub fn total_written(&self) -> u64 {
        self.total_written
    }

    pub fn snapshot(&self) -> Vec<u8> {
        let (a, b) = self.buf.as_slices();
        let mut out = Vec::with_capacity(a.len() + b.len());
        out.extend_from_slice(a);
        out.extend_from_slice(b);
        out
    }

    /// Atomically writes the buffer to `path` (write to a temp file, then rename).
    pub fn flush_to(&self, path: &Path) -> std::io::Result<()> {
        write_atomically(path, &self.snapshot())
    }
}

pub fn write_atomically(path: &Path, data: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(data)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)
}

/// Loads a flushed scrollback file, or `None` if there is none.
pub fn load(path: &Path) -> std::io::Result<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(d) => Ok(Some(d)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_everything_under_the_cap() {
        let mut s = Scrollback::new(100);
        s.push(b"hello ");
        s.push(b"world");
        assert_eq!(s.snapshot(), b"hello world");
        assert_eq!(s.total_written(), 11);
    }

    #[test]
    fn trims_oldest_bytes_to_a_line_boundary() {
        let mut s = Scrollback::new(16);
        s.push(b"line one\nline two\nline three\n");
        assert!(s.len() <= 16);
        assert_eq!(s.snapshot(), b"line three\n");
        assert_eq!(s.total_written(), 29);
    }

    #[test]
    fn trims_exactly_when_no_line_break_is_near() {
        let mut s = Scrollback::new(8);
        s.push(b"abcdefghij");
        assert_eq!(s.snapshot(), b"cdefghij");
        s.push(b"XY");
        assert_eq!(s.snapshot(), b"efghijXY");
    }

    #[test]
    fn a_single_huge_push_keeps_its_tail() {
        let mut s = Scrollback::new(4);
        s.push(b"0123456789");
        assert_eq!(s.snapshot(), b"6789");
    }

    #[test]
    fn flush_and_load_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sb").join("x.log");
        assert_eq!(load(&path).unwrap(), None);
        let s = Scrollback::with_contents(1024, b"\x1b[31mred\x1b[0m\r\n");
        s.flush_to(&path).unwrap();
        assert_eq!(load(&path).unwrap().unwrap(), s.snapshot());
    }
}
