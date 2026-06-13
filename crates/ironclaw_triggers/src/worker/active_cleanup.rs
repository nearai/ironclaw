use crate::{ActiveTriggerScanCursor, ClearActiveFireRequest, TriggerError, TriggerRecord};

use super::{
    TriggerActiveRunState, TriggerActiveRunStateRequest, TriggerPollerFailureReason,
    TriggerPollerFireOutcome, TriggerPollerFireReport, TriggerPollerTickReport,
    TriggerPollerWorker,
};

struct ActiveLookupItem {
    record: TriggerRecord,
    fire_slot: chrono::DateTime<chrono::Utc>,
    record_cursor: ActiveTriggerScanCursor,
    run_id: ironclaw_turns::TurnRunId,
    result_index: usize,
}

impl TriggerPollerWorker {
    pub(super) async fn clear_terminal_active_fires(
        &self,
        report: &mut TriggerPollerTickReport,
    ) -> Result<(), TriggerError> {
        let (cursor, active_records) = self.list_active_cleanup_page().await?;
        report.active_records = active_records.len();
        let mut next_cursor = cursor;
        let mut first_unadvanced_cursor: Option<ActiveTriggerScanCursor> = None;
        let mut lookup_items = Vec::new();
        let mut lookup_requests = Vec::new();
        for record in active_records {
            debug_assert!(
                record.active_fire_slot.is_some(),
                "list_active_triggers returned a record without active_fire_slot"
            );
            let Some(fire_slot) = record.active_fire_slot else {
                continue;
            };
            let Some(record_cursor) = ActiveTriggerScanCursor::from_active_record(&record) else {
                continue;
            };
            let Some(run_id) = record.active_run_ref else {
                // Keep claim-only rows blocked until recovery has lease or age
                // evidence that clearing cannot double-submit after a crash.
                // Still advance the scan cursor: otherwise one claim-only row
                // at the front of a page can starve every later active run.
                if first_unadvanced_cursor.is_none() {
                    next_cursor = Some(record_cursor);
                }
                report.results.push(TriggerPollerFireReport {
                    tenant_id: record.tenant_id,
                    trigger_id: record.trigger_id,
                    fire_slot,
                    outcome: TriggerPollerFireOutcome::SkippedAlreadyActive {
                        active_fire_slot: fire_slot,
                        active_run_ref: None,
                    },
                });
                continue;
            };
            let result_index = lookup_requests.len();
            lookup_requests.push(TriggerActiveRunStateRequest {
                tenant_id: record.tenant_id.clone(),
                trigger_id: record.trigger_id,
                fire_slot,
                run_id,
            });
            lookup_items.push(ActiveLookupItem {
                record,
                fire_slot,
                record_cursor,
                run_id,
                result_index,
            });
        }

        let mut lookup_results = self
            .deps
            .active_run_lookup
            .active_run_states(lookup_requests)
            .await
            .into_iter()
            .map(Some)
            .collect::<Vec<_>>();

        for item in lookup_items {
            let ActiveLookupItem {
                record,
                fire_slot,
                record_cursor,
                run_id,
                result_index,
            } = item;
            let state = match lookup_results
                .get_mut(result_index)
                .and_then(Option::take)
                .unwrap_or_else(|| {
                    Err(TriggerError::Backend {
                        reason: "active run lookup returned too few results".to_string(),
                    })
                }) {
                Ok(state) => state,
                Err(_error) => {
                    report.results.push(TriggerPollerFireReport {
                        tenant_id: record.tenant_id,
                        trigger_id: record.trigger_id,
                        fire_slot,
                        outcome: TriggerPollerFireOutcome::ActiveRunLookupFailed {
                            run_id,
                            reason: TriggerPollerFailureReason::ActiveRunLookup,
                        },
                    });
                    first_unadvanced_cursor.get_or_insert(record_cursor);
                    continue;
                }
            };
            match state {
                TriggerActiveRunState::Terminal { status } => {
                    if self
                        .deps
                        .repository
                        .clear_active_fire(ClearActiveFireRequest {
                            tenant_id: record.tenant_id.clone(),
                            trigger_id: record.trigger_id,
                            fire_slot,
                            run_id,
                            status,
                        })
                        .await?
                        .is_some()
                    {
                        report.results.push(TriggerPollerFireReport {
                            tenant_id: record.tenant_id,
                            trigger_id: record.trigger_id,
                            fire_slot,
                            outcome: TriggerPollerFireOutcome::ClearedTerminalActive { run_id },
                        });
                    } else {
                        report.results.push(TriggerPollerFireReport {
                            tenant_id: record.tenant_id,
                            trigger_id: record.trigger_id,
                            fire_slot,
                            outcome: TriggerPollerFireOutcome::SkippedAlreadyCleared { run_id },
                        });
                    }
                }
                TriggerActiveRunState::Missing | TriggerActiveRunState::Nonterminal => {
                    // Missing remains conservative until recovery can prove the
                    // active run lookup is not merely stale or temporarily empty.
                    report.results.push(TriggerPollerFireReport {
                        tenant_id: record.tenant_id,
                        trigger_id: record.trigger_id,
                        fire_slot,
                        outcome: TriggerPollerFireOutcome::SkippedAlreadyActive {
                            active_fire_slot: fire_slot,
                            active_run_ref: record.active_run_ref,
                        },
                    });
                }
            }
            if first_unadvanced_cursor.is_none() {
                next_cursor = Some(record_cursor);
            }
        }
        self.set_active_scan_cursor(next_cursor)?;
        Ok(())
    }

    async fn list_active_cleanup_page(
        &self,
    ) -> Result<(Option<ActiveTriggerScanCursor>, Vec<TriggerRecord>), TriggerError> {
        let mut cursor = self.active_scan_cursor()?;
        // trusted-poller: active scan cursors are derived from previous global
        // active scan results, not user input or tenant-scoped list paths.
        let mut active_records = self
            .deps
            .repository
            .list_active_triggers_after(cursor.clone(), self.config.fires_per_tick)
            .await?;
        if active_records.is_empty() && cursor.is_some() {
            cursor = None;
            active_records = self
                .deps
                .repository
                .list_active_triggers_after(cursor.clone(), self.config.fires_per_tick)
                .await?;
        }
        Ok((cursor, active_records))
    }

    fn active_scan_cursor(&self) -> Result<Option<ActiveTriggerScanCursor>, TriggerError> {
        self.active_scan_cursor
            .lock()
            .map(|cursor| cursor.clone())
            .map_err(|_| TriggerError::Backend {
                reason: "trigger poller active scan cursor mutex poisoned".to_string(),
            })
    }

    fn set_active_scan_cursor(
        &self,
        cursor: Option<ActiveTriggerScanCursor>,
    ) -> Result<(), TriggerError> {
        *self
            .active_scan_cursor
            .lock()
            .map_err(|_| TriggerError::Backend {
                reason: "trigger poller active scan cursor mutex poisoned".to_string(),
            })? = cursor;
        Ok(())
    }
}
