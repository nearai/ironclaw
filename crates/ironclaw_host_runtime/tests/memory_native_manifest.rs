//! Repo conformance tests for the bundled `ironclaw.memory.native` manifest
//! (issue #3537).
//!
//! These prove the native extension's declared capabilities semantically satisfy
//! the host-defined memory capability-profile catalog — not just that the TOML
//! parses. For each profile, the native capabilities that `implements` it are
//! aggregated into a profile claim (each capability mapped to the profile
//! operation whose schema refs it mirrors) and evaluated through the
//! `ironclaw_capabilities` conformance harness.

use std::collections::BTreeMap;

use ironclaw_capabilities::{
    CapabilityProfileClaim, CapabilityProfileClaimedOperation, evaluate_profile_conformance,
};
use ironclaw_host_api::{CapabilityProfileContract, CapabilityProfileId};
use ironclaw_host_runtime::memory_native_extension::native_memory_manifest;
use ironclaw_host_runtime::memory_profiles::{
    MEMORY_CAPABILITY_PROFILE_IDS, memory_capability_profiles,
};

/// Map each native capability that implements `contract`'s profile to the
/// profile operation whose input+output schema refs it mirrors, and aggregate
/// them into a single claim for the profile.
fn aggregate_native_claim(contract: &CapabilityProfileContract) -> CapabilityProfileClaim {
    let manifest = native_memory_manifest().expect("native memory manifest must parse");
    let profile_id = contract.id();

    let mut claim_capability_id = None;
    let mut claimed_operations = Vec::new();

    for capability in &manifest.capabilities {
        if !capability.implements.contains(profile_id) {
            continue;
        }
        claim_capability_id.get_or_insert_with(|| capability.id.clone());

        // Schema-driven: the operation this capability provides is the contract
        // operation whose schema refs match the capability's own refs.
        let operation = contract
            .required_operations()
            .iter()
            .find(|op| {
                op.input_schema_ref().as_str() == capability.input_schema_ref.as_str()
                    && op.output_schema_ref().as_str() == capability.output_schema_ref.as_str()
            })
            .unwrap_or_else(|| {
                panic!(
                    "capability {} implements {} but its schema refs match no required operation",
                    capability.id, profile_id
                )
            });

        claimed_operations.push(
            CapabilityProfileClaimedOperation::new(
                operation.id().clone(),
                capability.input_schema_ref.as_str(),
                capability.output_schema_ref.as_str(),
            )
            .expect("claimed operation must build"),
        );
    }

    let capability_id = claim_capability_id
        .unwrap_or_else(|| panic!("no native capability implements profile {profile_id}"));

    CapabilityProfileClaim::new(capability_id, profile_id.clone(), claimed_operations)
        .expect("claim must build")
}

/// The host-defined `memory.document_store.v1` contract from the catalog.
fn document_store_contract() -> CapabilityProfileContract {
    memory_capability_profiles()
        .expect("catalog must build")
        .into_iter()
        .find(|contract| contract.id().as_str() == "memory.document_store.v1")
        .expect("document_store profile must exist in the catalog")
}

#[test]
fn native_extension_conforms_to_the_document_store_profile() {
    // Lean scope: the live native surface implements only
    // `memory.document_store.v1` (via the `read` + `write` model tools). The
    // `context_retrieval` / `interaction_log` profiles remain host-defined
    // contracts for the deferred host-managed flow, with no live implementer, so
    // they are not claimed here.
    let contract = document_store_contract();
    let claim = aggregate_native_claim(&contract);
    let report = evaluate_profile_conformance(&contract, &claim);
    assert!(
        report.is_conformant(),
        "native extension must conform to {}; findings: {:?}",
        contract.id(),
        report.findings()
    );
    assert!(report.findings().is_empty());
}

#[test]
fn every_implemented_profile_is_a_known_memory_profile() {
    let manifest = native_memory_manifest().expect("manifest");
    let known: Vec<CapabilityProfileId> = MEMORY_CAPABILITY_PROFILE_IDS
        .iter()
        .map(|id| CapabilityProfileId::new(*id).unwrap())
        .collect();
    // `search`/`tree` are native conveniences that implement no portable profile,
    // so an empty `implements` is allowed; only the profiles that ARE implemented
    // must be known host-defined memory profiles.
    for capability in &manifest.capabilities {
        for profile in &capability.implements {
            assert!(
                known.contains(profile),
                "{} implements unknown profile {}",
                capability.id,
                profile
            );
        }
    }
    // The live surface must implement document_store (read + write).
    let document_store = CapabilityProfileId::new("memory.document_store.v1").unwrap();
    assert!(
        manifest
            .capabilities
            .iter()
            .any(|capability| capability.implements.contains(&document_store)),
        "the native extension must implement memory.document_store.v1"
    );
}

#[test]
fn document_store_profile_is_satisfied_by_two_distinct_capabilities() {
    // memory.document_store.v1 has two required operations (read + write); the
    // native extension models them as two distinct capabilities, so the
    // aggregated claim must cover both.
    let manifest = native_memory_manifest().expect("manifest");
    let document_store = CapabilityProfileId::new("memory.document_store.v1").unwrap();
    let implementers: BTreeMap<&str, ()> = manifest
        .capabilities
        .iter()
        .filter(|c| c.implements.contains(&document_store))
        .map(|c| (c.id.as_str(), ()))
        .collect();
    assert_eq!(
        implementers.len(),
        2,
        "document_store must be implemented by exactly two capabilities (read + write)"
    );
}
