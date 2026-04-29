//! Host-controlled trust-class policy engine for IronClaw Reborn.
//!
//! `ironclaw_trust` is the bridge between the *requested* trust an untrusted
//! manifest declares and the *effective* trust ceiling that downstream
//! authorization consumes. The crate enforces three invariants:
//!
//! 1. **Effective trust is host-policy-only.** [`EffectiveTrustClass::FirstParty`]
//!    and [`EffectiveTrustClass::System`] are constructible only from inside
//!    this crate. A user-installed manifest cannot fabricate a privileged
//!    ceiling, even by deserializing into a wire type and calling a public
//!    constructor.
//! 2. **Trust is an authority *ceiling*, not a grant.** [`TrustDecision`]
//!    returns an [`AuthorityCeiling`] enumerating *what may be granted*;
//!    capability invocation still requires an explicit `CapabilityGrant`.
//! 3. **Trust changes invalidate active grants.** A trust downgrade,
//!    revocation, or authority-ceiling reduction publishes a [`TrustChange`]
//!    on the [`InvalidationBus`] synchronously, before any subsequent
//!    dispatch can produce a side effect under the stale ceiling. Runtime
//!    mutation goes through
//!    [`HostTrustPolicy::mutate_with`], which hard-wires the
//!    pre-evaluate / stage / commit / post-evaluate / publish dance so AC #6 is
//!    a compile-time guarantee — the per-source `upsert` / `remove`
//!    methods are `pub(crate)` and only reachable through
//!    [`SourceMutators`] inside a `mutate_with` closure.
//!
//! See `crates/ironclaw_trust/CONTRACT.md` for the full cross-crate
//! contract (evaluation matrix, `PackageIdentity` scope, mutation
//! orchestration, built-in tool migration intent), `CLAUDE.md` for the
//! per-file guardrails, and `docs/reborn/contracts/host-api.md` (in the
//! staging-track docs) for the broader Reborn vocabulary.

pub mod clock;
pub mod decision;
pub mod error;
pub mod invalidation;
pub mod policy;
pub mod sources;

#[cfg(test)]
mod fixtures;

pub use clock::{Clock, FixedClock, SystemClock};
pub use decision::{
    AuthorityCeiling, EffectiveTrustClass, HostTrustAssignment, TrustDecision, TrustProvenance,
};
pub use error::TrustError;
pub use invalidation::{
    InvalidationBus, TrustChange, TrustChangeListener, authority_changed, grant_retention_eligible,
    identity_changed,
};
pub use policy::{HostTrustPolicy, SourceMatch, SourceMutators, TrustPolicy, TrustPolicyInput};
pub use sources::{
    AdminConfig, AdminEntry, BundledEntry, BundledRegistry, LocalDevOverride, PolicySource,
    SignedRegistry, SignerEntry,
};

#[cfg(test)]
mod tests {
    //! Lib-level smoke tests that run on bare `cargo test -p ironclaw_trust`.
    //! The full contract suite lives in `policy_contract_tests` below. If
    //! this module is empty, anyone running the bare command sees `0 passed`
    //! and might think nothing exercised the crate — which would be misleading.
    use super::*;

    #[test]
    fn public_effective_trust_constructors_are_non_privileged() {
        assert!(!EffectiveTrustClass::sandbox().is_privileged());
        assert!(!EffectiveTrustClass::user_trusted().is_privileged());
    }

    #[test]
    fn empty_policy_returns_default_for_local_manifest() {
        use ironclaw_host_api::{PackageId, PackageIdentity, PackageSource, RequestedTrustClass};
        let policy = HostTrustPolicy::empty();
        let identity = PackageIdentity::new(
            PackageId::new("any").unwrap(),
            PackageSource::LocalManifest {
                path: "/tmp/manifest.toml".to_string(),
            },
            None,
            None,
        );
        let decision = policy
            .evaluate(&TrustPolicyInput {
                identity,
                requested_trust: RequestedTrustClass::SystemRequested,
                requested_authority: std::collections::BTreeSet::new(),
            })
            .unwrap();
        assert!(!decision.effective_trust.is_privileged());
        assert_eq!(decision.provenance, TrustProvenance::Default);
    }
}

#[cfg(test)]
#[path = "tests/policy_contract.rs"]
mod policy_contract_tests;
