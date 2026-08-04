use std::time::Instant;

use ironclaw_host_api::{ids::CapabilityId, resource::ResourceScope};
use serde_json::Value;

/// Serialized byte size of a JSON value, without materializing the bytes.
///
/// Local by design, and note that most of this crate's uses are **not**
/// observability: `web_access` and `gsuite` feed the result to
/// `ResourceUsage::set_output_bytes`, i.e. resource accounting. It used to
/// live in `ironclaw_observability`, which made it look like a shared
/// measurement contract; it is not. `output_bytes` is already measured three
/// different ways across the workspace — this counter here and in
/// `ironclaw_host_runtime`, `output.stdout.len()` in `ironclaw_scripts`, and
/// `Value::to_string().len()` in `ironclaw_loop_host` — because each producer
/// measures what *it* produced. (PROPOSAL §6.2.5, §12.12 D-K.)
///
/// Returns 0 if the value cannot be serialized, which `serde_json::Value`
/// cannot do in practice — a trace/accounting field never fails a caller.
#[inline]
pub(crate) fn json_bytes(value: &Value) -> u64 {
    let mut counter = JsonByteCounter::default();
    serde_json::to_writer(&mut counter, value)
        .map(|()| counter.bytes)
        .unwrap_or(0)
}

#[derive(Default)]
struct JsonByteCounter {
    bytes: u64,
}

impl std::io::Write for JsonByteCounter {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        self.bytes = self.bytes.saturating_add(buffer.len() as u64);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

pub(crate) struct FirstPartyToolLatencyFields<'a> {
    capability_id: &'a CapabilityId,
    scope: &'a ResourceScope,
    input_bytes: u64,
}

#[derive(Default)]
pub(crate) struct FirstPartyToolLatencyMetrics {
    pub(crate) request_bytes: u64,
    pub(crate) network_egress_bytes: u64,
    pub(crate) output_bytes: u64,
}

impl<'a> FirstPartyToolLatencyFields<'a> {
    pub(crate) fn from_input(
        capability_id: &'a CapabilityId,
        scope: &'a ResourceScope,
        input: &Value,
    ) -> Option<Self> {
        if !ironclaw_observability::live_latency_enabled() {
            return None;
        }
        Self::from_input_bytes(capability_id, scope, json_bytes(input))
    }

    pub(crate) fn from_input_bytes(
        capability_id: &'a CapabilityId,
        scope: &'a ResourceScope,
        input_bytes: u64,
    ) -> Option<Self> {
        ironclaw_observability::live_latency_enabled().then_some(Self {
            capability_id,
            scope,
            input_bytes,
        })
    }
}

pub(crate) fn started_at() -> Option<Instant> {
    ironclaw_observability::live_latency_started_at()
}

pub(crate) fn trace_tool_ok(
    component: &'static str,
    operation: &'static str,
    fields: Option<&FirstPartyToolLatencyFields<'_>>,
    started_at: Option<Instant>,
    metrics: FirstPartyToolLatencyMetrics,
) {
    let Some(fields) = fields else {
        return;
    };

    ironclaw_observability::live_latency_trace_ok!(
        component,
        operation,
        started_at,
        capability_id = %fields.capability_id,
        tenant_id = %fields.scope.tenant_id,
        user_id = %fields.scope.user_id,
        agent_id = fields.scope.agent_id.as_ref().map(|id| id.as_str()).unwrap_or(""),
        project_id = fields.scope.project_id.as_ref().map(|id| id.as_str()).unwrap_or(""),
        mission_id = fields.scope.mission_id.as_ref().map(|id| id.as_str()).unwrap_or(""),
        thread_id = fields.scope.thread_id.as_ref().map(|id| id.as_str()).unwrap_or(""),
        invocation_id = %fields.scope.invocation_id,
        input_bytes = fields.input_bytes,
        request_bytes = metrics.request_bytes,
        network_egress_bytes = metrics.network_egress_bytes,
        output_bytes = metrics.output_bytes,
        "first-party tool operation completed",
    );
}

pub(crate) fn trace_tool_error(
    component: &'static str,
    operation: &'static str,
    fields: Option<&FirstPartyToolLatencyFields<'_>>,
    started_at: Option<Instant>,
    error_kind: &str,
    metrics: FirstPartyToolLatencyMetrics,
) {
    let Some(fields) = fields else {
        return;
    };

    ironclaw_observability::live_latency_trace_error!(
        component,
        operation,
        started_at,
        error_kind,
        capability_id = %fields.capability_id,
        tenant_id = %fields.scope.tenant_id,
        user_id = %fields.scope.user_id,
        agent_id = fields.scope.agent_id.as_ref().map(|id| id.as_str()).unwrap_or(""),
        project_id = fields.scope.project_id.as_ref().map(|id| id.as_str()).unwrap_or(""),
        mission_id = fields.scope.mission_id.as_ref().map(|id| id.as_str()).unwrap_or(""),
        thread_id = fields.scope.thread_id.as_ref().map(|id| id.as_str()).unwrap_or(""),
        invocation_id = %fields.scope.invocation_id,
        input_bytes = fields.input_bytes,
        request_bytes = metrics.request_bytes,
        network_egress_bytes = metrics.network_egress_bytes,
        output_bytes = metrics.output_bytes,
        "first-party tool operation failed",
    );
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use serde_json::json;

    use super::*;

    #[test]
    fn json_bytes_matches_serialized_value_length() {
        let value = json!({
            "message": "hello",
            "count": 3,
            "items": ["a", "b"]
        });

        assert_eq!(
            json_bytes(&value),
            serde_json::to_vec(&value).unwrap().len() as u64
        );
    }

    #[test]
    fn json_byte_counter_saturates_on_write() {
        let mut counter = JsonByteCounter {
            bytes: u64::MAX - 1,
        };

        counter.write_all(b"abc").unwrap();

        assert_eq!(counter.bytes, u64::MAX);
    }
}
