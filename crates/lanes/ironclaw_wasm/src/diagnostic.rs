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
/// normalized to the same stable WASM marker before the result is bounded at a
/// valid UTF-8 boundary.
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
    truncate_log_message(normalized)
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

    #[test]
    fn truncate_log_message_respects_utf8_boundaries() {
        let message = format!("{}é", "x".repeat(WASM_DIAGNOSTIC_MAX_BYTES - 1));
        let truncated = truncate_log_message(message);
        assert_eq!(truncated.len(), WASM_DIAGNOSTIC_MAX_BYTES - 1);
        assert_eq!(truncated, "x".repeat(WASM_DIAGNOSTIC_MAX_BYTES - 1));
    }
}
