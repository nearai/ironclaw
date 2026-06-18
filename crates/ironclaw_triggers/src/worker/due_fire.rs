use crate::{
    ClaimDueFireOutcome, ClaimDueFireRequest, FireAcceptedRequest, FirePermanentFailedRequest,
    FireReplayedRequest, FireRetryableFailedRequest, FireTerminalFailedRequest,
    TriggerCompletionPolicy, TriggerError, TriggerRecord,
};
use ironclaw_host_api::Timestamp;

use super::{
    TriggerPollerFailureReason, TriggerPollerFireOutcome, TriggerPollerWorker,
    TrustedTriggerFireSubmitOutcome, TrustedTriggerSubmitRequest,
    failure::{
        FireFailureDisposition, SubmitFailureKind, classify_failure, next_run_at_after_fire,
    },
};

impl TriggerPollerWorker {
    pub(super) async fn process_due_record(
        &self,
        record: TriggerRecord,
        now: Timestamp,
    ) -> Result<TriggerPollerFireOutcome, TriggerError> {
        let tenant_id = record.tenant_id.clone();
        let trigger_id = record.trigger_id;
        let fire_slot = record.next_run_at;
        let claimed = self
            .deps
            .repository
            .claim_due_fire(ClaimDueFireRequest {
                tenant_id: tenant_id.clone(),
                trigger_id,
                fire_slot,
                now,
            })
            .await?;
        let outcome = match claimed {
            ClaimDueFireOutcome::Claimed(claimed) => {
                self.process_claimed_fire(claimed.record, claimed.fire_slot, now)
                    .await?
            }
            ClaimDueFireOutcome::AlreadyActive {
                active_fire_slot,
                active_run_ref,
            } => {
                let Some(active_fire_slot) = active_fire_slot else {
                    return Err(TriggerError::Backend {
                        reason: "AlreadyActive claim outcome did not include active_fire_slot"
                            .to_string(),
                    });
                };
                TriggerPollerFireOutcome::SkippedAlreadyActive {
                    active_fire_slot,
                    active_run_ref,
                }
            }
            ClaimDueFireOutcome::NotDue { .. } => TriggerPollerFireOutcome::SkippedNotDue,
            ClaimDueFireOutcome::NotFound => TriggerPollerFireOutcome::SkippedNotFound,
        };
        Ok(outcome)
    }

    async fn process_claimed_fire(
        &self,
        record: TriggerRecord,
        fire_slot: Timestamp,
        now: Timestamp,
    ) -> Result<TriggerPollerFireOutcome, TriggerError> {
        let is_fire_once =
            record.completion_policy == TriggerCompletionPolicy::CompleteAfterFirstFire;

        // For recurring triggers, compute next slot up front (terminal-fail if unavailable).
        // For fire-once triggers, next_run_at is None (no next slot by design).
        let recurring_next_run_at: Option<Timestamp> = if is_fire_once {
            None
        } else {
            match next_run_at_after_fire(&record, fire_slot) {
                Ok(next) => Some(next),
                Err(error) => {
                    let classification = classify_failure(&error);
                    return self
                        .persist_failed_fire(
                            record,
                            fire_slot,
                            FireFailureDisposition::PermanentTerminal,
                            classification.reason,
                        )
                        .await;
                }
            }
        };

        // Build failure disposition respecting fire-once semantics.
        // Fire-once permanent pre-submission failures (unpaired actor, source/materialize/submit
        // error) must NOT mark the trigger Completed — the one-shot never actually fired.
        // Leave it Scheduled (Retryable) so it can retry once the cause is fixed. A fire-once
        // only completes after a real run terminates via clear_active_fire. Recurring behavior
        // is unchanged: Permanent → PermanentReschedule(next_run_at).
        let failure_disposition = |kind: SubmitFailureKind| -> FireFailureDisposition {
            match kind {
                SubmitFailureKind::Retryable => FireFailureDisposition::Retryable,
                SubmitFailureKind::Permanent => {
                    if is_fire_once {
                        // Fail closed: keep Scheduled so the trigger retries when the
                        // underlying cause (e.g. unpaired actor) is resolved.
                        FireFailureDisposition::Retryable
                    } else {
                        // recurring_next_run_at is Some when is_fire_once is false
                        // (we computed it above and would have returned early on error)
                        FireFailureDisposition::PermanentReschedule(
                            recurring_next_run_at.unwrap_or(fire_slot),
                        )
                    }
                }
            }
        };

        let fire = match self.deps.source_provider.evaluate(&record, now).await {
            Ok(Some(fire)) => fire,
            Ok(None) => {
                return self
                    .persist_failed_fire(
                        record,
                        fire_slot,
                        failure_disposition(SubmitFailureKind::Permanent),
                        TriggerPollerFailureReason::SourceNoFire,
                    )
                    .await;
            }
            Err(error) => {
                let classification = classify_failure(&error);
                return self
                    .persist_failed_fire(
                        record,
                        fire_slot,
                        failure_disposition(classification.kind),
                        classification.reason,
                    )
                    .await;
            }
        };
        let materialized_prompt = match self
            .deps
            .materializer
            .materialize_prompt(fire.clone())
            .await
        {
            Ok(content_ref) => content_ref,
            Err(error) => {
                let classification = classify_failure(&error);
                return self
                    .persist_failed_fire(
                        record,
                        fire_slot,
                        failure_disposition(classification.kind),
                        classification.reason,
                    )
                    .await;
            }
        };
        match self
            .deps
            .trusted_submitter
            .submit_trusted_trigger_fire(TrustedTriggerSubmitRequest::new(
                fire,
                materialized_prompt,
                now,
            ))
            .await
        {
            Ok(TrustedTriggerFireSubmitOutcome::Accepted {
                run_id,
                submitted_at,
                turn_scope,
            }) => {
                let updated = self
                    .deps
                    .repository
                    .mark_fire_accepted(FireAcceptedRequest {
                        tenant_id: record.tenant_id,
                        trigger_id: record.trigger_id,
                        fire_slot,
                        run_id,
                        thread_id: turn_scope.thread_id,
                        submitted_at,
                        next_run_at: recurring_next_run_at,
                    })
                    .await?;
                if updated.is_none() {
                    return Err(TriggerError::Backend {
                        reason: "claimed trigger fire was not present when persisting accepted submit result"
                            .to_string(),
                    });
                }
                Ok(TriggerPollerFireOutcome::Submitted { run_id })
            }
            Ok(TrustedTriggerFireSubmitOutcome::Replayed {
                original_run_id,
                replayed_at,
                thread_id,
            }) => {
                let updated = self
                    .deps
                    .repository
                    .mark_fire_replayed(FireReplayedRequest {
                        tenant_id: record.tenant_id,
                        trigger_id: record.trigger_id,
                        fire_slot,
                        original_run_id,
                        thread_id,
                        replayed_at,
                        next_run_at: recurring_next_run_at,
                    })
                    .await?;
                if updated.is_none() {
                    return Err(TriggerError::Backend {
                        reason: "claimed trigger fire was not present when persisting replayed submit result"
                            .to_string(),
                    });
                }
                Ok(TriggerPollerFireOutcome::Replayed { original_run_id })
            }
            Err(error) => {
                let classification = classify_failure(&error);
                self.persist_failed_fire(
                    record,
                    fire_slot,
                    failure_disposition(classification.kind),
                    classification.reason,
                )
                .await
            }
        }
    }

    async fn persist_failed_fire(
        &self,
        record: TriggerRecord,
        fire_slot: Timestamp,
        disposition: FireFailureDisposition,
        reason: TriggerPollerFailureReason,
    ) -> Result<TriggerPollerFireOutcome, TriggerError> {
        match disposition {
            FireFailureDisposition::Retryable => {
                self.deps
                    .repository
                    .mark_fire_retryable_failed(FireRetryableFailedRequest {
                        tenant_id: record.tenant_id,
                        trigger_id: record.trigger_id,
                        fire_slot,
                    })
                    .await?;
                Ok(TriggerPollerFireOutcome::RetryableFailed { reason })
            }
            FireFailureDisposition::PermanentTerminal => {
                self.deps
                    .repository
                    .mark_fire_terminally_failed(FireTerminalFailedRequest {
                        tenant_id: record.tenant_id,
                        trigger_id: record.trigger_id,
                        fire_slot,
                    })
                    .await?;
                Ok(TriggerPollerFireOutcome::PermanentFailed { reason })
            }
            FireFailureDisposition::PermanentReschedule(next_run_at) => {
                self.deps
                    .repository
                    .mark_fire_permanently_failed(FirePermanentFailedRequest {
                        tenant_id: record.tenant_id,
                        trigger_id: record.trigger_id,
                        fire_slot,
                        next_run_at,
                    })
                    .await?;
                Ok(TriggerPollerFireOutcome::PermanentFailed { reason })
            }
        }
    }
}
