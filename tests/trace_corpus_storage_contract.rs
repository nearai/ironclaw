use std::collections::BTreeMap;

use ironclaw::trace_corpus_storage::{
    TenantScopedTraceObjectRef, TraceAuditSafeMetadata, TraceCorpusStatus, TraceCreditEventType,
    TraceObjectArtifactKind, TraceSubmissionWrite,
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
