use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock},
};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ironclaw_host_api::{
    ApprovalRequestId, CapabilityId, CorrelationId, ExtensionId, HostApiError,
    INPUT_ENCODE_HUMAN_SUMMARY, ProviderToolName, ResourceEstimate,
    RuntimeCredentialAuthRequirement, RuntimeKind, ThreadId,
};
use serde::{Deserialize, Deserializer, Serialize};
use thiserror::Error;

use crate::{
    AcceptedMessageRef, CapabilityActivityId, LoopDiagnosticRef, LoopGateRef, LoopMessageRef,
    LoopResultRef, ProductTurnContext, RedactedCheckpointPayload, RunProfileVersion, TurnActor,
    TurnCheckpointId, TurnId, TurnRunId, TurnScope,
};

use super::{
    compaction::{CompactionInitiator, LoopCompactionPort},
    content_digest::ContentDigest,
    instruction_bundle::InstructionBundleFingerprint,
    model_observation::{CapabilityFailureDetail, ModelVisibleToolObservation},
    prompt_text::{PromptTextSurface, validate_prompt_text},
    refs::{CheckpointSchemaId, LoopDriverId, ModelProfileId},
    snapshot::ResolvedRunProfile,
    system_inference::SystemInferenceTaskId,
};

const FORBIDDEN_MODEL_ROUTE_MARKERS: &[&str] = &[
    "access_token",
    "api_key",
    "apikey",
    "authorization",
    "password",
    "passwd",
    "secret",
];

const FORBIDDEN_EXACT_MODEL_ROUTE_MARKERS: &[&str] = &["bearer"];

fn validate_bounded_loop_string(
    value: String,
    label: &'static str,
    max_bytes: usize,
) -> Result<String, String> {
    if value.is_empty() {
        return Err(format!("{label} must not be empty"));
    }
    if value.len() > max_bytes {
        return Err(format!("{label} must be at most {max_bytes} bytes"));
    }
    if value
        .chars()
        .any(|character| character == '\0' || character.is_control())
    {
        return Err(format!("{label} must not contain NUL/control characters"));
    }
    Ok(value)
}

fn validate_prefixed_loop_ref(
    label: &'static str,
    prefix: &'static str,
    max_bytes: usize,
    value: String,
) -> Result<String, String> {
    let value = validate_bounded_loop_string(value, label, max_bytes)?;
    if !value.starts_with(prefix) {
        return Err(format!("{label} must start with `{prefix}`"));
    }
    Ok(value)
}

fn validate_prefixed_path_safe_loop_ref(
    label: &'static str,
    prefix: &'static str,
    max_bytes: usize,
    value: String,
) -> Result<String, String> {
    let value = validate_prefixed_loop_ref(label, prefix, max_bytes, value)?;
    if value.contains('/') || value.contains('\\') || value.contains("..") {
        return Err(format!(
            "{label} must not contain path separators or parent-directory markers"
        ));
    }
    Ok(value)
}

fn validate_loop_opaque_token(
    value: String,
    label: &'static str,
    max_bytes: usize,
) -> Result<String, String> {
    let value = validate_bounded_loop_string(value, label, max_bytes)?;
    if !value
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.'))
    {
        return Err(format!(
            "{label} must contain only ASCII letters, digits, _, -, or ."
        ));
    }
    Ok(value)
}

fn validate_loop_safe_identifier(
    value: String,
    label: &'static str,
    max_bytes: usize,
) -> Result<String, String> {
    let value = validate_bounded_loop_string(value, label, max_bytes)?;
    if !value.chars().all(|character| {
        character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.' | ':')
    }) {
        return Err(format!(
            "{label} must contain only ASCII letters, digits, _, -, ., or :"
        ));
    }

    let lower = value.to_ascii_lowercase();
    for forbidden in [
        "access_token",
        "access-token",
        "api_key",
        "apikey",
        "authorization",
        "bearer",
        "password",
        "passwd",
        "secret",
    ] {
        if lower.contains(forbidden) {
            return Err(format!(
                "{label} must not contain sensitive marker `{forbidden}`"
            ));
        }
    }
    if contains_secret_like_token(&lower) {
        return Err(format!("{label} must not contain API-key-like tokens"));
    }
    Ok(value)
}

fn validate_loop_safe_summary(value: String) -> Result<String, String> {
    let value = validate_bounded_loop_string(value, "loop safe summary", 512)?;
    if value == INPUT_ENCODE_HUMAN_SUMMARY {
        return Ok(value);
    }
    if value.chars().any(|character| {
        matches!(
            character,
            '{' | '}' | '[' | ']' | '`' | '<' | '>' | '/' | '\\'
        )
    }) {
        return Err(
            "loop safe summary must not contain raw payload or path delimiters".to_string(),
        );
    }

    let lower = value.to_ascii_lowercase();
    // Only credential markers are banned; descriptive error vocabulary
    // ("provider error", "stack trace", "tool input", …) is allowed because
    // the raw cause now rides the dedicated model-visible detail channel.
    for forbidden in [
        "access token",
        "api key",
        "api_key",
        "apikey",
        "authorization:",
        "bearer ",
        "password",
        "passwd",
        "secret",
    ] {
        if lower.contains(forbidden) {
            return Err(format!(
                "loop safe summary must not contain sensitive marker `{forbidden}`"
            ));
        }
    }
    if contains_secret_like_token(&lower) {
        return Err("loop safe summary must not contain API-key-like tokens".to_string());
    }
    Ok(value)
}

fn contains_secret_like_token(lower: &str) -> bool {
    lower
        .split(|character: char| {
            !character.is_ascii_alphanumeric() && !matches!(character, '-' | '_' | '.')
        })
        .any(is_secret_like_token)
}

fn is_secret_like_token(token: &str) -> bool {
    [
        "sk-",
        "sk-ant-",
        "ghp_",
        "github_pat_",
        "gho_",
        "ghu_",
        "ghs_",
        "ghr_",
        "glpat-",
        "gcp-",
        "ya29.",
        "aiza",
    ]
    .iter()
    .any(|prefix| token.starts_with(prefix))
        || (token.len() >= 16 && (token.starts_with("akia") || token.starts_with("asia")))
}

fn validate_loop_inline_message_body(value: String) -> Result<String, String> {
    validate_prompt_text(
        value,
        "loop inline message body",
        PromptTextSurface::GenericModelContent,
    )
    .map_err(|error| error.safe_summary)
}

macro_rules! bounded_loop_ref {
    ($name:ident, $label:literal, $prefix:literal, $max:expr) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, String> {
                validate_prefixed_loop_ref($label, $prefix, $max, value.into()).map(Self)
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                self.as_str()
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str(self.as_str())
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                let value = String::deserialize(deserializer)?;
                Self::new(value).map_err(serde::de::Error::custom)
            }
        }
    };
}

bounded_loop_ref!(CapabilityInputRef, "capability input ref", "input:", 256);
bounded_loop_ref!(
    LoopInputCursorToken,
    "loop input cursor token",
    "input-cursor:",
    256
);
bounded_loop_ref!(LoopInputAckToken, "loop input ack token", "input-ack:", 256);
bounded_loop_ref!(LoopProcessRef, "loop process ref", "process:", 256);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct LoopCheckpointStateRef(String);

impl LoopCheckpointStateRef {
    pub fn new(value: impl Into<String>) -> Result<Self, String> {
        validate_prefixed_path_safe_loop_ref(
            "loop checkpoint state ref",
            "checkpoint:",
            256,
            value.into(),
        )
        .map(Self)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for LoopCheckpointStateRef {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl std::fmt::Display for LoopCheckpointStateRef {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for LoopCheckpointStateRef {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

impl LoopCheckpointStateRef {
    pub(crate) fn legacy_unknown() -> Self {
        Self("checkpoint:unknown".to_string())
    }

    pub fn for_run(context: &LoopRunContext, token: impl Into<String>) -> Result<Self, String> {
        let token = validate_loop_opaque_token(token.into(), "loop checkpoint state token", 96)?;
        Self::new(format!("checkpoint:{}:{token}", context.run_id))
    }

    pub fn is_for_run(&self, context: &LoopRunContext) -> bool {
        let Some(token) = self
            .0
            .strip_prefix(&format!("checkpoint:{}:", context.run_id))
        else {
            return false;
        };
        validate_loop_opaque_token(token.to_string(), "loop checkpoint state token", 96).is_ok()
    }
}

/// Opaque reference to a host-built prompt bundle for one loop run.
///
/// Serialized refs use `prompt:{run_id}:{opaque_token}`. Consumers must treat
/// the token as opaque metadata and must not infer or persist raw prompt text
/// from this value.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct LoopPromptBundleRef(String);

impl LoopPromptBundleRef {
    pub fn new(value: impl Into<String>) -> Result<Self, String> {
        let value =
            validate_prefixed_loop_ref("loop prompt bundle ref", "prompt:", 256, value.into())?;
        let suffix = value
            .strip_prefix("prompt:")
            .ok_or_else(|| "loop prompt bundle ref must start with `prompt:`".to_string())?;
        let (run_id, token) = suffix.split_once(':').ok_or_else(|| {
            "loop prompt bundle ref must include scoped run id and opaque token".to_string()
        })?;
        uuid::Uuid::parse_str(run_id)
            .map_err(|_| "loop prompt bundle ref run id must be a UUID".to_string())?;
        validate_loop_opaque_token(token.to_string(), "loop prompt bundle token", 96)?;
        Ok(Self(value))
    }

    pub fn for_run(context: &LoopRunContext, token: impl Into<String>) -> Result<Self, String> {
        let token = validate_loop_opaque_token(token.into(), "loop prompt bundle token", 96)?;
        Self::new(format!("prompt:{}:{token}", context.run_id))
    }

    pub(crate) fn fresh_for_run(context: &LoopRunContext) -> Self {
        Self(format!(
            "prompt:{}:{}",
            context.run_id,
            uuid::Uuid::new_v4()
        ))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_for_run(&self, context: &LoopRunContext) -> bool {
        self.0.starts_with(&format!("prompt:{}:", context.run_id))
    }
}

impl AsRef<str> for LoopPromptBundleRef {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl std::fmt::Display for LoopPromptBundleRef {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for LoopPromptBundleRef {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct LoopSafeSummary(String);

impl LoopSafeSummary {
    pub fn new(value: impl Into<String>) -> Result<Self, String> {
        validate_loop_safe_summary(value.into()).map(Self)
    }

    /// Build a display-safe capability failure summary, replacing unsafe input
    /// with a fixed redaction marker.
    pub fn capability_failure_summary(value: impl Into<String>) -> Self {
        Self::new(value).unwrap_or_else(|_| Self::tool_failure_details_redacted())
    }

    /// Fixed summary used when a capability failure detail was intentionally
    /// redacted before reaching a user-visible or model-visible boundary.
    pub fn tool_failure_details_redacted() -> Self {
        Self("the tool failure details were redacted".to_string())
    }

    /// Fixed fallback for input-encoding failures when no narrower safe detail
    /// is available.
    pub fn tool_input_could_not_be_encoded() -> Self {
        Self(INPUT_ENCODE_HUMAN_SUMMARY.to_string())
    }

    pub fn model_gateway_failed() -> Self {
        Self("model gateway failed".to_string())
    }

    /// Sanitized summary for a primary model call that exceeded its timeout.
    /// Infallible because the literal is known to satisfy
    /// [`validate_loop_safe_summary`].
    pub fn model_gateway_timed_out() -> Self {
        Self("model gateway timed out".to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for LoopSafeSummary {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl std::fmt::Display for LoopSafeSummary {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Validated body for host-approved inline prompt messages.
///
/// Unlike [`LoopSafeSummary`], this preserves model-visible prompt formatting
/// and uses the generic model-content validation budget.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String")]
pub struct LoopInlineMessageBody(String);

impl LoopInlineMessageBody {
    pub fn new(value: impl Into<String>) -> Result<Self, String> {
        Self::try_from(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_inner(self) -> String {
        self.0
    }
}

impl AsRef<str> for LoopInlineMessageBody {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl std::fmt::Display for LoopInlineMessageBody {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl TryFrom<String> for LoopInlineMessageBody {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        validate_loop_inline_message_body(value).map(Self)
    }
}

impl<'de> Deserialize<'de> for LoopSafeSummary {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

fn origin_input_cursor_token() -> LoopInputCursorToken {
    LoopInputCursorToken("input-cursor:origin".to_string())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoopModelRouteSnapshot {
    pub provider_id: String,
    pub model_id: String,
    pub config_version: String,
    pub auth_version: String,
}

impl LoopModelRouteSnapshot {
    pub fn new(
        provider_id: impl Into<String>,
        model_id: impl Into<String>,
        config_version: impl Into<String>,
        auth_version: impl Into<String>,
    ) -> Self {
        Self {
            provider_id: provider_id.into(),
            model_id: model_id.into(),
            config_version: config_version.into(),
            auth_version: auth_version.into(),
        }
    }

    pub fn try_new(
        provider_id: impl Into<String>,
        model_id: impl Into<String>,
        config_version: impl Into<String>,
        auth_version: impl Into<String>,
    ) -> Result<Self, String> {
        let snapshot = Self::new(provider_id, model_id, config_version, auth_version);
        snapshot.validate()?;
        Ok(snapshot)
    }

    pub fn validate(&self) -> Result<(), String> {
        validate_model_route_component_value("provider_id", &self.provider_id, 128, |character| {
            character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.')
        })?;
        validate_model_route_component_value("model_id", &self.model_id, 256, |character| {
            character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.' | ':' | '/')
        })?;
        validate_model_route_component_value(
            "config_version",
            &self.config_version,
            128,
            |character| {
                character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.' | ':')
            },
        )?;
        validate_model_route_component_value(
            "auth_version",
            &self.auth_version,
            128,
            |character| {
                character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.' | ':')
            },
        )?;
        Ok(())
    }
}

/// Validate a persisted provider/model route component with the same redaction
/// marker policy used by host-owned loop snapshots and Reborn route keys.
pub fn validate_model_route_component_value(
    label: &'static str,
    value: &str,
    max_bytes: usize,
    allowed: impl Fn(char) -> bool,
) -> Result<(), String> {
    validate_bounded_loop_string(value.to_string(), label, max_bytes)?;
    if value.trim() != value {
        return Err(format!("{label} must not contain surrounding whitespace"));
    }
    if !value.chars().all(allowed) {
        return Err(format!("{label} contains unsupported characters"));
    }
    reject_sensitive_model_route_markers(label, value)?;
    Ok(())
}

fn reject_sensitive_model_route_markers(label: &'static str, value: &str) -> Result<(), String> {
    let lower = value.to_ascii_lowercase();
    for token in model_route_marker_tokens(&lower) {
        if FORBIDDEN_EXACT_MODEL_ROUTE_MARKERS.contains(&token)
            || FORBIDDEN_MODEL_ROUTE_MARKERS
                .iter()
                .any(|forbidden| token_contains_sensitive_marker(token, forbidden))
            || token.starts_with("sk-")
        {
            return Err(format!("{label} contains a forbidden marker"));
        }
    }
    Ok(())
}

fn model_route_marker_tokens(value: &str) -> impl Iterator<Item = &str> {
    value
        .split(|character: char| {
            !character.is_ascii_alphanumeric() && character != '-' && character != '_'
        })
        .filter(|token| !token.is_empty())
}

fn token_contains_sensitive_marker(token: &str, marker: &str) -> bool {
    let normalized = token.replace('-', "_");
    normalized == marker
        || normalized.starts_with(&format!("{marker}_"))
        || normalized.ends_with(&format!("_{marker}"))
        || normalized.contains(&format!("_{marker}_"))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoopRunContext {
    pub scope: TurnScope,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<TurnActor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accepted_message_ref: Option<AcceptedMessageRef>,
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub run_id: TurnRunId,
    pub resolved_run_profile: ResolvedRunProfile,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_model_route: Option<LoopModelRouteSnapshot>,
    pub loop_driver_id: LoopDriverId,
    pub loop_driver_version: RunProfileVersion,
    pub checkpoint_schema_id: CheckpointSchemaId,
    pub checkpoint_schema_version: RunProfileVersion,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub product_context: Option<ProductTurnContext>,
}

impl LoopRunContext {
    pub fn new(
        scope: TurnScope,
        turn_id: TurnId,
        run_id: TurnRunId,
        resolved_run_profile: ResolvedRunProfile,
    ) -> Self {
        let thread_id = scope.thread_id.clone();
        let loop_driver_id = resolved_run_profile.loop_driver.id.clone();
        let loop_driver_version = resolved_run_profile.loop_driver.version;
        let checkpoint_schema_id = resolved_run_profile.checkpoint_schema_id.clone();
        let checkpoint_schema_version = resolved_run_profile.checkpoint_schema_version;
        Self {
            scope,
            actor: None,
            accepted_message_ref: None,
            thread_id,
            turn_id,
            run_id,
            resolved_run_profile,
            resolved_model_route: None,
            loop_driver_id,
            loop_driver_version,
            checkpoint_schema_id,
            checkpoint_schema_version,
            product_context: None,
        }
    }

    pub fn with_actor(mut self, actor: TurnActor) -> Self {
        self.actor = Some(actor);
        self
    }

    pub fn with_accepted_message_ref(mut self, accepted_message_ref: AcceptedMessageRef) -> Self {
        self.accepted_message_ref = Some(accepted_message_ref);
        self
    }

    pub fn actor(&self) -> Option<&TurnActor> {
        self.actor.as_ref()
    }

    pub fn with_resolved_model_route(mut self, snapshot: LoopModelRouteSnapshot) -> Self {
        self.resolved_model_route = Some(snapshot);
        self
    }

    pub fn with_product_context(mut self, product_context: ProductTurnContext) -> Self {
        self.product_context = Some(product_context);
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentLoopHostErrorKind {
    Unauthorized,
    /// Host-owned credential acquisition failed for the requested provider/model.
    /// The error summary must stay sanitized and must not expose secret material,
    /// token refresh details, or backend-specific credential-store errors.
    CredentialUnavailable,
    ScopeMismatch,
    StaleSurface,
    InvalidInvocation,
    /// The request payload itself is well-formed but its content is invalid in
    /// the current host state (e.g. schema id/version mismatch on checkpoint load).
    Invalid,
    /// The model/provider output was structurally invalid for the active loop contract.
    InvalidOutput,
    PolicyDenied,
    BudgetExceeded,
    /// The model call would push utilization past the configured pause
    /// threshold. Callers surface an approval gate (foreground or
    /// background) and retry after the user resolves it.
    BudgetApprovalRequired,
    /// Durable budget accounting (reservation read/write/reconcile)
    /// failed. Distinct from `BudgetExceeded`/`BudgetApprovalRequired`
    /// because the failure is in the governor itself, not in the budget
    /// outcome — callers must fail closed.
    BudgetAccountingFailed,
    Unavailable,
    Cancelled,
    CheckpointRejected,
    TranscriptWriteFailed,
    Internal,
}

impl AgentLoopHostErrorKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unauthorized => "unauthorized",
            Self::CredentialUnavailable => "credential_unavailable",
            Self::ScopeMismatch => "scope_mismatch",
            Self::StaleSurface => "stale_surface",
            Self::InvalidInvocation => "invalid_invocation",
            Self::Invalid => "invalid",
            Self::InvalidOutput => "invalid_output",
            Self::PolicyDenied => "policy_denied",
            Self::BudgetExceeded => "budget_exceeded",
            Self::BudgetApprovalRequired => "budget_approval_required",
            Self::BudgetAccountingFailed => "budget_accounting_failed",
            Self::Unavailable => "unavailable",
            Self::Cancelled => "cancelled",
            Self::CheckpointRejected => "checkpoint_rejected",
            Self::TranscriptWriteFailed => "transcript_write_failed",
            Self::Internal => "internal",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentLoopHostErrorReasonKind {
    ModelCreditsExhausted,
}

impl AgentLoopHostErrorReasonKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ModelCreditsExhausted => "model_credits_exhausted",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Error)]
#[error("agent loop host {kind:?}: {safe_summary}")]
pub struct AgentLoopHostError {
    pub kind: AgentLoopHostErrorKind,
    pub safe_summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason_kind: Option<AgentLoopHostErrorReasonKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gate_ref: Option<LoopGateRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnostic_ref: Option<LoopDiagnosticRef>,
    /// Model-visible, secret-scrubbed raw cause. Unlike `safe_summary`, this
    /// carries the original error text (paths, codes, schema refs) so the model
    /// can retry or explain. Secret VALUES are redacted by the producer via
    /// [`sanitize_model_visible_text`]; the word/delimiter ban is NOT applied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl AgentLoopHostError {
    pub fn new(kind: AgentLoopHostErrorKind, safe_summary: impl Into<String>) -> Self {
        Self {
            kind,
            safe_summary: safe_summary.into(),
            reason_kind: None,
            gate_ref: None,
            diagnostic_ref: None,
            detail: None,
        }
    }

    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub fn with_reason_kind(mut self, reason_kind: AgentLoopHostErrorReasonKind) -> Self {
        self.reason_kind = Some(reason_kind);
        self
    }

    pub fn with_gate_ref(mut self, gate_ref: LoopGateRef) -> Self {
        self.gate_ref = Some(gate_ref);
        self
    }

    pub fn with_diagnostic_ref(mut self, diagnostic_ref: LoopDiagnosticRef) -> Self {
        self.diagnostic_ref = Some(diagnostic_ref);
        self
    }
}

pub trait LoopRunInfoPort: Send + Sync {
    fn run_context(&self) -> &LoopRunContext;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoopContextRequest {
    pub after: Option<LoopInputCursor>,
    pub limit: usize,
    #[serde(default = "default_prompt_mode")]
    pub mode: PromptMode,
}

fn default_prompt_mode() -> PromptMode {
    PromptMode::TextOnly
}

pub const LOOP_CONTEXT_SNIPPET_MODEL_CONTENT_MAX_BYTES: usize = 64 * 1024;
pub const LOOP_CONTEXT_TOTAL_MODEL_CONTENT_MAX_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LoopContextBundle {
    pub identity_messages: Vec<LoopContextMessage>,
    pub messages: Vec<LoopContextMessage>,
    pub compaction_message_index: Vec<LoopContextCompactionMetadata>,
    pub instruction_snippets: Vec<LoopContextSnippet>,
    pub memory_snippets: Vec<LoopContextSnippet>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoopContextMessage {
    /// Reference to the persisted message content.
    ///
    /// `None` means "summary-only entry; prompt port MUST NOT resolve content —
    /// use `safe_summary` verbatim instead." Mirrors the
    /// `SkillTrustLevel::Installed` carrying `prompt_content: None` pattern.
    pub message_ref: Option<LoopMessageRef>,
    pub role: String,
    pub safe_summary: String,
    pub compaction: Option<LoopContextCompactionMetadata>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoopContextCompactionMetadata {
    pub sequence: u64,
    pub kind: LoopContextCompactionKind,
    pub estimated_tokens: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopContextCompactionKind {
    User,
    Assistant,
    System,
    Summary,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoopContextSnippetMetadata {
    pub source_name: String,
    pub trust_level: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoopContextSnippet {
    pub snippet_ref: String,
    /// Full model-visible content for this context snippet.
    ///
    /// This is intentionally distinct from `safe_summary`: prompt assembly must
    /// materialize this field, while summaries remain short metadata for
    /// fingerprints, transcript displays, and diagnostics.
    pub model_content: String,
    pub safe_summary: String,
    /// Safe metadata for prompt milestones. Skill snippet producers using the
    /// `skill:` ref namespace must populate this so telemetry can record active
    /// skill name/trust without leaking prompt content.
    pub metadata: Option<LoopContextSnippetMetadata>,
}

#[async_trait]
pub trait LoopContextPort: Send + Sync {
    async fn load_loop_context(
        &self,
        request: LoopContextRequest,
    ) -> Result<LoopContextBundle, AgentLoopHostError>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoopInputCursor {
    scope: TurnScope,
    run_id: TurnRunId,
    token: LoopInputCursorToken,
}

impl LoopInputCursor {
    pub fn origin_for_run(context: &LoopRunContext) -> Self {
        Self {
            scope: context.scope.clone(),
            run_id: context.run_id,
            token: origin_input_cursor_token(),
        }
    }

    pub fn from_host_token(context: &LoopRunContext, token: LoopInputCursorToken) -> Self {
        Self {
            scope: context.scope.clone(),
            run_id: context.run_id,
            token,
        }
    }

    pub fn scope(&self) -> &TurnScope {
        &self.scope
    }

    pub fn run_id(&self) -> TurnRunId {
        self.run_id
    }

    pub fn token(&self) -> &LoopInputCursorToken {
        &self.token
    }

    pub fn is_for_run(&self, context: &LoopRunContext) -> bool {
        self.scope == context.scope && self.run_id == context.run_id
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoopInputBatch {
    pub inputs: Vec<LoopInput>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub input_acks: Vec<LoopInputAck>,
    pub next_cursor: LoopInputCursor,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoopInputAck {
    pub cursor: LoopInputCursor,
    pub token: LoopInputAckToken,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopInput {
    UserMessage { message_ref: LoopMessageRef },
    FollowUp { message_ref: LoopMessageRef },
    Steering { message_ref: LoopMessageRef },
    Interrupt { kind: LoopInterruptKind },
    Cancel { reason_kind: LoopCancelReasonKind },
    GateResolved { gate_ref: LoopGateRef },
    CapabilitySurfaceChanged { version: CapabilitySurfaceVersion },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopInterruptKind {
    UserInterrupt,
    HostShutdown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopCancelReasonKind {
    UserRequested,
    Superseded,
    Policy,
}

#[async_trait]
pub trait LoopInputPort: Send + Sync {
    async fn poll_inputs(
        &self,
        after: LoopInputCursor,
        limit: usize,
    ) -> Result<LoopInputBatch, AgentLoopHostError>;

    async fn ack_inputs(&self, tokens: Vec<LoopInputAckToken>) -> Result<(), AgentLoopHostError>;
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct CapabilitySurfaceVersion(String);

impl CapabilitySurfaceVersion {
    pub fn new(value: impl Into<String>) -> Result<Self, String> {
        validate_loop_safe_identifier(value.into(), "capability surface version", 128).map(Self)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for CapabilitySurfaceVersion {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl std::fmt::Display for CapabilitySurfaceVersion {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for CapabilitySurfaceVersion {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoopModelCapabilityView {
    /// Final capability IDs visible to this model call after the loop driver has
    /// applied its strategy to the host-owned capability surface.
    pub visible_capability_ids: Vec<CapabilityId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoopModelRequest {
    pub messages: Vec<LoopModelMessage>,
    #[serde(default)]
    pub inline_messages: Vec<LoopInlineMessage>,
    pub surface_version: Option<CapabilitySurfaceVersion>,
    pub model_preference: Option<ModelProfileId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability_view: Option<LoopModelCapabilityView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoopModelMessage {
    pub role: String,
    pub content_ref: LoopMessageRef,
}

/// Prompt construction mode requested by an agent-loop driver.
///
/// `TextOnly` builds a prompt from transcript/context message refs and is the
/// only mode supported by [`crate::run_profile::HostManagedLoopPromptPort`]
/// today. `CodeAct` is reserved for a future checkpoint/tool-aware prompt
/// bundle flow and is rejected by the text-only host port.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptMode {
    TextOnly,
    #[serde(rename = "codeact")]
    CodeAct,
}

impl PromptMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::TextOnly => "text_only",
            Self::CodeAct => "codeact",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopInlineMessageRole {
    System,
    User,
    Assistant,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoopInlineMessage {
    pub role: LoopInlineMessageRole,
    pub safe_body: LoopInlineMessageBody,
}

/// Request for a host-managed prompt bundle.
///
/// The optional cursor and checkpoint refs are run-scoped and are validated by
/// host ports before context is loaded. `max_messages` is a host budget hint;
/// zero is accepted only for inline-only context-free prompts, and oversized
/// values may be clamped by the implementation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoopPromptBundleRequest {
    pub mode: PromptMode,
    pub context_cursor: Option<LoopInputCursor>,
    pub surface_version: Option<CapabilitySurfaceVersion>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability_view: Option<LoopModelCapabilityView>,
    pub checkpoint_state_ref: Option<LoopCheckpointStateRef>,
    pub max_messages: Option<u32>,
    #[serde(default)]
    pub inline_messages: Vec<LoopInlineMessage>,
}

/// Prompt bundle returned to a driver.
///
/// The bundle carries model-message references rather than raw prompt text.
/// Drivers pass these refs to [`LoopModelPort`], allowing the host to resolve
/// content under the same run scope and policy checks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoopPromptBundle {
    pub bundle_ref: LoopPromptBundleRef,
    pub messages: Vec<LoopModelMessage>,
    pub surface_version: Option<CapabilitySurfaceVersion>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub compaction_message_index: Vec<LoopContextCompactionMetadata>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instruction_fingerprint: Option<InstructionBundleFingerprint>,
    #[serde(default)]
    pub identity_message_count: u32,
    #[serde(default)]
    pub instruction_snippet_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoopPromptBundleGrant {
    pub bundle_ref: LoopPromptBundleRef,
    pub messages: Vec<LoopModelMessage>,
    pub surface_version: Option<CapabilitySurfaceVersion>,
    pub instruction_fingerprint: Option<InstructionBundleFingerprint>,
}

#[derive(Clone, Default)]
pub struct LoopPromptBundleAuthority {
    inner: Arc<Mutex<LoopPromptBundleAuthorityState>>,
}

#[derive(Default)]
struct LoopPromptBundleAuthorityState {
    latest_by_run: HashMap<String, LoopPromptBundleGrant>,
}

impl LoopPromptBundleAuthority {
    pub fn shared() -> Self {
        static AUTHORITY: OnceLock<LoopPromptBundleAuthority> = OnceLock::new();
        AUTHORITY.get_or_init(Self::default).clone()
    }

    pub fn issue_bundle(
        &self,
        context: &LoopRunContext,
        bundle: &LoopPromptBundle,
    ) -> Result<(), AgentLoopHostError> {
        if !bundle.bundle_ref.is_for_run(context) {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::ScopeMismatch,
                "prompt bundle ref is not scoped to this loop run",
            ));
        }

        self.lock_state()?.latest_by_run.insert(
            context.run_id.to_string(),
            LoopPromptBundleGrant {
                bundle_ref: bundle.bundle_ref.clone(),
                messages: bundle.messages.clone(),
                surface_version: bundle.surface_version.clone(),
                instruction_fingerprint: bundle.instruction_fingerprint.clone(),
            },
        );
        Ok(())
    }

    pub fn authorize_latest_model_request(
        &self,
        context: &LoopRunContext,
        messages: &[LoopModelMessage],
        surface_version: &Option<CapabilitySurfaceVersion>,
    ) -> Result<LoopPromptBundleGrant, AgentLoopHostError> {
        let grant = self
            .lock_state()?
            .latest_by_run
            .remove(&context.run_id.to_string())
            .ok_or_else(|| {
                AgentLoopHostError::new(
                    AgentLoopHostErrorKind::InvalidInvocation,
                    "model request has no host-built prompt bundle",
                )
            })?;

        if !grant.bundle_ref.is_for_run(context) {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::ScopeMismatch,
                "prompt bundle ref is not scoped to this loop run",
            ));
        }
        if grant.messages != messages {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::InvalidInvocation,
                "model request messages do not match the host-built prompt bundle",
            ));
        }
        if &grant.surface_version != surface_version {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::StaleSurface,
                "model request surface version does not match the host-built prompt bundle",
            ));
        }

        Ok(grant)
    }

    fn lock_state(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, LoopPromptBundleAuthorityState>, AgentLoopHostError> {
        self.inner.lock().map_err(|_| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::Internal,
                "prompt bundle authority is unavailable",
            )
        })
    }
}

/// Host boundary for building prompt bundles before model invocation.
///
/// Implementations own context loading, scoping, prompt-shape policy, and
/// milestone emission. Drivers should not assemble raw prompt strings when a
/// prompt port is available.
#[async_trait]
pub trait LoopPromptPort: Send + Sync {
    async fn build_prompt_bundle(
        &self,
        request: LoopPromptBundleRequest,
    ) -> Result<LoopPromptBundle, AgentLoopHostError>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoopModelResponse {
    pub chunks: Vec<ModelStreamChunk>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub safe_reasoning_deltas: Vec<String>,
    pub output: ParentLoopOutput,
    pub effective_model_profile_id: ModelProfileId,
    /// Provider-reported token usage for this call. `None` when the gateway
    /// could not surface real numbers (replay test stubs, providers without
    /// a usage object); downstream budget accounting falls back to the
    /// reservation estimate in that case.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<LoopModelUsage>,
}

/// Token usage reported by a provider for a single model call. The accountant
/// uses this to record actual USD spend instead of the conservative
/// reservation estimate.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoopModelUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelStreamChunk {
    pub safe_text_delta: String,
}

/// Redact credential-looking tokens before model deltas cross public/loggable
/// loop surfaces.
pub fn sanitize_model_visible_text(value: impl Into<String>) -> String {
    let value = value.into();
    let mut sanitized = String::with_capacity(value.len());
    let mut token = String::new();

    for character in value.chars() {
        if character.is_whitespace() {
            flush_sanitized_model_token(&mut sanitized, &mut token);
            sanitized.push(character);
        } else {
            token.push(character);
        }
    }
    flush_sanitized_model_token(&mut sanitized, &mut token);

    sanitized
}

fn flush_sanitized_model_token(sanitized: &mut String, token: &mut String) {
    if token.is_empty() {
        return;
    }
    if model_token_needs_redaction(token) {
        sanitized.push_str("[redacted]");
    } else {
        sanitized.push_str(token);
    }
    token.clear();
}

fn model_token_needs_redaction(token: &str) -> bool {
    let normalized = token
        .trim_matches(|character: char| !character.is_ascii_alphanumeric() && character != '_')
        .to_ascii_lowercase();
    normalized.starts_with("sk-")
        || normalized.contains("api_key")
        || normalized.contains("access_token")
        || normalized.contains("raw_credential_sentinel")
        || normalized.contains("raw_provider_secret")
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParentLoopOutput {
    AssistantReply(AssistantReply),
    CapabilityCalls(Vec<CapabilityCallCandidate>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssistantReply {
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityCallCandidate {
    /// Stable activity identity assigned before capability dispatch. Hosts use
    /// this as the runtime invocation identity, and tokenless gate checkpoints
    /// persist it so terminal events can close the same activity.
    pub activity_id: CapabilityActivityId,
    pub surface_version: CapabilitySurfaceVersion,
    pub capability_id: CapabilityId,
    pub input_ref: CapabilityInputRef,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effective_capability_ids: Vec<CapabilityId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_replay: Option<ProviderToolCallReplay>,
}

/// Capability ids a provider tool call may touch before it is staged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderToolCallCapabilityIds {
    /// Canonical capability id backing the provider-facing tool name.
    pub provider_capability_id: CapabilityId,
    /// Capabilities whose policy surface is used by this call.
    pub effective_capability_ids: Vec<CapabilityId>,
}

impl ProviderToolCallCapabilityIds {
    pub fn single(capability_id: CapabilityId) -> Self {
        Self {
            provider_capability_id: capability_id.clone(),
            effective_capability_ids: vec![capability_id],
        }
    }
}

/// Provider-originated tool-call metadata needed to replay tool results back to the same provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderToolCallReplay {
    /// Provider identity selected by the host route.
    pub provider_id: String,
    /// Concrete provider model selected by the host route.
    pub provider_model_id: String,
    /// Provider turn grouping token for reconstructing assistant tool calls.
    pub provider_turn_id: String,
    /// Provider call id referenced by the matching tool result.
    pub provider_call_id: String,
    /// Provider-facing tool name advertised to the model.
    pub provider_tool_name: ProviderToolName,
    /// Provider-facing tool arguments captured from the model tool call.
    pub arguments: serde_json::Value,
    /// Provider response-level reasoning attached to the tool-call batch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_reasoning: Option<String>,
    /// Provider call-level reasoning attached to this tool call.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<String>,
    /// Opaque provider thought-signature metadata, not an IronClaw auth signature.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
}

impl ProviderToolCallReplay {
    pub fn from_tool_call(tool_call: ProviderToolCall, provider_turn_id: String) -> Self {
        Self {
            provider_id: tool_call.provider_id,
            provider_model_id: tool_call.provider_model_id,
            provider_turn_id,
            provider_call_id: tool_call.id,
            provider_tool_name: tool_call.name,
            arguments: tool_call.arguments,
            response_reasoning: tool_call.response_reasoning,
            reasoning: tool_call.reasoning,
            signature: tool_call.signature,
        }
    }
}

#[async_trait]
pub trait LoopModelPort: Send + Sync {
    async fn stream_model(
        &self,
        request: LoopModelRequest,
    ) -> Result<LoopModelResponse, AgentLoopHostError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct VisibleCapabilityRequest;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VisibleCapabilitySurface {
    pub version: CapabilitySurfaceVersion,
    pub descriptors: Vec<CapabilityDescriptorView>,
    /// Capability IDs the model may *invoke* this turn — the reachable authorized
    /// catalog — as distinct from `descriptors`, which is the *advertised* subset.
    ///
    /// Under progressive tool disclosure the advertised set (token economy) is a
    /// narrowed view, but the model can still reach catalog tools beyond it via
    /// the tool-call bridge / forgiving-direct path. Call-time authorization (the
    /// model-visible capability filter) must therefore validate against this
    /// wider "callable" set, while advertising and prompt rendering stay narrow.
    ///
    /// `None` means "same as `descriptors`" — no disclosure narrowing is in
    /// effect, so callable == advertised. `Some(_)` is an explicit callable set
    /// that may legitimately be empty (no callable capabilities this turn),
    /// which the sentinel-free encoding keeps distinct from the un-narrowed case.
    /// Producers that don't narrow leave this `None`; consumers fall back to
    /// `descriptors`.
    #[serde(default)]
    pub callable_capability_ids: Option<Vec<CapabilityId>>,
}

/// Concurrency hint for a capability surfaced to an agent loop driver.
///
/// Derived at the adapter boundary from the underlying
/// `CapabilityDescriptor.effects` Vec. The lower-layer `CapabilityDescriptor`
/// is NOT modified; `effects` remains the source of truth and the hint is a
/// computed projection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConcurrencyHint {
    /// Capability has no exclusive side effects; multiple invocations may run
    /// in parallel without ordering hazards.
    SafeForParallel,
    /// Capability must be invoked serially within a loop run — parallel
    /// invocation would violate ordering or isolation constraints.
    Exclusive,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityDescriptorView {
    pub capability_id: CapabilityId,
    pub provider: Option<ExtensionId>,
    pub runtime: RuntimeKind,
    pub safe_name: String,
    pub safe_description: String,
    pub concurrency_hint: ConcurrencyHint,
    #[serde(default)]
    pub parameters_schema: serde_json::Value,
}

/// Provider-facing tool definition derived from a visible IronClaw capability.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderToolDefinition {
    /// Canonical IronClaw capability id backing this provider tool.
    pub capability_id: CapabilityId,
    /// Provider-safe tool name sent to the model.
    pub name: ProviderToolName,
    /// Provider-safe tool description sent to the model.
    pub description: String,
    /// JSON object schema for provider tool arguments.
    pub parameters: serde_json::Value,
}

impl ProviderToolDefinition {
    pub fn from_parts(
        capability_id: CapabilityId,
        name: impl Into<String>,
        description: impl Into<String>,
        parameters: serde_json::Value,
    ) -> Result<Self, AgentLoopHostError> {
        let name = ProviderToolName::new(name.into()).map_err(provider_tool_name_error)?;
        Ok(Self::from_typed_parts(
            capability_id,
            name,
            description,
            parameters,
        ))
    }

    /// Builds a definition from a provider-safe name that has already passed
    /// [`ProviderToolName`] validation. Use [`Self::from_parts`] for raw
    /// provider names that still need validation.
    pub fn from_typed_parts(
        capability_id: CapabilityId,
        name: ProviderToolName,
        description: impl Into<String>,
        parameters: serde_json::Value,
    ) -> Self {
        Self {
            capability_id,
            name,
            description: description.into(),
            parameters,
        }
    }

    pub fn validate_name(name: &str) -> Result<ProviderToolName, AgentLoopHostError> {
        ProviderToolName::new(name).map_err(provider_tool_name_error)
    }
}

/// Tool call emitted by a provider-backed model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderToolCall {
    /// Provider identity selected by the host route.
    pub provider_id: String,
    /// Concrete provider model selected by the host route.
    pub provider_model_id: String,
    /// Provider turn grouping token for reconstructing assistant tool calls.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    /// Provider call id referenced by the matching tool result.
    pub id: String,
    /// Provider-facing tool name returned by the model.
    pub name: ProviderToolName,
    /// Provider-facing tool arguments returned by the model.
    pub arguments: serde_json::Value,
    /// Provider response-level reasoning attached to the tool-call batch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_reasoning: Option<String>,
    /// Provider call-level reasoning attached to this tool call.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<String>,
    /// Opaque provider thought-signature metadata, not an IronClaw auth signature.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
}

impl ProviderToolCall {
    pub fn from_parts(
        provider_id: impl Into<String>,
        provider_model_id: impl Into<String>,
        turn_id: Option<String>,
        id: impl Into<String>,
        name: impl Into<String>,
        arguments: serde_json::Value,
    ) -> Result<Self, AgentLoopHostError> {
        Ok(Self {
            provider_id: provider_id.into(),
            provider_model_id: provider_model_id.into(),
            turn_id,
            id: id.into(),
            name: ProviderToolName::new(name.into()).map_err(provider_tool_name_error)?,
            arguments,
            response_reasoning: None,
            reasoning: None,
            signature: None,
        })
    }
}

fn provider_tool_name_error(error: HostApiError) -> AgentLoopHostError {
    let detail = match error {
        HostApiError::InvalidId { reason, .. } => reason,
        other => other.to_string(),
    };
    AgentLoopHostError::new(
        AgentLoopHostErrorKind::InvalidInvocation,
        format!("tool name cannot be represented as a provider tool name: {detail}"),
    )
}

/// Durable reference to provider tool-call metadata for tool-result replay.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderToolCallReference {
    /// Provider identity selected by the host route.
    pub provider_id: String,
    /// Concrete provider model selected by the host route.
    pub provider_model_id: String,
    /// Provider turn grouping token for reconstructing assistant tool calls.
    pub provider_turn_id: String,
    /// Provider call id referenced by the matching tool result.
    pub provider_call_id: String,
    /// Provider-facing tool name returned by the model.
    pub provider_tool_name: ProviderToolName,
    /// Canonical IronClaw capability id backing this provider tool.
    pub capability_id: CapabilityId,
    /// Provider-facing tool arguments returned by the model.
    pub arguments: serde_json::Value,
    /// Provider response-level reasoning attached to the tool-call batch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_reasoning: Option<String>,
    /// Provider call-level reasoning attached to this tool call.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<String>,
    /// Opaque provider thought-signature metadata, not an IronClaw auth signature.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegisterProviderToolCallRequest {
    pub tool_call: ProviderToolCall,
    /// Activity identity to bind to this provider call. When set, the host
    /// must register the call with this id, rejecting if the same input_ref was
    /// already registered with another id. When absent, the host creates an id
    /// for the first registration and returns that same id for duplicate
    /// registrations of the same input_ref.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activity_id: Option<CapabilityActivityId>,
}

impl RegisterProviderToolCallRequest {
    pub fn new(tool_call: ProviderToolCall) -> Self {
        Self {
            tool_call,
            activity_id: None,
        }
    }

    pub fn for_activity(tool_call: ProviderToolCall, activity_id: CapabilityActivityId) -> Self {
        Self {
            tool_call,
            activity_id: Some(activity_id),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityInvocation {
    /// Stable activity identity for this invocation. Runtime hosts derive
    /// their execution identity from it rather than minting a second id.
    pub activity_id: CapabilityActivityId,
    pub surface_version: CapabilitySurfaceVersion,
    pub capability_id: CapabilityId,
    pub input_ref: CapabilityInputRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval_resume: Option<CapabilityApprovalResume>,
    /// Set when the invocation was previously auth-blocked and the auth
    /// gate has now been resolved. Carries the original activity token so
    /// re-dispatch reuses it rather than minting a new one, preserving any
    /// prior approval lease whose scope embeds that id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_resume: Option<CapabilityAuthResume>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct CapabilityResumeToken(String);

impl CapabilityResumeToken {
    pub fn new(value: impl Into<String>) -> Result<Self, String> {
        validate_bounded_loop_string(value.into(), "capability resume token", 128).map(Self)
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl AsRef<str> for CapabilityResumeToken {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl std::fmt::Display for CapabilityResumeToken {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for CapabilityResumeToken {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityApprovalResume {
    pub approval_request_id: ApprovalRequestId,
    pub resume_token: CapabilityResumeToken,
    #[serde(default = "CorrelationId::new")]
    pub correlation_id: CorrelationId,
    pub input_ref: CapabilityInputRef,
    pub input: serde_json::Value,
    pub estimate: ResourceEstimate,
}

/// Prior-approval identity carried through an auth-gate resume.
///
/// Both fields are semantically all-or-none: the pair is present only when
/// the invocation previously passed a one-shot approval gate.  Modelling
/// them as a single optional struct makes the compile-time invariant explicit —
/// `approval_request_id` and `correlation_id` cannot be independently absent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthResumeApprovalIdentity {
    /// Identifies the prior approval request so the host can locate and
    /// claim the matching fingerprinted lease without requiring a second
    /// human approval for the same action.
    pub approval_request_id: ApprovalRequestId,
    /// Original correlation identifier from the prior approval gate.
    /// Restored onto the invocation context so the same trace-correlation
    /// identifier flows through the full capability lifecycle.
    pub correlation_id: CorrelationId,
}

/// Auth-gate resume identity.
///
/// Carries the original activity identity (encoded as a resume token) so
/// that re-dispatch after credential completion reuses the same activity
/// rather than minting a fresh one.  When the prior invocation also passed
/// an approval gate, `prior_approval` carries the approval identity so the
/// host can claim the matching fingerprinted lease without requiring a second
/// human approval.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityAuthResume {
    /// Encodes the original activity identity so the host can reuse the
    /// matching execution context after auth completes.
    pub resume_token: CapabilityResumeToken,
    /// Present when the invocation previously passed a one-shot approval gate.
    /// The two sub-fields are always set together; see [`AuthResumeApprovalIdentity`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prior_approval: Option<AuthResumeApprovalIdentity>,
    /// Original runtime input captured when the auth gate was produced.
    ///
    /// Capability input refs are scoped to a loop run and may be consumed by the
    /// first dispatch before the auth gate is resolved. When present, this
    /// replay payload lets auth-resume re-dispatch without resolving a stale or
    /// already-consumed input ref.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replay: Option<CapabilityAuthResumeReplay>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityAuthResumeReplay {
    pub input: serde_json::Value,
    pub estimate: ResourceEstimate,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityBatchInvocation {
    pub invocations: Vec<CapabilityInvocation>,
    pub stop_on_first_suspension: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityBatchOutcome {
    pub outcomes: Vec<CapabilityOutcome>,
    pub stopped_on_suspension: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityOutcome {
    Completed(CapabilityResultMessage),
    ApprovalRequired {
        gate_ref: LoopGateRef,
        safe_summary: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        approval_resume: Option<CapabilityApprovalResume>,
    },
    AuthRequired {
        gate_ref: LoopGateRef,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        credential_requirements: Vec<RuntimeCredentialAuthRequirement>,
        safe_summary: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        auth_resume: Option<CapabilityAuthResume>,
    },
    ResourceBlocked {
        gate_ref: LoopGateRef,
        safe_summary: String,
    },
    /// The model called a client-supplied ("external") tool. The host does not
    /// execute it: the run parks and control returns to the API client, which
    /// resumes by submitting the tool output. Carries only the opaque gate ref
    /// and a bounded safe summary (never the raw caller tool args or output).
    ExternalToolPending {
        gate_ref: LoopGateRef,
        safe_summary: String,
    },
    SpawnedProcess(ProcessHandleSummary),
    AwaitDependentRun {
        gate_ref: LoopGateRef,
        result_ref: LoopResultRef,
        safe_summary: String,
        /// Size in bytes of the payload staged at `result_ref` time
        /// (i.e. the serialized capability output, not the size of this struct).
        /// Propagated from LoopCapabilityResultWriter::write_capability_result.
        /// Used by ByteCapStrategy to evaluate per-capability byte caps.
        #[serde(default)]
        byte_len: u64,
    },
    SpawnedChildRun {
        child_run_id: TurnRunId,
        result_ref: LoopResultRef,
        safe_summary: String,
        /// Size in bytes of the payload staged at `result_ref` time
        /// (i.e. the serialized capability output, not the size of this struct).
        /// Same semantics as AwaitDependentRun.byte_len.
        #[serde(default)]
        byte_len: u64,
    },
    Denied(CapabilityDenied),
    Failed(CapabilityFailure),
}

impl CapabilityOutcome {
    pub fn is_suspension(&self) -> bool {
        matches!(
            self,
            Self::ApprovalRequired { .. }
                | Self::AuthRequired { .. }
                | Self::ResourceBlocked { .. }
                | Self::ExternalToolPending { .. }
                | Self::AwaitDependentRun { .. }
                | Self::SpawnedProcess(_)
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityResultMessage {
    pub result_ref: LoopResultRef,
    pub safe_summary: String,
    /// Typed host signal describing whether this result advanced the loop's
    /// evidence/state. This lets the loop distinguish deterministic
    /// no-change outcomes from productive calls without inferring progress
    /// from prose summaries or token counts.
    #[serde(default)]
    pub progress: CapabilityProgress,
    /// Host hint that this completed capability result should end the loop
    /// naturally after the current batch. Defaults to false for compatibility
    /// with older hosts.
    #[serde(default)]
    pub terminate_hint: bool,
    /// Serialized output size in bytes — pure metadata, no PII.
    #[serde(default)]
    pub byte_len: u64,
    /// Digest over normalized output content. Optional for backward
    /// compatibility and for synthetic results that do not stage real output.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_digest: Option<ContentDigest>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityProgress {
    /// Older hosts, or hosts that cannot classify progress yet.
    #[default]
    Unknown,
    /// The capability produced new evidence or changed host/runtime state.
    #[serde(alias = "complete")]
    MadeProgress,
    /// The capability ran successfully but observed the same state/evidence as
    /// before.
    NoChange,
    /// The capability reached a deterministic non-suspending blocker.
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessHandleSummary {
    pub process_ref: LoopProcessRef,
    pub safe_summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityDenied {
    pub reason_kind: CapabilityDeniedReasonKind,
    pub safe_summary: String,
}

#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CapabilityDeniedReasonKind {
    EmptySurface,
    Unknown(CapabilityDeniedReasonKindValue),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CapabilityDeniedReasonKindValue(String);

impl CapabilityDeniedReasonKindValue {
    pub fn new(value: impl Into<String>) -> Result<Self, String> {
        validate_loop_safe_identifier(value.into(), "capability denied reason kind", 128).map(Self)
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl CapabilityDeniedReasonKind {
    pub fn unknown(value: impl Into<String>) -> Result<Self, String> {
        CapabilityDeniedReasonKindValue::new(value).map(Self::Unknown)
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::EmptySurface => "empty_surface",
            Self::Unknown(value) => value.as_str(),
        }
    }
}

impl std::fmt::Display for CapabilityDeniedReasonKind {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl Serialize for CapabilityDeniedReasonKind {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for CapabilityDeniedReasonKind {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        match value.as_str() {
            "empty_surface" => Ok(Self::EmptySurface),
            _ => Self::unknown(value).map_err(serde::de::Error::custom),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityFailure {
    pub error_kind: CapabilityFailureKind,
    pub safe_summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<CapabilityFailureDetail>,
}

// Deliberately NOT `#[non_exhaustive]`: the `Unknown(CapabilityFailureKindValue)`
// variant is the forward-compat / open-set escape hatch (a newer producer's
// unrecognized wire string deserializes into `Unknown`), and the manual
// `Serialize`/`Deserialize` impls below route every value through `as_str()` /
// that variant. Leaving the attribute on would force callers — notably the
// recovery classifier `capability_error_class` — to keep a wildcard `_ =>` arm,
// which silently buckets any newly-added *named* variant (e.g. a future
// `QuotaExceeded`) into a run-aborting class. Without the attribute, those
// classifiers match exhaustively, so a new named variant fails to compile until
// it is deliberately classified. See
// `docs/plans/2026-06-28-reborn-error-recoverability-audit.md` §6.1.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CapabilityFailureKind {
    Authorization,
    Backend,
    Cancelled,
    Dispatcher,
    GateDeclined,
    InvalidInput,
    InvalidOutput,
    MissingRuntime,
    Network,
    OperationFailed,
    OutputTooLarge,
    PolicyDenied,
    Process,
    Resource,
    Transient,
    Unavailable,
    Internal,
    Permanent,
    Unknown(CapabilityFailureKindValue),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CapabilityFailureKindValue(String);

impl CapabilityFailureKindValue {
    pub fn new(value: impl Into<String>) -> Result<Self, String> {
        validate_loop_safe_identifier(value.into(), "capability failure kind", 128).map(Self)
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl CapabilityFailureKind {
    pub fn unknown(value: impl Into<String>) -> Result<Self, String> {
        CapabilityFailureKindValue::new(value).map(Self::Unknown)
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::Authorization => "authorization",
            Self::Backend => "backend",
            Self::Cancelled => "cancelled",
            Self::Dispatcher => "dispatcher",
            Self::GateDeclined => "gate_declined",
            Self::InvalidInput => "invalid_input",
            Self::InvalidOutput => "invalid_output",
            Self::MissingRuntime => "missing_runtime",
            Self::Network => "network",
            Self::OperationFailed => "operation_failed",
            Self::OutputTooLarge => "output_too_large",
            Self::PolicyDenied => "policy_denied",
            Self::Process => "process",
            Self::Resource => "resource",
            Self::Transient => "transient",
            Self::Unavailable => "unavailable",
            Self::Internal => "internal",
            Self::Permanent => "permanent",
            Self::Unknown(value) => value.as_str(),
        }
    }
}

impl std::fmt::Display for CapabilityFailureKind {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl Serialize for CapabilityFailureKind {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for CapabilityFailureKind {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        match value.as_str() {
            "authorization" => Ok(Self::Authorization),
            "backend" => Ok(Self::Backend),
            "cancelled" => Ok(Self::Cancelled),
            "dispatcher" => Ok(Self::Dispatcher),
            "gate_declined" => Ok(Self::GateDeclined),
            "invalid_input" => Ok(Self::InvalidInput),
            "invalid_output" => Ok(Self::InvalidOutput),
            "missing_runtime" => Ok(Self::MissingRuntime),
            "network" => Ok(Self::Network),
            "operation_failed" => Ok(Self::OperationFailed),
            "output_too_large" => Ok(Self::OutputTooLarge),
            "policy_denied" => Ok(Self::PolicyDenied),
            "process" => Ok(Self::Process),
            "resource" => Ok(Self::Resource),
            "transient" => Ok(Self::Transient),
            "unavailable" => Ok(Self::Unavailable),
            "internal" => Ok(Self::Internal),
            "permanent" => Ok(Self::Permanent),
            _ => Self::unknown(value).map_err(serde::de::Error::custom),
        }
    }
}

#[async_trait]
pub trait LoopCapabilityPort: Send + Sync {
    fn tool_definitions(&self) -> Result<Vec<ProviderToolDefinition>, AgentLoopHostError> {
        Ok(Vec::new())
    }

    fn provider_tool_call_capability_ids(
        &self,
        tool_call: &ProviderToolCall,
    ) -> Result<ProviderToolCallCapabilityIds, AgentLoopHostError> {
        let Some(definition) = self
            .tool_definitions()?
            .into_iter()
            .find(|definition| definition.name == tool_call.name)
        else {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::InvalidInvocation,
                "provider tool call is outside the visible capability surface",
            ));
        };
        Ok(ProviderToolCallCapabilityIds::single(
            definition.capability_id,
        ))
    }

    fn validate_provider_tool_call(
        &self,
        _tool_call: &ProviderToolCall,
    ) -> Result<(), AgentLoopHostError> {
        Ok(())
    }

    async fn register_provider_tool_call(
        &self,
        _request: RegisterProviderToolCallRequest,
    ) -> Result<CapabilityCallCandidate, AgentLoopHostError> {
        Err(unsupported_host_method("register_provider_tool_call"))
    }

    async fn visible_capabilities(
        &self,
        request: VisibleCapabilityRequest,
    ) -> Result<VisibleCapabilitySurface, AgentLoopHostError>;

    fn current_visible_capabilities(
        &self,
    ) -> Result<Option<VisibleCapabilitySurface>, AgentLoopHostError> {
        Ok(None)
    }

    async fn invoke_capability(
        &self,
        request: CapabilityInvocation,
    ) -> Result<CapabilityOutcome, AgentLoopHostError>;

    async fn invoke_capability_batch(
        &self,
        request: CapabilityBatchInvocation,
    ) -> Result<CapabilityBatchOutcome, AgentLoopHostError>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BeginAssistantDraft {
    pub reply: AssistantReply,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateAssistantDraft {
    pub message_ref: LoopMessageRef,
    pub reply: AssistantReply,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FinalizeAssistantMessage {
    pub reply: AssistantReply,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppendCapabilityResultRef {
    pub result_ref: LoopResultRef,
    pub safe_summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_call: Option<ProviderToolCallReference>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_observation: Option<ModelVisibleToolObservation>,
}

#[async_trait]
pub trait LoopTranscriptPort: Send + Sync {
    async fn begin_assistant_draft(
        &self,
        _request: BeginAssistantDraft,
    ) -> Result<LoopMessageRef, AgentLoopHostError> {
        Err(unsupported_host_method("begin_assistant_draft"))
    }

    async fn update_assistant_draft(
        &self,
        _request: UpdateAssistantDraft,
    ) -> Result<(), AgentLoopHostError> {
        Err(unsupported_host_method("update_assistant_draft"))
    }

    async fn finalize_assistant_message(
        &self,
        request: FinalizeAssistantMessage,
    ) -> Result<LoopMessageRef, AgentLoopHostError>;

    async fn append_capability_result_ref(
        &self,
        _request: AppendCapabilityResultRef,
    ) -> Result<LoopMessageRef, AgentLoopHostError> {
        Err(unsupported_host_method("append_capability_result_ref"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoopCheckpointRequest {
    pub kind: LoopCheckpointKind,
    pub state_ref: LoopCheckpointStateRef,
    /// Gate identity for `BeforeBlock` checkpoints; `None` for other kinds.
    /// Defaults to `None` for backward-compatible deserialization of older
    /// records that predate this field.
    #[serde(default)]
    pub gate_ref: Option<crate::ids::LoopGateRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoadCheckpointPayloadRequest {
    pub checkpoint_id: TurnCheckpointId,
    pub expected_schema_id: CheckpointSchemaId,
    pub expected_schema_version: RunProfileVersion,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedCheckpointPayload {
    pub kind: LoopCheckpointKind,
    pub schema_id: CheckpointSchemaId,
    pub schema_version: RunProfileVersion,
    pub payload: RedactedCheckpointPayload,
}

/// Request to stage a checkpoint payload's raw bytes before calling
/// [`LoopCheckpointPort::checkpoint`] with the resulting state ref.
///
/// The two-step write keeps byte-storage and metadata-write responsibilities
/// cleanly split.
///
/// `kind` is required so adapters that bridge to
/// `CheckpointStateStore::put_checkpoint_state` can persist the correct kind
/// without having to guess. The subsequent `checkpoint(kind, state_ref)` call
/// must use the same `kind`; the read-side `get_checkpoint_state` validates
/// the staged kind against the metadata write's kind.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StageCheckpointPayloadRequest {
    /// Checkpoint boundary the staged payload belongs to. Must match the
    /// `kind` passed to the subsequent `LoopCheckpointPort::checkpoint(...)`
    /// call.
    pub kind: LoopCheckpointKind,
    /// Schema id of the payload — usually the framework's
    /// `CHECKPOINT_SCHEMA_ID` constant. Stored alongside the bytes so the
    /// read-side can authenticate the boundary on resume.
    pub schema_id: String,
    /// Canonical payload bytes (e.g. `serde_json::to_vec(&state)`). The
    /// implementation does not parse the bytes; it persists them and returns
    /// an opaque ref.
    pub payload: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopCheckpointKind {
    BeforeModel,
    BeforeSideEffect,
    BeforeBlock,
    Final,
}

impl LoopCheckpointKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::BeforeModel => "before_model",
            Self::BeforeSideEffect => "before_side_effect",
            Self::BeforeBlock => "before_block",
            Self::Final => "final",
        }
    }
}

#[async_trait]
pub trait LoopCheckpointPort: Send + Sync {
    async fn checkpoint(
        &self,
        request: LoopCheckpointRequest,
    ) -> Result<TurnCheckpointId, AgentLoopHostError>;

    /// Stage a checkpoint payload's raw bytes and return an opaque
    /// [`LoopCheckpointStateRef`] that subsequent `checkpoint(...)` calls
    /// can reference. The default impl fails closed; concrete impls live in
    /// `ironclaw_loop_support` and wrap the host's `CheckpointStateStore`.
    ///
    /// The executor's checkpoint helper calls this method before invoking
    /// `LoopCheckpointPort::checkpoint(...)` so the metadata write references
    /// a payload that's already durably stored.
    async fn stage_checkpoint_payload(
        &self,
        _request: StageCheckpointPayloadRequest,
    ) -> Result<LoopCheckpointStateRef, AgentLoopHostError> {
        Err(AgentLoopHostError::new(
            AgentLoopHostErrorKind::Unavailable,
            "stage_checkpoint_payload not implemented",
        ))
    }

    /// Load the redacted state payload behind a previously-written
    /// checkpoint. Resume callers go through this host port so metadata
    /// validation stays with the backend that owns checkpoint storage.
    async fn load_checkpoint_payload(
        &self,
        _request: LoadCheckpointPayloadRequest,
    ) -> Result<LoadedCheckpointPayload, AgentLoopHostError> {
        Err(unsupported_host_method("load_checkpoint_payload"))
    }
}

#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopProgressEvent {
    DriverNote {
        kind: LoopDriverNoteKind,
        safe_summary: LoopSafeSummary,
    },
    IterationStarted {
        iteration: u32,
    },
    PromptBundleBuilt {
        iteration: u32,
        bundle_ref: LoopPromptBundleRef,
        mode: PromptMode,
        surface_version: Option<CapabilitySurfaceVersion>,
        message_count: u32,
        identity_message_count: u32,
        instruction_snippet_count: u32,
    },
    CapabilityBatchStarted {
        iteration: u32,
        call_count: u32,
        policy: BatchPolicyKind,
    },
    CapabilityBatchCompleted {
        iteration: u32,
        result_count: u32,
        denied_count: u32,
        gated_count: u32,
        failed_count: u32,
    },
    CapabilityActivityFailed {
        activity_id: CapabilityActivityId,
        capability_id: CapabilityId,
        reason_kind: CapabilityFailureKind,
        /// Bounded, host-authored sanitized failure summary (e.g. a builtin's
        /// `"invalid JSON: ..."` message) so the live per-tool UI card can show
        /// the real reason, not just the kind. Additive; `None` when no
        /// host-authored summary is available.
        safe_summary: Option<String>,
    },
    GateBlocked {
        iteration: u32,
        gate_kind: LoopGateKind,
    },
    CheckpointWritten {
        iteration: u32,
        kind: LoopCheckpointKind,
    },
    CompactionStarted {
        task_id: SystemInferenceTaskId,
        initiator: CompactionInitiator,
    },
    CompactionCompleted {
        task_id: SystemInferenceTaskId,
        compression_ratio_ppm: u32,
    },
    CompactionFailed {
        task_id: SystemInferenceTaskId,
        reason_kind: LoopSafeSummary,
    },
    CompactionLeakDetected {
        task_id: SystemInferenceTaskId,
        reason_kind: LoopSafeSummary,
    },
    GoalRefreshStarted {
        task_id: SystemInferenceTaskId,
    },
    GoalRefreshCompleted {
        task_id: SystemInferenceTaskId,
    },
    GoalRefreshFailed {
        task_id: SystemInferenceTaskId,
        reason_kind: LoopSafeSummary,
    },
    GoalRefreshLeakDetected {
        task_id: SystemInferenceTaskId,
        reason_kind: LoopSafeSummary,
    },
}

impl LoopProgressEvent {
    pub fn driver_note(
        kind: LoopDriverNoteKind,
        safe_summary: impl Into<String>,
    ) -> Result<Self, String> {
        Ok(Self::DriverNote {
            kind,
            safe_summary: LoopSafeSummary::new(safe_summary)?,
        })
    }

    pub fn kind_name(&self) -> &'static str {
        match self {
            Self::DriverNote { .. } => "driver_note",
            Self::IterationStarted { .. } => "iteration_started",
            Self::PromptBundleBuilt { .. } => "prompt_bundle_built",
            Self::CapabilityBatchStarted { .. } => "capability_batch_started",
            Self::CapabilityBatchCompleted { .. } => "capability_batch_completed",
            Self::CapabilityActivityFailed { .. } => "capability_activity_failed",
            Self::GateBlocked { .. } => "gate_blocked",
            Self::CheckpointWritten { .. } => "checkpoint_written",
            Self::CompactionStarted { .. } => "compaction_started",
            Self::CompactionCompleted { .. } => "compaction_completed",
            Self::CompactionFailed { .. } => "compaction_failed",
            Self::CompactionLeakDetected { .. } => "compaction_leak_detected",
            Self::GoalRefreshStarted { .. } => "goal_refresh_started",
            Self::GoalRefreshCompleted { .. } => "goal_refresh_completed",
            Self::GoalRefreshFailed { .. } => "goal_refresh_failed",
            Self::GoalRefreshLeakDetected { .. } => "goal_refresh_leak_detected",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BatchPolicyKind {
    Sequential,
    Parallel,
}

#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopGateKind {
    Approval,
    Auth,
    ResourceWait,
    AwaitDependentRun,
    ExternalTool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopDriverNoteKind {
    Context,
    Planning,
    Waiting,
    Retrying,
    /// An event-triggered hook subscription stopped before the run did —
    /// typically because the durable event log reported a replay gap that
    /// the subscription cannot bridge without losing events. Surfaced as
    /// an operator-visible note so the missing telemetry isn't silently
    /// invisible (NOTE(#3640)).
    EventSubscriptionTerminated,
}

#[async_trait]
pub trait LoopProgressPort: Send + Sync {
    /// Emit observational progress for UI/status consumers.
    ///
    /// Progress events are best-effort and must not be used as
    /// recoverability-critical durability markers. A failed progress emission
    /// must not invalidate already-completed durable work; callers should treat
    /// this like host model milestone projection, where sink failures are
    /// logged/observed without changing the provider or checkpoint outcome.
    async fn emit_loop_progress(&self, event: LoopProgressEvent) -> Result<(), AgentLoopHostError>;
}

/// Per-run cancellation observation point.
///
/// The canonical executor consults this between strategy calls. The method is
/// intentionally synchronous and non-blocking: implementations should expose a
/// cheap snapshot, usually backed by an atomic flag plus immutable signal data.
///
/// Cancellation is cooperative. Most executor stages observe it only at
/// explicit boundaries via [`LoopCancellationPort::observe_cancellation`].
/// Executor-owned waits that can safely race host work, such as prompt
/// compaction, may also wait on
/// [`LoopCancellationPort::cancellation_requested`] to avoid timer polling.
#[async_trait]
pub trait LoopCancellationPort: Send + Sync {
    /// Returns `Some(signal)` once cancellation has been requested for this run.
    ///
    /// Implementations must be idempotent across reads. After the request fires,
    /// repeated calls must keep returning the same signal.
    fn observe_cancellation(&self) -> Option<LoopCancellationSignal>;

    /// Waits until cancellation has been requested for this run and returns the
    /// same stable signal reported by [`Self::observe_cancellation`].
    async fn cancellation_requested(&self) -> LoopCancellationSignal;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoopCancellationSignal {
    pub reason_kind: LoopCancelReasonKind,
    pub requested_at: DateTime<Utc>,
}

pub trait AgentLoopDriverHost:
    LoopRunInfoPort
    + LoopContextPort
    + LoopPromptPort
    + LoopInputPort
    + LoopModelPort
    + LoopCapabilityPort
    + LoopTranscriptPort
    + LoopCheckpointPort
    + LoopProgressPort
    + LoopCompactionPort
    + LoopCancellationPort
    + Send
    + Sync
{
}

impl<T> AgentLoopDriverHost for T where
    T: LoopRunInfoPort
        + LoopContextPort
        + LoopPromptPort
        + LoopInputPort
        + LoopModelPort
        + LoopCapabilityPort
        + LoopTranscriptPort
        + LoopCheckpointPort
        + LoopProgressPort
        + LoopCompactionPort
        + LoopCancellationPort
        + Send
        + Sync
{
}

fn unsupported_host_method(method: &'static str) -> AgentLoopHostError {
    AgentLoopHostError::new(
        AgentLoopHostErrorKind::Unavailable,
        format!("agent loop host method {method} is unavailable"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    struct DefinitionPort {
        definitions: Vec<ProviderToolDefinition>,
    }

    #[async_trait]
    impl LoopCapabilityPort for DefinitionPort {
        fn tool_definitions(&self) -> Result<Vec<ProviderToolDefinition>, AgentLoopHostError> {
            Ok(self.definitions.clone())
        }

        async fn visible_capabilities(
            &self,
            _request: VisibleCapabilityRequest,
        ) -> Result<VisibleCapabilitySurface, AgentLoopHostError> {
            unreachable!("not used by this test")
        }

        async fn invoke_capability(
            &self,
            _request: CapabilityInvocation,
        ) -> Result<CapabilityOutcome, AgentLoopHostError> {
            unreachable!("not used by this test")
        }

        async fn invoke_capability_batch(
            &self,
            _request: CapabilityBatchInvocation,
        ) -> Result<CapabilityBatchOutcome, AgentLoopHostError> {
            unreachable!("not used by this test")
        }
    }

    fn provider_tool_call(name: &str) -> ProviderToolCall {
        ProviderToolCall {
            provider_id: "provider".to_string(),
            provider_model_id: "model".to_string(),
            turn_id: Some("turn".to_string()),
            id: "call".to_string(),
            name: ProviderToolName::new(name).expect("provider tool name"),
            arguments: serde_json::json!({}),
            response_reasoning: None,
            reasoning: None,
            signature: None,
        }
    }

    #[test]
    fn provider_tool_call_capability_ids_rejects_unknown_tool_name() {
        let port = DefinitionPort {
            definitions: vec![ProviderToolDefinition {
                capability_id: CapabilityId::new("demo.allowed").expect("valid capability id"),
                name: ProviderToolName::new("demo__allowed").expect("provider tool name"),
                description: "allowed".to_string(),
                parameters: serde_json::json!({"type": "object"}),
            }],
        };

        let error = port
            .provider_tool_call_capability_ids(&provider_tool_call("demo__missing"))
            .expect_err("unknown provider tool must fail closed");

        assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
    }

    #[test]
    fn capability_failure_summary_redacts_secret_like_tokens() {
        for raw_summary in [
            "provider returned AKIAIOSFODNN7EXAMPLE",
            "provider returned gcp-live-secret",
            "provider returned sk-ant-live-secret",
            "provider returned ghp_live_secret",
            "provider returned github_pat_live_secret",
        ] {
            let summary = LoopSafeSummary::capability_failure_summary(raw_summary);
            assert_eq!(
                summary.as_str(),
                "the tool failure details were redacted",
                "summary must be redacted: {raw_summary}"
            );
        }
    }

    #[test]
    fn safe_summary_accepts_ordinary_error_vocabulary() {
        // Words that used to be banned outright are ordinary error vocabulary,
        // not secrets, and must now be accepted.
        for accepted in [
            "provider error occurred during the call",
            "stack trace was captured for diagnosis",
            "the tool input was malformed",
            "a traceback is available for review",
            "host path resolution did not complete",
            "raw runtime returned an unexpected status",
        ] {
            validate_loop_safe_summary(accepted.to_string())
                .unwrap_or_else(|error| panic!("`{accepted}` should be accepted: {error}"));
        }
    }

    #[test]
    fn loop_safe_summary_accepts_fixed_input_encode_summary() {
        let summary = LoopSafeSummary::new(INPUT_ENCODE_HUMAN_SUMMARY)
            .expect("fixed host-authored input encode summary is safe");
        assert_eq!(summary.as_str(), INPUT_ENCODE_HUMAN_SUMMARY);
    }

    #[test]
    fn safe_summary_still_rejects_secret_markers_and_delimiters() {
        // Credential markers must still be rejected.
        for rejected in [
            "leaked sk-LIVEsecretvalue token",
            "authorization header bearer abc123",
            "the api key was exposed",
            "user password was logged",
            "a secret slipped into the message",
        ] {
            validate_loop_safe_summary(rejected.to_string())
                .expect_err(&format!("`{rejected}` must still be rejected"));
        }

        // Path / payload delimiters must still be rejected.
        validate_loop_safe_summary("missing schema at /system/extensions".to_string())
            .expect_err("path delimiter `/` must still be rejected");
    }

    #[test]
    fn agent_loop_host_error_carries_optional_detail() {
        let path = "missing input_schema_ref at /system/extensions/google-calendar/list_calendars.input.v1.json";
        let error = AgentLoopHostError::new(
            AgentLoopHostErrorKind::InvalidInvocation,
            "host runtime rejected capability request",
        )
        .with_detail(path);
        assert_eq!(error.detail.as_deref(), Some(path));

        let plain = AgentLoopHostError::new(AgentLoopHostErrorKind::Internal, "boom");
        assert_eq!(plain.detail, None);
    }
}
