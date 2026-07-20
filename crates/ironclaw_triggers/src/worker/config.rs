use std::{sync::Arc, time::Duration};

use crate::{TriggerError, TriggerPromptMaterializer, TriggerRepository, TriggerSourceProvider};

use super::{TriggerActiveRunLookup, TriggerFireSettlementObserver, TrustedTriggerFireSubmitter};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TriggerPollerWorkerConfig {
    pub poll_interval: Duration,
    pub fires_per_tick: usize,
    pub max_concurrent_fires_per_trigger: usize,
    pub claim_only_recovery_grace: Duration,
}

impl Default for TriggerPollerWorkerConfig {
    fn default() -> Self {
        Self {
            poll_interval: Duration::from_secs(30),
            fires_per_tick: 32,
            max_concurrent_fires_per_trigger: 1,
            claim_only_recovery_grace: Duration::from_secs(60),
        }
    }
}

impl TriggerPollerWorkerConfig {
    pub fn set_poll_interval(mut self, poll_interval: Duration) -> Self {
        self.poll_interval = poll_interval;
        self
    }

    pub fn set_fires_per_tick(mut self, fires_per_tick: usize) -> Self {
        self.fires_per_tick = fires_per_tick;
        self
    }

    pub fn set_max_concurrent_fires_per_trigger(
        mut self,
        max_concurrent_fires_per_trigger: usize,
    ) -> Self {
        self.max_concurrent_fires_per_trigger = max_concurrent_fires_per_trigger;
        self
    }

    pub fn set_claim_only_recovery_grace(mut self, claim_only_recovery_grace: Duration) -> Self {
        self.claim_only_recovery_grace = claim_only_recovery_grace;
        self
    }

    pub fn validate(&self) -> Result<(), TriggerError> {
        if self.poll_interval.is_zero() {
            return Err(TriggerError::InvalidPollerConfig {
                reason: "poll_interval must be non-zero".to_string(),
            });
        }
        if self.fires_per_tick == 0 {
            return Err(TriggerError::InvalidPollerConfig {
                reason: "fires_per_tick must be non-zero".to_string(),
            });
        }
        if self.max_concurrent_fires_per_trigger != 1 {
            return Err(TriggerError::InvalidPollerConfig {
                reason: "V1 supports exactly one concurrent fire per trigger".to_string(),
            });
        }
        if self.claim_only_recovery_grace.is_zero() {
            return Err(TriggerError::InvalidPollerConfig {
                reason: "claim_only_recovery_grace must be non-zero".to_string(),
            });
        }
        Ok(())
    }
}

#[derive(Clone)]
pub struct TriggerPollerWorkerDeps {
    pub repository: Arc<dyn TriggerRepository>,
    pub source_provider: Arc<dyn TriggerSourceProvider>,
    pub materializer: Arc<dyn TriggerPromptMaterializer>,
    pub trusted_submitter: Arc<dyn TrustedTriggerFireSubmitter>,
    pub active_run_lookup: Arc<dyn TriggerActiveRunLookup>,
    pub fire_settlement_observer: Arc<dyn TriggerFireSettlementObserver>,
}
