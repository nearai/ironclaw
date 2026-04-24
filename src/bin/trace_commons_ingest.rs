use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context;
use axum::extract::{DefaultBodyLimit, Query};
use axum::http::header::AUTHORIZATION;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{delete, get, post};
use axum::{Json, Router, extract::Path as AxumPath, extract::State};
use chrono::{DateTime, Utc};
use ironclaw::trace_contribution::{
    ConsentScope, EmbeddingAnalysisMetadata, ResidualPiiRisk, TRACE_CONTRIBUTION_SCHEMA_VERSION,
    TraceContributionEnvelope, TraceSubmissionReceipt, TraceSubmissionStatusRequest,
    TraceSubmissionStatusUpdate, apply_credit_estimate_to_envelope,
    canonical_summary_for_embedding, rescrub_trace_envelope,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::net::TcpListener;
use uuid::Uuid;

const DEFAULT_BIND: &str = "127.0.0.1:3907";
const MAX_INGEST_BODY_BYTES: usize = 2 * 1024 * 1024;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = tracing_subscriber::fmt::try_init();
    let state = Arc::new(AppState::from_env()?);
    let bind = std::env::var("TRACE_COMMONS_BIND").unwrap_or_else(|_| DEFAULT_BIND.to_string());
    let addr = bind
        .parse::<SocketAddr>()
        .with_context(|| format!("invalid TRACE_COMMONS_BIND address: {bind}"))?;
    let listener = TcpListener::bind(addr)
        .await
        .with_context(|| format!("failed to bind trace commons ingestion service at {addr}"))?;
    tracing::info!(%addr, "Trace Commons ingestion service listening");
    axum::serve(listener, app(state))
        .await
        .context("trace commons ingestion service failed")
}

#[derive(Clone)]
struct AppState {
    root: PathBuf,
    tokens: Arc<BTreeMap<String, TenantAuth>>,
}

#[derive(Debug, Clone)]
struct TenantAuth {
    tenant_id: String,
    role: TokenRole,
    principal_ref: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum TokenRole {
    Contributor,
    Reviewer,
    Admin,
}

impl TokenRole {
    fn parse(raw: &str) -> anyhow::Result<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "contributor" => Ok(Self::Contributor),
            "reviewer" => Ok(Self::Reviewer),
            "admin" => Ok(Self::Admin),
            other => anyhow::bail!("unknown Trace Commons token role: {other}"),
        }
    }

    fn can_review(self) -> bool {
        matches!(self, Self::Reviewer | Self::Admin)
    }
}

impl AppState {
    fn from_env() -> anyhow::Result<Self> {
        let root = std::env::var("TRACE_COMMONS_DATA_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| default_data_dir());
        let tokens = parse_tenant_tokens_from_env()?;
        if tokens.is_empty() {
            anyhow::bail!(
                "TRACE_COMMONS_TENANT_TOKENS or TRACE_COMMONS_INGEST_TOKEN must be configured"
            );
        }
        Ok(Self {
            root,
            tokens: Arc::new(tokens),
        })
    }
}

fn app(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/health", get(health_handler))
        .route(
            "/v1/traces",
            get(list_traces_handler)
                .post(submit_trace_handler)
                .delete(revoke_trace_body_handler),
        )
        .route("/v1/traces/{submission_id}", delete(revoke_trace_handler))
        .route(
            "/v1/traces/{submission_id}/revoke",
            post(revoke_trace_handler),
        )
        .route("/v1/contributors/me/credit", get(credit_handler))
        .route(
            "/v1/contributors/me/credit-events",
            get(credit_events_handler),
        )
        .route(
            "/v1/contributors/me/submission-status",
            post(submission_status_handler),
        )
        .route("/v1/analytics/summary", get(analytics_handler))
        .route("/v1/review/quarantine", get(review_quarantine_handler))
        .route(
            "/v1/review/active-learning",
            get(active_learning_review_queue_handler),
        )
        .route(
            "/v1/review/{submission_id}/decision",
            post(review_decision_handler),
        )
        .route(
            "/v1/review/{submission_id}/credit-events",
            post(append_credit_event_handler),
        )
        .route("/v1/datasets/replay", get(dataset_replay_handler))
        .route("/v1/benchmarks/convert", post(benchmark_convert_handler))
        .route(
            "/v1/ranker/training-candidates",
            get(ranker_training_candidates_handler),
        )
        .route(
            "/v1/ranker/training-pairs",
            get(ranker_training_pairs_handler),
        )
        .route("/v1/admin/maintenance", post(maintenance_handler))
        .route("/v1/audit/events", get(audit_events_handler))
        .with_state(state)
        .layer(DefaultBodyLimit::max(MAX_INGEST_BODY_BYTES))
}

fn default_data_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join(".ironclaw")
        .join("trace_commons_ingest")
}

fn parse_tenant_tokens_from_env() -> anyhow::Result<BTreeMap<String, TenantAuth>> {
    let mut tokens = BTreeMap::new();
    if let Ok(configured) = std::env::var("TRACE_COMMONS_TENANT_TOKENS") {
        for pair in configured.split(',') {
            let pair = pair.trim();
            if pair.is_empty() {
                continue;
            }
            let parts = pair.split(':').collect::<Vec<_>>();
            match parts.as_slice() {
                [tenant_id, token] => {
                    insert_token(&mut tokens, tenant_id, token, TokenRole::Contributor);
                }
                [tenant_id, role, token] => {
                    insert_token(&mut tokens, tenant_id, token, TokenRole::parse(role)?);
                }
                _ => {
                    anyhow::bail!(
                        "TRACE_COMMONS_TENANT_TOKENS entries must use tenant_id:token or tenant_id:role:token syntax"
                    );
                }
            }
        }
    }

    if let Ok(token) = std::env::var("TRACE_COMMONS_INGEST_TOKEN") {
        insert_token(&mut tokens, "default", &token, TokenRole::Contributor);
    }

    Ok(tokens)
}

fn insert_token(
    tokens: &mut BTreeMap<String, TenantAuth>,
    tenant_id: &str,
    token: &str,
    role: TokenRole,
) {
    let tenant_id = tenant_id.trim();
    let token = token.trim();
    if tenant_id.is_empty() || token.is_empty() {
        return;
    }
    tokens.insert(
        token.to_string(),
        TenantAuth {
            tenant_id: tenant_id.to_string(),
            role,
            principal_ref: principal_storage_ref(token),
        },
    );
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    status: &'static str,
    schema_version: &'static str,
}

async fn health_handler() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        schema_version: TRACE_CONTRIBUTION_SCHEMA_VERSION,
    })
}

async fn submit_trace_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(mut envelope): Json<TraceContributionEnvelope>,
) -> ApiResult<Json<TraceSubmissionReceipt>> {
    let tenant = authenticate(state.as_ref(), &headers)?;
    validate_envelope(&envelope)?;

    if let Some(existing) =
        read_submission_record(&state.root, &tenant.tenant_id, envelope.submission_id)
            .map_err(internal_error)?
    {
        if !can_access_submission(&tenant, &existing) {
            return Err(api_error(
                StatusCode::CONFLICT,
                "submission id already belongs to another principal",
            ));
        }
        let receipt = receipt_from_record(&existing);
        append_audit_event(
            &state.root,
            &tenant.tenant_id,
            TraceCommonsAuditEvent::idempotent_submit(&tenant, envelope.submission_id),
        )
        .map_err(internal_error)?;
        return Ok(Json(receipt));
    }

    rescrub_trace_envelope(&mut envelope);
    let existing_derived =
        read_all_derived_records(&state.root, &tenant.tenant_id).map_err(internal_error)?;
    let derived_precheck = build_derived_precheck(&envelope, &existing_derived);
    apply_embedding_precheck(&mut envelope, &derived_precheck);
    apply_credit_estimate_to_envelope(&mut envelope);
    let corpus_status = status_for_risk(envelope.privacy.residual_pii_risk);
    if corpus_status != TraceCorpusStatus::Accepted {
        envelope.value.credit_points_pending = 0.0;
        envelope.value.explanation = vec![
            "Submission is quarantined until privacy review completes; credit is held at 0.0."
                .to_string(),
        ];
        envelope.value_card.user_visible_explanation = envelope.value.explanation.clone();
    }

    let object_key = store_envelope(&state.root, &tenant.tenant_id, corpus_status, &envelope)
        .map_err(internal_error)?;
    let derived_record = build_derived_record(
        &tenant.tenant_id,
        corpus_status,
        &envelope,
        derived_precheck,
    );
    let record = TraceCommonsSubmissionRecord {
        tenant_id: tenant.tenant_id.clone(),
        tenant_storage_ref: tenant_storage_ref(&tenant.tenant_id),
        auth_principal_ref: tenant.principal_ref.clone(),
        submitted_tenant_scope_ref: envelope.contributor.tenant_scope_ref.clone(),
        contributor_pseudonym: envelope.contributor.pseudonymous_contributor_id.clone(),
        submission_id: envelope.submission_id,
        trace_id: envelope.trace_id,
        status: corpus_status,
        privacy_risk: envelope.privacy.residual_pii_risk,
        submission_score: envelope.value.submission_score,
        credit_points_pending: envelope.value.credit_points_pending,
        credit_points_final: envelope.value.credit_points_final,
        consent_scopes: envelope.consent.scopes.clone(),
        redaction_counts: envelope.privacy.redaction_counts.clone(),
        received_at: Utc::now(),
        object_key,
    };
    write_submission_record(&state.root, &record).map_err(internal_error)?;
    write_derived_record(&state.root, &derived_record).map_err(internal_error)?;
    append_audit_event(
        &state.root,
        &tenant.tenant_id,
        TraceCommonsAuditEvent::submitted(&record),
    )
    .map_err(internal_error)?;

    Ok(Json(receipt_from_record(&record)))
}

async fn revoke_trace_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(submission_id): AxumPath<Uuid>,
) -> ApiResult<StatusCode> {
    revoke_submission(&state, &headers, submission_id)
}

#[derive(Debug, Deserialize)]
struct RevokeTraceBody {
    submission_id: Uuid,
}

async fn revoke_trace_body_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<RevokeTraceBody>,
) -> ApiResult<StatusCode> {
    revoke_submission(&state, &headers, body.submission_id)
}

fn revoke_submission(
    state: &AppState,
    headers: &HeaderMap,
    submission_id: Uuid,
) -> ApiResult<StatusCode> {
    let tenant = authenticate(state, headers)?;
    let tombstone = TraceCommonsRevocation {
        tenant_id: tenant.tenant_id.clone(),
        tenant_storage_ref: tenant_storage_ref(&tenant.tenant_id),
        submission_id,
        revoked_at: Utc::now(),
        reason: "contributor_revocation".to_string(),
    };
    write_revocation(&state.root, &tombstone).map_err(internal_error)?;

    if let Some(mut record) = read_submission_record(&state.root, &tenant.tenant_id, submission_id)
        .map_err(internal_error)?
    {
        if !can_access_submission(&tenant, &record) {
            return Err(api_error(
                StatusCode::NOT_FOUND,
                "trace submission not found",
            ));
        }
        record.status = TraceCorpusStatus::Revoked;
        record.credit_points_final = Some(0.0);
        write_submission_record(&state.root, &record).map_err(internal_error)?;
    }
    if let Some(mut derived) = read_derived_record(&state.root, &tenant.tenant_id, submission_id)
        .map_err(internal_error)?
    {
        derived.status = TraceCorpusStatus::Revoked;
        write_derived_record(&state.root, &derived).map_err(internal_error)?;
    }

    append_audit_event(
        &state.root,
        &tenant.tenant_id,
        TraceCommonsAuditEvent::revoked(&tenant, submission_id),
    )
    .map_err(internal_error)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn credit_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> ApiResult<Json<TraceCommonsTenantCreditResponse>> {
    let tenant = authenticate(state.as_ref(), &headers)?;
    let records = visible_submission_records(
        &tenant,
        read_all_submission_records(&state.root, &tenant.tenant_id).map_err(internal_error)?,
    );
    let credit_events = eligible_credit_events_for_records(
        &records,
        visible_credit_events(
            &tenant,
            read_all_credit_events(&state.root, &tenant.tenant_id).map_err(internal_error)?,
        ),
    );
    Ok(Json(
        TraceCommonsTenantCreditResponse::from_records_and_events(
            tenant.tenant_id,
            records,
            &credit_events,
        ),
    ))
}

async fn credit_events_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> ApiResult<Json<Vec<TraceCommonsCreditLedgerRecord>>> {
    let tenant = authenticate(state.as_ref(), &headers)?;
    let records = visible_submission_records(
        &tenant,
        read_all_submission_records(&state.root, &tenant.tenant_id).map_err(internal_error)?,
    );
    let credit_events = eligible_credit_events_for_records(
        &records,
        visible_credit_events(
            &tenant,
            read_all_credit_events(&state.root, &tenant.tenant_id).map_err(internal_error)?,
        ),
    );
    Ok(Json(credit_events))
}

async fn submission_status_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<TraceSubmissionStatusRequest>,
) -> ApiResult<Json<Vec<TraceSubmissionStatusUpdate>>> {
    let tenant = authenticate(state.as_ref(), &headers)?;
    if body.submission_ids.len() > 500 {
        return Err(api_error(
            StatusCode::PAYLOAD_TOO_LARGE,
            "submission status requests are limited to 500 ids",
        ));
    }

    let records =
        read_all_submission_records(&state.root, &tenant.tenant_id).map_err(internal_error)?;
    let visible_records = visible_submission_records(&tenant, records);
    let credit_events = eligible_credit_events_for_records(
        &visible_records,
        visible_credit_events(
            &tenant,
            read_all_credit_events(&state.root, &tenant.tenant_id).map_err(internal_error)?,
        ),
    );
    let visible_by_submission = visible_records
        .iter()
        .map(|record| (record.submission_id, record))
        .collect::<BTreeMap<_, _>>();
    let mut statuses = Vec::new();
    for submission_id in body.submission_ids {
        if let Some(record) = visible_by_submission.get(&submission_id) {
            statuses.push(submission_status_from_record(record, &credit_events));
        }
    }

    Ok(Json(statuses))
}

async fn analytics_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> ApiResult<Json<TraceCommonsAnalyticsResponse>> {
    let tenant = authenticate(state.as_ref(), &headers)?;
    require_reviewer(&tenant)?;
    let records =
        read_all_submission_records(&state.root, &tenant.tenant_id).map_err(internal_error)?;
    let derived =
        read_all_derived_records(&state.root, &tenant.tenant_id).map_err(internal_error)?;
    Ok(Json(TraceCommonsAnalyticsResponse::from_records(
        tenant.tenant_id,
        records,
        derived,
    )))
}

#[derive(Debug, Deserialize)]
struct TraceListQuery {
    status: Option<TraceCorpusStatus>,
    limit: Option<usize>,
    coverage_tag: Option<String>,
    tool: Option<String>,
    privacy_risk: Option<ResidualPiiRisk>,
    consent_scope: Option<String>,
}

async fn list_traces_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<TraceListQuery>,
) -> ApiResult<Json<Vec<TraceCommonsTraceListItem>>> {
    let tenant = authenticate(state.as_ref(), &headers)?;
    require_reviewer(&tenant)?;
    let records =
        read_all_submission_records(&state.root, &tenant.tenant_id).map_err(internal_error)?;
    let derived =
        read_all_derived_records(&state.root, &tenant.tenant_id).map_err(internal_error)?;
    let derived_by_submission = derived
        .into_iter()
        .map(|record| (record.submission_id, record))
        .collect::<BTreeMap<_, _>>();
    let limit = query.limit.unwrap_or(100).clamp(1, 500);
    let consent_scope = parse_consent_scope_filter(query.consent_scope.as_deref())?;

    let items = records
        .into_iter()
        .rev()
        .filter(|record| query.status == Some(TraceCorpusStatus::Revoked) || !record.is_revoked())
        .filter(|record| query.status.is_none_or(|status| record.status == status))
        .filter(|record| {
            query
                .privacy_risk
                .is_none_or(|risk| record.privacy_risk == risk)
        })
        .filter(|record| consent_scope.is_none_or(|scope| record.consent_scopes.contains(&scope)))
        .filter(|record| {
            trace_matches_derived_filters(
                derived_by_submission.get(&record.submission_id),
                query.coverage_tag.as_deref(),
                query.tool.as_deref(),
            )
        })
        .take(limit)
        .map(|record| TraceCommonsTraceListItem::from_record(record, &derived_by_submission))
        .collect();
    Ok(Json(items))
}

async fn review_quarantine_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> ApiResult<Json<Vec<TraceReviewQueueItem>>> {
    let tenant = authenticate(state.as_ref(), &headers)?;
    require_reviewer(&tenant)?;
    let records =
        read_all_submission_records(&state.root, &tenant.tenant_id).map_err(internal_error)?;
    let derived =
        read_all_derived_records(&state.root, &tenant.tenant_id).map_err(internal_error)?;
    let derived_by_submission = derived
        .into_iter()
        .map(|record| (record.submission_id, record))
        .collect::<BTreeMap<_, _>>();
    let queue = records
        .into_iter()
        .filter(|record| record.status == TraceCorpusStatus::Quarantined)
        .map(|record| TraceReviewQueueItem::from_record(record, &derived_by_submission))
        .collect();
    Ok(Json(queue))
}

#[derive(Debug, Deserialize)]
struct TraceReviewDecisionRequest {
    decision: TraceReviewDecision,
    #[serde(default)]
    reason: Option<String>,
    #[serde(default)]
    credit_points_pending: Option<f32>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum TraceReviewDecision {
    Approve,
    Reject,
}

#[derive(Debug, Deserialize)]
struct TraceCreditLedgerAppendRequest {
    event_type: TraceCreditLedgerEventType,
    credit_points_delta: f32,
    #[serde(default)]
    reason: Option<String>,
    #[serde(default)]
    external_ref: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum TraceCreditLedgerEventType {
    BenchmarkConversion,
    RegressionCatch,
    TrainingUtility,
    RankingUtility,
    ReviewerBonus,
    AbusePenalty,
}

async fn append_credit_event_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(submission_id): AxumPath<Uuid>,
    Json(body): Json<TraceCreditLedgerAppendRequest>,
) -> ApiResult<Json<TraceCommonsCreditLedgerRecord>> {
    let tenant = authenticate(state.as_ref(), &headers)?;
    require_reviewer(&tenant)?;
    if !body.credit_points_delta.is_finite() {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "credit_points_delta must be finite",
        ));
    }

    let submission = read_submission_record(&state.root, &tenant.tenant_id, submission_id)
        .map_err(internal_error)?
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "trace submission not found"))?;
    if submission.is_revoked() {
        return Err(api_error(
            StatusCode::CONFLICT,
            "revoked trace submissions are not eligible for delayed credit",
        ));
    }
    let event = TraceCommonsCreditLedgerRecord {
        event_id: Uuid::new_v4(),
        tenant_id: tenant.tenant_id.clone(),
        tenant_storage_ref: tenant_storage_ref(&tenant.tenant_id),
        submission_id,
        trace_id: submission.trace_id,
        auth_principal_ref: submission.auth_principal_ref,
        event_type: body.event_type,
        credit_points_delta: body.credit_points_delta,
        reason: body.reason,
        external_ref: body.external_ref,
        actor_role: tenant.role,
        actor_principal_ref: tenant.principal_ref,
        created_at: Utc::now(),
    };
    append_credit_event(&state.root, &tenant.tenant_id, &event).map_err(internal_error)?;
    Ok(Json(event))
}

async fn review_decision_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(submission_id): AxumPath<Uuid>,
    Json(body): Json<TraceReviewDecisionRequest>,
) -> ApiResult<Json<TraceSubmissionReceipt>> {
    let tenant = authenticate(state.as_ref(), &headers)?;
    require_reviewer(&tenant)?;
    let mut record = read_submission_record(&state.root, &tenant.tenant_id, submission_id)
        .map_err(internal_error)?
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "trace submission not found"))?;
    if record.is_revoked() {
        return Err(api_error(
            StatusCode::CONFLICT,
            "revoked trace submissions are not eligible for review approval",
        ));
    }
    let mut envelope = read_envelope_by_record(&state.root, &record).map_err(internal_error)?;

    match body.decision {
        TraceReviewDecision::Approve => {
            record.status = TraceCorpusStatus::Accepted;
            let pending_credit = body
                .credit_points_pending
                .unwrap_or_else(|| reviewer_credit_for_record(&record));
            record.credit_points_pending = pending_credit;
            record.credit_points_final = None;
            envelope.value.credit_points_pending = pending_credit;
            envelope.value.explanation =
                vec!["Approved after privacy review for the private redacted corpus.".to_string()];
            envelope.value_card.user_visible_explanation = envelope.value.explanation.clone();
            record.object_key = store_envelope(
                &state.root,
                &tenant.tenant_id,
                TraceCorpusStatus::Accepted,
                &envelope,
            )
            .map_err(internal_error)?;
        }
        TraceReviewDecision::Reject => {
            record.status = TraceCorpusStatus::Rejected;
            record.credit_points_pending = 0.0;
            record.credit_points_final = Some(0.0);
            envelope.value.credit_points_pending = 0.0;
            envelope.value.credit_points_final = Some(0.0);
            envelope.value.explanation =
                vec!["Rejected during privacy or quality review; no credit awarded.".to_string()];
            envelope.value_card.user_visible_explanation = envelope.value.explanation.clone();
            record.object_key = store_envelope(
                &state.root,
                &tenant.tenant_id,
                TraceCorpusStatus::Rejected,
                &envelope,
            )
            .map_err(internal_error)?;
        }
    }

    write_submission_record(&state.root, &record).map_err(internal_error)?;
    if let Some(mut derived) = read_derived_record(&state.root, &tenant.tenant_id, submission_id)
        .map_err(internal_error)?
    {
        derived.status = record.status;
        write_derived_record(&state.root, &derived).map_err(internal_error)?;
    }
    append_audit_event(
        &state.root,
        &tenant.tenant_id,
        TraceCommonsAuditEvent::review_decision(
            &tenant,
            submission_id,
            record.status,
            body.reason.as_deref(),
        ),
    )
    .map_err(internal_error)?;

    Ok(Json(receipt_from_record(&record)))
}

#[derive(Debug, Deserialize)]
struct DatasetExportQuery {
    limit: Option<usize>,
    purpose: Option<String>,
    status: Option<TraceCorpusStatus>,
    privacy_risk: Option<ResidualPiiRisk>,
    consent_scope: Option<String>,
}

async fn dataset_replay_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<DatasetExportQuery>,
) -> ApiResult<Json<TraceReplayDatasetExport>> {
    let tenant = authenticate(state.as_ref(), &headers)?;
    require_reviewer(&tenant)?;
    let records =
        read_all_submission_records(&state.root, &tenant.tenant_id).map_err(internal_error)?;
    let derived =
        read_all_derived_records(&state.root, &tenant.tenant_id).map_err(internal_error)?;
    let derived_by_submission = derived
        .into_iter()
        .map(|record| (record.submission_id, record))
        .collect::<BTreeMap<_, _>>();
    let limit = query.limit.unwrap_or(100).clamp(1, 500);
    let purpose = query
        .purpose
        .as_deref()
        .map(str::trim)
        .filter(|purpose| !purpose.is_empty())
        .unwrap_or("trace_commons_replay_dataset")
        .to_string();
    let consent_scope = parse_consent_scope_filter(query.consent_scope.as_deref())?;
    let mut items = Vec::new();
    for record in records
        .into_iter()
        .filter(|record| query.status.is_none_or(|status| record.status == status))
        .filter(|record| {
            query
                .privacy_risk
                .is_none_or(|risk| record.privacy_risk == risk)
        })
        .filter(|record| consent_scope.is_none_or(|scope| record.consent_scopes.contains(&scope)))
        .filter(TraceCommonsSubmissionRecord::is_export_eligible)
        .take(limit)
    {
        let envelope = read_envelope_by_record(&state.root, &record).map_err(internal_error)?;
        items.push(TraceReplayDatasetItem::from_record(
            &record,
            derived_by_submission.get(&record.submission_id),
            &envelope,
        ));
    }

    let export_id = Uuid::new_v4();
    let audit_event = TraceCommonsAuditEvent::dataset_export(&tenant, export_id, items.len());
    let audit_event_id = audit_event.event_id;
    let manifest = TraceReplayExportManifest::from_items(
        &tenant.tenant_id,
        export_id,
        audit_event_id,
        purpose,
        TraceReplayExportFilters {
            limit,
            consent_scope,
            status: query.status,
            privacy_risk: query.privacy_risk,
        },
        &items,
    );
    write_export_manifest(&state.root, &tenant.tenant_id, &manifest).map_err(internal_error)?;
    append_audit_event(&state.root, &tenant.tenant_id, audit_event).map_err(internal_error)?;
    Ok(Json(TraceReplayDatasetExport {
        tenant_id: tenant.tenant_id.clone(),
        tenant_storage_ref: tenant_storage_ref(&tenant.tenant_id),
        export_id,
        audit_event_id,
        created_at: Utc::now(),
        item_count: items.len(),
        manifest,
        items,
    }))
}

#[derive(Debug, Deserialize)]
struct BenchmarkConversionRequest {
    limit: Option<usize>,
    #[serde(default)]
    purpose: Option<String>,
    #[serde(default)]
    consent_scope: Option<String>,
    #[serde(default)]
    status: Option<TraceCorpusStatus>,
    #[serde(default)]
    privacy_risk: Option<ResidualPiiRisk>,
    #[serde(default)]
    external_ref: Option<String>,
}

async fn benchmark_convert_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<BenchmarkConversionRequest>,
) -> ApiResult<Json<TraceBenchmarkConversionArtifact>> {
    let tenant = authenticate(state.as_ref(), &headers)?;
    require_reviewer(&tenant)?;
    let records =
        read_all_submission_records(&state.root, &tenant.tenant_id).map_err(internal_error)?;
    let consent_scope = parse_consent_scope_filter(body.consent_scope.as_deref())?;
    let accepted_by_submission = records
        .into_iter()
        .filter(TraceCommonsSubmissionRecord::is_benchmark_eligible)
        .filter(|record| body.status.is_none_or(|status| record.status == status))
        .filter(|record| {
            body.privacy_risk
                .is_none_or(|risk| record.privacy_risk == risk)
        })
        .filter(|record| consent_scope.is_none_or(|scope| record.consent_scopes.contains(&scope)))
        .map(|record| (record.submission_id, record))
        .collect::<BTreeMap<_, _>>();
    let limit = body.limit.unwrap_or(100).clamp(1, 500);
    let purpose = body
        .purpose
        .filter(|purpose| !purpose.trim().is_empty())
        .unwrap_or_else(|| "trace_commons_benchmark_candidate_conversion".to_string());

    let derived =
        read_all_derived_records(&state.root, &tenant.tenant_id).map_err(internal_error)?;
    let mut candidates = Vec::new();
    for derived in derived
        .into_iter()
        .filter(|record| record.status == TraceCorpusStatus::Accepted)
    {
        let Some(submission) = accepted_by_submission.get(&derived.submission_id) else {
            continue;
        };
        candidates.push(TraceBenchmarkCandidate::from_records(submission, &derived));
        if candidates.len() >= limit {
            break;
        }
    }

    let conversion_id = Uuid::new_v4();
    let audit_event =
        TraceCommonsAuditEvent::benchmark_conversion(&tenant, conversion_id, candidates.len());
    let audit_event_id = audit_event.event_id;
    let artifact = TraceBenchmarkConversionArtifact {
        tenant_id: tenant.tenant_id.clone(),
        tenant_storage_ref: tenant_storage_ref(&tenant.tenant_id),
        conversion_id,
        audit_event_id,
        purpose,
        filters: TraceBenchmarkConversionFilters {
            limit,
            consent_scope,
            status: body.status,
            privacy_risk: body.privacy_risk,
            external_ref: body.external_ref,
        },
        source_submission_ids: candidates
            .iter()
            .map(|candidate| candidate.submission_id)
            .collect(),
        generated_at: Utc::now(),
        item_count: candidates.len(),
        candidates,
    };
    write_benchmark_artifact(&state.root, &tenant.tenant_id, &artifact).map_err(internal_error)?;
    append_audit_event(&state.root, &tenant.tenant_id, audit_event).map_err(internal_error)?;
    Ok(Json(artifact))
}

#[derive(Debug, Deserialize)]
struct RankerTrainingExportQuery {
    limit: Option<usize>,
    #[serde(default)]
    consent_scope: Option<String>,
    #[serde(default)]
    privacy_risk: Option<ResidualPiiRisk>,
}

async fn ranker_training_candidates_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<RankerTrainingExportQuery>,
) -> ApiResult<Json<TraceRankerTrainingCandidateExport>> {
    let tenant = authenticate(state.as_ref(), &headers)?;
    require_reviewer(&tenant)?;
    let consent_scope = parse_ranker_consent_scope_filter(query.consent_scope.as_deref())?;
    let mut candidate_query = query;
    candidate_query.limit = Some(candidate_query.limit.unwrap_or(100).clamp(1, 500));
    let candidates = collect_ranker_training_candidates(
        state.as_ref(),
        &tenant,
        &candidate_query,
        consent_scope,
    )
    .map_err(internal_error)?;
    let export_id = Uuid::new_v4();
    let audit_event = TraceCommonsAuditEvent::ranker_training_export(
        &tenant,
        export_id,
        "ranker_training_candidates_export",
        candidates.len(),
    );
    let audit_event_id = audit_event.event_id;
    append_audit_event(&state.root, &tenant.tenant_id, audit_event).map_err(internal_error)?;
    Ok(Json(TraceRankerTrainingCandidateExport {
        tenant_id: tenant.tenant_id.clone(),
        tenant_storage_ref: tenant_storage_ref(&tenant.tenant_id),
        export_id,
        audit_event_id,
        generated_at: Utc::now(),
        item_count: candidates.len(),
        candidates,
    }))
}

async fn ranker_training_pairs_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<RankerTrainingExportQuery>,
) -> ApiResult<Json<TraceRankerTrainingPairExport>> {
    let tenant = authenticate(state.as_ref(), &headers)?;
    require_reviewer(&tenant)?;
    let consent_scope = parse_ranker_consent_scope_filter(query.consent_scope.as_deref())?;
    let mut pair_query = query;
    let pair_limit = pair_query.limit.unwrap_or(100).clamp(1, 500);
    pair_query.limit = Some(pair_limit.saturating_add(1));
    let candidates =
        collect_ranker_training_candidates(state.as_ref(), &tenant, &pair_query, consent_scope)
            .map_err(internal_error)?;
    let pairs = build_ranker_training_pairs(&candidates, pair_limit);
    let export_id = Uuid::new_v4();
    let audit_event = TraceCommonsAuditEvent::ranker_training_export(
        &tenant,
        export_id,
        "ranker_training_pairs_export",
        pairs.len(),
    );
    let audit_event_id = audit_event.event_id;
    append_audit_event(&state.root, &tenant.tenant_id, audit_event).map_err(internal_error)?;
    Ok(Json(TraceRankerTrainingPairExport {
        tenant_id: tenant.tenant_id.clone(),
        tenant_storage_ref: tenant_storage_ref(&tenant.tenant_id),
        export_id,
        audit_event_id,
        generated_at: Utc::now(),
        item_count: pairs.len(),
        pairs,
    }))
}

#[derive(Debug, Deserialize)]
struct ActiveLearningQueueQuery {
    limit: Option<usize>,
    #[serde(default)]
    privacy_risk: Option<ResidualPiiRisk>,
}

async fn active_learning_review_queue_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<ActiveLearningQueueQuery>,
) -> ApiResult<Json<TraceActiveLearningReviewQueue>> {
    let tenant = authenticate(state.as_ref(), &headers)?;
    require_reviewer(&tenant)?;
    let records =
        read_all_submission_records(&state.root, &tenant.tenant_id).map_err(internal_error)?;
    let derived =
        read_all_derived_records(&state.root, &tenant.tenant_id).map_err(internal_error)?;
    let derived_by_submission = derived
        .into_iter()
        .map(|record| (record.submission_id, record))
        .collect::<BTreeMap<_, _>>();
    let limit = query.limit.unwrap_or(100).clamp(1, 501);
    let mut items = records
        .into_iter()
        .filter(|record| {
            matches!(
                record.status,
                TraceCorpusStatus::Accepted | TraceCorpusStatus::Quarantined
            )
        })
        .filter(|record| !record.is_revoked())
        .filter(|record| {
            query
                .privacy_risk
                .is_none_or(|risk| record.privacy_risk == risk)
        })
        .map(|record| {
            let submission_id = record.submission_id;
            TraceActiveLearningReviewItem::from_record(
                record,
                derived_by_submission.get(&submission_id),
            )
        })
        .collect::<Vec<_>>();
    items.sort_by(|left, right| {
        right
            .priority_score
            .total_cmp(&left.priority_score)
            .then_with(|| left.received_at.cmp(&right.received_at))
    });
    items.truncate(limit);
    Ok(Json(TraceActiveLearningReviewQueue {
        tenant_id: tenant.tenant_id.clone(),
        tenant_storage_ref: tenant_storage_ref(&tenant.tenant_id),
        generated_at: Utc::now(),
        item_count: items.len(),
        items,
    }))
}

#[derive(Debug, Deserialize)]
struct TraceMaintenanceRequest {
    #[serde(default)]
    purpose: Option<String>,
    #[serde(default)]
    dry_run: bool,
    #[serde(default = "default_true")]
    prune_export_cache: bool,
    #[serde(default)]
    max_export_age_hours: Option<i64>,
}

fn default_true() -> bool {
    true
}

async fn maintenance_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<TraceMaintenanceRequest>,
) -> ApiResult<Json<TraceMaintenanceResponse>> {
    let tenant = authenticate(state.as_ref(), &headers)?;
    require_reviewer(&tenant)?;
    let response = run_maintenance(state.as_ref(), &tenant, body).map_err(internal_error)?;
    Ok(Json(response))
}

#[derive(Debug, Deserialize)]
struct AuditEventsQuery {
    limit: Option<usize>,
}

async fn audit_events_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<AuditEventsQuery>,
) -> ApiResult<Json<Vec<TraceCommonsAuditEvent>>> {
    let tenant = authenticate(state.as_ref(), &headers)?;
    require_reviewer(&tenant)?;
    let limit = query.limit.unwrap_or(100).clamp(1, 500);
    let events = read_all_audit_events(&state.root, &tenant.tenant_id)
        .map_err(internal_error)?
        .into_iter()
        .rev()
        .take(limit)
        .collect();
    Ok(Json(events))
}

fn trace_matches_derived_filters(
    derived: Option<&TraceCommonsDerivedRecord>,
    coverage_tag: Option<&str>,
    tool: Option<&str>,
) -> bool {
    let Some(derived) = derived else {
        return coverage_tag.is_none() && tool.is_none();
    };
    let coverage_matches = coverage_tag.is_none_or(|coverage_tag| {
        derived
            .coverage_tags
            .iter()
            .any(|tag| tag.eq_ignore_ascii_case(coverage_tag))
    });
    let tool_matches = tool.is_none_or(|tool| {
        derived
            .tool_sequence
            .iter()
            .chain(derived.tool_categories.iter())
            .any(|candidate| candidate.eq_ignore_ascii_case(tool))
    });
    coverage_matches && tool_matches
}

fn collect_ranker_training_candidates(
    state: &AppState,
    tenant: &TenantAuth,
    query: &RankerTrainingExportQuery,
    consent_scope: Option<ConsentScope>,
) -> anyhow::Result<Vec<TraceRankerTrainingCandidate>> {
    let records = read_all_submission_records(&state.root, &tenant.tenant_id)?;
    let derived = read_all_derived_records(&state.root, &tenant.tenant_id)?;
    let derived_by_submission = derived
        .into_iter()
        .map(|record| (record.submission_id, record))
        .collect::<BTreeMap<_, _>>();
    let limit = query.limit.unwrap_or(100).clamp(1, 500);
    let mut candidates = records
        .into_iter()
        .filter(|record| matches!(record.status, TraceCorpusStatus::Accepted))
        .filter(|record| !record.is_revoked())
        .filter(|record| {
            query
                .privacy_risk
                .is_none_or(|risk| record.privacy_risk == risk)
        })
        .filter(|record| ranker_consent_matches(&record.consent_scopes, consent_scope))
        .filter_map(|record| {
            derived_by_submission
                .get(&record.submission_id)
                .map(|derived| TraceRankerTrainingCandidate::from_records(&record, derived))
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| {
        right
            .ranker_score
            .total_cmp(&left.ranker_score)
            .then_with(|| left.received_at.cmp(&right.received_at))
    });
    candidates.truncate(limit);
    Ok(candidates)
}

fn ranker_consent_matches(scopes: &[ConsentScope], requested: Option<ConsentScope>) -> bool {
    if let Some(requested) = requested {
        return is_ranker_training_consent_scope(requested) && scopes.contains(&requested);
    }
    scopes.iter().copied().any(is_ranker_training_consent_scope)
}

fn parse_ranker_consent_scope_filter(value: Option<&str>) -> ApiResult<Option<ConsentScope>> {
    let scope = parse_consent_scope_filter(value)?;
    if let Some(scope) = scope
        && !is_ranker_training_consent_scope(scope)
    {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "ranker training exports require ranking-training or model-training consent",
        ));
    }
    Ok(scope)
}

fn is_ranker_training_consent_scope(scope: ConsentScope) -> bool {
    matches!(
        scope,
        ConsentScope::RankingTraining | ConsentScope::ModelTraining
    )
}

fn build_ranker_training_pairs(
    candidates: &[TraceRankerTrainingCandidate],
    limit: usize,
) -> Vec<TraceRankerTrainingPair> {
    candidates
        .windows(2)
        .filter_map(|window| {
            let [preferred, rejected] = window else {
                return None;
            };
            if preferred.submission_id == rejected.submission_id {
                return None;
            }
            Some(TraceRankerTrainingPair::from_candidates(
                preferred, rejected,
            ))
        })
        .take(limit)
        .collect()
}

fn parse_consent_scope_filter(value: Option<&str>) -> ApiResult<Option<ConsentScope>> {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    let scope = match value {
        "debugging_evaluation" | "debugging-evaluation" => ConsentScope::DebuggingEvaluation,
        "benchmark_only" | "benchmark-only" => ConsentScope::BenchmarkOnly,
        "ranking_training" | "ranking-training" => ConsentScope::RankingTraining,
        "model_training" | "model-training" => ConsentScope::ModelTraining,
        _ => {
            return Err(api_error(
                StatusCode::BAD_REQUEST,
                format!("unsupported consent_scope filter: {value}"),
            ));
        }
    };
    Ok(Some(scope))
}

fn validate_envelope(envelope: &TraceContributionEnvelope) -> ApiResult<()> {
    if envelope.schema_version != TRACE_CONTRIBUTION_SCHEMA_VERSION {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "unsupported trace contribution schema version",
        ));
    }
    if !envelope.consent.revocable {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "trace contribution consent must be revocable",
        ));
    }
    if envelope
        .contributor
        .pseudonymous_contributor_id
        .as_deref()
        .unwrap_or_default()
        .trim()
        .is_empty()
    {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "trace contribution requires a pseudonymous contributor id",
        ));
    }
    Ok(())
}

fn authenticate(state: &AppState, headers: &HeaderMap) -> ApiResult<TenantAuth> {
    let authorization = headers
        .get(AUTHORIZATION)
        .ok_or_else(|| api_error(StatusCode::UNAUTHORIZED, "missing bearer token"))?
        .to_str()
        .map_err(|_| api_error(StatusCode::UNAUTHORIZED, "invalid bearer token header"))?;
    let token = authorization
        .strip_prefix("Bearer ")
        .ok_or_else(|| api_error(StatusCode::UNAUTHORIZED, "missing bearer token"))?
        .trim();
    state
        .tokens
        .get(token)
        .cloned()
        .ok_or_else(|| api_error(StatusCode::FORBIDDEN, "unknown tenant token"))
}

fn require_reviewer(auth: &TenantAuth) -> ApiResult<()> {
    if auth.role.can_review() {
        Ok(())
    } else {
        Err(api_error(
            StatusCode::FORBIDDEN,
            "reviewer or admin token required",
        ))
    }
}

fn can_access_submission(auth: &TenantAuth, record: &TraceCommonsSubmissionRecord) -> bool {
    auth.role.can_review()
        || record.auth_principal_ref == legacy_principal_ref()
        || record.auth_principal_ref == auth.principal_ref
}

fn visible_submission_records(
    auth: &TenantAuth,
    records: Vec<TraceCommonsSubmissionRecord>,
) -> Vec<TraceCommonsSubmissionRecord> {
    records
        .into_iter()
        .filter(|record| can_access_submission(auth, record))
        .collect()
}

fn can_access_credit_event(auth: &TenantAuth, event: &TraceCommonsCreditLedgerRecord) -> bool {
    auth.role.can_review()
        || event.auth_principal_ref == legacy_principal_ref()
        || event.auth_principal_ref == auth.principal_ref
}

fn visible_credit_events(
    auth: &TenantAuth,
    events: Vec<TraceCommonsCreditLedgerRecord>,
) -> Vec<TraceCommonsCreditLedgerRecord> {
    events
        .into_iter()
        .filter(|event| can_access_credit_event(auth, event))
        .collect()
}

fn eligible_credit_events_for_records(
    records: &[TraceCommonsSubmissionRecord],
    events: Vec<TraceCommonsCreditLedgerRecord>,
) -> Vec<TraceCommonsCreditLedgerRecord> {
    let eligible_submissions = records
        .iter()
        .filter(|record| !record.is_revoked())
        .map(|record| record.submission_id)
        .collect::<BTreeSet<_>>();
    events
        .into_iter()
        .filter(|event| eligible_submissions.contains(&event.submission_id))
        .collect()
}

fn reviewer_credit_for_record(record: &TraceCommonsSubmissionRecord) -> f32 {
    (0.5 + record.submission_score).clamp(0.5, 2.0)
}

fn status_for_risk(risk: ResidualPiiRisk) -> TraceCorpusStatus {
    match risk {
        ResidualPiiRisk::Low => TraceCorpusStatus::Accepted,
        ResidualPiiRisk::Medium | ResidualPiiRisk::High => TraceCorpusStatus::Quarantined,
    }
}

fn receipt_from_record(record: &TraceCommonsSubmissionRecord) -> TraceSubmissionReceipt {
    let explanation = match record.status {
        TraceCorpusStatus::Accepted => vec![
            "Accepted into the private redacted corpus.".to_string(),
            format!("Attributed to tenant {}", record.tenant_storage_ref),
        ],
        TraceCorpusStatus::Quarantined => vec![
            "Quarantined for privacy review; credit is pending review.".to_string(),
            format!("Attributed to tenant {}", record.tenant_storage_ref),
        ],
        TraceCorpusStatus::Revoked => vec!["Revoked and marked with a tombstone.".to_string()],
        TraceCorpusStatus::Rejected => vec!["Rejected by ingestion policy.".to_string()],
    };

    TraceSubmissionReceipt {
        status: record.status.as_str().to_string(),
        credit_points_pending: Some(record.credit_points_pending),
        credit_points_final: record.credit_points_final,
        explanation,
    }
}

fn submission_status_from_record(
    record: &TraceCommonsSubmissionRecord,
    credit_events: &[TraceCommonsCreditLedgerRecord],
) -> TraceSubmissionStatusUpdate {
    let receipt = receipt_from_record(record);
    let delayed_events = credit_events
        .iter()
        .filter(|event| event.submission_id == record.submission_id)
        .collect::<Vec<_>>();
    let ledger_points = delayed_events
        .iter()
        .map(|event| event.credit_points_delta)
        .sum::<f32>();
    let base_final = record
        .credit_points_final
        .unwrap_or(record.credit_points_pending);
    let credit_points_total = if delayed_events.is_empty() {
        None
    } else {
        Some(base_final + ledger_points)
    };
    let delayed_credit_explanations = delayed_events
        .iter()
        .rev()
        .take(5)
        .map(|event| {
            let reason = event
                .reason
                .as_deref()
                .filter(|reason| !reason.trim().is_empty())
                .unwrap_or("delayed utility credit");
            format!(
                "{:?}: {:+.2} ({})",
                event.event_type, event.credit_points_delta, reason
            )
        })
        .collect::<Vec<_>>();
    TraceSubmissionStatusUpdate {
        submission_id: record.submission_id,
        trace_id: record.trace_id,
        status: record.status.as_str().to_string(),
        credit_points_pending: record.credit_points_pending,
        credit_points_final: record.credit_points_final,
        credit_points_ledger: ledger_points,
        credit_points_total,
        explanation: receipt.explanation,
        delayed_credit_explanations,
    }
}

fn store_envelope(
    root: &Path,
    tenant_id: &str,
    status: TraceCorpusStatus,
    envelope: &TraceContributionEnvelope,
) -> anyhow::Result<String> {
    let tenant_key = tenant_storage_key(tenant_id);
    let object_key = format!(
        "tenants/{tenant_key}/objects/{}/{}.json",
        status.as_str(),
        envelope.submission_id
    );
    let path = root.join(&object_key);
    write_json_file(&path, envelope, "trace contribution envelope")?;
    Ok(object_key)
}

fn write_submission_record(
    root: &Path,
    record: &TraceCommonsSubmissionRecord,
) -> anyhow::Result<()> {
    let tenant_key = tenant_storage_key(&record.tenant_id);
    let path = root
        .join("tenants")
        .join(tenant_key)
        .join("metadata")
        .join(format!("{}.json", record.submission_id));
    write_json_file(&path, record, "trace contribution metadata")
}

fn read_submission_record(
    root: &Path,
    tenant_id: &str,
    submission_id: Uuid,
) -> anyhow::Result<Option<TraceCommonsSubmissionRecord>> {
    let tenant_key = tenant_storage_key(tenant_id);
    let path = root
        .join("tenants")
        .join(tenant_key)
        .join("metadata")
        .join(format!("{submission_id}.json"));
    if !path.exists() {
        return Ok(None);
    }
    let body = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read trace metadata {}", path.display()))?;
    serde_json::from_str(&body)
        .with_context(|| format!("failed to parse trace metadata {}", path.display()))
}

fn read_envelope_by_record(
    root: &Path,
    record: &TraceCommonsSubmissionRecord,
) -> anyhow::Result<TraceContributionEnvelope> {
    let path = root.join(&record.object_key);
    let body = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read trace object {}", path.display()))?;
    serde_json::from_str(&body)
        .with_context(|| format!("failed to parse trace object {}", path.display()))
}

fn read_all_submission_records(
    root: &Path,
    tenant_id: &str,
) -> anyhow::Result<Vec<TraceCommonsSubmissionRecord>> {
    let tenant_key = tenant_storage_key(tenant_id);
    let dir = root.join("tenants").join(tenant_key).join("metadata");
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut records = Vec::new();
    for entry in std::fs::read_dir(&dir)
        .with_context(|| format!("failed to read trace metadata dir {}", dir.display()))?
    {
        let entry = entry.context("failed to read trace metadata entry")?;
        let path = entry.path();
        if path.extension().is_none_or(|ext| ext != "json") {
            continue;
        }
        let body = std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read trace metadata {}", path.display()))?;
        let record: TraceCommonsSubmissionRecord = serde_json::from_str(&body)
            .with_context(|| format!("failed to parse trace metadata {}", path.display()))?;
        records.push(record);
    }
    records.sort_by_key(|record| record.received_at);
    Ok(records)
}

fn build_derived_precheck(
    envelope: &TraceContributionEnvelope,
    existing: &[TraceCommonsDerivedRecord],
) -> TraceCommonsDerivedPrecheck {
    let canonical_summary = canonical_summary_for_embedding(envelope);
    let canonical_summary_hash = sha256_prefixed(&canonical_summary);
    let nearest_trace_ids = existing
        .iter()
        .filter(|record| record.canonical_summary_hash == canonical_summary_hash)
        .map(|record| record.trace_id.to_string())
        .take(5)
        .collect::<Vec<_>>();
    let duplicate_score = if nearest_trace_ids.is_empty() {
        0.0
    } else {
        1.0
    };
    let novelty_score = if nearest_trace_ids.is_empty() {
        0.65
    } else {
        0.05
    };
    let coverage_tags = coverage_tags_for_envelope(envelope);

    TraceCommonsDerivedPrecheck {
        canonical_summary,
        canonical_summary_hash,
        nearest_trace_ids,
        novelty_score,
        duplicate_score,
        coverage_tags,
    }
}

fn apply_embedding_precheck(
    envelope: &mut TraceContributionEnvelope,
    precheck: &TraceCommonsDerivedPrecheck,
) {
    let mut embedding = envelope
        .embedding_analysis
        .take()
        .unwrap_or(EmbeddingAnalysisMetadata {
            embedding_model: Some("redacted-summary-hash-precheck-v1".to_string()),
            canonical_summary_hash: String::new(),
            trace_vector_id: None,
            nearest_trace_ids: Vec::new(),
            cluster_id: None,
            nearest_cluster_id: None,
            novelty_score: None,
            duplicate_score: None,
            coverage_tags: Vec::new(),
        });

    if embedding.embedding_model.is_none() {
        embedding.embedding_model = Some("redacted-summary-hash-precheck-v1".to_string());
    }
    embedding.canonical_summary_hash = precheck.canonical_summary_hash.clone();
    embedding.nearest_trace_ids = precheck.nearest_trace_ids.clone();
    embedding.novelty_score = Some(precheck.novelty_score);
    embedding.duplicate_score = Some(precheck.duplicate_score);
    embedding.coverage_tags = precheck.coverage_tags.clone();
    envelope.embedding_analysis = Some(embedding);
}

fn build_derived_record(
    tenant_id: &str,
    status: TraceCorpusStatus,
    envelope: &TraceContributionEnvelope,
    precheck: TraceCommonsDerivedPrecheck,
) -> TraceCommonsDerivedRecord {
    TraceCommonsDerivedRecord {
        tenant_id: tenant_id.to_string(),
        tenant_storage_ref: tenant_storage_ref(tenant_id),
        submission_id: envelope.submission_id,
        trace_id: envelope.trace_id,
        status,
        privacy_risk: envelope.privacy.residual_pii_risk,
        task_success: format!("{:?}", envelope.outcome.task_success),
        canonical_summary: precheck.canonical_summary,
        canonical_summary_hash: precheck.canonical_summary_hash,
        summary_model: "redacted-summary-hash-precheck-v1".to_string(),
        event_count: envelope.events.len(),
        tool_sequence: envelope.replay.required_tools.clone(),
        tool_categories: envelope
            .events
            .iter()
            .filter_map(|event| event.tool_category.clone())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect(),
        coverage_tags: precheck.coverage_tags,
        duplicate_score: precheck.duplicate_score,
        novelty_score: precheck.novelty_score,
        created_at: Utc::now(),
    }
}

fn coverage_tags_for_envelope(envelope: &TraceContributionEnvelope) -> Vec<String> {
    let mut tags = std::collections::BTreeSet::new();
    tags.insert(format!("channel:{:?}", envelope.ironclaw.channel).to_ascii_lowercase());
    tags.insert(format!("outcome:{:?}", envelope.outcome.task_success).to_ascii_lowercase());
    tags.insert(format!("privacy:{:?}", envelope.privacy.residual_pii_risk).to_ascii_lowercase());

    for tool in &envelope.replay.required_tools {
        tags.insert(format!("tool:{tool}"));
    }
    for event in &envelope.events {
        if let Some(category) = &event.tool_category {
            tags.insert(format!("tool_category:{category}"));
        }
        for failure_mode in &event.failure_modes {
            tags.insert(format!("failure:{failure_mode:?}").to_ascii_lowercase());
        }
    }
    for failure_mode in &envelope.outcome.failure_modes {
        tags.insert(format!("failure:{failure_mode:?}").to_ascii_lowercase());
    }
    tags.into_iter().collect()
}

fn write_derived_record(root: &Path, record: &TraceCommonsDerivedRecord) -> anyhow::Result<()> {
    let tenant_key = tenant_storage_key(&record.tenant_id);
    let path = root
        .join("tenants")
        .join(tenant_key)
        .join("derived")
        .join(format!("{}.json", record.submission_id));
    write_json_file(&path, record, "trace derived record")
}

fn read_derived_record(
    root: &Path,
    tenant_id: &str,
    submission_id: Uuid,
) -> anyhow::Result<Option<TraceCommonsDerivedRecord>> {
    let tenant_key = tenant_storage_key(tenant_id);
    let path = root
        .join("tenants")
        .join(tenant_key)
        .join("derived")
        .join(format!("{submission_id}.json"));
    if !path.exists() {
        return Ok(None);
    }
    let body = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read trace derived record {}", path.display()))?;
    serde_json::from_str(&body)
        .with_context(|| format!("failed to parse trace derived record {}", path.display()))
}

fn read_all_derived_records(
    root: &Path,
    tenant_id: &str,
) -> anyhow::Result<Vec<TraceCommonsDerivedRecord>> {
    let tenant_key = tenant_storage_key(tenant_id);
    let dir = root.join("tenants").join(tenant_key).join("derived");
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut records = Vec::new();
    for entry in std::fs::read_dir(&dir)
        .with_context(|| format!("failed to read trace derived dir {}", dir.display()))?
    {
        let entry = entry.context("failed to read trace derived entry")?;
        let path = entry.path();
        if path.extension().is_none_or(|ext| ext != "json") {
            continue;
        }
        let body = std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read trace derived record {}", path.display()))?;
        let record: TraceCommonsDerivedRecord = serde_json::from_str(&body)
            .with_context(|| format!("failed to parse trace derived record {}", path.display()))?;
        records.push(record);
    }
    records.sort_by_key(|record| record.created_at);
    Ok(records)
}

fn append_credit_event(
    root: &Path,
    tenant_id: &str,
    event: &TraceCommonsCreditLedgerRecord,
) -> anyhow::Result<()> {
    let tenant_key = tenant_storage_key(tenant_id);
    let path = root
        .join("tenants")
        .join(tenant_key)
        .join("credit_ledger")
        .join("events.jsonl");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create credit ledger dir {}", parent.display()))?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("failed to open credit ledger {}", path.display()))?;
    let line = serde_json::to_string(event).context("failed to serialize credit ledger event")?;
    writeln!(file, "{line}")
        .with_context(|| format!("failed to append credit ledger {}", path.display()))
}

fn read_all_credit_events(
    root: &Path,
    tenant_id: &str,
) -> anyhow::Result<Vec<TraceCommonsCreditLedgerRecord>> {
    let tenant_key = tenant_storage_key(tenant_id);
    let path = root
        .join("tenants")
        .join(tenant_key)
        .join("credit_ledger")
        .join("events.jsonl");
    if !path.exists() {
        return Ok(Vec::new());
    }

    let body = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read credit ledger {}", path.display()))?;
    let mut events = Vec::new();
    for (index, line) in body.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let event = serde_json::from_str(line).with_context(|| {
            format!(
                "failed to parse credit ledger event {} line {}",
                path.display(),
                index + 1
            )
        })?;
        events.push(event);
    }
    events.sort_by_key(|event: &TraceCommonsCreditLedgerRecord| event.created_at);
    Ok(events)
}

fn write_revocation(root: &Path, tombstone: &TraceCommonsRevocation) -> anyhow::Result<()> {
    let tenant_key = tenant_storage_key(&tombstone.tenant_id);
    let path = root
        .join("tenants")
        .join(tenant_key)
        .join("revocations")
        .join(format!("{}.json", tombstone.submission_id));
    write_json_file(&path, tombstone, "trace revocation tombstone")
}

fn append_audit_event(
    root: &Path,
    tenant_id: &str,
    event: TraceCommonsAuditEvent,
) -> anyhow::Result<()> {
    let tenant_key = tenant_storage_key(tenant_id);
    let path = root
        .join("tenants")
        .join(tenant_key)
        .join("audit")
        .join("events.jsonl");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create audit dir {}", parent.display()))?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("failed to open audit log {}", path.display()))?;
    let line = serde_json::to_string(&event).context("failed to serialize audit event")?;
    writeln!(file, "{line}")
        .with_context(|| format!("failed to append audit log {}", path.display()))
}

fn read_all_audit_events(
    root: &Path,
    tenant_id: &str,
) -> anyhow::Result<Vec<TraceCommonsAuditEvent>> {
    let tenant_key = tenant_storage_key(tenant_id);
    let path = root
        .join("tenants")
        .join(tenant_key)
        .join("audit")
        .join("events.jsonl");
    if !path.exists() {
        return Ok(Vec::new());
    }

    let body = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read audit log {}", path.display()))?;
    let mut events = Vec::new();
    for (index, line) in body.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let event: TraceCommonsAuditEvent = serde_json::from_str(line).with_context(|| {
            format!(
                "failed to parse audit event {} line {}",
                path.display(),
                index + 1
            )
        })?;
        if event.tenant_id == tenant_id {
            events.push(event);
        }
    }
    events.sort_by_key(|event| event.created_at);
    Ok(events)
}

fn read_all_revocations(
    root: &Path,
    tenant_id: &str,
) -> anyhow::Result<Vec<TraceCommonsRevocation>> {
    let tenant_key = tenant_storage_key(tenant_id);
    let dir = root.join("tenants").join(tenant_key).join("revocations");
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut revocations = Vec::new();
    for entry in std::fs::read_dir(&dir)
        .with_context(|| format!("failed to read revocation dir {}", dir.display()))?
    {
        let entry = entry.context("failed to read revocation entry")?;
        let path = entry.path();
        if path.extension().is_none_or(|ext| ext != "json") {
            continue;
        }
        let body = std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read revocation {}", path.display()))?;
        let revocation: TraceCommonsRevocation = serde_json::from_str(&body)
            .with_context(|| format!("failed to parse revocation {}", path.display()))?;
        revocations.push(revocation);
    }
    revocations.sort_by_key(|revocation| revocation.revoked_at);
    Ok(revocations)
}

fn write_export_manifest(
    root: &Path,
    tenant_id: &str,
    manifest: &TraceReplayExportManifest,
) -> anyhow::Result<()> {
    let path = export_artifact_dir(root, tenant_id, manifest.export_id).join("manifest.json");
    write_json_file(&path, manifest, "trace replay export manifest")
}

fn read_export_manifest(path: &Path) -> anyhow::Result<TraceReplayExportManifest> {
    let body = std::fs::read_to_string(path).with_context(|| {
        format!(
            "failed to read trace replay export manifest {}",
            path.display()
        )
    })?;
    serde_json::from_str(&body).with_context(|| {
        format!(
            "failed to parse trace replay export manifest {}",
            path.display()
        )
    })
}

fn write_benchmark_artifact(
    root: &Path,
    tenant_id: &str,
    artifact: &TraceBenchmarkConversionArtifact,
) -> anyhow::Result<()> {
    let path = benchmark_artifact_path(root, tenant_id, artifact.conversion_id);
    write_json_file(&path, artifact, "trace benchmark conversion artifact")
}

fn benchmark_artifact_path(root: &Path, tenant_id: &str, conversion_id: Uuid) -> PathBuf {
    let tenant_key = tenant_storage_key(tenant_id);
    root.join("tenants")
        .join(tenant_key)
        .join("benchmarks")
        .join(conversion_id.to_string())
        .join("artifact.json")
}

fn export_artifact_dir(root: &Path, tenant_id: &str, export_id: Uuid) -> PathBuf {
    let tenant_key = tenant_storage_key(tenant_id);
    root.join("tenants")
        .join(tenant_key)
        .join("exports")
        .join(export_id.to_string())
}

fn run_maintenance(
    state: &AppState,
    tenant: &TenantAuth,
    request: TraceMaintenanceRequest,
) -> anyhow::Result<TraceMaintenanceResponse> {
    let purpose = request
        .purpose
        .as_deref()
        .map(str::trim)
        .filter(|purpose| !purpose.is_empty())
        .unwrap_or("trace_commons_retention_revocation_maintenance")
        .to_string();
    let mut revoked_submission_ids = read_all_revocations(&state.root, &tenant.tenant_id)?
        .into_iter()
        .map(|revocation| revocation.submission_id)
        .collect::<BTreeSet<_>>();

    let records = read_all_submission_records(&state.root, &tenant.tenant_id)?;
    let mut records_marked_revoked = 0usize;
    for mut record in records {
        if record.is_revoked() {
            revoked_submission_ids.insert(record.submission_id);
            continue;
        }
        if revoked_submission_ids.contains(&record.submission_id) {
            records_marked_revoked += 1;
            if !request.dry_run {
                record.status = TraceCorpusStatus::Revoked;
                record.credit_points_final = Some(0.0);
                write_submission_record(&state.root, &record)?;
            }
        }
    }

    let derived = read_all_derived_records(&state.root, &tenant.tenant_id)?;
    let mut derived_marked_revoked = 0usize;
    for mut record in derived {
        if revoked_submission_ids.contains(&record.submission_id)
            && record.status != TraceCorpusStatus::Revoked
        {
            derived_marked_revoked += 1;
            if !request.dry_run {
                record.status = TraceCorpusStatus::Revoked;
                write_derived_record(&state.root, &record)?;
            }
        }
    }

    let export_cache_files_pruned = if request.prune_export_cache {
        prune_export_cache_files(
            &state.root,
            &tenant.tenant_id,
            &revoked_submission_ids,
            request.max_export_age_hours,
            request.dry_run,
        )?
    } else {
        0
    };

    let audit_event = TraceCommonsAuditEvent::maintenance(
        tenant,
        &purpose,
        request.dry_run,
        records_marked_revoked,
        derived_marked_revoked,
        export_cache_files_pruned,
    );
    let audit_event_id = audit_event.event_id;
    append_audit_event(&state.root, &tenant.tenant_id, audit_event)?;

    Ok(TraceMaintenanceResponse {
        tenant_id: tenant.tenant_id.clone(),
        tenant_storage_ref: tenant_storage_ref(&tenant.tenant_id),
        purpose,
        dry_run: request.dry_run,
        audit_event_id,
        revoked_submission_count: revoked_submission_ids.len(),
        records_marked_revoked,
        derived_marked_revoked,
        export_cache_files_pruned,
    })
}

fn prune_export_cache_files(
    root: &Path,
    tenant_id: &str,
    revoked_submission_ids: &BTreeSet<Uuid>,
    max_export_age_hours: Option<i64>,
    dry_run: bool,
) -> anyhow::Result<usize> {
    let tenant_key = tenant_storage_key(tenant_id);
    let exports_dir = root.join("tenants").join(tenant_key).join("exports");
    if !exports_dir.exists() {
        return Ok(0);
    }

    let mut pruned = 0usize;
    for entry in std::fs::read_dir(&exports_dir)
        .with_context(|| format!("failed to read export dir {}", exports_dir.display()))?
    {
        let entry = entry.context("failed to read export entry")?;
        let export_dir = entry.path();
        if !export_dir.is_dir() {
            continue;
        }
        let manifest_path = export_dir.join("manifest.json");
        if !manifest_path.exists() {
            continue;
        }
        let manifest = read_export_manifest(&manifest_path)?;
        let contains_revoked_source = manifest
            .source_submission_ids
            .iter()
            .any(|submission_id| revoked_submission_ids.contains(submission_id));
        let expired = max_export_age_hours
            .filter(|hours| *hours >= 0)
            .is_some_and(|hours| {
                manifest.generated_at <= Utc::now() - chrono::Duration::hours(hours)
            });
        if !contains_revoked_source && !expired {
            continue;
        }

        for cache_name in ["dataset.json", "export.json", "cache.json"] {
            let cache_path = export_dir.join(cache_name);
            if cache_path.exists() {
                pruned += 1;
                if !dry_run {
                    std::fs::remove_file(&cache_path).with_context(|| {
                        format!("failed to prune export cache file {}", cache_path.display())
                    })?;
                }
            }
        }
        if dry_run {
            continue;
        }
        let marker = TraceExportCachePruneMarker {
            pruned_at: Utc::now(),
            reason: if contains_revoked_source {
                "revoked_source".to_string()
            } else {
                "expired".to_string()
            },
            source_submission_ids: manifest.source_submission_ids,
        };
        write_json_file(
            &export_dir.join("pruned.json"),
            &marker,
            "trace replay export prune marker",
        )?;
    }

    Ok(pruned)
}

fn write_json_file<T: Serialize + ?Sized>(
    path: &Path,
    value: &T,
    label: &str,
) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {} dir {}", label, parent.display()))?;
    }
    let body = serde_json::to_string_pretty(value)
        .with_context(|| format!("failed to serialize {label}"))?;
    std::fs::write(path, body)
        .with_context(|| format!("failed to write {} {}", label, path.display()))
}

fn tenant_storage_key(tenant_id: &str) -> String {
    let digest = Sha256::digest(tenant_id.as_bytes());
    hex::encode(&digest[..16])
}

fn sha256_prefixed(input: &str) -> String {
    let digest = Sha256::digest(input.as_bytes());
    format!("sha256:{}", hex::encode(digest))
}

fn principal_storage_ref(token: &str) -> String {
    format!("principal_{}", sha256_prefixed(token))
}

fn legacy_principal_ref() -> String {
    "principal_legacy".to_string()
}

fn tenant_storage_ref(tenant_id: &str) -> String {
    format!("tenant_sha256:{}", tenant_storage_key(tenant_id))
}

type ApiResult<T> = Result<T, (StatusCode, Json<ApiError>)>;

#[derive(Debug, Serialize)]
struct ApiError {
    error: String,
}

fn api_error(status: StatusCode, message: impl Into<String>) -> (StatusCode, Json<ApiError>) {
    (
        status,
        Json(ApiError {
            error: message.into(),
        }),
    )
}

fn internal_error(error: impl std::fmt::Display) -> (StatusCode, Json<ApiError>) {
    tracing::error!(%error, "Trace Commons ingestion operation failed");
    api_error(
        StatusCode::INTERNAL_SERVER_ERROR,
        "trace commons operation failed",
    )
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum TraceCorpusStatus {
    Accepted,
    Quarantined,
    Rejected,
    Revoked,
}

impl TraceCorpusStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Quarantined => "quarantined",
            Self::Rejected => "rejected",
            Self::Revoked => "revoked",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TraceCommonsSubmissionRecord {
    tenant_id: String,
    tenant_storage_ref: String,
    #[serde(default = "legacy_principal_ref")]
    auth_principal_ref: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    submitted_tenant_scope_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    contributor_pseudonym: Option<String>,
    submission_id: Uuid,
    trace_id: Uuid,
    status: TraceCorpusStatus,
    privacy_risk: ResidualPiiRisk,
    submission_score: f32,
    credit_points_pending: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    credit_points_final: Option<f32>,
    consent_scopes: Vec<ConsentScope>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    redaction_counts: BTreeMap<String, u32>,
    received_at: DateTime<Utc>,
    object_key: String,
}

impl TraceCommonsSubmissionRecord {
    fn is_revoked(&self) -> bool {
        self.status == TraceCorpusStatus::Revoked
    }

    fn is_export_eligible(&self) -> bool {
        self.status == TraceCorpusStatus::Accepted && !self.is_revoked()
    }

    fn is_benchmark_eligible(&self) -> bool {
        self.is_export_eligible()
    }
}

#[derive(Debug, Clone)]
struct TraceCommonsDerivedPrecheck {
    canonical_summary: String,
    canonical_summary_hash: String,
    nearest_trace_ids: Vec<String>,
    novelty_score: f32,
    duplicate_score: f32,
    coverage_tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TraceCommonsDerivedRecord {
    tenant_id: String,
    tenant_storage_ref: String,
    submission_id: Uuid,
    trace_id: Uuid,
    status: TraceCorpusStatus,
    privacy_risk: ResidualPiiRisk,
    task_success: String,
    canonical_summary: String,
    canonical_summary_hash: String,
    summary_model: String,
    event_count: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    tool_sequence: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    tool_categories: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    coverage_tags: Vec<String>,
    duplicate_score: f32,
    novelty_score: f32,
    created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TraceCommonsCreditLedgerRecord {
    event_id: Uuid,
    tenant_id: String,
    tenant_storage_ref: String,
    submission_id: Uuid,
    trace_id: Uuid,
    auth_principal_ref: String,
    event_type: TraceCreditLedgerEventType,
    credit_points_delta: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    external_ref: Option<String>,
    actor_role: TokenRole,
    actor_principal_ref: String,
    created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
struct TraceCommonsTraceListItem {
    tenant_storage_ref: String,
    submission_id: Uuid,
    trace_id: Uuid,
    status: TraceCorpusStatus,
    privacy_risk: ResidualPiiRisk,
    submission_score: f32,
    credit_points_pending: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    credit_points_final: Option<f32>,
    consent_scopes: Vec<ConsentScope>,
    redaction_counts: BTreeMap<String, u32>,
    received_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    event_count: Option<usize>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    coverage_tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    tool_sequence: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    tool_categories: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    duplicate_score: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    novelty_score: Option<f32>,
}

impl TraceCommonsTraceListItem {
    fn from_record(
        record: TraceCommonsSubmissionRecord,
        derived_by_submission: &BTreeMap<Uuid, TraceCommonsDerivedRecord>,
    ) -> Self {
        let derived = derived_by_submission.get(&record.submission_id);
        Self {
            tenant_storage_ref: record.tenant_storage_ref,
            submission_id: record.submission_id,
            trace_id: record.trace_id,
            status: record.status,
            privacy_risk: record.privacy_risk,
            submission_score: record.submission_score,
            credit_points_pending: record.credit_points_pending,
            credit_points_final: record.credit_points_final,
            consent_scopes: record.consent_scopes,
            redaction_counts: record.redaction_counts,
            received_at: record.received_at,
            event_count: derived.map(|record| record.event_count),
            coverage_tags: derived
                .map(|record| record.coverage_tags.clone())
                .unwrap_or_default(),
            tool_sequence: derived
                .map(|record| record.tool_sequence.clone())
                .unwrap_or_default(),
            tool_categories: derived
                .map(|record| record.tool_categories.clone())
                .unwrap_or_default(),
            duplicate_score: derived.map(|record| record.duplicate_score),
            novelty_score: derived.map(|record| record.novelty_score),
        }
    }
}

#[derive(Debug, Serialize)]
struct TraceReviewQueueItem {
    submission_id: Uuid,
    trace_id: Uuid,
    status: TraceCorpusStatus,
    privacy_risk: ResidualPiiRisk,
    submission_score: f32,
    redaction_counts: BTreeMap<String, u32>,
    received_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    canonical_summary: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    coverage_tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    tool_sequence: Vec<String>,
}

impl TraceReviewQueueItem {
    fn from_record(
        record: TraceCommonsSubmissionRecord,
        derived_by_submission: &BTreeMap<Uuid, TraceCommonsDerivedRecord>,
    ) -> Self {
        let derived = derived_by_submission.get(&record.submission_id);
        Self {
            submission_id: record.submission_id,
            trace_id: record.trace_id,
            status: record.status,
            privacy_risk: record.privacy_risk,
            submission_score: record.submission_score,
            redaction_counts: record.redaction_counts,
            received_at: record.received_at,
            canonical_summary: derived.map(|record| record.canonical_summary.clone()),
            coverage_tags: derived
                .map(|record| record.coverage_tags.clone())
                .unwrap_or_default(),
            tool_sequence: derived
                .map(|record| record.tool_sequence.clone())
                .unwrap_or_default(),
        }
    }
}

#[derive(Debug, Serialize)]
struct TraceReplayDatasetExport {
    tenant_id: String,
    tenant_storage_ref: String,
    export_id: Uuid,
    audit_event_id: Uuid,
    created_at: DateTime<Utc>,
    item_count: usize,
    manifest: TraceReplayExportManifest,
    items: Vec<TraceReplayDatasetItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TraceReplayExportManifest {
    tenant_id: String,
    tenant_storage_ref: String,
    export_id: Uuid,
    purpose: String,
    filters: TraceReplayExportFilters,
    source_submission_ids: Vec<Uuid>,
    consent_scopes: Vec<ConsentScope>,
    generated_at: DateTime<Utc>,
    audit_event_id: Uuid,
}

impl TraceReplayExportManifest {
    fn from_items(
        tenant_id: &str,
        export_id: Uuid,
        audit_event_id: Uuid,
        purpose: String,
        filters: TraceReplayExportFilters,
        items: &[TraceReplayDatasetItem],
    ) -> Self {
        Self {
            tenant_id: tenant_id.to_string(),
            tenant_storage_ref: tenant_storage_ref(tenant_id),
            export_id,
            purpose,
            filters,
            source_submission_ids: items.iter().map(|item| item.submission_id).collect(),
            consent_scopes: items
                .iter()
                .flat_map(|item| item.consent_scopes.clone())
                .collect(),
            generated_at: Utc::now(),
            audit_event_id,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TraceReplayExportFilters {
    limit: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    consent_scope: Option<ConsentScope>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    status: Option<TraceCorpusStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    privacy_risk: Option<ResidualPiiRisk>,
}

#[derive(Debug, Serialize)]
struct TraceReplayDatasetItem {
    submission_id: Uuid,
    trace_id: Uuid,
    schema_version: String,
    consent_scopes: Vec<ConsentScope>,
    replayable: bool,
    required_tools: Vec<String>,
    tool_manifest_hashes: BTreeMap<String, String>,
    expected_assertions: Vec<serde_json::Value>,
    task_success: String,
    canonical_summary_hash: Option<String>,
    canonical_summary: Option<String>,
    coverage_tags: Vec<String>,
    submission_score: f32,
}

impl TraceReplayDatasetItem {
    fn from_record(
        record: &TraceCommonsSubmissionRecord,
        derived: Option<&TraceCommonsDerivedRecord>,
        envelope: &TraceContributionEnvelope,
    ) -> Self {
        Self {
            submission_id: record.submission_id,
            trace_id: record.trace_id,
            schema_version: envelope.schema_version.clone(),
            consent_scopes: envelope.consent.scopes.clone(),
            replayable: envelope.replay.replayable,
            required_tools: envelope.replay.required_tools.clone(),
            tool_manifest_hashes: envelope.replay.tool_manifest_hashes.clone(),
            expected_assertions: envelope.replay.expected_assertions.clone(),
            task_success: format!("{:?}", envelope.outcome.task_success),
            canonical_summary_hash: derived.map(|record| record.canonical_summary_hash.clone()),
            canonical_summary: derived.map(|record| record.canonical_summary.clone()),
            coverage_tags: derived
                .map(|record| record.coverage_tags.clone())
                .unwrap_or_default(),
            submission_score: record.submission_score,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct TraceBenchmarkConversionArtifact {
    tenant_id: String,
    tenant_storage_ref: String,
    conversion_id: Uuid,
    audit_event_id: Uuid,
    purpose: String,
    filters: TraceBenchmarkConversionFilters,
    source_submission_ids: Vec<Uuid>,
    generated_at: DateTime<Utc>,
    item_count: usize,
    candidates: Vec<TraceBenchmarkCandidate>,
}

#[derive(Debug, Serialize, Deserialize)]
struct TraceBenchmarkConversionFilters {
    limit: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    consent_scope: Option<ConsentScope>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    status: Option<TraceCorpusStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    privacy_risk: Option<ResidualPiiRisk>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    external_ref: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct TraceBenchmarkCandidate {
    submission_id: Uuid,
    trace_id: Uuid,
    canonical_summary_hash: String,
    canonical_summary: String,
    summary_model: String,
    task_success: String,
    event_count: usize,
    tool_sequence: Vec<String>,
    tool_categories: Vec<String>,
    coverage_tags: Vec<String>,
    novelty_score: f32,
    duplicate_score: f32,
    submission_score: f32,
    consent_scopes: Vec<ConsentScope>,
}

impl TraceBenchmarkCandidate {
    fn from_records(
        submission: &TraceCommonsSubmissionRecord,
        derived: &TraceCommonsDerivedRecord,
    ) -> Self {
        Self {
            submission_id: submission.submission_id,
            trace_id: submission.trace_id,
            canonical_summary_hash: derived.canonical_summary_hash.clone(),
            canonical_summary: derived.canonical_summary.clone(),
            summary_model: derived.summary_model.clone(),
            task_success: derived.task_success.clone(),
            event_count: derived.event_count,
            tool_sequence: derived.tool_sequence.clone(),
            tool_categories: derived.tool_categories.clone(),
            coverage_tags: derived.coverage_tags.clone(),
            novelty_score: derived.novelty_score,
            duplicate_score: derived.duplicate_score,
            submission_score: submission.submission_score,
            consent_scopes: submission.consent_scopes.clone(),
        }
    }
}

#[derive(Debug, Serialize)]
struct TraceRankerTrainingCandidateExport {
    tenant_id: String,
    tenant_storage_ref: String,
    export_id: Uuid,
    audit_event_id: Uuid,
    generated_at: DateTime<Utc>,
    item_count: usize,
    candidates: Vec<TraceRankerTrainingCandidate>,
}

#[derive(Debug, Serialize)]
struct TraceRankerTrainingPairExport {
    tenant_id: String,
    tenant_storage_ref: String,
    export_id: Uuid,
    audit_event_id: Uuid,
    generated_at: DateTime<Utc>,
    item_count: usize,
    pairs: Vec<TraceRankerTrainingPair>,
}

#[derive(Debug, Clone, Serialize)]
struct TraceRankerTrainingCandidate {
    submission_id: Uuid,
    trace_id: Uuid,
    status: TraceCorpusStatus,
    privacy_risk: ResidualPiiRisk,
    label: TraceRankerTrainingLabel,
    ranker_score: f32,
    submission_score: f32,
    credit_points_pending: f32,
    consent_scopes: Vec<ConsentScope>,
    redaction_counts: BTreeMap<String, u32>,
    canonical_summary_hash: String,
    canonical_summary: String,
    summary_model: String,
    task_success: String,
    event_count: usize,
    tool_sequence: Vec<String>,
    tool_categories: Vec<String>,
    coverage_tags: Vec<String>,
    novelty_score: f32,
    duplicate_score: f32,
    received_at: DateTime<Utc>,
}

impl TraceRankerTrainingCandidate {
    fn from_records(
        submission: &TraceCommonsSubmissionRecord,
        derived: &TraceCommonsDerivedRecord,
    ) -> Self {
        let label = TraceRankerTrainingLabel::from_status(submission.status);
        Self {
            submission_id: submission.submission_id,
            trace_id: submission.trace_id,
            status: submission.status,
            privacy_risk: submission.privacy_risk,
            label,
            ranker_score: label.score_prior() + submission.submission_score,
            submission_score: submission.submission_score,
            credit_points_pending: submission.credit_points_pending,
            consent_scopes: submission.consent_scopes.clone(),
            redaction_counts: submission.redaction_counts.clone(),
            canonical_summary_hash: derived.canonical_summary_hash.clone(),
            canonical_summary: derived.canonical_summary.clone(),
            summary_model: derived.summary_model.clone(),
            task_success: derived.task_success.clone(),
            event_count: derived.event_count,
            tool_sequence: derived.tool_sequence.clone(),
            tool_categories: derived.tool_categories.clone(),
            coverage_tags: derived.coverage_tags.clone(),
            novelty_score: derived.novelty_score,
            duplicate_score: derived.duplicate_score,
            received_at: submission.received_at,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum TraceRankerTrainingLabel {
    Accepted,
    NeedsReview,
}

impl TraceRankerTrainingLabel {
    fn from_status(status: TraceCorpusStatus) -> Self {
        match status {
            TraceCorpusStatus::Accepted => Self::Accepted,
            TraceCorpusStatus::Quarantined => Self::NeedsReview,
            TraceCorpusStatus::Rejected | TraceCorpusStatus::Revoked => Self::NeedsReview,
        }
    }

    fn score_prior(self) -> f32 {
        match self {
            Self::Accepted => 1.0,
            Self::NeedsReview => 0.0,
        }
    }
}

#[derive(Debug, Serialize)]
struct TraceRankerTrainingPair {
    preferred_submission_id: Uuid,
    rejected_submission_id: Uuid,
    preferred_trace_id: Uuid,
    rejected_trace_id: Uuid,
    preferred_score: f32,
    rejected_score: f32,
    reason: String,
    preferred: TraceRankerTrainingCandidate,
    rejected: TraceRankerTrainingCandidate,
}

impl TraceRankerTrainingPair {
    fn from_candidates(
        preferred: &TraceRankerTrainingCandidate,
        rejected: &TraceRankerTrainingCandidate,
    ) -> Self {
        Self {
            preferred_submission_id: preferred.submission_id,
            rejected_submission_id: rejected.submission_id,
            preferred_trace_id: preferred.trace_id,
            rejected_trace_id: rejected.trace_id,
            preferred_score: preferred.ranker_score,
            rejected_score: rejected.ranker_score,
            reason: "higher_ranker_score".to_string(),
            preferred: preferred.clone(),
            rejected: rejected.clone(),
        }
    }
}

#[derive(Debug, Serialize)]
struct TraceActiveLearningReviewQueue {
    tenant_id: String,
    tenant_storage_ref: String,
    generated_at: DateTime<Utc>,
    item_count: usize,
    items: Vec<TraceActiveLearningReviewItem>,
}

#[derive(Debug, Serialize)]
struct TraceActiveLearningReviewItem {
    submission_id: Uuid,
    trace_id: Uuid,
    status: TraceCorpusStatus,
    privacy_risk: ResidualPiiRisk,
    priority_score: f32,
    priority_reasons: Vec<String>,
    submission_score: f32,
    redaction_counts: BTreeMap<String, u32>,
    received_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    canonical_summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    canonical_summary_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    coverage_tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    tool_sequence: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    tool_categories: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    novelty_score: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    duplicate_score: Option<f32>,
}

impl TraceActiveLearningReviewItem {
    fn from_record(
        record: TraceCommonsSubmissionRecord,
        derived: Option<&TraceCommonsDerivedRecord>,
    ) -> Self {
        let (priority_score, priority_reasons) = active_learning_priority(&record, derived);
        Self {
            submission_id: record.submission_id,
            trace_id: record.trace_id,
            status: record.status,
            privacy_risk: record.privacy_risk,
            priority_score,
            priority_reasons,
            submission_score: record.submission_score,
            redaction_counts: record.redaction_counts,
            received_at: record.received_at,
            canonical_summary: derived.map(|record| record.canonical_summary.clone()),
            canonical_summary_hash: derived.map(|record| record.canonical_summary_hash.clone()),
            coverage_tags: derived
                .map(|record| record.coverage_tags.clone())
                .unwrap_or_default(),
            tool_sequence: derived
                .map(|record| record.tool_sequence.clone())
                .unwrap_or_default(),
            tool_categories: derived
                .map(|record| record.tool_categories.clone())
                .unwrap_or_default(),
            novelty_score: derived.map(|record| record.novelty_score),
            duplicate_score: derived.map(|record| record.duplicate_score),
        }
    }
}

fn active_learning_priority(
    record: &TraceCommonsSubmissionRecord,
    derived: Option<&TraceCommonsDerivedRecord>,
) -> (f32, Vec<String>) {
    let mut score = 0.0;
    let mut reasons = Vec::new();
    if record.status == TraceCorpusStatus::Quarantined {
        score += 2.0;
        reasons.push("quarantined_for_privacy_review".to_string());
    }
    match record.privacy_risk {
        ResidualPiiRisk::High => {
            score += 1.0;
            reasons.push("high_residual_pii_risk".to_string());
        }
        ResidualPiiRisk::Medium => {
            score += 0.5;
            reasons.push("medium_residual_pii_risk".to_string());
        }
        ResidualPiiRisk::Low => {}
    }
    let uncertainty = 1.0 - ((record.submission_score - 0.5).abs() * 2.0).clamp(0.0, 1.0);
    if uncertainty > 0.0 {
        score += uncertainty;
        reasons.push("uncertain_submission_score".to_string());
    }
    if let Some(derived) = derived {
        if derived.novelty_score >= 0.6 {
            score += 0.25;
            reasons.push("novel_trace_cluster".to_string());
        }
        if derived.duplicate_score >= 0.8 {
            score += 0.25;
            reasons.push("possible_duplicate".to_string());
        }
    }
    (score, reasons)
}

#[derive(Debug, Serialize, Deserialize)]
struct TraceExportCachePruneMarker {
    pruned_at: DateTime<Utc>,
    reason: String,
    source_submission_ids: Vec<Uuid>,
}

#[derive(Debug, Serialize)]
struct TraceMaintenanceResponse {
    tenant_id: String,
    tenant_storage_ref: String,
    purpose: String,
    dry_run: bool,
    audit_event_id: Uuid,
    revoked_submission_count: usize,
    records_marked_revoked: usize,
    derived_marked_revoked: usize,
    export_cache_files_pruned: usize,
}

#[derive(Debug, Serialize, Deserialize)]
struct TraceCommonsRevocation {
    tenant_id: String,
    tenant_storage_ref: String,
    submission_id: Uuid,
    revoked_at: DateTime<Utc>,
    reason: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct TraceCommonsAuditEvent {
    event_id: Uuid,
    tenant_id: String,
    submission_id: Uuid,
    kind: String,
    created_at: DateTime<Utc>,
    status: Option<TraceCorpusStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    actor_role: Option<TokenRole>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    actor_principal_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    export_count: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    export_id: Option<Uuid>,
}

impl TraceCommonsAuditEvent {
    fn submitted(record: &TraceCommonsSubmissionRecord) -> Self {
        Self {
            event_id: Uuid::new_v4(),
            tenant_id: record.tenant_id.clone(),
            submission_id: record.submission_id,
            kind: "submitted".to_string(),
            created_at: Utc::now(),
            status: Some(record.status),
            actor_role: None,
            actor_principal_ref: Some(record.auth_principal_ref.clone()),
            reason: None,
            export_count: None,
            export_id: None,
        }
    }

    fn idempotent_submit(auth: &TenantAuth, submission_id: Uuid) -> Self {
        Self {
            event_id: Uuid::new_v4(),
            tenant_id: auth.tenant_id.clone(),
            submission_id,
            kind: "idempotent_submit".to_string(),
            created_at: Utc::now(),
            status: None,
            actor_role: Some(auth.role),
            actor_principal_ref: Some(auth.principal_ref.clone()),
            reason: None,
            export_count: None,
            export_id: None,
        }
    }

    fn revoked(auth: &TenantAuth, submission_id: Uuid) -> Self {
        Self {
            event_id: Uuid::new_v4(),
            tenant_id: auth.tenant_id.clone(),
            submission_id,
            kind: "revoked".to_string(),
            created_at: Utc::now(),
            status: Some(TraceCorpusStatus::Revoked),
            actor_role: Some(auth.role),
            actor_principal_ref: Some(auth.principal_ref.clone()),
            reason: None,
            export_count: None,
            export_id: None,
        }
    }

    fn review_decision(
        auth: &TenantAuth,
        submission_id: Uuid,
        status: TraceCorpusStatus,
        reason: Option<&str>,
    ) -> Self {
        Self {
            event_id: Uuid::new_v4(),
            tenant_id: auth.tenant_id.clone(),
            submission_id,
            kind: "review_decision".to_string(),
            created_at: Utc::now(),
            status: Some(status),
            actor_role: Some(auth.role),
            actor_principal_ref: Some(auth.principal_ref.clone()),
            reason: reason.map(ToOwned::to_owned),
            export_count: None,
            export_id: None,
        }
    }

    fn dataset_export(auth: &TenantAuth, export_id: Uuid, export_count: usize) -> Self {
        Self {
            event_id: Uuid::new_v4(),
            tenant_id: auth.tenant_id.clone(),
            submission_id: Uuid::nil(),
            kind: "dataset_export".to_string(),
            created_at: Utc::now(),
            status: None,
            actor_role: Some(auth.role),
            actor_principal_ref: Some(auth.principal_ref.clone()),
            reason: None,
            export_count: Some(export_count),
            export_id: Some(export_id),
        }
    }

    fn benchmark_conversion(
        auth: &TenantAuth,
        conversion_id: Uuid,
        candidate_count: usize,
    ) -> Self {
        Self {
            event_id: Uuid::new_v4(),
            tenant_id: auth.tenant_id.clone(),
            submission_id: Uuid::nil(),
            kind: "benchmark_conversion".to_string(),
            created_at: Utc::now(),
            status: None,
            actor_role: Some(auth.role),
            actor_principal_ref: Some(auth.principal_ref.clone()),
            reason: None,
            export_count: Some(candidate_count),
            export_id: Some(conversion_id),
        }
    }

    fn ranker_training_export(
        auth: &TenantAuth,
        export_id: Uuid,
        kind: &str,
        item_count: usize,
    ) -> Self {
        Self {
            event_id: Uuid::new_v4(),
            tenant_id: auth.tenant_id.clone(),
            submission_id: Uuid::nil(),
            kind: kind.to_string(),
            created_at: Utc::now(),
            status: None,
            actor_role: Some(auth.role),
            actor_principal_ref: Some(auth.principal_ref.clone()),
            reason: None,
            export_count: Some(item_count),
            export_id: Some(export_id),
        }
    }

    fn maintenance(
        auth: &TenantAuth,
        purpose: &str,
        dry_run: bool,
        records_marked_revoked: usize,
        derived_marked_revoked: usize,
        export_cache_files_pruned: usize,
    ) -> Self {
        Self {
            event_id: Uuid::new_v4(),
            tenant_id: auth.tenant_id.clone(),
            submission_id: Uuid::nil(),
            kind: "maintenance".to_string(),
            created_at: Utc::now(),
            status: None,
            actor_role: Some(auth.role),
            actor_principal_ref: Some(auth.principal_ref.clone()),
            reason: Some(format!(
                "purpose={purpose};dry_run={dry_run};records_marked_revoked={records_marked_revoked};derived_marked_revoked={derived_marked_revoked};export_cache_files_pruned={export_cache_files_pruned}"
            )),
            export_count: Some(export_cache_files_pruned),
            export_id: None,
        }
    }
}

#[derive(Debug, Serialize)]
struct TraceCommonsTenantCreditResponse {
    tenant_id: String,
    tenant_storage_ref: String,
    accepted: usize,
    quarantined: usize,
    revoked: usize,
    rejected: usize,
    credit_points_pending: f32,
    credit_points_final: f32,
    credit_points_ledger: f32,
    credit_points_total: f32,
}

impl TraceCommonsTenantCreditResponse {
    fn from_records_and_events(
        tenant_id: String,
        records: Vec<TraceCommonsSubmissionRecord>,
        credit_events: &[TraceCommonsCreditLedgerRecord],
    ) -> Self {
        let mut response = Self {
            tenant_storage_ref: tenant_storage_ref(&tenant_id),
            tenant_id,
            accepted: 0,
            quarantined: 0,
            revoked: 0,
            rejected: 0,
            credit_points_pending: 0.0,
            credit_points_final: 0.0,
            credit_points_ledger: 0.0,
            credit_points_total: 0.0,
        };

        for record in records {
            match record.status {
                TraceCorpusStatus::Accepted => {
                    response.accepted += 1;
                    response.credit_points_pending += record.credit_points_pending;
                    response.credit_points_final += record
                        .credit_points_final
                        .unwrap_or(record.credit_points_pending);
                }
                TraceCorpusStatus::Quarantined => response.quarantined += 1,
                TraceCorpusStatus::Revoked => response.revoked += 1,
                TraceCorpusStatus::Rejected => response.rejected += 1,
            }
        }

        response.credit_points_ledger = credit_events
            .iter()
            .map(|event| event.credit_points_delta)
            .sum();
        response.credit_points_total = response.credit_points_final + response.credit_points_ledger;
        response
    }
}

#[derive(Debug, Serialize)]
struct TraceCommonsAnalyticsResponse {
    tenant_id: String,
    tenant_storage_ref: String,
    submissions_total: usize,
    by_status: BTreeMap<String, usize>,
    by_privacy_risk: BTreeMap<String, usize>,
    by_task_success: BTreeMap<String, usize>,
    by_tool: BTreeMap<String, usize>,
    by_tool_category: BTreeMap<String, usize>,
    coverage_tags: BTreeMap<String, usize>,
    duplicate_groups: usize,
    average_novelty_score: f32,
}

impl TraceCommonsAnalyticsResponse {
    fn from_records(
        tenant_id: String,
        records: Vec<TraceCommonsSubmissionRecord>,
        derived: Vec<TraceCommonsDerivedRecord>,
    ) -> Self {
        let mut response = Self {
            tenant_storage_ref: tenant_storage_ref(&tenant_id),
            tenant_id,
            submissions_total: records.len(),
            by_status: BTreeMap::new(),
            by_privacy_risk: BTreeMap::new(),
            by_task_success: BTreeMap::new(),
            by_tool: BTreeMap::new(),
            by_tool_category: BTreeMap::new(),
            coverage_tags: BTreeMap::new(),
            duplicate_groups: 0,
            average_novelty_score: 0.0,
        };

        for record in &records {
            *response
                .by_status
                .entry(record.status.as_str().to_string())
                .or_insert(0) += 1;
            *response
                .by_privacy_risk
                .entry(format!("{:?}", record.privacy_risk).to_ascii_lowercase())
                .or_insert(0) += 1;
        }

        let mut summary_hash_counts = BTreeMap::<String, usize>::new();
        let mut novelty_total = 0.0f32;
        for record in &derived {
            *response
                .by_task_success
                .entry(record.task_success.to_ascii_lowercase())
                .or_insert(0) += 1;
            for tool in &record.tool_sequence {
                *response.by_tool.entry(tool.clone()).or_insert(0) += 1;
            }
            for category in &record.tool_categories {
                *response
                    .by_tool_category
                    .entry(category.clone())
                    .or_insert(0) += 1;
            }
            for tag in &record.coverage_tags {
                *response.coverage_tags.entry(tag.clone()).or_insert(0) += 1;
            }
            *summary_hash_counts
                .entry(record.canonical_summary_hash.clone())
                .or_insert(0) += 1;
            novelty_total += record.novelty_score;
        }

        response.duplicate_groups = summary_hash_counts
            .values()
            .filter(|count| **count > 1)
            .count();
        if !derived.is_empty() {
            response.average_novelty_score = novelty_total / derived.len() as f32;
        }
        response
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        Json(self).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;
    use ironclaw::llm::recording::{TraceFile, TraceResponse, TraceStep};
    use ironclaw::trace_contribution::{
        DeterministicTraceRedactor, RecordedTraceContributionOptions, TraceRedactor,
    };

    fn test_state(root: PathBuf) -> Arc<AppState> {
        let mut tokens = BTreeMap::new();
        insert_token(&mut tokens, "tenant-a", "token-a", TokenRole::Contributor);
        insert_token(&mut tokens, "tenant-a", "token-a-2", TokenRole::Contributor);
        insert_token(
            &mut tokens,
            "tenant-a",
            "review-token-a",
            TokenRole::Reviewer,
        );
        insert_token(&mut tokens, "tenant-b", "token-b", TokenRole::Contributor);
        insert_token(
            &mut tokens,
            "tenant-b",
            "review-token-b",
            TokenRole::Reviewer,
        );
        Arc::new(AppState {
            root,
            tokens: Arc::new(tokens),
        })
    }

    fn auth_headers(token: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        let value = format!("Bearer {token}");
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&value).expect("valid auth header"),
        );
        headers
    }

    async fn sample_envelope() -> TraceContributionEnvelope {
        let trace = TraceFile {
            model_name: "test-model".to_string(),
            memory_snapshot: Vec::new(),
            http_exchanges: Vec::new(),
            steps: vec![TraceStep {
                request_hint: None,
                response: TraceResponse::UserInput {
                    content: "Please inspect the workspace".to_string(),
                },
                expected_tool_results: Vec::new(),
            }],
        };
        let raw = ironclaw::trace_contribution::RawTraceContribution::from_recorded_trace(
            &trace,
            RecordedTraceContributionOptions {
                include_message_text: true,
                pseudonymous_contributor_id: Some("sha256:contributor".to_string()),
                tenant_scope_ref: Some("tenant_sha256:client".to_string()),
                ..Default::default()
            },
        );
        DeterministicTraceRedactor::default()
            .redact_trace(raw)
            .await
            .expect("redaction should succeed")
    }

    fn make_metadata_only_low_risk(envelope: &mut TraceContributionEnvelope) {
        envelope.privacy.residual_pii_risk = ResidualPiiRisk::Low;
        envelope.consent.message_text_included = false;
        envelope.consent.tool_payloads_included = false;
        for event in &mut envelope.events {
            event.redacted_content = None;
            event.structured_payload = serde_json::Value::Null;
        }
    }

    #[tokio::test]
    async fn submit_rescrubs_and_stores_under_authenticated_tenant() {
        let temp = tempfile::tempdir().expect("temp dir");
        let state = test_state(temp.path().to_path_buf());
        let mut envelope = sample_envelope().await;
        envelope.events[0].redacted_content =
            Some("late leak at /tmp/ironclaw/private/token.txt".to_string());

        let Json(receipt) = submit_trace_handler(
            State(state.clone()),
            auth_headers("token-a"),
            Json(envelope.clone()),
        )
        .await
        .expect("submission succeeds");

        assert_eq!(receipt.status, "quarantined");
        let record = read_submission_record(temp.path(), "tenant-a", envelope.submission_id)
            .expect("record reads")
            .expect("record exists");
        assert_eq!(record.tenant_id, "tenant-a");
        assert_eq!(record.status, TraceCorpusStatus::Quarantined);
        let stored = std::fs::read_to_string(temp.path().join(record.object_key))
            .expect("stored envelope reads");
        assert!(stored.contains("server-rescrub-v1"));
        assert!(!stored.contains("/tmp/ironclaw/private/token.txt"));
    }

    #[tokio::test]
    async fn tenant_token_scopes_metadata_and_credit() {
        let temp = tempfile::tempdir().expect("temp dir");
        let state = test_state(temp.path().to_path_buf());
        let envelope = sample_envelope().await;
        let submission_id = envelope.submission_id;

        let _ = submit_trace_handler(
            State(state.clone()),
            auth_headers("token-b"),
            Json(envelope),
        )
        .await
        .expect("submission succeeds");

        assert!(
            read_submission_record(temp.path(), "tenant-a", submission_id)
                .expect("tenant a read")
                .is_none()
        );
        assert!(
            read_submission_record(temp.path(), "tenant-b", submission_id)
                .expect("tenant b read")
                .is_some()
        );
        let Json(credit) = credit_handler(State(state), auth_headers("token-b"))
            .await
            .expect("credit succeeds");
        assert_eq!(credit.tenant_id, "tenant-b");
        assert_eq!(credit.quarantined, 1);
    }

    #[tokio::test]
    async fn ingestion_writes_derived_analytics_and_duplicate_precheck() {
        let temp = tempfile::tempdir().expect("temp dir");
        let state = test_state(temp.path().to_path_buf());
        let first = sample_envelope().await;
        let second = sample_envelope().await;
        let second_id = second.submission_id;

        let _ = submit_trace_handler(State(state.clone()), auth_headers("token-a"), Json(first))
            .await
            .expect("first submission succeeds");
        let _ = submit_trace_handler(State(state.clone()), auth_headers("token-a"), Json(second))
            .await
            .expect("second submission succeeds");

        let derived = read_all_derived_records(temp.path(), "tenant-a").expect("derived reads");
        assert_eq!(derived.len(), 2);
        assert_eq!(
            derived[0].canonical_summary_hash,
            derived[1].canonical_summary_hash
        );
        assert_eq!(derived[0].duplicate_score, 0.0);
        assert_eq!(derived[1].duplicate_score, 1.0);
        assert!(
            derived[1]
                .coverage_tags
                .iter()
                .any(|tag| tag == "privacy:medium")
        );

        let record = read_submission_record(temp.path(), "tenant-a", second_id)
            .expect("record reads")
            .expect("record exists");
        let stored = std::fs::read_to_string(temp.path().join(record.object_key))
            .expect("stored envelope reads");
        assert!(stored.contains("\"duplicate_score\": 1.0"));

        let contributor_analytics_error =
            analytics_handler(State(state.clone()), auth_headers("token-a"))
                .await
                .expect_err("contributor cannot access tenant-wide analytics");
        assert_eq!(contributor_analytics_error.0, StatusCode::FORBIDDEN);

        let Json(analytics) = analytics_handler(State(state), auth_headers("review-token-a"))
            .await
            .expect("analytics succeeds");
        assert_eq!(analytics.submissions_total, 2);
        assert_eq!(analytics.duplicate_groups, 1);
        assert_eq!(analytics.by_privacy_risk.get("medium"), Some(&2));
    }

    #[tokio::test]
    async fn reviewer_can_approve_quarantined_trace_and_export_dataset() {
        let temp = tempfile::tempdir().expect("temp dir");
        let state = test_state(temp.path().to_path_buf());
        let envelope = sample_envelope().await;
        let submission_id = envelope.submission_id;

        let _ = submit_trace_handler(
            State(state.clone()),
            auth_headers("token-a"),
            Json(envelope),
        )
        .await
        .expect("submission succeeds");

        let contributor_error =
            review_quarantine_handler(State(state.clone()), auth_headers("token-a"))
                .await
                .expect_err("contributor cannot review");
        assert_eq!(contributor_error.0, StatusCode::FORBIDDEN);

        let Json(queue) =
            review_quarantine_handler(State(state.clone()), auth_headers("review-token-a"))
                .await
                .expect("review queue loads");
        assert_eq!(queue.len(), 1);
        assert_eq!(queue[0].submission_id, submission_id);

        let Json(receipt) = review_decision_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            AxumPath(submission_id),
            Json(TraceReviewDecisionRequest {
                decision: TraceReviewDecision::Approve,
                reason: Some("redaction looks safe".to_string()),
                credit_points_pending: Some(1.25),
            }),
        )
        .await
        .expect("review decision succeeds");
        assert_eq!(receipt.status, "accepted");
        assert_eq!(receipt.credit_points_pending, Some(1.25));

        let Json(statuses) = submission_status_handler(
            State(state.clone()),
            auth_headers("token-a"),
            Json(TraceSubmissionStatusRequest {
                submission_ids: vec![submission_id],
            }),
        )
        .await
        .expect("contributor can sync own known submission status");
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].submission_id, submission_id);
        assert_eq!(statuses[0].status, "accepted");
        assert_eq!(statuses[0].credit_points_pending, 1.25);

        let Json(cross_tenant_statuses) = submission_status_handler(
            State(state.clone()),
            auth_headers("token-b"),
            Json(TraceSubmissionStatusRequest {
                submission_ids: vec![submission_id],
            }),
        )
        .await
        .expect("cross-tenant status lookup returns no records");
        assert!(cross_tenant_statuses.is_empty());

        let Json(other_contributor_statuses) = submission_status_handler(
            State(state.clone()),
            auth_headers("token-a-2"),
            Json(TraceSubmissionStatusRequest {
                submission_ids: vec![submission_id],
            }),
        )
        .await
        .expect("same-tenant contributor status lookup is principal scoped");
        assert!(other_contributor_statuses.is_empty());

        let Json(other_contributor_credit) =
            credit_handler(State(state.clone()), auth_headers("token-a-2"))
                .await
                .expect("same-tenant contributor credit is principal scoped");
        assert_eq!(other_contributor_credit.accepted, 0);
        assert_eq!(other_contributor_credit.credit_points_pending, 0.0);

        let Json(export) = dataset_replay_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            Query(DatasetExportQuery {
                limit: Some(10),
                purpose: None,
                status: None,
                privacy_risk: None,
                consent_scope: None,
            }),
        )
        .await
        .expect("dataset export succeeds");
        assert_eq!(export.item_count, 1);
        assert_eq!(export.items[0].submission_id, submission_id);
        let audit_events =
            read_all_audit_events(temp.path(), "tenant-a").expect("audit events read");
        assert!(audit_events.iter().any(|event| {
            event.event_id == export.audit_event_id
                && event.export_id == Some(export.export_id)
                && event.kind == "dataset_export"
        }));

        let contributor_export_error = dataset_replay_handler(
            State(state),
            auth_headers("token-a"),
            Query(DatasetExportQuery {
                limit: Some(10),
                purpose: None,
                status: None,
                privacy_risk: None,
                consent_scope: None,
            }),
        )
        .await
        .expect_err("contributor cannot export datasets");
        assert_eq!(contributor_export_error.0, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn revoked_traces_are_excluded_from_export_and_benchmark_with_manifest_artifacts() {
        let temp = tempfile::tempdir().expect("temp dir");
        let state = test_state(temp.path().to_path_buf());
        let mut revoked = sample_envelope().await;
        make_metadata_only_low_risk(&mut revoked);
        let revoked_id = revoked.submission_id;
        let mut kept = sample_envelope().await;
        make_metadata_only_low_risk(&mut kept);
        let kept_id = kept.submission_id;

        let _ = submit_trace_handler(State(state.clone()), auth_headers("token-a"), Json(revoked))
            .await
            .expect("revoked candidate submission succeeds");
        let _ = submit_trace_handler(State(state.clone()), auth_headers("token-a"), Json(kept))
            .await
            .expect("kept submission succeeds");
        revoke_trace_handler(
            State(state.clone()),
            auth_headers("token-a"),
            AxumPath(revoked_id),
        )
        .await
        .expect("contributor can revoke own trace");

        let Json(export) = dataset_replay_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            Query(DatasetExportQuery {
                limit: Some(10),
                purpose: None,
                status: None,
                privacy_risk: None,
                consent_scope: None,
            }),
        )
        .await
        .expect("dataset export succeeds");
        assert_eq!(export.item_count, 1);
        assert_eq!(export.items[0].submission_id, kept_id);
        assert_eq!(export.manifest.source_submission_ids, vec![kept_id]);
        assert_eq!(export.manifest.audit_event_id, export.audit_event_id);
        assert!(!export.manifest.source_submission_ids.contains(&revoked_id));
        assert!(
            export_artifact_dir(temp.path(), "tenant-a", export.export_id)
                .join("manifest.json")
                .exists()
        );

        let Json(benchmark) = benchmark_convert_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            Json(BenchmarkConversionRequest {
                limit: Some(10),
                purpose: None,
                consent_scope: None,
                status: None,
                privacy_risk: None,
                external_ref: None,
            }),
        )
        .await
        .expect("benchmark conversion succeeds");
        assert_eq!(benchmark.item_count, 1);
        assert_eq!(benchmark.source_submission_ids, vec![kept_id]);
        assert_eq!(benchmark.candidates[0].submission_id, kept_id);
        assert!(!benchmark.source_submission_ids.contains(&revoked_id));
        assert!(benchmark_artifact_path(temp.path(), "tenant-a", benchmark.conversion_id).exists());

        let audit_events =
            read_all_audit_events(temp.path(), "tenant-a").expect("audit events read");
        assert!(audit_events.iter().any(|event| {
            event.event_id == benchmark.audit_event_id && event.kind == "benchmark_conversion"
        }));
    }

    #[tokio::test]
    async fn ranker_training_exports_are_tenant_scoped_and_exclude_revoked_traces() {
        let temp = tempfile::tempdir().expect("temp dir");
        let state = test_state(temp.path().to_path_buf());
        let mut tenant_a_best = sample_envelope().await;
        make_metadata_only_low_risk(&mut tenant_a_best);
        tenant_a_best.consent.scopes = vec![ConsentScope::RankingTraining];
        let tenant_a_best_id = tenant_a_best.submission_id;
        let mut tenant_a_lower = sample_envelope().await;
        make_metadata_only_low_risk(&mut tenant_a_lower);
        tenant_a_lower.consent.scopes = vec![ConsentScope::RankingTraining];
        tenant_a_lower.value.submission_score = 0.25;
        let tenant_a_lower_id = tenant_a_lower.submission_id;
        let mut tenant_a_quarantined = sample_envelope().await;
        tenant_a_quarantined.consent.scopes = vec![ConsentScope::RankingTraining];
        let tenant_a_quarantined_id = tenant_a_quarantined.submission_id;
        let mut tenant_a_revoked = sample_envelope().await;
        make_metadata_only_low_risk(&mut tenant_a_revoked);
        tenant_a_revoked.consent.scopes = vec![ConsentScope::RankingTraining];
        let tenant_a_revoked_id = tenant_a_revoked.submission_id;
        let mut tenant_b = sample_envelope().await;
        make_metadata_only_low_risk(&mut tenant_b);
        tenant_b.consent.scopes = vec![ConsentScope::RankingTraining];
        let tenant_b_id = tenant_b.submission_id;

        let _ = submit_trace_handler(
            State(state.clone()),
            auth_headers("token-a"),
            Json(tenant_a_best),
        )
        .await
        .expect("tenant a best submission succeeds");
        let _ = submit_trace_handler(
            State(state.clone()),
            auth_headers("token-a"),
            Json(tenant_a_lower),
        )
        .await
        .expect("tenant a lower submission succeeds");
        let _ = submit_trace_handler(
            State(state.clone()),
            auth_headers("token-a"),
            Json(tenant_a_quarantined),
        )
        .await
        .expect("tenant a quarantined submission succeeds");
        let _ = submit_trace_handler(
            State(state.clone()),
            auth_headers("token-a"),
            Json(tenant_a_revoked),
        )
        .await
        .expect("tenant a revoked submission succeeds");
        let _ = submit_trace_handler(
            State(state.clone()),
            auth_headers("token-b"),
            Json(tenant_b),
        )
        .await
        .expect("tenant b submission succeeds");
        revoke_trace_handler(
            State(state.clone()),
            auth_headers("token-a"),
            AxumPath(tenant_a_revoked_id),
        )
        .await
        .expect("contributor can revoke own trace");

        let contributor_error = ranker_training_candidates_handler(
            State(state.clone()),
            auth_headers("token-a"),
            Query(RankerTrainingExportQuery {
                limit: Some(10),
                consent_scope: None,
                privacy_risk: None,
            }),
        )
        .await
        .expect_err("contributors cannot export ranker candidates");
        assert_eq!(contributor_error.0, StatusCode::FORBIDDEN);

        let Json(candidates) = ranker_training_candidates_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            Query(RankerTrainingExportQuery {
                limit: Some(10),
                consent_scope: Some("ranking-training".to_string()),
                privacy_risk: None,
            }),
        )
        .await
        .expect("reviewer can export ranker candidates");
        assert_eq!(candidates.item_count, 2);
        assert_eq!(candidates.tenant_id, "tenant-a");
        assert!(
            candidates
                .candidates
                .iter()
                .any(|candidate| candidate.submission_id == tenant_a_best_id)
        );
        assert!(
            candidates
                .candidates
                .iter()
                .any(|candidate| candidate.submission_id == tenant_a_lower_id)
        );
        assert!(
            candidates
                .candidates
                .iter()
                .all(|candidate| candidate.status == TraceCorpusStatus::Accepted)
        );
        assert!(
            candidates
                .candidates
                .iter()
                .all(|candidate| candidate.submission_id != tenant_a_quarantined_id)
        );
        assert!(
            candidates
                .candidates
                .iter()
                .all(|candidate| candidate.submission_id != tenant_a_revoked_id)
        );
        assert!(
            candidates
                .candidates
                .iter()
                .all(|candidate| candidate.submission_id != tenant_b_id)
        );

        let Json(pairs) = ranker_training_pairs_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            Query(RankerTrainingExportQuery {
                limit: Some(10),
                consent_scope: Some("ranking-training".to_string()),
                privacy_risk: None,
            }),
        )
        .await
        .expect("reviewer can export ranker pairs");
        assert_eq!(pairs.item_count, 1);
        assert_eq!(pairs.pairs[0].preferred_submission_id, tenant_a_best_id);
        assert_eq!(pairs.pairs[0].rejected_submission_id, tenant_a_lower_id);
        assert!(
            pairs
                .pairs
                .iter()
                .all(|pair| pair.preferred_submission_id != tenant_a_revoked_id
                    && pair.rejected_submission_id != tenant_a_revoked_id
                    && pair.preferred_submission_id != tenant_a_quarantined_id
                    && pair.rejected_submission_id != tenant_a_quarantined_id
                    && pair.preferred_submission_id != tenant_b_id
                    && pair.rejected_submission_id != tenant_b_id)
        );

        let Json(limited_pairs) = ranker_training_pairs_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            Query(RankerTrainingExportQuery {
                limit: Some(1),
                consent_scope: Some("ranking-training".to_string()),
                privacy_risk: None,
            }),
        )
        .await
        .expect("pair limit counts pairs, not candidates");
        assert_eq!(limited_pairs.item_count, 1);

        let debugging_scope_error = ranker_training_candidates_handler(
            State(state),
            auth_headers("review-token-a"),
            Query(RankerTrainingExportQuery {
                limit: Some(10),
                consent_scope: Some("debugging-evaluation".to_string()),
                privacy_risk: None,
            }),
        )
        .await
        .expect_err("ranker exports require training consent");
        assert_eq!(debugging_scope_error.0, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn active_learning_queue_is_tenant_scoped_and_excludes_revoked_traces() {
        let temp = tempfile::tempdir().expect("temp dir");
        let state = test_state(temp.path().to_path_buf());
        let tenant_a_quarantined = sample_envelope().await;
        let tenant_a_quarantined_id = tenant_a_quarantined.submission_id;
        let mut tenant_a_accepted = sample_envelope().await;
        make_metadata_only_low_risk(&mut tenant_a_accepted);
        let tenant_a_accepted_id = tenant_a_accepted.submission_id;
        let mut tenant_a_revoked = sample_envelope().await;
        make_metadata_only_low_risk(&mut tenant_a_revoked);
        let tenant_a_revoked_id = tenant_a_revoked.submission_id;
        let tenant_b_quarantined = sample_envelope().await;
        let tenant_b_quarantined_id = tenant_b_quarantined.submission_id;

        let _ = submit_trace_handler(
            State(state.clone()),
            auth_headers("token-a"),
            Json(tenant_a_quarantined),
        )
        .await
        .expect("tenant a quarantined submission succeeds");
        let _ = submit_trace_handler(
            State(state.clone()),
            auth_headers("token-a"),
            Json(tenant_a_accepted),
        )
        .await
        .expect("tenant a accepted submission succeeds");
        let _ = submit_trace_handler(
            State(state.clone()),
            auth_headers("token-a"),
            Json(tenant_a_revoked),
        )
        .await
        .expect("tenant a revoked submission succeeds");
        let _ = submit_trace_handler(
            State(state.clone()),
            auth_headers("token-b"),
            Json(tenant_b_quarantined),
        )
        .await
        .expect("tenant b quarantined submission succeeds");
        revoke_trace_handler(
            State(state.clone()),
            auth_headers("token-a"),
            AxumPath(tenant_a_revoked_id),
        )
        .await
        .expect("contributor can revoke own trace");

        let Json(queue) = active_learning_review_queue_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            Query(ActiveLearningQueueQuery {
                limit: Some(10),
                privacy_risk: None,
            }),
        )
        .await
        .expect("reviewer can read active-learning queue");
        assert_eq!(queue.item_count, 2);
        assert_eq!(queue.tenant_id, "tenant-a");
        assert_eq!(queue.items[0].submission_id, tenant_a_quarantined_id);
        assert!(
            queue
                .items
                .iter()
                .any(|item| item.submission_id == tenant_a_accepted_id)
        );
        assert!(
            queue
                .items
                .iter()
                .all(|item| item.submission_id != tenant_a_revoked_id)
        );
        assert!(
            queue
                .items
                .iter()
                .all(|item| item.submission_id != tenant_b_quarantined_id)
        );
    }

    #[tokio::test]
    async fn maintenance_is_tenant_scoped_denies_contributors_and_prunes_revoked_export_cache() {
        let temp = tempfile::tempdir().expect("temp dir");
        let state = test_state(temp.path().to_path_buf());
        let mut tenant_a = sample_envelope().await;
        make_metadata_only_low_risk(&mut tenant_a);
        let tenant_a_id = tenant_a.submission_id;
        let mut tenant_b = sample_envelope().await;
        make_metadata_only_low_risk(&mut tenant_b);
        let tenant_b_id = tenant_b.submission_id;

        let _ = submit_trace_handler(
            State(state.clone()),
            auth_headers("token-a"),
            Json(tenant_a),
        )
        .await
        .expect("tenant a submission succeeds");
        let _ = submit_trace_handler(
            State(state.clone()),
            auth_headers("token-b"),
            Json(tenant_b),
        )
        .await
        .expect("tenant b submission succeeds");

        write_revocation(
            temp.path(),
            &TraceCommonsRevocation {
                tenant_id: "tenant-a".to_string(),
                tenant_storage_ref: tenant_storage_ref("tenant-a"),
                submission_id: tenant_a_id,
                revoked_at: Utc::now(),
                reason: "test_tombstone_only".to_string(),
            },
        )
        .expect("revocation tombstone writes");
        let export_id = Uuid::new_v4();
        let manifest = TraceReplayExportManifest {
            tenant_id: "tenant-a".to_string(),
            tenant_storage_ref: tenant_storage_ref("tenant-a"),
            export_id,
            purpose: "test_export_cache".to_string(),
            filters: TraceReplayExportFilters {
                limit: 10,
                consent_scope: None,
                status: None,
                privacy_risk: None,
            },
            source_submission_ids: vec![tenant_a_id],
            consent_scopes: read_submission_record(temp.path(), "tenant-a", tenant_a_id)
                .expect("tenant a record reads")
                .expect("tenant a record exists")
                .consent_scopes,
            generated_at: Utc::now(),
            audit_event_id: Uuid::new_v4(),
        };
        write_export_manifest(temp.path(), "tenant-a", &manifest).expect("export manifest writes");
        let cached_export_path =
            export_artifact_dir(temp.path(), "tenant-a", export_id).join("dataset.json");
        write_json_file(
            &cached_export_path,
            &serde_json::json!({"cached": true}),
            "test export cache",
        )
        .expect("test cache writes");

        let contributor_error = maintenance_handler(
            State(state.clone()),
            auth_headers("token-a"),
            Json(TraceMaintenanceRequest {
                purpose: None,
                dry_run: false,
                prune_export_cache: true,
                max_export_age_hours: None,
            }),
        )
        .await
        .expect_err("contributors cannot run maintenance");
        assert_eq!(contributor_error.0, StatusCode::FORBIDDEN);

        let Json(response) = maintenance_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            Json(TraceMaintenanceRequest {
                purpose: Some("test_retention".to_string()),
                dry_run: false,
                prune_export_cache: true,
                max_export_age_hours: None,
            }),
        )
        .await
        .expect("reviewer can run maintenance");
        assert_eq!(response.tenant_id, "tenant-a");
        assert_eq!(response.records_marked_revoked, 1);
        assert_eq!(response.derived_marked_revoked, 1);
        assert_eq!(response.export_cache_files_pruned, 1);
        assert!(!cached_export_path.exists());
        assert!(
            export_artifact_dir(temp.path(), "tenant-a", export_id)
                .join("pruned.json")
                .exists()
        );

        let tenant_a_record = read_submission_record(temp.path(), "tenant-a", tenant_a_id)
            .expect("tenant a record reads")
            .expect("tenant a record exists");
        assert_eq!(tenant_a_record.status, TraceCorpusStatus::Revoked);
        let tenant_a_derived = read_derived_record(temp.path(), "tenant-a", tenant_a_id)
            .expect("tenant a derived reads")
            .expect("tenant a derived exists");
        assert_eq!(tenant_a_derived.status, TraceCorpusStatus::Revoked);
        let tenant_b_record = read_submission_record(temp.path(), "tenant-b", tenant_b_id)
            .expect("tenant b record reads")
            .expect("tenant b record exists");
        assert_eq!(tenant_b_record.status, TraceCorpusStatus::Accepted);

        let audit_events =
            read_all_audit_events(temp.path(), "tenant-a").expect("audit events read");
        assert!(audit_events.iter().any(|event| {
            event.event_id == response.audit_event_id && event.kind == "maintenance"
        }));
    }

    #[tokio::test]
    async fn revoked_trace_cannot_be_approved_or_receive_credit_and_listing_skips_by_default() {
        let temp = tempfile::tempdir().expect("temp dir");
        let state = test_state(temp.path().to_path_buf());
        let envelope = sample_envelope().await;
        let submission_id = envelope.submission_id;

        let _ = submit_trace_handler(
            State(state.clone()),
            auth_headers("token-a"),
            Json(envelope),
        )
        .await
        .expect("submission succeeds");
        revoke_trace_handler(
            State(state.clone()),
            auth_headers("token-a"),
            AxumPath(submission_id),
        )
        .await
        .expect("contributor can revoke own trace");

        let approval_error = review_decision_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            AxumPath(submission_id),
            Json(TraceReviewDecisionRequest {
                decision: TraceReviewDecision::Approve,
                reason: Some("should be gated".to_string()),
                credit_points_pending: None,
            }),
        )
        .await
        .expect_err("revoked trace cannot be approved");
        assert_eq!(approval_error.0, StatusCode::CONFLICT);

        let credit_error = append_credit_event_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            AxumPath(submission_id),
            Json(TraceCreditLedgerAppendRequest {
                event_type: TraceCreditLedgerEventType::TrainingUtility,
                credit_points_delta: 1.0,
                reason: None,
                external_ref: None,
            }),
        )
        .await
        .expect_err("revoked trace cannot receive delayed credit");
        assert_eq!(credit_error.0, StatusCode::CONFLICT);

        let Json(default_listing) = list_traces_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            Query(TraceListQuery {
                status: None,
                limit: Some(10),
                coverage_tag: None,
                tool: None,
                privacy_risk: None,
                consent_scope: None,
            }),
        )
        .await
        .expect("reviewer can list traces");
        assert!(default_listing.is_empty());

        let Json(revoked_listing) = list_traces_handler(
            State(state),
            auth_headers("review-token-a"),
            Query(TraceListQuery {
                status: Some(TraceCorpusStatus::Revoked),
                limit: Some(10),
                coverage_tag: None,
                tool: None,
                privacy_risk: None,
                consent_scope: None,
            }),
        )
        .await
        .expect("reviewer can explicitly list revoked traces");
        assert_eq!(revoked_listing.len(), 1);
        assert_eq!(revoked_listing[0].submission_id, submission_id);
    }

    #[tokio::test]
    async fn reviewer_tokens_are_tenant_scoped() {
        let temp = tempfile::tempdir().expect("temp dir");
        let state = test_state(temp.path().to_path_buf());
        let envelope = sample_envelope().await;
        let submission_id = envelope.submission_id;

        let _ = submit_trace_handler(
            State(state.clone()),
            auth_headers("token-b"),
            Json(envelope),
        )
        .await
        .expect("submission succeeds");

        let error = review_decision_handler(
            State(state),
            auth_headers("review-token-a"),
            AxumPath(submission_id),
            Json(TraceReviewDecisionRequest {
                decision: TraceReviewDecision::Reject,
                reason: Some("wrong tenant should not see this".to_string()),
                credit_points_pending: None,
            }),
        )
        .await
        .expect_err("reviewer cannot cross tenant boundary");
        assert_eq!(error.0, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn contributor_cannot_access_trace_list_or_audit_events() {
        let temp = tempfile::tempdir().expect("temp dir");
        let state = test_state(temp.path().to_path_buf());
        let envelope = sample_envelope().await;

        let _ = submit_trace_handler(
            State(state.clone()),
            auth_headers("token-a"),
            Json(envelope),
        )
        .await
        .expect("submission succeeds");

        let list_error = list_traces_handler(
            State(state.clone()),
            auth_headers("token-a"),
            Query(TraceListQuery {
                status: None,
                limit: None,
                coverage_tag: None,
                tool: None,
                privacy_risk: None,
                consent_scope: None,
            }),
        )
        .await
        .expect_err("contributors cannot list tenant traces");
        assert_eq!(list_error.0, StatusCode::FORBIDDEN);

        let audit_error = audit_events_handler(
            State(state),
            auth_headers("token-a"),
            Query(AuditEventsQuery { limit: None }),
        )
        .await
        .expect_err("contributors cannot read audit events");
        assert_eq!(audit_error.0, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn reviewer_trace_list_filters_metadata_by_status_tool_tag_and_risk() {
        let temp = tempfile::tempdir().expect("temp dir");
        let state = test_state(temp.path().to_path_buf());
        let mut shell = sample_envelope().await;
        make_metadata_only_low_risk(&mut shell);
        shell.replay.required_tools.push("shell".to_string());
        let shell_id = shell.submission_id;
        let mut browser = sample_envelope().await;
        make_metadata_only_low_risk(&mut browser);
        browser.replay.required_tools.push("browser".to_string());
        let browser_id = browser.submission_id;
        let quarantined = sample_envelope().await;
        let quarantined_id = quarantined.submission_id;

        let _ = submit_trace_handler(State(state.clone()), auth_headers("token-a"), Json(shell))
            .await
            .expect("shell submission succeeds");
        let _ = submit_trace_handler(State(state.clone()), auth_headers("token-a"), Json(browser))
            .await
            .expect("browser submission succeeds");
        let _ = submit_trace_handler(
            State(state.clone()),
            auth_headers("token-a"),
            Json(quarantined),
        )
        .await
        .expect("quarantined submission succeeds");

        let Json(accepted_shell) = list_traces_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            Query(TraceListQuery {
                status: Some(TraceCorpusStatus::Accepted),
                limit: Some(10),
                coverage_tag: Some("tool:shell".to_string()),
                tool: Some("shell".to_string()),
                privacy_risk: Some(ResidualPiiRisk::Low),
                consent_scope: None,
            }),
        )
        .await
        .expect("reviewer can list filtered traces");
        assert_eq!(accepted_shell.len(), 1);
        assert_eq!(accepted_shell[0].submission_id, shell_id);
        assert_eq!(accepted_shell[0].status, TraceCorpusStatus::Accepted);
        assert!(
            accepted_shell[0]
                .coverage_tags
                .contains(&"tool:shell".to_string())
        );
        assert!(
            accepted_shell[0]
                .tool_sequence
                .contains(&"shell".to_string())
        );

        let Json(accepted) = list_traces_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            Query(TraceListQuery {
                status: Some(TraceCorpusStatus::Accepted),
                limit: Some(10),
                coverage_tag: None,
                tool: None,
                privacy_risk: Some(ResidualPiiRisk::Low),
                consent_scope: None,
            }),
        )
        .await
        .expect("reviewer can list accepted traces");
        assert_eq!(accepted.len(), 2);
        assert!(
            accepted
                .iter()
                .any(|record| record.submission_id == browser_id)
        );

        let Json(debugging_scope_records) = list_traces_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            Query(TraceListQuery {
                status: Some(TraceCorpusStatus::Accepted),
                limit: Some(10),
                coverage_tag: None,
                tool: None,
                privacy_risk: Some(ResidualPiiRisk::Low),
                consent_scope: Some("debugging-evaluation".to_string()),
            }),
        )
        .await
        .expect("reviewer can list by consent scope");
        assert_eq!(debugging_scope_records.len(), 2);

        let Json(model_scope_records) = list_traces_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            Query(TraceListQuery {
                status: Some(TraceCorpusStatus::Accepted),
                limit: Some(10),
                coverage_tag: None,
                tool: None,
                privacy_risk: Some(ResidualPiiRisk::Low),
                consent_scope: Some("model-training".to_string()),
            }),
        )
        .await
        .expect("reviewer can list by absent consent scope");
        assert!(model_scope_records.is_empty());

        let Json(quarantined_records) = list_traces_handler(
            State(state),
            auth_headers("review-token-a"),
            Query(TraceListQuery {
                status: Some(TraceCorpusStatus::Quarantined),
                limit: Some(10),
                coverage_tag: None,
                tool: None,
                privacy_risk: Some(ResidualPiiRisk::Medium),
                consent_scope: None,
            }),
        )
        .await
        .expect("reviewer can list quarantined traces");
        assert_eq!(quarantined_records.len(), 1);
        assert_eq!(quarantined_records[0].submission_id, quarantined_id);
    }

    #[tokio::test]
    async fn audit_events_are_tenant_scoped_for_reviewers() {
        let temp = tempfile::tempdir().expect("temp dir");
        let state = test_state(temp.path().to_path_buf());
        let tenant_a = sample_envelope().await;
        let tenant_a_id = tenant_a.submission_id;
        let tenant_b = sample_envelope().await;
        let tenant_b_id = tenant_b.submission_id;

        let _ = submit_trace_handler(
            State(state.clone()),
            auth_headers("token-a"),
            Json(tenant_a),
        )
        .await
        .expect("tenant a submission succeeds");
        let _ = submit_trace_handler(
            State(state.clone()),
            auth_headers("token-b"),
            Json(tenant_b),
        )
        .await
        .expect("tenant b submission succeeds");

        let Json(tenant_a_events) = audit_events_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            Query(AuditEventsQuery { limit: Some(50) }),
        )
        .await
        .expect("tenant a reviewer can read audit events");
        assert!(!tenant_a_events.is_empty());
        assert!(
            tenant_a_events
                .iter()
                .all(|event| event.tenant_id == "tenant-a")
        );
        assert!(
            tenant_a_events
                .iter()
                .any(|event| event.submission_id == tenant_a_id)
        );
        assert!(
            tenant_a_events
                .iter()
                .all(|event| event.submission_id != tenant_b_id)
        );
        let Json(tenant_a_traces) = list_traces_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            Query(TraceListQuery {
                status: None,
                limit: Some(50),
                coverage_tag: None,
                tool: None,
                privacy_risk: None,
                consent_scope: None,
            }),
        )
        .await
        .expect("tenant a reviewer can list own traces");
        assert_eq!(tenant_a_traces.len(), 1);
        assert_eq!(tenant_a_traces[0].submission_id, tenant_a_id);
        assert_ne!(tenant_a_traces[0].submission_id, tenant_b_id);

        let Json(tenant_b_traces) = list_traces_handler(
            State(state),
            auth_headers("review-token-b"),
            Query(TraceListQuery {
                status: None,
                limit: Some(50),
                coverage_tag: None,
                tool: None,
                privacy_risk: None,
                consent_scope: None,
            }),
        )
        .await
        .expect("tenant b reviewer can list own traces");
        assert_eq!(tenant_b_traces.len(), 1);
        assert_eq!(tenant_b_traces[0].submission_id, tenant_b_id);
    }

    #[tokio::test]
    async fn reviewer_can_append_delayed_credit_event() {
        let temp = tempfile::tempdir().expect("temp dir");
        let state = test_state(temp.path().to_path_buf());
        let envelope = sample_envelope().await;
        let submission_id = envelope.submission_id;

        let _ = submit_trace_handler(
            State(state.clone()),
            auth_headers("token-a"),
            Json(envelope),
        )
        .await
        .expect("submission succeeds");

        let Json(event) = append_credit_event_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            AxumPath(submission_id),
            Json(TraceCreditLedgerAppendRequest {
                event_type: TraceCreditLedgerEventType::BenchmarkConversion,
                credit_points_delta: 2.5,
                reason: Some("converted into benchmark".to_string()),
                external_ref: Some("benchmark:trace-replay-smoke".to_string()),
            }),
        )
        .await
        .expect("reviewer can append delayed credit");

        assert_eq!(event.submission_id, submission_id);
        assert_eq!(
            event.event_type,
            TraceCreditLedgerEventType::BenchmarkConversion
        );
        assert_eq!(event.credit_points_delta, 2.5);

        let events = read_all_credit_events(temp.path(), "tenant-a").expect("ledger reads");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_id, event.event_id);
    }

    #[tokio::test]
    async fn contributor_sees_own_delayed_credit_events_in_summary() {
        let temp = tempfile::tempdir().expect("temp dir");
        let state = test_state(temp.path().to_path_buf());
        let envelope = sample_envelope().await;
        let submission_id = envelope.submission_id;

        let _ = submit_trace_handler(
            State(state.clone()),
            auth_headers("token-a"),
            Json(envelope),
        )
        .await
        .expect("submission succeeds");

        let Json(event) = append_credit_event_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            AxumPath(submission_id),
            Json(TraceCreditLedgerAppendRequest {
                event_type: TraceCreditLedgerEventType::TrainingUtility,
                credit_points_delta: 1.75,
                reason: None,
                external_ref: None,
            }),
        )
        .await
        .expect("reviewer can append delayed credit");

        let Json(events) = credit_events_handler(State(state.clone()), auth_headers("token-a"))
            .await
            .expect("contributor can list own credit events");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_id, event.event_id);

        let Json(credit) = credit_handler(State(state.clone()), auth_headers("token-a"))
            .await
            .expect("credit summary succeeds");
        assert_eq!(credit.credit_points_ledger, 1.75);
        assert_eq!(
            credit.credit_points_total,
            credit.credit_points_final + 1.75
        );

        let Json(statuses) = submission_status_handler(
            State(state),
            auth_headers("token-a"),
            Json(TraceSubmissionStatusRequest {
                submission_ids: vec![submission_id],
            }),
        )
        .await
        .expect("contributor can sync delayed credit status");
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].credit_points_ledger, 1.75);
        assert_eq!(
            statuses[0].credit_points_total,
            Some(statuses[0].credit_points_pending + 1.75)
        );
        assert!(
            statuses[0]
                .delayed_credit_explanations
                .iter()
                .any(|explanation| explanation.contains("TrainingUtility"))
        );
    }

    #[tokio::test]
    async fn other_same_tenant_contributor_cannot_see_delayed_credit_events() {
        let temp = tempfile::tempdir().expect("temp dir");
        let state = test_state(temp.path().to_path_buf());
        let envelope = sample_envelope().await;
        let submission_id = envelope.submission_id;

        let _ = submit_trace_handler(
            State(state.clone()),
            auth_headers("token-a"),
            Json(envelope),
        )
        .await
        .expect("submission succeeds");

        let _ = append_credit_event_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            AxumPath(submission_id),
            Json(TraceCreditLedgerAppendRequest {
                event_type: TraceCreditLedgerEventType::RegressionCatch,
                credit_points_delta: 3.0,
                reason: None,
                external_ref: None,
            }),
        )
        .await
        .expect("reviewer can append delayed credit");

        let Json(events) = credit_events_handler(State(state.clone()), auth_headers("token-a-2"))
            .await
            .expect("other contributor can list visible credit events");
        assert!(events.is_empty());

        let Json(credit) = credit_handler(State(state), auth_headers("token-a-2"))
            .await
            .expect("other contributor credit summary succeeds");
        assert_eq!(credit.credit_points_ledger, 0.0);
        assert_eq!(credit.credit_points_total, 0.0);
    }

    #[tokio::test]
    async fn contributor_cannot_append_delayed_credit_event() {
        let temp = tempfile::tempdir().expect("temp dir");
        let state = test_state(temp.path().to_path_buf());
        let envelope = sample_envelope().await;
        let submission_id = envelope.submission_id;

        let _ = submit_trace_handler(
            State(state.clone()),
            auth_headers("token-a"),
            Json(envelope),
        )
        .await
        .expect("submission succeeds");

        let error = append_credit_event_handler(
            State(state),
            auth_headers("token-a"),
            AxumPath(submission_id),
            Json(TraceCreditLedgerAppendRequest {
                event_type: TraceCreditLedgerEventType::ReviewerBonus,
                credit_points_delta: 1.0,
                reason: None,
                external_ref: None,
            }),
        )
        .await
        .expect_err("contributor cannot append delayed credit");
        assert_eq!(error.0, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn reviewer_can_append_negative_abuse_penalty() {
        let temp = tempfile::tempdir().expect("temp dir");
        let state = test_state(temp.path().to_path_buf());
        let envelope = sample_envelope().await;
        let submission_id = envelope.submission_id;

        let _ = submit_trace_handler(
            State(state.clone()),
            auth_headers("token-a"),
            Json(envelope),
        )
        .await
        .expect("submission succeeds");

        let Json(event) = append_credit_event_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            AxumPath(submission_id),
            Json(TraceCreditLedgerAppendRequest {
                event_type: TraceCreditLedgerEventType::AbusePenalty,
                credit_points_delta: -4.0,
                reason: Some("abuse pattern confirmed".to_string()),
                external_ref: None,
            }),
        )
        .await
        .expect("reviewer can append abuse penalty");
        assert_eq!(event.event_type, TraceCreditLedgerEventType::AbusePenalty);
        assert_eq!(event.credit_points_delta, -4.0);

        let Json(credit) = credit_handler(State(state), auth_headers("token-a"))
            .await
            .expect("credit summary succeeds");
        assert_eq!(credit.credit_points_ledger, -4.0);
        assert_eq!(credit.credit_points_total, credit.credit_points_final - 4.0);
    }

    #[tokio::test]
    async fn rejects_unknown_tenant_token() {
        let temp = tempfile::tempdir().expect("temp dir");
        let state = test_state(temp.path().to_path_buf());
        let envelope = sample_envelope().await;

        let error = submit_trace_handler(State(state), auth_headers("bad-token"), Json(envelope))
            .await
            .expect_err("unknown token is rejected");

        assert_eq!(error.0, StatusCode::FORBIDDEN);
    }
}
