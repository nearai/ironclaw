use std::{fs::OpenOptions, io::Write, path::PathBuf};

use tokio::io::AsyncReadExt;
use uuid::Uuid;

use crate::RuntimeProcessError;

/// Maximum captured output before middle truncation.
pub(crate) const COMMAND_MAX_OUTPUT_SIZE: usize = 64 * 1024;
const COMMAND_MAX_SAVED_STREAM_SIZE: usize = 16 * 1024 * 1024;
const SHELL_OUTPUT_TEMP_PREFIX: &str = "ironclaw-shell-output-";

/// Metadata for full process output persisted outside the model preview.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedCommandOutput {
    pub path: PathBuf,
    pub secret_redacted: bool,
    pub secret_blocked: bool,
    pub stream_was_capped: bool,
    pub max_saved_stream_size: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CapturedCommandOutput {
    pub(crate) preview: String,
    pub(crate) saved_output: Option<SavedCommandOutput>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct StreamCapture {
    pub(crate) output: String,
    pub(crate) was_capped: bool,
}

pub(crate) async fn read_stream_capped<R>(mut stream: R) -> StreamCapture
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut buf = Vec::new();
    (&mut stream)
        .take((COMMAND_MAX_SAVED_STREAM_SIZE + 1) as u64)
        .read_to_end(&mut buf)
        .await
        .ok();
    let was_capped = buf.len() > COMMAND_MAX_SAVED_STREAM_SIZE;
    if was_capped {
        buf.truncate(COMMAND_MAX_SAVED_STREAM_SIZE);
    }
    tokio::io::copy(&mut stream, &mut tokio::io::sink())
        .await
        .ok();
    StreamCapture {
        output: String::from_utf8_lossy(&buf).to_string(),
        was_capped,
    }
}

pub(crate) fn combine_streams(stdout: &str, stderr: &str) -> String {
    if stderr.is_empty() {
        stdout.to_string()
    } else if stdout.is_empty() {
        stderr.to_string()
    } else {
        format!("{stdout}\n\n--- stderr ---\n{stderr}")
    }
}

pub(crate) fn capture_command_output(
    output: &str,
    stream_was_capped: bool,
) -> Result<CapturedCommandOutput, RuntimeProcessError> {
    if output.len() <= COMMAND_MAX_OUTPUT_SIZE && !stream_was_capped {
        return Ok(CapturedCommandOutput {
            preview: output.to_string(),
            saved_output: None,
        });
    }

    let preview = truncate_output(output);
    let saved_output = persist_shell_output(output, stream_was_capped)?;
    Ok(CapturedCommandOutput {
        preview,
        saved_output: Some(saved_output),
    })
}

fn persist_shell_output(
    output: &str,
    stream_was_capped: bool,
) -> Result<SavedCommandOutput, RuntimeProcessError> {
    let sanitized = sanitize_saved_shell_output(output);
    let path =
        std::env::temp_dir().join(format!("{SHELL_OUTPUT_TEMP_PREFIX}{}.log", Uuid::new_v4()));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&path).map_err(|e| {
        RuntimeProcessError::ExecutionFailed(format!("Failed to create shell output file: {e}"))
    })?;
    file.write_all(sanitized.output.as_bytes()).map_err(|e| {
        RuntimeProcessError::ExecutionFailed(format!("Failed to write shell output file: {e}"))
    })?;
    Ok(SavedCommandOutput {
        path,
        secret_redacted: sanitized.was_redacted,
        secret_blocked: sanitized.was_blocked,
        stream_was_capped,
        max_saved_stream_size: COMMAND_MAX_SAVED_STREAM_SIZE,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SanitizedSavedOutput {
    output: String,
    was_redacted: bool,
    was_blocked: bool,
}

fn sanitize_saved_shell_output(output: &str) -> SanitizedSavedOutput {
    let detector = ironclaw_safety::LeakDetector::new();
    match detector.scan_and_clean(output) {
        Ok(cleaned) => SanitizedSavedOutput {
            was_redacted: cleaned != output,
            was_blocked: false,
            output: cleaned,
        },
        Err(_) => SanitizedSavedOutput {
            output: "[Full shell output blocked due to potential secret leakage]\n".to_string(),
            was_redacted: false,
            was_blocked: true,
        },
    }
}

pub(crate) fn truncate_output(s: &str) -> String {
    if s.len() <= COMMAND_MAX_OUTPUT_SIZE {
        s.to_string()
    } else {
        let half = COMMAND_MAX_OUTPUT_SIZE / 2;
        let head_end = floor_char_boundary(s, half);
        let tail_start = floor_char_boundary(s, s.len() - half);
        format!(
            "{}\n\n... [truncated {} bytes] ...\n\n{}",
            &s[..head_end],
            s.len() - COMMAND_MAX_OUTPUT_SIZE,
            &s[tail_start..]
        )
    }
}

fn floor_char_boundary(s: &str, pos: usize) -> usize {
    if pos >= s.len() {
        return s.len();
    }
    let mut i = pos;
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_output_preserves_exact_limit() {
        let output = "x".repeat(COMMAND_MAX_OUTPUT_SIZE);

        assert_eq!(truncate_output(&output), output);
    }

    #[test]
    fn truncate_output_respects_utf8_boundaries() {
        let output = format!(
            "{}{}{}",
            "a".repeat(COMMAND_MAX_OUTPUT_SIZE / 2 - 1),
            "é",
            "b".repeat(COMMAND_MAX_OUTPUT_SIZE)
        );

        let truncated = truncate_output(&output);

        assert!(truncated.is_char_boundary(COMMAND_MAX_OUTPUT_SIZE / 2 - 1));
        assert!(truncated.contains("... [truncated "));
        assert!(truncated.starts_with(&"a".repeat(COMMAND_MAX_OUTPUT_SIZE / 2 - 1)));
        assert!(truncated.ends_with(&"b".repeat(COMMAND_MAX_OUTPUT_SIZE / 2)));
    }

    #[tokio::test]
    async fn read_stream_capped_keeps_output_beyond_preview_limit() {
        let input = "x".repeat(COMMAND_MAX_OUTPUT_SIZE + 1);

        let output = read_stream_capped(input.as_bytes()).await;

        assert_eq!(output.output.len(), COMMAND_MAX_OUTPUT_SIZE + 1);
        assert!(!output.was_capped);
    }

    #[test]
    fn capture_command_output_preserves_small_output() {
        let output = capture_command_output("small output", false).expect("capture succeeds");

        assert_eq!(output.preview, "small output");
        assert_eq!(output.saved_output, None);
    }

    #[test]
    fn capture_command_output_saves_large_output_file() {
        let middle = "middle content that current preview would omit";
        let raw = format!(
            "{}{middle}{}",
            "a".repeat(COMMAND_MAX_OUTPUT_SIZE),
            "z".repeat(COMMAND_MAX_OUTPUT_SIZE)
        );

        let output = capture_command_output(&raw, false).expect("capture succeeds");
        let saved_output = output.saved_output.expect("saved output metadata");
        let saved = std::fs::read_to_string(&saved_output.path).expect("saved output readable");
        let _ = std::fs::remove_file(&saved_output.path);

        assert!(output.preview.contains("... [truncated "));
        assert!(!output.preview.contains(middle));
        assert!(!saved_output.secret_blocked);
        assert!(!saved_output.secret_redacted);
        assert!(!saved_output.stream_was_capped);
        assert_eq!(saved, raw);
    }

    #[test]
    fn capture_command_output_blocks_secret_like_saved_output() {
        let secret = "sk-proj-test1234567890abcdefghij";
        let raw = format!("{}{}", "x".repeat(COMMAND_MAX_OUTPUT_SIZE + 1), secret);

        let output = capture_command_output(&raw, false).expect("capture succeeds");
        let saved_output = output.saved_output.expect("saved output metadata");
        let saved = std::fs::read_to_string(&saved_output.path).expect("saved output readable");
        let _ = std::fs::remove_file(&saved_output.path);

        assert!(saved_output.secret_blocked);
        assert_eq!(
            saved,
            "[Full shell output blocked due to potential secret leakage]\n"
        );
        assert!(!saved.contains(secret));
    }

    #[cfg(unix)]
    #[test]
    fn persist_shell_output_uses_owner_only_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let persisted =
            persist_shell_output("large clean output", false).expect("persist succeeds");
        let mode = std::fs::metadata(&persisted.path)
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777;
        let _ = std::fs::remove_file(&persisted.path);

        assert_eq!(mode, 0o600);
    }
}
