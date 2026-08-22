use std::sync::LazyLock;

use ironclaw_host_api::ids::CapabilityId;
use ironclaw_host_api::{
    dispatch::truncate_capability_display_text, result_meta::MODEL_DIAGNOSTIC_MAX_BYTES,
};
use ironclaw_safety::LeakDetector;
use ironclaw_wasm::{WasmError, WasmLogLevel, WasmLogRecord};

static WASM_DIAGNOSTIC_LEAK_DETECTOR: LazyLock<LeakDetector> = LazyLock::new(LeakDetector::new);

pub(super) fn log_wasm_runtime_error(capability_id: &CapabilityId, error: &WasmError) {
    if let WasmError::ExecutionFailed { message, logs, .. } = error {
        log_wasm_guest_logs(capability_id, logs);
        tracing::debug!(
            capability_id = %capability_id,
            wasm_error = %sanitize_wasm_diagnostic(message),
            "WASM runtime execution failed with guest error"
        );
        return;
    }

    let wasm_error = sanitize_wasm_diagnostic(&error.to_string());
    tracing::debug!(
        capability_id = %capability_id,
        wasm_error = %wasm_error,
        "WASM runtime execution failed"
    );
}

pub(super) fn log_wasm_guest_error(
    capability_id: &CapabilityId,
    logs: &[WasmLogRecord],
    error: &str,
) {
    log_wasm_guest_logs(capability_id, logs);
    tracing::debug!(
        capability_id = %capability_id,
        wasm_error = %sanitize_wasm_diagnostic(error),
        "WASM guest returned capability error"
    );
}

fn log_wasm_guest_logs(capability_id: &CapabilityId, logs: &[WasmLogRecord]) {
    for log in logs {
        let message = sanitize_wasm_diagnostic(&log.message);
        match log.level {
            WasmLogLevel::Trace => tracing::trace!(
                capability_id = %capability_id,
                wasm_log = %message,
                "WASM guest log"
            ),
            WasmLogLevel::Debug => tracing::debug!(
                capability_id = %capability_id,
                wasm_log = %message,
                "WASM guest log"
            ),
            WasmLogLevel::Info => tracing::info!(
                capability_id = %capability_id,
                wasm_log = %message,
                "WASM guest log"
            ),
            WasmLogLevel::Warn => tracing::warn!(
                capability_id = %capability_id,
                wasm_log = %message,
                "WASM guest log"
            ),
            WasmLogLevel::Error => tracing::error!(
                capability_id = %capability_id,
                wasm_log = %message,
                "WASM guest log"
            ),
        }
    }
}

fn sanitize_wasm_diagnostic(value: &str) -> String {
    let (redacted, _) = WASM_DIAGNOSTIC_LEAK_DETECTOR.redact_all_secrets(value);
    truncate_capability_display_text(&redacted, MODEL_DIAGNOSTIC_MAX_BYTES).text
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracing_test::traced_test;

    const GITHUB_TOKEN_SHAPE: &str = "ghp_xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx";

    #[test]
    #[traced_test]
    fn guest_error_and_logs_are_redacted_before_host_tracing() {
        let capability_id = CapabilityId::new("github.search_issues").expect("capability id");
        let guest_logs = [WasmLogRecord {
            level: WasmLogLevel::Warn,
            message: format!("provider response echoed {GITHUB_TOKEN_SHAPE}"),
        }];

        log_wasm_guest_error(
            &capability_id,
            &guest_logs,
            &format!("auth failed with {GITHUB_TOKEN_SHAPE}"),
        );

        assert!(
            !logs_contain(GITHUB_TOKEN_SHAPE),
            "host diagnostics must not trace credential-shaped guest text"
        );
        assert!(logs_contain("WASM guest log"));
        assert!(logs_contain("WASM guest returned capability error"));
    }
}
