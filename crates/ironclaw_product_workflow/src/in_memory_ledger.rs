//! In-memory product workflow idempotency ledger.
//!
//! This implementation is suitable for local-dev composition and deterministic
//! integration tests. Durable production deployments should wire a database
//! ledger with the same lease semantics.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};

use crate::{
    ActionFingerprintKey, IdempotencyDecision, IdempotencyLedger, ProductInboundAction,
    ProductWorkflowError,
};

const DEFAULT_IN_FLIGHT_LEASE: Duration = Duration::seconds(60);

/// Local in-memory implementation of [`IdempotencyLedger`].
#[derive(Clone)]
pub struct InMemoryIdempotencyLedger {
    state: Arc<Mutex<InMemoryIdempotencyState>>,
    in_flight_lease: Duration,
}

#[derive(Default)]
struct InMemoryIdempotencyState {
    in_flight: HashMap<ActionFingerprintKey, ProductInboundAction>,
    settled: HashMap<ActionFingerprintKey, ProductInboundAction>,
}

impl InMemoryIdempotencyLedger {
    pub fn new() -> Self {
        Self::with_in_flight_lease(DEFAULT_IN_FLIGHT_LEASE)
    }

    pub fn with_in_flight_lease(in_flight_lease: Duration) -> Self {
        Self {
            state: Arc::new(Mutex::new(InMemoryIdempotencyState::default())),
            in_flight_lease,
        }
    }

    /// Reclaim expired non-terminal reservations. Exposed so local-dev hosts
    /// and tests can model the same recovery contract a durable ledger needs.
    pub fn expire_in_flight_before(
        &self,
        cutoff: DateTime<Utc>,
    ) -> Result<usize, ProductWorkflowError> {
        let mut state = self.lock_state()?;
        let before = state.in_flight.len();
        state
            .in_flight
            .retain(|_, action| action.received_at >= cutoff);
        Ok(before - state.in_flight.len())
    }

    fn lock_state(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, InMemoryIdempotencyState>, ProductWorkflowError> {
        self.state
            .lock()
            .map_err(|_| ProductWorkflowError::Transient {
                reason: "idempotency ledger state lock poisoned".into(),
            })
    }
}

impl Default for InMemoryIdempotencyLedger {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl IdempotencyLedger for InMemoryIdempotencyLedger {
    async fn begin_or_replay(
        &self,
        fingerprint: ActionFingerprintKey,
        received_at: DateTime<Utc>,
    ) -> Result<IdempotencyDecision, ProductWorkflowError> {
        let mut state = self.lock_state()?;
        if let Some(prior) = state.settled.get(&fingerprint) {
            return Ok(IdempotencyDecision::Replay(prior.clone()));
        }
        if let Some(in_flight) = state.in_flight.get(&fingerprint) {
            let expires_at = in_flight.received_at + self.in_flight_lease;
            if expires_at > received_at {
                return Err(ProductWorkflowError::Transient {
                    reason: "idempotency fingerprint already in flight; retry after recovery lease"
                        .into(),
                });
            }
            state.in_flight.remove(&fingerprint);
        }

        let action = ProductInboundAction::begin(fingerprint.clone(), received_at);
        state.in_flight.insert(fingerprint, action.clone());
        Ok(IdempotencyDecision::New(action))
    }

    async fn settle(&self, action: ProductInboundAction) -> Result<(), ProductWorkflowError> {
        let mut state = self.lock_state()?;
        if let Some(prior) = state.settled.get(&action.fingerprint) {
            if prior.action_id == action.action_id {
                return Ok(());
            }
            return Err(ProductWorkflowError::Transient {
                reason: "idempotency reservation was superseded before terminal settle".into(),
            });
        }
        let Some(current) = state.in_flight.get(&action.fingerprint) else {
            return Err(ProductWorkflowError::Transient {
                reason: "idempotency reservation missing before terminal settle".into(),
            });
        };
        if current.action_id != action.action_id {
            return Err(ProductWorkflowError::Transient {
                reason: "idempotency reservation was superseded before terminal settle".into(),
            });
        }
        state.in_flight.remove(&action.fingerprint);
        state.settled.insert(action.fingerprint.clone(), action);
        Ok(())
    }

    async fn release(&self, action: ProductInboundAction) -> Result<(), ProductWorkflowError> {
        let mut state = self.lock_state()?;
        if matches!(
            state.in_flight.get(&action.fingerprint),
            Some(current) if current.action_id == action.action_id
        ) {
            state.in_flight.remove(&action.fingerprint);
        }
        Ok(())
    }
}
