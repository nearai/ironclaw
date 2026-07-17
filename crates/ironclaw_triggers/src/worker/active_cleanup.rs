use crate::{
    ActiveTriggerScanCursor, ClearActiveFireRequest, TriggerError, TriggerRecord,
    TriggerRunHistoryStatus,
};

use super::{
    TriggerActiveRunState, TriggerActiveRunStateRequest, TriggerPollerFailureReason,
    TriggerPollerFireOutcome, TriggerPollerFireReport, TriggerPollerTickReport,
    TriggerPollerWorker, failure::classify_failure,
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
                let outcome = match self
                    .recover_stale_claim_only_fire(record.clone(), fire_slot, report.now)
                    .await
                {
                    Ok(Some(outcome)) => outcome,
                    Ok(None) => TriggerPollerFireOutcome::SkippedAlreadyActive {
                        active_fire_slot: fire_slot,
                        active_run_ref: None,
                    },
                    Err(error) => {
                        let classification = classify_failure(&error);
                        TriggerPollerFireOutcome::DueFireFailed {
                            reason: classification.reason,
                        }
                    }
                };
                report.results.push(TriggerPollerFireReport {
                    tenant_id: record.tenant_id,
                    trigger_id: record.trigger_id,
                    fire_slot,
                    outcome,
                });
                // Advance the scan cursor even when recovery is deferred or
                // fails, so one claim-only row cannot starve later active runs.
                if first_unadvanced_cursor.is_none() {
                    next_cursor = Some(record_cursor);
                }
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
            // Map the run state to "should this active fire be cleared, and if
            // so with what status". Human-interaction gates keep active
            // back-pressure here: clearing a blocked active fire without
            // atomically terminating the underlying turn can let the same run
            // resume after a failed trigger-history entry has already been
            // recorded. Terminal runs clear with whatever status they reached.
            // Exhaustive (no wildcard) so a new variant forces a compile error.
            let clear: Option<(TriggerPollerFireOutcome, TriggerRunHistoryStatus)> = match state {
                TriggerActiveRunState::Terminal { status } => Some((
                    TriggerPollerFireOutcome::ClearedTerminalActive { run_id },
                    status,
                )),
                // Missing remains conservative until recovery can prove the
                // active run lookup is not merely stale or temporarily empty.
                TriggerActiveRunState::Blocked { .. }
                | TriggerActiveRunState::Missing
                | TriggerActiveRunState::Nonterminal => None,
            };
            match clear {
                Some((cleared_outcome, status)) => {
                    let outcome = if self
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
                        cleared_outcome
                    } else {
                        TriggerPollerFireOutcome::SkippedAlreadyCleared { run_id }
                    };
                    report.results.push(TriggerPollerFireReport {
                        tenant_id: record.tenant_id,
                        trigger_id: record.trigger_id,
                        fire_slot,
                        outcome,
                    });
                }
                None => {
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

    async fn recover_stale_claim_only_fire(
        &self,
        record: TriggerRecord,
        fire_slot: chrono::DateTime<chrono::Utc>,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<Option<TriggerPollerFireOutcome>, TriggerError> {
        let runs = self
            .deps
            .repository
            .list_trigger_run_history(record.tenant_id.clone(), record.trigger_id, 1)
            .await?;
        let Some(run) = runs.into_iter().find(|run| run.fire_slot == fire_slot) else {
            return Ok(None);
        };
        let Ok(age) = now.signed_duration_since(run.submitted_at).to_std() else {
            return Ok(None);
        };
        if age < self.config.claim_only_recovery_grace {
            return Ok(None);
        }

        self.process_claimed_fire(record, fire_slot, now)
            .await
            .map(Some)
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
