//! The standing per-scope contribution policy and the preflight acceptance
//! decision taken before any trace is queued.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use super::*;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TraceUploadAuthMode {
    /// Operator-minted workload token read from env (legacy/back-compat path).
    #[default]
    WorkloadTokenEnv,
    /// Self-signed workload JWTs using the standaloneice key (agent onboarding path).
    DeviceKey,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StandingTraceContributionPolicy {
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ingestion_endpoint: Option<String>,
    pub bearer_token_env: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upload_token_issuer_url: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub upload_token_issuer_allowed_hosts: BTreeSet<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upload_token_audience: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upload_token_tenant_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upload_token_workload_token_env: Option<String>,
    /// Operator-issued pilot invite code. When set, the trace-commons
    /// upload-claim request includes it (mirrored into the request body;
    /// the server-side issuer reads `WorkloadClaims.invite_code` today,
    /// the body field is forward-compat for a later server slice). The
    /// client surfaces the issuer's typed `PilotAllowlist*` refusals
    /// directly — there is no local JWT pre-flight that decodes the
    /// configured workload token to verify the embedded `invite_code`
    /// matches this value. Off by default; only required when the issuer
    /// is allowlist-gated. A follow-up may add the pre-flight check.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upload_token_invite_code: Option<String>,
    #[serde(default = "default_trace_upload_claim_issuer_timeout_ms")]
    pub upload_token_issuer_timeout_ms: u64,
    pub include_message_text: bool,
    pub include_tool_payloads: bool,
    pub auto_submit_failed_traces: bool,
    pub auto_submit_high_value_traces: bool,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub selected_tools: BTreeSet<String>,
    pub require_manual_approval_when_pii_detected: bool,
    pub min_submission_score: f32,
    pub credit_notice_interval_hours: u32,
    pub default_scope: ConsentScope,
    #[serde(default)]
    pub auth_mode: TraceUploadAuthMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_key_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceContributionAcceptance {
    PreviewOnly,
    QueueFromPreview,
    ManualSubmit,
    AutonomousSubmit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceContributionPolicyRejection {
    OptInDisabled,
    EndpointMissing,
}

impl std::fmt::Display for TraceContributionPolicyRejection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OptInDisabled => write!(f, "trace contribution opt-in is disabled"),
            Self::EndpointMissing => write!(f, "trace contribution endpoint is not configured"),
        }
    }
}

impl std::error::Error for TraceContributionPolicyRejection {}

pub fn preflight_trace_contribution_policy(
    policy: &StandingTraceContributionPolicy,
    intent: TraceContributionAcceptance,
) -> Result<(), TraceContributionPolicyRejection> {
    if intent == TraceContributionAcceptance::PreviewOnly {
        return Ok(());
    }
    if !policy.enabled {
        return Err(TraceContributionPolicyRejection::OptInDisabled);
    }
    if policy.ingestion_endpoint.is_none() {
        return Err(TraceContributionPolicyRejection::EndpointMissing);
    }
    Ok(())
}

pub fn normalize_trace_selected_tools<I, S>(selected_tools: I) -> BTreeSet<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    selected_tools
        .into_iter()
        .map(|tool| tool.as_ref().trim().to_string())
        .filter(|tool| !tool.is_empty())
        .collect()
}

impl Default for StandingTraceContributionPolicy {
    fn default() -> Self {
        Self {
            enabled: false,
            ingestion_endpoint: None,
            bearer_token_env: "IRONCLAW_TRACE_SUBMIT_TOKEN".to_string(),
            upload_token_issuer_url: None,
            upload_token_issuer_allowed_hosts: BTreeSet::new(),
            upload_token_audience: None,
            upload_token_tenant_id: None,
            upload_token_workload_token_env: None,
            upload_token_invite_code: None,
            upload_token_issuer_timeout_ms: TRACE_UPLOAD_CLAIM_DEFAULT_TIMEOUT_MS,
            include_message_text: false,
            include_tool_payloads: false,
            auto_submit_failed_traces: true,
            auto_submit_high_value_traces: true,
            selected_tools: BTreeSet::new(),
            require_manual_approval_when_pii_detected: true,
            min_submission_score: 0.35,
            credit_notice_interval_hours: 168,
            default_scope: ConsentScope::DebuggingEvaluation,
            auth_mode: TraceUploadAuthMode::default(),
            device_key_id: None,
        }
    }
}

impl StandingTraceContributionPolicy {
    pub fn set_enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    pub fn set_ingestion_endpoint(mut self, ingestion_endpoint: impl Into<String>) -> Self {
        self.ingestion_endpoint = Some(ingestion_endpoint.into());
        self
    }

    pub fn set_bearer_token_env(mut self, bearer_token_env: impl Into<String>) -> Self {
        self.bearer_token_env = bearer_token_env.into();
        self
    }

    pub fn set_upload_token_issuer_url(
        mut self,
        upload_token_issuer_url: impl Into<String>,
    ) -> Self {
        self.upload_token_issuer_url = Some(upload_token_issuer_url.into());
        self
    }

    pub fn set_upload_token_issuer_allowed_hosts<I, S>(
        mut self,
        upload_token_issuer_allowed_hosts: I,
    ) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.upload_token_issuer_allowed_hosts = upload_token_issuer_allowed_hosts
            .into_iter()
            .map(Into::into)
            .collect();
        self
    }

    pub fn set_upload_token_audience(mut self, upload_token_audience: impl Into<String>) -> Self {
        self.upload_token_audience = Some(upload_token_audience.into());
        self
    }

    pub fn set_upload_token_tenant_id(mut self, upload_token_tenant_id: impl Into<String>) -> Self {
        self.upload_token_tenant_id = Some(upload_token_tenant_id.into());
        self
    }

    pub fn set_upload_token_workload_token_env(
        mut self,
        upload_token_workload_token_env: impl Into<String>,
    ) -> Self {
        self.upload_token_workload_token_env = Some(upload_token_workload_token_env.into());
        self
    }

    pub fn set_upload_token_invite_code(
        mut self,
        upload_token_invite_code: impl Into<String>,
    ) -> Self {
        self.upload_token_invite_code = Some(upload_token_invite_code.into());
        self
    }

    pub fn set_upload_token_issuer_timeout_ms(
        mut self,
        upload_token_issuer_timeout_ms: u64,
    ) -> Self {
        self.upload_token_issuer_timeout_ms = upload_token_issuer_timeout_ms;
        self
    }

    pub fn set_include_message_text(mut self, include_message_text: bool) -> Self {
        self.include_message_text = include_message_text;
        self
    }

    pub fn set_include_tool_payloads(mut self, include_tool_payloads: bool) -> Self {
        self.include_tool_payloads = include_tool_payloads;
        self
    }

    pub fn set_auto_submit_failed_traces(mut self, auto_submit_failed_traces: bool) -> Self {
        self.auto_submit_failed_traces = auto_submit_failed_traces;
        self
    }

    pub fn set_auto_submit_high_value_traces(
        mut self,
        auto_submit_high_value_traces: bool,
    ) -> Self {
        self.auto_submit_high_value_traces = auto_submit_high_value_traces;
        self
    }

    pub fn set_selected_tools<I, S>(mut self, selected_tools: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.selected_tools = selected_tools.into_iter().map(Into::into).collect();
        self
    }

    pub fn set_require_manual_approval_when_pii_detected(
        mut self,
        require_manual_approval_when_pii_detected: bool,
    ) -> Self {
        self.require_manual_approval_when_pii_detected = require_manual_approval_when_pii_detected;
        self
    }

    pub fn set_min_submission_score(mut self, min_submission_score: f32) -> Self {
        self.min_submission_score = min_submission_score;
        self
    }

    pub fn set_credit_notice_interval_hours(mut self, credit_notice_interval_hours: u32) -> Self {
        self.credit_notice_interval_hours = credit_notice_interval_hours;
        self
    }

    pub fn set_default_scope(mut self, default_scope: ConsentScope) -> Self {
        self.default_scope = default_scope;
        self
    }

    pub fn set_auth_mode(mut self, auth_mode: TraceUploadAuthMode) -> Self {
        self.auth_mode = auth_mode;
        self
    }

    pub fn set_device_key_id(mut self, device_key_id: impl Into<String>) -> Self {
        self.device_key_id = Some(device_key_id.into());
        self
    }
}

pub(crate) fn default_trace_upload_claim_issuer_timeout_ms() -> u64 {
    TRACE_UPLOAD_CLAIM_DEFAULT_TIMEOUT_MS
}
