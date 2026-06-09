use async_trait::async_trait;
use ironclaw_host_api::{TenantId, Timestamp};
use ironclaw_turns::TurnRunId;

use crate::{
    TriggerError, TriggerFire, TriggerId, TriggerMaterializedPrompt, TriggerRunHistoryStatus,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustedTriggerSubmitRequest {
    fire: TriggerFire,
    materialized_prompt: TriggerMaterializedPrompt,
    received_at: Timestamp,
}

impl TrustedTriggerSubmitRequest {
    /// Create a sealed trusted trigger submit request.
    ///
    /// `materialized_prompt` must have been produced from the exact `fire`
    /// supplied here. The worker is the only crate allowed to pair those values,
    /// so downstream trusted submitters cannot forge or mix prompt content and
    /// trigger identity.
    pub(crate) fn new(
        fire: TriggerFire,
        materialized_prompt: TriggerMaterializedPrompt,
        received_at: Timestamp,
    ) -> Self {
        Self {
            fire,
            materialized_prompt,
            received_at,
        }
    }

    pub fn fire(&self) -> &TriggerFire {
        &self.fire
    }

    pub fn materialized_prompt(&self) -> &TriggerMaterializedPrompt {
        &self.materialized_prompt
    }

    pub fn content_ref(&self) -> &crate::TriggerInboundContentRef {
        self.materialized_prompt.content_ref()
    }

    pub fn received_at(&self) -> Timestamp {
        self.received_at
    }

    pub fn into_parts(self) -> (TriggerFire, TriggerMaterializedPrompt, Timestamp) {
        (self.fire, self.materialized_prompt, self.received_at)
    }
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
    Terminal { status: TriggerRunHistoryStatus },
}

#[async_trait]
pub trait TriggerActiveRunLookup: Send + Sync {
    /// Resolve a single active-run state.
    ///
    /// The default composition-root implementation reads a full
    /// `TurnPersistenceSnapshot` for each call, so batch-oriented
    /// implementations should prefer overriding `active_run_states` and
    /// handling single-record lookups through the shared batch path when
    /// they need to amortize snapshot reads.
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
