//! Attested-signing signer-continuation wiring for the reborn runtime (PR10).
//!
//! This is the composition seam that turns an `AttestedResolved` turn into a
//! real, ledger-guarded sign + broadcast. It assembles the
//! [`AttestedSignerContinuationDriver`] from the in-memory substrate stores
//! (gate bindings shared with the resume port, sealed grants, broadcast ledger)
//! and the external-wallet provider registry.
//!
//! The driver is constructed here rather than buried in the giant
//! `RebornRuntime` struct so the runtime does not have to name the custodial
//! signer's concrete keystore/grant/ledger generic parameters. PR11's web
//! ingress (`/api/chat/gate/resolve`) calls
//! [`RebornAttestedComposition::driver`] to continue a resolved gate; this
//! module owns the deny-first default policy and the in-memory stores (durable
//! backends are PR12).
//!
//! Why in-memory only: the prompt for this slice mandates the existing
//! in-memory stores and explicitly defers durable PG/libSQL backends to PR12,
//! so no single-backend persistence feature is introduced here (dual-backend
//! rule).

use std::sync::Arc;

use ironclaw_attestation::{InMemorySealedGrantStore, InMemorySigningLedger};
use ironclaw_attested_runtime::{
    AttestedSignerContinuationDriver, BroadcastOutcome, Broadcaster, ContinuationError,
    InMemoryAttestedGateBindingStore, ProviderRegistry,
};
use ironclaw_chain_signing::{CustodialSigner, DenyFirstCustodyPolicy, SecretsKeyStore, ShipGate};
use ironclaw_signing_provider::SigningContext;

/// The concrete custodial signer type the local-dev composition assembles. Its
/// generic parameters are pinned here so the rest of the runtime never names
/// them.
pub(crate) type LocalDevCustodialSigner =
    CustodialSigner<SecretsKeyStore, InMemorySealedGrantStore, InMemorySigningLedger>;

/// The concrete driver type the local-dev composition assembles.
pub(crate) type LocalDevContinuationDriver = AttestedSignerContinuationDriver<
    NoopBroadcaster,
    InMemorySigningLedger,
    LocalDevCustodialSigner,
>;

/// A dry-run broadcaster that records intent but performs NO network I/O and,
/// critically, NEVER advances the ledger to `BroadcastSubmitted`.
///
/// It reports [`Broadcaster::submits`] == `false`, so the driver leaves the
/// ledger at `Signed` and surfaces a
/// [`ironclaw_attested_runtime::BroadcastDisposition::NotBroadcast`] outcome —
/// the local-dev path can never be mislabeled as a real broadcast. A real
/// per-chain broadcaster (PR12 / production) reports `submits() == true` and
/// returns [`BroadcastOutcome::Submitted`].
#[derive(Debug, Default)]
pub struct NoopBroadcaster;

#[async_trait::async_trait]
impl Broadcaster for NoopBroadcaster {
    fn submits(&self) -> bool {
        false
    }

    async fn broadcast(
        &self,
        _context: &SigningContext,
        _signed: &[u8],
    ) -> Result<BroadcastOutcome, ContinuationError> {
        // Deliberately does not submit. The driver will NOT advance the ledger
        // to BroadcastSubmitted for a NotBroadcast outcome.
        Ok(BroadcastOutcome::NotBroadcast {
            reason: "local-dev noop broadcaster: signed but not submitted".to_string(),
        })
    }
}

/// Bundles the attested-signing composition the reborn runtime exposes to the
/// PR11 ingress: the shared binding store, the shared sealed-grant store (so
/// external-wallet providers can be registered against the SAME one-shot CAS
/// the driver uses), and the assembled continuation driver.
pub struct RebornAttestedComposition {
    bindings: Arc<InMemoryAttestedGateBindingStore>,
    grants: Arc<InMemorySealedGrantStore>,
    driver: Arc<LocalDevContinuationDriver>,
}

impl RebornAttestedComposition {
    /// Assemble the composition for local-dev from the gate-binding store the
    /// resume port already shares, a custodial keystore, the operator
    /// ship-gate, and the shared sealed-grant store.
    ///
    /// `build_providers` is the **provider-registration seam**: it is handed the
    /// shared sealed-grant store and returns the external-wallet
    /// [`ProviderRegistry`] to wire into the driver. Building the registry here
    /// — BEFORE the driver is constructed, over the exact `grants` the custodial
    /// signer also uses — guarantees the one-shot CAS (threat #1) is
    /// authoritative across BOTH the custodial signer and every external-wallet
    /// provider, so external-wallet continuations cannot fail `ProviderMismatch`
    /// and cannot claim a grant out of a different store. PR13's
    /// `AttestedProvidersConfig` / provider registration layers cleanly on top:
    /// it implements `build_providers` to register WalletConnect / Injected /
    /// NEAR providers over this shared store.
    pub fn new(
        bindings: Arc<InMemoryAttestedGateBindingStore>,
        keystore: Arc<SecretsKeyStore>,
        ship_gate: ShipGate,
        grants: Arc<InMemorySealedGrantStore>,
        build_providers: impl FnOnce(&Arc<InMemorySealedGrantStore>) -> ProviderRegistry,
    ) -> Self {
        let providers = build_providers(&grants);
        let ledger = Arc::new(InMemorySigningLedger::new());
        let custodial_signer = Arc::new(CustodialSigner::new(
            keystore,
            Arc::clone(&grants),
            Arc::clone(&ledger),
            ship_gate,
            Arc::new(DenyFirstCustodyPolicy),
        ));
        let driver = Arc::new(AttestedSignerContinuationDriver::new(
            Arc::clone(&bindings) as Arc<dyn ironclaw_attested_runtime::AttestedGateBindingStore>,
            providers,
            custodial_signer,
            ledger,
            Arc::new(NoopBroadcaster),
        ));
        Self {
            bindings,
            grants,
            driver,
        }
    }

    /// The authoritative gate-binding store. The PR11 ingress persists a
    /// binding here when it raises an attested gate, and the driver reads it
    /// back on continuation.
    pub fn bindings(&self) -> &Arc<InMemoryAttestedGateBindingStore> {
        &self.bindings
    }

    /// The shared sealed-grant store. Exposed so downstream provider
    /// registration (PR13) can build providers that claim grants out of the
    /// exact same store the driver's custodial signer uses (shared one-shot CAS,
    /// threat #1).
    pub fn grants(&self) -> &Arc<InMemorySealedGrantStore> {
        &self.grants
    }

    /// The assembled signer-continuation driver dispatched when a turn reaches
    /// `AttestedResolved`.
    pub fn driver(&self) -> &Arc<LocalDevContinuationDriver> {
        &self.driver
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use alloy_consensus::TxEip1559;
    use alloy_primitives::{Address, Bytes, TxKind, U256};
    use ironclaw_attestation::{
        AttestedSigningGrant, DecodedTransaction, GrantKey, RenderingSchemaVersion,
        SealedGrantStore,
    };
    use ironclaw_attested_runtime::{
        AttestedGateBinding, AttestedGateBindingStore, BroadcastDisposition, ContinuationError,
        CustodialMainnetShipGate,
    };
    use ironclaw_chain_signing::{ChainKeyBinding, ChainKeyId, KeyStore, evm};
    use ironclaw_host_api::{InvocationId, ProjectId, ResourceScope, TenantId, UserId};
    use ironclaw_secrets::SecretsCrypto;
    use ironclaw_signing_provider::{
        ActorId, ChainId, GateRef, KeyOrAccountId, ProviderId, RunId, ScopeId, SigningContext,
        SigningProof, TenantId as SigningTenantId, UserId as SigningUserId,
    };
    use secrecy::SecretString;

    const GATE: &str = "gate:reborn-attested-e2e";
    const TESTNET: &str = "eip155:11155111";

    /// End-to-end test driving the REAL `RebornAttestedComposition` (not a
    /// hand-assembled driver): a custodial continuation signs the tx rebuilt
    /// from the authoritative binding and, because local-dev wires the dry-run
    /// `NoopBroadcaster`, reports `NotBroadcast` with the ledger left at
    /// `Signed` — never a false `BroadcastSubmitted`. A replay then fails closed.
    #[tokio::test]
    async fn reborn_composition_signs_and_does_not_falsely_broadcast() {
        // Custodial keystore with a bound EVM key.
        let crypto = SecretsCrypto::new(SecretString::from(
            "0123456789abcdef0123456789ABCDEF".to_string(),
        ))
        .unwrap();
        let keystore = Arc::new(SecretsKeyStore::new(crypto));
        let priv_bytes = [0x31u8; 32];
        let key = k256::ecdsa::SigningKey::from_slice(&priv_bytes).unwrap();
        let addr_hex = hex::encode(evm::address_of(&key).as_slice());
        let scope = ResourceScope {
            tenant_id: TenantId::new("default").unwrap(),
            user_id: UserId::new("alice").unwrap(),
            agent_id: None,
            project_id: Some(ProjectId::new("bootstrap").unwrap()),
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        };
        keystore
            .bind(
                &scope,
                ChainKeyBinding {
                    chain: ChainKeyId::new(TESTNET).expect("valid chain id in test"),
                    public_address_hex: addr_hex.clone(),
                    evm_chain_id: Some(11155111),
                    derivation_path: "m/44'/60'/0'/0/0".to_string(),
                    kms_key_ref: None,
                },
                priv_bytes.to_vec(),
            )
            .await
            .unwrap();

        let tx = TxEip1559 {
            chain_id: 11155111,
            nonce: 1,
            gas_limit: 21_000,
            max_fee_per_gas: 30_000_000_000,
            max_priority_fee_per_gas: 1_000_000_000,
            to: TxKind::Call(Address::repeat_byte(0x11)),
            value: U256::from(5u64),
            input: Bytes::new(),
            access_list: Default::default(),
        };
        let decoded: DecodedTransaction = evm::decode_eip1559(&tx);
        // Fold in the SAME gate-bound signer the SigningContext below carries
        // (`addr_hex`) — WYSIWYS — so the driver's resume-time recompute (which
        // reads `binding.context.key_or_account_id`) reproduces this hash.
        let hash = ironclaw_chain_signing::recompute_approved_hash(
            &decoded,
            &addr_hex,
            RenderingSchemaVersion::CURRENT,
        )
        .expect("recompute approved hash in test");

        let ctx = SigningContext {
            tenant: SigningTenantId::new("default"),
            user: SigningUserId::new("alice"),
            scope: ScopeId::new("scope"),
            actor: ActorId::new("actor"),
            run_id: RunId::new("run"),
            gate_ref: GateRef::new(GATE),
            chain_id: ChainId::new(TESTNET),
            key_or_account_id: KeyOrAccountId::new(addr_hex.clone()),
        };

        let bindings = Arc::new(InMemoryAttestedGateBindingStore::new());
        let ship_gate = CustodialMainnetShipGate::new(false).build_chain_ship_gate(None);
        let grants = Arc::new(InMemorySealedGrantStore::new());

        let composition = RebornAttestedComposition::new(
            Arc::clone(&bindings),
            keystore,
            ship_gate,
            Arc::clone(&grants),
            |_grants| ProviderRegistry::new(),
        );

        // Seal the grant the custodial signer will claim, over the SHARED store.
        let grant_key = GrantKey::from_context(&ctx, hash);
        composition
            .grants()
            .seal(AttestedSigningGrant::seal(grant_key, 0, None))
            .await
            .unwrap();

        // Persist the authoritative binding (as the PR11 ingress would).
        composition
            .bindings()
            .put(
                GateRef::new(GATE),
                AttestedGateBinding {
                    provider_id: ProviderId::Custodial,
                    context: ctx.clone(),
                    approved_tx_hash: hash,
                    decoded,
                    chain: ChainKeyId::new(TESTNET).expect("valid chain id in test"),
                    scope,
                    schema_version: RenderingSchemaVersion::CURRENT,
                },
            )
            .await
            .expect("binding insert succeeds");

        let gate = GateRef::new(GATE);
        let proof = SigningProof::WebAuthnAssertionProof(vec![]);

        let outcome = composition
            .driver()
            .continue_after_resolved(&gate, &proof)
            .await
            .expect("custodial continuation signs");

        // The local-dev path signs but NEVER reports a real broadcast.
        assert!(
            matches!(outcome.broadcast, BroadcastDisposition::NotBroadcast { .. }),
            "noop path must not report a real broadcast, got {:?}",
            outcome.broadcast
        );
        assert_eq!(
            outcome.ledger_state,
            ironclaw_attestation::SigningLedgerState::Signed,
            "noop path must leave the ledger at Signed, not BroadcastSubmitted"
        );

        // Replay fails closed (grant already claimed / ledger guard).
        let err = composition
            .driver()
            .continue_after_resolved(&gate, &proof)
            .await
            .expect_err("replay must fail closed");
        assert!(
            matches!(
                err,
                ContinuationError::Ledger(_) | ContinuationError::ChainSigning(_)
            ),
            "expected fail-closed replay, got {err:?}"
        );
    }
}
