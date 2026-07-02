use std::time::Instant;

use ironclaw_host_api::{CapabilityId, ResourceScope};
pub(crate) use ironclaw_observability::json_value_bytes as json_bytes;
use serde_json::Value;

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
