//! The signer-continuation driver: the deterministic post-approval continuation
//! that runs once the turn store transitions `BlockedAttested ->
//! AttestedResolved`.
//!
//! This consumes the `// PR10:` handoff stubs left in PR7-PR9:
//!
//! * `crates/ironclaw_wallet_external/src/walletconnect/mod.rs` ("hand the
//!   verified proof back to the gate / runner for the continuation") — the
//!   driver routes a `WalletConnect` / `Injected` / `NearRedirect` resolved gate
//!   to the matching [`SigningProvider::verify_resume`], turning the verified
//!   proof into a ledger-guarded broadcast.
//! * `src/channels/web/features/chat/attested.rs` ("build `ResumeTurnRequest {
//!   attestation: Some(..) }` + dispatch the broadcast through the gate-resolve
//!   path") — the broadcast half of that handoff lives here; the web ingress
//!   that calls into it is PR11.
//!
//! ## Invariants enforced here
//!
//! * **Threat #1 (sealed-grant replay):** the authoritative one-shot grant is
//!   claimed (atomic CAS) before any signing. The custodial path claims it
//!   inside [`CustodialSigner`]; the external-wallet path claims it inside the
//!   provider's `verify_resume`.
//! * **Threats #6 / #7 (broadcast retry / `Stuck->InProgress` double-broadcast):**
//!   every state move goes through the [`SigningLedger`], which refuses to
//!   re-enter signing for a `gate_ref` already past `BroadcastSubmitted` — and
//!   is keyed on ledger state, not job state, so a job recovery cannot
//!   re-broadcast.
//! * **Threat #16 (LLM-loop reinterpretation):** the driver is only reachable
//!   from the `AttestedResolved` continuation; it validates + signs + broadcasts
//!   and NEVER requeues the agent loop.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;

use ironclaw_attestation::{SigningLedger, SigningLedgerState};
use ironclaw_chain_signing::{
    ChainSigningError, CustodialSignRequest, CustodialSigner, recompute_approved_hash,
};

mod rebuild;
use ironclaw_signing_provider::{
    GateRef, ProviderId, SigningContext, SigningProof, SigningProvider, SigningProviderError,
    TrustModel,
};
pub use rebuild::{EvmSignable, RebuildError};

use crate::binding::{AttestedGateBinding, AttestedGateBindingStore};

/// Registry mapping a [`ProviderId`] to the external-wallet
/// [`SigningProvider`] that verifies its proofs.
///
/// The custodial path is NOT in this registry — it is the
/// [`CustodialSigner`], which both claims the grant and signs, and is wired
/// separately into the driver.
#[derive(Default)]
pub struct ProviderRegistry {
    providers: HashMap<ProviderId, Arc<dyn SigningProvider>>,
}

impl ProviderRegistry {
    /// Construct an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register an external-wallet provider under its own [`ProviderId`].
    pub fn with_provider(mut self, provider: Arc<dyn SigningProvider>) -> Self {
        self.providers.insert(provider.provider_id(), provider);
        self
    }

    fn get(&self, id: ProviderId) -> Option<&Arc<dyn SigningProvider>> {
        self.providers.get(&id)
    }
}

/// Whether the continuation actually submitted the signed transaction to the
/// chain, or signed-only (dry-run / local-dev). Kept TYPED and distinct so a
/// non-broadcasting path can NEVER be mistaken for a real broadcast: a caller
/// that wants a real submit must match on [`BroadcastDisposition::Submitted`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BroadcastDisposition {
    /// The signed transaction was submitted to the chain. The ledger reached
    /// `BroadcastSubmitted`.
    Submitted {
        /// Opaque chain transaction id / hash (never key material).
        tx_id: String,
    },
    /// The transaction was signed but NOT submitted (a dry-run / local-dev
    /// broadcaster). The ledger is left at `Signed` — it does NOT reach
    /// `BroadcastSubmitted`, so the runtime never reports a successful broadcast
    /// for a non-broadcast.
    NotBroadcast {
        /// Opaque reason the submit was skipped.
        reason: String,
    },
}

/// What a [`Broadcaster`] produced for one submit attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BroadcastOutcome {
    /// The signed transaction was accepted by the chain.
    Submitted {
        /// Opaque chain transaction id / hash.
        tx_id: String,
    },
    /// The broadcaster deliberately did not submit (dry-run / local-dev). This
    /// is NOT an error and NOT a broadcast.
    NotBroadcast {
        /// Opaque reason the submit was skipped.
        reason: String,
    },
}

/// What the continuation produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignerContinuationOutcome {
    /// The `gate_ref` that was continued.
    pub gate_ref: GateRef,
    /// The ledger state reached. `BroadcastSubmitted` ONLY when the signed tx
    /// was actually submitted; a dry-run / non-broadcasting path leaves this at
    /// `Signed`. The driver leaves finalization to the chain watcher.
    pub ledger_state: SigningLedgerState,
    /// Whether the signed tx was actually submitted to the chain.
    pub broadcast: BroadcastDisposition,
    /// The signer/account the broadcast was attributed to (public).
    pub signer: String,
}

/// The product of the verify + claim + sign half of the continuation
/// ([`AttestedSignerContinuationDriver::verify_and_sign`]). Holds everything the
/// broadcast half needs and PROVES the heavyweight crypto already ran: the
/// proof was verified, the one-shot sealed grant was claimed, and the signed
/// bytes ready to broadcast were produced.
///
/// [`AttestedSignerContinuationDriver::broadcast_signed_continuation`] consumes
/// it to advance the ledger to `BroadcastSubmitted` and submit. The signed
/// bytes never re-trigger verification or a second grant claim: the heavyweight
/// crypto runs exactly once, in `verify_and_sign`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedContinuation {
    gate_ref: GateRef,
    context: SigningContext,
    signed: Vec<u8>,
    signer: String,
}

impl VerifiedContinuation {
    /// The gate this verified continuation belongs to.
    pub fn gate_ref(&self) -> &GateRef {
        &self.gate_ref
    }

    /// The signer/account the eventual broadcast is attributed to (public).
    pub fn signer(&self) -> &str {
        &self.signer
    }
}

/// Errors the signer-continuation driver can surface. Every variant is
/// fail-closed: the ledger is never advanced past where the failure occurred.
#[derive(Debug)]
pub enum ContinuationError {
    /// No authoritative binding exists for the resolved `gate_ref`.
    MissingBinding,
    /// The carried proof's provider does not match the bound provider, or no
    /// provider is registered for it.
    ProviderMismatch {
        /// The bound provider id.
        bound: ProviderId,
    },
    /// The external-wallet provider rejected the proof (signer mismatch, hash
    /// mismatch, grant-claim failure, scope violation).
    ProofRejected(SigningProviderError),
    /// The custodial chain signer rejected or failed the signing.
    ChainSigning(ChainSigningError),
    /// A ledger transition was rejected (e.g. broadcast idempotency guard).
    Ledger(ironclaw_attestation::LedgerError),
    /// The sign-time approved-tx-hash re-check failed (threat #3): the hash
    /// recomputed from the persisted decoded tx diverged from the bound hash.
    ApprovedHashMismatch,
    /// The binding's `chain` does not match the chain/network of its own decoded
    /// transaction (a malformed / tampered binding). Fail-closed before any key
    /// use so a testnet `chain` can never carry a mainnet decoded tx (or vice
    /// versa) past the ship-gate.
    BindingChainMismatch,
    /// The authoritative decoded binding could not be reconstructed into a
    /// signable transaction (unsupported chain/tx-type, or a field overflow).
    Rebuild(RebuildError),
    /// A broadcaster-side failure.
    Broadcast {
        /// Opaque description (never key material).
        reason: String,
    },
    /// A startup / assembly-time misconfiguration (a malformed RPC endpoint, a
    /// failed store migration, etc.) detected while building the durable
    /// composition — distinct from a runtime [`Self::Broadcast`] outage so ops
    /// can tell "misconfigured at boot" from "RPC is down at runtime". Never
    /// carries key material.
    Config {
        /// Opaque description (never key material).
        reason: String,
    },
    /// A broadcast-idempotency ledger row already exists for this `gate_ref`
    /// (a prior continuation attempt). This is the one-shot guard firing on
    /// re-entry — distinct from a generic invalid ledger transition. Carries the
    /// existing ledger state for diagnostics.
    LedgerRowExists {
        /// The state the existing row is currently in.
        current: SigningLedgerState,
    },
}

impl std::fmt::Display for ContinuationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingBinding => write!(f, "no authoritative binding for the resolved gate"),
            Self::ProviderMismatch { bound } => {
                write!(f, "provider mismatch: bound provider is {bound:?}")
            }
            Self::ProofRejected(e) => write!(f, "external-wallet proof rejected: {e}"),
            Self::ChainSigning(e) => write!(f, "custodial chain signing failed: {e}"),
            Self::Ledger(e) => write!(f, "signing-ledger transition rejected: {e}"),
            Self::ApprovedHashMismatch => {
                write!(f, "sign-time approved-tx-hash re-check failed")
            }
            Self::BindingChainMismatch => {
                write!(f, "binding chain does not match its decoded transaction")
            }
            Self::Rebuild(e) => write!(f, "decoded-binding rebuild failed: {e}"),
            Self::Broadcast { reason } => write!(f, "broadcast failed: {reason}"),
            Self::Config { reason } => write!(f, "attested composition misconfigured: {reason}"),
            Self::LedgerRowExists { current } => {
                write!(
                    f,
                    "broadcast-idempotency ledger row already exists (current state: {current:?})"
                )
            }
        }
    }
}

impl std::error::Error for ContinuationError {}

impl From<ironclaw_attestation::LedgerError> for ContinuationError {
    fn from(value: ironclaw_attestation::LedgerError) -> Self {
        Self::Ledger(value)
    }
}

/// Broadcasts a signed transaction to its chain. Injected so the driver stays
/// testable without real network I/O. PR12 / production wires a real
/// per-chain broadcaster; the ledger guard around it is identical regardless.
#[async_trait]
pub trait Broadcaster: Send + Sync {
    /// Whether this broadcaster actually submits to a chain. A real per-chain
    /// broadcaster returns `true`; a dry-run / local-dev broadcaster returns
    /// `false`. The driver consults this BEFORE advancing the ledger so a
    /// non-broadcasting path never reaches `BroadcastSubmitted`.
    fn submits(&self) -> bool;

    /// Submit the signed transaction for `context`'s chain. Returns a typed
    /// [`BroadcastOutcome`] so a deliberate non-submit ([`BroadcastOutcome::NotBroadcast`])
    /// is distinguishable from a real submit and from an error.
    async fn broadcast(
        &self,
        context: &SigningContext,
        signed: &[u8],
    ) -> Result<BroadcastOutcome, ContinuationError>;
}

/// The signer-continuation driver, wired with the authoritative binding store,
/// the external-wallet provider registry, the custodial signer, the broadcast
/// idempotency ledger, and a broadcaster.
///
/// `K`/`G`/`L` are the custodial signer's keystore / grant store / ledger
/// types; the same ledger `L` is shared so the broadcast-idempotency guard
/// covers both the custodial and external-wallet paths.
pub struct AttestedSignerContinuationDriver<B, L, S> {
    bindings: Arc<dyn AttestedGateBindingStore>,
    providers: ProviderRegistry,
    custodial_signer: Arc<S>,
    ledger: Arc<L>,
    broadcaster: Arc<B>,
}

impl<B, L, S> AttestedSignerContinuationDriver<B, L, S>
where
    B: Broadcaster,
    L: SigningLedger,
{
    /// Construct the driver.
    pub fn new(
        bindings: Arc<dyn AttestedGateBindingStore>,
        providers: ProviderRegistry,
        custodial_signer: Arc<S>,
        ledger: Arc<L>,
        broadcaster: Arc<B>,
    ) -> Self {
        Self {
            bindings,
            providers,
            custodial_signer,
            ledger,
            broadcaster,
        }
    }

    /// Run the deterministic continuation for a gate that has reached
    /// `AttestedResolved`. `proof` is the verified-proof payload carried back
    /// from the ceremony (external-wallet paths) — for the custodial path the
    /// proof is the WebAuthn assertion that authorized the in-house signer.
    ///
    /// The driver NEVER accepts a caller-supplied signable transaction: the
    /// custodial path reconstructs the signable *from the authoritative decoded
    /// binding* and signs exactly that, so a resolver cannot pass an unapproved
    /// tx (or a mainnet tx aimed past a testnet `binding.chain` ship-gate) after
    /// approval (byte-drift defense, same class as PR6's `CustodialSigner`).
    ///
    /// Steps, all fail-closed and ledger-guarded:
    /// 1. Read the authoritative binding for `gate_ref` (never trust the
    ///    caller).
    /// 2. Create the ledger row (one-shot per `gate_ref`; an existing row from
    ///    a prior broadcast attempt that is already past `Signed` makes any
    ///    re-broadcast fail — threats #6 / #7).
    /// 3. Route to the bound provider / custodial signer to verify + claim the
    ///    sealed grant (threat #1) and produce the signature.
    /// 4. Advance the ledger to `BroadcastSubmitted` (the in-flight marker) and
    ///    broadcast. On a confirmed submit the row stays at `BroadcastSubmitted`;
    ///    on a broadcast error / ambiguous outcome it moves to the `Unknown`
    ///    terminal for out-of-band recovery (never auto-rebroadcast).
    pub async fn continue_after_resolved(
        &self,
        gate_ref: &GateRef,
        proof: &SigningProof,
    ) -> Result<SignerContinuationOutcome, ContinuationError>
    where
        S: CustodialSignerLike,
    {
        // Legacy single-shot entrypoint, retained for the threat-matrix tests
        // and any caller that drives both halves under one lock. The
        // verify-before-resume facade (PR11 item B) instead calls
        // [`Self::verify_and_sign`] BEFORE the turn transitions and
        // [`Self::broadcast_signed_continuation`] AFTER, so the heavyweight
        // verification + grant claim gate the `BlockedAttested ->
        // AttestedResolved` transition. The crypto runs exactly once in either
        // arrangement.
        let verified = self.verify_and_sign(gate_ref, proof).await?;
        self.broadcast_signed_continuation(verified).await
    }

    /// Verify + claim + sign half of the continuation. Runs BEFORE the turn
    /// transitions to `AttestedResolved`, so the FULL cryptographic verification
    /// and the one-shot grant claim gate the transition: a malformed or forged
    /// proof is rejected here, with no broadcast and no `AttestedResolved`
    /// transition (the facade only calls `resume_turn` after this returns `Ok`).
    ///
    /// The driver NEVER accepts a caller-supplied signable transaction: the
    /// custodial path reconstructs the signable *from the authoritative decoded
    /// binding* and signs exactly that, so a resolver cannot pass an unapproved
    /// tx (or a mainnet tx aimed past a testnet `binding.chain` ship-gate) after
    /// approval (byte-drift defense, same class as PR6's `CustodialSigner`).
    ///
    /// 1. Read the authoritative binding for `gate_ref` (never trust the
    ///    caller).
    /// 2. Route to the bound provider / custodial signer to verify + claim the
    ///    sealed grant (threat #1) and produce the signature, under the
    ///    broadcast-idempotency ledger guard (threats #6 / #7).
    ///
    /// Fail-closed retry semantics: each path creates / advances the ledger only
    /// once verification is committed to, so a proof that fails verification
    /// (malformed, forged, signer/hash mismatch) leaves NO blocking ledger row
    /// and does NOT claim the grant — a follow-up VALID proof for the same gate
    /// can still succeed. After a SUCCESSFUL verify+claim, the grant CAS and the
    /// ledger row are both consumed, so a same-key retry fails closed (the
    /// continuation is genuinely single-drive).
    ///
    /// The returned [`VerifiedContinuation`] is the only way to reach
    /// [`Self::broadcast_signed_continuation`]; the broadcast half NEVER
    /// re-verifies or re-claims.
    pub async fn verify_and_sign(
        &self,
        gate_ref: &GateRef,
        proof: &SigningProof,
    ) -> Result<VerifiedContinuation, ContinuationError>
    where
        S: CustodialSignerLike,
    {
        let binding = self
            .bindings
            .get(gate_ref)
            .await
            .ok_or(ContinuationError::MissingBinding)?;

        match binding.provider_id {
            ProviderId::Custodial => self.sign_custodial(gate_ref, &binding).await,
            external => {
                self.verify_external_wallet(gate_ref, external, &binding, proof)
                    .await
            }
        }
    }

    /// One-shot broadcast-idempotency ledger create (threats #6 / #7): an
    /// existing row for this `gate_ref` (a prior attempt) makes any re-entry fail
    /// closed. Surfaced as a dedicated idempotency-guard error carrying the
    /// existing state, rather than fabricating an `InvalidTransition` with a
    /// synthetic `to` that never actually occurred.
    async fn create_ledger_row(&self, gate_ref: &GateRef) -> Result<(), ContinuationError> {
        match self.ledger.create(gate_ref).await {
            Ok(()) => Ok(()),
            Err(ironclaw_attestation::LedgerError::AlreadyExists) => {
                let current = self.ledger.state(gate_ref).await?;
                Err(ContinuationError::LedgerRowExists { current })
            }
            Err(other) => Err(other.into()),
        }
    }

    /// Broadcast half of the continuation. Consumes a [`VerifiedContinuation`]
    /// (proof already verified + grant already claimed in
    /// [`Self::verify_and_sign`]) and broadcasts the signed bytes under the
    /// ledger guard. This NEVER calls `verify_resume` and NEVER re-claims the
    /// grant. The broadcast-failure recovery (item C) lives in the shared
    /// [`Self::broadcast_signed`] tail: a network error / ambiguous outcome moves
    /// the row to the `Unknown` terminal and surfaces a fail-closed error rather
    /// than reporting a false success.
    pub async fn broadcast_signed_continuation(
        &self,
        verified: VerifiedContinuation,
    ) -> Result<SignerContinuationOutcome, ContinuationError> {
        let VerifiedContinuation {
            gate_ref,
            context,
            signed,
            signer,
        } = verified;
        self.broadcast_signed(&gate_ref, &context, &signed, signer)
            .await
    }

    /// External-wallet verify + claim: the wallet already signed natively. We
    /// verify the proof through the bound provider (signer recovery + hash
    /// binding + one-shot sealed-grant CAS) FIRST, so a rejected proof
    /// (malformed, forged, signer/hash mismatch) never touches the ledger and
    /// never claims the grant — leaving the gate cleanly retryable. Only after
    /// the proof verifies + the grant is claimed do we create the
    /// broadcast-idempotency ledger row and advance it `Approved -> Signing ->
    /// Signed`. The wallet-signed bytes (the proof payload) become the
    /// [`VerifiedContinuation`] to broadcast.
    async fn verify_external_wallet(
        &self,
        gate_ref: &GateRef,
        provider_id: ProviderId,
        binding: &AttestedGateBinding,
        proof: &SigningProof,
    ) -> Result<VerifiedContinuation, ContinuationError> {
        let provider = self
            .providers
            .get(provider_id)
            .ok_or(ContinuationError::ProviderMismatch { bound: provider_id })?;
        debug_assert_eq!(provider.trust_model(), TrustModel::ExternalWallet);

        // Verify + claim the sealed one-shot grant (threat #1 lives inside
        // `verify_resume`) BEFORE touching the ledger. A rejected proof returns
        // here with no ledger row created and the grant unclaimed, so the gate
        // stays cleanly retryable. Verify-before-advance keeps the transition
        // atomic from the ledger's perspective — the row only moves once we hold
        // a verified proof.
        let verified = provider
            .verify_resume(&binding.context, &binding.approved_tx_hash, proof)
            .await
            .map_err(ContinuationError::ProofRejected)?;

        // Proof verified + grant claimed. Now open the broadcast-idempotency
        // ledger row and advance it to `Signed`. The grant CAS already made this
        // single-drive; the ledger guards broadcast retry (threats #6/#7).
        self.create_ledger_row(gate_ref).await?;
        self.ledger
            .advance(gate_ref, SigningLedgerState::Signing)
            .await?;
        self.ledger
            .advance(gate_ref, SigningLedgerState::Signed)
            .await?;

        let signer = binding.context.key_or_account_id.to_string();
        Ok(VerifiedContinuation {
            gate_ref: gate_ref.clone(),
            context: binding.context.clone(),
            signed: verified.proof().payload().to_vec(),
            signer,
        })
    }

    /// Custodial verify + sign: IronClaw holds the key. The driver reconstructs
    /// the signable *from `binding.decoded`* (never from any caller-supplied tx)
    /// and delegates to the [`CustodialSigner`], which runs the ship-gate,
    /// claims the sealed grant (threat #1), re-checks the approved hash
    /// (threat #3), and signs with the ecrecover binding check (threat #5). The
    /// signer advances the ledger `Approved -> Signing -> Signed` itself; the
    /// produced signature becomes the [`VerifiedContinuation`] to broadcast.
    async fn sign_custodial(
        &self,
        gate_ref: &GateRef,
        binding: &AttestedGateBinding,
    ) -> Result<VerifiedContinuation, ContinuationError>
    where
        S: CustodialSignerLike,
    {
        // Pre-flight enforcement point #2 mirror (threat #3): re-check the hash
        // from the persisted decoded tx before doing anything chain-side, so a
        // mutated binding fails closed with a precise error. The signer folded
        // into the recompute is the GATE-BOUND signer carried in the binding's
        // SigningContext (`context.key_or_account_id`) — never the decoded tx
        // body, which could have been mutated post-approval. This is the WYSIWYS
        // binding. Recompute is fallible (render/canonicalization can fail);
        // propagate that as a chain-signing error so it fails closed rather than
        // proceeding against an under-described transaction.
        let recomputed = recompute_approved_hash(
            &binding.decoded,
            binding.context.key_or_account_id.as_str(),
            binding.schema_version,
        )
        .map_err(ContinuationError::ChainSigning)?;
        if recomputed != binding.approved_tx_hash {
            return Err(ContinuationError::ApprovedHashMismatch);
        }

        // The binding's authoritative `chain` must match the chain/network its
        // OWN decoded tx encodes. This closes the testnet-chain / mainnet-tx
        // smuggle: the ship-gate keys off `binding.chain`, so the tx we sign
        // must belong to that exact chain. (The signer re-checks family; this
        // adds the precise network identity check at the driver.)
        if binding.chain.as_str() != binding.decoded.chain_network() {
            return Err(ContinuationError::BindingChainMismatch);
        }

        // Reconstruct the signable from the approved decoded tx — the only tx
        // the driver ever signs. No caller-supplied signable is accepted.
        let signable =
            rebuild::rebuild_evm_signable(&binding.decoded).map_err(ContinuationError::Rebuild)?;

        // Open the broadcast-idempotency ledger row (threats #6/#7) before the
        // custodial signer advances it `Approved -> Signing -> Signed` itself. A
        // pre-existing row (a prior attempt) fails closed here.
        self.create_ledger_row(gate_ref).await?;

        let req = CustodialSignRequest {
            context: binding.context.clone(),
            scope: binding.scope.clone(),
            chain: binding.chain.clone(),
            decoded: binding.decoded.clone(),
            approved_tx_hash: binding.approved_tx_hash,
            schema_version: binding.schema_version,
        };

        let outcome = self
            .custodial_signer
            .sign_rebuilt_evm(&req, &signable)
            .await
            .map_err(ContinuationError::ChainSigning)?;

        Ok(VerifiedContinuation {
            gate_ref: gate_ref.clone(),
            context: binding.context.clone(),
            signed: outcome.signature,
            signer: outcome.signer,
        })
    }

    /// Shared broadcast tail. For a broadcaster that actually submits
    /// ([`Broadcaster::submits`] == true), advance the ledger to
    /// `BroadcastSubmitted` BEFORE the network submit so a `Stuck->InProgress`
    /// recovery that re-enters here sees the row already at `BroadcastSubmitted`
    /// and the guard refuses a second signing (threat #7). `BroadcastSubmitted`
    /// here is the post-submit-attempt in-flight marker, NOT yet a confirmed
    /// landing.
    ///
    /// **Broadcast-failure recovery (item C):** a network error, timeout, or an
    /// ambiguous/contradictory outcome leaves the chain status genuinely unknown
    /// (the tx may or may not have been accepted). Rather than leaving the row
    /// stuck with no path forward, we move it to the [`SigningLedgerState::Unknown`]
    /// terminal and carry the attempted-tx / error evidence in the surfaced
    /// error. `Unknown` is terminal and is NEVER auto-retried with a fresh
    /// nonce/blockhash — recovery requires explicit out-of-band resolution /
    /// re-approval, so the one-shot + idempotency guarantees still hold (a
    /// retried continuation against the existing row fails closed at `create`).
    ///
    /// We confirm the ledger to a SUCCESS-only terminal contract: the returned
    /// outcome reports `BroadcastSubmitted` only on a confirmed
    /// [`BroadcastOutcome::Submitted`] with a real tx id.
    ///
    /// Follow-up: `BroadcastSubmitted` currently doubles as the in-flight marker
    /// because the `SigningLedger` state machine in `ironclaw_attestation` has no
    /// distinct `Broadcasting` state. Adding one (so `BroadcastSubmitted` strictly
    /// means "accepted by the network") is a cross-crate, additive change deferred
    /// to a follow-up to avoid colliding with the PR11 driver refactor.
    ///
    /// For a non-broadcasting (dry-run / local-dev) broadcaster, the ledger is
    /// left at `Signed` and the outcome is [`BroadcastDisposition::NotBroadcast`]:
    /// the runtime never advances to `BroadcastSubmitted` for a non-broadcast,
    /// so a signed-only continuation can never be reported as a real broadcast.
    async fn broadcast_signed(
        &self,
        gate_ref: &GateRef,
        context: &SigningContext,
        signed: &[u8],
        signer: String,
    ) -> Result<SignerContinuationOutcome, ContinuationError> {
        if self.broadcaster.submits() {
            // Real submit: advance the ledger to the in-flight marker first
            // (idempotency guard — a recovery re-entry now sees a broadcast row
            // and is refused a second signing), then submit under the guard.
            self.ledger
                .advance(gate_ref, SigningLedgerState::BroadcastSubmitted)
                .await?;
            match self.broadcaster.broadcast(context, signed).await {
                Ok(BroadcastOutcome::Submitted { tx_id }) => Ok(SignerContinuationOutcome {
                    gate_ref: gate_ref.clone(),
                    ledger_state: SigningLedgerState::BroadcastSubmitted,
                    broadcast: BroadcastDisposition::Submitted { tx_id },
                    signer,
                }),
                // A broadcaster that declared `submits() == true` but returned
                // NotBroadcast is contradictory: we cannot trust that the tx did
                // NOT go out, so treat it as an unknown-outcome failure and move
                // to the terminal recovery state rather than reporting a false
                // broadcast or a false non-broadcast.
                Ok(BroadcastOutcome::NotBroadcast { reason }) => {
                    self.recover_unknown(gate_ref).await;
                    Err(ContinuationError::Broadcast {
                        reason: format!(
                            "broadcaster declared submits()==true but did not broadcast: \
                             {reason} (ledger moved to Unknown for recovery)"
                        ),
                    })
                }
                // A broadcast error (RPC timeout, rejected/invalid response). The
                // tx may or may not have landed — genuinely unknown. Move the row
                // to the Unknown terminal so it is never left stuck and never
                // auto-rebroadcast, and surface the evidence.
                Err(err) => {
                    self.recover_unknown(gate_ref).await;
                    Err(Self::annotate_broadcast_failure(err))
                }
            }
        } else {
            // Dry-run / local-dev: never advance to BroadcastSubmitted. Record
            // the intent but leave the ledger at Signed.
            let reason = match self.broadcaster.broadcast(context, signed).await? {
                BroadcastOutcome::NotBroadcast { reason } => reason,
                // A non-submitting broadcaster that claims a real submit is
                // contradictory; fail closed.
                BroadcastOutcome::Submitted { .. } => {
                    return Err(ContinuationError::Broadcast {
                        reason: "broadcaster declared submits()==false but reported a submit"
                            .to_string(),
                    });
                }
            };
            Ok(SignerContinuationOutcome {
                gate_ref: gate_ref.clone(),
                ledger_state: SigningLedgerState::Signed,
                broadcast: BroadcastDisposition::NotBroadcast { reason },
                signer,
            })
        }
    }

    /// Move a row that has reached the in-flight `BroadcastSubmitted` marker to
    /// the [`SigningLedgerState::Unknown`] terminal after a failed/ambiguous
    /// broadcast, so it is never left stuck and never auto-rebroadcast.
    ///
    /// Best-effort: the original broadcast failure is the authoritative error we
    /// surface, so a secondary ledger error here (e.g. the row is already past
    /// `BroadcastSubmitted` due to a concurrent finalize) must NOT mask it. We
    /// only need the row to NOT be stuck at a non-terminal state; if the
    /// transition is rejected the row was already at/heading to a terminal.
    async fn recover_unknown(&self, gate_ref: &GateRef) {
        if let Err(e) = self
            .ledger
            .advance(gate_ref, SigningLedgerState::Unknown)
            .await
        {
            // An `InvalidTransition` is expected and benign here: the row is
            // already at or past a terminal. Any OTHER ledger error (e.g. a
            // backend failure) means we could NOT confirm the row is safely
            // terminal — surface it at `warn!` so it is visible in release
            // builds, not silently swallowed by a `debug_assert!` that compiles
            // out. The original broadcast error stays authoritative.
            if matches!(
                e,
                ironclaw_attestation::LedgerError::InvalidTransition { .. }
            ) {
                tracing::debug!(
                    gate_ref = %gate_ref.as_str(),
                    "recover_unknown: row already at/past a terminal state ({e:?})"
                );
            } else {
                tracing::warn!(
                    gate_ref = %gate_ref.as_str(),
                    "recover_unknown: failed to move row to Unknown terminal after a \
                     broadcast failure ({e:?}); the row may be left non-terminal"
                );
            }
        }
    }

    /// Annotate a broadcaster-side failure so the surfaced error records that the
    /// ledger was moved to the `Unknown` terminal for recovery, preserving any
    /// available evidence (the broadcaster's opaque reason / attempted tx hash)
    /// without leaking key material.
    fn annotate_broadcast_failure(err: ContinuationError) -> ContinuationError {
        match err {
            ContinuationError::Broadcast { reason } => ContinuationError::Broadcast {
                reason: format!("{reason} (ledger moved to Unknown for recovery)"),
            },
            // A non-Broadcast error from the broadcaster path is unexpected but
            // still fail-closed; pass it through unchanged.
            other => other,
        }
    }
}

/// Abstracts the custodial signer so the driver is not generic over the
/// concrete [`CustodialSigner`] type parameters and never has to accept a
/// caller-supplied signable. The driver hands the signer the EVM signable it
/// reconstructed from the authoritative decoded binding ([`rebuild::EvmSignable`]),
/// and the signer runs both enforcement points and the ecrecover binding check.
#[async_trait]
pub trait CustodialSignerLike: Send + Sync {
    /// Sign the EVM signable the driver rebuilt from `req.decoded`, dispatching
    /// on its concrete tx type. Runs both enforcement points and the ecrecover
    /// binding check.
    async fn sign_rebuilt_evm(
        &self,
        req: &CustodialSignRequest,
        signable: &rebuild::EvmSignable,
    ) -> Result<ironclaw_chain_signing::CustodialSignOutcome, ChainSigningError>;
}

#[async_trait]
impl<K, G, L> CustodialSignerLike for CustodialSigner<K, G, L>
where
    K: ironclaw_chain_signing::KeyStore,
    G: ironclaw_attestation::SealedGrantStore,
    L: SigningLedger,
{
    async fn sign_rebuilt_evm(
        &self,
        req: &CustodialSignRequest,
        signable: &rebuild::EvmSignable,
    ) -> Result<ironclaw_chain_signing::CustodialSignOutcome, ChainSigningError> {
        // The custodial signer now RECONSTRUCTS the signable transaction itself
        // from `req.decoded` (the authoritative decoded tx the approved hash was
        // computed over), so it never accepts a caller-supplied signable that
        // could drift from the approved one (chain_signing review finding #1).
        // The driver's own pre-rebuild (`signable`) is retained as the fail-fast
        // decodability check before any key access; it is not forwarded, because
        // the signer rebuilds from the same `req.decoded` and re-checks the
        // approved hash (enforcement point #2) internally.
        let _ = signable;
        CustodialSigner::sign_evm(self, req).await
    }
}
