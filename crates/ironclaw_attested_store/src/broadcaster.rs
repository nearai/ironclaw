//! The real per-chain [`Broadcaster`] wired under the signing-ledger state
//! machine.
//!
//! The driver's [`Broadcaster`] trait is chain-agnostic
//! (`broadcast(context, signed) -> tx_id`); this implementation routes to the
//! per-chain JSON-RPC broadcaster (`eth_sendRawTransaction` /
//! `sendTransaction` / `broadcast_tx_async`) selected from
//! `context.chain_id`'s family, using RPC endpoints supplied from config.
//!
//! Every per-chain broadcaster is a ONE-SHOT submitter of an already-signed
//! payload — none re-signs, bumps a nonce, or refreshes a blockhash. The driver
//! advances the [`ironclaw_attestation::SigningLedger`] to `BroadcastSubmitted`
//! around this call, so a `Stuck -> InProgress` recovery that re-enters
//! continuation hits the broadcast-idempotency guard and can never produce a
//! second submission with fresh chain metadata.
//!
//! Endpoints are injected (the composition layer resolves them from config and
//! applies the network allowlist); this type never hard-codes a URL.

#![cfg(feature = "broadcast-http")]

use async_trait::async_trait;

use ironclaw_attested_runtime::{BroadcastOutcome, Broadcaster, ContinuationError};
use ironclaw_chain_signing::evm::{EvmBroadcaster, JsonRpcEvmBroadcaster};
use ironclaw_chain_signing::near::{JsonRpcNearBroadcaster, NearBroadcaster};
use ironclaw_chain_signing::solana::{JsonRpcSolanaBroadcaster, SolanaBroadcaster};
use ironclaw_signing_provider::SigningContext;

/// Per-chain RPC endpoints, resolved from config by the composition layer.
///
/// A `None` endpoint means the chain family is not configured for broadcast;
/// an attempt to broadcast for it fails closed (no submission).
#[derive(Debug, Clone, Default)]
pub struct ChainRpcEndpoints {
    /// EVM JSON-RPC URL (`eth_sendRawTransaction`).
    pub evm: Option<String>,
    /// Solana JSON-RPC URL (`sendTransaction`).
    pub solana: Option<String>,
    /// NEAR JSON-RPC URL (`broadcast_tx_async`).
    pub near: Option<String>,
}

/// Routes signed payloads to the per-chain live broadcaster by chain family.
pub struct MultiChainBroadcaster {
    evm: Option<JsonRpcEvmBroadcaster>,
    solana: Option<JsonRpcSolanaBroadcaster>,
    near: Option<JsonRpcNearBroadcaster>,
}

impl MultiChainBroadcaster {
    /// Build from the configured endpoints. Each configured family gets a live
    /// JSON-RPC broadcaster; unconfigured families fail closed at broadcast time.
    pub fn from_endpoints(endpoints: ChainRpcEndpoints) -> Result<Self, ContinuationError> {
        let evm = match endpoints.evm {
            Some(url) => Some(JsonRpcEvmBroadcaster::new(url).map_err(|error| {
                ContinuationError::Broadcast {
                    reason: error.to_string(),
                }
            })?),
            None => None,
        };
        let solana = match endpoints.solana {
            Some(url) => Some(JsonRpcSolanaBroadcaster::new(url).map_err(|error| {
                ContinuationError::Broadcast {
                    reason: error.to_string(),
                }
            })?),
            None => None,
        };
        let near = match endpoints.near {
            Some(url) => Some(JsonRpcNearBroadcaster::new(url).map_err(|error| {
                ContinuationError::Broadcast {
                    reason: error.to_string(),
                }
            })?),
            None => None,
        };
        Ok(Self { evm, solana, near })
    }
}

#[async_trait]
impl Broadcaster for MultiChainBroadcaster {
    /// A real submitter: the driver advances the ledger to `BroadcastSubmitted`
    /// around a confirmed [`BroadcastOutcome::Submitted`].
    fn submits(&self) -> bool {
        true
    }

    async fn broadcast(
        &self,
        context: &SigningContext,
        signed: &[u8],
    ) -> Result<BroadcastOutcome, ContinuationError> {
        let chain = context.chain_id.as_str();
        let tx_id = if chain.starts_with("eip155:") {
            let broadcaster = self.evm.as_ref().ok_or(ContinuationError::Broadcast {
                reason: "no EVM RPC endpoint configured".to_string(),
            })?;
            let outcome = broadcaster.send_raw(signed).await.map_err(|error| {
                ContinuationError::Broadcast {
                    reason: error.to_string(),
                }
            })?;
            format!("0x{}", hex_encode(&outcome.tx_hash))
        } else if chain.starts_with("solana:") {
            let broadcaster = self.solana.as_ref().ok_or(ContinuationError::Broadcast {
                reason: "no Solana RPC endpoint configured".to_string(),
            })?;
            let outcome = broadcaster
                .send_transaction(signed)
                .await
                .map_err(|error| ContinuationError::Broadcast {
                    reason: error.to_string(),
                })?;
            bs58_encode(&outcome.signature)
        } else if chain.starts_with("near:") {
            let broadcaster = self.near.as_ref().ok_or(ContinuationError::Broadcast {
                reason: "no NEAR RPC endpoint configured".to_string(),
            })?;
            let outcome = broadcaster.broadcast_tx(signed).await.map_err(|error| {
                ContinuationError::Broadcast {
                    reason: error.to_string(),
                }
            })?;
            outcome.tx_hash
        } else {
            // Fail closed on an unrecognized chain (never submit blind).
            return Err(ContinuationError::Broadcast {
                reason: format!("unrecognized chain id for broadcast: {chain}"),
            });
        };
        Ok(BroadcastOutcome::Submitted { tx_id })
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    hex::encode(bytes)
}

fn bs58_encode(bytes: &[u8]) -> String {
    bs58::encode(bytes).into_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_signing_provider::{
        ActorId, ChainId, GateRef, KeyOrAccountId, RunId, ScopeId, TenantId, UserId,
    };

    /// A [`SigningContext`] whose only test-relevant field is `chain_id`; the
    /// broadcaster routes purely on the chain-id family prefix.
    fn ctx_for_chain(chain_id: &str) -> SigningContext {
        SigningContext {
            tenant: TenantId::new("tenant-a"),
            user: UserId::new("user-1"),
            scope: ScopeId::new("scope-x"),
            actor: ActorId::new("actor-7"),
            run_id: RunId::new("run-42"),
            gate_ref: GateRef::new("gate:abc"),
            chain_id: ChainId::new(chain_id),
            key_or_account_id: KeyOrAccountId::new("0xabc"),
        }
    }

    fn broadcast_reason(err: ContinuationError) -> String {
        match err {
            ContinuationError::Broadcast { reason } => reason,
            other => panic!("expected Broadcast error, got {other:?}"),
        }
    }

    /// With NO endpoints configured, every recognized chain family must fail
    /// closed at broadcast time — never submit blind to a default RPC. This is
    /// the security-critical missing-endpoint guard.
    #[tokio::test]
    async fn fails_closed_on_missing_endpoint_per_family() {
        let broadcaster = MultiChainBroadcaster::from_endpoints(ChainRpcEndpoints::default())
            .expect("constructing with no endpoints must succeed");
        let signed = [0xAAu8; 8];

        for (chain, expected) in [
            ("eip155:1", "no EVM RPC endpoint configured"),
            ("solana:mainnet", "no Solana RPC endpoint configured"),
            ("near:mainnet", "no NEAR RPC endpoint configured"),
        ] {
            let err = broadcaster
                .broadcast(&ctx_for_chain(chain), &signed)
                .await
                .expect_err("missing endpoint must fail closed");
            assert_eq!(broadcast_reason(err), expected, "chain {chain}");
        }
    }

    /// An unrecognized chain-id family must fail closed rather than route to any
    /// configured broadcaster — even when every family IS configured.
    #[tokio::test]
    async fn fails_closed_on_unrecognized_chain() {
        let broadcaster = MultiChainBroadcaster::from_endpoints(ChainRpcEndpoints {
            evm: Some("https://rpc.example.com".to_string()),
            solana: Some("https://rpc.example.com".to_string()),
            near: Some("https://rpc.example.com".to_string()),
        })
        .expect("construct");

        let err = broadcaster
            .broadcast(&ctx_for_chain("bitcoin:mainnet"), &[0x01])
            .await
            .expect_err("unknown chain must fail closed");
        let reason = broadcast_reason(err);
        assert!(
            reason.contains("unrecognized chain id"),
            "reason was: {reason}"
        );
        assert!(reason.contains("bitcoin:mainnet"), "reason was: {reason}");
    }

    /// Routing is selected by chain-family prefix: an EVM chain id with the EVM
    /// endpoint UNset still reports the EVM-specific missing-endpoint message,
    /// proving the `eip155:` arm was selected (not Solana/NEAR) even when the
    /// other families ARE configured. This pins the dispatch table without a
    /// live RPC server.
    #[tokio::test]
    async fn routes_to_evm_arm_by_chain_family() {
        let broadcaster = MultiChainBroadcaster::from_endpoints(ChainRpcEndpoints {
            evm: None,
            solana: Some("https://rpc.example.com".to_string()),
            near: Some("https://rpc.example.com".to_string()),
        })
        .expect("construct");

        let err = broadcaster
            .broadcast(&ctx_for_chain("eip155:8453"), &[0x02; 4])
            .await
            .expect_err("EVM arm with no EVM endpoint fails closed");
        assert_eq!(broadcast_reason(err), "no EVM RPC endpoint configured");
    }

    /// `submits()` is `true`: the driver advances the ledger to
    /// `BroadcastSubmitted` only around a broadcaster that actually submits.
    #[tokio::test]
    async fn reports_it_submits() {
        let broadcaster =
            MultiChainBroadcaster::from_endpoints(ChainRpcEndpoints::default()).expect("construct");
        assert!(broadcaster.submits());
    }
}
