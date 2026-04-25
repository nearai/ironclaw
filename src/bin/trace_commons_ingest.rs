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
use chrono::{DateTime, Duration, Utc};
use ironclaw::config::{DatabaseBackend, DatabaseConfig};
use ironclaw::db::Database;
use ironclaw::secrets::SecretsCrypto;
use ironclaw::trace_artifact_store::{
    EncryptedTraceArtifactReceipt, LocalEncryptedTraceArtifactStore, TraceArtifactKind,
};
use ironclaw::trace_contribution::{
    ConsentScope, EmbeddingAnalysisMetadata, ResidualPiiRisk, TRACE_CONTRIBUTION_SCHEMA_VERSION,
    TraceContributionEnvelope, TraceSubmissionReceipt, TraceSubmissionStatusRequest,
    TraceSubmissionStatusUpdate, apply_credit_estimate_to_envelope,
    canonical_summary_for_embedding, rescrub_trace_envelope, retention_policy_for_trace,
};
use ironclaw::trace_corpus_storage::{
    TraceAuditAction as StorageTraceAuditAction,
    TraceAuditEventRecord as StorageTraceAuditEventRecord,
    TraceAuditEventWrite as StorageTraceAuditEventWrite,
    TraceAuditSafeMetadata as StorageTraceAuditSafeMetadata,
    TraceCorpusStatus as StorageTraceCorpusStatus,
    TraceCreditEventRecord as StorageTraceCreditEventRecord,
    TraceCreditEventType as StorageTraceCreditEventType,
    TraceCreditEventWrite as StorageTraceCreditEventWrite,
    TraceCreditSettlementState as StorageTraceCreditSettlementState,
    TraceDerivedRecord as StorageTraceDerivedRecord,
    TraceDerivedRecordWrite as StorageTraceDerivedRecordWrite,
    TraceDerivedStatus as StorageTraceDerivedStatus,
    TraceExportManifestItemInvalidationReason as StorageTraceExportManifestItemInvalidationReason,
    TraceExportManifestItemWrite as StorageTraceExportManifestItemWrite,
    TraceExportManifestRecord as StorageTraceExportManifestRecord,
    TraceExportManifestWrite as StorageTraceExportManifestWrite,
    TraceObjectArtifactKind as StorageTraceObjectArtifactKind,
    TraceObjectRefRecord as StorageTraceObjectRefRecord,
    TraceObjectRefWrite as StorageTraceObjectRefWrite,
    TraceSubmissionRecord as StorageTraceSubmissionRecord,
    TraceSubmissionWrite as StorageTraceSubmissionWrite,
    TraceTombstoneWrite as StorageTraceTombstoneWrite,
    TraceVectorEntrySourceProjection as StorageTraceVectorEntrySourceProjection,
    TraceVectorEntryStatus as StorageTraceVectorEntryStatus,
    TraceVectorEntryWrite as StorageTraceVectorEntryWrite,
    TraceWorkerKind as StorageTraceWorkerKind,
};
use secrecy::SecretString;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::net::TcpListener;
use uuid::Uuid;

const DEFAULT_BIND: &str = "127.0.0.1:3907";
const MAX_INGEST_BODY_BYTES: usize = 2 * 1024 * 1024;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = tracing_subscriber::fmt::try_init();
    let state = Arc::new(AppState::from_env().await?);
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
    db_mirror: Option<Arc<dyn Database>>,
    db_contributor_reads: bool,
    db_reviewer_reads: bool,
    db_replay_export_reads: bool,
    db_audit_reads: bool,
    artifact_store: Option<Arc<LocalEncryptedTraceArtifactStore>>,
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

    fn storage_name(self) -> &'static str {
        match self {
            Self::Contributor => "contributor",
            Self::Reviewer => "reviewer",
            Self::Admin => "admin",
        }
    }
}

impl AppState {
    async fn from_env() -> anyhow::Result<Self> {
        let root = std::env::var("TRACE_COMMONS_DATA_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| default_data_dir());
        let tokens = parse_tenant_tokens_from_env()?;
        if tokens.is_empty() {
            anyhow::bail!(
                "TRACE_COMMONS_TENANT_TOKENS or TRACE_COMMONS_INGEST_TOKEN must be configured"
            );
        }
        let db_mirror = trace_corpus_db_mirror_from_env().await?;
        let db_contributor_reads = env_truthy("TRACE_COMMONS_DB_CONTRIBUTOR_READS");
        if db_contributor_reads && db_mirror.is_none() {
            anyhow::bail!(
                "TRACE_COMMONS_DB_CONTRIBUTOR_READS requires TRACE_COMMONS_DB_DUAL_WRITE"
            );
        }
        let db_reviewer_reads = env_truthy("TRACE_COMMONS_DB_REVIEWER_READS");
        if db_reviewer_reads && db_mirror.is_none() {
            anyhow::bail!("TRACE_COMMONS_DB_REVIEWER_READS requires TRACE_COMMONS_DB_DUAL_WRITE");
        }
        let db_replay_export_reads = env_truthy("TRACE_COMMONS_DB_REPLAY_EXPORT_READS");
        if db_replay_export_reads && db_mirror.is_none() {
            anyhow::bail!(
                "TRACE_COMMONS_DB_REPLAY_EXPORT_READS requires TRACE_COMMONS_DB_DUAL_WRITE"
            );
        }
        let db_audit_reads = env_truthy("TRACE_COMMONS_DB_AUDIT_READS");
        if db_audit_reads && db_mirror.is_none() {
            anyhow::bail!("TRACE_COMMONS_DB_AUDIT_READS requires TRACE_COMMONS_DB_DUAL_WRITE");
        }
        let artifact_store = trace_artifact_store_from_env(&root)?;
        Ok(Self {
            root,
            tokens: Arc::new(tokens),
            db_mirror,
            db_contributor_reads,
            db_reviewer_reads,
            db_replay_export_reads,
            db_audit_reads,
            artifact_store,
        })
    }
}

fn trace_artifact_store_from_env(
    default_root: &Path,
) -> anyhow::Result<Option<Arc<LocalEncryptedTraceArtifactStore>>> {
    let key = std::env::var("TRACE_COMMONS_ARTIFACT_KEY_HEX").ok();
    if key.is_none() && !env_truthy("TRACE_COMMONS_ENCRYPTED_ARTIFACTS") {
        return Ok(None);
    }
    let key =
        key.context("TRACE_COMMONS_ENCRYPTED_ARTIFACTS requires TRACE_COMMONS_ARTIFACT_KEY_HEX")?;
    let root = std::env::var("TRACE_COMMONS_ARTIFACT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| default_root.join("encrypted_artifacts"));
    let crypto = SecretsCrypto::new(SecretString::from(key))
        .context("failed to initialize Trace Commons artifact encryption")?;
    Ok(Some(Arc::new(LocalEncryptedTraceArtifactStore::new(
        root, crypto,
    ))))
}

async fn trace_corpus_db_mirror_from_env() -> anyhow::Result<Option<Arc<dyn Database>>> {
    if !env_truthy("TRACE_COMMONS_DB_DUAL_WRITE") {
        return Ok(None);
    }

    let backend = std::env::var("DATABASE_BACKEND")
        .unwrap_or_else(|_| DatabaseBackend::default().to_string())
        .parse::<DatabaseBackend>()
        .map_err(|message| {
            anyhow::anyhow!("invalid DATABASE_BACKEND for trace mirror: {message}")
        })?;
    let config = match backend {
        DatabaseBackend::LibSql => {
            let path = std::env::var("LIBSQL_PATH")
                .map(PathBuf::from)
                .unwrap_or_else(|_| ironclaw::config::default_libsql_path());
            let path = path.to_string_lossy().into_owned();
            let turso_url = std::env::var("LIBSQL_URL").ok();
            let turso_token = std::env::var("LIBSQL_AUTH_TOKEN").ok();
            DatabaseConfig::from_libsql_path(&path, turso_url.as_deref(), turso_token.as_deref())
        }
        DatabaseBackend::Postgres => {
            let url = std::env::var("DATABASE_URL").context(
                "TRACE_COMMONS_DB_DUAL_WRITE requires DATABASE_URL when DATABASE_BACKEND=postgres",
            )?;
            let pool_size = std::env::var("DATABASE_POOL_SIZE")
                .ok()
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or(5);
            DatabaseConfig::from_postgres_url(&url, pool_size)
        }
    };
    let db = ironclaw::db::connect_from_config(&config)
        .await
        .context("failed to connect Trace Commons DB dual-write mirror")?;
    tracing::info!(backend = %backend, "Trace Commons DB dual-write mirror enabled");
    Ok(Some(db))
}

fn env_truthy(key: &str) -> bool {
    std::env::var(key).ok().is_some_and(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
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
        .route(
            "/v1/datasets/replay/manifests",
            get(replay_export_manifests_handler),
        )
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

    let stored_envelope = store_envelope(&state, &tenant.tenant_id, corpus_status, &envelope)
        .map_err(internal_error)?;
    let derived_record = build_derived_record(
        &tenant.tenant_id,
        corpus_status,
        &envelope,
        derived_precheck,
    );
    let retention_policy = retention_policy_for_trace(&envelope);
    let received_at = Utc::now();
    let expires_at = retention_policy
        .max_age_days
        .map(|days| received_at + Duration::days(i64::from(days)));
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
        received_at,
        retention_policy_id: retention_policy.name,
        expires_at,
        purged_at: None,
        object_key: stored_envelope.object_key,
        artifact_receipt: stored_envelope.artifact_receipt,
    };
    write_submission_record(&state.root, &record).map_err(internal_error)?;
    write_derived_record(&state.root, &derived_record).map_err(internal_error)?;
    append_audit_event(
        &state.root,
        &tenant.tenant_id,
        TraceCommonsAuditEvent::submitted(&record),
    )
    .map_err(internal_error)?;
    if let Err(error) =
        mirror_submission_to_db(&state, &tenant, &record, &derived_record, &envelope).await
    {
        tracing::warn!(%error, submission_id = %record.submission_id, "Trace Commons DB dual-write mirror failed");
    }

    Ok(Json(receipt_from_record(&record)))
}

async fn revoke_trace_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(submission_id): AxumPath<Uuid>,
) -> ApiResult<StatusCode> {
    revoke_submission(&state, &headers, submission_id).await
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
    revoke_submission(&state, &headers, body.submission_id).await
}

async fn revoke_submission(
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

    let mut mirrored_record = None;
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
        mirrored_record = Some(record);
    }
    if let Some(mut derived) = read_derived_record(&state.root, &tenant.tenant_id, submission_id)
        .map_err(internal_error)?
    {
        derived.status = TraceCorpusStatus::Revoked;
        write_derived_record(&state.root, &derived).map_err(internal_error)?;
    }
    invalidate_export_provenance_for_source(
        &state.root,
        &tenant.tenant_id,
        submission_id,
        "contributor_revocation",
    )
    .map_err(internal_error)?;

    append_audit_event(
        &state.root,
        &tenant.tenant_id,
        TraceCommonsAuditEvent::revoked(&tenant, submission_id),
    )
    .map_err(internal_error)?;
    if let Err(error) =
        mirror_revocation_to_db(state, &tenant, submission_id, mirrored_record.as_ref()).await
    {
        tracing::warn!(%error, %submission_id, "Trace Commons DB dual-write revocation mirror failed");
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn credit_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> ApiResult<Json<TraceCommonsTenantCreditResponse>> {
    let tenant = authenticate(state.as_ref(), &headers)?;
    let credit_view = read_contributor_credit_view(state.as_ref(), &tenant)
        .await
        .map_err(internal_error)?;
    Ok(Json(
        TraceCommonsTenantCreditResponse::from_records_and_events(
            tenant.tenant_id,
            credit_view.records,
            &credit_view.credit_events,
        ),
    ))
}

async fn credit_events_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> ApiResult<Json<Vec<TraceCommonsCreditLedgerRecord>>> {
    let tenant = authenticate(state.as_ref(), &headers)?;
    let credit_view = read_contributor_credit_view(state.as_ref(), &tenant)
        .await
        .map_err(internal_error)?;
    Ok(Json(credit_view.credit_events))
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

    let credit_view = read_contributor_credit_view(state.as_ref(), &tenant)
        .await
        .map_err(internal_error)?;
    let visible_by_submission = credit_view
        .records
        .iter()
        .map(|record| (record.submission_id, record))
        .collect::<BTreeMap<_, _>>();
    let mut statuses = Vec::new();
    for submission_id in body.submission_ids {
        if let Some(record) = visible_by_submission.get(&submission_id) {
            statuses.push(submission_status_from_record(
                record,
                &credit_view.credit_events,
            ));
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
    let TraceCommonsMetadataView { records, derived } =
        read_reviewer_metadata_view(state.as_ref(), &tenant)
            .await
            .map_err(internal_error)?;
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
    let TraceCommonsMetadataView { records, derived } =
        read_reviewer_metadata_view(state.as_ref(), &tenant)
            .await
            .map_err(internal_error)?;
    let derived_by_submission = derived
        .into_iter()
        .map(|record| (record.submission_id, record))
        .collect::<BTreeMap<_, _>>();
    let limit = query.limit.unwrap_or(100).clamp(1, 500);
    let consent_scope = parse_consent_scope_filter(query.consent_scope.as_deref())?;

    let items: Vec<_> = records
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
    append_audit_event_with_db_mirror(
        state.as_ref(),
        &tenant,
        TraceCommonsAuditEvent::read(&tenant, "trace_list", items.len()),
        StorageTraceAuditAction::Read,
        StorageTraceAuditSafeMetadata::Empty,
    )
    .await
    .map_err(internal_error)?;
    Ok(Json(items))
}

async fn review_quarantine_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> ApiResult<Json<Vec<TraceReviewQueueItem>>> {
    let tenant = authenticate(state.as_ref(), &headers)?;
    require_reviewer(&tenant)?;
    let TraceCommonsMetadataView { records, derived } =
        read_reviewer_metadata_view(state.as_ref(), &tenant)
            .await
            .map_err(internal_error)?;
    let derived_by_submission = derived
        .into_iter()
        .map(|record| (record.submission_id, record))
        .collect::<BTreeMap<_, _>>();
    let queue = records
        .into_iter()
        .filter(|record| record.status == TraceCorpusStatus::Quarantined)
        .map(|record| TraceReviewQueueItem::from_record(record, &derived_by_submission))
        .collect::<Vec<_>>();
    append_audit_event_with_db_mirror(
        state.as_ref(),
        &tenant,
        TraceCommonsAuditEvent::read(&tenant, "review_quarantine", queue.len()),
        StorageTraceAuditAction::Read,
        StorageTraceAuditSafeMetadata::Empty,
    )
    .await
    .map_err(internal_error)?;
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

const MAX_DELAYED_CREDIT_POINTS_DELTA: f32 = 100.0;

impl TraceCreditLedgerEventType {
    fn requires_external_ref(self) -> bool {
        matches!(
            self,
            Self::BenchmarkConversion
                | Self::RegressionCatch
                | Self::TrainingUtility
                | Self::RankingUtility
        )
    }
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
    if submission.is_terminal() {
        return Err(api_error(
            StatusCode::CONFLICT,
            "terminal trace submissions are not eligible for delayed credit",
        ));
    }
    if body.credit_points_delta.abs() > MAX_DELAYED_CREDIT_POINTS_DELTA {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "credit_points_delta exceeds the delayed credit policy limit",
        ));
    }
    let reason = body
        .reason
        .as_deref()
        .map(str::trim)
        .filter(|reason| !reason.is_empty())
        .ok_or_else(|| {
            api_error(
                StatusCode::BAD_REQUEST,
                "delayed credit events require a non-empty reason",
            )
        })?
        .to_string();
    let external_ref = body
        .external_ref
        .as_deref()
        .map(str::trim)
        .filter(|external_ref| !external_ref.is_empty())
        .map(ToOwned::to_owned);
    if body.event_type.requires_external_ref() && external_ref.is_none() {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "benchmark, regression, training, and ranking utility credit require external_ref",
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
        reason: Some(reason),
        external_ref,
        actor_role: tenant.role,
        actor_principal_ref: tenant.principal_ref.clone(),
        created_at: Utc::now(),
    };
    append_credit_event(&state.root, &tenant.tenant_id, &event).map_err(internal_error)?;
    if let Err(error) = mirror_credit_event_to_db(&state, &event).await {
        tracing::warn!(%error, submission_id = %event.submission_id, "Trace Commons DB dual-write credit mirror failed");
    }
    append_audit_event_with_db_mirror(
        state.as_ref(),
        &tenant,
        TraceCommonsAuditEvent::credit_mutation(
            &tenant,
            submission_id,
            body.credit_points_delta,
            event.reason.as_deref(),
        ),
        StorageTraceAuditAction::CreditMutate,
        StorageTraceAuditSafeMetadata::Empty,
    )
    .await
    .map_err(internal_error)?;
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
    if record.is_terminal() {
        return Err(api_error(
            StatusCode::CONFLICT,
            "terminal trace submissions are not eligible for review approval",
        ));
    }
    let mut envelope = read_envelope_by_record(state.as_ref(), &record).map_err(internal_error)?;

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
            let stored = store_envelope(
                &state,
                &tenant.tenant_id,
                TraceCorpusStatus::Accepted,
                &envelope,
            )
            .map_err(internal_error)?;
            record.object_key = stored.object_key;
            record.artifact_receipt = stored.artifact_receipt;
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
            let stored = store_envelope(
                &state,
                &tenant.tenant_id,
                TraceCorpusStatus::Rejected,
                &envelope,
            )
            .map_err(internal_error)?;
            record.object_key = stored.object_key;
            record.artifact_receipt = stored.artifact_receipt;
        }
    }

    write_submission_record(&state.root, &record).map_err(internal_error)?;
    let mut mirrored_derived = None;
    if let Some(mut derived) = read_derived_record(&state.root, &tenant.tenant_id, submission_id)
        .map_err(internal_error)?
    {
        derived.status = record.status;
        write_derived_record(&state.root, &derived).map_err(internal_error)?;
        mirrored_derived = Some(derived);
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
    if let Err(error) = mirror_review_decision_to_db(
        &state,
        &tenant,
        &record,
        &envelope,
        mirrored_derived.as_ref(),
    )
    .await
    {
        tracing::warn!(%error, %submission_id, "Trace Commons DB dual-write review mirror failed");
    }

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
    let TraceCommonsMetadataView { records, derived } =
        read_replay_export_metadata_view(state.as_ref(), &tenant)
            .await
            .map_err(internal_error)?;
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
        let body_read = read_envelope_for_replay_export(
            state.as_ref(),
            &tenant,
            &record,
            "replay_dataset_export",
            Some(&purpose),
        )
        .await
        .map_err(internal_error)?;
        items.push(TraceReplayDatasetItem::from_record(
            &record,
            derived_by_submission.get(&record.submission_id),
            &body_read.envelope,
            body_read.object_ref_id,
        ));
    }
    let source_submission_ids = items
        .iter()
        .map(|item| item.submission_id)
        .collect::<Vec<_>>();
    let source_submission_ids_hash =
        source_submission_ids_hash("replay_dataset", &source_submission_ids);

    let export_id = Uuid::new_v4();
    let audit_event = TraceCommonsAuditEvent::dataset_export(
        &tenant,
        export_id,
        items.len(),
        source_submission_ids_hash.clone(),
    );
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
        source_submission_ids_hash,
    );
    write_export_manifest(&state.root, &tenant.tenant_id, &manifest).map_err(internal_error)?;
    if let Err(error) = mirror_export_manifest_to_db(
        state.as_ref(),
        StorageTraceObjectArtifactKind::ExportArtifact,
        &manifest,
        &items,
    )
    .await
    {
        tracing::warn!(
            %error,
            export_id = %manifest.export_id,
            "Trace Commons DB dual-write export manifest mirror failed"
        );
    }
    append_audit_event_with_db_mirror(
        state.as_ref(),
        &tenant,
        audit_event,
        StorageTraceAuditAction::Export,
        StorageTraceAuditSafeMetadata::Export {
            artifact_kind: StorageTraceObjectArtifactKind::ExportArtifact,
            purpose_code: Some(manifest.purpose.clone()),
            item_count: items.len().min(u32::MAX as usize) as u32,
        },
    )
    .await
    .map_err(internal_error)?;
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

async fn replay_export_manifests_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> ApiResult<Json<Vec<TraceExportManifestSummary>>> {
    let tenant = authenticate(state.as_ref(), &headers)?;
    require_reviewer(&tenant)?;
    let manifests = read_replay_export_manifest_summaries(state.as_ref(), &tenant)
        .await
        .map_err(internal_error)?;
    append_audit_event_with_db_mirror(
        state.as_ref(),
        &tenant,
        TraceCommonsAuditEvent::read(&tenant, "replay_export_manifests", manifests.len()),
        StorageTraceAuditAction::Read,
        StorageTraceAuditSafeMetadata::Empty,
    )
    .await
    .map_err(internal_error)?;
    Ok(Json(manifests))
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
    let TraceCommonsMetadataView { records, derived } =
        read_reviewer_metadata_view(state.as_ref(), &tenant)
            .await
            .map_err(internal_error)?;
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
    let source_submission_ids = candidates
        .iter()
        .map(|candidate| candidate.submission_id)
        .collect::<Vec<_>>();
    let source_submission_ids_hash =
        source_submission_ids_hash("benchmark_conversion", &source_submission_ids);
    let audit_event = TraceCommonsAuditEvent::benchmark_conversion(
        &tenant,
        conversion_id,
        candidates.len(),
        source_submission_ids_hash.clone(),
    );
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
        source_submission_ids,
        source_submission_ids_hash,
        generated_at: Utc::now(),
        item_count: candidates.len(),
        candidates,
    };
    write_benchmark_artifact(&state.root, &tenant.tenant_id, &artifact).map_err(internal_error)?;
    let provenance = TraceExportProvenanceManifest::new(
        &tenant.tenant_id,
        conversion_id,
        audit_event_id,
        TraceExportProvenanceKind::BenchmarkConversion,
        artifact.purpose.clone(),
        artifact.source_submission_ids.clone(),
        artifact.source_submission_ids_hash.clone(),
    );
    write_export_provenance(
        &benchmark_provenance_path(&state.root, &tenant.tenant_id, conversion_id),
        &provenance,
    )
    .map_err(internal_error)?;
    if let Err(error) = mirror_benchmark_export_provenance_to_db(state.as_ref(), &artifact).await {
        tracing::warn!(
            %error,
            export_id = %conversion_id,
            "Trace Commons DB dual-write benchmark provenance mirror failed"
        );
    }
    append_audit_event_with_db_mirror(
        state.as_ref(),
        &tenant,
        audit_event,
        StorageTraceAuditAction::BenchmarkConvert,
        StorageTraceAuditSafeMetadata::Export {
            artifact_kind: StorageTraceObjectArtifactKind::BenchmarkArtifact,
            purpose_code: Some(artifact.purpose.clone()),
            item_count: artifact.item_count.min(u32::MAX as usize) as u32,
        },
    )
    .await
    .map_err(internal_error)?;
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
    .await
    .map_err(internal_error)?;
    let export_id = Uuid::new_v4();
    let source_submission_ids = candidates
        .iter()
        .map(|candidate| candidate.submission_id)
        .collect::<Vec<_>>();
    let source_item_list_hash =
        source_submission_ids_hash("ranker_training_candidates_export", &source_submission_ids);
    let audit_event = TraceCommonsAuditEvent::ranker_training_export(
        &tenant,
        export_id,
        "ranker_training_candidates_export",
        candidates.len(),
        source_item_list_hash.clone(),
    );
    let audit_event_id = audit_event.event_id;
    let provenance = TraceExportProvenanceManifest::new(
        &tenant.tenant_id,
        export_id,
        audit_event_id,
        TraceExportProvenanceKind::RankerTrainingCandidates,
        "ranker_training_candidates_export".to_string(),
        source_submission_ids,
        source_item_list_hash.clone(),
    );
    write_export_provenance(
        &ranker_provenance_path(&state.root, &tenant.tenant_id, export_id),
        &provenance,
    )
    .map_err(internal_error)?;
    if let Err(error) =
        mirror_ranker_candidate_export_provenance_to_db(state.as_ref(), &provenance, &candidates)
            .await
    {
        tracing::warn!(
            %error,
            export_id = %export_id,
            "Trace Commons DB dual-write ranker candidate provenance mirror failed"
        );
    }
    append_audit_event_with_db_mirror(
        state.as_ref(),
        &tenant,
        audit_event,
        StorageTraceAuditAction::Export,
        StorageTraceAuditSafeMetadata::Export {
            artifact_kind: StorageTraceObjectArtifactKind::ExportArtifact,
            purpose_code: Some("ranker_training_candidates_export".to_string()),
            item_count: candidates.len().min(u32::MAX as usize) as u32,
        },
    )
    .await
    .map_err(internal_error)?;
    Ok(Json(TraceRankerTrainingCandidateExport {
        tenant_id: tenant.tenant_id.clone(),
        tenant_storage_ref: tenant_storage_ref(&tenant.tenant_id),
        export_id,
        audit_event_id,
        generated_at: Utc::now(),
        item_count: candidates.len(),
        source_item_list_hash,
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
            .await
            .map_err(internal_error)?;
    let pairs = build_ranker_training_pairs(&candidates, pair_limit);
    let export_id = Uuid::new_v4();
    let source_item_list_hash = ranker_pair_list_hash(&pairs);
    let audit_event = TraceCommonsAuditEvent::ranker_training_export(
        &tenant,
        export_id,
        "ranker_training_pairs_export",
        pairs.len(),
        source_item_list_hash.clone(),
    );
    let audit_event_id = audit_event.event_id;
    let source_submission_ids = ranker_pair_source_submission_ids(&pairs);
    let provenance = TraceExportProvenanceManifest::new(
        &tenant.tenant_id,
        export_id,
        audit_event_id,
        TraceExportProvenanceKind::RankerTrainingPairs,
        "ranker_training_pairs_export".to_string(),
        source_submission_ids,
        source_item_list_hash.clone(),
    );
    write_export_provenance(
        &ranker_provenance_path(&state.root, &tenant.tenant_id, export_id),
        &provenance,
    )
    .map_err(internal_error)?;
    if let Err(error) =
        mirror_ranker_pair_export_provenance_to_db(state.as_ref(), &provenance, &pairs).await
    {
        tracing::warn!(
            %error,
            export_id = %export_id,
            "Trace Commons DB dual-write ranker pair provenance mirror failed"
        );
    }
    append_audit_event_with_db_mirror(
        state.as_ref(),
        &tenant,
        audit_event,
        StorageTraceAuditAction::Export,
        StorageTraceAuditSafeMetadata::Export {
            artifact_kind: StorageTraceObjectArtifactKind::ExportArtifact,
            purpose_code: Some("ranker_training_pairs_export".to_string()),
            item_count: pairs.len().min(u32::MAX as usize) as u32,
        },
    )
    .await
    .map_err(internal_error)?;
    Ok(Json(TraceRankerTrainingPairExport {
        tenant_id: tenant.tenant_id.clone(),
        tenant_storage_ref: tenant_storage_ref(&tenant.tenant_id),
        export_id,
        audit_event_id,
        generated_at: Utc::now(),
        item_count: pairs.len(),
        source_item_list_hash,
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
    let TraceCommonsMetadataView { records, derived } =
        read_reviewer_metadata_view(state.as_ref(), &tenant)
            .await
            .map_err(internal_error)?;
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
    append_audit_event_with_db_mirror(
        state.as_ref(),
        &tenant,
        TraceCommonsAuditEvent::read(&tenant, "active_learning_review_queue", items.len()),
        StorageTraceAuditAction::Read,
        StorageTraceAuditSafeMetadata::Empty,
    )
    .await
    .map_err(internal_error)?;
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
    #[serde(default)]
    backfill_db_mirror: bool,
    #[serde(default)]
    index_vectors: bool,
    #[serde(default)]
    reconcile_db_mirror: bool,
    #[serde(default = "default_true")]
    prune_export_cache: bool,
    #[serde(default)]
    max_export_age_hours: Option<i64>,
    #[serde(default)]
    purge_expired_before: Option<DateTime<Utc>>,
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
    let response = run_maintenance(state.as_ref(), &tenant, body)
        .await
        .map_err(internal_error)?;
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
    let events = read_audit_events(state.as_ref(), &tenant)
        .await
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

async fn collect_ranker_training_candidates(
    state: &AppState,
    tenant: &TenantAuth,
    query: &RankerTrainingExportQuery,
    consent_scope: Option<ConsentScope>,
) -> anyhow::Result<Vec<TraceRankerTrainingCandidate>> {
    let TraceCommonsMetadataView { records, derived } =
        read_reviewer_metadata_view(state, tenant).await?;
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

fn source_submission_ids_hash(kind: &str, source_submission_ids: &[Uuid]) -> String {
    let mut payload = String::from(kind);
    for submission_id in source_submission_ids {
        payload.push('\n');
        payload.push_str(&submission_id.to_string());
    }
    sha256_prefixed(&payload)
}

fn ranker_pair_list_hash(pairs: &[TraceRankerTrainingPair]) -> String {
    let mut payload = String::from("ranker_training_pairs_export");
    for pair in pairs {
        payload.push('\n');
        payload.push_str(&pair.preferred_submission_id.to_string());
        payload.push('>');
        payload.push_str(&pair.rejected_submission_id.to_string());
    }
    sha256_prefixed(&payload)
}

fn ranker_pair_source_submission_ids(pairs: &[TraceRankerTrainingPair]) -> Vec<Uuid> {
    pairs
        .iter()
        .flat_map(|pair| [pair.preferred_submission_id, pair.rejected_submission_id])
        .collect::<BTreeSet<_>>()
        .into_iter()
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

#[derive(Debug)]
struct TraceContributorCreditView {
    records: Vec<TraceCommonsSubmissionRecord>,
    credit_events: Vec<TraceCommonsCreditLedgerRecord>,
}

#[derive(Debug)]
struct TraceCommonsMetadataView {
    records: Vec<TraceCommonsSubmissionRecord>,
    derived: Vec<TraceCommonsDerivedRecord>,
}

async fn read_reviewer_metadata_view(
    state: &AppState,
    tenant: &TenantAuth,
) -> anyhow::Result<TraceCommonsMetadataView> {
    if state.db_reviewer_reads {
        return read_reviewer_metadata_view_from_db(state, tenant).await;
    }

    Ok(TraceCommonsMetadataView {
        records: read_all_submission_records(&state.root, &tenant.tenant_id)?,
        derived: read_all_derived_records(&state.root, &tenant.tenant_id)?,
    })
}

async fn read_replay_export_metadata_view(
    state: &AppState,
    tenant: &TenantAuth,
) -> anyhow::Result<TraceCommonsMetadataView> {
    if state.db_replay_export_reads {
        return read_reviewer_metadata_view_from_db(state, tenant).await;
    }

    Ok(TraceCommonsMetadataView {
        records: read_all_submission_records(&state.root, &tenant.tenant_id)?,
        derived: read_all_derived_records(&state.root, &tenant.tenant_id)?,
    })
}

async fn read_replay_export_manifest_summaries(
    state: &AppState,
    tenant: &TenantAuth,
) -> anyhow::Result<Vec<TraceExportManifestSummary>> {
    if let Some(db) = state.db_mirror.as_ref() {
        return Ok(db
            .list_trace_export_manifests(&tenant.tenant_id)
            .await
            .context("failed to read Trace Commons export manifests from DB mirror")?
            .into_iter()
            .map(TraceExportManifestSummary::from_storage_record)
            .filter(TraceExportManifestSummary::is_replay_dataset_manifest)
            .collect());
    }

    Ok(read_all_export_manifests(&state.root, &tenant.tenant_id)?
        .into_iter()
        .map(TraceExportManifestSummary::from_replay_manifest)
        .collect())
}

async fn read_reviewer_metadata_view_from_db(
    state: &AppState,
    tenant: &TenantAuth,
) -> anyhow::Result<TraceCommonsMetadataView> {
    let db = state
        .db_mirror
        .as_ref()
        .context("TRACE_COMMONS_DB_REVIEWER_READS is enabled without a DB mirror")?;
    let records = db
        .list_trace_submissions(&tenant.tenant_id)
        .await
        .context("failed to read Trace Commons submissions from DB mirror")?
        .into_iter()
        .filter_map(trace_commons_record_from_storage_submission)
        .collect::<anyhow::Result<Vec<_>>>()?;
    let submission_metadata = records
        .iter()
        .map(|record| {
            (
                record.submission_id,
                (record.status, record.privacy_risk, record.tenant_id.clone()),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let derived = db
        .list_trace_derived_records(&tenant.tenant_id)
        .await
        .context("failed to read Trace Commons derived records from DB mirror")?
        .into_iter()
        .filter_map(|record| {
            trace_commons_derived_record_from_storage(record, &submission_metadata)
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    Ok(TraceCommonsMetadataView { records, derived })
}

async fn read_contributor_credit_view(
    state: &AppState,
    tenant: &TenantAuth,
) -> anyhow::Result<TraceContributorCreditView> {
    if state.db_contributor_reads {
        return read_contributor_credit_view_from_db(state, tenant).await;
    }

    let records = visible_submission_records(
        tenant,
        read_all_submission_records(&state.root, &tenant.tenant_id)?,
    );
    let credit_events = eligible_credit_events_for_records(
        &records,
        visible_credit_events(
            tenant,
            read_all_credit_events(&state.root, &tenant.tenant_id)?,
        ),
    );
    Ok(TraceContributorCreditView {
        records,
        credit_events,
    })
}

async fn read_contributor_credit_view_from_db(
    state: &AppState,
    tenant: &TenantAuth,
) -> anyhow::Result<TraceContributorCreditView> {
    let db = state
        .db_mirror
        .as_ref()
        .context("TRACE_COMMONS_DB_CONTRIBUTOR_READS is enabled without a DB mirror")?;
    let records = db
        .list_trace_submissions(&tenant.tenant_id)
        .await
        .context("failed to read Trace Commons submissions from DB mirror")?
        .into_iter()
        .filter_map(trace_commons_record_from_storage_submission)
        .collect::<anyhow::Result<Vec<_>>>()?;
    let records = visible_submission_records(tenant, records);
    let owner_by_submission = records
        .iter()
        .map(|record| (record.submission_id, record.auth_principal_ref.clone()))
        .collect::<BTreeMap<_, _>>();

    let mut credit_events = Vec::new();
    for event in db
        .list_trace_credit_events(&tenant.tenant_id)
        .await
        .context("failed to read Trace Commons credit events from DB mirror")?
    {
        let Some(owner_principal_ref) = owner_by_submission.get(&event.submission_id) else {
            continue;
        };
        if let Some(event) =
            trace_commons_credit_event_from_storage(event, owner_principal_ref.as_str())?
        {
            credit_events.push(event);
        }
    }
    let credit_events =
        eligible_credit_events_for_records(&records, visible_credit_events(tenant, credit_events));
    Ok(TraceContributorCreditView {
        records,
        credit_events,
    })
}

async fn read_audit_events(
    state: &AppState,
    tenant: &TenantAuth,
) -> anyhow::Result<Vec<TraceCommonsAuditEvent>> {
    if state.db_audit_reads {
        return read_audit_events_from_db(state, tenant).await;
    }

    read_all_audit_events(&state.root, &tenant.tenant_id)
}

async fn read_audit_events_from_db(
    state: &AppState,
    tenant: &TenantAuth,
) -> anyhow::Result<Vec<TraceCommonsAuditEvent>> {
    let db = state
        .db_mirror
        .as_ref()
        .context("TRACE_COMMONS_DB_AUDIT_READS is enabled without a DB mirror")?;
    db.list_trace_audit_events(&tenant.tenant_id)
        .await
        .context("failed to read Trace Commons audit events from DB mirror")?
        .into_iter()
        .map(trace_commons_audit_event_from_storage)
        .collect()
}

fn trace_commons_record_from_storage_submission(
    record: StorageTraceSubmissionRecord,
) -> Option<anyhow::Result<TraceCommonsSubmissionRecord>> {
    let status = trace_corpus_status_from_storage(record.status)?;
    Some((|| {
        let object_key = trace_envelope_object_key(&record.tenant_id, status, record.submission_id);
        Ok(TraceCommonsSubmissionRecord {
            tenant_storage_ref: tenant_storage_ref(&record.tenant_id),
            tenant_id: record.tenant_id,
            auth_principal_ref: record.auth_principal_ref,
            submitted_tenant_scope_ref: record.submitted_tenant_scope_ref,
            contributor_pseudonym: record.contributor_pseudonym,
            submission_id: record.submission_id,
            trace_id: record.trace_id,
            status,
            privacy_risk: storage_string_as(&record.privacy_risk, "privacy_risk")?,
            submission_score: record.submission_score.unwrap_or(0.0),
            credit_points_pending: record.credit_points_pending.unwrap_or(0.0),
            credit_points_final: record.credit_points_final,
            consent_scopes: record
                .consent_scopes
                .iter()
                .map(|scope| storage_string_as(scope, "consent_scope"))
                .collect::<anyhow::Result<Vec<_>>>()?,
            redaction_counts: record.redaction_counts,
            received_at: record.received_at,
            retention_policy_id: record.retention_policy_id,
            expires_at: record.expires_at,
            purged_at: record.purged_at,
            object_key,
            artifact_receipt: None,
        })
    })())
}

fn trace_commons_audit_event_from_storage(
    event: StorageTraceAuditEventRecord,
) -> anyhow::Result<TraceCommonsAuditEvent> {
    let mut kind = storage_audit_event_kind(event.action, &event.metadata);
    if event.action == StorageTraceAuditAction::Read
        && event.submission_id.is_some()
        && event
            .reason
            .as_deref()
            .is_some_and(|reason| reason.contains("surface=replay_dataset_export"))
    {
        kind = "trace_content_read".to_string();
    }
    let (status, reason, export_count) = match &event.metadata {
        StorageTraceAuditSafeMetadata::Submission {
            status,
            privacy_risk: _,
        } => (
            trace_corpus_status_from_storage(*status),
            event.reason.clone(),
            None,
        ),
        StorageTraceAuditSafeMetadata::ReviewDecision {
            decision: _,
            resulting_status,
            reason_code,
        } => (
            trace_corpus_status_from_storage(*resulting_status),
            reason_code.clone().or_else(|| event.reason.clone()),
            None,
        ),
        StorageTraceAuditSafeMetadata::Export {
            artifact_kind: _,
            purpose_code,
            item_count,
        } => (
            None,
            purpose_code.clone().or_else(|| event.reason.clone()),
            Some(*item_count as usize),
        ),
        StorageTraceAuditSafeMetadata::Maintenance {
            dry_run,
            action_counts,
        } => (
            None,
            Some(format!(
                "dry_run={dry_run};action_counts={}",
                serde_json::to_string(action_counts)
                    .context("failed to serialize trace audit action_counts")?
            )),
            Some(
                action_counts
                    .values()
                    .copied()
                    .map(|count| count as usize)
                    .sum(),
            ),
        ),
        StorageTraceAuditSafeMetadata::Empty => (None, event.reason.clone(), None),
    };
    Ok(TraceCommonsAuditEvent {
        event_id: event.audit_event_id,
        tenant_id: event.tenant_id,
        submission_id: event.submission_id.unwrap_or_else(Uuid::nil),
        kind,
        created_at: event.occurred_at,
        status,
        actor_role: TokenRole::parse(&event.actor_role).ok(),
        actor_principal_ref: Some(event.actor_principal_ref),
        reason,
        export_count,
        export_id: event.export_manifest_id,
        decision_inputs_hash: event.decision_inputs_hash,
    })
}

fn storage_audit_event_kind(
    action: StorageTraceAuditAction,
    metadata: &StorageTraceAuditSafeMetadata,
) -> String {
    if let StorageTraceAuditAction::Export = action
        && let StorageTraceAuditSafeMetadata::Export {
            purpose_code: Some(purpose_code),
            ..
        } = metadata
        && matches!(
            purpose_code.as_str(),
            "ranker_training_candidates_export" | "ranker_training_pairs_export"
        )
    {
        return purpose_code.clone();
    }
    storage_audit_action_kind(action).to_string()
}

fn storage_audit_action_kind(action: StorageTraceAuditAction) -> &'static str {
    match action {
        StorageTraceAuditAction::Submit => "submitted",
        StorageTraceAuditAction::Read => "read",
        StorageTraceAuditAction::Review => "review_decision",
        StorageTraceAuditAction::CreditMutate => "credit_mutate",
        StorageTraceAuditAction::Revoke => "revoked",
        StorageTraceAuditAction::Export => "dataset_export",
        StorageTraceAuditAction::Retain => "retain",
        StorageTraceAuditAction::Purge => "purge",
        StorageTraceAuditAction::VectorIndex => "vector_index",
        StorageTraceAuditAction::BenchmarkConvert => "benchmark_conversion",
    }
}

fn trace_commons_derived_record_from_storage(
    record: StorageTraceDerivedRecord,
    submission_metadata: &BTreeMap<Uuid, (TraceCorpusStatus, ResidualPiiRisk, String)>,
) -> Option<anyhow::Result<TraceCommonsDerivedRecord>> {
    let (submission_status, submission_privacy_risk, tenant_id) =
        submission_metadata.get(&record.submission_id)?;
    let status = match record.status {
        StorageTraceDerivedStatus::Current => *submission_status,
        StorageTraceDerivedStatus::Revoked => TraceCorpusStatus::Revoked,
        StorageTraceDerivedStatus::Expired => TraceCorpusStatus::Expired,
        StorageTraceDerivedStatus::Invalidated | StorageTraceDerivedStatus::Superseded => {
            return None;
        }
    };
    Some((|| {
        let privacy_risk = match record.privacy_risk.as_deref() {
            Some(raw) => storage_string_as(raw, "derived_privacy_risk")?,
            None => *submission_privacy_risk,
        };
        Ok(TraceCommonsDerivedRecord {
            tenant_storage_ref: tenant_storage_ref(tenant_id),
            tenant_id: tenant_id.clone(),
            derived_id: Some(record.derived_id),
            submission_id: record.submission_id,
            trace_id: record.trace_id,
            status,
            privacy_risk,
            task_success: record.task_success.unwrap_or_else(|| "unknown".to_string()),
            canonical_summary: record.canonical_summary.unwrap_or_default(),
            canonical_summary_hash: record.canonical_summary_hash.unwrap_or_default(),
            summary_model: record.summary_model,
            event_count: record
                .event_count
                .and_then(|value| usize::try_from(value).ok())
                .unwrap_or_default(),
            tool_sequence: record.tool_sequence,
            tool_categories: record.tool_categories,
            coverage_tags: record.coverage_tags,
            duplicate_score: record.duplicate_score.unwrap_or_default(),
            novelty_score: record.novelty_score.unwrap_or_default(),
            created_at: record.created_at,
        })
    })())
}

fn trace_commons_credit_event_from_storage(
    event: StorageTraceCreditEventRecord,
    owner_principal_ref: &str,
) -> anyhow::Result<Option<TraceCommonsCreditLedgerRecord>> {
    let Some(event_type) = trace_credit_event_type_from_storage(event.event_type) else {
        return Ok(None);
    };
    let credit_points_delta = event.points_delta.parse::<f32>().with_context(|| {
        format!(
            "failed to parse trace credit points_delta for event {}",
            event.credit_event_id
        )
    })?;
    if !credit_points_delta.is_finite() {
        anyhow::bail!(
            "trace credit points_delta is not finite for event {}",
            event.credit_event_id
        );
    }
    Ok(Some(TraceCommonsCreditLedgerRecord {
        event_id: event.credit_event_id,
        tenant_storage_ref: tenant_storage_ref(&event.tenant_id),
        tenant_id: event.tenant_id,
        submission_id: event.submission_id,
        trace_id: event.trace_id,
        auth_principal_ref: owner_principal_ref.to_string(),
        event_type,
        credit_points_delta,
        reason: Some(event.reason),
        external_ref: event.external_ref,
        actor_role: TokenRole::parse(&event.actor_role)?,
        actor_principal_ref: event.actor_principal_ref,
        created_at: event.occurred_at,
    }))
}

fn trace_corpus_status_from_storage(status: StorageTraceCorpusStatus) -> Option<TraceCorpusStatus> {
    match status {
        StorageTraceCorpusStatus::Accepted => Some(TraceCorpusStatus::Accepted),
        StorageTraceCorpusStatus::Quarantined => Some(TraceCorpusStatus::Quarantined),
        StorageTraceCorpusStatus::Rejected => Some(TraceCorpusStatus::Rejected),
        StorageTraceCorpusStatus::Revoked => Some(TraceCorpusStatus::Revoked),
        StorageTraceCorpusStatus::Expired => Some(TraceCorpusStatus::Expired),
        StorageTraceCorpusStatus::Purged => Some(TraceCorpusStatus::Purged),
        StorageTraceCorpusStatus::Received => None,
    }
}

fn trace_credit_event_type_from_storage(
    event_type: StorageTraceCreditEventType,
) -> Option<TraceCreditLedgerEventType> {
    match event_type {
        StorageTraceCreditEventType::Accepted
        | StorageTraceCreditEventType::PrivacyRejection
        | StorageTraceCreditEventType::DuplicateRejection => None,
        StorageTraceCreditEventType::BenchmarkConversion => {
            Some(TraceCreditLedgerEventType::BenchmarkConversion)
        }
        StorageTraceCreditEventType::RegressionCatch => {
            Some(TraceCreditLedgerEventType::RegressionCatch)
        }
        StorageTraceCreditEventType::TrainingUtility => {
            Some(TraceCreditLedgerEventType::TrainingUtility)
        }
        StorageTraceCreditEventType::ReviewerBonus => {
            Some(TraceCreditLedgerEventType::ReviewerBonus)
        }
        StorageTraceCreditEventType::AbusePenalty => Some(TraceCreditLedgerEventType::AbusePenalty),
    }
}

fn storage_string_as<T: DeserializeOwned>(raw: &str, label: &str) -> anyhow::Result<T> {
    serde_json::from_value(serde_json::Value::String(raw.to_string()))
        .with_context(|| format!("failed to parse Trace Commons storage {label}: {raw}"))
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
        TraceCorpusStatus::Expired => vec!["Expired under the retention policy.".to_string()],
        TraceCorpusStatus::Purged => vec!["Purged under the retention policy.".to_string()],
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

#[derive(Debug, Clone)]
struct StoredTraceEnvelope {
    object_key: String,
    artifact_receipt: Option<EncryptedTraceArtifactReceipt>,
}

fn store_envelope(
    state: &AppState,
    tenant_id: &str,
    status: TraceCorpusStatus,
    envelope: &TraceContributionEnvelope,
) -> anyhow::Result<StoredTraceEnvelope> {
    let object_key = trace_envelope_object_key(tenant_id, status, envelope.submission_id);
    let path = state.root.join(&object_key);
    write_json_file(&path, envelope, "trace contribution envelope")?;
    let artifact_receipt = if let Some(store) = state.artifact_store.as_ref() {
        Some(store.put_json(
            &tenant_storage_ref(tenant_id),
            TraceArtifactKind::ContributionEnvelope,
            &envelope.submission_id.to_string(),
            envelope,
        )?)
    } else {
        None
    };
    Ok(StoredTraceEnvelope {
        object_key,
        artifact_receipt,
    })
}

fn trace_envelope_object_key(
    tenant_id: &str,
    status: TraceCorpusStatus,
    submission_id: Uuid,
) -> String {
    let tenant_key = tenant_storage_key(tenant_id);
    format!(
        "tenants/{tenant_key}/objects/{}/{}.json",
        status.as_str(),
        submission_id
    )
}

async fn mirror_submission_to_db(
    state: &AppState,
    tenant: &TenantAuth,
    record: &TraceCommonsSubmissionRecord,
    derived_record: &TraceCommonsDerivedRecord,
    envelope: &TraceContributionEnvelope,
) -> anyhow::Result<()> {
    let Some(db) = state.db_mirror.as_ref() else {
        return Ok(());
    };
    let envelope_json = serde_json::to_string_pretty(envelope)
        .context("failed to serialize trace envelope for DB mirror hashing")?;
    let object_ref_id = deterministic_trace_uuid("submitted-envelope", record);
    let derived_id = deterministic_trace_uuid("derived-precheck", record);
    let plaintext_sha256 = sha256_prefixed(&envelope_json);
    let (object_store, object_key, content_sha256) =
        if let Some(receipt) = record.artifact_receipt.as_ref() {
            (
                "trace_commons_encrypted_artifact_store".to_string(),
                receipt.object_key.clone(),
                format!("sha256:{}", receipt.ciphertext_sha256),
            )
        } else {
            (
                "trace_commons_file_store".to_string(),
                record.object_key.clone(),
                plaintext_sha256,
            )
        };
    let privacy_risk = serde_storage_string(&record.privacy_risk)?;
    let credit_account_ref = envelope
        .contributor
        .credit_account_ref
        .clone()
        .or_else(|| record.contributor_pseudonym.clone())
        .unwrap_or_else(|| record.auth_principal_ref.clone());

    db.upsert_trace_submission(storage_submission_write_from_record(
        record,
        envelope,
        Some(derived_record.canonical_summary_hash.clone()),
    )?)
    .await
    .context("failed to mirror trace submission metadata")?;

    db.append_trace_object_ref(StorageTraceObjectRefWrite {
        object_ref_id,
        tenant_id: record.tenant_id.clone(),
        submission_id: record.submission_id,
        artifact_kind: StorageTraceObjectArtifactKind::SubmittedEnvelope,
        object_store,
        object_key,
        content_sha256: content_sha256.clone(),
        encryption_key_ref: format!("tenant:{}", tenant_storage_ref(&record.tenant_id)),
        size_bytes: i64::try_from(envelope_json.len()).unwrap_or(i64::MAX),
        compression: None,
        created_by_job_id: None,
    })
    .await
    .context("failed to mirror trace object ref")?;

    db.append_trace_derived_record(StorageTraceDerivedRecordWrite {
        derived_id,
        tenant_id: record.tenant_id.clone(),
        submission_id: record.submission_id,
        trace_id: record.trace_id,
        status: storage_derived_status(record.status),
        worker_kind: StorageTraceWorkerKind::DuplicatePrecheck,
        worker_version: "trace_commons_ingest_v1".to_string(),
        input_object_ref: Some(ironclaw::trace_corpus_storage::TenantScopedTraceObjectRef {
            tenant_id: record.tenant_id.clone(),
            submission_id: record.submission_id,
            object_ref_id,
        }),
        input_hash: content_sha256,
        output_object_ref: None,
        canonical_summary: Some(derived_record.canonical_summary.clone()),
        canonical_summary_hash: Some(derived_record.canonical_summary_hash.clone()),
        summary_model: derived_record.summary_model.clone(),
        task_success: Some(derived_record.task_success.clone()),
        privacy_risk: Some(privacy_risk.clone()),
        event_count: Some(derived_record.event_count.min(i32::MAX as usize) as i32),
        tool_sequence: derived_record.tool_sequence.clone(),
        tool_categories: derived_record.tool_categories.clone(),
        coverage_tags: derived_record.coverage_tags.clone(),
        duplicate_score: Some(derived_record.duplicate_score),
        novelty_score: Some(derived_record.novelty_score),
        cluster_id: envelope.embedding_analysis.as_ref().and_then(|analysis| {
            analysis
                .cluster_id
                .clone()
                .or_else(|| analysis.nearest_cluster_id.clone())
        }),
    })
    .await
    .context("failed to mirror trace derived metadata")?;

    db.append_trace_audit_event(StorageTraceAuditEventWrite {
        audit_event_id: deterministic_trace_uuid("submit-audit", record),
        tenant_id: record.tenant_id.clone(),
        actor_principal_ref: record.auth_principal_ref.clone(),
        actor_role: format!("{:?}", tenant.role).to_ascii_lowercase(),
        action: StorageTraceAuditAction::Submit,
        reason: None,
        request_id: None,
        submission_id: Some(record.submission_id),
        object_ref_id: Some(object_ref_id),
        export_manifest_id: None,
        decision_inputs_hash: Some(derived_record.canonical_summary_hash.clone()),
        metadata: StorageTraceAuditSafeMetadata::Submission {
            status: storage_corpus_status(record.status),
            privacy_risk: privacy_risk.clone(),
        },
    })
    .await
    .context("failed to mirror trace audit event")?;

    if record.status == TraceCorpusStatus::Accepted && record.credit_points_pending > 0.0 {
        db.append_trace_credit_event(StorageTraceCreditEventWrite {
            credit_event_id: deterministic_trace_uuid("accepted-credit", record),
            tenant_id: record.tenant_id.clone(),
            submission_id: record.submission_id,
            trace_id: record.trace_id,
            credit_account_ref,
            event_type: StorageTraceCreditEventType::Accepted,
            points_delta: format!("{:.4}", record.credit_points_pending),
            reason: "Accepted by Trace Commons ingest privacy checks.".to_string(),
            external_ref: None,
            actor_principal_ref: record.auth_principal_ref.clone(),
            actor_role: "system".to_string(),
            settlement_state: StorageTraceCreditSettlementState::Pending,
        })
        .await
        .context("failed to mirror trace credit event")?;
    }

    Ok(())
}

async fn mirror_revocation_to_db(
    state: &AppState,
    tenant: &TenantAuth,
    submission_id: Uuid,
    record: Option<&TraceCommonsSubmissionRecord>,
) -> anyhow::Result<()> {
    let Some(db) = state.db_mirror.as_ref() else {
        return Ok(());
    };

    if let Some(record) = record {
        db.write_trace_tombstone(StorageTraceTombstoneWrite {
            tombstone_id: deterministic_trace_uuid("revocation-tombstone", record),
            tenant_id: record.tenant_id.clone(),
            submission_id: record.submission_id,
            trace_id: Some(record.trace_id),
            redaction_hash: None,
            canonical_summary_hash: None,
            reason: "contributor_revocation".to_string(),
            effective_at: Utc::now(),
            retain_until: None,
            created_by_principal_ref: tenant.principal_ref.clone(),
        })
        .await
        .context("failed to mirror trace revocation tombstone")?;
    }

    db.update_trace_submission_status(
        &tenant.tenant_id,
        submission_id,
        StorageTraceCorpusStatus::Revoked,
        &tenant.principal_ref,
        Some("contributor_revocation"),
    )
    .await
    .context("failed to mirror trace revocation status")?;

    let invalidation_counts = db
        .invalidate_trace_submission_artifacts(
            &tenant.tenant_id,
            submission_id,
            StorageTraceDerivedStatus::Revoked,
        )
        .await
        .context("failed to mirror trace artifact invalidation")?;
    let vector_entries_invalidated = db
        .invalidate_trace_vector_entries_for_submission(&tenant.tenant_id, submission_id)
        .await
        .context("failed to mirror trace vector invalidation")?;
    let export_manifests_invalidated = db
        .invalidate_trace_export_manifests_for_submission(&tenant.tenant_id, submission_id)
        .await
        .context("failed to mirror trace export manifest invalidation")?;
    let export_manifest_items_invalidated = db
        .invalidate_trace_export_manifest_items_for_submission(
            &tenant.tenant_id,
            submission_id,
            StorageTraceExportManifestItemInvalidationReason::Revoked,
        )
        .await
        .context("failed to mirror trace export manifest item invalidation")?;

    if let Some(record) = record
        && (invalidation_counts.object_refs_invalidated > 0
            || invalidation_counts.derived_records_invalidated > 0
            || vector_entries_invalidated > 0
            || export_manifests_invalidated > 0
            || export_manifest_items_invalidated > 0)
    {
        let mut action_counts = BTreeMap::new();
        action_counts.insert(
            "object_refs_invalidated".to_string(),
            invalidation_counts
                .object_refs_invalidated
                .min(u64::from(u32::MAX)) as u32,
        );
        action_counts.insert(
            "derived_records_invalidated".to_string(),
            invalidation_counts
                .derived_records_invalidated
                .min(u64::from(u32::MAX)) as u32,
        );
        action_counts.insert(
            "vector_entries_invalidated".to_string(),
            vector_entries_invalidated.min(u64::from(u32::MAX)) as u32,
        );
        action_counts.insert(
            "export_manifests_invalidated".to_string(),
            export_manifests_invalidated.min(u64::from(u32::MAX)) as u32,
        );
        action_counts.insert(
            "export_manifest_items_invalidated".to_string(),
            export_manifest_items_invalidated.min(u64::from(u32::MAX)) as u32,
        );
        db.append_trace_audit_event(StorageTraceAuditEventWrite {
            audit_event_id: deterministic_trace_uuid("revocation-artifact-invalidation", record),
            tenant_id: record.tenant_id.clone(),
            actor_principal_ref: tenant.principal_ref.clone(),
            actor_role: format!("{:?}", tenant.role).to_ascii_lowercase(),
            action: StorageTraceAuditAction::Revoke,
            reason: Some("contributor_revocation_artifact_invalidation".to_string()),
            request_id: None,
            submission_id: Some(record.submission_id),
            object_ref_id: None,
            export_manifest_id: None,
            decision_inputs_hash: None,
            metadata: StorageTraceAuditSafeMetadata::Maintenance {
                dry_run: false,
                action_counts,
            },
        })
        .await
        .context("failed to mirror trace artifact invalidation audit")?;
    }

    Ok(())
}

async fn mirror_expiration_to_db(
    state: &AppState,
    tenant: &TenantAuth,
    submission_id: Uuid,
) -> anyhow::Result<()> {
    let Some(db) = state.db_mirror.as_ref() else {
        return Ok(());
    };
    if db
        .get_trace_submission(&tenant.tenant_id, submission_id)
        .await
        .context("failed to check trace submission before expiration mirror")?
        .is_none()
    {
        return Ok(());
    }
    db.update_trace_submission_status(
        &tenant.tenant_id,
        submission_id,
        StorageTraceCorpusStatus::Expired,
        &tenant.principal_ref,
        Some("retention_expired"),
    )
    .await
    .context("failed to mirror trace expiration status")?;
    db.invalidate_trace_submission_artifacts(
        &tenant.tenant_id,
        submission_id,
        StorageTraceDerivedStatus::Expired,
    )
    .await
    .context("failed to mirror trace expiration artifact invalidation")?;
    db.invalidate_trace_vector_entries_for_submission(&tenant.tenant_id, submission_id)
        .await
        .context("failed to mirror trace expiration vector invalidation")?;
    db.invalidate_trace_export_manifests_for_submission(&tenant.tenant_id, submission_id)
        .await
        .context("failed to mirror trace expiration export manifest invalidation")?;
    db.invalidate_trace_export_manifest_items_for_submission(
        &tenant.tenant_id,
        submission_id,
        StorageTraceExportManifestItemInvalidationReason::Expired,
    )
    .await
    .context("failed to mirror trace expiration export manifest item invalidation")?;
    Ok(())
}

async fn mirror_purge_to_db(
    state: &AppState,
    tenant: &TenantAuth,
    submission_id: Uuid,
) -> anyhow::Result<()> {
    let Some(db) = state.db_mirror.as_ref() else {
        return Ok(());
    };
    if db
        .get_trace_submission(&tenant.tenant_id, submission_id)
        .await
        .context("failed to check trace submission before purge mirror")?
        .is_none()
    {
        return Ok(());
    }
    db.update_trace_submission_status(
        &tenant.tenant_id,
        submission_id,
        StorageTraceCorpusStatus::Purged,
        &tenant.principal_ref,
        Some("retention_purged"),
    )
    .await
    .context("failed to mirror trace purge status")?;
    db.invalidate_trace_submission_artifacts(
        &tenant.tenant_id,
        submission_id,
        StorageTraceDerivedStatus::Expired,
    )
    .await
    .context("failed to mirror trace purge artifact invalidation")?;
    db.invalidate_trace_vector_entries_for_submission(&tenant.tenant_id, submission_id)
        .await
        .context("failed to mirror trace purge vector invalidation")?;
    db.invalidate_trace_export_manifests_for_submission(&tenant.tenant_id, submission_id)
        .await
        .context("failed to mirror trace purge export manifest invalidation")?;
    db.invalidate_trace_export_manifest_items_for_submission(
        &tenant.tenant_id,
        submission_id,
        StorageTraceExportManifestItemInvalidationReason::Purged,
    )
    .await
    .context("failed to mirror trace purge export manifest item invalidation")?;
    Ok(())
}

async fn mirror_review_decision_to_db(
    state: &AppState,
    tenant: &TenantAuth,
    record: &TraceCommonsSubmissionRecord,
    envelope: &TraceContributionEnvelope,
    derived_record: Option<&TraceCommonsDerivedRecord>,
) -> anyhow::Result<()> {
    let Some(db) = state.db_mirror.as_ref() else {
        return Ok(());
    };
    db.upsert_trace_submission(storage_submission_write_from_record(
        record,
        envelope,
        derived_record.map(|derived| derived.canonical_summary_hash.clone()),
    )?)
    .await
    .context("failed to mirror reviewed trace submission metadata")?;
    db.update_trace_submission_status(
        &record.tenant_id,
        record.submission_id,
        storage_corpus_status(record.status),
        &tenant.principal_ref,
        Some("review_decision"),
    )
    .await
    .context("failed to mirror trace review status")?;
    Ok(())
}

async fn mirror_credit_event_to_db(
    state: &AppState,
    event: &TraceCommonsCreditLedgerRecord,
) -> anyhow::Result<()> {
    let Some(db) = state.db_mirror.as_ref() else {
        return Ok(());
    };
    db.append_trace_credit_event(StorageTraceCreditEventWrite {
        credit_event_id: event.event_id,
        tenant_id: event.tenant_id.clone(),
        submission_id: event.submission_id,
        trace_id: event.trace_id,
        credit_account_ref: event.auth_principal_ref.clone(),
        event_type: storage_credit_event_type(event.event_type),
        points_delta: format!("{:.4}", event.credit_points_delta),
        reason: event
            .reason
            .clone()
            .unwrap_or_else(|| "delayed credit event".to_string()),
        external_ref: event.external_ref.clone(),
        actor_principal_ref: event.actor_principal_ref.clone(),
        actor_role: format!("{:?}", event.actor_role).to_ascii_lowercase(),
        settlement_state: StorageTraceCreditSettlementState::Final,
    })
    .await
    .context("failed to mirror trace credit ledger event")
}

async fn mirror_export_manifest_to_db(
    state: &AppState,
    artifact_kind: StorageTraceObjectArtifactKind,
    manifest: &TraceReplayExportManifest,
    items: &[TraceReplayDatasetItem],
) -> anyhow::Result<()> {
    let Some(db) = state.db_mirror.as_ref() else {
        return Ok(());
    };
    db.upsert_trace_export_manifest(StorageTraceExportManifestWrite {
        tenant_id: manifest.tenant_id.clone(),
        export_manifest_id: manifest.export_id,
        artifact_kind,
        purpose_code: Some(manifest.purpose.clone()),
        audit_event_id: Some(manifest.audit_event_id),
        source_submission_ids: manifest.source_submission_ids.clone(),
        source_submission_ids_hash: manifest.source_submission_ids_hash.clone(),
        item_count: manifest.source_submission_ids.len().min(u32::MAX as usize) as u32,
        generated_at: manifest.generated_at,
    })
    .await
    .context("failed to mirror trace export manifest metadata")?;
    for item in items {
        db.upsert_trace_export_manifest_item(StorageTraceExportManifestItemWrite {
            tenant_id: manifest.tenant_id.clone(),
            export_manifest_id: manifest.export_id,
            submission_id: item.submission_id,
            trace_id: item.trace_id,
            derived_id: None,
            object_ref_id: item.object_ref_id,
            vector_entry_id: None,
            source_status_at_export: storage_corpus_status(item.source_status_at_export),
            source_hash_at_export: item.source_hash_at_export.clone(),
        })
        .await
        .with_context(|| {
            format!(
                "failed to mirror trace export manifest item metadata for {}",
                item.submission_id
            )
        })?;
    }
    Ok(())
}

async fn mirror_benchmark_export_provenance_to_db(
    state: &AppState,
    artifact: &TraceBenchmarkConversionArtifact,
) -> anyhow::Result<()> {
    let Some(db) = state.db_mirror.as_ref() else {
        return Ok(());
    };
    db.upsert_trace_export_manifest(StorageTraceExportManifestWrite {
        tenant_id: artifact.tenant_id.clone(),
        export_manifest_id: artifact.conversion_id,
        artifact_kind: StorageTraceObjectArtifactKind::BenchmarkArtifact,
        purpose_code: Some(artifact.purpose.clone()),
        audit_event_id: Some(artifact.audit_event_id),
        source_submission_ids: artifact.source_submission_ids.clone(),
        source_submission_ids_hash: artifact.source_submission_ids_hash.clone(),
        item_count: artifact.item_count.min(u32::MAX as usize) as u32,
        generated_at: artifact.generated_at,
    })
    .await
    .context("failed to mirror benchmark provenance manifest metadata")?;
    for candidate in &artifact.candidates {
        db.upsert_trace_export_manifest_item(StorageTraceExportManifestItemWrite {
            tenant_id: artifact.tenant_id.clone(),
            export_manifest_id: artifact.conversion_id,
            submission_id: candidate.submission_id,
            trace_id: candidate.trace_id,
            derived_id: Some(candidate.derived_id),
            object_ref_id: None,
            vector_entry_id: None,
            source_status_at_export: StorageTraceCorpusStatus::Accepted,
            source_hash_at_export: candidate.canonical_summary_hash.clone(),
        })
        .await
        .with_context(|| {
            format!(
                "failed to mirror benchmark provenance item metadata for {}",
                candidate.submission_id
            )
        })?;
    }
    Ok(())
}

async fn mirror_ranker_candidate_export_provenance_to_db(
    state: &AppState,
    provenance: &TraceExportProvenanceManifest,
    candidates: &[TraceRankerTrainingCandidate],
) -> anyhow::Result<()> {
    let Some(db) = state.db_mirror.as_ref() else {
        return Ok(());
    };
    upsert_provenance_manifest_to_db(
        db.as_ref(),
        provenance,
        StorageTraceObjectArtifactKind::ExportArtifact,
        candidates.len(),
    )
    .await?;
    for candidate in candidates {
        db.upsert_trace_export_manifest_item(StorageTraceExportManifestItemWrite {
            tenant_id: provenance.tenant_id.clone(),
            export_manifest_id: provenance.export_id,
            submission_id: candidate.submission_id,
            trace_id: candidate.trace_id,
            derived_id: Some(candidate.derived_id),
            object_ref_id: None,
            vector_entry_id: None,
            source_status_at_export: storage_corpus_status(candidate.status),
            source_hash_at_export: candidate.canonical_summary_hash.clone(),
        })
        .await
        .with_context(|| {
            format!(
                "failed to mirror ranker candidate provenance item metadata for {}",
                candidate.submission_id
            )
        })?;
    }
    Ok(())
}

async fn mirror_ranker_pair_export_provenance_to_db(
    state: &AppState,
    provenance: &TraceExportProvenanceManifest,
    pairs: &[TraceRankerTrainingPair],
) -> anyhow::Result<()> {
    let Some(db) = state.db_mirror.as_ref() else {
        return Ok(());
    };
    upsert_provenance_manifest_to_db(
        db.as_ref(),
        provenance,
        StorageTraceObjectArtifactKind::ExportArtifact,
        provenance.source_submission_ids.len(),
    )
    .await?;
    for pair in pairs {
        for candidate in [&pair.preferred, &pair.rejected] {
            db.upsert_trace_export_manifest_item(StorageTraceExportManifestItemWrite {
                tenant_id: provenance.tenant_id.clone(),
                export_manifest_id: provenance.export_id,
                submission_id: candidate.submission_id,
                trace_id: candidate.trace_id,
                derived_id: Some(candidate.derived_id),
                object_ref_id: None,
                vector_entry_id: None,
                source_status_at_export: storage_corpus_status(candidate.status),
                source_hash_at_export: candidate.canonical_summary_hash.clone(),
            })
            .await
            .with_context(|| {
                format!(
                    "failed to mirror ranker pair provenance item metadata for {}",
                    candidate.submission_id
                )
            })?;
        }
    }
    Ok(())
}

async fn upsert_provenance_manifest_to_db(
    db: &dyn Database,
    provenance: &TraceExportProvenanceManifest,
    artifact_kind: StorageTraceObjectArtifactKind,
    item_count: usize,
) -> anyhow::Result<()> {
    db.upsert_trace_export_manifest(StorageTraceExportManifestWrite {
        tenant_id: provenance.tenant_id.clone(),
        export_manifest_id: provenance.export_id,
        artifact_kind,
        purpose_code: Some(provenance.purpose.clone()),
        audit_event_id: Some(provenance.audit_event_id),
        source_submission_ids: provenance.source_submission_ids.clone(),
        source_submission_ids_hash: provenance.source_submission_ids_hash.clone(),
        item_count: item_count.min(u32::MAX as usize) as u32,
        generated_at: provenance.generated_at,
    })
    .await
    .context("failed to mirror export provenance manifest metadata")?;
    Ok(())
}

fn storage_submission_write_from_record(
    record: &TraceCommonsSubmissionRecord,
    envelope: &TraceContributionEnvelope,
    canonical_summary_hash: Option<String>,
) -> anyhow::Result<StorageTraceSubmissionWrite> {
    let consent_scopes = consent_scope_storage_strings(&record.consent_scopes)?;
    Ok(StorageTraceSubmissionWrite {
        tenant_id: record.tenant_id.clone(),
        submission_id: record.submission_id,
        trace_id: record.trace_id,
        auth_principal_ref: record.auth_principal_ref.clone(),
        contributor_pseudonym: record.contributor_pseudonym.clone(),
        submitted_tenant_scope_ref: record.submitted_tenant_scope_ref.clone(),
        schema_version: envelope.schema_version.clone(),
        consent_policy_version: envelope.consent.policy_version.clone(),
        consent_scopes: consent_scopes.clone(),
        allowed_uses: consent_scopes,
        retention_policy_id: record.retention_policy_id.clone(),
        status: storage_corpus_status(record.status),
        privacy_risk: serde_storage_string(&record.privacy_risk)?,
        redaction_pipeline_version: envelope.privacy.redaction_pipeline_version.clone(),
        redaction_counts: record.redaction_counts.clone(),
        redaction_hash: envelope.privacy.redaction_hash.clone(),
        canonical_summary_hash,
        submission_score: Some(record.submission_score),
        credit_points_pending: Some(record.credit_points_pending),
        credit_points_final: record.credit_points_final,
        expires_at: record.expires_at,
    })
}

fn deterministic_trace_uuid(label: &str, record: &TraceCommonsSubmissionRecord) -> Uuid {
    deterministic_trace_uuid_for(label, &record.tenant_id, record.submission_id)
}

fn deterministic_trace_uuid_for(label: &str, tenant_id: &str, submission_id: Uuid) -> Uuid {
    let input = format!(
        "ironclaw.trace_commons.{label}:{}:{}",
        tenant_id, submission_id
    );
    Uuid::new_v5(&Uuid::NAMESPACE_URL, input.as_bytes())
}

fn storage_corpus_status(status: TraceCorpusStatus) -> StorageTraceCorpusStatus {
    match status {
        TraceCorpusStatus::Accepted => StorageTraceCorpusStatus::Accepted,
        TraceCorpusStatus::Quarantined => StorageTraceCorpusStatus::Quarantined,
        TraceCorpusStatus::Rejected => StorageTraceCorpusStatus::Rejected,
        TraceCorpusStatus::Revoked => StorageTraceCorpusStatus::Revoked,
        TraceCorpusStatus::Expired => StorageTraceCorpusStatus::Expired,
        TraceCorpusStatus::Purged => StorageTraceCorpusStatus::Purged,
    }
}

fn storage_derived_status(status: TraceCorpusStatus) -> StorageTraceDerivedStatus {
    match status {
        TraceCorpusStatus::Revoked => StorageTraceDerivedStatus::Revoked,
        TraceCorpusStatus::Expired | TraceCorpusStatus::Purged => {
            StorageTraceDerivedStatus::Expired
        }
        TraceCorpusStatus::Rejected => StorageTraceDerivedStatus::Invalidated,
        TraceCorpusStatus::Accepted | TraceCorpusStatus::Quarantined => {
            StorageTraceDerivedStatus::Current
        }
    }
}

fn storage_credit_event_type(
    event_type: TraceCreditLedgerEventType,
) -> StorageTraceCreditEventType {
    match event_type {
        TraceCreditLedgerEventType::BenchmarkConversion => {
            StorageTraceCreditEventType::BenchmarkConversion
        }
        TraceCreditLedgerEventType::RegressionCatch => StorageTraceCreditEventType::RegressionCatch,
        TraceCreditLedgerEventType::TrainingUtility
        | TraceCreditLedgerEventType::RankingUtility => {
            StorageTraceCreditEventType::TrainingUtility
        }
        TraceCreditLedgerEventType::ReviewerBonus => StorageTraceCreditEventType::ReviewerBonus,
        TraceCreditLedgerEventType::AbusePenalty => StorageTraceCreditEventType::AbusePenalty,
    }
}

fn consent_scope_storage_strings(scopes: &[ConsentScope]) -> anyhow::Result<Vec<String>> {
    scopes.iter().map(serde_storage_string).collect()
}

fn serde_storage_string<T: Serialize>(value: &T) -> anyhow::Result<String> {
    let value = serde_json::to_value(value).context("failed to serialize storage enum")?;
    value
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| anyhow::anyhow!("storage enum did not serialize to a string"))
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
    state: &AppState,
    record: &TraceCommonsSubmissionRecord,
) -> anyhow::Result<TraceContributionEnvelope> {
    if let (Some(store), Some(receipt)) = (
        state.artifact_store.as_ref(),
        record.artifact_receipt.as_ref(),
    ) {
        return store.get_json(&record.tenant_storage_ref, receipt);
    }

    let path = state.root.join(&record.object_key);
    let body = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read trace object {}", path.display()))?;
    serde_json::from_str(&body)
        .with_context(|| format!("failed to parse trace object {}", path.display()))
}

struct TraceEnvelopeBodyRead {
    envelope: TraceContributionEnvelope,
    object_ref_id: Option<Uuid>,
}

async fn read_envelope_for_replay_export(
    state: &AppState,
    tenant: &TenantAuth,
    record: &TraceCommonsSubmissionRecord,
    surface: &str,
    purpose: Option<&str>,
) -> anyhow::Result<TraceEnvelopeBodyRead> {
    anyhow::ensure!(
        tenant.role.can_review(),
        "trace body read requires reviewer or admin role"
    );
    anyhow::ensure!(
        record.tenant_id == tenant.tenant_id,
        "trace body read tenant mismatch"
    );
    anyhow::ensure!(
        record.is_export_eligible(),
        "trace body read source is not export eligible"
    );
    let body_read = read_envelope_body_for_replay_export(state, tenant, record).await?;
    append_trace_content_read_audit(
        state,
        tenant,
        record.submission_id,
        body_read.object_ref_id,
        surface,
        purpose,
    )
    .await?;
    Ok(body_read)
}

async fn read_envelope_body_for_replay_export(
    state: &AppState,
    tenant: &TenantAuth,
    record: &TraceCommonsSubmissionRecord,
) -> anyhow::Result<TraceEnvelopeBodyRead> {
    if state.db_replay_export_reads
        && let Some(envelope) =
            read_envelope_from_active_db_object_ref(state, &tenant.tenant_id, record.submission_id)
                .await?
    {
        return Ok(envelope);
    }
    Ok(TraceEnvelopeBodyRead {
        envelope: read_envelope_by_record(state, record)?,
        object_ref_id: None,
    })
}

async fn read_envelope_from_active_db_object_ref(
    state: &AppState,
    tenant_id: &str,
    submission_id: Uuid,
) -> anyhow::Result<Option<TraceEnvelopeBodyRead>> {
    let Some(db) = state.db_mirror.as_ref() else {
        return Ok(None);
    };
    let Some(object_ref) = db
        .get_latest_active_trace_object_ref(
            tenant_id,
            submission_id,
            StorageTraceObjectArtifactKind::SubmittedEnvelope,
        )
        .await
        .with_context(|| {
            format!(
                "failed to read active submitted envelope object ref for submission {submission_id}"
            )
        })?
    else {
        return Ok(None);
    };
    let envelope = read_envelope_from_object_ref(state, tenant_id, &object_ref)?;
    Ok(Some(TraceEnvelopeBodyRead {
        envelope,
        object_ref_id: Some(object_ref.object_ref_id),
    }))
}

fn read_envelope_from_object_ref(
    state: &AppState,
    tenant_id: &str,
    object_ref: &StorageTraceObjectRefRecord,
) -> anyhow::Result<TraceContributionEnvelope> {
    anyhow::ensure!(
        object_ref.tenant_id == tenant_id,
        "trace object ref tenant mismatch"
    );
    anyhow::ensure!(
        object_ref.artifact_kind == StorageTraceObjectArtifactKind::SubmittedEnvelope,
        "trace object ref artifact kind mismatch"
    );
    anyhow::ensure!(
        object_ref.compression.is_none(),
        "compressed trace object refs are not supported"
    );

    match object_ref.object_store.as_str() {
        "trace_commons_encrypted_artifact_store" => {
            let store = state
                .artifact_store
                .as_ref()
                .context("encrypted trace artifact store is not configured")?;
            store.get_json_by_object_key(
                &tenant_storage_ref(tenant_id),
                TraceArtifactKind::ContributionEnvelope,
                &object_ref.object_key,
                &object_ref.content_sha256,
            )
        }
        "trace_commons_file_store" => read_file_store_envelope_from_object_ref(state, object_ref),
        other => anyhow::bail!("unsupported trace object store: {other}"),
    }
}

fn read_file_store_envelope_from_object_ref(
    state: &AppState,
    object_ref: &StorageTraceObjectRefRecord,
) -> anyhow::Result<TraceContributionEnvelope> {
    let path = trace_object_ref_file_path(&state.root, &object_ref.object_key)?;
    let body = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read trace object {}", path.display()))?;
    let content_sha256 = sha256_prefixed(&body);
    anyhow::ensure!(
        content_sha256 == object_ref.content_sha256,
        "trace object ref content hash mismatch"
    );
    serde_json::from_str(&body)
        .with_context(|| format!("failed to parse trace object {}", path.display()))
}

fn trace_object_ref_file_path(root: &Path, object_key: &str) -> anyhow::Result<PathBuf> {
    let relative_path = Path::new(object_key);
    anyhow::ensure!(
        relative_path.is_relative(),
        "trace object ref file key must be relative"
    );
    anyhow::ensure!(
        relative_path
            .components()
            .all(|component| matches!(component, std::path::Component::Normal(_))),
        "trace object ref file key contains unsafe path components"
    );
    Ok(root.join(relative_path))
}

#[derive(Debug, Default)]
struct TraceObjectDeletionCounts {
    file_deleted: bool,
    encrypted_artifact_deleted: bool,
}

fn delete_trace_objects_for_record(
    state: &AppState,
    record: &TraceCommonsSubmissionRecord,
) -> anyhow::Result<TraceObjectDeletionCounts> {
    let mut counts = TraceObjectDeletionCounts::default();
    let path = state.root.join(&record.object_key);
    if path.exists() {
        std::fs::remove_file(&path)
            .with_context(|| format!("failed to delete trace object {}", path.display()))?;
        counts.file_deleted = true;
    }
    if let (Some(store), Some(receipt)) = (
        state.artifact_store.as_ref(),
        record.artifact_receipt.as_ref(),
    ) {
        counts.encrypted_artifact_deleted =
            store.delete_artifact(&record.tenant_storage_ref, receipt)?;
    }
    Ok(counts)
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
        derived_id: Some(deterministic_trace_uuid_for(
            "derived-precheck",
            tenant_id,
            envelope.submission_id,
        )),
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

async fn append_audit_event_with_db_mirror(
    state: &AppState,
    tenant: &TenantAuth,
    event: TraceCommonsAuditEvent,
    action: StorageTraceAuditAction,
    metadata: StorageTraceAuditSafeMetadata,
) -> anyhow::Result<()> {
    append_audit_event(&state.root, &tenant.tenant_id, event.clone())?;
    if let Err(error) = mirror_audit_event_to_db(state, tenant, &event, action, metadata).await {
        tracing::warn!(
            %error,
            event_id = %event.event_id,
            "Trace Commons DB dual-write audit mirror failed"
        );
    }
    Ok(())
}

async fn append_trace_content_read_audit(
    state: &AppState,
    tenant: &TenantAuth,
    submission_id: Uuid,
    object_ref_id: Option<Uuid>,
    surface: &str,
    purpose: Option<&str>,
) -> anyhow::Result<()> {
    let event = TraceCommonsAuditEvent::trace_content_read(tenant, submission_id, surface, purpose);
    append_audit_event(&state.root, &tenant.tenant_id, event.clone())?;
    if let Err(error) = mirror_audit_event_to_db_with_object_ref(
        state,
        tenant,
        &event,
        StorageTraceAuditAction::Read,
        StorageTraceAuditSafeMetadata::Empty,
        object_ref_id,
    )
    .await
    {
        tracing::warn!(
            %error,
            event_id = %event.event_id,
            "Trace Commons DB dual-write audit mirror failed"
        );
    }
    Ok(())
}

async fn mirror_audit_event_to_db(
    state: &AppState,
    tenant: &TenantAuth,
    event: &TraceCommonsAuditEvent,
    action: StorageTraceAuditAction,
    metadata: StorageTraceAuditSafeMetadata,
) -> anyhow::Result<()> {
    mirror_audit_event_to_db_with_object_ref(state, tenant, event, action, metadata, None).await
}

async fn mirror_audit_event_to_db_with_object_ref(
    state: &AppState,
    tenant: &TenantAuth,
    event: &TraceCommonsAuditEvent,
    action: StorageTraceAuditAction,
    metadata: StorageTraceAuditSafeMetadata,
    object_ref_id: Option<Uuid>,
) -> anyhow::Result<()> {
    let Some(db) = state.db_mirror.as_ref() else {
        return Ok(());
    };
    db.append_trace_audit_event(StorageTraceAuditEventWrite {
        audit_event_id: event.event_id,
        tenant_id: tenant.tenant_id.clone(),
        actor_principal_ref: event
            .actor_principal_ref
            .clone()
            .unwrap_or_else(|| tenant.principal_ref.clone()),
        actor_role: event
            .actor_role
            .unwrap_or(tenant.role)
            .storage_name()
            .to_string(),
        action,
        reason: event.reason.clone(),
        request_id: None,
        submission_id: (event.submission_id != Uuid::nil()).then_some(event.submission_id),
        object_ref_id,
        export_manifest_id: event.export_id,
        decision_inputs_hash: event.decision_inputs_hash.clone(),
        metadata,
    })
    .await
    .context("failed to mirror trace audit event")
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

fn read_all_export_manifests(
    root: &Path,
    tenant_id: &str,
) -> anyhow::Result<Vec<TraceReplayExportManifest>> {
    let tenant_key = tenant_storage_key(tenant_id);
    let exports_dir = root.join("tenants").join(tenant_key).join("exports");
    if !exports_dir.exists() {
        return Ok(Vec::new());
    }

    let mut manifests = Vec::new();
    for entry in std::fs::read_dir(&exports_dir)
        .with_context(|| format!("failed to read exports dir {}", exports_dir.display()))?
    {
        let entry = entry.context("failed to read export dir entry")?;
        if !entry
            .file_type()
            .context("failed to inspect export dir entry")?
            .is_dir()
        {
            continue;
        }
        let manifest_path = entry.path().join("manifest.json");
        if manifest_path.exists() {
            manifests.push(read_export_manifest(&manifest_path)?);
        }
    }
    manifests.sort_by_key(|manifest| manifest.generated_at);
    Ok(manifests)
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

fn benchmark_provenance_path(root: &Path, tenant_id: &str, conversion_id: Uuid) -> PathBuf {
    let tenant_key = tenant_storage_key(tenant_id);
    root.join("tenants")
        .join(tenant_key)
        .join("benchmarks")
        .join(conversion_id.to_string())
        .join("provenance.json")
}

fn ranker_provenance_path(root: &Path, tenant_id: &str, export_id: Uuid) -> PathBuf {
    let tenant_key = tenant_storage_key(tenant_id);
    root.join("tenants")
        .join(tenant_key)
        .join("ranker_exports")
        .join(export_id.to_string())
        .join("provenance.json")
}

fn write_export_provenance(
    path: &Path,
    provenance: &TraceExportProvenanceManifest,
) -> anyhow::Result<()> {
    write_json_file(path, provenance, "trace export provenance manifest")
}

fn read_export_provenance(path: &Path) -> anyhow::Result<TraceExportProvenanceManifest> {
    let body = std::fs::read_to_string(path).with_context(|| {
        format!(
            "failed to read trace export provenance manifest {}",
            path.display()
        )
    })?;
    serde_json::from_str(&body).with_context(|| {
        format!(
            "failed to parse trace export provenance manifest {}",
            path.display()
        )
    })
}

fn export_artifact_dir(root: &Path, tenant_id: &str, export_id: Uuid) -> PathBuf {
    let tenant_key = tenant_storage_key(tenant_id);
    root.join("tenants")
        .join(tenant_key)
        .join("exports")
        .join(export_id.to_string())
}

async fn run_maintenance(
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
    let revocation_reasons = read_all_revocations(&state.root, &tenant.tenant_id)?
        .into_iter()
        .map(|revocation| (revocation.submission_id, revocation.reason))
        .collect::<BTreeMap<_, _>>();
    let mut revoked_submission_ids = revocation_reasons.keys().copied().collect::<BTreeSet<_>>();
    let mut expired_submission_ids = BTreeSet::new();

    let mut records = read_all_submission_records(&state.root, &tenant.tenant_id)?;
    let mut records_marked_revoked = 0usize;
    let mut records_marked_expired = 0usize;
    let now = Utc::now();
    for record in &mut records {
        if record.is_revoked() {
            revoked_submission_ids.insert(record.submission_id);
            continue;
        }
        if record.status == TraceCorpusStatus::Expired {
            expired_submission_ids.insert(record.submission_id);
            continue;
        }
        if revoked_submission_ids.contains(&record.submission_id) {
            records_marked_revoked += 1;
            if !request.dry_run {
                record.status = TraceCorpusStatus::Revoked;
                record.credit_points_final = Some(0.0);
                write_submission_record(&state.root, record)?;
            }
            continue;
        }
        if record.is_expired_at(now) {
            records_marked_expired += 1;
            expired_submission_ids.insert(record.submission_id);
            if !request.dry_run {
                record.status = TraceCorpusStatus::Expired;
                record.credit_points_final = Some(
                    record
                        .credit_points_final
                        .unwrap_or(record.credit_points_pending),
                );
                write_submission_record(&state.root, record)?;
                mirror_expiration_to_db(state, tenant, record.submission_id).await?;
            }
        }
    }

    let mut derived = read_all_derived_records(&state.root, &tenant.tenant_id)?;
    let mut derived_marked_revoked = 0usize;
    let mut derived_marked_expired = 0usize;
    for record in &mut derived {
        if revoked_submission_ids.contains(&record.submission_id)
            && record.status != TraceCorpusStatus::Revoked
        {
            derived_marked_revoked += 1;
            if !request.dry_run {
                record.status = TraceCorpusStatus::Revoked;
                write_derived_record(&state.root, record)?;
            }
        } else if expired_submission_ids.contains(&record.submission_id)
            && !matches!(
                record.status,
                TraceCorpusStatus::Revoked | TraceCorpusStatus::Expired
            )
        {
            derived_marked_expired += 1;
            if !request.dry_run {
                record.status = TraceCorpusStatus::Expired;
                write_derived_record(&state.root, record)?;
            }
        }
    }

    let mut records_marked_purged = 0usize;
    let mut trace_object_files_deleted = 0usize;
    let mut encrypted_artifacts_deleted = 0usize;
    if let Some(purge_cutoff) = request.purge_expired_before {
        for record in &mut records {
            if record.status != TraceCorpusStatus::Expired
                || record
                    .expires_at
                    .is_none_or(|expires_at| expires_at > purge_cutoff)
            {
                continue;
            }
            records_marked_purged += 1;
            if request.dry_run {
                continue;
            }
            let deletion_counts = delete_trace_objects_for_record(state, record)?;
            trace_object_files_deleted += usize::from(deletion_counts.file_deleted);
            encrypted_artifacts_deleted += usize::from(deletion_counts.encrypted_artifact_deleted);
            record.status = TraceCorpusStatus::Purged;
            record.purged_at = Some(now);
            write_submission_record(&state.root, record)?;
            mirror_purge_to_db(state, tenant, record.submission_id).await?;
        }
    }

    let export_cache_files_pruned = if request.prune_export_cache {
        prune_export_cache_files(
            &state.root,
            &tenant.tenant_id,
            &revoked_submission_ids,
            &expired_submission_ids,
            request.max_export_age_hours,
            request.dry_run,
        )?
    } else {
        0
    };
    let export_provenance_invalidated =
        if !revoked_submission_ids.is_empty() || !expired_submission_ids.is_empty() {
            invalidate_export_provenance_for_sources(
                &state.root,
                &tenant.tenant_id,
                &revocation_reasons,
                &expired_submission_ids,
                request.dry_run,
            )?
        } else {
            0
        };
    let db_mirror_backfilled = backfill_db_mirror_from_files(
        state,
        tenant,
        &records,
        &derived,
        request.backfill_db_mirror,
        request.dry_run,
    )
    .await?;
    let vector_entries_indexed =
        index_vector_metadata_from_db(state, tenant, request.index_vectors, request.dry_run)
            .await?;
    let db_reconciliation = reconcile_db_mirror(
        state,
        tenant,
        &records,
        &derived,
        request.reconcile_db_mirror,
    )
    .await?;

    let maintenance_counts = TraceMaintenanceAuditCounts {
        records_marked_revoked,
        records_marked_expired,
        records_marked_purged,
        derived_marked_revoked,
        derived_marked_expired,
        export_cache_files_pruned,
        export_provenance_invalidated,
        trace_object_files_deleted,
        encrypted_artifacts_deleted,
        db_mirror_backfilled,
        vector_entries_indexed,
    };
    let audit_event =
        TraceCommonsAuditEvent::maintenance(tenant, &purpose, request.dry_run, maintenance_counts);
    let audit_event_id = audit_event.event_id;
    append_audit_event_with_db_mirror(
        state,
        tenant,
        audit_event,
        StorageTraceAuditAction::Retain,
        StorageTraceAuditSafeMetadata::Maintenance {
            dry_run: request.dry_run,
            action_counts: maintenance_counts.action_counts(),
        },
    )
    .await?;
    if request.index_vectors {
        append_audit_event_with_db_mirror(
            state,
            tenant,
            TraceCommonsAuditEvent::vector_index(tenant, vector_entries_indexed, request.dry_run),
            StorageTraceAuditAction::VectorIndex,
            StorageTraceAuditSafeMetadata::Maintenance {
                dry_run: request.dry_run,
                action_counts: {
                    let mut counts = BTreeMap::new();
                    counts.insert(
                        "vector_entries_indexed".to_string(),
                        vector_entries_indexed.min(u32::MAX as usize) as u32,
                    );
                    counts
                },
            },
        )
        .await?;
    }

    Ok(TraceMaintenanceResponse {
        tenant_id: tenant.tenant_id.clone(),
        tenant_storage_ref: tenant_storage_ref(&tenant.tenant_id),
        purpose,
        dry_run: request.dry_run,
        audit_event_id,
        revoked_submission_count: revoked_submission_ids.len(),
        expired_submission_count: expired_submission_ids.len(),
        records_marked_revoked,
        records_marked_expired,
        records_marked_purged,
        derived_marked_revoked,
        derived_marked_expired,
        export_cache_files_pruned,
        export_provenance_invalidated,
        trace_object_files_deleted,
        encrypted_artifacts_deleted,
        db_mirror_backfilled,
        vector_entries_indexed,
        db_reconciliation,
    })
}

async fn backfill_db_mirror_from_files(
    state: &AppState,
    tenant: &TenantAuth,
    records: &[TraceCommonsSubmissionRecord],
    derived: &[TraceCommonsDerivedRecord],
    enabled: bool,
    dry_run: bool,
) -> anyhow::Result<usize> {
    if !enabled {
        return Ok(0);
    }
    if state.db_mirror.is_none() && !dry_run {
        anyhow::bail!(
            "Trace Commons DB mirror backfill requested but TRACE_COMMONS_DB_DUAL_WRITE is not configured"
        );
    }
    let db = state.db_mirror.as_ref();
    let derived_by_submission = derived
        .iter()
        .map(|record| (record.submission_id, record))
        .collect::<BTreeMap<_, _>>();
    let mut backfilled = 0usize;
    for record in records {
        if record.status == TraceCorpusStatus::Purged {
            continue;
        }
        let envelope = read_envelope_by_record(state, record)
            .with_context(|| format!("failed to validate envelope {}", record.submission_id))?;
        let derived_record = derived_by_submission
            .get(&record.submission_id)
            .copied()
            .with_context(|| {
                format!(
                    "trace submission {} is missing a derived precheck record",
                    record.submission_id
                )
            })?;
        if dry_run {
            backfilled += 1;
            continue;
        }
        if let Some(db) = db
            && db
                .get_trace_submission(&tenant.tenant_id, record.submission_id)
                .await?
                .is_some()
        {
            continue;
        }
        mirror_submission_to_db(state, tenant, record, derived_record, &envelope).await?;
        if record.is_revoked() {
            mirror_revocation_to_db(state, tenant, record.submission_id, Some(record)).await?;
        }
        backfilled += 1;
    }
    Ok(backfilled)
}

async fn index_vector_metadata_from_db(
    state: &AppState,
    tenant: &TenantAuth,
    enabled: bool,
    dry_run: bool,
) -> anyhow::Result<usize> {
    if !enabled {
        return Ok(0);
    }
    let db = state
        .db_mirror
        .as_ref()
        .context("Trace Commons vector indexing requires TRACE_COMMONS_DB_DUAL_WRITE")?;
    let submissions = db
        .list_trace_submissions(&tenant.tenant_id)
        .await
        .context("failed to list trace submissions for vector indexing")?;
    let accepted_submission_ids = submissions
        .into_iter()
        .filter(|record| record.status == StorageTraceCorpusStatus::Accepted)
        .filter(|record| record.revoked_at.is_none() && record.purged_at.is_none())
        .map(|record| record.submission_id)
        .collect::<BTreeSet<_>>();
    let derived_records = db
        .list_trace_derived_records(&tenant.tenant_id)
        .await
        .context("failed to list trace derived records for vector indexing")?;
    let active_vector_ids = db
        .list_trace_vector_entries(&tenant.tenant_id)
        .await
        .context("failed to list trace vector entries for vector indexing")?
        .into_iter()
        .filter(|entry| entry.status == StorageTraceVectorEntryStatus::Active)
        .map(|entry| entry.vector_entry_id)
        .collect::<BTreeSet<_>>();

    let eligible = derived_records
        .iter()
        .filter(|record| record.status == StorageTraceDerivedStatus::Current)
        .filter(|record| accepted_submission_ids.contains(&record.submission_id))
        .filter(|record| record.canonical_summary_hash.is_some())
        .collect::<Vec<_>>();

    let mut indexed = 0usize;
    for record in &eligible {
        let Some(source_hash) = record.canonical_summary_hash.clone() else {
            continue;
        };
        let vector_entry_id = deterministic_vector_entry_uuid(
            &tenant.tenant_id,
            record.submission_id,
            record.derived_id,
            &source_hash,
        );
        if active_vector_ids.contains(&vector_entry_id) {
            continue;
        }
        indexed += 1;
        if dry_run {
            continue;
        }
        let nearest_trace_ids = eligible
            .iter()
            .filter(|candidate| candidate.submission_id != record.submission_id)
            .filter(|candidate| candidate.canonical_summary_hash.as_ref() == Some(&source_hash))
            .map(|candidate| candidate.trace_id.to_string())
            .take(5)
            .collect::<Vec<_>>();
        let duplicate_score = if nearest_trace_ids.is_empty() {
            record.duplicate_score.unwrap_or_default()
        } else {
            1.0
        };
        let novelty_score = if nearest_trace_ids.is_empty() {
            record.novelty_score.unwrap_or(0.5)
        } else {
            0.1
        };
        db.upsert_trace_vector_entry(StorageTraceVectorEntryWrite {
            tenant_id: tenant.tenant_id.clone(),
            submission_id: record.submission_id,
            derived_id: record.derived_id,
            vector_entry_id,
            vector_store: "trace_commons_metadata_precheck".to_string(),
            embedding_model: "canonical-summary-hash-v1".to_string(),
            embedding_dimension: 1,
            embedding_version: "trace_commons_vector_metadata_v1".to_string(),
            source_projection: StorageTraceVectorEntrySourceProjection::CanonicalSummary,
            source_hash: source_hash.clone(),
            status: StorageTraceVectorEntryStatus::Active,
            nearest_trace_ids,
            cluster_id: record
                .cluster_id
                .clone()
                .or_else(|| Some(format!("summary:{}", hash_fragment(&source_hash, 16)))),
            duplicate_score: Some(duplicate_score),
            novelty_score: Some(novelty_score),
            indexed_at: Some(Utc::now()),
            invalidated_at: None,
            deleted_at: None,
        })
        .await
        .context("failed to upsert trace vector entry")?;
    }
    Ok(indexed)
}

async fn reconcile_db_mirror(
    state: &AppState,
    tenant: &TenantAuth,
    file_records: &[TraceCommonsSubmissionRecord],
    file_derived: &[TraceCommonsDerivedRecord],
    enabled: bool,
) -> anyhow::Result<Option<TraceDbReconciliationReport>> {
    if !enabled {
        return Ok(None);
    }
    let db = state
        .db_mirror
        .as_ref()
        .context("Trace Commons DB reconciliation requires TRACE_COMMONS_DB_DUAL_WRITE")?;
    let db_records = db
        .list_trace_submissions(&tenant.tenant_id)
        .await
        .context("failed to list trace submissions for DB reconciliation")?;
    let db_derived = db
        .list_trace_derived_records(&tenant.tenant_id)
        .await
        .context("failed to list trace derived records for DB reconciliation")?;
    let db_vectors = db
        .list_trace_vector_entries(&tenant.tenant_id)
        .await
        .context("failed to list trace vector entries for DB reconciliation")?;
    let db_credit_events = db
        .list_trace_credit_events(&tenant.tenant_id)
        .await
        .context("failed to list trace credit events for DB reconciliation")?;
    let db_audit_events = db
        .list_trace_audit_events(&tenant.tenant_id)
        .await
        .context("failed to list trace audit events for DB reconciliation")?;
    let db_export_manifests = db
        .list_trace_export_manifests(&tenant.tenant_id)
        .await
        .context("failed to list trace export manifests for DB reconciliation")?;
    let db_tombstones = db
        .list_trace_tombstones(&tenant.tenant_id)
        .await
        .context("failed to list trace tombstones for DB reconciliation")?;
    let mut db_export_manifest_item_count = 0usize;
    for manifest in &db_export_manifests {
        db_export_manifest_item_count += db
            .list_trace_export_manifest_items(&tenant.tenant_id, manifest.export_manifest_id)
            .await
            .with_context(|| {
                format!(
                    "failed to list trace export manifest items for manifest {}",
                    manifest.export_manifest_id
                )
            })?
            .len();
    }
    let file_credit_events = read_all_credit_events(&state.root, &tenant.tenant_id)?;
    let file_audit_events = read_all_audit_events(&state.root, &tenant.tenant_id)?;
    let file_replay_export_manifests = read_all_export_manifests(&state.root, &tenant.tenant_id)?;
    let file_revocations = read_all_revocations(&state.root, &tenant.tenant_id)?;
    let mut db_object_ref_count = 0usize;
    let mut accepted_without_active_envelope_object_ref = Vec::new();
    for record in &db_records {
        let object_refs = db
            .list_trace_object_refs(&tenant.tenant_id, record.submission_id)
            .await
            .with_context(|| {
                format!(
                    "failed to list trace object refs for submission {}",
                    record.submission_id
                )
            })?;
        db_object_ref_count += object_refs.len();
        if record.status == StorageTraceCorpusStatus::Accepted
            && record.revoked_at.is_none()
            && record.purged_at.is_none()
            && db
                .get_latest_active_trace_object_ref(
                    &tenant.tenant_id,
                    record.submission_id,
                    StorageTraceObjectArtifactKind::SubmittedEnvelope,
                )
                .await
                .with_context(|| {
                    format!(
                        "failed to get latest active trace object ref for submission {}",
                        record.submission_id
                    )
                })?
                .is_none()
        {
            accepted_without_active_envelope_object_ref.push(record.submission_id);
        }
    }

    let file_by_submission = file_records
        .iter()
        .map(|record| (record.submission_id, record))
        .collect::<BTreeMap<_, _>>();
    let db_by_submission = db_records
        .iter()
        .map(|record| (record.submission_id, record))
        .collect::<BTreeMap<_, _>>();
    let file_derived_by_submission = file_derived
        .iter()
        .map(|record| (record.submission_id, record))
        .collect::<BTreeMap<_, _>>();
    let db_derived_by_submission = db_derived
        .iter()
        .map(|record| (record.submission_id, record))
        .collect::<BTreeMap<_, _>>();

    let mut missing_submission_ids_in_db = Vec::new();
    let mut status_mismatches = Vec::new();
    for (submission_id, file_record) in &file_by_submission {
        let Some(db_record) = db_by_submission.get(submission_id) else {
            missing_submission_ids_in_db.push(*submission_id);
            continue;
        };
        let file_status = storage_corpus_status(file_record.status);
        if db_record.status != file_status {
            status_mismatches.push(TraceDbStatusMismatch {
                submission_id: *submission_id,
                file_status,
                db_status: db_record.status,
            });
        }
    }

    let missing_submission_ids_in_files = db_by_submission
        .keys()
        .filter(|submission_id| !file_by_submission.contains_key(submission_id))
        .copied()
        .collect::<Vec<_>>();
    let missing_derived_submission_ids_in_db = file_derived_by_submission
        .keys()
        .filter(|submission_id| !db_derived_by_submission.contains_key(submission_id))
        .copied()
        .collect::<Vec<_>>();

    let accepted_submission_ids = db_records
        .iter()
        .filter(|record| record.status == StorageTraceCorpusStatus::Accepted)
        .filter(|record| record.revoked_at.is_none() && record.purged_at.is_none())
        .map(|record| record.submission_id)
        .collect::<BTreeSet<_>>();
    let current_derived_ids = db_derived
        .iter()
        .filter(|record| record.status == StorageTraceDerivedStatus::Current)
        .map(|record| (record.submission_id, record.derived_id))
        .collect::<BTreeSet<_>>();
    let active_vector_entries = db_vectors
        .iter()
        .filter(|entry| entry.status == StorageTraceVectorEntryStatus::Active)
        .count();
    let invalid_active_vector_entries = db_vectors
        .iter()
        .filter(|entry| entry.status == StorageTraceVectorEntryStatus::Active)
        .filter(|entry| {
            !accepted_submission_ids.contains(&entry.submission_id)
                || !current_derived_ids.contains(&(entry.submission_id, entry.derived_id))
        })
        .count();

    let file_credit_view =
        contributor_credit_view_from_file_records(tenant, file_records, &file_credit_events);
    let db_credit_view = read_contributor_credit_view_from_db(state, tenant).await?;
    let file_metadata_view = metadata_view_from_file_records(file_records, file_derived);
    let db_metadata_view = read_reviewer_metadata_view_from_db(state, tenant).await?;
    let file_analytics = TraceCommonsAnalyticsResponse::from_records(
        tenant.tenant_id.clone(),
        file_metadata_view.records.clone(),
        file_metadata_view.derived.clone(),
    );
    let db_analytics = TraceCommonsAnalyticsResponse::from_records(
        tenant.tenant_id.clone(),
        db_metadata_view.records.clone(),
        db_metadata_view.derived.clone(),
    );
    let db_audit_events_for_reader = read_audit_events_from_db(state, tenant).await?;
    let file_export_manifest_projection = export_manifest_reader_projection(
        file_replay_export_manifests
            .iter()
            .cloned()
            .map(TraceExportManifestSummary::from_replay_manifest)
            .collect(),
    );
    let db_export_manifest_projection = export_manifest_reader_projection(
        db_export_manifests
            .iter()
            .cloned()
            .map(TraceExportManifestSummary::from_storage_record)
            .filter(TraceExportManifestSummary::is_replay_dataset_manifest)
            .collect(),
    );

    let mut db_reader_parity_failures = Vec::new();
    let contributor_credit_reader_parity_ok = {
        let file_submissions = submission_reader_projection(&file_credit_view.records);
        let db_submissions = submission_reader_projection(&db_credit_view.records);
        let file_events = credit_event_reader_projection(&file_credit_view.credit_events);
        let db_events = credit_event_reader_projection(&db_credit_view.credit_events);
        record_reader_parity(
            &mut db_reader_parity_failures,
            "contributor_credit",
            file_submissions == db_submissions && file_events == db_events,
            format!(
                "file_submissions={} db_submissions={} file_events={} db_events={}",
                file_submissions.len(),
                db_submissions.len(),
                file_events.len(),
                db_events.len()
            ),
        )
    };
    let reviewer_metadata_reader_parity_ok = {
        let file_submissions = submission_reader_projection(&file_metadata_view.records);
        let db_submissions = submission_reader_projection(&db_metadata_view.records);
        let file_derived = derived_reader_projection(&file_metadata_view.derived);
        let db_derived = derived_reader_projection(&db_metadata_view.derived);
        record_reader_parity(
            &mut db_reader_parity_failures,
            "reviewer_metadata",
            file_submissions == db_submissions && file_derived == db_derived,
            format!(
                "file_submissions={} db_submissions={} file_derived={} db_derived={}",
                file_submissions.len(),
                db_submissions.len(),
                file_derived.len(),
                db_derived.len()
            ),
        )
    };
    let analytics_reader_parity_ok = {
        let file_projection = analytics_reader_projection(file_analytics);
        let db_projection = analytics_reader_projection(db_analytics);
        record_reader_parity(
            &mut db_reader_parity_failures,
            "analytics",
            file_projection == db_projection,
            format!(
                "file_submissions={} db_submissions={} file_duplicate_groups={} db_duplicate_groups={}",
                file_projection.submissions_total,
                db_projection.submissions_total,
                file_projection.duplicate_groups,
                db_projection.duplicate_groups
            ),
        )
    };
    let audit_reader_parity_ok = {
        record_reader_parity(
            &mut db_reader_parity_failures,
            "audit",
            file_audit_events.len() == db_audit_events_for_reader.len(),
            format!(
                "file_events={} db_events={}",
                file_audit_events.len(),
                db_audit_events_for_reader.len()
            ),
        )
    };
    let replay_export_manifest_reader_parity_ok = record_reader_parity(
        &mut db_reader_parity_failures,
        "replay_export_manifests",
        file_export_manifest_projection == db_export_manifest_projection,
        format!(
            "file_manifests={} db_manifests={}",
            file_export_manifest_projection.len(),
            db_export_manifest_projection.len()
        ),
    );

    Ok(Some(TraceDbReconciliationReport {
        file_submission_count: file_records.len(),
        db_submission_count: db_records.len(),
        missing_submission_ids_in_db,
        missing_submission_ids_in_files,
        status_mismatches,
        file_derived_count: file_derived.len(),
        db_derived_count: db_derived.len(),
        missing_derived_submission_ids_in_db,
        file_credit_event_count: file_credit_events.len(),
        db_credit_event_count: db_credit_events.len(),
        file_audit_event_count: file_audit_events.len(),
        db_audit_event_count: db_audit_events.len(),
        file_replay_export_manifest_count: file_replay_export_manifests.len(),
        db_export_manifest_count: db_export_manifests.len(),
        db_export_manifest_item_count,
        file_revocation_tombstone_count: file_revocations.len(),
        db_tombstone_count: db_tombstones.len(),
        db_object_ref_count,
        accepted_without_active_envelope_object_ref,
        contributor_credit_reader_parity_ok,
        reviewer_metadata_reader_parity_ok,
        analytics_reader_parity_ok,
        audit_reader_parity_ok,
        replay_export_manifest_reader_parity_ok,
        db_reader_parity_failures,
        active_vector_entries,
        invalid_active_vector_entries,
    }))
}

fn deterministic_vector_entry_uuid(
    tenant_id: &str,
    submission_id: Uuid,
    derived_id: Uuid,
    source_hash: &str,
) -> Uuid {
    let input = format!(
        "ironclaw.trace_commons.vector:{tenant_id}:{submission_id}:{derived_id}:{source_hash}"
    );
    Uuid::new_v5(&Uuid::NAMESPACE_URL, input.as_bytes())
}

#[derive(Debug, Clone, Copy)]
struct TraceMaintenanceAuditCounts {
    records_marked_revoked: usize,
    records_marked_expired: usize,
    records_marked_purged: usize,
    derived_marked_revoked: usize,
    derived_marked_expired: usize,
    export_cache_files_pruned: usize,
    export_provenance_invalidated: usize,
    trace_object_files_deleted: usize,
    encrypted_artifacts_deleted: usize,
    db_mirror_backfilled: usize,
    vector_entries_indexed: usize,
}

impl TraceMaintenanceAuditCounts {
    fn action_counts(self) -> BTreeMap<String, u32> {
        let mut counts = BTreeMap::new();
        counts.insert(
            "records_marked_revoked".to_string(),
            self.records_marked_revoked.min(u32::MAX as usize) as u32,
        );
        counts.insert(
            "records_marked_expired".to_string(),
            self.records_marked_expired.min(u32::MAX as usize) as u32,
        );
        counts.insert(
            "records_marked_purged".to_string(),
            self.records_marked_purged.min(u32::MAX as usize) as u32,
        );
        counts.insert(
            "derived_marked_revoked".to_string(),
            self.derived_marked_revoked.min(u32::MAX as usize) as u32,
        );
        counts.insert(
            "derived_marked_expired".to_string(),
            self.derived_marked_expired.min(u32::MAX as usize) as u32,
        );
        counts.insert(
            "export_cache_files_pruned".to_string(),
            self.export_cache_files_pruned.min(u32::MAX as usize) as u32,
        );
        counts.insert(
            "export_provenance_invalidated".to_string(),
            self.export_provenance_invalidated.min(u32::MAX as usize) as u32,
        );
        counts.insert(
            "trace_object_files_deleted".to_string(),
            self.trace_object_files_deleted.min(u32::MAX as usize) as u32,
        );
        counts.insert(
            "encrypted_artifacts_deleted".to_string(),
            self.encrypted_artifacts_deleted.min(u32::MAX as usize) as u32,
        );
        counts.insert(
            "db_mirror_backfilled".to_string(),
            self.db_mirror_backfilled.min(u32::MAX as usize) as u32,
        );
        counts.insert(
            "vector_entries_indexed".to_string(),
            self.vector_entries_indexed.min(u32::MAX as usize) as u32,
        );
        counts
    }
}

fn prune_export_cache_files(
    root: &Path,
    tenant_id: &str,
    revoked_submission_ids: &BTreeSet<Uuid>,
    expired_submission_ids: &BTreeSet<Uuid>,
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
        let contains_expired_source = manifest
            .source_submission_ids
            .iter()
            .any(|submission_id| expired_submission_ids.contains(submission_id));
        let expired = max_export_age_hours
            .filter(|hours| *hours >= 0)
            .is_some_and(|hours| {
                manifest.generated_at <= Utc::now() - chrono::Duration::hours(hours)
            });
        if !contains_revoked_source && !contains_expired_source && !expired {
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
            } else if contains_expired_source {
                "retention_expired_source".to_string()
            } else {
                "export_age_expired".to_string()
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

fn invalidate_export_provenance_for_source(
    root: &Path,
    tenant_id: &str,
    submission_id: Uuid,
    reason: &str,
) -> anyhow::Result<usize> {
    let mut revoked_sources = BTreeMap::new();
    revoked_sources.insert(submission_id, reason.to_string());
    invalidate_export_provenance_for_sources(
        root,
        tenant_id,
        &revoked_sources,
        &BTreeSet::new(),
        false,
    )
}

fn invalidate_export_provenance_for_sources(
    root: &Path,
    tenant_id: &str,
    revoked_submission_reasons: &BTreeMap<Uuid, String>,
    expired_submission_ids: &BTreeSet<Uuid>,
    dry_run: bool,
) -> anyhow::Result<usize> {
    let provenance_paths = read_export_provenance_paths(root, tenant_id)?;
    let mut invalidated = 0usize;
    for path in provenance_paths {
        let mut provenance = read_export_provenance(&path)?;
        if provenance.invalidated_at.is_some() {
            continue;
        }
        let reason = provenance
            .source_submission_ids
            .iter()
            .find_map(|submission_id| {
                revoked_submission_reasons
                    .get(submission_id)
                    .cloned()
                    .or_else(|| {
                        expired_submission_ids
                            .contains(submission_id)
                            .then(|| "retention_expired_source".to_string())
                    })
            });
        let Some(reason) = reason else {
            continue;
        };
        invalidated += 1;
        if dry_run {
            continue;
        }
        provenance.invalidated_at = Some(Utc::now());
        provenance.invalidation_reason = Some(reason);
        write_export_provenance(&path, &provenance)?;
    }
    Ok(invalidated)
}

fn read_export_provenance_paths(root: &Path, tenant_id: &str) -> anyhow::Result<Vec<PathBuf>> {
    let tenant_key = tenant_storage_key(tenant_id);
    let tenant_dir = root.join("tenants").join(tenant_key);
    let mut paths = Vec::new();
    for child_dir_name in ["benchmarks", "ranker_exports"] {
        let child_dir = tenant_dir.join(child_dir_name);
        if !child_dir.exists() {
            continue;
        }
        for entry in std::fs::read_dir(&child_dir)
            .with_context(|| format!("failed to read provenance dir {}", child_dir.display()))?
        {
            let entry = entry.context("failed to read provenance dir entry")?;
            if !entry
                .file_type()
                .context("failed to inspect provenance dir entry")?
                .is_dir()
            {
                continue;
            }
            let provenance_path = entry.path().join("provenance.json");
            if provenance_path.exists() {
                paths.push(provenance_path);
            }
        }
    }
    paths.sort();
    Ok(paths)
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

fn hash_fragment(hash: &str, len: usize) -> String {
    hash.strip_prefix("sha256:")
        .unwrap_or(hash)
        .chars()
        .take(len)
        .collect()
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
    Expired,
    Purged,
}

impl TraceCorpusStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Quarantined => "quarantined",
            Self::Rejected => "rejected",
            Self::Revoked => "revoked",
            Self::Expired => "expired",
            Self::Purged => "purged",
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
    #[serde(default = "default_retention_policy_id")]
    retention_policy_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    expires_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    purged_at: Option<DateTime<Utc>>,
    object_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    artifact_receipt: Option<EncryptedTraceArtifactReceipt>,
}

impl TraceCommonsSubmissionRecord {
    fn is_revoked(&self) -> bool {
        self.status == TraceCorpusStatus::Revoked
    }

    fn is_terminal(&self) -> bool {
        matches!(
            self.status,
            TraceCorpusStatus::Revoked | TraceCorpusStatus::Expired | TraceCorpusStatus::Purged
        )
    }

    fn is_expired_at(&self, now: DateTime<Utc>) -> bool {
        self.expires_at.is_some_and(|expires_at| expires_at <= now)
            && !matches!(
                self.status,
                TraceCorpusStatus::Revoked | TraceCorpusStatus::Expired | TraceCorpusStatus::Purged
            )
    }

    fn is_export_eligible(&self) -> bool {
        self.status == TraceCorpusStatus::Accepted && !self.is_revoked()
    }

    fn is_benchmark_eligible(&self) -> bool {
        self.is_export_eligible()
    }
}

fn default_retention_policy_id() -> String {
    "private_corpus_revocable".to_string()
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    derived_id: Option<Uuid>,
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

#[derive(Debug, Serialize)]
struct TraceExportManifestSummary {
    tenant_id: String,
    tenant_storage_ref: String,
    export_manifest_id: Uuid,
    artifact_kind: StorageTraceObjectArtifactKind,
    purpose_code: Option<String>,
    audit_event_id: Option<Uuid>,
    source_submission_ids: Vec<Uuid>,
    source_submission_ids_hash: String,
    item_count: u32,
    generated_at: DateTime<Utc>,
    invalidated_at: Option<DateTime<Utc>>,
    deleted_at: Option<DateTime<Utc>>,
}

impl TraceExportManifestSummary {
    fn from_storage_record(record: StorageTraceExportManifestRecord) -> Self {
        Self {
            tenant_storage_ref: tenant_storage_ref(&record.tenant_id),
            tenant_id: record.tenant_id,
            export_manifest_id: record.export_manifest_id,
            artifact_kind: record.artifact_kind,
            purpose_code: record.purpose_code,
            audit_event_id: record.audit_event_id,
            source_submission_ids: record.source_submission_ids,
            source_submission_ids_hash: record.source_submission_ids_hash,
            item_count: record.item_count,
            generated_at: record.generated_at,
            invalidated_at: record.invalidated_at,
            deleted_at: record.deleted_at,
        }
    }

    fn from_replay_manifest(manifest: TraceReplayExportManifest) -> Self {
        Self {
            tenant_storage_ref: manifest.tenant_storage_ref,
            tenant_id: manifest.tenant_id,
            export_manifest_id: manifest.export_id,
            artifact_kind: StorageTraceObjectArtifactKind::ExportArtifact,
            purpose_code: Some(manifest.purpose),
            audit_event_id: Some(manifest.audit_event_id),
            item_count: manifest.source_submission_ids.len().min(u32::MAX as usize) as u32,
            source_submission_ids: manifest.source_submission_ids,
            source_submission_ids_hash: manifest.source_submission_ids_hash,
            generated_at: manifest.generated_at,
            invalidated_at: None,
            deleted_at: None,
        }
    }

    fn is_replay_dataset_manifest(&self) -> bool {
        self.artifact_kind == StorageTraceObjectArtifactKind::ExportArtifact
            && !matches!(
                self.purpose_code.as_deref(),
                Some("ranker_training_candidates_export" | "ranker_training_pairs_export")
            )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TraceReplayExportManifest {
    tenant_id: String,
    tenant_storage_ref: String,
    export_id: Uuid,
    purpose: String,
    filters: TraceReplayExportFilters,
    source_submission_ids: Vec<Uuid>,
    source_submission_ids_hash: String,
    consent_scopes: Vec<ConsentScope>,
    generated_at: DateTime<Utc>,
    audit_event_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TraceExportProvenanceManifest {
    tenant_id: String,
    tenant_storage_ref: String,
    export_id: Uuid,
    audit_event_id: Uuid,
    export_kind: TraceExportProvenanceKind,
    purpose: String,
    source_submission_ids: Vec<Uuid>,
    source_submission_ids_hash: String,
    generated_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    invalidated_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    invalidation_reason: Option<String>,
}

impl TraceExportProvenanceManifest {
    fn new(
        tenant_id: &str,
        export_id: Uuid,
        audit_event_id: Uuid,
        export_kind: TraceExportProvenanceKind,
        purpose: String,
        source_submission_ids: Vec<Uuid>,
        source_submission_ids_hash: String,
    ) -> Self {
        Self {
            tenant_id: tenant_id.to_string(),
            tenant_storage_ref: tenant_storage_ref(tenant_id),
            export_id,
            audit_event_id,
            export_kind,
            purpose,
            source_submission_ids,
            source_submission_ids_hash,
            generated_at: Utc::now(),
            invalidated_at: None,
            invalidation_reason: None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum TraceExportProvenanceKind {
    BenchmarkConversion,
    RankerTrainingCandidates,
    RankerTrainingPairs,
}

impl TraceReplayExportManifest {
    fn from_items(
        tenant_id: &str,
        export_id: Uuid,
        audit_event_id: Uuid,
        purpose: String,
        filters: TraceReplayExportFilters,
        items: &[TraceReplayDatasetItem],
        source_submission_ids_hash: String,
    ) -> Self {
        Self {
            tenant_id: tenant_id.to_string(),
            tenant_storage_ref: tenant_storage_ref(tenant_id),
            export_id,
            purpose,
            filters,
            source_submission_ids: items.iter().map(|item| item.submission_id).collect(),
            source_submission_ids_hash,
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
    #[serde(skip)]
    source_status_at_export: TraceCorpusStatus,
    #[serde(skip)]
    source_hash_at_export: String,
    #[serde(skip)]
    object_ref_id: Option<Uuid>,
}

impl TraceReplayDatasetItem {
    fn from_record(
        record: &TraceCommonsSubmissionRecord,
        derived: Option<&TraceCommonsDerivedRecord>,
        envelope: &TraceContributionEnvelope,
        object_ref_id: Option<Uuid>,
    ) -> Self {
        let canonical_summary_hash = derived.map(|record| record.canonical_summary_hash.clone());
        let source_hash_at_export = canonical_summary_hash
            .clone()
            .unwrap_or_else(|| fallback_replay_source_hash(record, envelope));
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
            canonical_summary_hash,
            canonical_summary: derived.map(|record| record.canonical_summary.clone()),
            coverage_tags: derived
                .map(|record| record.coverage_tags.clone())
                .unwrap_or_default(),
            submission_score: record.submission_score,
            source_status_at_export: record.status,
            source_hash_at_export,
            object_ref_id,
        }
    }
}

fn fallback_replay_source_hash(
    record: &TraceCommonsSubmissionRecord,
    envelope: &TraceContributionEnvelope,
) -> String {
    sha256_prefixed(&format!(
        "trace_replay_source:{}:{}:{}:{}",
        record.tenant_id, record.submission_id, record.trace_id, envelope.schema_version
    ))
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
    source_submission_ids_hash: String,
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
    derived_id: Uuid,
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
            derived_id: derived
                .derived_id
                .unwrap_or_else(|| deterministic_trace_uuid("derived-precheck", submission)),
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
    source_item_list_hash: String,
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
    source_item_list_hash: String,
    pairs: Vec<TraceRankerTrainingPair>,
}

#[derive(Debug, Clone, Serialize)]
struct TraceRankerTrainingCandidate {
    submission_id: Uuid,
    trace_id: Uuid,
    derived_id: Uuid,
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
            derived_id: derived
                .derived_id
                .unwrap_or_else(|| deterministic_trace_uuid("derived-precheck", submission)),
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
            TraceCorpusStatus::Rejected
            | TraceCorpusStatus::Revoked
            | TraceCorpusStatus::Expired
            | TraceCorpusStatus::Purged => Self::NeedsReview,
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
    expired_submission_count: usize,
    records_marked_revoked: usize,
    records_marked_expired: usize,
    records_marked_purged: usize,
    derived_marked_revoked: usize,
    derived_marked_expired: usize,
    export_cache_files_pruned: usize,
    export_provenance_invalidated: usize,
    trace_object_files_deleted: usize,
    encrypted_artifacts_deleted: usize,
    db_mirror_backfilled: usize,
    vector_entries_indexed: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    db_reconciliation: Option<TraceDbReconciliationReport>,
}

#[derive(Debug, Serialize)]
struct TraceDbReconciliationReport {
    file_submission_count: usize,
    db_submission_count: usize,
    missing_submission_ids_in_db: Vec<Uuid>,
    missing_submission_ids_in_files: Vec<Uuid>,
    status_mismatches: Vec<TraceDbStatusMismatch>,
    file_derived_count: usize,
    db_derived_count: usize,
    missing_derived_submission_ids_in_db: Vec<Uuid>,
    file_credit_event_count: usize,
    db_credit_event_count: usize,
    file_audit_event_count: usize,
    db_audit_event_count: usize,
    file_replay_export_manifest_count: usize,
    db_export_manifest_count: usize,
    db_export_manifest_item_count: usize,
    file_revocation_tombstone_count: usize,
    db_tombstone_count: usize,
    db_object_ref_count: usize,
    accepted_without_active_envelope_object_ref: Vec<Uuid>,
    contributor_credit_reader_parity_ok: bool,
    reviewer_metadata_reader_parity_ok: bool,
    analytics_reader_parity_ok: bool,
    audit_reader_parity_ok: bool,
    replay_export_manifest_reader_parity_ok: bool,
    db_reader_parity_failures: Vec<String>,
    active_vector_entries: usize,
    invalid_active_vector_entries: usize,
}

#[derive(Debug, Serialize)]
struct TraceDbStatusMismatch {
    submission_id: Uuid,
    file_status: StorageTraceCorpusStatus,
    db_status: StorageTraceCorpusStatus,
}

#[derive(Debug, PartialEq, Eq)]
struct TraceReaderSubmissionProjection {
    trace_id: Uuid,
    status: TraceCorpusStatus,
    privacy_risk: ResidualPiiRisk,
    auth_principal_ref: String,
    submitted_tenant_scope_ref: Option<String>,
    contributor_pseudonym: Option<String>,
    submission_score_bits: u32,
    credit_points_pending_bits: u32,
    credit_points_final_bits: Option<u32>,
    consent_scopes: Vec<ConsentScope>,
    redaction_counts: BTreeMap<String, u32>,
    retention_policy_id: String,
    expires_at_millis: Option<i64>,
    purged_at_millis: Option<i64>,
}

#[derive(Debug, PartialEq, Eq)]
struct TraceReaderDerivedProjection {
    trace_id: Uuid,
    status: TraceCorpusStatus,
    privacy_risk: ResidualPiiRisk,
    task_success: String,
    canonical_summary_hash: String,
    summary_model: String,
    event_count: usize,
    tool_sequence: Vec<String>,
    tool_categories: Vec<String>,
    coverage_tags: Vec<String>,
    duplicate_score_bits: u32,
    novelty_score_bits: u32,
}

#[derive(Debug, PartialEq, Eq)]
struct TraceReaderCreditEventProjection {
    submission_id: Uuid,
    trace_id: Uuid,
    event_type: TraceCreditLedgerEventType,
    credit_points_delta_bits: u32,
    reason: Option<String>,
    external_ref: Option<String>,
    actor_role: TokenRole,
    actor_principal_ref: String,
}

#[derive(Debug, PartialEq, Eq)]
struct TraceReaderAnalyticsProjection {
    submissions_total: usize,
    by_status: BTreeMap<String, usize>,
    by_privacy_risk: BTreeMap<String, usize>,
    by_task_success: BTreeMap<String, usize>,
    by_tool: BTreeMap<String, usize>,
    by_tool_category: BTreeMap<String, usize>,
    coverage_tags: BTreeMap<String, usize>,
    duplicate_groups: usize,
    average_novelty_score_bits: u32,
}

#[derive(Debug, PartialEq, Eq)]
struct TraceReaderExportManifestProjection {
    artifact_kind: StorageTraceObjectArtifactKind,
    purpose_code: Option<String>,
    audit_event_id: Option<Uuid>,
    source_submission_ids: Vec<Uuid>,
    source_submission_ids_hash: String,
    item_count: u32,
    generated_at_millis: i64,
    invalidated_at_millis: Option<i64>,
    deleted_at_millis: Option<i64>,
}

fn contributor_credit_view_from_file_records(
    tenant: &TenantAuth,
    records: &[TraceCommonsSubmissionRecord],
    credit_events: &[TraceCommonsCreditLedgerRecord],
) -> TraceContributorCreditView {
    let records = visible_submission_records(tenant, records.to_vec());
    let credit_events = eligible_credit_events_for_records(
        &records,
        visible_credit_events(tenant, credit_events.to_vec()),
    );
    TraceContributorCreditView {
        records,
        credit_events,
    }
}

fn metadata_view_from_file_records(
    records: &[TraceCommonsSubmissionRecord],
    derived: &[TraceCommonsDerivedRecord],
) -> TraceCommonsMetadataView {
    TraceCommonsMetadataView {
        records: records.to_vec(),
        derived: derived.to_vec(),
    }
}

fn timestamp_millis(timestamp: DateTime<Utc>) -> i64 {
    timestamp.timestamp_millis()
}

fn optional_timestamp_millis(timestamp: Option<DateTime<Utc>>) -> Option<i64> {
    timestamp.map(timestamp_millis)
}

fn submission_reader_projection(
    records: &[TraceCommonsSubmissionRecord],
) -> BTreeMap<Uuid, TraceReaderSubmissionProjection> {
    records
        .iter()
        .map(|record| {
            (
                record.submission_id,
                TraceReaderSubmissionProjection {
                    trace_id: record.trace_id,
                    status: record.status,
                    privacy_risk: record.privacy_risk,
                    auth_principal_ref: record.auth_principal_ref.clone(),
                    submitted_tenant_scope_ref: record.submitted_tenant_scope_ref.clone(),
                    contributor_pseudonym: record.contributor_pseudonym.clone(),
                    submission_score_bits: record.submission_score.to_bits(),
                    credit_points_pending_bits: record.credit_points_pending.to_bits(),
                    credit_points_final_bits: record.credit_points_final.map(f32::to_bits),
                    consent_scopes: record.consent_scopes.clone(),
                    redaction_counts: record.redaction_counts.clone(),
                    retention_policy_id: record.retention_policy_id.clone(),
                    expires_at_millis: optional_timestamp_millis(record.expires_at),
                    purged_at_millis: optional_timestamp_millis(record.purged_at),
                },
            )
        })
        .collect()
}

fn derived_reader_projection(
    records: &[TraceCommonsDerivedRecord],
) -> BTreeMap<Uuid, TraceReaderDerivedProjection> {
    records
        .iter()
        .map(|record| {
            (
                record.submission_id,
                TraceReaderDerivedProjection {
                    trace_id: record.trace_id,
                    status: record.status,
                    privacy_risk: record.privacy_risk,
                    task_success: record.task_success.clone(),
                    canonical_summary_hash: record.canonical_summary_hash.clone(),
                    summary_model: record.summary_model.clone(),
                    event_count: record.event_count,
                    tool_sequence: record.tool_sequence.clone(),
                    tool_categories: record.tool_categories.clone(),
                    coverage_tags: record.coverage_tags.clone(),
                    duplicate_score_bits: record.duplicate_score.to_bits(),
                    novelty_score_bits: record.novelty_score.to_bits(),
                },
            )
        })
        .collect()
}

fn credit_event_reader_projection(
    events: &[TraceCommonsCreditLedgerRecord],
) -> BTreeMap<Uuid, TraceReaderCreditEventProjection> {
    events
        .iter()
        .map(|event| {
            (
                event.event_id,
                TraceReaderCreditEventProjection {
                    submission_id: event.submission_id,
                    trace_id: event.trace_id,
                    event_type: event.event_type,
                    credit_points_delta_bits: event.credit_points_delta.to_bits(),
                    reason: event.reason.clone(),
                    external_ref: event.external_ref.clone(),
                    actor_role: event.actor_role,
                    actor_principal_ref: event.actor_principal_ref.clone(),
                },
            )
        })
        .collect()
}

fn analytics_reader_projection(
    response: TraceCommonsAnalyticsResponse,
) -> TraceReaderAnalyticsProjection {
    TraceReaderAnalyticsProjection {
        submissions_total: response.submissions_total,
        by_status: response.by_status,
        by_privacy_risk: response.by_privacy_risk,
        by_task_success: response.by_task_success,
        by_tool: response.by_tool,
        by_tool_category: response.by_tool_category,
        coverage_tags: response.coverage_tags,
        duplicate_groups: response.duplicate_groups,
        average_novelty_score_bits: response.average_novelty_score.to_bits(),
    }
}

fn export_manifest_reader_projection(
    summaries: Vec<TraceExportManifestSummary>,
) -> BTreeMap<Uuid, TraceReaderExportManifestProjection> {
    summaries
        .into_iter()
        .map(|summary| {
            (
                summary.export_manifest_id,
                TraceReaderExportManifestProjection {
                    artifact_kind: summary.artifact_kind,
                    purpose_code: summary.purpose_code,
                    audit_event_id: summary.audit_event_id,
                    source_submission_ids: summary.source_submission_ids,
                    source_submission_ids_hash: summary.source_submission_ids_hash,
                    item_count: summary.item_count,
                    generated_at_millis: timestamp_millis(summary.generated_at),
                    invalidated_at_millis: optional_timestamp_millis(summary.invalidated_at),
                    deleted_at_millis: optional_timestamp_millis(summary.deleted_at),
                },
            )
        })
        .collect()
}

fn record_reader_parity(
    failures: &mut Vec<String>,
    name: &'static str,
    ok: bool,
    detail: String,
) -> bool {
    if !ok {
        failures.push(format!("{name}: {detail}"));
    }
    ok
}

#[derive(Debug, Serialize, Deserialize)]
struct TraceCommonsRevocation {
    tenant_id: String,
    tenant_storage_ref: String,
    submission_id: Uuid,
    revoked_at: DateTime<Utc>,
    reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    decision_inputs_hash: Option<String>,
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
            decision_inputs_hash: None,
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
            decision_inputs_hash: None,
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
            decision_inputs_hash: None,
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
            decision_inputs_hash: None,
        }
    }

    fn credit_mutation(
        auth: &TenantAuth,
        submission_id: Uuid,
        credit_points_delta: f32,
        reason: Option<&str>,
    ) -> Self {
        let reason = reason
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| "delayed credit event".to_string());
        Self {
            event_id: Uuid::new_v4(),
            tenant_id: auth.tenant_id.clone(),
            submission_id,
            kind: "credit_mutate".to_string(),
            created_at: Utc::now(),
            status: None,
            actor_role: Some(auth.role),
            actor_principal_ref: Some(auth.principal_ref.clone()),
            reason: Some(format!("points_delta={credit_points_delta:.4};{reason}")),
            export_count: None,
            export_id: None,
            decision_inputs_hash: None,
        }
    }

    fn read(auth: &TenantAuth, surface: &str, item_count: usize) -> Self {
        Self {
            event_id: Uuid::new_v4(),
            tenant_id: auth.tenant_id.clone(),
            submission_id: Uuid::nil(),
            kind: "read".to_string(),
            created_at: Utc::now(),
            status: None,
            actor_role: Some(auth.role),
            actor_principal_ref: Some(auth.principal_ref.clone()),
            reason: Some(format!("surface={surface};item_count={item_count}")),
            export_count: Some(item_count),
            export_id: None,
            decision_inputs_hash: None,
        }
    }

    fn trace_content_read(
        auth: &TenantAuth,
        submission_id: Uuid,
        surface: &str,
        purpose: Option<&str>,
    ) -> Self {
        let mut reason = format!("surface={surface}");
        if let Some(purpose) = purpose.map(str::trim).filter(|purpose| !purpose.is_empty()) {
            reason.push_str(";purpose=");
            reason.push_str(purpose);
        }
        Self {
            event_id: Uuid::new_v4(),
            tenant_id: auth.tenant_id.clone(),
            submission_id,
            kind: "trace_content_read".to_string(),
            created_at: Utc::now(),
            status: None,
            actor_role: Some(auth.role),
            actor_principal_ref: Some(auth.principal_ref.clone()),
            reason: Some(reason),
            export_count: None,
            export_id: None,
            decision_inputs_hash: None,
        }
    }

    fn dataset_export(
        auth: &TenantAuth,
        export_id: Uuid,
        export_count: usize,
        source_submission_ids_hash: String,
    ) -> Self {
        Self {
            event_id: Uuid::new_v4(),
            tenant_id: auth.tenant_id.clone(),
            submission_id: Uuid::nil(),
            kind: "dataset_export".to_string(),
            created_at: Utc::now(),
            status: None,
            actor_role: Some(auth.role),
            actor_principal_ref: Some(auth.principal_ref.clone()),
            reason: Some(format!(
                "source_submission_ids_hash={source_submission_ids_hash}"
            )),
            export_count: Some(export_count),
            export_id: Some(export_id),
            decision_inputs_hash: Some(source_submission_ids_hash),
        }
    }

    fn benchmark_conversion(
        auth: &TenantAuth,
        conversion_id: Uuid,
        candidate_count: usize,
        source_submission_ids_hash: String,
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
            reason: Some(format!(
                "source_submission_ids_hash={source_submission_ids_hash}"
            )),
            export_count: Some(candidate_count),
            export_id: Some(conversion_id),
            decision_inputs_hash: Some(source_submission_ids_hash),
        }
    }

    fn ranker_training_export(
        auth: &TenantAuth,
        export_id: Uuid,
        kind: &str,
        item_count: usize,
        source_item_list_hash: String,
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
            reason: Some(format!("source_item_list_hash={source_item_list_hash}")),
            export_count: Some(item_count),
            export_id: Some(export_id),
            decision_inputs_hash: Some(source_item_list_hash),
        }
    }

    fn vector_index(auth: &TenantAuth, vector_entries_indexed: usize, dry_run: bool) -> Self {
        Self {
            event_id: Uuid::new_v4(),
            tenant_id: auth.tenant_id.clone(),
            submission_id: Uuid::nil(),
            kind: "vector_index".to_string(),
            created_at: Utc::now(),
            status: None,
            actor_role: Some(auth.role),
            actor_principal_ref: Some(auth.principal_ref.clone()),
            reason: Some(format!(
                "dry_run={dry_run};vector_entries_indexed={vector_entries_indexed}"
            )),
            export_count: Some(vector_entries_indexed),
            export_id: None,
            decision_inputs_hash: None,
        }
    }

    fn maintenance(
        auth: &TenantAuth,
        purpose: &str,
        dry_run: bool,
        counts: TraceMaintenanceAuditCounts,
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
                "purpose={purpose};dry_run={dry_run};records_marked_revoked={};records_marked_expired={};records_marked_purged={};derived_marked_revoked={};derived_marked_expired={};export_cache_files_pruned={};export_provenance_invalidated={};trace_object_files_deleted={};encrypted_artifacts_deleted={};db_mirror_backfilled={};vector_entries_indexed={}",
                counts.records_marked_revoked,
                counts.records_marked_expired,
                counts.records_marked_purged,
                counts.derived_marked_revoked,
                counts.derived_marked_expired,
                counts.export_cache_files_pruned,
                counts.export_provenance_invalidated,
                counts.trace_object_files_deleted,
                counts.encrypted_artifacts_deleted,
                counts.db_mirror_backfilled,
                counts.vector_entries_indexed
            )),
            export_count: Some(counts.export_cache_files_pruned),
            export_id: None,
            decision_inputs_hash: None,
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
    expired: usize,
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
            expired: 0,
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
                TraceCorpusStatus::Expired | TraceCorpusStatus::Purged => response.expired += 1,
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
    use ironclaw::trace_corpus_storage::TraceCorpusStore;

    fn test_state(root: PathBuf) -> Arc<AppState> {
        test_state_with_options(root, None, None, false, false, false, false)
    }

    fn test_state_with_db(root: PathBuf, db_mirror: Option<Arc<dyn Database>>) -> Arc<AppState> {
        test_state_with_options(root, db_mirror, None, false, false, false, false)
    }

    fn test_state_with_db_contributor_reads(
        root: PathBuf,
        db_mirror: Option<Arc<dyn Database>>,
    ) -> Arc<AppState> {
        test_state_with_options(root, db_mirror, None, true, false, false, false)
    }

    fn test_state_with_db_reviewer_reads(
        root: PathBuf,
        db_mirror: Option<Arc<dyn Database>>,
    ) -> Arc<AppState> {
        test_state_with_options(root, db_mirror, None, false, true, false, false)
    }

    fn test_state_with_db_replay_export_reads(
        root: PathBuf,
        db_mirror: Option<Arc<dyn Database>>,
    ) -> Arc<AppState> {
        test_state_with_options(root, db_mirror, None, false, false, true, false)
    }

    fn test_state_with_options(
        root: PathBuf,
        db_mirror: Option<Arc<dyn Database>>,
        artifact_store: Option<Arc<LocalEncryptedTraceArtifactStore>>,
        db_contributor_reads: bool,
        db_reviewer_reads: bool,
        db_replay_export_reads: bool,
        db_audit_reads: bool,
    ) -> Arc<AppState> {
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
            db_mirror,
            db_contributor_reads,
            db_reviewer_reads,
            db_replay_export_reads,
            db_audit_reads,
            artifact_store,
        })
    }

    fn test_artifact_store(root: &Path) -> Arc<LocalEncryptedTraceArtifactStore> {
        let key = ironclaw::secrets::keychain::generate_master_key_hex();
        let crypto = SecretsCrypto::new(SecretString::from(key)).expect("test crypto");
        Arc::new(LocalEncryptedTraceArtifactStore::new(root, crypto))
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
    async fn submit_writes_encrypted_artifact_receipt_when_configured() {
        let temp = tempfile::tempdir().expect("temp dir");
        let artifact_temp = tempfile::tempdir().expect("artifact temp dir");
        let artifact_store = test_artifact_store(artifact_temp.path());
        let state = test_state_with_options(
            temp.path().to_path_buf(),
            None,
            Some(artifact_store.clone()),
            false,
            false,
            false,
            false,
        );
        let mut envelope = sample_envelope().await;
        make_metadata_only_low_risk(&mut envelope);
        let submission_id = envelope.submission_id;

        let Json(receipt) = submit_trace_handler(
            State(state.clone()),
            auth_headers("token-a"),
            Json(envelope),
        )
        .await
        .expect("submission succeeds");

        assert_eq!(receipt.status, "accepted");
        let record = read_submission_record(temp.path(), "tenant-a", submission_id)
            .expect("record reads")
            .expect("record exists");
        let receipt = record
            .artifact_receipt
            .as_ref()
            .expect("encrypted artifact receipt should be persisted");
        let encrypted = artifact_store
            .read_artifact(&record.tenant_storage_ref, receipt)
            .expect("encrypted artifact reads");
        let encrypted_json = serde_json::to_string(&encrypted).expect("artifact serializes");
        assert!(!encrypted_json.contains("Please inspect the workspace"));

        let round_trip =
            read_envelope_by_record(state.as_ref(), &record).expect("encrypted envelope reads");
        assert_eq!(round_trip.submission_id, submission_id);
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn submit_dual_writes_to_db_mirror_when_configured() {
        use ironclaw::db::libsql::LibSqlBackend;

        let temp = tempfile::tempdir().expect("temp dir");
        let db_temp = tempfile::tempdir().expect("db temp dir");
        let db_path = db_temp.path().join("trace-mirror.db");
        let db = Arc::new(
            LibSqlBackend::new_local(&db_path)
                .await
                .expect("create libsql mirror"),
        );
        db.run_migrations().await.expect("run migrations");
        let state = test_state_with_db(
            temp.path().to_path_buf(),
            Some(db.clone() as Arc<dyn Database>),
        );
        let mut envelope = sample_envelope().await;
        make_metadata_only_low_risk(&mut envelope);
        let submission_id = envelope.submission_id;

        let Json(receipt) = submit_trace_handler(
            State(state.clone()),
            auth_headers("token-a"),
            Json(envelope),
        )
        .await
        .expect("submission succeeds");

        assert_eq!(receipt.status, "accepted");
        let mirrored = db
            .get_trace_submission("tenant-a", submission_id)
            .await
            .expect("mirror query succeeds")
            .expect("mirrored submission exists");
        assert_eq!(mirrored.status, StorageTraceCorpusStatus::Accepted);
        assert!(
            db.get_trace_submission("tenant-b", submission_id)
                .await
                .expect("tenant-isolated mirror query succeeds")
                .is_none()
        );

        let Json(credit_event) = append_credit_event_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            AxumPath(submission_id),
            Json(TraceCreditLedgerAppendRequest {
                event_type: TraceCreditLedgerEventType::ReviewerBonus,
                credit_points_delta: 2.5,
                reason: Some("reviewer found high utility".to_string()),
                external_ref: Some("review:test".to_string()),
            }),
        )
        .await
        .expect("credit append succeeds");
        assert_eq!(credit_event.credit_points_delta, 2.5);
        let conn = db.connect().await.expect("connect to mirror");
        let mut rows = conn
            .query(
                "SELECT COUNT(*) FROM trace_credit_ledger WHERE tenant_id = ?1 AND submission_id = ?2 AND event_type = ?3",
                libsql::params!["tenant-a", submission_id.to_string(), "reviewer_bonus"],
            )
            .await
            .expect("credit ledger query succeeds");
        let row = rows
            .next()
            .await
            .expect("credit ledger row fetch succeeds")
            .expect("credit ledger count row exists");
        let mirrored_credit_events = row.get::<i64>(0).expect("count column reads");
        assert_eq!(mirrored_credit_events, 1);

        let Json(review_receipt) = review_decision_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            AxumPath(submission_id),
            Json(TraceReviewDecisionRequest {
                decision: TraceReviewDecision::Reject,
                reason: Some("test rejection".to_string()),
                credit_points_pending: None,
            }),
        )
        .await
        .expect("review decision succeeds");
        assert_eq!(review_receipt.status, "rejected");
        let reviewed = db
            .get_trace_submission("tenant-a", submission_id)
            .await
            .expect("review mirror query succeeds")
            .expect("mirrored submission remains queryable");
        assert_eq!(reviewed.status, StorageTraceCorpusStatus::Rejected);

        let status = revoke_trace_handler(
            State(state),
            auth_headers("token-a"),
            AxumPath(submission_id),
        )
        .await
        .expect("revocation succeeds");
        assert_eq!(status, StatusCode::NO_CONTENT);
        let revoked = db
            .get_trace_submission("tenant-a", submission_id)
            .await
            .expect("revoked mirror query succeeds")
            .expect("mirrored submission remains queryable");
        assert_eq!(revoked.status, StorageTraceCorpusStatus::Revoked);
        assert!(revoked.revoked_at.is_some());
        let conn = db.connect().await.expect("connect to mirror after revoke");
        let mut rows = conn
            .query(
                "SELECT COUNT(*) FROM trace_object_refs WHERE tenant_id = ?1 AND submission_id = ?2 AND invalidated_at IS NOT NULL",
                libsql::params!["tenant-a", submission_id.to_string()],
            )
            .await
            .expect("object invalidation query succeeds");
        let row = rows
            .next()
            .await
            .expect("object invalidation row fetch succeeds")
            .expect("object invalidation count row exists");
        assert_eq!(row.get::<i64>(0).expect("object count reads"), 1);
        let mut rows = conn
            .query(
                "SELECT COUNT(*) FROM trace_derived_records WHERE tenant_id = ?1 AND submission_id = ?2 AND status = ?3",
                libsql::params!["tenant-a", submission_id.to_string(), "revoked"],
            )
            .await
            .expect("derived invalidation query succeeds");
        let row = rows
            .next()
            .await
            .expect("derived invalidation row fetch succeeds")
            .expect("derived invalidation count row exists");
        assert_eq!(row.get::<i64>(0).expect("derived count reads"), 1);
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn contributor_credit_status_can_read_from_db_mirror_when_enabled() {
        use ironclaw::db::libsql::LibSqlBackend;

        let temp = tempfile::tempdir().expect("temp dir");
        let db_temp = tempfile::tempdir().expect("db temp dir");
        let db_path = db_temp.path().join("trace-mirror.db");
        let db = Arc::new(
            LibSqlBackend::new_local(&db_path)
                .await
                .expect("create libsql mirror"),
        );
        db.run_migrations().await.expect("run migrations");
        let state = test_state_with_db_contributor_reads(
            temp.path().to_path_buf(),
            Some(db.clone() as Arc<dyn Database>),
        );
        let mut envelope = sample_envelope().await;
        make_metadata_only_low_risk(&mut envelope);
        let submission_id = envelope.submission_id;

        let Json(receipt) = submit_trace_handler(
            State(state.clone()),
            auth_headers("token-a"),
            Json(envelope),
        )
        .await
        .expect("submission succeeds");
        assert_eq!(receipt.status, "accepted");

        let Json(appended) = append_credit_event_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            AxumPath(submission_id),
            Json(TraceCreditLedgerAppendRequest {
                event_type: TraceCreditLedgerEventType::ReviewerBonus,
                credit_points_delta: 2.5,
                reason: Some("reviewer found downstream utility".to_string()),
                external_ref: Some("review:test".to_string()),
            }),
        )
        .await
        .expect("credit append succeeds");
        assert_eq!(
            appended.event_type,
            TraceCreditLedgerEventType::ReviewerBonus
        );

        let tenant_dir = temp
            .path()
            .join("tenants")
            .join(tenant_storage_key("tenant-a"));
        std::fs::remove_file(
            tenant_dir
                .join("metadata")
                .join(format!("{submission_id}.json")),
        )
        .expect("remove file-backed metadata to prove DB read path");
        std::fs::remove_file(tenant_dir.join("credit_ledger").join("events.jsonl"))
            .expect("remove file-backed ledger to prove DB read path");

        let Json(events) = credit_events_handler(State(state.clone()), auth_headers("token-a"))
            .await
            .expect("credit events load from DB");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].submission_id, submission_id);
        assert_eq!(
            events[0].event_type,
            TraceCreditLedgerEventType::ReviewerBonus
        );
        assert_eq!(events[0].credit_points_delta, 2.5);

        let Json(credit) = credit_handler(State(state.clone()), auth_headers("token-a"))
            .await
            .expect("credit summary loads from DB");
        assert_eq!(credit.accepted, 1);
        assert_eq!(credit.credit_points_ledger, 2.5);
        assert!(credit.credit_points_final > 0.0);
        assert_eq!(
            credit.credit_points_total,
            credit.credit_points_final + credit.credit_points_ledger
        );

        let Json(statuses) = submission_status_handler(
            State(state.clone()),
            auth_headers("token-a"),
            Json(TraceSubmissionStatusRequest {
                submission_ids: vec![submission_id],
            }),
        )
        .await
        .expect("status loads from DB");
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].submission_id, submission_id);
        assert_eq!(statuses[0].status, "accepted");
        assert_eq!(statuses[0].credit_points_ledger, 2.5);
        assert_eq!(statuses[0].delayed_credit_explanations.len(), 1);

        let Json(other_contributor_credit) =
            credit_handler(State(state), auth_headers("token-a-2"))
                .await
                .expect("same-tenant contributor remains principal scoped");
        assert_eq!(other_contributor_credit.accepted, 0);
        assert_eq!(other_contributor_credit.credit_points_ledger, 0.0);
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
        assert_eq!(export.manifest.source_submission_ids, vec![submission_id]);
        assert!(
            export
                .manifest
                .source_submission_ids_hash
                .starts_with("sha256:")
        );
        let audit_events =
            read_all_audit_events(temp.path(), "tenant-a").expect("audit events read");
        assert!(audit_events.iter().any(|event| {
            event.event_id == export.audit_event_id
                && event.export_id == Some(export.export_id)
                && event.kind == "dataset_export"
                && event.decision_inputs_hash
                    == Some(export.manifest.source_submission_ids_hash.clone())
        }));
        assert!(audit_events.iter().any(|event| {
            event.submission_id == submission_id
                && event.kind == "trace_content_read"
                && event.reason.as_deref().is_some_and(|reason| {
                    reason.contains("surface=replay_dataset_export")
                        && reason.contains("purpose=trace_commons_replay_dataset")
                })
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
        assert!(
            export
                .manifest
                .source_submission_ids_hash
                .starts_with("sha256:")
        );
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
        assert!(benchmark.source_submission_ids_hash.starts_with("sha256:"));
        assert_eq!(benchmark.candidates[0].submission_id, kept_id);
        assert!(!benchmark.source_submission_ids.contains(&revoked_id));
        assert!(benchmark_artifact_path(temp.path(), "tenant-a", benchmark.conversion_id).exists());

        let audit_events =
            read_all_audit_events(temp.path(), "tenant-a").expect("audit events read");
        assert!(audit_events.iter().any(|event| {
            event.event_id == benchmark.audit_event_id
                && event.kind == "benchmark_conversion"
                && event.decision_inputs_hash == Some(benchmark.source_submission_ids_hash.clone())
        }));
    }

    #[tokio::test]
    async fn benchmark_conversion_writes_provenance_and_revocation_invalidates_it() {
        let temp = tempfile::tempdir().expect("temp dir");
        let state = test_state(temp.path().to_path_buf());
        let mut envelope = sample_envelope().await;
        make_metadata_only_low_risk(&mut envelope);
        let submission_id = envelope.submission_id;

        let _ = submit_trace_handler(
            State(state.clone()),
            auth_headers("token-a"),
            Json(envelope),
        )
        .await
        .expect("benchmark source submission succeeds");

        let Json(benchmark) = benchmark_convert_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            Json(BenchmarkConversionRequest {
                limit: Some(10),
                purpose: Some("provenance_benchmark".to_string()),
                consent_scope: None,
                status: None,
                privacy_risk: None,
                external_ref: Some("benchmark:provenance".to_string()),
            }),
        )
        .await
        .expect("benchmark conversion succeeds");

        let provenance_path = temp
            .path()
            .join("tenants")
            .join(tenant_storage_key("tenant-a"))
            .join("benchmarks")
            .join(benchmark.conversion_id.to_string())
            .join("provenance.json");
        let provenance: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(&provenance_path).expect("benchmark provenance reads"),
        )
        .expect("benchmark provenance parses");
        assert_eq!(provenance["tenant_id"], "tenant-a");
        assert_eq!(provenance["export_id"], benchmark.conversion_id.to_string());
        assert_eq!(provenance["purpose"], "provenance_benchmark");
        assert_eq!(
            provenance["source_submission_ids"][0],
            submission_id.to_string()
        );
        assert_eq!(
            provenance["source_submission_ids_hash"],
            benchmark.source_submission_ids_hash
        );
        assert!(provenance["invalidated_at"].is_null());

        revoke_trace_handler(
            State(state.clone()),
            auth_headers("token-a"),
            AxumPath(submission_id),
        )
        .await
        .expect("contributor can revoke benchmark source");

        let invalidated: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(&provenance_path)
                .expect("invalidated benchmark provenance reads"),
        )
        .expect("invalidated benchmark provenance parses");
        assert!(invalidated["invalidated_at"].as_str().is_some());
        assert_eq!(invalidated["invalidation_reason"], "contributor_revocation");
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
        assert!(candidates.source_item_list_hash.starts_with("sha256:"));

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
        assert!(pairs.source_item_list_hash.starts_with("sha256:"));
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
    async fn ranker_exports_write_provenance_and_maintenance_invalidates_sources() {
        let temp = tempfile::tempdir().expect("temp dir");
        let state = test_state(temp.path().to_path_buf());
        let mut preferred = sample_envelope().await;
        make_metadata_only_low_risk(&mut preferred);
        preferred.consent.scopes = vec![ConsentScope::RankingTraining];
        let preferred_id = preferred.submission_id;
        let mut rejected = sample_envelope().await;
        make_metadata_only_low_risk(&mut rejected);
        rejected.consent.scopes = vec![ConsentScope::RankingTraining];
        rejected.value.submission_score = 0.1;
        let rejected_id = rejected.submission_id;

        let _ = submit_trace_handler(
            State(state.clone()),
            auth_headers("token-a"),
            Json(preferred),
        )
        .await
        .expect("preferred ranker source submission succeeds");
        let _ = submit_trace_handler(
            State(state.clone()),
            auth_headers("token-a"),
            Json(rejected),
        )
        .await
        .expect("rejected ranker source submission succeeds");

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
        .expect("ranker candidates export succeeds");
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
        .expect("ranker pairs export succeeds");

        let candidate_provenance_path = temp
            .path()
            .join("tenants")
            .join(tenant_storage_key("tenant-a"))
            .join("ranker_exports")
            .join(candidates.export_id.to_string())
            .join("provenance.json");
        let pair_provenance_path = temp
            .path()
            .join("tenants")
            .join(tenant_storage_key("tenant-a"))
            .join("ranker_exports")
            .join(pairs.export_id.to_string())
            .join("provenance.json");
        let candidate_provenance: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(&candidate_provenance_path)
                .expect("candidate provenance reads"),
        )
        .expect("candidate provenance parses");
        assert_eq!(
            candidate_provenance["source_submission_ids_hash"],
            candidates.source_item_list_hash
        );
        assert!(
            candidate_provenance["source_submission_ids"]
                .as_array()
                .expect("candidate source ids are an array")
                .iter()
                .any(|value| value == &serde_json::Value::String(preferred_id.to_string()))
        );

        let pair_provenance: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(&pair_provenance_path).expect("pair provenance reads"),
        )
        .expect("pair provenance parses");
        assert_eq!(
            pair_provenance["source_submission_ids_hash"],
            pairs.source_item_list_hash
        );
        assert_eq!(pair_provenance["export_kind"], "ranker_training_pairs");
        assert!(
            pair_provenance["source_submission_ids"]
                .as_array()
                .expect("pair source ids are an array")
                .iter()
                .any(|value| value == &serde_json::Value::String(rejected_id.to_string()))
        );

        write_revocation(
            temp.path(),
            &TraceCommonsRevocation {
                tenant_id: "tenant-a".to_string(),
                tenant_storage_ref: tenant_storage_ref("tenant-a"),
                submission_id: preferred_id,
                revoked_at: Utc::now(),
                reason: "test_maintenance_revocation".to_string(),
            },
        )
        .expect("revocation tombstone writes");
        let Json(response) = maintenance_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            Json(TraceMaintenanceRequest {
                dry_run: false,
                prune_export_cache: false,
                max_export_age_hours: None,
                purpose: Some("ranker_provenance_invalidation".to_string()),
                backfill_db_mirror: false,
                purge_expired_before: None,
                index_vectors: false,
                reconcile_db_mirror: false,
            }),
        )
        .await
        .expect("maintenance invalidates ranker provenance");
        assert_eq!(response.records_marked_revoked, 1);
        assert_eq!(response.export_provenance_invalidated, 2);

        let invalidated_candidate: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(&candidate_provenance_path)
                .expect("invalidated candidate provenance reads"),
        )
        .expect("invalidated candidate provenance parses");
        assert!(invalidated_candidate["invalidated_at"].as_str().is_some());
        assert_eq!(
            invalidated_candidate["invalidation_reason"],
            "test_maintenance_revocation"
        );
        let invalidated_pair: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(&pair_provenance_path)
                .expect("invalidated pair provenance reads"),
        )
        .expect("invalidated pair provenance parses");
        assert!(invalidated_pair["invalidated_at"].as_str().is_some());
        assert_eq!(
            invalidated_pair["invalidation_reason"],
            "test_maintenance_revocation"
        );
        let audit_events =
            read_all_audit_events(temp.path(), "tenant-a").expect("audit events read");
        let maintenance_audit = audit_events
            .iter()
            .find(|event| event.event_id == response.audit_event_id)
            .expect("maintenance audit event written");
        assert!(
            maintenance_audit
                .reason
                .as_deref()
                .expect("maintenance audit reason")
                .contains("export_provenance_invalidated=2")
        );
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
            source_submission_ids_hash: source_submission_ids_hash(
                "test_export_cache",
                &[tenant_a_id],
            ),
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
                backfill_db_mirror: false,
                index_vectors: false,
                reconcile_db_mirror: false,
                prune_export_cache: true,
                max_export_age_hours: None,
                purge_expired_before: None,
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
                backfill_db_mirror: false,
                index_vectors: false,
                reconcile_db_mirror: false,
                prune_export_cache: true,
                max_export_age_hours: None,
                purge_expired_before: None,
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
    async fn maintenance_marks_expired_traces_and_excludes_them_from_exports() {
        let temp = tempfile::tempdir().expect("temp dir");
        let state = test_state(temp.path().to_path_buf());
        let mut envelope = sample_envelope().await;
        make_metadata_only_low_risk(&mut envelope);
        let submission_id = envelope.submission_id;

        let _ = submit_trace_handler(
            State(state.clone()),
            auth_headers("token-a"),
            Json(envelope),
        )
        .await
        .expect("submission succeeds");
        let record = read_submission_record(temp.path(), "tenant-a", submission_id)
            .expect("record reads")
            .expect("record exists");
        let Json(pre_expiry_export) = dataset_replay_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            Query(DatasetExportQuery {
                limit: Some(10),
                purpose: Some("test_expired_export_cache".to_string()),
                status: None,
                privacy_risk: None,
                consent_scope: None,
            }),
        )
        .await
        .expect("pre-expiry export succeeds");
        assert_eq!(pre_expiry_export.item_count, 1);
        let cached_export_path =
            export_artifact_dir(temp.path(), "tenant-a", pre_expiry_export.export_id)
                .join("dataset.json");
        write_json_file(
            &cached_export_path,
            &pre_expiry_export,
            "test expired source export cache",
        )
        .expect("expired source cache writes");

        let metadata_path = temp
            .path()
            .join("tenants")
            .join(tenant_storage_key("tenant-a"))
            .join("metadata")
            .join(format!("{submission_id}.json"));
        let mut metadata_json = serde_json::to_value(record).expect("record serializes");
        metadata_json["expires_at"] =
            serde_json::json!((Utc::now() - chrono::Duration::days(1)).to_rfc3339());
        write_json_file(&metadata_path, &metadata_json, "expired trace metadata")
            .expect("expired metadata writes");

        let Json(response) = maintenance_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            Json(TraceMaintenanceRequest {
                purpose: Some("test_expiration".to_string()),
                dry_run: false,
                backfill_db_mirror: false,
                index_vectors: false,
                reconcile_db_mirror: false,
                prune_export_cache: true,
                max_export_age_hours: None,
                purge_expired_before: None,
            }),
        )
        .await
        .expect("maintenance expires traces");
        assert_eq!(response.records_marked_expired, 1);
        assert_eq!(response.derived_marked_expired, 1);
        assert_eq!(response.export_cache_files_pruned, 1);
        assert!(!cached_export_path.exists());
        let pruned_marker_path =
            export_artifact_dir(temp.path(), "tenant-a", pre_expiry_export.export_id)
                .join("pruned.json");
        let pruned_marker: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(pruned_marker_path).expect("pruned marker reads"),
        )
        .expect("pruned marker parses");
        assert_eq!(pruned_marker["reason"], "retention_expired_source");

        let expired_metadata: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(&metadata_path).expect("expired metadata reads"),
        )
        .expect("expired metadata parses");
        assert_eq!(expired_metadata["status"], "expired");

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
        .expect("expired trace export query succeeds");
        assert_eq!(export.item_count, 0);

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
        .expect("expired trace benchmark query succeeds");
        assert_eq!(benchmark.item_count, 0);

        let review_error = review_decision_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            AxumPath(submission_id),
            Json(TraceReviewDecisionRequest {
                decision: TraceReviewDecision::Approve,
                reason: Some("too late".to_string()),
                credit_points_pending: None,
            }),
        )
        .await
        .expect_err("expired trace cannot be reviewed");
        assert_eq!(review_error.0, StatusCode::CONFLICT);

        let credit_error = append_credit_event_handler(
            State(state),
            auth_headers("review-token-a"),
            AxumPath(submission_id),
            Json(TraceCreditLedgerAppendRequest {
                event_type: TraceCreditLedgerEventType::ReviewerBonus,
                credit_points_delta: 1.0,
                reason: Some("too late".to_string()),
                external_ref: None,
            }),
        )
        .await
        .expect_err("expired trace cannot receive credit");
        assert_eq!(credit_error.0, StatusCode::CONFLICT);
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn maintenance_expiration_updates_db_mirror_and_invalidates_artifacts() {
        use ironclaw::db::libsql::LibSqlBackend;

        let temp = tempfile::tempdir().expect("temp dir");
        let db_temp = tempfile::tempdir().expect("db temp dir");
        let db_path = db_temp.path().join("trace-expiration-mirror.db");
        let db = Arc::new(
            LibSqlBackend::new_local(&db_path)
                .await
                .expect("create libsql mirror"),
        );
        db.run_migrations().await.expect("run migrations");
        let state = test_state_with_db(
            temp.path().to_path_buf(),
            Some(db.clone() as Arc<dyn Database>),
        );
        let mut envelope = sample_envelope().await;
        make_metadata_only_low_risk(&mut envelope);
        let submission_id = envelope.submission_id;

        let _ = submit_trace_handler(
            State(state.clone()),
            auth_headers("token-a"),
            Json(envelope),
        )
        .await
        .expect("submission succeeds");
        let record = read_submission_record(temp.path(), "tenant-a", submission_id)
            .expect("record reads")
            .expect("record exists");
        let metadata_path = temp
            .path()
            .join("tenants")
            .join(tenant_storage_key("tenant-a"))
            .join("metadata")
            .join(format!("{submission_id}.json"));
        let mut metadata_json = serde_json::to_value(record).expect("record serializes");
        metadata_json["expires_at"] =
            serde_json::json!((Utc::now() - chrono::Duration::days(1)).to_rfc3339());
        write_json_file(&metadata_path, &metadata_json, "expired trace metadata")
            .expect("expired metadata writes");

        let Json(response) = maintenance_handler(
            State(state),
            auth_headers("review-token-a"),
            Json(TraceMaintenanceRequest {
                purpose: Some("test_db_expiration".to_string()),
                dry_run: false,
                backfill_db_mirror: false,
                index_vectors: false,
                reconcile_db_mirror: false,
                prune_export_cache: false,
                max_export_age_hours: None,
                purge_expired_before: None,
            }),
        )
        .await
        .expect("maintenance expires traces");
        assert_eq!(response.records_marked_expired, 1);
        assert_eq!(response.derived_marked_expired, 1);

        let mirrored = db
            .get_trace_submission("tenant-a", submission_id)
            .await
            .expect("mirrored submission reads")
            .expect("mirrored submission exists");
        assert_eq!(mirrored.status, StorageTraceCorpusStatus::Expired);
        let object_refs = db
            .list_trace_object_refs("tenant-a", submission_id)
            .await
            .expect("object refs read");
        assert!(!object_refs.is_empty());
        assert!(
            object_refs
                .iter()
                .all(|record| record.invalidated_at.is_some())
        );
        let derived = db
            .list_trace_derived_records("tenant-a")
            .await
            .expect("derived records read");
        assert!(derived.iter().any(|record| {
            record.submission_id == submission_id
                && record.status == StorageTraceDerivedStatus::Expired
        }));
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn maintenance_purge_updates_db_mirror_status_and_invalidates_refs() {
        use ironclaw::db::libsql::LibSqlBackend;

        let temp = tempfile::tempdir().expect("temp dir");
        let db_temp = tempfile::tempdir().expect("db temp dir");
        let db_path = db_temp.path().join("trace-purge-mirror.db");
        let db = Arc::new(
            LibSqlBackend::new_local(&db_path)
                .await
                .expect("create libsql mirror"),
        );
        db.run_migrations().await.expect("run migrations");
        let state = test_state_with_db(
            temp.path().to_path_buf(),
            Some(db.clone() as Arc<dyn Database>),
        );
        let mut envelope = sample_envelope().await;
        make_metadata_only_low_risk(&mut envelope);
        let submission_id = envelope.submission_id;

        let _ = submit_trace_handler(
            State(state.clone()),
            auth_headers("token-a"),
            Json(envelope),
        )
        .await
        .expect("submission succeeds");
        let record = read_submission_record(temp.path(), "tenant-a", submission_id)
            .expect("record reads")
            .expect("record exists");
        let metadata_path = temp
            .path()
            .join("tenants")
            .join(tenant_storage_key("tenant-a"))
            .join("metadata")
            .join(format!("{submission_id}.json"));
        let mut metadata_json = serde_json::to_value(record).expect("record serializes");
        metadata_json["expires_at"] =
            serde_json::json!((Utc::now() - chrono::Duration::days(1)).to_rfc3339());
        write_json_file(&metadata_path, &metadata_json, "expired trace metadata")
            .expect("expired metadata writes");

        let Json(response) = maintenance_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            Json(TraceMaintenanceRequest {
                purpose: Some("test_db_purge".to_string()),
                dry_run: false,
                backfill_db_mirror: false,
                index_vectors: false,
                reconcile_db_mirror: false,
                prune_export_cache: false,
                max_export_age_hours: None,
                purge_expired_before: Some(Utc::now()),
            }),
        )
        .await
        .expect("maintenance purges traces");
        assert_eq!(response.records_marked_expired, 1);
        assert_eq!(response.records_marked_purged, 1);

        let mirrored = db
            .get_trace_submission("tenant-a", submission_id)
            .await
            .expect("mirrored submission reads")
            .expect("mirrored submission exists");
        assert_eq!(mirrored.status, StorageTraceCorpusStatus::Purged);
        assert!(mirrored.purged_at.is_some());
        let object_refs = db
            .list_trace_object_refs("tenant-a", submission_id)
            .await
            .expect("object refs read");
        assert!(!object_refs.is_empty());
        assert!(
            object_refs
                .iter()
                .all(|record| record.invalidated_at.is_some())
        );
        let derived = db
            .list_trace_derived_records("tenant-a")
            .await
            .expect("derived records read");
        assert!(derived.iter().any(|record| {
            record.submission_id == submission_id
                && record.status == StorageTraceDerivedStatus::Expired
        }));
    }

    #[tokio::test]
    async fn maintenance_purges_expired_trace_objects_only_with_explicit_cutoff() {
        let temp = tempfile::tempdir().expect("temp dir");
        let artifact_temp = tempfile::tempdir().expect("artifact temp dir");
        let artifact_store = test_artifact_store(artifact_temp.path());
        let state = test_state_with_options(
            temp.path().to_path_buf(),
            None,
            Some(artifact_store.clone()),
            false,
            false,
            false,
            false,
        );
        let mut envelope = sample_envelope().await;
        make_metadata_only_low_risk(&mut envelope);
        let submission_id = envelope.submission_id;

        let _ = submit_trace_handler(
            State(state.clone()),
            auth_headers("token-a"),
            Json(envelope),
        )
        .await
        .expect("submission succeeds");
        let record = read_submission_record(temp.path(), "tenant-a", submission_id)
            .expect("record reads")
            .expect("record exists");
        let object_path = temp.path().join(&record.object_key);
        assert!(object_path.exists());
        let receipt = record
            .artifact_receipt
            .clone()
            .expect("encrypted receipt exists");
        artifact_store
            .read_artifact(&record.tenant_storage_ref, &receipt)
            .expect("encrypted artifact exists");

        let metadata_path = temp
            .path()
            .join("tenants")
            .join(tenant_storage_key("tenant-a"))
            .join("metadata")
            .join(format!("{submission_id}.json"));
        let mut metadata_json = serde_json::to_value(record).expect("record serializes");
        let expired_at = Utc::now() - chrono::Duration::days(2);
        metadata_json["expires_at"] = serde_json::json!(expired_at.to_rfc3339());
        write_json_file(&metadata_path, &metadata_json, "expired trace metadata")
            .expect("expired metadata writes");

        let Json(dry_run) = maintenance_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            Json(TraceMaintenanceRequest {
                purpose: Some("test_retention_purge_dry_run".to_string()),
                dry_run: true,
                backfill_db_mirror: false,
                index_vectors: false,
                reconcile_db_mirror: false,
                prune_export_cache: false,
                max_export_age_hours: None,
                purge_expired_before: Some(Utc::now()),
            }),
        )
        .await
        .expect("dry-run purge succeeds");
        assert_eq!(dry_run.records_marked_purged, 0);
        assert_eq!(dry_run.trace_object_files_deleted, 0);
        assert!(object_path.exists());
        artifact_store
            .read_artifact(&tenant_storage_ref("tenant-a"), &receipt)
            .expect("dry-run keeps encrypted artifact");

        let Json(response) = maintenance_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            Json(TraceMaintenanceRequest {
                purpose: Some("test_retention_purge".to_string()),
                dry_run: false,
                backfill_db_mirror: false,
                index_vectors: false,
                reconcile_db_mirror: false,
                prune_export_cache: false,
                max_export_age_hours: None,
                purge_expired_before: Some(Utc::now()),
            }),
        )
        .await
        .expect("purge succeeds");
        assert_eq!(response.records_marked_expired, 1);
        assert_eq!(response.records_marked_purged, 1);
        assert_eq!(response.trace_object_files_deleted, 1);
        assert_eq!(response.encrypted_artifacts_deleted, 1);
        assert!(!object_path.exists());
        let purged_record = read_submission_record(temp.path(), "tenant-a", submission_id)
            .expect("purged record reads")
            .expect("purged record exists");
        assert_eq!(purged_record.status, TraceCorpusStatus::Purged);
        assert!(purged_record.purged_at.is_some());
        artifact_store
            .read_artifact(&tenant_storage_ref("tenant-a"), &receipt)
            .expect_err("encrypted artifact was deleted");
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn maintenance_can_backfill_file_backed_records_to_db_mirror() {
        use ironclaw::db::libsql::LibSqlBackend;

        let temp = tempfile::tempdir().expect("temp dir");
        let file_state = test_state(temp.path().to_path_buf());
        let mut envelope = sample_envelope().await;
        make_metadata_only_low_risk(&mut envelope);
        let submission_id = envelope.submission_id;

        let Json(receipt) =
            submit_trace_handler(State(file_state), auth_headers("token-a"), Json(envelope))
                .await
                .expect("file-backed submission succeeds");
        assert_eq!(receipt.status, "accepted");

        let db_temp = tempfile::tempdir().expect("db temp dir");
        let db_path = db_temp.path().join("trace-mirror-backfill.db");
        let db = Arc::new(
            LibSqlBackend::new_local(&db_path)
                .await
                .expect("create libsql mirror"),
        );
        db.run_migrations().await.expect("run migrations");
        let state = test_state_with_db(
            temp.path().to_path_buf(),
            Some(db.clone() as Arc<dyn Database>),
        );

        let Json(response) = maintenance_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            Json(TraceMaintenanceRequest {
                purpose: Some("test_db_backfill".to_string()),
                dry_run: false,
                backfill_db_mirror: true,
                index_vectors: false,
                reconcile_db_mirror: false,
                prune_export_cache: false,
                max_export_age_hours: None,
                purge_expired_before: None,
            }),
        )
        .await
        .expect("reviewer can backfill DB mirror");
        assert_eq!(response.db_mirror_backfilled, 1);

        let mirrored = db
            .get_trace_submission("tenant-a", submission_id)
            .await
            .expect("mirror query succeeds")
            .expect("mirrored submission exists");
        assert_eq!(mirrored.status, StorageTraceCorpusStatus::Accepted);
        let conn = db.connect().await.expect("connect to mirror");
        let mut rows = conn
            .query(
                "SELECT COUNT(*) FROM trace_object_refs WHERE tenant_id = ?1 AND submission_id = ?2",
                libsql::params!["tenant-a", submission_id.to_string()],
            )
            .await
            .expect("object ref query succeeds");
        let row = rows
            .next()
            .await
            .expect("object ref row fetch succeeds")
            .expect("object ref count exists");
        assert_eq!(row.get::<i64>(0).expect("object ref count reads"), 1);

        let Json(vector_response) = maintenance_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            Json(TraceMaintenanceRequest {
                purpose: Some("test_vector_index".to_string()),
                dry_run: false,
                backfill_db_mirror: false,
                index_vectors: true,
                reconcile_db_mirror: false,
                prune_export_cache: false,
                max_export_age_hours: None,
                purge_expired_before: None,
            }),
        )
        .await
        .expect("reviewer can index vector metadata");
        assert_eq!(vector_response.vector_entries_indexed, 1);
        let vector_entries = db
            .list_trace_vector_entries("tenant-a")
            .await
            .expect("vector entries read");
        assert_eq!(vector_entries.len(), 1);
        assert_eq!(vector_entries[0].submission_id, submission_id);
        assert_eq!(
            vector_entries[0].status,
            StorageTraceVectorEntryStatus::Active
        );
        assert_eq!(
            vector_entries[0].source_projection,
            StorageTraceVectorEntrySourceProjection::CanonicalSummary
        );

        let Json(vector_idempotent_response) = maintenance_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            Json(TraceMaintenanceRequest {
                purpose: Some("test_vector_index_idempotent".to_string()),
                dry_run: false,
                backfill_db_mirror: false,
                index_vectors: true,
                reconcile_db_mirror: false,
                prune_export_cache: false,
                max_export_age_hours: None,
                purge_expired_before: None,
            }),
        )
        .await
        .expect("vector indexing can be rerun");
        assert_eq!(vector_idempotent_response.vector_entries_indexed, 0);

        let Json(delayed_credit) = append_credit_event_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            AxumPath(submission_id),
            Json(TraceCreditLedgerAppendRequest {
                event_type: TraceCreditLedgerEventType::ReviewerBonus,
                credit_points_delta: 1.25,
                reason: Some("reconciliation coverage credit".to_string()),
                external_ref: None,
            }),
        )
        .await
        .expect("reviewer can append delayed credit before reconciliation");
        assert_eq!(delayed_credit.submission_id, submission_id);

        let Json(reconciliation_response) = maintenance_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            Json(TraceMaintenanceRequest {
                purpose: Some("test_db_reconciliation".to_string()),
                dry_run: true,
                backfill_db_mirror: false,
                index_vectors: false,
                reconcile_db_mirror: true,
                prune_export_cache: false,
                max_export_age_hours: None,
                purge_expired_before: None,
            }),
        )
        .await
        .expect("reviewer can reconcile DB mirror");
        let reconciliation = reconciliation_response
            .db_reconciliation
            .expect("reconciliation report exists");
        assert_eq!(reconciliation.file_submission_count, 1);
        assert_eq!(reconciliation.db_submission_count, 1);
        assert!(reconciliation.missing_submission_ids_in_db.is_empty());
        assert!(reconciliation.status_mismatches.is_empty());
        assert_eq!(reconciliation.db_object_ref_count, 1);
        assert!(reconciliation.file_credit_event_count >= 1);
        assert!(reconciliation.db_credit_event_count >= 1);
        assert!(reconciliation.file_audit_event_count >= 1);
        assert!(reconciliation.db_audit_event_count >= 1);
        assert_eq!(reconciliation.file_replay_export_manifest_count, 0);
        assert_eq!(reconciliation.db_export_manifest_count, 0);
        assert_eq!(reconciliation.db_export_manifest_item_count, 0);
        assert_eq!(reconciliation.file_revocation_tombstone_count, 0);
        assert_eq!(reconciliation.db_tombstone_count, 0);
        assert!(
            reconciliation.contributor_credit_reader_parity_ok,
            "{:?}",
            reconciliation.db_reader_parity_failures
        );
        assert!(
            reconciliation.reviewer_metadata_reader_parity_ok,
            "{:?}",
            reconciliation.db_reader_parity_failures
        );
        assert!(
            reconciliation.analytics_reader_parity_ok,
            "{:?}",
            reconciliation.db_reader_parity_failures
        );
        assert!(
            reconciliation.audit_reader_parity_ok,
            "{:?}",
            reconciliation.db_reader_parity_failures
        );
        assert!(
            reconciliation.replay_export_manifest_reader_parity_ok,
            "{:?}",
            reconciliation.db_reader_parity_failures
        );
        assert!(reconciliation.db_reader_parity_failures.is_empty());
        assert!(
            reconciliation
                .accepted_without_active_envelope_object_ref
                .is_empty()
        );
        assert_eq!(reconciliation.active_vector_entries, 1);
        assert_eq!(reconciliation.invalid_active_vector_entries, 0);

        let revoke_status = revoke_trace_handler(
            State(state.clone()),
            auth_headers("token-a"),
            AxumPath(submission_id),
        )
        .await
        .expect("contributor can revoke indexed trace");
        assert_eq!(revoke_status, StatusCode::NO_CONTENT);
        let vector_entries = db
            .list_trace_vector_entries("tenant-a")
            .await
            .expect("invalidated vector entries read");
        assert_eq!(
            vector_entries[0].status,
            StorageTraceVectorEntryStatus::Invalidated
        );

        let Json(second_response) = maintenance_handler(
            State(state),
            auth_headers("review-token-a"),
            Json(TraceMaintenanceRequest {
                purpose: Some("test_db_backfill_idempotent".to_string()),
                dry_run: false,
                backfill_db_mirror: true,
                index_vectors: false,
                reconcile_db_mirror: false,
                prune_export_cache: false,
                max_export_age_hours: None,
                purge_expired_before: None,
            }),
        )
        .await
        .expect("backfill can be rerun");
        assert_eq!(second_response.db_mirror_backfilled, 0);
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn reviewer_metadata_reads_can_use_db_mirror_without_file_records() {
        use ironclaw::db::libsql::LibSqlBackend;

        let temp = tempfile::tempdir().expect("temp dir");
        let db_temp = tempfile::tempdir().expect("db temp dir");
        let db_path = db_temp.path().join("trace-mirror-reviewer-reads.db");
        let db = Arc::new(
            LibSqlBackend::new_local(&db_path)
                .await
                .expect("create libsql mirror"),
        );
        db.run_migrations().await.expect("run migrations");

        let accepted_id = Uuid::new_v4();
        let accepted_trace_id = Uuid::new_v4();
        let mut accepted_redactions = BTreeMap::new();
        accepted_redactions.insert("secret".to_string(), 1);
        db.upsert_trace_submission(StorageTraceSubmissionWrite {
            tenant_id: "tenant-a".to_string(),
            submission_id: accepted_id,
            trace_id: accepted_trace_id,
            auth_principal_ref: principal_storage_ref("token-a"),
            contributor_pseudonym: Some("contributor-a".to_string()),
            submitted_tenant_scope_ref: Some("tenant-scope-a".to_string()),
            schema_version: TRACE_CONTRIBUTION_SCHEMA_VERSION.to_string(),
            consent_policy_version: "2026-04-24".to_string(),
            consent_scopes: vec!["ranking_training".to_string()],
            allowed_uses: vec!["ranking_training".to_string()],
            retention_policy_id: "private_corpus_revocable".to_string(),
            status: StorageTraceCorpusStatus::Accepted,
            privacy_risk: "low".to_string(),
            redaction_pipeline_version: "server-rescrub-v1".to_string(),
            redaction_counts: accepted_redactions,
            redaction_hash: "sha256:accepted-redaction".to_string(),
            canonical_summary_hash: Some("sha256:accepted-summary".to_string()),
            submission_score: Some(0.92),
            credit_points_pending: Some(1.4),
            credit_points_final: None,
            expires_at: None,
        })
        .await
        .expect("accepted submission writes");
        db.append_trace_derived_record(StorageTraceDerivedRecordWrite {
            derived_id: Uuid::new_v4(),
            tenant_id: "tenant-a".to_string(),
            submission_id: accepted_id,
            trace_id: accepted_trace_id,
            status: StorageTraceDerivedStatus::Current,
            worker_kind: StorageTraceWorkerKind::DuplicatePrecheck,
            worker_version: "trace_commons_ingest_v1".to_string(),
            input_object_ref: None,
            input_hash: "sha256:accepted-input".to_string(),
            output_object_ref: None,
            canonical_summary: Some("Accepted DB-only trace summary.".to_string()),
            canonical_summary_hash: Some("sha256:accepted-summary".to_string()),
            summary_model: "db-summary-v1".to_string(),
            task_success: Some("success".to_string()),
            privacy_risk: Some("low".to_string()),
            event_count: Some(4),
            tool_sequence: vec!["shell".to_string()],
            tool_categories: vec!["filesystem".to_string()],
            coverage_tags: vec!["tool:shell".to_string(), "privacy:low".to_string()],
            duplicate_score: Some(0.2),
            novelty_score: Some(0.8),
            cluster_id: Some("cluster:db-only".to_string()),
        })
        .await
        .expect("accepted derived record writes");

        let quarantined_id = Uuid::new_v4();
        let quarantined_trace_id = Uuid::new_v4();
        let mut quarantined_redactions = BTreeMap::new();
        quarantined_redactions.insert("private_email".to_string(), 2);
        db.upsert_trace_submission(StorageTraceSubmissionWrite {
            tenant_id: "tenant-a".to_string(),
            submission_id: quarantined_id,
            trace_id: quarantined_trace_id,
            auth_principal_ref: principal_storage_ref("token-a"),
            contributor_pseudonym: Some("contributor-a".to_string()),
            submitted_tenant_scope_ref: Some("tenant-scope-a".to_string()),
            schema_version: TRACE_CONTRIBUTION_SCHEMA_VERSION.to_string(),
            consent_policy_version: "2026-04-24".to_string(),
            consent_scopes: vec!["debugging_evaluation".to_string()],
            allowed_uses: vec!["debugging_evaluation".to_string()],
            retention_policy_id: "private_corpus_revocable".to_string(),
            status: StorageTraceCorpusStatus::Quarantined,
            privacy_risk: "medium".to_string(),
            redaction_pipeline_version: "server-rescrub-v1".to_string(),
            redaction_counts: quarantined_redactions,
            redaction_hash: "sha256:quarantined-redaction".to_string(),
            canonical_summary_hash: Some("sha256:quarantined-summary".to_string()),
            submission_score: Some(0.35),
            credit_points_pending: Some(0.0),
            credit_points_final: None,
            expires_at: None,
        })
        .await
        .expect("quarantined submission writes");
        db.append_trace_derived_record(StorageTraceDerivedRecordWrite {
            derived_id: Uuid::new_v4(),
            tenant_id: "tenant-a".to_string(),
            submission_id: quarantined_id,
            trace_id: quarantined_trace_id,
            status: StorageTraceDerivedStatus::Current,
            worker_kind: StorageTraceWorkerKind::DuplicatePrecheck,
            worker_version: "trace_commons_ingest_v1".to_string(),
            input_object_ref: None,
            input_hash: "sha256:quarantined-input".to_string(),
            output_object_ref: None,
            canonical_summary: Some("Quarantined DB-only trace summary.".to_string()),
            canonical_summary_hash: Some("sha256:quarantined-summary".to_string()),
            summary_model: "db-summary-v1".to_string(),
            task_success: Some("partial".to_string()),
            privacy_risk: Some("medium".to_string()),
            event_count: Some(2),
            tool_sequence: vec!["calendar_create".to_string()],
            tool_categories: vec!["calendar".to_string()],
            coverage_tags: vec![
                "tool:calendar_create".to_string(),
                "privacy:medium".to_string(),
            ],
            duplicate_score: Some(0.4),
            novelty_score: Some(0.6),
            cluster_id: Some("cluster:db-review".to_string()),
        })
        .await
        .expect("quarantined derived record writes");

        let state = test_state_with_db_reviewer_reads(
            temp.path().to_path_buf(),
            Some(db as Arc<dyn Database>),
        );

        let Json(analytics) =
            analytics_handler(State(state.clone()), auth_headers("review-token-a"))
                .await
                .expect("analytics can read DB mirror");
        assert_eq!(analytics.submissions_total, 2);
        assert_eq!(analytics.by_tool.get("shell"), Some(&1));
        assert_eq!(
            analytics.by_tool_category.get("calendar"),
            Some(&1),
            "derived tool categories should come from DB"
        );

        let Json(list) = list_traces_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            Query(TraceListQuery {
                status: Some(TraceCorpusStatus::Accepted),
                limit: Some(10),
                coverage_tag: Some("tool:shell".to_string()),
                tool: Some("shell".to_string()),
                privacy_risk: Some(ResidualPiiRisk::Low),
                consent_scope: Some("ranking-training".to_string()),
            }),
        )
        .await
        .expect("trace list can read DB mirror");
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].submission_id, accepted_id);
        assert_eq!(list[0].redaction_counts.get("secret"), Some(&1));

        let Json(queue) =
            review_quarantine_handler(State(state.clone()), auth_headers("review-token-a"))
                .await
                .expect("quarantine queue can read DB mirror");
        assert_eq!(queue.len(), 1);
        assert_eq!(queue[0].submission_id, quarantined_id);
        assert_eq!(queue[0].redaction_counts.get("private_email"), Some(&2));

        let Json(benchmark) = benchmark_convert_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            Json(BenchmarkConversionRequest {
                limit: Some(10),
                purpose: Some("db_metadata_benchmark".to_string()),
                consent_scope: None,
                status: Some(TraceCorpusStatus::Accepted),
                privacy_risk: Some(ResidualPiiRisk::Low),
                external_ref: None,
            }),
        )
        .await
        .expect("benchmark conversion can read DB metadata");
        assert_eq!(benchmark.item_count, 1);
        assert_eq!(benchmark.candidates[0].summary_model, "db-summary-v1");

        let Json(candidates) = ranker_training_candidates_handler(
            State(state),
            auth_headers("review-token-a"),
            Query(RankerTrainingExportQuery {
                limit: Some(10),
                consent_scope: Some("ranking-training".to_string()),
                privacy_risk: Some(ResidualPiiRisk::Low),
            }),
        )
        .await
        .expect("ranker candidates can read DB metadata");
        assert_eq!(candidates.item_count, 1);
        assert_eq!(candidates.candidates[0].submission_id, accepted_id);
        assert_eq!(candidates.candidates[0].tool_sequence, vec!["shell"]);
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn replay_export_can_select_from_db_mirror_without_file_metadata() {
        use ironclaw::db::libsql::LibSqlBackend;

        let temp = tempfile::tempdir().expect("temp dir");
        let db_temp = tempfile::tempdir().expect("db temp dir");
        let db_path = db_temp.path().join("trace-mirror-replay-export.db");
        let db = Arc::new(
            LibSqlBackend::new_local(&db_path)
                .await
                .expect("create libsql mirror"),
        );
        db.run_migrations().await.expect("run migrations");
        let submit_state = test_state_with_db(
            temp.path().to_path_buf(),
            Some(db.clone() as Arc<dyn Database>),
        );
        let mut envelope = sample_envelope().await;
        make_metadata_only_low_risk(&mut envelope);
        envelope.replay.replayable = true;
        envelope.replay.required_tools.push("shell".to_string());
        let submission_id = envelope.submission_id;

        let Json(receipt) =
            submit_trace_handler(State(submit_state), auth_headers("token-a"), Json(envelope))
                .await
                .expect("submission dual-writes to DB mirror");
        assert_eq!(receipt.status, "accepted");

        let tenant_dir = temp
            .path()
            .join("tenants")
            .join(tenant_storage_key("tenant-a"));
        std::fs::remove_dir_all(tenant_dir.join("metadata")).expect("remove file metadata");
        std::fs::remove_dir_all(tenant_dir.join("derived")).expect("remove derived metadata");

        let replay_state =
            test_state_with_db_replay_export_reads(temp.path().to_path_buf(), Some(db));
        let Json(export) = dataset_replay_handler(
            State(replay_state),
            auth_headers("review-token-a"),
            Query(DatasetExportQuery {
                limit: Some(10),
                purpose: Some("db_replay_export".to_string()),
                status: Some(TraceCorpusStatus::Accepted),
                privacy_risk: Some(ResidualPiiRisk::Low),
                consent_scope: None,
            }),
        )
        .await
        .expect("replay export can select DB metadata and read envelope object");
        assert_eq!(export.item_count, 1);
        assert_eq!(export.items[0].submission_id, submission_id);
        assert_eq!(export.items[0].required_tools, vec!["shell"]);
        assert!(export.items[0].canonical_summary_hash.is_some());
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn replay_export_can_read_encrypted_artifact_from_db_object_ref_without_file_objects() {
        use ironclaw::db::libsql::LibSqlBackend;

        let temp = tempfile::tempdir().expect("temp dir");
        let artifact_root = temp.path().join("encrypted-artifacts");
        let artifact_store = test_artifact_store(&artifact_root);
        let db_temp = tempfile::tempdir().expect("db temp dir");
        let db_path = db_temp
            .path()
            .join("trace-mirror-replay-export-artifact.db");
        let db = Arc::new(
            LibSqlBackend::new_local(&db_path)
                .await
                .expect("create libsql mirror"),
        );
        db.run_migrations().await.expect("run migrations");
        let submit_state = test_state_with_options(
            temp.path().to_path_buf(),
            Some(db.clone() as Arc<dyn Database>),
            Some(artifact_store.clone()),
            false,
            false,
            false,
            false,
        );
        let mut envelope = sample_envelope().await;
        make_metadata_only_low_risk(&mut envelope);
        envelope.replay.replayable = true;
        envelope.replay.required_tools.push("shell".to_string());
        let submission_id = envelope.submission_id;

        let Json(receipt) =
            submit_trace_handler(State(submit_state), auth_headers("token-a"), Json(envelope))
                .await
                .expect("submission dual-writes to DB mirror and artifact store");
        assert_eq!(receipt.status, "accepted");
        let object_ref = db
            .get_latest_active_trace_object_ref(
                "tenant-a",
                submission_id,
                StorageTraceObjectArtifactKind::SubmittedEnvelope,
            )
            .await
            .expect("object ref reads")
            .expect("submitted envelope object ref exists");
        assert_eq!(
            object_ref.object_store,
            "trace_commons_encrypted_artifact_store"
        );

        let tenant_dir = temp
            .path()
            .join("tenants")
            .join(tenant_storage_key("tenant-a"));
        std::fs::remove_dir_all(tenant_dir.join("metadata")).expect("remove file metadata");
        std::fs::remove_dir_all(tenant_dir.join("derived")).expect("remove derived metadata");
        std::fs::remove_dir_all(tenant_dir.join("objects")).expect("remove plaintext objects");

        let replay_state = test_state_with_options(
            temp.path().to_path_buf(),
            Some(db),
            Some(artifact_store),
            false,
            false,
            true,
            false,
        );
        let Json(export) = dataset_replay_handler(
            State(replay_state),
            auth_headers("review-token-a"),
            Query(DatasetExportQuery {
                limit: Some(10),
                purpose: Some("db_artifact_replay_export".to_string()),
                status: Some(TraceCorpusStatus::Accepted),
                privacy_risk: Some(ResidualPiiRisk::Low),
                consent_scope: None,
            }),
        )
        .await
        .expect("replay export reads encrypted artifact through DB object ref");
        assert_eq!(export.item_count, 1);
        assert_eq!(export.items[0].submission_id, submission_id);
        assert_eq!(export.items[0].required_tools, vec!["shell"]);
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn replay_export_mirrors_manifest_metadata_to_db() {
        use ironclaw::db::libsql::LibSqlBackend;

        let temp = tempfile::tempdir().expect("temp dir");
        let db_temp = tempfile::tempdir().expect("db temp dir");
        let db_path = db_temp.path().join("trace-export-manifests.db");
        let db = Arc::new(
            LibSqlBackend::new_local(&db_path)
                .await
                .expect("create libsql mirror"),
        );
        db.run_migrations().await.expect("run migrations");
        let state = test_state_with_db(
            temp.path().to_path_buf(),
            Some(db.clone() as Arc<dyn Database>),
        );
        let mut envelope = sample_envelope().await;
        make_metadata_only_low_risk(&mut envelope);
        let submission_id = envelope.submission_id;

        let Json(receipt) = submit_trace_handler(
            State(state.clone()),
            auth_headers("token-a"),
            Json(envelope),
        )
        .await
        .expect("submission dual-writes to DB mirror");
        assert_eq!(receipt.status, "accepted");

        let Json(export) = dataset_replay_handler(
            State(state),
            auth_headers("review-token-a"),
            Query(DatasetExportQuery {
                limit: Some(10),
                purpose: Some("db_export_manifest".to_string()),
                status: Some(TraceCorpusStatus::Accepted),
                privacy_risk: Some(ResidualPiiRisk::Low),
                consent_scope: None,
            }),
        )
        .await
        .expect("dataset export succeeds");
        assert_eq!(export.item_count, 1);
        assert_eq!(export.items[0].submission_id, submission_id);

        let manifests = db
            .list_trace_export_manifests("tenant-a")
            .await
            .expect("list export manifest metadata");
        assert_eq!(manifests.len(), 1);
        let manifest = &manifests[0];
        assert_eq!(manifest.tenant_id, "tenant-a");
        assert_eq!(manifest.export_manifest_id, export.export_id);
        assert_eq!(
            manifest.artifact_kind,
            StorageTraceObjectArtifactKind::ExportArtifact
        );
        assert_eq!(manifest.purpose_code.as_deref(), Some("db_export_manifest"));
        assert_eq!(manifest.audit_event_id, Some(export.audit_event_id));
        assert_eq!(manifest.source_submission_ids, vec![submission_id]);
        assert_eq!(
            manifest.source_submission_ids_hash,
            export.manifest.source_submission_ids_hash
        );
        assert_eq!(manifest.item_count, 1);
        assert!(manifest.invalidated_at.is_none());
        assert!(manifest.deleted_at.is_none());

        let other_tenant_manifests = db
            .list_trace_export_manifests("tenant-b")
            .await
            .expect("list other tenant export manifest metadata");
        assert!(other_tenant_manifests.is_empty());
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn revocation_invalidates_db_export_manifest_metadata() {
        use ironclaw::db::libsql::LibSqlBackend;

        let temp = tempfile::tempdir().expect("temp dir");
        let db_temp = tempfile::tempdir().expect("db temp dir");
        let db_path = db_temp.path().join("trace-export-manifest-revocation.db");
        let db = Arc::new(
            LibSqlBackend::new_local(&db_path)
                .await
                .expect("create libsql mirror"),
        );
        db.run_migrations().await.expect("run migrations");
        let state = test_state_with_db(
            temp.path().to_path_buf(),
            Some(db.clone() as Arc<dyn Database>),
        );
        let mut envelope = sample_envelope().await;
        make_metadata_only_low_risk(&mut envelope);
        let submission_id = envelope.submission_id;

        let _ = submit_trace_handler(
            State(state.clone()),
            auth_headers("token-a"),
            Json(envelope),
        )
        .await
        .expect("submission dual-writes to DB mirror");
        let Json(export) = dataset_replay_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            Query(DatasetExportQuery {
                limit: Some(10),
                purpose: Some("revocation_manifest".to_string()),
                status: Some(TraceCorpusStatus::Accepted),
                privacy_risk: Some(ResidualPiiRisk::Low),
                consent_scope: None,
            }),
        )
        .await
        .expect("dataset export succeeds");
        assert_eq!(export.item_count, 1);

        revoke_trace_handler(
            State(state),
            auth_headers("token-a"),
            AxumPath(submission_id),
        )
        .await
        .expect("contributor can revoke own trace");

        let manifests = db
            .list_trace_export_manifests("tenant-a")
            .await
            .expect("list export manifest metadata");
        assert_eq!(manifests.len(), 1);
        assert_eq!(manifests[0].export_manifest_id, export.export_id);
        assert!(manifests[0].invalidated_at.is_some());
        assert!(manifests[0].deleted_at.is_none());
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn replay_export_mirrors_item_metadata_and_revocation_invalidates_items() {
        use ironclaw::db::libsql::LibSqlBackend;

        let temp = tempfile::tempdir().expect("temp dir");
        let db_temp = tempfile::tempdir().expect("db temp dir");
        let db_path = db_temp.path().join("trace-export-manifest-items.db");
        let db = Arc::new(
            LibSqlBackend::new_local(&db_path)
                .await
                .expect("create libsql mirror"),
        );
        db.run_migrations().await.expect("run migrations");
        let state = test_state_with_db_replay_export_reads(
            temp.path().to_path_buf(),
            Some(db.clone() as Arc<dyn Database>),
        );
        let mut envelope = sample_envelope().await;
        make_metadata_only_low_risk(&mut envelope);
        let submission_id = envelope.submission_id;
        let trace_id = envelope.trace_id;

        let _ = submit_trace_handler(
            State(state.clone()),
            auth_headers("token-a"),
            Json(envelope),
        )
        .await
        .expect("submission dual-writes to DB mirror");
        let Json(export) = dataset_replay_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            Query(DatasetExportQuery {
                limit: Some(10),
                purpose: Some("item_manifest".to_string()),
                status: Some(TraceCorpusStatus::Accepted),
                privacy_risk: Some(ResidualPiiRisk::Low),
                consent_scope: None,
            }),
        )
        .await
        .expect("dataset export succeeds");
        assert_eq!(export.item_count, 1);
        let exported_item = &export.items[0];
        let expected_source_hash = exported_item
            .canonical_summary_hash
            .clone()
            .expect("accepted test trace has derived canonical hash");
        let active_object_ref = db
            .get_latest_active_trace_object_ref(
                "tenant-a",
                submission_id,
                StorageTraceObjectArtifactKind::SubmittedEnvelope,
            )
            .await
            .expect("active object ref reads")
            .expect("active submitted envelope object ref exists");

        let items = db
            .list_trace_export_manifest_items("tenant-a", export.export_id)
            .await
            .expect("list export manifest item metadata");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].tenant_id, "tenant-a");
        assert_eq!(items[0].export_manifest_id, export.export_id);
        assert_eq!(items[0].submission_id, submission_id);
        assert_eq!(items[0].trace_id, trace_id);
        assert_eq!(
            items[0].source_status_at_export,
            StorageTraceCorpusStatus::Accepted
        );
        assert_eq!(
            items[0].object_ref_id,
            Some(active_object_ref.object_ref_id)
        );
        assert_eq!(items[0].source_hash_at_export, expected_source_hash);
        assert!(items[0].source_invalidated_at.is_none());
        assert!(items[0].source_invalidation_reason.is_none());
        assert!(
            db.list_trace_export_manifest_items("tenant-b", export.export_id)
                .await
                .expect("list other tenant item metadata")
                .is_empty()
        );

        revoke_trace_handler(
            State(state),
            auth_headers("token-a"),
            AxumPath(submission_id),
        )
        .await
        .expect("contributor can revoke own trace");

        let items = db
            .list_trace_export_manifest_items("tenant-a", export.export_id)
            .await
            .expect("list invalidated export manifest item metadata");
        assert_eq!(items.len(), 1);
        assert!(items[0].source_invalidated_at.is_some());
        assert_eq!(
            items[0].source_invalidation_reason,
            Some(StorageTraceExportManifestItemInvalidationReason::Revoked)
        );
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn reviewer_can_list_db_export_manifest_metadata() {
        use ironclaw::db::libsql::LibSqlBackend;

        let temp = tempfile::tempdir().expect("temp dir");
        let db_temp = tempfile::tempdir().expect("db temp dir");
        let db_path = db_temp.path().join("trace-export-manifest-list.db");
        let db = Arc::new(
            LibSqlBackend::new_local(&db_path)
                .await
                .expect("create libsql mirror"),
        );
        db.run_migrations().await.expect("run migrations");
        let state = test_state_with_db(
            temp.path().to_path_buf(),
            Some(db.clone() as Arc<dyn Database>),
        );
        let mut envelope = sample_envelope().await;
        make_metadata_only_low_risk(&mut envelope);
        envelope.consent.scopes = vec![
            ConsentScope::DebuggingEvaluation,
            ConsentScope::RankingTraining,
        ];
        let submission_id = envelope.submission_id;

        let _ = submit_trace_handler(
            State(state.clone()),
            auth_headers("token-a"),
            Json(envelope),
        )
        .await
        .expect("submission dual-writes to DB mirror");
        let Json(export) = dataset_replay_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            Query(DatasetExportQuery {
                limit: Some(10),
                purpose: Some("manifest_listing".to_string()),
                status: Some(TraceCorpusStatus::Accepted),
                privacy_risk: Some(ResidualPiiRisk::Low),
                consent_scope: None,
            }),
        )
        .await
        .expect("dataset export succeeds");

        let Json(benchmark) = benchmark_convert_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            Json(BenchmarkConversionRequest {
                limit: Some(10),
                purpose: Some("manifest_listing_benchmark".to_string()),
                consent_scope: None,
                status: Some(TraceCorpusStatus::Accepted),
                privacy_risk: Some(ResidualPiiRisk::Low),
                external_ref: None,
            }),
        )
        .await
        .expect("benchmark conversion succeeds");
        assert_eq!(benchmark.item_count, 1);

        let Json(ranker) = ranker_training_candidates_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            Query(RankerTrainingExportQuery {
                limit: Some(10),
                consent_scope: None,
                privacy_risk: Some(ResidualPiiRisk::Low),
            }),
        )
        .await
        .expect("ranker export succeeds");
        assert_eq!(ranker.item_count, 1);

        assert_eq!(
            db.list_trace_export_manifests("tenant-a")
                .await
                .expect("all export manifest metadata reads")
                .len(),
            3,
            "DB stores replay, benchmark, and ranker provenance manifests"
        );
        let derived_id = db
            .list_trace_derived_records("tenant-a")
            .await
            .expect("derived metadata reads")
            .into_iter()
            .find(|record| record.submission_id == submission_id)
            .expect("derived metadata exists")
            .derived_id;
        let benchmark_items = db
            .list_trace_export_manifest_items("tenant-a", benchmark.conversion_id)
            .await
            .expect("benchmark provenance items read");
        assert_eq!(benchmark_items.len(), 1);
        assert_eq!(benchmark_items[0].derived_id, Some(derived_id));
        let ranker_items = db
            .list_trace_export_manifest_items("tenant-a", ranker.export_id)
            .await
            .expect("ranker provenance items read");
        assert_eq!(ranker_items.len(), 1);
        assert_eq!(ranker_items[0].derived_id, Some(derived_id));

        let Json(manifests) =
            replay_export_manifests_handler(State(state.clone()), auth_headers("review-token-a"))
                .await
                .expect("reviewer can list export manifests");
        assert_eq!(manifests.len(), 1);
        assert_eq!(manifests[0].export_manifest_id, export.export_id);
        assert_eq!(
            manifests[0].purpose_code.as_deref(),
            Some("manifest_listing")
        );
        assert_eq!(manifests[0].source_submission_ids, vec![submission_id]);
        assert_eq!(
            manifests[0].source_submission_ids_hash,
            export.manifest.source_submission_ids_hash
        );
        assert_eq!(manifests[0].item_count, 1);
        assert!(manifests[0].invalidated_at.is_none());
        let audit_events =
            read_all_audit_events(temp.path(), "tenant-a").expect("audit events read");
        assert!(audit_events.iter().any(|event| {
            event.kind == "read"
                && event.reason.as_deref() == Some("surface=replay_export_manifests;item_count=1")
        }));

        let contributor_error =
            replay_export_manifests_handler(State(state.clone()), auth_headers("token-a"))
                .await
                .expect_err("contributor cannot list export manifests");
        assert_eq!(contributor_error.0, StatusCode::FORBIDDEN);

        revoke_trace_handler(
            State(state.clone()),
            auth_headers("token-a"),
            AxumPath(submission_id),
        )
        .await
        .expect("contributor can revoke own trace");

        let Json(manifests) =
            replay_export_manifests_handler(State(state), auth_headers("review-token-a"))
                .await
                .expect("reviewer can list invalidated export manifests");
        assert_eq!(manifests.len(), 1);
        assert_eq!(manifests[0].export_manifest_id, export.export_id);
        assert!(manifests[0].invalidated_at.is_some());
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn replay_export_uses_db_object_ref_after_review_status_change() {
        use ironclaw::db::libsql::LibSqlBackend;

        let temp = tempfile::tempdir().expect("temp dir");
        let db_temp = tempfile::tempdir().expect("db temp dir");
        let db_path = db_temp
            .path()
            .join("trace-mirror-replay-export-reviewed.db");
        let db = Arc::new(
            LibSqlBackend::new_local(&db_path)
                .await
                .expect("create libsql mirror"),
        );
        db.run_migrations().await.expect("run migrations");
        let state = test_state_with_db(
            temp.path().to_path_buf(),
            Some(db.clone() as Arc<dyn Database>),
        );
        let mut envelope = sample_envelope().await;
        envelope.replay.replayable = true;
        envelope
            .replay
            .required_tools
            .push("calendar_create".to_string());
        let submission_id = envelope.submission_id;

        let Json(receipt) = submit_trace_handler(
            State(state.clone()),
            auth_headers("token-a"),
            Json(envelope),
        )
        .await
        .expect("quarantined submission dual-writes");
        assert_eq!(receipt.status, "quarantined");
        let Json(review_receipt) = review_decision_handler(
            State(state),
            auth_headers("review-token-a"),
            AxumPath(submission_id),
            Json(TraceReviewDecisionRequest {
                decision: TraceReviewDecision::Approve,
                reason: Some("approved for replay export".to_string()),
                credit_points_pending: Some(1.0),
            }),
        )
        .await
        .expect("reviewer approves quarantined trace");
        assert_eq!(review_receipt.status, "accepted");

        let tenant_dir = temp
            .path()
            .join("tenants")
            .join(tenant_storage_key("tenant-a"));
        assert!(
            tenant_dir
                .join("objects")
                .join("quarantined")
                .join(format!("{submission_id}.json"))
                .exists(),
            "the envelope object stays under its original quarantine path"
        );
        std::fs::remove_dir_all(tenant_dir.join("metadata")).expect("remove file metadata");
        std::fs::remove_dir_all(tenant_dir.join("derived")).expect("remove derived metadata");

        let replay_state =
            test_state_with_db_replay_export_reads(temp.path().to_path_buf(), Some(db));
        let Json(export) = dataset_replay_handler(
            State(replay_state),
            auth_headers("review-token-a"),
            Query(DatasetExportQuery {
                limit: Some(10),
                purpose: Some("db_reviewed_replay_export".to_string()),
                status: Some(TraceCorpusStatus::Accepted),
                privacy_risk: None,
                consent_scope: None,
            }),
        )
        .await
        .expect("replay export resolves DB object ref after review");
        assert_eq!(export.item_count, 1);
        assert_eq!(export.items[0].submission_id, submission_id);
        assert_eq!(export.items[0].required_tools, vec!["calendar_create"]);
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn audit_events_can_read_from_db_mirror_when_enabled() {
        use ironclaw::db::libsql::LibSqlBackend;

        let temp = tempfile::tempdir().expect("temp dir");
        let db_temp = tempfile::tempdir().expect("db temp dir");
        let db_path = db_temp.path().join("trace-mirror-audit-reads.db");
        let db = Arc::new(
            LibSqlBackend::new_local(&db_path)
                .await
                .expect("create libsql mirror"),
        );
        db.run_migrations().await.expect("run migrations");
        let submit_state = test_state_with_db(
            temp.path().to_path_buf(),
            Some(db.clone() as Arc<dyn Database>),
        );
        let mut envelope = sample_envelope().await;
        make_metadata_only_low_risk(&mut envelope);
        let submission_id = envelope.submission_id;

        let _ = submit_trace_handler(State(submit_state), auth_headers("token-a"), Json(envelope))
            .await
            .expect("submission dual-writes audit event");
        let tenant_dir = temp
            .path()
            .join("tenants")
            .join(tenant_storage_key("tenant-a"));
        std::fs::remove_dir_all(tenant_dir.join("audit")).expect("remove file audit events");

        let audit_state = test_state_with_options(
            temp.path().to_path_buf(),
            Some(db.clone() as Arc<dyn Database>),
            None,
            false,
            false,
            true,
            true,
        );
        let Json(list) = list_traces_handler(
            State(audit_state.clone()),
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
        .expect("trace list read mirrors audit event");
        assert_eq!(list.len(), 1);
        let Json(quarantine) =
            review_quarantine_handler(State(audit_state.clone()), auth_headers("review-token-a"))
                .await
                .expect("quarantine read mirrors audit event");
        assert!(quarantine.is_empty());
        let Json(active_learning) = active_learning_review_queue_handler(
            State(audit_state.clone()),
            auth_headers("review-token-a"),
            Query(ActiveLearningQueueQuery {
                limit: Some(10),
                privacy_risk: None,
            }),
        )
        .await
        .expect("active-learning read mirrors audit event");
        assert_eq!(active_learning.item_count, 1);

        let Json(credit_event) = append_credit_event_handler(
            State(audit_state.clone()),
            auth_headers("review-token-a"),
            AxumPath(submission_id),
            Json(TraceCreditLedgerAppendRequest {
                event_type: TraceCreditLedgerEventType::ReviewerBonus,
                credit_points_delta: 0.25,
                reason: Some("useful privacy-safe example".to_string()),
                external_ref: None,
            }),
        )
        .await
        .expect("credit mutation mirrors audit event");
        assert_eq!(credit_event.submission_id, submission_id);

        let Json(export) = dataset_replay_handler(
            State(audit_state.clone()),
            auth_headers("review-token-a"),
            Query(DatasetExportQuery {
                limit: Some(10),
                purpose: Some("db_audit_export".to_string()),
                status: Some(TraceCorpusStatus::Accepted),
                privacy_risk: Some(ResidualPiiRisk::Low),
                consent_scope: None,
            }),
        )
        .await
        .expect("dataset export mirrors audit event");
        assert_eq!(export.item_count, 1);
        assert!(
            export
                .manifest
                .source_submission_ids_hash
                .starts_with("sha256:")
        );
        let active_object_ref = db
            .get_latest_active_trace_object_ref(
                "tenant-a",
                submission_id,
                StorageTraceObjectArtifactKind::SubmittedEnvelope,
            )
            .await
            .expect("active object ref reads")
            .expect("active submitted envelope object ref exists");
        let db_audit_events = db
            .list_trace_audit_events("tenant-a")
            .await
            .expect("audit events read from db");
        assert!(
            db_audit_events.iter().any(|event| {
                event.action == StorageTraceAuditAction::Read
                    && event.submission_id == Some(submission_id)
                    && event.object_ref_id == Some(active_object_ref.object_ref_id)
                    && event.reason.as_deref().is_some_and(|reason| {
                        reason.contains("surface=replay_dataset_export")
                            && reason.contains("purpose=db_audit_export")
                    })
            }),
            "DB content-read audit events should name the object ref that passed the read gate"
        );

        let Json(events) = audit_events_handler(
            State(audit_state),
            auth_headers("review-token-a"),
            Query(AuditEventsQuery { limit: Some(50) }),
        )
        .await
        .expect("audit events can read DB mirror");
        assert!(events.iter().any(|event| {
            event.submission_id == submission_id
                && event.kind == "submitted"
                && event.status == Some(TraceCorpusStatus::Accepted)
        }));
        assert!(events.iter().any(|event| {
            event.kind == "read"
                && event.reason.as_deref() == Some("surface=trace_list;item_count=1")
        }));
        assert!(events.iter().any(|event| {
            event.kind == "read"
                && event.reason.as_deref() == Some("surface=review_quarantine;item_count=0")
        }));
        assert!(events.iter().any(|event| {
            event.kind == "read"
                && event.reason.as_deref()
                    == Some("surface=active_learning_review_queue;item_count=1")
        }));
        assert!(events.iter().any(|event| {
            event.submission_id == submission_id && event.kind == "credit_mutate"
        }));
        assert!(events.iter().any(|event| {
            event.export_id == Some(export.export_id)
                && event.kind == "dataset_export"
                && event.decision_inputs_hash
                    == Some(export.manifest.source_submission_ids_hash.clone())
        }));
        assert!(events.iter().any(|event| {
            event.submission_id == submission_id
                && event.kind == "trace_content_read"
                && event.reason.as_deref().is_some_and(|reason| {
                    reason.contains("surface=replay_dataset_export")
                        && reason.contains("purpose=db_audit_export")
                })
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
    async fn delayed_credit_requires_reason_artifact_ref_and_bounded_delta() {
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

        let missing_reason = append_credit_event_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            AxumPath(submission_id),
            Json(TraceCreditLedgerAppendRequest {
                event_type: TraceCreditLedgerEventType::ReviewerBonus,
                credit_points_delta: 1.0,
                reason: Some(" ".to_string()),
                external_ref: None,
            }),
        )
        .await
        .expect_err("delayed credit requires a reason");
        assert_eq!(missing_reason.0, StatusCode::BAD_REQUEST);

        let missing_artifact_ref = append_credit_event_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            AxumPath(submission_id),
            Json(TraceCreditLedgerAppendRequest {
                event_type: TraceCreditLedgerEventType::TrainingUtility,
                credit_points_delta: 1.0,
                reason: Some("training job improved ranker".to_string()),
                external_ref: None,
            }),
        )
        .await
        .expect_err("utility credit requires artifact reference");
        assert_eq!(missing_artifact_ref.0, StatusCode::BAD_REQUEST);

        let excessive_delta = append_credit_event_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            AxumPath(submission_id),
            Json(TraceCreditLedgerAppendRequest {
                event_type: TraceCreditLedgerEventType::ReviewerBonus,
                credit_points_delta: 101.0,
                reason: Some("too much at once".to_string()),
                external_ref: None,
            }),
        )
        .await
        .expect_err("delayed credit delta is bounded");
        assert_eq!(excessive_delta.0, StatusCode::BAD_REQUEST);
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
                reason: Some("training utility job selected this trace".to_string()),
                external_ref: Some("training-job:summary-ranker-smoke".to_string()),
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
                reason: Some("caught regression in replay suite".to_string()),
                external_ref: Some("regression:trace-replay-smoke".to_string()),
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
