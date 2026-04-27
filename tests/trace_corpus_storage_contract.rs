use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use ironclaw::trace_corpus_storage::{
    TenantScopedTraceObjectRef, TraceAuditSafeMetadata, TraceCorpusStatus, TraceCreditEventType,
    TraceDerivedRecord, TraceDerivedStatus, TraceExportManifestItemInvalidationReason,
    TraceExportManifestItemRecord, TraceExportManifestRecord, TraceObjectArtifactKind,
    TraceObjectRefRecord, TraceReviewLeaseAuditAction, TraceSubmissionWrite, TraceTombstoneRecord,
    TraceVectorEntryRecord, TraceVectorEntrySourceProjection, TraceVectorEntryStatus,
    TraceWorkerKind,
};
use uuid::Uuid;

#[test]
fn storage_contract_status_values_are_snake_case() {
    let status = serde_json::to_value(TraceCorpusStatus::Quarantined).unwrap();
    assert_eq!(status, "quarantined");

    let artifact = serde_json::to_value(TraceObjectArtifactKind::RescrubbedEnvelope).unwrap();
    assert_eq!(artifact, "rescrubbed_envelope");
}

#[test]
fn submission_write_keeps_auth_tenant_separate_from_envelope_scope() {
    let submission = TraceSubmissionWrite {
        tenant_id: "tenant-a".to_string(),
        submission_id: Uuid::nil(),
        trace_id: Uuid::nil(),
        auth_principal_ref: "principal-hash".to_string(),
        contributor_pseudonym: Some("contributor-1".to_string()),
        submitted_tenant_scope_ref: Some("envelope-scope".to_string()),
        schema_version: "ironclaw.trace_contribution.v1".to_string(),
        consent_policy_version: "2026-04-24".to_string(),
        consent_scopes: vec!["debugging_evaluation".to_string()],
        allowed_uses: vec!["debugging".to_string(), "evaluation".to_string()],
        retention_policy_id: "private_corpus_revocable".to_string(),
        status: TraceCorpusStatus::Accepted,
        privacy_risk: "low".to_string(),
        redaction_pipeline_version: "server-rescrub-v1".to_string(),
        redaction_counts: Default::default(),
        redaction_hash: "redaction-hash".to_string(),
        canonical_summary_hash: Some("summary-hash".to_string()),
        submission_score: Some(0.75),
        credit_points_pending: Some(1.25),
        credit_points_final: None,
        expires_at: None,
    };

    assert_eq!(submission.tenant_id, "tenant-a");
    assert_eq!(
        submission.submitted_tenant_scope_ref.as_deref(),
        Some("envelope-scope")
    );
}

#[test]
fn tenant_scoped_object_refs_carry_submission_and_tenant() {
    let object_ref = TenantScopedTraceObjectRef {
        tenant_id: "tenant-a".to_string(),
        submission_id: Uuid::nil(),
        object_ref_id: Uuid::nil(),
    };

    assert_eq!(object_ref.tenant_id, "tenant-a");
    assert_eq!(object_ref.submission_id, Uuid::nil());
}

#[test]
fn audit_metadata_contract_is_typed_not_arbitrary_json() {
    let metadata = TraceAuditSafeMetadata::Export {
        artifact_kind: TraceObjectArtifactKind::ExportArtifact,
        purpose_code: Some("ranker_training".to_string()),
        item_count: 3,
    };
    let json = serde_json::to_value(metadata).unwrap();

    assert_eq!(json["kind"], "export");
    assert_eq!(json["artifact_kind"], "export_artifact");
    assert!(json.get("request_body").is_none());
    assert!(json.get("tool_payload").is_none());
}

#[test]
fn review_lease_audit_metadata_is_typed_and_request_safe() {
    let metadata = TraceAuditSafeMetadata::ReviewLease {
        action: TraceReviewLeaseAuditAction::Claim,
        lease_expires_at: None,
        review_due_at: None,
    };
    let json = serde_json::to_value(metadata).unwrap();

    assert_eq!(json["kind"], "review_lease");
    assert_eq!(json["action"], "claim");
    assert!(json.get("request_body").is_none());
    assert!(json.get("bearer_token").is_none());
    assert!(json.get("reviewer_email").is_none());
}

#[test]
fn credit_mutation_audit_metadata_hashes_sensitive_refs() {
    let metadata = TraceAuditSafeMetadata::CreditMutation {
        event_type: TraceCreditEventType::RankingUtility,
        credit_points_delta_micros: 1_250_000,
        reason_hash: "sha256:reason".to_string(),
        external_ref_hash: Some("sha256:artifact-ref".to_string()),
    };
    let json = serde_json::to_value(metadata).unwrap();

    assert_eq!(json["kind"], "credit_mutation");
    assert_eq!(json["event_type"], "ranking_utility");
    assert_eq!(json["credit_points_delta_micros"], 1_250_000);
    assert_eq!(json["external_ref_hash"], "sha256:artifact-ref");
    assert!(json.get("reason").is_none());
    assert!(json.get("external_ref").is_none());
}

#[test]
fn process_evaluation_audit_metadata_is_hash_and_count_only() {
    let metadata = TraceAuditSafeMetadata::ProcessEvaluation {
        evaluator_version_hash: "sha256:evaluator-version".to_string(),
        label_count: 2,
        rating_counts: BTreeMap::from([("pass".to_string(), 2), ("partial".to_string(), 1)]),
        score_band: Some("high".to_string()),
        utility_credit_delta_micros: Some(500_000),
        utility_external_ref_hash: Some("sha256:process-eval-job".to_string()),
    };
    let json = serde_json::to_value(metadata).unwrap();

    assert_eq!(json["kind"], "process_evaluation");
    assert_eq!(json["evaluator_version_hash"], "sha256:evaluator-version");
    assert_eq!(json["label_count"], 2);
    assert_eq!(json["rating_counts"]["pass"], 2);
    assert_eq!(json["score_band"], "high");
    assert_eq!(json["utility_credit_delta_micros"], 500_000);
    assert_eq!(json["utility_external_ref_hash"], "sha256:process-eval-job");
    assert!(json.get("evaluator_version").is_none());
    assert!(json.get("labels").is_none());
    assert!(json.get("utility_external_ref").is_none());
}

#[test]
fn revocation_invalidation_contract_is_tenant_scoped_across_artifact_rows() {
    let tenant_id = "tenant-alpha";
    let other_tenant_id = "tenant-beta";
    let submission_id = Uuid::from_u128(0x10);
    let trace_id = Uuid::from_u128(0x20);
    let invalidated_at = contract_time("2026-04-25T12:30:00Z");

    let target_object_ref_id = Uuid::from_u128(0x30);
    let other_object_ref_id = Uuid::from_u128(0x31);
    let mut object_refs = vec![
        object_ref_record(tenant_id, submission_id, target_object_ref_id),
        object_ref_record(other_tenant_id, submission_id, other_object_ref_id),
    ];
    for object_ref in &mut object_refs {
        if matches_submission_scope(
            object_ref.tenant_id.as_str(),
            object_ref.submission_id,
            tenant_id,
            submission_id,
        ) {
            object_ref.invalidated_at = Some(invalidated_at);
        }
    }
    assert_eq!(
        invalidated_object_ref_ids(&object_refs),
        vec![target_object_ref_id]
    );
    assert!(
        object_refs
            .iter()
            .filter(|object_ref| object_ref.tenant_id == other_tenant_id)
            .all(|object_ref| object_ref.invalidated_at.is_none())
    );

    let target_derived_id = Uuid::from_u128(0x40);
    let other_derived_id = Uuid::from_u128(0x41);
    let mut derived_rows = vec![
        derived_record(
            tenant_id,
            submission_id,
            trace_id,
            target_derived_id,
            target_object_ref_id,
        ),
        derived_record(
            other_tenant_id,
            submission_id,
            trace_id,
            other_derived_id,
            other_object_ref_id,
        ),
    ];
    for derived in &mut derived_rows {
        if matches_submission_scope(
            derived.tenant_id.as_str(),
            derived.submission_id,
            tenant_id,
            submission_id,
        ) {
            derived.status = TraceDerivedStatus::Revoked;
        }
    }
    assert_eq!(revoked_derived_ids(&derived_rows), vec![target_derived_id]);
    assert!(
        derived_rows
            .iter()
            .filter(|derived| derived.tenant_id == other_tenant_id)
            .all(|derived| derived.status == TraceDerivedStatus::Current)
    );

    let target_vector_entry_id = Uuid::from_u128(0x50);
    let other_vector_entry_id = Uuid::from_u128(0x51);
    let mut vector_entries = vec![
        vector_entry_record(
            tenant_id,
            submission_id,
            target_derived_id,
            target_vector_entry_id,
        ),
        vector_entry_record(
            other_tenant_id,
            submission_id,
            other_derived_id,
            other_vector_entry_id,
        ),
    ];
    for vector_entry in &mut vector_entries {
        if matches_submission_scope(
            vector_entry.tenant_id.as_str(),
            vector_entry.submission_id,
            tenant_id,
            submission_id,
        ) {
            vector_entry.status = TraceVectorEntryStatus::Invalidated;
            vector_entry.invalidated_at = Some(invalidated_at);
        }
    }
    assert_eq!(
        invalidated_vector_entry_ids(&vector_entries),
        vec![target_vector_entry_id]
    );
    assert!(
        vector_entries
            .iter()
            .filter(|vector_entry| vector_entry.tenant_id == other_tenant_id)
            .all(|vector_entry| {
                vector_entry.status == TraceVectorEntryStatus::Active
                    && vector_entry.invalidated_at.is_none()
            })
    );

    let target_manifest_id = Uuid::from_u128(0x60);
    let other_manifest_id = Uuid::from_u128(0x61);
    let mut export_manifests = vec![
        export_manifest_record(tenant_id, target_manifest_id, vec![submission_id]),
        export_manifest_record(other_tenant_id, other_manifest_id, vec![submission_id]),
    ];
    for manifest in &mut export_manifests {
        if manifest.tenant_id == tenant_id
            && manifest.source_submission_ids.contains(&submission_id)
        {
            manifest.invalidated_at = Some(invalidated_at);
        }
    }
    assert_eq!(
        invalidated_manifest_ids(&export_manifests),
        vec![target_manifest_id]
    );
    assert!(
        export_manifests
            .iter()
            .filter(|manifest| manifest.tenant_id == other_tenant_id)
            .all(|manifest| manifest.invalidated_at.is_none())
    );

    let mut export_items = vec![
        export_manifest_item_record(
            tenant_id,
            target_manifest_id,
            submission_id,
            trace_id,
            Some(target_derived_id),
            Some(target_object_ref_id),
            Some(target_vector_entry_id),
        ),
        export_manifest_item_record(
            other_tenant_id,
            other_manifest_id,
            submission_id,
            trace_id,
            Some(other_derived_id),
            Some(other_object_ref_id),
            Some(other_vector_entry_id),
        ),
    ];
    for item in &mut export_items {
        if matches_submission_scope(
            item.tenant_id.as_str(),
            item.submission_id,
            tenant_id,
            submission_id,
        ) {
            item.source_invalidated_at = Some(invalidated_at);
            item.source_invalidation_reason =
                Some(TraceExportManifestItemInvalidationReason::Revoked);
        }
    }
    assert_eq!(
        revoked_export_item_manifest_ids(&export_items),
        vec![target_manifest_id]
    );
    assert!(
        export_items
            .iter()
            .filter(|item| item.tenant_id == other_tenant_id)
            .all(|item| {
                item.source_invalidated_at.is_none() && item.source_invalidation_reason.is_none()
            })
    );

    let tombstones = [
        tombstone_record(tenant_id, submission_id, trace_id),
        tombstone_record(other_tenant_id, submission_id, trace_id),
    ];
    let tenant_tombstones: Vec<_> = tombstones
        .iter()
        .filter(|tombstone| tombstone.tenant_id == tenant_id)
        .collect();
    assert_eq!(tenant_tombstones.len(), 1);
    assert_eq!(tenant_tombstones[0].submission_id, submission_id);
    assert_eq!(tenant_tombstones[0].trace_id, Some(trace_id));
    assert!(
        tombstones
            .iter()
            .filter(|tombstone| tombstone.tenant_id == other_tenant_id)
            .all(|tombstone| tombstone.reason == "revocation requested")
    );
}

fn matches_submission_scope(
    row_tenant_id: &str,
    row_submission_id: Uuid,
    tenant_id: &str,
    submission_id: Uuid,
) -> bool {
    row_tenant_id == tenant_id && row_submission_id == submission_id
}

fn contract_time(timestamp: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(timestamp)
        .unwrap()
        .with_timezone(&Utc)
}

fn object_ref_record(
    tenant_id: &str,
    submission_id: Uuid,
    object_ref_id: Uuid,
) -> TraceObjectRefRecord {
    TraceObjectRefRecord {
        tenant_id: tenant_id.to_string(),
        submission_id,
        object_ref_id,
        artifact_kind: TraceObjectArtifactKind::SubmittedEnvelope,
        object_store: "s3://private-corpus".to_string(),
        object_key: format!("{tenant_id}/{submission_id}/submitted.json"),
        content_sha256: format!("sha256:{tenant_id}:submitted"),
        encryption_key_ref: format!("kms:{tenant_id}"),
        size_bytes: 512,
        compression: None,
        created_by_job_id: None,
        invalidated_at: None,
        deleted_at: None,
        updated_at: contract_time("2026-04-25T12:00:00Z"),
        created_at: contract_time("2026-04-25T12:00:00Z"),
    }
}

fn derived_record(
    tenant_id: &str,
    submission_id: Uuid,
    trace_id: Uuid,
    derived_id: Uuid,
    input_object_ref_id: Uuid,
) -> TraceDerivedRecord {
    TraceDerivedRecord {
        derived_id,
        tenant_id: tenant_id.to_string(),
        submission_id,
        trace_id,
        status: TraceDerivedStatus::Current,
        worker_kind: TraceWorkerKind::Summary,
        worker_version: "summary-worker-v1".to_string(),
        input_object_ref: Some(TenantScopedTraceObjectRef {
            tenant_id: tenant_id.to_string(),
            submission_id,
            object_ref_id: input_object_ref_id,
        }),
        input_hash: format!("sha256:{tenant_id}:input"),
        output_object_ref: None,
        canonical_summary: Some(format!("{tenant_id} summary")),
        canonical_summary_hash: Some(format!("sha256:{tenant_id}:summary")),
        summary_model: "summary-model-v1".to_string(),
        task_success: Some("success".to_string()),
        privacy_risk: Some("low".to_string()),
        event_count: Some(2),
        tool_sequence: vec!["memory_search".to_string()],
        tool_categories: vec!["memory".to_string()],
        coverage_tags: vec!["tool:memory_search".to_string()],
        duplicate_score: Some(0.1),
        novelty_score: Some(0.4),
        cluster_id: Some(format!("cluster:{tenant_id}")),
        created_at: contract_time("2026-04-25T12:00:00Z"),
        updated_at: contract_time("2026-04-25T12:00:00Z"),
    }
}

fn vector_entry_record(
    tenant_id: &str,
    submission_id: Uuid,
    derived_id: Uuid,
    vector_entry_id: Uuid,
) -> TraceVectorEntryRecord {
    TraceVectorEntryRecord {
        tenant_id: tenant_id.to_string(),
        submission_id,
        derived_id,
        vector_entry_id,
        vector_store: "trace-commons-main".to_string(),
        embedding_model: "text-embedding-3-small".to_string(),
        embedding_dimension: 1536,
        embedding_version: "embedding-v1".to_string(),
        source_projection: TraceVectorEntrySourceProjection::CanonicalSummary,
        source_hash: format!("sha256:{tenant_id}:summary"),
        status: TraceVectorEntryStatus::Active,
        nearest_trace_ids: Vec::new(),
        cluster_id: Some(format!("cluster:{tenant_id}")),
        duplicate_score: Some(0.1),
        novelty_score: Some(0.4),
        indexed_at: Some(contract_time("2026-04-25T12:10:00Z")),
        invalidated_at: None,
        deleted_at: None,
        created_at: contract_time("2026-04-25T12:00:00Z"),
        updated_at: contract_time("2026-04-25T12:00:00Z"),
    }
}

fn export_manifest_record(
    tenant_id: &str,
    export_manifest_id: Uuid,
    source_submission_ids: Vec<Uuid>,
) -> TraceExportManifestRecord {
    TraceExportManifestRecord {
        tenant_id: tenant_id.to_string(),
        export_manifest_id,
        artifact_kind: TraceObjectArtifactKind::ExportArtifact,
        purpose_code: Some("replay_dataset".to_string()),
        audit_event_id: Some(Uuid::from_u128(0x70)),
        source_submission_ids,
        source_submission_ids_hash: format!("sha256:{tenant_id}:sources"),
        item_count: 1,
        generated_at: contract_time("2026-04-25T12:15:00Z"),
        invalidated_at: None,
        deleted_at: None,
        created_at: contract_time("2026-04-25T12:15:00Z"),
        updated_at: contract_time("2026-04-25T12:15:00Z"),
    }
}

fn export_manifest_item_record(
    tenant_id: &str,
    export_manifest_id: Uuid,
    submission_id: Uuid,
    trace_id: Uuid,
    derived_id: Option<Uuid>,
    object_ref_id: Option<Uuid>,
    vector_entry_id: Option<Uuid>,
) -> TraceExportManifestItemRecord {
    TraceExportManifestItemRecord {
        tenant_id: tenant_id.to_string(),
        export_manifest_id,
        submission_id,
        trace_id,
        derived_id,
        object_ref_id,
        vector_entry_id,
        source_status_at_export: TraceCorpusStatus::Accepted,
        source_hash_at_export: format!("sha256:{tenant_id}:source"),
        source_invalidated_at: None,
        source_invalidation_reason: None,
        created_at: contract_time("2026-04-25T12:15:00Z"),
        updated_at: contract_time("2026-04-25T12:15:00Z"),
    }
}

fn tombstone_record(tenant_id: &str, submission_id: Uuid, trace_id: Uuid) -> TraceTombstoneRecord {
    TraceTombstoneRecord {
        tombstone_id: Uuid::from_u128(if tenant_id == "tenant-alpha" {
            0x80
        } else {
            0x81
        }),
        tenant_id: tenant_id.to_string(),
        submission_id,
        trace_id: Some(trace_id),
        redaction_hash: Some(format!("sha256:{tenant_id}:redaction")),
        canonical_summary_hash: Some(format!("sha256:{tenant_id}:summary")),
        reason: "revocation requested".to_string(),
        effective_at: contract_time("2026-04-25T12:30:00Z"),
        retain_until: Some(contract_time("2026-05-25T12:30:00Z")),
        created_by_principal_ref: "principal:test-user".to_string(),
        created_at: contract_time("2026-04-25T12:30:00Z"),
    }
}

fn invalidated_object_ref_ids(object_refs: &[TraceObjectRefRecord]) -> Vec<Uuid> {
    object_refs
        .iter()
        .filter(|object_ref| object_ref.invalidated_at.is_some())
        .map(|object_ref| object_ref.object_ref_id)
        .collect()
}

fn revoked_derived_ids(derived_rows: &[TraceDerivedRecord]) -> Vec<Uuid> {
    derived_rows
        .iter()
        .filter(|derived| derived.status == TraceDerivedStatus::Revoked)
        .map(|derived| derived.derived_id)
        .collect()
}

fn invalidated_vector_entry_ids(vector_entries: &[TraceVectorEntryRecord]) -> Vec<Uuid> {
    vector_entries
        .iter()
        .filter(|vector_entry| vector_entry.status == TraceVectorEntryStatus::Invalidated)
        .map(|vector_entry| vector_entry.vector_entry_id)
        .collect()
}

fn invalidated_manifest_ids(manifests: &[TraceExportManifestRecord]) -> Vec<Uuid> {
    manifests
        .iter()
        .filter(|manifest| manifest.invalidated_at.is_some())
        .map(|manifest| manifest.export_manifest_id)
        .collect()
}

fn revoked_export_item_manifest_ids(items: &[TraceExportManifestItemRecord]) -> Vec<Uuid> {
    items
        .iter()
        .filter(|item| {
            item.source_invalidation_reason
                == Some(TraceExportManifestItemInvalidationReason::Revoked)
        })
        .map(|item| item.export_manifest_id)
        .collect()
}
