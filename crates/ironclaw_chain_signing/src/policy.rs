//! Injectable custody / bootstrap policy hooks with conservative deny-first
//! defaults.
//!
//! Several governance questions are deliberately left open by the
//! attested-signing plan (see `docs/plans/2026-05-23-attested-signing-substrate.md`,
//! "Open Questions"):
//!
//! * the trust anchor for the **first** custodial key bootstrap,
//! * **key rotation**,
//! * **custody recovery / backup**,
//! * the exact **HSM/KMS mainnet threshold**.
//!
//! Rather than hardcode an answer, these are surfaced as injectable traits so
//! the composition layer can supply a real policy later. The crate ships
//! deny-first defaults: bootstrap of a brand-new key is **denied** unless an
//! explicit policy allows it, and custody operations default to the most
//! conservative outcome. This keeps the security ambiguity fail-closed.

use crate::chain::ChainKeyId;
use ironclaw_signing_provider::SigningContext;

/// Decision returned by a custody policy hook.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CustodyDecision {
    /// The operation is permitted.
    Allow,
    /// The operation is denied with a human-readable reason.
    Deny {
        /// Why the operation was denied (never key material).
        reason: String,
    },
}

impl CustodyDecision {
    /// Whether the decision permits the operation.
    pub fn is_allowed(&self) -> bool {
        matches!(self, CustodyDecision::Allow)
    }
}

/// Policy hook governing **first-key bootstrap** — the trust anchor question.
///
/// Invoked before a custodial key is first created/bound for a `(user, chain)`.
/// The default ([`DenyFirstBootstrapPolicy`]) denies, forcing an explicit
/// operator decision rather than silently minting custody.
pub trait BootstrapPolicy: Send + Sync {
    /// Decide whether a brand-new custodial key may be bootstrapped for the
    /// given context and chain.
    fn authorize_bootstrap(&self, ctx: &SigningContext, chain: &ChainKeyId) -> CustodyDecision;
}

/// Policy hook governing ongoing custody operations: signing authorization
/// beyond the grant/ledger machinery, rotation, and recovery.
///
/// The default ([`DenyFirstCustodyPolicy`]) allows signing (the grant, ledger,
/// and sign-time hash re-check already gate it) but denies rotation and
/// recovery, which are open questions with no safe default.
pub trait KeyCustodyPolicy: Send + Sync {
    /// Decide whether a signing operation may proceed for a bound key. This is
    /// an *additional* hook on top of the grant claim and sign-time hash
    /// re-check; it exists so a deployment can layer extra custody rules.
    fn authorize_sign(&self, ctx: &SigningContext, chain: &ChainKeyId) -> CustodyDecision;

    /// Decide whether key rotation is permitted. Default impls deny.
    fn authorize_rotation(&self, ctx: &SigningContext, chain: &ChainKeyId) -> CustodyDecision;

    /// Decide whether custody recovery / backup export is permitted. Default
    /// impls deny.
    fn authorize_recovery(&self, ctx: &SigningContext, chain: &ChainKeyId) -> CustodyDecision;
}

/// Conservative default [`BootstrapPolicy`]: denies all first-key bootstrap.
///
/// Forces the deployment to wire a real bootstrap trust anchor before any
/// custodial key can be created — the safest answer to the open trust-anchor
/// question.
#[derive(Debug, Default, Clone, Copy)]
pub struct DenyFirstBootstrapPolicy;

impl BootstrapPolicy for DenyFirstBootstrapPolicy {
    fn authorize_bootstrap(&self, _ctx: &SigningContext, _chain: &ChainKeyId) -> CustodyDecision {
        CustodyDecision::Deny {
            reason: "first-key bootstrap requires an explicit BootstrapPolicy; the default \
                     deny-first policy refuses to mint custody (open question: trust anchor)"
                .to_string(),
        }
    }
}

/// A bootstrap policy that allows bootstrap unconditionally. Intended for
/// tests and trusted single-operator dev setups only — never the production
/// default.
#[derive(Debug, Default, Clone, Copy)]
pub struct AllowBootstrapPolicy;

impl BootstrapPolicy for AllowBootstrapPolicy {
    fn authorize_bootstrap(&self, _ctx: &SigningContext, _chain: &ChainKeyId) -> CustodyDecision {
        CustodyDecision::Allow
    }
}

/// Conservative default [`KeyCustodyPolicy`]: signing is allowed (gated by the
/// grant + ledger + hash re-check), rotation and recovery are denied.
#[derive(Debug, Default, Clone, Copy)]
pub struct DenyFirstCustodyPolicy;

impl KeyCustodyPolicy for DenyFirstCustodyPolicy {
    fn authorize_sign(&self, _ctx: &SigningContext, _chain: &ChainKeyId) -> CustodyDecision {
        CustodyDecision::Allow
    }

    fn authorize_rotation(&self, _ctx: &SigningContext, _chain: &ChainKeyId) -> CustodyDecision {
        CustodyDecision::Deny {
            reason: "key rotation requires an explicit KeyCustodyPolicy (open question)"
                .to_string(),
        }
    }

    fn authorize_recovery(&self, _ctx: &SigningContext, _chain: &ChainKeyId) -> CustodyDecision {
        CustodyDecision::Deny {
            reason: "custody recovery/backup requires an explicit KeyCustodyPolicy (open question)"
                .to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_signing_provider::{
        ActorId, ChainId, GateRef, KeyOrAccountId, RunId, ScopeId, TenantId, UserId,
    };

    fn ctx() -> SigningContext {
        SigningContext {
            tenant: TenantId::new("t"),
            user: UserId::new("u"),
            scope: ScopeId::new("s"),
            actor: ActorId::new("a"),
            run_id: RunId::new("r"),
            gate_ref: GateRef::new("g"),
            chain_id: ChainId::new("eip155:1"),
            key_or_account_id: KeyOrAccountId::new("0xabc"),
        }
    }

    #[test]
    fn default_bootstrap_is_deny_first() {
        let decision = DenyFirstBootstrapPolicy
            .authorize_bootstrap(&ctx(), &ChainKeyId::new("eip155:1").unwrap());
        assert!(!decision.is_allowed());
    }

    #[test]
    fn default_custody_allows_sign_denies_rotation_and_recovery() {
        let p = DenyFirstCustodyPolicy;
        let c = ChainKeyId::new("eip155:1").unwrap();
        assert!(p.authorize_sign(&ctx(), &c).is_allowed());
        assert!(!p.authorize_rotation(&ctx(), &c).is_allowed());
        assert!(!p.authorize_recovery(&ctx(), &c).is_allowed());
    }
}
