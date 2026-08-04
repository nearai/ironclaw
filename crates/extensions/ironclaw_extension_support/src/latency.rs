use std::time::Instant;

use ironclaw_host_api::{ids::CapabilityId, resource::ResourceScope};
use serde_json::Value;

/// Serializes `value` purely to count the bytes it would occupy.
///
/// Cheap per byte but *not* free: it walks the whole value, and a `read_file`
/// output can be large. Every caller must therefore establish that latency
/// tracing is live before calling — the counter below is how tests prove they
/// do, since "no work happened" has no other observable signature.
#[inline]
pub(crate) fn json_bytes(value: &Value) -> u64 {
    #[cfg(test)]
    JSON_BYTES_CALLS.with(|calls| calls.set(calls.get() + 1));
    ironclaw_observability::json_value_bytes(value)
}

#[cfg(test)]
thread_local! {
    /// Thread-local on purpose: `#[tokio::test]` runs on a current-thread
    /// runtime, so a task's measurements stay on its own thread and a sibling
    /// test running in parallel cannot pollute the count.
    pub(crate) static JSON_BYTES_CALLS: std::cell::Cell<usize> =
        const { std::cell::Cell::new(0) };
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
