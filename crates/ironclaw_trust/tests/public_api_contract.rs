use std::collections::BTreeSet;

use ironclaw_host_api::{
    EffectKind, PackageId, PackageIdentity, PackageSource, RequestedTrustClass, TrustClass,
};
use ironclaw_trust::{
    AdminConfig, AdminEntry, BundledEntry, BundledRegistry, HostTrustAssignment, HostTrustPolicy,
    TrustPolicy, TrustPolicyInput, TrustProvenance,
};

fn pkg(id: &str) -> PackageId {
    PackageId::new(id).unwrap()
}

fn bundled_identity(id: &str) -> PackageIdentity {
    PackageIdentity::new(pkg(id), PackageSource::Bundled, None, None)
}

fn input(identity: PackageIdentity, requested: RequestedTrustClass) -> TrustPolicyInput {
    TrustPolicyInput {
        identity,
        requested_trust: requested,
        requested_authority: BTreeSet::new(),
    }
}

#[test]
fn public_api_can_seed_privileged_bundled_policy_entries() {
    let entry = BundledEntry::new(
        pkg("ironclaw_core"),
        None,
        HostTrustAssignment::system(),
        vec![EffectKind::DispatchCapability],
        None,
    );
    let policy = HostTrustPolicy::new(vec![Box::new(BundledRegistry::with_entries([entry]))])
        .expect("unique policy source chain should construct");

    let decision = policy
        .evaluate(&input(
            bundled_identity("ironclaw_core"),
            RequestedTrustClass::SystemRequested,
        ))
        .unwrap();

    assert_eq!(decision.effective_trust.class(), TrustClass::System);
    assert_eq!(decision.provenance, TrustProvenance::Bundled);
}

#[test]
fn public_api_can_seed_privileged_admin_policy_entries() {
    let entry = AdminEntry::for_bundled(
        pkg("operator_blessed"),
        None,
        HostTrustAssignment::first_party(),
        vec![EffectKind::ReadFilesystem],
        None,
    );
    let policy = HostTrustPolicy::new(vec![Box::new(AdminConfig::with_entries([entry]))])
        .expect("unique policy source chain should construct");

    let decision = policy
        .evaluate(&input(
            bundled_identity("operator_blessed"),
            RequestedTrustClass::FirstPartyRequested,
        ))
        .unwrap();

    assert_eq!(decision.effective_trust.class(), TrustClass::FirstParty);
    assert_eq!(decision.provenance, TrustProvenance::AdminConfig);
}

#[test]
fn duplicate_policy_source_types_are_rejected() {
    let result = HostTrustPolicy::new(vec![
        Box::new(AdminConfig::new()),
        Box::new(AdminConfig::new()),
    ]);

    assert!(
        result.is_err(),
        "duplicated source types make mutation targeting ambiguous"
    );
}
