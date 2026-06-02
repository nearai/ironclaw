use std::{sync::Arc, time::Duration};

use crate::{TriggerError, TriggerPromptMaterializer, TriggerRepository, TriggerSourceProvider};

use super::{TriggerActiveRunLookup, TrustedTriggerFireSubmitter};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TriggerPollerWorkerConfig {
    pub poll_interval: Duration,
    pub fires_per_tick: usize,
    pub max_concurrent_fires_per_trigger: usize,
}

impl Default for TriggerPollerWorkerConfig {
    fn default() -> Self {
        Self {
            poll_interval: Duration::from_secs(30),
            fires_per_tick: 32,
            max_concurrent_fires_per_trigger: 1,
        }
    }
}

impl TriggerPollerWorkerConfig {
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
}
