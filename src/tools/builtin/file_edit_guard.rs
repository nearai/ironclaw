//! Read-state tracking and string matching utilities for file edit tools.
//!
//! Provides:
//! - **ReadFileState**: tracks which files have been read and their mtime at read time,
//!   enabling "must read before edit" and staleness detection.
//! - **Fuzzy matching**: trailing whitespace normalization and quote normalization
//!   for when the LLM's `old_string` doesn't exactly match the file content.
//! - **Encoding detection**: UTF-16LE BOM detection with line-ending preservation.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use tokio::sync::RwLock;
use uuid::Uuid;

use crate::tools::tool::ToolError;

// ── Read-state tracker ───────────────────────────────────────────────

/// Tracks files that have been read per job/session, along with their mtime.
///
/// State is keyed by `job_id` so that concurrent sessions sharing the same
/// registry do not leak read-state across job boundaries.
#[derive(Debug, Default)]
pub struct ReadFileState {
    /// Maps (job_id, canonical path) → mtime at read time.
    entries: HashMap<(Uuid, PathBuf), ReadEntry>,
}

#[derive(Debug, Clone)]
struct ReadEntry {
    /// File modification time when it was last read.
    mtime: SystemTime,
    /// Whether the read was a partial view (offset/limit).
    partial: bool,
}

impl ReadFileState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record that a file was read within a specific job.
    pub fn record_read(&mut self, job_id: Uuid, path: &Path, mtime: SystemTime, partial: bool) {
        self.entries
            .insert((job_id, path.to_path_buf()), ReadEntry { mtime, partial });
    }

    /// Check whether the file was read before editing within the given job.
    /// Returns an appropriate error if not read, or if the file is stale.
    pub fn check_before_edit(
        &self,
        job_id: Uuid,
        path: &Path,
        current_mtime: SystemTime,
    ) -> Result<(), ToolError> {
        let key = (job_id, path.to_path_buf());
        let Some(entry) = self.entries.get(&key) else {
            return Err(ToolError::ExecutionFailed(format!(
                "File has not been read yet: {}. Use read_file first before editing.",
                path.display()
            )));
        };

        if entry.partial {
            return Err(ToolError::ExecutionFailed(format!(
                "File was read with offset/limit (partial view): {}. \
                 Read the full file before editing to avoid overwriting unseen content.",
                path.display()
            )));
        }

        // Allow a small tolerance for filesystem timestamp granularity (1 second).
        if let Ok(delta) = current_mtime.duration_since(entry.mtime)
            && delta.as_secs() > 1
        {
            return Err(ToolError::ExecutionFailed(format!(
                "File has been modified since it was last read: {}. \
                 Read it again before editing to see the current content.",
                path.display()
            )));
        }

        Ok(())
    }

    /// Update the mtime after a successful write (so subsequent edits don't
    /// falsely report staleness).
    pub fn update_mtime(&mut self, job_id: Uuid, path: &Path, mtime: SystemTime) {
        let key = (job_id, path.to_path_buf());
        if let Some(entry) = self.entries.get_mut(&key) {
            entry.mtime = mtime;
            entry.partial = false;
        }
    }
}

/// Shared read-file state for injection into tools.
pub type SharedReadFileState = Arc<RwLock<ReadFileState>>;

/// Create a new shared read-file state.
pub fn shared_read_file_state() -> SharedReadFileState {
    Arc::new(RwLock::new(ReadFileState::new()))
}

// ── Fuzzy string matching ────────────────────────────────────────────

/// Result of a fuzzy match attempt.
#[derive(Debug)]
pub struct FuzzyMatch {
    /// The actual string found in the file (preserving original formatting).
    pub actual: String,
    /// What normalization was applied.
    pub method: MatchMethod,
}

#[derive(Debug, PartialEq)]
pub enum MatchMethod {
    Exact,
    TrailingWhitespace,
    QuoteNormalization,
    Both,
}

/// Try to find `needle` in `haystack`, falling back to normalized forms.
///
/// Returns the actual substring from `haystack` that matched, so replacements
/// preserve the file's original formatting.
pub fn find_match(haystack: &str, needle: &str) -> Option<FuzzyMatch> {
    // 1. Exact match
    if haystack.contains(needle) {
        return Some(FuzzyMatch {
            actual: needle.to_string(),
            method: MatchMethod::Exact,
        });
    }

    // 2. Trailing whitespace normalization
    let needle_stripped = strip_trailing_whitespace(needle);
    let haystack_stripped = strip_trailing_whitespace(haystack);
    if let Some(actual) = find_original_span(haystack, &haystack_stripped, &needle_stripped) {
        return Some(FuzzyMatch {
            actual,
            method: MatchMethod::TrailingWhitespace,
        });
    }

    // 3. Quote normalization (curly → straight on both sides)
    let needle_normalized = normalize_quotes(needle);
    let haystack_normalized = normalize_quotes(haystack);
    if let Some(idx) = haystack_normalized.find(&needle_normalized) {
        // Map back to original haystack to preserve curly quotes in the actual string.
        // Quote normalization is char-for-char (same byte length for ASCII replacements),
        // but curly quotes are multi-byte while straight quotes are single-byte, so lengths
        // may differ. Use char-based indexing instead.
        let char_start = haystack_normalized[..idx].chars().count();
        let char_len = needle_normalized.chars().count();
        let actual: String = haystack.chars().skip(char_start).take(char_len).collect();
        return Some(FuzzyMatch {
            actual,
            method: MatchMethod::QuoteNormalization,
        });
    }

    // 4. Both normalizations
    let needle_both = normalize_quotes(&needle_stripped);
    let haystack_both = normalize_quotes(&haystack_stripped);
    if let Some(actual) = find_original_span(haystack, &haystack_both, &needle_both) {
        return Some(FuzzyMatch {
            actual,
            method: MatchMethod::Both,
        });
    }

    None
}

/// Count how many times `needle` occurs in `haystack`, trying fuzzy matching
/// if exact fails.
pub fn count_matches(haystack: &str, needle: &str) -> (usize, MatchMethod) {
    let exact = haystack.matches(needle).count();
    if exact > 0 {
        return (exact, MatchMethod::Exact);
    }

    let needle_stripped = strip_trailing_whitespace(needle);
    let haystack_stripped = strip_trailing_whitespace(haystack);
    let stripped_count = haystack_stripped.matches(&needle_stripped).count();
    if stripped_count > 0 {
        return (stripped_count, MatchMethod::TrailingWhitespace);
    }

    let needle_normalized = normalize_quotes(needle);
    let haystack_normalized = normalize_quotes(haystack);
    let normalized_count = haystack_normalized.matches(&needle_normalized).count();
    if normalized_count > 0 {
        return (normalized_count, MatchMethod::QuoteNormalization);
    }

    let needle_both = normalize_quotes(&needle_stripped);
    let haystack_both = normalize_quotes(&haystack_stripped);
    let both_count = haystack_both.matches(&needle_both).count();
    if both_count > 0 {
        return (both_count, MatchMethod::Both);
    }

    (0, MatchMethod::Exact)
}

/// Strip trailing whitespace from each line while preserving line endings.
fn strip_trailing_whitespace(s: &str) -> String {
    s.lines()
        .collect::<Vec<_>>()
        .join("\n")
        .lines()
        .map(|line| line.trim_end())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Normalize curly/smart quotes to their ASCII equivalents.
pub fn normalize_quotes(s: &str) -> String {
    s.replace(['\u{2018}', '\u{2019}', '\u{2032}'], "'") // left/right single + prime
        .replace(['\u{201C}', '\u{201D}', '\u{2033}'], "\"") // left/right double + double prime
}

/// Given a normalized haystack where a needle was found, map back to the
/// original (un-normalized) haystack to extract the actual span.
///
/// This works by tracking line offsets: normalization only affects trailing
/// whitespace, so line starts are stable.
fn find_original_span(
    original: &str,
    normalized_haystack: &str,
    normalized_needle: &str,
) -> Option<String> {
    let idx = normalized_haystack.find(normalized_needle)?;

    // Map normalized byte offset → original byte offset via line tracking.
    // Both strings have the same number of lines, and normalization only removes
    // trailing whitespace, so we can map line-by-line.
    let norm_lines: Vec<&str> = normalized_haystack.lines().collect();
    let orig_lines: Vec<&str> = original.lines().collect();

    if norm_lines.len() != orig_lines.len() {
        return None;
    }

    // Find which line and column the normalized index falls on
    let mut norm_offset = 0;
    let mut start_line = 0;
    let mut start_col = 0;
    for (i, line) in norm_lines.iter().enumerate() {
        let line_end = norm_offset + line.len();
        if idx >= norm_offset && idx <= line_end {
            start_line = i;
            start_col = idx - norm_offset;
            break;
        }
        norm_offset = line_end + 1; // +1 for \n
    }

    let end_idx = idx + normalized_needle.len();
    let mut end_line = start_line;
    let mut end_col = 0;
    norm_offset = 0;
    for (i, line) in norm_lines.iter().enumerate() {
        let line_end = norm_offset + line.len();
        if end_idx >= norm_offset && end_idx <= line_end {
            end_line = i;
            end_col = end_idx - norm_offset;
            break;
        }
        norm_offset = line_end + 1;
    }

    // Now extract from original using line/col positions
    let mut result = String::new();
    for i in start_line..=end_line {
        if i >= orig_lines.len() {
            return None;
        }
        let line = orig_lines[i];
        let from = if i == start_line { start_col } else { 0 };
        let to = if i == end_line {
            end_col.min(line.len())
        } else {
            line.len()
        };
        if from > line.len() || to > line.len() {
            return None;
        }
        result.push_str(&line[from..to]);
        if i < end_line {
            result.push('\n');
        }
    }

    Some(result)
}

// ── Encoding detection ───────────────────────────────────────────────

/// Detected file encoding.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FileEncoding {
    Utf8,
    Utf16Le,
}

/// Detected line ending style.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LineEnding {
    Lf,
    CrLf,
    Cr,
}

impl LineEnding {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Lf => "\n",
            Self::CrLf => "\r\n",
            Self::Cr => "\r",
        }
    }
}

/// Detect encoding from raw bytes (checks for UTF-16LE BOM).
pub fn detect_encoding(bytes: &[u8]) -> FileEncoding {
    if bytes.len() >= 2 && bytes[0] == 0xFF && bytes[1] == 0xFE {
        FileEncoding::Utf16Le
    } else {
        FileEncoding::Utf8
    }
}

/// Detect the dominant line ending style in a string.
pub fn detect_line_ending(content: &str) -> LineEnding {
    let crlf = content.matches("\r\n").count();
    let cr_only = content.matches('\r').count().saturating_sub(crlf);
    let lf_only = content.matches('\n').count().saturating_sub(crlf);

    if crlf >= lf_only && crlf >= cr_only {
        if crlf == 0 {
            LineEnding::Lf // default
        } else {
            LineEnding::CrLf
        }
    } else if cr_only > lf_only {
        LineEnding::Cr
    } else {
        LineEnding::Lf
    }
}

/// Read a file as a String, handling encoding detection.
/// Returns (content with LF-normalized line endings, original encoding, original line ending).
pub async fn read_file_with_encoding(
    path: &Path,
) -> Result<(String, FileEncoding, LineEnding), ToolError> {
    let bytes = tokio::fs::read(path)
        .await
        .map_err(|e| ToolError::ExecutionFailed(format!("Failed to read file: {}", e)))?;

    let encoding = detect_encoding(&bytes);

    let raw_content = match encoding {
        FileEncoding::Utf8 => String::from_utf8(bytes)
            .map_err(|e| ToolError::ExecutionFailed(format!("File is not valid UTF-8: {}", e)))?,
        FileEncoding::Utf16Le => {
            // Skip BOM (2 bytes), decode as UTF-16LE
            let data = if bytes.len() >= 2 {
                &bytes[2..]
            } else {
                &bytes
            };
            let u16s: Vec<u16> = data
                .chunks_exact(2)
                .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                .collect();
            String::from_utf16(&u16s).map_err(|e| {
                ToolError::ExecutionFailed(format!("File is not valid UTF-16LE: {}", e))
            })?
        }
    };

    let line_ending = detect_line_ending(&raw_content);
    // Normalize to LF internally
    let content = raw_content.replace("\r\n", "\n").replace('\r', "\n");

    Ok((content, encoding, line_ending))
}

/// Write content back to a file, converting from LF to the original line ending
/// and encoding.
pub async fn write_file_with_encoding(
    path: &Path,
    content: &str,
    encoding: FileEncoding,
    line_ending: LineEnding,
) -> Result<(), ToolError> {
    // Convert LF back to original line ending
    let output = match line_ending {
        LineEnding::Lf => content.to_string(),
        LineEnding::CrLf => content.replace('\n', "\r\n"),
        LineEnding::Cr => content.replace('\n', "\r"),
    };

    let bytes = match encoding {
        FileEncoding::Utf8 => output.into_bytes(),
        FileEncoding::Utf16Le => {
            let u16s: Vec<u16> = output.encode_utf16().collect();
            // Write BOM + content
            let mut bytes = vec![0xFF, 0xFE];
            for u in u16s {
                bytes.extend_from_slice(&u.to_le_bytes());
            }
            bytes
        }
    };

    tokio::fs::write(path, &bytes)
        .await
        .map_err(|e| ToolError::ExecutionFailed(format!("Failed to write file: {}", e)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    // ── ReadFileState tests ──────────────────────────────────────

    fn test_job_id() -> Uuid {
        Uuid::new_v4()
    }

    #[test]
    fn test_read_state_unread_file_rejected() {
        let state = ReadFileState::new();
        let job = test_job_id();
        let path = Path::new("/tmp/test.rs");
        let err = state
            .check_before_edit(job, path, SystemTime::now())
            .unwrap_err();
        assert!(err.to_string().contains("has not been read yet"));
    }

    #[test]
    fn test_read_state_fresh_file_allowed() {
        let mut state = ReadFileState::new();
        let job = test_job_id();
        let path = Path::new("/tmp/test.rs");
        let now = SystemTime::now();
        state.record_read(job, path, now, false);
        assert!(state.check_before_edit(job, path, now).is_ok());
    }

    #[test]
    fn test_read_state_stale_file_rejected() {
        let mut state = ReadFileState::new();
        let job = test_job_id();
        let path = Path::new("/tmp/test.rs");
        let read_time = SystemTime::now();
        state.record_read(job, path, read_time, false);
        let stale_time = read_time + Duration::from_secs(5);
        let err = state.check_before_edit(job, path, stale_time).unwrap_err();
        assert!(err.to_string().contains("has been modified since"));
    }

    #[test]
    fn test_read_state_partial_read_rejected() {
        let mut state = ReadFileState::new();
        let job = test_job_id();
        let path = Path::new("/tmp/test.rs");
        let now = SystemTime::now();
        state.record_read(job, path, now, true);
        let err = state.check_before_edit(job, path, now).unwrap_err();
        assert!(err.to_string().contains("partial view"));
    }

    #[test]
    fn test_read_state_mtime_updated_after_write() {
        let mut state = ReadFileState::new();
        let job = test_job_id();
        let path = Path::new("/tmp/test.rs");
        let t1 = SystemTime::now();
        state.record_read(job, path, t1, false);
        let t2 = t1 + Duration::from_secs(3);
        state.update_mtime(job, path, t2);
        // Now the file at t2 should be considered fresh
        assert!(state.check_before_edit(job, path, t2).is_ok());
    }

    #[test]
    fn test_read_state_isolated_across_jobs() {
        let mut state = ReadFileState::new();
        let job_a = test_job_id();
        let job_b = test_job_id();
        let path = Path::new("/tmp/test.rs");
        let now = SystemTime::now();
        // Job A reads the file
        state.record_read(job_a, path, now, false);
        // Job B should NOT be able to edit (hasn't read)
        let err = state.check_before_edit(job_b, path, now).unwrap_err();
        assert!(err.to_string().contains("has not been read yet"));
        // Job A can still edit
        assert!(state.check_before_edit(job_a, path, now).is_ok());
    }

    // ── Fuzzy matching tests ─────────────────────────────────────

    #[test]
    fn test_exact_match() {
        let m = find_match("fn main() {}", "fn main()").unwrap();
        assert_eq!(m.method, MatchMethod::Exact);
        assert_eq!(m.actual, "fn main()");
    }

    #[test]
    fn test_trailing_whitespace_match() {
        let file = "fn main() {  \n    body  \n}";
        let needle = "fn main() {\n    body\n}";
        let m = find_match(file, needle).unwrap();
        assert_eq!(m.method, MatchMethod::TrailingWhitespace);
    }

    #[test]
    fn test_quote_normalization_match() {
        let file = "let msg = \u{201C}hello\u{201D};";
        let needle = "let msg = \"hello\";";
        let m = find_match(file, needle).unwrap();
        assert_eq!(m.method, MatchMethod::QuoteNormalization);
    }

    #[test]
    fn test_no_match() {
        assert!(find_match("fn main() {}", "fn other()").is_none());
    }

    #[test]
    fn test_count_matches_exact() {
        let (count, method) = count_matches("aaa", "a");
        assert_eq!(count, 3);
        assert_eq!(method, MatchMethod::Exact);
    }

    #[test]
    fn test_count_matches_whitespace() {
        let file = "a  \na  \n";
        let needle = "a\na\n";
        let (count, method) = count_matches(file, needle);
        assert_eq!(count, 1);
        assert_eq!(method, MatchMethod::TrailingWhitespace);
    }

    // ── Encoding tests ───────────────────────────────────────────

    #[test]
    fn test_detect_utf8() {
        assert_eq!(detect_encoding(b"hello"), FileEncoding::Utf8);
    }

    #[test]
    fn test_detect_utf16le_bom() {
        assert_eq!(
            detect_encoding(&[0xFF, 0xFE, 0x41, 0x00]),
            FileEncoding::Utf16Le
        );
    }

    #[test]
    fn test_detect_line_ending_lf() {
        assert_eq!(detect_line_ending("a\nb\nc"), LineEnding::Lf);
    }

    #[test]
    fn test_detect_line_ending_crlf() {
        assert_eq!(detect_line_ending("a\r\nb\r\nc"), LineEnding::CrLf);
    }

    #[test]
    fn test_detect_line_ending_empty() {
        assert_eq!(detect_line_ending("no newlines"), LineEnding::Lf);
    }

    #[test]
    fn test_normalize_quotes() {
        assert_eq!(normalize_quotes("\u{201C}hello\u{201D}"), "\"hello\"");
        assert_eq!(normalize_quotes("it\u{2019}s"), "it's");
    }

    #[tokio::test]
    async fn test_read_write_utf8_roundtrip() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("test.txt");
        std::fs::write(&path, "line1\nline2\n").unwrap();

        let (content, enc, le) = read_file_with_encoding(&path).await.unwrap();
        assert_eq!(enc, FileEncoding::Utf8);
        assert_eq!(le, LineEnding::Lf);
        assert_eq!(content, "line1\nline2\n");

        write_file_with_encoding(&path, &content, enc, le)
            .await
            .unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "line1\nline2\n");
    }

    #[tokio::test]
    async fn test_read_write_crlf_roundtrip() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("test.txt");
        std::fs::write(&path, "line1\r\nline2\r\n").unwrap();

        let (content, enc, le) = read_file_with_encoding(&path).await.unwrap();
        assert_eq!(enc, FileEncoding::Utf8);
        assert_eq!(le, LineEnding::CrLf);
        assert_eq!(content, "line1\nline2\n"); // normalized to LF

        write_file_with_encoding(&path, &content, enc, le)
            .await
            .unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"line1\r\nline2\r\n");
    }

    #[tokio::test]
    async fn test_read_write_utf16le_roundtrip() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("test.txt");
        // Write UTF-16LE with BOM
        let text = "hello";
        let u16s: Vec<u16> = text.encode_utf16().collect();
        let mut bytes = vec![0xFF, 0xFE];
        for u in u16s {
            bytes.extend_from_slice(&u.to_le_bytes());
        }
        std::fs::write(&path, &bytes).unwrap();

        let (content, enc, _le) = read_file_with_encoding(&path).await.unwrap();
        assert_eq!(enc, FileEncoding::Utf16Le);
        assert_eq!(content, "hello");

        write_file_with_encoding(&path, &content, enc, LineEnding::Lf)
            .await
            .unwrap();
        let written = std::fs::read(&path).unwrap();
        assert_eq!(written[0..2], [0xFF, 0xFE]); // BOM preserved
    }
}
