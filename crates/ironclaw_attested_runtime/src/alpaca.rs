//! The Rust side of the Alpaca sidecar contract (attested-signing §E2).
//!
//! ## The sidecar is untrusted input
//!
//! Everything authoritative stays in Rust: the canonical bytes, the
//! [`ApprovedTxHash`](ironclaw_signing_provider::ApprovedTxHash), the signer
//! identity, the one-shot grant CAS, and broadcast admission. The sidecar
//! *proposes* a crafted transaction and *executes* a submit; it decides
//! nothing. A crafted proposal is only ever accepted after
//! `ironclaw_attestation` decodes and canonicalizes it — that decode IS the
//! equivalence check, and anything Rust cannot decode fails closed before a
//! gate is raised.
//!
//! ## Why this is a port
//!
//! [`AlpacaPort`] is the test seam. Every Rust test drives
//! [`RecordingAlpacaPort`] rather than a live process, so `cargo test` needs no
//! Node toolchain and no network. The production implementation (HTTP over the
//! Unix socket) is a separate, thin adapter.
//!
//! ## The bytes invariant this port exists to protect
//!
//! At `combine` and `broadcast` the caller passes bytes reconstructed **from
//! the authoritative binding**, never bytes the sidecar returned earlier. The
//! recording double captures every argument precisely so a test can assert
//! that — a double that dropped arguments would let a re-craft slip through
//! unnoticed.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;

/// Chain selector: the Ledger currency id the sidecar was configured with.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CurrencyId(String);

impl CurrencyId {
    /// Wrap a currency id.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// The id as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A transaction proposal request.
///
/// Deliberately opaque `params`: the sidecar owns the per-chain intent schema,
/// and Rust does not re-model it — Rust's authority comes from DECODING what
/// comes back, not from having described what went in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CraftRequest {
    /// Which chain.
    pub currency_id: CurrencyId,
    /// Chain-specific craft intent, passed through verbatim.
    pub params: serde_json::Value,
}

/// A signature attached to a crafted transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CombineRequest {
    /// Which chain.
    pub currency_id: CurrencyId,
    /// The unsigned transaction, reconstructed FROM THE BINDING by the caller.
    /// Never the sidecar's own earlier output.
    pub unsigned_tx: String,
    /// The signature the device produced.
    pub signature: String,
}

/// A submit request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BroadcastRequest {
    /// Which chain.
    pub currency_id: CurrencyId,
    /// The signed raw transaction, derived from the binding.
    pub raw_tx: String,
}

/// Why an Alpaca call failed.
///
/// Mirrors the sidecar's coarse categories. It stays coarse on purpose: the
/// sidecar is untrusted at this boundary, so a richer error surface would only
/// invite the backend to trust its self-description more than it should.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum AlpacaError {
    /// The sidecar is not configured, not running, or unreachable.
    #[error("alpaca sidecar unavailable: {reason}")]
    Unavailable {
        /// Sanitized description.
        reason: String,
    },

    /// The sidecar rejected the request shape or the wire version.
    #[error("alpaca rejected the request: {reason}")]
    BadRequest {
        /// Sanitized description.
        reason: String,
    },

    /// The sidecar has no api for that chain.
    #[error("alpaca has no api for that chain")]
    UnsupportedChain,

    /// The chain RPC failed or timed out upstream of the sidecar.
    ///
    /// Retry-safety is NOT decided here: the caller's idempotency ledger
    /// governs whether a broadcast may be re-driven, and a timeout on broadcast
    /// specifically must never auto-retry (the submit may have landed).
    #[error("alpaca upstream failure: {reason}")]
    Upstream {
        /// Sanitized description.
        reason: String,
    },
}

/// The sidecar call surface Rust actually uses.
///
/// Only the three flow-critical methods are modelled. Fee/balance/chain-height
/// reads are advisory and can be added when something needs them; keeping the
/// port minimal keeps the trusted surface minimal.
#[async_trait]
pub trait AlpacaPort: Send + Sync {
    /// Propose a crafted transaction. The result is UNTRUSTED until
    /// `ironclaw_attestation` decodes and canonicalizes it.
    async fn craft_transaction(&self, request: CraftRequest) -> Result<String, AlpacaError>;

    /// Attach a signature to the binding-derived unsigned transaction.
    async fn combine(&self, request: CombineRequest) -> Result<String, AlpacaError>;

    /// Submit a signed transaction. The caller MUST have admitted this through
    /// the idempotency ledger first.
    async fn broadcast(&self, request: BroadcastRequest) -> Result<String, AlpacaError>;

    /// Liveness probe for the supervisor.
    async fn healthy(&self) -> bool;
}

/// Recording test double.
///
/// Lock access recovers from poisoning rather than panicking: a poisoned mutex
/// here only means a previous test panicked while holding it, the recorded data
/// is still sound, and a recorder that panics turns one failing test into a
/// cascade of confusing ones.
///
/// Captures EVERY argument of every call (the mock-hygiene rule): a test must
/// be able to assert that `combine`/`broadcast` received exactly the
/// binding-derived bytes, and a double that dropped fields could not show that.
pub struct RecordingAlpacaPort {
    crafts: Mutex<Vec<CraftRequest>>,
    combines: Mutex<Vec<CombineRequest>>,
    broadcasts: Mutex<Vec<BroadcastRequest>>,
    craft_response: Mutex<Result<String, AlpacaError>>,
    combine_response: Mutex<Result<String, AlpacaError>>,
    broadcast_response: Mutex<Result<String, AlpacaError>>,
    healthy: Mutex<bool>,
}

impl Default for RecordingAlpacaPort {
    fn default() -> Self {
        Self {
            crafts: Mutex::new(Vec::new()),
            combines: Mutex::new(Vec::new()),
            broadcasts: Mutex::new(Vec::new()),
            craft_response: Mutex::new(Ok("0xcrafted".to_string())),
            combine_response: Mutex::new(Ok("0xcombined".to_string())),
            broadcast_response: Mutex::new(Ok("0xtxid".to_string())),
            healthy: Mutex::new(true),
        }
    }
}

impl RecordingAlpacaPort {
    /// A double that succeeds with placeholder values.
    pub fn new() -> Self {
        Self::default()
    }

    /// Script the craft response (including a failure).
    pub fn with_craft_response(self, response: Result<String, AlpacaError>) -> Self {
        *self
            .craft_response
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = response;
        self
    }

    /// Script the combine response.
    pub fn with_combine_response(self, response: Result<String, AlpacaError>) -> Self {
        *self
            .combine_response
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = response;
        self
    }

    /// Script the broadcast response.
    pub fn with_broadcast_response(self, response: Result<String, AlpacaError>) -> Self {
        *self
            .broadcast_response
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = response;
        self
    }

    /// Script the health probe.
    pub fn with_healthy(self, healthy: bool) -> Self {
        *self
            .healthy
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = healthy;
        self
    }

    /// Every craft request received, in order.
    pub fn crafts(&self) -> Vec<CraftRequest> {
        self.crafts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// Every combine request received, in order.
    pub fn combines(&self) -> Vec<CombineRequest> {
        self.combines
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// Every broadcast request received, in order.
    pub fn broadcasts(&self) -> Vec<BroadcastRequest> {
        self.broadcasts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

#[async_trait]
impl AlpacaPort for RecordingAlpacaPort {
    async fn craft_transaction(&self, request: CraftRequest) -> Result<String, AlpacaError> {
        self.crafts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(request);
        self.craft_response
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    async fn combine(&self, request: CombineRequest) -> Result<String, AlpacaError> {
        self.combines
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(request);
        self.combine_response
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    async fn broadcast(&self, request: BroadcastRequest) -> Result<String, AlpacaError> {
        self.broadcasts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(request);
        self.broadcast_response
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    async fn healthy(&self) -> bool {
        *self
            .healthy
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// A port that refuses every call.
///
/// The wiring for a deployment with `ATTESTED_ALPACA_SIDECAR=off`: chains that
/// need the sidecar fail closed with a clear configuration error instead of
/// silently taking some other path, while chains `ironclaw_chain_signing`
/// covers natively are unaffected because they never reach this port.
pub struct UnconfiguredAlpacaPort;

#[async_trait]
impl AlpacaPort for UnconfiguredAlpacaPort {
    async fn craft_transaction(&self, _request: CraftRequest) -> Result<String, AlpacaError> {
        Err(unconfigured())
    }

    async fn combine(&self, _request: CombineRequest) -> Result<String, AlpacaError> {
        Err(unconfigured())
    }

    async fn broadcast(&self, _request: BroadcastRequest) -> Result<String, AlpacaError> {
        Err(unconfigured())
    }

    async fn healthy(&self) -> bool {
        false
    }
}

fn unconfigured() -> AlpacaError {
    AlpacaError::Unavailable {
        reason: "no alpaca sidecar is configured for this deployment".to_string(),
    }
}

/// Convenience alias for the shared port handle composition passes around.
pub type SharedAlpacaPort = Arc<dyn AlpacaPort>;

#[cfg(test)]
mod tests {
    use super::*;

    fn currency() -> CurrencyId {
        CurrencyId::new("ethereum_sepolia")
    }

    #[tokio::test]
    async fn the_double_captures_every_argument_of_every_call() {
        let port = RecordingAlpacaPort::new();

        port.craft_transaction(CraftRequest {
            currency_id: currency(),
            params: serde_json::json!({"nonce": 7}),
        })
        .await
        .expect("craft");
        port.combine(CombineRequest {
            currency_id: currency(),
            unsigned_tx: "0xfromthebinding".to_string(),
            signature: "0xsig".to_string(),
        })
        .await
        .expect("combine");
        port.broadcast(BroadcastRequest {
            currency_id: currency(),
            raw_tx: "0xsignedfromthebinding".to_string(),
        })
        .await
        .expect("broadcast");

        // Mock hygiene: every field of every call is recoverable, which is what
        // lets a caller-side test assert the §1.11 bytes invariant.
        let craft = port.crafts();
        assert_eq!(craft.len(), 1);
        assert_eq!(craft[0].currency_id, currency());
        assert_eq!(craft[0].params, serde_json::json!({"nonce": 7}));

        let combine = port.combines();
        assert_eq!(combine[0].unsigned_tx, "0xfromthebinding");
        assert_eq!(combine[0].signature, "0xsig");
        assert_eq!(combine[0].currency_id, currency());

        let broadcast = port.broadcasts();
        assert_eq!(broadcast[0].raw_tx, "0xsignedfromthebinding");
        assert_eq!(broadcast[0].currency_id, currency());
    }

    /// The failure the plan calls out specifically: a craft failure must be
    /// clean, because no gate has been raised yet and there is nothing to
    /// unwind.
    #[tokio::test]
    async fn a_scripted_craft_failure_surfaces_unchanged() {
        let port = RecordingAlpacaPort::new().with_craft_response(Err(AlpacaError::Upstream {
            reason: "rpc timeout".to_string(),
        }));
        assert_eq!(
            port.craft_transaction(CraftRequest {
                currency_id: currency(),
                params: serde_json::Value::Null,
            })
            .await,
            Err(AlpacaError::Upstream {
                reason: "rpc timeout".to_string()
            })
        );
        // The attempt is still recorded — a caller-side test can assert the
        // port was reached exactly once even on the failure path.
        assert_eq!(port.crafts().len(), 1);
    }

    /// `ATTESTED_ALPACA_SIDECAR=off`: every call fails closed with a
    /// configuration error, and the probe reports unhealthy so a supervisor
    /// never believes it is running.
    #[tokio::test]
    async fn the_unconfigured_port_fails_closed_on_every_call() {
        let port = UnconfiguredAlpacaPort;
        assert!(!port.healthy().await);
        assert!(matches!(
            port.craft_transaction(CraftRequest {
                currency_id: currency(),
                params: serde_json::Value::Null,
            })
            .await,
            Err(AlpacaError::Unavailable { .. })
        ));
        assert!(matches!(
            port.combine(CombineRequest {
                currency_id: currency(),
                unsigned_tx: "0x".to_string(),
                signature: "0x".to_string(),
            })
            .await,
            Err(AlpacaError::Unavailable { .. })
        ));
        assert!(matches!(
            port.broadcast(BroadcastRequest {
                currency_id: currency(),
                raw_tx: "0x".to_string(),
            })
            .await,
            Err(AlpacaError::Unavailable { .. })
        ));
    }

    #[tokio::test]
    async fn the_health_probe_is_scriptable_for_supervisor_tests() {
        assert!(RecordingAlpacaPort::new().healthy().await);
        assert!(
            !RecordingAlpacaPort::new()
                .with_healthy(false)
                .healthy()
                .await
        );
    }
}
