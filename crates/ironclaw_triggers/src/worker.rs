use std::sync::Mutex;

use ironclaw_host_api::Timestamp;

use crate::{ActiveTriggerScanCursor, TriggerError};

mod active_cleanup;
mod config;
mod due_fire;
mod failure;
mod ports;
mod report;

pub use config::{TriggerPollerWorkerConfig, TriggerPollerWorkerDeps};
pub use ports::{
    TriggerActiveRunLookup, TriggerActiveRunState, TriggerActiveRunStateRequest,
    TrustedTriggerFireSubmitOutcome, TrustedTriggerFireSubmitter, TrustedTriggerSubmitRequest,
};
pub use report::{
    TriggerPollerFailureReason, TriggerPollerFireOutcome, TriggerPollerFireReport,
    TriggerPollerTickReport,
};

use failure::classify_failure;

pub struct TriggerPollerWorker {
    config: TriggerPollerWorkerConfig,
    deps: TriggerPollerWorkerDeps,
    tick_guard: tokio::sync::Mutex<()>,
    // active_scan_cursor's sync mutex is held only around cursor clone/set
    // operations, never across repository, materialization, submit, or lookup awaits.
    active_scan_cursor: Mutex<Option<ActiveTriggerScanCursor>>,
}

impl TriggerPollerWorker {
    pub fn new(
        config: TriggerPollerWorkerConfig,
        deps: TriggerPollerWorkerDeps,
    ) -> Result<Self, TriggerError> {
        config.validate()?;
        Ok(Self {
            config,
            deps,
            tick_guard: tokio::sync::Mutex::new(()),
            active_scan_cursor: Mutex::new(None),
        })
    }

    /// Executes one serialized poller tick.
    ///
    /// Production lifecycle wiring must not run overlapping ticks for the same
    /// worker instance. The active-scan cursor is a per-worker progress marker
    /// and is advanced by the single supervisor-owned tick loop.
    pub async fn tick_once(&self, now: Timestamp) -> Result<TriggerPollerTickReport, TriggerError> {
        let _tick_guard = self.tick_guard.lock().await;
        let mut report = TriggerPollerTickReport::new(now);
        self.clear_terminal_active_fires(&mut report).await?;
        // trusted-poller: this is host-owned background work, not a tenant/API list.
        let due_records = self
            .deps
            .repository
            .list_due_triggers(now, self.config.fires_per_tick)
            .await?;
        report.due_records = due_records.len();
        for record in due_records {
            let tenant_id = record.tenant_id.clone();
            let trigger_id = record.trigger_id;
            let fire_slot = record.next_run_at;
            let outcome = match self.process_due_record(record, now).await {
                Ok(outcome) => outcome,
                Err(error) => {
                    let classification = classify_failure(&error);
                    report.results.push(TriggerPollerFireReport {
                        tenant_id,
                        trigger_id,
                        fire_slot,
                        outcome: TriggerPollerFireOutcome::DueFireFailed {
                            reason: classification.reason,
                        },
                    });
                    continue;
                }
            };
            report.results.push(TriggerPollerFireReport {
                tenant_id,
                trigger_id,
                fire_slot,
                outcome,
            });
        }
        Ok(report)
    }
}

#[cfg(test)]
mod tests;
