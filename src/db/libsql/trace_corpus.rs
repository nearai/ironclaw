use std::collections::BTreeMap;

use async_trait::async_trait;
use uuid::Uuid;

use super::{LibSqlBackend, fmt_opt_ts, fmt_ts, get_opt_text, get_opt_ts, get_text, get_ts};
use crate::db::trace_corpus_common::{
    audit_action_for_status, enum_from_storage, enum_to_storage, parse_uuid,
};
use crate::error::DatabaseError;
use crate::trace_corpus_storage::{
    TenantScopedTraceObjectRef, TraceArtifactInvalidationCounts, TraceAuditEventRecord,
    TraceAuditEventWrite, TraceAuditSafeMetadata, TraceCorpusStatus, TraceCorpusStore,
    TraceCreditEventRecord, TraceCreditEventWrite, TraceCreditSettlementState, TraceDerivedRecord,
    TraceDerivedRecordWrite, TraceDerivedStatus, TraceObjectArtifactKind, TraceObjectRefRecord,
    TraceObjectRefWrite, TraceSubmissionRecord, TraceSubmissionWrite, TraceTombstoneWrite,
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
    reviewed_at, revoked_at, expires_at, purged_at";

const TRACE_OBJECT_REF_COLUMNS: &str = "\
    tenant_id, submission_id, object_ref_id, artifact_kind, object_store, object_key, \
    content_sha256, encryption_key_ref, size_bytes, compression, created_by_job_id, \
    invalidated_at, deleted_at, updated_at, created_at";

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

fn json_u32_map(raw: &str, column: &str) -> Result<BTreeMap<String, u32>, DatabaseError> {
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
        revoked_at: get_opt_ts(row, 23),
        expires_at: get_opt_ts(row, 24),
        purged_at: get_opt_ts(row, 25),
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

fn row_to_audit_event(row: &libsql::Row) -> Result<TraceAuditEventRecord, DatabaseError> {
    let submission_id = get_opt_text(row, 7)
        .map(|id| parse_uuid(&id, "trace_audit_events.submission_id"))
        .transpose()?;
    let object_ref_id = get_opt_text(row, 8)
        .map(|id| parse_uuid(&id, "trace_audit_events.object_ref_id"))
        .transpose()?;
    let export_manifest_id = get_opt_text(row, 9)
        .map(|id| parse_uuid(&id, "trace_audit_events.export_manifest_id"))
        .transpose()?;
    let metadata = serde_json::from_str(&get_text(row, 11)).map_err(|e| {
        DatabaseError::Serialization(format!("trace audit metadata JSON decode failed: {e}"))
    })?;
    Ok(TraceAuditEventRecord {
        tenant_id: get_text(row, 0),
        audit_event_id: parse_uuid(&get_text(row, 1), "trace_audit_events.audit_event_id")?,
        actor_principal_ref: get_text(row, 2),
        actor_role: get_text(row, 3),
        action: enum_from_storage(&get_text(row, 4), "TraceAuditAction")?,
        reason: get_opt_text(row, 5),
        request_id: get_opt_text(row, 6),
        submission_id,
        object_ref_id,
        export_manifest_id,
        decision_inputs_hash: get_opt_text(row, 10),
        metadata,
        occurred_at: get_ts(row, 12),
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
                 revoked_at = CASE
                     WHEN ?3 = 'revoked' THEN strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                     ELSE revoked_at
                 END,
                 purged_at = CASE
                     WHEN ?3 = 'purged' THEN strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                     ELSE purged_at
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
            metadata: TraceAuditSafeMetadata::ReviewDecision {
                decision: enum_to_storage(status)?,
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
        conn.execute(
            "INSERT INTO trace_audit_events (
                tenant_id, audit_event_id, actor_principal_ref, actor_role, action, reason,
                request_id, submission_id, object_ref_id, export_manifest_id,
                decision_inputs_hash, metadata_json
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            libsql::params![
                audit_event.tenant_id.as_str(),
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
                json_string(&audit_event.metadata)?,
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn list_trace_audit_events(
        &self,
        tenant_id: &str,
    ) -> Result<Vec<TraceAuditEventRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT
                    tenant_id, audit_event_id, actor_principal_ref, actor_role, action, reason,
                    request_id, submission_id, object_ref_id, export_manifest_id,
                    decision_inputs_hash, metadata_json, occurred_at
                 FROM trace_audit_events
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
             ON CONFLICT (tenant_id, submission_id) DO UPDATE SET
                tombstone_id = excluded.tombstone_id,
                trace_id = excluded.trace_id,
                redaction_hash = excluded.redaction_hash,
                canonical_summary_hash = excluded.canonical_summary_hash,
                reason = excluded.reason,
                effective_at = excluded.effective_at,
                retain_until = excluded.retain_until,
                created_by_principal_ref = excluded.created_by_principal_ref",
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
}
