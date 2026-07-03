use ironclaw_host_api::CapabilityId;
use ironclaw_wasm::{WasmError, WasmLogLevel, WasmLogRecord};

pub(super) fn log_wasm_runtime_error(capability_id: &CapabilityId, error: &WasmError) {
    if let WasmError::ExecutionFailed { message, logs, .. } = error {
        log_wasm_guest_logs(capability_id, logs);
        tracing::debug!(
            capability_id = %capability_id,
            wasm_error = %message,
            "WASM runtime execution failed with raw guest error"
        );
        return;
    }

    tracing::debug!(
        capability_id = %capability_id,
        wasm_error = %error,
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
        wasm_error = %error,
        "WASM guest returned raw capability error"
    );
}

fn log_wasm_guest_logs(capability_id: &CapabilityId, logs: &[WasmLogRecord]) {
    for log in logs {
        match log.level {
            WasmLogLevel::Trace => tracing::trace!(
                capability_id = %capability_id,
                wasm_log = %log.message,
                "WASM guest log"
            ),
            WasmLogLevel::Debug => tracing::debug!(
                capability_id = %capability_id,
                wasm_log = %log.message,
                "WASM guest log"
            ),
            WasmLogLevel::Info => tracing::info!(
                capability_id = %capability_id,
                wasm_log = %log.message,
                "WASM guest log"
            ),
            WasmLogLevel::Warn => tracing::warn!(
                capability_id = %capability_id,
                wasm_log = %log.message,
                "WASM guest log"
            ),
            WasmLogLevel::Error => tracing::error!(
                capability_id = %capability_id,
                wasm_log = %log.message,
                "WASM guest log"
            ),
        }
    }
}
