use ironclaw::trace_corpus_storage::{
    TenantScopedTraceObjectRef, TraceAuditSafeMetadata, TraceCorpusStatus, TraceObjectArtifactKind,
    TraceSubmissionWrite,
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
