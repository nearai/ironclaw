//! Truncating terminal writer for tracing.
//!
//! Tracing events from LLM providers can dump 10KB+ JSON bodies to stderr.
//! Rather than truncating at every call site (fragile, easy to miss), we
//! handle it at the writer level: the fmt layer gets a `TruncatingStderr`
//! that caps each event before flushing, while the web gateway `WebLogLayer`
//! still sees the full, untruncated content.
//!
//! ```text
//! tracing::debug!("body: {huge_json}")
//!        |
//!        v
//!   tracing_subscriber::registry()
//!        |
//!        +-- fmt::layer().with_writer(TruncatingStderr)  <-- caps at 500B
//!        |       \-- stderr (truncated)
//!        |
//!        \-- WebLogLayer (unchanged)
//!                \-- SSE broadcast (full)
//! ```

use std::io::{self, Write};

use tracing_subscriber::fmt::MakeWriter;

/// Maximum bytes per tracing event written to the terminal.
const TERMINAL_MAX_EVENT_BYTES: usize = 500;

/// A `MakeWriter` that creates per-event buffers which truncate on flush.
///
/// Each call to `make_writer()` returns an `EventBuffer`. All `write()`
/// calls accumulate into the buffer. When the buffer drops (after the fmt
/// layer finishes writing one event), it flushes to stderr, truncating if
/// the total exceeds `TERMINAL_MAX_EVENT_BYTES`.
#[derive(Clone)]
pub struct TruncatingStderr {
    max_bytes: usize,
}

impl Default for TruncatingStderr {
    fn default() -> Self {
        Self {
            max_bytes: TERMINAL_MAX_EVENT_BYTES,
        }
    }
}

impl TruncatingStderr {
    #[cfg(test)]
    fn with_max_bytes(max_bytes: usize) -> Self {
        Self { max_bytes }
    }
}

impl<'a> MakeWriter<'a> for TruncatingStderr {
    type Writer = EventBuffer;

    fn make_writer(&'a self) -> Self::Writer {
        EventBuffer {
            buf: Vec::with_capacity(256),
            max_bytes: self.max_bytes,
            #[cfg(test)]
            sink: None,
        }
    }
}

/// Per-event buffer that truncates on drop.
pub struct EventBuffer {
    buf: Vec<u8>,
    max_bytes: usize,
    /// Test-only: capture output instead of writing to stderr.
    #[cfg(test)]
    sink: Option<std::sync::Arc<std::sync::Mutex<Vec<u8>>>>,
}

impl Write for EventBuffer {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.buf.extend_from_slice(data);
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Find the last valid UTF-8 char boundary at or before `pos` in `bytes`.
///
/// Walks backwards from `pos` until we find a byte that isn't a UTF-8
/// continuation byte (0x80..0xBF). Returns 0 if the entire prefix is
/// somehow invalid (shouldn't happen with valid UTF-8 input from tracing).
fn utf8_floor(bytes: &[u8], pos: usize) -> usize {
    let mut i = pos;
    // UTF-8 continuation bytes have the form 10xxxxxx (0x80..0xBF).
    // Walk backwards past them to find the start of the last character.
    while i > 0 && bytes[i] & 0xC0 == 0x80 {
        i -= 1;
    }
    i
}

impl Drop for EventBuffer {
    fn drop(&mut self) {
        if self.buf.is_empty() {
            return;
        }

        let output = if self.buf.len() <= self.max_bytes {
            &self.buf[..]
        } else {
            // Truncate at a UTF-8 safe boundary
            let cut = utf8_floor(&self.buf, self.max_bytes);
            let suffix = format!("...[{}B total]\n", self.buf.len());
            let mut truncated = Vec::with_capacity(cut + suffix.len());
            // Strip trailing newline from the cut portion (we add our own via suffix)
            let cut_slice = &self.buf[..cut];
            let trimmed = if cut_slice.last() == Some(&b'\n') {
                &cut_slice[..cut_slice.len() - 1]
            } else {
                cut_slice
            };
            truncated.extend_from_slice(trimmed);
            truncated.extend_from_slice(suffix.as_bytes());

            #[cfg(test)]
            if let Some(ref sink) = self.sink {
                let mut s = sink.lock().expect("test sink lock poisoned");
                s.extend_from_slice(&truncated);
                return;
            }

            let _ = io::stderr().write_all(&truncated);
            return;
        };

        #[cfg(test)]
        if let Some(ref sink) = self.sink {
            let mut s = sink.lock().expect("test sink lock poisoned");
            s.extend_from_slice(output);
            return;
        }

        let _ = io::stderr().write_all(output);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use crate::tracing_fmt::{EventBuffer, TruncatingStderr, utf8_floor};

    use std::io::Write;

    /// Helper: create an EventBuffer that captures output to a shared Vec
    /// instead of writing to stderr.
    fn test_buffer(max_bytes: usize) -> (EventBuffer, Arc<Mutex<Vec<u8>>>) {
        let sink = Arc::new(Mutex::new(Vec::new()));
        let buf = EventBuffer {
            buf: Vec::new(),
            max_bytes,
            sink: Some(Arc::clone(&sink)),
        };
        (buf, sink)
    }

    #[test]
    fn test_short_event_not_truncated() {
        let (mut buf, sink) = test_buffer(500);
        buf.write_all(b"hello world\n").unwrap();
        drop(buf);

        let output = sink.lock().unwrap();
        assert_eq!(&*output, b"hello world\n");
    }

    #[test]
    fn test_long_event_truncated() {
        let (mut buf, sink) = test_buffer(20);
        let data = "abcdefghijklmnopqrstuvwxyz0123456789\n";
        buf.write_all(data.as_bytes()).unwrap();
        let total = data.len();
        drop(buf);

        let output = sink.lock().unwrap();
        let output_str = String::from_utf8_lossy(&output);
        // Should contain the suffix with total byte count
        assert!(
            output_str.contains(&format!("...[{}B total]", total)),
            "expected truncation suffix, got: {}",
            output_str
        );
        // Should be shorter than the original
        assert!(output.len() < total);
    }

    #[test]
    fn test_utf8_boundary_safe() {
        // "Helloé" = [72, 101, 108, 108, 111, 195, 169]
        //                                         ^-- 2-byte UTF-8 char
        // If we truncate at 6 bytes, we'd land in the middle of 'é'.
        // utf8_floor should back up to byte 5 (start of 'é' = 195).
        let (mut buf, sink) = test_buffer(6);
        let data = "Helloé world";
        buf.write_all(data.as_bytes()).unwrap();
        drop(buf);

        let output = sink.lock().unwrap();
        let output_str = String::from_utf8(output.clone());
        assert!(
            output_str.is_ok(),
            "output should be valid UTF-8, got bytes: {:?}",
            &*output
        );
        let s = output_str.unwrap();
        assert!(
            s.contains("...["),
            "should be truncated with suffix, got: {}",
            s
        );
        // The truncated prefix must be valid UTF-8 up to the cut point.
        // "Hello" (5 bytes) is the last valid cut before the 2-byte é.
        assert!(
            s.starts_with("Hello"),
            "should start with 'Hello', got: {}",
            s
        );
    }

    #[test]
    fn test_utf8_floor_basic() {
        // ASCII: every byte is a valid boundary
        assert_eq!(utf8_floor(b"hello", 3), 3);

        // 2-byte UTF-8 char é = [0xC3, 0xA9]
        // Landing on the continuation byte (0xA9) should back up to 0xC3
        let bytes = "Hé".as_bytes(); // [72, 0xC3, 0xA9]
        assert_eq!(utf8_floor(bytes, 2), 1); // backs up to start of é

        // 3-byte UTF-8 char (e.g. あ = [0xE3, 0x81, 0x82])
        let bytes = "aあ".as_bytes(); // [97, 0xE3, 0x81, 0x82]
        assert_eq!(utf8_floor(bytes, 2), 1); // backs up past continuation to 0xE3
        assert_eq!(utf8_floor(bytes, 3), 1); // same: 0x82 is continuation, 0x81 is too
    }

    #[test]
    fn test_multiple_writes_accumulated() {
        let (mut buf, sink) = test_buffer(500);
        buf.write_all(b"hello ").unwrap();
        buf.write_all(b"world\n").unwrap();
        drop(buf);

        let output = sink.lock().unwrap();
        assert_eq!(&*output, b"hello world\n");
    }

    #[test]
    fn test_empty_buffer_no_output() {
        let (_buf, sink) = test_buffer(500);
        // drop without writing
        drop(_buf);

        let output = sink.lock().unwrap();
        assert!(output.is_empty());
    }

    #[test]
    fn test_default_max_bytes() {
        let writer = TruncatingStderr::default();
        assert_eq!(writer.max_bytes, 500);
    }

    #[test]
    fn test_custom_max_bytes() {
        let writer = TruncatingStderr::with_max_bytes(100);
        assert_eq!(writer.max_bytes, 100);
    }

    #[test]
    fn test_exactly_at_limit_not_truncated() {
        let (mut buf, sink) = test_buffer(5);
        buf.write_all(b"hello").unwrap();
        drop(buf);

        let output = sink.lock().unwrap();
        assert_eq!(&*output, b"hello");
    }

    #[test]
    fn test_one_over_limit_truncated() {
        let (mut buf, sink) = test_buffer(5);
        buf.write_all(b"hello!").unwrap();
        drop(buf);

        let output = sink.lock().unwrap();
        let s = String::from_utf8_lossy(&output);
        assert!(s.contains("...[6B total]"), "got: {}", s);
    }

    #[test]
    fn test_4byte_utf8_boundary() {
        // 4-byte UTF-8 char: 𝄞 (musical symbol) = [0xF0, 0x9D, 0x84, 0x9E]
        let data = "AB𝄞CD";
        // bytes: [65, 66, 0xF0, 0x9D, 0x84, 0x9E, 67, 68]
        // Truncating at byte 4 lands in the middle of the 4-byte char
        let (mut buf, sink) = test_buffer(4);
        buf.write_all(data.as_bytes()).unwrap();
        drop(buf);

        let output = sink.lock().unwrap();
        let s = String::from_utf8(output.clone());
        assert!(s.is_ok(), "output must be valid UTF-8, got: {:?}", &*output);
        let s = s.unwrap();
        // Should back up to byte 2 (just "AB"), since bytes 2..5 are all part of 𝄞
        assert!(s.starts_with("AB"), "expected 'AB', got: {}", s);
        assert!(s.contains("...["), "should be truncated, got: {}", s);
    }
}
