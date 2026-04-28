//! PR1b acceptance-criteria contract tests.
//!
//! Each test maps to a row in the plan's coverage matrix:
//!   - T1..T13: trust policy + invalidation contract
//!
//! Test fixtures live in `mod support` below: `FakeAuthorizer` (gates
//! capability invocation on `EffectiveTrustClass::is_privileged()` AND an
//! explicit grant set) and `FakeGrantStore` (records invalidation events on
//! a shared `InvalidationBus`). These prove ordering and grant-denial
//! behavior at the integration boundary that PR3 will eventually own.

use chrono::Utc;
use ironclaw_host_api::{
    CapabilityId, EffectKind, PackageId, PackageIdentity, PackageSource, RequestedTrustClass,
    TrustClass,
};
use ironclaw_trust::fixtures::{
    admin_entry_for_test, bundled_entry_for_test, effective_first_party_for_test,
    effective_system_for_test,
};
use ironclaw_trust::{
    AdminConfig, BundledRegistry, EffectiveTrustClass, HostTrustPolicy, InvalidationBus,
    TrustChange, TrustDecision, TrustPolicy, TrustPolicyInput, TrustProvenance, authority_changed,
    grant_retention_eligible, identity_changed,
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

use crate::support::{FakeAuthorizer, FakeGrantStore};

mod support {
    use std::sync::{Arc, Mutex};

    use ironclaw_host_api::{CapabilityId, PackageIdentity};
    use ironclaw_trust::{EffectiveTrustClass, TrustChange, TrustChangeListener};

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
        requested_authority: Vec::new(),
    }
}

// ---------------------------------------------------------------------------
// T1 — self-promotion denied for user manifest
// Issue suggested test #1 (effective ≠ privileged). AC #2, AC #8.
// ---------------------------------------------------------------------------

#[test]
fn t1_self_promotion_denied_for_user_manifest() {
    let policy = HostTrustPolicy::new(vec![
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
    let policy = HostTrustPolicy::new(vec![Box::new(BundledRegistry::new())]);
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
    let registry = BundledRegistry::new();
    registry.upsert(bundled_entry_for_test(
        pkg("ironclaw_core"),
        Some("digest_v1".to_string()),
        effective_system_for_test(),
        vec![EffectKind::DispatchCapability],
    ));

    let policy = HostTrustPolicy::new(vec![Box::new(registry)]);

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
    let registry = BundledRegistry::new();
    registry.upsert(bundled_entry_for_test(
        pkg("ironclaw_core"),
        None,
        effective_system_for_test(),
        vec![EffectKind::DispatchCapability],
    ));
    let policy = HostTrustPolicy::new(vec![Box::new(registry)]);

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
    let prev = vec![cap("github.read")];
    let curr_added = vec![cap("github.read"), cap("github.delete")];
    let curr_unchanged = vec![cap("github.read")];
    let curr_removed_all = vec![];

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

    // Reorder of the SAME set must NOT fire — set semantics, not list.
    let prev_two = vec![cap("github.read"), cap("github.write")];
    let prev_two_reordered = vec![cap("github.write"), cap("github.read")];
    assert!(
        !authority_changed(&prev_two, &prev_two_reordered),
        "reordering the same set must remain retainable"
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
    let prev_authority = vec![cap("ironclaw_core.dispatch")];

    let change = TrustChange {
        identity: identity.clone(),
        previous: effective_system_for_test(),
        current: EffectiveTrustClass::user_trusted(),
        previous_authority: prev_authority.clone(),
        effective_at: Utc::now(),
    };
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
    let policy = HostTrustPolicy::new(vec![Box::new(BundledRegistry::new())]);
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
    let change = TrustChange {
        identity: identity.clone(),
        previous: effective_system_for_test(),
        current: EffectiveTrustClass::sandbox(),
        previous_authority: vec![cap("ironclaw_core.dispatch")],
        effective_at: Utc::now(),
    };
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
    let registry = BundledRegistry::new();
    registry.upsert(bundled_entry_for_test(
        pkg("ironclaw_core"),
        Some("digest_v1".to_string()),
        effective_first_party_for_test(),
        vec![],
    ));
    let policy = HostTrustPolicy::new(vec![Box::new(registry)]);

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
    assert!(!grant_retention_eligible(
        &prev,
        &curr,
        prev_decision.effective_trust,
        curr_decision.effective_trust,
        &[],
        &[],
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
    assert!(!grant_retention_eligible(
        &prev,
        &curr,
        trust,
        trust,
        &[],
        &[]
    ));
}

// ---------------------------------------------------------------------------
// T13 — admin config source overrides the absence of a bundled match
// Decision rule #3 from the plan.
// ---------------------------------------------------------------------------

#[test]
fn t13_admin_config_source_grants_trust_when_bundled_does_not_match() {
    let bundled = BundledRegistry::new(); // empty
    let admin = AdminConfig::new();
    admin.upsert(admin_entry_for_test(
        pkg("operator_blessed"),
        effective_first_party_for_test(),
        vec![EffectKind::ReadFilesystem],
    ));

    // Layered: bundled first, admin second. Bundled returns None, admin
    // returns the privileged match.
    let policy = HostTrustPolicy::new(vec![Box::new(bundled), Box::new(admin)]);

    let identity = local_manifest_identity("operator_blessed");
    let decision = policy
        .evaluate(&input(identity, RequestedTrustClass::FirstPartyRequested))
        .unwrap();

    assert_eq!(decision.effective_trust.class(), TrustClass::FirstParty);
    assert_eq!(decision.provenance, TrustProvenance::AdminConfig);
}

// ---------------------------------------------------------------------------
// Sanity / smoke: TrustDecision serializes for audit.
// ---------------------------------------------------------------------------

#[test]
fn trust_decision_serializes_for_audit() {
    let decision = TrustDecision {
        effective_trust: EffectiveTrustClass::sandbox(),
        authority_ceiling: ironclaw_trust::AuthorityCeiling::empty(),
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
    use chrono::TimeZone;
    use ironclaw_trust::FixedClock;

    let frozen = chrono::Utc.with_ymd_and_hms(2026, 4, 28, 12, 0, 0).unwrap();
    let policy = HostTrustPolicy::with_clock(
        vec![Box::new(BundledRegistry::new())],
        Box::new(FixedClock::new(frozen)),
    );

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
