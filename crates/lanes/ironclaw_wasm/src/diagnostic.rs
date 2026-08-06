use std::sync::LazyLock;

use ironclaw_safety::LeakDetector;

use crate::config::{WASM_DIAGNOSTIC_MAX_BYTES, WASM_DIAGNOSTIC_REDACTION_MARKER};

const LEAK_DETECTOR_REDACTION_MARKER: &str = "[REDACTED]";

static LEAK_DETECTOR: LazyLock<LeakDetector> = LazyLock::new(LeakDetector::new);

/// Sanitizes one untrusted WASM diagnostic for public results and tracing.
///
/// Inputs larger than [`WASM_DIAGNOSTIC_MAX_BYTES`] fail closed to
/// [`WASM_DIAGNOSTIC_REDACTION_MARKER`]. Accepted inputs are scanned for every
/// secret class recognized by [`LeakDetector`], whose replacements are
/// normalized to the same stable WASM marker; every control character
/// (including `\0`, CR/LF, and ANSI/terminal escape bytes) is then mapped to
/// a space so a WASM guest cannot forge log lines or terminal escapes at the
/// tracing sink, before the result is bounded at a valid UTF-8 boundary.
pub fn sanitize_wasm_diagnostic(message: impl AsRef<str>) -> String {
    let message = message.as_ref();
    if message == WASM_DIAGNOSTIC_REDACTION_MARKER {
        return message.to_string();
    }
    if message.len() > WASM_DIAGNOSTIC_MAX_BYTES {
        return WASM_DIAGNOSTIC_REDACTION_MARKER.to_string();
    }

    let (redacted, changed) = LEAK_DETECTOR.redact_all_secrets(message);
    let normalized = if changed {
        redacted.replace(
            LEAK_DETECTOR_REDACTION_MARKER,
            WASM_DIAGNOSTIC_REDACTION_MARKER,
        )
    } else {
        redacted
    };
    let stripped = strip_control_characters(normalized);
    if stripped.len() > WASM_DIAGNOSTIC_MAX_BYTES {
        return WASM_DIAGNOSTIC_REDACTION_MARKER.to_string();
    }
    truncate_log_message(stripped)
}

/// Maps every control character (including `\0`) to a space, mirroring
/// `sanitize_untrusted_text_body`'s approach in
/// `ironclaw_turn_runner::subagent::untrusted_text` for neutralizing
/// terminal/log injection from untrusted text before it reaches a sink.
///
/// Runs after secret redaction (so `LeakDetector`'s regexes see the original,
/// unmodified guest text) and before truncation (so the final byte-boundary
/// cut in [`truncate_log_message`] operates on the fully sanitized string).
fn strip_control_characters(message: String) -> String {
    message
        .chars()
        .map(|character| match character {
            character if character == '\0' || character.is_control() => ' ',
            character => character,
        })
        .collect()
}

fn truncate_log_message(message: String) -> String {
    if message.len() <= WASM_DIAGNOSTIC_MAX_BYTES {
        return message;
    }

    let mut end = WASM_DIAGNOSTIC_MAX_BYTES;
    while !message.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    message[..end].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const DETECTABLE_SECRET: &str = "AKIAIOSFODNN7EXAMPLE";

    #[test]
    fn benign_diagnostic_is_preserved() {
        assert_eq!(
            sanitize_wasm_diagnostic("benign guest detail"),
            "benign guest detail"
        );
    }

    #[test]
    fn detected_secret_uses_stable_wasm_marker() {
        assert_eq!(
            sanitize_wasm_diagnostic(format!(
                "request failed for {DETECTABLE_SECRET}; status=503"
            )),
            format!("request failed for {WASM_DIAGNOSTIC_REDACTION_MARKER}; status=503")
        );
    }

    #[test]
    fn exact_byte_boundary_is_preserved() {
        let at_limit = "é".repeat(WASM_DIAGNOSTIC_MAX_BYTES / "é".len());

        assert_eq!(at_limit.len(), WASM_DIAGNOSTIC_MAX_BYTES);
        assert_eq!(sanitize_wasm_diagnostic(&at_limit), at_limit);
    }

    #[test]
    fn oversize_diagnostic_fails_closed() {
        let oversize = "x".repeat(WASM_DIAGNOSTIC_MAX_BYTES + 1);

        assert_eq!(
            sanitize_wasm_diagnostic(oversize),
            WASM_DIAGNOSTIC_REDACTION_MARKER
        );
    }

    /// Regression test: a detectable secret sitting at the end of an
    /// otherwise limit-sized input passes the initial raw-byte-limit check,
    /// but swapping the secret for the (longer) `WASM_DIAGNOSTIC_REDACTION_MARKER`
    /// pushes the normalized string past `WASM_DIAGNOSTIC_MAX_BYTES`. Before the
    /// post-strip length check was added, `truncate_log_message` would cut
    /// straight through the marker text, leaving a mangled fragment instead
    /// of the complete, stable marker.
    #[test]
    fn secret_redaction_growth_past_limit_returns_whole_marker() {
        let filler = "x".repeat(WASM_DIAGNOSTIC_MAX_BYTES - DETECTABLE_SECRET.len());
        let at_limit_with_trailing_secret = format!("{filler}{DETECTABLE_SECRET}");
        assert_eq!(at_limit_with_trailing_secret.len(), WASM_DIAGNOSTIC_MAX_BYTES);

        let sanitized = sanitize_wasm_diagnostic(&at_limit_with_trailing_secret);

        assert_eq!(sanitized, WASM_DIAGNOSTIC_REDACTION_MARKER);
    }

    #[test]
    fn truncate_log_message_respects_utf8_boundaries() {
        let message = format!("{}é", "x".repeat(WASM_DIAGNOSTIC_MAX_BYTES - 1));
        let truncated = truncate_log_message(message);
        assert_eq!(truncated.len(), WASM_DIAGNOSTIC_MAX_BYTES - 1);
        assert_eq!(truncated, "x".repeat(WASM_DIAGNOSTIC_MAX_BYTES - 1));
    }

    /// Regression test for a log/terminal-injection PoC: a guest diagnostic
    /// with no detectable secret pattern that embeds a newline, a forged
    /// host `ERROR` log line, and ANSI escape sequences. Before control
    /// characters were stripped, this text passed through
    /// `sanitize_wasm_diagnostic` unchanged and, once traced at debug level
    /// with `%message` (Display, not Debug), could forge a fake host error
    /// line and clear the terminal at any stderr sink.
    #[test]
    fn embedded_fake_log_line_and_ansi_escapes_are_neutralized() {
        let payload = "benign output\n2026-08-05T00:00:00.000000Z ERROR ironclaw_host_runtime: fake alert: host credentials exfiltrated\x1b[2J\x1b[H";

        let sanitized = sanitize_wasm_diagnostic(payload);

        assert!(
            !sanitized.chars().any(|character| character.is_control()),
            "sanitized diagnostic must contain no raw control characters: {sanitized:?}"
        );
        assert!(!sanitized.contains('\n'));
        assert!(!sanitized.contains('\r'));
        assert!(!sanitized.contains('\x1b'));
    }
}
