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
    TraceAllowedUse, TraceContributionEnvelope, TraceSubmissionReceipt,
    TraceSubmissionStatusRequest, TraceSubmissionStatusUpdate, apply_credit_estimate_to_envelope,
    canonical_summary_for_embedding, rescrub_trace_envelope, retention_policy_for_trace,
};
use ironclaw::trace_corpus_storage::{
    TraceArtifactInvalidationCounts as StorageTraceArtifactInvalidationCounts,
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
    TraceTenantPolicyRecord as StorageTraceTenantPolicyRecord,
    TraceTenantPolicyWrite as StorageTraceTenantPolicyWrite,
    TraceTombstoneRecord as StorageTraceTombstoneRecord,
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
const TRACE_COMMONS_FILE_OBJECT_STORE: &str = "trace_commons_file_store";
const TRACE_COMMONS_LEGACY_ENCRYPTED_OBJECT_STORE: &str = "trace_commons_encrypted_artifact_store";
const TRACE_COMMONS_SERVICE_LOCAL_ENCRYPTED_OBJECT_STORE: &str =
    "trace_commons_service_local_encrypted";
const TRACE_COMMONS_OBJECT_PRIMARY_SUBMIT_REVIEW: &str =
    "TRACE_COMMONS_OBJECT_PRIMARY_SUBMIT_REVIEW";
const TRACE_COMMONS_OBJECT_PRIMARY_REPLAY_EXPORT: &str =
    "TRACE_COMMONS_OBJECT_PRIMARY_REPLAY_EXPORT";
const TRACE_COMMONS_OBJECT_PRIMARY_DERIVED_EXPORTS: &str =
    "TRACE_COMMONS_OBJECT_PRIMARY_DERIVED_EXPORTS";
const TRACE_COMMONS_REQUIRE_DB_RECONCILIATION_CLEAN: &str =
    "TRACE_COMMONS_REQUIRE_DB_RECONCILIATION_CLEAN";
const TRACE_COMMONS_LEGAL_HOLD_RETENTION_POLICIES: &str =
    "TRACE_COMMONS_LEGAL_HOLD_RETENTION_POLICIES";
const TRACE_COMMONS_MAX_EXPORT_ITEMS_PER_REQUEST: &str =
    "TRACE_COMMONS_MAX_EXPORT_ITEMS_PER_REQUEST";
const DEFAULT_TRACE_COMMONS_MAX_EXPORT_ITEMS_PER_REQUEST: usize = 500;
const TRACE_COMMONS_MAX_SUBMISSIONS_PER_TENANT_PER_HOUR: &str =
    "TRACE_COMMONS_MAX_SUBMISSIONS_PER_TENANT_PER_HOUR";
const TRACE_COMMONS_MAX_SUBMISSIONS_PER_PRINCIPAL_PER_HOUR: &str =
    "TRACE_COMMONS_MAX_SUBMISSIONS_PER_PRINCIPAL_PER_HOUR";
const TRACE_BACKFILL_FAILURE_DETAIL_LIMIT: usize = 20;

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
    tenant_policies: Arc<BTreeMap<String, TenantSubmissionPolicy>>,
    require_tenant_submission_policy: bool,
    db_mirror: Option<Arc<dyn Database>>,
    db_contributor_reads: bool,
    db_reviewer_reads: bool,
    db_reviewer_require_object_refs: bool,
    db_replay_export_reads: bool,
    db_replay_export_require_object_refs: bool,
    db_audit_reads: bool,
    db_tenant_policy_reads: bool,
    require_db_mirror_writes: bool,
    require_derived_export_object_refs: bool,
    object_primary_submit_review: bool,
    object_primary_replay_export: bool,
    object_primary_derived_exports: bool,
    require_db_reconciliation_clean: bool,
    require_export_guardrails: bool,
    max_export_items_per_request: usize,
    submission_quota: TraceSubmissionQuotaConfig,
    legal_hold_retention_policy_ids: Arc<BTreeSet<String>>,
    artifact_store: Option<ConfiguredTraceArtifactStore>,
}

#[derive(Debug, Clone, Copy, Default, Serialize)]
struct TraceSubmissionQuotaConfig {
    max_per_tenant_per_hour: usize,
    max_per_principal_per_hour: usize,
}

impl TraceSubmissionQuotaConfig {
    fn is_disabled(self) -> bool {
        self.max_per_tenant_per_hour == 0 && self.max_per_principal_per_hour == 0
    }
}

#[derive(Clone)]
struct ConfiguredTraceArtifactStore {
    object_store_name: String,
    store: Arc<LocalEncryptedTraceArtifactStore>,
}

impl ConfiguredTraceArtifactStore {
    fn new(
        object_store_name: impl Into<String>,
        store: Arc<LocalEncryptedTraceArtifactStore>,
    ) -> Self {
        Self {
            object_store_name: object_store_name.into(),
            store,
        }
    }

    #[cfg(test)]
    fn legacy(store: Arc<LocalEncryptedTraceArtifactStore>) -> Self {
        Self::new(TRACE_COMMONS_LEGACY_ENCRYPTED_OBJECT_STORE, store)
    }

    fn object_store_name(&self) -> &str {
        &self.object_store_name
    }

    fn put_json<T: Serialize>(
        &self,
        tenant_storage_ref: &str,
        artifact_kind: TraceArtifactKind,
        object_id: &str,
        value: &T,
    ) -> anyhow::Result<EncryptedTraceArtifactReceipt> {
        self.store
            .put_json(tenant_storage_ref, artifact_kind, object_id, value)
    }

    fn get_json<T: DeserializeOwned>(
        &self,
        expected_tenant_storage_ref: &str,
        receipt: &EncryptedTraceArtifactReceipt,
    ) -> anyhow::Result<T> {
        self.store.get_json(expected_tenant_storage_ref, receipt)
    }

    fn get_json_by_object_key<T: DeserializeOwned>(
        &self,
        expected_tenant_storage_ref: &str,
        expected_artifact_kind: TraceArtifactKind,
        object_key: &str,
        expected_ciphertext_sha256: &str,
    ) -> anyhow::Result<T> {
        self.store.get_json_by_object_key(
            expected_tenant_storage_ref,
            expected_artifact_kind,
            object_key,
            expected_ciphertext_sha256,
        )
    }

    fn delete_artifact(
        &self,
        expected_tenant_storage_ref: &str,
        receipt: &EncryptedTraceArtifactReceipt,
    ) -> anyhow::Result<bool> {
        self.store
            .delete_artifact(expected_tenant_storage_ref, receipt)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TraceEncryptedObjectStoreKind {
    LegacyArtifactSidecar,
    ServiceLocal,
}

impl TraceEncryptedObjectStoreKind {
    fn from_config(
        raw_mode: Option<&str>,
        encrypted_artifacts_requested: bool,
    ) -> anyhow::Result<Option<Self>> {
        let Some(raw_mode) = raw_mode else {
            return Ok(encrypted_artifacts_requested.then_some(Self::LegacyArtifactSidecar));
        };
        let normalized = raw_mode.trim().to_ascii_lowercase();
        match normalized.as_str() {
            "" => Ok(encrypted_artifacts_requested.then_some(Self::LegacyArtifactSidecar)),
            "file" | "none" => Ok(None),
            "local_encrypted" | "encrypted_artifact" => Ok(Some(Self::LegacyArtifactSidecar)),
            "local_service" | "local_service_encrypted" | "service_local_encrypted" => {
                Ok(Some(Self::ServiceLocal))
            }
            other => anyhow::bail!("unsupported TRACE_COMMONS_OBJECT_STORE value: {other}"),
        }
    }

    fn object_store_name(self) -> &'static str {
        match self {
            Self::LegacyArtifactSidecar => TRACE_COMMONS_LEGACY_ENCRYPTED_OBJECT_STORE,
            Self::ServiceLocal => TRACE_COMMONS_SERVICE_LOCAL_ENCRYPTED_OBJECT_STORE,
        }
    }

    fn root_from_env(self, default_root: &Path) -> PathBuf {
        match self {
            Self::LegacyArtifactSidecar => std::env::var("TRACE_COMMONS_ARTIFACT_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|_| default_root.join("encrypted_artifacts")),
            Self::ServiceLocal => std::env::var("TRACE_COMMONS_SERVICE_OBJECT_STORE_DIR")
                .or_else(|_| std::env::var("TRACE_COMMONS_ARTIFACT_DIR"))
                .map(PathBuf::from)
                .unwrap_or_else(|_| default_root.join("service_object_store")),
        }
    }
}

#[derive(Debug, Clone)]
struct TenantAuth {
    tenant_id: String,
    role: TokenRole,
    principal_ref: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct TenantSubmissionPolicy {
    #[serde(default)]
    allowed_consent_scopes: BTreeSet<ConsentScope>,
    #[serde(default)]
    allowed_uses: BTreeSet<TraceAllowedUse>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum TokenRole {
    Contributor,
    Reviewer,
    Admin,
    ExportWorker,
    RetentionWorker,
    VectorWorker,
    BenchmarkWorker,
    UtilityWorker,
}

impl TokenRole {
    fn parse(raw: &str) -> anyhow::Result<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "contributor" => Ok(Self::Contributor),
            "reviewer" => Ok(Self::Reviewer),
            "admin" => Ok(Self::Admin),
            "export_worker" | "export-worker" => Ok(Self::ExportWorker),
            "retention_worker" | "retention-worker" => Ok(Self::RetentionWorker),
            "vector_worker" | "vector-worker" => Ok(Self::VectorWorker),
            "benchmark_worker" | "benchmark-worker" => Ok(Self::BenchmarkWorker),
            "utility_worker" | "utility-worker" => Ok(Self::UtilityWorker),
            other => anyhow::bail!("unknown Trace Commons token role: {other}"),
        }
    }

    fn can_review(self) -> bool {
        matches!(self, Self::Reviewer | Self::Admin)
    }

    fn can_export(self) -> bool {
        matches!(self, Self::Reviewer | Self::Admin | Self::ExportWorker)
    }

    fn can_benchmark(self) -> bool {
        matches!(self, Self::Reviewer | Self::Admin | Self::BenchmarkWorker)
    }

    fn can_admin(self) -> bool {
        matches!(self, Self::Admin)
    }

    fn storage_name(self) -> &'static str {
        match self {
            Self::Contributor => "contributor",
            Self::Reviewer => "reviewer",
            Self::Admin => "admin",
            Self::ExportWorker => "export_worker",
            Self::RetentionWorker => "retention_worker",
            Self::VectorWorker => "vector_worker",
            Self::BenchmarkWorker => "benchmark_worker",
            Self::UtilityWorker => "utility_worker",
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
        let tenant_policies = parse_tenant_submission_policies_from_env()?;
        let require_tenant_submission_policy =
            env_truthy("TRACE_COMMONS_REQUIRE_TENANT_SUBMISSION_POLICY");
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
        let db_reviewer_require_object_refs =
            env_truthy("TRACE_COMMONS_DB_REVIEWER_REQUIRE_OBJECT_REFS");
        if db_reviewer_require_object_refs && !db_reviewer_reads {
            anyhow::bail!(
                "TRACE_COMMONS_DB_REVIEWER_REQUIRE_OBJECT_REFS requires TRACE_COMMONS_DB_REVIEWER_READS"
            );
        }
        let db_replay_export_reads = env_truthy("TRACE_COMMONS_DB_REPLAY_EXPORT_READS");
        if db_replay_export_reads && db_mirror.is_none() {
            anyhow::bail!(
                "TRACE_COMMONS_DB_REPLAY_EXPORT_READS requires TRACE_COMMONS_DB_DUAL_WRITE"
            );
        }
        let db_replay_export_require_object_refs =
            env_truthy("TRACE_COMMONS_DB_REPLAY_EXPORT_REQUIRE_OBJECT_REFS");
        if db_replay_export_require_object_refs && !db_replay_export_reads {
            anyhow::bail!(
                "TRACE_COMMONS_DB_REPLAY_EXPORT_REQUIRE_OBJECT_REFS requires TRACE_COMMONS_DB_REPLAY_EXPORT_READS"
            );
        }
        let db_audit_reads = env_truthy("TRACE_COMMONS_DB_AUDIT_READS");
        if db_audit_reads && db_mirror.is_none() {
            anyhow::bail!("TRACE_COMMONS_DB_AUDIT_READS requires TRACE_COMMONS_DB_DUAL_WRITE");
        }
        let db_tenant_policy_reads = env_truthy("TRACE_COMMONS_DB_TENANT_POLICY_READS");
        if db_tenant_policy_reads && db_mirror.is_none() {
            anyhow::bail!(
                "TRACE_COMMONS_DB_TENANT_POLICY_READS requires TRACE_COMMONS_DB_DUAL_WRITE"
            );
        }
        let require_db_mirror_writes = env_truthy("TRACE_COMMONS_REQUIRE_DB_MIRROR_WRITES");
        if require_db_mirror_writes && db_mirror.is_none() {
            anyhow::bail!(
                "TRACE_COMMONS_REQUIRE_DB_MIRROR_WRITES requires TRACE_COMMONS_DB_DUAL_WRITE"
            );
        }
        let require_derived_export_object_refs =
            env_truthy("TRACE_COMMONS_DERIVED_EXPORT_REQUIRE_OBJECT_REFS");
        if require_derived_export_object_refs && db_mirror.is_none() {
            anyhow::bail!(
                "TRACE_COMMONS_DERIVED_EXPORT_REQUIRE_OBJECT_REFS requires TRACE_COMMONS_DB_DUAL_WRITE"
            );
        }
        let require_db_reconciliation_clean =
            env_truthy(TRACE_COMMONS_REQUIRE_DB_RECONCILIATION_CLEAN);
        if require_db_reconciliation_clean && db_mirror.is_none() {
            anyhow::bail!(
                "{TRACE_COMMONS_REQUIRE_DB_RECONCILIATION_CLEAN} requires TRACE_COMMONS_DB_DUAL_WRITE"
            );
        }
        let require_export_guardrails = env_truthy("TRACE_COMMONS_REQUIRE_EXPORT_GUARDRAILS");
        let max_export_items_per_request = parse_max_export_items_per_request_from_env()?;
        let submission_quota = parse_submission_quota_config_from_env()?;
        let legal_hold_retention_policy_ids = parse_legal_hold_retention_policy_ids_from_env()?;
        let artifact_store = trace_artifact_store_from_env(&root)?;
        let object_primary_submit_review = env_truthy(TRACE_COMMONS_OBJECT_PRIMARY_SUBMIT_REVIEW);
        let object_primary_replay_export = env_truthy(TRACE_COMMONS_OBJECT_PRIMARY_REPLAY_EXPORT);
        let object_primary_derived_exports =
            env_truthy(TRACE_COMMONS_OBJECT_PRIMARY_DERIVED_EXPORTS);
        validate_object_primary_submit_review_config(
            object_primary_submit_review,
            db_mirror.is_some(),
            require_db_mirror_writes,
            db_reviewer_reads,
            db_reviewer_require_object_refs,
            artifact_store
                .as_ref()
                .map(ConfiguredTraceArtifactStore::object_store_name),
        )?;
        validate_object_primary_replay_export_config(
            object_primary_replay_export,
            db_mirror.is_some(),
            require_db_mirror_writes,
            db_replay_export_reads,
            db_replay_export_require_object_refs,
            artifact_store
                .as_ref()
                .map(ConfiguredTraceArtifactStore::object_store_name),
        )?;
        validate_object_primary_derived_exports_config(
            object_primary_derived_exports,
            db_mirror.is_some(),
            require_db_mirror_writes,
            db_reviewer_reads,
            require_derived_export_object_refs,
            require_export_guardrails,
            artifact_store
                .as_ref()
                .map(ConfiguredTraceArtifactStore::object_store_name),
        )?;
        Ok(Self {
            root,
            tokens: Arc::new(tokens),
            tenant_policies: Arc::new(tenant_policies),
            require_tenant_submission_policy,
            db_mirror,
            db_contributor_reads,
            db_reviewer_reads,
            db_reviewer_require_object_refs,
            db_replay_export_reads,
            db_replay_export_require_object_refs,
            db_audit_reads,
            db_tenant_policy_reads,
            require_db_mirror_writes,
            require_derived_export_object_refs,
            object_primary_submit_review,
            object_primary_replay_export,
            object_primary_derived_exports,
            require_db_reconciliation_clean,
            require_export_guardrails,
            max_export_items_per_request,
            submission_quota,
            legal_hold_retention_policy_ids: Arc::new(legal_hold_retention_policy_ids),
            artifact_store,
        })
    }
}

fn validate_object_primary_submit_review_config(
    enabled: bool,
    db_mirror_configured: bool,
    require_db_mirror_writes: bool,
    db_reviewer_reads: bool,
    db_reviewer_require_object_refs: bool,
    artifact_store_name: Option<&str>,
) -> anyhow::Result<()> {
    if !enabled {
        return Ok(());
    }
    anyhow::ensure!(
        db_mirror_configured,
        "{TRACE_COMMONS_OBJECT_PRIMARY_SUBMIT_REVIEW} requires TRACE_COMMONS_DB_DUAL_WRITE"
    );
    anyhow::ensure!(
        require_db_mirror_writes,
        "{TRACE_COMMONS_OBJECT_PRIMARY_SUBMIT_REVIEW} requires TRACE_COMMONS_REQUIRE_DB_MIRROR_WRITES"
    );
    anyhow::ensure!(
        db_reviewer_reads,
        "{TRACE_COMMONS_OBJECT_PRIMARY_SUBMIT_REVIEW} requires TRACE_COMMONS_DB_REVIEWER_READS"
    );
    anyhow::ensure!(
        db_reviewer_require_object_refs,
        "{TRACE_COMMONS_OBJECT_PRIMARY_SUBMIT_REVIEW} requires TRACE_COMMONS_DB_REVIEWER_REQUIRE_OBJECT_REFS"
    );
    anyhow::ensure!(
        artifact_store_name == Some(TRACE_COMMONS_SERVICE_LOCAL_ENCRYPTED_OBJECT_STORE),
        "{TRACE_COMMONS_OBJECT_PRIMARY_SUBMIT_REVIEW} requires TRACE_COMMONS_OBJECT_STORE=local_service"
    );
    Ok(())
}

fn validate_object_primary_replay_export_config(
    enabled: bool,
    db_mirror_configured: bool,
    require_db_mirror_writes: bool,
    db_replay_export_reads: bool,
    db_replay_export_require_object_refs: bool,
    artifact_store_name: Option<&str>,
) -> anyhow::Result<()> {
    if !enabled {
        return Ok(());
    }
    anyhow::ensure!(
        db_mirror_configured,
        "{TRACE_COMMONS_OBJECT_PRIMARY_REPLAY_EXPORT} requires TRACE_COMMONS_DB_DUAL_WRITE"
    );
    anyhow::ensure!(
        require_db_mirror_writes,
        "{TRACE_COMMONS_OBJECT_PRIMARY_REPLAY_EXPORT} requires TRACE_COMMONS_REQUIRE_DB_MIRROR_WRITES"
    );
    anyhow::ensure!(
        db_replay_export_reads,
        "{TRACE_COMMONS_OBJECT_PRIMARY_REPLAY_EXPORT} requires TRACE_COMMONS_DB_REPLAY_EXPORT_READS"
    );
    anyhow::ensure!(
        db_replay_export_require_object_refs,
        "{TRACE_COMMONS_OBJECT_PRIMARY_REPLAY_EXPORT} requires TRACE_COMMONS_DB_REPLAY_EXPORT_REQUIRE_OBJECT_REFS"
    );
    anyhow::ensure!(
        artifact_store_name == Some(TRACE_COMMONS_SERVICE_LOCAL_ENCRYPTED_OBJECT_STORE),
        "{TRACE_COMMONS_OBJECT_PRIMARY_REPLAY_EXPORT} requires TRACE_COMMONS_OBJECT_STORE=local_service"
    );
    Ok(())
}

fn validate_object_primary_derived_exports_config(
    enabled: bool,
    db_mirror_configured: bool,
    require_db_mirror_writes: bool,
    db_reviewer_reads: bool,
    require_derived_export_object_refs: bool,
    require_export_guardrails: bool,
    artifact_store_name: Option<&str>,
) -> anyhow::Result<()> {
    if !enabled {
        return Ok(());
    }
    anyhow::ensure!(
        db_mirror_configured,
        "{TRACE_COMMONS_OBJECT_PRIMARY_DERIVED_EXPORTS} requires TRACE_COMMONS_DB_DUAL_WRITE"
    );
    anyhow::ensure!(
        require_db_mirror_writes,
        "{TRACE_COMMONS_OBJECT_PRIMARY_DERIVED_EXPORTS} requires TRACE_COMMONS_REQUIRE_DB_MIRROR_WRITES"
    );
    anyhow::ensure!(
        db_reviewer_reads,
        "{TRACE_COMMONS_OBJECT_PRIMARY_DERIVED_EXPORTS} requires TRACE_COMMONS_DB_REVIEWER_READS"
    );
    anyhow::ensure!(
        require_derived_export_object_refs,
        "{TRACE_COMMONS_OBJECT_PRIMARY_DERIVED_EXPORTS} requires TRACE_COMMONS_DERIVED_EXPORT_REQUIRE_OBJECT_REFS"
    );
    anyhow::ensure!(
        require_export_guardrails,
        "{TRACE_COMMONS_OBJECT_PRIMARY_DERIVED_EXPORTS} requires TRACE_COMMONS_REQUIRE_EXPORT_GUARDRAILS"
    );
    anyhow::ensure!(
        artifact_store_name == Some(TRACE_COMMONS_SERVICE_LOCAL_ENCRYPTED_OBJECT_STORE),
        "{TRACE_COMMONS_OBJECT_PRIMARY_DERIVED_EXPORTS} requires TRACE_COMMONS_OBJECT_STORE=local_service"
    );
    Ok(())
}

fn enforce_db_mirror_write_result(
    state: &AppState,
    operation: &str,
    result: anyhow::Result<()>,
) -> anyhow::Result<()> {
    match result {
        Ok(()) => {
            if state.require_db_mirror_writes && state.db_mirror.is_none() {
                anyhow::bail!(
                    "TRACE_COMMONS_REQUIRE_DB_MIRROR_WRITES requires TRACE_COMMONS_DB_DUAL_WRITE for {operation}"
                );
            }
            Ok(())
        }
        Err(error) if state.require_db_mirror_writes => Err(error.context(format!(
            "required Trace Commons DB mirror write failed: {operation}"
        ))),
        Err(_) => Ok(()),
    }
}

fn trace_artifact_store_from_env(
    default_root: &Path,
) -> anyhow::Result<Option<ConfiguredTraceArtifactStore>> {
    let object_store_mode = std::env::var("TRACE_COMMONS_OBJECT_STORE").ok();
    let key = std::env::var("TRACE_COMMONS_ARTIFACT_KEY_HEX").ok();
    let encrypted_store_kind = TraceEncryptedObjectStoreKind::from_config(
        object_store_mode.as_deref(),
        key.is_some() || env_truthy("TRACE_COMMONS_ENCRYPTED_ARTIFACTS"),
    )?;
    let Some(encrypted_store_kind) = encrypted_store_kind else {
        return Ok(None);
    };
    let key = key.context(
        "encrypted Trace Commons object storage requires TRACE_COMMONS_ARTIFACT_KEY_HEX",
    )?;
    let root = encrypted_store_kind.root_from_env(default_root);
    let object_store_name = encrypted_store_kind.object_store_name();
    let crypto = SecretsCrypto::new(SecretString::from(key))
        .context("failed to initialize Trace Commons artifact encryption")?;
    Ok(Some(ConfiguredTraceArtifactStore::new(
        object_store_name,
        Arc::new(LocalEncryptedTraceArtifactStore::new(root, crypto)),
    )))
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
            "/v1/benchmarks/{conversion_id}/lifecycle",
            post(benchmark_lifecycle_handler),
        )
        .route(
            "/v1/workers/benchmark-convert",
            post(benchmark_worker_convert_handler),
        )
        .route(
            "/v1/ranker/training-candidates",
            get(ranker_training_candidates_handler),
        )
        .route(
            "/v1/ranker/training-pairs",
            get(ranker_training_pairs_handler),
        )
        .route(
            "/v1/admin/tenant-policy",
            get(get_tenant_policy_handler)
                .post(put_tenant_policy_handler)
                .put(put_tenant_policy_handler),
        )
        .route("/v1/admin/config-status", get(config_status_handler))
        .route("/v1/admin/maintenance", post(maintenance_handler))
        .route(
            "/v1/workers/retention-maintenance",
            post(retention_maintenance_handler),
        )
        .route("/v1/workers/vector-index", post(vector_index_handler))
        .route("/v1/workers/utility-credit", post(utility_credit_handler))
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

fn parse_tenant_submission_policies_from_env()
-> anyhow::Result<BTreeMap<String, TenantSubmissionPolicy>> {
    match std::env::var("TRACE_COMMONS_TENANT_POLICIES") {
        Ok(configured) => parse_tenant_submission_policies(&configured),
        Err(std::env::VarError::NotPresent) => Ok(BTreeMap::new()),
        Err(error) => Err(error).context("failed to read TRACE_COMMONS_TENANT_POLICIES"),
    }
}

fn parse_tenant_submission_policies(
    configured: &str,
) -> anyhow::Result<BTreeMap<String, TenantSubmissionPolicy>> {
    let configured = configured.trim();
    if configured.is_empty() {
        return Ok(BTreeMap::new());
    }

    serde_json::from_str(configured)
        .context("TRACE_COMMONS_TENANT_POLICIES must be a JSON object keyed by tenant id")
}

fn parse_legal_hold_retention_policy_ids_from_env() -> anyhow::Result<BTreeSet<String>> {
    match std::env::var(TRACE_COMMONS_LEGAL_HOLD_RETENTION_POLICIES) {
        Ok(configured) => parse_legal_hold_retention_policy_ids(&configured),
        Err(std::env::VarError::NotPresent) => Ok(BTreeSet::new()),
        Err(error) => Err(error).with_context(|| {
            format!("failed to read {TRACE_COMMONS_LEGAL_HOLD_RETENTION_POLICIES}")
        }),
    }
}

fn parse_legal_hold_retention_policy_ids(configured: &str) -> anyhow::Result<BTreeSet<String>> {
    let mut policy_ids = BTreeSet::new();
    for policy_id in configured.split(',').map(str::trim) {
        if policy_id.is_empty() {
            continue;
        }
        validate_retention_policy_id(policy_id)?;
        policy_ids.insert(policy_id.to_string());
    }
    Ok(policy_ids)
}

fn parse_max_export_items_per_request_from_env() -> anyhow::Result<usize> {
    match std::env::var(TRACE_COMMONS_MAX_EXPORT_ITEMS_PER_REQUEST) {
        Ok(configured) => parse_max_export_items_per_request(&configured),
        Err(std::env::VarError::NotPresent) => {
            Ok(DEFAULT_TRACE_COMMONS_MAX_EXPORT_ITEMS_PER_REQUEST)
        }
        Err(error) => Err(error).with_context(|| {
            format!("failed to read {TRACE_COMMONS_MAX_EXPORT_ITEMS_PER_REQUEST}")
        }),
    }
}

fn parse_max_export_items_per_request(configured: &str) -> anyhow::Result<usize> {
    let parsed = configured.trim().parse::<usize>().with_context(|| {
        format!("{TRACE_COMMONS_MAX_EXPORT_ITEMS_PER_REQUEST} must be a positive integer")
    })?;
    if parsed == 0 {
        anyhow::bail!("{TRACE_COMMONS_MAX_EXPORT_ITEMS_PER_REQUEST} must be at least 1");
    }
    Ok(parsed)
}

fn parse_submission_quota_config_from_env() -> anyhow::Result<TraceSubmissionQuotaConfig> {
    Ok(TraceSubmissionQuotaConfig {
        max_per_tenant_per_hour: parse_submission_quota_limit_from_env(
            TRACE_COMMONS_MAX_SUBMISSIONS_PER_TENANT_PER_HOUR,
        )?,
        max_per_principal_per_hour: parse_submission_quota_limit_from_env(
            TRACE_COMMONS_MAX_SUBMISSIONS_PER_PRINCIPAL_PER_HOUR,
        )?,
    })
}

fn parse_submission_quota_limit_from_env(var_name: &'static str) -> anyhow::Result<usize> {
    match std::env::var(var_name) {
        Ok(configured) => parse_submission_quota_limit(var_name, &configured),
        Err(std::env::VarError::NotPresent) => Ok(0),
        Err(error) => Err(error).with_context(|| format!("failed to read {var_name}")),
    }
}

fn parse_submission_quota_limit(var_name: &'static str, configured: &str) -> anyhow::Result<usize> {
    configured
        .trim()
        .parse::<usize>()
        .with_context(|| format!("{var_name} must be a non-negative integer"))
}

fn validate_retention_policy_id(policy_id: &str) -> anyhow::Result<()> {
    let valid = policy_id.len() <= 128
        && policy_id.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.' | ':')
        });
    if !valid {
        anyhow::bail!(
            "{TRACE_COMMONS_LEGAL_HOLD_RETENTION_POLICIES} contains an invalid retention policy id"
        );
    }
    Ok(())
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

#[derive(Debug, Deserialize)]
struct TraceTenantPolicyRequest {
    policy_version: String,
    #[serde(default)]
    allowed_consent_scopes: Vec<ConsentScope>,
    #[serde(default)]
    allowed_uses: Vec<TraceAllowedUse>,
}

#[derive(Debug, Serialize)]
struct TraceTenantPolicyResponse {
    tenant_id: String,
    policy_version: String,
    allowed_consent_scopes: Vec<String>,
    allowed_uses: Vec<String>,
    updated_by_principal_ref: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
struct TraceCommonsConfigStatusResponse {
    schema_version: &'static str,
    db_mirror_configured: bool,
    db_contributor_reads: bool,
    db_reviewer_reads: bool,
    db_reviewer_require_object_refs: bool,
    db_replay_export_reads: bool,
    db_replay_export_require_object_refs: bool,
    db_audit_reads: bool,
    db_tenant_policy_reads: bool,
    require_tenant_submission_policy: bool,
    require_db_mirror_writes: bool,
    require_derived_export_object_refs: bool,
    object_primary_submit_review: bool,
    object_primary_replay_export: bool,
    object_primary_derived_exports: bool,
    require_db_reconciliation_clean: bool,
    require_export_guardrails: bool,
    max_export_items_per_request: usize,
    submission_quota: TraceSubmissionQuotaConfig,
    legal_hold_retention_policy_ids: Vec<String>,
    artifact_store_configured: bool,
    artifact_object_store: Option<String>,
}

fn trace_commons_config_status_response(state: &AppState) -> TraceCommonsConfigStatusResponse {
    TraceCommonsConfigStatusResponse {
        schema_version: TRACE_CONTRIBUTION_SCHEMA_VERSION,
        db_mirror_configured: state.db_mirror.is_some(),
        db_contributor_reads: state.db_contributor_reads,
        db_reviewer_reads: state.db_reviewer_reads,
        db_reviewer_require_object_refs: state.db_reviewer_require_object_refs,
        db_replay_export_reads: state.db_replay_export_reads,
        db_replay_export_require_object_refs: state.db_replay_export_require_object_refs,
        db_audit_reads: state.db_audit_reads,
        db_tenant_policy_reads: state.db_tenant_policy_reads,
        require_tenant_submission_policy: state.require_tenant_submission_policy,
        require_db_mirror_writes: state.require_db_mirror_writes,
        require_derived_export_object_refs: state.require_derived_export_object_refs,
        object_primary_submit_review: state.object_primary_submit_review,
        object_primary_replay_export: state.object_primary_replay_export,
        object_primary_derived_exports: state.object_primary_derived_exports,
        require_db_reconciliation_clean: state.require_db_reconciliation_clean,
        require_export_guardrails: state.require_export_guardrails,
        max_export_items_per_request: state.max_export_items_per_request,
        submission_quota: state.submission_quota,
        legal_hold_retention_policy_ids: state
            .legal_hold_retention_policy_ids
            .iter()
            .cloned()
            .collect(),
        artifact_store_configured: state.artifact_store.is_some(),
        artifact_object_store: state
            .artifact_store
            .as_ref()
            .map(|store| store.object_store_name().to_string()),
    }
}

async fn config_status_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> ApiResult<Json<TraceCommonsConfigStatusResponse>> {
    let tenant = authenticate(state.as_ref(), &headers)?;
    require_admin(&tenant)?;
    let response = trace_commons_config_status_response(state.as_ref());
    append_audit_event_with_db_mirror(
        state.as_ref(),
        &tenant,
        TraceCommonsAuditEvent::read(&tenant, "config_status", 1),
        StorageTraceAuditAction::Read,
        StorageTraceAuditSafeMetadata::Empty,
    )
    .await
    .map_err(internal_error)?;
    Ok(Json(response))
}

async fn get_tenant_policy_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> ApiResult<Json<TraceTenantPolicyResponse>> {
    let tenant = authenticate(state.as_ref(), &headers)?;
    require_admin(&tenant)?;
    let db = trace_tenant_policy_db(state.as_ref())?;
    let policy = db
        .get_trace_tenant_policy(&tenant.tenant_id)
        .await
        .map_err(internal_error)?
        .ok_or_else(|| {
            api_error(
                StatusCode::NOT_FOUND,
                "trace tenant contribution policy does not exist",
            )
        })?;
    let response = trace_tenant_policy_response(policy);
    append_audit_event_with_db_mirror(
        state.as_ref(),
        &tenant,
        TraceCommonsAuditEvent::read(&tenant, "tenant_policy", 1),
        StorageTraceAuditAction::Read,
        StorageTraceAuditSafeMetadata::Empty,
    )
    .await
    .map_err(internal_error)?;
    Ok(Json(response))
}

async fn put_tenant_policy_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<TraceTenantPolicyRequest>,
) -> ApiResult<Json<TraceTenantPolicyResponse>> {
    let tenant = authenticate(state.as_ref(), &headers)?;
    require_admin(&tenant)?;
    let db = trace_tenant_policy_db(state.as_ref())?;
    let policy_version = request.policy_version.trim();
    if policy_version.is_empty() {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "trace tenant contribution policy requires a policy_version",
        ));
    }
    let allowed_consent_scopes = request
        .allowed_consent_scopes
        .iter()
        .map(serde_storage_string)
        .collect::<anyhow::Result<Vec<_>>>()
        .map_err(internal_error)?;
    let allowed_uses = request
        .allowed_uses
        .iter()
        .map(serde_storage_string)
        .collect::<anyhow::Result<Vec<_>>>()
        .map_err(internal_error)?;
    let policy_projection_hash =
        trace_tenant_policy_projection_hash(policy_version, &allowed_consent_scopes, &allowed_uses)
            .map_err(internal_error)?;
    let policy = db
        .upsert_trace_tenant_policy(StorageTraceTenantPolicyWrite {
            tenant_id: tenant.tenant_id.clone(),
            policy_version: policy_version.to_string(),
            allowed_consent_scopes,
            allowed_uses,
            updated_by_principal_ref: tenant.principal_ref.clone(),
        })
        .await
        .map_err(internal_error)?;
    let response = trace_tenant_policy_response(policy);
    append_audit_event_with_db_mirror(
        state.as_ref(),
        &tenant,
        TraceCommonsAuditEvent::tenant_policy_update(
            &tenant,
            &response.policy_version,
            response.allowed_consent_scopes.len(),
            response.allowed_uses.len(),
            &policy_projection_hash,
        ),
        StorageTraceAuditAction::PolicyUpdate,
        StorageTraceAuditSafeMetadata::TenantPolicy {
            policy_version: response.policy_version.clone(),
            allowed_consent_scope_count: response.allowed_consent_scopes.len() as u32,
            allowed_use_count: response.allowed_uses.len() as u32,
            policy_projection_hash,
        },
    )
    .await
    .map_err(internal_error)?;
    Ok(Json(response))
}

fn trace_tenant_policy_db(state: &AppState) -> ApiResult<Arc<dyn Database>> {
    state.db_mirror.as_ref().cloned().ok_or_else(|| {
        api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "trace tenant policy DB is not configured",
        )
    })
}

fn trace_tenant_policy_response(
    policy: StorageTraceTenantPolicyRecord,
) -> TraceTenantPolicyResponse {
    TraceTenantPolicyResponse {
        tenant_id: policy.tenant_id,
        policy_version: policy.policy_version,
        allowed_consent_scopes: policy.allowed_consent_scopes,
        allowed_uses: policy.allowed_uses,
        updated_by_principal_ref: policy.updated_by_principal_ref,
        created_at: policy.created_at,
        updated_at: policy.updated_at,
    }
}

fn trace_tenant_policy_projection_hash(
    policy_version: &str,
    allowed_consent_scopes: &[String],
    allowed_uses: &[String],
) -> anyhow::Result<String> {
    #[derive(Serialize)]
    struct TenantPolicyAuditProjection<'a> {
        policy_version: &'a str,
        allowed_consent_scopes: &'a [String],
        allowed_uses: &'a [String],
    }

    let projection = TenantPolicyAuditProjection {
        policy_version,
        allowed_consent_scopes,
        allowed_uses,
    };
    let json = serde_json::to_string(&projection)
        .context("failed to serialize trace tenant policy audit projection")?;
    Ok(sha256_prefixed(&json))
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

    let tenant_policy = tenant_submission_policy_for_request(state.as_ref(), &tenant).await?;
    enforce_tenant_submission_policy(
        &tenant,
        &envelope,
        tenant_policy.as_ref(),
        state.require_tenant_submission_policy,
    )?;

    rescrub_trace_envelope(&mut envelope);
    let existing_revocations = read_revocations_for_submit(state.as_ref(), &tenant.tenant_id)
        .await
        .map_err(internal_error)?;
    let existing_derived =
        read_all_derived_records(&state.root, &tenant.tenant_id).map_err(internal_error)?;
    let derived_precheck = build_derived_precheck(&envelope, &existing_derived);
    ensure_not_revoked_by_tombstone(
        &existing_revocations,
        envelope.submission_id,
        &envelope.privacy.redaction_hash,
        &derived_precheck.canonical_summary_hash,
    )?;
    enforce_submission_quota(state.as_ref(), &tenant)?;
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
        allowed_uses: envelope.trace_card.allowed_uses.clone(),
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
    let mirror_result =
        mirror_submission_to_db(&state, &tenant, &record, &derived_record, &envelope).await;
    if let Err(error) = &mirror_result {
        tracing::warn!(%error, submission_id = %record.submission_id, "Trace Commons DB dual-write mirror failed");
    }
    enforce_db_mirror_write_result(state.as_ref(), "submission", mirror_result)
        .map_err(internal_error)?;

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
    let mut record = read_submission_record(&state.root, &tenant.tenant_id, submission_id)
        .map_err(internal_error)?;
    if let Some(record) = record.as_ref()
        && !can_access_submission(&tenant, record)
    {
        return Err(api_error(
            StatusCode::NOT_FOUND,
            "trace submission not found",
        ));
    }
    let db_record = if record.is_none() {
        db_submission_record_for_revocation(state, &tenant.tenant_id, submission_id)
            .await
            .map_err(internal_error)?
    } else {
        None
    };
    if let Some(db_record) = db_record.as_ref()
        && !can_access_storage_submission(&tenant, db_record)
    {
        return Err(api_error(
            StatusCode::NOT_FOUND,
            "trace submission not found",
        ));
    }
    let mut derived = read_derived_record(&state.root, &tenant.tenant_id, submission_id)
        .map_err(internal_error)?;
    let tombstone = TraceCommonsRevocation {
        tenant_id: tenant.tenant_id.clone(),
        tenant_storage_ref: tenant_storage_ref(&tenant.tenant_id),
        submission_id,
        revoked_at: Utc::now(),
        reason: "contributor_revocation".to_string(),
        redaction_hash: record
            .as_ref()
            .and_then(|record| redaction_hash_for_record(state, record)),
        canonical_summary_hash: derived
            .as_ref()
            .map(|record| record.canonical_summary_hash.clone()),
    };
    write_revocation(&state.root, &tombstone).map_err(internal_error)?;

    let mut mirrored_record = None;
    if let Some(mut record) = record.take() {
        record.status = TraceCorpusStatus::Revoked;
        record.credit_points_final = Some(0.0);
        write_submission_record(&state.root, &record).map_err(internal_error)?;
        mirrored_record = Some(record);
    }
    if let Some(mut derived) = derived.take() {
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
    let mut benchmark_revocation_reasons = BTreeMap::new();
    benchmark_revocation_reasons.insert(submission_id, "contributor_revocation".to_string());
    propagate_benchmark_artifact_source_invalidation(
        state,
        &tenant,
        &benchmark_revocation_reasons,
        &BTreeSet::new(),
        false,
    )
    .await
    .map_err(internal_error)?;

    append_audit_event(
        &state.root,
        &tenant.tenant_id,
        TraceCommonsAuditEvent::revoked(&tenant, submission_id),
    )
    .map_err(internal_error)?;
    let mirror_result = mirror_revocation_to_db(
        state,
        &tenant,
        submission_id,
        mirrored_record.as_ref(),
        db_record.as_ref(),
    )
    .await;
    if let Err(error) = &mirror_result {
        tracing::warn!(%error, %submission_id, "Trace Commons DB dual-write revocation mirror failed");
    }
    enforce_db_mirror_write_result(state, "revocation", mirror_result).map_err(internal_error)?;
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
    let item_count = credit_view.records.len();
    append_audit_event_with_db_mirror(
        state.as_ref(),
        &tenant,
        TraceCommonsAuditEvent::read(&tenant, "contributor_credit", item_count),
        StorageTraceAuditAction::Read,
        StorageTraceAuditSafeMetadata::Empty,
    )
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
    append_audit_event_with_db_mirror(
        state.as_ref(),
        &tenant,
        TraceCommonsAuditEvent::read(
            &tenant,
            "contributor_credit_events",
            credit_view.credit_events.len(),
        ),
        StorageTraceAuditAction::Read,
        StorageTraceAuditSafeMetadata::Empty,
    )
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

    append_audit_event_with_db_mirror(
        state.as_ref(),
        &tenant,
        TraceCommonsAuditEvent::read(&tenant, "submission_status", statuses.len()),
        StorageTraceAuditAction::Read,
        StorageTraceAuditSafeMetadata::Empty,
    )
    .await
    .map_err(internal_error)?;
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
    let response =
        TraceCommonsAnalyticsResponse::from_records(tenant.tenant_id.clone(), records, derived);
    append_audit_event_with_db_mirror(
        state.as_ref(),
        &tenant,
        TraceCommonsAuditEvent::read(&tenant, "analytics_summary", response.submissions_total),
        StorageTraceAuditAction::Read,
        StorageTraceAuditSafeMetadata::Empty,
    )
    .await
    .map_err(internal_error)?;
    Ok(Json(response))
}

#[derive(Debug, Deserialize)]
struct TraceListQuery {
    status: Option<TraceCorpusStatus>,
    limit: Option<usize>,
    purpose: Option<String>,
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
    let purpose_submission_ids =
        trace_list_purpose_submission_ids(state.as_ref(), &tenant, query.purpose.as_deref())
            .await
            .map_err(internal_error)?;

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
            purpose_submission_ids
                .as_ref()
                .is_none_or(|submission_ids| submission_ids.contains(&record.submission_id))
        })
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
const BENCHMARK_CONVERSION_CREDIT_POINTS_DELTA: f32 = 2.0;
const RANKER_TRAINING_CANDIDATE_CREDIT_POINTS_DELTA: f32 = 0.5;
const RANKER_TRAINING_PAIR_CREDIT_POINTS_DELTA: f32 = 0.75;
const RANKER_TRAINING_CANDIDATES_EXPORT_PURPOSE_CODE: &str = "ranker_training_candidates_export";
const RANKER_TRAINING_PAIRS_EXPORT_PURPOSE_CODE: &str = "ranker_training_pairs_export";

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

    fn is_utility_job_type(self) -> bool {
        matches!(
            self,
            Self::RegressionCatch | Self::TrainingUtility | Self::RankingUtility
        )
    }

    fn utility_idempotency_label(self) -> &'static str {
        match self {
            Self::RegressionCatch => "utility-regression-credit",
            Self::TrainingUtility => "utility-training-credit",
            Self::RankingUtility => "utility-ranking-credit",
            Self::BenchmarkConversion => "utility-benchmark-credit",
            Self::ReviewerBonus => "utility-reviewer-bonus",
            Self::AbusePenalty => "utility-abuse-penalty",
        }
    }
}

#[derive(Debug, Deserialize)]
struct TraceUtilityCreditJobRequest {
    event_type: TraceCreditLedgerEventType,
    credit_points_delta: f32,
    reason: String,
    external_ref: String,
    submission_ids: Vec<Uuid>,
}

#[derive(Debug, Serialize)]
struct TraceUtilityCreditJobResponse {
    tenant_id: String,
    tenant_storage_ref: String,
    event_type: TraceCreditLedgerEventType,
    credit_points_delta: f32,
    external_ref: String,
    requested_count: usize,
    appended_count: usize,
    skipped_existing_count: usize,
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

    let submission = read_reviewer_submission_record(state.as_ref(), &tenant, submission_id)
        .await
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
    let mirror_result = mirror_credit_event_to_db(&state, &event).await;
    if let Err(error) = &mirror_result {
        tracing::warn!(%error, submission_id = %event.submission_id, "Trace Commons DB dual-write credit mirror failed");
    }
    enforce_db_mirror_write_result(state.as_ref(), "credit ledger event", mirror_result)
        .map_err(internal_error)?;
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
        StorageTraceAuditSafeMetadata::CreditMutation {
            event_type: storage_credit_event_type(body.event_type),
            credit_points_delta_micros: credit_delta_micros(body.credit_points_delta),
            reason_hash: sha256_prefixed(event.reason.as_deref().unwrap_or_default()),
            external_ref_hash: event.external_ref.as_deref().map(sha256_prefixed),
        },
    )
    .await
    .map_err(internal_error)?;
    Ok(Json(event))
}

async fn utility_credit_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<TraceUtilityCreditJobRequest>,
) -> ApiResult<Json<TraceUtilityCreditJobResponse>> {
    let tenant = authenticate(state.as_ref(), &headers)?;
    require_utility_operator(&tenant)?;
    if !body.event_type.is_utility_job_type() {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "utility credit jobs support regression_catch, training_utility, or ranking_utility",
        ));
    }
    if !body.credit_points_delta.is_finite() {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "credit_points_delta must be finite",
        ));
    }
    if body.credit_points_delta.abs() > MAX_DELAYED_CREDIT_POINTS_DELTA {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "credit_points_delta exceeds the delayed credit policy limit",
        ));
    }
    let reason = body.reason.trim().to_string();
    if reason.is_empty() {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "utility credit jobs require a non-empty reason",
        ));
    }
    let external_ref = body.external_ref.trim().to_string();
    if external_ref.is_empty() {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "utility credit jobs require a non-empty external_ref",
        ));
    }
    if body.submission_ids.is_empty() {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "utility credit jobs require at least one submission_id",
        ));
    }
    let unique_submission_ids = body
        .submission_ids
        .iter()
        .copied()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let mut sources = Vec::with_capacity(unique_submission_ids.len());
    for submission_id in &unique_submission_ids {
        let submission = read_utility_submission_record(state.as_ref(), &tenant, *submission_id)
            .await
            .map_err(internal_error)?
            .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "trace submission not found"))?;
        if submission.status != TraceCorpusStatus::Accepted {
            return Err(api_error(
                StatusCode::CONFLICT,
                "utility credit jobs can only credit accepted trace submissions",
            ));
        }
        sources.push(AutomaticUtilityCreditSource {
            submission_id: submission.submission_id,
            trace_id: submission.trace_id,
            auth_principal_ref: submission.auth_principal_ref,
        });
    }

    let counts = append_automatic_utility_credit_events_once_with_counts(
        state.as_ref(),
        &tenant,
        AutomaticUtilityCreditConfig {
            idempotency_label: body.event_type.utility_idempotency_label(),
            idempotency_ref: Some(external_ref.clone()),
            event_type: body.event_type,
            credit_points_delta: body.credit_points_delta,
            reason,
            external_ref: external_ref.clone(),
        },
        sources,
    )
    .await
    .map_err(internal_error)?;

    Ok(Json(TraceUtilityCreditJobResponse {
        tenant_id: tenant.tenant_id.clone(),
        tenant_storage_ref: tenant_storage_ref(&tenant.tenant_id),
        event_type: body.event_type,
        credit_points_delta: body.credit_points_delta,
        external_ref,
        requested_count: unique_submission_ids.len(),
        appended_count: counts.appended,
        skipped_existing_count: counts.skipped_existing,
    }))
}

struct AutomaticUtilityCreditSource {
    submission_id: Uuid,
    trace_id: Uuid,
    auth_principal_ref: String,
}

impl AutomaticUtilityCreditSource {
    fn from_benchmark_candidate(candidate: &TraceBenchmarkCandidate) -> Self {
        Self {
            submission_id: candidate.submission_id,
            trace_id: candidate.trace_id,
            auth_principal_ref: candidate.auth_principal_ref.clone(),
        }
    }

    fn from_ranker_candidate(candidate: &TraceRankerTrainingCandidate) -> Self {
        Self {
            submission_id: candidate.submission_id,
            trace_id: candidate.trace_id,
            auth_principal_ref: candidate.auth_principal_ref.clone(),
        }
    }
}

struct AutomaticUtilityCreditConfig {
    idempotency_label: &'static str,
    idempotency_ref: Option<String>,
    event_type: TraceCreditLedgerEventType,
    credit_points_delta: f32,
    reason: String,
    external_ref: String,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct AutomaticUtilityCreditAppendCounts {
    appended: usize,
    skipped_existing: usize,
}

async fn append_automatic_utility_credit_events_once(
    state: &AppState,
    tenant: &TenantAuth,
    config: AutomaticUtilityCreditConfig,
    sources: impl IntoIterator<Item = AutomaticUtilityCreditSource>,
) -> anyhow::Result<usize> {
    Ok(
        append_automatic_utility_credit_events_once_with_counts(state, tenant, config, sources)
            .await?
            .appended,
    )
}

async fn append_automatic_utility_credit_events_once_with_counts(
    state: &AppState,
    tenant: &TenantAuth,
    config: AutomaticUtilityCreditConfig,
    sources: impl IntoIterator<Item = AutomaticUtilityCreditSource>,
) -> anyhow::Result<AutomaticUtilityCreditAppendCounts> {
    let mut existing_event_ids = read_all_credit_events(&state.root, &tenant.tenant_id)?
        .into_iter()
        .map(|event| event.event_id)
        .collect::<BTreeSet<_>>();
    let mut counts = AutomaticUtilityCreditAppendCounts::default();
    for source in sources {
        let event_id = if let Some(idempotency_ref) = config.idempotency_ref.as_ref() {
            deterministic_trace_uuid_for_external_ref(
                config.idempotency_label,
                &tenant.tenant_id,
                source.submission_id,
                idempotency_ref,
            )
        } else {
            deterministic_trace_uuid_for(
                config.idempotency_label,
                &tenant.tenant_id,
                source.submission_id,
            )
        };
        if existing_event_ids.contains(&event_id) {
            counts.skipped_existing += 1;
            continue;
        }
        let event = TraceCommonsCreditLedgerRecord {
            event_id,
            tenant_id: tenant.tenant_id.clone(),
            tenant_storage_ref: tenant_storage_ref(&tenant.tenant_id),
            submission_id: source.submission_id,
            trace_id: source.trace_id,
            auth_principal_ref: source.auth_principal_ref,
            event_type: config.event_type,
            credit_points_delta: config.credit_points_delta,
            reason: Some(config.reason.clone()),
            external_ref: Some(config.external_ref.clone()),
            actor_role: tenant.role,
            actor_principal_ref: tenant.principal_ref.clone(),
            created_at: Utc::now(),
        };
        append_credit_event(&state.root, &tenant.tenant_id, &event)?;
        let mirror_result = mirror_credit_event_to_db(state, &event).await;
        if let Err(error) = &mirror_result {
            tracing::warn!(%error, submission_id = %event.submission_id, "Trace Commons DB dual-write automatic credit mirror failed");
        }
        enforce_db_mirror_write_result(state, "automatic credit ledger event", mirror_result)?;
        append_audit_event_with_db_mirror(
            state,
            tenant,
            TraceCommonsAuditEvent::credit_mutation(
                tenant,
                event.submission_id,
                config.credit_points_delta,
                event.reason.as_deref(),
            ),
            StorageTraceAuditAction::CreditMutate,
            StorageTraceAuditSafeMetadata::CreditMutation {
                event_type: storage_credit_event_type(config.event_type),
                credit_points_delta_micros: credit_delta_micros(config.credit_points_delta),
                reason_hash: sha256_prefixed(event.reason.as_deref().unwrap_or_default()),
                external_ref_hash: event.external_ref.as_deref().map(sha256_prefixed),
            },
        )
        .await?;
        existing_event_ids.insert(event_id);
        counts.appended += 1;
    }
    Ok(counts)
}

async fn review_decision_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(submission_id): AxumPath<Uuid>,
    Json(body): Json<TraceReviewDecisionRequest>,
) -> ApiResult<Json<TraceSubmissionReceipt>> {
    let tenant = authenticate(state.as_ref(), &headers)?;
    require_reviewer(&tenant)?;
    let reason = body
        .reason
        .as_deref()
        .map(str::trim)
        .filter(|reason| !reason.is_empty())
        .ok_or_else(|| {
            api_error(
                StatusCode::BAD_REQUEST,
                "review decisions require a non-empty reason",
            )
        })?
        .to_string();
    let ReviewDecisionRecord {
        mut record,
        mut canonical_summary_hash,
        file_record_available,
        allow_file_body_fallback,
    } = read_review_decision_record(state.as_ref(), &tenant, submission_id)
        .await
        .map_err(internal_error)?
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "trace submission not found"))?;
    if record.is_terminal() {
        return Err(api_error(
            StatusCode::CONFLICT,
            "terminal trace submissions are not eligible for review approval",
        ));
    }
    let mut envelope = read_envelope_for_review_decision(
        state.as_ref(),
        &tenant,
        &record,
        allow_file_body_fallback,
    )
    .await
    .map_err(internal_error)?
    .envelope;

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

    if file_record_available {
        write_submission_record(&state.root, &record).map_err(internal_error)?;
    }
    if let Some(mut derived) = read_derived_record(&state.root, &tenant.tenant_id, submission_id)
        .map_err(internal_error)?
    {
        derived.status = record.status;
        canonical_summary_hash = Some(derived.canonical_summary_hash.clone());
        write_derived_record(&state.root, &derived).map_err(internal_error)?;
    }
    append_audit_event(
        &state.root,
        &tenant.tenant_id,
        TraceCommonsAuditEvent::review_decision(
            &tenant,
            submission_id,
            record.status,
            Some(reason.as_str()),
        ),
    )
    .map_err(internal_error)?;
    let mirror_result =
        mirror_review_decision_to_db(&state, &tenant, &record, &envelope, canonical_summary_hash)
            .await;
    if let Err(error) = &mirror_result {
        tracing::warn!(%error, %submission_id, "Trace Commons DB dual-write review mirror failed");
    }
    enforce_db_mirror_write_result(state.as_ref(), "review decision", mirror_result)
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
    require_exporter(&tenant)?;
    let consent_scope = parse_consent_scope_filter(query.consent_scope.as_deref())?;
    enforce_dataset_export_guardrails(
        state.as_ref(),
        "replay dataset",
        query.purpose.as_deref(),
        query.status,
        query.privacy_risk,
        consent_scope,
    )?;
    let tenant_policy = tenant_export_policy_for_request(
        state.as_ref(),
        &tenant,
        "replay dataset",
        consent_scope,
        TraceAllowedUse::Evaluation,
    )
    .await?;
    let purpose =
        normalized_export_purpose(query.purpose.as_deref(), "trace_commons_replay_dataset");
    let TraceCommonsMetadataView { records, derived } =
        read_replay_export_metadata_view(state.as_ref(), &tenant)
            .await
            .map_err(internal_error)?;
    let derived_by_submission = derived
        .into_iter()
        .map(|record| (record.submission_id, record))
        .collect::<BTreeMap<_, _>>();
    let limit = resolve_export_limit(state.as_ref(), query.limit);
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
        .filter(|record| {
            record_matches_export_policy_abac(
                record,
                tenant_policy.as_ref(),
                TraceAllowedUse::Evaluation,
            )
        })
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
    let mirror_result = mirror_export_manifest_to_db(
        state.as_ref(),
        StorageTraceObjectArtifactKind::ExportArtifact,
        &manifest,
        &items,
    )
    .await;
    if let Err(error) = &mirror_result {
        tracing::warn!(
            %error,
            export_id = %manifest.export_id,
            "Trace Commons DB dual-write export manifest mirror failed"
        );
    }
    enforce_db_mirror_write_result(state.as_ref(), "replay export manifest", mirror_result)
        .map_err(internal_error)?;
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
    require_exporter(&tenant)?;
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

#[derive(Debug, Deserialize)]
struct BenchmarkLifecycleUpdateRequest {
    #[serde(default)]
    registry: Option<TraceBenchmarkRegistryPatch>,
    #[serde(default)]
    evaluation: Option<TraceBenchmarkEvaluationPatch>,
    #[serde(default)]
    reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TraceBenchmarkRegistryPatch {
    #[serde(default)]
    status: Option<TraceBenchmarkRegistryStatus>,
    #[serde(default)]
    registry_ref: Option<String>,
    #[serde(default)]
    published_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
struct TraceBenchmarkEvaluationPatch {
    #[serde(default)]
    status: Option<TraceBenchmarkEvaluationStatus>,
    #[serde(default)]
    evaluator_ref: Option<String>,
    #[serde(default)]
    evaluated_at: Option<DateTime<Utc>>,
    #[serde(default)]
    score: Option<f32>,
    #[serde(default)]
    pass_count: Option<u32>,
    #[serde(default)]
    fail_count: Option<u32>,
}

async fn benchmark_convert_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<BenchmarkConversionRequest>,
) -> ApiResult<Json<TraceBenchmarkConversionArtifact>> {
    let tenant = authenticate(state.as_ref(), &headers)?;
    require_benchmarker(&tenant)?;
    run_benchmark_conversion(state.as_ref(), &tenant, body).await
}

async fn benchmark_worker_convert_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<BenchmarkConversionRequest>,
) -> ApiResult<Json<TraceBenchmarkConversionArtifact>> {
    let tenant = authenticate(state.as_ref(), &headers)?;
    require_benchmarker(&tenant)?;
    run_benchmark_conversion(state.as_ref(), &tenant, body).await
}

async fn benchmark_lifecycle_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(conversion_id): AxumPath<Uuid>,
    Json(body): Json<BenchmarkLifecycleUpdateRequest>,
) -> ApiResult<Json<TraceBenchmarkConversionArtifact>> {
    let tenant = authenticate(state.as_ref(), &headers)?;
    require_benchmarker(&tenant)?;
    update_benchmark_lifecycle(state.as_ref(), &tenant, conversion_id, body).await
}

async fn run_benchmark_conversion(
    state: &AppState,
    tenant: &TenantAuth,
    body: BenchmarkConversionRequest,
) -> ApiResult<Json<TraceBenchmarkConversionArtifact>> {
    let consent_scope = parse_consent_scope_filter(body.consent_scope.as_deref())?;
    enforce_dataset_export_guardrails(
        state,
        "benchmark conversion",
        body.purpose.as_deref(),
        body.status,
        body.privacy_risk,
        consent_scope,
    )?;
    let tenant_policy = tenant_export_policy_for_request(
        state,
        tenant,
        "benchmark conversion",
        consent_scope,
        TraceAllowedUse::BenchmarkGeneration,
    )
    .await?;
    let purpose = normalized_export_purpose(
        body.purpose.as_deref(),
        "trace_commons_benchmark_candidate_conversion",
    );
    let TraceCommonsMetadataView { records, derived } = read_reviewer_metadata_view(state, tenant)
        .await
        .map_err(internal_error)?;
    let accepted_by_submission = records
        .into_iter()
        .filter(TraceCommonsSubmissionRecord::is_benchmark_eligible)
        .filter(|record| body.status.is_none_or(|status| record.status == status))
        .filter(|record| {
            body.privacy_risk
                .is_none_or(|risk| record.privacy_risk == risk)
        })
        .filter(|record| consent_scope.is_none_or(|scope| record.consent_scopes.contains(&scope)))
        .filter(|record| {
            record_matches_export_policy_abac(
                record,
                tenant_policy.as_ref(),
                TraceAllowedUse::BenchmarkGeneration,
            )
        })
        .map(|record| (record.submission_id, record))
        .collect::<BTreeMap<_, _>>();
    let limit = resolve_export_limit(state, body.limit);

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
    let source_object_refs = revalidate_db_export_sources(
        state,
        tenant,
        &source_submission_ids,
        state.require_derived_export_object_refs,
    )
    .await
    .map_err(internal_error)?;
    append_derived_source_read_audits(
        state,
        tenant,
        &source_submission_ids,
        &source_object_refs,
        "benchmark_conversion",
        Some(&purpose),
    )
    .await
    .map_err(internal_error)?;
    let source_submission_ids_hash =
        source_submission_ids_hash("benchmark_conversion", &source_submission_ids);
    let audit_event = TraceCommonsAuditEvent::benchmark_conversion(
        tenant,
        conversion_id,
        candidates.len(),
        source_submission_ids_hash.clone(),
    );
    let audit_event_id = audit_event.event_id;
    let artifact = TraceBenchmarkConversionArtifact {
        artifact_schema_version: TRACE_BENCHMARK_CONVERSION_SCHEMA_VERSION.to_string(),
        tenant_id: tenant.tenant_id.clone(),
        tenant_storage_ref: tenant_storage_ref(&tenant.tenant_id),
        conversion_id,
        audit_event_id,
        purpose,
        registry: TraceBenchmarkRegistryMetadata::default(),
        evaluation: TraceBenchmarkEvaluationMetadata::default(),
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
    if !state.object_primary_derived_exports {
        write_benchmark_artifact(&state.root, &tenant.tenant_id, &artifact)
            .map_err(internal_error)?;
    }
    let provenance = TraceExportProvenanceManifest::new(
        &tenant.tenant_id,
        conversion_id,
        audit_event_id,
        TraceExportProvenanceKind::BenchmarkConversion,
        artifact.purpose.clone(),
        artifact.source_submission_ids.clone(),
        artifact.source_submission_ids_hash.clone(),
    );
    if !state.object_primary_derived_exports {
        write_export_provenance(
            &benchmark_provenance_path(&state.root, &tenant.tenant_id, conversion_id),
            &provenance,
        )
        .map_err(internal_error)?;
    }
    let artifact_object_ref_material = if state.db_mirror.is_some() {
        Some(
            trace_export_artifact_object_ref_material(
                state,
                &tenant.tenant_id,
                TraceArtifactKind::BenchmarkConversion,
                conversion_id,
                &benchmark_artifact_path(&state.root, &tenant.tenant_id, conversion_id),
                &artifact,
            )
            .map_err(internal_error)?,
        )
    } else {
        None
    };
    let mirror_result = mirror_benchmark_export_provenance_to_db(
        state,
        &artifact,
        artifact_object_ref_material.as_ref(),
    )
    .await;
    if let Err(error) = &mirror_result {
        tracing::warn!(
            %error,
            export_id = %conversion_id,
            "Trace Commons DB dual-write benchmark provenance mirror failed"
        );
    }
    enforce_db_mirror_write_result(state, "benchmark provenance", mirror_result)
        .map_err(internal_error)?;
    append_audit_event_with_db_mirror(
        state,
        tenant,
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
    append_automatic_utility_credit_events_once(
        state,
        tenant,
        AutomaticUtilityCreditConfig {
            idempotency_label: "benchmark-conversion-credit",
            idempotency_ref: None,
            event_type: TraceCreditLedgerEventType::BenchmarkConversion,
            credit_points_delta: BENCHMARK_CONVERSION_CREDIT_POINTS_DELTA,
            reason: format!(
                "Converted into benchmark artifact {}.",
                artifact.conversion_id
            ),
            external_ref: artifact
                .filters
                .external_ref
                .clone()
                .unwrap_or_else(|| format!("benchmark_conversion:{}", artifact.conversion_id)),
        },
        artifact
            .candidates
            .iter()
            .map(AutomaticUtilityCreditSource::from_benchmark_candidate),
    )
    .await
    .map_err(internal_error)?;
    Ok(Json(artifact))
}

async fn update_benchmark_lifecycle(
    state: &AppState,
    tenant: &TenantAuth,
    conversion_id: Uuid,
    body: BenchmarkLifecycleUpdateRequest,
) -> ApiResult<Json<TraceBenchmarkConversionArtifact>> {
    if body.registry.is_none() && body.evaluation.is_none() {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "benchmark lifecycle update requires registry or evaluation metadata",
        ));
    }

    let mut artifact = read_benchmark_conversion_artifact(state, tenant, conversion_id)
        .await
        .map_err(internal_error)?
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "benchmark artifact not found"))?;
    apply_benchmark_lifecycle_update(&mut artifact, body)?;

    persist_benchmark_lifecycle_artifact(state, tenant, &artifact, "benchmark lifecycle")
        .await
        .map_err(internal_error)?;

    Ok(Json(artifact))
}

async fn persist_benchmark_lifecycle_artifact(
    state: &AppState,
    tenant: &TenantAuth,
    artifact: &TraceBenchmarkConversionArtifact,
    operation: &str,
) -> anyhow::Result<()> {
    if !state.object_primary_derived_exports {
        write_benchmark_artifact(&state.root, &tenant.tenant_id, artifact)?;
    }
    let artifact_object_ref_material = if state.db_mirror.is_some() {
        Some(trace_export_artifact_object_ref_material(
            state,
            &tenant.tenant_id,
            TraceArtifactKind::BenchmarkConversion,
            artifact.conversion_id,
            &benchmark_artifact_path(&state.root, &tenant.tenant_id, artifact.conversion_id),
            artifact,
        )?)
    } else {
        None
    };
    let mirror_result = mirror_benchmark_export_provenance_to_db(
        state,
        artifact,
        artifact_object_ref_material.as_ref(),
    )
    .await;
    if let Err(error) = &mirror_result {
        tracing::warn!(
            %error,
            export_id = %artifact.conversion_id,
            "Trace Commons DB dual-write benchmark lifecycle mirror failed"
        );
    }
    enforce_db_mirror_write_result(state, operation, mirror_result)?;

    append_audit_event_with_db_mirror(
        state,
        tenant,
        TraceCommonsAuditEvent::benchmark_lifecycle_update(tenant, artifact),
        StorageTraceAuditAction::BenchmarkConvert,
        StorageTraceAuditSafeMetadata::Export {
            artifact_kind: StorageTraceObjectArtifactKind::BenchmarkArtifact,
            purpose_code: Some(artifact.purpose.clone()),
            item_count: artifact.item_count.min(u32::MAX as usize) as u32,
        },
    )
    .await?;
    Ok(())
}

#[derive(Debug, Deserialize)]
struct RankerTrainingExportQuery {
    limit: Option<usize>,
    #[serde(default)]
    purpose: Option<String>,
    #[serde(default)]
    status: Option<TraceCorpusStatus>,
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
    require_exporter(&tenant)?;
    let consent_scope = parse_ranker_consent_scope_filter(query.consent_scope.as_deref())?;
    let purpose = normalized_export_purpose(
        query.purpose.as_deref(),
        "ranker_training_candidates_export",
    );
    enforce_ranker_export_guardrails(
        state.as_ref(),
        query.purpose.as_deref(),
        query.status,
        query.privacy_risk,
        consent_scope,
    )?;
    let tenant_policy = tenant_export_policy_for_request(
        state.as_ref(),
        &tenant,
        "ranker training candidates",
        consent_scope,
        TraceAllowedUse::RankingModelTraining,
    )
    .await?;
    let mut candidate_query = query;
    candidate_query.limit = Some(resolve_export_limit(state.as_ref(), candidate_query.limit));
    let candidates = collect_ranker_training_candidates(
        state.as_ref(),
        &tenant,
        &candidate_query,
        consent_scope,
        tenant_policy.as_ref(),
    )
    .await
    .map_err(internal_error)?;
    let export_id = Uuid::new_v4();
    let source_submission_ids = candidates
        .iter()
        .map(|candidate| candidate.submission_id)
        .collect::<Vec<_>>();
    let source_object_refs = revalidate_db_export_sources(
        state.as_ref(),
        &tenant,
        &source_submission_ids,
        state.require_derived_export_object_refs,
    )
    .await
    .map_err(internal_error)?;
    append_derived_source_read_audits(
        state.as_ref(),
        &tenant,
        &source_submission_ids,
        &source_object_refs,
        "ranker_training_candidates",
        Some(&purpose),
    )
    .await
    .map_err(internal_error)?;
    let source_item_list_hash =
        source_submission_ids_hash("ranker_training_candidates_export", &source_submission_ids);
    let audit_event = TraceCommonsAuditEvent::ranker_training_export(
        &tenant,
        export_id,
        purpose.as_str(),
        candidates.len(),
        source_item_list_hash.clone(),
    );
    let audit_event_id = audit_event.event_id;
    let provenance = TraceExportProvenanceManifest::new(
        &tenant.tenant_id,
        export_id,
        audit_event_id,
        TraceExportProvenanceKind::RankerTrainingCandidates,
        purpose.clone(),
        source_submission_ids,
        source_item_list_hash.clone(),
    );
    if !state.object_primary_derived_exports {
        write_export_provenance(
            &ranker_provenance_path(&state.root, &tenant.tenant_id, export_id),
            &provenance,
        )
        .map_err(internal_error)?;
    }
    let artifact_object_ref_material = if state.db_mirror.is_some() {
        Some(
            trace_export_artifact_object_ref_material(
                state.as_ref(),
                &tenant.tenant_id,
                TraceArtifactKind::RankerTrainingExport,
                export_id,
                &ranker_provenance_path(&state.root, &tenant.tenant_id, export_id),
                &provenance,
            )
            .map_err(internal_error)?,
        )
    } else {
        None
    };
    let mirror_result = mirror_ranker_candidate_export_provenance_to_db(
        state.as_ref(),
        &provenance,
        &candidates,
        artifact_object_ref_material.as_ref(),
    )
    .await;
    if let Err(error) = &mirror_result {
        tracing::warn!(
            %error,
            export_id = %export_id,
            "Trace Commons DB dual-write ranker candidate provenance mirror failed"
        );
    }
    enforce_db_mirror_write_result(state.as_ref(), "ranker candidate provenance", mirror_result)
        .map_err(internal_error)?;
    append_audit_event_with_db_mirror(
        state.as_ref(),
        &tenant,
        audit_event,
        StorageTraceAuditAction::Export,
        StorageTraceAuditSafeMetadata::Export {
            artifact_kind: StorageTraceObjectArtifactKind::ExportArtifact,
            purpose_code: Some(purpose.clone()),
            item_count: candidates.len().min(u32::MAX as usize) as u32,
        },
    )
    .await
    .map_err(internal_error)?;
    append_automatic_utility_credit_events_once(
        state.as_ref(),
        &tenant,
        AutomaticUtilityCreditConfig {
            idempotency_label: "ranker-training-candidate-credit",
            idempotency_ref: None,
            event_type: TraceCreditLedgerEventType::TrainingUtility,
            credit_points_delta: RANKER_TRAINING_CANDIDATE_CREDIT_POINTS_DELTA,
            reason: format!("Exported as ranker training candidate {}.", export_id),
            external_ref: format!("ranker_training_candidates_export:{export_id}"),
        },
        candidates
            .iter()
            .map(AutomaticUtilityCreditSource::from_ranker_candidate),
    )
    .await
    .map_err(internal_error)?;
    Ok(Json(TraceRankerTrainingCandidateExport {
        tenant_id: tenant.tenant_id.clone(),
        tenant_storage_ref: tenant_storage_ref(&tenant.tenant_id),
        export_id,
        audit_event_id,
        purpose,
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
    require_exporter(&tenant)?;
    let consent_scope = parse_ranker_consent_scope_filter(query.consent_scope.as_deref())?;
    let purpose =
        normalized_export_purpose(query.purpose.as_deref(), "ranker_training_pairs_export");
    enforce_ranker_export_guardrails(
        state.as_ref(),
        query.purpose.as_deref(),
        query.status,
        query.privacy_risk,
        consent_scope,
    )?;
    let tenant_policy = tenant_export_policy_for_request(
        state.as_ref(),
        &tenant,
        "ranker training pairs",
        consent_scope,
        TraceAllowedUse::RankingModelTraining,
    )
    .await?;
    let mut pair_query = query;
    let pair_limit = resolve_export_limit(state.as_ref(), pair_query.limit);
    pair_query.limit = Some(pair_limit.saturating_add(1));
    let candidates = collect_ranker_training_candidates(
        state.as_ref(),
        &tenant,
        &pair_query,
        consent_scope,
        tenant_policy.as_ref(),
    )
    .await
    .map_err(internal_error)?;
    let pairs = build_ranker_training_pairs(&candidates, pair_limit);
    let source_submission_ids = ranker_pair_source_submission_ids(&pairs);
    let source_object_refs = revalidate_db_export_sources(
        state.as_ref(),
        &tenant,
        &source_submission_ids,
        state.require_derived_export_object_refs,
    )
    .await
    .map_err(internal_error)?;
    append_derived_source_read_audits(
        state.as_ref(),
        &tenant,
        &source_submission_ids,
        &source_object_refs,
        "ranker_training_pairs",
        Some(&purpose),
    )
    .await
    .map_err(internal_error)?;
    let export_id = Uuid::new_v4();
    let source_item_list_hash = ranker_pair_list_hash(&pairs);
    let audit_event = TraceCommonsAuditEvent::ranker_training_export(
        &tenant,
        export_id,
        purpose.as_str(),
        pairs.len(),
        source_item_list_hash.clone(),
    );
    let audit_event_id = audit_event.event_id;
    let provenance = TraceExportProvenanceManifest::new(
        &tenant.tenant_id,
        export_id,
        audit_event_id,
        TraceExportProvenanceKind::RankerTrainingPairs,
        purpose.clone(),
        source_submission_ids,
        source_item_list_hash.clone(),
    );
    if !state.object_primary_derived_exports {
        write_export_provenance(
            &ranker_provenance_path(&state.root, &tenant.tenant_id, export_id),
            &provenance,
        )
        .map_err(internal_error)?;
    }
    let artifact_object_ref_material = if state.db_mirror.is_some() {
        Some(
            trace_export_artifact_object_ref_material(
                state.as_ref(),
                &tenant.tenant_id,
                TraceArtifactKind::RankerTrainingExport,
                export_id,
                &ranker_provenance_path(&state.root, &tenant.tenant_id, export_id),
                &provenance,
            )
            .map_err(internal_error)?,
        )
    } else {
        None
    };
    let mirror_result = mirror_ranker_pair_export_provenance_to_db(
        state.as_ref(),
        &provenance,
        &pairs,
        artifact_object_ref_material.as_ref(),
    )
    .await;
    if let Err(error) = &mirror_result {
        tracing::warn!(
            %error,
            export_id = %export_id,
            "Trace Commons DB dual-write ranker pair provenance mirror failed"
        );
    }
    enforce_db_mirror_write_result(state.as_ref(), "ranker pair provenance", mirror_result)
        .map_err(internal_error)?;
    append_audit_event_with_db_mirror(
        state.as_ref(),
        &tenant,
        audit_event,
        StorageTraceAuditAction::Export,
        StorageTraceAuditSafeMetadata::Export {
            artifact_kind: StorageTraceObjectArtifactKind::ExportArtifact,
            purpose_code: Some(purpose.clone()),
            item_count: pairs.len().min(u32::MAX as usize) as u32,
        },
    )
    .await
    .map_err(internal_error)?;
    let pair_credit_sources = pairs
        .iter()
        .flat_map(|pair| {
            [
                AutomaticUtilityCreditSource::from_ranker_candidate(&pair.preferred),
                AutomaticUtilityCreditSource::from_ranker_candidate(&pair.rejected),
            ]
        })
        .collect::<Vec<_>>();
    append_automatic_utility_credit_events_once(
        state.as_ref(),
        &tenant,
        AutomaticUtilityCreditConfig {
            idempotency_label: "ranker-training-pair-credit",
            idempotency_ref: None,
            event_type: TraceCreditLedgerEventType::RankingUtility,
            credit_points_delta: RANKER_TRAINING_PAIR_CREDIT_POINTS_DELTA,
            reason: format!("Exported as ranker training pair {}.", export_id),
            external_ref: format!("ranker_training_pairs_export:{export_id}"),
        },
        pair_credit_sources,
    )
    .await
    .map_err(internal_error)?;
    Ok(Json(TraceRankerTrainingPairExport {
        tenant_id: tenant.tenant_id.clone(),
        tenant_storage_ref: tenant_storage_ref(&tenant.tenant_id),
        export_id,
        audit_event_id,
        purpose,
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
    let limit = query.limit.unwrap_or(100).clamp(1, 500);
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
    #[serde(default)]
    verify_audit_chain: bool,
    #[serde(default = "default_true")]
    prune_export_cache: bool,
    #[serde(default)]
    max_export_age_hours: Option<i64>,
    #[serde(default)]
    purge_expired_before: Option<DateTime<Utc>>,
}

impl TraceMaintenanceRequest {
    fn is_retention_worker_request(&self) -> bool {
        !self.backfill_db_mirror
            && !self.index_vectors
            && !self.reconcile_db_mirror
            && !self.verify_audit_chain
    }

    fn is_vector_worker_request(&self) -> bool {
        self.index_vectors
            && !self.backfill_db_mirror
            && !self.reconcile_db_mirror
            && !self.verify_audit_chain
            && !self.prune_export_cache
            && self.max_export_age_hours.is_none()
            && self.purge_expired_before.is_none()
    }
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
    require_maintenance_operator(&tenant, &body)?;
    require_purge_purpose(
        body.dry_run,
        body.purge_expired_before,
        body.purpose.as_deref(),
    )?;
    let response = run_maintenance(state.as_ref(), &tenant, body)
        .await
        .map_err(maintenance_error)?;
    Ok(Json(response))
}

#[derive(Debug, Deserialize)]
struct TraceRetentionMaintenanceRequest {
    #[serde(default)]
    purpose: Option<String>,
    #[serde(default)]
    dry_run: bool,
    #[serde(default = "default_true")]
    prune_export_cache: bool,
    #[serde(default)]
    max_export_age_hours: Option<i64>,
    #[serde(default)]
    purge_expired_before: Option<DateTime<Utc>>,
}

async fn retention_maintenance_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<TraceRetentionMaintenanceRequest>,
) -> ApiResult<Json<TraceMaintenanceResponse>> {
    let tenant = authenticate(state.as_ref(), &headers)?;
    require_retention_operator(&tenant)?;
    require_purge_purpose(
        body.dry_run,
        body.purge_expired_before,
        body.purpose.as_deref(),
    )?;
    let response = run_maintenance(
        state.as_ref(),
        &tenant,
        TraceMaintenanceRequest {
            purpose: Some(
                body.purpose
                    .unwrap_or_else(|| "trace_commons_retention_worker".to_string()),
            ),
            dry_run: body.dry_run,
            backfill_db_mirror: false,
            index_vectors: false,
            reconcile_db_mirror: false,
            verify_audit_chain: false,
            prune_export_cache: body.prune_export_cache,
            max_export_age_hours: body.max_export_age_hours,
            purge_expired_before: body.purge_expired_before,
        },
    )
    .await
    .map_err(maintenance_error)?;
    Ok(Json(response))
}

fn require_purge_purpose(
    dry_run: bool,
    purge_expired_before: Option<DateTime<Utc>>,
    purpose: Option<&str>,
) -> ApiResult<()> {
    if dry_run || purge_expired_before.is_none() {
        return Ok(());
    }

    if purpose
        .map(str::trim)
        .filter(|purpose| !purpose.is_empty())
        .is_some()
    {
        return Ok(());
    }

    Err(api_error(
        StatusCode::BAD_REQUEST,
        "purging expired traces requires an explicit purpose",
    ))
}

#[derive(Debug, Deserialize)]
struct TraceVectorIndexRequest {
    #[serde(default)]
    purpose: Option<String>,
    #[serde(default)]
    dry_run: bool,
}

async fn vector_index_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<TraceVectorIndexRequest>,
) -> ApiResult<Json<TraceMaintenanceResponse>> {
    let tenant = authenticate(state.as_ref(), &headers)?;
    require_vector_operator(&tenant)?;
    let response = run_maintenance(
        state.as_ref(),
        &tenant,
        TraceMaintenanceRequest {
            purpose: Some(
                body.purpose
                    .unwrap_or_else(|| "trace_commons_vector_index_worker".to_string()),
            ),
            dry_run: body.dry_run,
            backfill_db_mirror: false,
            index_vectors: true,
            reconcile_db_mirror: false,
            verify_audit_chain: false,
            prune_export_cache: false,
            max_export_age_hours: None,
            purge_expired_before: None,
        },
    )
    .await
    .map_err(maintenance_error)?;
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
    let events: Vec<_> = read_audit_events(state.as_ref(), &tenant)
        .await
        .map_err(internal_error)?
        .into_iter()
        .rev()
        .take(limit)
        .collect();
    append_audit_event_with_db_mirror(
        state.as_ref(),
        &tenant,
        TraceCommonsAuditEvent::read(&tenant, "audit_events", events.len()),
        StorageTraceAuditAction::Read,
        StorageTraceAuditSafeMetadata::Empty,
    )
    .await
    .map_err(internal_error)?;
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
    tenant_policy: Option<&TenantSubmissionPolicy>,
) -> anyhow::Result<Vec<TraceRankerTrainingCandidate>> {
    let TraceCommonsMetadataView { records, derived } =
        read_reviewer_metadata_view(state, tenant).await?;
    let derived_by_submission = derived
        .into_iter()
        .map(|record| (record.submission_id, record))
        .collect::<BTreeMap<_, _>>();
    let limit = query
        .limit
        .unwrap_or(DEFAULT_TRACE_COMMONS_MAX_EXPORT_ITEMS_PER_REQUEST);
    let requested_status = query.status.unwrap_or(TraceCorpusStatus::Accepted);
    let mut candidates = records
        .into_iter()
        .filter(|record| record.status == requested_status)
        .filter(|record| matches!(record.status, TraceCorpusStatus::Accepted))
        .filter(|record| !record.is_revoked())
        .filter(|record| {
            query
                .privacy_risk
                .is_none_or(|risk| record.privacy_risk == risk)
        })
        .filter(|record| ranker_consent_matches(&record.consent_scopes, consent_scope))
        .filter(|record| {
            record_matches_export_policy_abac(
                record,
                tenant_policy,
                TraceAllowedUse::RankingModelTraining,
            )
        })
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

fn normalized_export_purpose(value: Option<&str>, default: &str) -> String {
    value
        .map(str::trim)
        .filter(|purpose| !purpose.is_empty())
        .unwrap_or(default)
        .to_string()
}

fn resolve_export_limit(state: &AppState, requested_limit: Option<usize>) -> usize {
    requested_limit
        .unwrap_or(100)
        .clamp(1, state.max_export_items_per_request)
}

fn enforce_dataset_export_guardrails(
    state: &AppState,
    export_kind: &str,
    purpose: Option<&str>,
    status: Option<TraceCorpusStatus>,
    privacy_risk: Option<ResidualPiiRisk>,
    consent_scope: Option<ConsentScope>,
) -> ApiResult<()> {
    if !state.require_export_guardrails {
        return Ok(());
    }

    if purpose
        .map(str::trim)
        .filter(|purpose| !purpose.is_empty())
        .is_none()
    {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            format!("{export_kind} export requires an explicit purpose"),
        ));
    }
    if status != Some(TraceCorpusStatus::Accepted) {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            format!("{export_kind} export requires status=accepted"),
        ));
    }
    if privacy_risk != Some(ResidualPiiRisk::Low) {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            format!("{export_kind} export requires privacy_risk=low"),
        ));
    }
    if consent_scope.is_none() {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            format!("{export_kind} export requires an explicit consent_scope"),
        ));
    }

    Ok(())
}

fn enforce_ranker_export_guardrails(
    state: &AppState,
    purpose: Option<&str>,
    status: Option<TraceCorpusStatus>,
    privacy_risk: Option<ResidualPiiRisk>,
    consent_scope: Option<ConsentScope>,
) -> ApiResult<()> {
    if !state.require_export_guardrails {
        return Ok(());
    }

    if purpose
        .map(str::trim)
        .filter(|purpose| !purpose.is_empty())
        .is_none()
    {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "ranker training export requires an explicit purpose",
        ));
    }
    if status != Some(TraceCorpusStatus::Accepted) {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "ranker training export requires status=accepted",
        ));
    }
    if privacy_risk != Some(ResidualPiiRisk::Low) {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "ranker training export requires privacy_risk=low",
        ));
    }
    if consent_scope.is_none() {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "ranker training export requires explicit ranking-training or model-training consent_scope",
        ));
    }

    Ok(())
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

fn ensure_not_revoked_by_tombstone(
    tombstones: &[TraceCommonsRevocation],
    submission_id: Uuid,
    redaction_hash: &str,
    canonical_summary_hash: &str,
) -> ApiResult<()> {
    let redaction_hash = redaction_hash.trim();
    let canonical_summary_hash = canonical_summary_hash.trim();
    let revoked_match = tombstones.iter().any(|tombstone| {
        tombstone.submission_id == submission_id
            || (!redaction_hash.is_empty()
                && tombstone.redaction_hash.as_deref() == Some(redaction_hash))
            || tombstone.canonical_summary_hash.as_deref() == Some(canonical_summary_hash)
    });
    if revoked_match {
        return Err(api_error(
            StatusCode::CONFLICT,
            "trace content was previously revoked for this tenant",
        ));
    }
    Ok(())
}

fn redaction_hash_for_record(
    state: &AppState,
    record: &TraceCommonsSubmissionRecord,
) -> Option<String> {
    match read_envelope_by_record(state, record) {
        Ok(envelope) => Some(envelope.privacy.redaction_hash),
        Err(error) => {
            tracing::warn!(
                %error,
                submission_id = %record.submission_id,
                "Trace Commons revocation could not read stored envelope redaction hash"
            );
            None
        }
    }
}

async fn tenant_submission_policy_for_request(
    state: &AppState,
    tenant: &TenantAuth,
) -> ApiResult<Option<TenantSubmissionPolicy>> {
    if state.db_tenant_policy_reads {
        let db = state
            .db_mirror
            .as_ref()
            .ok_or_else(|| internal_error("DB tenant policy reads require DB mirror"))?;
        let policy = db
            .get_trace_tenant_policy(&tenant.tenant_id)
            .await
            .map_err(internal_error)?;
        return policy
            .map(tenant_submission_policy_from_storage)
            .transpose()
            .map_err(internal_error);
    }

    Ok(state.tenant_policies.get(&tenant.tenant_id).cloned())
}

fn tenant_submission_policy_from_storage(
    policy: StorageTraceTenantPolicyRecord,
) -> anyhow::Result<TenantSubmissionPolicy> {
    Ok(TenantSubmissionPolicy {
        allowed_consent_scopes: parse_storage_policy_values(
            &policy.allowed_consent_scopes,
            "allowed_consent_scopes",
        )?,
        allowed_uses: parse_storage_policy_values(&policy.allowed_uses, "allowed_uses")?,
    })
}

fn parse_storage_policy_values<T>(values: &[String], label: &str) -> anyhow::Result<BTreeSet<T>>
where
    T: for<'de> Deserialize<'de> + Ord,
{
    values
        .iter()
        .map(|value| {
            serde_json::from_value::<T>(serde_json::Value::String(value.clone()))
                .with_context(|| format!("failed to parse trace tenant policy {label} value"))
        })
        .collect()
}

fn enforce_tenant_submission_policy(
    tenant: &TenantAuth,
    envelope: &TraceContributionEnvelope,
    policy: Option<&TenantSubmissionPolicy>,
    require_policy: bool,
) -> ApiResult<()> {
    let Some(policy) = policy else {
        if require_policy {
            tracing::warn!(
                tenant_id = %tenant.tenant_id,
                "Trace Commons tenant policy rejected submission without tenant policy"
            );
            return Err(api_error(
                StatusCode::FORBIDDEN,
                "trace contribution tenant does not have a submission policy",
            ));
        }
        return Ok(());
    };

    if !policy.allowed_consent_scopes.is_empty() {
        let mut requested_scopes = envelope
            .consent
            .scopes
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        requested_scopes.insert(envelope.trace_card.consent_scope);
        if requested_scopes.is_empty() {
            return Err(api_error(
                StatusCode::FORBIDDEN,
                "tenant policy requires an allowed trace contribution consent scope",
            ));
        }
        if let Some(scope) = requested_scopes
            .iter()
            .find(|scope| !policy.allowed_consent_scopes.contains(scope))
        {
            tracing::warn!(
                tenant_id = %tenant.tenant_id,
                ?scope,
                "Trace Commons tenant policy rejected disallowed consent scope"
            );
            return Err(api_error(
                StatusCode::FORBIDDEN,
                "trace contribution consent scope is not allowed for this tenant",
            ));
        }
    }

    if !policy.allowed_uses.is_empty() {
        if envelope.trace_card.allowed_uses.is_empty() {
            return Err(api_error(
                StatusCode::FORBIDDEN,
                "tenant policy requires trace contribution allowed uses",
            ));
        }
        if let Some(allowed_use) = envelope
            .trace_card
            .allowed_uses
            .iter()
            .find(|allowed_use| !policy.allowed_uses.contains(allowed_use))
        {
            tracing::warn!(
                tenant_id = %tenant.tenant_id,
                ?allowed_use,
                "Trace Commons tenant policy rejected disallowed allowed use"
            );
            return Err(api_error(
                StatusCode::FORBIDDEN,
                "trace contribution allowed use is not allowed for this tenant",
            ));
        }
    }

    Ok(())
}

fn enforce_submission_quota(state: &AppState, tenant: &TenantAuth) -> ApiResult<()> {
    let quota = state.submission_quota;
    if quota.is_disabled() || tenant.role != TokenRole::Contributor {
        return Ok(());
    }

    let window_start = Utc::now() - Duration::hours(1);
    let records =
        read_all_submission_records(&state.root, &tenant.tenant_id).map_err(internal_error)?;
    if quota.max_per_tenant_per_hour > 0 {
        let tenant_count = records
            .iter()
            .filter(|record| submission_counts_toward_quota(record))
            .filter(|record| record.received_at >= window_start)
            .count();
        if tenant_count >= quota.max_per_tenant_per_hour {
            tracing::warn!(
                tenant_id = %tenant.tenant_id,
                limit = quota.max_per_tenant_per_hour,
                "Trace Commons tenant submission quota exceeded"
            );
            return Err(api_error(
                StatusCode::TOO_MANY_REQUESTS,
                "trace contribution tenant submission quota exceeded",
            ));
        }
    }

    if quota.max_per_principal_per_hour > 0 {
        let principal_count = records
            .iter()
            .filter(|record| submission_counts_toward_quota(record))
            .filter(|record| {
                record.auth_principal_ref == tenant.principal_ref
                    && record.received_at >= window_start
            })
            .count();
        if principal_count >= quota.max_per_principal_per_hour {
            tracing::warn!(
                tenant_id = %tenant.tenant_id,
                principal_ref = %tenant.principal_ref,
                limit = quota.max_per_principal_per_hour,
                "Trace Commons principal submission quota exceeded"
            );
            return Err(api_error(
                StatusCode::TOO_MANY_REQUESTS,
                "trace contribution principal submission quota exceeded",
            ));
        }
    }

    Ok(())
}

fn submission_counts_toward_quota(record: &TraceCommonsSubmissionRecord) -> bool {
    matches!(
        record.status,
        TraceCorpusStatus::Accepted | TraceCorpusStatus::Quarantined
    )
}

async fn tenant_export_policy_for_request(
    state: &AppState,
    tenant: &TenantAuth,
    surface: &str,
    requested_scope: Option<ConsentScope>,
    required_use: TraceAllowedUse,
) -> ApiResult<Option<TenantSubmissionPolicy>> {
    let policy = tenant_submission_policy_for_request(state, tenant).await?;
    enforce_tenant_export_policy(
        tenant,
        surface,
        policy.as_ref(),
        state.require_tenant_submission_policy,
        requested_scope,
        required_use,
    )?;
    Ok(policy)
}

fn enforce_tenant_export_policy(
    tenant: &TenantAuth,
    surface: &str,
    policy: Option<&TenantSubmissionPolicy>,
    require_policy: bool,
    requested_scope: Option<ConsentScope>,
    required_use: TraceAllowedUse,
) -> ApiResult<()> {
    let Some(policy) = policy else {
        if require_policy {
            tracing::warn!(
                tenant_id = %tenant.tenant_id,
                surface,
                "Trace Commons tenant policy rejected export without tenant policy"
            );
            return Err(api_error(
                StatusCode::FORBIDDEN,
                "trace export tenant does not have a contribution policy",
            ));
        }
        return Ok(());
    };

    if let Some(scope) = requested_scope
        && !policy.allowed_consent_scopes.is_empty()
        && !policy.allowed_consent_scopes.contains(&scope)
    {
        tracing::warn!(
            tenant_id = %tenant.tenant_id,
            surface,
            ?scope,
            "Trace Commons tenant policy rejected export consent scope"
        );
        return Err(api_error(
            StatusCode::FORBIDDEN,
            "trace export consent scope is not allowed for this tenant",
        ));
    }

    if !policy.allowed_uses.is_empty() && !policy.allowed_uses.contains(&required_use) {
        tracing::warn!(
            tenant_id = %tenant.tenant_id,
            surface,
            ?required_use,
            "Trace Commons tenant policy rejected export allowed use"
        );
        return Err(api_error(
            StatusCode::FORBIDDEN,
            "trace export use is not allowed for this tenant",
        ));
    }

    Ok(())
}

fn record_matches_export_policy_abac(
    record: &TraceCommonsSubmissionRecord,
    policy: Option<&TenantSubmissionPolicy>,
    required_use: TraceAllowedUse,
) -> bool {
    let Some(policy) = policy else {
        return true;
    };

    if !policy.allowed_consent_scopes.is_empty()
        && !record
            .consent_scopes
            .iter()
            .any(|scope| policy.allowed_consent_scopes.contains(scope))
    {
        return false;
    }

    policy.allowed_uses.is_empty() || record.allowed_uses.contains(&required_use)
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

fn require_exporter(auth: &TenantAuth) -> ApiResult<()> {
    if auth.role.can_export() {
        Ok(())
    } else {
        Err(api_error(
            StatusCode::FORBIDDEN,
            "reviewer, admin, or export worker token required",
        ))
    }
}

fn require_benchmarker(auth: &TenantAuth) -> ApiResult<()> {
    if auth.role.can_benchmark() {
        Ok(())
    } else {
        Err(api_error(
            StatusCode::FORBIDDEN,
            "reviewer, admin, or benchmark worker token required",
        ))
    }
}

fn require_vector_operator(auth: &TenantAuth) -> ApiResult<()> {
    if auth.role.can_review() || auth.role == TokenRole::VectorWorker {
        Ok(())
    } else {
        Err(api_error(
            StatusCode::FORBIDDEN,
            "reviewer, admin, or vector worker token required",
        ))
    }
}

fn require_retention_operator(auth: &TenantAuth) -> ApiResult<()> {
    if auth.role.can_review() || auth.role == TokenRole::RetentionWorker {
        Ok(())
    } else {
        Err(api_error(
            StatusCode::FORBIDDEN,
            "reviewer, admin, or retention worker token required",
        ))
    }
}

fn require_utility_operator(auth: &TenantAuth) -> ApiResult<()> {
    if auth.role.can_review() || auth.role == TokenRole::UtilityWorker {
        Ok(())
    } else {
        Err(api_error(
            StatusCode::FORBIDDEN,
            "reviewer, admin, or utility worker token required",
        ))
    }
}

fn require_maintenance_operator(
    auth: &TenantAuth,
    request: &TraceMaintenanceRequest,
) -> ApiResult<()> {
    if auth.role.can_review() {
        return Ok(());
    }
    if auth.role == TokenRole::RetentionWorker && request.is_retention_worker_request() {
        return Ok(());
    }
    if auth.role == TokenRole::VectorWorker && request.is_vector_worker_request() {
        return Ok(());
    }
    Err(api_error(
        StatusCode::FORBIDDEN,
        "reviewer/admin token or matching maintenance worker token required",
    ))
}

fn require_admin(auth: &TenantAuth) -> ApiResult<()> {
    if auth.role.can_admin() {
        Ok(())
    } else {
        Err(api_error(StatusCode::FORBIDDEN, "admin token required"))
    }
}

fn can_access_submission(auth: &TenantAuth, record: &TraceCommonsSubmissionRecord) -> bool {
    auth.role.can_review()
        || record.auth_principal_ref == legacy_principal_ref()
        || record.auth_principal_ref == auth.principal_ref
}

fn can_access_storage_submission(auth: &TenantAuth, record: &StorageTraceSubmissionRecord) -> bool {
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
        .filter(|record| !record.is_terminal())
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

#[derive(Debug)]
struct ReviewDecisionRecord {
    record: TraceCommonsSubmissionRecord,
    canonical_summary_hash: Option<String>,
    file_record_available: bool,
    allow_file_body_fallback: bool,
}

async fn read_reviewer_submission_record(
    state: &AppState,
    tenant: &TenantAuth,
    submission_id: Uuid,
) -> anyhow::Result<Option<TraceCommonsSubmissionRecord>> {
    if state.db_reviewer_reads {
        let db = state
            .db_mirror
            .as_ref()
            .context("TRACE_COMMONS_DB_REVIEWER_READS is enabled without a DB mirror")?;
        let Some(storage_record) = db
            .get_trace_submission(&tenant.tenant_id, submission_id)
            .await
            .context("failed to read Trace Commons reviewer submission from DB mirror")?
        else {
            return Ok(None);
        };
        if !can_access_storage_submission(tenant, &storage_record) {
            return Ok(None);
        }
        let Some(record) = trace_commons_record_from_storage_submission(storage_record) else {
            return Ok(None);
        };
        return Ok(Some(record?));
    }

    let Some(record) = read_submission_record(&state.root, &tenant.tenant_id, submission_id)?
    else {
        return Ok(None);
    };
    if !can_access_submission(tenant, &record) {
        return Ok(None);
    }
    Ok(Some(record))
}

async fn read_utility_submission_record(
    state: &AppState,
    tenant: &TenantAuth,
    submission_id: Uuid,
) -> anyhow::Result<Option<TraceCommonsSubmissionRecord>> {
    if state.db_reviewer_reads {
        let db = state
            .db_mirror
            .as_ref()
            .context("TRACE_COMMONS_DB_REVIEWER_READS is enabled without a DB mirror")?;
        let Some(storage_record) = db
            .get_trace_submission(&tenant.tenant_id, submission_id)
            .await
            .context("failed to read Trace Commons utility submission from DB mirror")?
        else {
            return Ok(None);
        };
        let Some(record) = trace_commons_record_from_storage_submission(storage_record) else {
            return Ok(None);
        };
        return Ok(Some(record?));
    }

    read_submission_record(&state.root, &tenant.tenant_id, submission_id)
}

async fn read_review_decision_record(
    state: &AppState,
    tenant: &TenantAuth,
    submission_id: Uuid,
) -> anyhow::Result<Option<ReviewDecisionRecord>> {
    if state.db_reviewer_reads {
        let db = state
            .db_mirror
            .as_ref()
            .context("TRACE_COMMONS_DB_REVIEWER_READS is enabled without a DB mirror")?;
        let Some(storage_record) = db
            .get_trace_submission(&tenant.tenant_id, submission_id)
            .await
            .context("failed to read Trace Commons review submission from DB mirror")?
        else {
            return Ok(None);
        };
        if !can_access_storage_submission(tenant, &storage_record) {
            return Ok(None);
        }

        let canonical_summary_hash = storage_record.canonical_summary_hash.clone();
        let Some(record) = trace_commons_record_from_storage_submission(storage_record) else {
            return Ok(None);
        };
        let file_record_available =
            submission_metadata_path(&state.root, &tenant.tenant_id, submission_id).exists();
        return Ok(Some(ReviewDecisionRecord {
            record: record?,
            canonical_summary_hash,
            file_record_available,
            allow_file_body_fallback: file_record_available,
        }));
    }

    let Some(record) = read_submission_record(&state.root, &tenant.tenant_id, submission_id)?
    else {
        return Ok(None);
    };
    Ok(Some(ReviewDecisionRecord {
        record,
        canonical_summary_hash: None,
        file_record_available: true,
        allow_file_body_fallback: true,
    }))
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

async fn trace_list_purpose_submission_ids(
    state: &AppState,
    tenant: &TenantAuth,
    purpose: Option<&str>,
) -> anyhow::Result<Option<BTreeSet<Uuid>>> {
    let Some(purpose) = purpose.map(str::trim).filter(|purpose| !purpose.is_empty()) else {
        return Ok(None);
    };

    if state.db_reviewer_reads {
        let db = state
            .db_mirror
            .as_ref()
            .context("TRACE_COMMONS_DB_REVIEWER_READS is enabled without a DB mirror")?;
        let submission_ids = db
            .list_trace_export_manifests(&tenant.tenant_id)
            .await
            .context("failed to read Trace Commons export manifests from DB mirror")?
            .into_iter()
            .filter(|manifest| {
                manifest.deleted_at.is_none()
                    && manifest.invalidated_at.is_none()
                    && storage_manifest_purpose_matches(manifest.purpose_code.as_deref(), purpose)
            })
            .flat_map(|manifest| manifest.source_submission_ids)
            .collect::<BTreeSet<_>>();
        return Ok(Some(submission_ids));
    }

    let mut submission_ids = BTreeSet::new();
    for manifest in read_all_export_manifests(&state.root, &tenant.tenant_id)? {
        if manifest.purpose == purpose {
            submission_ids.extend(manifest.source_submission_ids);
        }
    }
    for path in read_export_provenance_paths(&state.root, &tenant.tenant_id)? {
        let provenance = read_export_provenance(&path)?;
        if provenance.purpose == purpose && provenance.invalidated_at.is_none() {
            submission_ids.extend(provenance.source_submission_ids);
        }
    }
    Ok(Some(submission_ids))
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
            allowed_uses: record
                .allowed_uses
                .iter()
                .map(|allowed_use| storage_string_as(allowed_use, "allowed_use"))
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
        StorageTraceAuditSafeMetadata::CreditMutation {
            event_type: _,
            credit_points_delta_micros: _,
            reason_hash: _,
            external_ref_hash: _,
        } => (None, event.reason.clone(), None),
        StorageTraceAuditSafeMetadata::TenantPolicy {
            policy_version: _,
            allowed_consent_scope_count: _,
            allowed_use_count: _,
            policy_projection_hash: _,
        } => (None, event.reason.clone(), None),
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
        previous_event_hash: event.previous_event_hash,
        event_hash: event.event_hash,
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
        StorageTraceAuditAction::PolicyUpdate => "tenant_policy_update",
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
        StorageTraceCreditEventType::RankingUtility => {
            Some(TraceCreditLedgerEventType::RankingUtility)
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
    let ledger_points = if delayed_credit_applies_to_record(record) {
        delayed_events
            .iter()
            .map(|event| event.credit_points_delta)
            .sum::<f32>()
    } else {
        0.0
    };
    let base_final = record.credit_points_final.unwrap_or(0.0);
    let credit_points_total = if delayed_events.is_empty() {
        None
    } else {
        Some(base_final + ledger_points)
    };
    let delayed_credit_explanations = if record.is_terminal() && !delayed_events.is_empty() {
        vec![format!(
            "Delayed credit ledger events are retained for audit but excluded because the trace is {}.",
            record.status.as_str()
        )]
    } else {
        delayed_events
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
            .collect::<Vec<_>>()
    };
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

fn delayed_credit_applies_to_record(record: &TraceCommonsSubmissionRecord) -> bool {
    !record.is_terminal()
}

#[derive(Debug, Clone)]
struct StoredTraceEnvelope {
    object_key: String,
    artifact_receipt: Option<EncryptedTraceArtifactReceipt>,
}

#[derive(Debug, Clone)]
struct TraceExportArtifactObjectRefMaterial {
    object_store: String,
    object_key: String,
    content_sha256: String,
    encryption_key_ref: String,
    size_bytes: i64,
}

fn store_envelope(
    state: &AppState,
    tenant_id: &str,
    status: TraceCorpusStatus,
    envelope: &TraceContributionEnvelope,
) -> anyhow::Result<StoredTraceEnvelope> {
    let object_key = trace_envelope_object_key(tenant_id, status, envelope.submission_id);
    if !state.object_primary_submit_review {
        let path = state.root.join(&object_key);
        write_json_file(&path, envelope, "trace contribution envelope")?;
    }
    let artifact_receipt = if let Some(store) = state.artifact_store.as_ref() {
        Some(store.put_json(
            &tenant_storage_ref(tenant_id),
            TraceArtifactKind::ContributionEnvelope,
            &envelope.submission_id.to_string(),
            envelope,
        )?)
    } else {
        anyhow::ensure!(
            !state.object_primary_submit_review,
            "{TRACE_COMMONS_OBJECT_PRIMARY_SUBMIT_REVIEW} requires a configured encrypted object store"
        );
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

fn trace_object_ref_write_from_record(
    state: &AppState,
    audit_label: &str,
    artifact_kind: StorageTraceObjectArtifactKind,
    record: &TraceCommonsSubmissionRecord,
    envelope: &TraceContributionEnvelope,
) -> anyhow::Result<(StorageTraceObjectRefWrite, String)> {
    let envelope_json = serde_json::to_string_pretty(envelope)
        .context("failed to serialize trace envelope for DB mirror hashing")?;
    let plaintext_sha256 = sha256_prefixed(&envelope_json);
    let (object_store, object_key, content_sha256) =
        if let Some(receipt) = record.artifact_receipt.as_ref() {
            (
                state
                    .artifact_store
                    .as_ref()
                    .map(|store| store.object_store_name().to_string())
                    .unwrap_or_else(|| TRACE_COMMONS_LEGACY_ENCRYPTED_OBJECT_STORE.to_string()),
                receipt.object_key.clone(),
                format!("sha256:{}", receipt.ciphertext_sha256),
            )
        } else {
            (
                TRACE_COMMONS_FILE_OBJECT_STORE.to_string(),
                record.object_key.clone(),
                plaintext_sha256,
            )
        };
    Ok((
        StorageTraceObjectRefWrite {
            object_ref_id: deterministic_trace_uuid(audit_label, record),
            tenant_id: record.tenant_id.clone(),
            submission_id: record.submission_id,
            artifact_kind,
            object_store,
            object_key,
            content_sha256: content_sha256.clone(),
            encryption_key_ref: format!("tenant:{}", tenant_storage_ref(&record.tenant_id)),
            size_bytes: i64::try_from(envelope_json.len()).unwrap_or(i64::MAX),
            compression: None,
            created_by_job_id: None,
        },
        content_sha256,
    ))
}

fn trace_export_artifact_object_ref_material<T: Serialize>(
    state: &AppState,
    tenant_id: &str,
    artifact_kind: TraceArtifactKind,
    object_id: Uuid,
    file_path: &Path,
    value: &T,
) -> anyhow::Result<TraceExportArtifactObjectRefMaterial> {
    let json = serde_json::to_string_pretty(value)
        .context("failed to serialize trace export artifact for object ref")?;
    let tenant_ref = tenant_storage_ref(tenant_id);
    let (object_store, object_key, content_sha256) = if let Some(store) =
        state.artifact_store.as_ref()
    {
        let receipt = store.put_json(&tenant_ref, artifact_kind, &object_id.to_string(), value)?;
        (
            store.object_store_name().to_string(),
            receipt.object_key,
            format!("sha256:{}", receipt.ciphertext_sha256),
        )
    } else {
        (
            TRACE_COMMONS_FILE_OBJECT_STORE.to_string(),
            trace_file_object_key(&state.root, file_path)?,
            sha256_prefixed(&json),
        )
    };

    Ok(TraceExportArtifactObjectRefMaterial {
        object_store,
        object_key,
        content_sha256,
        encryption_key_ref: format!("tenant:{tenant_ref}"),
        size_bytes: i64::try_from(json.len()).unwrap_or(i64::MAX),
    })
}

fn trace_file_object_key(root: &Path, path: &Path) -> anyhow::Result<String> {
    let relative = path.strip_prefix(root).with_context(|| {
        format!(
            "trace object file {} is outside root {}",
            path.display(),
            root.display()
        )
    })?;
    anyhow::ensure!(
        relative
            .components()
            .all(|component| matches!(component, std::path::Component::Normal(_))),
        "trace object file key contains unsafe path components"
    );
    let segments = relative
        .components()
        .map(|component| match component {
            std::path::Component::Normal(segment) => segment.to_string_lossy().into_owned(),
            _ => String::new(),
        })
        .collect::<Vec<_>>();
    anyhow::ensure!(!segments.is_empty(), "trace object file key is empty");
    Ok(segments.join("/"))
}

fn trace_export_artifact_object_ref_write(
    tenant_id: &str,
    submission_id: Uuid,
    artifact_kind: StorageTraceObjectArtifactKind,
    export_id: Uuid,
    material: &TraceExportArtifactObjectRefMaterial,
) -> StorageTraceObjectRefWrite {
    StorageTraceObjectRefWrite {
        object_ref_id: deterministic_trace_export_object_ref_uuid(
            tenant_id,
            export_id,
            submission_id,
            artifact_kind,
        ),
        tenant_id: tenant_id.to_string(),
        submission_id,
        artifact_kind,
        object_store: material.object_store.clone(),
        object_key: material.object_key.clone(),
        content_sha256: material.content_sha256.clone(),
        encryption_key_ref: material.encryption_key_ref.clone(),
        size_bytes: material.size_bytes,
        compression: None,
        created_by_job_id: Some(export_id),
    }
}

fn deterministic_trace_export_object_ref_uuid(
    tenant_id: &str,
    export_id: Uuid,
    submission_id: Uuid,
    artifact_kind: StorageTraceObjectArtifactKind,
) -> Uuid {
    let input = format!(
        "ironclaw.trace_commons.export_artifact_object_ref:{tenant_id}:{export_id}:{submission_id}:{artifact_kind:?}"
    );
    Uuid::new_v5(&Uuid::NAMESPACE_URL, input.as_bytes())
}

async fn mirror_submission_to_db(
    state: &AppState,
    tenant: &TenantAuth,
    record: &TraceCommonsSubmissionRecord,
    derived_record: &TraceCommonsDerivedRecord,
    envelope: &TraceContributionEnvelope,
) -> anyhow::Result<()> {
    mirror_submission_to_db_with_options(state, tenant, record, derived_record, envelope, true)
        .await
}

async fn mirror_submission_to_db_with_options(
    state: &AppState,
    tenant: &TenantAuth,
    record: &TraceCommonsSubmissionRecord,
    derived_record: &TraceCommonsDerivedRecord,
    envelope: &TraceContributionEnvelope,
    append_submit_audit: bool,
) -> anyhow::Result<()> {
    let Some(db) = state.db_mirror.as_ref() else {
        return Ok(());
    };
    let (object_ref, content_sha256) = trace_object_ref_write_from_record(
        state,
        "submitted-envelope",
        StorageTraceObjectArtifactKind::SubmittedEnvelope,
        record,
        envelope,
    )?;
    let object_ref_id = object_ref.object_ref_id;
    let derived_id = deterministic_trace_uuid("derived-precheck", record);
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

    db.append_trace_object_ref(object_ref)
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

    if append_submit_audit {
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
            previous_event_hash: None,
            event_hash: None,
            canonical_event_json: None,
            metadata: StorageTraceAuditSafeMetadata::Submission {
                status: storage_corpus_status(record.status),
                privacy_risk: privacy_risk.clone(),
            },
        })
        .await
        .context("failed to mirror trace audit event")?;
    }

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
    db_record: Option<&StorageTraceSubmissionRecord>,
) -> anyhow::Result<()> {
    let Some(db) = state.db_mirror.as_ref() else {
        return Ok(());
    };

    if let Some(record) = record {
        let file_tombstone = read_revocation(&state.root, &tenant.tenant_id, submission_id)?;
        let redaction_hash = file_tombstone
            .as_ref()
            .and_then(|tombstone| tombstone.redaction_hash.clone())
            .or_else(|| redaction_hash_for_record(state, record));
        let canonical_summary_hash = file_tombstone
            .as_ref()
            .and_then(|tombstone| tombstone.canonical_summary_hash.clone())
            .or_else(|| {
                read_derived_record(&state.root, &tenant.tenant_id, submission_id)
                    .ok()
                    .flatten()
                    .map(|derived| derived.canonical_summary_hash)
            });
        db.write_trace_tombstone(StorageTraceTombstoneWrite {
            tombstone_id: deterministic_trace_uuid("revocation-tombstone", record),
            tenant_id: record.tenant_id.clone(),
            submission_id: record.submission_id,
            trace_id: Some(record.trace_id),
            redaction_hash,
            canonical_summary_hash,
            reason: "contributor_revocation".to_string(),
            effective_at: Utc::now(),
            retain_until: None,
            created_by_principal_ref: tenant.principal_ref.clone(),
        })
        .await
        .context("failed to mirror trace revocation tombstone")?;
    } else if let Some(db_record) = db_record {
        db.write_trace_tombstone(StorageTraceTombstoneWrite {
            tombstone_id: deterministic_trace_uuid_for(
                "revocation-tombstone",
                &db_record.tenant_id,
                db_record.submission_id,
            ),
            tenant_id: db_record.tenant_id.clone(),
            submission_id: db_record.submission_id,
            trace_id: Some(db_record.trace_id),
            redaction_hash: Some(db_record.redaction_hash.clone()),
            canonical_summary_hash: db_record.canonical_summary_hash.clone(),
            reason: "contributor_revocation".to_string(),
            effective_at: Utc::now(),
            retain_until: None,
            created_by_principal_ref: tenant.principal_ref.clone(),
        })
        .await
        .context("failed to mirror DB-only trace revocation tombstone")?;
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

    let audit_source = record
        .map(|record| {
            (
                deterministic_trace_uuid("revocation-artifact-invalidation", record),
                record.tenant_id.clone(),
                record.submission_id,
            )
        })
        .or_else(|| {
            db_record.map(|record| {
                (
                    deterministic_trace_uuid_for(
                        "revocation-artifact-invalidation",
                        &record.tenant_id,
                        record.submission_id,
                    ),
                    record.tenant_id.clone(),
                    record.submission_id,
                )
            })
        });
    if let Some((audit_event_id, audit_tenant_id, audit_submission_id)) = audit_source
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
            audit_event_id,
            tenant_id: audit_tenant_id,
            actor_principal_ref: tenant.principal_ref.clone(),
            actor_role: format!("{:?}", tenant.role).to_ascii_lowercase(),
            action: StorageTraceAuditAction::Revoke,
            reason: Some("contributor_revocation_artifact_invalidation".to_string()),
            request_id: None,
            submission_id: Some(audit_submission_id),
            object_ref_id: None,
            export_manifest_id: None,
            decision_inputs_hash: None,
            previous_event_hash: None,
            event_hash: None,
            canonical_event_json: None,
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

async fn db_submission_record_for_revocation(
    state: &AppState,
    tenant_id: &str,
    submission_id: Uuid,
) -> anyhow::Result<Option<StorageTraceSubmissionRecord>> {
    let Some(db) = state.db_mirror.as_ref() else {
        return Ok(None);
    };
    db.get_trace_submission(tenant_id, submission_id)
        .await
        .context("failed to read DB trace submission for revocation")
}

async fn mirror_expiration_to_db(
    state: &AppState,
    tenant: &TenantAuth,
    submission_id: Uuid,
) -> anyhow::Result<()> {
    let Some(db) = state.db_mirror.as_ref() else {
        return Ok(());
    };
    let Some(record) = db
        .get_trace_submission(&tenant.tenant_id, submission_id)
        .await
        .context("failed to check trace submission before expiration mirror")?
    else {
        return Ok(());
    };
    db.update_trace_submission_status(
        &tenant.tenant_id,
        submission_id,
        StorageTraceCorpusStatus::Expired,
        &tenant.principal_ref,
        Some("retention_expired"),
    )
    .await
    .context("failed to mirror trace expiration status")?;
    let invalidation_counts = db
        .invalidate_trace_submission_artifacts(
            &tenant.tenant_id,
            submission_id,
            StorageTraceDerivedStatus::Expired,
        )
        .await
        .context("failed to mirror trace expiration artifact invalidation")?;
    let vector_entries_invalidated = db
        .invalidate_trace_vector_entries_for_submission(&tenant.tenant_id, submission_id)
        .await
        .context("failed to mirror trace expiration vector invalidation")?;
    let export_manifests_invalidated = db
        .invalidate_trace_export_manifests_for_submission(&tenant.tenant_id, submission_id)
        .await
        .context("failed to mirror trace expiration export manifest invalidation")?;
    let export_manifest_items_invalidated = db
        .invalidate_trace_export_manifest_items_for_submission(
            &tenant.tenant_id,
            submission_id,
            StorageTraceExportManifestItemInvalidationReason::Expired,
        )
        .await
        .context("failed to mirror trace expiration export manifest item invalidation")?;
    append_lifecycle_invalidation_audit_to_db(
        db.as_ref(),
        tenant,
        &record,
        TraceLifecycleInvalidationAuditInput {
            action: StorageTraceAuditAction::Retain,
            audit_id_label: "retention-expiration-artifact-invalidation",
            reason: "retention_expired_artifact_invalidation",
            status_count_label: "records_marked_expired",
            invalidation_counts,
            vector_entries_invalidated,
            export_manifests_invalidated,
            export_manifest_items_invalidated,
        },
    )
    .await?;
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
    let Some(record) = db
        .get_trace_submission(&tenant.tenant_id, submission_id)
        .await
        .context("failed to check trace submission before purge mirror")?
    else {
        return Ok(());
    };
    db.update_trace_submission_status(
        &tenant.tenant_id,
        submission_id,
        StorageTraceCorpusStatus::Purged,
        &tenant.principal_ref,
        Some("retention_purged"),
    )
    .await
    .context("failed to mirror trace purge status")?;
    let invalidation_counts = db
        .invalidate_trace_submission_artifacts(
            &tenant.tenant_id,
            submission_id,
            StorageTraceDerivedStatus::Expired,
        )
        .await
        .context("failed to mirror trace purge artifact invalidation")?;
    let vector_entries_invalidated = db
        .invalidate_trace_vector_entries_for_submission(&tenant.tenant_id, submission_id)
        .await
        .context("failed to mirror trace purge vector invalidation")?;
    let export_manifests_invalidated = db
        .invalidate_trace_export_manifests_for_submission(&tenant.tenant_id, submission_id)
        .await
        .context("failed to mirror trace purge export manifest invalidation")?;
    let export_manifest_items_invalidated = db
        .invalidate_trace_export_manifest_items_for_submission(
            &tenant.tenant_id,
            submission_id,
            StorageTraceExportManifestItemInvalidationReason::Purged,
        )
        .await
        .context("failed to mirror trace purge export manifest item invalidation")?;
    append_lifecycle_invalidation_audit_to_db(
        db.as_ref(),
        tenant,
        &record,
        TraceLifecycleInvalidationAuditInput {
            action: StorageTraceAuditAction::Purge,
            audit_id_label: "retention-purge-artifact-invalidation",
            reason: "retention_purged_artifact_invalidation",
            status_count_label: "records_marked_purged",
            invalidation_counts,
            vector_entries_invalidated,
            export_manifests_invalidated,
            export_manifest_items_invalidated,
        },
    )
    .await?;
    Ok(())
}

struct TraceLifecycleInvalidationAuditInput {
    action: StorageTraceAuditAction,
    audit_id_label: &'static str,
    reason: &'static str,
    status_count_label: &'static str,
    invalidation_counts: StorageTraceArtifactInvalidationCounts,
    vector_entries_invalidated: u64,
    export_manifests_invalidated: u64,
    export_manifest_items_invalidated: u64,
}

async fn append_lifecycle_invalidation_audit_to_db(
    db: &dyn Database,
    tenant: &TenantAuth,
    record: &StorageTraceSubmissionRecord,
    input: TraceLifecycleInvalidationAuditInput,
) -> anyhow::Result<()> {
    let mut action_counts = lifecycle_invalidation_action_counts(
        input.invalidation_counts,
        input.vector_entries_invalidated,
        input.export_manifests_invalidated,
        input.export_manifest_items_invalidated,
    );
    action_counts.insert(input.status_count_label.to_string(), 1);

    db.append_trace_audit_event(StorageTraceAuditEventWrite {
        audit_event_id: deterministic_trace_uuid_for(
            input.audit_id_label,
            &record.tenant_id,
            record.submission_id,
        ),
        tenant_id: record.tenant_id.clone(),
        actor_principal_ref: tenant.principal_ref.clone(),
        actor_role: format!("{:?}", tenant.role).to_ascii_lowercase(),
        action: input.action,
        reason: Some(input.reason.to_string()),
        request_id: None,
        submission_id: Some(record.submission_id),
        object_ref_id: None,
        export_manifest_id: None,
        decision_inputs_hash: None,
        previous_event_hash: None,
        event_hash: None,
        canonical_event_json: None,
        metadata: StorageTraceAuditSafeMetadata::Maintenance {
            dry_run: false,
            action_counts,
        },
    })
    .await
    .context("failed to mirror trace lifecycle artifact invalidation audit")?;
    Ok(())
}

fn lifecycle_invalidation_action_counts(
    invalidation_counts: StorageTraceArtifactInvalidationCounts,
    vector_entries_invalidated: u64,
    export_manifests_invalidated: u64,
    export_manifest_items_invalidated: u64,
) -> BTreeMap<String, u32> {
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
    action_counts
}

async fn mirror_review_decision_to_db(
    state: &AppState,
    tenant: &TenantAuth,
    record: &TraceCommonsSubmissionRecord,
    envelope: &TraceContributionEnvelope,
    canonical_summary_hash: Option<String>,
) -> anyhow::Result<()> {
    let Some(db) = state.db_mirror.as_ref() else {
        return Ok(());
    };
    db.upsert_trace_submission(storage_submission_write_from_record(
        record,
        envelope,
        canonical_summary_hash,
    )?)
    .await
    .context("failed to mirror reviewed trace submission metadata")?;
    let (object_ref, _) = trace_object_ref_write_from_record(
        state,
        "reviewed-envelope",
        StorageTraceObjectArtifactKind::ReviewSnapshot,
        record,
        envelope,
    )?;
    db.append_trace_object_ref(object_ref)
        .await
        .context("failed to mirror reviewed trace object ref")?;
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
        actor_role: event.actor_role.storage_name().to_string(),
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
    artifact_object_ref_material: Option<&TraceExportArtifactObjectRefMaterial>,
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
    let vector_entry_ids = active_vector_entry_lookup_for_export(db.as_ref(), &artifact.tenant_id)
        .await
        .context("failed to read active vector entries for benchmark provenance")?;
    for candidate in &artifact.candidates {
        db.upsert_trace_export_manifest_item(StorageTraceExportManifestItemWrite {
            tenant_id: artifact.tenant_id.clone(),
            export_manifest_id: artifact.conversion_id,
            submission_id: candidate.submission_id,
            trace_id: candidate.trace_id,
            derived_id: Some(candidate.derived_id),
            object_ref_id: append_export_artifact_object_ref_to_db(
                db.as_ref(),
                &artifact.tenant_id,
                candidate.submission_id,
                StorageTraceObjectArtifactKind::BenchmarkArtifact,
                artifact.conversion_id,
                artifact_object_ref_material,
            )
            .await?,
            vector_entry_id: vector_entry_ids
                .get(&(
                    candidate.submission_id,
                    candidate.derived_id,
                    candidate.canonical_summary_hash.clone(),
                ))
                .copied(),
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
    artifact_object_ref_material: Option<&TraceExportArtifactObjectRefMaterial>,
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
    let vector_entry_ids =
        active_vector_entry_lookup_for_export(db.as_ref(), &provenance.tenant_id)
            .await
            .context("failed to read active vector entries for ranker candidate provenance")?;
    for candidate in candidates {
        db.upsert_trace_export_manifest_item(StorageTraceExportManifestItemWrite {
            tenant_id: provenance.tenant_id.clone(),
            export_manifest_id: provenance.export_id,
            submission_id: candidate.submission_id,
            trace_id: candidate.trace_id,
            derived_id: Some(candidate.derived_id),
            object_ref_id: append_export_artifact_object_ref_to_db(
                db.as_ref(),
                &provenance.tenant_id,
                candidate.submission_id,
                StorageTraceObjectArtifactKind::ExportArtifact,
                provenance.export_id,
                artifact_object_ref_material,
            )
            .await?,
            vector_entry_id: vector_entry_ids
                .get(&(
                    candidate.submission_id,
                    candidate.derived_id,
                    candidate.canonical_summary_hash.clone(),
                ))
                .copied(),
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
    artifact_object_ref_material: Option<&TraceExportArtifactObjectRefMaterial>,
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
    let vector_entry_ids =
        active_vector_entry_lookup_for_export(db.as_ref(), &provenance.tenant_id)
            .await
            .context("failed to read active vector entries for ranker pair provenance")?;
    for pair in pairs {
        for candidate in [&pair.preferred, &pair.rejected] {
            db.upsert_trace_export_manifest_item(StorageTraceExportManifestItemWrite {
                tenant_id: provenance.tenant_id.clone(),
                export_manifest_id: provenance.export_id,
                submission_id: candidate.submission_id,
                trace_id: candidate.trace_id,
                derived_id: Some(candidate.derived_id),
                object_ref_id: append_export_artifact_object_ref_to_db(
                    db.as_ref(),
                    &provenance.tenant_id,
                    candidate.submission_id,
                    StorageTraceObjectArtifactKind::ExportArtifact,
                    provenance.export_id,
                    artifact_object_ref_material,
                )
                .await?,
                vector_entry_id: vector_entry_ids
                    .get(&(
                        candidate.submission_id,
                        candidate.derived_id,
                        candidate.canonical_summary_hash.clone(),
                    ))
                    .copied(),
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

async fn append_export_artifact_object_ref_to_db(
    db: &dyn Database,
    tenant_id: &str,
    submission_id: Uuid,
    artifact_kind: StorageTraceObjectArtifactKind,
    export_id: Uuid,
    material: Option<&TraceExportArtifactObjectRefMaterial>,
) -> anyhow::Result<Option<Uuid>> {
    let Some(material) = material else {
        return Ok(None);
    };
    let object_ref = trace_export_artifact_object_ref_write(
        tenant_id,
        submission_id,
        artifact_kind,
        export_id,
        material,
    );
    let object_ref_id = object_ref.object_ref_id;
    db.append_trace_object_ref(object_ref)
        .await
        .with_context(|| {
            format!(
                "failed to mirror trace export artifact object ref for submission {submission_id}"
            )
        })?;
    Ok(Some(object_ref_id))
}

async fn active_vector_entry_lookup_for_export(
    db: &dyn Database,
    tenant_id: &str,
) -> anyhow::Result<BTreeMap<(Uuid, Uuid, String), Uuid>> {
    let entries = db
        .list_trace_vector_entries(tenant_id)
        .await
        .context("failed to list trace vector entries")?;
    Ok(entries
        .into_iter()
        .filter(|entry| entry.status == StorageTraceVectorEntryStatus::Active)
        .filter(|entry| {
            entry.source_projection == StorageTraceVectorEntrySourceProjection::CanonicalSummary
        })
        .map(|entry| {
            (
                (entry.submission_id, entry.derived_id, entry.source_hash),
                entry.vector_entry_id,
            )
        })
        .collect())
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
        purpose_code: Some(provenance_storage_purpose_code(provenance)),
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

fn provenance_storage_purpose_code(provenance: &TraceExportProvenanceManifest) -> String {
    match provenance.export_kind {
        TraceExportProvenanceKind::BenchmarkConversion => provenance.purpose.clone(),
        TraceExportProvenanceKind::RankerTrainingCandidates => ranker_storage_purpose_code(
            RANKER_TRAINING_CANDIDATES_EXPORT_PURPOSE_CODE,
            &provenance.purpose,
        ),
        TraceExportProvenanceKind::RankerTrainingPairs => ranker_storage_purpose_code(
            RANKER_TRAINING_PAIRS_EXPORT_PURPOSE_CODE,
            &provenance.purpose,
        ),
    }
}

fn ranker_storage_purpose_code(export_kind_code: &str, purpose: &str) -> String {
    let purpose = purpose.trim();
    if purpose == export_kind_code {
        export_kind_code.to_string()
    } else {
        format!("{export_kind_code}:{purpose}")
    }
}

fn is_ranker_training_purpose_code(purpose_code: &str) -> bool {
    purpose_code == RANKER_TRAINING_CANDIDATES_EXPORT_PURPOSE_CODE
        || purpose_code == RANKER_TRAINING_PAIRS_EXPORT_PURPOSE_CODE
        || purpose_code
            .strip_prefix(RANKER_TRAINING_CANDIDATES_EXPORT_PURPOSE_CODE)
            .is_some_and(|suffix| suffix.starts_with(':'))
        || purpose_code
            .strip_prefix(RANKER_TRAINING_PAIRS_EXPORT_PURPOSE_CODE)
            .is_some_and(|suffix| suffix.starts_with(':'))
}

fn storage_manifest_purpose_matches(purpose_code: Option<&str>, requested_purpose: &str) -> bool {
    let Some(purpose_code) = purpose_code else {
        return false;
    };
    purpose_code == requested_purpose
        || purpose_code
            .strip_prefix(RANKER_TRAINING_CANDIDATES_EXPORT_PURPOSE_CODE)
            .and_then(|suffix| suffix.strip_prefix(':'))
            .is_some_and(|purpose| purpose == requested_purpose)
        || purpose_code
            .strip_prefix(RANKER_TRAINING_PAIRS_EXPORT_PURPOSE_CODE)
            .and_then(|suffix| suffix.strip_prefix(':'))
            .is_some_and(|purpose| purpose == requested_purpose)
}

fn storage_submission_write_from_record(
    record: &TraceCommonsSubmissionRecord,
    envelope: &TraceContributionEnvelope,
    canonical_summary_hash: Option<String>,
) -> anyhow::Result<StorageTraceSubmissionWrite> {
    let consent_scopes = consent_scope_storage_strings(&record.consent_scopes)?;
    let allowed_uses = trace_allowed_use_storage_strings(&envelope.trace_card.allowed_uses)?;
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
        allowed_uses,
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

fn deterministic_trace_uuid_for_external_ref(
    label: &str,
    tenant_id: &str,
    submission_id: Uuid,
    external_ref: &str,
) -> Uuid {
    let input = format!(
        "ironclaw.trace_commons.{label}:{}:{}:{}",
        tenant_id, submission_id, external_ref
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
        TraceCreditLedgerEventType::TrainingUtility => StorageTraceCreditEventType::TrainingUtility,
        TraceCreditLedgerEventType::RankingUtility => StorageTraceCreditEventType::RankingUtility,
        TraceCreditLedgerEventType::ReviewerBonus => StorageTraceCreditEventType::ReviewerBonus,
        TraceCreditLedgerEventType::AbusePenalty => StorageTraceCreditEventType::AbusePenalty,
    }
}

fn credit_delta_micros(delta: f32) -> i64 {
    (delta * 1_000_000.0).round() as i64
}

fn consent_scope_storage_strings(scopes: &[ConsentScope]) -> anyhow::Result<Vec<String>> {
    scopes.iter().map(serde_storage_string).collect()
}

fn trace_allowed_use_storage_strings(
    allowed_uses: &[TraceAllowedUse],
) -> anyhow::Result<Vec<String>> {
    allowed_uses.iter().map(serde_storage_string).collect()
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
    let path = submission_metadata_path(root, &record.tenant_id, record.submission_id);
    write_json_file(&path, record, "trace contribution metadata")
}

fn submission_metadata_path(root: &Path, tenant_id: &str, submission_id: Uuid) -> PathBuf {
    let tenant_key = tenant_storage_key(tenant_id);
    root.join("tenants")
        .join(tenant_key)
        .join("metadata")
        .join(format!("{submission_id}.json"))
}

fn read_submission_record(
    root: &Path,
    tenant_id: &str,
    submission_id: Uuid,
) -> anyhow::Result<Option<TraceCommonsSubmissionRecord>> {
    let path = submission_metadata_path(root, tenant_id, submission_id);
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
        tenant.role.can_export(),
        "trace body read requires reviewer, admin, or export worker role"
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

async fn read_envelope_for_review_decision(
    state: &AppState,
    tenant: &TenantAuth,
    record: &TraceCommonsSubmissionRecord,
    allow_file_body_fallback: bool,
) -> anyhow::Result<TraceEnvelopeBodyRead> {
    anyhow::ensure!(
        tenant.role.can_review(),
        "trace review body read requires reviewer or admin role"
    );
    anyhow::ensure!(
        record.tenant_id == tenant.tenant_id,
        "trace review body read tenant mismatch"
    );
    let body_read =
        read_envelope_body_for_review_decision(state, tenant, record, allow_file_body_fallback)
            .await?;
    append_trace_content_read_audit(
        state,
        tenant,
        record.submission_id,
        body_read.object_ref_id,
        "review_decision",
        None,
    )
    .await?;
    Ok(body_read)
}

async fn read_envelope_body_for_review_decision(
    state: &AppState,
    tenant: &TenantAuth,
    record: &TraceCommonsSubmissionRecord,
    allow_file_body_fallback: bool,
) -> anyhow::Result<TraceEnvelopeBodyRead> {
    if state.db_reviewer_reads
        && let Some(envelope) =
            read_envelope_from_active_db_object_ref(state, &tenant.tenant_id, record.submission_id)
                .await?
    {
        return Ok(envelope);
    }
    anyhow::ensure!(
        !state.db_reviewer_require_object_refs && allow_file_body_fallback,
        "missing active submitted envelope object ref for review decision"
    );
    Ok(TraceEnvelopeBodyRead {
        envelope: read_envelope_by_record(state, record)?,
        object_ref_id: None,
    })
}

async fn read_envelope_body_for_replay_export(
    state: &AppState,
    tenant: &TenantAuth,
    record: &TraceCommonsSubmissionRecord,
) -> anyhow::Result<TraceEnvelopeBodyRead> {
    if state.db_replay_export_reads {
        if let Some(envelope) =
            read_envelope_from_active_db_object_ref(state, &tenant.tenant_id, record.submission_id)
                .await?
        {
            return Ok(envelope);
        }
        anyhow::ensure!(
            !state.db_replay_export_require_object_refs,
            "missing active submitted envelope object ref for replay export"
        );
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
        matches!(
            object_ref.artifact_kind,
            StorageTraceObjectArtifactKind::SubmittedEnvelope
                | StorageTraceObjectArtifactKind::ReviewSnapshot
        ),
        "trace object ref artifact kind mismatch"
    );
    anyhow::ensure!(
        object_ref.compression.is_none(),
        "compressed trace object refs are not supported"
    );

    match object_ref.object_store.as_str() {
        object_store if is_encrypted_trace_object_store(object_store) => {
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
        TRACE_COMMONS_FILE_OBJECT_STORE => {
            read_file_store_envelope_from_object_ref(state, object_ref)
        }
        other => anyhow::bail!("unsupported trace object store: {other}"),
    }
}

fn is_encrypted_trace_object_store(object_store: &str) -> bool {
    matches!(
        object_store,
        TRACE_COMMONS_LEGACY_ENCRYPTED_OBJECT_STORE
            | TRACE_COMMONS_SERVICE_LOCAL_ENCRYPTED_OBJECT_STORE
    )
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
        TRACE_OBJECT_REF_CONTENT_HASH_MISMATCH
    );
    serde_json::from_str(&body)
        .with_context(|| format!("failed to parse trace object {}", path.display()))
}

const TRACE_OBJECT_REF_CONTENT_HASH_MISMATCH: &str = "trace object ref content hash mismatch";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TraceObjectRefReadFailureKind {
    HashMismatch,
    Unreadable,
}

fn classify_trace_object_ref_read_failure(error: &anyhow::Error) -> TraceObjectRefReadFailureKind {
    if error.chain().any(|cause| {
        cause
            .to_string()
            .contains(TRACE_OBJECT_REF_CONTENT_HASH_MISMATCH)
    }) {
        TraceObjectRefReadFailureKind::HashMismatch
    } else {
        TraceObjectRefReadFailureKind::Unreadable
    }
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
    if path.exists() {
        return Ok(());
    }
    write_json_file(&path, tombstone, "trace revocation tombstone")
}

fn read_revocation(
    root: &Path,
    tenant_id: &str,
    submission_id: Uuid,
) -> anyhow::Result<Option<TraceCommonsRevocation>> {
    let tenant_key = tenant_storage_key(tenant_id);
    let path = root
        .join("tenants")
        .join(tenant_key)
        .join("revocations")
        .join(format!("{submission_id}.json"));
    if !path.exists() {
        return Ok(None);
    }
    let body = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read revocation {}", path.display()))?;
    let revocation = serde_json::from_str(&body)
        .with_context(|| format!("failed to parse revocation {}", path.display()))?;
    Ok(Some(revocation))
}

fn append_audit_event(
    root: &Path,
    tenant_id: &str,
    mut event: TraceCommonsAuditEvent,
) -> anyhow::Result<TraceCommonsAuditEvent> {
    let tenant_key = tenant_storage_key(tenant_id);
    let path = root
        .join("tenants")
        .join(tenant_key)
        .join("audit")
        .join("events.jsonl");
    let previous_event_hash = latest_audit_event_hash(&path)?
        .unwrap_or_else(|| TRACE_AUDIT_EVENT_GENESIS_HASH.to_string());
    event.previous_event_hash = Some(previous_event_hash.clone());
    event.event_hash = None;
    event.event_hash = Some(compute_audit_event_hash(&previous_event_hash, &event)?);
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
        .with_context(|| format!("failed to append audit log {}", path.display()))?;
    Ok(event)
}

const TRACE_AUDIT_EVENT_GENESIS_HASH: &str = "sha256:genesis";
const TRACE_AUDIT_EVENT_HASH_DOMAIN: &str = "trace_commons_audit_event:v1";

fn latest_audit_event_hash(path: &Path) -> anyhow::Result<Option<String>> {
    if !path.exists() {
        return Ok(None);
    }
    let body = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read audit log {}", path.display()))?;
    let Some(line) = body
        .lines()
        .rev()
        .map(str::trim)
        .find(|line| !line.is_empty())
    else {
        return Ok(None);
    };
    let event: TraceCommonsAuditEvent = serde_json::from_str(line).with_context(|| {
        format!(
            "failed to parse latest audit event for hash chain {}",
            path.display()
        )
    })?;
    Ok(event.event_hash)
}

fn compute_audit_event_hash(
    previous_event_hash: &str,
    event: &TraceCommonsAuditEvent,
) -> anyhow::Result<String> {
    let canonical_event = canonical_audit_event_json(previous_event_hash, event)?;
    Ok(compute_audit_event_hash_from_canonical(
        previous_event_hash,
        &canonical_event,
    ))
}

fn canonical_audit_event_json(
    previous_event_hash: &str,
    event: &TraceCommonsAuditEvent,
) -> anyhow::Result<String> {
    let mut event_for_hash = event.clone();
    event_for_hash.previous_event_hash = Some(previous_event_hash.to_string());
    event_for_hash.event_hash = None;
    serde_json::to_string(&event_for_hash).context("failed to serialize audit event hash")
}

fn compute_audit_event_hash_from_canonical(
    previous_event_hash: &str,
    canonical_event: &str,
) -> String {
    sha256_prefixed(&format!(
        "{TRACE_AUDIT_EVENT_HASH_DOMAIN}\n{previous_event_hash}\n{canonical_event}"
    ))
}

async fn append_audit_event_with_db_mirror(
    state: &AppState,
    tenant: &TenantAuth,
    event: TraceCommonsAuditEvent,
    action: StorageTraceAuditAction,
    metadata: StorageTraceAuditSafeMetadata,
) -> anyhow::Result<()> {
    let event = append_audit_event(&state.root, &tenant.tenant_id, event)?;
    let mirror_result = mirror_audit_event_to_db(state, tenant, &event, action, metadata).await;
    if let Err(error) = &mirror_result {
        tracing::warn!(
            %error,
            event_id = %event.event_id,
            "Trace Commons DB dual-write audit mirror failed"
        );
    }
    enforce_db_mirror_write_result(state, "audit event", mirror_result)?;
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
    let event = append_audit_event(&state.root, &tenant.tenant_id, event)?;
    let mirror_result = mirror_audit_event_to_db_with_object_ref(
        state,
        tenant,
        &event,
        StorageTraceAuditAction::Read,
        StorageTraceAuditSafeMetadata::Empty,
        object_ref_id,
    )
    .await;
    if let Err(error) = &mirror_result {
        tracing::warn!(
            %error,
            event_id = %event.event_id,
            "Trace Commons DB dual-write audit mirror failed"
        );
    }
    enforce_db_mirror_write_result(state, "trace content read audit event", mirror_result)?;
    Ok(())
}

async fn append_derived_source_read_audits(
    state: &AppState,
    tenant: &TenantAuth,
    submission_ids: &[Uuid],
    object_ref_ids: &BTreeMap<Uuid, Uuid>,
    surface: &str,
    purpose: Option<&str>,
) -> anyhow::Result<usize> {
    let mut seen = BTreeSet::new();
    let mut appended = 0usize;
    for submission_id in submission_ids.iter().copied() {
        if !seen.insert(submission_id) {
            continue;
        }
        append_trace_content_read_audit(
            state,
            tenant,
            submission_id,
            object_ref_ids.get(&submission_id).copied(),
            surface,
            purpose,
        )
        .await?;
        appended += 1;
    }
    Ok(appended)
}

async fn revalidate_db_export_sources(
    state: &AppState,
    tenant: &TenantAuth,
    submission_ids: &[Uuid],
    require_object_refs: bool,
) -> anyhow::Result<BTreeMap<Uuid, Uuid>> {
    let Some(db) = state.db_mirror.as_ref() else {
        return Ok(BTreeMap::new());
    };

    let mut seen = BTreeSet::new();
    let mut object_ref_ids = BTreeMap::new();
    for submission_id in submission_ids.iter().copied() {
        if !seen.insert(submission_id) {
            continue;
        }
        let record = db
            .get_trace_submission(&tenant.tenant_id, submission_id)
            .await
            .with_context(|| format!("failed to revalidate export source {submission_id}"))?
            .with_context(|| format!("missing DB submission for export source {submission_id}"))?;
        anyhow::ensure!(
            record.status == StorageTraceCorpusStatus::Accepted
                && record.revoked_at.is_none()
                && record.purged_at.is_none(),
            "trace export source {submission_id} is no longer accepted"
        );
        let object_ref = db
            .get_latest_active_trace_object_ref(
                &tenant.tenant_id,
                submission_id,
                StorageTraceObjectArtifactKind::SubmittedEnvelope,
            )
            .await
            .with_context(|| {
                format!("failed to read active submitted-envelope object ref for {submission_id}")
            })?;
        if let Some(object_ref) = object_ref {
            read_envelope_from_object_ref(state, &tenant.tenant_id, &object_ref).with_context(
                || format!("failed to verify export source object ref for {submission_id}"),
            )?;
            object_ref_ids.insert(submission_id, object_ref.object_ref_id);
        } else if require_object_refs {
            anyhow::bail!("missing active submitted-envelope object ref for {submission_id}");
        }
    }
    Ok(object_ref_ids)
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
    let canonical_event_json = event
        .previous_event_hash
        .as_deref()
        .map(|previous_event_hash| canonical_audit_event_json(previous_event_hash, event))
        .transpose()?;
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
        previous_event_hash: event.previous_event_hash.clone(),
        event_hash: event.event_hash.clone(),
        canonical_event_json,
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

async fn read_revocations_for_submit(
    state: &AppState,
    tenant_id: &str,
) -> anyhow::Result<Vec<TraceCommonsRevocation>> {
    let mut revocations = read_all_revocations(&state.root, tenant_id)?;
    if let Some(db) = state.db_mirror.as_ref() {
        let db_revocations = db
            .list_trace_tombstones(tenant_id)
            .await
            .context("failed to list trace tombstones for submit preflight")?;
        revocations.extend(
            db_revocations
                .into_iter()
                .map(trace_revocation_from_storage_tombstone),
        );
        revocations.sort_by_key(|revocation| revocation.revoked_at);
    }
    Ok(revocations)
}

fn trace_revocation_from_storage_tombstone(
    tombstone: StorageTraceTombstoneRecord,
) -> TraceCommonsRevocation {
    TraceCommonsRevocation {
        tenant_storage_ref: tenant_storage_ref(&tombstone.tenant_id),
        tenant_id: tombstone.tenant_id,
        submission_id: tombstone.submission_id,
        revoked_at: tombstone.effective_at,
        reason: tombstone.reason,
        redaction_hash: tombstone.redaction_hash,
        canonical_summary_hash: tombstone.canonical_summary_hash,
    }
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

async fn read_benchmark_conversion_artifact(
    state: &AppState,
    tenant: &TenantAuth,
    conversion_id: Uuid,
) -> anyhow::Result<Option<TraceBenchmarkConversionArtifact>> {
    read_benchmark_conversion_artifact_with_ref_policy(state, tenant, conversion_id, false).await
}

async fn read_benchmark_conversion_artifact_for_invalidation(
    state: &AppState,
    tenant: &TenantAuth,
    conversion_id: Uuid,
) -> anyhow::Result<Option<TraceBenchmarkConversionArtifact>> {
    read_benchmark_conversion_artifact_with_ref_policy(state, tenant, conversion_id, true).await
}

async fn read_benchmark_conversion_artifact_with_ref_policy(
    state: &AppState,
    tenant: &TenantAuth,
    conversion_id: Uuid,
    allow_invalidated_object_refs: bool,
) -> anyhow::Result<Option<TraceBenchmarkConversionArtifact>> {
    let path = benchmark_artifact_path(&state.root, &tenant.tenant_id, conversion_id);
    if path.exists() {
        let body = std::fs::read_to_string(&path).with_context(|| {
            format!(
                "failed to read trace benchmark conversion artifact {}",
                path.display()
            )
        })?;
        let artifact = serde_json::from_str(&body).with_context(|| {
            format!(
                "failed to parse trace benchmark conversion artifact {}",
                path.display()
            )
        })?;
        return Ok(Some(artifact));
    }

    let Some(db) = state.db_mirror.as_ref() else {
        return Ok(None);
    };
    let Some(store) = state.artifact_store.as_ref() else {
        return Ok(None);
    };
    let items = db
        .list_trace_export_manifest_items(&tenant.tenant_id, conversion_id)
        .await
        .context("failed to list benchmark export manifest items for artifact read")?;
    for item in items {
        let Some(object_ref_id) = item.object_ref_id else {
            continue;
        };
        let object_refs = db
            .list_trace_object_refs(&tenant.tenant_id, item.submission_id)
            .await
            .with_context(|| {
                format!(
                    "failed to list trace object refs for benchmark artifact source {}",
                    item.submission_id
                )
            })?;
        let Some(object_ref) = object_refs.into_iter().find(|object_ref| {
            object_ref.object_ref_id == object_ref_id
                && object_ref.artifact_kind == StorageTraceObjectArtifactKind::BenchmarkArtifact
                && (allow_invalidated_object_refs || object_ref.invalidated_at.is_none())
                && object_ref.deleted_at.is_none()
        }) else {
            continue;
        };
        let artifact = store.get_json_by_object_key(
            &tenant_storage_ref(&tenant.tenant_id),
            TraceArtifactKind::BenchmarkConversion,
            &object_ref.object_key,
            &object_ref.content_sha256,
        )?;
        return Ok(Some(artifact));
    }
    Ok(None)
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
    require_db_reconciliation_clean_request(state, &request)?;
    let mut revocation_reasons = read_all_revocations(&state.root, &tenant.tenant_id)?
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
            revocation_reasons
                .entry(record.submission_id)
                .or_insert_with(|| "contributor_revocation".to_string());
            if !request.dry_run {
                mirror_revocation_to_db(state, tenant, record.submission_id, Some(record), None)
                    .await?;
            }
            continue;
        }
        if record.status == TraceCorpusStatus::Expired {
            expired_submission_ids.insert(record.submission_id);
            continue;
        }
        if revoked_submission_ids.contains(&record.submission_id) {
            records_marked_revoked += 1;
            revocation_reasons
                .entry(record.submission_id)
                .or_insert_with(|| "contributor_revocation".to_string());
            if !request.dry_run {
                record.status = TraceCorpusStatus::Revoked;
                record.credit_points_final = Some(0.0);
                write_submission_record(&state.root, record)?;
                mirror_revocation_to_db(state, tenant, record.submission_id, Some(record), None)
                    .await?;
            }
            continue;
        }
        if record.is_expired_at(now) && !retention_policy_is_on_legal_hold(state, record) {
            records_marked_expired += 1;
            expired_submission_ids.insert(record.submission_id);
            if !request.dry_run {
                record.status = TraceCorpusStatus::Expired;
                record.credit_points_final = Some(record.credit_points_final.unwrap_or(0.0));
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
            if retention_policy_is_on_legal_hold(state, record) {
                continue;
            }
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
    let benchmark_artifacts_invalidated =
        if !revoked_submission_ids.is_empty() || !expired_submission_ids.is_empty() {
            propagate_benchmark_artifact_source_invalidation(
                state,
                tenant,
                &revocation_reasons,
                &expired_submission_ids,
                request.dry_run,
            )
            .await?
        } else {
            0
        };
    let db_mirror_backfill = backfill_db_mirror_from_files(
        state,
        tenant,
        &records,
        &derived,
        request.backfill_db_mirror,
        request.dry_run,
    )
    .await?;
    let db_mirror_backfilled = db_mirror_backfill.backfilled;
    let db_mirror_backfill_failed = db_mirror_backfill.failed;
    let vector_entries_indexed = index_vector_metadata_from_db(
        state,
        tenant,
        request.index_vectors,
        request.dry_run,
        &purpose,
    )
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
        benchmark_artifacts_invalidated,
        trace_object_files_deleted,
        encrypted_artifacts_deleted,
        db_mirror_backfilled,
        db_mirror_backfill_failed,
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
    enforce_db_reconciliation_clean(state, db_reconciliation.as_ref())?;
    let audit_chain = if request.verify_audit_chain {
        Some(verify_audit_chain(state, &tenant.tenant_id).await?)
    } else {
        None
    };

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
        benchmark_artifacts_invalidated,
        trace_object_files_deleted,
        encrypted_artifacts_deleted,
        db_mirror_backfilled,
        db_mirror_backfill_failed,
        db_mirror_backfill_failures: db_mirror_backfill.failures,
        vector_entries_indexed,
        audit_chain,
        db_reconciliation,
    })
}

fn retention_policy_is_on_legal_hold(
    state: &AppState,
    record: &TraceCommonsSubmissionRecord,
) -> bool {
    state
        .legal_hold_retention_policy_ids
        .contains(&record.retention_policy_id)
}

async fn verify_audit_chain(
    state: &AppState,
    tenant_id: &str,
) -> anyhow::Result<TraceAuditChainReport> {
    let mut report = verify_file_audit_chain(&state.root, tenant_id)?;
    if let Some(db) = state.db_mirror.as_ref() {
        report.db_mirror = Some(verify_db_audit_chain(db.as_ref(), tenant_id).await?);
    }
    Ok(report)
}

fn verify_file_audit_chain(root: &Path, tenant_id: &str) -> anyhow::Result<TraceAuditChainReport> {
    let path = root
        .join("tenants")
        .join(tenant_storage_key(tenant_id))
        .join("audit")
        .join("events.jsonl");
    if !path.exists() {
        return Ok(TraceAuditChainReport::default());
    }

    let body = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read audit log {}", path.display()))?;
    let mut report = TraceAuditChainReport::default();
    let mut expected_previous_hash = TRACE_AUDIT_EVENT_GENESIS_HASH.to_string();
    for (index, line) in body.lines().enumerate() {
        let line_number = index + 1;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        report.event_count += 1;
        let event: TraceCommonsAuditEvent = serde_json::from_str(line).with_context(|| {
            format!(
                "failed to parse audit event {} line {}",
                path.display(),
                line_number
            )
        })?;
        let Some(event_hash) = event.event_hash.as_deref() else {
            report.legacy_event_count += 1;
            expected_previous_hash = TRACE_AUDIT_EVENT_GENESIS_HASH.to_string();
            continue;
        };
        let previous_event_hash = event
            .previous_event_hash
            .as_deref()
            .unwrap_or(TRACE_AUDIT_EVENT_GENESIS_HASH);
        if previous_event_hash != expected_previous_hash {
            report
                .failures
                .push(format!("line {line_number}: previous_event_hash mismatch"));
        }
        let recomputed = compute_audit_event_hash(previous_event_hash, &event)?;
        if recomputed != event_hash {
            report
                .failures
                .push(format!("line {line_number}: event_hash mismatch"));
        }
        expected_previous_hash = event_hash.to_string();
        report.last_event_hash = Some(event_hash.to_string());
    }
    report.mismatch_count = report.failures.len();
    report.verified = report.mismatch_count == 0;
    Ok(report)
}

async fn verify_db_audit_chain(
    db: &dyn Database,
    tenant_id: &str,
) -> anyhow::Result<TraceDbAuditChainReport> {
    let events = db
        .list_trace_audit_events(tenant_id)
        .await
        .context("failed to list DB audit events for hash-chain verification")?;
    let mut report = TraceDbAuditChainReport::default();
    let mut expected_previous_hash = TRACE_AUDIT_EVENT_GENESIS_HASH.to_string();
    for (index, event) in events.into_iter().enumerate() {
        let row_number = index + 1;
        report.event_count += 1;
        let Some(event_hash) = event.event_hash.as_deref() else {
            report.legacy_event_count += 1;
            expected_previous_hash = TRACE_AUDIT_EVENT_GENESIS_HASH.to_string();
            continue;
        };
        if !event_hash.starts_with("sha256:") {
            report.failures.push(format!(
                "db row {row_number} event {}: event_hash has invalid format",
                event.audit_event_id
            ));
        }
        let previous_event_hash = event
            .previous_event_hash
            .as_deref()
            .unwrap_or(TRACE_AUDIT_EVENT_GENESIS_HASH);
        if previous_event_hash != expected_previous_hash {
            report.failures.push(format!(
                "db row {row_number} event {}: previous_event_hash mismatch",
                event.audit_event_id
            ));
        }
        if let Some(canonical_event_json) = event.canonical_event_json.as_deref() {
            report.payload_verified_event_count += 1;
            let recomputed =
                compute_audit_event_hash_from_canonical(previous_event_hash, canonical_event_json);
            if recomputed != event_hash {
                report.failures.push(format!(
                    "db row {row_number} event {}: canonical payload hash mismatch",
                    event.audit_event_id
                ));
            }
            let canonical_event: TraceCommonsAuditEvent =
                serde_json::from_str(canonical_event_json).with_context(|| {
                    format!(
                        "failed to parse canonical audit payload for DB audit event {}",
                        event.audit_event_id
                    )
                })?;
            verify_db_audit_projection(row_number, &event, &canonical_event, &mut report);
        } else {
            report.payload_unverified_event_count += 1;
        }
        expected_previous_hash = event_hash.to_string();
        report.last_event_hash = Some(event_hash.to_string());
    }
    report.mismatch_count = report.failures.len();
    report.verified = report.mismatch_count == 0;
    Ok(report)
}

fn verify_db_audit_projection(
    row_number: usize,
    event: &StorageTraceAuditEventRecord,
    canonical_event: &TraceCommonsAuditEvent,
    report: &mut TraceDbAuditChainReport,
) {
    let event_ref = event.audit_event_id;
    if canonical_event.event_id != event.audit_event_id {
        report.failures.push(format!(
            "db row {row_number} event {event_ref}: canonical event_id mismatch"
        ));
    }
    if canonical_event.tenant_id != event.tenant_id {
        report.failures.push(format!(
            "db row {row_number} event {event_ref}: canonical tenant_id mismatch"
        ));
    }
    if Some(canonical_event.submission_id).filter(|id| *id != Uuid::nil()) != event.submission_id {
        report.failures.push(format!(
            "db row {row_number} event {event_ref}: canonical submission_id mismatch"
        ));
    }
    if canonical_event.kind != storage_audit_canonical_kind(event) {
        report.failures.push(format!(
            "db row {row_number} event {event_ref}: canonical kind/action mismatch"
        ));
    }
    if canonical_event.status != storage_audit_canonical_status(event) {
        report.failures.push(format!(
            "db row {row_number} event {event_ref}: canonical status mismatch"
        ));
    }
    if let Some(actor_role) = canonical_event.actor_role
        && actor_role.storage_name() != event.actor_role
    {
        report.failures.push(format!(
            "db row {row_number} event {event_ref}: canonical actor_role mismatch"
        ));
    }
    if let Some(actor_principal_ref) = canonical_event.actor_principal_ref.as_deref()
        && actor_principal_ref != event.actor_principal_ref
    {
        report.failures.push(format!(
            "db row {row_number} event {event_ref}: canonical actor_principal_ref mismatch"
        ));
    }
    if canonical_event.reason != event.reason {
        report.failures.push(format!(
            "db row {row_number} event {event_ref}: canonical reason mismatch"
        ));
    }
    if canonical_event.export_id != event.export_manifest_id {
        report.failures.push(format!(
            "db row {row_number} event {event_ref}: canonical export_id mismatch"
        ));
    }
    if canonical_event.decision_inputs_hash != event.decision_inputs_hash {
        report.failures.push(format!(
            "db row {row_number} event {event_ref}: canonical decision_inputs_hash mismatch"
        ));
    }
    if canonical_event.previous_event_hash != event.previous_event_hash {
        report.failures.push(format!(
            "db row {row_number} event {event_ref}: canonical previous_event_hash mismatch"
        ));
    }
    if canonical_event.event_hash.is_some() {
        report.failures.push(format!(
            "db row {row_number} event {event_ref}: canonical payload should not include event_hash"
        ));
    }
    if let Some(projected_export_count) = storage_audit_canonical_export_count(event)
        && canonical_event.export_count != Some(projected_export_count)
    {
        report.failures.push(format!(
            "db row {row_number} event {event_ref}: canonical export_count mismatch"
        ));
    }
}

fn storage_audit_canonical_kind(event: &StorageTraceAuditEventRecord) -> String {
    if event.action == StorageTraceAuditAction::Read
        && event.submission_id.is_some()
        && event
            .reason
            .as_deref()
            .is_some_and(|reason| reason.contains("surface=replay_dataset_export"))
    {
        return "trace_content_read".to_string();
    }
    if event.action == StorageTraceAuditAction::Retain
        && matches!(
            &event.metadata,
            StorageTraceAuditSafeMetadata::Maintenance { .. }
        )
        && event
            .reason
            .as_deref()
            .is_some_and(|reason| reason.starts_with("purpose="))
    {
        return "maintenance".to_string();
    }
    storage_audit_event_kind(event.action, &event.metadata)
}

fn storage_audit_canonical_status(
    event: &StorageTraceAuditEventRecord,
) -> Option<TraceCorpusStatus> {
    match &event.metadata {
        StorageTraceAuditSafeMetadata::Submission { status, .. }
        | StorageTraceAuditSafeMetadata::ReviewDecision {
            resulting_status: status,
            ..
        } => trace_corpus_status_from_storage(*status),
        _ => None,
    }
}

fn storage_audit_canonical_export_count(event: &StorageTraceAuditEventRecord) -> Option<usize> {
    match &event.metadata {
        StorageTraceAuditSafeMetadata::Export { item_count, .. } => Some(*item_count as usize),
        _ => None,
    }
}

async fn backfill_db_mirror_from_files(
    state: &AppState,
    tenant: &TenantAuth,
    records: &[TraceCommonsSubmissionRecord],
    derived: &[TraceCommonsDerivedRecord],
    enabled: bool,
    dry_run: bool,
) -> anyhow::Result<TraceBackfillReport> {
    let mut report = TraceBackfillReport::default();
    if !enabled {
        return Ok(report);
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
    let records_by_submission = records
        .iter()
        .map(|record| (record.submission_id, record))
        .collect::<BTreeMap<_, _>>();
    for record in records {
        if record.status == TraceCorpusStatus::Purged {
            continue;
        }
        if let Some(db) = db
            && !dry_run
            && db
                .get_trace_submission(&tenant.tenant_id, record.submission_id)
                .await
                .with_context(|| {
                    format!(
                        "failed to query existing DB submission for backfill {}",
                        record.submission_id
                    )
                })?
                .is_some()
        {
            continue;
        }

        let envelope = match read_envelope_by_record(state, record)
            .with_context(|| format!("failed to validate envelope {}", record.submission_id))
        {
            Ok(envelope) => envelope,
            Err(error) => {
                report.record_failure(
                    "submission",
                    record.submission_id.to_string(),
                    error.to_string(),
                );
                continue;
            }
        };
        let derived_record = derived_by_submission
            .get(&record.submission_id)
            .copied()
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "trace submission {} is missing a derived precheck record",
                    record.submission_id
                )
            });
        let derived_record = match derived_record {
            Ok(record) => record,
            Err(error) => {
                report.record_failure(
                    "submission",
                    record.submission_id.to_string(),
                    error.to_string(),
                );
                continue;
            }
        };
        if dry_run {
            report.backfilled += 1;
            continue;
        }
        if let Err(error) = mirror_submission_to_db_with_options(
            state,
            tenant,
            record,
            derived_record,
            &envelope,
            false,
        )
        .await
        {
            report.record_failure(
                "submission",
                record.submission_id.to_string(),
                error.to_string(),
            );
            continue;
        }
        if record.is_revoked()
            && let Err(error) =
                mirror_revocation_to_db(state, tenant, record.submission_id, Some(record), None)
                    .await
        {
            report.record_failure(
                "submission",
                record.submission_id.to_string(),
                error.to_string(),
            );
            continue;
        }
        report.backfilled += 1;
    }

    let file_credit_events = read_all_credit_events(&state.root, &tenant.tenant_id)?;
    let file_audit_events = read_all_audit_events(&state.root, &tenant.tenant_id)?;
    let file_export_manifests = read_all_export_manifests(&state.root, &tenant.tenant_id)?;
    if dry_run {
        report.backfilled += file_credit_events.len() + file_audit_events.len();
        for manifest in &file_export_manifests {
            match replay_export_items_for_manifest_backfill(
                state,
                manifest,
                &records_by_submission,
                &derived_by_submission,
            ) {
                Ok(_) => report.backfilled += 1,
                Err(error) => report.record_failure(
                    "replay_export_manifest",
                    manifest.export_id.to_string(),
                    error.to_string(),
                ),
            }
        }
        return Ok(report);
    }

    let Some(db) = db else {
        return Ok(report);
    };
    let existing_credit_event_ids = db
        .list_trace_credit_events(&tenant.tenant_id)
        .await
        .context("failed to list trace credit events for DB backfill")?
        .into_iter()
        .map(|event| event.credit_event_id)
        .collect::<BTreeSet<_>>();
    for event in &file_credit_events {
        if existing_credit_event_ids.contains(&event.event_id) {
            continue;
        }
        match mirror_credit_event_to_db(state, event).await {
            Ok(()) => report.backfilled += 1,
            Err(error) => report.record_failure(
                "credit_event",
                event.event_id.to_string(),
                error.to_string(),
            ),
        }
    }

    let existing_audit_event_ids = db
        .list_trace_audit_events(&tenant.tenant_id)
        .await
        .context("failed to list trace audit events for DB backfill")?
        .into_iter()
        .map(|event| event.audit_event_id)
        .collect::<BTreeSet<_>>();
    for event in &file_audit_events {
        if existing_audit_event_ids.contains(&event.event_id) {
            continue;
        }
        let (action, metadata) = audit_backfill_storage_projection(event);
        match mirror_audit_event_to_db(state, tenant, event, action, metadata).await {
            Ok(()) => report.backfilled += 1,
            Err(error) => {
                report.record_failure("audit_event", event.event_id.to_string(), error.to_string())
            }
        }
    }

    let existing_export_manifest_ids = db
        .list_trace_export_manifests(&tenant.tenant_id)
        .await
        .context("failed to list trace export manifests for DB backfill")?
        .into_iter()
        .map(|manifest| manifest.export_manifest_id)
        .collect::<BTreeSet<_>>();
    for manifest in &file_export_manifests {
        if existing_export_manifest_ids.contains(&manifest.export_id) {
            continue;
        }
        let items = match replay_export_items_for_manifest_backfill(
            state,
            manifest,
            &records_by_submission,
            &derived_by_submission,
        ) {
            Ok(items) => items,
            Err(error) => {
                report.record_failure(
                    "replay_export_manifest",
                    manifest.export_id.to_string(),
                    error.to_string(),
                );
                continue;
            }
        };
        match mirror_export_manifest_to_db(
            state,
            StorageTraceObjectArtifactKind::ExportArtifact,
            manifest,
            &items,
        )
        .await
        {
            Ok(()) => report.backfilled += 1,
            Err(error) => report.record_failure(
                "replay_export_manifest",
                manifest.export_id.to_string(),
                error.to_string(),
            ),
        }
    }
    Ok(report)
}

fn audit_backfill_storage_projection(
    event: &TraceCommonsAuditEvent,
) -> (StorageTraceAuditAction, StorageTraceAuditSafeMetadata) {
    let action = match event.kind.as_str() {
        "submitted" => StorageTraceAuditAction::Submit,
        "read" => StorageTraceAuditAction::Read,
        "review_decision" => StorageTraceAuditAction::Review,
        "credit_mutate" => StorageTraceAuditAction::CreditMutate,
        "revoked" => StorageTraceAuditAction::Revoke,
        "dataset_export" | "ranker_training_candidates_export" | "ranker_training_pairs_export" => {
            StorageTraceAuditAction::Export
        }
        "maintenance" => StorageTraceAuditAction::Retain,
        "purge" => StorageTraceAuditAction::Purge,
        "vector_index" => StorageTraceAuditAction::VectorIndex,
        "benchmark_conversion" | "benchmark_lifecycle_update" => {
            StorageTraceAuditAction::BenchmarkConvert
        }
        "tenant_policy_update" => StorageTraceAuditAction::PolicyUpdate,
        _ => StorageTraceAuditAction::Read,
    };
    let metadata = match event.kind.as_str() {
        "submitted" => event
            .status
            .map(|status| StorageTraceAuditSafeMetadata::Submission {
                status: storage_corpus_status(status),
                privacy_risk: "unknown".to_string(),
            })
            .unwrap_or(StorageTraceAuditSafeMetadata::Empty),
        "dataset_export" | "ranker_training_candidates_export" | "ranker_training_pairs_export" => {
            StorageTraceAuditSafeMetadata::Export {
                artifact_kind: StorageTraceObjectArtifactKind::ExportArtifact,
                purpose_code: Some(event.kind.clone()),
                item_count: event
                    .export_count
                    .unwrap_or_default()
                    .min(u32::MAX as usize) as u32,
            }
        }
        "benchmark_conversion" | "benchmark_lifecycle_update" => {
            StorageTraceAuditSafeMetadata::Export {
                artifact_kind: StorageTraceObjectArtifactKind::BenchmarkArtifact,
                purpose_code: Some(event.kind.clone()),
                item_count: event
                    .export_count
                    .unwrap_or_default()
                    .min(u32::MAX as usize) as u32,
            }
        }
        _ => StorageTraceAuditSafeMetadata::Empty,
    };
    (action, metadata)
}

fn replay_export_items_for_manifest_backfill(
    state: &AppState,
    manifest: &TraceReplayExportManifest,
    records_by_submission: &BTreeMap<Uuid, &TraceCommonsSubmissionRecord>,
    derived_by_submission: &BTreeMap<Uuid, &TraceCommonsDerivedRecord>,
) -> anyhow::Result<Vec<TraceReplayDatasetItem>> {
    let mut items = Vec::new();
    for submission_id in &manifest.source_submission_ids {
        let Some(record) = records_by_submission.get(submission_id) else {
            continue;
        };
        let envelope = read_envelope_by_record(state, record).with_context(|| {
            format!(
                "failed to read envelope {} while backfilling replay export manifest {}",
                record.submission_id, manifest.export_id
            )
        })?;
        items.push(TraceReplayDatasetItem::from_record(
            record,
            derived_by_submission.get(submission_id).copied(),
            &envelope,
            None,
        ));
    }
    Ok(items)
}

async fn index_vector_metadata_from_db(
    state: &AppState,
    tenant: &TenantAuth,
    enabled: bool,
    dry_run: bool,
    purpose: &str,
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
        let body_read = read_envelope_from_active_db_object_ref(
            state,
            &tenant.tenant_id,
            record.submission_id,
        )
        .await?
        .with_context(|| {
            format!(
                "missing active submitted envelope object ref for vector indexing source {}",
                record.submission_id
            )
        })?;
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
        append_trace_content_read_audit(
            state,
            tenant,
            record.submission_id,
            body_read.object_ref_id,
            "vector_index",
            Some(purpose),
        )
        .await?;
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
        .ok_or_else(|| anyhow::Error::new(TraceDbDualWriteRequiredForReconciliation))?;
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
    let db_submission_export_eligibility = db_records
        .iter()
        .map(|record| {
            (
                record.submission_id,
                storage_submission_is_export_source_eligible(record),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let invalid_source_submission_ids = db_submission_export_eligibility
        .iter()
        .filter_map(|(submission_id, eligible)| (!eligible).then_some(*submission_id))
        .collect::<BTreeSet<_>>();
    let mut db_export_manifest_item_count = 0usize;
    let mut db_export_manifest_item_missing_object_ref_count = 0usize;
    let mut db_export_manifest_ids_with_missing_object_refs = BTreeSet::new();
    let mut active_export_manifest_ids_for_invalid_sources = BTreeSet::new();
    let mut active_export_manifest_items_for_invalid_sources = Vec::new();
    for manifest in &db_export_manifests {
        let items = db
            .list_trace_export_manifest_items(&tenant.tenant_id, manifest.export_manifest_id)
            .await
            .with_context(|| {
                format!(
                    "failed to list trace export manifest items for manifest {}",
                    manifest.export_manifest_id
                )
            })?;
        db_export_manifest_item_count += items.len();
        let missing_object_ref_count = items
            .iter()
            .filter(|item| item.object_ref_id.is_none())
            .count();
        db_export_manifest_item_missing_object_ref_count += missing_object_ref_count;
        if missing_object_ref_count > 0 {
            db_export_manifest_ids_with_missing_object_refs.insert(manifest.export_manifest_id);
        }
        let manifest_active = manifest.invalidated_at.is_none() && manifest.deleted_at.is_none();
        if manifest_active
            && manifest.source_submission_ids.iter().any(|submission_id| {
                !db_submission_export_eligibility
                    .get(submission_id)
                    .copied()
                    .unwrap_or(false)
            })
        {
            active_export_manifest_ids_for_invalid_sources.insert(manifest.export_manifest_id);
        }
        active_export_manifest_items_for_invalid_sources.extend(
            items
                .iter()
                .filter(|item| item.source_invalidated_at.is_none())
                .filter(|item| {
                    !db_submission_export_eligibility
                        .get(&item.submission_id)
                        .copied()
                        .unwrap_or(false)
                })
                .map(|item| TraceDbExportManifestItemInvalidSource {
                    export_manifest_id: item.export_manifest_id,
                    submission_id: item.submission_id,
                    derived_id: item.derived_id,
                    object_ref_id: item.object_ref_id,
                    vector_entry_id: item.vector_entry_id,
                    source_status_at_export: item.source_status_at_export,
                    source_invalidation_reason: item.source_invalidation_reason,
                }),
        );
    }
    let active_derived_submission_ids_for_invalid_sources = db_derived
        .iter()
        .filter(|record| record.status == StorageTraceDerivedStatus::Current)
        .filter_map(|record| {
            (invalid_source_submission_ids.contains(&record.submission_id))
                .then_some(record.submission_id)
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    active_export_manifest_items_for_invalid_sources.sort_by_key(|item| {
        (
            item.export_manifest_id,
            item.submission_id,
            item.derived_id,
            item.object_ref_id,
        )
    });
    let active_export_manifest_ids_with_ineligible_items =
        active_export_manifest_items_for_invalid_sources
            .iter()
            .map(|item| item.export_manifest_id)
            .collect::<BTreeSet<_>>();
    active_export_manifest_ids_for_invalid_sources.extend(
        active_export_manifest_items_for_invalid_sources
            .iter()
            .filter(|item| {
                !db_submission_export_eligibility
                    .get(&item.submission_id)
                    .copied()
                    .unwrap_or(false)
            })
            .map(|item| item.export_manifest_id),
    );
    let db_replay_export_manifest_count = db_export_manifests
        .iter()
        .filter(|manifest| is_replay_dataset_storage_manifest(manifest))
        .count();
    let db_benchmark_export_manifest_count = db_export_manifests
        .iter()
        .filter(|manifest| {
            manifest.artifact_kind == StorageTraceObjectArtifactKind::BenchmarkArtifact
        })
        .count();
    let db_ranker_export_manifest_count = db_export_manifests
        .iter()
        .filter(|manifest| is_ranker_training_storage_manifest(manifest))
        .count();
    let db_other_export_manifest_count = db_export_manifests.len().saturating_sub(
        db_replay_export_manifest_count
            + db_benchmark_export_manifest_count
            + db_ranker_export_manifest_count,
    );
    let file_credit_events = read_all_credit_events(&state.root, &tenant.tenant_id)?;
    let file_audit_events = read_all_audit_events(&state.root, &tenant.tenant_id)?;
    let file_replay_export_manifests = read_all_export_manifests(&state.root, &tenant.tenant_id)?;
    let file_revocations = read_all_revocations(&state.root, &tenant.tenant_id)?;
    let file_credit_event_ids = file_credit_events
        .iter()
        .map(|event| event.event_id)
        .collect::<BTreeSet<_>>();
    let db_credit_event_ids = db_credit_events
        .iter()
        .map(|event| event.credit_event_id)
        .collect::<BTreeSet<_>>();
    let db_file_projected_credit_event_ids = db_credit_events
        .iter()
        .filter(|event| trace_credit_event_type_from_storage(event.event_type).is_some())
        .map(|event| event.credit_event_id)
        .collect::<BTreeSet<_>>();
    let missing_credit_event_ids_in_db = file_credit_event_ids
        .difference(&db_credit_event_ids)
        .copied()
        .collect::<Vec<_>>();
    let missing_credit_event_ids_in_files = db_file_projected_credit_event_ids
        .difference(&file_credit_event_ids)
        .copied()
        .collect::<Vec<_>>();
    let file_audit_event_ids = file_audit_events
        .iter()
        .map(|event| event.event_id)
        .collect::<BTreeSet<_>>();
    let db_audit_event_ids = db_audit_events
        .iter()
        .map(|event| event.audit_event_id)
        .collect::<BTreeSet<_>>();
    let db_file_projected_audit_event_ids = db_audit_events
        .iter()
        .filter(|event| event.canonical_event_json.is_some())
        .map(|event| event.audit_event_id)
        .collect::<BTreeSet<_>>();
    let missing_audit_event_ids_in_db = file_audit_event_ids
        .difference(&db_audit_event_ids)
        .copied()
        .collect::<Vec<_>>();
    let missing_audit_event_ids_in_files = db_file_projected_audit_event_ids
        .difference(&file_audit_event_ids)
        .copied()
        .collect::<Vec<_>>();
    let mut db_object_ref_count = 0usize;
    let mut accepted_without_active_envelope_object_ref = Vec::new();
    let mut unreadable_active_envelope_object_refs = Vec::new();
    let mut hash_mismatched_active_envelope_object_refs = Vec::new();
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
        {
            let active_object_ref = db
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
                })?;
            if let Some(object_ref) = active_object_ref {
                if let Err(error) =
                    read_envelope_from_object_ref(state, &tenant.tenant_id, &object_ref)
                {
                    match classify_trace_object_ref_read_failure(&error) {
                        TraceObjectRefReadFailureKind::HashMismatch => {
                            hash_mismatched_active_envelope_object_refs.push(record.submission_id);
                        }
                        TraceObjectRefReadFailureKind::Unreadable => {
                            unreadable_active_envelope_object_refs.push(record.submission_id);
                        }
                    }
                }
            } else {
                accepted_without_active_envelope_object_ref.push(record.submission_id);
            }
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
    let missing_derived_submission_ids_in_files = db_derived_by_submission
        .keys()
        .filter(|submission_id| !file_derived_by_submission.contains_key(submission_id))
        .copied()
        .collect::<Vec<_>>();
    let mut derived_status_mismatches = Vec::new();
    let mut derived_hash_mismatches = Vec::new();
    for (submission_id, db_record) in &db_derived_by_submission {
        let Some(file_record) = file_derived_by_submission.get(submission_id) else {
            continue;
        };
        let file_status = storage_derived_status(file_record.status);
        if db_record.status != file_status {
            derived_status_mismatches.push(TraceDbDerivedStatusMismatch {
                submission_id: *submission_id,
                file_status,
                db_status: db_record.status,
            });
        }
        if db_record.canonical_summary_hash.as_deref()
            != Some(file_record.canonical_summary_hash.as_str())
        {
            derived_hash_mismatches.push(TraceDbDerivedHashMismatch {
                submission_id: *submission_id,
                file_canonical_summary_hash: file_record.canonical_summary_hash.clone(),
                db_canonical_summary_hash: db_record.canonical_summary_hash.clone(),
            });
        }
    }

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
    let active_canonical_vector_keys = db_vectors
        .iter()
        .filter(|entry| entry.status == StorageTraceVectorEntryStatus::Active)
        .filter(|entry| {
            entry.source_projection == StorageTraceVectorEntrySourceProjection::CanonicalSummary
        })
        .map(|entry| {
            (
                entry.submission_id,
                entry.derived_id,
                entry.source_hash.clone(),
            )
        })
        .collect::<BTreeSet<_>>();
    let eligible_canonical_vector_keys = db_derived
        .iter()
        .filter(|record| record.status == StorageTraceDerivedStatus::Current)
        .filter(|record| accepted_submission_ids.contains(&record.submission_id))
        .filter_map(|record| {
            record
                .canonical_summary_hash
                .as_ref()
                .map(|hash| (record.submission_id, record.derived_id, hash.clone()))
        })
        .collect::<BTreeSet<_>>();
    let accepted_current_derived_without_active_vector_entry = eligible_canonical_vector_keys
        .difference(&active_canonical_vector_keys)
        .map(|(submission_id, _, _)| *submission_id)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let invalid_active_vector_entries = db_vectors
        .iter()
        .filter(|entry| entry.status == StorageTraceVectorEntryStatus::Active)
        .filter(|entry| {
            !accepted_submission_ids.contains(&entry.submission_id)
                || !current_derived_ids.contains(&(entry.submission_id, entry.derived_id))
                || (entry.source_projection
                    == StorageTraceVectorEntrySourceProjection::CanonicalSummary
                    && !eligible_canonical_vector_keys.contains(&(
                        entry.submission_id,
                        entry.derived_id,
                        entry.source_hash.clone(),
                    )))
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

    let mut report = TraceDbReconciliationReport {
        file_submission_count: file_records.len(),
        db_submission_count: db_records.len(),
        missing_submission_ids_in_db,
        missing_submission_ids_in_files,
        status_mismatches,
        file_derived_count: file_derived.len(),
        db_derived_count: db_derived.len(),
        missing_derived_submission_ids_in_db,
        missing_derived_submission_ids_in_files,
        derived_status_mismatches,
        derived_hash_mismatches,
        file_credit_event_count: file_credit_events.len(),
        db_credit_event_count: db_credit_events.len(),
        missing_credit_event_ids_in_db,
        missing_credit_event_ids_in_files,
        file_audit_event_count: file_audit_events.len(),
        db_audit_event_count: db_audit_events.len(),
        missing_audit_event_ids_in_db,
        missing_audit_event_ids_in_files,
        file_replay_export_manifest_count: file_replay_export_manifests.len(),
        db_export_manifest_count: db_export_manifests.len(),
        db_replay_export_manifest_count,
        db_benchmark_export_manifest_count,
        db_ranker_export_manifest_count,
        db_other_export_manifest_count,
        db_export_manifest_item_count,
        db_export_manifest_item_missing_object_ref_count,
        db_export_manifest_ids_with_missing_object_refs:
            db_export_manifest_ids_with_missing_object_refs
                .into_iter()
                .collect(),
        active_derived_submission_ids_for_invalid_sources,
        active_export_manifest_ids_for_invalid_sources:
            active_export_manifest_ids_for_invalid_sources
                .into_iter()
                .collect(),
        active_export_manifest_items_for_invalid_sources,
        active_export_manifest_ids_with_ineligible_items:
            active_export_manifest_ids_with_ineligible_items
                .into_iter()
                .collect(),
        file_revocation_tombstone_count: file_revocations.len(),
        db_tombstone_count: db_tombstones.len(),
        db_object_ref_count,
        accepted_without_active_envelope_object_ref,
        unreadable_active_envelope_object_refs,
        hash_mismatched_active_envelope_object_refs,
        contributor_credit_reader_parity_ok,
        reviewer_metadata_reader_parity_ok,
        analytics_reader_parity_ok,
        audit_reader_parity_ok,
        replay_export_manifest_reader_parity_ok,
        db_reader_parity_failures,
        active_vector_entries,
        accepted_current_derived_without_active_vector_entry,
        invalid_active_vector_entries,
        blocking_gaps: Vec::new(),
    };
    report.blocking_gaps = report.compute_blocking_gap_summaries();
    Ok(Some(report))
}

fn require_db_reconciliation_clean_request(
    state: &AppState,
    request: &TraceMaintenanceRequest,
) -> anyhow::Result<()> {
    if state.require_db_reconciliation_clean && !request.reconcile_db_mirror {
        return Err(anyhow::Error::new(TraceDbReconciliationRequestRequired));
    }
    Ok(())
}

fn enforce_db_reconciliation_clean(
    state: &AppState,
    report: Option<&TraceDbReconciliationReport>,
) -> anyhow::Result<()> {
    if !state.require_db_reconciliation_clean {
        return Ok(());
    }
    let Some(report) = report else {
        return Ok(());
    };
    if !report.blocking_gaps.is_empty() {
        return Err(anyhow::Error::new(TraceDbReconciliationNotClean {
            gaps: report.blocking_gaps.clone(),
        }));
    }
    Ok(())
}

fn storage_submission_is_export_source_eligible(record: &StorageTraceSubmissionRecord) -> bool {
    record.status == StorageTraceCorpusStatus::Accepted
        && record.revoked_at.is_none()
        && record.purged_at.is_none()
}

fn is_ranker_training_storage_manifest(record: &StorageTraceExportManifestRecord) -> bool {
    record.artifact_kind == StorageTraceObjectArtifactKind::ExportArtifact
        && record
            .purpose_code
            .as_deref()
            .is_some_and(is_ranker_training_purpose_code)
}

fn is_replay_dataset_storage_manifest(record: &StorageTraceExportManifestRecord) -> bool {
    record.artifact_kind == StorageTraceObjectArtifactKind::ExportArtifact
        && !is_ranker_training_storage_manifest(record)
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

#[derive(Debug, Default, Clone, Serialize)]
struct TraceBackfillReport {
    backfilled: usize,
    failed: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    failures: Vec<TraceBackfillFailure>,
}

impl TraceBackfillReport {
    fn record_failure(&mut self, item_kind: &str, item_ref: String, reason: String) {
        self.failed += 1;
        tracing::warn!(
            %item_kind,
            %item_ref,
            %reason,
            "Trace Commons DB mirror backfill skipped item"
        );
        if self.failures.len() < TRACE_BACKFILL_FAILURE_DETAIL_LIMIT {
            self.failures.push(TraceBackfillFailure {
                item_kind: item_kind.to_string(),
                item_ref,
                reason,
            });
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct TraceBackfillFailure {
    item_kind: String,
    item_ref: String,
    reason: String,
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
    benchmark_artifacts_invalidated: usize,
    trace_object_files_deleted: usize,
    encrypted_artifacts_deleted: usize,
    db_mirror_backfilled: usize,
    db_mirror_backfill_failed: usize,
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
            "benchmark_artifacts_invalidated".to_string(),
            self.benchmark_artifacts_invalidated.min(u32::MAX as usize) as u32,
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
            "db_mirror_backfill_failed".to_string(),
            self.db_mirror_backfill_failed.min(u32::MAX as usize) as u32,
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

async fn propagate_benchmark_artifact_source_invalidation(
    state: &AppState,
    tenant: &TenantAuth,
    revoked_submission_reasons: &BTreeMap<Uuid, String>,
    expired_submission_ids: &BTreeSet<Uuid>,
    dry_run: bool,
) -> anyhow::Result<usize> {
    let affected = affected_benchmark_conversion_ids(
        state,
        tenant,
        revoked_submission_reasons,
        expired_submission_ids,
    )
    .await?;
    let mut invalidated = 0usize;
    for (conversion_id, reason) in affected {
        let Some(mut artifact) =
            read_benchmark_conversion_artifact_for_invalidation(state, tenant, conversion_id)
                .await?
        else {
            continue;
        };
        if !mark_benchmark_artifact_source_invalidated(&mut artifact, &reason) {
            continue;
        }
        invalidated += 1;
        if dry_run {
            continue;
        }
        persist_benchmark_lifecycle_artifact(
            state,
            tenant,
            &artifact,
            "benchmark source invalidation",
        )
        .await?;
        reapply_benchmark_source_invalidation_to_db(
            state,
            tenant,
            &artifact,
            revoked_submission_reasons,
            expired_submission_ids,
        )
        .await?;
    }
    Ok(invalidated)
}

async fn affected_benchmark_conversion_ids(
    state: &AppState,
    tenant: &TenantAuth,
    revoked_submission_reasons: &BTreeMap<Uuid, String>,
    expired_submission_ids: &BTreeSet<Uuid>,
) -> anyhow::Result<BTreeMap<Uuid, String>> {
    let mut affected = BTreeMap::new();
    for path in read_export_provenance_paths(&state.root, &tenant.tenant_id)? {
        let provenance = read_export_provenance(&path)?;
        if provenance.export_kind != TraceExportProvenanceKind::BenchmarkConversion {
            continue;
        }
        if let Some(reason) = benchmark_source_invalidation_reason(
            &provenance.source_submission_ids,
            revoked_submission_reasons,
            expired_submission_ids,
        ) {
            affected.entry(provenance.export_id).or_insert(reason);
        }
    }

    if let Some(db) = state.db_mirror.as_ref() {
        for manifest in db
            .list_trace_export_manifests(&tenant.tenant_id)
            .await
            .context("failed to list benchmark manifests for source invalidation")?
        {
            if manifest.artifact_kind != StorageTraceObjectArtifactKind::BenchmarkArtifact {
                continue;
            }
            if let Some(reason) = benchmark_source_invalidation_reason(
                &manifest.source_submission_ids,
                revoked_submission_reasons,
                expired_submission_ids,
            ) {
                affected
                    .entry(manifest.export_manifest_id)
                    .or_insert(reason);
            }
        }
    }

    Ok(affected)
}

fn benchmark_source_invalidation_reason(
    source_submission_ids: &[Uuid],
    revoked_submission_reasons: &BTreeMap<Uuid, String>,
    expired_submission_ids: &BTreeSet<Uuid>,
) -> Option<String> {
    source_submission_ids.iter().find_map(|submission_id| {
        revoked_submission_reasons
            .get(submission_id)
            .cloned()
            .or_else(|| {
                expired_submission_ids
                    .contains(submission_id)
                    .then(|| "retention_expired_source".to_string())
            })
    })
}

fn mark_benchmark_artifact_source_invalidated(
    artifact: &mut TraceBenchmarkConversionArtifact,
    reason: &str,
) -> bool {
    if artifact.registry.status != TraceBenchmarkRegistryStatus::Published {
        return false;
    }
    artifact.registry.status = TraceBenchmarkRegistryStatus::Revoked;
    artifact.registry.revoked_at = Some(Utc::now());
    artifact.registry.revocation_reason = Some(reason.to_string());
    artifact.evaluation.status = TraceBenchmarkEvaluationStatus::Inconclusive;
    artifact.evaluation.last_update_reason = Some(format!("source invalidated: {reason}"));
    true
}

async fn reapply_benchmark_source_invalidation_to_db(
    state: &AppState,
    tenant: &TenantAuth,
    artifact: &TraceBenchmarkConversionArtifact,
    revoked_submission_reasons: &BTreeMap<Uuid, String>,
    expired_submission_ids: &BTreeSet<Uuid>,
) -> anyhow::Result<()> {
    let Some(db) = state.db_mirror.as_ref() else {
        return Ok(());
    };
    for submission_id in &artifact.source_submission_ids {
        if revoked_submission_reasons.contains_key(submission_id) {
            db.invalidate_trace_export_manifests_for_submission(&tenant.tenant_id, *submission_id)
                .await
                .with_context(|| {
                    format!("failed to reapply benchmark manifest revocation for {submission_id}")
                })?;
            db.invalidate_trace_export_manifest_items_for_submission(
                &tenant.tenant_id,
                *submission_id,
                StorageTraceExportManifestItemInvalidationReason::Revoked,
            )
            .await
            .with_context(|| {
                format!("failed to reapply benchmark item revocation for {submission_id}")
            })?;
        } else if expired_submission_ids.contains(submission_id) {
            db.invalidate_trace_export_manifests_for_submission(&tenant.tenant_id, *submission_id)
                .await
                .with_context(|| {
                    format!("failed to reapply benchmark manifest expiration for {submission_id}")
                })?;
            db.invalidate_trace_export_manifest_items_for_submission(
                &tenant.tenant_id,
                *submission_id,
                StorageTraceExportManifestItemInvalidationReason::Expired,
            )
            .await
            .with_context(|| {
                format!("failed to reapply benchmark item expiration for {submission_id}")
            })?;
        }
    }
    Ok(())
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

fn maintenance_error(error: anyhow::Error) -> (StatusCode, Json<ApiError>) {
    if let Some(error) = error.downcast_ref::<TraceDbReconciliationRequestRequired>() {
        return api_error(StatusCode::BAD_REQUEST, error.to_string());
    }
    if let Some(error) = error.downcast_ref::<TraceDbReconciliationNotClean>() {
        return api_error(StatusCode::CONFLICT, error.to_string());
    }
    if let Some(error) = error.downcast_ref::<TraceDbDualWriteRequiredForReconciliation>() {
        return api_error(StatusCode::SERVICE_UNAVAILABLE, error.to_string());
    }
    internal_error(error)
}

fn internal_error(error: impl std::fmt::Display) -> (StatusCode, Json<ApiError>) {
    tracing::error!(%error, "Trace Commons ingestion operation failed");
    api_error(
        StatusCode::INTERNAL_SERVER_ERROR,
        "trace commons operation failed",
    )
}

#[derive(Debug)]
struct TraceDbReconciliationRequestRequired;

impl std::fmt::Display for TraceDbReconciliationRequestRequired {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{TRACE_COMMONS_REQUIRE_DB_RECONCILIATION_CLEAN} requires reconcile_db_mirror=true"
        )
    }
}

impl std::error::Error for TraceDbReconciliationRequestRequired {}

#[derive(Debug)]
struct TraceDbReconciliationNotClean {
    gaps: Vec<String>,
}

impl std::fmt::Display for TraceDbReconciliationNotClean {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "Trace Commons DB reconciliation is not clean: {}",
            self.gaps.join(", ")
        )
    }
}

impl std::error::Error for TraceDbReconciliationNotClean {}

#[derive(Debug)]
struct TraceDbDualWriteRequiredForReconciliation;

impl std::fmt::Display for TraceDbDualWriteRequiredForReconciliation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "Trace Commons DB reconciliation requires TRACE_COMMONS_DB_DUAL_WRITE"
        )
    }
}

impl std::error::Error for TraceDbDualWriteRequiredForReconciliation {}

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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    allowed_uses: Vec<TraceAllowedUse>,
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
            && !self
                .purpose_code
                .as_deref()
                .is_some_and(is_ranker_training_purpose_code)
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

const TRACE_BENCHMARK_CONVERSION_SCHEMA_VERSION: &str =
    "ironclaw.trace_commons.benchmark_conversion.v1";

fn trace_benchmark_conversion_schema_version() -> String {
    TRACE_BENCHMARK_CONVERSION_SCHEMA_VERSION.to_string()
}

#[derive(Debug, Serialize, Deserialize)]
struct TraceBenchmarkConversionArtifact {
    #[serde(default = "trace_benchmark_conversion_schema_version")]
    artifact_schema_version: String,
    tenant_id: String,
    tenant_storage_ref: String,
    conversion_id: Uuid,
    audit_event_id: Uuid,
    purpose: String,
    #[serde(default)]
    registry: TraceBenchmarkRegistryMetadata,
    #[serde(default)]
    evaluation: TraceBenchmarkEvaluationMetadata,
    filters: TraceBenchmarkConversionFilters,
    source_submission_ids: Vec<Uuid>,
    source_submission_ids_hash: String,
    generated_at: DateTime<Utc>,
    item_count: usize,
    candidates: Vec<TraceBenchmarkCandidate>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TraceBenchmarkRegistryMetadata {
    status: TraceBenchmarkRegistryStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    registry_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    published_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    revoked_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    revocation_reason: Option<String>,
}

impl Default for TraceBenchmarkRegistryMetadata {
    fn default() -> Self {
        Self {
            status: TraceBenchmarkRegistryStatus::Candidate,
            registry_ref: None,
            published_at: None,
            revoked_at: None,
            revocation_reason: None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum TraceBenchmarkRegistryStatus {
    Candidate,
    Published,
    Revoked,
}

impl TraceBenchmarkRegistryStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Candidate => "candidate",
            Self::Published => "published",
            Self::Revoked => "revoked",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TraceBenchmarkEvaluationMetadata {
    status: TraceBenchmarkEvaluationStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    evaluator_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    evaluated_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    score: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pass_count: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    fail_count: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_update_reason: Option<String>,
}

impl Default for TraceBenchmarkEvaluationMetadata {
    fn default() -> Self {
        Self {
            status: TraceBenchmarkEvaluationStatus::NotRun,
            evaluator_ref: None,
            evaluated_at: None,
            score: None,
            pass_count: None,
            fail_count: None,
            last_update_reason: None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum TraceBenchmarkEvaluationStatus {
    NotRun,
    Queued,
    Running,
    Passed,
    Failed,
    Inconclusive,
}

impl TraceBenchmarkEvaluationStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::NotRun => "not_run",
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Passed => "passed",
            Self::Failed => "failed",
            Self::Inconclusive => "inconclusive",
        }
    }
}

fn apply_benchmark_lifecycle_update(
    artifact: &mut TraceBenchmarkConversionArtifact,
    update: BenchmarkLifecycleUpdateRequest,
) -> ApiResult<()> {
    if let Some(registry) = update.registry {
        if let Some(status) = registry.status {
            artifact.registry.status = status;
        }
        if let Some(registry_ref) = registry.registry_ref {
            artifact.registry.registry_ref = normalize_benchmark_lifecycle_ref(registry_ref)?;
        }
        if let Some(published_at) = registry.published_at {
            artifact.registry.published_at = Some(published_at);
        }
    }

    if let Some(evaluation) = update.evaluation {
        if let Some(status) = evaluation.status {
            artifact.evaluation.status = status;
        }
        if let Some(evaluator_ref) = evaluation.evaluator_ref {
            artifact.evaluation.evaluator_ref = normalize_benchmark_lifecycle_ref(evaluator_ref)?;
        }
        if let Some(evaluated_at) = evaluation.evaluated_at {
            artifact.evaluation.evaluated_at = Some(evaluated_at);
        }
        if let Some(score) = evaluation.score {
            if !(0.0..=1.0).contains(&score) {
                return Err(api_error(
                    StatusCode::BAD_REQUEST,
                    "benchmark evaluation score must be between 0 and 1",
                ));
            }
            artifact.evaluation.score = Some(score);
        }
        if let Some(pass_count) = evaluation.pass_count {
            artifact.evaluation.pass_count = Some(pass_count);
        }
        if let Some(fail_count) = evaluation.fail_count {
            artifact.evaluation.fail_count = Some(fail_count);
        }
    }

    if let Some(reason) = update.reason.map(|reason| reason.trim().to_string())
        && !reason.is_empty()
    {
        artifact.evaluation.last_update_reason = Some(reason);
    }
    Ok(())
}

fn normalize_benchmark_lifecycle_ref(value: String) -> ApiResult<Option<String>> {
    let value = value.trim().to_string();
    if value.is_empty() {
        return Ok(None);
    }
    if value.len() > 512 {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "benchmark lifecycle references are limited to 512 characters",
        ));
    }
    Ok(Some(value))
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
    #[serde(skip)]
    auth_principal_ref: String,
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
            auth_principal_ref: submission.auth_principal_ref.clone(),
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
    purpose: String,
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
    purpose: String,
    generated_at: DateTime<Utc>,
    item_count: usize,
    source_item_list_hash: String,
    pairs: Vec<TraceRankerTrainingPair>,
}

#[derive(Debug, Clone, Serialize)]
struct TraceRankerTrainingCandidate {
    submission_id: Uuid,
    trace_id: Uuid,
    #[serde(skip)]
    auth_principal_ref: String,
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
            auth_principal_ref: submission.auth_principal_ref.clone(),
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
    benchmark_artifacts_invalidated: usize,
    trace_object_files_deleted: usize,
    encrypted_artifacts_deleted: usize,
    db_mirror_backfilled: usize,
    db_mirror_backfill_failed: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    db_mirror_backfill_failures: Vec<TraceBackfillFailure>,
    vector_entries_indexed: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    audit_chain: Option<TraceAuditChainReport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    db_reconciliation: Option<TraceDbReconciliationReport>,
}

#[derive(Debug, Default, Serialize)]
struct TraceAuditChainReport {
    verified: bool,
    event_count: usize,
    legacy_event_count: usize,
    mismatch_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_event_hash: Option<String>,
    failures: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    db_mirror: Option<TraceDbAuditChainReport>,
}

#[derive(Debug, Default, Serialize)]
struct TraceDbAuditChainReport {
    verified: bool,
    event_count: usize,
    legacy_event_count: usize,
    payload_verified_event_count: usize,
    payload_unverified_event_count: usize,
    mismatch_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_event_hash: Option<String>,
    failures: Vec<String>,
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
    missing_derived_submission_ids_in_files: Vec<Uuid>,
    derived_status_mismatches: Vec<TraceDbDerivedStatusMismatch>,
    derived_hash_mismatches: Vec<TraceDbDerivedHashMismatch>,
    file_credit_event_count: usize,
    db_credit_event_count: usize,
    missing_credit_event_ids_in_db: Vec<Uuid>,
    missing_credit_event_ids_in_files: Vec<Uuid>,
    file_audit_event_count: usize,
    db_audit_event_count: usize,
    missing_audit_event_ids_in_db: Vec<Uuid>,
    missing_audit_event_ids_in_files: Vec<Uuid>,
    file_replay_export_manifest_count: usize,
    db_export_manifest_count: usize,
    db_replay_export_manifest_count: usize,
    db_benchmark_export_manifest_count: usize,
    db_ranker_export_manifest_count: usize,
    db_other_export_manifest_count: usize,
    db_export_manifest_item_count: usize,
    db_export_manifest_item_missing_object_ref_count: usize,
    db_export_manifest_ids_with_missing_object_refs: Vec<Uuid>,
    active_derived_submission_ids_for_invalid_sources: Vec<Uuid>,
    active_export_manifest_ids_for_invalid_sources: Vec<Uuid>,
    active_export_manifest_items_for_invalid_sources: Vec<TraceDbExportManifestItemInvalidSource>,
    active_export_manifest_ids_with_ineligible_items: Vec<Uuid>,
    file_revocation_tombstone_count: usize,
    db_tombstone_count: usize,
    db_object_ref_count: usize,
    accepted_without_active_envelope_object_ref: Vec<Uuid>,
    unreadable_active_envelope_object_refs: Vec<Uuid>,
    hash_mismatched_active_envelope_object_refs: Vec<Uuid>,
    contributor_credit_reader_parity_ok: bool,
    reviewer_metadata_reader_parity_ok: bool,
    analytics_reader_parity_ok: bool,
    audit_reader_parity_ok: bool,
    replay_export_manifest_reader_parity_ok: bool,
    db_reader_parity_failures: Vec<String>,
    active_vector_entries: usize,
    accepted_current_derived_without_active_vector_entry: Vec<Uuid>,
    invalid_active_vector_entries: usize,
    blocking_gaps: Vec<String>,
}

impl TraceDbReconciliationReport {
    fn compute_blocking_gap_summaries(&self) -> Vec<String> {
        let mut gaps = Vec::new();
        push_gap_count(
            &mut gaps,
            "missing_submission_ids_in_db",
            self.missing_submission_ids_in_db.len(),
        );
        push_gap_count(
            &mut gaps,
            "missing_submission_ids_in_files",
            self.missing_submission_ids_in_files.len(),
        );
        push_gap_count(&mut gaps, "status_mismatches", self.status_mismatches.len());
        push_gap_count(
            &mut gaps,
            "missing_derived_submission_ids_in_db",
            self.missing_derived_submission_ids_in_db.len(),
        );
        push_gap_count(
            &mut gaps,
            "missing_derived_submission_ids_in_files",
            self.missing_derived_submission_ids_in_files.len(),
        );
        push_gap_count(
            &mut gaps,
            "derived_status_mismatches",
            self.derived_status_mismatches.len(),
        );
        push_gap_count(
            &mut gaps,
            "derived_hash_mismatches",
            self.derived_hash_mismatches.len(),
        );
        push_gap_count(
            &mut gaps,
            "missing_credit_event_ids_in_db",
            self.missing_credit_event_ids_in_db.len(),
        );
        push_gap_count(
            &mut gaps,
            "missing_credit_event_ids_in_files",
            self.missing_credit_event_ids_in_files.len(),
        );
        push_gap_count(
            &mut gaps,
            "missing_audit_event_ids_in_db",
            self.missing_audit_event_ids_in_db.len(),
        );
        push_gap_count(
            &mut gaps,
            "missing_audit_event_ids_in_files",
            self.missing_audit_event_ids_in_files.len(),
        );
        push_gap_count(
            &mut gaps,
            "db_export_manifest_item_missing_object_ref_count",
            self.db_export_manifest_item_missing_object_ref_count,
        );
        push_gap_count(
            &mut gaps,
            "active_derived_submission_ids_for_invalid_sources",
            self.active_derived_submission_ids_for_invalid_sources.len(),
        );
        push_gap_count(
            &mut gaps,
            "active_export_manifest_ids_for_invalid_sources",
            self.active_export_manifest_ids_for_invalid_sources.len(),
        );
        push_gap_count(
            &mut gaps,
            "active_export_manifest_items_for_invalid_sources",
            self.active_export_manifest_items_for_invalid_sources.len(),
        );
        push_gap_count(
            &mut gaps,
            "accepted_without_active_envelope_object_ref",
            self.accepted_without_active_envelope_object_ref.len(),
        );
        push_gap_count(
            &mut gaps,
            "unreadable_active_envelope_object_refs",
            self.unreadable_active_envelope_object_refs.len(),
        );
        push_gap_count(
            &mut gaps,
            "hash_mismatched_active_envelope_object_refs",
            self.hash_mismatched_active_envelope_object_refs.len(),
        );
        push_gap_bool(
            &mut gaps,
            "contributor_credit_reader_parity",
            self.contributor_credit_reader_parity_ok,
        );
        push_gap_bool(
            &mut gaps,
            "reviewer_metadata_reader_parity",
            self.reviewer_metadata_reader_parity_ok,
        );
        push_gap_bool(
            &mut gaps,
            "analytics_reader_parity",
            self.analytics_reader_parity_ok,
        );
        push_gap_bool(
            &mut gaps,
            "audit_reader_parity",
            self.audit_reader_parity_ok,
        );
        push_gap_bool(
            &mut gaps,
            "replay_export_manifest_reader_parity",
            self.replay_export_manifest_reader_parity_ok,
        );
        push_gap_count(
            &mut gaps,
            "db_reader_parity_failures",
            self.db_reader_parity_failures.len(),
        );
        push_gap_count(
            &mut gaps,
            "accepted_current_derived_without_active_vector_entry",
            self.accepted_current_derived_without_active_vector_entry
                .len(),
        );
        push_gap_count(
            &mut gaps,
            "invalid_active_vector_entries",
            self.invalid_active_vector_entries,
        );
        gaps
    }
}

fn push_gap_count(gaps: &mut Vec<String>, name: &str, count: usize) {
    if count > 0 {
        gaps.push(format!("{name}={count}"));
    }
}

fn push_gap_bool(gaps: &mut Vec<String>, name: &str, ok: bool) {
    if !ok {
        gaps.push(format!("{name}=failed"));
    }
}

#[derive(Debug, Serialize)]
struct TraceDbExportManifestItemInvalidSource {
    export_manifest_id: Uuid,
    submission_id: Uuid,
    derived_id: Option<Uuid>,
    object_ref_id: Option<Uuid>,
    vector_entry_id: Option<Uuid>,
    source_status_at_export: StorageTraceCorpusStatus,
    source_invalidation_reason: Option<StorageTraceExportManifestItemInvalidationReason>,
}

#[derive(Debug, Serialize)]
struct TraceDbStatusMismatch {
    submission_id: Uuid,
    file_status: StorageTraceCorpusStatus,
    db_status: StorageTraceCorpusStatus,
}

#[derive(Debug, Serialize)]
struct TraceDbDerivedStatusMismatch {
    submission_id: Uuid,
    file_status: StorageTraceDerivedStatus,
    db_status: StorageTraceDerivedStatus,
}

#[derive(Debug, Serialize)]
struct TraceDbDerivedHashMismatch {
    submission_id: Uuid,
    file_canonical_summary_hash: String,
    db_canonical_summary_hash: Option<String>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    redaction_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    canonical_summary_hash: Option<String>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    previous_event_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    event_hash: Option<String>,
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
            previous_event_hash: None,
            event_hash: None,
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
            previous_event_hash: None,
            event_hash: None,
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
            previous_event_hash: None,
            event_hash: None,
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
            previous_event_hash: None,
            event_hash: None,
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
            previous_event_hash: None,
            event_hash: None,
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
            previous_event_hash: None,
            event_hash: None,
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
            previous_event_hash: None,
            event_hash: None,
        }
    }

    fn tenant_policy_update(
        auth: &TenantAuth,
        policy_version: &str,
        allowed_consent_scope_count: usize,
        allowed_use_count: usize,
        policy_projection_hash: &str,
    ) -> Self {
        Self {
            event_id: Uuid::new_v4(),
            tenant_id: auth.tenant_id.clone(),
            submission_id: Uuid::nil(),
            kind: "tenant_policy_update".to_string(),
            created_at: Utc::now(),
            status: None,
            actor_role: Some(auth.role),
            actor_principal_ref: Some(auth.principal_ref.clone()),
            reason: Some(format!(
                "policy_version={policy_version};allowed_consent_scope_count={allowed_consent_scope_count};allowed_use_count={allowed_use_count};policy_projection_hash={policy_projection_hash}"
            )),
            export_count: None,
            export_id: None,
            decision_inputs_hash: Some(policy_projection_hash.to_string()),
            previous_event_hash: None,
            event_hash: None,
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
            previous_event_hash: None,
            event_hash: None,
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
            previous_event_hash: None,
            event_hash: None,
        }
    }

    fn benchmark_lifecycle_update(
        auth: &TenantAuth,
        artifact: &TraceBenchmarkConversionArtifact,
    ) -> Self {
        Self {
            event_id: Uuid::new_v4(),
            tenant_id: auth.tenant_id.clone(),
            submission_id: Uuid::nil(),
            kind: "benchmark_lifecycle_update".to_string(),
            created_at: Utc::now(),
            status: None,
            actor_role: Some(auth.role),
            actor_principal_ref: Some(auth.principal_ref.clone()),
            reason: Some(format!(
                "registry_status={};evaluation_status={}",
                artifact.registry.status.as_str(),
                artifact.evaluation.status.as_str()
            )),
            export_count: Some(artifact.item_count),
            export_id: Some(artifact.conversion_id),
            decision_inputs_hash: Some(artifact.source_submission_ids_hash.clone()),
            previous_event_hash: None,
            event_hash: None,
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
            previous_event_hash: None,
            event_hash: None,
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
            previous_event_hash: None,
            event_hash: None,
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
                "purpose={purpose};dry_run={dry_run};records_marked_revoked={};records_marked_expired={};records_marked_purged={};derived_marked_revoked={};derived_marked_expired={};export_cache_files_pruned={};export_provenance_invalidated={};benchmark_artifacts_invalidated={};trace_object_files_deleted={};encrypted_artifacts_deleted={};db_mirror_backfilled={};db_mirror_backfill_failed={};vector_entries_indexed={}",
                counts.records_marked_revoked,
                counts.records_marked_expired,
                counts.records_marked_purged,
                counts.derived_marked_revoked,
                counts.derived_marked_expired,
                counts.export_cache_files_pruned,
                counts.export_provenance_invalidated,
                counts.benchmark_artifacts_invalidated,
                counts.trace_object_files_deleted,
                counts.encrypted_artifacts_deleted,
                counts.db_mirror_backfilled,
                counts.db_mirror_backfill_failed,
                counts.vector_entries_indexed
            )),
            export_count: Some(counts.export_cache_files_pruned),
            export_id: None,
            decision_inputs_hash: None,
            previous_event_hash: None,
            event_hash: None,
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

        let delayed_credit_eligible_submission_ids = records
            .iter()
            .filter(|record| delayed_credit_applies_to_record(record))
            .map(|record| record.submission_id)
            .collect::<BTreeSet<_>>();

        for record in &records {
            match record.status {
                TraceCorpusStatus::Accepted => {
                    response.accepted += 1;
                    response.credit_points_pending += record.credit_points_pending;
                    response.credit_points_final += record.credit_points_final.unwrap_or(0.0);
                }
                TraceCorpusStatus::Quarantined => response.quarantined += 1,
                TraceCorpusStatus::Revoked => response.revoked += 1,
                TraceCorpusStatus::Rejected => response.rejected += 1,
                TraceCorpusStatus::Expired | TraceCorpusStatus::Purged => response.expired += 1,
            }
        }

        response.credit_points_ledger = credit_events
            .iter()
            .filter(|event| delayed_credit_eligible_submission_ids.contains(&event.submission_id))
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

    fn test_state_with_db_reviewer_reads_require_object_refs(
        root: PathBuf,
        db_mirror: Option<Arc<dyn Database>>,
    ) -> Arc<AppState> {
        let mut tokens = BTreeMap::new();
        insert_token(&mut tokens, "tenant-a", "token-a", TokenRole::Contributor);
        insert_token(
            &mut tokens,
            "tenant-a",
            "review-token-a",
            TokenRole::Reviewer,
        );
        Arc::new(AppState {
            root,
            tokens: Arc::new(tokens),
            tenant_policies: Arc::new(BTreeMap::new()),
            require_tenant_submission_policy: false,
            db_mirror,
            db_contributor_reads: false,
            db_reviewer_reads: true,
            db_reviewer_require_object_refs: true,
            db_replay_export_reads: false,
            db_replay_export_require_object_refs: false,
            db_audit_reads: false,
            db_tenant_policy_reads: false,
            require_db_mirror_writes: false,
            require_derived_export_object_refs: false,
            object_primary_submit_review: false,
            object_primary_replay_export: false,
            object_primary_derived_exports: false,
            require_db_reconciliation_clean: false,
            require_export_guardrails: false,
            max_export_items_per_request: DEFAULT_TRACE_COMMONS_MAX_EXPORT_ITEMS_PER_REQUEST,
            submission_quota: TraceSubmissionQuotaConfig::default(),
            legal_hold_retention_policy_ids: Arc::new(BTreeSet::new()),
            artifact_store: None,
        })
    }

    fn test_state_with_db_replay_export_reads(
        root: PathBuf,
        db_mirror: Option<Arc<dyn Database>>,
    ) -> Arc<AppState> {
        test_state_with_options(root, db_mirror, None, false, false, true, false)
    }

    fn test_state_with_db_replay_export_reads_require_object_refs(
        root: PathBuf,
        db_mirror: Option<Arc<dyn Database>>,
    ) -> Arc<AppState> {
        test_state_with_options_policies_and_export_guardrails(
            root,
            db_mirror,
            None,
            false,
            false,
            true,
            true,
            false,
            BTreeMap::new(),
            false,
            false,
        )
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
        test_state_with_options_and_policies(
            root,
            db_mirror,
            artifact_store,
            db_contributor_reads,
            db_reviewer_reads,
            db_replay_export_reads,
            db_audit_reads,
            BTreeMap::new(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn test_state_with_options_and_policies(
        root: PathBuf,
        db_mirror: Option<Arc<dyn Database>>,
        artifact_store: Option<Arc<LocalEncryptedTraceArtifactStore>>,
        db_contributor_reads: bool,
        db_reviewer_reads: bool,
        db_replay_export_reads: bool,
        db_audit_reads: bool,
        tenant_policies: BTreeMap<String, TenantSubmissionPolicy>,
    ) -> Arc<AppState> {
        test_state_with_options_policies_and_export_guardrails(
            root,
            db_mirror,
            artifact_store,
            db_contributor_reads,
            db_reviewer_reads,
            db_replay_export_reads,
            false,
            db_audit_reads,
            tenant_policies,
            false,
            false,
        )
    }

    fn test_state_with_required_tenant_policies(
        root: PathBuf,
        tenant_policies: BTreeMap<String, TenantSubmissionPolicy>,
    ) -> Arc<AppState> {
        test_state_with_options_policies_and_export_guardrails(
            root,
            None,
            None,
            false,
            false,
            false,
            false,
            false,
            tenant_policies,
            true,
            false,
        )
    }

    fn test_state_with_export_guardrails(root: PathBuf) -> Arc<AppState> {
        test_state_with_options_policies_and_export_guardrails(
            root,
            None,
            None,
            false,
            false,
            false,
            false,
            false,
            BTreeMap::new(),
            false,
            true,
        )
    }

    fn test_state_with_submission_quota(
        root: PathBuf,
        submission_quota: TraceSubmissionQuotaConfig,
    ) -> Arc<AppState> {
        let mut state = test_state(root);
        Arc::make_mut(&mut state).submission_quota = submission_quota;
        state
    }

    fn test_state_with_required_db_mirror_writes(
        root: PathBuf,
        db_mirror: Option<Arc<dyn Database>>,
    ) -> Arc<AppState> {
        test_state_with_configured_artifact_store_policies_export_guardrails_and_required_db_writes(
            root,
            db_mirror,
            None,
            false,
            false,
            false,
            false,
            false,
            false,
            BTreeMap::new(),
            false,
            false,
            true,
            false,
        )
    }

    fn test_state_with_required_derived_export_object_refs(
        root: PathBuf,
        db_mirror: Option<Arc<dyn Database>>,
    ) -> Arc<AppState> {
        test_state_with_configured_artifact_store_policies_export_guardrails_and_required_db_writes(
            root,
            db_mirror,
            None,
            false,
            false,
            false,
            false,
            false,
            false,
            BTreeMap::new(),
            false,
            false,
            false,
            true,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn test_state_with_options_policies_and_export_guardrails(
        root: PathBuf,
        db_mirror: Option<Arc<dyn Database>>,
        artifact_store: Option<Arc<LocalEncryptedTraceArtifactStore>>,
        db_contributor_reads: bool,
        db_reviewer_reads: bool,
        db_replay_export_reads: bool,
        db_replay_export_require_object_refs: bool,
        db_audit_reads: bool,
        tenant_policies: BTreeMap<String, TenantSubmissionPolicy>,
        require_tenant_submission_policy: bool,
        require_export_guardrails: bool,
    ) -> Arc<AppState> {
        test_state_with_configured_artifact_store_policies_export_guardrails_and_required_db_writes(
            root,
            db_mirror,
            artifact_store.map(ConfiguredTraceArtifactStore::legacy),
            db_contributor_reads,
            db_reviewer_reads,
            db_replay_export_reads,
            db_replay_export_require_object_refs,
            db_audit_reads,
            false,
            tenant_policies,
            require_tenant_submission_policy,
            require_export_guardrails,
            false,
            false,
        )
    }

    fn test_state_with_db_tenant_policy_reads(
        root: PathBuf,
        db_mirror: Arc<dyn Database>,
        require_tenant_submission_policy: bool,
    ) -> Arc<AppState> {
        test_state_with_configured_artifact_store_policies_export_guardrails_and_required_db_writes(
            root,
            Some(db_mirror),
            None,
            false,
            false,
            false,
            false,
            false,
            true,
            BTreeMap::new(),
            require_tenant_submission_policy,
            false,
            false,
            false,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn test_state_with_configured_artifact_store_policies_and_export_guardrails(
        root: PathBuf,
        db_mirror: Option<Arc<dyn Database>>,
        artifact_store: Option<ConfiguredTraceArtifactStore>,
        db_contributor_reads: bool,
        db_reviewer_reads: bool,
        db_replay_export_reads: bool,
        db_replay_export_require_object_refs: bool,
        db_audit_reads: bool,
        db_tenant_policy_reads: bool,
        tenant_policies: BTreeMap<String, TenantSubmissionPolicy>,
        require_tenant_submission_policy: bool,
        require_export_guardrails: bool,
    ) -> Arc<AppState> {
        test_state_with_configured_artifact_store_policies_export_guardrails_and_required_db_writes(
            root,
            db_mirror,
            artifact_store,
            db_contributor_reads,
            db_reviewer_reads,
            db_replay_export_reads,
            db_replay_export_require_object_refs,
            db_audit_reads,
            db_tenant_policy_reads,
            tenant_policies,
            require_tenant_submission_policy,
            require_export_guardrails,
            false,
            false,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn test_state_with_configured_artifact_store_policies_export_guardrails_and_required_db_writes(
        root: PathBuf,
        db_mirror: Option<Arc<dyn Database>>,
        artifact_store: Option<ConfiguredTraceArtifactStore>,
        db_contributor_reads: bool,
        db_reviewer_reads: bool,
        db_replay_export_reads: bool,
        db_replay_export_require_object_refs: bool,
        db_audit_reads: bool,
        db_tenant_policy_reads: bool,
        tenant_policies: BTreeMap<String, TenantSubmissionPolicy>,
        require_tenant_submission_policy: bool,
        require_export_guardrails: bool,
        require_db_mirror_writes: bool,
        require_derived_export_object_refs: bool,
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
        insert_token(&mut tokens, "tenant-a", "admin-token-a", TokenRole::Admin);
        insert_token(
            &mut tokens,
            "tenant-a",
            "export-worker-token-a",
            TokenRole::ExportWorker,
        );
        insert_token(
            &mut tokens,
            "tenant-a",
            "retention-worker-token-a",
            TokenRole::RetentionWorker,
        );
        insert_token(
            &mut tokens,
            "tenant-a",
            "vector-worker-token-a",
            TokenRole::VectorWorker,
        );
        insert_token(
            &mut tokens,
            "tenant-a",
            "benchmark-worker-token-a",
            TokenRole::BenchmarkWorker,
        );
        insert_token(
            &mut tokens,
            "tenant-a",
            "utility-worker-token-a",
            TokenRole::UtilityWorker,
        );
        insert_token(&mut tokens, "tenant-b", "token-b", TokenRole::Contributor);
        insert_token(
            &mut tokens,
            "tenant-b",
            "review-token-b",
            TokenRole::Reviewer,
        );
        insert_token(&mut tokens, "tenant-b", "admin-token-b", TokenRole::Admin);
        Arc::new(AppState {
            root,
            tokens: Arc::new(tokens),
            tenant_policies: Arc::new(tenant_policies),
            require_tenant_submission_policy,
            db_mirror,
            db_contributor_reads,
            db_reviewer_reads,
            db_reviewer_require_object_refs: false,
            db_replay_export_reads,
            db_replay_export_require_object_refs,
            db_audit_reads,
            db_tenant_policy_reads,
            require_db_mirror_writes,
            require_derived_export_object_refs,
            object_primary_submit_review: false,
            object_primary_replay_export: false,
            object_primary_derived_exports: false,
            require_db_reconciliation_clean: false,
            require_export_guardrails,
            max_export_items_per_request: DEFAULT_TRACE_COMMONS_MAX_EXPORT_ITEMS_PER_REQUEST,
            submission_quota: TraceSubmissionQuotaConfig::default(),
            legal_hold_retention_policy_ids: Arc::new(BTreeSet::new()),
            artifact_store,
        })
    }

    fn test_state_with_object_primary_submit_review(
        root: PathBuf,
        db_mirror: Arc<dyn Database>,
        artifact_store: ConfiguredTraceArtifactStore,
    ) -> Arc<AppState> {
        test_state_with_object_primary_submit_review_options(root, db_mirror, artifact_store, false)
    }

    fn test_state_with_object_primary_submit_review_and_replay_export(
        root: PathBuf,
        db_mirror: Arc<dyn Database>,
        artifact_store: ConfiguredTraceArtifactStore,
    ) -> Arc<AppState> {
        test_state_with_object_primary_submit_review_options(root, db_mirror, artifact_store, true)
    }

    fn test_state_with_object_primary_derived_exports(
        root: PathBuf,
        db_mirror: Arc<dyn Database>,
        artifact_store: ConfiguredTraceArtifactStore,
    ) -> Arc<AppState> {
        let mut tokens = BTreeMap::new();
        insert_token(&mut tokens, "tenant-a", "token-a", TokenRole::Contributor);
        insert_token(
            &mut tokens,
            "tenant-a",
            "review-token-a",
            TokenRole::Reviewer,
        );
        Arc::new(AppState {
            root,
            tokens: Arc::new(tokens),
            tenant_policies: Arc::new(BTreeMap::new()),
            require_tenant_submission_policy: false,
            db_mirror: Some(db_mirror),
            db_contributor_reads: false,
            db_reviewer_reads: true,
            db_reviewer_require_object_refs: false,
            db_replay_export_reads: false,
            db_replay_export_require_object_refs: false,
            db_audit_reads: false,
            db_tenant_policy_reads: false,
            require_db_mirror_writes: true,
            require_derived_export_object_refs: true,
            object_primary_submit_review: false,
            object_primary_replay_export: false,
            object_primary_derived_exports: true,
            require_db_reconciliation_clean: false,
            require_export_guardrails: true,
            max_export_items_per_request: DEFAULT_TRACE_COMMONS_MAX_EXPORT_ITEMS_PER_REQUEST,
            submission_quota: TraceSubmissionQuotaConfig::default(),
            legal_hold_retention_policy_ids: Arc::new(BTreeSet::new()),
            artifact_store: Some(artifact_store),
        })
    }

    fn test_state_with_object_primary_submit_review_options(
        root: PathBuf,
        db_mirror: Arc<dyn Database>,
        artifact_store: ConfiguredTraceArtifactStore,
        db_replay_export_reads: bool,
    ) -> Arc<AppState> {
        let mut tokens = BTreeMap::new();
        insert_token(&mut tokens, "tenant-a", "token-a", TokenRole::Contributor);
        insert_token(
            &mut tokens,
            "tenant-a",
            "review-token-a",
            TokenRole::Reviewer,
        );
        Arc::new(AppState {
            root,
            tokens: Arc::new(tokens),
            tenant_policies: Arc::new(BTreeMap::new()),
            require_tenant_submission_policy: false,
            db_mirror: Some(db_mirror),
            db_contributor_reads: false,
            db_reviewer_reads: true,
            db_reviewer_require_object_refs: true,
            db_replay_export_reads,
            db_replay_export_require_object_refs: db_replay_export_reads,
            db_audit_reads: false,
            db_tenant_policy_reads: false,
            require_db_mirror_writes: true,
            require_derived_export_object_refs: false,
            object_primary_submit_review: true,
            object_primary_replay_export: db_replay_export_reads,
            object_primary_derived_exports: false,
            require_db_reconciliation_clean: false,
            require_export_guardrails: false,
            max_export_items_per_request: DEFAULT_TRACE_COMMONS_MAX_EXPORT_ITEMS_PER_REQUEST,
            submission_quota: TraceSubmissionQuotaConfig::default(),
            legal_hold_retention_policy_ids: Arc::new(BTreeSet::new()),
            artifact_store: Some(artifact_store),
        })
    }

    fn test_state_with_required_db_reconciliation_clean(
        root: PathBuf,
        db_mirror: Arc<dyn Database>,
    ) -> Arc<AppState> {
        let mut tokens = BTreeMap::new();
        insert_token(&mut tokens, "tenant-a", "token-a", TokenRole::Contributor);
        insert_token(
            &mut tokens,
            "tenant-a",
            "review-token-a",
            TokenRole::Reviewer,
        );
        Arc::new(AppState {
            root,
            tokens: Arc::new(tokens),
            tenant_policies: Arc::new(BTreeMap::new()),
            require_tenant_submission_policy: false,
            db_mirror: Some(db_mirror),
            db_contributor_reads: false,
            db_reviewer_reads: false,
            db_reviewer_require_object_refs: false,
            db_replay_export_reads: false,
            db_replay_export_require_object_refs: false,
            db_audit_reads: false,
            db_tenant_policy_reads: false,
            require_db_mirror_writes: false,
            require_derived_export_object_refs: false,
            object_primary_submit_review: false,
            object_primary_replay_export: false,
            object_primary_derived_exports: false,
            require_db_reconciliation_clean: true,
            require_export_guardrails: false,
            max_export_items_per_request: DEFAULT_TRACE_COMMONS_MAX_EXPORT_ITEMS_PER_REQUEST,
            submission_quota: TraceSubmissionQuotaConfig::default(),
            legal_hold_retention_policy_ids: Arc::new(BTreeSet::new()),
            artifact_store: None,
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

    fn test_reviewer_auth(tenant_id: &str) -> TenantAuth {
        TenantAuth {
            tenant_id: tenant_id.to_string(),
            role: TokenRole::Reviewer,
            principal_ref: principal_storage_ref("review-token"),
        }
    }

    fn audit_log_path(root: &Path, tenant_id: &str) -> PathBuf {
        root.join("tenants")
            .join(tenant_storage_key(tenant_id))
            .join("audit")
            .join("events.jsonl")
    }

    fn read_raw_audit_events(
        root: &Path,
        tenant_id: &str,
    ) -> anyhow::Result<Vec<TraceCommonsAuditEvent>> {
        let path = audit_log_path(root, tenant_id);
        let body = std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read raw audit events {}", path.display()))?;
        body.lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(|line| serde_json::from_str(line).context("failed to parse raw audit event"))
            .collect()
    }

    #[test]
    fn token_role_parses_worker_roles() {
        let cases = [
            ("contributor", TokenRole::Contributor, "contributor"),
            ("reviewer", TokenRole::Reviewer, "reviewer"),
            ("admin", TokenRole::Admin, "admin"),
            ("export_worker", TokenRole::ExportWorker, "export_worker"),
            ("export-worker", TokenRole::ExportWorker, "export_worker"),
            (
                "retention_worker",
                TokenRole::RetentionWorker,
                "retention_worker",
            ),
            (
                "retention-worker",
                TokenRole::RetentionWorker,
                "retention_worker",
            ),
            ("vector_worker", TokenRole::VectorWorker, "vector_worker"),
            (
                "benchmark_worker",
                TokenRole::BenchmarkWorker,
                "benchmark_worker",
            ),
            ("utility_worker", TokenRole::UtilityWorker, "utility_worker"),
            ("utility-worker", TokenRole::UtilityWorker, "utility_worker"),
        ];

        for (raw, expected, storage_name) in cases {
            let parsed = TokenRole::parse(raw).expect("role parses");
            assert_eq!(parsed, expected);
            assert_eq!(parsed.storage_name(), storage_name);
        }
        assert!(TokenRole::parse("trainer").is_err());
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
    async fn submit_quota_limits_tenant_new_submissions_but_allows_idempotent_retry() {
        let temp = tempfile::tempdir().expect("temp dir");
        let state = test_state_with_submission_quota(
            temp.path().to_path_buf(),
            TraceSubmissionQuotaConfig {
                max_per_tenant_per_hour: 1,
                max_per_principal_per_hour: 0,
            },
        );
        let envelope = sample_envelope().await;

        let Json(first_receipt) = submit_trace_handler(
            State(state.clone()),
            auth_headers("token-a"),
            Json(envelope.clone()),
        )
        .await
        .expect("first submission succeeds");
        let Json(retry_receipt) = submit_trace_handler(
            State(state.clone()),
            auth_headers("token-a"),
            Json(envelope),
        )
        .await
        .expect("idempotent retry bypasses quota");
        assert_eq!(retry_receipt.status, first_receipt.status);
        assert_eq!(
            retry_receipt.credit_points_pending,
            first_receipt.credit_points_pending
        );

        let blocked = submit_trace_handler(
            State(state.clone()),
            auth_headers("token-a-2"),
            Json(sample_envelope().await),
        )
        .await
        .expect_err("new tenant submission is rate limited");
        assert_eq!(blocked.0, StatusCode::TOO_MANY_REQUESTS);
        assert!(blocked.1.0.error.contains("tenant submission quota"));

        let _ = submit_trace_handler(
            State(state),
            auth_headers("token-b"),
            Json(sample_envelope().await),
        )
        .await
        .expect("quota is isolated by tenant");
    }

    #[tokio::test]
    async fn submit_quota_limits_principal_without_blocking_other_contributors() {
        let temp = tempfile::tempdir().expect("temp dir");
        let state = test_state_with_submission_quota(
            temp.path().to_path_buf(),
            TraceSubmissionQuotaConfig {
                max_per_tenant_per_hour: 0,
                max_per_principal_per_hour: 1,
            },
        );

        let _ = submit_trace_handler(
            State(state.clone()),
            auth_headers("token-a"),
            Json(sample_envelope().await),
        )
        .await
        .expect("first principal submission succeeds");
        let _ = submit_trace_handler(
            State(state.clone()),
            auth_headers("token-a-2"),
            Json(sample_envelope().await),
        )
        .await
        .expect("different contributor in same tenant is not blocked by principal quota");

        let blocked = submit_trace_handler(
            State(state),
            auth_headers("token-a"),
            Json(sample_envelope().await),
        )
        .await
        .expect_err("same principal is rate limited");
        assert_eq!(blocked.0, StatusCode::TOO_MANY_REQUESTS);
        assert!(blocked.1.0.error.contains("principal submission quota"));
    }

    #[tokio::test]
    async fn submit_quota_ignores_revoked_submissions() {
        let temp = tempfile::tempdir().expect("temp dir");
        let state = test_state_with_submission_quota(
            temp.path().to_path_buf(),
            TraceSubmissionQuotaConfig {
                max_per_tenant_per_hour: 1,
                max_per_principal_per_hour: 1,
            },
        );
        let first = sample_envelope().await;
        let first_submission_id = first.submission_id;
        let _ = submit_trace_handler(State(state.clone()), auth_headers("token-a"), Json(first))
            .await
            .expect("first submission succeeds");
        revoke_trace_handler(
            State(state.clone()),
            auth_headers("token-a"),
            AxumPath(first_submission_id),
        )
        .await
        .expect("contributor can revoke own submission");

        let mut replacement = sample_envelope().await;
        replacement.events[0].redacted_content =
            Some("Please inspect a different quota-safe workspace".to_string());
        replacement.privacy.redaction_hash = sha256_prefixed("quota replacement trace");

        let _ = submit_trace_handler(State(state), auth_headers("token-a"), Json(replacement))
            .await
            .expect("revoked submission no longer consumes quota");
    }

    #[test]
    fn parses_tenant_submission_policies_from_json() {
        let policies = parse_tenant_submission_policies(
            r#"{
                "tenant-a": {
                    "allowed_consent_scopes": ["debugging_evaluation", "benchmark_only"],
                    "allowed_uses": ["debugging", "evaluation", "aggregate_analytics"]
                }
            }"#,
        )
        .expect("policy parses");
        let policy = policies.get("tenant-a").expect("tenant policy exists");
        assert!(
            policy
                .allowed_consent_scopes
                .contains(&ConsentScope::DebuggingEvaluation)
        );
        assert!(
            policy
                .allowed_consent_scopes
                .contains(&ConsentScope::BenchmarkOnly)
        );
        assert!(policy.allowed_uses.contains(&TraceAllowedUse::Debugging));
        assert!(
            policy
                .allowed_uses
                .contains(&TraceAllowedUse::AggregateAnalytics)
        );
    }

    #[test]
    fn parses_legal_hold_retention_policy_ids_from_csv() {
        let policies = parse_legal_hold_retention_policy_ids(
            "private_corpus_revocable, benchmark-revocable,tenant:policy.1",
        )
        .expect("legal hold policies parse");

        assert!(policies.contains("private_corpus_revocable"));
        assert!(policies.contains("benchmark-revocable"));
        assert!(policies.contains("tenant:policy.1"));

        let error = parse_legal_hold_retention_policy_ids("private_corpus_revocable,sk-test/token")
            .expect_err("unsafe policy id should fail");
        assert!(
            error
                .to_string()
                .contains(TRACE_COMMONS_LEGAL_HOLD_RETENTION_POLICIES)
        );
        assert!(!error.to_string().contains("sk-test/token"));
    }

    #[test]
    fn parses_max_export_items_per_request() {
        assert_eq!(
            parse_max_export_items_per_request("25").expect("export cap parses"),
            25
        );

        let zero_error =
            parse_max_export_items_per_request("0").expect_err("zero export cap is invalid");
        assert!(
            zero_error
                .to_string()
                .contains(TRACE_COMMONS_MAX_EXPORT_ITEMS_PER_REQUEST)
        );

        let parse_error =
            parse_max_export_items_per_request("many").expect_err("non-numeric cap is invalid");
        assert!(
            parse_error
                .to_string()
                .contains(TRACE_COMMONS_MAX_EXPORT_ITEMS_PER_REQUEST)
        );
    }

    #[test]
    fn parses_submission_quota_limits() {
        assert_eq!(
            parse_submission_quota_limit(TRACE_COMMONS_MAX_SUBMISSIONS_PER_TENANT_PER_HOUR, "25")
                .expect("tenant quota parses"),
            25
        );
        assert_eq!(
            parse_submission_quota_limit(TRACE_COMMONS_MAX_SUBMISSIONS_PER_PRINCIPAL_PER_HOUR, "0")
                .expect("zero disables principal quota"),
            0
        );

        let parse_error =
            parse_submission_quota_limit(TRACE_COMMONS_MAX_SUBMISSIONS_PER_TENANT_PER_HOUR, "many")
                .expect_err("non-numeric quota is invalid");
        assert!(
            parse_error
                .to_string()
                .contains(TRACE_COMMONS_MAX_SUBMISSIONS_PER_TENANT_PER_HOUR)
        );
    }

    #[test]
    fn trace_object_store_kind_parses_operator_modes() {
        assert_eq!(
            TraceEncryptedObjectStoreKind::from_config(None, false).expect("mode parses"),
            None
        );
        assert_eq!(
            TraceEncryptedObjectStoreKind::from_config(None, true).expect("mode parses"),
            Some(TraceEncryptedObjectStoreKind::LegacyArtifactSidecar)
        );
        assert_eq!(
            TraceEncryptedObjectStoreKind::from_config(Some("file"), true).expect("mode parses"),
            None
        );
        assert_eq!(
            TraceEncryptedObjectStoreKind::from_config(Some("local_encrypted"), false)
                .expect("mode parses"),
            Some(TraceEncryptedObjectStoreKind::LegacyArtifactSidecar)
        );
        assert_eq!(
            TraceEncryptedObjectStoreKind::from_config(Some("local_service"), false)
                .expect("mode parses"),
            Some(TraceEncryptedObjectStoreKind::ServiceLocal)
        );

        let error = TraceEncryptedObjectStoreKind::from_config(Some("mystery_store"), false)
            .expect_err("unknown object store mode must fail");
        assert!(
            error
                .to_string()
                .contains("unsupported TRACE_COMMONS_OBJECT_STORE")
        );
    }

    #[test]
    fn object_primary_submit_review_validates_production_guards() {
        validate_object_primary_submit_review_config(
            true,
            true,
            true,
            true,
            true,
            Some(TRACE_COMMONS_SERVICE_LOCAL_ENCRYPTED_OBJECT_STORE),
        )
        .expect("complete production guard config is valid");

        let cases = [
            (
                false,
                true,
                true,
                true,
                Some(TRACE_COMMONS_SERVICE_LOCAL_ENCRYPTED_OBJECT_STORE),
                "TRACE_COMMONS_DB_DUAL_WRITE",
            ),
            (
                true,
                false,
                true,
                true,
                Some(TRACE_COMMONS_SERVICE_LOCAL_ENCRYPTED_OBJECT_STORE),
                "TRACE_COMMONS_REQUIRE_DB_MIRROR_WRITES",
            ),
            (
                true,
                true,
                false,
                true,
                Some(TRACE_COMMONS_SERVICE_LOCAL_ENCRYPTED_OBJECT_STORE),
                "TRACE_COMMONS_DB_REVIEWER_READS",
            ),
            (
                true,
                true,
                true,
                false,
                Some(TRACE_COMMONS_SERVICE_LOCAL_ENCRYPTED_OBJECT_STORE),
                "TRACE_COMMONS_DB_REVIEWER_REQUIRE_OBJECT_REFS",
            ),
            (true, true, true, true, None, "TRACE_COMMONS_OBJECT_STORE"),
            (
                true,
                true,
                true,
                true,
                Some(TRACE_COMMONS_LEGACY_ENCRYPTED_OBJECT_STORE),
                "TRACE_COMMONS_OBJECT_STORE",
            ),
        ];
        for (
            db_mirror_configured,
            require_db_mirror_writes,
            db_reviewer_reads,
            db_reviewer_require_object_refs,
            artifact_store_name,
            expected,
        ) in cases
        {
            let error = validate_object_primary_submit_review_config(
                true,
                db_mirror_configured,
                require_db_mirror_writes,
                db_reviewer_reads,
                db_reviewer_require_object_refs,
                artifact_store_name,
            )
            .expect_err("incomplete object-primary config must fail");
            assert!(
                error.to_string().contains(expected),
                "expected {expected} in {error}"
            );
        }
    }

    #[test]
    fn object_primary_replay_export_validates_production_guards() {
        validate_object_primary_replay_export_config(
            true,
            true,
            true,
            true,
            true,
            Some(TRACE_COMMONS_SERVICE_LOCAL_ENCRYPTED_OBJECT_STORE),
        )
        .expect("complete replay export guard config is valid");

        let cases = [
            (
                false,
                true,
                true,
                true,
                Some(TRACE_COMMONS_SERVICE_LOCAL_ENCRYPTED_OBJECT_STORE),
                "TRACE_COMMONS_DB_DUAL_WRITE",
            ),
            (
                true,
                false,
                true,
                true,
                Some(TRACE_COMMONS_SERVICE_LOCAL_ENCRYPTED_OBJECT_STORE),
                "TRACE_COMMONS_REQUIRE_DB_MIRROR_WRITES",
            ),
            (
                true,
                true,
                false,
                true,
                Some(TRACE_COMMONS_SERVICE_LOCAL_ENCRYPTED_OBJECT_STORE),
                "TRACE_COMMONS_DB_REPLAY_EXPORT_READS",
            ),
            (
                true,
                true,
                true,
                false,
                Some(TRACE_COMMONS_SERVICE_LOCAL_ENCRYPTED_OBJECT_STORE),
                "TRACE_COMMONS_DB_REPLAY_EXPORT_REQUIRE_OBJECT_REFS",
            ),
            (true, true, true, true, None, "TRACE_COMMONS_OBJECT_STORE"),
            (
                true,
                true,
                true,
                true,
                Some(TRACE_COMMONS_LEGACY_ENCRYPTED_OBJECT_STORE),
                "TRACE_COMMONS_OBJECT_STORE",
            ),
        ];
        for (
            db_mirror_configured,
            require_db_mirror_writes,
            db_replay_export_reads,
            db_replay_export_require_object_refs,
            artifact_store_name,
            expected,
        ) in cases
        {
            let error = validate_object_primary_replay_export_config(
                true,
                db_mirror_configured,
                require_db_mirror_writes,
                db_replay_export_reads,
                db_replay_export_require_object_refs,
                artifact_store_name,
            )
            .expect_err("incomplete object-primary replay config must fail");
            assert!(
                error.to_string().contains(expected),
                "expected {expected} in {error}"
            );
        }
    }

    #[test]
    fn object_primary_derived_exports_validates_production_guards() {
        validate_object_primary_derived_exports_config(
            true,
            true,
            true,
            true,
            true,
            true,
            Some(TRACE_COMMONS_SERVICE_LOCAL_ENCRYPTED_OBJECT_STORE),
        )
        .expect("complete derived export guard config is valid");

        let cases = [
            (
                false,
                true,
                true,
                true,
                true,
                Some(TRACE_COMMONS_SERVICE_LOCAL_ENCRYPTED_OBJECT_STORE),
                "TRACE_COMMONS_DB_DUAL_WRITE",
            ),
            (
                true,
                false,
                true,
                true,
                true,
                Some(TRACE_COMMONS_SERVICE_LOCAL_ENCRYPTED_OBJECT_STORE),
                "TRACE_COMMONS_REQUIRE_DB_MIRROR_WRITES",
            ),
            (
                true,
                true,
                false,
                true,
                true,
                Some(TRACE_COMMONS_SERVICE_LOCAL_ENCRYPTED_OBJECT_STORE),
                "TRACE_COMMONS_DB_REVIEWER_READS",
            ),
            (
                true,
                true,
                true,
                false,
                true,
                Some(TRACE_COMMONS_SERVICE_LOCAL_ENCRYPTED_OBJECT_STORE),
                "TRACE_COMMONS_DERIVED_EXPORT_REQUIRE_OBJECT_REFS",
            ),
            (
                true,
                true,
                true,
                true,
                false,
                Some(TRACE_COMMONS_SERVICE_LOCAL_ENCRYPTED_OBJECT_STORE),
                "TRACE_COMMONS_REQUIRE_EXPORT_GUARDRAILS",
            ),
            (
                true,
                true,
                true,
                true,
                true,
                None,
                "TRACE_COMMONS_OBJECT_STORE",
            ),
            (
                true,
                true,
                true,
                true,
                true,
                Some(TRACE_COMMONS_LEGACY_ENCRYPTED_OBJECT_STORE),
                "TRACE_COMMONS_OBJECT_STORE",
            ),
        ];
        for (
            db_mirror_configured,
            require_db_mirror_writes,
            db_reviewer_reads,
            require_derived_export_object_refs,
            require_export_guardrails,
            artifact_store_name,
            expected,
        ) in cases
        {
            let error = validate_object_primary_derived_exports_config(
                true,
                db_mirror_configured,
                require_db_mirror_writes,
                db_reviewer_reads,
                require_derived_export_object_refs,
                require_export_guardrails,
                artifact_store_name,
            )
            .expect_err("incomplete object-primary derived export config must fail");
            assert!(
                error.to_string().contains(expected),
                "expected {expected} in {error}"
            );
        }
    }

    #[tokio::test]
    async fn submit_rejects_scope_disallowed_by_tenant_policy() {
        let temp = tempfile::tempdir().expect("temp dir");
        let mut policies = BTreeMap::new();
        policies.insert(
            "tenant-a".to_string(),
            TenantSubmissionPolicy {
                allowed_consent_scopes: BTreeSet::from([ConsentScope::DebuggingEvaluation]),
                allowed_uses: BTreeSet::new(),
            },
        );
        let state = test_state_with_options_and_policies(
            temp.path().to_path_buf(),
            None,
            None,
            false,
            false,
            false,
            false,
            policies,
        );
        let mut envelope = sample_envelope().await;
        envelope.consent.scopes = vec![ConsentScope::ModelTraining];
        envelope.trace_card.consent_scope = ConsentScope::ModelTraining;

        let error = submit_trace_handler(
            State(state.clone()),
            auth_headers("token-a"),
            Json(envelope.clone()),
        )
        .await
        .expect_err("tenant policy rejects disallowed consent scope");
        assert_eq!(error.0, StatusCode::FORBIDDEN);

        let Json(receipt) =
            submit_trace_handler(State(state), auth_headers("token-b"), Json(envelope))
                .await
                .expect("tenant without explicit policy can submit");
        assert_eq!(receipt.status, "quarantined");
    }

    #[tokio::test]
    async fn submit_rejects_allowed_use_disallowed_by_tenant_policy() {
        let temp = tempfile::tempdir().expect("temp dir");
        let mut policies = BTreeMap::new();
        policies.insert(
            "tenant-a".to_string(),
            TenantSubmissionPolicy {
                allowed_consent_scopes: BTreeSet::from([ConsentScope::ModelTraining]),
                allowed_uses: BTreeSet::from([
                    TraceAllowedUse::Debugging,
                    TraceAllowedUse::Evaluation,
                    TraceAllowedUse::AggregateAnalytics,
                ]),
            },
        );
        let state = test_state_with_options_and_policies(
            temp.path().to_path_buf(),
            None,
            None,
            false,
            false,
            false,
            false,
            policies,
        );
        let mut envelope = sample_envelope().await;
        envelope.consent.scopes = vec![ConsentScope::ModelTraining];
        envelope.trace_card.consent_scope = ConsentScope::ModelTraining;
        envelope
            .trace_card
            .allowed_uses
            .push(TraceAllowedUse::ModelTraining);

        let error = submit_trace_handler(State(state), auth_headers("token-a"), Json(envelope))
            .await
            .expect_err("tenant policy rejects disallowed allowed use");
        assert_eq!(error.0, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn submit_requires_explicit_tenant_policy_when_configured() {
        let temp = tempfile::tempdir().expect("temp dir");
        let mut policies = BTreeMap::new();
        policies.insert(
            "tenant-a".to_string(),
            TenantSubmissionPolicy {
                allowed_consent_scopes: BTreeSet::from([ConsentScope::DebuggingEvaluation]),
                allowed_uses: BTreeSet::from([
                    TraceAllowedUse::Debugging,
                    TraceAllowedUse::Evaluation,
                    TraceAllowedUse::AggregateAnalytics,
                ]),
            },
        );
        let state = test_state_with_required_tenant_policies(temp.path().to_path_buf(), policies);
        let envelope = sample_envelope().await;

        let error = submit_trace_handler(
            State(state.clone()),
            auth_headers("token-b"),
            Json(envelope.clone()),
        )
        .await
        .expect_err("tenant without policy cannot submit when required");
        assert_eq!(error.0, StatusCode::FORBIDDEN);

        let Json(receipt) =
            submit_trace_handler(State(state), auth_headers("token-a"), Json(envelope))
                .await
                .expect("tenant with policy can submit");
        assert_eq!(receipt.status, "quarantined");
    }

    #[tokio::test]
    async fn admin_config_status_route_returns_safe_projection() {
        use axum::body::Body;
        use tower::ServiceExt;

        let temp = tempfile::tempdir().expect("temp dir");
        let artifact_store = ConfiguredTraceArtifactStore::new(
            TRACE_COMMONS_SERVICE_LOCAL_ENCRYPTED_OBJECT_STORE,
            test_artifact_store(temp.path()),
        );
        let state = test_state_with_configured_artifact_store_policies_and_export_guardrails(
            temp.path().to_path_buf(),
            None,
            Some(artifact_store),
            false,
            false,
            false,
            false,
            false,
            false,
            BTreeMap::new(),
            true,
            true,
        );

        let contributor_response = app(state.clone())
            .oneshot(
                axum::http::Request::builder()
                    .method("GET")
                    .uri("/v1/admin/config-status")
                    .header(AUTHORIZATION, "Bearer token-a")
                    .body(Body::empty())
                    .expect("request builds"),
            )
            .await
            .expect("contributor response");
        assert_eq!(contributor_response.status(), StatusCode::FORBIDDEN);

        let admin_response = app(state)
            .oneshot(
                axum::http::Request::builder()
                    .method("GET")
                    .uri("/v1/admin/config-status")
                    .header(AUTHORIZATION, "Bearer admin-token-a")
                    .body(Body::empty())
                    .expect("request builds"),
            )
            .await
            .expect("admin response");
        assert_eq!(admin_response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(admin_response.into_body(), 4096)
            .await
            .expect("body reads");
        let value: serde_json::Value = serde_json::from_slice(&body).expect("status json parses");
        assert_eq!(
            value["schema_version"],
            serde_json::json!(TRACE_CONTRIBUTION_SCHEMA_VERSION)
        );
        assert_eq!(value["db_mirror_configured"], serde_json::json!(false));
        assert_eq!(
            value["require_tenant_submission_policy"],
            serde_json::json!(true)
        );
        assert_eq!(value["require_export_guardrails"], serde_json::json!(true));
        assert_eq!(
            value["max_export_items_per_request"],
            serde_json::json!(DEFAULT_TRACE_COMMONS_MAX_EXPORT_ITEMS_PER_REQUEST)
        );
        assert_eq!(
            value["submission_quota"],
            serde_json::json!({
                "max_per_tenant_per_hour": 0,
                "max_per_principal_per_hour": 0
            })
        );
        assert_eq!(
            value["artifact_object_store"],
            serde_json::json!(TRACE_COMMONS_SERVICE_LOCAL_ENCRYPTED_OBJECT_STORE)
        );
        let object = value.as_object().expect("status response is object");
        for forbidden_key in [
            "root",
            "tokens",
            "tenant_policies",
            "artifact_store_root",
            "bearer_token",
            "principal_ref",
        ] {
            assert!(
                !object.contains_key(forbidden_key),
                "config status leaked {forbidden_key}"
            );
        }
        let body_text = std::str::from_utf8(&body).expect("body is utf8");
        assert!(!body_text.contains(temp.path().to_string_lossy().as_ref()));
        assert!(!body_text.contains("admin-token-a"));
        assert!(!body_text.contains("token-a"));

        let file_audit_events =
            read_all_audit_events(temp.path(), "tenant-a").expect("file audit events read");
        assert!(file_audit_events.iter().any(|event| {
            event.kind == "read"
                && event.reason.as_deref() == Some("surface=config_status;item_count=1")
        }));
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn submit_enforces_db_backed_tenant_policy_when_enabled() {
        use ironclaw::db::libsql::LibSqlBackend;
        use ironclaw::trace_corpus_storage::TraceTenantPolicyWrite;

        let temp = tempfile::tempdir().expect("temp dir");
        let db_temp = tempfile::tempdir().expect("db temp dir");
        let db_path = db_temp.path().join("trace-tenant-policy.db");
        let db = Arc::new(
            LibSqlBackend::new_local(&db_path)
                .await
                .expect("create libsql mirror"),
        );
        db.run_migrations().await.expect("run migrations");
        db.upsert_trace_tenant_policy(TraceTenantPolicyWrite {
            tenant_id: "tenant-a".to_string(),
            policy_version: "tenant-policy-v1".to_string(),
            allowed_consent_scopes: vec!["debugging_evaluation".to_string()],
            allowed_uses: vec!["debugging".to_string()],
            updated_by_principal_ref: "admin:test".to_string(),
        })
        .await
        .expect("tenant policy writes");
        let state = test_state_with_db_tenant_policy_reads(
            temp.path().to_path_buf(),
            db.clone() as Arc<dyn Database>,
            true,
        );

        let mut allowed = sample_envelope().await;
        make_metadata_only_low_risk(&mut allowed);
        allowed.consent.scopes = vec![ConsentScope::DebuggingEvaluation];
        allowed.trace_card.consent_scope = ConsentScope::DebuggingEvaluation;
        allowed.trace_card.allowed_uses = vec![TraceAllowedUse::Debugging];
        let Json(receipt) =
            submit_trace_handler(State(state.clone()), auth_headers("token-a"), Json(allowed))
                .await
                .expect("DB-backed tenant policy allows matching submission");
        assert_eq!(receipt.status, "accepted");

        let mut disallowed = sample_envelope().await;
        make_metadata_only_low_risk(&mut disallowed);
        disallowed.consent.scopes = vec![ConsentScope::ModelTraining];
        disallowed.trace_card.consent_scope = ConsentScope::ModelTraining;
        disallowed.trace_card.allowed_uses = vec![TraceAllowedUse::ModelTraining];
        let error = submit_trace_handler(State(state), auth_headers("token-a"), Json(disallowed))
            .await
            .expect_err("DB-backed tenant policy rejects disallowed submission");
        assert_eq!(error.0, StatusCode::FORBIDDEN);

        assert!(
            db.get_trace_tenant_policy("tenant-b")
                .await
                .expect("other tenant policy reads")
                .is_none()
        );
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn admin_can_manage_db_backed_tenant_policy() {
        use ironclaw::db::libsql::LibSqlBackend;

        let temp = tempfile::tempdir().expect("temp dir");
        let db_temp = tempfile::tempdir().expect("db temp dir");
        let db_path = db_temp.path().join("trace-tenant-policy-admin.db");
        let db = Arc::new(
            LibSqlBackend::new_local(&db_path)
                .await
                .expect("create libsql mirror"),
        );
        db.run_migrations().await.expect("run migrations");
        let state = test_state_with_db_tenant_policy_reads(
            temp.path().to_path_buf(),
            db.clone() as Arc<dyn Database>,
            true,
        );

        let contributor_error = put_tenant_policy_handler(
            State(state.clone()),
            auth_headers("token-a"),
            Json(TraceTenantPolicyRequest {
                policy_version: "tenant-policy-v1".to_string(),
                allowed_consent_scopes: vec![ConsentScope::DebuggingEvaluation],
                allowed_uses: vec![TraceAllowedUse::Debugging],
            }),
        )
        .await
        .expect_err("contributors cannot manage tenant policy");
        assert_eq!(contributor_error.0, StatusCode::FORBIDDEN);

        let Json(written) = put_tenant_policy_handler(
            State(state.clone()),
            auth_headers("admin-token-a"),
            Json(TraceTenantPolicyRequest {
                policy_version: "tenant-policy-v1".to_string(),
                allowed_consent_scopes: vec![
                    ConsentScope::DebuggingEvaluation,
                    ConsentScope::BenchmarkOnly,
                ],
                allowed_uses: vec![TraceAllowedUse::Debugging, TraceAllowedUse::Evaluation],
            }),
        )
        .await
        .expect("admin can write tenant policy");
        assert_eq!(written.tenant_id, "tenant-a");
        assert_eq!(written.policy_version, "tenant-policy-v1");
        assert_eq!(
            written.allowed_consent_scopes,
            vec!["debugging_evaluation", "benchmark_only"]
        );
        assert_eq!(written.allowed_uses, vec!["debugging", "evaluation"]);
        assert_eq!(
            written.updated_by_principal_ref,
            principal_storage_ref("admin-token-a")
        );

        let Json(read) =
            get_tenant_policy_handler(State(state.clone()), auth_headers("admin-token-a"))
                .await
                .expect("admin can read tenant policy");
        assert_eq!(read.policy_version, written.policy_version);
        assert_eq!(read.allowed_consent_scopes, written.allowed_consent_scopes);
        assert_eq!(read.allowed_uses, written.allowed_uses);

        let file_audit_events =
            read_all_audit_events(temp.path(), "tenant-a").expect("file audit events read");
        let file_policy_update = file_audit_events
            .iter()
            .find(|event| event.kind == "tenant_policy_update")
            .expect("tenant policy update writes file audit event");
        assert_eq!(
            file_policy_update.actor_principal_ref.as_deref(),
            Some(principal_storage_ref("admin-token-a").as_str())
        );
        assert!(file_policy_update.reason.as_deref().is_some_and(|reason| {
            reason.contains("policy_version=tenant-policy-v1")
                && reason.contains("allowed_consent_scope_count=2")
                && reason.contains("allowed_use_count=2")
                && reason.contains("policy_projection_hash=sha256:")
        }));
        assert!(
            file_policy_update
                .event_hash
                .as_deref()
                .is_some_and(|hash| hash.starts_with("sha256:"))
        );
        assert!(file_audit_events.iter().any(|event| {
            event.kind == "read"
                && event.reason.as_deref() == Some("surface=tenant_policy;item_count=1")
        }));

        let db_audit_events = db
            .list_trace_audit_events("tenant-a")
            .await
            .expect("tenant policy audit events read");
        let db_policy_update = db_audit_events
            .iter()
            .find(|event| event.action == StorageTraceAuditAction::PolicyUpdate)
            .expect("tenant policy update mirrors DB audit event");
        match &db_policy_update.metadata {
            StorageTraceAuditSafeMetadata::TenantPolicy {
                policy_version,
                allowed_consent_scope_count,
                allowed_use_count,
                policy_projection_hash,
            } => {
                assert_eq!(policy_version, "tenant-policy-v1");
                assert_eq!(*allowed_consent_scope_count, 2);
                assert_eq!(*allowed_use_count, 2);
                assert!(policy_projection_hash.starts_with("sha256:"));
                assert_eq!(
                    db_policy_update.decision_inputs_hash.as_ref(),
                    Some(policy_projection_hash)
                );
            }
            metadata => panic!("unexpected tenant policy audit metadata: {metadata:?}"),
        }
        assert!(db_audit_events.iter().any(|event| {
            event.action == StorageTraceAuditAction::Read
                && event.reason.as_deref() == Some("surface=tenant_policy;item_count=1")
        }));

        let other_tenant_error =
            get_tenant_policy_handler(State(state), auth_headers("admin-token-b"))
                .await
                .expect_err("admin only reads own tenant policy");
        assert_eq!(other_tenant_error.0, StatusCode::NOT_FOUND);

        assert!(
            db.get_trace_tenant_policy("tenant-b")
                .await
                .expect("other tenant policy reads")
                .is_none()
        );
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn db_backed_tenant_policy_controls_export_surfaces() {
        use ironclaw::db::libsql::LibSqlBackend;

        let temp = tempfile::tempdir().expect("temp dir");
        let db_temp = tempfile::tempdir().expect("db temp dir");
        let db_path = db_temp.path().join("trace-export-policy-abac.db");
        let db = Arc::new(
            LibSqlBackend::new_local(&db_path)
                .await
                .expect("create libsql mirror"),
        );
        db.run_migrations().await.expect("run migrations");
        let state = test_state_with_db_tenant_policy_reads(
            temp.path().to_path_buf(),
            db.clone() as Arc<dyn Database>,
            true,
        );

        let Json(policy) = put_tenant_policy_handler(
            State(state.clone()),
            auth_headers("admin-token-a"),
            Json(TraceTenantPolicyRequest {
                policy_version: "export-policy-v1".to_string(),
                allowed_consent_scopes: vec![ConsentScope::BenchmarkOnly],
                allowed_uses: vec![TraceAllowedUse::BenchmarkGeneration],
            }),
        )
        .await
        .expect("admin can write benchmark-only export policy");
        assert_eq!(policy.allowed_uses, vec!["benchmark_generation"]);

        let mut source = sample_envelope().await;
        make_metadata_only_low_risk(&mut source);
        source.consent.scopes = vec![ConsentScope::BenchmarkOnly];
        source.trace_card.consent_scope = ConsentScope::BenchmarkOnly;
        source.trace_card.allowed_uses = vec![TraceAllowedUse::BenchmarkGeneration];
        let submission_id = source.submission_id;
        let Json(receipt) =
            submit_trace_handler(State(state.clone()), auth_headers("token-a"), Json(source))
                .await
                .expect("benchmark-only source can submit under tenant policy");
        assert_eq!(receipt.status, "accepted");

        let Json(benchmark) = benchmark_convert_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            Json(BenchmarkConversionRequest {
                limit: Some(10),
                purpose: Some("abac_benchmark_allowed".to_string()),
                consent_scope: Some("benchmark-only".to_string()),
                status: Some(TraceCorpusStatus::Accepted),
                privacy_risk: Some(ResidualPiiRisk::Low),
                external_ref: None,
            }),
        )
        .await
        .expect("benchmark export is allowed by tenant policy");
        assert_eq!(benchmark.item_count, 1);
        assert_eq!(benchmark.source_submission_ids, vec![submission_id]);

        let replay_error = dataset_replay_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            Query(DatasetExportQuery {
                limit: Some(10),
                purpose: Some("abac_replay_denied".to_string()),
                status: Some(TraceCorpusStatus::Accepted),
                privacy_risk: Some(ResidualPiiRisk::Low),
                consent_scope: Some("benchmark-only".to_string()),
            }),
        )
        .await
        .expect_err("replay export requires evaluation use");
        assert_eq!(replay_error.0, StatusCode::FORBIDDEN);

        let candidates_error = ranker_training_candidates_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            Query(RankerTrainingExportQuery {
                limit: Some(10),
                purpose: Some("abac_ranker_candidates_denied".to_string()),
                status: Some(TraceCorpusStatus::Accepted),
                consent_scope: Some("ranking-training".to_string()),
                privacy_risk: Some(ResidualPiiRisk::Low),
            }),
        )
        .await
        .expect_err("ranker candidates require ranking use and consent");
        assert_eq!(candidates_error.0, StatusCode::FORBIDDEN);

        let pairs_error = ranker_training_pairs_handler(
            State(state),
            auth_headers("review-token-a"),
            Query(RankerTrainingExportQuery {
                limit: Some(10),
                purpose: Some("abac_ranker_pairs_denied".to_string()),
                status: Some(TraceCorpusStatus::Accepted),
                consent_scope: Some("ranking-training".to_string()),
                privacy_risk: Some(ResidualPiiRisk::Low),
            }),
        )
        .await
        .expect_err("ranker pairs require ranking use and consent");
        assert_eq!(pairs_error.0, StatusCode::FORBIDDEN);
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn export_policy_filters_sources_without_required_allowed_use() {
        use ironclaw::db::libsql::LibSqlBackend;

        let temp = tempfile::tempdir().expect("temp dir");
        let db_temp = tempfile::tempdir().expect("db temp dir");
        let db_path = db_temp.path().join("trace-export-source-policy-abac.db");
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
        let mut source = sample_envelope().await;
        make_metadata_only_low_risk(&mut source);
        source.consent.scopes = vec![ConsentScope::RankingTraining];
        source.trace_card.consent_scope = ConsentScope::RankingTraining;
        source.trace_card.allowed_uses = vec![TraceAllowedUse::Evaluation];
        let submission_id = source.submission_id;
        let _ = submit_trace_handler(State(submit_state), auth_headers("token-a"), Json(source))
            .await
            .expect("pre-policy source can submit");

        db.upsert_trace_tenant_policy(StorageTraceTenantPolicyWrite {
            tenant_id: "tenant-a".to_string(),
            policy_version: "ranking-export-policy-v1".to_string(),
            allowed_consent_scopes: vec!["ranking_training".to_string()],
            allowed_uses: vec!["ranking_model_training".to_string()],
            updated_by_principal_ref: principal_storage_ref("admin-token-a"),
        })
        .await
        .expect("ranking tenant policy writes");
        let export_state = test_state_with_db_tenant_policy_reads(
            temp.path().to_path_buf(),
            db.clone() as Arc<dyn Database>,
            true,
        );

        let Json(candidates) = ranker_training_candidates_handler(
            State(export_state.clone()),
            auth_headers("review-token-a"),
            Query(RankerTrainingExportQuery {
                limit: Some(10),
                purpose: Some("source_abac_ranker_candidates".to_string()),
                status: Some(TraceCorpusStatus::Accepted),
                consent_scope: Some("ranking-training".to_string()),
                privacy_risk: Some(ResidualPiiRisk::Low),
            }),
        )
        .await
        .expect("ranker export request is policy-allowed");
        assert_eq!(candidates.item_count, 0);
        assert!(
            candidates
                .candidates
                .iter()
                .all(|candidate| candidate.submission_id != submission_id)
        );

        let Json(pairs) = ranker_training_pairs_handler(
            State(export_state),
            auth_headers("review-token-a"),
            Query(RankerTrainingExportQuery {
                limit: Some(10),
                purpose: Some("source_abac_ranker_pairs".to_string()),
                status: Some(TraceCorpusStatus::Accepted),
                consent_scope: Some("ranking-training".to_string()),
                privacy_risk: Some(ResidualPiiRisk::Low),
            }),
        )
        .await
        .expect("ranker pair request is policy-allowed");
        assert_eq!(pairs.item_count, 0);
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn required_tenant_policy_blocks_exports_without_policy_row() {
        use ironclaw::db::libsql::LibSqlBackend;

        let temp = tempfile::tempdir().expect("temp dir");
        let db_temp = tempfile::tempdir().expect("db temp dir");
        let db_path = db_temp.path().join("trace-export-missing-policy.db");
        let db = Arc::new(
            LibSqlBackend::new_local(&db_path)
                .await
                .expect("create libsql mirror"),
        );
        db.run_migrations().await.expect("run migrations");
        let state = test_state_with_db_tenant_policy_reads(
            temp.path().to_path_buf(),
            db.clone() as Arc<dyn Database>,
            true,
        );

        let replay_error = dataset_replay_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            Query(DatasetExportQuery {
                limit: Some(10),
                purpose: Some("missing_policy_replay".to_string()),
                status: Some(TraceCorpusStatus::Accepted),
                privacy_risk: Some(ResidualPiiRisk::Low),
                consent_scope: Some("debugging-evaluation".to_string()),
            }),
        )
        .await
        .expect_err("required tenant policy blocks replay export");
        assert_eq!(replay_error.0, StatusCode::FORBIDDEN);

        let benchmark_error = benchmark_convert_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            Json(BenchmarkConversionRequest {
                limit: Some(10),
                purpose: Some("missing_policy_benchmark".to_string()),
                consent_scope: Some("benchmark-only".to_string()),
                status: Some(TraceCorpusStatus::Accepted),
                privacy_risk: Some(ResidualPiiRisk::Low),
                external_ref: None,
            }),
        )
        .await
        .expect_err("required tenant policy blocks benchmark export");
        assert_eq!(benchmark_error.0, StatusCode::FORBIDDEN);

        let candidates_error = ranker_training_candidates_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            Query(RankerTrainingExportQuery {
                limit: Some(10),
                purpose: Some("missing_policy_ranker_candidates".to_string()),
                status: Some(TraceCorpusStatus::Accepted),
                consent_scope: Some("ranking-training".to_string()),
                privacy_risk: Some(ResidualPiiRisk::Low),
            }),
        )
        .await
        .expect_err("required tenant policy blocks ranker candidate export");
        assert_eq!(candidates_error.0, StatusCode::FORBIDDEN);

        let pairs_error = ranker_training_pairs_handler(
            State(state),
            auth_headers("review-token-a"),
            Query(RankerTrainingExportQuery {
                limit: Some(10),
                purpose: Some("missing_policy_ranker_pairs".to_string()),
                status: Some(TraceCorpusStatus::Accepted),
                consent_scope: Some("ranking-training".to_string()),
                privacy_risk: Some(ResidualPiiRisk::Low),
            }),
        )
        .await
        .expect_err("required tenant policy blocks ranker pair export");
        assert_eq!(pairs_error.0, StatusCode::FORBIDDEN);

        assert!(
            db.list_trace_export_manifests("tenant-a")
                .await
                .expect("export manifests read")
                .is_empty()
        );
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn worker_roles_are_scoped_to_trace_job_surfaces() {
        use ironclaw::db::libsql::LibSqlBackend;

        let temp = tempfile::tempdir().expect("temp dir");
        let db_temp = tempfile::tempdir().expect("db temp dir");
        let db_path = db_temp.path().join("trace-worker-role-scope.db");
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
        envelope.consent.scopes = vec![ConsentScope::RankingTraining];
        envelope.trace_card.consent_scope = ConsentScope::RankingTraining;
        let submission_id = envelope.submission_id;
        let _ = submit_trace_handler(
            State(state.clone()),
            auth_headers("token-a"),
            Json(envelope),
        )
        .await
        .expect("training-consented submission succeeds");

        let Json(replay_export) = dataset_replay_handler(
            State(state.clone()),
            auth_headers("export-worker-token-a"),
            Query(DatasetExportQuery {
                limit: Some(10),
                purpose: Some("worker_replay".to_string()),
                status: Some(TraceCorpusStatus::Accepted),
                privacy_risk: Some(ResidualPiiRisk::Low),
                consent_scope: None,
            }),
        )
        .await
        .expect("export worker can build replay dataset");
        assert_eq!(replay_export.item_count, 1);

        let Json(manifests) = replay_export_manifests_handler(
            State(state.clone()),
            auth_headers("export-worker-token-a"),
        )
        .await
        .expect("export worker can list replay manifests");
        assert_eq!(manifests.len(), 1);

        let Json(ranker_candidates) = ranker_training_candidates_handler(
            State(state.clone()),
            auth_headers("export-worker-token-a"),
            Query(RankerTrainingExportQuery {
                limit: Some(10),
                purpose: Some("worker_ranker_candidates".to_string()),
                status: Some(TraceCorpusStatus::Accepted),
                consent_scope: Some("ranking_training".to_string()),
                privacy_risk: Some(ResidualPiiRisk::Low),
            }),
        )
        .await
        .expect("export worker can build ranker candidates");
        assert_eq!(ranker_candidates.item_count, 1);

        let utility_export_error = dataset_replay_handler(
            State(state.clone()),
            auth_headers("utility-worker-token-a"),
            Query(DatasetExportQuery {
                limit: Some(10),
                purpose: Some("utility_worker_replay_denied".to_string()),
                status: Some(TraceCorpusStatus::Accepted),
                privacy_risk: Some(ResidualPiiRisk::Low),
                consent_scope: None,
            }),
        )
        .await
        .expect_err("utility worker cannot build replay exports");
        assert_eq!(utility_export_error.0, StatusCode::FORBIDDEN);

        let export_utility_error = utility_credit_handler(
            State(state.clone()),
            auth_headers("export-worker-token-a"),
            Json(TraceUtilityCreditJobRequest {
                event_type: TraceCreditLedgerEventType::RegressionCatch,
                credit_points_delta: 2.0,
                reason: "export workers cannot mutate utility credit".to_string(),
                external_ref: "regression-job:export-worker-denied".to_string(),
                submission_ids: vec![submission_id],
            }),
        )
        .await
        .expect_err("export worker cannot use utility credit route");
        assert_eq!(export_utility_error.0, StatusCode::FORBIDDEN);

        let Json(utility_credit) = utility_credit_handler(
            State(state.clone()),
            auth_headers("utility-worker-token-a"),
            Json(TraceUtilityCreditJobRequest {
                event_type: TraceCreditLedgerEventType::RegressionCatch,
                credit_points_delta: 2.0,
                reason: "utility worker regression catch".to_string(),
                external_ref: "regression-job:worker-scope".to_string(),
                submission_ids: vec![submission_id],
            }),
        )
        .await
        .expect("utility worker can append regression utility credit");
        assert_eq!(utility_credit.appended_count, 1);

        let export_review_error = review_decision_handler(
            State(state.clone()),
            auth_headers("export-worker-token-a"),
            AxumPath(submission_id),
            Json(TraceReviewDecisionRequest {
                decision: TraceReviewDecision::Reject,
                reason: Some("export workers cannot review".to_string()),
                credit_points_pending: None,
            }),
        )
        .await
        .expect_err("export worker cannot make review decisions");
        assert_eq!(export_review_error.0, StatusCode::FORBIDDEN);

        let utility_review_error = review_decision_handler(
            State(state.clone()),
            auth_headers("utility-worker-token-a"),
            AxumPath(submission_id),
            Json(TraceReviewDecisionRequest {
                decision: TraceReviewDecision::Reject,
                reason: Some("utility workers cannot review".to_string()),
                credit_points_pending: None,
            }),
        )
        .await
        .expect_err("utility worker cannot make review decisions");
        assert_eq!(utility_review_error.0, StatusCode::FORBIDDEN);

        let export_audit_error = audit_events_handler(
            State(state.clone()),
            auth_headers("export-worker-token-a"),
            Query(AuditEventsQuery { limit: Some(10) }),
        )
        .await
        .expect_err("export worker cannot read audit events");
        assert_eq!(export_audit_error.0, StatusCode::FORBIDDEN);

        let export_policy_error = put_tenant_policy_handler(
            State(state.clone()),
            auth_headers("export-worker-token-a"),
            Json(TraceTenantPolicyRequest {
                policy_version: "worker-policy-v1".to_string(),
                allowed_consent_scopes: vec![ConsentScope::RankingTraining],
                allowed_uses: vec![TraceAllowedUse::RankingModelTraining],
            }),
        )
        .await
        .expect_err("export worker cannot manage tenant policies");
        assert_eq!(export_policy_error.0, StatusCode::FORBIDDEN);

        let Json(benchmark) = benchmark_convert_handler(
            State(state.clone()),
            auth_headers("benchmark-worker-token-a"),
            Json(BenchmarkConversionRequest {
                limit: Some(10),
                purpose: Some("benchmark_worker_conversion".to_string()),
                consent_scope: None,
                status: Some(TraceCorpusStatus::Accepted),
                privacy_risk: Some(ResidualPiiRisk::Low),
                external_ref: Some("benchmark-job:worker-scope".to_string()),
            }),
        )
        .await
        .expect("benchmark worker can convert benchmark artifacts");
        assert_eq!(benchmark.item_count, 1);

        let export_benchmark_route_error = benchmark_worker_convert_handler(
            State(state.clone()),
            auth_headers("export-worker-token-a"),
            Json(BenchmarkConversionRequest {
                limit: Some(10),
                purpose: Some("export_worker_benchmark_route_denied".to_string()),
                consent_scope: None,
                status: Some(TraceCorpusStatus::Accepted),
                privacy_risk: Some(ResidualPiiRisk::Low),
                external_ref: Some("benchmark-job:export-worker-denied".to_string()),
            }),
        )
        .await
        .expect_err("export worker cannot use dedicated benchmark route");
        assert_eq!(export_benchmark_route_error.0, StatusCode::FORBIDDEN);

        let Json(dedicated_benchmark) = benchmark_worker_convert_handler(
            State(state.clone()),
            auth_headers("benchmark-worker-token-a"),
            Json(BenchmarkConversionRequest {
                limit: Some(10),
                purpose: Some("benchmark_worker_dedicated_conversion".to_string()),
                consent_scope: None,
                status: Some(TraceCorpusStatus::Accepted),
                privacy_risk: Some(ResidualPiiRisk::Low),
                external_ref: Some("benchmark-job:worker-dedicated-scope".to_string()),
            }),
        )
        .await
        .expect("benchmark worker can use dedicated benchmark route");
        assert_eq!(dedicated_benchmark.item_count, 1);

        let benchmark_export_error = dataset_replay_handler(
            State(state.clone()),
            auth_headers("benchmark-worker-token-a"),
            Query(DatasetExportQuery {
                limit: Some(10),
                purpose: Some("benchmark_worker_replay_denied".to_string()),
                status: Some(TraceCorpusStatus::Accepted),
                privacy_risk: Some(ResidualPiiRisk::Low),
                consent_scope: None,
            }),
        )
        .await
        .expect_err("benchmark worker cannot build replay exports");
        assert_eq!(benchmark_export_error.0, StatusCode::FORBIDDEN);

        let Json(retention_dry_run) = maintenance_handler(
            State(state.clone()),
            auth_headers("retention-worker-token-a"),
            Json(TraceMaintenanceRequest {
                purpose: Some("retention_worker_dry_run".to_string()),
                dry_run: true,
                backfill_db_mirror: false,
                index_vectors: false,
                reconcile_db_mirror: false,
                verify_audit_chain: false,
                prune_export_cache: false,
                max_export_age_hours: None,
                purge_expired_before: None,
            }),
        )
        .await
        .expect("retention worker can run retention-scoped maintenance");
        assert_eq!(retention_dry_run.vector_entries_indexed, 0);

        let vector_retention_route_error = retention_maintenance_handler(
            State(state.clone()),
            auth_headers("vector-worker-token-a"),
            Json(TraceRetentionMaintenanceRequest {
                purpose: Some("vector_worker_retention_route_denied".to_string()),
                dry_run: true,
                prune_export_cache: false,
                max_export_age_hours: None,
                purge_expired_before: None,
            }),
        )
        .await
        .expect_err("vector worker cannot use dedicated retention route");
        assert_eq!(vector_retention_route_error.0, StatusCode::FORBIDDEN);

        let Json(retention_route_dry_run) = retention_maintenance_handler(
            State(state.clone()),
            auth_headers("retention-worker-token-a"),
            Json(TraceRetentionMaintenanceRequest {
                purpose: Some("retention_worker_route_dry_run".to_string()),
                dry_run: true,
                prune_export_cache: false,
                max_export_age_hours: None,
                purge_expired_before: None,
            }),
        )
        .await
        .expect("retention worker can run dedicated retention route");
        assert_eq!(retention_route_dry_run.vector_entries_indexed, 0);

        let retention_vector_error = maintenance_handler(
            State(state.clone()),
            auth_headers("retention-worker-token-a"),
            Json(TraceMaintenanceRequest {
                purpose: Some("retention_worker_vector_denied".to_string()),
                dry_run: true,
                backfill_db_mirror: false,
                index_vectors: true,
                reconcile_db_mirror: false,
                verify_audit_chain: false,
                prune_export_cache: false,
                max_export_age_hours: None,
                purge_expired_before: None,
            }),
        )
        .await
        .expect_err("retention worker cannot index vectors");
        assert_eq!(retention_vector_error.0, StatusCode::FORBIDDEN);

        let retention_vector_route_error = vector_index_handler(
            State(state.clone()),
            auth_headers("retention-worker-token-a"),
            Json(TraceVectorIndexRequest {
                purpose: Some("retention_worker_vector_route_denied".to_string()),
                dry_run: true,
            }),
        )
        .await
        .expect_err("retention worker cannot use dedicated vector route");
        assert_eq!(retention_vector_route_error.0, StatusCode::FORBIDDEN);

        let Json(vector_route_index) = vector_index_handler(
            State(state.clone()),
            auth_headers("vector-worker-token-a"),
            Json(TraceVectorIndexRequest {
                purpose: Some("vector_worker_route_index".to_string()),
                dry_run: false,
            }),
        )
        .await
        .expect("vector worker can run dedicated vector index route");
        assert_eq!(vector_route_index.vector_entries_indexed, 1);

        let Json(vector_index) = maintenance_handler(
            State(state.clone()),
            auth_headers("vector-worker-token-a"),
            Json(TraceMaintenanceRequest {
                purpose: Some("vector_worker_index".to_string()),
                dry_run: false,
                backfill_db_mirror: false,
                index_vectors: true,
                reconcile_db_mirror: false,
                verify_audit_chain: false,
                prune_export_cache: false,
                max_export_age_hours: None,
                purge_expired_before: None,
            }),
        )
        .await
        .expect("vector worker can run vector-scoped maintenance");
        assert_eq!(vector_index.vector_entries_indexed, 0);

        let vector_retention_error = maintenance_handler(
            State(state),
            auth_headers("vector-worker-token-a"),
            Json(TraceMaintenanceRequest {
                purpose: Some("vector_worker_retention_denied".to_string()),
                dry_run: true,
                backfill_db_mirror: false,
                index_vectors: true,
                reconcile_db_mirror: false,
                verify_audit_chain: false,
                prune_export_cache: true,
                max_export_age_hours: None,
                purge_expired_before: None,
            }),
        )
        .await
        .expect_err("vector worker cannot run retention cleanup");
        assert_eq!(vector_retention_error.0, StatusCode::FORBIDDEN);
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
    async fn object_primary_submit_writes_no_plaintext_envelope_body() {
        use ironclaw::db::libsql::LibSqlBackend;

        let temp = tempfile::tempdir().expect("temp dir");
        let artifact_root = temp.path().join("service-object-store");
        let artifact_store = test_artifact_store(&artifact_root);
        let configured_store = ConfiguredTraceArtifactStore::new(
            TRACE_COMMONS_SERVICE_LOCAL_ENCRYPTED_OBJECT_STORE,
            artifact_store,
        );
        let db_temp = tempfile::tempdir().expect("db temp dir");
        let db_path = db_temp.path().join("trace-object-primary-submit.db");
        let db = Arc::new(
            LibSqlBackend::new_local(&db_path)
                .await
                .expect("create libsql mirror"),
        );
        db.run_migrations().await.expect("run migrations");
        let state = test_state_with_object_primary_submit_review(
            temp.path().to_path_buf(),
            db.clone() as Arc<dyn Database>,
            configured_store,
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
        .expect("object-primary submission succeeds");

        assert_eq!(receipt.status, "accepted");
        let record = read_submission_record(temp.path(), "tenant-a", submission_id)
            .expect("record reads")
            .expect("record exists");
        assert!(
            !temp.path().join(&record.object_key).exists(),
            "object-primary mode should not leave a plaintext envelope body"
        );
        let artifact_receipt = record
            .artifact_receipt
            .as_ref()
            .expect("artifact receipt is persisted in file metadata");
        let metadata_round_trip =
            read_envelope_by_record(state.as_ref(), &record).expect("metadata receipt reads");
        assert_eq!(metadata_round_trip.submission_id, submission_id);

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
            TRACE_COMMONS_SERVICE_LOCAL_ENCRYPTED_OBJECT_STORE
        );
        assert_eq!(object_ref.object_key, artifact_receipt.object_key);
        let object_ref_round_trip =
            read_envelope_from_object_ref(state.as_ref(), "tenant-a", &object_ref)
                .expect("DB object ref reads encrypted envelope");
        assert_eq!(object_ref_round_trip.submission_id, submission_id);
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn object_primary_replay_export_reads_object_ref_without_plaintext_body() {
        use ironclaw::db::libsql::LibSqlBackend;

        let temp = tempfile::tempdir().expect("temp dir");
        let artifact_root = temp.path().join("replay-service-object-store");
        let artifact_store = test_artifact_store(&artifact_root);
        let configured_store = ConfiguredTraceArtifactStore::new(
            TRACE_COMMONS_SERVICE_LOCAL_ENCRYPTED_OBJECT_STORE,
            artifact_store,
        );
        let db_temp = tempfile::tempdir().expect("db temp dir");
        let db_path = db_temp.path().join("trace-object-primary-replay.db");
        let db = Arc::new(
            LibSqlBackend::new_local(&db_path)
                .await
                .expect("create libsql mirror"),
        );
        db.run_migrations().await.expect("run migrations");
        let state = test_state_with_object_primary_submit_review_and_replay_export(
            temp.path().to_path_buf(),
            db.clone() as Arc<dyn Database>,
            configured_store,
        );
        let mut envelope = sample_envelope().await;
        make_metadata_only_low_risk(&mut envelope);
        envelope.replay.replayable = true;
        envelope.replay.required_tools.push("shell".to_string());
        let submission_id = envelope.submission_id;

        let Json(receipt) = submit_trace_handler(
            State(state.clone()),
            auth_headers("token-a"),
            Json(envelope),
        )
        .await
        .expect("object-primary replay source submits");
        assert_eq!(receipt.status, "accepted");
        let record = read_submission_record(temp.path(), "tenant-a", submission_id)
            .expect("record reads")
            .expect("record exists");
        assert!(
            !temp.path().join(&record.object_key).exists(),
            "object-primary replay source should not write a plaintext envelope body"
        );
        let submitted_ref = db
            .get_latest_active_trace_object_ref(
                "tenant-a",
                submission_id,
                StorageTraceObjectArtifactKind::SubmittedEnvelope,
            )
            .await
            .expect("submitted object ref reads")
            .expect("submitted object ref exists");

        let Json(export) = dataset_replay_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            Query(DatasetExportQuery {
                limit: Some(10),
                purpose: Some("object_primary_replay_export".to_string()),
                status: Some(TraceCorpusStatus::Accepted),
                privacy_risk: Some(ResidualPiiRisk::Low),
                consent_scope: None,
            }),
        )
        .await
        .expect("replay export reads service object store through DB object ref");
        assert_eq!(export.item_count, 1);
        assert_eq!(export.items[0].submission_id, submission_id);
        assert_eq!(export.items[0].required_tools, vec!["shell"]);
        assert_eq!(
            export.items[0].object_ref_id,
            Some(submitted_ref.object_ref_id)
        );

        let db_audit_events = db
            .list_trace_audit_events("tenant-a")
            .await
            .expect("DB audit events read");
        assert!(db_audit_events.iter().any(|event| {
            event.action == StorageTraceAuditAction::Read
                && event.submission_id == Some(submission_id)
                && event.object_ref_id == Some(submitted_ref.object_ref_id)
                && event
                    .reason
                    .as_deref()
                    .is_some_and(|reason| reason.contains("surface=replay_dataset_export"))
        }));

        let invalidated = db
            .invalidate_trace_submission_artifacts(
                "tenant-a",
                submission_id,
                StorageTraceDerivedStatus::Current,
            )
            .await
            .expect("invalidate submitted object ref");
        assert_eq!(invalidated.object_refs_invalidated, 1);
        let error = dataset_replay_handler(
            State(state),
            auth_headers("review-token-a"),
            Query(DatasetExportQuery {
                limit: Some(10),
                purpose: Some("object_primary_replay_export_missing_ref".to_string()),
                status: Some(TraceCorpusStatus::Accepted),
                privacy_risk: Some(ResidualPiiRisk::Low),
                consent_scope: None,
            }),
        )
        .await
        .expect_err("object-primary replay export fails closed without active object ref");
        assert_eq!(error.0, StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn object_primary_review_writes_review_snapshot_without_plaintext_body() {
        use ironclaw::db::libsql::LibSqlBackend;

        let temp = tempfile::tempdir().expect("temp dir");
        let artifact_root = temp.path().join("review-service-object-store");
        let artifact_store = test_artifact_store(&artifact_root);
        let configured_store = ConfiguredTraceArtifactStore::new(
            TRACE_COMMONS_SERVICE_LOCAL_ENCRYPTED_OBJECT_STORE,
            artifact_store,
        );
        let db_temp = tempfile::tempdir().expect("db temp dir");
        let db_path = db_temp.path().join("trace-object-primary-review.db");
        let db = Arc::new(
            LibSqlBackend::new_local(&db_path)
                .await
                .expect("create libsql mirror"),
        );
        db.run_migrations().await.expect("run migrations");
        let state = test_state_with_object_primary_submit_review(
            temp.path().to_path_buf(),
            db.clone() as Arc<dyn Database>,
            configured_store,
        );
        let envelope = sample_envelope().await;
        let submission_id = envelope.submission_id;

        let Json(submit_receipt) = submit_trace_handler(
            State(state.clone()),
            auth_headers("token-a"),
            Json(envelope),
        )
        .await
        .expect("quarantined submission succeeds");
        assert_eq!(submit_receipt.status, "quarantined");
        let submitted_record = read_submission_record(temp.path(), "tenant-a", submission_id)
            .expect("submitted record reads")
            .expect("submitted record exists");
        assert!(
            !temp.path().join(&submitted_record.object_key).exists(),
            "submitted plaintext body should not exist"
        );
        let submitted_ref = db
            .get_latest_active_trace_object_ref(
                "tenant-a",
                submission_id,
                StorageTraceObjectArtifactKind::SubmittedEnvelope,
            )
            .await
            .expect("submitted object ref reads")
            .expect("submitted object ref exists");

        let Json(review_receipt) = review_decision_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            AxumPath(submission_id),
            Json(TraceReviewDecisionRequest {
                decision: TraceReviewDecision::Approve,
                reason: Some("object-primary reviewer approval".to_string()),
                credit_points_pending: Some(1.75),
            }),
        )
        .await
        .expect("review decision reads body through object ref");
        assert_eq!(review_receipt.status, "accepted");

        let reviewed_record = read_submission_record(temp.path(), "tenant-a", submission_id)
            .expect("reviewed record reads")
            .expect("reviewed record exists");
        assert_eq!(reviewed_record.status, TraceCorpusStatus::Accepted);
        assert!(
            !temp.path().join(&reviewed_record.object_key).exists(),
            "reviewed plaintext body should not exist"
        );
        let reviewed_ref = db
            .get_latest_active_trace_object_ref(
                "tenant-a",
                submission_id,
                StorageTraceObjectArtifactKind::ReviewSnapshot,
            )
            .await
            .expect("review snapshot object ref reads")
            .expect("review snapshot object ref exists");
        assert_eq!(
            reviewed_ref.object_store,
            TRACE_COMMONS_SERVICE_LOCAL_ENCRYPTED_OBJECT_STORE
        );
        let reviewed_envelope =
            read_envelope_from_object_ref(state.as_ref(), "tenant-a", &reviewed_ref)
                .expect("review snapshot reads through object ref");
        assert_eq!(reviewed_envelope.value.credit_points_pending, 1.75);

        let db_audit_events = db
            .list_trace_audit_events("tenant-a")
            .await
            .expect("DB audit events read");
        assert!(db_audit_events.iter().any(|event| {
            event.action == StorageTraceAuditAction::Read
                && event.submission_id == Some(submission_id)
                && event.object_ref_id == Some(submitted_ref.object_ref_id)
                && event.reason.as_deref() == Some("surface=review_decision")
        }));
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
        envelope.consent.scopes = vec![ConsentScope::RankingTraining];
        envelope.trace_card.consent_scope = ConsentScope::RankingTraining;
        envelope.trace_card.allowed_uses = vec![TraceAllowedUse::RankingModelTraining];
        let expected_allowed_uses = vec!["ranking_model_training".to_string()];
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
        assert_eq!(mirrored.allowed_uses, expected_allowed_uses);
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
        let object_refs = db
            .list_trace_object_refs("tenant-a", submission_id)
            .await
            .expect("object refs read after revoke");
        assert!(
            object_refs.len() >= 2,
            "submit and review should both have mirrored object refs"
        );
        assert!(
            object_refs
                .iter()
                .all(|record| record.invalidated_at.is_some())
        );
        let conn = db.connect().await.expect("connect to mirror after revoke");
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
    async fn required_db_mirror_writes_fail_closed_on_submission_mirror_failure() {
        use ironclaw::db::libsql::LibSqlBackend;

        let temp = tempfile::tempdir().expect("temp dir");
        let db_temp = tempfile::tempdir().expect("db temp dir");
        let db_path = db_temp.path().join("trace-required-submit-mirror.db");
        let db = Arc::new(
            LibSqlBackend::new_local(&db_path)
                .await
                .expect("create libsql mirror"),
        );
        db.run_migrations().await.expect("run migrations");
        let state = test_state_with_required_db_mirror_writes(
            temp.path().to_path_buf(),
            Some(db.clone() as Arc<dyn Database>),
        );
        let conn = db.connect().await.expect("connect to mirror");
        conn.execute("DROP TABLE trace_submissions", ())
            .await
            .expect("drop mirrored submissions table");
        let mut envelope = sample_envelope().await;
        make_metadata_only_low_risk(&mut envelope);

        let error = submit_trace_handler(State(state), auth_headers("token-a"), Json(envelope))
            .await
            .expect_err("required DB mirror writes fail closed when submit mirror fails");
        assert_eq!(error.0, StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn required_db_mirror_writes_fail_closed_on_delayed_credit_mirror_failure() {
        use ironclaw::db::libsql::LibSqlBackend;

        let temp = tempfile::tempdir().expect("temp dir");
        let db_temp = tempfile::tempdir().expect("db temp dir");
        let db_path = db_temp.path().join("trace-required-credit-mirror.db");
        let db = Arc::new(
            LibSqlBackend::new_local(&db_path)
                .await
                .expect("create libsql mirror"),
        );
        db.run_migrations().await.expect("run migrations");
        let state = test_state_with_required_db_mirror_writes(
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
        .expect("submission succeeds before mirror table is removed");
        let conn = db.connect().await.expect("connect to mirror");
        conn.execute("DROP TABLE trace_credit_ledger", ())
            .await
            .expect("drop mirrored credit table");

        let error = append_credit_event_handler(
            State(state),
            auth_headers("review-token-a"),
            AxumPath(submission_id),
            Json(TraceCreditLedgerAppendRequest {
                event_type: TraceCreditLedgerEventType::ReviewerBonus,
                credit_points_delta: 2.0,
                reason: Some("required DB credit mirror".to_string()),
                external_ref: None,
            }),
        )
        .await
        .expect_err("required DB mirror writes fail closed when credit mirror fails");
        assert_eq!(error.0, StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn required_db_mirror_writes_fail_closed_on_export_provenance_mirror_failure() {
        use ironclaw::db::libsql::LibSqlBackend;

        let temp = tempfile::tempdir().expect("temp dir");
        let db_temp = tempfile::tempdir().expect("db temp dir");
        let db_path = db_temp.path().join("trace-required-export-mirror.db");
        let db = Arc::new(
            LibSqlBackend::new_local(&db_path)
                .await
                .expect("create libsql mirror"),
        );
        db.run_migrations().await.expect("run migrations");
        let state = test_state_with_required_db_mirror_writes(
            temp.path().to_path_buf(),
            Some(db.clone() as Arc<dyn Database>),
        );
        for score in [0.9_f32, 0.1_f32] {
            let mut envelope = sample_envelope().await;
            make_metadata_only_low_risk(&mut envelope);
            envelope.consent.scopes = vec![ConsentScope::RankingTraining];
            envelope.trace_card.consent_scope = ConsentScope::RankingTraining;
            envelope.value.submission_score = score;
            let _ = submit_trace_handler(
                State(state.clone()),
                auth_headers("token-a"),
                Json(envelope),
            )
            .await
            .expect("ranker source submission succeeds before mirror table is removed");
        }
        let conn = db.connect().await.expect("connect to mirror");
        conn.execute("DROP TABLE trace_export_manifests", ())
            .await
            .expect("drop mirrored export manifest table");

        let benchmark_error = benchmark_convert_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            Json(BenchmarkConversionRequest {
                limit: Some(10),
                purpose: Some("required_db_benchmark".to_string()),
                consent_scope: None,
                status: None,
                privacy_risk: None,
                external_ref: None,
            }),
        )
        .await
        .expect_err("required DB mirror writes fail closed when benchmark provenance fails");
        assert_eq!(benchmark_error.0, StatusCode::INTERNAL_SERVER_ERROR);

        let candidates_error = ranker_training_candidates_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            Query(RankerTrainingExportQuery {
                limit: Some(10),
                purpose: Some("required_db_ranker_candidates".to_string()),
                status: None,
                consent_scope: Some("ranking_training".to_string()),
                privacy_risk: None,
            }),
        )
        .await
        .expect_err("required DB mirror writes fail closed when ranker candidate provenance fails");
        assert_eq!(candidates_error.0, StatusCode::INTERNAL_SERVER_ERROR);

        let pairs_error = ranker_training_pairs_handler(
            State(state),
            auth_headers("review-token-a"),
            Query(RankerTrainingExportQuery {
                limit: Some(10),
                purpose: Some("required_db_ranker_pairs".to_string()),
                status: None,
                consent_scope: Some("ranking_training".to_string()),
                privacy_risk: None,
            }),
        )
        .await
        .expect_err("required DB mirror writes fail closed when ranker pair provenance fails");
        assert_eq!(pairs_error.0, StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn revocation_tombstone_records_hashes_and_blocks_reingest() {
        use ironclaw::db::libsql::LibSqlBackend;

        let temp = tempfile::tempdir().expect("temp dir");
        let db_temp = tempfile::tempdir().expect("db temp dir");
        let db_path = db_temp.path().join("trace-revocation-tombstone-hash.db");
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
        let original_submission_id = envelope.submission_id;
        let mut duplicate_envelope = envelope.clone();
        duplicate_envelope.submission_id = Uuid::new_v4();
        duplicate_envelope.trace_id = Uuid::new_v4();
        duplicate_envelope.contributor.revocation_handle = Uuid::new_v4();
        duplicate_envelope.trace_card.revocation_handle =
            duplicate_envelope.contributor.revocation_handle.to_string();

        let _ = submit_trace_handler(
            State(state.clone()),
            auth_headers("token-a"),
            Json(envelope),
        )
        .await
        .expect("submission succeeds");
        let record = read_submission_record(temp.path(), "tenant-a", original_submission_id)
            .expect("submission record reads")
            .expect("submission record exists");
        let stored_envelope =
            read_envelope_by_record(state.as_ref(), &record).expect("stored envelope reads");
        let redaction_hash = stored_envelope.privacy.redaction_hash.clone();
        let derived = read_derived_record(temp.path(), "tenant-a", original_submission_id)
            .expect("derived record reads")
            .expect("derived record exists");
        let canonical_summary_hash = derived.canonical_summary_hash.clone();

        let status = revoke_trace_handler(
            State(state.clone()),
            auth_headers("token-a"),
            AxumPath(original_submission_id),
        )
        .await
        .expect("revocation succeeds");
        assert_eq!(status, StatusCode::NO_CONTENT);

        let file_tombstones =
            read_all_revocations(temp.path(), "tenant-a").expect("file tombstones read");
        assert_eq!(file_tombstones.len(), 1);
        assert_eq!(
            file_tombstones[0].canonical_summary_hash.as_deref(),
            Some(canonical_summary_hash.as_str())
        );
        assert_eq!(
            file_tombstones[0].redaction_hash.as_deref(),
            Some(redaction_hash.as_str())
        );
        let db_tombstones = db
            .list_trace_tombstones("tenant-a")
            .await
            .expect("DB tombstones read");
        assert_eq!(db_tombstones.len(), 1);
        assert_eq!(
            db_tombstones[0].canonical_summary_hash.as_deref(),
            Some(canonical_summary_hash.as_str())
        );
        assert_eq!(
            db_tombstones[0].redaction_hash.as_deref(),
            Some(redaction_hash.as_str())
        );

        let duplicate_error = submit_trace_handler(
            State(state),
            auth_headers("token-a"),
            Json(duplicate_envelope),
        )
        .await
        .expect_err("reingesting revoked trace content is blocked");
        assert_eq!(duplicate_error.0, StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn revocation_tombstone_redaction_hash_blocks_reingest_without_summary_hash() {
        let temp = tempfile::tempdir().expect("temp dir");
        let state = test_state(temp.path().to_path_buf());
        let mut envelope = sample_envelope().await;
        make_metadata_only_low_risk(&mut envelope);
        let original_submission_id = envelope.submission_id;
        let mut duplicate_envelope = envelope.clone();
        duplicate_envelope.submission_id = Uuid::new_v4();
        duplicate_envelope.trace_id = Uuid::new_v4();
        duplicate_envelope.contributor.revocation_handle = Uuid::new_v4();
        duplicate_envelope.trace_card.revocation_handle =
            duplicate_envelope.contributor.revocation_handle.to_string();

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
            AxumPath(original_submission_id),
        )
        .await
        .expect("revocation succeeds");

        let mut tombstones =
            read_all_revocations(temp.path(), "tenant-a").expect("file tombstones read");
        assert_eq!(tombstones.len(), 1);
        assert!(tombstones[0].redaction_hash.is_some());
        tombstones[0].canonical_summary_hash = None;
        let tombstone_path = temp
            .path()
            .join("tenants")
            .join(tenant_storage_key("tenant-a"))
            .join("revocations")
            .join(format!("{original_submission_id}.json"));
        write_json_file(
            &tombstone_path,
            &tombstones[0],
            "test redaction-only revocation tombstone",
        )
        .expect("test tombstone overwrite succeeds");

        let duplicate_error = submit_trace_handler(
            State(state),
            auth_headers("token-a"),
            Json(duplicate_envelope),
        )
        .await
        .expect_err("redaction-only tombstone blocks reingest");
        assert_eq!(duplicate_error.0, StatusCode::CONFLICT);
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn submit_rejects_reingest_matching_db_revocation_tombstone_without_file_tombstone() {
        use ironclaw::db::libsql::LibSqlBackend;

        let temp = tempfile::tempdir().expect("temp dir");
        let db_temp = tempfile::tempdir().expect("db temp dir");
        let db_path = db_temp.path().join("trace-db-tombstone-reingest.db");
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
        let original_submission_id = envelope.submission_id;
        let mut duplicate_envelope = envelope.clone();
        duplicate_envelope.submission_id = Uuid::new_v4();
        duplicate_envelope.trace_id = Uuid::new_v4();
        duplicate_envelope.contributor.revocation_handle = Uuid::new_v4();
        duplicate_envelope.trace_card.revocation_handle =
            duplicate_envelope.contributor.revocation_handle.to_string();

        let _ = submit_trace_handler(
            State(state.clone()),
            auth_headers("token-a"),
            Json(envelope),
        )
        .await
        .expect("submission succeeds");
        let status = revoke_trace_handler(
            State(state.clone()),
            auth_headers("token-a"),
            AxumPath(original_submission_id),
        )
        .await
        .expect("revocation succeeds");
        assert_eq!(status, StatusCode::NO_CONTENT);
        assert_eq!(
            db.list_trace_tombstones("tenant-a")
                .await
                .expect("DB tombstones read")
                .len(),
            1
        );

        let file_tombstone_path = temp
            .path()
            .join("tenants")
            .join(tenant_storage_key("tenant-a"))
            .join("revocations")
            .join(format!("{original_submission_id}.json"));
        std::fs::remove_file(file_tombstone_path).expect("remove file-backed tombstone");
        assert!(
            read_all_revocations(temp.path(), "tenant-a")
                .expect("file tombstones read")
                .is_empty()
        );

        let duplicate_error = submit_trace_handler(
            State(state),
            auth_headers("token-a"),
            Json(duplicate_envelope),
        )
        .await
        .expect_err("DB tombstone blocks reingest without file tombstone");
        assert_eq!(duplicate_error.0, StatusCode::CONFLICT);
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn ranking_utility_credit_preserves_db_event_type() {
        use ironclaw::db::libsql::LibSqlBackend;

        let temp = tempfile::tempdir().expect("temp dir");
        let db_temp = tempfile::tempdir().expect("db temp dir");
        let db_path = db_temp.path().join("trace-ranking-utility-credit.db");
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

        let _ = submit_trace_handler(
            State(state.clone()),
            auth_headers("token-a"),
            Json(envelope),
        )
        .await
        .expect("submission succeeds");
        let Json(appended) = append_credit_event_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            AxumPath(submission_id),
            Json(TraceCreditLedgerAppendRequest {
                event_type: TraceCreditLedgerEventType::RankingUtility,
                credit_points_delta: 1.75,
                reason: Some("ranker pair improved ordering".to_string()),
                external_ref: Some("ranker_training_pairs_export:test".to_string()),
            }),
        )
        .await
        .expect("ranking utility credit append succeeds");
        assert_eq!(
            appended.event_type,
            TraceCreditLedgerEventType::RankingUtility
        );

        let db_credit_events = db
            .list_trace_credit_events("tenant-a")
            .await
            .expect("DB credit events read");
        assert!(
            db_credit_events.iter().any(|event| {
                event.submission_id == submission_id
                    && event.event_type == StorageTraceCreditEventType::RankingUtility
            }),
            "DB mirror should retain ranking utility rather than collapsing it into training utility"
        );

        let tenant_dir = temp
            .path()
            .join("tenants")
            .join(tenant_storage_key("tenant-a"));
        std::fs::remove_file(tenant_dir.join("credit_ledger").join("events.jsonl"))
            .expect("remove file-backed ledger to prove DB read path");
        let Json(events) = credit_events_handler(State(state), auth_headers("token-a"))
            .await
            .expect("credit events load from DB");
        assert!(
            events.iter().any(|event| {
                event.submission_id == submission_id
                    && event.event_type == TraceCreditLedgerEventType::RankingUtility
                    && (event.credit_points_delta - 1.75).abs() < f32::EPSILON
            }),
            "DB-backed contributor credit events should round-trip ranking utility"
        );
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn utility_credit_worker_appends_idempotent_credit_for_accepted_traces() {
        use ironclaw::db::libsql::LibSqlBackend;

        let temp = tempfile::tempdir().expect("temp dir");
        let db_temp = tempfile::tempdir().expect("db temp dir");
        let db_path = db_temp.path().join("trace-utility-credit-worker.db");
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

        let _ = submit_trace_handler(
            State(state.clone()),
            auth_headers("token-a"),
            Json(envelope),
        )
        .await
        .expect("submission succeeds");
        let request = TraceUtilityCreditJobRequest {
            event_type: TraceCreditLedgerEventType::TrainingUtility,
            credit_points_delta: 1.5,
            reason: "offline model utility job selected this trace".to_string(),
            external_ref: "training-job:2026-04-utility".to_string(),
            submission_ids: vec![submission_id],
        };

        let Json(appended) = utility_credit_handler(
            State(state.clone()),
            auth_headers("utility-worker-token-a"),
            Json(request),
        )
        .await
        .expect("utility worker can append training utility credit");
        assert_eq!(appended.requested_count, 1);
        assert_eq!(appended.appended_count, 1);
        assert_eq!(appended.skipped_existing_count, 0);
        assert_eq!(
            appended.event_type,
            TraceCreditLedgerEventType::TrainingUtility
        );

        let Json(retry) = utility_credit_handler(
            State(state.clone()),
            auth_headers("utility-worker-token-a"),
            Json(TraceUtilityCreditJobRequest {
                event_type: TraceCreditLedgerEventType::TrainingUtility,
                credit_points_delta: 1.5,
                reason: "offline model utility job selected this trace".to_string(),
                external_ref: "training-job:2026-04-utility".to_string(),
                submission_ids: vec![submission_id],
            }),
        )
        .await
        .expect("utility worker retry is idempotent");
        assert_eq!(retry.appended_count, 0);
        assert_eq!(retry.skipped_existing_count, 1);

        let db_credit_events = db
            .list_trace_credit_events("tenant-a")
            .await
            .expect("DB credit events read");
        let utility_events = db_credit_events
            .iter()
            .filter(|event| {
                event.submission_id == submission_id
                    && event.event_type == StorageTraceCreditEventType::TrainingUtility
            })
            .collect::<Vec<_>>();
        assert_eq!(utility_events.len(), 1);

        let Json(events) = credit_events_handler(State(state), auth_headers("token-a"))
            .await
            .expect("contributor can see utility credit event");
        assert!(events.iter().any(|event| {
            event.submission_id == submission_id
                && event.event_type == TraceCreditLedgerEventType::TrainingUtility
                && (event.credit_points_delta - 1.5).abs() < f32::EPSILON
        }));
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn delayed_credit_append_can_use_db_metadata_without_file_record() {
        use ironclaw::db::libsql::LibSqlBackend;

        let temp = tempfile::tempdir().expect("temp dir");
        let db_temp = tempfile::tempdir().expect("db temp dir");
        let db_path = db_temp.path().join("trace-db-credit-append.db");
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
            .expect("submission succeeds");
        let metadata_path = submission_metadata_path(temp.path(), "tenant-a", submission_id);
        std::fs::remove_file(&metadata_path).expect("remove file-backed metadata");

        let credit_state =
            test_state_with_db_reviewer_reads(temp.path().to_path_buf(), Some(db.clone()));
        let Json(appended) = append_credit_event_handler(
            State(credit_state),
            auth_headers("review-token-a"),
            AxumPath(submission_id),
            Json(TraceCreditLedgerAppendRequest {
                event_type: TraceCreditLedgerEventType::ReviewerBonus,
                credit_points_delta: 1.25,
                reason: Some("DB-backed reviewer credit".to_string()),
                external_ref: None,
            }),
        )
        .await
        .expect("credit append can read submission metadata from DB");
        assert_eq!(appended.submission_id, submission_id);
        assert_eq!(
            appended.auth_principal_ref,
            principal_storage_ref("token-a")
        );
        assert!(
            !metadata_path.exists(),
            "DB-backed credit append should not recreate missing file metadata"
        );

        let db_credit_events = db
            .list_trace_credit_events("tenant-a")
            .await
            .expect("DB credit events read");
        assert!(db_credit_events.iter().any(|event| {
            event.submission_id == submission_id
                && event.event_type == StorageTraceCreditEventType::ReviewerBonus
                && event.points_delta == "1.2500"
        }));
        let db_audit_events = db
            .list_trace_audit_events("tenant-a")
            .await
            .expect("DB audit events read");
        assert!(db_audit_events.iter().any(|event| {
            event.action == StorageTraceAuditAction::CreditMutate
                && event.submission_id == Some(submission_id)
        }));
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn revoked_trace_delayed_credit_is_excluded_from_db_backed_totals() {
        use ironclaw::db::libsql::LibSqlBackend;

        let temp = tempfile::tempdir().expect("temp dir");
        let db_temp = tempfile::tempdir().expect("db temp dir");
        let db_path = db_temp.path().join("trace-db-revoked-credit.db");
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

        let _ = submit_trace_handler(
            State(state.clone()),
            auth_headers("token-a"),
            Json(envelope),
        )
        .await
        .expect("submission succeeds");
        let Json(appended) = append_credit_event_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            AxumPath(submission_id),
            Json(TraceCreditLedgerAppendRequest {
                event_type: TraceCreditLedgerEventType::TrainingUtility,
                credit_points_delta: 3.0,
                reason: Some("training job selected this trace".to_string()),
                external_ref: Some("training-job:revoked-credit".to_string()),
            }),
        )
        .await
        .expect("credit append succeeds");
        assert_eq!(appended.credit_points_delta, 3.0);

        let status = revoke_trace_handler(
            State(state.clone()),
            auth_headers("token-a"),
            AxumPath(submission_id),
        )
        .await
        .expect("revocation succeeds");
        assert_eq!(status, StatusCode::NO_CONTENT);

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
        assert!(
            events.is_empty(),
            "terminal trace credit events remain in the audit ledger but are hidden from contributor credit projections"
        );

        let Json(credit) = credit_handler(State(state.clone()), auth_headers("token-a"))
            .await
            .expect("credit summary loads from DB");
        assert_eq!(credit.revoked, 1);
        assert_eq!(credit.credit_points_ledger, 0.0);
        assert_eq!(credit.credit_points_final, 0.0);
        assert_eq!(credit.credit_points_total, 0.0);

        let Json(statuses) = submission_status_handler(
            State(state),
            auth_headers("token-a"),
            Json(TraceSubmissionStatusRequest {
                submission_ids: vec![submission_id],
            }),
        )
        .await
        .expect("status loads from DB");
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].status, "revoked");
        assert_eq!(statuses[0].credit_points_ledger, 0.0);
        assert_eq!(statuses[0].credit_points_final, Some(0.0));
        assert_eq!(statuses[0].credit_points_total, None);
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
        assert_eq!(credit.credit_points_final, 0.0);
        assert_eq!(credit.credit_points_total, 2.5);

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
        assert_eq!(statuses[0].credit_points_final, None);
        assert_eq!(statuses[0].credit_points_ledger, 2.5);
        assert_eq!(statuses[0].credit_points_total, Some(2.5));
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
        let audit_events = read_all_audit_events(temp.path(), "tenant-a").expect("audit reads");
        assert!(audit_events.iter().any(|event| {
            event.kind == "read"
                && event
                    .reason
                    .as_deref()
                    .is_some_and(|reason| reason.contains("surface=analytics_summary"))
                && event.export_count == Some(2)
        }));
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

        let missing_reason_error = review_decision_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            AxumPath(submission_id),
            Json(TraceReviewDecisionRequest {
                decision: TraceReviewDecision::Approve,
                reason: None,
                credit_points_pending: Some(1.25),
            }),
        )
        .await
        .expect_err("review decisions require an explicit reason");
        assert_eq!(missing_reason_error.0, StatusCode::BAD_REQUEST);
        let blank_reason_error = review_decision_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            AxumPath(submission_id),
            Json(TraceReviewDecisionRequest {
                decision: TraceReviewDecision::Approve,
                reason: Some(" ".to_string()),
                credit_points_pending: Some(1.25),
            }),
        )
        .await
        .expect_err("review decisions reject blank reasons");
        assert_eq!(blank_reason_error.0, StatusCode::BAD_REQUEST);

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
    async fn export_guardrails_require_explicit_filters_when_enabled() {
        let temp = tempfile::tempdir().expect("temp dir");
        let state = test_state_with_export_guardrails(temp.path().to_path_buf());

        let replay_error = dataset_replay_handler(
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
        .expect_err("guarded replay export requires explicit filters");
        assert_eq!(replay_error.0, StatusCode::BAD_REQUEST);

        let Json(replay_export) = dataset_replay_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            Query(DatasetExportQuery {
                limit: Some(10),
                purpose: Some("guarded_replay".to_string()),
                status: Some(TraceCorpusStatus::Accepted),
                privacy_risk: Some(ResidualPiiRisk::Low),
                consent_scope: Some("debugging_evaluation".to_string()),
            }),
        )
        .await
        .expect("fully filtered replay export is allowed");
        assert_eq!(replay_export.item_count, 0);
        assert_eq!(replay_export.manifest.purpose, "guarded_replay");

        let benchmark_error = benchmark_convert_handler(
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
        .expect_err("guarded benchmark conversion requires explicit filters");
        assert_eq!(benchmark_error.0, StatusCode::BAD_REQUEST);

        let Json(benchmark) = benchmark_convert_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            Json(BenchmarkConversionRequest {
                limit: Some(10),
                purpose: Some("guarded_benchmark".to_string()),
                consent_scope: Some("benchmark_only".to_string()),
                status: Some(TraceCorpusStatus::Accepted),
                privacy_risk: Some(ResidualPiiRisk::Low),
                external_ref: None,
            }),
        )
        .await
        .expect("fully filtered benchmark conversion is allowed");
        assert_eq!(benchmark.item_count, 0);
        assert_eq!(benchmark.purpose, "guarded_benchmark");

        let ranker_error = ranker_training_candidates_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            Query(RankerTrainingExportQuery {
                limit: Some(10),
                purpose: None,
                status: None,
                consent_scope: None,
                privacy_risk: None,
            }),
        )
        .await
        .expect_err("guarded ranker export requires explicit filters");
        assert_eq!(ranker_error.0, StatusCode::BAD_REQUEST);

        let ranker_missing_purpose = ranker_training_candidates_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            Query(RankerTrainingExportQuery {
                limit: Some(10),
                purpose: None,
                status: Some(TraceCorpusStatus::Accepted),
                consent_scope: Some("ranking_training".to_string()),
                privacy_risk: Some(ResidualPiiRisk::Low),
            }),
        )
        .await
        .expect_err("guarded ranker export requires an explicit purpose");
        assert_eq!(ranker_missing_purpose.0, StatusCode::BAD_REQUEST);

        let ranker_pair_missing_purpose = ranker_training_pairs_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            Query(RankerTrainingExportQuery {
                limit: Some(10),
                purpose: None,
                status: Some(TraceCorpusStatus::Accepted),
                consent_scope: Some("model_training".to_string()),
                privacy_risk: Some(ResidualPiiRisk::Low),
            }),
        )
        .await
        .expect_err("guarded ranker pair export requires an explicit purpose");
        assert_eq!(ranker_pair_missing_purpose.0, StatusCode::BAD_REQUEST);

        let Json(candidates) = ranker_training_candidates_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            Query(RankerTrainingExportQuery {
                limit: Some(10),
                purpose: Some("guarded_ranker_candidates".to_string()),
                status: Some(TraceCorpusStatus::Accepted),
                consent_scope: Some("ranking_training".to_string()),
                privacy_risk: Some(ResidualPiiRisk::Low),
            }),
        )
        .await
        .expect("fully filtered ranker candidates export is allowed");
        assert_eq!(candidates.item_count, 0);
        assert_eq!(candidates.purpose, "guarded_ranker_candidates");

        let Json(pairs) = ranker_training_pairs_handler(
            State(state),
            auth_headers("review-token-a"),
            Query(RankerTrainingExportQuery {
                limit: Some(10),
                purpose: Some("guarded_ranker_pairs".to_string()),
                status: Some(TraceCorpusStatus::Accepted),
                consent_scope: Some("model_training".to_string()),
                privacy_risk: Some(ResidualPiiRisk::Low),
            }),
        )
        .await
        .expect("fully filtered ranker pairs export is allowed");
        assert_eq!(pairs.item_count, 0);
        assert_eq!(pairs.purpose, "guarded_ranker_pairs");
    }

    #[tokio::test]
    async fn bulk_export_limit_cap_is_applied_by_export_callers() {
        let temp = tempfile::tempdir().expect("temp dir");
        let mut state = test_state(temp.path().to_path_buf());
        Arc::get_mut(&mut state)
            .expect("state is uniquely owned")
            .max_export_items_per_request = 1;

        for index in 0..3 {
            let mut envelope = sample_envelope().await;
            make_metadata_only_low_risk(&mut envelope);
            envelope.consent.scopes = vec![
                ConsentScope::DebuggingEvaluation,
                ConsentScope::BenchmarkOnly,
                ConsentScope::RankingTraining,
            ];
            envelope.value.submission_score = 0.9 - (index as f32 * 0.1);
            let _ = submit_trace_handler(
                State(state.clone()),
                auth_headers("token-a"),
                Json(envelope),
            )
            .await
            .expect("submission succeeds");
        }

        let Json(replay) = dataset_replay_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            Query(DatasetExportQuery {
                limit: Some(50),
                purpose: Some("capped_replay".to_string()),
                status: None,
                privacy_risk: None,
                consent_scope: None,
            }),
        )
        .await
        .expect("replay export succeeds");
        assert_eq!(replay.item_count, 1);
        assert_eq!(replay.manifest.filters.limit, 1);

        let Json(benchmark) = benchmark_convert_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            Json(BenchmarkConversionRequest {
                limit: Some(50),
                purpose: Some("capped_benchmark".to_string()),
                consent_scope: None,
                status: None,
                privacy_risk: None,
                external_ref: None,
            }),
        )
        .await
        .expect("benchmark conversion succeeds");
        assert_eq!(benchmark.item_count, 1);
        assert_eq!(benchmark.filters.limit, 1);

        let Json(candidates) = ranker_training_candidates_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            Query(RankerTrainingExportQuery {
                limit: Some(50),
                purpose: Some("capped_ranker_candidates".to_string()),
                status: None,
                consent_scope: None,
                privacy_risk: None,
            }),
        )
        .await
        .expect("ranker candidate export succeeds");
        assert_eq!(candidates.item_count, 1);

        let Json(pairs) = ranker_training_pairs_handler(
            State(state),
            auth_headers("review-token-a"),
            Query(RankerTrainingExportQuery {
                limit: Some(50),
                purpose: Some("capped_ranker_pairs".to_string()),
                status: None,
                consent_scope: None,
                privacy_risk: None,
            }),
        )
        .await
        .expect("ranker pair export succeeds");
        assert_eq!(pairs.item_count, 1);
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
        let credit_events =
            read_all_credit_events(temp.path(), "tenant-a").expect("credit events read");
        let benchmark_credit_events = credit_events
            .iter()
            .filter(|event| event.event_type == TraceCreditLedgerEventType::BenchmarkConversion)
            .collect::<Vec<_>>();
        assert_eq!(benchmark_credit_events.len(), 1);
        assert_eq!(benchmark_credit_events[0].submission_id, kept_id);
        assert_ne!(benchmark_credit_events[0].submission_id, revoked_id);

        let _ = benchmark_convert_handler(
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
        .expect("benchmark conversion rerun succeeds");
        let rerun_credit_events =
            read_all_credit_events(temp.path(), "tenant-a").expect("rerun credit events read");
        assert_eq!(
            rerun_credit_events
                .iter()
                .filter(|event| event.event_type == TraceCreditLedgerEventType::BenchmarkConversion)
                .count(),
            1,
            "benchmark conversion utility credit must be idempotent"
        );

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

        let Json(published_benchmark) = benchmark_lifecycle_handler(
            State(state.clone()),
            auth_headers("benchmark-worker-token-a"),
            AxumPath(benchmark.conversion_id),
            Json(BenchmarkLifecycleUpdateRequest {
                registry: Some(TraceBenchmarkRegistryPatch {
                    status: Some(TraceBenchmarkRegistryStatus::Published),
                    registry_ref: Some("benchmark-registry:provenance".to_string()),
                    published_at: Some(Utc::now()),
                }),
                evaluation: Some(TraceBenchmarkEvaluationPatch {
                    status: Some(TraceBenchmarkEvaluationStatus::Passed),
                    evaluator_ref: Some("evaluator:provenance".to_string()),
                    evaluated_at: Some(Utc::now()),
                    score: Some(1.0),
                    pass_count: Some(1),
                    fail_count: Some(0),
                }),
                reason: Some("published provenance benchmark".to_string()),
            }),
        )
        .await
        .expect("benchmark lifecycle publish succeeds");
        assert_eq!(
            published_benchmark.registry.status,
            TraceBenchmarkRegistryStatus::Published
        );

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

        let invalidated_artifact: TraceBenchmarkConversionArtifact = serde_json::from_str(
            &std::fs::read_to_string(benchmark_artifact_path(
                temp.path(),
                "tenant-a",
                benchmark.conversion_id,
            ))
            .expect("invalidated benchmark artifact reads"),
        )
        .expect("invalidated benchmark artifact parses");
        assert_eq!(
            invalidated_artifact.registry.status,
            TraceBenchmarkRegistryStatus::Revoked
        );
        assert!(invalidated_artifact.registry.revoked_at.is_some());
        assert_eq!(
            invalidated_artifact.registry.revocation_reason.as_deref(),
            Some("contributor_revocation")
        );
        assert_eq!(
            invalidated_artifact.evaluation.status,
            TraceBenchmarkEvaluationStatus::Inconclusive
        );
        assert!(
            invalidated_artifact
                .evaluation
                .last_update_reason
                .as_deref()
                .is_some_and(|reason| reason.contains("contributor_revocation"))
        );
        let audit_events =
            read_all_audit_events(temp.path(), "tenant-a").expect("audit events read");
        assert!(audit_events.iter().any(|event| {
            event.kind == "benchmark_lifecycle_update"
                && event.export_id == Some(benchmark.conversion_id)
                && event.reason.as_deref().is_some_and(|reason| {
                    reason.contains("registry_status=revoked")
                        && reason.contains("evaluation_status=inconclusive")
                })
        }));
    }

    #[tokio::test]
    async fn benchmark_conversion_artifact_tracks_registry_and_evaluator_state() {
        let temp = tempfile::tempdir().expect("temp dir");
        let state = test_state(temp.path().to_path_buf());
        let mut envelope = sample_envelope().await;
        make_metadata_only_low_risk(&mut envelope);

        let _ = submit_trace_handler(
            State(state.clone()),
            auth_headers("token-a"),
            Json(envelope),
        )
        .await
        .expect("benchmark source submission succeeds");

        let Json(benchmark) = benchmark_convert_handler(
            State(state),
            auth_headers("review-token-a"),
            Json(BenchmarkConversionRequest {
                limit: Some(10),
                purpose: Some("registry_evaluator_contract".to_string()),
                consent_scope: None,
                status: None,
                privacy_risk: None,
                external_ref: Some("benchmark-registry:pending".to_string()),
            }),
        )
        .await
        .expect("benchmark conversion succeeds");

        assert_eq!(
            benchmark.artifact_schema_version,
            TRACE_BENCHMARK_CONVERSION_SCHEMA_VERSION
        );
        assert_eq!(
            benchmark.registry.status,
            TraceBenchmarkRegistryStatus::Candidate
        );
        assert!(benchmark.registry.registry_ref.is_none());
        assert!(benchmark.registry.published_at.is_none());
        assert_eq!(
            benchmark.evaluation.status,
            TraceBenchmarkEvaluationStatus::NotRun
        );
        assert!(benchmark.evaluation.evaluator_ref.is_none());
        assert!(benchmark.evaluation.evaluated_at.is_none());
        assert!(benchmark.evaluation.score.is_none());

        let artifact_path =
            benchmark_artifact_path(temp.path(), "tenant-a", benchmark.conversion_id);
        let persisted: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(artifact_path).expect("benchmark artifact reads"),
        )
        .expect("benchmark artifact parses");
        assert_eq!(
            persisted["artifact_schema_version"],
            TRACE_BENCHMARK_CONVERSION_SCHEMA_VERSION
        );
        assert_eq!(persisted["registry"]["status"], "candidate");
        assert_eq!(persisted["evaluation"]["status"], "not_run");
        assert!(persisted["registry"]["registry_ref"].is_null());
        assert!(persisted["evaluation"]["evaluator_ref"].is_null());
    }

    #[tokio::test]
    async fn benchmark_lifecycle_update_persists_registry_and_evaluator_state() {
        let temp = tempfile::tempdir().expect("temp dir");
        let state = test_state(temp.path().to_path_buf());
        let mut envelope = sample_envelope().await;
        make_metadata_only_low_risk(&mut envelope);

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
                purpose: Some("lifecycle_update".to_string()),
                consent_scope: None,
                status: None,
                privacy_risk: None,
                external_ref: None,
            }),
        )
        .await
        .expect("benchmark conversion succeeds");

        let evaluated_at = Utc::now();
        let Json(updated) = benchmark_lifecycle_handler(
            State(state.clone()),
            auth_headers("benchmark-worker-token-a"),
            AxumPath(benchmark.conversion_id),
            Json(BenchmarkLifecycleUpdateRequest {
                registry: Some(TraceBenchmarkRegistryPatch {
                    status: Some(TraceBenchmarkRegistryStatus::Published),
                    registry_ref: Some("benchmark-registry:trace-replay-smoke".to_string()),
                    published_at: Some(evaluated_at),
                }),
                evaluation: Some(TraceBenchmarkEvaluationPatch {
                    status: Some(TraceBenchmarkEvaluationStatus::Passed),
                    evaluator_ref: Some("evaluator:trace-replay-smoke".to_string()),
                    evaluated_at: Some(evaluated_at),
                    score: Some(0.97),
                    pass_count: Some(7),
                    fail_count: Some(0),
                }),
                reason: Some("published by evaluator worker".to_string()),
            }),
        )
        .await
        .expect("benchmark lifecycle update succeeds");

        assert_eq!(
            updated.registry.status,
            TraceBenchmarkRegistryStatus::Published
        );
        assert_eq!(
            updated.registry.registry_ref.as_deref(),
            Some("benchmark-registry:trace-replay-smoke")
        );
        assert_eq!(
            updated.evaluation.status,
            TraceBenchmarkEvaluationStatus::Passed
        );
        assert_eq!(updated.evaluation.score, Some(0.97));
        assert_eq!(updated.evaluation.pass_count, Some(7));
        assert_eq!(updated.evaluation.fail_count, Some(0));

        let artifact_path =
            benchmark_artifact_path(temp.path(), "tenant-a", benchmark.conversion_id);
        let persisted: TraceBenchmarkConversionArtifact = serde_json::from_str(
            &std::fs::read_to_string(artifact_path).expect("benchmark artifact reads"),
        )
        .expect("benchmark artifact parses");
        assert_eq!(
            persisted.registry.status,
            TraceBenchmarkRegistryStatus::Published
        );
        assert_eq!(
            persisted.evaluation.status,
            TraceBenchmarkEvaluationStatus::Passed
        );

        let audit_events =
            read_all_audit_events(temp.path(), "tenant-a").expect("audit events read");
        assert!(audit_events.iter().any(|event| {
            event.kind == "benchmark_lifecycle_update"
                && event.export_id == Some(benchmark.conversion_id)
                && event.reason.as_deref().is_some_and(|reason| {
                    reason.contains("registry_status=published")
                        && reason.contains("evaluation_status=passed")
                })
        }));
    }

    #[test]
    fn legacy_benchmark_conversion_artifact_defaults_lifecycle_metadata() {
        let artifact: TraceBenchmarkConversionArtifact =
            serde_json::from_value(serde_json::json!({
                "tenant_id": "tenant-a",
                "tenant_storage_ref": tenant_storage_ref("tenant-a"),
                "conversion_id": Uuid::new_v4(),
                "audit_event_id": Uuid::new_v4(),
                "purpose": "legacy_artifact",
                "filters": { "limit": 1 },
                "source_submission_ids": [],
                "source_submission_ids_hash": "sha256:legacy",
                "generated_at": "2026-04-25T00:00:00Z",
                "item_count": 0,
                "candidates": []
            }))
            .expect("legacy benchmark artifact still deserializes");

        assert_eq!(
            artifact.artifact_schema_version,
            TRACE_BENCHMARK_CONVERSION_SCHEMA_VERSION
        );
        assert_eq!(
            artifact.registry.status,
            TraceBenchmarkRegistryStatus::Candidate
        );
        assert_eq!(
            artifact.evaluation.status,
            TraceBenchmarkEvaluationStatus::NotRun
        );
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
                purpose: None,
                status: None,
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
                purpose: Some("ranker_candidates_tenant_scope".to_string()),
                status: None,
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
        let credit_events =
            read_all_credit_events(temp.path(), "tenant-a").expect("credit events read");
        let training_credit_events = credit_events
            .iter()
            .filter(|event| event.event_type == TraceCreditLedgerEventType::TrainingUtility)
            .collect::<Vec<_>>();
        assert_eq!(training_credit_events.len(), 2);
        assert!(
            training_credit_events
                .iter()
                .any(|event| event.submission_id == tenant_a_best_id)
        );
        assert!(
            training_credit_events
                .iter()
                .any(|event| event.submission_id == tenant_a_lower_id)
        );
        assert!(
            training_credit_events
                .iter()
                .all(|event| event.submission_id != tenant_a_quarantined_id
                    && event.submission_id != tenant_a_revoked_id
                    && event.submission_id != tenant_b_id)
        );

        let _ = ranker_training_candidates_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            Query(RankerTrainingExportQuery {
                limit: Some(10),
                purpose: Some("ranker_candidates_tenant_scope".to_string()),
                status: None,
                consent_scope: Some("ranking-training".to_string()),
                privacy_risk: None,
            }),
        )
        .await
        .expect("ranker candidates export rerun succeeds");
        let rerun_credit_events =
            read_all_credit_events(temp.path(), "tenant-a").expect("rerun credit events read");
        assert_eq!(
            rerun_credit_events
                .iter()
                .filter(|event| event.event_type == TraceCreditLedgerEventType::TrainingUtility)
                .count(),
            2,
            "ranker candidate utility credit must be idempotent"
        );

        let Json(pairs) = ranker_training_pairs_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            Query(RankerTrainingExportQuery {
                limit: Some(10),
                purpose: Some("ranker_pairs_tenant_scope".to_string()),
                status: None,
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
        let pair_credit_events =
            read_all_credit_events(temp.path(), "tenant-a").expect("pair credit events read");
        let ranking_utility_events = pair_credit_events
            .iter()
            .filter(|event| event.event_type == TraceCreditLedgerEventType::RankingUtility)
            .collect::<Vec<_>>();
        assert_eq!(ranking_utility_events.len(), 2);
        assert!(
            ranking_utility_events
                .iter()
                .any(|event| event.submission_id == tenant_a_best_id)
        );
        assert!(
            ranking_utility_events
                .iter()
                .any(|event| event.submission_id == tenant_a_lower_id)
        );

        let _ = ranker_training_pairs_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            Query(RankerTrainingExportQuery {
                limit: Some(10),
                purpose: Some("ranker_pairs_tenant_scope".to_string()),
                status: None,
                consent_scope: Some("ranking-training".to_string()),
                privacy_risk: None,
            }),
        )
        .await
        .expect("ranker pairs export rerun succeeds");
        let rerun_pair_credit_events =
            read_all_credit_events(temp.path(), "tenant-a").expect("rerun pair credit events read");
        assert_eq!(
            rerun_pair_credit_events
                .iter()
                .filter(|event| event.event_type == TraceCreditLedgerEventType::RankingUtility)
                .count(),
            2,
            "ranker pair utility credit must be idempotent"
        );

        let Json(limited_pairs) = ranker_training_pairs_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            Query(RankerTrainingExportQuery {
                limit: Some(1),
                purpose: Some("ranker_pairs_limited".to_string()),
                status: None,
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
                purpose: None,
                status: None,
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
                purpose: Some("ranker_provenance_candidates".to_string()),
                status: None,
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
                purpose: Some("ranker_provenance_pairs".to_string()),
                status: None,
                consent_scope: Some("ranking-training".to_string()),
                privacy_risk: None,
            }),
        )
        .await
        .expect("ranker pairs export succeeds");
        assert_eq!(candidates.purpose, "ranker_provenance_candidates");
        assert_eq!(pairs.purpose, "ranker_provenance_pairs");
        let Json(ranker_purpose_records) = list_traces_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            Query(TraceListQuery {
                status: None,
                limit: Some(10),
                purpose: Some("ranker_provenance_candidates".to_string()),
                coverage_tag: None,
                tool: None,
                privacy_risk: None,
                consent_scope: None,
            }),
        )
        .await
        .expect("trace list filters by ranker export purpose");
        assert_eq!(ranker_purpose_records.len(), 2);
        assert!(
            ranker_purpose_records
                .iter()
                .any(|record| record.submission_id == preferred_id)
        );
        assert!(
            ranker_purpose_records
                .iter()
                .any(|record| record.submission_id == rejected_id)
        );

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
        assert_eq!(
            candidate_provenance["purpose"],
            "ranker_provenance_candidates"
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
        assert_eq!(pair_provenance["purpose"], "ranker_provenance_pairs");
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
                redaction_hash: None,
                canonical_summary_hash: None,
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
                verify_audit_chain: false,
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

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn benchmark_and_ranker_exports_revalidate_db_source_status_before_publish() {
        use ironclaw::db::libsql::LibSqlBackend;

        let temp = tempfile::tempdir().expect("temp dir");
        let db_temp = tempfile::tempdir().expect("db temp dir");
        let db_path = db_temp.path().join("trace-export-source-revalidation.db");
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
        let mut preferred = sample_envelope().await;
        make_metadata_only_low_risk(&mut preferred);
        preferred.consent.scopes = vec![ConsentScope::RankingTraining];
        preferred.value.submission_score = 0.9;
        let preferred_id = preferred.submission_id;
        let mut revoked_in_db = sample_envelope().await;
        make_metadata_only_low_risk(&mut revoked_in_db);
        revoked_in_db.consent.scopes = vec![ConsentScope::RankingTraining];
        revoked_in_db.value.submission_score = 0.1;
        let revoked_id = revoked_in_db.submission_id;

        let _ = submit_trace_handler(
            State(state.clone()),
            auth_headers("token-a"),
            Json(preferred),
        )
        .await
        .expect("preferred source submits");
        let _ = submit_trace_handler(
            State(state.clone()),
            auth_headers("token-a"),
            Json(revoked_in_db),
        )
        .await
        .expect("revoked-in-db source submits");
        db.update_trace_submission_status(
            "tenant-a",
            revoked_id,
            StorageTraceCorpusStatus::Revoked,
            &principal_storage_ref("review-token-a"),
            Some("test_db_status_race"),
        )
        .await
        .expect("DB source status updates");

        let benchmark_error = benchmark_convert_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            Json(BenchmarkConversionRequest {
                limit: Some(10),
                purpose: Some("stale_benchmark_source".to_string()),
                consent_scope: None,
                status: Some(TraceCorpusStatus::Accepted),
                privacy_risk: Some(ResidualPiiRisk::Low),
                external_ref: None,
            }),
        )
        .await
        .expect_err("benchmark conversion revalidates DB source status");
        assert_eq!(benchmark_error.0, StatusCode::INTERNAL_SERVER_ERROR);

        let ranker_error = ranker_training_candidates_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            Query(RankerTrainingExportQuery {
                limit: Some(10),
                purpose: Some("stale_ranker_source".to_string()),
                status: Some(TraceCorpusStatus::Accepted),
                consent_scope: Some("ranking-training".to_string()),
                privacy_risk: Some(ResidualPiiRisk::Low),
            }),
        )
        .await
        .expect_err("ranker candidate export revalidates DB source status");
        assert_eq!(ranker_error.0, StatusCode::INTERNAL_SERVER_ERROR);

        let pair_error = ranker_training_pairs_handler(
            State(state),
            auth_headers("review-token-a"),
            Query(RankerTrainingExportQuery {
                limit: Some(10),
                purpose: Some("stale_ranker_pair_source".to_string()),
                status: Some(TraceCorpusStatus::Accepted),
                consent_scope: Some("ranking-training".to_string()),
                privacy_risk: Some(ResidualPiiRisk::Low),
            }),
        )
        .await
        .expect_err("ranker pair export revalidates DB source status");
        assert_eq!(pair_error.0, StatusCode::INTERNAL_SERVER_ERROR);
        assert!(
            db.list_trace_export_manifests("tenant-a")
                .await
                .expect("export manifests read")
                .is_empty(),
            "stale-source exports should fail before publishing manifests"
        );
        assert_ne!(preferred_id, revoked_id);
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn benchmark_and_ranker_exports_can_require_active_submitted_object_refs() {
        use ironclaw::db::libsql::LibSqlBackend;

        let temp = tempfile::tempdir().expect("temp dir");
        let db_temp = tempfile::tempdir().expect("db temp dir");
        let db_path = db_temp
            .path()
            .join("trace-derived-export-object-ref-gate.db");
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
        let mut submission_ids = Vec::new();
        for score in [0.9_f32, 0.1_f32] {
            let mut envelope = sample_envelope().await;
            make_metadata_only_low_risk(&mut envelope);
            envelope.consent.scopes = vec![ConsentScope::RankingTraining];
            envelope.trace_card.consent_scope = ConsentScope::RankingTraining;
            envelope.trace_card.allowed_uses = vec![TraceAllowedUse::RankingModelTraining];
            envelope.value.submission_score = score;
            submission_ids.push(envelope.submission_id);
            let _ = submit_trace_handler(
                State(submit_state.clone()),
                auth_headers("token-a"),
                Json(envelope),
            )
            .await
            .expect("ranker source submission succeeds");
        }
        for submission_id in submission_ids.iter().copied() {
            let invalidation_counts = db
                .invalidate_trace_submission_artifacts(
                    "tenant-a",
                    submission_id,
                    StorageTraceDerivedStatus::Current,
                )
                .await
                .expect("invalidate submitted object refs");
            assert_eq!(invalidation_counts.object_refs_invalidated, 1);
        }

        let utility_credit_count = || {
            read_all_credit_events(temp.path(), "tenant-a")
                .expect("credit events read")
                .iter()
                .filter(|event| {
                    matches!(
                        event.event_type,
                        TraceCreditLedgerEventType::BenchmarkConversion
                            | TraceCreditLedgerEventType::TrainingUtility
                            | TraceCreditLedgerEventType::RankingUtility
                    )
                })
                .count()
        };
        let initial_utility_credit_count = utility_credit_count();
        let export_state = test_state_with_required_derived_export_object_refs(
            temp.path().to_path_buf(),
            Some(db.clone() as Arc<dyn Database>),
        );

        let benchmark_error = benchmark_convert_handler(
            State(export_state.clone()),
            auth_headers("review-token-a"),
            Json(BenchmarkConversionRequest {
                limit: Some(10),
                purpose: Some("require_source_object_ref_benchmark".to_string()),
                consent_scope: None,
                status: Some(TraceCorpusStatus::Accepted),
                privacy_risk: Some(ResidualPiiRisk::Low),
                external_ref: None,
            }),
        )
        .await
        .expect_err("benchmark export requires active submitted-envelope object refs");
        assert_eq!(benchmark_error.0, StatusCode::INTERNAL_SERVER_ERROR);

        let candidates_error = ranker_training_candidates_handler(
            State(export_state.clone()),
            auth_headers("review-token-a"),
            Query(RankerTrainingExportQuery {
                limit: Some(10),
                purpose: Some("require_source_object_ref_candidates".to_string()),
                status: Some(TraceCorpusStatus::Accepted),
                consent_scope: Some("ranking-training".to_string()),
                privacy_risk: Some(ResidualPiiRisk::Low),
            }),
        )
        .await
        .expect_err("ranker candidate export requires active submitted-envelope object refs");
        assert_eq!(candidates_error.0, StatusCode::INTERNAL_SERVER_ERROR);

        let pairs_error = ranker_training_pairs_handler(
            State(export_state),
            auth_headers("review-token-a"),
            Query(RankerTrainingExportQuery {
                limit: Some(10),
                purpose: Some("require_source_object_ref_pairs".to_string()),
                status: Some(TraceCorpusStatus::Accepted),
                consent_scope: Some("ranking-training".to_string()),
                privacy_risk: Some(ResidualPiiRisk::Low),
            }),
        )
        .await
        .expect_err("ranker pair export requires active submitted-envelope object refs");
        assert_eq!(pairs_error.0, StatusCode::INTERNAL_SERVER_ERROR);

        let tenant_dir = temp
            .path()
            .join("tenants")
            .join(tenant_storage_key("tenant-a"));
        assert!(!tenant_dir.join("benchmarks").exists());
        assert!(!tenant_dir.join("ranker_exports").exists());
        assert!(
            db.list_trace_export_manifests("tenant-a")
                .await
                .expect("export manifests read")
                .is_empty()
        );
        assert_eq!(utility_credit_count(), initial_utility_credit_count);
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

        let Json(limited_queue) = active_learning_review_queue_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            Query(ActiveLearningQueueQuery {
                limit: Some(0),
                privacy_risk: None,
            }),
        )
        .await
        .expect("limit is clamped to at least one item");
        assert_eq!(limited_queue.item_count, 1);

        let Json(clamped_queue) = active_learning_review_queue_handler(
            State(state),
            auth_headers("review-token-a"),
            Query(ActiveLearningQueueQuery {
                limit: Some(usize::MAX),
                privacy_risk: None,
            }),
        )
        .await
        .expect("limit is clamped to the reviewer page maximum");
        assert_eq!(clamped_queue.item_count, 2);
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
                redaction_hash: None,
                canonical_summary_hash: None,
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
                verify_audit_chain: false,
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
                verify_audit_chain: false,
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

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn maintenance_discovered_revocations_update_db_mirror_and_invalidate_artifacts() {
        use ironclaw::db::libsql::LibSqlBackend;

        let temp = tempfile::tempdir().expect("temp dir");
        let db_temp = tempfile::tempdir().expect("db temp dir");
        let db_path = db_temp.path().join("trace-maintenance-revocation.db");
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
        let Json(vector_response) = maintenance_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            Json(TraceMaintenanceRequest {
                purpose: Some("test_revocation_vector_index".to_string()),
                dry_run: false,
                backfill_db_mirror: false,
                index_vectors: true,
                reconcile_db_mirror: false,
                verify_audit_chain: false,
                prune_export_cache: false,
                max_export_age_hours: None,
                purge_expired_before: None,
            }),
        )
        .await
        .expect("vector metadata indexing succeeds");
        assert_eq!(vector_response.vector_entries_indexed, 1);
        let Json(export) = dataset_replay_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            Query(DatasetExportQuery {
                limit: Some(10),
                purpose: Some("maintenance_revocation_replay".to_string()),
                status: Some(TraceCorpusStatus::Accepted),
                privacy_risk: Some(ResidualPiiRisk::Low),
                consent_scope: None,
            }),
        )
        .await
        .expect("replay export mirrors manifest metadata");
        assert_eq!(export.item_count, 1);
        let Json(benchmark) = benchmark_convert_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            Json(BenchmarkConversionRequest {
                limit: Some(10),
                purpose: Some("maintenance_revocation_benchmark".to_string()),
                consent_scope: None,
                status: Some(TraceCorpusStatus::Accepted),
                privacy_risk: Some(ResidualPiiRisk::Low),
                external_ref: None,
            }),
        )
        .await
        .expect("benchmark conversion mirrors manifest metadata");
        let Json(published_benchmark) = benchmark_lifecycle_handler(
            State(state.clone()),
            auth_headers("benchmark-worker-token-a"),
            AxumPath(benchmark.conversion_id),
            Json(BenchmarkLifecycleUpdateRequest {
                registry: Some(TraceBenchmarkRegistryPatch {
                    status: Some(TraceBenchmarkRegistryStatus::Published),
                    registry_ref: Some("benchmark-registry:maintenance".to_string()),
                    published_at: Some(Utc::now()),
                }),
                evaluation: Some(TraceBenchmarkEvaluationPatch {
                    status: Some(TraceBenchmarkEvaluationStatus::Passed),
                    evaluator_ref: Some("evaluator:maintenance".to_string()),
                    evaluated_at: Some(Utc::now()),
                    score: Some(1.0),
                    pass_count: Some(1),
                    fail_count: Some(0),
                }),
                reason: Some("published maintenance benchmark".to_string()),
            }),
        )
        .await
        .expect("benchmark lifecycle publish succeeds");
        assert_eq!(
            published_benchmark.registry.status,
            TraceBenchmarkRegistryStatus::Published
        );

        write_revocation(
            temp.path(),
            &TraceCommonsRevocation {
                tenant_id: "tenant-a".to_string(),
                tenant_storage_ref: tenant_storage_ref("tenant-a"),
                submission_id,
                revoked_at: Utc::now(),
                reason: "maintenance_tombstone_only".to_string(),
                redaction_hash: None,
                canonical_summary_hash: None,
            },
        )
        .expect("file revocation tombstone writes");
        let Json(response) = maintenance_handler(
            State(state),
            auth_headers("review-token-a"),
            Json(TraceMaintenanceRequest {
                purpose: Some("maintenance_revocation_db_mirror".to_string()),
                dry_run: false,
                backfill_db_mirror: false,
                index_vectors: false,
                reconcile_db_mirror: false,
                verify_audit_chain: false,
                prune_export_cache: false,
                max_export_age_hours: None,
                purge_expired_before: None,
            }),
        )
        .await
        .expect("maintenance mirrors discovered revocation");
        assert_eq!(response.records_marked_revoked, 1);
        assert_eq!(response.derived_marked_revoked, 1);
        assert_eq!(response.benchmark_artifacts_invalidated, 1);

        let db_submission = db
            .get_trace_submission("tenant-a", submission_id)
            .await
            .expect("DB submission status reads")
            .expect("DB submission exists");
        assert_eq!(db_submission.status, StorageTraceCorpusStatus::Revoked);
        assert!(
            db.list_trace_object_refs("tenant-a", submission_id)
                .await
                .expect("DB object refs read")
                .iter()
                .all(|record| record.invalidated_at.is_some())
        );
        assert!(
            db.list_trace_derived_records("tenant-a")
                .await
                .expect("DB derived records read")
                .iter()
                .any(|record| {
                    record.submission_id == submission_id
                        && record.status == StorageTraceDerivedStatus::Revoked
                })
        );
        assert!(
            db.list_trace_vector_entries("tenant-a")
                .await
                .expect("DB vector entries read")
                .iter()
                .any(|record| {
                    record.submission_id == submission_id
                        && record.status == StorageTraceVectorEntryStatus::Invalidated
                })
        );
        let db_manifests = db
            .list_trace_export_manifests("tenant-a")
            .await
            .expect("DB export manifests read");
        assert!(db_manifests.iter().any(|manifest| {
            manifest.export_manifest_id == export.export_id && manifest.invalidated_at.is_some()
        }));
        assert!(db_manifests.iter().any(|manifest| {
            manifest.export_manifest_id == benchmark.conversion_id
                && manifest.invalidated_at.is_some()
        }));
        let db_items = db
            .list_trace_export_manifest_items("tenant-a", export.export_id)
            .await
            .expect("DB export manifest items read");
        assert_eq!(db_items.len(), 1);
        assert!(db_items[0].source_invalidated_at.is_some());
        assert_eq!(
            db_items[0].source_invalidation_reason,
            Some(StorageTraceExportManifestItemInvalidationReason::Revoked)
        );
        let invalidated_artifact: TraceBenchmarkConversionArtifact = serde_json::from_str(
            &std::fs::read_to_string(benchmark_artifact_path(
                temp.path(),
                "tenant-a",
                benchmark.conversion_id,
            ))
            .expect("invalidated benchmark artifact reads"),
        )
        .expect("invalidated benchmark artifact parses");
        assert_eq!(
            invalidated_artifact.registry.status,
            TraceBenchmarkRegistryStatus::Revoked
        );
        assert_eq!(
            invalidated_artifact.evaluation.status,
            TraceBenchmarkEvaluationStatus::Inconclusive
        );
        assert_eq!(
            db.list_trace_tombstones("tenant-a")
                .await
                .expect("DB tombstones read")
                .len(),
            1
        );
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn maintenance_repairs_db_revocation_for_already_revoked_file_records() {
        use ironclaw::db::libsql::LibSqlBackend;

        let temp = tempfile::tempdir().expect("temp dir");
        let db_temp = tempfile::tempdir().expect("db temp dir");
        let db_path = db_temp
            .path()
            .join("trace-maintenance-existing-revocation.db");
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
        let Json(vector_response) = maintenance_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            Json(TraceMaintenanceRequest {
                purpose: Some("existing_revocation_vector_index".to_string()),
                dry_run: false,
                backfill_db_mirror: false,
                index_vectors: true,
                reconcile_db_mirror: false,
                verify_audit_chain: false,
                prune_export_cache: false,
                max_export_age_hours: None,
                purge_expired_before: None,
            }),
        )
        .await
        .expect("vector metadata indexing succeeds");
        assert_eq!(vector_response.vector_entries_indexed, 1);
        let Json(export) = dataset_replay_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            Query(DatasetExportQuery {
                limit: Some(10),
                purpose: Some("existing_revocation_replay".to_string()),
                status: Some(TraceCorpusStatus::Accepted),
                privacy_risk: Some(ResidualPiiRisk::Low),
                consent_scope: None,
            }),
        )
        .await
        .expect("replay export mirrors manifest metadata");
        assert_eq!(export.item_count, 1);

        let mut record = read_submission_record(temp.path(), "tenant-a", submission_id)
            .expect("file record reads")
            .expect("file record exists");
        record.status = TraceCorpusStatus::Revoked;
        record.credit_points_final = Some(0.0);
        write_submission_record(temp.path(), &record).expect("file record marks revoked");

        let Json(response) = maintenance_handler(
            State(state),
            auth_headers("review-token-a"),
            Json(TraceMaintenanceRequest {
                purpose: Some("repair_existing_revocation_db_mirror".to_string()),
                dry_run: false,
                backfill_db_mirror: false,
                index_vectors: false,
                reconcile_db_mirror: false,
                verify_audit_chain: false,
                prune_export_cache: false,
                max_export_age_hours: None,
                purge_expired_before: None,
            }),
        )
        .await
        .expect("maintenance repairs existing file revocation in DB");
        assert_eq!(response.records_marked_revoked, 0);
        assert_eq!(response.derived_marked_revoked, 1);

        let db_submission = db
            .get_trace_submission("tenant-a", submission_id)
            .await
            .expect("DB submission status reads")
            .expect("DB submission exists");
        assert_eq!(db_submission.status, StorageTraceCorpusStatus::Revoked);
        assert!(
            db.list_trace_object_refs("tenant-a", submission_id)
                .await
                .expect("DB object refs read")
                .iter()
                .all(|record| record.invalidated_at.is_some())
        );
        assert!(
            db.list_trace_derived_records("tenant-a")
                .await
                .expect("DB derived records read")
                .iter()
                .any(|record| {
                    record.submission_id == submission_id
                        && record.status == StorageTraceDerivedStatus::Revoked
                })
        );
        assert!(
            db.list_trace_vector_entries("tenant-a")
                .await
                .expect("DB vector entries read")
                .iter()
                .any(|record| {
                    record.submission_id == submission_id
                        && record.status == StorageTraceVectorEntryStatus::Invalidated
                })
        );
        let db_manifests = db
            .list_trace_export_manifests("tenant-a")
            .await
            .expect("DB export manifests read");
        assert!(db_manifests.iter().any(|manifest| {
            manifest.export_manifest_id == export.export_id && manifest.invalidated_at.is_some()
        }));
        let db_items = db
            .list_trace_export_manifest_items("tenant-a", export.export_id)
            .await
            .expect("DB export manifest items read");
        assert_eq!(db_items.len(), 1);
        assert!(db_items[0].source_invalidated_at.is_some());
        assert_eq!(
            db_items[0].source_invalidation_reason,
            Some(StorageTraceExportManifestItemInvalidationReason::Revoked)
        );
        assert_eq!(
            db.list_trace_tombstones("tenant-a")
                .await
                .expect("DB tombstones read")
                .len(),
            1
        );
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
                verify_audit_chain: false,
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
                verify_audit_chain: false,
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
        let audit_events = db
            .list_trace_audit_events("tenant-a")
            .await
            .expect("audit events read");
        let expiration_audit = audit_events
            .iter()
            .find(|event| {
                event.action == StorageTraceAuditAction::Retain
                    && event.submission_id == Some(submission_id)
                    && event.reason.as_deref() == Some("retention_expired_artifact_invalidation")
            })
            .expect("expiration invalidation audit is mirrored");
        match &expiration_audit.metadata {
            StorageTraceAuditSafeMetadata::Maintenance {
                dry_run,
                action_counts,
            } => {
                assert!(!dry_run);
                assert_eq!(
                    action_counts
                        .get("records_marked_expired")
                        .copied()
                        .unwrap_or_default(),
                    1
                );
                assert!(
                    action_counts
                        .get("object_refs_invalidated")
                        .copied()
                        .unwrap_or_default()
                        >= 1
                );
                assert!(
                    action_counts
                        .get("derived_records_invalidated")
                        .copied()
                        .unwrap_or_default()
                        >= 1
                );
            }
            metadata => panic!("unexpected expiration audit metadata: {metadata:?}"),
        }
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
                verify_audit_chain: false,
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
        let audit_events = db
            .list_trace_audit_events("tenant-a")
            .await
            .expect("audit events read");
        let purge_audit = audit_events
            .iter()
            .find(|event| {
                event.action == StorageTraceAuditAction::Purge
                    && event.submission_id == Some(submission_id)
                    && event.reason.as_deref() == Some("retention_purged_artifact_invalidation")
            })
            .expect("purge invalidation audit is mirrored");
        match &purge_audit.metadata {
            StorageTraceAuditSafeMetadata::Maintenance {
                dry_run,
                action_counts,
            } => {
                assert!(!dry_run);
                assert_eq!(
                    action_counts
                        .get("records_marked_purged")
                        .copied()
                        .unwrap_or_default(),
                    1
                );
                assert!(action_counts.contains_key("object_refs_invalidated"));
            }
            metadata => panic!("unexpected purge audit metadata: {metadata:?}"),
        }
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn maintenance_purge_deletes_service_local_object_store_and_invalidates_refs() {
        use ironclaw::db::libsql::LibSqlBackend;

        let temp = tempfile::tempdir().expect("temp dir");
        let artifact_root = temp.path().join("service-object-store");
        let artifact_store = test_artifact_store(&artifact_root);
        let configured_store = ConfiguredTraceArtifactStore::new(
            TRACE_COMMONS_SERVICE_LOCAL_ENCRYPTED_OBJECT_STORE,
            artifact_store.clone(),
        );
        let db_temp = tempfile::tempdir().expect("db temp dir");
        let db_path = db_temp.path().join("trace-service-object-store-purge.db");
        let db = Arc::new(
            LibSqlBackend::new_local(&db_path)
                .await
                .expect("create libsql mirror"),
        );
        db.run_migrations().await.expect("run migrations");
        let state = test_state_with_configured_artifact_store_policies_and_export_guardrails(
            temp.path().to_path_buf(),
            Some(db.clone() as Arc<dyn Database>),
            Some(configured_store),
            false,
            false,
            false,
            false,
            false,
            false,
            BTreeMap::new(),
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
            TRACE_COMMONS_SERVICE_LOCAL_ENCRYPTED_OBJECT_STORE
        );
        let record = read_submission_record(temp.path(), "tenant-a", submission_id)
            .expect("record reads")
            .expect("record exists");
        let receipt = record
            .artifact_receipt
            .clone()
            .expect("service object receipt exists");
        artifact_store
            .read_artifact(&record.tenant_storage_ref, &receipt)
            .expect("service-local encrypted object exists");

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
                purpose: Some("test_service_object_store_purge".to_string()),
                dry_run: false,
                backfill_db_mirror: false,
                index_vectors: false,
                reconcile_db_mirror: false,
                verify_audit_chain: false,
                prune_export_cache: false,
                max_export_age_hours: None,
                purge_expired_before: Some(Utc::now()),
            }),
        )
        .await
        .expect("maintenance purges service-local object");
        assert_eq!(response.records_marked_expired, 1);
        assert_eq!(response.records_marked_purged, 1);
        assert_eq!(response.encrypted_artifacts_deleted, 1);
        artifact_store
            .read_artifact(&tenant_storage_ref("tenant-a"), &receipt)
            .expect_err("service-local encrypted object was deleted");

        let object_refs = db
            .list_trace_object_refs("tenant-a", submission_id)
            .await
            .expect("object refs read");
        assert_eq!(object_refs.len(), 1);
        assert_eq!(
            object_refs[0].object_store,
            TRACE_COMMONS_SERVICE_LOCAL_ENCRYPTED_OBJECT_STORE
        );
        assert!(object_refs[0].invalidated_at.is_some());
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
        let missing_admin_purpose = maintenance_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            Json(TraceMaintenanceRequest {
                purpose: None,
                dry_run: false,
                backfill_db_mirror: false,
                index_vectors: false,
                reconcile_db_mirror: false,
                verify_audit_chain: false,
                prune_export_cache: false,
                max_export_age_hours: None,
                purge_expired_before: Some(Utc::now()),
            }),
        )
        .await
        .expect_err("destructive purge requires an explicit purpose");
        assert_eq!(missing_admin_purpose.0, StatusCode::BAD_REQUEST);

        let missing_worker_purpose = retention_maintenance_handler(
            State(state.clone()),
            auth_headers("retention-worker-token-a"),
            Json(TraceRetentionMaintenanceRequest {
                purpose: None,
                dry_run: false,
                prune_export_cache: false,
                max_export_age_hours: None,
                purge_expired_before: Some(Utc::now()),
            }),
        )
        .await
        .expect_err("retention worker purge requires an explicit purpose");
        assert_eq!(missing_worker_purpose.0, StatusCode::BAD_REQUEST);

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
                verify_audit_chain: false,
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
                verify_audit_chain: false,
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

    #[tokio::test]
    async fn maintenance_legal_hold_retention_policy_blocks_expiration_and_purge() {
        let temp = tempfile::tempdir().expect("temp dir");
        let mut tokens = BTreeMap::new();
        insert_token(&mut tokens, "tenant-a", "token-a", TokenRole::Contributor);
        insert_token(
            &mut tokens,
            "tenant-a",
            "review-token-a",
            TokenRole::Reviewer,
        );
        let state = Arc::new(AppState {
            root: temp.path().to_path_buf(),
            tokens: Arc::new(tokens),
            tenant_policies: Arc::new(BTreeMap::new()),
            require_tenant_submission_policy: false,
            db_mirror: None,
            db_contributor_reads: false,
            db_reviewer_reads: false,
            db_reviewer_require_object_refs: false,
            db_replay_export_reads: false,
            db_replay_export_require_object_refs: false,
            db_audit_reads: false,
            db_tenant_policy_reads: false,
            require_db_mirror_writes: false,
            require_derived_export_object_refs: false,
            object_primary_submit_review: false,
            object_primary_replay_export: false,
            object_primary_derived_exports: false,
            require_db_reconciliation_clean: false,
            require_export_guardrails: false,
            max_export_items_per_request: DEFAULT_TRACE_COMMONS_MAX_EXPORT_ITEMS_PER_REQUEST,
            submission_quota: TraceSubmissionQuotaConfig::default(),
            legal_hold_retention_policy_ids: Arc::new(BTreeSet::from([
                "private_corpus_revocable".to_string()
            ])),
            artifact_store: None,
        });

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
        assert_eq!(record.retention_policy_id, "private_corpus_revocable");
        let object_path = temp.path().join(&record.object_key);
        assert!(object_path.exists());

        let metadata_path = temp
            .path()
            .join("tenants")
            .join(tenant_storage_key("tenant-a"))
            .join("metadata")
            .join(format!("{submission_id}.json"));
        let mut metadata_json = serde_json::to_value(record).expect("record serializes");
        metadata_json["expires_at"] =
            serde_json::json!((Utc::now() - chrono::Duration::days(2)).to_rfc3339());
        write_json_file(&metadata_path, &metadata_json, "legal-hold trace metadata")
            .expect("metadata writes");

        let Json(response) = maintenance_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            Json(TraceMaintenanceRequest {
                purpose: Some("legal_hold_retention".to_string()),
                dry_run: false,
                backfill_db_mirror: false,
                index_vectors: false,
                reconcile_db_mirror: false,
                verify_audit_chain: false,
                prune_export_cache: false,
                max_export_age_hours: None,
                purge_expired_before: Some(Utc::now()),
            }),
        )
        .await
        .expect("maintenance succeeds");
        assert_eq!(response.records_marked_expired, 0);
        assert_eq!(response.records_marked_purged, 0);
        assert_eq!(response.expired_submission_count, 0);
        assert_eq!(response.trace_object_files_deleted, 0);
        assert!(object_path.exists());

        let held_record = read_submission_record(temp.path(), "tenant-a", submission_id)
            .expect("held record reads")
            .expect("held record exists");
        assert_eq!(held_record.status, TraceCorpusStatus::Accepted);
        assert!(held_record.purged_at.is_none());

        let Json(export) = dataset_replay_handler(
            State(state),
            auth_headers("review-token-a"),
            Query(DatasetExportQuery {
                limit: Some(10),
                purpose: Some("legal_hold_export_check".to_string()),
                status: None,
                privacy_risk: None,
                consent_scope: None,
            }),
        )
        .await
        .expect("legal-hold trace remains exportable while held");
        assert_eq!(export.item_count, 1);
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

        let Json(receipt) = submit_trace_handler(
            State(file_state.clone()),
            auth_headers("token-a"),
            Json(envelope),
        )
        .await
        .expect("file-backed submission succeeds");
        assert_eq!(receipt.status, "accepted");
        let Json(delayed_credit_before_backfill) = append_credit_event_handler(
            State(file_state.clone()),
            auth_headers("review-token-a"),
            AxumPath(submission_id),
            Json(TraceCreditLedgerAppendRequest {
                event_type: TraceCreditLedgerEventType::ReviewerBonus,
                credit_points_delta: 1.25,
                reason: Some("file-side credit before DB backfill".to_string()),
                external_ref: None,
            }),
        )
        .await
        .expect("file-backed delayed credit succeeds before DB backfill");
        let Json(file_export) = dataset_replay_handler(
            State(file_state),
            auth_headers("review-token-a"),
            Query(DatasetExportQuery {
                limit: Some(10),
                purpose: Some("file_export_before_db_backfill".to_string()),
                status: Some(TraceCorpusStatus::Accepted),
                privacy_risk: Some(ResidualPiiRisk::Low),
                consent_scope: None,
            }),
        )
        .await
        .expect("file-backed replay export succeeds before DB backfill");
        assert_eq!(file_export.item_count, 1);

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
                verify_audit_chain: false,
                prune_export_cache: false,
                max_export_age_hours: None,
                purge_expired_before: None,
            }),
        )
        .await
        .expect("reviewer can backfill DB mirror");
        assert!(
            response.db_mirror_backfilled >= 4,
            "backfill should mirror submission plus existing file-side credit/audit/export rows"
        );

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
        let db_credit_events = db
            .list_trace_credit_events("tenant-a")
            .await
            .expect("credit events mirrored during backfill");
        assert!(
            db_credit_events
                .iter()
                .any(|event| event.credit_event_id == delayed_credit_before_backfill.event_id)
        );
        let file_audit_events =
            read_all_audit_events(temp.path(), "tenant-a").expect("file audit events read");
        let db_audit_events = db
            .list_trace_audit_events("tenant-a")
            .await
            .expect("audit events mirrored during backfill");
        for file_event in &file_audit_events {
            assert!(
                db_audit_events
                    .iter()
                    .any(|db_event| db_event.audit_event_id == file_event.event_id),
                "file audit event {} should be mirrored during backfill",
                file_event.event_id
            );
        }
        let db_export_manifests = db
            .list_trace_export_manifests("tenant-a")
            .await
            .expect("export manifests mirrored during backfill");
        assert!(
            db_export_manifests
                .iter()
                .any(|manifest| manifest.export_manifest_id == file_export.export_id)
        );
        let db_export_items = db
            .list_trace_export_manifest_items("tenant-a", file_export.export_id)
            .await
            .expect("export manifest items mirrored during backfill");
        assert_eq!(db_export_items.len(), 1);

        let Json(vector_response) = maintenance_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            Json(TraceMaintenanceRequest {
                purpose: Some("test_vector_index".to_string()),
                dry_run: false,
                backfill_db_mirror: false,
                index_vectors: true,
                reconcile_db_mirror: false,
                verify_audit_chain: false,
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
                verify_audit_chain: false,
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
                verify_audit_chain: false,
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
        assert!(
            reconciliation
                .missing_derived_submission_ids_in_files
                .is_empty()
        );
        assert!(reconciliation.derived_status_mismatches.is_empty());
        assert!(reconciliation.derived_hash_mismatches.is_empty());
        assert_eq!(reconciliation.db_object_ref_count, 1);
        assert!(reconciliation.file_credit_event_count >= 1);
        assert!(reconciliation.db_credit_event_count >= 1);
        assert!(reconciliation.file_audit_event_count >= 1);
        assert!(reconciliation.db_audit_event_count >= 1);
        assert_eq!(reconciliation.file_replay_export_manifest_count, 1);
        assert_eq!(reconciliation.db_export_manifest_count, 1);
        assert_eq!(reconciliation.db_replay_export_manifest_count, 1);
        assert_eq!(reconciliation.db_benchmark_export_manifest_count, 0);
        assert_eq!(reconciliation.db_ranker_export_manifest_count, 0);
        assert_eq!(reconciliation.db_other_export_manifest_count, 0);
        assert_eq!(reconciliation.db_export_manifest_item_count, 1);
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
        assert!(
            reconciliation
                .unreadable_active_envelope_object_refs
                .is_empty()
        );
        assert!(
            reconciliation
                .hash_mismatched_active_envelope_object_refs
                .is_empty()
        );
        assert_eq!(reconciliation.active_vector_entries, 1);
        assert!(
            reconciliation
                .accepted_current_derived_without_active_vector_entry
                .is_empty()
        );
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
            State(state.clone()),
            auth_headers("review-token-a"),
            Json(TraceMaintenanceRequest {
                purpose: Some("test_db_backfill_idempotent".to_string()),
                dry_run: false,
                backfill_db_mirror: true,
                index_vectors: false,
                reconcile_db_mirror: false,
                verify_audit_chain: false,
                prune_export_cache: false,
                max_export_age_hours: None,
                purge_expired_before: None,
            }),
        )
        .await
        .expect("backfill can be rerun");
        assert_eq!(second_response.db_mirror_backfilled, 1);

        let Json(third_response) = maintenance_handler(
            State(state),
            auth_headers("review-token-a"),
            Json(TraceMaintenanceRequest {
                purpose: Some("test_db_backfill_really_idempotent".to_string()),
                dry_run: false,
                backfill_db_mirror: true,
                index_vectors: false,
                reconcile_db_mirror: false,
                verify_audit_chain: false,
                prune_export_cache: false,
                max_export_age_hours: None,
                purge_expired_before: None,
            }),
        )
        .await
        .expect("backfill can be rerun after repairing file-only revoke audit");
        assert_eq!(third_response.db_mirror_backfilled, 0);
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn maintenance_backfill_isolates_bad_file_backed_submissions() {
        use ironclaw::db::libsql::LibSqlBackend;

        let temp = tempfile::tempdir().expect("temp dir");
        let file_state = test_state(temp.path().to_path_buf());
        let mut good = sample_envelope().await;
        make_metadata_only_low_risk(&mut good);
        let good_id = good.submission_id;
        let mut bad = sample_envelope().await;
        make_metadata_only_low_risk(&mut bad);
        let bad_id = bad.submission_id;

        let _ = submit_trace_handler(
            State(file_state.clone()),
            auth_headers("token-a"),
            Json(good),
        )
        .await
        .expect("good file-backed submission succeeds");
        let _ = submit_trace_handler(State(file_state), auth_headers("token-a"), Json(bad))
            .await
            .expect("bad file-backed submission initially succeeds");

        let bad_derived_path = temp
            .path()
            .join("tenants")
            .join(tenant_storage_key("tenant-a"))
            .join("derived")
            .join(format!("{bad_id}.json"));
        std::fs::remove_file(&bad_derived_path).expect("remove bad derived record");

        let db_temp = tempfile::tempdir().expect("db temp dir");
        let db_path = db_temp.path().join("trace-backfill-isolates-bad-record.db");
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
            State(state),
            auth_headers("review-token-a"),
            Json(TraceMaintenanceRequest {
                purpose: Some("test_backfill_bad_record_isolation".to_string()),
                dry_run: false,
                backfill_db_mirror: true,
                index_vectors: false,
                reconcile_db_mirror: false,
                verify_audit_chain: false,
                prune_export_cache: false,
                max_export_age_hours: None,
                purge_expired_before: None,
            }),
        )
        .await
        .expect("maintenance isolates bad backfill records");
        assert!(response.db_mirror_backfilled >= 1);
        assert_eq!(response.db_mirror_backfill_failed, 1);
        assert_eq!(response.db_mirror_backfill_failures.len(), 1);
        assert_eq!(
            response.db_mirror_backfill_failures[0].item_kind,
            "submission"
        );
        assert_eq!(
            response.db_mirror_backfill_failures[0].item_ref,
            bad_id.to_string()
        );
        assert!(
            response.db_mirror_backfill_failures[0]
                .reason
                .contains("missing a derived precheck record")
        );

        assert!(
            db.get_trace_submission("tenant-a", good_id)
                .await
                .expect("good mirror query succeeds")
                .is_some()
        );
        assert!(
            db.get_trace_submission("tenant-a", bad_id)
                .await
                .expect("bad mirror query succeeds")
                .is_none()
        );
        let audit_events = db
            .list_trace_audit_events("tenant-a")
            .await
            .expect("audit events read");
        let maintenance_audit = audit_events
            .iter()
            .find(|event| {
                event.action == StorageTraceAuditAction::Retain
                    && event
                        .reason
                        .as_deref()
                        .is_some_and(|reason| reason.contains("test_backfill_bad_record_isolation"))
            })
            .expect("maintenance audit event mirrored");
        match &maintenance_audit.metadata {
            StorageTraceAuditSafeMetadata::Maintenance { action_counts, .. } => {
                assert_eq!(
                    action_counts
                        .get("db_mirror_backfill_failed")
                        .copied()
                        .unwrap_or_default(),
                    1
                );
            }
            metadata => panic!("unexpected maintenance metadata: {metadata:?}"),
        }
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn maintenance_reconciliation_reports_missing_ledger_and_audit_event_ids() {
        use ironclaw::db::libsql::LibSqlBackend;

        let temp = tempfile::tempdir().expect("temp dir");
        let db_temp = tempfile::tempdir().expect("db temp dir");
        let db_path = db_temp.path().join("trace-mirror-reconciliation-events.db");
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

        let Json(delayed_credit) = append_credit_event_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            AxumPath(submission_id),
            Json(TraceCreditLedgerAppendRequest {
                event_type: TraceCreditLedgerEventType::ReviewerBonus,
                credit_points_delta: 1.0,
                reason: Some("mirror reconciliation gap".to_string()),
                external_ref: None,
            }),
        )
        .await
        .expect("delayed credit succeeds");

        let audit_event_id = read_all_audit_events(temp.path(), "tenant-a")
            .expect("file audit events read")
            .into_iter()
            .find(|event| event.submission_id == submission_id)
            .expect("submission audit event exists")
            .event_id;
        let conn = db.connect().await.expect("connect to mirror");
        conn.execute(
            "DELETE FROM trace_credit_ledger WHERE tenant_id = ?1 AND credit_event_id = ?2",
            libsql::params!["tenant-a", delayed_credit.event_id.to_string()],
        )
        .await
        .expect("delete mirrored credit event");
        conn.execute(
            "DELETE FROM trace_audit_events WHERE tenant_id = ?1 AND audit_event_id = ?2",
            libsql::params!["tenant-a", audit_event_id.to_string()],
        )
        .await
        .expect("delete mirrored audit event");

        let Json(response) = maintenance_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            Json(TraceMaintenanceRequest {
                purpose: Some("test_event_gap_reconciliation".to_string()),
                dry_run: true,
                backfill_db_mirror: false,
                index_vectors: false,
                reconcile_db_mirror: true,
                verify_audit_chain: false,
                prune_export_cache: false,
                max_export_age_hours: None,
                purge_expired_before: None,
            }),
        )
        .await
        .expect("reviewer can reconcile DB mirror");
        let reconciliation = response
            .db_reconciliation
            .expect("reconciliation report exists");
        assert_eq!(
            reconciliation.missing_credit_event_ids_in_db,
            vec![delayed_credit.event_id]
        );
        assert!(reconciliation.missing_credit_event_ids_in_files.is_empty());
        assert_eq!(
            reconciliation.missing_audit_event_ids_in_db,
            vec![audit_event_id]
        );
        assert!(reconciliation.missing_audit_event_ids_in_files.is_empty());
        assert!(
            reconciliation
                .blocking_gaps
                .iter()
                .any(|gap| gap == "missing_credit_event_ids_in_db=1")
        );
        assert!(
            reconciliation
                .blocking_gaps
                .iter()
                .any(|gap| gap == "missing_audit_event_ids_in_db=1")
        );
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn maintenance_reconciliation_reports_export_items_missing_object_refs() {
        use ironclaw::db::libsql::LibSqlBackend;

        let temp = tempfile::tempdir().expect("temp dir");
        let db_temp = tempfile::tempdir().expect("db temp dir");
        let db_path = db_temp
            .path()
            .join("trace-reconcile-export-items-missing-object-refs.db");
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

        let _ = submit_trace_handler(
            State(state.clone()),
            auth_headers("token-a"),
            Json(envelope),
        )
        .await
        .expect("submission succeeds");
        let Json(export) = dataset_replay_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            Query(DatasetExportQuery {
                limit: Some(10),
                purpose: Some("missing_object_ref_reconciliation".to_string()),
                status: Some(TraceCorpusStatus::Accepted),
                privacy_risk: Some(ResidualPiiRisk::Low),
                consent_scope: None,
            }),
        )
        .await
        .expect("compatibility-mode replay export succeeds");

        let Json(response) = maintenance_handler(
            State(state),
            auth_headers("review-token-a"),
            Json(TraceMaintenanceRequest {
                purpose: Some("test_export_item_object_ref_reconciliation".to_string()),
                dry_run: true,
                backfill_db_mirror: false,
                index_vectors: false,
                reconcile_db_mirror: true,
                verify_audit_chain: false,
                prune_export_cache: false,
                max_export_age_hours: None,
                purge_expired_before: None,
            }),
        )
        .await
        .expect("reviewer can reconcile DB mirror");
        let reconciliation = response
            .db_reconciliation
            .expect("reconciliation report exists");
        assert_eq!(reconciliation.db_export_manifest_item_count, 1);
        assert_eq!(
            reconciliation.db_export_manifest_item_missing_object_ref_count,
            1
        );
        assert_eq!(
            reconciliation.db_export_manifest_ids_with_missing_object_refs,
            vec![export.export_id]
        );
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn maintenance_reconciliation_reports_active_exports_with_ineligible_sources() {
        use ironclaw::db::libsql::LibSqlBackend;

        let temp = tempfile::tempdir().expect("temp dir");
        let db_temp = tempfile::tempdir().expect("db temp dir");
        let db_path = db_temp
            .path()
            .join("trace-reconcile-active-export-ineligible-source.db");
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
        let Json(export) = dataset_replay_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            Query(DatasetExportQuery {
                limit: Some(10),
                purpose: Some("ineligible_source_reconciliation".to_string()),
                status: Some(TraceCorpusStatus::Accepted),
                privacy_risk: Some(ResidualPiiRisk::Low),
                consent_scope: None,
            }),
        )
        .await
        .expect("replay export succeeds");

        db.update_trace_submission_status(
            "tenant-a",
            submission_id,
            StorageTraceCorpusStatus::Revoked,
            "test-reconciler",
            Some("simulate missed export invalidation"),
        )
        .await
        .expect("DB source status can be changed without export invalidation");

        let Json(response) = maintenance_handler(
            State(state),
            auth_headers("review-token-a"),
            Json(TraceMaintenanceRequest {
                purpose: Some("test_ineligible_export_source_reconciliation".to_string()),
                dry_run: true,
                backfill_db_mirror: false,
                index_vectors: false,
                reconcile_db_mirror: true,
                verify_audit_chain: false,
                prune_export_cache: false,
                max_export_age_hours: None,
                purge_expired_before: None,
            }),
        )
        .await
        .expect("reviewer can reconcile DB mirror");
        let reconciliation = response
            .db_reconciliation
            .expect("reconciliation report exists");
        assert_eq!(
            reconciliation.active_derived_submission_ids_for_invalid_sources,
            vec![submission_id]
        );
        assert_eq!(
            reconciliation.active_export_manifest_ids_for_invalid_sources,
            vec![export.export_id]
        );
        assert_eq!(
            reconciliation
                .active_export_manifest_items_for_invalid_sources
                .len(),
            1
        );
        let invalid_item = &reconciliation.active_export_manifest_items_for_invalid_sources[0];
        assert_eq!(invalid_item.export_manifest_id, export.export_id);
        assert_eq!(invalid_item.submission_id, submission_id);
        assert_eq!(
            invalid_item.source_status_at_export,
            StorageTraceCorpusStatus::Accepted
        );
        assert_eq!(
            reconciliation.active_export_manifest_ids_with_ineligible_items,
            vec![export.export_id]
        );
    }

    #[tokio::test]
    async fn maintenance_reconciliation_without_db_mirror_returns_operator_error() {
        let temp = tempfile::tempdir().expect("temp dir");
        let state = test_state(temp.path().to_path_buf());

        let error = maintenance_handler(
            State(state),
            auth_headers("review-token-a"),
            Json(TraceMaintenanceRequest {
                purpose: Some("test_reconciliation_requires_db".to_string()),
                dry_run: true,
                backfill_db_mirror: false,
                index_vectors: false,
                reconcile_db_mirror: true,
                verify_audit_chain: false,
                prune_export_cache: false,
                max_export_age_hours: None,
                purge_expired_before: None,
            }),
        )
        .await
        .expect_err("DB reconciliation requires configured DB mirror");
        assert_eq!(error.0, StatusCode::SERVICE_UNAVAILABLE);
        assert!(error.1.0.error.contains("TRACE_COMMONS_DB_DUAL_WRITE"));
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn maintenance_reconciliation_clean_gate_accepts_empty_clean_report() {
        use ironclaw::db::libsql::LibSqlBackend;

        let temp = tempfile::tempdir().expect("temp dir");
        let db_temp = tempfile::tempdir().expect("db temp dir");
        let db_path = db_temp.path().join("trace-reconcile-clean-empty.db");
        let db = Arc::new(
            LibSqlBackend::new_local(&db_path)
                .await
                .expect("create libsql mirror"),
        );
        db.run_migrations().await.expect("run migrations");
        let state = test_state_with_required_db_reconciliation_clean(
            temp.path().to_path_buf(),
            db as Arc<dyn Database>,
        );

        let Json(response) = maintenance_handler(
            State(state),
            auth_headers("review-token-a"),
            Json(TraceMaintenanceRequest {
                purpose: Some("test_clean_reconciliation_gate".to_string()),
                dry_run: true,
                backfill_db_mirror: false,
                index_vectors: false,
                reconcile_db_mirror: true,
                verify_audit_chain: false,
                prune_export_cache: false,
                max_export_age_hours: None,
                purge_expired_before: None,
            }),
        )
        .await
        .expect("clean reconciliation gate allows empty report");

        let reconciliation = response
            .db_reconciliation
            .expect("reconciliation report exists");
        assert!(reconciliation.blocking_gaps.is_empty());
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn maintenance_reconciliation_clean_gate_requires_reconciliation_request() {
        use ironclaw::db::libsql::LibSqlBackend;

        let temp = tempfile::tempdir().expect("temp dir");
        let db_temp = tempfile::tempdir().expect("db temp dir");
        let db_path = db_temp.path().join("trace-reconcile-required-clean.db");
        let db = Arc::new(
            LibSqlBackend::new_local(&db_path)
                .await
                .expect("create libsql mirror"),
        );
        db.run_migrations().await.expect("run migrations");
        let state = test_state_with_required_db_reconciliation_clean(
            temp.path().to_path_buf(),
            db as Arc<dyn Database>,
        );

        let error = maintenance_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            Json(TraceMaintenanceRequest {
                purpose: Some("test_reconciliation_required".to_string()),
                dry_run: true,
                backfill_db_mirror: false,
                index_vectors: false,
                reconcile_db_mirror: false,
                verify_audit_chain: false,
                prune_export_cache: false,
                max_export_age_hours: None,
                purge_expired_before: None,
            }),
        )
        .await
        .expect_err("clean reconciliation gate requires reconcile_db_mirror");
        assert_eq!(error.0, StatusCode::BAD_REQUEST);
        assert!(error.1.0.error.contains("reconcile_db_mirror=true"));

        let audit_events =
            read_all_audit_events(temp.path(), "tenant-a").expect("audit events read");
        assert!(audit_events.iter().all(|event| event.kind != "maintenance"));
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn maintenance_reconciliation_can_fail_closed_on_blocking_gaps() {
        use ironclaw::db::libsql::LibSqlBackend;

        let temp = tempfile::tempdir().expect("temp dir");
        let db_temp = tempfile::tempdir().expect("db temp dir");
        let db_path = db_temp.path().join("trace-reconcile-fail-closed-gaps.db");
        let db = Arc::new(
            LibSqlBackend::new_local(&db_path)
                .await
                .expect("create libsql mirror"),
        );
        db.run_migrations().await.expect("run migrations");
        let state = test_state_with_required_db_reconciliation_clean(
            temp.path().to_path_buf(),
            db.clone() as Arc<dyn Database>,
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
        db.update_trace_submission_status(
            "tenant-a",
            submission_id,
            StorageTraceCorpusStatus::Revoked,
            "test-reconciler",
            Some("simulate missed reconciliation repair"),
        )
        .await
        .expect("DB source status can be changed");

        let error = maintenance_handler(
            State(state),
            auth_headers("review-token-a"),
            Json(TraceMaintenanceRequest {
                purpose: Some("test_reconciliation_fail_closed".to_string()),
                dry_run: true,
                backfill_db_mirror: false,
                index_vectors: false,
                reconcile_db_mirror: true,
                verify_audit_chain: false,
                prune_export_cache: false,
                max_export_age_hours: None,
                purge_expired_before: None,
            }),
        )
        .await
        .expect_err("required clean reconciliation fails closed on drift");
        assert_eq!(error.0, StatusCode::CONFLICT);
        assert!(error.1.0.error.contains("status_mismatches=1"));

        let audit_events =
            read_all_audit_events(temp.path(), "tenant-a").expect("audit events read");
        assert!(audit_events.iter().any(|event| {
            event.kind == "maintenance"
                && event
                    .reason
                    .as_deref()
                    .is_some_and(|reason| reason.contains("test_reconciliation_fail_closed"))
        }));
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn maintenance_reconciliation_splits_db_export_manifest_counts_by_kind() {
        use ironclaw::db::libsql::LibSqlBackend;

        let temp = tempfile::tempdir().expect("temp dir");
        let db_temp = tempfile::tempdir().expect("db temp dir");
        let db_path = db_temp.path().join("trace-reconcile-export-kinds.db");
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
        let source_submission_id = Uuid::new_v4();
        for (artifact_kind, purpose_code) in [
            (
                StorageTraceObjectArtifactKind::ExportArtifact,
                Some("debug_replay".to_string()),
            ),
            (
                StorageTraceObjectArtifactKind::BenchmarkArtifact,
                Some("benchmark_conversion".to_string()),
            ),
            (
                StorageTraceObjectArtifactKind::ExportArtifact,
                Some("ranker_training_candidates_export".to_string()),
            ),
            (
                StorageTraceObjectArtifactKind::WorkerIntermediate,
                Some("operator_probe".to_string()),
            ),
        ] {
            db.upsert_trace_export_manifest(StorageTraceExportManifestWrite {
                tenant_id: "tenant-a".to_string(),
                export_manifest_id: Uuid::new_v4(),
                artifact_kind,
                purpose_code,
                audit_event_id: None,
                source_submission_ids: vec![source_submission_id],
                source_submission_ids_hash: "sha256:source-list".to_string(),
                item_count: 0,
                generated_at: Utc::now(),
            })
            .await
            .expect("export manifest can be seeded");
        }

        let Json(reconciliation_response) = maintenance_handler(
            State(state),
            auth_headers("review-token-a"),
            Json(TraceMaintenanceRequest {
                purpose: Some("test_export_kind_reconciliation".to_string()),
                dry_run: true,
                backfill_db_mirror: false,
                index_vectors: false,
                reconcile_db_mirror: true,
                verify_audit_chain: false,
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
        assert_eq!(reconciliation.db_export_manifest_count, 4);
        assert_eq!(reconciliation.db_replay_export_manifest_count, 1);
        assert_eq!(reconciliation.db_benchmark_export_manifest_count, 1);
        assert_eq!(reconciliation.db_ranker_export_manifest_count, 1);
        assert_eq!(reconciliation.db_other_export_manifest_count, 1);
        assert_eq!(reconciliation.file_replay_export_manifest_count, 0);
        assert!(!reconciliation.replay_export_manifest_reader_parity_ok);
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn maintenance_reconciliation_reports_db_derived_records_missing_in_files() {
        use ironclaw::db::libsql::LibSqlBackend;

        let temp = tempfile::tempdir().expect("temp dir");
        let db_temp = tempfile::tempdir().expect("db temp dir");
        let db_path = db_temp.path().join("trace-reconcile-derived-missing.db");
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
        let derived_path = temp
            .path()
            .join("tenants")
            .join(tenant_storage_key("tenant-a"))
            .join("derived")
            .join(format!("{submission_id}.json"));
        std::fs::remove_file(derived_path).expect("remove file-backed derived record");

        let Json(reconciliation_response) = maintenance_handler(
            State(state),
            auth_headers("review-token-a"),
            Json(TraceMaintenanceRequest {
                purpose: Some("test_missing_derived_file_reconciliation".to_string()),
                dry_run: true,
                backfill_db_mirror: false,
                index_vectors: false,
                reconcile_db_mirror: true,
                verify_audit_chain: false,
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
        assert!(
            reconciliation
                .missing_derived_submission_ids_in_db
                .is_empty()
        );
        assert_eq!(
            reconciliation.missing_derived_submission_ids_in_files,
            vec![submission_id]
        );
        assert!(reconciliation.derived_status_mismatches.is_empty());
        assert!(reconciliation.derived_hash_mismatches.is_empty());
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn maintenance_reconciliation_reports_derived_status_and_hash_mismatches() {
        use ironclaw::db::libsql::LibSqlBackend;

        let temp = tempfile::tempdir().expect("temp dir");
        let db_temp = tempfile::tempdir().expect("db temp dir");
        let db_path = db_temp.path().join("trace-reconcile-derived-mismatch.db");
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
        let mut derived = read_derived_record(temp.path(), "tenant-a", submission_id)
            .expect("derived record reads")
            .expect("derived record exists");
        derived.status = TraceCorpusStatus::Revoked;
        derived.canonical_summary_hash = "sha256:file-side-derived-mismatch".to_string();
        write_derived_record(temp.path(), &derived).expect("derived record can be tampered");

        let Json(reconciliation_response) = maintenance_handler(
            State(state),
            auth_headers("review-token-a"),
            Json(TraceMaintenanceRequest {
                purpose: Some("test_derived_mismatch_reconciliation".to_string()),
                dry_run: true,
                backfill_db_mirror: false,
                index_vectors: false,
                reconcile_db_mirror: true,
                verify_audit_chain: false,
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
        assert!(
            reconciliation
                .missing_derived_submission_ids_in_files
                .is_empty()
        );
        assert_eq!(reconciliation.derived_status_mismatches.len(), 1);
        assert_eq!(
            reconciliation.derived_status_mismatches[0].submission_id,
            submission_id
        );
        assert_eq!(
            reconciliation.derived_status_mismatches[0].file_status,
            StorageTraceDerivedStatus::Revoked
        );
        assert_eq!(
            reconciliation.derived_status_mismatches[0].db_status,
            StorageTraceDerivedStatus::Current
        );
        assert_eq!(reconciliation.derived_hash_mismatches.len(), 1);
        assert_eq!(
            reconciliation.derived_hash_mismatches[0].submission_id,
            submission_id
        );
        assert_eq!(
            reconciliation.derived_hash_mismatches[0].file_canonical_summary_hash,
            "sha256:file-side-derived-mismatch"
        );
        assert!(
            reconciliation.derived_hash_mismatches[0]
                .db_canonical_summary_hash
                .as_deref()
                .is_some_and(|hash| hash.starts_with("sha256:"))
        );
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn maintenance_reconciliation_reports_missing_eligible_vector_entries() {
        use ironclaw::db::libsql::LibSqlBackend;

        let temp = tempfile::tempdir().expect("temp dir");
        let db_temp = tempfile::tempdir().expect("db temp dir");
        let db_path = db_temp.path().join("trace-reconcile-missing-vector.db");
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

        let Json(reconciliation_response) = maintenance_handler(
            State(state),
            auth_headers("review-token-a"),
            Json(TraceMaintenanceRequest {
                purpose: Some("test_missing_vector_reconciliation".to_string()),
                dry_run: true,
                backfill_db_mirror: false,
                index_vectors: false,
                reconcile_db_mirror: true,
                verify_audit_chain: false,
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
        assert_eq!(reconciliation.active_vector_entries, 0);
        assert_eq!(
            reconciliation.accepted_current_derived_without_active_vector_entry,
            vec![submission_id]
        );
        assert_eq!(reconciliation.invalid_active_vector_entries, 0);
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn maintenance_reconciliation_reports_unreadable_active_object_refs() {
        use ironclaw::db::libsql::LibSqlBackend;

        let temp = tempfile::tempdir().expect("temp dir");
        let db_temp = tempfile::tempdir().expect("db temp dir");
        let db_path = db_temp.path().join("trace-reconcile-object-ref.db");
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
        let object_ref = db
            .get_latest_active_trace_object_ref(
                "tenant-a",
                submission_id,
                StorageTraceObjectArtifactKind::SubmittedEnvelope,
            )
            .await
            .expect("active object ref reads")
            .expect("active submitted envelope object ref exists");
        let object_path = trace_object_ref_file_path(temp.path(), &object_ref.object_key)
            .expect("file object ref path is safe");
        std::fs::remove_file(&object_path).expect("remove object ref body");

        let Json(reconciliation_response) = maintenance_handler(
            State(state),
            auth_headers("review-token-a"),
            Json(TraceMaintenanceRequest {
                purpose: Some("test_unreadable_object_ref_reconciliation".to_string()),
                dry_run: true,
                backfill_db_mirror: false,
                index_vectors: false,
                reconcile_db_mirror: true,
                verify_audit_chain: false,
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
        assert!(
            reconciliation
                .accepted_without_active_envelope_object_ref
                .is_empty()
        );
        assert_eq!(
            reconciliation.unreadable_active_envelope_object_refs,
            vec![submission_id]
        );
        assert!(
            reconciliation
                .hash_mismatched_active_envelope_object_refs
                .is_empty()
        );
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn maintenance_reconciliation_reports_hash_mismatched_active_object_refs() {
        use ironclaw::db::libsql::LibSqlBackend;

        let temp = tempfile::tempdir().expect("temp dir");
        let db_temp = tempfile::tempdir().expect("db temp dir");
        let db_path = db_temp.path().join("trace-reconcile-hash-mismatch.db");
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
        let object_ref = db
            .get_latest_active_trace_object_ref(
                "tenant-a",
                submission_id,
                StorageTraceObjectArtifactKind::SubmittedEnvelope,
            )
            .await
            .expect("active object ref reads")
            .expect("active submitted envelope object ref exists");
        let object_path = trace_object_ref_file_path(temp.path(), &object_ref.object_key)
            .expect("file object ref path is safe");
        std::fs::write(&object_path, "{}").expect("corrupt object ref body");

        let Json(reconciliation_response) = maintenance_handler(
            State(state),
            auth_headers("review-token-a"),
            Json(TraceMaintenanceRequest {
                purpose: Some("test_hash_mismatched_object_ref_reconciliation".to_string()),
                dry_run: true,
                backfill_db_mirror: false,
                index_vectors: false,
                reconcile_db_mirror: true,
                verify_audit_chain: false,
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
        assert!(
            reconciliation
                .accepted_without_active_envelope_object_ref
                .is_empty()
        );
        assert!(
            reconciliation
                .unreadable_active_envelope_object_refs
                .is_empty()
        );
        assert_eq!(
            reconciliation.hash_mismatched_active_envelope_object_refs,
            vec![submission_id]
        );
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
            allowed_uses: vec!["ranking_model_training".to_string()],
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
            allowed_uses: vec!["debugging".to_string()],
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
                purpose: None,
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

        let Json(purpose_list) = list_traces_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            Query(TraceListQuery {
                status: None,
                limit: Some(10),
                purpose: Some("db_metadata_benchmark".to_string()),
                coverage_tag: None,
                tool: None,
                privacy_risk: None,
                consent_scope: None,
            }),
        )
        .await
        .expect("trace list filters by DB export purpose");
        assert_eq!(purpose_list.len(), 1);
        assert_eq!(purpose_list[0].submission_id, accepted_id);

        let Json(candidates) = ranker_training_candidates_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            Query(RankerTrainingExportQuery {
                limit: Some(10),
                purpose: Some("db_metadata_ranker".to_string()),
                status: None,
                consent_scope: Some("ranking-training".to_string()),
                privacy_risk: Some(ResidualPiiRisk::Low),
            }),
        )
        .await
        .expect("ranker candidates can read DB metadata");
        assert_eq!(candidates.item_count, 1);
        assert_eq!(candidates.candidates[0].submission_id, accepted_id);
        assert_eq!(candidates.candidates[0].tool_sequence, vec!["shell"]);

        let credit_events = read_all_credit_events(temp.path(), "tenant-a")
            .expect("DB-backed utility credits read");
        assert!(credit_events.iter().any(|event| {
            event.submission_id == accepted_id
                && event.auth_principal_ref == principal_storage_ref("token-a")
                && event.event_type == TraceCreditLedgerEventType::BenchmarkConversion
        }));
        assert!(credit_events.iter().any(|event| {
            event.submission_id == accepted_id
                && event.auth_principal_ref == principal_storage_ref("token-a")
                && event.event_type == TraceCreditLedgerEventType::TrainingUtility
        }));
        assert!(
            credit_events
                .iter()
                .all(|event| event.submission_id != quarantined_id)
        );
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
    async fn replay_export_can_require_active_db_object_ref_for_body_reads() {
        use ironclaw::db::libsql::LibSqlBackend;

        let temp = tempfile::tempdir().expect("temp dir");
        let db_temp = tempfile::tempdir().expect("db temp dir");
        let db_path = db_temp.path().join("trace-mirror-require-object-ref.db");
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

        let invalidation_counts = db
            .invalidate_trace_submission_artifacts(
                "tenant-a",
                submission_id,
                StorageTraceDerivedStatus::Current,
            )
            .await
            .expect("invalidate submitted envelope object ref");
        assert_eq!(invalidation_counts.object_refs_invalidated, 1);

        let fallback_state =
            test_state_with_db_replay_export_reads(temp.path().to_path_buf(), Some(db.clone()));
        let Json(fallback_export) = dataset_replay_handler(
            State(fallback_state),
            auth_headers("review-token-a"),
            Query(DatasetExportQuery {
                limit: Some(10),
                purpose: Some("fallback_without_object_ref".to_string()),
                status: Some(TraceCorpusStatus::Accepted),
                privacy_risk: Some(ResidualPiiRisk::Low),
                consent_scope: None,
            }),
        )
        .await
        .expect("compatibility mode can fall back to file-backed envelope body");
        assert_eq!(fallback_export.item_count, 1);
        assert_eq!(fallback_export.items[0].submission_id, submission_id);
        assert_eq!(fallback_export.items[0].object_ref_id, None);

        let fail_closed_state = test_state_with_db_replay_export_reads_require_object_refs(
            temp.path().to_path_buf(),
            Some(db),
        );
        let error = dataset_replay_handler(
            State(fail_closed_state),
            auth_headers("review-token-a"),
            Query(DatasetExportQuery {
                limit: Some(10),
                purpose: Some("require_object_ref".to_string()),
                status: Some(TraceCorpusStatus::Accepted),
                privacy_risk: Some(ResidualPiiRisk::Low),
                consent_scope: None,
            }),
        )
        .await
        .expect_err("missing active DB object ref prevents replay body read");
        assert_eq!(error.0, StatusCode::INTERNAL_SERVER_ERROR);
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
    async fn replay_export_reads_service_local_object_store_ref_without_file_objects() {
        use ironclaw::db::libsql::LibSqlBackend;

        let temp = tempfile::tempdir().expect("temp dir");
        let artifact_root = temp.path().join("service-object-store");
        let artifact_store = test_artifact_store(&artifact_root);
        let configured_store = ConfiguredTraceArtifactStore::new(
            TRACE_COMMONS_SERVICE_LOCAL_ENCRYPTED_OBJECT_STORE,
            artifact_store,
        );
        let db_temp = tempfile::tempdir().expect("db temp dir");
        let db_path = db_temp
            .path()
            .join("trace-mirror-replay-export-service-object-store.db");
        let db = Arc::new(
            LibSqlBackend::new_local(&db_path)
                .await
                .expect("create libsql mirror"),
        );
        db.run_migrations().await.expect("run migrations");
        let submit_state = test_state_with_configured_artifact_store_policies_and_export_guardrails(
            temp.path().to_path_buf(),
            Some(db.clone() as Arc<dyn Database>),
            Some(configured_store.clone()),
            false,
            false,
            false,
            false,
            false,
            false,
            BTreeMap::new(),
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
                .expect("submission dual-writes to DB mirror and service object store");
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
            TRACE_COMMONS_SERVICE_LOCAL_ENCRYPTED_OBJECT_STORE
        );
        assert!(object_ref.content_sha256.starts_with("sha256:"));

        let tenant_dir = temp
            .path()
            .join("tenants")
            .join(tenant_storage_key("tenant-a"));
        std::fs::remove_dir_all(tenant_dir.join("metadata")).expect("remove file metadata");
        std::fs::remove_dir_all(tenant_dir.join("derived")).expect("remove derived metadata");
        std::fs::remove_dir_all(tenant_dir.join("objects")).expect("remove plaintext objects");

        let replay_state = test_state_with_configured_artifact_store_policies_and_export_guardrails(
            temp.path().to_path_buf(),
            Some(db),
            Some(configured_store),
            false,
            false,
            true,
            true,
            false,
            false,
            BTreeMap::new(),
            false,
            false,
        );
        let Json(export) = dataset_replay_handler(
            State(replay_state),
            auth_headers("review-token-a"),
            Query(DatasetExportQuery {
                limit: Some(10),
                purpose: Some("db_service_object_store_replay_export".to_string()),
                status: Some(TraceCorpusStatus::Accepted),
                privacy_risk: Some(ResidualPiiRisk::Low),
                consent_scope: None,
            }),
        )
        .await
        .expect("replay export reads service object store through DB object ref");
        assert_eq!(export.item_count, 1);
        assert_eq!(export.items[0].submission_id, submission_id);
        assert_eq!(export.items[0].required_tools, vec!["shell"]);
        assert!(export.items[0].object_ref_id.is_some());
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn review_decision_reads_service_local_object_ref_when_db_reviewer_reads_enabled() {
        use ironclaw::db::libsql::LibSqlBackend;

        let temp = tempfile::tempdir().expect("temp dir");
        let artifact_root = temp.path().join("review-service-object-store");
        let artifact_store = test_artifact_store(&artifact_root);
        let configured_store = ConfiguredTraceArtifactStore::new(
            TRACE_COMMONS_SERVICE_LOCAL_ENCRYPTED_OBJECT_STORE,
            artifact_store,
        );
        let db_temp = tempfile::tempdir().expect("db temp dir");
        let db_path = db_temp.path().join("trace-review-service-object-store.db");
        let db = Arc::new(
            LibSqlBackend::new_local(&db_path)
                .await
                .expect("create libsql mirror"),
        );
        db.run_migrations().await.expect("run migrations");
        let submit_state = test_state_with_configured_artifact_store_policies_and_export_guardrails(
            temp.path().to_path_buf(),
            Some(db.clone() as Arc<dyn Database>),
            Some(configured_store.clone()),
            false,
            false,
            false,
            false,
            false,
            false,
            BTreeMap::new(),
            false,
            false,
        );
        let envelope = sample_envelope().await;
        let submission_id = envelope.submission_id;

        let Json(receipt) =
            submit_trace_handler(State(submit_state), auth_headers("token-a"), Json(envelope))
                .await
                .expect("quarantined submission dual-writes object ref");
        assert_eq!(receipt.status, "quarantined");
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
            TRACE_COMMONS_SERVICE_LOCAL_ENCRYPTED_OBJECT_STORE
        );

        let mut record = read_submission_record(temp.path(), "tenant-a", submission_id)
            .expect("file record reads")
            .expect("file record exists");
        record.object_key = "missing-review-body.json".to_string();
        record.artifact_receipt = None;
        write_submission_record(temp.path(), &record).expect("file record points at missing body");

        let review_state = test_state_with_configured_artifact_store_policies_and_export_guardrails(
            temp.path().to_path_buf(),
            Some(db.clone() as Arc<dyn Database>),
            Some(configured_store),
            false,
            true,
            false,
            false,
            false,
            false,
            BTreeMap::new(),
            false,
            false,
        );
        let Json(review_receipt) = review_decision_handler(
            State(review_state.clone()),
            auth_headers("review-token-a"),
            AxumPath(submission_id),
            Json(TraceReviewDecisionRequest {
                decision: TraceReviewDecision::Approve,
                reason: Some("db object-ref-backed review".to_string()),
                credit_points_pending: Some(1.25),
            }),
        )
        .await
        .expect("review decision reads body from DB object ref");
        assert_eq!(review_receipt.status, "accepted");

        let submitted_ref_after_review = db
            .get_latest_active_trace_object_ref(
                "tenant-a",
                submission_id,
                StorageTraceObjectArtifactKind::SubmittedEnvelope,
            )
            .await
            .expect("submitted object ref reads after review")
            .expect("submitted envelope object ref remains active after review");
        assert_eq!(
            submitted_ref_after_review.object_ref_id,
            object_ref.object_ref_id
        );

        let reviewed_ref = db
            .get_latest_active_trace_object_ref(
                "tenant-a",
                submission_id,
                StorageTraceObjectArtifactKind::ReviewSnapshot,
            )
            .await
            .expect("review snapshot object ref reads")
            .expect("review snapshot object ref exists");
        assert_ne!(reviewed_ref.object_ref_id, object_ref.object_ref_id);
        assert_eq!(
            reviewed_ref.object_store,
            TRACE_COMMONS_SERVICE_LOCAL_ENCRYPTED_OBJECT_STORE
        );
        let reviewed_envelope =
            read_envelope_from_object_ref(review_state.as_ref(), "tenant-a", &reviewed_ref)
                .expect("reviewed envelope reads through DB object ref");
        assert_eq!(reviewed_envelope.value.credit_points_pending, 1.25);

        let db_audit_events = db
            .list_trace_audit_events("tenant-a")
            .await
            .expect("DB audit events read");
        assert!(db_audit_events.iter().any(|event| {
            event.action == StorageTraceAuditAction::Read
                && event.submission_id == Some(submission_id)
                && event.object_ref_id == Some(object_ref.object_ref_id)
                && event.reason.as_deref() == Some("surface=review_decision")
        }));
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn review_decision_can_use_db_metadata_without_file_record() {
        use ironclaw::db::libsql::LibSqlBackend;

        let temp = tempfile::tempdir().expect("temp dir");
        let db_temp = tempfile::tempdir().expect("db temp dir");
        let db_path = db_temp.path().join("trace-review-db-metadata.db");
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
        let envelope = sample_envelope().await;
        let submission_id = envelope.submission_id;

        let Json(receipt) =
            submit_trace_handler(State(submit_state), auth_headers("token-a"), Json(envelope))
                .await
                .expect("quarantined submission dual-writes to DB mirror");
        assert_eq!(receipt.status, "quarantined");
        let canonical_summary_hash = db
            .list_trace_submissions("tenant-a")
            .await
            .expect("DB submissions read")
            .into_iter()
            .find(|record| record.submission_id == submission_id)
            .and_then(|record| record.canonical_summary_hash)
            .expect("DB submission retains canonical summary hash");
        let submitted_ref = db
            .get_latest_active_trace_object_ref(
                "tenant-a",
                submission_id,
                StorageTraceObjectArtifactKind::SubmittedEnvelope,
            )
            .await
            .expect("submitted object ref reads")
            .expect("submitted envelope object ref exists");

        let tenant_dir = temp
            .path()
            .join("tenants")
            .join(tenant_storage_key("tenant-a"));
        let metadata_path = submission_metadata_path(temp.path(), "tenant-a", submission_id);
        std::fs::remove_dir_all(tenant_dir.join("metadata")).expect("remove file metadata");
        std::fs::remove_dir_all(tenant_dir.join("derived")).expect("remove derived metadata");

        let review_state = test_state_with_db_reviewer_reads_require_object_refs(
            temp.path().to_path_buf(),
            Some(db.clone()),
        );
        let Json(review_receipt) = review_decision_handler(
            State(review_state),
            auth_headers("review-token-a"),
            AxumPath(submission_id),
            Json(TraceReviewDecisionRequest {
                decision: TraceReviewDecision::Approve,
                reason: Some("db metadata-backed review".to_string()),
                credit_points_pending: Some(1.5),
            }),
        )
        .await
        .expect("review decision can read metadata and body from DB");
        assert_eq!(review_receipt.status, "accepted");

        let reviewed_submission = db
            .list_trace_submissions("tenant-a")
            .await
            .expect("DB submissions read after review")
            .into_iter()
            .find(|record| record.submission_id == submission_id)
            .expect("reviewed DB submission exists");
        assert_eq!(
            reviewed_submission.status,
            StorageTraceCorpusStatus::Accepted
        );
        assert_eq!(
            reviewed_submission.canonical_summary_hash.as_deref(),
            Some(canonical_summary_hash.as_str())
        );
        assert_eq!(reviewed_submission.credit_points_pending, Some(1.5));
        assert!(
            !metadata_path.exists(),
            "DB-backed review should not recreate missing file metadata"
        );

        let reviewed_ref = db
            .get_latest_active_trace_object_ref(
                "tenant-a",
                submission_id,
                StorageTraceObjectArtifactKind::ReviewSnapshot,
            )
            .await
            .expect("review snapshot object ref reads")
            .expect("review snapshot object ref exists");
        assert_eq!(
            reviewed_ref.artifact_kind,
            StorageTraceObjectArtifactKind::ReviewSnapshot
        );
        let db_audit_events = db
            .list_trace_audit_events("tenant-a")
            .await
            .expect("DB audit events read");
        assert!(db_audit_events.iter().any(|event| {
            event.action == StorageTraceAuditAction::Read
                && event.submission_id == Some(submission_id)
                && event.object_ref_id == Some(submitted_ref.object_ref_id)
                && event.reason.as_deref() == Some("surface=review_decision")
        }));
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn review_decision_can_require_active_db_object_ref_for_body_reads() {
        use ironclaw::db::libsql::LibSqlBackend;

        let temp = tempfile::tempdir().expect("temp dir");
        let db_temp = tempfile::tempdir().expect("db temp dir");
        let db_path = db_temp.path().join("trace-review-require-object-ref.db");
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
        let envelope = sample_envelope().await;
        let submission_id = envelope.submission_id;

        let Json(receipt) =
            submit_trace_handler(State(submit_state), auth_headers("token-a"), Json(envelope))
                .await
                .expect("quarantined submission dual-writes object ref");
        assert_eq!(receipt.status, "quarantined");

        let invalidation_counts = db
            .invalidate_trace_submission_artifacts(
                "tenant-a",
                submission_id,
                StorageTraceDerivedStatus::Current,
            )
            .await
            .expect("invalidate submitted envelope object ref");
        assert_eq!(invalidation_counts.object_refs_invalidated, 1);

        let fail_closed_state = test_state_with_db_reviewer_reads_require_object_refs(
            temp.path().to_path_buf(),
            Some(db.clone()),
        );
        let error = review_decision_handler(
            State(fail_closed_state),
            auth_headers("review-token-a"),
            AxumPath(submission_id),
            Json(TraceReviewDecisionRequest {
                decision: TraceReviewDecision::Approve,
                reason: Some("require reviewer object ref".to_string()),
                credit_points_pending: Some(0.5),
            }),
        )
        .await
        .expect_err("missing active DB object ref prevents review body read");
        assert_eq!(error.0, StatusCode::INTERNAL_SERVER_ERROR);

        let fallback_state = test_state_with_db_reviewer_reads(temp.path().to_path_buf(), Some(db));
        let Json(fallback_receipt) = review_decision_handler(
            State(fallback_state),
            auth_headers("review-token-a"),
            AxumPath(submission_id),
            Json(TraceReviewDecisionRequest {
                decision: TraceReviewDecision::Approve,
                reason: Some("fallback reviewer read".to_string()),
                credit_points_pending: Some(0.5),
            }),
        )
        .await
        .expect("compatibility mode can fall back to file-backed envelope body");
        assert_eq!(fallback_receipt.status, "accepted");
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
            State(state.clone()),
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

        let Json(response) = maintenance_handler(
            State(state),
            auth_headers("review-token-a"),
            Json(TraceMaintenanceRequest {
                purpose: Some("revocation_invalidated_export_reconciliation".to_string()),
                dry_run: true,
                backfill_db_mirror: false,
                index_vectors: false,
                reconcile_db_mirror: true,
                verify_audit_chain: false,
                prune_export_cache: false,
                max_export_age_hours: None,
                purge_expired_before: None,
            }),
        )
        .await
        .expect("reviewer can reconcile after revocation invalidation");
        let reconciliation = response
            .db_reconciliation
            .expect("reconciliation report exists");
        assert!(
            reconciliation
                .active_derived_submission_ids_for_invalid_sources
                .is_empty()
        );
        assert!(
            reconciliation
                .active_export_manifest_ids_for_invalid_sources
                .is_empty()
        );
        assert!(
            reconciliation
                .active_export_manifest_items_for_invalid_sources
                .is_empty()
        );
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn db_only_revocation_requires_original_contributor_or_reviewer() {
        use ironclaw::db::libsql::LibSqlBackend;

        let temp = tempfile::tempdir().expect("temp dir");
        let db_temp = tempfile::tempdir().expect("db temp dir");
        let db_path = db_temp.path().join("trace-db-only-revocation-auth.db");
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
        let tenant_dir = temp
            .path()
            .join("tenants")
            .join(tenant_storage_key("tenant-a"));
        std::fs::remove_dir_all(tenant_dir.join("metadata")).expect("remove file metadata");
        std::fs::remove_dir_all(tenant_dir.join("derived")).expect("remove file derived");

        let unauthorized = revoke_trace_handler(
            State(state.clone()),
            auth_headers("token-a-2"),
            AxumPath(submission_id),
        )
        .await
        .expect_err("same-tenant non-owner cannot revoke DB-only submission");
        assert_eq!(unauthorized.0, StatusCode::NOT_FOUND);
        let db_submission = db
            .get_trace_submission("tenant-a", submission_id)
            .await
            .expect("DB submission reads")
            .expect("DB submission exists");
        assert_eq!(db_submission.status, StorageTraceCorpusStatus::Accepted);
        assert!(
            db.list_trace_tombstones("tenant-a")
                .await
                .expect("DB tombstones read")
                .is_empty()
        );

        let status = revoke_trace_handler(
            State(state),
            auth_headers("token-a"),
            AxumPath(submission_id),
        )
        .await
        .expect("original contributor can revoke DB-only submission");
        assert_eq!(status, StatusCode::NO_CONTENT);
        let revoked = db
            .get_trace_submission("tenant-a", submission_id)
            .await
            .expect("revoked DB submission reads")
            .expect("revoked DB submission exists");
        assert_eq!(revoked.status, StorageTraceCorpusStatus::Revoked);
        let tombstones = db
            .list_trace_tombstones("tenant-a")
            .await
            .expect("DB tombstones read after owner revocation");
        assert_eq!(tombstones.len(), 1);
        assert_eq!(tombstones[0].submission_id, submission_id);
        assert_eq!(
            tombstones[0].created_by_principal_ref,
            db_submission.auth_principal_ref
        );
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
        let mut pair_source = sample_envelope().await;
        make_metadata_only_low_risk(&mut pair_source);
        pair_source.consent.scopes = vec![ConsentScope::RankingTraining];
        pair_source.value.submission_score = 0.1;
        let pair_source_id = pair_source.submission_id;

        let _ = submit_trace_handler(
            State(state.clone()),
            auth_headers("token-a"),
            Json(pair_source),
        )
        .await
        .expect("second ranker source dual-writes to DB mirror");
        let Json(vector_response) = maintenance_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            Json(TraceMaintenanceRequest {
                purpose: Some("manifest_listing_vector_index".to_string()),
                dry_run: false,
                backfill_db_mirror: false,
                index_vectors: true,
                reconcile_db_mirror: false,
                verify_audit_chain: false,
                prune_export_cache: false,
                max_export_age_hours: None,
                purge_expired_before: None,
            }),
        )
        .await
        .expect("vector metadata indexing succeeds before derived exports");
        assert_eq!(vector_response.vector_entries_indexed, 2);
        let Json(export) = dataset_replay_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            Query(DatasetExportQuery {
                limit: Some(10),
                purpose: Some("manifest_listing".to_string()),
                status: Some(TraceCorpusStatus::Accepted),
                privacy_risk: Some(ResidualPiiRisk::Low),
                consent_scope: Some("debugging-evaluation".to_string()),
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
                consent_scope: Some("debugging-evaluation".to_string()),
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
                purpose: Some("manifest_listing_ranker".to_string()),
                status: None,
                consent_scope: Some("ranking-training".to_string()),
                privacy_risk: Some(ResidualPiiRisk::Low),
            }),
        )
        .await
        .expect("ranker export succeeds");
        assert_eq!(ranker.item_count, 2);

        let Json(ranker_pairs) = ranker_training_pairs_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            Query(RankerTrainingExportQuery {
                limit: Some(10),
                purpose: Some("manifest_listing_ranker_pairs".to_string()),
                status: None,
                consent_scope: Some("ranking-training".to_string()),
                privacy_risk: Some(ResidualPiiRisk::Low),
            }),
        )
        .await
        .expect("ranker pair export succeeds");
        assert_eq!(ranker_pairs.item_count, 1);

        assert_eq!(
            db.list_trace_export_manifests("tenant-a")
                .await
                .expect("all export manifest metadata reads")
                .len(),
            4,
            "DB stores replay, benchmark, ranker candidate, and ranker pair provenance manifests"
        );
        let vector_by_submission = db
            .list_trace_vector_entries("tenant-a")
            .await
            .expect("vector metadata reads")
            .into_iter()
            .filter(|record| record.status == StorageTraceVectorEntryStatus::Active)
            .map(|record| {
                (
                    record.submission_id,
                    (record.derived_id, record.vector_entry_id),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let (derived_id, vector_entry_id) = *vector_by_submission
            .get(&submission_id)
            .expect("active vector metadata exists for derived summary");
        let (pair_source_derived_id, pair_source_vector_entry_id) = *vector_by_submission
            .get(&pair_source_id)
            .expect("active vector metadata exists for ranker pair source");
        let benchmark_items = db
            .list_trace_export_manifest_items("tenant-a", benchmark.conversion_id)
            .await
            .expect("benchmark provenance items read");
        assert_eq!(benchmark_items.len(), 1);
        assert_eq!(benchmark_items[0].derived_id, Some(derived_id));
        assert!(benchmark_items[0].object_ref_id.is_some());
        assert_eq!(benchmark_items[0].vector_entry_id, Some(vector_entry_id));
        let ranker_items = db
            .list_trace_export_manifest_items("tenant-a", ranker.export_id)
            .await
            .expect("ranker provenance items read");
        assert_eq!(ranker_items.len(), 2);
        assert!(ranker_items.iter().all(|item| item.object_ref_id.is_some()));
        assert!(ranker_items.iter().any(|item| {
            item.submission_id == submission_id
                && item.derived_id == Some(derived_id)
                && item.vector_entry_id == Some(vector_entry_id)
        }));
        assert!(ranker_items.iter().any(|item| {
            item.submission_id == pair_source_id
                && item.derived_id == Some(pair_source_derived_id)
                && item.vector_entry_id == Some(pair_source_vector_entry_id)
        }));
        let ranker_pair_items = db
            .list_trace_export_manifest_items("tenant-a", ranker_pairs.export_id)
            .await
            .expect("ranker pair provenance items read");
        assert_eq!(ranker_pair_items.len(), 2);
        assert!(
            ranker_pair_items
                .iter()
                .all(|item| item.object_ref_id.is_some())
        );
        assert!(
            ranker_pair_items
                .iter()
                .all(|item| item.vector_entry_id.is_some())
        );

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
    async fn benchmark_and_ranker_exports_write_service_local_object_refs() {
        use ironclaw::db::libsql::LibSqlBackend;

        let temp = tempfile::tempdir().expect("temp dir");
        let artifact_root = temp.path().join("service-object-store");
        let artifact_store = test_artifact_store(&artifact_root);
        let configured_store = ConfiguredTraceArtifactStore::new(
            TRACE_COMMONS_SERVICE_LOCAL_ENCRYPTED_OBJECT_STORE,
            artifact_store,
        );
        let db_temp = tempfile::tempdir().expect("db temp dir");
        let db_path = db_temp
            .path()
            .join("trace-derived-export-service-object-store.db");
        let db = Arc::new(
            LibSqlBackend::new_local(&db_path)
                .await
                .expect("create libsql mirror"),
        );
        db.run_migrations().await.expect("run migrations");
        let state = test_state_with_configured_artifact_store_policies_and_export_guardrails(
            temp.path().to_path_buf(),
            Some(db.clone() as Arc<dyn Database>),
            Some(configured_store.clone()),
            false,
            false,
            false,
            false,
            false,
            false,
            BTreeMap::new(),
            false,
            false,
        );
        let mut benchmark_source = sample_envelope().await;
        make_metadata_only_low_risk(&mut benchmark_source);
        benchmark_source.consent.scopes = vec![
            ConsentScope::DebuggingEvaluation,
            ConsentScope::RankingTraining,
        ];
        let benchmark_submission_id = benchmark_source.submission_id;
        let mut ranker_source = sample_envelope().await;
        make_metadata_only_low_risk(&mut ranker_source);
        ranker_source.consent.scopes = vec![ConsentScope::RankingTraining];

        let _ = submit_trace_handler(
            State(state.clone()),
            auth_headers("token-a"),
            Json(benchmark_source),
        )
        .await
        .expect("benchmark/ranker source submission succeeds");
        let _ = submit_trace_handler(
            State(state.clone()),
            auth_headers("token-a"),
            Json(ranker_source),
        )
        .await
        .expect("ranker source submission succeeds");

        let Json(benchmark) = benchmark_convert_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            Json(BenchmarkConversionRequest {
                limit: Some(10),
                purpose: Some("service_local_benchmark".to_string()),
                consent_scope: Some("debugging-evaluation".to_string()),
                status: Some(TraceCorpusStatus::Accepted),
                privacy_risk: Some(ResidualPiiRisk::Low),
                external_ref: None,
            }),
        )
        .await
        .expect("benchmark conversion succeeds");
        assert_eq!(benchmark.item_count, 1);
        let benchmark_items = db
            .list_trace_export_manifest_items("tenant-a", benchmark.conversion_id)
            .await
            .expect("benchmark item metadata reads");
        let benchmark_object_ref_id = benchmark_items[0]
            .object_ref_id
            .expect("benchmark item has artifact object ref");
        let benchmark_object_ref = db
            .list_trace_object_refs("tenant-a", benchmark_submission_id)
            .await
            .expect("benchmark object refs read")
            .into_iter()
            .find(|object_ref| object_ref.object_ref_id == benchmark_object_ref_id)
            .expect("benchmark artifact object ref exists");
        assert_eq!(
            benchmark_object_ref.artifact_kind,
            StorageTraceObjectArtifactKind::BenchmarkArtifact
        );
        assert_eq!(
            benchmark_object_ref.object_store,
            TRACE_COMMONS_SERVICE_LOCAL_ENCRYPTED_OBJECT_STORE
        );
        let benchmark_artifact: TraceBenchmarkConversionArtifact = configured_store
            .get_json_by_object_key(
                &tenant_storage_ref("tenant-a"),
                TraceArtifactKind::BenchmarkConversion,
                &benchmark_object_ref.object_key,
                &benchmark_object_ref.content_sha256,
            )
            .expect("benchmark artifact reads from service-local object store");
        assert_eq!(benchmark_artifact.conversion_id, benchmark.conversion_id);
        configured_store
            .get_json_by_object_key::<TraceBenchmarkConversionArtifact>(
                &tenant_storage_ref("tenant-b"),
                TraceArtifactKind::BenchmarkConversion,
                &benchmark_object_ref.object_key,
                &benchmark_object_ref.content_sha256,
            )
            .expect_err("benchmark artifact object ref is tenant-scoped");

        let Json(ranker) = ranker_training_candidates_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            Query(RankerTrainingExportQuery {
                limit: Some(10),
                purpose: Some("service_local_ranker".to_string()),
                status: Some(TraceCorpusStatus::Accepted),
                consent_scope: Some("ranking-training".to_string()),
                privacy_risk: Some(ResidualPiiRisk::Low),
            }),
        )
        .await
        .expect("ranker candidate export succeeds");
        assert_eq!(ranker.item_count, 2);
        let ranker_items = db
            .list_trace_export_manifest_items("tenant-a", ranker.export_id)
            .await
            .expect("ranker item metadata reads");
        let ranker_object_ref_id = ranker_items
            .iter()
            .find(|item| item.submission_id == benchmark_submission_id)
            .and_then(|item| item.object_ref_id)
            .expect("ranker item has artifact object ref");
        let ranker_object_ref = db
            .list_trace_object_refs("tenant-a", benchmark_submission_id)
            .await
            .expect("ranker object refs read")
            .into_iter()
            .find(|object_ref| object_ref.object_ref_id == ranker_object_ref_id)
            .expect("ranker artifact object ref exists");
        assert_eq!(
            ranker_object_ref.artifact_kind,
            StorageTraceObjectArtifactKind::ExportArtifact
        );
        assert_eq!(
            ranker_object_ref.object_store,
            TRACE_COMMONS_SERVICE_LOCAL_ENCRYPTED_OBJECT_STORE
        );
        let provenance: TraceExportProvenanceManifest = configured_store
            .get_json_by_object_key(
                &tenant_storage_ref("tenant-a"),
                TraceArtifactKind::RankerTrainingExport,
                &ranker_object_ref.object_key,
                &ranker_object_ref.content_sha256,
            )
            .expect("ranker provenance reads from service-local object store");
        assert_eq!(provenance.export_id, ranker.export_id);
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn object_primary_derived_exports_skip_plaintext_files_and_keep_db_object_refs() {
        use ironclaw::db::libsql::LibSqlBackend;

        let temp = tempfile::tempdir().expect("temp dir");
        let artifact_root = temp.path().join("service-object-store");
        let artifact_store = test_artifact_store(&artifact_root);
        let configured_store = ConfiguredTraceArtifactStore::new(
            TRACE_COMMONS_SERVICE_LOCAL_ENCRYPTED_OBJECT_STORE,
            artifact_store,
        );
        let db_temp = tempfile::tempdir().expect("db temp dir");
        let db_path = db_temp
            .path()
            .join("trace-object-primary-derived-exports.db");
        let db = Arc::new(
            LibSqlBackend::new_local(&db_path)
                .await
                .expect("create libsql mirror"),
        );
        db.run_migrations().await.expect("run migrations");
        let state = test_state_with_object_primary_derived_exports(
            temp.path().to_path_buf(),
            db.clone() as Arc<dyn Database>,
            configured_store.clone(),
        );
        let mut first_source = sample_envelope().await;
        make_metadata_only_low_risk(&mut first_source);
        first_source.consent.scopes = vec![
            ConsentScope::DebuggingEvaluation,
            ConsentScope::RankingTraining,
        ];
        let first_submission_id = first_source.submission_id;
        let mut second_source = sample_envelope().await;
        make_metadata_only_low_risk(&mut second_source);
        second_source.consent.scopes = vec![ConsentScope::RankingTraining];

        let _ = submit_trace_handler(
            State(state.clone()),
            auth_headers("token-a"),
            Json(first_source),
        )
        .await
        .expect("first source submission succeeds");
        let _ = submit_trace_handler(
            State(state.clone()),
            auth_headers("token-a"),
            Json(second_source),
        )
        .await
        .expect("second source submission succeeds");

        let Json(benchmark) = benchmark_convert_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            Json(BenchmarkConversionRequest {
                limit: Some(10),
                purpose: Some("object_primary_benchmark".to_string()),
                consent_scope: Some("debugging-evaluation".to_string()),
                status: Some(TraceCorpusStatus::Accepted),
                privacy_risk: Some(ResidualPiiRisk::Low),
                external_ref: None,
            }),
        )
        .await
        .expect("object-primary benchmark conversion succeeds");
        assert_eq!(benchmark.item_count, 1);
        assert!(
            !benchmark_artifact_path(temp.path(), "tenant-a", benchmark.conversion_id).exists()
        );
        assert!(
            !benchmark_provenance_path(temp.path(), "tenant-a", benchmark.conversion_id).exists()
        );
        let benchmark_items = db
            .list_trace_export_manifest_items("tenant-a", benchmark.conversion_id)
            .await
            .expect("benchmark manifest items read");
        let benchmark_object_ref_id = benchmark_items[0]
            .object_ref_id
            .expect("benchmark item has object ref");
        let benchmark_object_ref = db
            .list_trace_object_refs("tenant-a", first_submission_id)
            .await
            .expect("benchmark object refs read")
            .into_iter()
            .find(|object_ref| object_ref.object_ref_id == benchmark_object_ref_id)
            .expect("benchmark artifact object ref exists");
        assert_eq!(
            benchmark_object_ref.object_store,
            TRACE_COMMONS_SERVICE_LOCAL_ENCRYPTED_OBJECT_STORE
        );
        let stored_benchmark: TraceBenchmarkConversionArtifact = configured_store
            .get_json_by_object_key(
                &tenant_storage_ref("tenant-a"),
                TraceArtifactKind::BenchmarkConversion,
                &benchmark_object_ref.object_key,
                &benchmark_object_ref.content_sha256,
            )
            .expect("benchmark artifact reads from object store");
        assert_eq!(stored_benchmark.conversion_id, benchmark.conversion_id);

        let Json(updated_benchmark) = benchmark_lifecycle_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            AxumPath(benchmark.conversion_id),
            Json(BenchmarkLifecycleUpdateRequest {
                registry: Some(TraceBenchmarkRegistryPatch {
                    status: Some(TraceBenchmarkRegistryStatus::Published),
                    registry_ref: Some("benchmark-registry:object-primary".to_string()),
                    published_at: Some(Utc::now()),
                }),
                evaluation: Some(TraceBenchmarkEvaluationPatch {
                    status: Some(TraceBenchmarkEvaluationStatus::Passed),
                    evaluator_ref: Some("evaluator:object-primary".to_string()),
                    evaluated_at: Some(Utc::now()),
                    score: Some(1.0),
                    pass_count: Some(1),
                    fail_count: Some(0),
                }),
                reason: Some("object-primary lifecycle update".to_string()),
            }),
        )
        .await
        .expect("object-primary benchmark lifecycle update succeeds");
        assert_eq!(
            updated_benchmark.registry.status,
            TraceBenchmarkRegistryStatus::Published
        );
        assert!(
            !benchmark_artifact_path(temp.path(), "tenant-a", benchmark.conversion_id).exists(),
            "object-primary lifecycle update must not recreate plaintext benchmark artifact"
        );
        let updated_benchmark_object_ref = db
            .list_trace_object_refs("tenant-a", first_submission_id)
            .await
            .expect("updated benchmark object refs read")
            .into_iter()
            .find(|object_ref| object_ref.object_ref_id == benchmark_object_ref_id)
            .expect("updated benchmark artifact object ref exists");
        let stored_updated_benchmark: TraceBenchmarkConversionArtifact = configured_store
            .get_json_by_object_key(
                &tenant_storage_ref("tenant-a"),
                TraceArtifactKind::BenchmarkConversion,
                &updated_benchmark_object_ref.object_key,
                &updated_benchmark_object_ref.content_sha256,
            )
            .expect("updated benchmark artifact reads from object store");
        assert_eq!(
            stored_updated_benchmark.evaluation.status,
            TraceBenchmarkEvaluationStatus::Passed
        );

        let Json(ranker_candidates) = ranker_training_candidates_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            Query(RankerTrainingExportQuery {
                limit: Some(10),
                purpose: Some("object_primary_ranker_candidates".to_string()),
                status: Some(TraceCorpusStatus::Accepted),
                consent_scope: Some("ranking-training".to_string()),
                privacy_risk: Some(ResidualPiiRisk::Low),
            }),
        )
        .await
        .expect("object-primary ranker candidate export succeeds");
        assert_eq!(ranker_candidates.item_count, 2);
        assert!(
            !ranker_provenance_path(temp.path(), "tenant-a", ranker_candidates.export_id).exists()
        );
        let candidate_items = db
            .list_trace_export_manifest_items("tenant-a", ranker_candidates.export_id)
            .await
            .expect("ranker candidate manifest items read");
        let candidate_object_ref_id = candidate_items
            .iter()
            .find(|item| item.submission_id == first_submission_id)
            .and_then(|item| item.object_ref_id)
            .expect("ranker candidate item has object ref");
        let candidate_object_ref = db
            .list_trace_object_refs("tenant-a", first_submission_id)
            .await
            .expect("ranker candidate object refs read")
            .into_iter()
            .find(|object_ref| object_ref.object_ref_id == candidate_object_ref_id)
            .expect("ranker candidate export object ref exists");
        let candidate_provenance: TraceExportProvenanceManifest = configured_store
            .get_json_by_object_key(
                &tenant_storage_ref("tenant-a"),
                TraceArtifactKind::RankerTrainingExport,
                &candidate_object_ref.object_key,
                &candidate_object_ref.content_sha256,
            )
            .expect("ranker candidate provenance reads from object store");
        assert_eq!(candidate_provenance.export_id, ranker_candidates.export_id);

        let Json(ranker_pairs) = ranker_training_pairs_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            Query(RankerTrainingExportQuery {
                limit: Some(10),
                purpose: Some("object_primary_ranker_pairs".to_string()),
                status: Some(TraceCorpusStatus::Accepted),
                consent_scope: Some("ranking-training".to_string()),
                privacy_risk: Some(ResidualPiiRisk::Low),
            }),
        )
        .await
        .expect("object-primary ranker pair export succeeds");
        assert_eq!(ranker_pairs.item_count, 1);
        assert!(!ranker_provenance_path(temp.path(), "tenant-a", ranker_pairs.export_id).exists());
        let pair_items = db
            .list_trace_export_manifest_items("tenant-a", ranker_pairs.export_id)
            .await
            .expect("ranker pair manifest items read");
        let pair_object_ref_id = pair_items
            .iter()
            .find(|item| item.submission_id == first_submission_id)
            .and_then(|item| item.object_ref_id)
            .expect("ranker pair item has object ref");
        let pair_object_ref = db
            .list_trace_object_refs("tenant-a", first_submission_id)
            .await
            .expect("ranker pair object refs read")
            .into_iter()
            .find(|object_ref| object_ref.object_ref_id == pair_object_ref_id)
            .expect("ranker pair export object ref exists");
        let pair_provenance: TraceExportProvenanceManifest = configured_store
            .get_json_by_object_key(
                &tenant_storage_ref("tenant-a"),
                TraceArtifactKind::RankerTrainingExport,
                &pair_object_ref.object_key,
                &pair_object_ref.content_sha256,
            )
            .expect("ranker pair provenance reads from object store");
        assert_eq!(pair_provenance.export_id, ranker_pairs.export_id);

        let Json(filtered_records) = list_traces_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            Query(TraceListQuery {
                status: None,
                limit: Some(10),
                purpose: Some("object_primary_ranker_candidates".to_string()),
                coverage_tag: None,
                tool: None,
                privacy_risk: None,
                consent_scope: None,
            }),
        )
        .await
        .expect("DB-backed purpose filter sees object-primary provenance");
        assert_eq!(filtered_records.len(), 2);

        revoke_trace_handler(
            State(state.clone()),
            auth_headers("token-a"),
            AxumPath(first_submission_id),
        )
        .await
        .expect("revocation invalidates DB manifest metadata");
        let invalidated_manifest_count = db
            .list_trace_export_manifests("tenant-a")
            .await
            .expect("manifest metadata reads")
            .into_iter()
            .filter(|manifest| manifest.invalidated_at.is_some())
            .count();
        assert_eq!(invalidated_manifest_count, 3);
        assert!(
            !benchmark_artifact_path(temp.path(), "tenant-a", benchmark.conversion_id).exists(),
            "object-primary source invalidation must not recreate plaintext benchmark artifact"
        );
        let revoked_benchmark_manifest = db
            .list_trace_export_manifests("tenant-a")
            .await
            .expect("benchmark manifest metadata reads")
            .into_iter()
            .find(|manifest| manifest.export_manifest_id == benchmark.conversion_id)
            .expect("benchmark manifest exists");
        assert!(revoked_benchmark_manifest.invalidated_at.is_some());
        let revoked_benchmark_items = db
            .list_trace_export_manifest_items("tenant-a", benchmark.conversion_id)
            .await
            .expect("benchmark manifest items read after revocation");
        assert_eq!(revoked_benchmark_items.len(), 1);
        assert!(revoked_benchmark_items[0].source_invalidated_at.is_some());
        assert_eq!(
            revoked_benchmark_items[0].source_invalidation_reason,
            Some(StorageTraceExportManifestItemInvalidationReason::Revoked)
        );
        let revoked_benchmark_object_ref = db
            .list_trace_object_refs("tenant-a", first_submission_id)
            .await
            .expect("revoked benchmark object refs read")
            .into_iter()
            .find(|object_ref| object_ref.object_ref_id == benchmark_object_ref_id)
            .expect("revoked benchmark artifact object ref exists");
        assert!(revoked_benchmark_object_ref.invalidated_at.is_some());
        let stored_revoked_benchmark: TraceBenchmarkConversionArtifact = configured_store
            .get_json_by_object_key(
                &tenant_storage_ref("tenant-a"),
                TraceArtifactKind::BenchmarkConversion,
                &revoked_benchmark_object_ref.object_key,
                &revoked_benchmark_object_ref.content_sha256,
            )
            .expect("revoked benchmark artifact reads from object store");
        assert_eq!(
            stored_revoked_benchmark.registry.status,
            TraceBenchmarkRegistryStatus::Revoked
        );
        assert_eq!(
            stored_revoked_benchmark
                .registry
                .revocation_reason
                .as_deref(),
            Some("contributor_revocation")
        );
        assert_eq!(
            stored_revoked_benchmark.evaluation.status,
            TraceBenchmarkEvaluationStatus::Inconclusive
        );
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
                purpose: None,
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

        let Json(credit_summary) =
            credit_handler(State(audit_state.clone()), auth_headers("token-a"))
                .await
                .expect("contributor credit summary mirrors audit event");
        assert_eq!(credit_summary.accepted, 1);
        let Json(contributor_credit_events) =
            credit_events_handler(State(audit_state.clone()), auth_headers("token-a"))
                .await
                .expect("contributor credit events mirror audit event");
        assert_eq!(contributor_credit_events.len(), 0);
        let Json(status_updates) = submission_status_handler(
            State(audit_state.clone()),
            auth_headers("token-a"),
            Json(TraceSubmissionStatusRequest {
                submission_ids: vec![submission_id],
            }),
        )
        .await
        .expect("contributor status sync mirrors audit event");
        assert_eq!(status_updates.len(), 1);

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
        let db_content_read_audit = db_audit_events
            .iter()
            .find(|event| {
                event.action == StorageTraceAuditAction::Read
                    && event.submission_id == Some(submission_id)
                    && event.object_ref_id == Some(active_object_ref.object_ref_id)
                    && event.reason.as_deref().is_some_and(|reason| {
                        reason.contains("surface=replay_dataset_export")
                            && reason.contains("purpose=db_audit_export")
                    })
            })
            .expect(
                "DB content-read audit event should name the object ref that passed the read gate",
            );
        assert!(
            db_content_read_audit
                .previous_event_hash
                .as_deref()
                .is_some_and(|hash| hash.starts_with("sha256:")),
            "DB content-read audit events should retain their previous file audit hash"
        );
        assert!(
            db_content_read_audit
                .event_hash
                .as_deref()
                .is_some_and(|hash| hash.starts_with("sha256:")),
            "DB content-read audit events should retain the file audit hash chain"
        );
        let credit_audit_event = db_audit_events
            .iter()
            .find(|event| {
                event.action == StorageTraceAuditAction::CreditMutate
                    && event.submission_id == Some(submission_id)
            })
            .expect("credit mutation audit event is mirrored");
        match &credit_audit_event.metadata {
            StorageTraceAuditSafeMetadata::CreditMutation {
                event_type,
                credit_points_delta_micros,
                reason_hash,
                external_ref_hash,
            } => {
                assert_eq!(*event_type, StorageTraceCreditEventType::ReviewerBonus);
                assert_eq!(*credit_points_delta_micros, 250_000);
                assert!(reason_hash.starts_with("sha256:"));
                assert!(external_ref_hash.is_none());
            }
            metadata => panic!("unexpected credit audit metadata: {metadata:?}"),
        }

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
            event.kind == "read"
                && event.reason.as_deref() == Some("surface=contributor_credit;item_count=1")
        }));
        assert!(events.iter().any(|event| {
            event.kind == "read"
                && event.reason.as_deref() == Some("surface=contributor_credit_events;item_count=0")
        }));
        assert!(events.iter().any(|event| {
            event.kind == "read"
                && event.reason.as_deref() == Some("surface=submission_status;item_count=1")
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
                && event
                    .event_hash
                    .as_deref()
                    .is_some_and(|hash| hash.starts_with("sha256:"))
                && event.reason.as_deref().is_some_and(|reason| {
                    reason.contains("surface=replay_dataset_export")
                        && reason.contains("purpose=db_audit_export")
                })
        }));
        let db_audit_events_after_read = db
            .list_trace_audit_events("tenant-a")
            .await
            .expect("audit read event mirrors to db");
        assert!(db_audit_events_after_read.iter().any(|event| {
            event.action == StorageTraceAuditAction::Read
                && event
                    .reason
                    .as_deref()
                    .is_some_and(|reason| reason.contains("surface=audit_events"))
        }));
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn derived_worker_exports_append_per_source_content_read_audits() {
        use ironclaw::db::libsql::LibSqlBackend;

        let temp = tempfile::tempdir().expect("temp dir");
        let db_temp = tempfile::tempdir().expect("db temp dir");
        let db_path = db_temp.path().join("trace-derived-read-audits.db");
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

        let mut preferred = sample_envelope().await;
        make_metadata_only_low_risk(&mut preferred);
        preferred.consent.scopes = vec![ConsentScope::RankingTraining];
        preferred.value.submission_score = 0.9;
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
        .expect("preferred source submits");
        let _ = submit_trace_handler(
            State(state.clone()),
            auth_headers("token-a"),
            Json(rejected),
        )
        .await
        .expect("rejected source submits");

        let Json(benchmark) = benchmark_convert_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            Json(BenchmarkConversionRequest {
                limit: Some(10),
                purpose: Some("audit_benchmark_sources".to_string()),
                consent_scope: None,
                status: Some(TraceCorpusStatus::Accepted),
                privacy_risk: Some(ResidualPiiRisk::Low),
                external_ref: None,
            }),
        )
        .await
        .expect("benchmark conversion succeeds");
        assert_eq!(benchmark.item_count, 2);

        let Json(candidates) = ranker_training_candidates_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            Query(RankerTrainingExportQuery {
                limit: Some(10),
                purpose: Some("audit_ranker_candidate_sources".to_string()),
                status: Some(TraceCorpusStatus::Accepted),
                consent_scope: Some("ranking-training".to_string()),
                privacy_risk: Some(ResidualPiiRisk::Low),
            }),
        )
        .await
        .expect("ranker candidates export succeeds");
        assert_eq!(candidates.item_count, 2);

        let Json(pairs) = ranker_training_pairs_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            Query(RankerTrainingExportQuery {
                limit: Some(10),
                purpose: Some("audit_ranker_pair_sources".to_string()),
                status: Some(TraceCorpusStatus::Accepted),
                consent_scope: Some("ranking-training".to_string()),
                privacy_risk: Some(ResidualPiiRisk::Low),
            }),
        )
        .await
        .expect("ranker pairs export succeeds");
        assert_eq!(pairs.item_count, 1);

        let Json(vector_response) = vector_index_handler(
            State(state),
            auth_headers("vector-worker-token-a"),
            Json(TraceVectorIndexRequest {
                purpose: Some("audit_vector_sources".to_string()),
                dry_run: false,
            }),
        )
        .await
        .expect("vector indexing succeeds");
        assert_eq!(vector_response.vector_entries_indexed, 2);

        let db_audit_events = db
            .list_trace_audit_events("tenant-a")
            .await
            .expect("DB audit events read");
        for submission_id in [preferred_id, rejected_id] {
            let active_object_ref = db
                .get_latest_active_trace_object_ref(
                    "tenant-a",
                    submission_id,
                    StorageTraceObjectArtifactKind::SubmittedEnvelope,
                )
                .await
                .expect("active object ref reads")
                .expect("active submitted envelope object ref exists");
            for (surface, purpose) in [
                ("benchmark_conversion", "audit_benchmark_sources"),
                (
                    "ranker_training_candidates",
                    "audit_ranker_candidate_sources",
                ),
                ("ranker_training_pairs", "audit_ranker_pair_sources"),
                ("vector_index", "audit_vector_sources"),
            ] {
                assert!(
                    db_audit_events.iter().any(|event| {
                        event.action == StorageTraceAuditAction::Read
                            && event.submission_id == Some(submission_id)
                            && event.object_ref_id == Some(active_object_ref.object_ref_id)
                            && event.reason.as_deref().is_some_and(|reason| {
                                reason.contains(&format!("surface={surface}"))
                                    && reason.contains(&format!("purpose={purpose}"))
                            })
                    }),
                    "missing {surface} content-read audit for {submission_id}"
                );
            }
        }
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn vector_index_requires_readable_active_submitted_object_ref() {
        use ironclaw::db::libsql::LibSqlBackend;

        let temp = tempfile::tempdir().expect("temp dir");
        let db_temp = tempfile::tempdir().expect("db temp dir");
        let db_path = db_temp.path().join("trace-vector-object-ref-gate.db");
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
        let conn = db.connect().await.expect("connect to mirror");
        conn.execute(
            "UPDATE trace_object_refs
             SET invalidated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             WHERE tenant_id = ?1 AND submission_id = ?2",
            libsql::params!["tenant-a", submission_id.to_string()],
        )
        .await
        .expect("invalidate active submitted object ref");

        let error = vector_index_handler(
            State(state),
            auth_headers("vector-worker-token-a"),
            Json(TraceVectorIndexRequest {
                purpose: Some("vector_requires_object_ref".to_string()),
                dry_run: false,
            }),
        )
        .await
        .expect_err("vector indexing fails closed without readable active object ref");
        assert_eq!(error.0, StatusCode::INTERNAL_SERVER_ERROR);
        assert!(
            db.list_trace_vector_entries("tenant-a")
                .await
                .expect("vector entries read")
                .is_empty()
        );
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
                purpose: None,
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
                purpose: None,
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
                purpose: None,
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

    #[test]
    fn append_audit_event_adds_genesis_and_chains_hashes() {
        let temp = tempfile::tempdir().expect("temp dir");
        let auth = test_reviewer_auth("tenant-a");
        let first = TraceCommonsAuditEvent::read(&auth, "trace_list", 2);
        append_audit_event(temp.path(), "tenant-a", first).expect("first audit event appends");
        let second = TraceCommonsAuditEvent::read(&auth, "audit_events", 1);
        append_audit_event(temp.path(), "tenant-a", second).expect("second audit event appends");

        let raw_events =
            read_raw_audit_events(temp.path(), "tenant-a").expect("raw audit events read");
        assert_eq!(raw_events.len(), 2);
        assert_eq!(
            raw_events[0].previous_event_hash.as_deref(),
            Some(TRACE_AUDIT_EVENT_GENESIS_HASH)
        );
        let first_hash = raw_events[0]
            .event_hash
            .as_deref()
            .expect("first audit event hash");
        assert!(first_hash.starts_with("sha256:"));
        assert_eq!(
            raw_events[1].previous_event_hash.as_deref(),
            Some(first_hash)
        );
        let second_hash = raw_events[1]
            .event_hash
            .as_deref()
            .expect("second audit event hash");
        assert_ne!(first_hash, second_hash);
        assert_eq!(
            compute_audit_event_hash(
                raw_events[1]
                    .previous_event_hash
                    .as_deref()
                    .expect("second previous hash"),
                &raw_events[1]
            )
            .expect("recompute second audit hash"),
            second_hash
        );

        let events = read_all_audit_events(temp.path(), "tenant-a").expect("audit events read");
        assert_eq!(events.len(), 2);
    }

    #[test]
    fn audit_event_hash_changes_when_event_is_tampered() {
        let temp = tempfile::tempdir().expect("temp dir");
        let auth = test_reviewer_auth("tenant-a");
        append_audit_event(
            temp.path(),
            "tenant-a",
            TraceCommonsAuditEvent::read(&auth, "trace_list", 2),
        )
        .expect("audit event appends");
        let event = read_raw_audit_events(temp.path(), "tenant-a")
            .expect("raw audit events read")
            .into_iter()
            .next()
            .expect("audit event exists");
        let stored_hash = event.event_hash.as_deref().expect("stored event hash");
        let mut tampered = event.clone();
        tampered.kind = "tampered_read".to_string();
        let tampered_hash = compute_audit_event_hash(
            tampered
                .previous_event_hash
                .as_deref()
                .expect("previous event hash"),
            &tampered,
        )
        .expect("tampered event hash recomputes");
        assert_ne!(stored_hash, tampered_hash);
    }

    #[test]
    fn read_all_audit_events_accepts_legacy_rows_without_chain_fields() {
        let temp = tempfile::tempdir().expect("temp dir");
        let auth = test_reviewer_auth("tenant-a");
        let legacy_event = TraceCommonsAuditEvent::read(&auth, "legacy_audit", 1);
        let path = audit_log_path(temp.path(), "tenant-a");
        std::fs::create_dir_all(path.parent().expect("audit parent")).expect("create audit dir");
        std::fs::write(
            &path,
            format!(
                "{}\n",
                serde_json::to_string(&legacy_event).expect("legacy event serializes")
            ),
        )
        .expect("write legacy audit event");

        let events = read_all_audit_events(temp.path(), "tenant-a").expect("legacy audit reads");
        assert_eq!(events.len(), 1);
        assert!(events[0].previous_event_hash.is_none());
        assert!(events[0].event_hash.is_none());

        append_audit_event(
            temp.path(),
            "tenant-a",
            TraceCommonsAuditEvent::read(&auth, "post_legacy_audit", 1),
        )
        .expect("post-legacy audit event appends");
        let raw_events =
            read_raw_audit_events(temp.path(), "tenant-a").expect("raw audit events read");
        assert_eq!(raw_events.len(), 2);
        assert_eq!(
            raw_events[1].previous_event_hash.as_deref(),
            Some(TRACE_AUDIT_EVENT_GENESIS_HASH)
        );
        assert!(raw_events[1].event_hash.is_some());
    }

    #[tokio::test]
    async fn maintenance_can_verify_file_audit_chain() {
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

        let Json(response) = maintenance_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            Json(TraceMaintenanceRequest {
                purpose: Some("audit_chain_verify".to_string()),
                dry_run: true,
                backfill_db_mirror: false,
                index_vectors: false,
                reconcile_db_mirror: false,
                verify_audit_chain: true,
                prune_export_cache: false,
                max_export_age_hours: None,
                purge_expired_before: None,
            }),
        )
        .await
        .expect("maintenance verifies audit chain");
        let report = response.audit_chain.expect("audit chain report exists");
        assert!(report.verified);
        assert_eq!(report.event_count, 2);
        assert_eq!(report.mismatch_count, 0);

        let path = audit_log_path(temp.path(), "tenant-a");
        let body = std::fs::read_to_string(&path).expect("audit log reads");
        std::fs::write(
            &path,
            body.replacen("\"submitted\"", "\"submitted_tampered\"", 1),
        )
        .expect("tamper audit log");

        let Json(response) = maintenance_handler(
            State(state),
            auth_headers("review-token-a"),
            Json(TraceMaintenanceRequest {
                purpose: Some("audit_chain_verify_after_tamper".to_string()),
                dry_run: true,
                backfill_db_mirror: false,
                index_vectors: false,
                reconcile_db_mirror: false,
                verify_audit_chain: true,
                prune_export_cache: false,
                max_export_age_hours: None,
                purge_expired_before: None,
            }),
        )
        .await
        .expect("maintenance reports audit chain tampering");
        let report = response.audit_chain.expect("audit chain report exists");
        assert!(!report.verified);
        assert!(report.mismatch_count >= 1);
        assert!(
            report
                .failures
                .iter()
                .any(|failure| failure.contains("event_hash mismatch"))
        );
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn maintenance_can_verify_db_audit_chain() {
        use ironclaw::db::libsql::LibSqlBackend;

        let temp = tempfile::tempdir().expect("temp dir");
        let db_temp = tempfile::tempdir().expect("db temp dir");
        let db_path = db_temp.path().join("trace-db-audit-chain.db");
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

        let _ = submit_trace_handler(
            State(state.clone()),
            auth_headers("token-a"),
            Json(envelope),
        )
        .await
        .expect("submission succeeds");

        let Json(response) = maintenance_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            Json(TraceMaintenanceRequest {
                purpose: Some("db_audit_chain_verify".to_string()),
                dry_run: false,
                backfill_db_mirror: true,
                index_vectors: false,
                reconcile_db_mirror: false,
                verify_audit_chain: true,
                prune_export_cache: false,
                max_export_age_hours: None,
                purge_expired_before: None,
            }),
        )
        .await
        .expect("maintenance verifies DB audit chain");
        let report = response.audit_chain.expect("audit chain report exists");
        assert!(report.verified);
        let db_report = report.db_mirror.expect("DB audit chain report exists");
        assert!(db_report.verified, "{db_report:?}");
        assert!(db_report.event_count >= 3);
        assert!(db_report.legacy_event_count >= 1);
        assert!(db_report.payload_verified_event_count >= 2);
        assert_eq!(db_report.payload_unverified_event_count, 0);
        assert_eq!(db_report.mismatch_count, 0);

        let conn = db.connect().await.expect("connect to mirror");
        conn.execute(
            "UPDATE trace_audit_events
             SET event_hash = 'sha256:tampered'
             WHERE tenant_id = ?1
               AND audit_event_id = (
                   SELECT audit_event_id
                   FROM trace_audit_events
                   WHERE tenant_id = ?1
                     AND event_hash IS NOT NULL
                   ORDER BY occurred_at ASC
                   LIMIT 1
               )",
            libsql::params!["tenant-a"],
        )
        .await
        .expect("tamper DB audit event hash");

        let Json(response) = maintenance_handler(
            State(state),
            auth_headers("review-token-a"),
            Json(TraceMaintenanceRequest {
                purpose: Some("db_audit_chain_verify_after_tamper".to_string()),
                dry_run: true,
                backfill_db_mirror: false,
                index_vectors: false,
                reconcile_db_mirror: false,
                verify_audit_chain: true,
                prune_export_cache: false,
                max_export_age_hours: None,
                purge_expired_before: None,
            }),
        )
        .await
        .expect("maintenance reports DB audit chain tampering");
        let db_report = response
            .audit_chain
            .expect("audit chain report exists")
            .db_mirror
            .expect("DB audit chain report exists");
        assert!(!db_report.verified);
        assert!(db_report.mismatch_count >= 1);
        assert!(
            db_report
                .failures
                .iter()
                .any(|failure| failure.contains("previous_event_hash mismatch"))
        );
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn maintenance_detects_db_audit_projection_tampering() {
        use ironclaw::db::libsql::LibSqlBackend;

        let temp = tempfile::tempdir().expect("temp dir");
        let db_temp = tempfile::tempdir().expect("db temp dir");
        let db_path = db_temp.path().join("trace-db-audit-projection.db");
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

        let _ = submit_trace_handler(
            State(state.clone()),
            auth_headers("token-a"),
            Json(envelope),
        )
        .await
        .expect("submission succeeds");
        let _ = maintenance_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            Json(TraceMaintenanceRequest {
                purpose: Some("db_audit_projection_backfill".to_string()),
                dry_run: false,
                backfill_db_mirror: true,
                index_vectors: false,
                reconcile_db_mirror: false,
                verify_audit_chain: false,
                prune_export_cache: false,
                max_export_age_hours: None,
                purge_expired_before: None,
            }),
        )
        .await
        .expect("backfill file audit events before export");
        let Json(export) = dataset_replay_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            Query(DatasetExportQuery {
                limit: Some(10),
                purpose: Some("projection_tamper".to_string()),
                status: Some(TraceCorpusStatus::Accepted),
                privacy_risk: Some(ResidualPiiRisk::Low),
                consent_scope: None,
            }),
        )
        .await
        .expect("dataset export creates canonical DB audit event");
        assert_eq!(export.item_count, 1);

        let Json(response) = maintenance_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            Json(TraceMaintenanceRequest {
                purpose: Some("db_audit_projection_verify".to_string()),
                dry_run: true,
                backfill_db_mirror: false,
                index_vectors: false,
                reconcile_db_mirror: false,
                verify_audit_chain: true,
                prune_export_cache: false,
                max_export_age_hours: None,
                purge_expired_before: None,
            }),
        )
        .await
        .expect("maintenance verifies canonical DB audit projection");
        let db_report = response
            .audit_chain
            .expect("audit chain report exists")
            .db_mirror
            .expect("DB audit chain report exists");
        assert!(db_report.verified, "{db_report:?}");

        let conn = db.connect().await.expect("connect to mirror");
        conn.execute(
            "UPDATE trace_audit_events
             SET action = 'read',
                 metadata_json = ?3
             WHERE tenant_id = ?1
               AND export_manifest_id = ?2",
            libsql::params![
                "tenant-a",
                export.export_id.to_string(),
                r#"{"kind":"export","artifact_kind":"export_artifact","purpose_code":"projection_tamper","item_count":99}"#,
            ],
        )
        .await
        .expect("tamper DB audit projection");

        let Json(response) = maintenance_handler(
            State(state),
            auth_headers("review-token-a"),
            Json(TraceMaintenanceRequest {
                purpose: Some("db_audit_projection_after_tamper".to_string()),
                dry_run: true,
                backfill_db_mirror: false,
                index_vectors: false,
                reconcile_db_mirror: false,
                verify_audit_chain: true,
                prune_export_cache: false,
                max_export_age_hours: None,
                purge_expired_before: None,
            }),
        )
        .await
        .expect("maintenance reports projection tampering");
        let db_report = response
            .audit_chain
            .expect("audit chain report exists")
            .db_mirror
            .expect("DB audit chain report exists");
        assert!(!db_report.verified);
        assert!(
            db_report
                .failures
                .iter()
                .any(|failure| failure.contains("canonical kind/action mismatch"))
        );
        assert!(
            db_report
                .failures
                .iter()
                .any(|failure| failure.contains("canonical export_count mismatch"))
        );
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
                purpose: None,
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
                purpose: None,
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
                purpose: None,
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
                purpose: None,
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
                purpose: None,
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
    async fn reviewer_trace_list_filters_by_export_purpose() {
        let temp = tempfile::tempdir().expect("temp dir");
        let state = test_state(temp.path().to_path_buf());
        let mut accepted = sample_envelope().await;
        make_metadata_only_low_risk(&mut accepted);
        accepted.replay.replayable = true;
        let accepted_id = accepted.submission_id;
        let quarantined = sample_envelope().await;
        let quarantined_id = quarantined.submission_id;

        let _ = submit_trace_handler(
            State(state.clone()),
            auth_headers("token-a"),
            Json(accepted),
        )
        .await
        .expect("accepted submission succeeds");
        let _ = submit_trace_handler(
            State(state.clone()),
            auth_headers("token-a"),
            Json(quarantined),
        )
        .await
        .expect("quarantined submission succeeds");

        let Json(export) = dataset_replay_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            Query(DatasetExportQuery {
                limit: Some(10),
                purpose: Some("purpose_filter_export".to_string()),
                status: Some(TraceCorpusStatus::Accepted),
                privacy_risk: Some(ResidualPiiRisk::Low),
                consent_scope: None,
            }),
        )
        .await
        .expect("replay export writes purpose manifest");
        assert_eq!(export.item_count, 1);
        assert_eq!(export.items[0].submission_id, accepted_id);

        let Json(purpose_records) = list_traces_handler(
            State(state.clone()),
            auth_headers("review-token-a"),
            Query(TraceListQuery {
                status: None,
                limit: Some(10),
                purpose: Some("purpose_filter_export".to_string()),
                coverage_tag: None,
                tool: None,
                privacy_risk: None,
                consent_scope: None,
            }),
        )
        .await
        .expect("reviewer can list traces by export purpose");
        assert_eq!(purpose_records.len(), 1);
        assert_eq!(purpose_records[0].submission_id, accepted_id);
        assert_ne!(purpose_records[0].submission_id, quarantined_id);

        let Json(missing_purpose_records) = list_traces_handler(
            State(state),
            auth_headers("review-token-a"),
            Query(TraceListQuery {
                status: None,
                limit: Some(10),
                purpose: Some("missing_export_purpose".to_string()),
                coverage_tag: None,
                tool: None,
                privacy_risk: None,
                consent_scope: None,
            }),
        )
        .await
        .expect("missing purpose returns no records");
        assert!(missing_purpose_records.is_empty());
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
                purpose: None,
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
                purpose: None,
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
        assert_eq!(credit.credit_points_final, 0.0);
        assert_eq!(credit.credit_points_total, 1.75);

        let Json(statuses) = submission_status_handler(
            State(state.clone()),
            auth_headers("token-a"),
            Json(TraceSubmissionStatusRequest {
                submission_ids: vec![submission_id],
            }),
        )
        .await
        .expect("contributor can sync delayed credit status");
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].credit_points_ledger, 1.75);
        assert_eq!(statuses[0].credit_points_final, None);
        assert_eq!(statuses[0].credit_points_total, Some(1.75));
        assert!(
            statuses[0]
                .delayed_credit_explanations
                .iter()
                .any(|explanation| explanation.contains("TrainingUtility"))
        );

        let revoke_status = revoke_trace_handler(
            State(state.clone()),
            auth_headers("token-a"),
            AxumPath(submission_id),
        )
        .await
        .expect("contributor can revoke trace with existing delayed credit");
        assert_eq!(revoke_status, StatusCode::NO_CONTENT);

        let Json(events_after_revoke) =
            credit_events_handler(State(state.clone()), auth_headers("token-a"))
                .await
                .expect(
                    "credit events remain hidden from contributor projections after revocation",
                );
        assert!(
            events_after_revoke.is_empty(),
            "terminal trace credit events remain in the audit ledger but are hidden from contributor credit projections"
        );

        let Json(credit_after_revoke) =
            credit_handler(State(state.clone()), auth_headers("token-a"))
                .await
                .expect("credit summary succeeds after revocation");
        assert_eq!(credit_after_revoke.revoked, 1);
        assert_eq!(credit_after_revoke.credit_points_ledger, 0.0);
        assert_eq!(credit_after_revoke.credit_points_final, 0.0);
        assert_eq!(credit_after_revoke.credit_points_total, 0.0);

        let Json(statuses_after_revoke) = submission_status_handler(
            State(state),
            auth_headers("token-a"),
            Json(TraceSubmissionStatusRequest {
                submission_ids: vec![submission_id],
            }),
        )
        .await
        .expect("contributor can sync revoked delayed credit status");
        assert_eq!(statuses_after_revoke.len(), 1);
        assert_eq!(statuses_after_revoke[0].status, "revoked");
        assert_eq!(statuses_after_revoke[0].credit_points_ledger, 0.0);
        assert_eq!(statuses_after_revoke[0].credit_points_final, Some(0.0));
        assert_eq!(statuses_after_revoke[0].credit_points_total, None);
        assert!(
            statuses_after_revoke[0]
                .explanation
                .iter()
                .any(|explanation| explanation.contains("Revoked"))
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
        assert_eq!(credit.credit_points_final, 0.0);
        assert_eq!(credit.credit_points_total, -4.0);
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
