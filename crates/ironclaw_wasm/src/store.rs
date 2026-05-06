use std::time::{Duration, Instant};

use ironclaw_host_api::ResourceUsage;
use wasmtime_wasi::{ResourceTable, WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

use crate::bindings;
use crate::config::{DEFAULT_HTTP_TIMEOUT_MS, MAX_LOG_MESSAGE_BYTES, MAX_LOGS_PER_EXECUTION};
use crate::host::{WasmHttpRequest, WitToolHost};
use crate::limiter::WasmResourceLimiter;
use crate::types::{WasmLogLevel, WasmLogRecord};

pub(crate) struct StoreData {
    host: WitToolHost,
    pub(crate) limiter: WasmResourceLimiter,
    wasi: WasiCtx,
    table: ResourceTable,
    pub(crate) usage: ResourceUsage,
    pub(crate) logs: Vec<WasmLogRecord>,
    deadline: Option<Instant>,
}

impl StoreData {
    pub(crate) fn new(host: WitToolHost, memory_limit: u64, timeout: Duration) -> Self {
        Self {
            host,
            limiter: WasmResourceLimiter::new(memory_limit),
            wasi: WasiCtxBuilder::new().build(),
            table: ResourceTable::new(),
            usage: ResourceUsage::default(),
            logs: Vec::new(),
            deadline: Instant::now().checked_add(timeout),
        }
    }

    pub(crate) fn deadline_exceeded(&self) -> bool {
        self.deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
    }

    fn deadline_error(&self) -> Option<String> {
        self.deadline_exceeded()
            .then(|| "WASM execution deadline exceeded during host import".to_string())
    }

    fn remaining_timeout_ms(&self, requested_timeout_ms: Option<u32>) -> Option<u32> {
        let requested_timeout_ms = requested_timeout_ms.unwrap_or(DEFAULT_HTTP_TIMEOUT_MS);
        let deadline_timeout_ms = self.deadline.map(|deadline| {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let remaining_ms = remaining.as_millis();
            if remaining_ms == 0 {
                1
            } else {
                remaining_ms.min(u128::from(u32::MAX)) as u32
            }
        });

        Some(match deadline_timeout_ms {
            Some(deadline) => requested_timeout_ms.min(deadline),
            None => requested_timeout_ms,
        })
    }

    fn record_network_egress(&mut self, request_body_bytes: u64) {
        self.usage.network_egress_bytes = self
            .usage
            .network_egress_bytes
            .saturating_add(request_body_bytes);
    }
}

impl WasiView for StoreData {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

impl bindings::near::agent::host::Host for StoreData {
    fn log(&mut self, level: bindings::near::agent::host::LogLevel, message: String) {
        if self.logs.len() >= MAX_LOGS_PER_EXECUTION {
            return;
        }
        let message = truncate_log_message(message);
        let level = match level {
            bindings::near::agent::host::LogLevel::Trace => WasmLogLevel::Trace,
            bindings::near::agent::host::LogLevel::Debug => WasmLogLevel::Debug,
            bindings::near::agent::host::LogLevel::Info => WasmLogLevel::Info,
            bindings::near::agent::host::LogLevel::Warn => WasmLogLevel::Warn,
            bindings::near::agent::host::LogLevel::Error => WasmLogLevel::Error,
        };
        self.logs.push(WasmLogRecord { level, message });
    }

    fn now_millis(&mut self) -> u64 {
        self.host.clock.now_millis()
    }

    fn workspace_read(&mut self, path: String) -> Option<String> {
        if self.deadline_exceeded() {
            return None;
        }
        let result = self.host.workspace.read(&path);
        if self.deadline_exceeded() {
            return None;
        }
        result
    }

    fn http_request(
        &mut self,
        method: String,
        url: String,
        headers_json: String,
        body: Option<Vec<u8>>,
        timeout_ms: Option<u32>,
    ) -> Result<bindings::near::agent::host::HttpResponse, String> {
        if let Some(error) = self.deadline_error() {
            return Err(error);
        }

        let request_body_bytes = body.as_ref().map(|body| body.len() as u64).unwrap_or(0);
        let response = self.host.http.request(WasmHttpRequest {
            method,
            url,
            headers_json,
            body,
            timeout_ms: self.remaining_timeout_ms(timeout_ms),
        });
        match response {
            Ok(response) => {
                self.record_network_egress(request_body_bytes);
                if let Some(error) = self.deadline_error() {
                    return Err(error);
                }
                Ok(bindings::near::agent::host::HttpResponse {
                    status: response.status,
                    headers_json: response.headers_json,
                    body: response.body,
                })
            }
            Err(error) => {
                if error.request_was_sent() {
                    self.record_network_egress(request_body_bytes);
                }
                Err(error.to_string())
            }
        }
    }

    fn tool_invoke(&mut self, alias: String, params_json: String) -> Result<String, String> {
        if let Some(error) = self.deadline_error() {
            return Err(error);
        }
        let result = self
            .host
            .tools
            .invoke(&alias, &params_json)
            .map_err(|error| error.to_string());
        if let Some(error) = self.deadline_error() {
            return Err(error);
        }
        result
    }

    fn secret_exists(&mut self, name: String) -> bool {
        if self.deadline_exceeded() {
            return false;
        }
        let exists = self.host.secrets.exists(&name);
        if self.deadline_exceeded() {
            return false;
        }
        exists
    }
}

fn truncate_log_message(message: String) -> String {
    if message.len() <= MAX_LOG_MESSAGE_BYTES {
        return message;
    }

    let mut end = MAX_LOG_MESSAGE_BYTES;
    while !message.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    message[..end].to_string()
}

#[cfg(test)]
mod tests {
    use super::{MAX_LOG_MESSAGE_BYTES, truncate_log_message};

    #[test]
    fn truncate_log_message_respects_utf8_boundaries() {
        let message = "é".repeat(MAX_LOG_MESSAGE_BYTES);
        let truncated = truncate_log_message(message);
        assert!(truncated.len() <= MAX_LOG_MESSAGE_BYTES);
        assert!(truncated.is_char_boundary(truncated.len()));
    }
}
