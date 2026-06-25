//! Repo conformance tests for the host-defined memory capability-profile
//! catalog (issue #3537).
//!
//! These drive the real `ironclaw_host_runtime::memory_profiles` catalog through
//! the `ironclaw_capabilities` conformance harness (semantic conformance, not
//! just JSON shape): a claim that faithfully implements a profile's operations
//! with matching schema refs conforms, and a claim missing a required operation
//! does not. The native `ironclaw.memory.native` manifest's claims are validated
//! against this same catalog in the native-manifest conformance test.

use ironclaw_capabilities::{
    CapabilityProfileClaim, CapabilityProfileClaimedOperation,
    CapabilityProfileConformanceFindingKind, evaluate_profile_conformance,
};
use ironclaw_host_api::{CapabilityId, CapabilityProfileContract};
use ironclaw_host_runtime::memory_profiles::{
    MEMORY_CONTEXT_RETRIEVAL_PROFILE_ID, MEMORY_DOCUMENT_STORE_PROFILE_ID,
    MEMORY_INTERACTION_LOG_PROFILE_ID, context_retrieval_profile, document_store_profile,
    interaction_log_profile, memory_capability_profiles,
};

/// Build a claim that faithfully implements every required operation of
/// `contract`, mirroring each operation's id + schema refs.
fn faithful_claim(
    contract: &CapabilityProfileContract,
    capability_id: &str,
) -> CapabilityProfileClaim {
    let operations = contract
        .required_operations()
        .iter()
        .map(|op| {
            CapabilityProfileClaimedOperation::new(
                op.id().clone(),
                op.input_schema_ref().as_str(),
                op.output_schema_ref().as_str(),
            )
            .expect("claimed operation must build from a valid contract operation")
        })
        .collect();
    CapabilityProfileClaim::new(
        CapabilityId::new(capability_id).expect("capability id"),
        contract.id().clone(),
        operations,
    )
    .expect("claim must build")
}

#[test]
fn every_memory_profile_in_the_catalog_accepts_a_faithful_claim() {
    for contract in memory_capability_profiles().expect("catalog must build") {
        let claim = faithful_claim(&contract, "ironclaw.memory.native.example");
        let report = evaluate_profile_conformance(&contract, &claim);
        assert!(
            report.is_conformant(),
            "profile {} must accept a faithful claim, findings: {:?}",
            contract.id(),
            report.findings()
        );
        assert!(report.findings().is_empty());
    }
}

#[test]
fn context_retrieval_rejects_a_claim_missing_the_retrieve_operation() {
    let contract = context_retrieval_profile().expect("context retrieval profile");
    // A claim that implements no operations is missing the required one.
    let claim = CapabilityProfileClaim::new(
        CapabilityId::new("ironclaw.memory.native.context.retrieve").unwrap(),
        contract.id().clone(),
        Vec::new(),
    )
    .unwrap();

    let report = evaluate_profile_conformance(&contract, &claim);

    assert!(!report.is_conformant());
    assert!(report.findings().iter().any(|finding| {
        finding.kind() == CapabilityProfileConformanceFindingKind::MissingRequiredOperation
    }));
}

#[test]
fn document_store_rejects_a_claim_missing_the_write_operation() {
    let contract = document_store_profile().expect("document store profile");
    // Implement only the read operation; the write operation is required too.
    let read_only = contract
        .required_operations()
        .iter()
        .find(|op| op.id().as_str() == "memory.document.read.v1")
        .expect("document store must require read");
    let claim = CapabilityProfileClaim::new(
        CapabilityId::new("ironclaw.memory.native.document.read").unwrap(),
        contract.id().clone(),
        vec![
            CapabilityProfileClaimedOperation::new(
                read_only.id().clone(),
                read_only.input_schema_ref().as_str(),
                read_only.output_schema_ref().as_str(),
            )
            .unwrap(),
        ],
    )
    .unwrap();

    let report = evaluate_profile_conformance(&contract, &claim);

    assert!(!report.is_conformant());
    assert!(report.findings().iter().any(|finding| {
        finding.kind() == CapabilityProfileConformanceFindingKind::MissingRequiredOperation
            && finding.subject() == "memory.document.write.v1"
    }));
}

#[test]
fn interaction_log_detects_a_schema_ref_mismatch() {
    let contract = interaction_log_profile().expect("interaction log profile");
    let op = &contract.required_operations()[0];
    // Faithful operation id, but a wrong input schema ref.
    let claim = CapabilityProfileClaim::new(
        CapabilityId::new("ironclaw.memory.native.interaction.record").unwrap(),
        contract.id().clone(),
        vec![
            CapabilityProfileClaimedOperation::new(
                op.id().clone(),
                "schemas/memory/wrong.input.v1.json",
                op.output_schema_ref().as_str(),
            )
            .unwrap(),
        ],
    )
    .unwrap();

    let report = evaluate_profile_conformance(&contract, &claim);

    assert!(!report.is_conformant());
    assert!(report.findings().iter().any(|finding| {
        finding.kind() == CapabilityProfileConformanceFindingKind::InputSchemaRefMismatch
    }));
}

#[test]
fn catalog_ids_are_the_three_documented_profiles() {
    let ids: Vec<String> = memory_capability_profiles()
        .expect("catalog")
        .iter()
        .map(|c| c.id().as_str().to_string())
        .collect();
    assert_eq!(
        ids,
        vec![
            MEMORY_CONTEXT_RETRIEVAL_PROFILE_ID.to_string(),
            MEMORY_INTERACTION_LOG_PROFILE_ID.to_string(),
            MEMORY_DOCUMENT_STORE_PROFILE_ID.to_string(),
        ]
    );
}
