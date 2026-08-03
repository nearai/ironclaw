use std::sync::LazyLock;

use ironclaw_safety::LeakDetector;

use crate::config::MAX_LOG_MESSAGE_BYTES;
use crate::store::truncate_log_message;

pub(crate) const WASM_DIAGNOSTIC_REDACTED: &str = "[WASM_DIAGNOSTIC_REDACTED]";

static LEAK_DETECTOR: LazyLock<LeakDetector> = LazyLock::new(LeakDetector::new);

pub(crate) fn sanitize_wasm_diagnostic(message: String) -> String {
    if message == WASM_DIAGNOSTIC_REDACTED {
        return message;
    }
    if message.len() > MAX_LOG_MESSAGE_BYTES {
        return WASM_DIAGNOSTIC_REDACTED.to_string();
    }

    let (redacted, _) = LEAK_DETECTOR.redact_all_secrets(&message);
    truncate_log_message(redacted)
}
