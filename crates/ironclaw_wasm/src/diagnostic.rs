use std::sync::LazyLock;

use ironclaw_safety::LeakDetector;

use crate::config::{WASM_DIAGNOSTIC_MAX_BYTES, WASM_DIAGNOSTIC_REDACTION_MARKER};
use crate::store::truncate_log_message;

static LEAK_DETECTOR: LazyLock<LeakDetector> = LazyLock::new(LeakDetector::new);

pub(crate) fn sanitize_wasm_diagnostic(message: String) -> String {
    if message == WASM_DIAGNOSTIC_REDACTION_MARKER {
        return message;
    }
    if message.len() > WASM_DIAGNOSTIC_MAX_BYTES {
        return WASM_DIAGNOSTIC_REDACTION_MARKER.to_string();
    }

    let (redacted, _) = LEAK_DETECTOR.redact_all_secrets(&message);
    truncate_log_message(redacted)
}
