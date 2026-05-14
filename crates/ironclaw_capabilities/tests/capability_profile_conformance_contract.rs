use ironclaw_capabilities::{
    CapabilityProfileClaim, CapabilityProfileClaimedOperation, CapabilityProfileConformanceFinding,
    CapabilityProfileConformanceFindingKind, CapabilityProfileConformanceReport,
    evaluate_profile_conformance,
};
use ironclaw_host_api::{
    CapabilityId, CapabilityProfileContract, CapabilityProfileId,
    CapabilityProfileOperationContract, CapabilityProfileOperationId,
};

#[test]
fn capability_profile_conformance_reports_missing_required_operations() {
    let contract = context_retrieval_contract();
    let claim = CapabilityProfileClaim::new(
        CapabilityId::new("ironclaw.memory.native.context.retrieve").unwrap(),
        CapabilityProfileId::new("memory.context_retrieval.v1").unwrap(),
        Vec::new(),
    )
    .unwrap();

    let report = CapabilityProfileConformanceReport::evaluate(&contract, &claim);

    assert!(!report.is_conformant());
    assert_eq!(
        report.findings(),
        &[CapabilityProfileConformanceFinding::new(
            CapabilityProfileConformanceFindingKind::MissingRequiredOperation,
            "memory.context.retrieve.v1",
        )]
    );
}

#[test]
fn capability_profile_conformance_reports_schema_mismatches_and_extra_operations() {
    let contract = context_retrieval_contract();
    let claim = CapabilityProfileClaim::new(
        CapabilityId::new("ironclaw.memory.native.context.retrieve").unwrap(),
        CapabilityProfileId::new("memory.context_retrieval.v1").unwrap(),
        vec![
            CapabilityProfileClaimedOperation::new(
                CapabilityProfileOperationId::new("memory.context.retrieve.v1").unwrap(),
                "schemas/memory/wrong.input.v1.json",
                "schemas/memory/context-retrieve.output.v1.json",
            )
            .unwrap(),
            CapabilityProfileClaimedOperation::new(
                CapabilityProfileOperationId::new("memory.context.extra.v1").unwrap(),
                "schemas/memory/extra.input.v1.json",
                "schemas/memory/extra.output.v1.json",
            )
            .unwrap(),
        ],
    )
    .unwrap();

    let report = CapabilityProfileConformanceReport::evaluate(&contract, &claim);

    assert!(!report.is_conformant());
    assert_eq!(
        report.findings(),
        &[
            CapabilityProfileConformanceFinding::new(
                CapabilityProfileConformanceFindingKind::InputSchemaRefMismatch,
                "memory.context.retrieve.v1",
            ),
            CapabilityProfileConformanceFinding::new(
                CapabilityProfileConformanceFindingKind::UnexpectedOperation,
                "memory.context.extra.v1",
            ),
        ]
    );
}

#[test]
fn capability_profile_conformance_accepts_matching_claims() {
    let contract = context_retrieval_contract();
    let claim = CapabilityProfileClaim::new(
        CapabilityId::new("ironclaw.memory.native.context.retrieve").unwrap(),
        CapabilityProfileId::new("memory.context_retrieval.v1").unwrap(),
        vec![
            CapabilityProfileClaimedOperation::new(
                CapabilityProfileOperationId::new("memory.context.retrieve.v1").unwrap(),
                "schemas/memory/context-retrieve.input.v1.json",
                "schemas/memory/context-retrieve.output.v1.json",
            )
            .unwrap(),
        ],
    )
    .unwrap();

    let report = evaluate_profile_conformance(&contract, &claim);
    let claim_method_report = claim.evaluate_against(&contract);

    assert!(report.is_conformant());
    assert!(report.findings().is_empty());
    assert_eq!(report, claim_method_report);
}

#[test]
fn capability_profile_conformance_finding_renders_for_logs() {
    let finding = CapabilityProfileConformanceFinding::new(
        CapabilityProfileConformanceFindingKind::MissingRequiredOperation,
        "memory.context.retrieve.v1",
    );

    assert_eq!(
        finding.to_string(),
        "missing required operation: memory.context.retrieve.v1"
    );
}

#[test]
fn capability_profile_conformance_reports_profile_id_mismatch_with_profile_subject() {
    let contract = context_retrieval_contract();
    let mismatched_profile = CapabilityProfileId::new("memory.document_store.v1").unwrap();
    let claim = CapabilityProfileClaim::new(
        CapabilityId::new("ironclaw.memory.native.context.retrieve").unwrap(),
        mismatched_profile.clone(),
        vec![
            CapabilityProfileClaimedOperation::new(
                CapabilityProfileOperationId::new("memory.context.retrieve.v1").unwrap(),
                "schemas/memory/context-retrieve.input.v1.json",
                "schemas/memory/context-retrieve.output.v1.json",
            )
            .unwrap(),
        ],
    )
    .unwrap();

    let report = CapabilityProfileConformanceReport::evaluate(&contract, &claim);

    assert!(!report.is_conformant());
    let first = report
        .findings()
        .first()
        .expect("profile_id mismatch must produce a finding");
    assert_eq!(
        first.kind(),
        CapabilityProfileConformanceFindingKind::ProfileIdMismatch,
    );
    assert_eq!(first.subject(), mismatched_profile.as_str());
}

fn context_retrieval_contract() -> CapabilityProfileContract {
    CapabilityProfileContract::new(
        CapabilityProfileId::new("memory.context_retrieval.v1").unwrap(),
        vec![
            CapabilityProfileOperationContract::new(
                CapabilityProfileOperationId::new("memory.context.retrieve.v1").unwrap(),
                "schemas/memory/context-retrieve.input.v1.json",
                "schemas/memory/context-retrieve.output.v1.json",
            )
            .unwrap(),
        ],
    )
    .unwrap()
}
