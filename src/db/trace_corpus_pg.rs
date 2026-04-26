use std::collections::BTreeMap;

use async_trait::async_trait;
use tokio_postgres::{Row, Transaction};
use uuid::Uuid;

use crate::db::postgres::PgBackend;
use crate::db::trace_corpus_common::{
    audit_action_for_status, enum_from_storage, enum_to_storage,
    validate_tenant_scoped_trace_object_ref, validate_trace_audit_append_chain,
};
use crate::error::DatabaseError;
use crate::trace_corpus_storage::{
    TenantScopedTraceObjectRef, TraceArtifactInvalidationCounts, TraceAuditEventRecord,
    TraceAuditEventWrite, TraceAuditSafeMetadata, TraceCorpusStatus, TraceCorpusStore,
    TraceCreditEventRecord, TraceCreditEventWrite, TraceCreditSettlementState, TraceDerivedRecord,
    TraceDerivedRecordWrite, TraceDerivedStatus, TraceExportManifestItemInvalidationReason,
    TraceExportManifestItemRecord, TraceExportManifestItemWrite, TraceExportManifestRecord,
    TraceExportManifestWrite, TraceObjectArtifactKind, TraceObjectRefRecord, TraceObjectRefWrite,
    TraceSubmissionRecord, TraceSubmissionWrite, TraceTenantPolicyRecord, TraceTenantPolicyWrite,
    TraceTombstoneRecord, TraceTombstoneWrite, TraceVectorEntryRecord,
    TraceVectorEntrySourceProjection, TraceVectorEntryStatus, TraceVectorEntryWrite,
    TraceWorkerKind,
};

const TRACE_OBJECT_REF_COLUMNS: &str = "\
    tenant_id, submission_id, object_ref_id, artifact_kind, object_store, object_key, \
    content_sha256, encryption_key_ref, size_bytes, compression, created_by_job_id, \
    invalidated_at, deleted_at, updated_at, created_at";

const TRACE_EXPORT_MANIFEST_COLUMNS: &str = "\
    tenant_id, export_manifest_id, artifact_kind, purpose_code, audit_event_id, \
    source_submission_ids, source_submission_ids_hash, item_count, generated_at, \
    invalidated_at, deleted_at, created_at, updated_at";

const TRACE_EXPORT_MANIFEST_ITEM_COLUMNS: &str = "\
    tenant_id, export_manifest_id, submission_id, trace_id, derived_id, object_ref_id, \
    vector_entry_id, source_status_at_export, source_hash_at_export, source_invalidated_at, \
    source_invalidation_reason, created_at, updated_at";

const TRACE_TOMBSTONE_COLUMNS: &str = "\
    tenant_id, tombstone_id, submission_id, trace_id, redaction_hash, canonical_summary_hash, \
    reason, effective_at, retain_until, created_by_principal_ref, created_at";

async fn ensure_pg_object_ref_belongs_to_submission(
    tx: &Transaction<'_>,
    tenant_id: &str,
    submission_id: Uuid,
    object_ref_id: Uuid,
    field: &str,
) -> Result<(), DatabaseError> {
    let exists = tx
        .query_opt(
            "SELECT 1
             FROM trace_object_refs
             WHERE tenant_id = $1
               AND submission_id = $2
               AND object_ref_id = $3
             LIMIT 1",
            &[&tenant_id, &submission_id, &object_ref_id],
        )
        .await
        .map_err(DatabaseError::Postgres)?
        .is_some();
    if exists {
        return Ok(());
    }

    Err(DatabaseError::Constraint(format!(
        "trace {field} object_ref_id {object_ref_id} does not belong to tenant {tenant_id} submission {submission_id}"
    )))
}

async fn ensure_pg_derived_record_belongs_to_submission(
    tx: &Transaction<'_>,
    tenant_id: &str,
    submission_id: Uuid,
    derived_id: Uuid,
) -> Result<(), DatabaseError> {
    let exists = tx
        .query_opt(
            "SELECT 1
             FROM trace_derived_records
             WHERE tenant_id = $1
               AND submission_id = $2
               AND derived_id = $3
             LIMIT 1",
            &[&tenant_id, &submission_id, &derived_id],
        )
        .await
        .map_err(DatabaseError::Postgres)?
        .is_some();
    if exists {
        return Ok(());
    }

    Err(DatabaseError::Constraint(format!(
        "trace export manifest derived_id {derived_id} does not belong to tenant {tenant_id} submission {submission_id}"
    )))
}

async fn ensure_pg_vector_entry_belongs_to_submission(
    tx: &Transaction<'_>,
    tenant_id: &str,
    submission_id: Uuid,
    vector_entry_id: Uuid,
) -> Result<(), DatabaseError> {
    let exists = tx
        .query_opt(
            "SELECT 1
             FROM trace_vector_entries
             WHERE tenant_id = $1
               AND submission_id = $2
               AND vector_entry_id = $3
             LIMIT 1",
            &[&tenant_id, &submission_id, &vector_entry_id],
        )
        .await
        .map_err(DatabaseError::Postgres)?
        .is_some();
    if exists {
        return Ok(());
    }

    Err(DatabaseError::Constraint(format!(
        "trace export manifest vector_entry_id {vector_entry_id} does not belong to tenant {tenant_id} submission {submission_id}"
    )))
}

fn json_array_strings(
    value: serde_json::Value,
    column: &str,
) -> Result<Vec<String>, DatabaseError> {
    let values = value.as_array().ok_or_else(|| {
        DatabaseError::Serialization(format!("trace {column} column is not a JSON array"))
    })?;
    values
        .iter()
        .map(|value| {
            value.as_str().map(str::to_string).ok_or_else(|| {
                DatabaseError::Serialization(format!(
                    "trace {column} column contains a non-string value"
                ))
            })
        })
        .collect()
}

fn json_u32_map(
    value: serde_json::Value,
    column: &str,
) -> Result<BTreeMap<String, u32>, DatabaseError> {
    serde_json::from_value(value).map_err(|e| {
        DatabaseError::Serialization(format!("trace {column} column JSON decode failed: {e}"))
    })
}

fn row_to_submission(row: &Row) -> Result<TraceSubmissionRecord, DatabaseError> {
    let status: String = row.get("status");
    let consent_scopes: serde_json::Value = row.get("consent_scopes");
    let allowed_uses: serde_json::Value = row.get("allowed_uses");
    let redaction_counts: serde_json::Value = row.get("redaction_counts");
    Ok(TraceSubmissionRecord {
        tenant_id: row.get("tenant_id"),
        submission_id: row.get("submission_id"),
        trace_id: row.get("trace_id"),
        status: enum_from_storage::<TraceCorpusStatus>(&status, "TraceCorpusStatus")?,
        auth_principal_ref: row.get("auth_principal_ref"),
        contributor_pseudonym: row.get("contributor_pseudonym"),
        submitted_tenant_scope_ref: row.get("submitted_tenant_scope_ref"),
        schema_version: row.get("schema_version"),
        consent_policy_version: row.get("consent_policy_version"),
        consent_scopes: json_array_strings(consent_scopes, "consent_scopes")?,
        allowed_uses: json_array_strings(allowed_uses, "allowed_uses")?,
        retention_policy_id: row.get("retention_policy_id"),
        privacy_risk: row.get("privacy_risk"),
        redaction_pipeline_version: row.get("redaction_pipeline_version"),
        redaction_counts: json_u32_map(redaction_counts, "redaction_counts")?,
        redaction_hash: row.get("redaction_hash"),
        canonical_summary_hash: row.get("canonical_summary_hash"),
        submission_score: row.get("submission_score"),
        credit_points_pending: row.get("credit_points_pending"),
        credit_points_final: row.get("credit_points_final"),
        received_at: row.get("received_at"),
        updated_at: row.get("updated_at"),
        reviewed_at: row.get("reviewed_at"),
        revoked_at: row.get("revoked_at"),
        expires_at: row.get("expires_at"),
        purged_at: row.get("purged_at"),
    })
}

fn row_to_tenant_policy(row: &Row) -> Result<TraceTenantPolicyRecord, DatabaseError> {
    let allowed_consent_scopes: serde_json::Value = row.get("allowed_consent_scopes");
    let allowed_uses: serde_json::Value = row.get("allowed_uses");
    Ok(TraceTenantPolicyRecord {
        tenant_id: row.get("tenant_id"),
        policy_version: row.get("policy_version"),
        allowed_consent_scopes: json_array_strings(
            allowed_consent_scopes,
            "allowed_consent_scopes",
        )?,
        allowed_uses: json_array_strings(allowed_uses, "allowed_uses")?,
        updated_by_principal_ref: row.get("updated_by_principal_ref"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

fn row_to_object_ref(row: &Row) -> Result<TraceObjectRefRecord, DatabaseError> {
    let artifact_kind: String = row.get("artifact_kind");
    Ok(TraceObjectRefRecord {
        tenant_id: row.get("tenant_id"),
        submission_id: row.get("submission_id"),
        object_ref_id: row.get("object_ref_id"),
        artifact_kind: enum_from_storage::<TraceObjectArtifactKind>(
            &artifact_kind,
            "TraceObjectArtifactKind",
        )?,
        object_store: row.get("object_store"),
        object_key: row.get("object_key"),
        content_sha256: row.get("content_sha256"),
        encryption_key_ref: row.get("encryption_key_ref"),
        size_bytes: row.get("size_bytes"),
        compression: row.get("compression"),
        created_by_job_id: row.get("created_by_job_id"),
        invalidated_at: row.get("invalidated_at"),
        deleted_at: row.get("deleted_at"),
        updated_at: row.get("updated_at"),
        created_at: row.get("created_at"),
    })
}

fn row_to_credit_event(row: &Row) -> Result<TraceCreditEventRecord, DatabaseError> {
    let event_type: String = row.get("event_type");
    let settlement_state: String = row.get("settlement_state");
    Ok(TraceCreditEventRecord {
        tenant_id: row.get("tenant_id"),
        credit_event_id: row.get("credit_event_id"),
        submission_id: row.get("submission_id"),
        trace_id: row.get("trace_id"),
        credit_account_ref: row.get("credit_account_ref"),
        event_type: enum_from_storage(&event_type, "TraceCreditEventType")?,
        points_delta: row.get("points_delta"),
        reason: row.get("reason"),
        external_ref: row.get("external_ref"),
        actor_principal_ref: row.get("actor_principal_ref"),
        actor_role: row.get("actor_role"),
        settlement_state: enum_from_storage::<TraceCreditSettlementState>(
            &settlement_state,
            "TraceCreditSettlementState",
        )?,
        occurred_at: row.get("occurred_at"),
    })
}

fn row_to_derived_record(row: &Row) -> Result<TraceDerivedRecord, DatabaseError> {
    let status: String = row.get("status");
    let worker_kind: String = row.get("worker_kind");
    let tenant_id: String = row.get("tenant_id");
    let submission_id: Uuid = row.get("submission_id");
    let input_object_ref_id: Option<Uuid> = row.get("input_object_ref_id");
    let output_object_ref_id: Option<Uuid> = row.get("output_object_ref_id");
    let tool_sequence: serde_json::Value = row.get("tool_sequence");
    let tool_categories: serde_json::Value = row.get("tool_categories");
    let coverage_tags: serde_json::Value = row.get("coverage_tags");
    Ok(TraceDerivedRecord {
        derived_id: row.get("derived_id"),
        tenant_id: tenant_id.clone(),
        submission_id,
        trace_id: row.get("trace_id"),
        status: enum_from_storage::<TraceDerivedStatus>(&status, "TraceDerivedStatus")?,
        worker_kind: enum_from_storage::<TraceWorkerKind>(&worker_kind, "TraceWorkerKind")?,
        worker_version: row.get("worker_version"),
        input_object_ref: input_object_ref_id.map(|object_ref_id| TenantScopedTraceObjectRef {
            tenant_id: tenant_id.clone(),
            submission_id,
            object_ref_id,
        }),
        input_hash: row.get("input_hash"),
        output_object_ref: output_object_ref_id.map(|object_ref_id| TenantScopedTraceObjectRef {
            tenant_id: tenant_id.clone(),
            submission_id,
            object_ref_id,
        }),
        canonical_summary: row.get("canonical_summary"),
        canonical_summary_hash: row.get("canonical_summary_hash"),
        summary_model: row.get("summary_model"),
        task_success: row.get("task_success"),
        privacy_risk: row.get("privacy_risk"),
        event_count: row.get("event_count"),
        tool_sequence: json_array_strings(tool_sequence, "tool_sequence")?,
        tool_categories: json_array_strings(tool_categories, "tool_categories")?,
        coverage_tags: json_array_strings(coverage_tags, "coverage_tags")?,
        duplicate_score: row.get("duplicate_score"),
        novelty_score: row.get("novelty_score"),
        cluster_id: row.get("cluster_id"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

fn row_to_vector_entry(row: &Row) -> Result<TraceVectorEntryRecord, DatabaseError> {
    let source_projection: String = row.get("source_projection");
    let status: String = row.get("status");
    Ok(TraceVectorEntryRecord {
        tenant_id: row.get("tenant_id"),
        submission_id: row.get("submission_id"),
        derived_id: row.get("derived_id"),
        vector_entry_id: row.get("vector_entry_id"),
        vector_store: row.get("vector_store"),
        embedding_model: row.get("embedding_model"),
        embedding_dimension: row.get("embedding_dimension"),
        embedding_version: row.get("embedding_version"),
        source_projection: enum_from_storage::<TraceVectorEntrySourceProjection>(
            &source_projection,
            "TraceVectorEntrySourceProjection",
        )?,
        source_hash: row.get("source_hash"),
        status: enum_from_storage::<TraceVectorEntryStatus>(&status, "TraceVectorEntryStatus")?,
        nearest_trace_ids: row.get("nearest_trace_ids"),
        cluster_id: row.get("cluster_id"),
        duplicate_score: row.get("duplicate_score"),
        novelty_score: row.get("novelty_score"),
        indexed_at: row.get("indexed_at"),
        invalidated_at: row.get("invalidated_at"),
        deleted_at: row.get("deleted_at"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

fn row_to_export_manifest(row: &Row) -> Result<TraceExportManifestRecord, DatabaseError> {
    let artifact_kind: String = row.get("artifact_kind");
    Ok(TraceExportManifestRecord {
        tenant_id: row.get("tenant_id"),
        export_manifest_id: row.get("export_manifest_id"),
        artifact_kind: enum_from_storage::<TraceObjectArtifactKind>(
            &artifact_kind,
            "TraceObjectArtifactKind",
        )?,
        purpose_code: row.get("purpose_code"),
        audit_event_id: row.get("audit_event_id"),
        source_submission_ids: row.get("source_submission_ids"),
        source_submission_ids_hash: row.get("source_submission_ids_hash"),
        item_count: row.get::<_, i32>("item_count").try_into().map_err(|e| {
            DatabaseError::Serialization(format!(
                "invalid trace_export_manifests.item_count column value: {e}"
            ))
        })?,
        generated_at: row.get("generated_at"),
        invalidated_at: row.get("invalidated_at"),
        deleted_at: row.get("deleted_at"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

fn row_to_export_manifest_item(row: &Row) -> Result<TraceExportManifestItemRecord, DatabaseError> {
    let source_status_at_export: String = row.get("source_status_at_export");
    let source_invalidation_reason: Option<String> = row.get("source_invalidation_reason");
    Ok(TraceExportManifestItemRecord {
        tenant_id: row.get("tenant_id"),
        export_manifest_id: row.get("export_manifest_id"),
        submission_id: row.get("submission_id"),
        trace_id: row.get("trace_id"),
        derived_id: row.get("derived_id"),
        object_ref_id: row.get("object_ref_id"),
        vector_entry_id: row.get("vector_entry_id"),
        source_status_at_export: enum_from_storage::<TraceCorpusStatus>(
            &source_status_at_export,
            "TraceCorpusStatus",
        )?,
        source_hash_at_export: row.get("source_hash_at_export"),
        source_invalidated_at: row.get("source_invalidated_at"),
        source_invalidation_reason: source_invalidation_reason
            .as_deref()
            .map(|reason| {
                enum_from_storage::<TraceExportManifestItemInvalidationReason>(
                    reason,
                    "TraceExportManifestItemInvalidationReason",
                )
            })
            .transpose()?,
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

fn row_to_audit_event(row: &Row) -> Result<TraceAuditEventRecord, DatabaseError> {
    let action: String = row.get("action");
    let metadata: serde_json::Value = row.get("metadata_json");
    Ok(TraceAuditEventRecord {
        tenant_id: row.get("tenant_id"),
        audit_sequence: row.get("audit_sequence"),
        audit_event_id: row.get("audit_event_id"),
        actor_principal_ref: row.get("actor_principal_ref"),
        actor_role: row.get("actor_role"),
        action: enum_from_storage(&action, "TraceAuditAction")?,
        reason: row.get("reason"),
        request_id: row.get("request_id"),
        submission_id: row.get("submission_id"),
        object_ref_id: row.get("object_ref_id"),
        export_manifest_id: row.get("export_manifest_id"),
        decision_inputs_hash: row.get("decision_inputs_hash"),
        previous_event_hash: row.get("previous_event_hash"),
        event_hash: row.get("event_hash"),
        canonical_event_json: row.get("canonical_event_json"),
        metadata: serde_json::from_value(metadata).map_err(|e| {
            DatabaseError::Serialization(format!("trace audit metadata JSON decode failed: {e}"))
        })?,
        occurred_at: row.get("occurred_at"),
    })
}

fn row_to_tombstone(row: &Row) -> Result<TraceTombstoneRecord, DatabaseError> {
    Ok(TraceTombstoneRecord {
        tenant_id: row.get("tenant_id"),
        tombstone_id: row.get("tombstone_id"),
        submission_id: row.get("submission_id"),
        trace_id: row.get("trace_id"),
        redaction_hash: row.get("redaction_hash"),
        canonical_summary_hash: row.get("canonical_summary_hash"),
        reason: row.get("reason"),
        effective_at: row.get("effective_at"),
        retain_until: row.get("retain_until"),
        created_by_principal_ref: row.get("created_by_principal_ref"),
        created_at: row.get("created_at"),
    })
}

impl PgBackend {
    async fn ensure_trace_tenant(&self, tenant_id: &str) -> Result<(), DatabaseError> {
        let mut client = self.pool().get().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant_id).await?;
        tx.execute(
            "INSERT INTO trace_tenants (tenant_id) VALUES ($1)
             ON CONFLICT (tenant_id) DO UPDATE SET updated_at = NOW()",
            &[&tenant_id],
        )
        .await
        .map_err(DatabaseError::Postgres)?;
        tx.commit().await.map_err(DatabaseError::Postgres)?;
        Ok(())
    }

    async fn begin_trace_tenant_transaction<'a>(
        client: &'a mut deadpool_postgres::Client,
        tenant_id: &str,
    ) -> Result<deadpool_postgres::Transaction<'a>, DatabaseError> {
        let tx = client
            .transaction()
            .await
            .map_err(DatabaseError::Postgres)?;
        tx.execute(
            "SELECT set_config('ironclaw.trace_tenant_id', $1, true)",
            &[&tenant_id],
        )
        .await
        .map_err(DatabaseError::Postgres)?;
        Ok(tx)
    }
}

#[async_trait]
impl TraceCorpusStore for PgBackend {
    async fn upsert_trace_submission(
        &self,
        submission: TraceSubmissionWrite,
    ) -> Result<TraceSubmissionRecord, DatabaseError> {
        self.ensure_trace_tenant(&submission.tenant_id).await?;
        let mut client = self.pool().get().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, &submission.tenant_id).await?;
        let status = enum_to_storage(submission.status)?;
        let consent_scopes = serde_json::to_value(&submission.consent_scopes).map_err(|e| {
            DatabaseError::Serialization(format!("trace consent scopes encode failed: {e}"))
        })?;
        let allowed_uses = serde_json::to_value(&submission.allowed_uses).map_err(|e| {
            DatabaseError::Serialization(format!("trace allowed uses encode failed: {e}"))
        })?;
        let redaction_counts = serde_json::to_value(&submission.redaction_counts).map_err(|e| {
            DatabaseError::Serialization(format!("trace redaction counts encode failed: {e}"))
        })?;

        let row = tx
            .query_one(
                "INSERT INTO trace_submissions (
                    tenant_id, submission_id, trace_id, auth_principal_ref, contributor_pseudonym,
                    submitted_tenant_scope_ref, schema_version, consent_policy_version,
                    consent_scopes, allowed_uses, retention_policy_id, status, privacy_risk,
                    redaction_pipeline_version, redaction_hash, redaction_counts, canonical_summary_hash,
                    submission_score, credit_points_pending, credit_points_final, expires_at
                 ) VALUES (
                    $1, $2, $3, $4, $5, $6, $7, $8, $9, $10,
                    $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21
                 )
                 ON CONFLICT (tenant_id, submission_id) DO UPDATE SET
                    trace_id = excluded.trace_id,
                    auth_principal_ref = excluded.auth_principal_ref,
                    contributor_pseudonym = excluded.contributor_pseudonym,
                    submitted_tenant_scope_ref = excluded.submitted_tenant_scope_ref,
                    schema_version = excluded.schema_version,
                    consent_policy_version = excluded.consent_policy_version,
                    consent_scopes = excluded.consent_scopes,
                    allowed_uses = excluded.allowed_uses,
                    retention_policy_id = excluded.retention_policy_id,
                    status = excluded.status,
                    privacy_risk = excluded.privacy_risk,
                    redaction_pipeline_version = excluded.redaction_pipeline_version,
                    redaction_hash = excluded.redaction_hash,
                    redaction_counts = excluded.redaction_counts,
                    canonical_summary_hash = excluded.canonical_summary_hash,
                    submission_score = excluded.submission_score,
                    credit_points_pending = excluded.credit_points_pending,
                    credit_points_final = excluded.credit_points_final,
                    expires_at = excluded.expires_at,
                    updated_at = NOW()
                 RETURNING
                    tenant_id, submission_id, trace_id, status, auth_principal_ref,
                    contributor_pseudonym, submitted_tenant_scope_ref, schema_version,
                    consent_policy_version, consent_scopes, allowed_uses, retention_policy_id,
                    privacy_risk, redaction_pipeline_version, redaction_hash,
                    redaction_counts, canonical_summary_hash, submission_score, credit_points_pending,
                    credit_points_final, received_at, updated_at, reviewed_at, revoked_at,
                    expires_at, purged_at",
                &[
                    &submission.tenant_id,
                    &submission.submission_id,
                    &submission.trace_id,
                    &submission.auth_principal_ref,
                    &submission.contributor_pseudonym,
                    &submission.submitted_tenant_scope_ref,
                    &submission.schema_version,
                    &submission.consent_policy_version,
                    &consent_scopes,
                    &allowed_uses,
                    &submission.retention_policy_id,
                    &status,
                    &submission.privacy_risk,
                    &submission.redaction_pipeline_version,
                    &submission.redaction_hash,
                    &redaction_counts,
                    &submission.canonical_summary_hash,
                    &submission.submission_score,
                    &submission.credit_points_pending,
                    &submission.credit_points_final,
                    &submission.expires_at,
                ],
            )
            .await
            .map_err(DatabaseError::Postgres)?;
        let record = row_to_submission(&row)?;
        tx.commit().await.map_err(DatabaseError::Postgres)?;
        Ok(record)
    }

    async fn get_trace_submission(
        &self,
        tenant_id: &str,
        submission_id: Uuid,
    ) -> Result<Option<TraceSubmissionRecord>, DatabaseError> {
        let mut client = self.pool().get().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant_id).await?;
        let row = tx
            .query_opt(
                "SELECT
                    tenant_id, submission_id, trace_id, status, auth_principal_ref,
                    contributor_pseudonym, submitted_tenant_scope_ref, schema_version,
                    consent_policy_version, consent_scopes, allowed_uses, retention_policy_id,
                    privacy_risk, redaction_pipeline_version, redaction_hash,
                    redaction_counts, canonical_summary_hash, submission_score, credit_points_pending,
                    credit_points_final, received_at, updated_at, reviewed_at, revoked_at,
                    expires_at, purged_at
                 FROM trace_submissions
                 WHERE tenant_id = $1 AND submission_id = $2",
                &[&tenant_id, &submission_id],
            )
            .await
            .map_err(DatabaseError::Postgres)?;
        let record = row.as_ref().map(row_to_submission).transpose()?;
        tx.commit().await.map_err(DatabaseError::Postgres)?;
        Ok(record)
    }

    async fn list_trace_submissions(
        &self,
        tenant_id: &str,
    ) -> Result<Vec<TraceSubmissionRecord>, DatabaseError> {
        let mut client = self.pool().get().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant_id).await?;
        let rows = tx
            .query(
                "SELECT
                    tenant_id, submission_id, trace_id, status, auth_principal_ref,
                    contributor_pseudonym, submitted_tenant_scope_ref, schema_version,
                    consent_policy_version, consent_scopes, allowed_uses, retention_policy_id,
                    privacy_risk, redaction_pipeline_version, redaction_hash,
                    redaction_counts, canonical_summary_hash, submission_score, credit_points_pending,
                    credit_points_final, received_at, updated_at, reviewed_at, revoked_at,
                    expires_at, purged_at
                 FROM trace_submissions
                 WHERE tenant_id = $1
                 ORDER BY received_at ASC",
                &[&tenant_id],
            )
            .await
            .map_err(DatabaseError::Postgres)?;
        let records = rows.iter().map(row_to_submission).collect();
        tx.commit().await.map_err(DatabaseError::Postgres)?;
        records
    }

    async fn upsert_trace_tenant_policy(
        &self,
        policy: TraceTenantPolicyWrite,
    ) -> Result<TraceTenantPolicyRecord, DatabaseError> {
        self.ensure_trace_tenant(&policy.tenant_id).await?;
        let mut client = self.pool().get().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, &policy.tenant_id).await?;
        let allowed_consent_scopes =
            serde_json::to_value(&policy.allowed_consent_scopes).map_err(|e| {
                DatabaseError::Serialization(format!(
                    "trace tenant policy consent scopes encode failed: {e}"
                ))
            })?;
        let allowed_uses = serde_json::to_value(&policy.allowed_uses).map_err(|e| {
            DatabaseError::Serialization(format!(
                "trace tenant policy allowed uses encode failed: {e}"
            ))
        })?;
        let row = tx
            .query_one(
                "INSERT INTO trace_tenant_policies (
                    tenant_id, policy_version, allowed_consent_scopes, allowed_uses,
                    updated_by_principal_ref
                 ) VALUES ($1, $2, $3, $4, $5)
                 ON CONFLICT (tenant_id) DO UPDATE SET
                    policy_version = excluded.policy_version,
                    allowed_consent_scopes = excluded.allowed_consent_scopes,
                    allowed_uses = excluded.allowed_uses,
                    updated_by_principal_ref = excluded.updated_by_principal_ref,
                    updated_at = NOW()
                 RETURNING
                    tenant_id, policy_version, allowed_consent_scopes, allowed_uses,
                    updated_by_principal_ref, created_at, updated_at",
                &[
                    &policy.tenant_id,
                    &policy.policy_version,
                    &allowed_consent_scopes,
                    &allowed_uses,
                    &policy.updated_by_principal_ref,
                ],
            )
            .await
            .map_err(DatabaseError::Postgres)?;
        let record = row_to_tenant_policy(&row)?;
        tx.commit().await.map_err(DatabaseError::Postgres)?;
        Ok(record)
    }

    async fn get_trace_tenant_policy(
        &self,
        tenant_id: &str,
    ) -> Result<Option<TraceTenantPolicyRecord>, DatabaseError> {
        let mut client = self.pool().get().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant_id).await?;
        let row = tx
            .query_opt(
                "SELECT
                    tenant_id, policy_version, allowed_consent_scopes, allowed_uses,
                    updated_by_principal_ref, created_at, updated_at
                 FROM trace_tenant_policies
                 WHERE tenant_id = $1",
                &[&tenant_id],
            )
            .await
            .map_err(DatabaseError::Postgres)?;
        let record = row.as_ref().map(row_to_tenant_policy).transpose()?;
        tx.commit().await.map_err(DatabaseError::Postgres)?;
        Ok(record)
    }

    async fn list_trace_credit_events(
        &self,
        tenant_id: &str,
    ) -> Result<Vec<TraceCreditEventRecord>, DatabaseError> {
        let mut client = self.pool().get().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant_id).await?;
        let rows = tx
            .query(
                "SELECT
                    tenant_id, credit_event_id, submission_id, trace_id, credit_account_ref,
                    event_type, points_delta, reason, external_ref, actor_principal_ref,
                    actor_role, settlement_state, occurred_at
                 FROM trace_credit_ledger
                 WHERE tenant_id = $1
                 ORDER BY occurred_at ASC",
                &[&tenant_id],
            )
            .await
            .map_err(DatabaseError::Postgres)?;
        let records = rows.iter().map(row_to_credit_event).collect();
        tx.commit().await.map_err(DatabaseError::Postgres)?;
        records
    }

    async fn update_trace_submission_status(
        &self,
        tenant_id: &str,
        submission_id: Uuid,
        status: TraceCorpusStatus,
        actor_principal_ref: &str,
        reason: Option<&str>,
    ) -> Result<(), DatabaseError> {
        let mut client = self.pool().get().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant_id).await?;
        let status_value = enum_to_storage(status)?;
        let updated = tx
            .execute(
                "UPDATE trace_submissions
                 SET status = $3,
                     updated_at = NOW(),
                     reviewed_at = CASE
                         WHEN $3 IN ('accepted', 'quarantined', 'rejected') THEN NOW()
                         ELSE reviewed_at
                     END,
                     revoked_at = CASE WHEN $3 = 'revoked' THEN NOW() ELSE revoked_at END,
                     purged_at = CASE WHEN $3 = 'purged' THEN NOW() ELSE purged_at END,
                     credit_points_pending = CASE
                         WHEN $3 IN ('revoked', 'expired', 'purged') THEN 0
                         ELSE credit_points_pending
                     END,
                     credit_points_final = CASE
                         WHEN $3 IN ('revoked', 'expired', 'purged') THEN 0
                         ELSE credit_points_final
                     END
                 WHERE tenant_id = $1 AND submission_id = $2",
                &[&tenant_id, &submission_id, &status_value],
            )
            .await
            .map_err(DatabaseError::Postgres)?;
        if updated == 0 {
            return Err(DatabaseError::NotFound {
                entity: "trace_submission".to_string(),
                id: submission_id.to_string(),
            });
        }
        tx.commit().await.map_err(DatabaseError::Postgres)?;

        self.append_trace_audit_event(TraceAuditEventWrite {
            audit_event_id: Uuid::new_v4(),
            tenant_id: tenant_id.to_string(),
            actor_principal_ref: actor_principal_ref.to_string(),
            actor_role: "system".to_string(),
            action: audit_action_for_status(status),
            reason: reason.map(str::to_string),
            request_id: None,
            submission_id: Some(submission_id),
            object_ref_id: None,
            export_manifest_id: None,
            decision_inputs_hash: None,
            previous_event_hash: None,
            event_hash: None,
            canonical_event_json: None,
            metadata: TraceAuditSafeMetadata::ReviewDecision {
                decision: status_value,
                resulting_status: status,
                reason_code: reason.map(str::to_string),
            },
        })
        .await
    }

    async fn append_trace_object_ref(
        &self,
        object_ref: TraceObjectRefWrite,
    ) -> Result<(), DatabaseError> {
        self.ensure_trace_tenant(&object_ref.tenant_id).await?;
        let mut client = self.pool().get().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, &object_ref.tenant_id).await?;
        let artifact_kind = enum_to_storage(object_ref.artifact_kind)?;
        tx.execute(
            "INSERT INTO trace_object_refs (
                    tenant_id, submission_id, object_ref_id, artifact_kind, object_store,
                    object_key, content_sha256, encryption_key_ref, size_bytes, compression,
                    created_by_job_id
                 ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
                 ON CONFLICT (tenant_id, submission_id, object_ref_id) DO UPDATE SET
                    artifact_kind = excluded.artifact_kind,
                    object_store = excluded.object_store,
                    object_key = excluded.object_key,
                    content_sha256 = excluded.content_sha256,
                    encryption_key_ref = excluded.encryption_key_ref,
                    size_bytes = excluded.size_bytes,
                    compression = excluded.compression,
                    created_by_job_id = excluded.created_by_job_id,
                    updated_at = NOW()",
            &[
                &object_ref.tenant_id,
                &object_ref.submission_id,
                &object_ref.object_ref_id,
                &artifact_kind,
                &object_ref.object_store,
                &object_ref.object_key,
                &object_ref.content_sha256,
                &object_ref.encryption_key_ref,
                &object_ref.size_bytes,
                &object_ref.compression,
                &object_ref.created_by_job_id,
            ],
        )
        .await
        .map_err(DatabaseError::Postgres)?;
        tx.commit().await.map_err(DatabaseError::Postgres)?;
        Ok(())
    }

    async fn list_trace_object_refs(
        &self,
        tenant_id: &str,
        submission_id: Uuid,
    ) -> Result<Vec<TraceObjectRefRecord>, DatabaseError> {
        let mut client = self.pool().get().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant_id).await?;
        let rows = tx
            .query(
                &format!(
                    "SELECT {TRACE_OBJECT_REF_COLUMNS}
                     FROM trace_object_refs
                     WHERE tenant_id = $1 AND submission_id = $2
                     ORDER BY created_at ASC"
                ),
                &[&tenant_id, &submission_id],
            )
            .await
            .map_err(DatabaseError::Postgres)?;
        let records = rows.iter().map(row_to_object_ref).collect();
        tx.commit().await.map_err(DatabaseError::Postgres)?;
        records
    }

    async fn get_latest_active_trace_object_ref(
        &self,
        tenant_id: &str,
        submission_id: Uuid,
        artifact_kind: TraceObjectArtifactKind,
    ) -> Result<Option<TraceObjectRefRecord>, DatabaseError> {
        let mut client = self.pool().get().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant_id).await?;
        let artifact_kind = enum_to_storage(artifact_kind)?;
        let row = tx
            .query_opt(
                &format!(
                    "SELECT {TRACE_OBJECT_REF_COLUMNS}
                     FROM trace_object_refs
                     WHERE tenant_id = $1
                       AND submission_id = $2
                       AND artifact_kind = $3
                       AND invalidated_at IS NULL
                       AND deleted_at IS NULL
                     ORDER BY created_at DESC
                     LIMIT 1"
                ),
                &[&tenant_id, &submission_id, &artifact_kind],
            )
            .await
            .map_err(DatabaseError::Postgres)?;
        let record = row.as_ref().map(row_to_object_ref).transpose()?;
        tx.commit().await.map_err(DatabaseError::Postgres)?;
        Ok(record)
    }

    async fn append_trace_derived_record(
        &self,
        derived_record: TraceDerivedRecordWrite,
    ) -> Result<(), DatabaseError> {
        self.ensure_trace_tenant(&derived_record.tenant_id).await?;
        let mut client = self.pool().get().await?;
        let tx =
            Self::begin_trace_tenant_transaction(&mut client, &derived_record.tenant_id).await?;
        if let Some(object_ref) = derived_record.input_object_ref.as_ref() {
            validate_tenant_scoped_trace_object_ref(
                "derived input",
                object_ref,
                &derived_record.tenant_id,
                derived_record.submission_id,
            )?;
            ensure_pg_object_ref_belongs_to_submission(
                &tx,
                &derived_record.tenant_id,
                derived_record.submission_id,
                object_ref.object_ref_id,
                "derived input",
            )
            .await?;
        }
        if let Some(object_ref) = derived_record.output_object_ref.as_ref() {
            validate_tenant_scoped_trace_object_ref(
                "derived output",
                object_ref,
                &derived_record.tenant_id,
                derived_record.submission_id,
            )?;
            ensure_pg_object_ref_belongs_to_submission(
                &tx,
                &derived_record.tenant_id,
                derived_record.submission_id,
                object_ref.object_ref_id,
                "derived output",
            )
            .await?;
        }
        let status = enum_to_storage(derived_record.status)?;
        let worker_kind = enum_to_storage(derived_record.worker_kind)?;
        let input_object_ref_id = derived_record
            .input_object_ref
            .as_ref()
            .map(|object_ref| object_ref.object_ref_id);
        let output_object_ref_id = derived_record
            .output_object_ref
            .as_ref()
            .map(|object_ref| object_ref.object_ref_id);
        let tool_sequence = serde_json::to_value(&derived_record.tool_sequence).map_err(|e| {
            DatabaseError::Serialization(format!("trace tool sequence encode failed: {e}"))
        })?;
        let tool_categories =
            serde_json::to_value(&derived_record.tool_categories).map_err(|e| {
                DatabaseError::Serialization(format!("trace tool categories encode failed: {e}"))
            })?;
        let coverage_tags = serde_json::to_value(&derived_record.coverage_tags).map_err(|e| {
            DatabaseError::Serialization(format!("trace coverage tags encode failed: {e}"))
        })?;

        tx.execute(
            "INSERT INTO trace_derived_records (
                    tenant_id, derived_id, submission_id, trace_id, status, worker_kind,
                    worker_version, input_object_ref_id, input_hash, output_object_ref_id,
                    canonical_summary, canonical_summary_hash, summary_model, task_success,
                    privacy_risk, event_count, tool_sequence, tool_categories, coverage_tags,
                    duplicate_score, novelty_score, cluster_id
                 ) VALUES (
                    $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12,
                    $13, $14, $15, $16, $17, $18, $19, $20, $21, $22
                 )
                 ON CONFLICT (tenant_id, derived_id) DO UPDATE SET
                    status = excluded.status,
                    worker_kind = excluded.worker_kind,
                    worker_version = excluded.worker_version,
                    input_object_ref_id = excluded.input_object_ref_id,
                    input_hash = excluded.input_hash,
                    output_object_ref_id = excluded.output_object_ref_id,
                    canonical_summary = excluded.canonical_summary,
                    canonical_summary_hash = excluded.canonical_summary_hash,
                    summary_model = excluded.summary_model,
                    task_success = excluded.task_success,
                    privacy_risk = excluded.privacy_risk,
                    event_count = excluded.event_count,
                    tool_sequence = excluded.tool_sequence,
                    tool_categories = excluded.tool_categories,
                    coverage_tags = excluded.coverage_tags,
                    duplicate_score = excluded.duplicate_score,
                    novelty_score = excluded.novelty_score,
                    cluster_id = excluded.cluster_id,
                    updated_at = NOW()",
            &[
                &derived_record.tenant_id,
                &derived_record.derived_id,
                &derived_record.submission_id,
                &derived_record.trace_id,
                &status,
                &worker_kind,
                &derived_record.worker_version,
                &input_object_ref_id,
                &derived_record.input_hash,
                &output_object_ref_id,
                &derived_record.canonical_summary,
                &derived_record.canonical_summary_hash,
                &derived_record.summary_model,
                &derived_record.task_success,
                &derived_record.privacy_risk,
                &derived_record.event_count,
                &tool_sequence,
                &tool_categories,
                &coverage_tags,
                &derived_record.duplicate_score,
                &derived_record.novelty_score,
                &derived_record.cluster_id,
            ],
        )
        .await
        .map_err(DatabaseError::Postgres)?;
        tx.commit().await.map_err(DatabaseError::Postgres)?;
        Ok(())
    }

    async fn list_trace_derived_records(
        &self,
        tenant_id: &str,
    ) -> Result<Vec<TraceDerivedRecord>, DatabaseError> {
        let mut client = self.pool().get().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant_id).await?;
        let rows = tx
            .query(
                "SELECT
                    tenant_id, derived_id, submission_id, trace_id, status, worker_kind,
                    worker_version, input_object_ref_id, input_hash, output_object_ref_id,
                    canonical_summary, canonical_summary_hash, summary_model, task_success,
                    privacy_risk, event_count, tool_sequence, tool_categories, coverage_tags,
                    duplicate_score, novelty_score, cluster_id, created_at, updated_at
                 FROM trace_derived_records
                 WHERE tenant_id = $1
                 ORDER BY created_at ASC",
                &[&tenant_id],
            )
            .await
            .map_err(DatabaseError::Postgres)?;
        let records = rows.iter().map(row_to_derived_record).collect();
        tx.commit().await.map_err(DatabaseError::Postgres)?;
        records
    }

    async fn upsert_trace_vector_entry(
        &self,
        vector_entry: TraceVectorEntryWrite,
    ) -> Result<TraceVectorEntryRecord, DatabaseError> {
        self.ensure_trace_tenant(&vector_entry.tenant_id).await?;
        let mut client = self.pool().get().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, &vector_entry.tenant_id).await?;
        ensure_pg_derived_record_belongs_to_submission(
            &tx,
            &vector_entry.tenant_id,
            vector_entry.submission_id,
            vector_entry.derived_id,
        )
        .await?;
        let source_projection = enum_to_storage(vector_entry.source_projection)?;
        let status = enum_to_storage(vector_entry.status)?;
        let row = tx
            .query_one(
                "INSERT INTO trace_vector_entries (
                    tenant_id, submission_id, derived_id, vector_entry_id, vector_store,
                    embedding_model, embedding_dimension, embedding_version, source_projection,
                    source_hash, status, nearest_trace_ids, cluster_id, duplicate_score,
                    novelty_score, indexed_at, invalidated_at, deleted_at
                 ) VALUES (
                    $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13,
                    $14, $15, $16, $17, $18
                 )
                 ON CONFLICT (tenant_id, submission_id, vector_entry_id) DO UPDATE SET
                    derived_id = excluded.derived_id,
                    vector_store = excluded.vector_store,
                    embedding_model = excluded.embedding_model,
                    embedding_dimension = excluded.embedding_dimension,
                    embedding_version = excluded.embedding_version,
                    source_projection = excluded.source_projection,
                    source_hash = excluded.source_hash,
                    status = excluded.status,
                    nearest_trace_ids = excluded.nearest_trace_ids,
                    cluster_id = excluded.cluster_id,
                    duplicate_score = excluded.duplicate_score,
                    novelty_score = excluded.novelty_score,
                    indexed_at = excluded.indexed_at,
                    invalidated_at = excluded.invalidated_at,
                    deleted_at = excluded.deleted_at,
                    updated_at = NOW()
                 RETURNING
                    tenant_id, submission_id, derived_id, vector_entry_id, vector_store,
                    embedding_model, embedding_dimension, embedding_version, source_projection,
                    source_hash, status, nearest_trace_ids, cluster_id, duplicate_score,
                    novelty_score, indexed_at, invalidated_at, deleted_at, created_at, updated_at",
                &[
                    &vector_entry.tenant_id,
                    &vector_entry.submission_id,
                    &vector_entry.derived_id,
                    &vector_entry.vector_entry_id,
                    &vector_entry.vector_store,
                    &vector_entry.embedding_model,
                    &vector_entry.embedding_dimension,
                    &vector_entry.embedding_version,
                    &source_projection,
                    &vector_entry.source_hash,
                    &status,
                    &vector_entry.nearest_trace_ids,
                    &vector_entry.cluster_id,
                    &vector_entry.duplicate_score,
                    &vector_entry.novelty_score,
                    &vector_entry.indexed_at,
                    &vector_entry.invalidated_at,
                    &vector_entry.deleted_at,
                ],
            )
            .await
            .map_err(DatabaseError::Postgres)?;
        let record = row_to_vector_entry(&row)?;
        tx.commit().await.map_err(DatabaseError::Postgres)?;
        Ok(record)
    }

    async fn list_trace_vector_entries(
        &self,
        tenant_id: &str,
    ) -> Result<Vec<TraceVectorEntryRecord>, DatabaseError> {
        let mut client = self.pool().get().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant_id).await?;
        let rows = tx
            .query(
                "SELECT
                    tenant_id, submission_id, derived_id, vector_entry_id, vector_store,
                    embedding_model, embedding_dimension, embedding_version, source_projection,
                    source_hash, status, nearest_trace_ids, cluster_id, duplicate_score,
                    novelty_score, indexed_at, invalidated_at, deleted_at, created_at, updated_at
                 FROM trace_vector_entries
                 WHERE tenant_id = $1
                 ORDER BY created_at ASC",
                &[&tenant_id],
            )
            .await
            .map_err(DatabaseError::Postgres)?;
        let records = rows.iter().map(row_to_vector_entry).collect();
        tx.commit().await.map_err(DatabaseError::Postgres)?;
        records
    }

    async fn upsert_trace_export_manifest(
        &self,
        manifest: TraceExportManifestWrite,
    ) -> Result<TraceExportManifestRecord, DatabaseError> {
        self.ensure_trace_tenant(&manifest.tenant_id).await?;
        let mut client = self.pool().get().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, &manifest.tenant_id).await?;
        let artifact_kind = enum_to_storage(manifest.artifact_kind)?;
        let item_count = i32::try_from(manifest.item_count).map_err(|e| {
            DatabaseError::Serialization(format!(
                "trace export manifest item_count exceeds PostgreSQL integer range: {e}"
            ))
        })?;
        let row = tx
            .query_one(
                &format!(
                    "INSERT INTO trace_export_manifests (
                        tenant_id, export_manifest_id, artifact_kind, purpose_code,
                        audit_event_id, source_submission_ids, source_submission_ids_hash,
                        item_count, generated_at
                     ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
                     ON CONFLICT (tenant_id, export_manifest_id) DO UPDATE SET
                        artifact_kind = excluded.artifact_kind,
                        purpose_code = excluded.purpose_code,
                        audit_event_id = excluded.audit_event_id,
                        source_submission_ids = excluded.source_submission_ids,
                        source_submission_ids_hash = excluded.source_submission_ids_hash,
                        item_count = excluded.item_count,
                        generated_at = excluded.generated_at,
                        invalidated_at = NULL,
                        deleted_at = NULL,
                        updated_at = NOW()
                     RETURNING {TRACE_EXPORT_MANIFEST_COLUMNS}"
                ),
                &[
                    &manifest.tenant_id,
                    &manifest.export_manifest_id,
                    &artifact_kind,
                    &manifest.purpose_code,
                    &manifest.audit_event_id,
                    &manifest.source_submission_ids,
                    &manifest.source_submission_ids_hash,
                    &item_count,
                    &manifest.generated_at,
                ],
            )
            .await
            .map_err(DatabaseError::Postgres)?;
        let record = row_to_export_manifest(&row)?;
        tx.commit().await.map_err(DatabaseError::Postgres)?;
        Ok(record)
    }

    async fn list_trace_export_manifests(
        &self,
        tenant_id: &str,
    ) -> Result<Vec<TraceExportManifestRecord>, DatabaseError> {
        let mut client = self.pool().get().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant_id).await?;
        let rows = tx
            .query(
                &format!(
                    "SELECT {TRACE_EXPORT_MANIFEST_COLUMNS}
                     FROM trace_export_manifests
                     WHERE tenant_id = $1
                     ORDER BY generated_at ASC"
                ),
                &[&tenant_id],
            )
            .await
            .map_err(DatabaseError::Postgres)?;
        let records = rows.iter().map(row_to_export_manifest).collect();
        tx.commit().await.map_err(DatabaseError::Postgres)?;
        records
    }

    async fn upsert_trace_export_manifest_item(
        &self,
        item: TraceExportManifestItemWrite,
    ) -> Result<TraceExportManifestItemRecord, DatabaseError> {
        self.ensure_trace_tenant(&item.tenant_id).await?;
        let mut client = self.pool().get().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, &item.tenant_id).await?;
        if let Some(derived_id) = item.derived_id {
            ensure_pg_derived_record_belongs_to_submission(
                &tx,
                &item.tenant_id,
                item.submission_id,
                derived_id,
            )
            .await?;
        }
        if let Some(object_ref_id) = item.object_ref_id {
            ensure_pg_object_ref_belongs_to_submission(
                &tx,
                &item.tenant_id,
                item.submission_id,
                object_ref_id,
                "export manifest item",
            )
            .await?;
        }
        if let Some(vector_entry_id) = item.vector_entry_id {
            ensure_pg_vector_entry_belongs_to_submission(
                &tx,
                &item.tenant_id,
                item.submission_id,
                vector_entry_id,
            )
            .await?;
        }
        let source_status_at_export = enum_to_storage(item.source_status_at_export)?;
        let row = tx
            .query_one(
                &format!(
                    "INSERT INTO trace_export_manifest_items (
                        tenant_id, export_manifest_id, submission_id, trace_id, derived_id,
                        object_ref_id, vector_entry_id, source_status_at_export,
                        source_hash_at_export
                     ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
                     ON CONFLICT (tenant_id, export_manifest_id, submission_id) DO UPDATE SET
                        trace_id = excluded.trace_id,
                        derived_id = excluded.derived_id,
                        object_ref_id = excluded.object_ref_id,
                        vector_entry_id = excluded.vector_entry_id,
                        source_status_at_export = excluded.source_status_at_export,
                        source_hash_at_export = excluded.source_hash_at_export,
                        source_invalidated_at = NULL,
                        source_invalidation_reason = NULL,
                        updated_at = NOW()
                     RETURNING {TRACE_EXPORT_MANIFEST_ITEM_COLUMNS}"
                ),
                &[
                    &item.tenant_id,
                    &item.export_manifest_id,
                    &item.submission_id,
                    &item.trace_id,
                    &item.derived_id,
                    &item.object_ref_id,
                    &item.vector_entry_id,
                    &source_status_at_export,
                    &item.source_hash_at_export,
                ],
            )
            .await
            .map_err(DatabaseError::Postgres)?;
        let record = row_to_export_manifest_item(&row)?;
        tx.commit().await.map_err(DatabaseError::Postgres)?;
        Ok(record)
    }

    async fn list_trace_export_manifest_items(
        &self,
        tenant_id: &str,
        export_manifest_id: Uuid,
    ) -> Result<Vec<TraceExportManifestItemRecord>, DatabaseError> {
        let mut client = self.pool().get().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant_id).await?;
        let rows = tx
            .query(
                &format!(
                    "SELECT {TRACE_EXPORT_MANIFEST_ITEM_COLUMNS}
                     FROM trace_export_manifest_items
                     WHERE tenant_id = $1 AND export_manifest_id = $2
                     ORDER BY created_at ASC"
                ),
                &[&tenant_id, &export_manifest_id],
            )
            .await
            .map_err(DatabaseError::Postgres)?;
        let records = rows.iter().map(row_to_export_manifest_item).collect();
        tx.commit().await.map_err(DatabaseError::Postgres)?;
        records
    }

    async fn invalidate_trace_export_manifests_for_submission(
        &self,
        tenant_id: &str,
        submission_id: Uuid,
    ) -> Result<u64, DatabaseError> {
        let mut client = self.pool().get().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant_id).await?;
        let updated = tx
            .execute(
                "UPDATE trace_export_manifests
                 SET invalidated_at = COALESCE(invalidated_at, NOW()),
                     updated_at = NOW()
                 WHERE tenant_id = $1
                   AND $2 = ANY(source_submission_ids)
                   AND invalidated_at IS NULL
                   AND deleted_at IS NULL",
                &[&tenant_id, &submission_id],
            )
            .await
            .map_err(DatabaseError::Postgres)?;
        tx.commit().await.map_err(DatabaseError::Postgres)?;
        Ok(updated)
    }

    async fn invalidate_trace_export_manifest_items_for_submission(
        &self,
        tenant_id: &str,
        submission_id: Uuid,
        reason: TraceExportManifestItemInvalidationReason,
    ) -> Result<u64, DatabaseError> {
        let mut client = self.pool().get().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant_id).await?;
        let reason = enum_to_storage(reason)?;
        let updated = tx
            .execute(
                "UPDATE trace_export_manifest_items
                 SET source_invalidated_at = COALESCE(source_invalidated_at, NOW()),
                     source_invalidation_reason = $3,
                     updated_at = NOW()
                 WHERE tenant_id = $1
                   AND submission_id = $2
                   AND source_invalidated_at IS NULL",
                &[&tenant_id, &submission_id, &reason],
            )
            .await
            .map_err(DatabaseError::Postgres)?;
        tx.commit().await.map_err(DatabaseError::Postgres)?;
        Ok(updated)
    }

    async fn invalidate_trace_vector_entries_for_submission(
        &self,
        tenant_id: &str,
        submission_id: Uuid,
    ) -> Result<u64, DatabaseError> {
        let mut client = self.pool().get().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant_id).await?;
        let invalidated = enum_to_storage(TraceVectorEntryStatus::Invalidated)?;
        let updated = tx
            .execute(
                "UPDATE trace_vector_entries
                 SET status = $3,
                     invalidated_at = COALESCE(invalidated_at, NOW()),
                     updated_at = NOW()
                 WHERE tenant_id = $1
                   AND submission_id = $2
                   AND status <> $3
                   AND deleted_at IS NULL",
                &[&tenant_id, &submission_id, &invalidated],
            )
            .await
            .map_err(DatabaseError::Postgres)?;
        tx.commit().await.map_err(DatabaseError::Postgres)?;
        Ok(updated)
    }

    async fn append_trace_audit_event(
        &self,
        audit_event: TraceAuditEventWrite,
    ) -> Result<(), DatabaseError> {
        self.ensure_trace_tenant(&audit_event.tenant_id).await?;
        let mut client = self.pool().get().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, &audit_event.tenant_id).await?;
        tx.execute(
            "SELECT pg_advisory_xact_lock(hashtext($1)::bigint)",
            &[&audit_event.tenant_id],
        )
        .await
        .map_err(DatabaseError::Postgres)?;
        let latest_event_hash: Option<String> = tx
            .query_opt(
                "SELECT event_hash
                 FROM trace_audit_events
                 WHERE tenant_id = $1
                   AND event_hash IS NOT NULL
                 ORDER BY audit_sequence DESC
                 LIMIT 1",
                &[&audit_event.tenant_id],
            )
            .await
            .map_err(DatabaseError::Postgres)?
            .map(|row| row.get("event_hash"));
        validate_trace_audit_append_chain(
            &audit_event.tenant_id,
            audit_event.audit_event_id,
            latest_event_hash.as_deref(),
            audit_event.previous_event_hash.as_deref(),
            audit_event.event_hash.is_some(),
        )?;
        let next_audit_sequence: i64 = tx
            .query_one(
                "SELECT COALESCE(MAX(audit_sequence), 0) + 1
                 FROM trace_audit_events
                 WHERE tenant_id = $1",
                &[&audit_event.tenant_id],
            )
            .await
            .map_err(DatabaseError::Postgres)?
            .get(0);
        let action = enum_to_storage(audit_event.action)?;
        let metadata_json = serde_json::to_value(&audit_event.metadata).map_err(|e| {
            DatabaseError::Serialization(format!("trace audit metadata encode failed: {e}"))
        })?;
        tx.execute(
            "INSERT INTO trace_audit_events (
                    tenant_id, audit_sequence, audit_event_id, actor_principal_ref, actor_role,
                    action, reason, request_id, submission_id, object_ref_id, export_manifest_id,
                    decision_inputs_hash, previous_event_hash, event_hash, canonical_event_json,
                    metadata_json
                 ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16)",
            &[
                &audit_event.tenant_id,
                &next_audit_sequence,
                &audit_event.audit_event_id,
                &audit_event.actor_principal_ref,
                &audit_event.actor_role,
                &action,
                &audit_event.reason,
                &audit_event.request_id,
                &audit_event.submission_id,
                &audit_event.object_ref_id,
                &audit_event.export_manifest_id,
                &audit_event.decision_inputs_hash,
                &audit_event.previous_event_hash,
                &audit_event.event_hash,
                &audit_event.canonical_event_json,
                &metadata_json,
            ],
        )
        .await
        .map_err(DatabaseError::Postgres)?;
        tx.commit().await.map_err(DatabaseError::Postgres)?;
        Ok(())
    }

    async fn list_trace_audit_events(
        &self,
        tenant_id: &str,
    ) -> Result<Vec<TraceAuditEventRecord>, DatabaseError> {
        let mut client = self.pool().get().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant_id).await?;
        let rows = tx
            .query(
                "SELECT
                    tenant_id, audit_sequence, audit_event_id, actor_principal_ref, actor_role,
                    action, reason, request_id, submission_id, object_ref_id, export_manifest_id,
                    decision_inputs_hash, previous_event_hash, event_hash, canonical_event_json,
                    metadata_json,
                    occurred_at
                 FROM trace_audit_events
                 WHERE tenant_id = $1
                 ORDER BY audit_sequence ASC",
                &[&tenant_id],
            )
            .await
            .map_err(DatabaseError::Postgres)?;
        let records = rows.iter().map(row_to_audit_event).collect();
        tx.commit().await.map_err(DatabaseError::Postgres)?;
        records
    }

    async fn append_trace_credit_event(
        &self,
        credit_event: TraceCreditEventWrite,
    ) -> Result<(), DatabaseError> {
        self.ensure_trace_tenant(&credit_event.tenant_id).await?;
        let mut client = self.pool().get().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, &credit_event.tenant_id).await?;
        let event_type = enum_to_storage(credit_event.event_type)?;
        let settlement_state = enum_to_storage(credit_event.settlement_state)?;
        tx.execute(
            "INSERT INTO trace_credit_ledger (
                    tenant_id, credit_event_id, submission_id, trace_id, credit_account_ref,
                    event_type, points_delta, reason, external_ref, actor_principal_ref,
                    actor_role, settlement_state
                 ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
            &[
                &credit_event.tenant_id,
                &credit_event.credit_event_id,
                &credit_event.submission_id,
                &credit_event.trace_id,
                &credit_event.credit_account_ref,
                &event_type,
                &credit_event.points_delta,
                &credit_event.reason,
                &credit_event.external_ref,
                &credit_event.actor_principal_ref,
                &credit_event.actor_role,
                &settlement_state,
            ],
        )
        .await
        .map_err(DatabaseError::Postgres)?;
        tx.commit().await.map_err(DatabaseError::Postgres)?;
        Ok(())
    }

    async fn write_trace_tombstone(
        &self,
        tombstone: TraceTombstoneWrite,
    ) -> Result<(), DatabaseError> {
        self.ensure_trace_tenant(&tombstone.tenant_id).await?;
        let mut client = self.pool().get().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, &tombstone.tenant_id).await?;
        tx.execute(
            "INSERT INTO trace_tombstones (
                    tenant_id, tombstone_id, submission_id, trace_id, redaction_hash,
                    canonical_summary_hash, reason, effective_at, retain_until,
                    created_by_principal_ref
                 ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
                 ON CONFLICT (tenant_id, submission_id) DO NOTHING",
            &[
                &tombstone.tenant_id,
                &tombstone.tombstone_id,
                &tombstone.submission_id,
                &tombstone.trace_id,
                &tombstone.redaction_hash,
                &tombstone.canonical_summary_hash,
                &tombstone.reason,
                &tombstone.effective_at,
                &tombstone.retain_until,
                &tombstone.created_by_principal_ref,
            ],
        )
        .await
        .map_err(DatabaseError::Postgres)?;
        tx.commit().await.map_err(DatabaseError::Postgres)?;
        Ok(())
    }

    async fn list_trace_tombstones(
        &self,
        tenant_id: &str,
    ) -> Result<Vec<TraceTombstoneRecord>, DatabaseError> {
        let mut client = self.pool().get().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant_id).await?;
        let rows = tx
            .query(
                &format!(
                    "SELECT {TRACE_TOMBSTONE_COLUMNS}
                     FROM trace_tombstones
                     WHERE tenant_id = $1
                     ORDER BY effective_at ASC, created_at ASC"
                ),
                &[&tenant_id],
            )
            .await
            .map_err(DatabaseError::Postgres)?;
        let records = rows.iter().map(row_to_tombstone).collect();
        tx.commit().await.map_err(DatabaseError::Postgres)?;
        records
    }

    async fn invalidate_trace_submission_artifacts(
        &self,
        tenant_id: &str,
        submission_id: Uuid,
        derived_status: TraceDerivedStatus,
    ) -> Result<TraceArtifactInvalidationCounts, DatabaseError> {
        let mut client = self.pool().get().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant_id).await?;
        let derived_status = enum_to_storage(derived_status)?;
        let object_refs_invalidated = tx
            .execute(
                "UPDATE trace_object_refs
                 SET invalidated_at = COALESCE(invalidated_at, NOW()),
                     updated_at = NOW()
                 WHERE tenant_id = $1
                   AND submission_id = $2
                   AND invalidated_at IS NULL
                   AND deleted_at IS NULL",
                &[&tenant_id, &submission_id],
            )
            .await
            .map_err(DatabaseError::Postgres)?;
        let derived_records_invalidated = tx
            .execute(
                "UPDATE trace_derived_records
                 SET status = $3,
                     updated_at = NOW()
                 WHERE tenant_id = $1
                   AND submission_id = $2
                   AND status <> $3",
                &[&tenant_id, &submission_id, &derived_status],
            )
            .await
            .map_err(DatabaseError::Postgres)?;
        tx.commit().await.map_err(DatabaseError::Postgres)?;
        Ok(TraceArtifactInvalidationCounts {
            object_refs_invalidated,
            derived_records_invalidated,
        })
    }
}
