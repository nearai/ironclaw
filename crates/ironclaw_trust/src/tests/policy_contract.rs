//! PR1b acceptance-criteria contract tests.
//!
//! Each test maps to a row in the plan's coverage matrix:
//!   - T1..T12: trust policy + invalidation contract
//!   - T13, T13b–e: AdminConfig source/digest binding (PR3043 review fix)
//!   - T14, T14b–e: HostTrustPolicy::mutate_with orchestration (AC #6
//!     enforcement via type-private mutators)
//!   - T15: default_decision fail-closed across every PackageSource
//!   - T16, T16b–c: TrustChange no-op / downgrade / upgrade / kind-change
//!     semantics + InvalidationBus publish-time guard
//!   - T17: LocalDevOverride inert-contract pinning (forward-compat seam)
//!   - T18: EffectiveTrustClass audit wire-shape for all four variants
//!
//! Test fixtures live in `mod support` below: `FakeAuthorizer` (gates
//! capability invocation on `EffectiveTrustClass::is_privileged()` AND an
//! explicit grant set) and `FakeGrantStore` (records invalidation events on
//! a shared `InvalidationBus`). These prove ordering and grant-denial
//! behavior at the integration boundary that PR3 will eventually own.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use crate::fixtures::{
    admin_entry_for_test, admin_entry_with_digest_for_test, bundled_entry_for_test,
    effective_first_party_for_test, effective_system_for_test,
};
use crate::{
    AdminConfig, AdminEntry, BundledRegistry, EffectiveTrustClass, HostTrustAssignment,
    HostTrustPolicy, InvalidationBus, LocalDevOverride, PolicySource, TrustChange,
    TrustChangeListener, TrustDecision, TrustError, TrustPolicy, TrustPolicyInput, TrustProvenance,
    authority_changed, grant_retention_eligible, identity_changed,
};
use chrono::Utc;
use ironclaw_host_api::{
    CapabilityId, EffectKind, PackageId, PackageIdentity, PackageSource, RequestedTrustClass,
    ResourceCeiling, TrustClass,
};
use static_assertions::assert_not_impl_any;

// ---------------------------------------------------------------------------
// Compile-time invariant for AC #1: `EffectiveTrustClass` must NOT implement
// `DeserializeOwned`. If a future change accidentally adds a Deserialize
// impl, this check fires at compile time rather than letting wire payloads
// forge privileged effective trust. Mirrors the existing `host_api` pattern
// (`assert_not_impl_any!(HostPath: serde::Serialize)`).
//
// `DeserializeOwned` is the practical attack surface — JSON / TOML / binary
// codecs that produce owned strings need `DeserializeOwned`. A bare
// `Deserialize<'de>` impl would also fail this check on `Copy` types.
// ---------------------------------------------------------------------------
assert_not_impl_any!(EffectiveTrustClass: serde::de::DeserializeOwned);
assert_not_impl_any!(HostTrustAssignment: serde::de::DeserializeOwned);

use self::support::{FakeAuthorizer, FakeGrantStore};

mod support {
    use std::sync::{Arc, Mutex};

    use crate::{EffectiveTrustClass, TrustChange, TrustChangeListener};
    use ironclaw_host_api::{CapabilityId, PackageIdentity};

    /// Records every invalidation that fires on the bus, in order, with the
    /// timestamp at which it was observed. Used to assert ordering against
    /// subsequent policy evaluations.
    pub struct FakeGrantStore {
        invalidations: Mutex<Vec<TrustChange>>,
    }

    impl FakeGrantStore {
        pub fn new() -> Arc<Self> {
            Arc::new(Self {
                invalidations: Mutex::new(Vec::new()),
            })
        }

        pub fn invalidations(&self) -> Vec<TrustChange> {
            self.invalidations
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .clone()
        }
    }

    impl TrustChangeListener for FakeGrantStore {
        fn on_trust_changed(&self, change: &TrustChange) {
            let mut guard = self.invalidations.lock().unwrap_or_else(|p| p.into_inner());
            guard.push(change.clone());
        }
    }

    /// Stand-in for the PR3 authorization layer. Holds an explicit grant set
    /// keyed by `(PackageIdentity, CapabilityId)` and consults the supplied
    /// `EffectiveTrustClass` to decide whether to grant a privileged-effect
    /// capability — this is the surface the issue's suggested test #1 ("AND
    /// privileged capability grant attempts fail") is verified against.
    pub struct FakeAuthorizer {
        grants: Mutex<Vec<(PackageIdentity, CapabilityId)>>,
    }

    impl FakeAuthorizer {
        pub fn new() -> Self {
            Self {
                grants: Mutex::new(Vec::new()),
            }
        }

        pub fn grant(&self, identity: PackageIdentity, capability: CapabilityId) {
            self.grants
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .push((identity, capability));
        }

        /// Returns true iff the grant exists *and* the policy-validated
        /// effective trust is privileged. Mimics the PR3 contract: trust
        /// alone grants nothing; grant alone without privileged trust does
        /// not unlock a privileged capability either.
        pub fn invoke_privileged(
            &self,
            identity: &PackageIdentity,
            capability: &CapabilityId,
            effective_trust: EffectiveTrustClass,
        ) -> bool {
            if !effective_trust.is_privileged() {
                return false;
            }
            let grants = self.grants.lock().unwrap_or_else(|p| p.into_inner());
            grants
                .iter()
                .any(|(pid, cap)| pid == identity && cap == capability)
        }

        /// Equivalent of `invoke_privileged` but for non-privileged effects:
        /// grant must exist; trust class is irrelevant for non-privileged.
        pub fn invoke(&self, identity: &PackageIdentity, capability: &CapabilityId) -> bool {
            let grants = self.grants.lock().unwrap_or_else(|p| p.into_inner());
            grants
                .iter()
                .any(|(pid, cap)| pid == identity && cap == capability)
        }
    }
}

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

fn pkg(id: &str) -> PackageId {
    PackageId::new(id).unwrap()
}

fn cap(id: &str) -> CapabilityId {
    CapabilityId::new(id).unwrap()
}

fn local_manifest_identity(id: &str) -> PackageIdentity {
    PackageIdentity::new(
        pkg(id),
        PackageSource::LocalManifest {
            path: format!("/extensions/{id}/manifest.toml"),
        },
        None,
        None,
    )
}

fn bundled_identity(id: &str, digest: Option<&str>) -> PackageIdentity {
    PackageIdentity::new(
        pkg(id),
        PackageSource::Bundled,
        digest.map(|s| s.to_string()),
        None,
    )
}

fn input(identity: PackageIdentity, requested: RequestedTrustClass) -> TrustPolicyInput {
    TrustPolicyInput {
        identity,
        requested_trust: requested,
        requested_authority: BTreeSet::new(),
    }
}

fn policy(sources: Vec<Box<dyn PolicySource>>) -> HostTrustPolicy {
    HostTrustPolicy::new(sources).unwrap()
}

/// Build a capability set from a list of names. Convenience helper for
/// tests that exercise authority sets — duplicates and reorderings
/// collapse into the same canonical `BTreeSet`, which is exactly the
/// invariant the type signature is enforcing in production.
fn caps(names: &[&str]) -> BTreeSet<CapabilityId> {
    names.iter().map(|n| cap(n)).collect()
}

fn decision_for_test(
    effective_trust: EffectiveTrustClass,
    allowed_effects: Vec<EffectKind>,
) -> TrustDecision {
    TrustDecision {
        effective_trust,
        authority_ceiling: crate::AuthorityCeiling {
            allowed_effects,
            max_resource_ceiling: None,
        },
        provenance: TrustProvenance::AdminConfig,
        evaluated_at: Utc::now(),
    }
}

fn trust_change_for_test(
    identity: PackageIdentity,
    previous: EffectiveTrustClass,
    current: EffectiveTrustClass,
) -> Option<TrustChange> {
    let previous = decision_for_test(previous, Vec::new());
    let current = decision_for_test(current, Vec::new());
    TrustChange::new(identity, &previous, &current)
}

fn output_token_ceiling(max_output_tokens: u64) -> ResourceCeiling {
    ResourceCeiling {
        max_usd: None,
        max_input_tokens: None,
        max_output_tokens: Some(max_output_tokens),
        max_wall_clock_ms: None,
        max_output_bytes: None,
        sandbox: None,
    }
}

// ---------------------------------------------------------------------------
// T1 — self-promotion denied for user manifest
// Issue suggested test #1 (effective ≠ privileged). AC #2, AC #8.
// ---------------------------------------------------------------------------

#[test]
fn t1_self_promotion_denied_for_user_manifest() {
    let policy = policy(vec![
        Box::new(BundledRegistry::new()),
        Box::new(AdminConfig::new()),
    ]);

    let identity = local_manifest_identity("rogue");
    let decision = policy
        .evaluate(&input(identity, RequestedTrustClass::SystemRequested))
        .unwrap();

    assert!(
        !decision.effective_trust.is_privileged(),
        "user-installed manifest must not produce privileged effective trust"
    );
    assert_eq!(decision.provenance, TrustProvenance::Default);
    // Defense in depth: the underlying TrustClass is not FirstParty/System.
    assert!(matches!(
        decision.effective_trust.class(),
        TrustClass::Sandbox | TrustClass::UserTrusted
    ));
}

// ---------------------------------------------------------------------------
// T2 — self-promotion blocks privileged grant via FakeAuthorizer
// Issue suggested test #1 (privileged grant attempts fail). AC #2 (second half).
// ---------------------------------------------------------------------------

#[test]
fn t2_self_promotion_blocks_privileged_grant_via_fake_authorizer() {
    let policy = policy(vec![Box::new(BundledRegistry::new())]);
    let authorizer = FakeAuthorizer::new();

    let identity = local_manifest_identity("rogue");
    let capability = cap("rogue.delete_filesystem");
    // Even if a grant *somehow* existed for this identity, the authorizer
    // must refuse because trust came back non-privileged.
    authorizer.grant(identity.clone(), capability.clone());

    let decision = policy
        .evaluate(&input(
            identity.clone(),
            RequestedTrustClass::SystemRequested,
        ))
        .unwrap();

    assert!(!authorizer.invoke_privileged(&identity, &capability, decision.effective_trust,));
}

// ---------------------------------------------------------------------------
// T3 — host assignment via bundled registry grants effective trust
// Issue suggested test #2 (effective can be FirstParty/System). AC #3.
// ---------------------------------------------------------------------------

#[test]
fn t3_host_assignment_via_bundled_registry_grants_effective_trust() {
    let registry = BundledRegistry::with_entries([bundled_entry_for_test(
        pkg("ironclaw_core"),
        Some("digest_v1".to_string()),
        effective_system_for_test(),
        vec![EffectKind::DispatchCapability],
    )]);
    let policy = policy(vec![Box::new(registry)]);

    let identity = bundled_identity("ironclaw_core", Some("digest_v1"));
    let decision = policy
        .evaluate(&input(identity, RequestedTrustClass::SystemRequested))
        .unwrap();

    assert!(decision.effective_trust.is_privileged());
    assert_eq!(decision.effective_trust.class(), TrustClass::System);
    assert_eq!(decision.provenance, TrustProvenance::Bundled);
}

// ---------------------------------------------------------------------------
// T4 — host assignment alone grants no capability
// Issue suggested test #2 (no capabilities granted unless explicit grant).
// AC #4, AC #9.
// ---------------------------------------------------------------------------

#[test]
fn t4_host_assignment_alone_grants_no_capability() {
    let registry = BundledRegistry::with_entries([bundled_entry_for_test(
        pkg("ironclaw_core"),
        None,
        effective_system_for_test(),
        vec![EffectKind::DispatchCapability],
    )]);
    let policy = policy(vec![Box::new(registry)]);

    let identity = bundled_identity("ironclaw_core", None);
    let decision = policy
        .evaluate(&input(
            identity.clone(),
            RequestedTrustClass::SystemRequested,
        ))
        .unwrap();

    // Effective trust is System — but no grant exists in the authorizer.
    let authorizer = FakeAuthorizer::new();
    let capability = cap("ironclaw_core.shutdown");
    assert!(!authorizer.invoke_privileged(&identity, &capability, decision.effective_trust));
}

// ---------------------------------------------------------------------------
// T5 — effective system without grant denies invocation
// Issue suggested test #3. AC #4, AC #9.
// ---------------------------------------------------------------------------

#[test]
fn t5_effective_system_without_grant_denies_invocation() {
    let identity = bundled_identity("ironclaw_core", None);
    let capability = cap("ironclaw_core.purge_workspace");
    let authorizer = FakeAuthorizer::new();

    // No grant added. Even with the highest possible effective trust, the
    // authorizer must say no.
    assert!(!authorizer.invoke_privileged(&identity, &capability, effective_system_for_test()));
    // And for non-privileged effects, grant alone (without trust) is also
    // insufficient: the test asserts the *contract* — grant must exist.
    assert!(!authorizer.invoke(&identity, &capability));
}

// ---------------------------------------------------------------------------
// T6 — expanded authority requires renewed approval (uses authority_changed)
// AC #5.
// ---------------------------------------------------------------------------

#[test]
fn t6_expanded_authority_requires_renewed_approval() {
    let prev = caps(&["github.read"]);
    let curr_added = caps(&["github.read", "github.delete"]);
    let curr_unchanged = caps(&["github.read"]);
    let curr_removed_all = caps(&[]);

    assert!(
        authority_changed(&prev, &curr_added),
        "growth in requested authority must force re-approval"
    );
    assert!(
        !authority_changed(&prev, &curr_unchanged),
        "identical authority sets must remain retainable"
    );
    // Removal also fires per the documented over-firing semantic in
    // `authority_changed`: any set difference invalidates retention.
    assert!(
        authority_changed(&prev, &curr_removed_all),
        "removal of authority entries must also force re-evaluation \
         (deliberate over-firing — see authority_changed docs)"
    );

    // BTreeSet typing means insertion-order/duplicates are erased at the
    // type boundary — `[a, b]` and `[b, a]` produce the same set.
    let prev_two = caps(&["github.read", "github.write"]);
    let prev_two_reordered = caps(&["github.write", "github.read"]);
    assert!(
        !authority_changed(&prev_two, &prev_two_reordered),
        "reordering the same set must remain retainable"
    );

    // Regression for the "authority_changed over-fires on duplicate
    // entries" review finding: a hypothetical caller passing
    // `[a, a, b]` (multiset) and `[a, b]` (set) used to fire because
    // the slice-based check length-guarded against multiset drift.
    // Set typing makes the multiset literally inexpressible — both
    // collapse into `{a, b}` at construction.
    let multiset_collapsed: BTreeSet<CapabilityId> =
        [cap("github.read"), cap("github.read"), cap("github.write")]
            .into_iter()
            .collect();
    let plain = caps(&["github.read", "github.write"]);
    assert!(
        !authority_changed(&multiset_collapsed, &plain),
        "duplicates in the source list must canonicalize away — \
         set typing prevents the [a, a, b] vs [a, b] over-fire"
    );

    // grant_retention_eligible composes identity + trust + authority:
    // identity stable, trust stable, authority grew ⇒ retention denied.
    let identity = bundled_identity("github", None);
    let trust = effective_first_party_for_test();
    assert!(!grant_retention_eligible(
        &identity,
        &identity,
        trust,
        trust,
        &prev,
        &curr_added,
    ));
}

// ---------------------------------------------------------------------------
// T7 — downgrade publishes invalidation before next dispatch
// Issue suggested test #4. AC #6.
// ---------------------------------------------------------------------------

#[test]
fn t7_downgrade_publishes_invalidation_before_next_dispatch() {
    let bus = InvalidationBus::new();
    let store = FakeGrantStore::new();
    bus.register(store.clone());

    let identity = bundled_identity("ironclaw_core", Some("digest_v1"));

    let change = trust_change_for_test(
        identity.clone(),
        effective_system_for_test(),
        EffectiveTrustClass::user_trusted(),
    )
    .expect("System → UserTrusted is a real change, not a no-op");
    bus.publish(change.clone());

    // Synchronous fan-out: invalidation must be observable immediately.
    let recorded = store.invalidations();
    assert_eq!(
        recorded.len(),
        1,
        "publish must run listeners synchronously"
    );
    assert_eq!(recorded[0], change);

    // Modeling "next dispatch": we now build a policy that returns the
    // downgraded decision. The grant store has already recorded the
    // invalidation, so any subsequent policy result returning the lower
    // trust is observed *after* the invalidation, not before.
    let policy = policy(vec![Box::new(BundledRegistry::new())]);
    let next_decision = policy
        .evaluate(&input(identity, RequestedTrustClass::SystemRequested))
        .unwrap();
    assert!(!next_decision.effective_trust.is_privileged());
    assert!(
        !store.invalidations().is_empty(),
        "invalidation visible before downgraded evaluation returns"
    );
}

// ---------------------------------------------------------------------------
// T8 — revocation publishes invalidation before next dispatch
// Issue suggested test #4. AC #6.
// ---------------------------------------------------------------------------

#[test]
fn t8_revocation_publishes_invalidation_before_next_dispatch() {
    let bus = InvalidationBus::new();
    let store = FakeGrantStore::new();
    bus.register(store.clone());

    // Revocation models a complete drop to non-privileged trust due to a
    // policy-source removal (admin removed the entry).
    let identity = bundled_identity("ironclaw_core", None);
    let change = trust_change_for_test(
        identity.clone(),
        effective_system_for_test(),
        EffectiveTrustClass::sandbox(),
    )
    .expect("System → Sandbox is a real revocation, not a no-op");
    bus.publish(change.clone());

    let recorded = store.invalidations();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].current.class(), TrustClass::Sandbox);
}

// ---------------------------------------------------------------------------
// T9 — requested trust class cannot satisfy effective trust argument
// Issue suggested test #5 (compile-time). AC #1.
// ---------------------------------------------------------------------------

/// The compile-time half of this guarantee lives at the top of the file
/// (`assert_not_impl_any!(EffectiveTrustClass: serde::de::DeserializeOwned)`):
/// no `Deserialize`-shaped path can produce an `EffectiveTrustClass` from a
/// wire payload. The runtime half — that the *publicly constructible*
/// `EffectiveTrustClass` values are never privileged — is asserted here.
///
/// Together these prove that `RequestedTrustClass` (which freely
/// deserializes from manifest JSON) cannot be coerced or wire-decoded into
/// a privileged effective ceiling. The only path to privileged effective
/// trust is `TrustPolicy::evaluate`, exercised by T3.
#[test]
fn t9_requested_trust_class_cannot_satisfy_effective_trust_argument() {
    // RequestedTrustClass exists and freely deserializes ...
    let requested = RequestedTrustClass::SystemRequested;
    let _ = requested;
    // ... but every publicly constructible EffectiveTrustClass is non-privileged.
    let public_constructors = [
        EffectiveTrustClass::sandbox(),
        EffectiveTrustClass::user_trusted(),
    ];
    for trust in public_constructors {
        assert!(
            !trust.is_privileged(),
            "publicly constructible EffectiveTrustClass must never be privileged"
        );
    }
    // And `TrustClass`'s underlying serde gate for privileged variants is
    // independently asserted in `host_api_contract.rs`.
}

// ---------------------------------------------------------------------------
// T10 — manifest JSON with system field parses only into requested type
// Issue suggested test #5 (runtime). AC #1.
// ---------------------------------------------------------------------------

#[test]
fn t10_manifest_json_with_system_field_parses_only_into_requested_type() {
    // RequestedTrustClass round-trips system_requested...
    let parsed: RequestedTrustClass =
        serde_json::from_value(serde_json::json!("system_requested")).unwrap();
    assert_eq!(parsed, RequestedTrustClass::SystemRequested);

    // ...but TrustClass deserialization rejects "system":
    assert!(serde_json::from_value::<TrustClass>(serde_json::json!("system")).is_err());

    // And EffectiveTrustClass does NOT implement Deserialize at all (the
    // trait bound below would not compile if it did). We verify by checking
    // the wire-only round-trip: serialize ok, but no public Deserialize
    // impl exists. The compile-time absence is the actual guarantee — the
    // assertion below is a sanity check that the serialized form matches
    // host_api::TrustClass exactly.
    let value = serde_json::to_value(EffectiveTrustClass::user_trusted()).unwrap();
    assert_eq!(value, serde_json::json!("user_trusted"));
}

// ---------------------------------------------------------------------------
// T11 — digest drift forces grant reissue (uses identity_changed)
// AC #7.
// ---------------------------------------------------------------------------

#[test]
fn t11_digest_drift_forces_grant_reissue() {
    let prev = bundled_identity("ironclaw_core", Some("digest_v1"));
    let curr = bundled_identity("ironclaw_core", Some("digest_v2"));

    assert!(identity_changed(&prev, &curr));

    // Bundled registry pins on digest: a digest mismatch forces a fall-through
    // to the default downgrade, which is exactly the AC #7 grant-reissue
    // trigger.
    let registry = BundledRegistry::with_entries([bundled_entry_for_test(
        pkg("ironclaw_core"),
        Some("digest_v1".to_string()),
        effective_first_party_for_test(),
        vec![],
    )]);
    let policy = policy(vec![Box::new(registry)]);

    let prev_decision = policy
        .evaluate(&input(
            prev.clone(),
            RequestedTrustClass::FirstPartyRequested,
        ))
        .unwrap();
    let curr_decision = policy
        .evaluate(&input(
            curr.clone(),
            RequestedTrustClass::FirstPartyRequested,
        ))
        .unwrap();

    assert!(prev_decision.effective_trust.is_privileged());
    assert!(!curr_decision.effective_trust.is_privileged());
    let empty_authority = BTreeSet::<CapabilityId>::new();
    assert!(!grant_retention_eligible(
        &prev,
        &curr,
        prev_decision.effective_trust,
        curr_decision.effective_trust,
        &empty_authority,
        &empty_authority,
    ));
}

// ---------------------------------------------------------------------------
// T12 — signer drift forces grant reissue
// AC #7.
// ---------------------------------------------------------------------------

#[test]
fn t12_signer_drift_forces_grant_reissue() {
    let prev = PackageIdentity::new(
        pkg("ironclaw_core"),
        PackageSource::Bundled,
        Some("digest".to_string()),
        Some("signer_a".to_string()),
    );
    let curr = PackageIdentity::new(
        pkg("ironclaw_core"),
        PackageSource::Bundled,
        Some("digest".to_string()),
        Some("signer_b".to_string()),
    );

    assert!(identity_changed(&prev, &curr));
    let trust = effective_first_party_for_test();
    let empty_authority = BTreeSet::<CapabilityId>::new();
    assert!(!grant_retention_eligible(
        &prev,
        &curr,
        trust,
        trust,
        &empty_authority,
        &empty_authority,
    ));
}

// ---------------------------------------------------------------------------
// T13 — admin config grants trust when source binding matches.
// Decision rule #3 from the plan.
// ---------------------------------------------------------------------------

#[test]
fn t13_admin_config_grants_trust_when_source_binding_matches() {
    let bundled = BundledRegistry::new(); // empty
    let admin = AdminConfig::with_entries([admin_entry_for_test(
        pkg("operator_blessed"),
        PackageSource::Bundled,
        effective_first_party_for_test(),
        vec![EffectKind::ReadFilesystem],
    )]);

    // Layered: bundled first, admin second. Bundled is empty, admin's
    // entry binds to PackageSource::Bundled, and the input identity is
    // also PackageSource::Bundled, so the admin entry matches and grants
    // FirstParty.
    let policy = policy(vec![Box::new(bundled), Box::new(admin)]);

    let identity = bundled_identity("operator_blessed", None);
    let decision = policy
        .evaluate(&input(identity, RequestedTrustClass::FirstPartyRequested))
        .unwrap();

    assert_eq!(decision.effective_trust.class(), TrustClass::FirstParty);
    assert_eq!(decision.provenance, TrustProvenance::AdminConfig);
}

// ---------------------------------------------------------------------------
// T13b — admin config does NOT shadow across sources.
//
// Regression for the PR3043 review finding: an `AdminEntry` for a
// `Bundled` package_id must not let a `LocalManifest` package with the
// same id pick up the elevation. Pre-fix, this exact path returned
// `TrustClass::FirstParty`; post-fix it must fall through to the default
// downgrade, which puts a LocalManifest at `Sandbox`.
// ---------------------------------------------------------------------------

#[test]
fn t13b_admin_config_does_not_shadow_local_manifest_with_bundled_entry() {
    let admin = AdminConfig::with_entries([admin_entry_for_test(
        pkg("operator_blessed"),
        PackageSource::Bundled,
        effective_first_party_for_test(),
        vec![EffectKind::ReadFilesystem],
    )]);
    let policy = policy(vec![Box::new(BundledRegistry::new()), Box::new(admin)]);

    // Same package_id as the admin entry, but coming from a user-writable
    // LocalManifest. Must NOT inherit the elevation.
    let identity = local_manifest_identity("operator_blessed");
    let decision = policy
        .evaluate(&input(identity, RequestedTrustClass::FirstPartyRequested))
        .unwrap();

    assert!(
        !decision.effective_trust.is_privileged(),
        "LocalManifest must not pick up an admin elevation aimed at Bundled"
    );
    assert_eq!(decision.effective_trust.class(), TrustClass::Sandbox);
    assert_eq!(decision.provenance, TrustProvenance::Default);
}

// ---------------------------------------------------------------------------
// T13c — admin config rejects cross-registry source shadowing.
//
// An `AdminEntry::for_registry("https://trusted.example/...")` must not
// match a `Bundled` package with the same id — different origin, no
// elevation. Mirror of T13b for the Registry/Bundled cross-source case.
// ---------------------------------------------------------------------------

#[test]
fn t13c_admin_config_does_not_shadow_bundled_with_registry_entry() {
    let admin = AdminConfig::with_entries([AdminEntry::for_registry(
        pkg("operator_blessed"),
        "https://trusted.example/registry".to_string(),
        None,
        HostTrustAssignment::first_party(),
        vec![EffectKind::ReadFilesystem],
        None,
    )]);
    let policy = policy(vec![Box::new(admin)]);

    // Same package_id as the admin entry, but Bundled origin instead of
    // the entry's pinned Registry. Must fall through.
    let identity = bundled_identity("operator_blessed", None);
    let decision = policy
        .evaluate(&input(identity, RequestedTrustClass::FirstPartyRequested))
        .unwrap();

    assert!(!decision.effective_trust.is_privileged());
    assert_eq!(decision.provenance, TrustProvenance::Default);
}

// ---------------------------------------------------------------------------
// T13d — admin digest pin drift falls through to default.
//
// AdminEntry parallels BundledEntry digest semantics: when the entry pins
// a digest, the package's digest must match exactly. Drift falls through
// to the default downgrade, which is the AC #7 grant-reissue trigger.
// ---------------------------------------------------------------------------

#[test]
fn t13d_admin_digest_pin_drift_falls_through() {
    let admin = AdminConfig::with_entries([admin_entry_with_digest_for_test(
        pkg("operator_blessed"),
        PackageSource::Bundled,
        "digest_v1".to_string(),
        effective_first_party_for_test(),
        vec![EffectKind::ReadFilesystem],
    )]);
    let policy = policy(vec![Box::new(admin)]);

    // Digest drift: package presents v2, entry pins v1. No match.
    let drifted = bundled_identity("operator_blessed", Some("digest_v2"));
    let decision = policy
        .evaluate(&input(drifted, RequestedTrustClass::FirstPartyRequested))
        .unwrap();

    assert!(!decision.effective_trust.is_privileged());
    assert_eq!(decision.provenance, TrustProvenance::Default);

    // Sanity: when the digest matches, the entry does match.
    let aligned = bundled_identity("operator_blessed", Some("digest_v1"));
    let decision = policy
        .evaluate(&input(aligned, RequestedTrustClass::FirstPartyRequested))
        .unwrap();
    assert_eq!(decision.effective_trust.class(), TrustClass::FirstParty);
    assert_eq!(decision.provenance, TrustProvenance::AdminConfig);
}

// ---------------------------------------------------------------------------
// T13e — explicit `for_local_manifest` constructor elevates when chosen.
//
// The fix doesn't ban LocalManifest elevation outright; it requires the
// operator to *spell it out* via the dedicated constructor. This test
// pins the contract that the explicit path still works — and the
// constructor name (`for_local_manifest`) is what `rg` will surface for
// review.
// ---------------------------------------------------------------------------

#[test]
fn t13e_admin_for_local_manifest_elevates_when_explicit() {
    let path = "/extensions/operator_blessed/manifest.toml".to_string();
    let admin = AdminConfig::with_entries([AdminEntry::for_local_manifest(
        pkg("operator_blessed"),
        path.clone(),
        None,
        HostTrustAssignment::first_party(),
        vec![EffectKind::ReadFilesystem],
        None,
    )]);
    let policy = policy(vec![Box::new(admin)]);

    // Identity path matches the entry's pinned manifest path — the
    // PackageSource::LocalManifest equality covers both kind and path.
    let identity = local_manifest_identity("operator_blessed");
    assert_eq!(
        identity.source,
        PackageSource::LocalManifest { path: path.clone() },
        "test-helper path must match the entry's pinned path"
    );
    let decision = policy
        .evaluate(&input(identity, RequestedTrustClass::FirstPartyRequested))
        .unwrap();

    assert_eq!(decision.effective_trust.class(), TrustClass::FirstParty);
    assert_eq!(decision.provenance, TrustProvenance::AdminConfig);

    // A different manifest path with the same package_id must not match —
    // the `path` is part of the source equality.
    let other_path_identity = PackageIdentity::new(
        pkg("operator_blessed"),
        PackageSource::LocalManifest {
            path: "/elsewhere/manifest.toml".to_string(),
        },
        None,
        None,
    );
    let decision = policy
        .evaluate(&input(
            other_path_identity,
            RequestedTrustClass::FirstPartyRequested,
        ))
        .unwrap();
    assert!(
        !decision.effective_trust.is_privileged(),
        "for_local_manifest must bind to a specific manifest path, not any LocalManifest"
    );
    assert_eq!(decision.provenance, TrustProvenance::Default);
}

// ---------------------------------------------------------------------------
// T13f — admin config stores same-id entries independently by source.
//
// Regression for the PR3043 inline review: the trust subject is the full
// `(package_id, source)` pair, not `package_id` alone. A source-pinned
// map must retain both entries and evaluate each independently.
// ---------------------------------------------------------------------------

#[test]
fn t13f_admin_config_keeps_same_id_entries_for_distinct_sources() {
    let package_id = pkg("shared_pkg");
    let admin = AdminConfig::with_entries([
        admin_entry_for_test(
            package_id.clone(),
            PackageSource::Bundled,
            effective_first_party_for_test(),
            vec![EffectKind::ReadFilesystem],
        ),
        AdminEntry::for_registry(
            package_id.clone(),
            "https://trusted.example/registry".to_string(),
            None,
            HostTrustAssignment::system(),
            vec![EffectKind::Network],
            None,
        ),
    ]);
    let policy = policy(vec![Box::new(admin)]);

    let bundled = policy
        .evaluate(&input(
            bundled_identity("shared_pkg", None),
            RequestedTrustClass::FirstPartyRequested,
        ))
        .unwrap();
    assert_eq!(bundled.effective_trust.class(), TrustClass::FirstParty);
    assert_eq!(
        bundled.authority_ceiling.allowed_effects,
        vec![EffectKind::ReadFilesystem]
    );

    let registry = policy
        .evaluate(&input(
            PackageIdentity::new(
                package_id,
                PackageSource::Registry {
                    url: "https://trusted.example/registry".to_string(),
                },
                None,
                None,
            ),
            RequestedTrustClass::SystemRequested,
        ))
        .unwrap();
    assert_eq!(registry.effective_trust.class(), TrustClass::System);
    assert_eq!(
        registry.authority_ceiling.allowed_effects,
        vec![EffectKind::Network]
    );
}

#[test]
fn t13g_admin_remove_is_source_aware() {
    let package_id = pkg("shared_pkg");
    let registry_source = PackageSource::Registry {
        url: "https://trusted.example/registry".to_string(),
    };
    let admin = AdminConfig::with_entries([
        admin_entry_for_test(
            package_id.clone(),
            PackageSource::Bundled,
            effective_first_party_for_test(),
            vec![EffectKind::ReadFilesystem],
        ),
        admin_entry_for_test(
            package_id.clone(),
            registry_source.clone(),
            effective_system_for_test(),
            vec![EffectKind::Network],
        ),
    ]);
    let policy = policy(vec![Box::new(admin)]);
    let bus = InvalidationBus::new();

    let removed = policy
        .mutate_with(
            &bus,
            PackageIdentity::new(package_id.clone(), registry_source.clone(), None, None),
            caps(&["shared_pkg.network"]),
            RequestedTrustClass::SystemRequested,
            |m| m.admin_remove(&package_id, &registry_source),
        )
        .unwrap();
    assert!(removed.is_some());

    let bundled = policy
        .evaluate(&input(
            bundled_identity("shared_pkg", None),
            RequestedTrustClass::FirstPartyRequested,
        ))
        .unwrap();
    assert_eq!(
        bundled.effective_trust.class(),
        TrustClass::FirstParty,
        "removing the registry entry must not remove the bundled entry"
    );

    let registry = policy
        .evaluate(&input(
            PackageIdentity::new(package_id, registry_source, None, None),
            RequestedTrustClass::SystemRequested,
        ))
        .unwrap();
    assert_eq!(registry.effective_trust.class(), TrustClass::Sandbox);
}

// ---------------------------------------------------------------------------
// T14 — `mutate_with` orchestrates pre-eval / mutate / post-eval / publish.
//
// Regression for the "InvalidationBus orchestration is enforced only by
// caller discipline" review finding: AC #6 is now a compile-time guarantee
// because the per-source `upsert` / `remove` methods are `pub(crate)` and
// the only public path to them is `HostTrustPolicy::mutate_with`, which
// hard-wires the publish step into the orchestration.
//
// T14 — publish fires when the affected identity's trust class drops.
// T14b — no publish when the closure leaves the affected identity's class
//        unchanged (e.g., mutating an unrelated package).
// T14c — closure return value is surfaced to the caller.
// T14d — missing source kind yields `InvariantViolation` with the type
//        spelled out in the message.
// T14e — closure errors roll back staged mutations: no state change and no
//        publish.
// T14f — same-trust allowed-effect reductions publish.
// T14g — evaluate waits until synchronous invalidation publish completes.
// T14h — same-trust resource-ceiling reductions publish.
// T14i — listeners can re-enter evaluate without deadlocking.
// ---------------------------------------------------------------------------

#[test]
fn t14_mutate_with_publishes_when_affected_trust_class_drops() {
    let admin = AdminConfig::with_entries([admin_entry_for_test(
        pkg("operator_blessed"),
        PackageSource::Bundled,
        effective_first_party_for_test(),
        vec![EffectKind::ReadFilesystem],
    )]);
    let policy = policy(vec![Box::new(admin)]);
    let bus = InvalidationBus::new();
    let store = FakeGrantStore::new();
    bus.register(store.clone());

    let identity = bundled_identity("operator_blessed", None);
    let prev_authority = caps(&["operator_blessed.read"]);

    policy
        .mutate_with(
            &bus,
            identity.clone(),
            prev_authority.clone(),
            RequestedTrustClass::FirstPartyRequested,
            |m| {
                m.admin_remove(&pkg("operator_blessed"), &PackageSource::Bundled)?;
                Ok(())
            },
        )
        .unwrap();

    let recorded = store.invalidations();
    assert_eq!(
        recorded.len(),
        1,
        "removal that drops trust class must publish exactly one TrustChange"
    );
    assert_eq!(recorded[0].identity, identity);
    assert_eq!(recorded[0].previous.class(), TrustClass::FirstParty);
    assert!(
        !recorded[0].current.is_privileged(),
        "post-removal trust must not be privileged \
         (unmatched Bundled falls to Sandbox per default_decision fail-closed contract)"
    );
}

#[test]
fn t14b_mutate_with_does_not_publish_when_affected_trust_class_unchanged() {
    let admin = AdminConfig::with_entries([admin_entry_for_test(
        pkg("operator_blessed"),
        PackageSource::Bundled,
        effective_first_party_for_test(),
        vec![EffectKind::ReadFilesystem],
    )]);
    let policy = policy(vec![Box::new(admin)]);
    let bus = InvalidationBus::new();
    let store = FakeGrantStore::new();
    bus.register(store.clone());

    // Affected identity is "unrelated" — the mutation touches a different
    // package, so the affected identity's trust class stays at the default.
    policy
        .mutate_with(
            &bus,
            bundled_identity("unrelated", None),
            BTreeSet::new(),
            RequestedTrustClass::FirstPartyRequested,
            |m| {
                m.admin_upsert(admin_entry_for_test(
                    pkg("other_pkg"),
                    PackageSource::Bundled,
                    effective_system_for_test(),
                    vec![],
                ))?;
                Ok(())
            },
        )
        .unwrap();

    assert!(
        store.invalidations().is_empty(),
        "no publish when the affected identity's effective trust class \
         did not change"
    );
}

#[test]
fn t14c_mutate_with_returns_closure_result() {
    let admin = AdminConfig::with_entries([admin_entry_for_test(
        pkg("operator_blessed"),
        PackageSource::Bundled,
        effective_first_party_for_test(),
        vec![EffectKind::ReadFilesystem],
    )]);
    let policy = policy(vec![Box::new(admin)]);
    let bus = InvalidationBus::new();

    let removed = policy
        .mutate_with(
            &bus,
            bundled_identity("operator_blessed", None),
            BTreeSet::new(),
            RequestedTrustClass::FirstPartyRequested,
            |m| m.admin_remove(&pkg("operator_blessed"), &PackageSource::Bundled),
        )
        .unwrap();

    assert!(
        removed.is_some(),
        "closure must surface the removed entry through mutate_with's return"
    );
}

#[test]
fn t14d_mutate_with_surfaces_missing_source_kind() {
    // Policy chain has no AdminConfig — calling admin_remove must yield
    // InvariantViolation, not silently no-op.
    let policy = policy(vec![Box::new(BundledRegistry::new())]);
    let bus = InvalidationBus::new();

    let result = policy.mutate_with(
        &bus,
        bundled_identity("anything", None),
        BTreeSet::new(),
        RequestedTrustClass::ThirdParty,
        |m| {
            m.admin_remove(&pkg("anything"), &PackageSource::Bundled)?;
            Ok(())
        },
    );

    let TrustError::InvariantViolation { reason } = result.unwrap_err();
    assert!(
        reason.contains("AdminConfig"),
        "missing-source error must name the type — got: {reason}"
    );
}

#[test]
fn t14e_mutate_with_rolls_back_staged_mutations_on_closure_error() {
    let admin = AdminConfig::with_entries([admin_entry_for_test(
        pkg("operator_blessed"),
        PackageSource::Bundled,
        effective_first_party_for_test(),
        vec![EffectKind::ReadFilesystem],
    )]);
    let policy = policy(vec![Box::new(admin)]);
    let bus = InvalidationBus::new();
    let store = FakeGrantStore::new();
    bus.register(store.clone());

    let identity = bundled_identity("operator_blessed", None);

    // Closure stages a real removal, then errors. The staged mutation must
    // roll back: policy state remains unchanged and no invalidation is
    // necessary because no lower decision became visible.
    let result: Result<(), TrustError> = policy.mutate_with(
        &bus,
        identity.clone(),
        BTreeSet::new(),
        RequestedTrustClass::FirstPartyRequested,
        |m| {
            m.admin_remove(&pkg("operator_blessed"), &PackageSource::Bundled)?;
            Err(TrustError::InvariantViolation {
                reason: "simulated downstream failure".to_string(),
            })
        },
    );

    assert!(matches!(result, Err(TrustError::InvariantViolation { .. })));
    assert!(
        store.invalidations().is_empty(),
        "rolled-back closure errors must not publish"
    );

    let decision = policy
        .evaluate(&input(identity, RequestedTrustClass::FirstPartyRequested))
        .unwrap();
    assert_eq!(
        decision.effective_trust.class(),
        TrustClass::FirstParty,
        "staged removal must not be committed after the closure returns Err"
    );
}

#[test]
fn t14f_mutate_with_publishes_when_same_trust_loses_allowed_effects() {
    let admin = AdminConfig::with_entries([admin_entry_for_test(
        pkg("operator_blessed"),
        PackageSource::Bundled,
        effective_first_party_for_test(),
        vec![EffectKind::ReadFilesystem, EffectKind::WriteFilesystem],
    )]);
    let policy = policy(vec![Box::new(admin)]);
    let bus = InvalidationBus::new();
    let store = FakeGrantStore::new();
    bus.register(store.clone());

    let identity = bundled_identity("operator_blessed", None);
    policy
        .mutate_with(
            &bus,
            identity.clone(),
            caps(&["operator_blessed.read", "operator_blessed.write"]),
            RequestedTrustClass::FirstPartyRequested,
            |m| {
                m.admin_upsert(admin_entry_for_test(
                    pkg("operator_blessed"),
                    PackageSource::Bundled,
                    effective_first_party_for_test(),
                    vec![EffectKind::ReadFilesystem],
                ))?;
                Ok(())
            },
        )
        .unwrap();

    let recorded = store.invalidations();
    assert_eq!(
        recorded.len(),
        1,
        "same-trust authority-ceiling reductions must still publish"
    );
    assert_eq!(recorded[0].identity, identity);
    assert_eq!(recorded[0].previous.class(), TrustClass::FirstParty);
    assert_eq!(recorded[0].current.class(), TrustClass::FirstParty);
    assert!(
        recorded[0].authority_ceiling_reduced(),
        "event should explain that the invalidating change was a ceiling reduction"
    );
    assert_eq!(
        recorded[0].previous_authority_ceiling.allowed_effects,
        vec![EffectKind::ReadFilesystem, EffectKind::WriteFilesystem]
    );
    assert_eq!(
        recorded[0].current_authority_ceiling.allowed_effects,
        vec![EffectKind::ReadFilesystem]
    );
}

#[test]
fn t14h_mutate_with_publishes_when_same_trust_lowers_resource_ceiling() {
    let admin = AdminConfig::with_entries([AdminEntry::for_bundled(
        pkg("operator_blessed"),
        None,
        HostTrustAssignment::first_party(),
        vec![EffectKind::ReadFilesystem],
        Some(output_token_ceiling(1_000)),
    )]);
    let policy = policy(vec![Box::new(admin)]);
    let bus = InvalidationBus::new();
    let store = FakeGrantStore::new();
    bus.register(store.clone());

    policy
        .mutate_with(
            &bus,
            bundled_identity("operator_blessed", None),
            caps(&["operator_blessed.read"]),
            RequestedTrustClass::FirstPartyRequested,
            |m| {
                m.admin_upsert(AdminEntry::for_bundled(
                    pkg("operator_blessed"),
                    None,
                    HostTrustAssignment::first_party(),
                    vec![EffectKind::ReadFilesystem],
                    Some(output_token_ceiling(100)),
                ))?;
                Ok(())
            },
        )
        .unwrap();

    let recorded = store.invalidations();
    assert_eq!(
        recorded.len(),
        1,
        "lower resource ceilings must publish even when trust/effects stay equal"
    );
    assert!(recorded[0].authority_ceiling_reduced());
    assert_eq!(
        recorded[0]
            .previous_authority_ceiling
            .max_resource_ceiling
            .as_ref()
            .and_then(|ceiling| ceiling.max_output_tokens),
        Some(1_000)
    );
    assert_eq!(
        recorded[0]
            .current_authority_ceiling
            .max_resource_ceiling
            .as_ref()
            .and_then(|ceiling| ceiling.max_output_tokens),
        Some(100)
    );
}

struct BlockingTrustChangeListener {
    started: Mutex<Option<mpsc::Sender<()>>>,
    finish: Mutex<mpsc::Receiver<()>>,
}

impl TrustChangeListener for BlockingTrustChangeListener {
    fn on_trust_changed(&self, _change: &TrustChange) {
        if let Some(started) = self
            .started
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take()
        {
            started.send(()).unwrap();
        }
        self.finish
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .recv()
            .unwrap();
    }
}

struct ReentrantEvaluateListener {
    policy: Arc<HostTrustPolicy>,
    observed: Mutex<Option<mpsc::Sender<TrustClass>>>,
}

impl TrustChangeListener for ReentrantEvaluateListener {
    fn on_trust_changed(&self, change: &TrustChange) {
        let decision = self
            .policy
            .evaluate(&input(
                change.identity.clone(),
                RequestedTrustClass::FirstPartyRequested,
            ))
            .unwrap();
        if let Some(observed) = self
            .observed
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take()
        {
            observed.send(decision.effective_trust.class()).unwrap();
        }
    }
}

#[test]
fn t14g_evaluate_waits_until_mutation_invalidation_publish_completes() {
    let admin = AdminConfig::with_entries([admin_entry_for_test(
        pkg("operator_blessed"),
        PackageSource::Bundled,
        effective_first_party_for_test(),
        vec![EffectKind::ReadFilesystem],
    )]);
    let policy = Arc::new(policy(vec![Box::new(admin)]));
    let bus = Arc::new(InvalidationBus::new());

    let (started_tx, started_rx) = mpsc::channel();
    let (finish_tx, finish_rx) = mpsc::channel();
    bus.register(Arc::new(BlockingTrustChangeListener {
        started: Mutex::new(Some(started_tx)),
        finish: Mutex::new(finish_rx),
    }));

    let mutate_policy = policy.clone();
    let mutate_bus = bus.clone();
    let mutator = std::thread::spawn(move || {
        mutate_policy
            .mutate_with(
                &mutate_bus,
                bundled_identity("operator_blessed", None),
                caps(&["operator_blessed.read"]),
                RequestedTrustClass::FirstPartyRequested,
                |m| {
                    m.admin_remove(&pkg("operator_blessed"), &PackageSource::Bundled)?;
                    Ok(())
                },
            )
            .unwrap();
    });

    started_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("mutation should reach synchronous publish");

    let (eval_tx, eval_rx) = mpsc::channel();
    let eval_policy = policy.clone();
    let evaluator = std::thread::spawn(move || {
        let decision = eval_policy
            .evaluate(&input(
                bundled_identity("operator_blessed", None),
                RequestedTrustClass::FirstPartyRequested,
            ))
            .unwrap();
        eval_tx.send(decision).unwrap();
    });

    assert!(
        eval_rx.recv_timeout(Duration::from_millis(200)).is_err(),
        "evaluate returned while invalidation publish was still blocked; \
         the downgraded decision became visible before grants were invalidated"
    );

    finish_tx.send(()).unwrap();
    mutator.join().unwrap();

    let decision = eval_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("evaluate should complete after publish returns");
    evaluator.join().unwrap();
    assert_eq!(decision.effective_trust.class(), TrustClass::Sandbox);
}

#[test]
fn t14i_trust_change_listener_can_reenter_evaluate_without_deadlock() {
    let admin = AdminConfig::with_entries([admin_entry_for_test(
        pkg("operator_blessed"),
        PackageSource::Bundled,
        effective_first_party_for_test(),
        vec![EffectKind::ReadFilesystem],
    )]);
    let policy = Arc::new(policy(vec![Box::new(admin)]));
    let bus = InvalidationBus::new();
    let (observed_tx, observed_rx) = mpsc::channel();
    bus.register(Arc::new(ReentrantEvaluateListener {
        policy: policy.clone(),
        observed: Mutex::new(Some(observed_tx)),
    }));

    let identity = bundled_identity("operator_blessed", None);
    policy
        .mutate_with(
            &bus,
            identity,
            caps(&["operator_blessed.read"]),
            RequestedTrustClass::FirstPartyRequested,
            |m| {
                m.admin_remove(&pkg("operator_blessed"), &PackageSource::Bundled)?;
                Ok(())
            },
        )
        .unwrap();

    assert_eq!(
        observed_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("listener should re-enter evaluate without deadlocking"),
        TrustClass::Sandbox
    );
}

// ---------------------------------------------------------------------------
// T15 — `default_decision` is fail-closed for every PackageSource.
//
// Regression for the "default_decision for unmatched Bundled / Registry is
// UserTrusted" review finding. Pre-fix, an unmatched Bundled / Registry /
// Admin package picked up `UserTrusted` purely on the basis of its origin
// string — fail-open, since `SignedRegistry` is inert and "Bundled but
// not in the registry" is a host-config bug rather than a third-party
// credential. Post-fix, all unmatched origins drop to `Sandbox`.
//
// This test pins the contract at the public-API level so a future change
// re-introducing the soft fallback fails loudly.
// ---------------------------------------------------------------------------

#[test]
fn t15_default_decision_is_sandbox_for_every_unmatched_package_source() {
    // Empty policy chain — every input falls through to `default_decision`.
    let policy = policy(vec![
        Box::new(BundledRegistry::new()),
        Box::new(AdminConfig::new()),
    ]);

    let unmatched_origins = [
        // Already correct pre-fix; included so the test asserts the full
        // matrix uniformly.
        local_manifest_identity("any_local"),
        bundled_identity("missing_from_bundled_registry", None),
        // Most security-critical case — an unverified remote package
        // must NOT pick up UserTrusted just because it carries a
        // `Registry { url }` origin tag.
        PackageIdentity::new(
            pkg("untrusted_remote_pkg"),
            PackageSource::Registry {
                url: "https://anywhere.evil/registry".to_string(),
            },
            None,
            None,
        ),
        PackageIdentity::new(pkg("orphaned_admin_pkg"), PackageSource::Admin, None, None),
    ];

    for identity in unmatched_origins {
        let decision = policy
            .evaluate(&input(
                identity.clone(),
                RequestedTrustClass::SystemRequested,
            ))
            .unwrap();
        assert_eq!(
            decision.effective_trust.class(),
            TrustClass::Sandbox,
            "unmatched {:?} must fall to Sandbox, not UserTrusted",
            identity.source
        );
        assert_eq!(decision.provenance, TrustProvenance::Default);
        assert!(
            decision.authority_ceiling.allowed_effects.is_empty(),
            "default decision must carry an empty authority ceiling"
        );
    }
}

// ---------------------------------------------------------------------------
// T16 — `TrustChange` no-op / downgrade / upgrade / kind-change semantics.
//
// Regression for the "TrustChange with previous == current is publishable"
// review finding. Listeners coding the naive pattern "any TrustChange
// fired ⇒ revoke grants" would over-revoke on benign upgrades and
// no-ops. The fix layers three guards:
//
//   1. `TrustChange::new` returns `None` for no-ops at construction.
//   2. `InvalidationBus::publish` drops no-ops as defense-in-depth (and
//      `debug_assert!`s in dev so the offending caller is loud).
//   3. `is_downgrade` / `is_upgrade` / `is_kind_change` helpers let
//      sophisticated listeners gate behavior precisely.
// ---------------------------------------------------------------------------

#[test]
fn t16_trust_change_new_filters_no_ops() {
    let identity = bundled_identity("any", None);
    let trust = EffectiveTrustClass::user_trusted();
    assert!(
        trust_change_for_test(identity, trust, trust).is_none(),
        "TrustChange::new must return None when previous == current"
    );
}

#[test]
fn t16b_trust_change_classifies_downgrade_upgrade_and_kind_change() {
    let identity = bundled_identity("any", None);
    let sandbox = EffectiveTrustClass::sandbox();
    let user_trusted = EffectiveTrustClass::user_trusted();
    let first_party = effective_first_party_for_test();
    let system = effective_system_for_test();

    // Downgrade: FirstParty → UserTrusted.
    let down = trust_change_for_test(identity.clone(), first_party, user_trusted).unwrap();
    assert!(down.is_downgrade());
    assert!(!down.is_upgrade());
    assert!(!down.is_kind_change());

    // Upgrade: Sandbox → UserTrusted.
    let up = trust_change_for_test(identity.clone(), sandbox, user_trusted).unwrap();
    assert!(!up.is_downgrade());
    assert!(up.is_upgrade());
    assert!(!up.is_kind_change());

    // Kind change: FirstParty ↔ System (both privileged, level 2, but
    // semantically different privilege kinds).
    let kind = trust_change_for_test(identity, first_party, system).unwrap();
    assert!(!kind.is_downgrade());
    assert!(!kind.is_upgrade());
    assert!(
        kind.is_kind_change(),
        "FirstParty ↔ System is a kind change — different privilege classes \
         at the same authority level"
    );
}

#[test]
fn t16c_invalidation_bus_publish_drops_no_op_struct_literal_in_release() {
    // `TrustChange::new` is the recommended path, but a hand-built
    // struct literal can still produce a no-op. Release builds must
    // silently drop it from the bus rather than fanning out to
    // listeners. (Debug builds trip a `debug_assert!` — that path is
    // tested in `cfg(debug_assertions)` builds via cargo test default.)
    //
    // We can only assert the release-mode behavior portably here, so
    // the test gates the assertion on `cfg(not(debug_assertions))` to
    // avoid the debug-mode panic. The debug-mode panic itself is the
    // intent — assert that *no listener fired* in release-mode.
    if cfg!(debug_assertions) {
        return;
    }
    let bus = InvalidationBus::new();
    let store = FakeGrantStore::new();
    bus.register(store.clone());

    let identity = bundled_identity("any", None);
    let same = EffectiveTrustClass::user_trusted();
    let no_op = TrustChange {
        identity,
        previous: same,
        current: same,
        previous_authority_ceiling: crate::AuthorityCeiling::empty(),
        current_authority_ceiling: crate::AuthorityCeiling::empty(),
        effective_at: Utc::now(),
    };
    bus.publish(no_op);

    assert!(
        store.invalidations().is_empty(),
        "no-op TrustChange must not fan out to listeners in release builds"
    );
}

// ---------------------------------------------------------------------------
// T17 — `LocalDevOverride` stays inert even when enabled and pre-staged.
//
// PR1b ships `LocalDevOverride` as a forward-compat seam: the type and
// `PolicySource` impl exist, but the implementation is intentionally
// non-functional until a future PR wires up explicit operator opt-in +
// audit logging. The `enabled_for_test` fixture lets the test suite
// exercise that contract — enabling the source and staging overrides
// must NOT produce trust matches.
// ---------------------------------------------------------------------------

#[test]
fn t17_local_dev_override_remains_inert_even_when_enabled_and_staged() {
    let staged = LocalDevOverride::enabled_for_test(vec![(
        pkg("dev_pkg"),
        admin_entry_for_test(
            pkg("dev_pkg"),
            PackageSource::LocalManifest {
                path: "/extensions/dev_pkg/manifest.toml".to_string(),
            },
            effective_first_party_for_test(),
            vec![EffectKind::ReadFilesystem],
        ),
    )]);
    assert!(
        staged.is_enabled(),
        "fixture must produce an enabled instance"
    );
    assert_eq!(
        staged.override_count(),
        1,
        "fixture must surface the staged entry through override_count"
    );

    // Even though `staged` is enabled and has an override that *would*
    // elevate `dev_pkg` to FirstParty if the seam were live, evaluate
    // must return Ok(None). The dev-override implementation is not in
    // PR1b — the inert contract pins that promise.
    let probe = TrustPolicyInput {
        identity: PackageIdentity::new(
            pkg("dev_pkg"),
            PackageSource::LocalManifest {
                path: "/extensions/dev_pkg/manifest.toml".to_string(),
            },
            None,
            None,
        ),
        requested_trust: RequestedTrustClass::FirstPartyRequested,
        requested_authority: BTreeSet::new(),
    };
    assert!(
        staged.evaluate(&probe).unwrap().is_none(),
        "LocalDevOverride must remain inert until the dev-override \
         implementation lands — a future PR is what flips this assertion"
    );
}

// ---------------------------------------------------------------------------
// T18 — `EffectiveTrustClass` audit wire shape across all four variants.
//
// The crate's serialization output is part of the audit contract — a
// rename of any underlying `TrustClass` variant would silently change
// what audit envelopes record. The existing
// `trust_decision_serializes_for_audit` smoke test only exercises
// `Sandbox`; this test pins all four wire strings, including the
// privileged variants reachable only from crate-internal test fixtures.
// ---------------------------------------------------------------------------

#[test]
fn t18_effective_trust_class_serializes_to_canonical_wire_strings() {
    use serde_json::json;
    assert_eq!(
        serde_json::to_value(EffectiveTrustClass::sandbox()).unwrap(),
        json!("sandbox")
    );
    assert_eq!(
        serde_json::to_value(EffectiveTrustClass::user_trusted()).unwrap(),
        json!("user_trusted")
    );
    assert_eq!(
        serde_json::to_value(effective_first_party_for_test()).unwrap(),
        json!("first_party"),
        "first_party wire string must remain stable for audit log compatibility"
    );
    assert_eq!(
        serde_json::to_value(effective_system_for_test()).unwrap(),
        json!("system"),
        "system wire string must remain stable for audit log compatibility"
    );
}

// ---------------------------------------------------------------------------
// Sanity / smoke: TrustDecision serializes for audit.
// ---------------------------------------------------------------------------

#[test]
fn trust_decision_serializes_for_audit() {
    let decision = TrustDecision {
        effective_trust: EffectiveTrustClass::sandbox(),
        authority_ceiling: crate::AuthorityCeiling::empty(),
        provenance: TrustProvenance::Default,
        evaluated_at: Utc::now(),
    };
    let value = serde_json::to_value(&decision).unwrap();
    assert_eq!(value["effective_trust"], serde_json::json!("sandbox"));
    assert_eq!(value["provenance"]["kind"], serde_json::json!("default"));
}

// ---------------------------------------------------------------------------
// Clock determinism — `HostTrustPolicy::with_clock` makes evaluation
// reproducible, removing nondeterminism from a security-critical path.
// ---------------------------------------------------------------------------

#[test]
fn evaluate_uses_injected_clock_for_evaluated_at() {
    use crate::FixedClock;
    use chrono::TimeZone;

    let frozen = chrono::Utc.with_ymd_and_hms(2026, 4, 28, 12, 0, 0).unwrap();
    let policy = HostTrustPolicy::with_clock(
        vec![Box::new(BundledRegistry::new())],
        Box::new(FixedClock::new(frozen)),
    )
    .unwrap();

    let identity = bundled_identity("ironclaw_core", None);
    let first = policy
        .evaluate(&input(identity.clone(), RequestedTrustClass::ThirdParty))
        .unwrap();
    let second = policy
        .evaluate(&input(identity, RequestedTrustClass::ThirdParty))
        .unwrap();

    assert_eq!(first.evaluated_at, frozen);
    assert_eq!(second.evaluated_at, frozen);
    assert_eq!(first.evaluated_at, second.evaluated_at);
}
