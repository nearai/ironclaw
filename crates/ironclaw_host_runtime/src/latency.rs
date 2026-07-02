use std::time::Instant;

use ironclaw_host_api::{CapabilityId, ResourceScope};
use ironclaw_observability::json_value_bytes;

pub(crate) struct RuntimeLatencyFields {
    capability_id: String,
    runtime: String,
    tenant_id: String,
    user_id: String,
    agent_id: String,
    project_id: String,
    mission_id: String,
    thread_id: String,
    invocation_id: String,
    input_bytes: u64,
    method: String,
    request_body_bytes: u64,
    response_body_limit: u64,
    credential_injection_count: usize,
    saves_body: bool,
    allow_partial_response_body: bool,
}

#[derive(Default)]
pub(crate) struct RuntimeLatencyMetrics {
    pub(crate) request_bytes: u64,
    pub(crate) response_bytes: u64,
    pub(crate) output_bytes: u64,
    pub(crate) used_prepared_reservation: bool,
}

impl RuntimeLatencyFields {
    pub(crate) fn from_json_input(
        capability_id: &CapabilityId,
        scope: &ResourceScope,
        runtime: impl Into<String>,
        input: &serde_json::Value,
    ) -> Option<Self> {
        if !ironclaw_observability::live_latency_enabled() {
            return None;
        }
        Self::from_scope(capability_id, scope, runtime, json_value_bytes(input))
    }

    pub(crate) fn from_scope(
        capability_id: &CapabilityId,
        scope: &ResourceScope,
        runtime: impl Into<String>,
        input_bytes: u64,
    ) -> Option<Self> {
        ironclaw_observability::live_latency_enabled().then_some(Self {
            capability_id: capability_id.to_string(),
            runtime: runtime.into(),
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
            method: String::new(),
            request_body_bytes: 0,
            response_body_limit: 0,
            credential_injection_count: 0,
            saves_body: false,
            allow_partial_response_body: false,
        })
    }

    pub(crate) fn with_http_details(
        mut self,
        method: impl Into<String>,
        request_body_bytes: u64,
        response_body_limit: u64,
        credential_injection_count: usize,
        saves_body: bool,
        allow_partial_response_body: bool,
    ) -> Self {
        self.method = method.into();
        self.request_body_bytes = request_body_bytes;
        self.response_body_limit = response_body_limit;
        self.credential_injection_count = credential_injection_count;
        self.saves_body = saves_body;
        self.allow_partial_response_body = allow_partial_response_body;
        self
    }
}

pub(crate) fn started_at() -> Option<Instant> {
    ironclaw_observability::live_latency_started_at()
}

pub(crate) fn trace_runtime_ok(
    component: &'static str,
    operation: &'static str,
    fields: Option<&RuntimeLatencyFields>,
    started_at: Option<Instant>,
    metrics: RuntimeLatencyMetrics,
) {
    let Some(fields) = fields else {
        return;
    };

    ironclaw_observability::live_latency_trace_ok!(
        component,
        operation,
        started_at,
        capability_id = fields.capability_id.as_str(),
        runtime = fields.runtime.as_str(),
        tenant_id = fields.tenant_id.as_str(),
        user_id = fields.user_id.as_str(),
        agent_id = fields.agent_id.as_str(),
        project_id = fields.project_id.as_str(),
        mission_id = fields.mission_id.as_str(),
        thread_id = fields.thread_id.as_str(),
        invocation_id = fields.invocation_id.as_str(),
        input_bytes = fields.input_bytes,
        method = fields.method.as_str(),
        request_body_bytes = fields.request_body_bytes,
        response_body_limit = fields.response_body_limit,
        credential_injection_count = fields.credential_injection_count,
        saves_body = fields.saves_body,
        allow_partial_response_body = fields.allow_partial_response_body,
        request_bytes = metrics.request_bytes,
        response_bytes = metrics.response_bytes,
        output_bytes = metrics.output_bytes,
        used_prepared_reservation = metrics.used_prepared_reservation,
        "host runtime operation completed",
    );
}

pub(crate) fn trace_runtime_error(
    component: &'static str,
    operation: &'static str,
    fields: Option<&RuntimeLatencyFields>,
    started_at: Option<Instant>,
    error_kind: &str,
    metrics: RuntimeLatencyMetrics,
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
        runtime = fields.runtime.as_str(),
        tenant_id = fields.tenant_id.as_str(),
        user_id = fields.user_id.as_str(),
        agent_id = fields.agent_id.as_str(),
        project_id = fields.project_id.as_str(),
        mission_id = fields.mission_id.as_str(),
        thread_id = fields.thread_id.as_str(),
        invocation_id = fields.invocation_id.as_str(),
        input_bytes = fields.input_bytes,
        method = fields.method.as_str(),
        request_body_bytes = fields.request_body_bytes,
        response_body_limit = fields.response_body_limit,
        credential_injection_count = fields.credential_injection_count,
        saves_body = fields.saves_body,
        allow_partial_response_body = fields.allow_partial_response_body,
        request_bytes = metrics.request_bytes,
        response_bytes = metrics.response_bytes,
        output_bytes = metrics.output_bytes,
        used_prepared_reservation = metrics.used_prepared_reservation,
        "host runtime operation failed",
    );
}
