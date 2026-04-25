//! Backend-agnostic storage contracts for Trace Commons corpus metadata.
//!
//! These types describe the DB-backed production storage surface without
//! changing the current file-backed ingest path.

use std::collections::BTreeMap;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::DatabaseError;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TraceCorpusStatus {
    Received,
    Accepted,
    Quarantined,
    Rejected,
    Revoked,
    Expired,
    Purged,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TraceObjectArtifactKind {
    SubmittedEnvelope,
    RescrubbedEnvelope,
    ReviewSnapshot,
    BenchmarkArtifact,
    ExportArtifact,
    WorkerIntermediate,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TraceDerivedStatus {
    Current,
    Invalidated,
    Superseded,
    Revoked,
    Expired,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TraceWorkerKind {
    ServerRescrub,
    Summary,
    DuplicatePrecheck,
    Embedding,
    Ranking,
    BenchmarkConversion,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TraceAuditAction {
    Submit,
    Read,
    Review,
    CreditMutate,
    Revoke,
    Export,
    Retain,
    Purge,
    VectorIndex,
    BenchmarkConvert,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TraceCreditEventType {
    Accepted,
    PrivacyRejection,
    DuplicateRejection,
    BenchmarkConversion,
    RegressionCatch,
    TrainingUtility,
    ReviewerBonus,
    AbusePenalty,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TraceCreditSettlementState {
    Pending,
    Final,
    Reversed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TraceSubmissionWrite {
    pub tenant_id: String,
    pub submission_id: Uuid,
    pub trace_id: Uuid,
    pub auth_principal_ref: String,
    pub contributor_pseudonym: Option<String>,
    pub submitted_tenant_scope_ref: Option<String>,
    pub schema_version: String,
    pub consent_policy_version: String,
    pub consent_scopes: Vec<String>,
    pub allowed_uses: Vec<String>,
    pub retention_policy_id: String,
    pub status: TraceCorpusStatus,
    pub privacy_risk: String,
    pub redaction_pipeline_version: String,
    pub redaction_counts: BTreeMap<String, u32>,
    pub redaction_hash: String,
    pub canonical_summary_hash: Option<String>,
    pub submission_score: Option<f32>,
    pub credit_points_pending: Option<f32>,
    pub credit_points_final: Option<f32>,
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TraceSubmissionRecord {
    pub tenant_id: String,
    pub submission_id: Uuid,
    pub trace_id: Uuid,
    pub status: TraceCorpusStatus,
    pub auth_principal_ref: String,
    pub contributor_pseudonym: Option<String>,
    pub submitted_tenant_scope_ref: Option<String>,
    pub schema_version: String,
    pub consent_policy_version: String,
    pub consent_scopes: Vec<String>,
    pub allowed_uses: Vec<String>,
    pub retention_policy_id: String,
    pub privacy_risk: String,
    pub redaction_pipeline_version: String,
    pub redaction_counts: BTreeMap<String, u32>,
    pub redaction_hash: String,
    pub canonical_summary_hash: Option<String>,
    pub submission_score: Option<f32>,
    pub credit_points_pending: Option<f32>,
    pub credit_points_final: Option<f32>,
    pub received_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub reviewed_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
    pub purged_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceObjectRefWrite {
    pub object_ref_id: Uuid,
    pub tenant_id: String,
    pub submission_id: Uuid,
    pub artifact_kind: TraceObjectArtifactKind,
    pub object_store: String,
    pub object_key: String,
    pub content_sha256: String,
    pub encryption_key_ref: String,
    pub size_bytes: i64,
    pub compression: Option<String>,
    pub created_by_job_id: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TenantScopedTraceObjectRef {
    pub tenant_id: String,
    pub submission_id: Uuid,
    pub object_ref_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TraceDerivedRecordWrite {
    pub derived_id: Uuid,
    pub tenant_id: String,
    pub submission_id: Uuid,
    pub trace_id: Uuid,
    pub status: TraceDerivedStatus,
    pub worker_kind: TraceWorkerKind,
    pub worker_version: String,
    pub input_object_ref: Option<TenantScopedTraceObjectRef>,
    pub input_hash: String,
    pub output_object_ref: Option<TenantScopedTraceObjectRef>,
    pub canonical_summary: Option<String>,
    pub canonical_summary_hash: Option<String>,
    pub summary_model: String,
    pub task_success: Option<String>,
    pub privacy_risk: Option<String>,
    pub event_count: Option<i32>,
    pub tool_sequence: Vec<String>,
    pub tool_categories: Vec<String>,
    pub coverage_tags: Vec<String>,
    pub duplicate_score: Option<f32>,
    pub novelty_score: Option<f32>,
    pub cluster_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TraceDerivedRecord {
    pub derived_id: Uuid,
    pub tenant_id: String,
    pub submission_id: Uuid,
    pub trace_id: Uuid,
    pub status: TraceDerivedStatus,
    pub worker_kind: TraceWorkerKind,
    pub worker_version: String,
    pub input_object_ref: Option<TenantScopedTraceObjectRef>,
    pub input_hash: String,
    pub output_object_ref: Option<TenantScopedTraceObjectRef>,
    pub canonical_summary: Option<String>,
    pub canonical_summary_hash: Option<String>,
    pub summary_model: String,
    pub task_success: Option<String>,
    pub privacy_risk: Option<String>,
    pub event_count: Option<i32>,
    pub tool_sequence: Vec<String>,
    pub tool_categories: Vec<String>,
    pub coverage_tags: Vec<String>,
    pub duplicate_score: Option<f32>,
    pub novelty_score: Option<f32>,
    pub cluster_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TraceAuditEventWrite {
    pub audit_event_id: Uuid,
    pub tenant_id: String,
    pub actor_principal_ref: String,
    pub actor_role: String,
    pub action: TraceAuditAction,
    pub reason: Option<String>,
    pub request_id: Option<String>,
    pub submission_id: Option<Uuid>,
    pub object_ref_id: Option<Uuid>,
    pub export_manifest_id: Option<Uuid>,
    pub decision_inputs_hash: Option<String>,
    pub metadata: TraceAuditSafeMetadata,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TraceAuditEventRecord {
    pub audit_event_id: Uuid,
    pub tenant_id: String,
    pub actor_principal_ref: String,
    pub actor_role: String,
    pub action: TraceAuditAction,
    pub reason: Option<String>,
    pub request_id: Option<String>,
    pub submission_id: Option<Uuid>,
    pub object_ref_id: Option<Uuid>,
    pub export_manifest_id: Option<Uuid>,
    pub decision_inputs_hash: Option<String>,
    pub metadata: TraceAuditSafeMetadata,
    pub occurred_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TraceAuditSafeMetadata {
    #[default]
    Empty,
    Submission {
        status: TraceCorpusStatus,
        privacy_risk: String,
    },
    ReviewDecision {
        decision: String,
        resulting_status: TraceCorpusStatus,
        reason_code: Option<String>,
    },
    Export {
        artifact_kind: TraceObjectArtifactKind,
        purpose_code: Option<String>,
        item_count: u32,
    },
    Maintenance {
        dry_run: bool,
        action_counts: BTreeMap<String, u32>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TraceCreditEventWrite {
    pub credit_event_id: Uuid,
    pub tenant_id: String,
    pub submission_id: Uuid,
    pub trace_id: Uuid,
    pub credit_account_ref: String,
    pub event_type: TraceCreditEventType,
    pub points_delta: String,
    pub reason: String,
    pub external_ref: Option<String>,
    pub actor_principal_ref: String,
    pub actor_role: String,
    pub settlement_state: TraceCreditSettlementState,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TraceCreditEventRecord {
    pub credit_event_id: Uuid,
    pub tenant_id: String,
    pub submission_id: Uuid,
    pub trace_id: Uuid,
    pub credit_account_ref: String,
    pub event_type: TraceCreditEventType,
    pub points_delta: String,
    pub reason: String,
    pub external_ref: Option<String>,
    pub actor_principal_ref: String,
    pub actor_role: String,
    pub settlement_state: TraceCreditSettlementState,
    pub occurred_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceTombstoneWrite {
    pub tombstone_id: Uuid,
    pub tenant_id: String,
    pub submission_id: Uuid,
    pub trace_id: Option<Uuid>,
    pub redaction_hash: Option<String>,
    pub canonical_summary_hash: Option<String>,
    pub reason: String,
    pub effective_at: DateTime<Utc>,
    pub retain_until: Option<DateTime<Utc>>,
    pub created_by_principal_ref: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct TraceArtifactInvalidationCounts {
    pub object_refs_invalidated: u64,
    pub derived_records_invalidated: u64,
}

#[async_trait]
pub trait TraceCorpusStore: Send + Sync {
    async fn upsert_trace_submission(
        &self,
        submission: TraceSubmissionWrite,
    ) -> Result<TraceSubmissionRecord, DatabaseError>;

    async fn get_trace_submission(
        &self,
        tenant_id: &str,
        submission_id: Uuid,
    ) -> Result<Option<TraceSubmissionRecord>, DatabaseError>;

    async fn list_trace_submissions(
        &self,
        tenant_id: &str,
    ) -> Result<Vec<TraceSubmissionRecord>, DatabaseError>;

    async fn list_trace_credit_events(
        &self,
        tenant_id: &str,
    ) -> Result<Vec<TraceCreditEventRecord>, DatabaseError>;

    async fn update_trace_submission_status(
        &self,
        tenant_id: &str,
        submission_id: Uuid,
        status: TraceCorpusStatus,
        actor_principal_ref: &str,
        reason: Option<&str>,
    ) -> Result<(), DatabaseError>;

    async fn append_trace_object_ref(
        &self,
        object_ref: TraceObjectRefWrite,
    ) -> Result<(), DatabaseError>;

    async fn append_trace_derived_record(
        &self,
        derived_record: TraceDerivedRecordWrite,
    ) -> Result<(), DatabaseError>;

    async fn list_trace_derived_records(
        &self,
        tenant_id: &str,
    ) -> Result<Vec<TraceDerivedRecord>, DatabaseError>;

    async fn append_trace_audit_event(
        &self,
        audit_event: TraceAuditEventWrite,
    ) -> Result<(), DatabaseError>;

    async fn list_trace_audit_events(
        &self,
        tenant_id: &str,
    ) -> Result<Vec<TraceAuditEventRecord>, DatabaseError>;

    async fn append_trace_credit_event(
        &self,
        credit_event: TraceCreditEventWrite,
    ) -> Result<(), DatabaseError>;

    async fn write_trace_tombstone(
        &self,
        tombstone: TraceTombstoneWrite,
    ) -> Result<(), DatabaseError>;

    async fn invalidate_trace_submission_artifacts(
        &self,
        tenant_id: &str,
        submission_id: Uuid,
        derived_status: TraceDerivedStatus,
    ) -> Result<TraceArtifactInvalidationCounts, DatabaseError>;
}
