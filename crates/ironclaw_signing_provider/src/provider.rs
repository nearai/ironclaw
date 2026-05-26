//! The provider identity / trust-model enums and the [`SigningProvider`]
//! trait itself.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::context::SigningContext;
use crate::error::SigningProviderError;
use crate::proof::{SigningProof, VerifiedProof};
use crate::transaction::{ApprovedTxHash, DecodedTransaction, RenderedTx};

/// Wire-stable identity of a signing backend.
///
/// `#[serde(rename_all = "snake_case")]` pins the wire form (see
/// `.claude/rules/types.md`): these tags appear in persisted gate state and on
/// the resume wire, so they must not drift.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderId {
    /// WalletConnect v2 (PR9).
    WalletConnect,
    /// Browser injected provider (`window.ethereum` / `window.solana`, PR7).
    Injected,
    /// NEAR browser-wallet redirect protocol (PR8).
    NearRedirect,
    /// Custodial keys + WebAuthn (PR4 / PR6).
    Custodial,
}

/// Which trust model a provider operates under.
///
/// See the plan's "Two Trust Models" section: external wallets hold their own
/// keys and render+sign the real transaction (true WYSIWYS); custodial keys are
/// held by IronClaw and a WebAuthn assertion authorizes signing (weaker,
/// IronClaw-rendered WYSIWYS).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrustModel {
    /// The user's wallet holds the keys and signs; IronClaw never has custody.
    ExternalWallet,
    /// IronClaw holds the keys; a WebAuthn assertion authorizes signing.
    Custodial,
}

/// The outcome of [`SigningProvider::initiate`].
///
/// `initiate` does not sign. It prepares whatever the chosen provider needs to
/// drive the human-in-the-loop ceremony — a redirect / deep-link the user must
/// visit, or an indication that the next step happens entirely client-side. The
/// opaque directive bytes are interpreted by the channel that owns the user
/// interaction (PR7+).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum InitiationOutcome {
    /// The user must be sent to an external surface (a WalletConnect pairing
    /// URI, a NEAR wallet redirect, a browser prompt). The opaque directive is
    /// rendered / launched by the owning channel.
    AwaitingUserAction {
        /// Opaque, provider-specific directive describing the action the user
        /// must take (e.g. a deep-link or pairing payload). Not interpreted by
        /// this crate.
        directive: Vec<u8>,
    },
    /// The ceremony proceeds without a server-issued directive (e.g. an
    /// already-paired injected provider that prompts in-page).
    ReadyForProof,
}

/// A provider-agnostic signing backend.
///
/// Implementations bridge a specific wallet / authn mechanism to the
/// attested-signing gate. The trait is intentionally small and
/// chain/crypto-free at this layer:
///
/// * [`initiate`](SigningProvider::initiate) prepares the human-in-the-loop
///   ceremony for an already-decoded, already-rendered, already-bound
///   transaction. It must not sign or broadcast.
/// * [`verify_resume`](SigningProvider::verify_resume) validates the proof
///   carried back from the ceremony against the bound [`ApprovedTxHash`] and
///   [`SigningContext`], returning a [`VerifiedProof`] on success. It must not
///   broadcast — broadcast (chain I/O) lives in the reborn/runner layer (see
///   the plan's deterministic post-approval continuation invariant).
///
/// The trait is object-safe so providers can be held as
/// `std::sync::Arc<dyn SigningProvider>` in a registry.
#[async_trait]
pub trait SigningProvider: Send + Sync {
    /// The wire-stable identity of this provider.
    fn provider_id(&self) -> ProviderId;

    /// The trust model this provider operates under.
    fn trust_model(&self) -> TrustModel;

    /// Prepare the human-in-the-loop ceremony for a bound transaction.
    ///
    /// Receives the decoded transaction, its rendered view, the binding hash,
    /// and the signing context. Returns what the owning channel must do next.
    /// Must not sign or broadcast.
    async fn initiate(
        &self,
        context: &SigningContext,
        decoded: &DecodedTransaction,
        rendered: &RenderedTx,
        approved_tx_hash: &ApprovedTxHash,
    ) -> Result<InitiationOutcome, SigningProviderError>;

    /// Validate a proof carried back from the ceremony.
    ///
    /// Checks the proof against the bound [`ApprovedTxHash`] and the
    /// [`SigningContext`] (signer match, scope, proof validity). Returns a
    /// [`VerifiedProof`] on success. Must not broadcast.
    async fn verify_resume(
        &self,
        context: &SigningContext,
        approved_tx_hash: &ApprovedTxHash,
        proof: &SigningProof,
    ) -> Result<VerifiedProof, SigningProviderError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::{
        ActorId, ChainId, GateRef, KeyOrAccountId, RunId, ScopeId, TenantId, UserId,
    };
    use std::sync::Arc;

    fn sample_context() -> SigningContext {
        SigningContext {
            tenant: TenantId::new("t"),
            user: UserId::new("u"),
            scope: ScopeId::new("s"),
            actor: ActorId::new("a"),
            run_id: RunId::new("r"),
            gate_ref: GateRef::new("gate:1"),
            chain_id: ChainId::new("eip155:1"),
            key_or_account_id: KeyOrAccountId::new("0xabc"),
        }
    }

    #[test]
    fn provider_id_uses_snake_case_wire_form() {
        let cases = [
            (ProviderId::WalletConnect, "\"wallet_connect\""),
            (ProviderId::Injected, "\"injected\""),
            (ProviderId::NearRedirect, "\"near_redirect\""),
            (ProviderId::Custodial, "\"custodial\""),
        ];
        for (id, expected) in cases {
            let json = serde_json::to_string(&id).expect("serialize");
            assert_eq!(json, expected);
            let back: ProviderId = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(back, id);
        }
    }

    #[test]
    fn trust_model_uses_snake_case_wire_form() {
        assert_eq!(
            serde_json::to_string(&TrustModel::ExternalWallet).expect("ser"),
            "\"external_wallet\""
        );
        assert_eq!(
            serde_json::to_string(&TrustModel::Custodial).expect("ser"),
            "\"custodial\""
        );
    }

    #[test]
    fn initiation_outcome_round_trips() {
        let outcome = InitiationOutcome::AwaitingUserAction {
            directive: vec![1, 2, 3],
        };
        let json = serde_json::to_string(&outcome).expect("ser");
        let back: InitiationOutcome = serde_json::from_str(&json).expect("de");
        assert_eq!(back, outcome);

        let ready = InitiationOutcome::ReadyForProof;
        let back2: InitiationOutcome =
            serde_json::from_str(&serde_json::to_string(&ready).expect("ser")).expect("de");
        assert_eq!(back2, ready);
    }

    /// A minimal mock proving the trait is object-safe and usable behind
    /// `Arc<dyn SigningProvider>`.
    struct MockProvider {
        id: ProviderId,
        trust: TrustModel,
        accept: bool,
    }

    #[async_trait]
    impl SigningProvider for MockProvider {
        fn provider_id(&self) -> ProviderId {
            self.id
        }

        fn trust_model(&self) -> TrustModel {
            self.trust
        }

        async fn initiate(
            &self,
            _context: &SigningContext,
            _decoded: &DecodedTransaction,
            _rendered: &RenderedTx,
            _approved_tx_hash: &ApprovedTxHash,
        ) -> Result<InitiationOutcome, SigningProviderError> {
            Ok(InitiationOutcome::ReadyForProof)
        }

        async fn verify_resume(
            &self,
            _context: &SigningContext,
            approved_tx_hash: &ApprovedTxHash,
            proof: &SigningProof,
        ) -> Result<VerifiedProof, SigningProviderError> {
            if self.accept {
                Ok(VerifiedProof::new(
                    self.id,
                    *approved_tx_hash,
                    proof.clone(),
                ))
            } else {
                Err(SigningProviderError::SignerMismatch)
            }
        }
    }

    #[tokio::test]
    async fn mock_provider_is_object_safe_and_drivable_via_dyn_arc() {
        let provider: Arc<dyn SigningProvider> = Arc::new(MockProvider {
            id: ProviderId::Injected,
            trust: TrustModel::ExternalWallet,
            accept: true,
        });
        assert_eq!(provider.provider_id(), ProviderId::Injected);
        assert_eq!(provider.trust_model(), TrustModel::ExternalWallet);

        let ctx = sample_context();
        let decoded = DecodedTransaction::from_opaque(vec![0]);
        let rendered = RenderedTx::from_opaque(vec![0]);
        let hash = ApprovedTxHash::from_bytes([0u8; 32]);

        let outcome = provider
            .initiate(&ctx, &decoded, &rendered, &hash)
            .await
            .expect("initiate");
        assert_eq!(outcome, InitiationOutcome::ReadyForProof);

        let proof = SigningProof::InjectedProof(vec![1, 2]);
        let verified = provider
            .verify_resume(&ctx, &hash, &proof)
            .await
            .expect("verify");
        assert_eq!(verified.proof(), &proof);
    }

    #[tokio::test]
    async fn mock_provider_surfaces_signer_mismatch() {
        let provider: Arc<dyn SigningProvider> = Arc::new(MockProvider {
            id: ProviderId::Custodial,
            trust: TrustModel::Custodial,
            accept: false,
        });
        let ctx = sample_context();
        let hash = ApprovedTxHash::from_bytes([0u8; 32]);
        let err = provider
            .verify_resume(&ctx, &hash, &SigningProof::WebAuthnAssertionProof(vec![1]))
            .await
            .expect_err("should reject");
        assert!(matches!(err, SigningProviderError::SignerMismatch));
    }
}
