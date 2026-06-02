use async_trait::async_trait;
use ironclaw_host_api::{TenantId, Timestamp};
use ironclaw_turns::TurnRunId;

use crate::{TriggerError, TriggerFire, TriggerId, TriggerInboundContentRef};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustedTriggerSubmitRequest {
    pub fire: TriggerFire,
    pub content_ref: TriggerInboundContentRef,
    pub received_at: Timestamp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustedTriggerSubmitFailureReason {
    Retryable,
    Permanent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrustedTriggerFireSubmitOutcome {
    Accepted {
        run_id: TurnRunId,
        submitted_at: Timestamp,
    },
    Replayed {
        original_run_id: TurnRunId,
        replayed_at: Timestamp,
    },
    RetryableFailed {
        reason: TrustedTriggerSubmitFailureReason,
    },
    PermanentFailed {
        reason: TrustedTriggerSubmitFailureReason,
    },
}

#[async_trait]
pub trait TrustedTriggerFireSubmitter: Send + Sync {
    async fn submit_trusted_trigger_fire(
        &self,
        request: TrustedTriggerSubmitRequest,
    ) -> Result<TrustedTriggerFireSubmitOutcome, TriggerError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TriggerActiveRunStateRequest {
    pub tenant_id: TenantId,
    pub trigger_id: TriggerId,
    pub fire_slot: Timestamp,
    pub run_id: TurnRunId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriggerActiveRunState {
    Missing,
    Nonterminal,
    Terminal,
}

#[async_trait]
pub trait TriggerActiveRunLookup: Send + Sync {
    async fn active_run_state(
        &self,
        request: TriggerActiveRunStateRequest,
    ) -> Result<TriggerActiveRunState, TriggerError>;

    /// Resolve active run states for a batch of requests.
    ///
    /// Implementations must return exactly one result per request, in the same
    /// order as the input vector. Callers use positional matching to preserve
    /// per-trigger cleanup report semantics across batched backend reads.
    async fn active_run_states(
        &self,
        requests: Vec<TriggerActiveRunStateRequest>,
    ) -> Vec<Result<TriggerActiveRunState, TriggerError>> {
        let mut results = Vec::with_capacity(requests.len());
        for request in requests {
            results.push(self.active_run_state(request).await);
        }
        results
    }
}
