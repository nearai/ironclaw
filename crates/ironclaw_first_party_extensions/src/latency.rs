use std::time::Instant;

use ironclaw_host_api::{CapabilityId, ResourceScope};
use serde_json::Value;

pub(crate) struct FirstPartyToolLatencyFields {
    capability_id: String,
    tenant_id: String,
    user_id: String,
    agent_id: String,
    project_id: String,
    mission_id: String,
    thread_id: String,
    invocation_id: String,
    input_bytes: u64,
}

#[derive(Default)]
pub(crate) struct FirstPartyToolLatencyMetrics {
    pub(crate) request_bytes: u64,
    pub(crate) network_egress_bytes: u64,
    pub(crate) output_bytes: u64,
}

impl FirstPartyToolLatencyFields {
    pub(crate) fn from_input(
        capability_id: &CapabilityId,
        scope: &ResourceScope,
        input: &Value,
    ) -> Option<Self> {
        Self::from_input_bytes(capability_id, scope, json_bytes(input))
    }

    pub(crate) fn from_input_bytes(
        capability_id: &CapabilityId,
        scope: &ResourceScope,
        input_bytes: u64,
    ) -> Option<Self> {
        Self::from_input_bytes_name(capability_id.to_string(), scope, input_bytes)
    }

    pub(crate) fn from_input_bytes_name(
        capability_id: impl Into<String>,
        scope: &ResourceScope,
        input_bytes: u64,
    ) -> Option<Self> {
        ironclaw_observability::live_latency_enabled().then(|| Self {
            capability_id: capability_id.into(),
            tenant_id: scope.tenant_id.as_str().to_string(),
            user_id: scope.user_id.as_str().to_string(),
            agent_id: scope
                .agent_id
                .as_ref()
                .map(|id| id.as_str().to_string())
                .unwrap_or_default(),
            project_id: scope
                .project_id
                .as_ref()
                .map(|id| id.as_str().to_string())
                .unwrap_or_default(),
            mission_id: scope
                .mission_id
                .as_ref()
                .map(|id| id.as_str().to_string())
                .unwrap_or_default(),
            thread_id: scope
                .thread_id
                .as_ref()
                .map(|id| id.as_str().to_string())
                .unwrap_or_default(),
            invocation_id: scope.invocation_id.to_string(),
            input_bytes,
        })
    }
}

pub(crate) fn started_at() -> Option<Instant> {
    ironclaw_observability::live_latency_started_at()
}

pub(crate) fn json_bytes(value: &Value) -> u64 {
    serde_json::to_vec(value)
        .map(|bytes| bytes.len() as u64)
        .unwrap_or(0)
}

pub(crate) fn trace_tool_ok(
    component: &'static str,
    operation: &'static str,
    fields: Option<&FirstPartyToolLatencyFields>,
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
        capability_id = fields.capability_id.as_str(),
        tenant_id = fields.tenant_id.as_str(),
        user_id = fields.user_id.as_str(),
        agent_id = fields.agent_id.as_str(),
        project_id = fields.project_id.as_str(),
        mission_id = fields.mission_id.as_str(),
        thread_id = fields.thread_id.as_str(),
        invocation_id = fields.invocation_id.as_str(),
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
    fields: Option<&FirstPartyToolLatencyFields>,
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
        capability_id = fields.capability_id.as_str(),
        tenant_id = fields.tenant_id.as_str(),
        user_id = fields.user_id.as_str(),
        agent_id = fields.agent_id.as_str(),
        project_id = fields.project_id.as_str(),
        mission_id = fields.mission_id.as_str(),
        thread_id = fields.thread_id.as_str(),
        invocation_id = fields.invocation_id.as_str(),
        input_bytes = fields.input_bytes,
        request_bytes = metrics.request_bytes,
        network_egress_bytes = metrics.network_egress_bytes,
        output_bytes = metrics.output_bytes,
        "first-party tool operation failed",
    );
}
