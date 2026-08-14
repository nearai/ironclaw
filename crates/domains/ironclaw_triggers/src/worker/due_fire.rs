use crate::{
    ClaimDueFireOutcome, ClaimDueFireRequest, FireAcceptedRequest, FirePermanentFailedRequest,
    FireReplayedRequest, FireRetryableFailedRequest, FireTerminalFailedRequest, TriggerError,
    TriggerRecord, TriggerSchedule,
};
use ironclaw_host_api::Timestamp;

use super::{
    TriggerAcceptedFireSettlement, TriggerFailedFireSettlement, TriggerPollerFailureReason,
    TriggerPollerFireOutcome, TriggerPollerWorker, TrustedTriggerFireSubmitOutcome,
    TrustedTriggerSubmitRequest,
    failure::{SubmitFailureKind, classify_failure, classify_submit_failure},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FailedFireDisposition {
    Retryable,
    RecurringPermanentReschedule(Timestamp),
    OncePermanentComplete,
}

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

    pub(super) async fn process_claimed_fire(
        &self,
        record: TriggerRecord,
        fire_slot: Timestamp,
        now: Timestamp,
    ) -> Result<TriggerPollerFireOutcome, TriggerError> {
        let fire = match self.deps.source_provider.evaluate(&record, now).await {
            Ok(Some(fire)) => fire,
            Ok(None) => {
                let disposition = permanent_failure_disposition(&record.schedule, fire_slot)?;
                return self
                    .persist_failed_fire(
                        record,
                        fire_slot,
                        disposition,
                        TriggerPollerFailureReason::SourceNoFire,
                    )
                    .await;
            }
            Err(error) => {
                let classification = classify_failure(&error);
                let disposition = match classification.kind {
                    SubmitFailureKind::Retryable => FailedFireDisposition::Retryable,
                    SubmitFailureKind::Permanent => {
                        permanent_failure_disposition(&record.schedule, fire_slot)?
                    }
                };
                return self
                    .persist_failed_fire(record, fire_slot, disposition, classification.reason)
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
                let disposition = match classification.kind {
                    SubmitFailureKind::Retryable => FailedFireDisposition::Retryable,
                    SubmitFailureKind::Permanent => {
                        permanent_failure_disposition(&record.schedule, fire_slot)?
                    }
                };
                return self
                    .persist_failed_fire(record, fire_slot, disposition, classification.reason)
                    .await;
            }
        };
        let submitted_fire = fire.clone();
        // Minting runs the trusted-prompt safety scan (PROPOSAL §6.4.2): a fire
        // whose durable prompt fails it never becomes a submit request, so it
        // can never reach any `TrustedTriggerFireSubmitter`.
        let submit_request = match TrustedTriggerSubmitRequest::new(fire, materialized_prompt, now)
        {
            Ok(request) => request,
            Err(error) => {
                let classification = classify_submit_failure(&error);
                let disposition = match classification.kind {
                    SubmitFailureKind::Retryable => FailedFireDisposition::Retryable,
                    SubmitFailureKind::Permanent => {
                        permanent_failure_disposition(&record.schedule, fire_slot)?
                    }
                };
                return self
                    .persist_failed_fire(record, fire_slot, disposition, classification.reason)
                    .await;
            }
        };
        match self
            .deps
            .trusted_submitter
            .submit_trusted_trigger_fire(submit_request)
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
                        thread_id: turn_scope.thread_id.clone(),
                        submitted_at,
                    })
                    .await?;
                if updated.is_none() {
                    return Err(TriggerError::Backend {
                        reason: "claimed trigger fire was not present when persisting accepted submit result"
                            .to_string(),
                    });
                }
                self.deps
                    .fire_settlement_observer
                    .on_accepted_fire_settled(TriggerAcceptedFireSettlement {
                        fire: submitted_fire,
                        run_id,
                        turn_scope,
                    })
                    .await;
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
                let classification = classify_submit_failure(&error);
                let disposition = match classification.kind {
                    SubmitFailureKind::Retryable => FailedFireDisposition::Retryable,
                    SubmitFailureKind::Permanent => {
                        permanent_failure_disposition(&record.schedule, fire_slot)?
                    }
                };
                self.persist_failed_fire(record, fire_slot, disposition, classification.reason)
                    .await
            }
        }
    }

    async fn persist_failed_fire(
        &self,
        record: TriggerRecord,
        fire_slot: Timestamp,
        disposition: FailedFireDisposition,
        reason: TriggerPollerFailureReason,
    ) -> Result<TriggerPollerFireOutcome, TriggerError> {
        let failed_fire = crate::TriggerFire {
            identity: crate::TriggerFireIdentity::new(
                record.tenant_id.clone(),
                record.trigger_id,
                fire_slot,
            ),
            creator_user_id: record.creator_user_id.clone(),
            agent_id: record.agent_id.clone(),
            project_id: record.project_id.clone(),
            prompt: record.prompt.clone(),
            execution_policy: record
                .execution_spec
                .as_ref()
                .map(|spec| spec.policy.clone()),
        };
        match disposition {
            FailedFireDisposition::Retryable => {
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
            FailedFireDisposition::RecurringPermanentReschedule(next_run_at) => {
                let updated = self
                    .deps
                    .repository
                    .mark_fire_permanently_failed(FirePermanentFailedRequest {
                        tenant_id: record.tenant_id,
                        trigger_id: record.trigger_id,
                        fire_slot,
                        next_run_at,
                    })
                    .await?;
                if updated.is_some() {
                    self.deps
                        .fire_settlement_observer
                        .on_failed_fire_settled(TriggerFailedFireSettlement {
                            fire: failed_fire,
                            reason,
                        })
                        .await;
                }
                Ok(TriggerPollerFireOutcome::PermanentFailed { reason })
            }
            FailedFireDisposition::OncePermanentComplete => {
                let updated = self
                    .deps
                    .repository
                    .mark_fire_terminally_failed(FireTerminalFailedRequest {
                        tenant_id: record.tenant_id,
                        trigger_id: record.trigger_id,
                        fire_slot,
                    })
                    .await?;
                if updated.is_some() {
                    self.deps
                        .fire_settlement_observer
                        .on_failed_fire_settled(TriggerFailedFireSettlement {
                            fire: failed_fire,
                            reason,
                        })
                        .await;
                }
                Ok(TriggerPollerFireOutcome::OncePermanentFailed { reason })
            }
        }
    }
}

fn permanent_failure_disposition(
    schedule: &TriggerSchedule,
    fire_slot: Timestamp,
) -> Result<FailedFireDisposition, TriggerError> {
    match schedule {
        TriggerSchedule::Once { .. } => Ok(FailedFireDisposition::OncePermanentComplete),
        TriggerSchedule::Cron { .. } => match schedule.next_slot_after(fire_slot)? {
            Some(next) => Ok(FailedFireDisposition::RecurringPermanentReschedule(next)),
            None => Ok(FailedFireDisposition::Retryable),
        },
    }
}
