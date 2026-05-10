//! Idempotency ledger port for the product workflow saga.
//!
//! The ledger records inbound action fingerprints so that duplicate webhook
//! deliveries or client retries are detected and replay the prior outcome
//! instead of re-executing side effects.

use async_trait::async_trait;

use crate::action::{ActionFingerprintKey, ProductInboundAction};
use crate::error::ProductWorkflowError;

/// Port for the durable inbound action idempotency ledger.
///
/// Host runtimes provide a durable implementation backed by the DB layer.
/// Tests use the in-memory fake from the `test-support` feature.
///
/// # Non-terminal action recovery contract
///
/// `begin_or_replay` reserves a fingerprint by recording a `Received` action
/// before workflow dispatch starts. A durable implementation must not leave that
/// reservation permanent if the process crashes or a transient dispatch failure
/// prevents `settle` from recording a terminal outcome. Implementations must use
/// a recovery lease/TTL (or an equivalent background sweeper) for non-terminal
/// `Received`/`Dispatched` actions:
///
/// - settled actions replay forever;
/// - fresh non-terminal actions return a transient error so callers retry later
///   instead of dispatching the same side effect concurrently;
/// - expired non-terminal actions are reclaimed by the sweeper or by
///   `begin_or_replay` before returning `New` for the fingerprint again.
///
/// The test-support fake models fresh in-flight reservations and exposes
/// `expire_in_flight_before` to simulate the sweeper path in downstream tests.
#[async_trait]
pub trait IdempotencyLedger: Send + Sync {
    /// Attempt to begin a new action for the given fingerprint.
    ///
    /// If a prior action with the same fingerprint already exists and has
    /// settled, the ledger returns the prior action record (allowing the
    /// caller to replay the outcome). If a prior action exists but is not
    /// yet settled, the ledger should return a transient error so the
    /// caller can retry or wait.
    async fn begin_or_replay(
        &self,
        fingerprint: ActionFingerprintKey,
        received_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<IdempotencyDecision, ProductWorkflowError>;

    /// Settle an in-progress action with a terminal outcome.
    async fn settle(&self, action: ProductInboundAction) -> Result<(), ProductWorkflowError>;
}

/// Result of an idempotency check.
#[derive(Debug, Clone)]
pub enum IdempotencyDecision {
    /// This is a new action; proceed with dispatch.
    New(ProductInboundAction),
    /// A prior action with the same fingerprint already settled. Replay it.
    Replay(ProductInboundAction),
}

impl IdempotencyDecision {
    /// Whether this is a new action.
    pub fn is_new(&self) -> bool {
        matches!(self, Self::New(_))
    }

    /// Whether this is a replay of a prior action.
    pub fn is_replay(&self) -> bool {
        matches!(self, Self::Replay(_))
    }

    /// Get the action record regardless of decision kind.
    pub fn action(&self) -> &ProductInboundAction {
        match self {
            Self::New(action) | Self::Replay(action) => action,
        }
    }
}
