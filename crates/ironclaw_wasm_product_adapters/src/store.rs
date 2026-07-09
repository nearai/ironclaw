use std::time::Duration;

use ironclaw_wasm::wasm_sandbox_core::{
    MinimalWasiCtxView, MinimalWasiView, SandboxStoreCore, SandboxStoreData, WasmResourceLimiter,
};

use crate::bindings;
use crate::config::{MAX_LOG_MESSAGE_BYTES, MAX_LOGS_PER_EXECUTION};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComponentLogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComponentLogRecord {
    pub level: ComponentLogLevel,
    pub message: String,
}

pub(crate) struct StoreData {
    sandbox: SandboxStoreCore,
    pub(crate) logs: Vec<ComponentLogRecord>,
}

impl StoreData {
    pub(crate) fn new(memory_limit: u64, timeout: Duration) -> Self {
        Self {
            sandbox: SandboxStoreCore::new(memory_limit, timeout),
            logs: Vec::new(),
        }
    }

    pub(crate) fn deadline_exceeded(&self) -> bool {
        self.sandbox.deadline_exceeded()
    }

    pub(crate) fn limiter_mut(&mut self) -> &mut WasmResourceLimiter {
        self.sandbox.limiter_mut()
    }
}

impl SandboxStoreData for StoreData {
    fn sandbox_limiter(&mut self) -> &mut WasmResourceLimiter {
        self.limiter_mut()
    }
}

impl MinimalWasiView for StoreData {
    fn ctx(&mut self) -> MinimalWasiCtxView<'_> {
        self.sandbox.ctx()
    }
}

impl bindings::near::product_adapter::product_adapter_host::Host for StoreData {
    fn log(
        &mut self,
        level: bindings::near::product_adapter::product_adapter_host::LogLevel,
        message: String,
    ) {
        if self.logs.len() >= MAX_LOGS_PER_EXECUTION {
            return;
        }
        let level = match level {
            bindings::near::product_adapter::product_adapter_host::LogLevel::Trace => {
                ComponentLogLevel::Trace
            }
            bindings::near::product_adapter::product_adapter_host::LogLevel::Debug => {
                ComponentLogLevel::Debug
            }
            bindings::near::product_adapter::product_adapter_host::LogLevel::Info => {
                ComponentLogLevel::Info
            }
            bindings::near::product_adapter::product_adapter_host::LogLevel::Warn => {
                ComponentLogLevel::Warn
            }
            bindings::near::product_adapter::product_adapter_host::LogLevel::Error => {
                ComponentLogLevel::Error
            }
        };
        self.logs.push(ComponentLogRecord {
            level,
            message: truncate_log_message(message),
        });
    }

    fn now_millis(&mut self) -> u64 {
        // Wall-clock UTC milliseconds since the Unix epoch. NOT monotonic:
        // NTP corrections, leap-second smearing, and operator clock changes
        // can push this value backwards. Adapter authors MUST NOT build
        // TTLs, idempotency windows, or token-validity comparisons on the
        // assumption that successive calls are non-decreasing; use the
        // values for logging/timestamps only.
        u64::try_from(chrono::Utc::now().timestamp_millis()).unwrap_or(0)
    }

    fn http_egress(
        &mut self,
        _request: bindings::near::product_adapter::product_adapter_host::EgressRequest,
    ) -> Result<
        bindings::near::product_adapter::product_adapter_host::EgressResponse,
        bindings::near::product_adapter::product_adapter_host::EgressError,
    > {
        Err(bindings::near::product_adapter::product_adapter_host::EgressError {
            kind: bindings::near::product_adapter::product_adapter_host::EgressErrorKind::PolicyDenied,
            message: "host http egress is not wired for ProductAdapter components yet".to_string(),
        })
    }
}

fn truncate_log_message(mut message: String) -> String {
    if message.len() <= MAX_LOG_MESSAGE_BYTES {
        return message;
    }

    let mut end = MAX_LOG_MESSAGE_BYTES;
    while !message.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    message.truncate(end);
    message
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crate::bindings::near::product_adapter::product_adapter_host::{Host, LogLevel};
    use crate::config::MAX_LOGS_PER_EXECUTION;

    use super::{MAX_LOG_MESSAGE_BYTES, StoreData, truncate_log_message};

    #[test]
    fn truncate_log_message_respects_utf8_boundaries() {
        let message = "é".repeat(MAX_LOG_MESSAGE_BYTES);
        let truncated = truncate_log_message(message);
        assert!(truncated.len() <= MAX_LOG_MESSAGE_BYTES);
        assert!(truncated.is_char_boundary(truncated.len()));
    }

    #[test]
    fn host_log_import_caps_records_at_runtime_boundary() {
        let mut data = StoreData::new(1024 * 1024, Duration::from_secs(1));

        for index in 0..=MAX_LOGS_PER_EXECUTION {
            Host::log(&mut data, LogLevel::Info, format!("log-{index}"));
        }

        assert_eq!(data.logs.len(), MAX_LOGS_PER_EXECUTION);
        assert_eq!(
            data.logs.last().expect("last retained log").message,
            format!("log-{}", MAX_LOGS_PER_EXECUTION - 1)
        );
    }

    #[test]
    fn host_log_import_truncates_messages_at_runtime_boundary() {
        let mut data = StoreData::new(1024 * 1024, Duration::from_secs(1));

        Host::log(&mut data, LogLevel::Warn, "é".repeat(MAX_LOG_MESSAGE_BYTES));

        let message = &data.logs[0].message;
        assert!(message.len() <= MAX_LOG_MESSAGE_BYTES);
        assert!(message.is_char_boundary(message.len()));
    }
}
