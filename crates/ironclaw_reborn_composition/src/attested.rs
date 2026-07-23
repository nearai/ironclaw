//! Attested-signing signer-continuation wiring for the reborn runtime.
//!
//! This is the composition seam that turns an `AttestedResolved` turn into a
//! real, ledger-guarded sign + broadcast. It assembles the
//! [`AttestedSignerContinuationDriver`] from the substrate stores (gate
//! bindings shared with the resume port, sealed grants, broadcast ledger) and
//! the external-wallet provider registry.
//!
//! The driver is constructed here rather than buried in the giant
//! `RebornRuntime` struct so the runtime does not have to name the custodial
//! signer's concrete keystore/grant/ledger generic parameters. PR11's web
//! ingress (`/api/chat/gate/resolve`) calls
//! [`RebornAttestedComposition::driver`] to continue a resolved gate.
//!
//! ## Backend selection (PR12)
//!
//! [`RebornAttestedComposition`] is generic over the grant store `G`, ledger
//! `L`, and broadcaster `B`. Local-dev/tests use the in-memory stores and the
//! [`NoopBroadcaster`] (the [`LocalDevAttestedComposition`] alias). Production
//! selects the durable PG / libSQL grant store + ledger from
//! `ironclaw_attested_store` and the real [`MultiChainBroadcaster`]; the
//! ledger-guard behaviour (threats #6 / #7) is identical regardless of backend,
//! because the guard lives in the `SigningLedger` state machine, not the
//! broadcaster. Backend choice mirrors every other reborn store: it follows the
//! configured database backend.

use std::sync::Arc;

use ironclaw_attestation::{
    AttestedSigningGrant, GrantError, GrantKey, InMemorySealedGrantStore, InMemorySigningLedger,
    SealedGrantStore, SigningLedger,
};
use ironclaw_attested_runtime::{
    AttestedGateBinding, AttestedGateBindingStore, AttestedSignerContinuationDriver, BindingError,
    BroadcastOutcome, Broadcaster, ContinuationError, InMemoryAttestedGateBindingStore,
    ProviderRegistry,
};
use ironclaw_chain_signing::{CustodialSigner, DenyFirstCustodyPolicy, SecretsKeyStore, ShipGate};
use ironclaw_signing_provider::{GateRef, SigningContext};

/// Error from [`RebornAttestedComposition::register_attested_gate`]. Distinct
/// from [`ironclaw_attestation::GrantError`] so the gate-raise caller can tell
/// a hardening rejection (mismatched gate_ref / duplicate raise) apart from a
/// grant-store / binding-store backend failure.
#[derive(Debug)]
pub enum RegisterAttestedGateError {
    /// The supplied `gate_ref` did not equal `binding.context.gate_ref`.
    GateRefMismatch,
    /// A binding (or sealed grant) already exists for this gate: registration is
    /// insert-only and the first raise wins.
    DuplicateBinding,
    /// The underlying sealed-grant store failed.
    Grant(GrantError),
    /// The binding store rejected or could not record the binding (validation
    /// failure or backend error). Fail closed — the gate is not registered.
    BindingStore(BindingError),
}

impl std::fmt::Display for RegisterAttestedGateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::GateRefMismatch => {
                write!(f, "gate_ref does not match binding.context.gate_ref")
            }
            Self::DuplicateBinding => {
                write!(f, "attested gate already registered (insert-only)")
            }
            Self::Grant(e) => write!(f, "sealed-grant store failed: {e}"),
            Self::BindingStore(e) => write!(f, "binding store failed: {e}"),
        }
    }
}

impl std::error::Error for RegisterAttestedGateError {}

/// The custodial signer type, generic over the grant store and ledger backend.
pub(crate) type ComposedCustodialSigner<G, L> = CustodialSigner<SecretsKeyStore, G, L>;

/// The continuation driver type, generic over broadcaster / ledger / signer.
pub(crate) type ComposedContinuationDriver<B, G, L> =
    AttestedSignerContinuationDriver<B, L, ComposedCustodialSigner<G, L>>;

/// The local-dev / test monomorphization of [`RebornAttestedComposition`] the
/// `RebornRuntime` holds (in-memory stores + no-op broadcaster).
pub(crate) type LocalDevContinuationDriver =
    ComposedContinuationDriver<NoopBroadcaster, InMemorySealedGrantStore, InMemorySigningLedger>;

pub type LocalDevAttestedComposition =
    RebornAttestedComposition<NoopBroadcaster, InMemorySealedGrantStore, InMemorySigningLedger>;

/// A dry-run broadcaster that records intent but performs NO network I/O and,
/// critically, NEVER advances the ledger to `BroadcastSubmitted`.
///
/// It reports [`Broadcaster::submits`] == `false`, so the driver leaves the
/// ledger at `Signed` and surfaces a
/// [`ironclaw_attested_runtime::BroadcastDisposition::NotBroadcast`] outcome —
/// the local-dev path can never be mislabeled as a real broadcast. A real
/// per-chain broadcaster (`ironclaw_attested_store::MultiChainBroadcaster`,
/// selected in production) reports `submits() == true` and returns
/// [`BroadcastOutcome::Submitted`].
///
/// # PRODUCTION WARNING
///
/// This broadcaster intentionally NEVER submits. It exists for local-dev /
/// test wiring ONLY. Do NOT wire it into a production composition: a real
/// deployment MUST inject a per-chain broadcaster whose `submits()` returns
/// `true`. The `submits() -> false` contract is the compile-independent
/// guard — the driver leaves the ledger at `Signed` and reports `NotBroadcast`
/// rather than a false success — but a silent mis-wire here would mean
/// transactions are signed and never broadcast. PR13/PR14 production wiring
/// must select the real broadcaster, not this one.
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

/// PR11 ingress: the shared binding store and the assembled continuation driver.
///
/// Generic over the durable-vs-in-memory grant store `G`, ledger `L`, and
/// broadcaster `B`.
pub struct RebornAttestedComposition<B, G, L>
where
    B: Broadcaster + 'static,
    G: SealedGrantStore + 'static,
    L: SigningLedger + 'static,
{
    bindings: Arc<dyn AttestedGateBindingStore>,
    /// Shared sealed-grant store. Held so `register_attested_gate` (the raise
    /// path) seals into the SAME store the driver's custodial signer claims from
    /// — the one-shot CAS is authoritative across raise and continuation.
    grants: Arc<G>,
    driver: Arc<ComposedContinuationDriver<B, G, L>>,
}

impl<B, G, L> RebornAttestedComposition<B, G, L>
where
    B: Broadcaster + 'static,
    G: SealedGrantStore + 'static,
    L: SigningLedger + 'static,
{
    /// Assemble the composition from the gate-binding store the resume port
    /// already shares, a custodial keystore, the operator ship-gate, the shared
    /// sealed-grant store, the broadcast ledger, the provider registry, and the
    /// broadcaster.
    ///
    /// The grant store is shared so the one-shot CAS (threat #1) is
    /// authoritative across both the custodial signer and the external-wallet
    /// providers. The ledger is shared between the custodial signer and the
    /// driver so the broadcast-idempotency guard covers both paths.
    // arch-exempt: too_many_args, assemble fans the substrate stores
    // (grants/ledger/broadcaster/providers) plus keystore/ship-gate into one
    // driver; needs an AttestedSigningServices bundle,
    // plan docs/plans/2026-05-23-attested-signing-substrate.md
    #[allow(clippy::too_many_arguments)]
    pub fn assemble(
        bindings: Arc<dyn AttestedGateBindingStore>,
        keystore: Arc<SecretsKeyStore>,
        ship_gate: ShipGate,
        grants: Arc<G>,
        ledger: Arc<L>,
        broadcaster: Arc<B>,
        providers: ProviderRegistry,
    ) -> Self {
        let custodial_signer = Arc::new(CustodialSigner::new(
            keystore,
            Arc::clone(&grants),
            Arc::clone(&ledger),
            ship_gate,
            Arc::new(DenyFirstCustodyPolicy),
        ));
        let driver = Arc::new(AttestedSignerContinuationDriver::new(
            Arc::clone(&bindings),
            providers,
            custodial_signer,
            ledger,
            broadcaster,
        ));
        Self {
            bindings,
            grants,
            driver,
        }
    }

    /// Register an attested gate: seal its one-shot grant and persist its
    /// authoritative binding. This is the PR11 ingress entry point invoked when
    /// a gate is raised.
    ///
    /// In-memory only (PR11); durable PG / libSQL backends are PR12.
    ///
    /// Hardening invariants enforced here:
    /// - The supplied `gate_ref` MUST equal `binding.context.gate_ref`. A
    ///   mismatch would let the binding be filed under a key that names a
    ///   different gate than the one the authoritative context describes — the
    ///   resume port and driver both look the binding up by `gate_ref`, so a
    ///   mismatch is a binding-confusion vector. Fail closed.
    /// - Registration is INSERT-ONLY: an existing binding for the same gate
    ///   (request id) is never overwritten. The first raise wins; a second raise
    ///   for the same gate is refused so an attacker cannot redefine the
    ///   authoritative `(hash, signer, decoded tx)` after the fact (threats
    ///   #2/#3/#4). The grant seal is likewise one-shot.
    pub async fn register_attested_gate(
        &self,
        gate_ref: GateRef,
        binding: AttestedGateBinding,
        created_at_ms: i64,
        expiry_ms: Option<i64>,
    ) -> Result<(), RegisterAttestedGateError> {
        // gate_ref must match the authoritative context's gate_ref.
        if binding.context.gate_ref.as_str() != gate_ref.as_str() {
            return Err(RegisterAttestedGateError::GateRefMismatch);
        }

        // Seal the one-shot grant first. The seal is an atomic CAS
        // (`AlreadySealed` on a second seal of the same key), so it is the gate
        // that serializes concurrent raises of the same gate: at most one caller
        // wins the seal and reaches the binding insert below. A duplicate seal
        // means the gate was already raised; surface it as a duplicate rather
        // than proceeding to (re)write the binding.
        let grant_key = GrantKey::from_context(&binding.context, binding.approved_tx_hash);
        match self
            .grants
            .seal(AttestedSigningGrant::new(
                grant_key,
                created_at_ms,
                expiry_ms,
            ).map_err(RegisterAttestedGateError::Grant)?)
            .await
        {
            Ok(()) => {}
            Err(GrantError::AlreadySealed) => {
                return Err(RegisterAttestedGateError::DuplicateBinding);
            }
            Err(other) => return Err(RegisterAttestedGateError::Grant(other)),
        }

        // Insert-only, ATOMIC + VALIDATED: the store's `put` is insert-only (the
        // existence check and the insert happen under a single critical section,
        // closing the check-then-act TOCTOU window) and fully validates the
        // binding (`gate_ref`/hash/chain/signer self-consistency) before
        // persisting. An existing binding for this gate fails closed with
        // `AlreadyExists` — treat it as a duplicate, consistent with the grant
        // CAS above; any other validation/backend error fails closed too.
        match self.bindings.put(gate_ref, binding).await {
            Ok(()) => Ok(()),
            Err(BindingError::AlreadyExists) => Err(RegisterAttestedGateError::DuplicateBinding),
            Err(other) => Err(RegisterAttestedGateError::BindingStore(other)),
        }
    }

    /// The authoritative gate-binding store. The PR11 ingress persists a
    /// binding here when it raises an attested gate, and the driver reads it
    /// back on continuation.
    pub fn bindings(&self) -> &Arc<dyn AttestedGateBindingStore> {
        &self.bindings
    }

    /// The assembled signer-continuation driver dispatched when a turn reaches
    /// `AttestedResolved`.
    pub fn grants(&self) -> &Arc<G> {
        &self.grants
    }

    pub fn driver(&self) -> &Arc<ComposedContinuationDriver<B, G, L>> {
        &self.driver
    }
}

impl RebornAttestedComposition<NoopBroadcaster, InMemorySealedGrantStore, InMemorySigningLedger> {
    /// Local-dev / test constructor: in-memory grant store + ledger + no-op
    /// broadcaster. Matches the pre-PR12 behaviour.
    pub fn new_in_memory(
        bindings: Arc<InMemoryAttestedGateBindingStore>,
        keystore: Arc<SecretsKeyStore>,
        ship_gate: ShipGate,
        grants: Arc<InMemorySealedGrantStore>,
        providers: ProviderRegistry,
    ) -> Self {
        let ledger = Arc::new(InMemorySigningLedger::new());
        Self::assemble(
            bindings as Arc<dyn AttestedGateBindingStore>,
            keystore,
            ship_gate,
            grants,
            ledger,
            Arc::new(NoopBroadcaster),
            providers,
        )
    }
}

/// Durable PostgreSQL attested-signing composition: PG sealed-grant store + PG
/// signing ledger + the real per-chain broadcaster. The DB-enforced one-shot
/// CAS / broadcast-idempotency guards hold across process restarts and the
/// `Stuck -> InProgress` recovery race.
#[cfg(all(feature = "postgres", feature = "attested-broadcast"))]
pub type PostgresAttestedComposition = RebornAttestedComposition<
    ironclaw_attested_store::MultiChainBroadcaster,
    ironclaw_attested_store::PostgresSealedGrantStore,
    ironclaw_attested_store::PostgresSigningLedger,
>;

/// Durable libSQL / Turso attested-signing composition.
#[cfg(all(feature = "libsql", feature = "attested-broadcast"))]
pub type LibSqlAttestedComposition = RebornAttestedComposition<
    ironclaw_attested_store::MultiChainBroadcaster,
    ironclaw_attested_store::LibSqlSealedGrantStore,
    ironclaw_attested_store::LibSqlSigningLedger,
>;

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
        AttestedGateBinding, BroadcastDisposition, ContinuationError, CustodialMainnetShipGate,
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

        let composition = RebornAttestedComposition::new_in_memory(
            Arc::clone(&bindings),
            keystore,
            ship_gate,
            Arc::clone(&grants),
            ProviderRegistry::new(),
        );

        // Seal the grant the custodial signer will claim, over the SHARED store
        // (the same `grants` Arc handed to `new_in_memory`).
        let grant_key = GrantKey::from_context(&ctx, hash);
        composition
            .grants()
            .seal(AttestedSigningGrant::new(grant_key, 0, None).expect("valid grant"))
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
                ContinuationError::Ledger(_)
                    | ContinuationError::LedgerRowExists { .. }
                    | ContinuationError::ChainSigning(_)
            ),
            "expected fail-closed replay, got {err:?}"
        );
    }
}
