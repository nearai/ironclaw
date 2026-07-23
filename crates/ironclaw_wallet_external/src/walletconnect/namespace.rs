//! CAIP-2 chain-family resolution and WalletConnect v2 session-namespace
//! pinning.
//!
//! The attested-signing gate has *already* decided exactly one chain
//! ([`SigningContext::chain_id`](ironclaw_signing_provider::SigningContext)) and
//! exactly one signing operation. A WalletConnect v2 session, by contrast,
//! negotiates an arbitrarily-broad set of CAIP-2 chains, RPC methods, and events
//! between dapp and wallet. If we let the wallet (or a compromised relay) settle
//! a session whose scope is broader than the gate's single bound operation, a
//! later request could sign a *different* chain or call a *different* method
//! than the human approved — threats **T17** (chain/method scope broadening) and
//! **T19** (multi-chain session reuse).
//!
//! This module derives the *single* CAIP-2 chain + *single* signing method the
//! gate authorizes, and validates any proposed/settled session scope against it,
//! rejecting fail-closed with
//! [`SigningProviderError::ScopeViolation`](ironclaw_signing_provider::SigningProviderError::ScopeViolation)
//! on any superset.

use ironclaw_signing_provider::{ChainId, SigningProviderError};

/// A parsed CAIP-2 chain id (`namespace:reference`).
///
/// Parsed via a real CAIP-2 grammar check (not a bare `split_once`): exactly one
/// `:`, a non-empty `[-a-z0-9]{3,8}` namespace, and a non-empty
/// `[-_a-zA-Z0-9]{1,32}` reference. Anything else is rejected fail-closed so a
/// malformed/relay-supplied chain id can never be coerced into a family.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Caip2ChainId {
    /// The full `namespace:reference` string.
    full: String,
    /// Byte length of the namespace (the part before the single `:`).
    ns_len: usize,
}

impl Caip2ChainId {
    /// Parse and validate a CAIP-2 chain id.
    pub fn parse(s: &str) -> Result<Self, SigningProviderError> {
        let viol = |reason: String| SigningProviderError::ScopeViolation { reason };
        let (ns, reference) = s.split_once(':').ok_or_else(|| {
            viol(format!(
                "chain id `{s}` is not a CAIP-2 `namespace:reference`"
            ))
        })?;
        // Reject a second `:` (a bare split_once would silently accept it).
        if reference.contains(':') {
            return Err(viol(format!("chain id `{s}` has more than one `:`")));
        }
        if !(3..=8).contains(&ns.len())
            || !ns
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        {
            return Err(viol(format!(
                "chain id `{s}` has an invalid CAIP-2 namespace"
            )));
        }
        if !(1..=32).contains(&reference.len())
            || !reference
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return Err(viol(format!(
                "chain id `{s}` has an invalid CAIP-2 reference"
            )));
        }
        Ok(Self {
            full: s.to_string(),
            ns_len: ns.len(),
        })
    }

    /// The CAIP-2 namespace (the part before the `:`), e.g. `eip155`.
    pub fn namespace(&self) -> &str {
        &self.full[..self.ns_len]
    }

    /// The full `namespace:reference` string.
    pub fn as_str(&self) -> &str {
        &self.full
    }
}

/// A parsed CAIP-10 account id (`namespace:reference:address` =
/// `caip2_chain:address`).
///
/// Binds an account to the EXACT chain it lives on. Parsing settled accounts as
/// typed CAIP-10 (rather than treating them as raw strings) lets the verifier
/// prove the WC account is `eip155:1:<addr>` and not the same address on another
/// chain (#2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Caip10Account {
    /// The CAIP-2 chain id the account lives on.
    chain: Caip2ChainId,
    /// The account address component (chain-specific; normalized only for the
    /// signer comparison, not here).
    address: String,
}

impl Caip10Account {
    /// Parse and validate a CAIP-10 account id `caip2_chain:address`.
    pub fn parse(s: &str) -> Result<Self, SigningProviderError> {
        let viol = |reason: String| SigningProviderError::ScopeViolation { reason };
        // CAIP-10 = `namespace:reference:account_address`. Split off the address
        // (last `:` segment); the remainder must be a valid CAIP-2 chain id.
        let (chain_part, address) = s
            .rsplit_once(':')
            .ok_or_else(|| viol(format!("account `{s}` is not a CAIP-10 `chain:address`")))?;
        if address.is_empty() {
            return Err(viol(format!(
                "account `{s}` has an empty address component"
            )));
        }
        let chain = Caip2ChainId::parse(chain_part)?;
        Ok(Self {
            chain,
            address: address.to_string(),
        })
    }

    /// The CAIP-2 chain id this account is pinned to.
    pub fn chain_id(&self) -> &Caip2ChainId {
        &self.chain
    }

    /// The address component (chain-specific; normalize for signer comparison).
    pub fn address(&self) -> &str {
        &self.address
    }
}

/// The wallet/crypto family a CAIP-2 chain id belongs to.
///
/// Determines which signing RPC method and which signer-recovery scheme apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChainFamily {
    /// EVM chains (`eip155:*`). Signs via `eth_signTransaction`; signer
    /// recovered via secp256k1 ecrecover.
    Evm,
    /// Solana clusters (`solana:*`). Signs via `solana_signTransaction`; signer
    /// is the connected ed25519 account.
    Solana,
}

impl ChainFamily {
    /// The single WalletConnect v2 RPC method this family uses to *sign* the
    /// gate-bound transaction.
    ///
    /// We deliberately pin to the **sign**, not the **send/broadcast**, method:
    /// broadcasting is the deterministic post-approval continuation owned by
    /// PR10 (`ironclaw_chain_signing`), never the wallet. Pinning to the
    /// sign-only method also narrows the relay/session attack surface (the
    /// session is never authorized to broadcast on the user's behalf).
    pub fn signing_method(self) -> &'static str {
        match self {
            ChainFamily::Evm => "eth_signTransaction",
            ChainFamily::Solana => "solana_signTransaction",
        }
    }
}

/// The single chain + single method a WalletConnect session is permitted to
/// carry for this gate. Anything broader is a [`SigningProviderError::ScopeViolation`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinnedScope {
    /// The exact, typed CAIP-2 chain id (e.g. `eip155:1`, `solana:5eykt4...`).
    pub caip2_chain: Caip2ChainId,
    /// The chain family resolved from the CAIP-2 namespace.
    pub family: ChainFamily,
    /// The single signing RPC method permitted.
    pub method: String,
}

impl PinnedScope {
    /// Derive the pinned scope from the gate's bound chain id.
    ///
    /// The gate's [`ChainId`] is treated as a CAIP-2 chain id
    /// (`namespace:reference`). The namespace selects the family; the full id is
    /// the single permitted chain; the family selects the single permitted
    /// signing method.
    pub fn from_chain_id(chain_id: &ChainId) -> Result<Self, SigningProviderError> {
        let caip2 = Caip2ChainId::parse(chain_id.as_str())?;
        let family = match caip2.namespace() {
            "eip155" => ChainFamily::Evm,
            "solana" => ChainFamily::Solana,
            // NEAR is explicitly unsupported on the WalletConnect provider until
            // a real NEAR account/signature verifier exists. Fail-closed rather
            // than accept a chain we cannot verify a signer for.
            "near" => {
                return Err(SigningProviderError::ScopeViolation {
                    reason:
                        "CAIP-2 namespace `near` is not supported by the walletconnect provider"
                            .to_string(),
                });
            }
            other => {
                return Err(SigningProviderError::ScopeViolation {
                    reason: format!("unsupported CAIP-2 namespace `{other}`"),
                });
            }
        };
        Ok(Self {
            family,
            method: family.signing_method().to_string(),
            caip2_chain: caip2,
        })
    }

    /// The CAIP-2 namespace (the part before the first `:`), e.g. `eip155`.
    pub fn namespace(&self) -> &str {
        self.caip2_chain.namespace()
    }

    /// The pinned CAIP-2 chain id as a string.
    pub fn chain_str(&self) -> &str {
        self.caip2_chain.as_str()
    }
}

/// A session scope as *proposed or settled* by the wallet/relay, to be checked
/// against the gate's [`PinnedScope`].
///
/// Mirrors the CAIP-25 `chains` / `methods` arrays of a single namespace.
/// Modeled minimally here (PR9 verifies the negotiated scope; the encrypted
/// CAIP-25 envelope round-trip over the relay is PR10).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProposedScope {
    /// CAIP-2 chain ids the session would authorize.
    pub chains: Vec<String>,
    /// RPC methods the session would authorize.
    pub methods: Vec<String>,
    /// CAIP-10 accounts the session settled on. Each is parsed as a typed
    /// CAIP-10 account and required to live on exactly the pinned CAIP-2 chain
    /// (#2). Empty is permitted for a pure *proposal* (pre-settlement) but
    /// rejected for a settled scope by [`enforce_pinned_scope`] when present.
    pub accounts: Vec<String>,
}

/// Validate a proposed/settled session scope against the gate's pinned scope,
/// fail-closed.
///
/// Rejects (T17/T19) when the proposal:
/// * authorizes any chain other than the single pinned chain, or no chains;
/// * authorizes any method other than the single pinned signing method, or no
///   methods.
///
/// Equality — not subset — is required: the session must be scoped to *exactly*
/// the one chain and one method the human approved. A proposal that is a strict
/// superset (extra chains/methods) is a scope-broadening attempt and is
/// rejected.
pub fn enforce_pinned_scope(
    pinned: &PinnedScope,
    proposed: &ProposedScope,
) -> Result<(), SigningProviderError> {
    let viol = |reason: String| SigningProviderError::ScopeViolation { reason };

    // Singleton-exact: exactly one chain and exactly one method, both equal to
    // the pinned values. A length != 1 is either empty (authorizes nothing) or a
    // duplicate/superset (scope broadening) — both fail closed (T17/T19).
    if proposed.chains.len() != 1 {
        return Err(viol(format!(
            "session scope must authorize exactly one chain, got {}",
            proposed.chains.len()
        )));
    }
    if proposed.methods.len() != 1 {
        return Err(viol(format!(
            "session scope must authorize exactly one method, got {}",
            proposed.methods.len()
        )));
    }
    // Compare the chain via the typed CAIP-2 parser, not a raw string compare:
    // a malformed chain id can never silently equal the pinned chain.
    let proposed_chain = Caip2ChainId::parse(&proposed.chains[0])?;
    if proposed_chain != pinned.caip2_chain {
        return Err(viol(format!(
            "session scope chain `{}` broadens scope beyond the pinned chain `{}`",
            proposed.chains[0],
            pinned.chain_str()
        )));
    }
    if proposed.methods[0] != pinned.method {
        return Err(viol(format!(
            "session scope method `{}` broadens scope beyond the pinned method `{}`",
            proposed.methods[0], pinned.method
        )));
    }

    // Accounts (#2): every settled account must be a typed CAIP-10 account that
    // lives on EXACTLY the pinned CAIP-2 chain. A `proposal` (pre-settlement)
    // may carry no accounts; a settled scope that lists accounts must bind them
    // all to the pinned chain.
    for account in &proposed.accounts {
        let parsed = Caip10Account::parse(account)?;
        if parsed.chain_id() != &pinned.caip2_chain {
            return Err(viol(format!(
                "settled account `{account}` is not on the pinned chain `{}`",
                pinned.chain_str()
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_evm_family_and_method() {
        let pinned = PinnedScope::from_chain_id(&ChainId::new("eip155:1")).expect("evm");
        assert_eq!(pinned.family, ChainFamily::Evm);
        assert_eq!(pinned.method, "eth_signTransaction");
        assert_eq!(pinned.namespace(), "eip155");
        assert_eq!(pinned.chain_str(), "eip155:1");
    }

    #[test]
    fn resolves_solana_family() {
        assert_eq!(
            PinnedScope::from_chain_id(&ChainId::new("solana:mainnet"))
                .expect("sol")
                .method,
            "solana_signTransaction"
        );
    }

    #[test]
    fn near_is_explicitly_unsupported() {
        let err = PinnedScope::from_chain_id(&ChainId::new("near:mainnet"))
            .expect_err("near unsupported");
        assert!(matches!(err, SigningProviderError::ScopeViolation { .. }));
    }

    #[test]
    fn multi_colon_chain_id_is_rejected() {
        let err =
            Caip2ChainId::parse("eip155:1:extra").expect_err("more than one colon must reject");
        assert!(matches!(err, SigningProviderError::ScopeViolation { .. }));
    }

    #[test]
    fn caip10_account_wrong_chain_is_rejected() {
        let pinned = PinnedScope::from_chain_id(&ChainId::new("eip155:1")).expect("evm");
        let proposed = ProposedScope {
            chains: vec!["eip155:1".to_string()],
            methods: vec!["eth_signTransaction".to_string()],
            // Same address, WRONG chain.
            accounts: vec!["eip155:137:0x00000000000000000000000000000000000000aa".to_string()],
        };
        let err = enforce_pinned_scope(&pinned, &proposed).expect_err("wrong-chain account");
        assert!(matches!(err, SigningProviderError::ScopeViolation { .. }));
    }

    #[test]
    fn caip10_account_on_pinned_chain_is_accepted() {
        let pinned = PinnedScope::from_chain_id(&ChainId::new("eip155:1")).expect("evm");
        let proposed = ProposedScope {
            chains: vec!["eip155:1".to_string()],
            methods: vec!["eth_signTransaction".to_string()],
            accounts: vec!["eip155:1:0x00000000000000000000000000000000000000aa".to_string()],
        };
        enforce_pinned_scope(&pinned, &proposed).expect("matching-chain account accepted");
    }

    #[test]
    fn duplicate_chain_array_is_rejected() {
        let pinned = PinnedScope::from_chain_id(&ChainId::new("eip155:1")).expect("evm");
        let proposed = ProposedScope {
            chains: vec!["eip155:1".to_string(), "eip155:1".to_string()],
            methods: vec!["eth_signTransaction".to_string()],
            accounts: vec![],
        };
        let err = enforce_pinned_scope(&pinned, &proposed).expect_err("duplicate chain");
        assert!(matches!(err, SigningProviderError::ScopeViolation { .. }));
    }

    #[test]
    fn duplicate_method_array_is_rejected() {
        let pinned = PinnedScope::from_chain_id(&ChainId::new("eip155:1")).expect("evm");
        let proposed = ProposedScope {
            chains: vec!["eip155:1".to_string()],
            methods: vec![
                "eth_signTransaction".to_string(),
                "eth_signTransaction".to_string(),
            ],
            accounts: vec![],
        };
        let err = enforce_pinned_scope(&pinned, &proposed).expect_err("duplicate method");
        assert!(matches!(err, SigningProviderError::ScopeViolation { .. }));
    }

    #[test]
    fn non_caip2_chain_id_is_scope_violation() {
        let err = PinnedScope::from_chain_id(&ChainId::new("ethereum")).expect_err("no colon");
        assert!(matches!(err, SigningProviderError::ScopeViolation { .. }));
    }

    #[test]
    fn unsupported_namespace_is_scope_violation() {
        let err = PinnedScope::from_chain_id(&ChainId::new("cosmos:cosmoshub-4"))
            .expect_err("unsupported");
        assert!(matches!(err, SigningProviderError::ScopeViolation { .. }));
    }

    fn evm_pinned() -> PinnedScope {
        PinnedScope::from_chain_id(&ChainId::new("eip155:1")).expect("evm")
    }

    #[test]
    fn exact_pinned_scope_is_accepted() {
        let proposed = ProposedScope {
            chains: vec!["eip155:1".to_string()],
            methods: vec!["eth_signTransaction".to_string()],
            accounts: vec![],
        };
        enforce_pinned_scope(&evm_pinned(), &proposed).expect("exact match accepted");
    }

    #[test]
    fn extra_chain_is_rejected_t19() {
        let proposed = ProposedScope {
            chains: vec!["eip155:1".to_string(), "eip155:137".to_string()],
            methods: vec!["eth_signTransaction".to_string()],
            accounts: vec![],
        };
        let err = enforce_pinned_scope(&evm_pinned(), &proposed).expect_err("extra chain");
        assert!(matches!(err, SigningProviderError::ScopeViolation { .. }));
    }

    #[test]
    fn extra_method_is_rejected_t17() {
        let proposed = ProposedScope {
            chains: vec!["eip155:1".to_string()],
            methods: vec![
                "eth_signTransaction".to_string(),
                "eth_sendTransaction".to_string(),
            ],
            accounts: vec![],
        };
        let err = enforce_pinned_scope(&evm_pinned(), &proposed).expect_err("extra method");
        assert!(matches!(err, SigningProviderError::ScopeViolation { .. }));
    }

    #[test]
    fn wrong_single_chain_is_rejected() {
        let proposed = ProposedScope {
            chains: vec!["eip155:137".to_string()],
            methods: vec!["eth_signTransaction".to_string()],
            accounts: vec![],
        };
        let err = enforce_pinned_scope(&evm_pinned(), &proposed).expect_err("wrong chain");
        assert!(matches!(err, SigningProviderError::ScopeViolation { .. }));
    }

    #[test]
    fn empty_chains_or_methods_rejected() {
        let no_chains = ProposedScope {
            chains: vec![],
            methods: vec!["eth_signTransaction".to_string()],
            accounts: vec![],
        };
        assert!(matches!(
            enforce_pinned_scope(&evm_pinned(), &no_chains).expect_err("no chains"),
            SigningProviderError::ScopeViolation { .. }
        ));
        let no_methods = ProposedScope {
            chains: vec!["eip155:1".to_string()],
            methods: vec![],
            accounts: vec![],
        };
        assert!(matches!(
            enforce_pinned_scope(&evm_pinned(), &no_methods).expect_err("no methods"),
            SigningProviderError::ScopeViolation { .. }
        ));
    }
}
