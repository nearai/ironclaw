use std::collections::BTreeMap;

use async_trait::async_trait;
use tokio_postgres::Row;
use uuid::Uuid;

use crate::db::postgres::PgBackend;
use crate::db::trace_corpus_common::{audit_action_for_status, enum_from_storage, enum_to_storage};
use crate::error::DatabaseError;
use crate::trace_corpus_storage::{
    TenantScopedTraceObjectRef, TraceArtifactInvalidationCounts, TraceAuditEventWrite,
    TraceAuditSafeMetadata, TraceCorpusStatus, TraceCorpusStore, TraceCreditEventRecord,
    TraceCreditEventWrite, TraceCreditSettlementState, TraceDerivedRecord, TraceDerivedRecordWrite,
    TraceDerivedStatus, TraceObjectRefWrite, TraceSubmissionRecord, TraceSubmissionWrite,
    TraceTombstoneWrite, TraceWorkerKind,
};

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

impl PgBackend {
    async fn ensure_trace_tenant(&self, tenant_id: &str) -> Result<(), DatabaseError> {
        let client = self.pool().get().await?;
        client
            .execute(
                "INSERT INTO trace_tenants (tenant_id) VALUES ($1)
                 ON CONFLICT (tenant_id) DO UPDATE SET updated_at = NOW()",
                &[&tenant_id],
            )
            .await
            .map_err(DatabaseError::Postgres)?;
        Ok(())
    }
}

#[async_trait]
impl TraceCorpusStore for PgBackend {
    async fn upsert_trace_submission(
        &self,
        submission: TraceSubmissionWrite,
    ) -> Result<TraceSubmissionRecord, DatabaseError> {
        self.ensure_trace_tenant(&submission.tenant_id).await?;
        let client = self.pool().get().await?;
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

        let row = client
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
        row_to_submission(&row)
    }

    async fn get_trace_submission(
        &self,
        tenant_id: &str,
        submission_id: Uuid,
    ) -> Result<Option<TraceSubmissionRecord>, DatabaseError> {
        let client = self.pool().get().await?;
        let row = client
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
        row.as_ref().map(row_to_submission).transpose()
    }

    async fn list_trace_submissions(
        &self,
        tenant_id: &str,
    ) -> Result<Vec<TraceSubmissionRecord>, DatabaseError> {
        let client = self.pool().get().await?;
        let rows = client
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
        rows.iter().map(row_to_submission).collect()
    }

    async fn list_trace_credit_events(
        &self,
        tenant_id: &str,
    ) -> Result<Vec<TraceCreditEventRecord>, DatabaseError> {
        let client = self.pool().get().await?;
        let rows = client
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
        rows.iter().map(row_to_credit_event).collect()
    }

    async fn update_trace_submission_status(
        &self,
        tenant_id: &str,
        submission_id: Uuid,
        status: TraceCorpusStatus,
        actor_principal_ref: &str,
        reason: Option<&str>,
    ) -> Result<(), DatabaseError> {
        let client = self.pool().get().await?;
        let status_value = enum_to_storage(status)?;
        let updated = client
            .execute(
                "UPDATE trace_submissions
                 SET status = $3,
                     updated_at = NOW(),
                     reviewed_at = CASE
                         WHEN $3 IN ('accepted', 'quarantined', 'rejected') THEN NOW()
                         ELSE reviewed_at
                     END,
                     revoked_at = CASE WHEN $3 = 'revoked' THEN NOW() ELSE revoked_at END,
                     purged_at = CASE WHEN $3 = 'purged' THEN NOW() ELSE purged_at END
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
        let client = self.pool().get().await?;
        let artifact_kind = enum_to_storage(object_ref.artifact_kind)?;
        client
            .execute(
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
        Ok(())
    }

    async fn append_trace_derived_record(
        &self,
        derived_record: TraceDerivedRecordWrite,
    ) -> Result<(), DatabaseError> {
        self.ensure_trace_tenant(&derived_record.tenant_id).await?;
        let client = self.pool().get().await?;
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

        client
            .execute(
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
        Ok(())
    }

    async fn list_trace_derived_records(
        &self,
        tenant_id: &str,
    ) -> Result<Vec<TraceDerivedRecord>, DatabaseError> {
        let client = self.pool().get().await?;
        let rows = client
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
        rows.iter().map(row_to_derived_record).collect()
    }

    async fn append_trace_audit_event(
        &self,
        audit_event: TraceAuditEventWrite,
    ) -> Result<(), DatabaseError> {
        self.ensure_trace_tenant(&audit_event.tenant_id).await?;
        let client = self.pool().get().await?;
        let action = enum_to_storage(audit_event.action)?;
        let metadata_json = serde_json::to_value(&audit_event.metadata).map_err(|e| {
            DatabaseError::Serialization(format!("trace audit metadata encode failed: {e}"))
        })?;
        client
            .execute(
                "INSERT INTO trace_audit_events (
                    tenant_id, audit_event_id, actor_principal_ref, actor_role, action,
                    reason, request_id, submission_id, object_ref_id, export_manifest_id,
                    decision_inputs_hash, metadata_json
                 ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
                &[
                    &audit_event.tenant_id,
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
                    &metadata_json,
                ],
            )
            .await
            .map_err(DatabaseError::Postgres)?;
        Ok(())
    }

    async fn append_trace_credit_event(
        &self,
        credit_event: TraceCreditEventWrite,
    ) -> Result<(), DatabaseError> {
        self.ensure_trace_tenant(&credit_event.tenant_id).await?;
        let client = self.pool().get().await?;
        let event_type = enum_to_storage(credit_event.event_type)?;
        let settlement_state = enum_to_storage(credit_event.settlement_state)?;
        client
            .execute(
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
        Ok(())
    }

    async fn write_trace_tombstone(
        &self,
        tombstone: TraceTombstoneWrite,
    ) -> Result<(), DatabaseError> {
        self.ensure_trace_tenant(&tombstone.tenant_id).await?;
        let client = self.pool().get().await?;
        client
            .execute(
                "INSERT INTO trace_tombstones (
                    tenant_id, tombstone_id, submission_id, trace_id, redaction_hash,
                    canonical_summary_hash, reason, effective_at, retain_until,
                    created_by_principal_ref
                 ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
                 ON CONFLICT (tenant_id, submission_id) DO UPDATE SET
                    tombstone_id = excluded.tombstone_id,
                    trace_id = excluded.trace_id,
                    redaction_hash = excluded.redaction_hash,
                    canonical_summary_hash = excluded.canonical_summary_hash,
                    reason = excluded.reason,
                    effective_at = excluded.effective_at,
                    retain_until = excluded.retain_until,
                    created_by_principal_ref = excluded.created_by_principal_ref",
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
        Ok(())
    }

    async fn invalidate_trace_submission_artifacts(
        &self,
        tenant_id: &str,
        submission_id: Uuid,
        derived_status: TraceDerivedStatus,
    ) -> Result<TraceArtifactInvalidationCounts, DatabaseError> {
        let client = self.pool().get().await?;
        let derived_status = enum_to_storage(derived_status)?;
        let object_refs_invalidated = client
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
        let derived_records_invalidated = client
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
        Ok(TraceArtifactInvalidationCounts {
            object_refs_invalidated,
            derived_records_invalidated,
        })
    }
}
