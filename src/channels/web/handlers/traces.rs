//! Trace contribution API handlers.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::channels::web::auth::AuthenticatedUser;
use crate::channels::web::server::GatewayState;
use crate::trace_contribution::{
    ConsentScope, CreditSummary, DeterministicTraceRedactor, LocalTraceSubmissionRecord,
    RawTraceContribution, RecordedTraceContributionOptions, StandingTraceContributionPolicy,
    TRACE_CREDIT_NOTICE_MAX_SNOOZE_HOURS, TraceChannel, TraceContributionAcceptance,
    TraceContributionEnvelope, TraceContributionPolicyRejection, TraceCreditReport,
    TraceQueueDiagnostics, TraceQueueFlushReport, TraceQueueHold, TraceRedactor,
    acknowledge_trace_credit_notice_for_scope, apply_credit_estimate_to_envelope,
    capture_turns_from_conversation_messages, flush_trace_contribution_queue_for_scope,
    local_pseudonymous_contributor_id, local_pseudonymous_tenant_scope_ref,
    mark_trace_credit_notice_due_for_scope, preflight_trace_contribution_policy,
    queue_trace_envelope_for_scope, queued_trace_envelope_paths_for_scope,
    read_local_trace_records_for_scope, read_trace_policy_for_scope,
    read_trace_queue_holds_for_scope, revoke_trace_submission_for_scope_with_policy,
    snooze_trace_credit_notice_for_scope, sync_remote_trace_submission_records_for_scope,
    trace_credit_report, trace_credit_summary, trace_queue_diagnostics_for_scope,
    write_trace_policy_for_scope,
};

#[derive(Debug, Deserialize)]
pub struct TracePolicyRequest {
    pub enabled: Option<bool>,
    pub endpoint: Option<String>,
    pub bearer_token_env: Option<String>,
    pub upload_token_issuer_url: Option<String>,
    pub upload_token_issuer_allowed_hosts: Option<Vec<String>>,
    pub upload_token_audience: Option<String>,
    pub upload_token_tenant_id: Option<String>,
    pub upload_token_workload_token_env: Option<String>,
    pub upload_token_issuer_timeout_ms: Option<u64>,
    pub include_message_text: Option<bool>,
    pub include_tool_payloads: Option<bool>,
    pub auto_submit_failed_traces: Option<bool>,
    pub auto_submit_high_value_traces: Option<bool>,
    pub selected_tools: Option<Vec<String>>,
    pub require_manual_approval_when_pii_detected: Option<bool>,
    pub min_submission_score: Option<f32>,
    pub credit_notice_interval_hours: Option<u32>,
    pub default_scope: Option<ConsentScope>,
}

#[derive(Debug, Serialize)]
pub struct TracePolicyResponse {
    pub policy: StandingTraceContributionPolicy,
    pub queued_envelopes: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub held_queue: Vec<TraceQueueHold>,
}

#[derive(Debug, Deserialize)]
pub struct TraceContributionPreviewRequest {
    pub thread_id: Option<Uuid>,
    #[serde(default)]
    pub include_message_text: bool,
    #[serde(default)]
    pub include_tool_payloads: bool,
    pub include_turn_count: Option<usize>,
    pub scope: Option<ConsentScope>,
    #[serde(default)]
    pub enqueue: bool,
}

#[derive(Debug, Deserialize)]
pub struct TraceContributionSubmitRequest {
    pub thread_id: Option<Uuid>,
    #[serde(default)]
    pub include_message_text: bool,
    #[serde(default)]
    pub include_tool_payloads: bool,
    pub include_turn_count: Option<usize>,
    pub scope: Option<ConsentScope>,
    pub user_previewed: bool,
    #[serde(default)]
    pub flush: bool,
}

#[derive(Debug, Serialize)]
pub struct TraceContributionPreviewResponse {
    pub submission_id: Uuid,
    pub queued: bool,
    pub privacy_risk: String,
    pub redaction_counts: BTreeMap<String, u32>,
    pub preview_markdown: String,
    pub envelope: TraceContributionEnvelope,
}

#[derive(Debug, Serialize)]
pub struct TraceContributionSubmitResponse {
    pub submission_id: Uuid,
    pub queued: bool,
    pub flushed: bool,
    pub privacy_risk: String,
    pub redaction_counts: BTreeMap<String, u32>,
    pub flush_report: Option<TraceQueueFlushReport>,
}

#[derive(Debug, Deserialize)]
pub struct TraceQueueFlushQuery {
    pub limit: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct TraceCreditResponse {
    pub summary: CreditSummary,
    pub report: TraceCreditReport,
    pub records: Vec<LocalTraceSubmissionRecord>,
}

#[derive(Debug, Serialize)]
pub struct TraceCreditNoticeResponse {
    pub credit_notice: Option<CreditSummary>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TraceCreditNoticeAction {
    Acknowledge,
    Snooze,
}

#[derive(Debug, Deserialize)]
pub struct TraceCreditNoticeActionRequest {
    pub action: TraceCreditNoticeAction,
    pub snooze_hours: Option<u32>,
}

#[derive(Debug, Serialize)]
pub struct TraceQueueStatusResponse {
    pub queued_envelopes: u32,
    pub held_envelopes: u32,
    #[serde(flatten)]
    pub diagnostics: TraceQueueDiagnostics,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub held_queue: Vec<TraceQueueHold>,
}

#[derive(Debug, Deserialize)]
pub struct TraceRevokeRequest {
    #[serde(default)]
    pub call_remote: bool,
    pub endpoint: Option<String>,
}

struct TraceCaptureOptions {
    thread_id: Option<Uuid>,
    include_message_text: bool,
    include_tool_payloads: bool,
    include_turn_count: Option<usize>,
    scope: Option<ConsentScope>,
}

pub async fn traces_policy_get_handler(
    AuthenticatedUser(user): AuthenticatedUser,
) -> Result<Json<TracePolicyResponse>, (StatusCode, String)> {
    let scope = Some(user.user_id.as_str());
    let policy = read_trace_policy_for_scope(scope).map_err(internal_error)?;
    let queued_envelopes = queued_trace_envelope_paths_for_scope(scope)
        .map_err(internal_error)?
        .len();
    let held_queue = read_trace_queue_holds_for_scope(scope).map_err(internal_error)?;
    Ok(Json(TracePolicyResponse {
        policy,
        queued_envelopes,
        held_queue,
    }))
}

pub async fn traces_policy_put_handler(
    AuthenticatedUser(user): AuthenticatedUser,
    Json(body): Json<TracePolicyRequest>,
) -> Result<Json<TracePolicyResponse>, (StatusCode, String)> {
    let user_scope = Some(user.user_id.as_str());
    let mut policy = read_trace_policy_for_scope(user_scope).map_err(internal_error)?;

    if let Some(enabled) = body.enabled {
        policy.enabled = enabled;
    }
    if let Some(endpoint) = body.endpoint {
        policy.ingestion_endpoint = if endpoint.trim().is_empty() {
            None
        } else {
            Some(endpoint)
        };
    }
    if let Some(env) = body.bearer_token_env {
        policy.bearer_token_env = env;
    }
    if let Some(url) = body.upload_token_issuer_url {
        policy.upload_token_issuer_url = if url.trim().is_empty() {
            None
        } else {
            Some(url)
        };
    }
    if let Some(allowed_hosts) = body.upload_token_issuer_allowed_hosts {
        policy.upload_token_issuer_allowed_hosts = allowed_hosts
            .into_iter()
            .map(|host| host.trim().to_ascii_lowercase())
            .filter(|host| !host.is_empty())
            .collect::<BTreeSet<_>>();
    }
    if let Some(audience) = body.upload_token_audience {
        policy.upload_token_audience = if audience.trim().is_empty() {
            None
        } else {
            Some(audience)
        };
    }
    if let Some(tenant_id) = body.upload_token_tenant_id {
        policy.upload_token_tenant_id = if tenant_id.trim().is_empty() {
            None
        } else {
            Some(tenant_id)
        };
    }
    if let Some(env) = body.upload_token_workload_token_env {
        policy.upload_token_workload_token_env = if env.trim().is_empty() {
            None
        } else {
            Some(env)
        };
    }
    if let Some(timeout_ms) = body.upload_token_issuer_timeout_ms {
        policy.upload_token_issuer_timeout_ms = timeout_ms;
    }
    if let Some(include) = body.include_message_text {
        policy.include_message_text = include;
    }
    if let Some(include) = body.include_tool_payloads {
        policy.include_tool_payloads = include;
    }
    if let Some(enabled) = body.auto_submit_failed_traces {
        policy.auto_submit_failed_traces = enabled;
    }
    if let Some(enabled) = body.auto_submit_high_value_traces {
        policy.auto_submit_high_value_traces = enabled;
    }
    if let Some(selected_tools) = body.selected_tools {
        policy.selected_tools = selected_tools
            .into_iter()
            .filter(|tool| !tool.trim().is_empty())
            .collect::<BTreeSet<_>>();
    }
    if let Some(required) = body.require_manual_approval_when_pii_detected {
        policy.require_manual_approval_when_pii_detected = required;
    }
    if let Some(score) = body.min_submission_score {
        policy.min_submission_score = score.clamp(0.0, 1.0);
    }
    if let Some(hours) = body.credit_notice_interval_hours {
        policy.credit_notice_interval_hours = hours;
    }
    if let Some(default_scope) = body.default_scope {
        policy.default_scope = default_scope;
    }

    if policy.enabled && policy.ingestion_endpoint.is_none() {
        return Err((
            StatusCode::BAD_REQUEST,
            "enabled trace contribution requires an ingestion endpoint".to_string(),
        ));
    }

    write_trace_policy_for_scope(user_scope, &policy).map_err(internal_error)?;
    traces_policy_get_handler(AuthenticatedUser(user)).await
}

pub async fn traces_preview_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Json(body): Json<TraceContributionPreviewRequest>,
) -> Result<Json<TraceContributionPreviewResponse>, (StatusCode, String)> {
    if body.enqueue {
        preflight_trace_upload_for_user(
            &user.user_id,
            TraceContributionAcceptance::QueueFromPreview,
            body.include_message_text,
            body.include_tool_payloads,
        )?;
    }

    let envelope = build_redacted_trace_envelope(
        &state,
        &user.user_id,
        TraceCaptureOptions {
            thread_id: body.thread_id,
            include_message_text: body.include_message_text,
            include_tool_payloads: body.include_tool_payloads,
            include_turn_count: body.include_turn_count,
            scope: body.scope,
        },
    )
    .await?;

    let queued = if body.enqueue {
        queue_trace_envelope_for_scope(Some(user.user_id.as_str()), &envelope)
            .map_err(internal_error)?;
        true
    } else {
        false
    };

    Ok(Json(TraceContributionPreviewResponse {
        submission_id: envelope.submission_id,
        queued,
        privacy_risk: format!("{:?}", envelope.privacy.residual_pii_risk),
        redaction_counts: envelope.privacy.redaction_counts.clone(),
        preview_markdown: trace_preview_markdown(&envelope),
        envelope,
    }))
}

pub async fn traces_submit_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Json(body): Json<TraceContributionSubmitRequest>,
) -> Result<Json<TraceContributionSubmitResponse>, (StatusCode, String)> {
    if !body.user_previewed {
        return Err((
            StatusCode::BAD_REQUEST,
            "Trace submission requires explicit preview acknowledgement".to_string(),
        ));
    }
    preflight_trace_upload_for_user(
        &user.user_id,
        TraceContributionAcceptance::ManualSubmit,
        body.include_message_text,
        body.include_tool_payloads,
    )?;

    let envelope = build_redacted_trace_envelope(
        &state,
        &user.user_id,
        TraceCaptureOptions {
            thread_id: body.thread_id,
            include_message_text: body.include_message_text,
            include_tool_payloads: body.include_tool_payloads,
            include_turn_count: body.include_turn_count,
            scope: body.scope,
        },
    )
    .await?;
    queue_trace_envelope_for_scope(Some(user.user_id.as_str()), &envelope)
        .map_err(internal_error)?;

    let flush_report = if body.flush {
        Some(
            flush_trace_contribution_queue_for_scope(Some(user.user_id.as_str()), 25)
                .await
                .map_err(internal_error)?,
        )
    } else {
        None
    };

    Ok(Json(TraceContributionSubmitResponse {
        submission_id: envelope.submission_id,
        queued: true,
        flushed: flush_report.is_some(),
        privacy_risk: format!("{:?}", envelope.privacy.residual_pii_risk),
        redaction_counts: envelope.privacy.redaction_counts.clone(),
        flush_report,
    }))
}

pub async fn traces_flush_handler(
    AuthenticatedUser(user): AuthenticatedUser,
    Query(query): Query<TraceQueueFlushQuery>,
) -> Result<Json<TraceQueueFlushReport>, (StatusCode, String)> {
    let report = flush_trace_contribution_queue_for_scope(
        Some(user.user_id.as_str()),
        query.limit.unwrap_or(25).clamp(1, 100),
    )
    .await
    .map_err(internal_error)?;
    Ok(Json(report))
}

pub async fn traces_credit_handler(
    AuthenticatedUser(user): AuthenticatedUser,
) -> Result<Json<TraceCreditResponse>, (StatusCode, String)> {
    if let Err(error) =
        sync_remote_trace_submission_records_for_scope(Some(user.user_id.as_str())).await
    {
        tracing::debug!(%error, "Failed to sync Trace Commons credit before web credit response");
    }
    let records =
        read_local_trace_records_for_scope(Some(user.user_id.as_str())).map_err(internal_error)?;
    let summary = trace_credit_summary(&records);
    let report = trace_credit_report(&records);
    Ok(Json(TraceCreditResponse {
        summary,
        report,
        records,
    }))
}

pub async fn traces_credit_notice_handler(
    AuthenticatedUser(user): AuthenticatedUser,
) -> Result<Json<TraceCreditNoticeResponse>, (StatusCode, String)> {
    if let Err(error) =
        sync_remote_trace_submission_records_for_scope(Some(user.user_id.as_str())).await
    {
        tracing::debug!(%error, "Failed to sync Trace Commons credit before web credit notice response");
    }
    let credit_notice = mark_trace_credit_notice_due_for_scope(Some(user.user_id.as_str()))
        .map_err(internal_error)?;
    Ok(Json(TraceCreditNoticeResponse { credit_notice }))
}

pub async fn traces_credit_notice_action_handler(
    AuthenticatedUser(user): AuthenticatedUser,
    Json(body): Json<TraceCreditNoticeActionRequest>,
) -> Result<Json<TraceCreditNoticeResponse>, (StatusCode, String)> {
    let scope = Some(user.user_id.as_str());
    let credit_notice = match body.action {
        TraceCreditNoticeAction::Acknowledge => {
            acknowledge_trace_credit_notice_for_scope(scope).map_err(internal_error)?
        }
        TraceCreditNoticeAction::Snooze => {
            let hours = body.snooze_hours.ok_or_else(|| {
                (
                    StatusCode::BAD_REQUEST,
                    "snooze_hours is required for snooze credit notice action".to_string(),
                )
            })?;
            if hours == 0 || hours > TRACE_CREDIT_NOTICE_MAX_SNOOZE_HOURS {
                return Err((
                    StatusCode::BAD_REQUEST,
                    format!(
                        "snooze_hours must be between 1 and {}",
                        TRACE_CREDIT_NOTICE_MAX_SNOOZE_HOURS
                    ),
                ));
            }
            snooze_trace_credit_notice_for_scope(scope, chrono::Duration::hours(i64::from(hours)))
                .map_err(internal_error)?
        }
    };
    Ok(Json(TraceCreditNoticeResponse { credit_notice }))
}

pub async fn traces_queue_status_handler(
    AuthenticatedUser(user): AuthenticatedUser,
) -> Result<Json<TraceQueueStatusResponse>, (StatusCode, String)> {
    let scope = Some(user.user_id.as_str());
    let diagnostics = trace_queue_diagnostics_for_scope(scope).map_err(internal_error)?;
    let held_queue = read_trace_queue_holds_for_scope(scope).map_err(internal_error)?;
    Ok(Json(TraceQueueStatusResponse {
        queued_envelopes: diagnostics.queued_count,
        held_envelopes: diagnostics.held_count,
        diagnostics,
        held_queue,
    }))
}

pub async fn traces_submissions_handler(
    AuthenticatedUser(user): AuthenticatedUser,
) -> Result<Json<Vec<LocalTraceSubmissionRecord>>, (StatusCode, String)> {
    if let Err(error) =
        sync_remote_trace_submission_records_for_scope(Some(user.user_id.as_str())).await
    {
        tracing::debug!(%error, "Failed to sync Trace Commons credit before web submissions response");
    }
    let records =
        read_local_trace_records_for_scope(Some(user.user_id.as_str())).map_err(internal_error)?;
    Ok(Json(records))
}

pub async fn traces_revoke_handler(
    AuthenticatedUser(user): AuthenticatedUser,
    Path(submission_id): Path<Uuid>,
    Json(body): Json<TraceRevokeRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
    let policy =
        read_trace_policy_for_scope(Some(user.user_id.as_str())).map_err(internal_error)?;
    let endpoint = if body.call_remote {
        body.endpoint
            .as_deref()
            .or(policy.ingestion_endpoint.as_deref())
    } else {
        None
    };
    revoke_trace_submission_for_scope_with_policy(
        Some(user.user_id.as_str()),
        submission_id,
        endpoint,
        &policy,
    )
    .await
    .map_err(internal_error)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn build_redacted_trace_envelope(
    state: &GatewayState,
    user_id: &str,
    options: TraceCaptureOptions,
) -> Result<TraceContributionEnvelope, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let thread_id = resolve_trace_thread_id(state, user_id, options.thread_id).await?;
    let tenant_store = crate::tenant::TenantScope::new(user_id.to_string(), Arc::clone(store));

    let include_turn_count = options.include_turn_count.unwrap_or(5).clamp(1, 20);
    let (messages, _) = tenant_store
        .list_conversation_messages_paginated(thread_id, None, (include_turn_count * 4) as i64)
        .await
        .map_err(|error| match error {
            crate::error::DatabaseError::NotFound { .. } => {
                (StatusCode::NOT_FOUND, "Thread not found".to_string())
            }
            other => internal_error(other),
        })?;
    let turns = capture_turns_from_conversation_messages(&messages);
    if turns.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "Thread has no completed turns to contribute".to_string(),
        ));
    }

    let capture_turns = turns
        .into_iter()
        .rev()
        .take(include_turn_count)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>();

    let policy = read_trace_policy_for_scope(Some(user_id)).map_err(internal_error)?;
    let capture_options = RecordedTraceContributionOptions {
        include_message_text: options.include_message_text || policy.include_message_text,
        include_tool_payloads: options.include_tool_payloads || policy.include_tool_payloads,
        consent_scopes: vec![options.scope.unwrap_or(policy.default_scope)],
        channel: TraceChannel::Web,
        engine_version: None,
        feature_flags: BTreeMap::new(),
        pseudonymous_contributor_id: Some(local_pseudonymous_contributor_id(user_id)),
        tenant_scope_ref: Some(local_pseudonymous_tenant_scope_ref(user_id)),
        credit_account_ref: None,
    };

    let raw = RawTraceContribution::from_capture_turns(&capture_turns, capture_options);
    let redactor = DeterministicTraceRedactor::default();
    let mut envelope = redactor.redact_trace(raw).await.map_err(internal_error)?;
    apply_credit_estimate_to_envelope(&mut envelope);
    Ok(envelope)
}

async fn resolve_trace_thread_id(
    state: &GatewayState,
    user_id: &str,
    thread_id: Option<Uuid>,
) -> Result<Uuid, (StatusCode, String)> {
    if let Some(thread_id) = thread_id {
        return Ok(thread_id);
    }

    let session_manager = state.session_manager.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Session manager not available".to_string(),
    ))?;
    let session = session_manager.get_or_create_session(user_id).await;
    let session = session.lock().await;
    session
        .active_thread
        .ok_or((StatusCode::NOT_FOUND, "No active thread".to_string()))
}

fn trace_preview_markdown(envelope: &TraceContributionEnvelope) -> String {
    let redactions = if envelope.privacy.redaction_counts.is_empty() {
        "none".to_string()
    } else {
        envelope
            .privacy
            .redaction_counts
            .iter()
            .map(|(label, count)| format!("{count} {label}"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    format!(
        "Submission: {}\nPrivacy risk: {:?}\nRedactions: {}\nScore: {:.2}\nPending credit: +{:.2}",
        envelope.submission_id,
        envelope.privacy.residual_pii_risk,
        redactions,
        envelope.value.submission_score,
        envelope.value.credit_points_pending
    )
}

fn internal_error(error: impl std::fmt::Display) -> (StatusCode, String) {
    tracing::error!(%error, "Trace contribution handler failed");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        "Trace contribution operation failed".to_string(),
    )
}

fn preflight_trace_upload_for_user(
    user_id: &str,
    intent: TraceContributionAcceptance,
    include_message_text: bool,
    include_tool_payloads: bool,
) -> Result<(), (StatusCode, String)> {
    let policy = read_trace_policy_for_scope(Some(user_id)).map_err(internal_error)?;
    preflight_trace_contribution_policy(&policy, intent).map_err(trace_policy_rejection)?;
    if intent != TraceContributionAcceptance::PreviewOnly {
        if include_message_text && !policy.include_message_text {
            return Err((
                StatusCode::CONFLICT,
                "trace contribution policy does not allow message text upload".to_string(),
            ));
        }
        if include_tool_payloads && !policy.include_tool_payloads {
            return Err((
                StatusCode::CONFLICT,
                "trace contribution policy does not allow tool payload upload".to_string(),
            ));
        }
    }
    Ok(())
}

fn trace_policy_rejection(rejection: TraceContributionPolicyRejection) -> (StatusCode, String) {
    (StatusCode::CONFLICT, rejection.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channels::web::auth::UserIdentity;
    use crate::llm::recording::{TraceFile, TraceResponse, TraceStep, TraceToolCall};
    use crate::trace_contribution::{
        LocalTraceSubmissionStatus, TraceCreditEvent, TraceCreditEventKind, TraceQueueWarningKind,
        trace_contribution_dir_for_scope, write_trace_policy_for_scope,
    };
    use chrono::Utc;

    fn write_trace_records(scope: &str, records: &[LocalTraceSubmissionRecord]) {
        let path = trace_contribution_dir_for_scope(Some(scope)).join("submissions.json");
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("trace record dir creates");
        }
        let body = serde_json::to_string_pretty(records).expect("trace records serialize");
        std::fs::write(path, body).expect("trace records write");
    }

    fn write_queue_fixture(scope: &str, held_reason: Option<&str>) -> Uuid {
        let submission_id = Uuid::new_v4();
        let dir = trace_contribution_dir_for_scope(Some(scope)).join("queue");
        std::fs::create_dir_all(&dir).expect("trace queue dir creates");
        std::fs::write(dir.join(format!("{submission_id}.json")), "{}")
            .expect("trace queue fixture writes");
        if let Some(reason) = held_reason {
            let hold = serde_json::json!({ "reason": reason });
            let body = serde_json::to_string_pretty(&hold).expect("hold fixture serializes");
            std::fs::write(dir.join(format!("{submission_id}.held.json")), body)
                .expect("trace queue hold fixture writes");
        }
        submission_id
    }

    fn submitted_record(points: f32) -> LocalTraceSubmissionRecord {
        let submission_id = Uuid::new_v4();
        LocalTraceSubmissionRecord {
            submission_id,
            trace_id: Uuid::new_v4(),
            endpoint: Some("https://trace.example.com/v1/traces".to_string()),
            status: LocalTraceSubmissionStatus::Submitted,
            server_status: Some("accepted".to_string()),
            submitted_at: Some(Utc::now()),
            revoked_at: None,
            privacy_risk: "low".to_string(),
            redaction_counts: BTreeMap::new(),
            credit_points_pending: points,
            credit_points_final: Some(points + 1.0),
            credit_explanation: vec![format!("Scoped credit {points:.1}")],
            credit_events: vec![TraceCreditEvent {
                event_id: Uuid::new_v4(),
                submission_id,
                contributor_pseudonym: "test".to_string(),
                kind: TraceCreditEventKind::CreditSynced,
                points_delta: points,
                reason: "Delayed credit synced.".to_string(),
                created_at: Utc::now(),
            }],
            history: Vec::new(),
            last_credit_notice_at: None,
            credit_notice_state: crate::trace_contribution::TraceCreditNoticeState::default(),
        }
    }

    fn queue_status_sensitive_trace() -> TraceFile {
        TraceFile {
            model_name: "test-model".to_string(),
            memory_snapshot: Vec::new(),
            http_exchanges: Vec::new(),
            steps: vec![
                TraceStep {
                    request_hint: None,
                    response: TraceResponse::UserInput {
                        content:
                            "Please submit TRACE_SECRET_TOKEN_VALUE to https://private.example/v1/traces"
                                .to_string(),
                    },
                    expected_tool_results: Vec::new(),
                },
                TraceStep {
                    request_hint: None,
                    response: TraceResponse::ToolCalls {
                        tool_calls: vec![TraceToolCall {
                            id: "call_private_trace".to_string(),
                            name: "http".to_string(),
                            arguments: serde_json::json!({
                                "url": "https://private.example/v1/traces",
                                "Authorization": "Bearer TRACE_SECRET_TOKEN_VALUE",
                                "body": "raw tool body containing TRACE_SECRET_TOKEN_VALUE",
                            }),
                        }],
                        input_tokens: 10,
                        output_tokens: 3,
                    },
                    expected_tool_results: Vec::new(),
                },
            ],
        }
    }

    #[tokio::test]
    async fn traces_flush_handler_returns_authenticated_user_scoped_credit_notice() {
        let user_id = format!("trace-web-flush-user-{}", Uuid::new_v4());
        let other_user_id = format!("trace-web-flush-other-{}", Uuid::new_v4());
        let policy = StandingTraceContributionPolicy {
            enabled: true,
            ingestion_endpoint: Some("https://trace.example.com/v1/traces".to_string()),
            credit_notice_interval_hours: 168,
            ..Default::default()
        };
        write_trace_policy_for_scope(Some(&user_id), &policy).expect("user policy writes");
        write_trace_policy_for_scope(Some(&other_user_id), &policy).expect("other policy writes");
        write_trace_records(&user_id, &[submitted_record(2.0)]);
        write_trace_records(&other_user_id, &[submitted_record(99.0)]);

        let Json(report) = traces_flush_handler(
            AuthenticatedUser(UserIdentity {
                user_id: user_id.clone(),
                role: "member".to_string(),
                workspace_read_scopes: Vec::new(),
            }),
            Query(TraceQueueFlushQuery { limit: Some(25) }),
        )
        .await
        .expect("flush handler succeeds");

        let notice = report
            .credit_notice
            .expect("scoped due credit notice is returned");
        assert_eq!(notice.submissions_submitted, 1);
        assert_eq!(notice.pending_credit, 2.0);
        assert_eq!(notice.final_credit, 3.0);
        assert!(
            notice
                .recent_explanations
                .iter()
                .any(|reason| reason.contains("Scoped credit 2.0"))
        );
        assert!(
            notice
                .recent_explanations
                .iter()
                .all(|reason| !reason.contains("99.0"))
        );
    }

    #[tokio::test]
    async fn traces_credit_notice_action_acknowledges_authenticated_user_only() {
        let user_id = format!("trace-web-credit-ack-user-{}", Uuid::new_v4());
        let other_user_id = format!("trace-web-credit-ack-other-{}", Uuid::new_v4());
        let policy = StandingTraceContributionPolicy {
            enabled: true,
            ingestion_endpoint: Some("https://trace.example.com/v1/traces".to_string()),
            credit_notice_interval_hours: 168,
            ..Default::default()
        };
        write_trace_policy_for_scope(Some(&user_id), &policy).expect("user policy writes");
        write_trace_policy_for_scope(Some(&other_user_id), &policy).expect("other policy writes");
        write_trace_records(&user_id, &[submitted_record(2.0)]);
        write_trace_records(&other_user_id, &[submitted_record(99.0)]);

        let Json(response) = traces_credit_notice_action_handler(
            AuthenticatedUser(UserIdentity {
                user_id: user_id.clone(),
                role: "member".to_string(),
                workspace_read_scopes: Vec::new(),
            }),
            Json(TraceCreditNoticeActionRequest {
                action: TraceCreditNoticeAction::Acknowledge,
                snooze_hours: None,
            }),
        )
        .await
        .expect("credit notice action succeeds");

        assert_eq!(
            response
                .credit_notice
                .expect("ack returns current credit summary")
                .pending_credit,
            2.0
        );
        let Json(after_ack) = traces_credit_notice_handler(AuthenticatedUser(UserIdentity {
            user_id: user_id.clone(),
            role: "member".to_string(),
            workspace_read_scopes: Vec::new(),
        }))
        .await
        .expect("credit notice handler succeeds");
        assert_eq!(after_ack.credit_notice, None);

        let other_due =
            crate::trace_contribution::trace_credit_notice_due_for_scope(Some(&other_user_id))
                .expect("other notice due check succeeds")
                .expect("other user notice remains due");
        assert_eq!(other_due.pending_credit, 99.0);
    }

    #[tokio::test]
    async fn traces_credit_notice_action_rejects_zero_snooze_hours() {
        let user_id = format!("trace-web-credit-snooze-bad-user-{}", Uuid::new_v4());
        write_trace_policy_for_scope(
            Some(&user_id),
            &StandingTraceContributionPolicy {
                enabled: true,
                ingestion_endpoint: Some("https://trace.example.com/v1/traces".to_string()),
                credit_notice_interval_hours: 168,
                ..Default::default()
            },
        )
        .expect("policy writes");
        write_trace_records(&user_id, &[submitted_record(2.0)]);

        let error = traces_credit_notice_action_handler(
            AuthenticatedUser(UserIdentity {
                user_id,
                role: "member".to_string(),
                workspace_read_scopes: Vec::new(),
            }),
            Json(TraceCreditNoticeActionRequest {
                action: TraceCreditNoticeAction::Snooze,
                snooze_hours: Some(0),
            }),
        )
        .await
        .expect_err("zero-hour snooze should be rejected");

        assert_eq!(error.0, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn traces_queue_status_handler_surfaces_safe_hardened_diagnostics() {
        let user_id = format!("trace-web-queue-hardened-user-{}", Uuid::new_v4());
        let other_user_id = format!("trace-web-queue-hardened-other-{}", Uuid::new_v4());
        let policy = StandingTraceContributionPolicy {
            enabled: true,
            ingestion_endpoint: Some("https://private.example/v1/traces".to_string()),
            bearer_token_env: "TRACE_SECRET_TOKEN_VALUE".to_string(),
            ..Default::default()
        };
        write_trace_policy_for_scope(Some(&user_id), &policy).expect("user policy writes");
        write_trace_policy_for_scope(Some(&other_user_id), &policy).expect("other policy writes");

        let raw = RawTraceContribution::from_recorded_trace(
            &queue_status_sensitive_trace(),
            RecordedTraceContributionOptions::default(),
        );
        let mut envelope = DeterministicTraceRedactor::default()
            .redact_trace(raw)
            .await
            .expect("redaction should succeed");
        envelope.schema_version = "ironclaw.trace_contribution.v0".to_string();
        envelope.consent.policy_version = "2025-01-01".to_string();
        envelope.privacy.redaction_pipeline_version = "legacy-redactor".to_string();
        envelope.trace_card.redaction_pipeline_version = "legacy-trace-card-redactor".to_string();
        queue_trace_envelope_for_scope(Some(&user_id), &envelope)
            .expect("mismatched user envelope queues");
        write_queue_fixture(&other_user_id, Some("other user hold"));

        let Json(response) = traces_queue_status_handler(AuthenticatedUser(UserIdentity {
            user_id: user_id.clone(),
            role: "member".to_string(),
            workspace_read_scopes: Vec::new(),
        }))
        .await
        .expect("queue status handler succeeds");

        assert_eq!(response.queued_envelopes, 1);
        assert_eq!(response.held_envelopes, 0);
        assert!(
            response
                .diagnostics
                .warnings
                .iter()
                .any(|warning| warning.kind == TraceQueueWarningKind::SchemaVersionMismatch)
        );
        assert!(
            response
                .diagnostics
                .warnings
                .iter()
                .any(|warning| warning.kind == TraceQueueWarningKind::PolicyVersionMismatch)
        );
        assert!(
            response
                .diagnostics
                .warnings
                .iter()
                .any(|warning| warning.kind == TraceQueueWarningKind::RedactionPipelineMismatch)
        );
        assert!(response.diagnostics.warnings.iter().any(|warning| {
            warning.kind == TraceQueueWarningKind::TraceCardRedactionPipelineMismatch
        }));
        assert_eq!(response.held_queue.len(), 0);

        let serialized = serde_json::to_string(&response).expect("response serializes");
        assert!(serialized.contains("\"telemetry\""));
        assert!(serialized.contains("schema_version_mismatch"));
        assert!(serialized.contains("policy_version_mismatch"));
        assert!(serialized.contains("redaction_pipeline_mismatch"));
        assert!(serialized.contains("trace_card_redaction_pipeline_mismatch"));
        assert!(!serialized.contains("TRACE_SECRET_TOKEN_VALUE"));
        assert!(!serialized.contains("https://private.example/v1/traces"));
        assert!(!serialized.contains("raw tool body containing"));
        assert!(!serialized.contains("legacy-redactor"));
        assert!(!serialized.contains("legacy-trace-card-redactor"));
        assert!(!serialized.contains("other user hold"));

        let _ = std::fs::remove_dir_all(trace_contribution_dir_for_scope(Some(&user_id)));
        let _ = std::fs::remove_dir_all(trace_contribution_dir_for_scope(Some(&other_user_id)));
    }

    #[tokio::test]
    async fn traces_queue_status_handler_returns_authenticated_user_scoped_diagnostics() {
        let user_id = format!("trace-web-queue-user-{}", Uuid::new_v4());
        let other_user_id = format!("trace-web-queue-other-{}", Uuid::new_v4());
        let policy = StandingTraceContributionPolicy {
            enabled: true,
            ingestion_endpoint: Some("https://trace.example.com/v1/traces".to_string()),
            ..Default::default()
        };
        write_trace_policy_for_scope(Some(&user_id), &policy).expect("user policy writes");
        write_trace_policy_for_scope(Some(&other_user_id), &policy).expect("other policy writes");
        let held_submission_id = write_queue_fixture(&user_id, Some("requires manual review"));
        write_queue_fixture(&user_id, None);
        write_queue_fixture(&other_user_id, Some("other user hold"));
        write_queue_fixture(&other_user_id, None);

        let Json(response) = traces_queue_status_handler(AuthenticatedUser(UserIdentity {
            user_id: user_id.clone(),
            role: "member".to_string(),
            workspace_read_scopes: Vec::new(),
        }))
        .await
        .expect("queue status handler succeeds");

        assert_eq!(response.queued_envelopes, 2);
        assert_eq!(response.held_envelopes, 1);
        assert_eq!(response.diagnostics.queued_count, 2);
        assert_eq!(response.diagnostics.held_count, 1);
        assert!(response.diagnostics.policy_enabled);
        assert_eq!(response.held_queue.len(), 1);
        assert_eq!(response.held_queue[0].submission_id, held_submission_id);
        assert_eq!(response.held_queue[0].reason, "requires manual review");
        assert!(
            response
                .held_queue
                .iter()
                .all(|hold| hold.reason != "other user hold")
        );
    }

    #[tokio::test]
    async fn traces_credit_notice_handler_returns_authenticated_user_scoped_due_notice() {
        let user_id = format!("trace-web-credit-notice-user-{}", Uuid::new_v4());
        let other_user_id = format!("trace-web-credit-notice-other-{}", Uuid::new_v4());
        let policy = StandingTraceContributionPolicy {
            enabled: true,
            ingestion_endpoint: Some("https://trace.example.com/v1/traces".to_string()),
            credit_notice_interval_hours: 168,
            ..Default::default()
        };
        write_trace_policy_for_scope(Some(&user_id), &policy).expect("user policy writes");
        write_trace_policy_for_scope(Some(&other_user_id), &policy).expect("other policy writes");
        write_trace_records(&user_id, &[submitted_record(4.0)]);
        write_trace_records(&other_user_id, &[submitted_record(88.0)]);

        let Json(response) = traces_credit_notice_handler(AuthenticatedUser(UserIdentity {
            user_id: user_id.clone(),
            role: "member".to_string(),
            workspace_read_scopes: Vec::new(),
        }))
        .await
        .expect("credit notice handler succeeds");

        let notice = response
            .credit_notice
            .expect("scoped due credit notice is returned");
        assert_eq!(notice.submissions_submitted, 1);
        assert_eq!(notice.pending_credit, 4.0);
        assert_eq!(notice.final_credit, 5.0);
        assert!(
            notice
                .recent_explanations
                .iter()
                .any(|reason| reason.contains("Scoped credit 4.0"))
        );
        assert!(
            notice
                .recent_explanations
                .iter()
                .all(|reason| !reason.contains("88.0"))
        );
    }
}
