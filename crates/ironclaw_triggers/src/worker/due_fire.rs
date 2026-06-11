use crate::{
    ClaimDueFireOutcome, ClaimDueFireRequest, FireAcceptedRequest, FirePermanentFailedRequest,
    FireReplayedRequest, FireRetryableFailedRequest, FireTerminalFailedRequest, TriggerError,
    TriggerRecord,
};
use ironclaw_host_api::Timestamp;

use super::{
    TriggerPollerFailureReason, TriggerPollerFireOutcome, TriggerPollerWorker,
    TrustedTriggerFireSubmitOutcome, TrustedTriggerSubmitRequest,
    failure::{FireFailureDisposition, classify_failure, next_run_at_after_fire},
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
        let next_run_at = match next_run_at_after_fire(&record, fire_slot) {
            Ok(next_run_at) => next_run_at,
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
        };
        let fire = match self.deps.source_provider.evaluate(&record, now).await {
            Ok(Some(fire)) => fire,
            Ok(None) => {
                return self
                    .persist_failed_fire(
                        record,
                        fire_slot,
                        FireFailureDisposition::PermanentReschedule(next_run_at),
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
                        FireFailureDisposition::from_kind(classification.kind, next_run_at),
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
                        FireFailureDisposition::from_kind(classification.kind, next_run_at),
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
                        next_run_at,
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
                        next_run_at,
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
                    FireFailureDisposition::from_kind(classification.kind, next_run_at),
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
