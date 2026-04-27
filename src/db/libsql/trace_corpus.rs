use std::collections::BTreeMap;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use super::{LibSqlBackend, fmt_opt_ts, fmt_ts, get_opt_text, get_opt_ts, get_text, get_ts};
use crate::db::trace_corpus_common::{
    audit_action_for_status, enum_from_storage, enum_to_storage, parse_uuid,
    validate_tenant_scoped_trace_object_ref, validate_trace_audit_append_chain,
};
use crate::error::DatabaseError;
use crate::trace_corpus_storage::{
    TenantScopedTraceObjectRef, TraceArtifactInvalidationCounts, TraceAuditEventRecord,
    TraceAuditEventWrite, TraceAuditSafeMetadata, TraceCorpusStatus, TraceCorpusStore,
    TraceCreditEventRecord, TraceCreditEventWrite, TraceCreditSettlementState, TraceDerivedRecord,
    TraceDerivedRecordWrite, TraceDerivedStatus, TraceExportAccessGrantRecord,
    TraceExportAccessGrantStatus, TraceExportAccessGrantWrite, TraceExportJobRecord,
    TraceExportJobStatus, TraceExportJobStatusUpdate, TraceExportJobWrite,
    TraceExportManifestItemInvalidationReason, TraceExportManifestItemRecord,
    TraceExportManifestItemWrite, TraceExportManifestMirrorWrite, TraceExportManifestRecord,
    TraceExportManifestWrite, TraceObjectArtifactKind, TraceObjectRefRecord, TraceObjectRefWrite,
    TraceRetentionJobItemAction, TraceRetentionJobItemRecord, TraceRetentionJobItemStatus,
    TraceRetentionJobItemWrite, TraceRetentionJobRecord, TraceRetentionJobStatus,
    TraceRetentionJobWrite, TraceRevocationPropagationAction, TraceRevocationPropagationItemRecord,
    TraceRevocationPropagationItemStatus, TraceRevocationPropagationItemStatusUpdate,
    TraceRevocationPropagationItemWrite, TraceRevocationPropagationTarget,
    TraceRevocationPropagationTargetKind, TraceSubmissionRecord, TraceSubmissionWrite,
    TraceTenantPolicyRecord, TraceTenantPolicyWrite, TraceTombstoneRecord, TraceTombstoneWrite,
    TraceVectorEntryRecord, TraceVectorEntrySourceProjection, TraceVectorEntryStatus,
    TraceVectorEntryWrite, TraceWorkerKind,
};

const TRACE_SUBMISSION_COLUMNS: &str = "\
    tenant_id, submission_id, trace_id, status, auth_principal_ref, \
    contributor_pseudonym, submitted_tenant_scope_ref, schema_version, \
    consent_policy_version, consent_scopes, allowed_uses, retention_policy_id, \
    privacy_risk, redaction_pipeline_version, redaction_hash, \
    redaction_counts, canonical_summary_hash, submission_score, \
    credit_points_pending, credit_points_final, received_at, updated_at, \
    reviewed_at, review_assigned_to_principal_ref, review_assigned_at, \
    review_lease_expires_at, review_due_at, revoked_at, expires_at, purged_at";

const TRACE_TENANT_POLICY_COLUMNS: &str = "\
    tenant_id, policy_version, allowed_consent_scopes, allowed_uses, \
    updated_by_principal_ref, created_at, updated_at";

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

const TRACE_RETENTION_JOB_COLUMNS: &str = "\
    tenant_id, retention_job_id, purpose, dry_run, status, requested_by_principal_ref, \
    requested_by_role, purge_expired_before, prune_export_cache, max_export_age_hours, \
    audit_event_id, action_counts, selected_revoked_count, selected_expired_count, \
    started_at, completed_at, created_at, updated_at";

const TRACE_RETENTION_JOB_ITEM_COLUMNS: &str = "\
    tenant_id, retention_job_id, submission_id, action, status, reason, action_counts, \
    verified_at, created_at, updated_at";

const TRACE_REVOCATION_PROPAGATION_ITEM_COLUMNS: &str = "\
    tenant_id, propagation_item_id, source_submission_id, trace_id, target_kind, target_json, \
    action, status, idempotency_key, reason, attempt_count, last_error, next_attempt_at, \
    completed_at, evidence_hash, metadata_json, created_at, updated_at";

const TRACE_EXPORT_ACCESS_GRANT_COLUMNS: &str = "\
    tenant_id, export_job_id, grant_id, caller_principal_ref, requested_dataset_kind, \
    purpose, max_item_cap, status, requested_at, expires_at, metadata_json, created_at, updated_at";

const TRACE_EXPORT_JOB_COLUMNS: &str = "\
    tenant_id, export_job_id, grant_id, caller_principal_ref, requested_dataset_kind, \
    purpose, max_item_cap, status, requested_at, started_at, finished_at, expires_at, \
    result_manifest_id, item_count, last_error, metadata_json, created_at, updated_at";

async fn ensure_libsql_object_ref_belongs_to_submission(
    conn: &libsql::Connection,
    tenant_id: &str,
    submission_id: Uuid,
    object_ref_id: Uuid,
    field: &str,
) -> Result<(), DatabaseError> {
    let mut rows = conn
        .query(
            "SELECT 1
             FROM trace_object_refs
             WHERE tenant_id = ?1
               AND submission_id = ?2
               AND object_ref_id = ?3
             LIMIT 1",
            libsql::params![
                tenant_id,
                submission_id.to_string(),
                object_ref_id.to_string()
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
    let exists = rows
        .next()
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?
        .is_some();
    if exists {
        return Ok(());
    }

    Err(DatabaseError::Constraint(format!(
        "trace {field} object_ref_id {object_ref_id} does not belong to tenant {tenant_id} submission {submission_id}"
    )))
}

async fn ensure_libsql_derived_record_belongs_to_submission(
    conn: &libsql::Connection,
    tenant_id: &str,
    submission_id: Uuid,
    derived_id: Uuid,
) -> Result<(), DatabaseError> {
    let mut rows = conn
        .query(
            "SELECT 1
             FROM trace_derived_records
             WHERE tenant_id = ?1
               AND submission_id = ?2
               AND derived_id = ?3
             LIMIT 1",
            libsql::params![tenant_id, submission_id.to_string(), derived_id.to_string()],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
    let exists = rows
        .next()
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?
        .is_some();
    if exists {
        return Ok(());
    }

    Err(DatabaseError::Constraint(format!(
        "trace export manifest derived_id {derived_id} does not belong to tenant {tenant_id} submission {submission_id}"
    )))
}

async fn ensure_libsql_vector_entry_belongs_to_submission(
    conn: &libsql::Connection,
    tenant_id: &str,
    submission_id: Uuid,
    vector_entry_id: Uuid,
) -> Result<(), DatabaseError> {
    let mut rows = conn
        .query(
            "SELECT 1
             FROM trace_vector_entries
             WHERE tenant_id = ?1
               AND submission_id = ?2
               AND vector_entry_id = ?3
             LIMIT 1",
            libsql::params![
                tenant_id,
                submission_id.to_string(),
                vector_entry_id.to_string()
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
    let exists = rows
        .next()
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?
        .is_some();
    if exists {
        return Ok(());
    }

    Err(DatabaseError::Constraint(format!(
        "trace export manifest vector_entry_id {vector_entry_id} does not belong to tenant {tenant_id} submission {submission_id}"
    )))
}

fn opt_f32(value: Option<f32>) -> libsql::Value {
    match value {
        Some(value) => libsql::Value::Real(f64::from(value)),
        None => libsql::Value::Null,
    }
}

fn opt_i32(value: Option<i32>) -> libsql::Value {
    match value {
        Some(value) => libsql::Value::Integer(i64::from(value)),
        None => libsql::Value::Null,
    }
}

fn opt_u32(value: Option<u32>) -> libsql::Value {
    match value {
        Some(value) => libsql::Value::Integer(i64::from(value)),
        None => libsql::Value::Null,
    }
}

fn opt_uuid(value: Option<Uuid>) -> libsql::Value {
    match value {
        Some(value) => libsql::Value::Text(value.to_string()),
        None => libsql::Value::Null,
    }
}

fn opt_string(value: Option<String>) -> libsql::Value {
    match value {
        Some(value) => libsql::Value::Text(value),
        None => libsql::Value::Null,
    }
}

fn get_opt_f32(row: &libsql::Row, idx: i32) -> Option<f32> {
    row.get::<f64>(idx)
        .ok()
        .map(|value| value as f32)
        .or_else(|| row.get::<i64>(idx).ok().map(|value| value as f32))
}

fn get_opt_i32(row: &libsql::Row, idx: i32) -> Option<i32> {
    row.get::<i64>(idx)
        .ok()
        .and_then(|value| i32::try_from(value).ok())
}

fn get_i32(row: &libsql::Row, idx: i32, column: &str) -> Result<i32, DatabaseError> {
    let value = row.get::<i64>(idx).map_err(|e| {
        DatabaseError::Serialization(format!("trace {column} column read failed: {e}"))
    })?;
    i32::try_from(value).map_err(|e| {
        DatabaseError::Serialization(format!("invalid trace {column} column value: {e}"))
    })
}

fn get_u32(row: &libsql::Row, idx: i32, column: &str) -> Result<u32, DatabaseError> {
    let value = row.get::<i64>(idx).map_err(|e| {
        DatabaseError::Serialization(format!("trace {column} column read failed: {e}"))
    })?;
    u32::try_from(value).map_err(|e| {
        DatabaseError::Serialization(format!("invalid trace {column} column value: {e}"))
    })
}

fn json_string<T: serde::Serialize>(value: &T) -> Result<String, DatabaseError> {
    serde_json::to_string(value)
        .map_err(|e| DatabaseError::Serialization(format!("trace corpus JSON encode failed: {e}")))
}

fn json_array_strings(raw: &str, column: &str) -> Result<Vec<String>, DatabaseError> {
    let value: serde_json::Value = serde_json::from_str(raw).map_err(|e| {
        DatabaseError::Serialization(format!("trace {column} column JSON decode failed: {e}"))
    })?;
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

fn json_array_uuids(raw: &str, column: &str) -> Result<Vec<Uuid>, DatabaseError> {
    json_array_strings(raw, column)?
        .into_iter()
        .map(|id| parse_uuid(&id, column))
        .collect()
}

fn json_u32_map(raw: &str, column: &str) -> Result<BTreeMap<String, u32>, DatabaseError> {
    serde_json::from_str(raw).map_err(|e| {
        DatabaseError::Serialization(format!("trace {column} column JSON decode failed: {e}"))
    })
}

fn json_string_map(raw: &str, column: &str) -> Result<BTreeMap<String, String>, DatabaseError> {
    serde_json::from_str(raw).map_err(|e| {
        DatabaseError::Serialization(format!("trace {column} column JSON decode failed: {e}"))
    })
}

fn row_to_submission(row: &libsql::Row) -> Result<TraceSubmissionRecord, DatabaseError> {
    let status = enum_from_storage::<TraceCorpusStatus>(&get_text(row, 3), "TraceCorpusStatus")?;
    Ok(TraceSubmissionRecord {
        tenant_id: get_text(row, 0),
        submission_id: parse_uuid(&get_text(row, 1), "trace_submissions.submission_id")?,
        trace_id: parse_uuid(&get_text(row, 2), "trace_submissions.trace_id")?,
        status,
        auth_principal_ref: get_text(row, 4),
        contributor_pseudonym: get_opt_text(row, 5),
        submitted_tenant_scope_ref: get_opt_text(row, 6),
        schema_version: get_text(row, 7),
        consent_policy_version: get_text(row, 8),
        consent_scopes: json_array_strings(&get_text(row, 9), "consent_scopes")?,
        allowed_uses: json_array_strings(&get_text(row, 10), "allowed_uses")?,
        retention_policy_id: get_text(row, 11),
        privacy_risk: get_text(row, 12),
        redaction_pipeline_version: get_text(row, 13),
        redaction_hash: get_text(row, 14),
        redaction_counts: json_u32_map(&get_text(row, 15), "redaction_counts")?,
        canonical_summary_hash: get_opt_text(row, 16),
        submission_score: get_opt_f32(row, 17),
        credit_points_pending: get_opt_f32(row, 18),
        credit_points_final: get_opt_f32(row, 19),
        received_at: get_ts(row, 20),
        updated_at: get_ts(row, 21),
        reviewed_at: get_opt_ts(row, 22),
        review_assigned_to_principal_ref: get_opt_text(row, 23),
        review_assigned_at: get_opt_ts(row, 24),
        review_lease_expires_at: get_opt_ts(row, 25),
        review_due_at: get_opt_ts(row, 26),
        revoked_at: get_opt_ts(row, 27),
        expires_at: get_opt_ts(row, 28),
        purged_at: get_opt_ts(row, 29),
    })
}

fn row_to_tenant_policy(row: &libsql::Row) -> Result<TraceTenantPolicyRecord, DatabaseError> {
    Ok(TraceTenantPolicyRecord {
        tenant_id: get_text(row, 0),
        policy_version: get_text(row, 1),
        allowed_consent_scopes: json_array_strings(&get_text(row, 2), "allowed_consent_scopes")?,
        allowed_uses: json_array_strings(&get_text(row, 3), "allowed_uses")?,
        updated_by_principal_ref: get_text(row, 4),
        created_at: get_ts(row, 5),
        updated_at: get_ts(row, 6),
    })
}

fn row_to_object_ref(row: &libsql::Row) -> Result<TraceObjectRefRecord, DatabaseError> {
    Ok(TraceObjectRefRecord {
        tenant_id: get_text(row, 0),
        submission_id: parse_uuid(&get_text(row, 1), "trace_object_refs.submission_id")?,
        object_ref_id: parse_uuid(&get_text(row, 2), "trace_object_refs.object_ref_id")?,
        artifact_kind: enum_from_storage::<TraceObjectArtifactKind>(
            &get_text(row, 3),
            "TraceObjectArtifactKind",
        )?,
        object_store: get_text(row, 4),
        object_key: get_text(row, 5),
        content_sha256: get_text(row, 6),
        encryption_key_ref: get_text(row, 7),
        size_bytes: row.get::<i64>(8).map_err(|e| {
            DatabaseError::Serialization(format!("trace size_bytes column read failed: {e}"))
        })?,
        compression: get_opt_text(row, 9),
        created_by_job_id: get_opt_text(row, 10)
            .map(|id| parse_uuid(&id, "trace_object_refs.created_by_job_id"))
            .transpose()?,
        invalidated_at: get_opt_ts(row, 11),
        deleted_at: get_opt_ts(row, 12),
        updated_at: get_ts(row, 13),
        created_at: get_ts(row, 14),
    })
}

fn row_to_credit_event(row: &libsql::Row) -> Result<TraceCreditEventRecord, DatabaseError> {
    Ok(TraceCreditEventRecord {
        tenant_id: get_text(row, 0),
        credit_event_id: parse_uuid(&get_text(row, 1), "trace_credit_ledger.credit_event_id")?,
        submission_id: parse_uuid(&get_text(row, 2), "trace_credit_ledger.submission_id")?,
        trace_id: parse_uuid(&get_text(row, 3), "trace_credit_ledger.trace_id")?,
        credit_account_ref: get_text(row, 4),
        event_type: enum_from_storage(&get_text(row, 5), "TraceCreditEventType")?,
        points_delta: get_text(row, 6),
        reason: get_text(row, 7),
        external_ref: get_opt_text(row, 8),
        actor_principal_ref: get_text(row, 9),
        actor_role: get_text(row, 10),
        settlement_state: enum_from_storage::<TraceCreditSettlementState>(
            &get_text(row, 11),
            "TraceCreditSettlementState",
        )?,
        occurred_at: get_ts(row, 12),
    })
}

fn row_to_derived_record(row: &libsql::Row) -> Result<TraceDerivedRecord, DatabaseError> {
    let tenant_id = get_text(row, 0);
    let submission_id = parse_uuid(&get_text(row, 2), "trace_derived_records.submission_id")?;
    let input_object_ref_id = get_opt_text(row, 7)
        .map(|id| parse_uuid(&id, "trace_derived_records.input_object_ref_id"))
        .transpose()?;
    let output_object_ref_id = get_opt_text(row, 9)
        .map(|id| parse_uuid(&id, "trace_derived_records.output_object_ref_id"))
        .transpose()?;
    Ok(TraceDerivedRecord {
        tenant_id: tenant_id.clone(),
        derived_id: parse_uuid(&get_text(row, 1), "trace_derived_records.derived_id")?,
        submission_id,
        trace_id: parse_uuid(&get_text(row, 3), "trace_derived_records.trace_id")?,
        status: enum_from_storage::<TraceDerivedStatus>(&get_text(row, 4), "TraceDerivedStatus")?,
        worker_kind: enum_from_storage::<TraceWorkerKind>(&get_text(row, 5), "TraceWorkerKind")?,
        worker_version: get_text(row, 6),
        input_object_ref: input_object_ref_id.map(|object_ref_id| TenantScopedTraceObjectRef {
            tenant_id: tenant_id.clone(),
            submission_id,
            object_ref_id,
        }),
        input_hash: get_text(row, 8),
        output_object_ref: output_object_ref_id.map(|object_ref_id| TenantScopedTraceObjectRef {
            tenant_id: tenant_id.clone(),
            submission_id,
            object_ref_id,
        }),
        canonical_summary: get_opt_text(row, 10),
        canonical_summary_hash: get_opt_text(row, 11),
        summary_model: get_text(row, 12),
        task_success: get_opt_text(row, 13),
        privacy_risk: get_opt_text(row, 14),
        event_count: get_opt_i32(row, 15),
        tool_sequence: json_array_strings(&get_text(row, 16), "tool_sequence")?,
        tool_categories: json_array_strings(&get_text(row, 17), "tool_categories")?,
        coverage_tags: json_array_strings(&get_text(row, 18), "coverage_tags")?,
        duplicate_score: get_opt_f32(row, 19),
        novelty_score: get_opt_f32(row, 20),
        cluster_id: get_opt_text(row, 21),
        created_at: get_ts(row, 22),
        updated_at: get_ts(row, 23),
    })
}

fn row_to_vector_entry(row: &libsql::Row) -> Result<TraceVectorEntryRecord, DatabaseError> {
    Ok(TraceVectorEntryRecord {
        tenant_id: get_text(row, 0),
        submission_id: parse_uuid(&get_text(row, 1), "trace_vector_entries.submission_id")?,
        derived_id: parse_uuid(&get_text(row, 2), "trace_vector_entries.derived_id")?,
        vector_entry_id: parse_uuid(&get_text(row, 3), "trace_vector_entries.vector_entry_id")?,
        vector_store: get_text(row, 4),
        embedding_model: get_text(row, 5),
        embedding_dimension: get_i32(row, 6, "embedding_dimension")?,
        embedding_version: get_text(row, 7),
        source_projection: enum_from_storage::<TraceVectorEntrySourceProjection>(
            &get_text(row, 8),
            "TraceVectorEntrySourceProjection",
        )?,
        source_hash: get_text(row, 9),
        status: enum_from_storage::<TraceVectorEntryStatus>(
            &get_text(row, 10),
            "TraceVectorEntryStatus",
        )?,
        nearest_trace_ids: json_array_strings(&get_text(row, 11), "nearest_trace_ids")?,
        cluster_id: get_opt_text(row, 12),
        duplicate_score: get_opt_f32(row, 13),
        novelty_score: get_opt_f32(row, 14),
        indexed_at: get_opt_ts(row, 15),
        invalidated_at: get_opt_ts(row, 16),
        deleted_at: get_opt_ts(row, 17),
        created_at: get_ts(row, 18),
        updated_at: get_ts(row, 19),
    })
}

fn row_to_export_manifest(row: &libsql::Row) -> Result<TraceExportManifestRecord, DatabaseError> {
    let audit_event_id = get_opt_text(row, 4)
        .map(|id| parse_uuid(&id, "trace_export_manifests.audit_event_id"))
        .transpose()?;
    Ok(TraceExportManifestRecord {
        tenant_id: get_text(row, 0),
        export_manifest_id: parse_uuid(
            &get_text(row, 1),
            "trace_export_manifests.export_manifest_id",
        )?,
        artifact_kind: enum_from_storage::<TraceObjectArtifactKind>(
            &get_text(row, 2),
            "TraceObjectArtifactKind",
        )?,
        purpose_code: get_opt_text(row, 3),
        audit_event_id,
        source_submission_ids: json_array_uuids(
            &get_text(row, 5),
            "trace_export_manifests.source_submission_ids",
        )?,
        source_submission_ids_hash: get_text(row, 6),
        item_count: get_u32(row, 7, "trace_export_manifests.item_count")?,
        generated_at: get_ts(row, 8),
        invalidated_at: get_opt_ts(row, 9),
        deleted_at: get_opt_ts(row, 10),
        created_at: get_ts(row, 11),
        updated_at: get_ts(row, 12),
    })
}

fn row_to_export_manifest_item(
    row: &libsql::Row,
) -> Result<TraceExportManifestItemRecord, DatabaseError> {
    Ok(TraceExportManifestItemRecord {
        tenant_id: get_text(row, 0),
        export_manifest_id: parse_uuid(
            &get_text(row, 1),
            "trace_export_manifest_items.export_manifest_id",
        )?,
        submission_id: parse_uuid(
            &get_text(row, 2),
            "trace_export_manifest_items.submission_id",
        )?,
        trace_id: parse_uuid(&get_text(row, 3), "trace_export_manifest_items.trace_id")?,
        derived_id: get_opt_text(row, 4)
            .map(|id| parse_uuid(&id, "trace_export_manifest_items.derived_id"))
            .transpose()?,
        object_ref_id: get_opt_text(row, 5)
            .map(|id| parse_uuid(&id, "trace_export_manifest_items.object_ref_id"))
            .transpose()?,
        vector_entry_id: get_opt_text(row, 6)
            .map(|id| parse_uuid(&id, "trace_export_manifest_items.vector_entry_id"))
            .transpose()?,
        source_status_at_export: enum_from_storage::<TraceCorpusStatus>(
            &get_text(row, 7),
            "TraceCorpusStatus",
        )?,
        source_hash_at_export: get_text(row, 8),
        source_invalidated_at: get_opt_ts(row, 9),
        source_invalidation_reason: get_opt_text(row, 10)
            .map(|reason| {
                enum_from_storage::<TraceExportManifestItemInvalidationReason>(
                    &reason,
                    "TraceExportManifestItemInvalidationReason",
                )
            })
            .transpose()?,
        created_at: get_ts(row, 11),
        updated_at: get_ts(row, 12),
    })
}

fn row_to_audit_event(row: &libsql::Row) -> Result<TraceAuditEventRecord, DatabaseError> {
    let audit_sequence = row.get::<i64>(1).map_err(|e| {
        DatabaseError::Serialization(format!(
            "trace_audit_events.audit_sequence column read failed: {e}"
        ))
    })?;
    let submission_id = get_opt_text(row, 8)
        .map(|id| parse_uuid(&id, "trace_audit_events.submission_id"))
        .transpose()?;
    let object_ref_id = get_opt_text(row, 9)
        .map(|id| parse_uuid(&id, "trace_audit_events.object_ref_id"))
        .transpose()?;
    let export_manifest_id = get_opt_text(row, 10)
        .map(|id| parse_uuid(&id, "trace_audit_events.export_manifest_id"))
        .transpose()?;
    let metadata = serde_json::from_str(&get_text(row, 15)).map_err(|e| {
        DatabaseError::Serialization(format!("trace audit metadata JSON decode failed: {e}"))
    })?;
    Ok(TraceAuditEventRecord {
        tenant_id: get_text(row, 0),
        audit_sequence,
        audit_event_id: parse_uuid(&get_text(row, 2), "trace_audit_events.audit_event_id")?,
        actor_principal_ref: get_text(row, 3),
        actor_role: get_text(row, 4),
        action: enum_from_storage(&get_text(row, 5), "TraceAuditAction")?,
        reason: get_opt_text(row, 6),
        request_id: get_opt_text(row, 7),
        submission_id,
        object_ref_id,
        export_manifest_id,
        decision_inputs_hash: get_opt_text(row, 11),
        previous_event_hash: get_opt_text(row, 12),
        event_hash: get_opt_text(row, 13),
        canonical_event_json: get_opt_text(row, 14),
        metadata,
        occurred_at: get_ts(row, 16),
    })
}

fn row_to_tombstone(row: &libsql::Row) -> Result<TraceTombstoneRecord, DatabaseError> {
    Ok(TraceTombstoneRecord {
        tenant_id: get_text(row, 0),
        tombstone_id: parse_uuid(&get_text(row, 1), "trace_tombstones.tombstone_id")?,
        submission_id: parse_uuid(&get_text(row, 2), "trace_tombstones.submission_id")?,
        trace_id: get_opt_text(row, 3)
            .map(|id| parse_uuid(&id, "trace_tombstones.trace_id"))
            .transpose()?,
        redaction_hash: get_opt_text(row, 4),
        canonical_summary_hash: get_opt_text(row, 5),
        reason: get_text(row, 6),
        effective_at: get_ts(row, 7),
        retain_until: get_opt_ts(row, 8),
        created_by_principal_ref: get_text(row, 9),
        created_at: get_ts(row, 10),
    })
}

fn row_to_retention_job(row: &libsql::Row) -> Result<TraceRetentionJobRecord, DatabaseError> {
    Ok(TraceRetentionJobRecord {
        tenant_id: get_text(row, 0),
        retention_job_id: parse_uuid(&get_text(row, 1), "trace_retention_jobs.retention_job_id")?,
        purpose: get_text(row, 2),
        dry_run: row.get::<i64>(3).map_err(|e| {
            DatabaseError::Serialization(format!(
                "trace_retention_jobs.dry_run column read failed: {e}"
            ))
        })? != 0,
        status: enum_from_storage::<TraceRetentionJobStatus>(
            &get_text(row, 4),
            "TraceRetentionJobStatus",
        )?,
        requested_by_principal_ref: get_text(row, 5),
        requested_by_role: get_text(row, 6),
        purge_expired_before: get_opt_ts(row, 7),
        prune_export_cache: row.get::<i64>(8).map_err(|e| {
            DatabaseError::Serialization(format!(
                "trace_retention_jobs.prune_export_cache column read failed: {e}"
            ))
        })? != 0,
        max_export_age_hours: row.get::<i64>(9).ok(),
        audit_event_id: get_opt_text(row, 10)
            .map(|id| parse_uuid(&id, "trace_retention_jobs.audit_event_id"))
            .transpose()?,
        action_counts: json_u32_map(&get_text(row, 11), "trace_retention_jobs.action_counts")?,
        selected_revoked_count: get_u32(row, 12, "trace_retention_jobs.selected_revoked_count")?,
        selected_expired_count: get_u32(row, 13, "trace_retention_jobs.selected_expired_count")?,
        started_at: get_opt_ts(row, 14),
        completed_at: get_opt_ts(row, 15),
        created_at: get_ts(row, 16),
        updated_at: get_ts(row, 17),
    })
}

fn row_to_retention_job_item(
    row: &libsql::Row,
) -> Result<TraceRetentionJobItemRecord, DatabaseError> {
    Ok(TraceRetentionJobItemRecord {
        tenant_id: get_text(row, 0),
        retention_job_id: parse_uuid(
            &get_text(row, 1),
            "trace_retention_job_items.retention_job_id",
        )?,
        submission_id: parse_uuid(&get_text(row, 2), "trace_retention_job_items.submission_id")?,
        action: enum_from_storage::<TraceRetentionJobItemAction>(
            &get_text(row, 3),
            "TraceRetentionJobItemAction",
        )?,
        status: enum_from_storage::<TraceRetentionJobItemStatus>(
            &get_text(row, 4),
            "TraceRetentionJobItemStatus",
        )?,
        reason: get_text(row, 5),
        action_counts: json_u32_map(&get_text(row, 6), "trace_retention_job_items.action_counts")?,
        verified_at: get_opt_ts(row, 7),
        created_at: get_ts(row, 8),
        updated_at: get_ts(row, 9),
    })
}

fn row_to_export_access_grant(
    row: &libsql::Row,
) -> Result<TraceExportAccessGrantRecord, DatabaseError> {
    Ok(TraceExportAccessGrantRecord {
        tenant_id: get_text(row, 0),
        export_job_id: parse_uuid(
            &get_text(row, 1),
            "trace_export_access_grants.export_job_id",
        )?,
        grant_id: parse_uuid(&get_text(row, 2), "trace_export_access_grants.grant_id")?,
        caller_principal_ref: get_text(row, 3),
        requested_dataset_kind: get_text(row, 4),
        purpose: get_text(row, 5),
        max_item_cap: get_opt_i32(row, 6).and_then(|value| u32::try_from(value).ok()),
        status: enum_from_storage::<TraceExportAccessGrantStatus>(
            &get_text(row, 7),
            "TraceExportAccessGrantStatus",
        )?,
        requested_at: get_ts(row, 8),
        expires_at: get_ts(row, 9),
        metadata: json_string_map(
            &get_text(row, 10),
            "trace_export_access_grants.metadata_json",
        )?,
        created_at: get_ts(row, 11),
        updated_at: get_ts(row, 12),
    })
}

fn row_to_export_job(row: &libsql::Row) -> Result<TraceExportJobRecord, DatabaseError> {
    Ok(TraceExportJobRecord {
        tenant_id: get_text(row, 0),
        export_job_id: parse_uuid(&get_text(row, 1), "trace_export_jobs.export_job_id")?,
        grant_id: parse_uuid(&get_text(row, 2), "trace_export_jobs.grant_id")?,
        caller_principal_ref: get_text(row, 3),
        requested_dataset_kind: get_text(row, 4),
        purpose: get_text(row, 5),
        max_item_cap: get_opt_i32(row, 6).and_then(|value| u32::try_from(value).ok()),
        status: enum_from_storage::<TraceExportJobStatus>(
            &get_text(row, 7),
            "TraceExportJobStatus",
        )?,
        requested_at: get_ts(row, 8),
        started_at: get_opt_ts(row, 9),
        finished_at: get_opt_ts(row, 10),
        expires_at: get_ts(row, 11),
        result_manifest_id: get_opt_text(row, 12)
            .map(|id| parse_uuid(&id, "trace_export_jobs.result_manifest_id"))
            .transpose()?,
        item_count: get_opt_i32(row, 13).and_then(|value| u32::try_from(value).ok()),
        last_error: get_opt_text(row, 14),
        metadata: json_string_map(&get_text(row, 15), "trace_export_jobs.metadata_json")?,
        created_at: get_ts(row, 16),
        updated_at: get_ts(row, 17),
    })
}

fn row_to_revocation_propagation_item(
    row: &libsql::Row,
) -> Result<TraceRevocationPropagationItemRecord, DatabaseError> {
    Ok(TraceRevocationPropagationItemRecord {
        tenant_id: get_text(row, 0),
        propagation_item_id: parse_uuid(
            &get_text(row, 1),
            "trace_revocation_propagation_items.propagation_item_id",
        )?,
        source_submission_id: parse_uuid(
            &get_text(row, 2),
            "trace_revocation_propagation_items.source_submission_id",
        )?,
        trace_id: parse_uuid(
            &get_text(row, 3),
            "trace_revocation_propagation_items.trace_id",
        )?,
        target_kind: enum_from_storage::<TraceRevocationPropagationTargetKind>(
            &get_text(row, 4),
            "TraceRevocationPropagationTargetKind",
        )?,
        target: serde_json::from_str::<TraceRevocationPropagationTarget>(&get_text(row, 5))
            .map_err(|e| {
                DatabaseError::Serialization(format!(
                    "trace revocation propagation target decode failed: {e}"
                ))
            })?,
        action: enum_from_storage::<TraceRevocationPropagationAction>(
            &get_text(row, 6),
            "TraceRevocationPropagationAction",
        )?,
        status: enum_from_storage::<TraceRevocationPropagationItemStatus>(
            &get_text(row, 7),
            "TraceRevocationPropagationItemStatus",
        )?,
        idempotency_key: get_text(row, 8),
        reason: get_text(row, 9),
        attempt_count: get_u32(row, 10, "trace_revocation_propagation_items.attempt_count")?,
        last_error: get_opt_text(row, 11),
        next_attempt_at: get_opt_ts(row, 12),
        completed_at: get_opt_ts(row, 13),
        evidence_hash: get_opt_text(row, 14),
        metadata: serde_json::from_str(&get_text(row, 15)).map_err(|e| {
            DatabaseError::Serialization(format!(
                "trace revocation propagation metadata decode failed: {e}"
            ))
        })?,
        created_at: get_ts(row, 16),
        updated_at: get_ts(row, 17),
    })
}

impl LibSqlBackend {
    async fn ensure_trace_tenant(&self, tenant_id: &str) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        conn.execute(
            "INSERT INTO trace_tenants (tenant_id) VALUES (?1)
             ON CONFLICT (tenant_id) DO UPDATE SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
            libsql::params![tenant_id],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }
}

#[async_trait]
impl TraceCorpusStore for LibSqlBackend {
    async fn upsert_trace_submission(
        &self,
        submission: TraceSubmissionWrite,
    ) -> Result<TraceSubmissionRecord, DatabaseError> {
        self.ensure_trace_tenant(&submission.tenant_id).await?;
        let conn = self.connect().await?;
        let status = enum_to_storage(submission.status)?;
        let consent_scopes = json_string(&submission.consent_scopes)?;
        let allowed_uses = json_string(&submission.allowed_uses)?;
        let redaction_counts = json_string(&submission.redaction_counts)?;

        conn.execute(
            "INSERT INTO trace_submissions (
                tenant_id, submission_id, trace_id, auth_principal_ref, contributor_pseudonym,
                submitted_tenant_scope_ref, schema_version, consent_policy_version,
                consent_scopes, allowed_uses, retention_policy_id, status, privacy_risk,
                redaction_pipeline_version, redaction_hash, redaction_counts, canonical_summary_hash,
                submission_score, credit_points_pending, credit_points_final, expires_at
             ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15,
                ?16, ?17, ?18, ?19, ?20, ?21
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
                updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
            libsql::params![
                submission.tenant_id.as_str(),
                submission.submission_id.to_string(),
                submission.trace_id.to_string(),
                submission.auth_principal_ref.as_str(),
                opt_string(submission.contributor_pseudonym.clone()),
                opt_string(submission.submitted_tenant_scope_ref.clone()),
                submission.schema_version.as_str(),
                submission.consent_policy_version.as_str(),
                consent_scopes,
                allowed_uses,
                submission.retention_policy_id.as_str(),
                status,
                submission.privacy_risk.as_str(),
                submission.redaction_pipeline_version.as_str(),
                submission.redaction_hash.as_str(),
                redaction_counts,
                opt_string(submission.canonical_summary_hash.clone()),
                opt_f32(submission.submission_score),
                opt_f32(submission.credit_points_pending),
                opt_f32(submission.credit_points_final),
                fmt_opt_ts(&submission.expires_at),
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;

        self.get_trace_submission(&submission.tenant_id, submission.submission_id)
            .await?
            .ok_or_else(|| DatabaseError::NotFound {
                entity: "trace_submission".to_string(),
                id: submission.submission_id.to_string(),
            })
    }

    async fn get_trace_submission(
        &self,
        tenant_id: &str,
        submission_id: Uuid,
    ) -> Result<Option<TraceSubmissionRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                &format!(
                    "SELECT {TRACE_SUBMISSION_COLUMNS}
                     FROM trace_submissions
                     WHERE tenant_id = ?1 AND submission_id = ?2"
                ),
                libsql::params![tenant_id, submission_id.to_string()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        match rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            Some(row) => Ok(Some(row_to_submission(&row)?)),
            None => Ok(None),
        }
    }

    async fn list_trace_submissions(
        &self,
        tenant_id: &str,
    ) -> Result<Vec<TraceSubmissionRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                &format!(
                    "SELECT {TRACE_SUBMISSION_COLUMNS}
                     FROM trace_submissions
                     WHERE tenant_id = ?1
                     ORDER BY received_at ASC"
                ),
                libsql::params![tenant_id],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        let mut submissions = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            submissions.push(row_to_submission(&row)?);
        }
        Ok(submissions)
    }

    async fn upsert_trace_tenant_policy(
        &self,
        policy: TraceTenantPolicyWrite,
    ) -> Result<TraceTenantPolicyRecord, DatabaseError> {
        self.ensure_trace_tenant(&policy.tenant_id).await?;
        let conn = self.connect().await?;
        let allowed_consent_scopes = json_string(&policy.allowed_consent_scopes)?;
        let allowed_uses = json_string(&policy.allowed_uses)?;
        conn.execute(
            "INSERT INTO trace_tenant_policies (
                tenant_id, policy_version, allowed_consent_scopes, allowed_uses,
                updated_by_principal_ref
             ) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT (tenant_id) DO UPDATE SET
                policy_version = excluded.policy_version,
                allowed_consent_scopes = excluded.allowed_consent_scopes,
                allowed_uses = excluded.allowed_uses,
                updated_by_principal_ref = excluded.updated_by_principal_ref,
                updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
            libsql::params![
                policy.tenant_id.as_str(),
                policy.policy_version.as_str(),
                allowed_consent_scopes,
                allowed_uses,
                policy.updated_by_principal_ref.as_str(),
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;

        self.get_trace_tenant_policy(&policy.tenant_id)
            .await?
            .ok_or_else(|| DatabaseError::NotFound {
                entity: "trace_tenant_policy".to_string(),
                id: policy.tenant_id,
            })
    }

    async fn get_trace_tenant_policy(
        &self,
        tenant_id: &str,
    ) -> Result<Option<TraceTenantPolicyRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                &format!(
                    "SELECT {TRACE_TENANT_POLICY_COLUMNS}
                     FROM trace_tenant_policies
                     WHERE tenant_id = ?1"
                ),
                libsql::params![tenant_id],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        match rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            Some(row) => Ok(Some(row_to_tenant_policy(&row)?)),
            None => Ok(None),
        }
    }

    async fn list_trace_credit_events(
        &self,
        tenant_id: &str,
    ) -> Result<Vec<TraceCreditEventRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT
                    tenant_id, credit_event_id, submission_id, trace_id, credit_account_ref,
                    event_type, points_delta, reason, external_ref, actor_principal_ref,
                    actor_role, settlement_state, occurred_at
                 FROM trace_credit_ledger
                 WHERE tenant_id = ?1
                 ORDER BY occurred_at ASC",
                libsql::params![tenant_id],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        let mut events = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            events.push(row_to_credit_event(&row)?);
        }
        Ok(events)
    }

    async fn update_trace_submission_status(
        &self,
        tenant_id: &str,
        submission_id: Uuid,
        status: TraceCorpusStatus,
        actor_principal_ref: &str,
        reason: Option<&str>,
    ) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        let status_value = enum_to_storage(status)?;
        let updated = conn
            .execute(
                "UPDATE trace_submissions
             SET status = ?3,
                 updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                 reviewed_at = CASE
                     WHEN ?3 IN ('accepted', 'quarantined', 'rejected')
                     THEN strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                     ELSE reviewed_at
                 END,
                 review_assigned_to_principal_ref = CASE
                     WHEN ?3 IN ('accepted', 'rejected', 'revoked', 'expired', 'purged')
                     THEN NULL
                     ELSE review_assigned_to_principal_ref
                 END,
                 review_assigned_at = CASE
                     WHEN ?3 IN ('accepted', 'rejected', 'revoked', 'expired', 'purged')
                     THEN NULL
                     ELSE review_assigned_at
                 END,
                 review_lease_expires_at = CASE
                     WHEN ?3 IN ('accepted', 'rejected', 'revoked', 'expired', 'purged')
                     THEN NULL
                     ELSE review_lease_expires_at
                 END,
                 review_due_at = CASE
                     WHEN ?3 IN ('accepted', 'rejected', 'revoked', 'expired', 'purged')
                     THEN NULL
                     ELSE review_due_at
                 END,
                 revoked_at = CASE
                     WHEN ?3 = 'revoked' THEN strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                     ELSE revoked_at
                 END,
                 purged_at = CASE
                     WHEN ?3 = 'purged' THEN strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                     ELSE purged_at
                 END,
                 credit_points_pending = CASE
                     WHEN ?3 IN ('revoked', 'expired', 'purged') THEN 0
                     ELSE credit_points_pending
                 END,
                 credit_points_final = CASE
                     WHEN ?3 IN ('revoked', 'expired', 'purged') THEN 0
                     ELSE credit_points_final
                 END
             WHERE tenant_id = ?1 AND submission_id = ?2",
                libsql::params![tenant_id, submission_id.to_string(), status_value],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        if updated == 0 {
            return Err(DatabaseError::NotFound {
                entity: "trace_submission".to_string(),
                id: submission_id.to_string(),
            });
        }

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
                decision: enum_to_storage(status)?,
                resulting_status: status,
                reason_code: reason.map(str::to_string),
            },
        })
        .await
    }

    async fn claim_trace_review_lease(
        &self,
        tenant_id: &str,
        submission_id: Uuid,
        actor_principal_ref: &str,
        lease_expires_at: DateTime<Utc>,
        review_due_at: Option<DateTime<Utc>>,
        now: DateTime<Utc>,
    ) -> Result<Option<TraceSubmissionRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let updated = conn
            .execute(
                "UPDATE trace_submissions
                 SET review_assigned_to_principal_ref = ?3,
                     review_assigned_at = ?6,
                     review_lease_expires_at = ?4,
                     review_due_at = ?5,
                     updated_at = ?6
                 WHERE tenant_id = ?1
                   AND submission_id = ?2
                   AND status = 'quarantined'
                   AND (
                        review_lease_expires_at IS NULL
                        OR review_lease_expires_at <= ?6
                        OR review_assigned_to_principal_ref = ?3
                   )",
                libsql::params![
                    tenant_id,
                    submission_id.to_string(),
                    actor_principal_ref,
                    fmt_ts(&lease_expires_at),
                    fmt_opt_ts(&review_due_at),
                    fmt_ts(&now),
                ],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        if updated == 0 {
            return Ok(None);
        }
        self.get_trace_submission(tenant_id, submission_id).await
    }

    async fn release_trace_review_lease(
        &self,
        tenant_id: &str,
        submission_id: Uuid,
        actor_principal_ref: &str,
    ) -> Result<Option<TraceSubmissionRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let updated = conn
            .execute(
                "UPDATE trace_submissions
                 SET review_assigned_to_principal_ref = NULL,
                     review_assigned_at = NULL,
                     review_lease_expires_at = NULL,
                     review_due_at = NULL,
                     updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                 WHERE tenant_id = ?1
                   AND submission_id = ?2
                   AND status = 'quarantined'
                   AND review_assigned_to_principal_ref = ?3",
                libsql::params![tenant_id, submission_id.to_string(), actor_principal_ref],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        if updated == 0 {
            return Ok(None);
        }
        self.get_trace_submission(tenant_id, submission_id).await
    }

    async fn append_trace_object_ref(
        &self,
        object_ref: TraceObjectRefWrite,
    ) -> Result<(), DatabaseError> {
        self.ensure_trace_tenant(&object_ref.tenant_id).await?;
        let conn = self.connect().await?;
        conn.execute(
            "INSERT INTO trace_object_refs (
                tenant_id, submission_id, object_ref_id, artifact_kind, object_store, object_key,
                content_sha256, encryption_key_ref, size_bytes, compression, created_by_job_id
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
             ON CONFLICT (tenant_id, submission_id, object_ref_id) DO UPDATE SET
                artifact_kind = excluded.artifact_kind,
                object_store = excluded.object_store,
                object_key = excluded.object_key,
                content_sha256 = excluded.content_sha256,
                encryption_key_ref = excluded.encryption_key_ref,
                size_bytes = excluded.size_bytes,
                compression = excluded.compression,
                created_by_job_id = excluded.created_by_job_id,
                updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
            libsql::params![
                object_ref.tenant_id.as_str(),
                object_ref.submission_id.to_string(),
                object_ref.object_ref_id.to_string(),
                enum_to_storage(object_ref.artifact_kind)?,
                object_ref.object_store.as_str(),
                object_ref.object_key.as_str(),
                object_ref.content_sha256.as_str(),
                object_ref.encryption_key_ref.as_str(),
                object_ref.size_bytes,
                opt_string(object_ref.compression),
                opt_uuid(object_ref.created_by_job_id),
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn list_trace_object_refs(
        &self,
        tenant_id: &str,
        submission_id: Uuid,
    ) -> Result<Vec<TraceObjectRefRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                &format!(
                    "SELECT {TRACE_OBJECT_REF_COLUMNS}
                     FROM trace_object_refs
                     WHERE tenant_id = ?1 AND submission_id = ?2
                     ORDER BY created_at ASC"
                ),
                libsql::params![tenant_id, submission_id.to_string()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        let mut object_refs = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            object_refs.push(row_to_object_ref(&row)?);
        }
        Ok(object_refs)
    }

    async fn get_latest_active_trace_object_ref(
        &self,
        tenant_id: &str,
        submission_id: Uuid,
        artifact_kind: TraceObjectArtifactKind,
    ) -> Result<Option<TraceObjectRefRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let artifact_kind = enum_to_storage(artifact_kind)?;
        let mut rows = conn
            .query(
                &format!(
                    "SELECT {TRACE_OBJECT_REF_COLUMNS}
                     FROM trace_object_refs
                     WHERE tenant_id = ?1
                       AND submission_id = ?2
                       AND artifact_kind = ?3
                       AND invalidated_at IS NULL
                       AND deleted_at IS NULL
                     ORDER BY created_at DESC
                     LIMIT 1"
                ),
                libsql::params![tenant_id, submission_id.to_string(), artifact_kind],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        match rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            Some(row) => Ok(Some(row_to_object_ref(&row)?)),
            None => Ok(None),
        }
    }

    async fn append_trace_derived_record(
        &self,
        derived_record: TraceDerivedRecordWrite,
    ) -> Result<(), DatabaseError> {
        self.ensure_trace_tenant(&derived_record.tenant_id).await?;
        let conn = self.connect().await?;
        if let Some(object_ref) = derived_record.input_object_ref.as_ref() {
            validate_tenant_scoped_trace_object_ref(
                "derived input",
                object_ref,
                &derived_record.tenant_id,
                derived_record.submission_id,
            )?;
            ensure_libsql_object_ref_belongs_to_submission(
                &conn,
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
            ensure_libsql_object_ref_belongs_to_submission(
                &conn,
                &derived_record.tenant_id,
                derived_record.submission_id,
                object_ref.object_ref_id,
                "derived output",
            )
            .await?;
        }
        let input_object_ref_id = derived_record
            .input_object_ref
            .as_ref()
            .map(|object_ref| object_ref.object_ref_id);
        let output_object_ref_id = derived_record
            .output_object_ref
            .as_ref()
            .map(|object_ref| object_ref.object_ref_id);
        let tool_sequence = json_string(&derived_record.tool_sequence)?;
        let tool_categories = json_string(&derived_record.tool_categories)?;
        let coverage_tags = json_string(&derived_record.coverage_tags)?;

        conn.execute(
            "INSERT INTO trace_derived_records (
                tenant_id, derived_id, submission_id, trace_id, status, worker_kind,
                worker_version, input_object_ref_id, input_hash, output_object_ref_id,
                canonical_summary, canonical_summary_hash, summary_model, task_success,
                privacy_risk, event_count, tool_sequence, tool_categories, coverage_tags,
                duplicate_score, novelty_score, cluster_id
             ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15,
                ?16, ?17, ?18, ?19, ?20, ?21, ?22
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
                updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
            libsql::params![
                derived_record.tenant_id.as_str(),
                derived_record.derived_id.to_string(),
                derived_record.submission_id.to_string(),
                derived_record.trace_id.to_string(),
                enum_to_storage(derived_record.status)?,
                enum_to_storage(derived_record.worker_kind)?,
                derived_record.worker_version.as_str(),
                opt_uuid(input_object_ref_id),
                derived_record.input_hash.as_str(),
                opt_uuid(output_object_ref_id),
                opt_string(derived_record.canonical_summary),
                opt_string(derived_record.canonical_summary_hash),
                derived_record.summary_model.as_str(),
                opt_string(derived_record.task_success),
                opt_string(derived_record.privacy_risk),
                opt_i32(derived_record.event_count),
                tool_sequence,
                tool_categories,
                coverage_tags,
                opt_f32(derived_record.duplicate_score),
                opt_f32(derived_record.novelty_score),
                opt_string(derived_record.cluster_id),
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn list_trace_derived_records(
        &self,
        tenant_id: &str,
    ) -> Result<Vec<TraceDerivedRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT
                    tenant_id, derived_id, submission_id, trace_id, status, worker_kind,
                    worker_version, input_object_ref_id, input_hash, output_object_ref_id,
                    canonical_summary, canonical_summary_hash, summary_model, task_success,
                    privacy_risk, event_count, tool_sequence, tool_categories, coverage_tags,
                    duplicate_score, novelty_score, cluster_id, created_at, updated_at
                 FROM trace_derived_records
                 WHERE tenant_id = ?1
                 ORDER BY created_at ASC",
                libsql::params![tenant_id],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        let mut records = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            records.push(row_to_derived_record(&row)?);
        }
        Ok(records)
    }

    async fn upsert_trace_vector_entry(
        &self,
        vector_entry: TraceVectorEntryWrite,
    ) -> Result<TraceVectorEntryRecord, DatabaseError> {
        self.ensure_trace_tenant(&vector_entry.tenant_id).await?;
        let conn = self.connect().await?;
        ensure_libsql_derived_record_belongs_to_submission(
            &conn,
            &vector_entry.tenant_id,
            vector_entry.submission_id,
            vector_entry.derived_id,
        )
        .await?;
        let nearest_trace_ids = json_string(&vector_entry.nearest_trace_ids)?;
        conn.execute(
            "INSERT INTO trace_vector_entries (
                tenant_id, submission_id, derived_id, vector_entry_id, vector_store,
                embedding_model, embedding_dimension, embedding_version, source_projection,
                source_hash, status, nearest_trace_ids, cluster_id, duplicate_score,
                novelty_score, indexed_at, invalidated_at, deleted_at
             ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14,
                ?15, ?16, ?17, ?18
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
                updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
            libsql::params![
                vector_entry.tenant_id.as_str(),
                vector_entry.submission_id.to_string(),
                vector_entry.derived_id.to_string(),
                vector_entry.vector_entry_id.to_string(),
                vector_entry.vector_store.as_str(),
                vector_entry.embedding_model.as_str(),
                vector_entry.embedding_dimension,
                vector_entry.embedding_version.as_str(),
                enum_to_storage(vector_entry.source_projection)?,
                vector_entry.source_hash.as_str(),
                enum_to_storage(vector_entry.status)?,
                nearest_trace_ids,
                opt_string(vector_entry.cluster_id),
                opt_f32(vector_entry.duplicate_score),
                opt_f32(vector_entry.novelty_score),
                fmt_opt_ts(&vector_entry.indexed_at),
                fmt_opt_ts(&vector_entry.invalidated_at),
                fmt_opt_ts(&vector_entry.deleted_at),
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let mut rows = conn
            .query(
                "SELECT
                    tenant_id, submission_id, derived_id, vector_entry_id, vector_store,
                    embedding_model, embedding_dimension, embedding_version, source_projection,
                    source_hash, status, nearest_trace_ids, cluster_id, duplicate_score,
                    novelty_score, indexed_at, invalidated_at, deleted_at, created_at, updated_at
                 FROM trace_vector_entries
                 WHERE tenant_id = ?1 AND submission_id = ?2 AND vector_entry_id = ?3",
                libsql::params![
                    vector_entry.tenant_id.as_str(),
                    vector_entry.submission_id.to_string(),
                    vector_entry.vector_entry_id.to_string(),
                ],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        match rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            Some(row) => row_to_vector_entry(&row),
            None => Err(DatabaseError::NotFound {
                entity: "trace_vector_entry".to_string(),
                id: vector_entry.vector_entry_id.to_string(),
            }),
        }
    }

    async fn list_trace_vector_entries(
        &self,
        tenant_id: &str,
    ) -> Result<Vec<TraceVectorEntryRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT
                    tenant_id, submission_id, derived_id, vector_entry_id, vector_store,
                    embedding_model, embedding_dimension, embedding_version, source_projection,
                    source_hash, status, nearest_trace_ids, cluster_id, duplicate_score,
                    novelty_score, indexed_at, invalidated_at, deleted_at, created_at, updated_at
                 FROM trace_vector_entries
                 WHERE tenant_id = ?1
                 ORDER BY created_at ASC",
                libsql::params![tenant_id],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        let mut entries = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            entries.push(row_to_vector_entry(&row)?);
        }
        Ok(entries)
    }

    async fn upsert_trace_export_manifest(
        &self,
        manifest: TraceExportManifestWrite,
    ) -> Result<TraceExportManifestRecord, DatabaseError> {
        self.ensure_trace_tenant(&manifest.tenant_id).await?;
        let conn = self.connect().await?;
        let source_submission_ids = manifest
            .source_submission_ids
            .iter()
            .map(Uuid::to_string)
            .collect::<Vec<_>>();
        let source_submission_ids = json_string(&source_submission_ids)?;
        conn.execute(
            "INSERT INTO trace_export_manifests (
                tenant_id, export_manifest_id, artifact_kind, purpose_code, audit_event_id,
                source_submission_ids, source_submission_ids_hash, item_count, generated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
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
                updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
            libsql::params![
                manifest.tenant_id.as_str(),
                manifest.export_manifest_id.to_string(),
                enum_to_storage(manifest.artifact_kind)?,
                opt_string(manifest.purpose_code.clone()),
                opt_uuid(manifest.audit_event_id),
                source_submission_ids,
                manifest.source_submission_ids_hash.as_str(),
                i64::from(manifest.item_count),
                fmt_ts(&manifest.generated_at),
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let mut rows = conn
            .query(
                &format!(
                    "SELECT {TRACE_EXPORT_MANIFEST_COLUMNS}
                     FROM trace_export_manifests
                     WHERE tenant_id = ?1 AND export_manifest_id = ?2"
                ),
                libsql::params![
                    manifest.tenant_id.as_str(),
                    manifest.export_manifest_id.to_string(),
                ],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        match rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            Some(row) => row_to_export_manifest(&row),
            None => Err(DatabaseError::NotFound {
                entity: "trace_export_manifest".to_string(),
                id: manifest.export_manifest_id.to_string(),
            }),
        }
    }

    async fn upsert_trace_export_manifest_mirror(
        &self,
        mirror: TraceExportManifestMirrorWrite,
    ) -> Result<TraceExportManifestRecord, DatabaseError> {
        let tenant_id = mirror.manifest.tenant_id.clone();
        for object_ref in &mirror.object_refs {
            if object_ref.tenant_id != tenant_id {
                return Err(DatabaseError::Serialization(format!(
                    "trace export mirror object ref tenant {} does not match manifest tenant {}",
                    object_ref.tenant_id, tenant_id
                )));
            }
        }
        for item in &mirror.items {
            if item.tenant_id != tenant_id {
                return Err(DatabaseError::Serialization(format!(
                    "trace export mirror item tenant {} does not match manifest tenant {}",
                    item.tenant_id, tenant_id
                )));
            }
            if item.export_manifest_id != mirror.manifest.export_manifest_id {
                return Err(DatabaseError::Serialization(format!(
                    "trace export mirror item manifest {} does not match manifest {}",
                    item.export_manifest_id, mirror.manifest.export_manifest_id
                )));
            }
        }

        let conn = self.connect().await?;
        let tx = conn
            .transaction_with_behavior(libsql::TransactionBehavior::Immediate)
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        tx.execute(
            "INSERT INTO trace_tenants (tenant_id) VALUES (?1)
             ON CONFLICT (tenant_id) DO UPDATE SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
            libsql::params![tenant_id.as_str()],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;

        for object_ref in &mirror.object_refs {
            tx.execute(
                "INSERT INTO trace_object_refs (
                    tenant_id, submission_id, object_ref_id, artifact_kind, object_store,
                    object_key, content_sha256, encryption_key_ref, size_bytes, compression,
                    created_by_job_id
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                 ON CONFLICT (tenant_id, submission_id, object_ref_id) DO UPDATE SET
                    artifact_kind = excluded.artifact_kind,
                    object_store = excluded.object_store,
                    object_key = excluded.object_key,
                    content_sha256 = excluded.content_sha256,
                    encryption_key_ref = excluded.encryption_key_ref,
                    size_bytes = excluded.size_bytes,
                    compression = excluded.compression,
                    created_by_job_id = excluded.created_by_job_id,
                    updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
                libsql::params![
                    object_ref.tenant_id.as_str(),
                    object_ref.submission_id.to_string(),
                    object_ref.object_ref_id.to_string(),
                    enum_to_storage(object_ref.artifact_kind)?,
                    object_ref.object_store.as_str(),
                    object_ref.object_key.as_str(),
                    object_ref.content_sha256.as_str(),
                    object_ref.encryption_key_ref.as_str(),
                    object_ref.size_bytes,
                    opt_string(object_ref.compression.clone()),
                    opt_uuid(object_ref.created_by_job_id),
                ],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        }

        let source_submission_ids = mirror
            .manifest
            .source_submission_ids
            .iter()
            .map(Uuid::to_string)
            .collect::<Vec<_>>();
        let source_submission_ids = json_string(&source_submission_ids)?;
        tx.execute(
            "INSERT INTO trace_export_manifests (
                tenant_id, export_manifest_id, artifact_kind, purpose_code, audit_event_id,
                source_submission_ids, source_submission_ids_hash, item_count, generated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
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
                updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
            libsql::params![
                mirror.manifest.tenant_id.as_str(),
                mirror.manifest.export_manifest_id.to_string(),
                enum_to_storage(mirror.manifest.artifact_kind)?,
                opt_string(mirror.manifest.purpose_code.clone()),
                opt_uuid(mirror.manifest.audit_event_id),
                source_submission_ids,
                mirror.manifest.source_submission_ids_hash.as_str(),
                i64::from(mirror.manifest.item_count),
                fmt_ts(&mirror.manifest.generated_at),
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let record = {
            let mut rows = tx
                .query(
                    &format!(
                        "SELECT {TRACE_EXPORT_MANIFEST_COLUMNS}
                         FROM trace_export_manifests
                         WHERE tenant_id = ?1 AND export_manifest_id = ?2"
                    ),
                    libsql::params![
                        mirror.manifest.tenant_id.as_str(),
                        mirror.manifest.export_manifest_id.to_string(),
                    ],
                )
                .await
                .map_err(|e| DatabaseError::Query(e.to_string()))?;
            match rows
                .next()
                .await
                .map_err(|e| DatabaseError::Query(e.to_string()))?
            {
                Some(row) => row_to_export_manifest(&row)?,
                None => {
                    return Err(DatabaseError::NotFound {
                        entity: "trace_export_manifest".to_string(),
                        id: mirror.manifest.export_manifest_id.to_string(),
                    });
                }
            }
        };

        for item in &mirror.items {
            if let Some(derived_id) = item.derived_id {
                let exists = {
                    let mut rows = tx
                        .query(
                            "SELECT 1
                             FROM trace_derived_records
                             WHERE tenant_id = ?1 AND submission_id = ?2 AND derived_id = ?3
                             LIMIT 1",
                            libsql::params![
                                item.tenant_id.as_str(),
                                item.submission_id.to_string(),
                                derived_id.to_string(),
                            ],
                        )
                        .await
                        .map_err(|e| DatabaseError::Query(e.to_string()))?;
                    rows.next()
                        .await
                        .map_err(|e| DatabaseError::Query(e.to_string()))?
                        .is_some()
                };
                if !exists {
                    return Err(DatabaseError::NotFound {
                        entity: "export manifest mirror item derived record".to_string(),
                        id: format!("{}:{}", item.submission_id, derived_id),
                    });
                }
            }
            if let Some(object_ref_id) = item.object_ref_id {
                let exists = {
                    let mut rows = tx
                        .query(
                            "SELECT 1
                             FROM trace_object_refs
                             WHERE tenant_id = ?1 AND submission_id = ?2 AND object_ref_id = ?3
                             LIMIT 1",
                            libsql::params![
                                item.tenant_id.as_str(),
                                item.submission_id.to_string(),
                                object_ref_id.to_string(),
                            ],
                        )
                        .await
                        .map_err(|e| DatabaseError::Query(e.to_string()))?;
                    rows.next()
                        .await
                        .map_err(|e| DatabaseError::Query(e.to_string()))?
                        .is_some()
                };
                if !exists {
                    return Err(DatabaseError::NotFound {
                        entity: "export manifest mirror item object_ref".to_string(),
                        id: format!("{}:{}", item.submission_id, object_ref_id),
                    });
                }
            }
            if let Some(vector_entry_id) = item.vector_entry_id {
                let exists = {
                    let mut rows = tx
                        .query(
                            "SELECT 1
                             FROM trace_vector_entries
                             WHERE tenant_id = ?1 AND submission_id = ?2 AND vector_entry_id = ?3
                             LIMIT 1",
                            libsql::params![
                                item.tenant_id.as_str(),
                                item.submission_id.to_string(),
                                vector_entry_id.to_string(),
                            ],
                        )
                        .await
                        .map_err(|e| DatabaseError::Query(e.to_string()))?;
                    rows.next()
                        .await
                        .map_err(|e| DatabaseError::Query(e.to_string()))?
                        .is_some()
                };
                if !exists {
                    return Err(DatabaseError::NotFound {
                        entity: "export manifest mirror item vector entry".to_string(),
                        id: format!("{}:{}", item.submission_id, vector_entry_id),
                    });
                }
            }
            tx.execute(
                "INSERT INTO trace_export_manifest_items (
                    tenant_id, export_manifest_id, submission_id, trace_id, derived_id,
                    object_ref_id, vector_entry_id, source_status_at_export, source_hash_at_export
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                 ON CONFLICT (tenant_id, export_manifest_id, submission_id) DO UPDATE SET
                    trace_id = excluded.trace_id,
                    derived_id = excluded.derived_id,
                    object_ref_id = excluded.object_ref_id,
                    vector_entry_id = excluded.vector_entry_id,
                    source_status_at_export = excluded.source_status_at_export,
                    source_hash_at_export = excluded.source_hash_at_export,
                    source_invalidated_at = NULL,
                    source_invalidation_reason = NULL,
                    updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
                libsql::params![
                    item.tenant_id.as_str(),
                    item.export_manifest_id.to_string(),
                    item.submission_id.to_string(),
                    item.trace_id.to_string(),
                    opt_uuid(item.derived_id),
                    opt_uuid(item.object_ref_id),
                    opt_uuid(item.vector_entry_id),
                    enum_to_storage(item.source_status_at_export)?,
                    item.source_hash_at_export.as_str(),
                ],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        }

        tx.commit()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(record)
    }

    async fn delete_trace_export_manifest_mirror(
        &self,
        tenant_id: &str,
        export_manifest_id: Uuid,
    ) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        let tx = conn
            .transaction_with_behavior(libsql::TransactionBehavior::Immediate)
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        tx.execute(
            "DELETE FROM trace_export_manifest_items
             WHERE tenant_id = ?1 AND export_manifest_id = ?2",
            libsql::params![tenant_id, export_manifest_id.to_string()],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        tx.execute(
            "DELETE FROM trace_object_refs
             WHERE tenant_id = ?1 AND created_by_job_id = ?2",
            libsql::params![tenant_id, export_manifest_id.to_string()],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        tx.execute(
            "DELETE FROM trace_export_manifests
             WHERE tenant_id = ?1 AND export_manifest_id = ?2",
            libsql::params![tenant_id, export_manifest_id.to_string()],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        tx.commit()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn list_trace_export_manifests(
        &self,
        tenant_id: &str,
    ) -> Result<Vec<TraceExportManifestRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                &format!(
                    "SELECT {TRACE_EXPORT_MANIFEST_COLUMNS}
                     FROM trace_export_manifests
                     WHERE tenant_id = ?1
                     ORDER BY generated_at ASC"
                ),
                libsql::params![tenant_id],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        let mut manifests = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            manifests.push(row_to_export_manifest(&row)?);
        }
        Ok(manifests)
    }

    async fn upsert_trace_export_manifest_item(
        &self,
        item: TraceExportManifestItemWrite,
    ) -> Result<TraceExportManifestItemRecord, DatabaseError> {
        self.ensure_trace_tenant(&item.tenant_id).await?;
        let conn = self.connect().await?;
        if let Some(derived_id) = item.derived_id {
            ensure_libsql_derived_record_belongs_to_submission(
                &conn,
                &item.tenant_id,
                item.submission_id,
                derived_id,
            )
            .await?;
        }
        if let Some(object_ref_id) = item.object_ref_id {
            ensure_libsql_object_ref_belongs_to_submission(
                &conn,
                &item.tenant_id,
                item.submission_id,
                object_ref_id,
                "export manifest item",
            )
            .await?;
        }
        if let Some(vector_entry_id) = item.vector_entry_id {
            ensure_libsql_vector_entry_belongs_to_submission(
                &conn,
                &item.tenant_id,
                item.submission_id,
                vector_entry_id,
            )
            .await?;
        }
        conn.execute(
            "INSERT INTO trace_export_manifest_items (
                tenant_id, export_manifest_id, submission_id, trace_id, derived_id,
                object_ref_id, vector_entry_id, source_status_at_export, source_hash_at_export
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT (tenant_id, export_manifest_id, submission_id) DO UPDATE SET
                trace_id = excluded.trace_id,
                derived_id = excluded.derived_id,
                object_ref_id = excluded.object_ref_id,
                vector_entry_id = excluded.vector_entry_id,
                source_status_at_export = excluded.source_status_at_export,
                source_hash_at_export = excluded.source_hash_at_export,
                source_invalidated_at = NULL,
                source_invalidation_reason = NULL,
                updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
            libsql::params![
                item.tenant_id.as_str(),
                item.export_manifest_id.to_string(),
                item.submission_id.to_string(),
                item.trace_id.to_string(),
                opt_uuid(item.derived_id),
                opt_uuid(item.object_ref_id),
                opt_uuid(item.vector_entry_id),
                enum_to_storage(item.source_status_at_export)?,
                item.source_hash_at_export.as_str(),
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let mut rows = conn
            .query(
                &format!(
                    "SELECT {TRACE_EXPORT_MANIFEST_ITEM_COLUMNS}
                     FROM trace_export_manifest_items
                     WHERE tenant_id = ?1 AND export_manifest_id = ?2 AND submission_id = ?3"
                ),
                libsql::params![
                    item.tenant_id.as_str(),
                    item.export_manifest_id.to_string(),
                    item.submission_id.to_string(),
                ],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        match rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            Some(row) => row_to_export_manifest_item(&row),
            None => Err(DatabaseError::NotFound {
                entity: "trace_export_manifest_item".to_string(),
                id: format!("{}:{}", item.export_manifest_id, item.submission_id),
            }),
        }
    }

    async fn list_trace_export_manifest_items(
        &self,
        tenant_id: &str,
        export_manifest_id: Uuid,
    ) -> Result<Vec<TraceExportManifestItemRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                &format!(
                    "SELECT {TRACE_EXPORT_MANIFEST_ITEM_COLUMNS}
                     FROM trace_export_manifest_items
                     WHERE tenant_id = ?1 AND export_manifest_id = ?2
                     ORDER BY created_at ASC"
                ),
                libsql::params![tenant_id, export_manifest_id.to_string()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        let mut items = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            items.push(row_to_export_manifest_item(&row)?);
        }
        Ok(items)
    }

    async fn invalidate_trace_export_manifests_for_submission(
        &self,
        tenant_id: &str,
        submission_id: Uuid,
    ) -> Result<u64, DatabaseError> {
        let conn = self.connect().await?;
        conn.execute(
            "UPDATE trace_export_manifests
             SET invalidated_at = COALESCE(invalidated_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
                 updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             WHERE tenant_id = ?1
               AND invalidated_at IS NULL
               AND deleted_at IS NULL
               AND EXISTS (
                   SELECT 1
                   FROM json_each(source_submission_ids)
                   WHERE value = ?2
               )",
            libsql::params![tenant_id, submission_id.to_string()],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))
    }

    async fn invalidate_trace_export_manifest_items_for_submission(
        &self,
        tenant_id: &str,
        submission_id: Uuid,
        reason: TraceExportManifestItemInvalidationReason,
    ) -> Result<u64, DatabaseError> {
        let conn = self.connect().await?;
        conn.execute(
            "UPDATE trace_export_manifest_items
             SET source_invalidated_at = COALESCE(source_invalidated_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
                 source_invalidation_reason = ?3,
                 updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             WHERE tenant_id = ?1
               AND submission_id = ?2
               AND source_invalidated_at IS NULL",
            libsql::params![tenant_id, submission_id.to_string(), enum_to_storage(reason)?],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))
    }

    async fn invalidate_trace_vector_entries_for_submission(
        &self,
        tenant_id: &str,
        submission_id: Uuid,
    ) -> Result<u64, DatabaseError> {
        let conn = self.connect().await?;
        let invalidated = enum_to_storage(TraceVectorEntryStatus::Invalidated)?;
        conn.execute(
            "UPDATE trace_vector_entries
             SET status = ?3,
                 invalidated_at = COALESCE(invalidated_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
                 updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             WHERE tenant_id = ?1
               AND submission_id = ?2
               AND status <> ?3
               AND deleted_at IS NULL",
            libsql::params![tenant_id, submission_id.to_string(), invalidated],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))
    }

    async fn append_trace_audit_event(
        &self,
        audit_event: TraceAuditEventWrite,
    ) -> Result<(), DatabaseError> {
        self.ensure_trace_tenant(&audit_event.tenant_id).await?;
        let conn = self.connect().await?;
        conn.execute("BEGIN IMMEDIATE", libsql::params![])
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        let result = async {
            let latest_event_hash = {
                let mut rows = conn
                    .query(
                        "SELECT event_hash
                         FROM trace_audit_events
                         WHERE tenant_id = ?1
                           AND event_hash IS NOT NULL
                         ORDER BY audit_sequence DESC
                         LIMIT 1",
                        libsql::params![audit_event.tenant_id.as_str()],
                    )
                    .await
                    .map_err(|e| DatabaseError::Query(e.to_string()))?;
                rows.next()
                    .await
                    .map_err(|e| DatabaseError::Query(e.to_string()))?
                    .map(|row| get_text(&row, 0))
            };
            validate_trace_audit_append_chain(
                &audit_event.tenant_id,
                audit_event.audit_event_id,
                latest_event_hash.as_deref(),
                audit_event.previous_event_hash.as_deref(),
                audit_event.event_hash.is_some(),
            )?;

            let next_audit_sequence = {
                let mut rows = conn
                    .query(
                        "SELECT COALESCE(MAX(audit_sequence), 0) + 1
                         FROM trace_audit_events
                         WHERE tenant_id = ?1",
                        libsql::params![audit_event.tenant_id.as_str()],
                    )
                    .await
                    .map_err(|e| DatabaseError::Query(e.to_string()))?;
                let row = rows
                    .next()
                    .await
                    .map_err(|e| DatabaseError::Query(e.to_string()))?
                    .ok_or_else(|| {
                        DatabaseError::Query(
                            "trace audit sequence query returned no rows".to_string(),
                        )
                    })?;
                row.get::<i64>(0)
                    .map_err(|e| DatabaseError::Query(e.to_string()))?
            };

            conn.execute(
                "INSERT INTO trace_audit_events (
                    tenant_id, audit_sequence, audit_event_id, actor_principal_ref, actor_role,
                    action, reason, request_id, submission_id, object_ref_id, export_manifest_id,
                    decision_inputs_hash, previous_event_hash, event_hash, canonical_event_json,
                    metadata_json
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
                libsql::params![
                    audit_event.tenant_id.as_str(),
                    next_audit_sequence,
                    audit_event.audit_event_id.to_string(),
                    audit_event.actor_principal_ref.as_str(),
                    audit_event.actor_role.as_str(),
                    enum_to_storage(audit_event.action)?,
                    opt_string(audit_event.reason),
                    opt_string(audit_event.request_id),
                    opt_uuid(audit_event.submission_id),
                    opt_uuid(audit_event.object_ref_id),
                    opt_uuid(audit_event.export_manifest_id),
                    opt_string(audit_event.decision_inputs_hash),
                    opt_string(audit_event.previous_event_hash),
                    opt_string(audit_event.event_hash),
                    opt_string(audit_event.canonical_event_json),
                    json_string(&audit_event.metadata)?,
                ],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
            Ok(())
        }
        .await;

        match result {
            Ok(()) => {
                conn.execute("COMMIT", libsql::params![])
                    .await
                    .map_err(|e| DatabaseError::Query(e.to_string()))?;
                Ok(())
            }
            Err(error) => {
                let _ = conn.execute("ROLLBACK", libsql::params![]).await;
                Err(error)
            }
        }
    }

    async fn list_trace_audit_events(
        &self,
        tenant_id: &str,
    ) -> Result<Vec<TraceAuditEventRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT
                    tenant_id, audit_sequence, audit_event_id, actor_principal_ref, actor_role,
                    action, reason, request_id, submission_id, object_ref_id, export_manifest_id,
                    decision_inputs_hash, previous_event_hash, event_hash, canonical_event_json,
                    metadata_json,
                    occurred_at
                 FROM trace_audit_events
                 WHERE tenant_id = ?1
                 ORDER BY audit_sequence ASC",
                libsql::params![tenant_id],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        let mut events = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            events.push(row_to_audit_event(&row)?);
        }
        Ok(events)
    }

    async fn append_trace_credit_event(
        &self,
        credit_event: TraceCreditEventWrite,
    ) -> Result<(), DatabaseError> {
        self.ensure_trace_tenant(&credit_event.tenant_id).await?;
        let conn = self.connect().await?;
        conn.execute(
            "INSERT INTO trace_credit_ledger (
                tenant_id, credit_event_id, submission_id, trace_id, credit_account_ref,
                event_type, points_delta, reason, external_ref, actor_principal_ref,
                actor_role, settlement_state
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            libsql::params![
                credit_event.tenant_id.as_str(),
                credit_event.credit_event_id.to_string(),
                credit_event.submission_id.to_string(),
                credit_event.trace_id.to_string(),
                credit_event.credit_account_ref.as_str(),
                enum_to_storage(credit_event.event_type)?,
                credit_event.points_delta.as_str(),
                credit_event.reason.as_str(),
                opt_string(credit_event.external_ref),
                credit_event.actor_principal_ref.as_str(),
                credit_event.actor_role.as_str(),
                enum_to_storage(credit_event.settlement_state)?,
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn write_trace_tombstone(
        &self,
        tombstone: TraceTombstoneWrite,
    ) -> Result<(), DatabaseError> {
        self.ensure_trace_tenant(&tombstone.tenant_id).await?;
        let conn = self.connect().await?;
        conn.execute(
            "INSERT INTO trace_tombstones (
                tenant_id, tombstone_id, submission_id, trace_id, redaction_hash,
                canonical_summary_hash, reason, effective_at, retain_until,
                created_by_principal_ref
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT (tenant_id, submission_id) DO NOTHING",
            libsql::params![
                tombstone.tenant_id.as_str(),
                tombstone.tombstone_id.to_string(),
                tombstone.submission_id.to_string(),
                opt_uuid(tombstone.trace_id),
                opt_string(tombstone.redaction_hash),
                opt_string(tombstone.canonical_summary_hash),
                tombstone.reason.as_str(),
                fmt_ts(&tombstone.effective_at),
                fmt_opt_ts(&tombstone.retain_until),
                tombstone.created_by_principal_ref.as_str(),
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn list_trace_tombstones(
        &self,
        tenant_id: &str,
    ) -> Result<Vec<TraceTombstoneRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                &format!(
                    "SELECT {TRACE_TOMBSTONE_COLUMNS}
                     FROM trace_tombstones
                     WHERE tenant_id = ?1
                     ORDER BY effective_at ASC, created_at ASC"
                ),
                libsql::params![tenant_id],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        let mut tombstones = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            tombstones.push(row_to_tombstone(&row)?);
        }
        Ok(tombstones)
    }

    async fn upsert_trace_retention_job(
        &self,
        job: TraceRetentionJobWrite,
    ) -> Result<TraceRetentionJobRecord, DatabaseError> {
        self.ensure_trace_tenant(&job.tenant_id).await?;
        let conn = self.connect().await?;
        let action_counts = json_string(&job.action_counts)?;
        conn.execute(
            "INSERT INTO trace_retention_jobs (
                tenant_id, retention_job_id, purpose, dry_run, status,
                requested_by_principal_ref, requested_by_role, purge_expired_before,
                prune_export_cache, max_export_age_hours, audit_event_id, action_counts,
                selected_revoked_count, selected_expired_count, started_at, completed_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)
             ON CONFLICT (tenant_id, retention_job_id) DO UPDATE SET
                purpose = excluded.purpose,
                dry_run = excluded.dry_run,
                status = excluded.status,
                requested_by_principal_ref = excluded.requested_by_principal_ref,
                requested_by_role = excluded.requested_by_role,
                purge_expired_before = excluded.purge_expired_before,
                prune_export_cache = excluded.prune_export_cache,
                max_export_age_hours = excluded.max_export_age_hours,
                audit_event_id = excluded.audit_event_id,
                action_counts = excluded.action_counts,
                selected_revoked_count = excluded.selected_revoked_count,
                selected_expired_count = excluded.selected_expired_count,
                started_at = excluded.started_at,
                completed_at = excluded.completed_at,
                updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
            libsql::params![
                job.tenant_id.as_str(),
                job.retention_job_id.to_string(),
                job.purpose.as_str(),
                i64::from(job.dry_run),
                enum_to_storage(job.status)?,
                job.requested_by_principal_ref.as_str(),
                job.requested_by_role.as_str(),
                fmt_opt_ts(&job.purge_expired_before),
                i64::from(job.prune_export_cache),
                job.max_export_age_hours,
                opt_uuid(job.audit_event_id),
                action_counts,
                i64::from(job.selected_revoked_count),
                i64::from(job.selected_expired_count),
                fmt_opt_ts(&job.started_at),
                fmt_opt_ts(&job.completed_at),
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let mut rows = conn
            .query(
                &format!(
                    "SELECT {TRACE_RETENTION_JOB_COLUMNS}
                     FROM trace_retention_jobs
                     WHERE tenant_id = ?1 AND retention_job_id = ?2"
                ),
                libsql::params![job.tenant_id.as_str(), job.retention_job_id.to_string()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        match rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            Some(row) => row_to_retention_job(&row),
            None => Err(DatabaseError::NotFound {
                entity: "trace_retention_job".to_string(),
                id: job.retention_job_id.to_string(),
            }),
        }
    }

    async fn upsert_trace_retention_job_item(
        &self,
        item: TraceRetentionJobItemWrite,
    ) -> Result<TraceRetentionJobItemRecord, DatabaseError> {
        self.ensure_trace_tenant(&item.tenant_id).await?;
        let conn = self.connect().await?;
        let action_counts = json_string(&item.action_counts)?;
        conn.execute(
            "INSERT INTO trace_retention_job_items (
                tenant_id, retention_job_id, submission_id, action, status, reason,
                action_counts, verified_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT (tenant_id, retention_job_id, submission_id, action) DO UPDATE SET
                status = excluded.status,
                reason = excluded.reason,
                action_counts = excluded.action_counts,
                verified_at = excluded.verified_at,
                updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
            libsql::params![
                item.tenant_id.as_str(),
                item.retention_job_id.to_string(),
                item.submission_id.to_string(),
                enum_to_storage(item.action)?,
                enum_to_storage(item.status)?,
                item.reason.as_str(),
                action_counts,
                fmt_opt_ts(&item.verified_at),
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let action = enum_to_storage(item.action)?;
        let mut rows = conn
            .query(
                &format!(
                    "SELECT {TRACE_RETENTION_JOB_ITEM_COLUMNS}
                     FROM trace_retention_job_items
                     WHERE tenant_id = ?1
                       AND retention_job_id = ?2
                       AND submission_id = ?3
                       AND action = ?4"
                ),
                libsql::params![
                    item.tenant_id.as_str(),
                    item.retention_job_id.to_string(),
                    item.submission_id.to_string(),
                    action.as_str(),
                ],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        match rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            Some(row) => row_to_retention_job_item(&row),
            None => Err(DatabaseError::NotFound {
                entity: "trace_retention_job_item".to_string(),
                id: format!(
                    "{}:{}:{}",
                    item.retention_job_id, item.submission_id, action
                ),
            }),
        }
    }

    async fn list_trace_retention_jobs(
        &self,
        tenant_id: &str,
    ) -> Result<Vec<TraceRetentionJobRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                &format!(
                    "SELECT {TRACE_RETENTION_JOB_COLUMNS}
                     FROM trace_retention_jobs
                     WHERE tenant_id = ?1
                     ORDER BY created_at ASC"
                ),
                libsql::params![tenant_id],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        let mut jobs = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            jobs.push(row_to_retention_job(&row)?);
        }
        Ok(jobs)
    }

    async fn list_trace_retention_job_items(
        &self,
        tenant_id: &str,
        retention_job_id: Uuid,
    ) -> Result<Vec<TraceRetentionJobItemRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                &format!(
                    "SELECT {TRACE_RETENTION_JOB_ITEM_COLUMNS}
                     FROM trace_retention_job_items
                     WHERE tenant_id = ?1 AND retention_job_id = ?2
                     ORDER BY created_at ASC, submission_id ASC, action ASC"
                ),
                libsql::params![tenant_id, retention_job_id.to_string()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        let mut items = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            items.push(row_to_retention_job_item(&row)?);
        }
        Ok(items)
    }

    async fn upsert_trace_export_access_grant(
        &self,
        grant: TraceExportAccessGrantWrite,
    ) -> Result<TraceExportAccessGrantRecord, DatabaseError> {
        self.ensure_trace_tenant(&grant.tenant_id).await?;
        let conn = self.connect().await?;
        let metadata_json = json_string(&grant.metadata)?;
        conn.execute(
            "INSERT INTO trace_export_access_grants (
                tenant_id, export_job_id, grant_id, caller_principal_ref, requested_dataset_kind,
                purpose, max_item_cap, status, requested_at, expires_at, metadata_json
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
             ON CONFLICT (tenant_id, grant_id) DO UPDATE SET
                export_job_id = excluded.export_job_id,
                caller_principal_ref = excluded.caller_principal_ref,
                requested_dataset_kind = excluded.requested_dataset_kind,
                purpose = excluded.purpose,
                max_item_cap = excluded.max_item_cap,
                status = excluded.status,
                requested_at = excluded.requested_at,
                expires_at = excluded.expires_at,
                metadata_json = excluded.metadata_json,
                updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
            libsql::params![
                grant.tenant_id.as_str(),
                grant.export_job_id.to_string(),
                grant.grant_id.to_string(),
                grant.caller_principal_ref.as_str(),
                grant.requested_dataset_kind.as_str(),
                grant.purpose.as_str(),
                opt_u32(grant.max_item_cap),
                enum_to_storage(grant.status)?,
                fmt_ts(&grant.requested_at),
                fmt_ts(&grant.expires_at),
                metadata_json,
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let mut rows = conn
            .query(
                &format!(
                    "SELECT {TRACE_EXPORT_ACCESS_GRANT_COLUMNS}
                     FROM trace_export_access_grants
                     WHERE tenant_id = ?1 AND grant_id = ?2"
                ),
                libsql::params![grant.tenant_id.as_str(), grant.grant_id.to_string()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        match rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            Some(row) => row_to_export_access_grant(&row),
            None => Err(DatabaseError::NotFound {
                entity: "trace_export_access_grant".to_string(),
                id: grant.grant_id.to_string(),
            }),
        }
    }

    async fn list_trace_export_access_grants(
        &self,
        tenant_id: &str,
    ) -> Result<Vec<TraceExportAccessGrantRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                &format!(
                    "SELECT {TRACE_EXPORT_ACCESS_GRANT_COLUMNS}
                     FROM trace_export_access_grants
                     WHERE tenant_id = ?1
                     ORDER BY requested_at ASC, created_at ASC"
                ),
                libsql::params![tenant_id],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        let mut grants = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            grants.push(row_to_export_access_grant(&row)?);
        }
        Ok(grants)
    }

    async fn upsert_trace_export_job(
        &self,
        job: TraceExportJobWrite,
    ) -> Result<TraceExportJobRecord, DatabaseError> {
        self.ensure_trace_tenant(&job.tenant_id).await?;
        let conn = self.connect().await?;
        let metadata_json = json_string(&job.metadata)?;
        conn.execute(
            "INSERT INTO trace_export_jobs (
                tenant_id, export_job_id, grant_id, caller_principal_ref, requested_dataset_kind,
                purpose, max_item_cap, status, requested_at, started_at, finished_at, expires_at,
                result_manifest_id, item_count, last_error, metadata_json
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)
             ON CONFLICT (tenant_id, export_job_id) DO UPDATE SET
                grant_id = excluded.grant_id,
                caller_principal_ref = excluded.caller_principal_ref,
                requested_dataset_kind = excluded.requested_dataset_kind,
                purpose = excluded.purpose,
                max_item_cap = excluded.max_item_cap,
                status = excluded.status,
                requested_at = excluded.requested_at,
                started_at = excluded.started_at,
                finished_at = excluded.finished_at,
                expires_at = excluded.expires_at,
                result_manifest_id = excluded.result_manifest_id,
                item_count = excluded.item_count,
                last_error = excluded.last_error,
                metadata_json = excluded.metadata_json,
                updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
            libsql::params![
                job.tenant_id.as_str(),
                job.export_job_id.to_string(),
                job.grant_id.to_string(),
                job.caller_principal_ref.as_str(),
                job.requested_dataset_kind.as_str(),
                job.purpose.as_str(),
                opt_u32(job.max_item_cap),
                enum_to_storage(job.status)?,
                fmt_ts(&job.requested_at),
                fmt_opt_ts(&job.started_at),
                fmt_opt_ts(&job.finished_at),
                fmt_ts(&job.expires_at),
                opt_uuid(job.result_manifest_id),
                opt_u32(job.item_count),
                opt_string(job.last_error),
                metadata_json,
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let mut rows = conn
            .query(
                &format!(
                    "SELECT {TRACE_EXPORT_JOB_COLUMNS}
                     FROM trace_export_jobs
                     WHERE tenant_id = ?1 AND export_job_id = ?2"
                ),
                libsql::params![job.tenant_id.as_str(), job.export_job_id.to_string()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        match rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            Some(row) => row_to_export_job(&row),
            None => Err(DatabaseError::NotFound {
                entity: "trace_export_job".to_string(),
                id: job.export_job_id.to_string(),
            }),
        }
    }

    async fn list_trace_export_jobs(
        &self,
        tenant_id: &str,
    ) -> Result<Vec<TraceExportJobRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                &format!(
                    "SELECT {TRACE_EXPORT_JOB_COLUMNS}
                     FROM trace_export_jobs
                     WHERE tenant_id = ?1
                     ORDER BY requested_at ASC, created_at ASC"
                ),
                libsql::params![tenant_id],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        let mut jobs = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            jobs.push(row_to_export_job(&row)?);
        }
        Ok(jobs)
    }

    async fn update_trace_export_job_status(
        &self,
        tenant_id: &str,
        export_job_id: Uuid,
        update: TraceExportJobStatusUpdate,
    ) -> Result<Option<TraceExportJobRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let metadata_json = json_string(&update.metadata)?;
        let affected = conn
            .execute(
                "UPDATE trace_export_jobs
                 SET status = ?3,
                     started_at = ?4,
                     finished_at = ?5,
                     result_manifest_id = ?6,
                     item_count = ?7,
                     last_error = ?8,
                     metadata_json = ?9,
                     updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                 WHERE tenant_id = ?1 AND export_job_id = ?2",
                libsql::params![
                    tenant_id,
                    export_job_id.to_string(),
                    enum_to_storage(update.status)?,
                    fmt_opt_ts(&update.started_at),
                    fmt_opt_ts(&update.finished_at),
                    opt_uuid(update.result_manifest_id),
                    opt_u32(update.item_count),
                    opt_string(update.last_error),
                    metadata_json,
                ],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        if affected == 0 {
            return Ok(None);
        }

        let mut rows = conn
            .query(
                &format!(
                    "SELECT {TRACE_EXPORT_JOB_COLUMNS}
                     FROM trace_export_jobs
                     WHERE tenant_id = ?1 AND export_job_id = ?2"
                ),
                libsql::params![tenant_id, export_job_id.to_string()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        rows.next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
            .map(|row| row_to_export_job(&row))
            .transpose()
    }

    async fn upsert_trace_revocation_propagation_item(
        &self,
        item: TraceRevocationPropagationItemWrite,
    ) -> Result<TraceRevocationPropagationItemRecord, DatabaseError> {
        self.ensure_trace_tenant(&item.tenant_id).await?;
        let target_kind = enum_to_storage(item.target.kind())?;
        let target_json = json_string(&item.target)?;
        let action = enum_to_storage(item.action)?;
        let status = enum_to_storage(item.status)?;
        let metadata_json = json_string(&item.metadata)?;
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                &format!(
                    "INSERT INTO trace_revocation_propagation_items (
                        tenant_id, propagation_item_id, source_submission_id, trace_id,
                        target_kind, target_json, action, status, idempotency_key, reason,
                        attempt_count, last_error, next_attempt_at, completed_at, evidence_hash,
                        metadata_json
                     )
                     SELECT ?1, ?2, ?3, submission.trace_id, ?4, ?5, ?6, ?7, ?8, ?9,
                            ?10, ?11, ?12, ?13, ?14, ?15
                     FROM trace_submissions submission
                     WHERE submission.tenant_id = ?1
                       AND submission.submission_id = ?3
                     ON CONFLICT (tenant_id, idempotency_key) DO UPDATE
                     SET target_kind = excluded.target_kind,
                         target_json = excluded.target_json,
                         action = excluded.action,
                         status = excluded.status,
                         reason = excluded.reason,
                         attempt_count = excluded.attempt_count,
                         last_error = excluded.last_error,
                         next_attempt_at = excluded.next_attempt_at,
                         completed_at = excluded.completed_at,
                         evidence_hash = excluded.evidence_hash,
                         metadata_json = excluded.metadata_json,
                         updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                     RETURNING {TRACE_REVOCATION_PROPAGATION_ITEM_COLUMNS}"
                ),
                libsql::params![
                    item.tenant_id.as_str(),
                    item.propagation_item_id.to_string(),
                    item.source_submission_id.to_string(),
                    target_kind,
                    target_json,
                    action,
                    status,
                    item.idempotency_key.as_str(),
                    item.reason.as_str(),
                    i64::from(item.attempt_count),
                    opt_string(item.last_error),
                    fmt_opt_ts(&item.next_attempt_at),
                    fmt_opt_ts(&item.completed_at),
                    opt_string(item.evidence_hash),
                    metadata_json,
                ],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        let row = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
            .ok_or_else(|| {
                DatabaseError::Constraint(format!(
                    "trace revocation propagation source submission {} does not belong to tenant {}",
                    item.source_submission_id, item.tenant_id
                ))
            })?;
        row_to_revocation_propagation_item(&row)
    }

    async fn list_trace_revocation_propagation_items(
        &self,
        tenant_id: &str,
        source_submission_id: Uuid,
    ) -> Result<Vec<TraceRevocationPropagationItemRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                &format!(
                    "SELECT {TRACE_REVOCATION_PROPAGATION_ITEM_COLUMNS}
                     FROM trace_revocation_propagation_items
                     WHERE tenant_id = ?1
                       AND source_submission_id = ?2
                     ORDER BY created_at ASC, propagation_item_id ASC"
                ),
                libsql::params![tenant_id, source_submission_id.to_string()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        let mut items = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            items.push(row_to_revocation_propagation_item(&row)?);
        }
        Ok(items)
    }

    async fn list_due_trace_revocation_propagation_items(
        &self,
        tenant_id: &str,
        now: DateTime<Utc>,
        limit: u32,
    ) -> Result<Vec<TraceRevocationPropagationItemRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let pending = enum_to_storage(TraceRevocationPropagationItemStatus::Pending)?;
        let failed = enum_to_storage(TraceRevocationPropagationItemStatus::Failed)?;
        let mut rows = conn
            .query(
                &format!(
                    "SELECT {TRACE_REVOCATION_PROPAGATION_ITEM_COLUMNS}
                     FROM trace_revocation_propagation_items
                     WHERE tenant_id = ?1
                       AND status IN (?2, ?3)
                       AND (next_attempt_at IS NULL OR next_attempt_at <= ?4)
                     ORDER BY created_at ASC, propagation_item_id ASC
                     LIMIT ?5"
                ),
                libsql::params![tenant_id, pending, failed, fmt_ts(&now), i64::from(limit)],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        let mut items = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            items.push(row_to_revocation_propagation_item(&row)?);
        }
        Ok(items)
    }

    async fn update_trace_revocation_propagation_item_status(
        &self,
        tenant_id: &str,
        propagation_item_id: Uuid,
        update: TraceRevocationPropagationItemStatusUpdate,
    ) -> Result<Option<TraceRevocationPropagationItemRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let status = enum_to_storage(update.status)?;
        let mut rows = conn
            .query(
                &format!(
                    "UPDATE trace_revocation_propagation_items
                     SET status = ?3,
                         attempt_count = ?4,
                         last_error = ?5,
                         next_attempt_at = ?6,
                         completed_at = ?7,
                         evidence_hash = ?8,
                         updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                     WHERE tenant_id = ?1
                       AND propagation_item_id = ?2
                     RETURNING {TRACE_REVOCATION_PROPAGATION_ITEM_COLUMNS}"
                ),
                libsql::params![
                    tenant_id,
                    propagation_item_id.to_string(),
                    status,
                    i64::from(update.attempt_count),
                    opt_string(update.last_error),
                    fmt_opt_ts(&update.next_attempt_at),
                    fmt_opt_ts(&update.completed_at),
                    opt_string(update.evidence_hash),
                ],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        rows.next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
            .map(|row| row_to_revocation_propagation_item(&row))
            .transpose()
    }

    async fn invalidate_trace_submission_artifacts(
        &self,
        tenant_id: &str,
        submission_id: Uuid,
        derived_status: TraceDerivedStatus,
    ) -> Result<TraceArtifactInvalidationCounts, DatabaseError> {
        let conn = self.connect().await?;
        let derived_status = enum_to_storage(derived_status)?;
        let object_refs_invalidated = conn
            .execute(
                "UPDATE trace_object_refs
                 SET invalidated_at = COALESCE(invalidated_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
                     updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                 WHERE tenant_id = ?1
                   AND submission_id = ?2
                   AND invalidated_at IS NULL
                   AND deleted_at IS NULL",
                libsql::params![tenant_id, submission_id.to_string()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        let derived_records_invalidated = conn
            .execute(
                "UPDATE trace_derived_records
                 SET status = ?3,
                     updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                 WHERE tenant_id = ?1
                   AND submission_id = ?2
                   AND status <> ?3",
                libsql::params![tenant_id, submission_id.to_string(), derived_status],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(TraceArtifactInvalidationCounts {
            object_refs_invalidated,
            derived_records_invalidated,
        })
    }

    async fn mark_trace_object_ref_deleted(
        &self,
        tenant_id: &str,
        submission_id: Uuid,
        object_store: &str,
        object_key: &str,
    ) -> Result<u64, DatabaseError> {
        let conn = self.connect().await?;
        conn.execute(
            "UPDATE trace_object_refs
             SET invalidated_at = COALESCE(invalidated_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
                 deleted_at = COALESCE(deleted_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
                 updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             WHERE tenant_id = ?1
               AND submission_id = ?2
               AND object_store = ?3
               AND object_key = ?4
               AND deleted_at IS NULL",
            libsql::params![
                tenant_id,
                submission_id.to_string(),
                object_store,
                object_key
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))
    }
}
