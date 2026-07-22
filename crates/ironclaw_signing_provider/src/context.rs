//! Identity newtypes and the [`SigningContext`] carried through every signing
//! flow.
//!
//! ## Reconciliation note (PR5)
//!
//! The identity newtypes here ([`TenantId`], [`UserId`], [`ScopeId`],
//! [`ActorId`], [`RunId`], [`ChainId`], [`KeyOrAccountId`]) are defined
//! locally rather than imported from `ironclaw_turns`
//! (`GateRef`, `ResourceScope`) or `ironclaw_host_api`. Those types live in
//! crates that will depend on *this* crate in PR5 (`ironclaw_turns` gains
//! `BlockedReason::Attested` and an injected `AttestedResumePort`). Importing
//! them here would create a dependency cycle and would violate this crate's
//! chain/crypto-free purity invariant. The conservative choice for PR1 is
//! minimal local newtypes; they will be reconciled against the canonical
//! `ironclaw_turns` identity vocabulary in PR5 once the dependency direction
//! is fixed.
//!
//! The `gate_ref` value uses [`GateRef`], a local minimal newtype mirroring
//! `ironclaw_turns::GateRef`'s wire shape (transparent string).

use serde::{Deserialize, Serialize};

/// Macro for a transparent, string-backed identity newtype.
///
/// These are deliberately validation-light at this layer: the trait crate only
/// needs to *name* identities and round-trip them on the wire. Domain
/// validation (charset, length, prefix rules) lives with the canonical
/// `ironclaw_turns` types these reconcile with in PR5.
macro_rules! string_newtype {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            /// Construct from any string-like value.
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            /// Borrow the underlying string.
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

string_newtype! {
    /// Tenant boundary identity (multi-tenant isolation key).
    TenantId
}

string_newtype! {
    /// End-user identity within a tenant.
    UserId
}

string_newtype! {
    /// Authorization scope identity.
    ///
    /// Reconciles with `ironclaw_turns`/`ironclaw_host_api` `ResourceScope` in
    /// PR5.
    ScopeId
}

string_newtype! {
    /// Acting principal identity (the agent run actor on whose behalf the
    /// signing flow runs).
    ActorId
}

string_newtype! {
    /// Run identity that the signing flow belongs to.
    RunId
}

string_newtype! {
    /// Gate reference the signing flow is blocked on.
    ///
    /// Mirrors the wire shape of `ironclaw_turns::GateRef`; reconciled in PR5.
    GateRef
}

string_newtype! {
    /// Chain / network identity (e.g. an EVM chain id, a Solana cluster, or a
    /// NEAR network). Kept as an opaque string at this layer — the chain crates
    /// own the concrete encoding.
    ChainId
}

string_newtype! {
    /// The signing key or account identity bound to the request (an EVM
    /// address, a Solana pubkey, or a NEAR account id). Opaque at this layer.
    KeyOrAccountId
}

/// Everything an attested-signing flow needs to identify *who* is signing
/// *what*, on *which* chain, under *which* gate.
///
/// Every field is a strong newtype so two same-shaped identities cannot be
/// confused (see `.claude/rules/types.md`). The grant key in
/// `ironclaw_attestation` (PR3) is derived from this context plus the
/// approved-tx hash.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SigningContext {
    /// Tenant boundary.
    pub tenant: TenantId,
    /// End user.
    pub user: UserId,
    /// Authorization scope.
    pub scope: ScopeId,
    /// Acting principal.
    pub actor: ActorId,
    /// Owning run.
    pub run_id: RunId,
    /// Gate the flow is blocked on.
    pub gate_ref: GateRef,
    /// Target chain / network.
    pub chain_id: ChainId,
    /// Signing key or account.
    pub key_or_account_id: KeyOrAccountId,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn string_newtype_round_trips_through_serde_transparently() {
        let id = UserId::new("user-123");
        let json = serde_json::to_string(&id).expect("serialize");
        assert_eq!(json, "\"user-123\"");
        let back: UserId = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, id);
        assert_eq!(back.as_str(), "user-123");
    }

    #[test]
    fn signing_context_round_trips() {
        let ctx = SigningContext {
            tenant: TenantId::new("tenant-a"),
            user: UserId::new("user-1"),
            scope: ScopeId::new("scope-x"),
            actor: ActorId::new("actor-7"),
            run_id: RunId::new("run-42"),
            gate_ref: GateRef::new("gate:abc"),
            chain_id: ChainId::new("eip155:1"),
            key_or_account_id: KeyOrAccountId::new("0xabc"),
        };
        let json = serde_json::to_string(&ctx).expect("serialize");
        let back: SigningContext = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, ctx);
    }

    #[test]
    fn display_matches_inner_string() {
        assert_eq!(ChainId::new("solana:mainnet").to_string(), "solana:mainnet");
    }
}
