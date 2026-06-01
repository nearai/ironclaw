// arch-exempt: large_file, deterministic tick core + ports + reports + tests co-located for PR15, plan #4303

use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use async_trait::async_trait;
use ironclaw_host_api::{TenantId, Timestamp};
use ironclaw_turns::TurnRunId;

use crate::{
    ActiveTriggerScanCursor, ClaimDueFireOutcome, ClaimDueFireRequest, ClearActiveFireRequest,
    FireAcceptedRequest, FirePermanentFailedRequest, FireReplayedRequest,
    FireRetryableFailedRequest, FireTerminalFailedRequest, TriggerError, TriggerFire, TriggerId,
    TriggerInboundContentRef, TriggerPromptMaterializer, TriggerRecord, TriggerRepository,
    TriggerSourceProvider, TrustedTriggerPollerRepository, TrustedTriggerPollerScope,
};

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

pub struct TriggerPollerWorker {
    config: TriggerPollerWorkerConfig,
    deps: TriggerPollerWorkerDeps,
    // This guard is held only around synchronous cursor clone/set operations,
    // never across repository, materialization, submit, or lookup awaits.
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
            active_scan_cursor: Mutex::new(None),
        })
    }

    /// Executes one serialized poller tick.
    ///
    /// Production lifecycle wiring must not run overlapping ticks for the same
    /// worker instance. The active-scan cursor is a per-worker progress marker
    /// and is advanced by the single supervisor-owned tick loop.
    pub async fn tick_once(&self, now: Timestamp) -> Result<TriggerPollerTickReport, TriggerError> {
        let mut report = TriggerPollerTickReport::new(now);
        self.clear_terminal_active_fires(&mut report).await?;
        let due_records = self
            .deps
            .repository
            .as_ref()
            .list_due_triggers_trusted(
                TrustedTriggerPollerScope::mint(),
                now,
                self.config.fires_per_tick,
            )
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

    async fn clear_terminal_active_fires(
        &self,
        report: &mut TriggerPollerTickReport,
    ) -> Result<(), TriggerError> {
        let mut cursor = self.active_scan_cursor()?;
        let mut active_records = self
            .deps
            .repository
            .as_ref()
            .list_active_triggers_after_trusted(
                TrustedTriggerPollerScope::mint(),
                cursor.clone(),
                self.config.fires_per_tick,
            )
            .await?;
        if active_records.is_empty() && cursor.is_some() {
            cursor = None;
            active_records = self
                .deps
                .repository
                .as_ref()
                .list_active_triggers_after_trusted(
                    TrustedTriggerPollerScope::mint(),
                    cursor.clone(),
                    self.config.fires_per_tick,
                )
                .await?;
        }
        report.active_records = active_records.len();
        let mut next_cursor = cursor;
        let mut first_unadvanced_cursor: Option<ActiveTriggerScanCursor> = None;
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
                report.results.push(TriggerPollerFireReport {
                    tenant_id: record.tenant_id,
                    trigger_id: record.trigger_id,
                    fire_slot,
                    outcome: TriggerPollerFireOutcome::SkippedAlreadyActive {
                        active_fire_slot: fire_slot,
                        active_run_ref: None,
                    },
                });
                if first_unadvanced_cursor.is_none() {
                    next_cursor = Some(record_cursor);
                }
                continue;
            };
            let state = match self
                .deps
                .active_run_lookup
                .active_run_state(TriggerActiveRunStateRequest {
                    tenant_id: record.tenant_id.clone(),
                    trigger_id: record.trigger_id,
                    fire_slot,
                    run_id,
                })
                .await
            {
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
                TriggerActiveRunState::Terminal => {
                    if self
                        .deps
                        .repository
                        .clear_active_fire(ClearActiveFireRequest {
                            tenant_id: record.tenant_id.clone(),
                            trigger_id: record.trigger_id,
                            fire_slot,
                            run_id,
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

    async fn process_due_record(
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
        let content_ref = match self
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
            .submit_trusted_trigger_fire(TrustedTriggerSubmitRequest {
                fire,
                content_ref,
                received_at: now,
            })
            .await
        {
            Ok(TrustedTriggerFireSubmitOutcome::Accepted {
                run_id,
                submitted_at,
            }) => {
                let updated = self
                    .deps
                    .repository
                    .mark_fire_accepted(FireAcceptedRequest {
                        tenant_id: record.tenant_id,
                        trigger_id: record.trigger_id,
                        fire_slot,
                        run_id,
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
            }) => {
                let updated = self
                    .deps
                    .repository
                    .mark_fire_replayed(FireReplayedRequest {
                        tenant_id: record.tenant_id,
                        trigger_id: record.trigger_id,
                        fire_slot,
                        original_run_id,
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
            Ok(TrustedTriggerFireSubmitOutcome::RetryableFailed { reason }) => {
                self.persist_failed_fire(
                    record,
                    fire_slot,
                    FireFailureDisposition::Retryable,
                    TriggerPollerFailureReason::from_trusted_submit_failure(reason),
                )
                .await
            }
            Ok(TrustedTriggerFireSubmitOutcome::PermanentFailed { reason }) => {
                self.persist_failed_fire(
                    record,
                    fire_slot,
                    FireFailureDisposition::PermanentReschedule(next_run_at),
                    TriggerPollerFailureReason::from_trusted_submit_failure(reason),
                )
                .await
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TriggerPollerTickReport {
    pub now: Timestamp,
    pub active_records: usize,
    pub due_records: usize,
    pub results: Vec<TriggerPollerFireReport>,
}

impl TriggerPollerTickReport {
    fn new(now: Timestamp) -> Self {
        Self {
            now,
            active_records: 0,
            due_records: 0,
            results: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TriggerPollerFireReport {
    pub tenant_id: TenantId,
    pub trigger_id: TriggerId,
    pub fire_slot: Timestamp,
    pub outcome: TriggerPollerFireOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TriggerPollerFireOutcome {
    Submitted {
        run_id: TurnRunId,
    },
    Replayed {
        original_run_id: TurnRunId,
    },
    RetryableFailed {
        reason: TriggerPollerFailureReason,
    },
    PermanentFailed {
        reason: TriggerPollerFailureReason,
    },
    ClearedTerminalActive {
        run_id: TurnRunId,
    },
    ActiveRunLookupFailed {
        run_id: TurnRunId,
        reason: TriggerPollerFailureReason,
    },
    SkippedAlreadyCleared {
        run_id: TurnRunId,
    },
    SkippedAlreadyActive {
        active_fire_slot: Timestamp,
        active_run_ref: Option<TurnRunId>,
    },
    DueFireFailed {
        reason: TriggerPollerFailureReason,
    },
    SkippedNotDue,
    SkippedNotFound,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriggerPollerFailureReason {
    Backend,
    InvalidTriggerId,
    InvalidFireIdentityComponent,
    InvalidRecord,
    InvalidPollerConfig,
    InvalidSchedule,
    InvalidMaterialization,
    NotFound,
    SourceNoFire,
    TrustedSubmitRetryable,
    TrustedSubmitPermanent,
    ActiveRunLookup,
}

impl TriggerPollerFailureReason {
    fn from_trusted_submit_failure(reason: TrustedTriggerSubmitFailureReason) -> Self {
        match reason {
            TrustedTriggerSubmitFailureReason::Retryable => Self::TrustedSubmitRetryable,
            TrustedTriggerSubmitFailureReason::Permanent => Self::TrustedSubmitPermanent,
        }
    }
}

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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SubmitFailureKind {
    Retryable,
    Permanent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FireFailureDisposition {
    Retryable,
    PermanentReschedule(Timestamp),
    PermanentTerminal,
}

impl FireFailureDisposition {
    fn from_kind(kind: SubmitFailureKind, next_run_at: Timestamp) -> Self {
        match kind {
            SubmitFailureKind::Retryable => Self::Retryable,
            SubmitFailureKind::Permanent => Self::PermanentReschedule(next_run_at),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FailureClassification {
    kind: SubmitFailureKind,
    reason: TriggerPollerFailureReason,
}

fn classify_failure(error: &TriggerError) -> FailureClassification {
    let (kind, reason) = match error {
        TriggerError::Backend { .. } => (
            SubmitFailureKind::Retryable,
            TriggerPollerFailureReason::Backend,
        ),
        TriggerError::InvalidTriggerId { .. } => (
            SubmitFailureKind::Permanent,
            TriggerPollerFailureReason::InvalidTriggerId,
        ),
        TriggerError::InvalidFireIdentityComponent { .. } => (
            SubmitFailureKind::Permanent,
            TriggerPollerFailureReason::InvalidFireIdentityComponent,
        ),
        TriggerError::InvalidRecord { .. } => (
            SubmitFailureKind::Permanent,
            TriggerPollerFailureReason::InvalidRecord,
        ),
        TriggerError::InvalidPollerConfig { .. } => (
            SubmitFailureKind::Permanent,
            TriggerPollerFailureReason::InvalidPollerConfig,
        ),
        TriggerError::InvalidSchedule { .. } => (
            SubmitFailureKind::Permanent,
            TriggerPollerFailureReason::InvalidSchedule,
        ),
        TriggerError::InvalidMaterialization { .. } => (
            SubmitFailureKind::Permanent,
            TriggerPollerFailureReason::InvalidMaterialization,
        ),
        TriggerError::NotFound => (
            SubmitFailureKind::Permanent,
            TriggerPollerFailureReason::NotFound,
        ),
    };
    FailureClassification { kind, reason }
}

fn next_run_at_after_fire(
    record: &TriggerRecord,
    fire_slot: Timestamp,
) -> Result<Timestamp, TriggerError> {
    record
        .schedule
        .next_slot_after(fire_slot)?
        .ok_or_else(|| TriggerError::InvalidSchedule {
            reason: "schedule has no next fire slot after claimed fire".to_string(),
        })
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use chrono::{TimeZone, Utc};
    use ironclaw_host_api::{AgentId, ProjectId, UserId};

    use super::*;
    use crate::{
        ClaimedTriggerFire, InMemoryTriggerRepository, TriggerCompletionPolicy, TriggerRunStatus,
        TriggerSchedule, TriggerSourceKind, TriggerState,
    };

    fn ts(seconds: i64) -> Timestamp {
        Utc.timestamp_opt(seconds, 0).single().expect("valid ts")
    }

    fn ymd_hms(year: i32, month: u32, day: u32, hour: u32, minute: u32, second: u32) -> Timestamp {
        Utc.with_ymd_and_hms(year, month, day, hour, minute, second)
            .single()
            .expect("valid ts")
    }

    fn tenant(value: &str) -> TenantId {
        TenantId::new(value).expect("valid tenant")
    }

    fn user(value: &str) -> UserId {
        UserId::new(value).expect("valid user")
    }

    fn sample_record(
        trigger_id: TriggerId,
        tenant_id: TenantId,
        next_run_at: Timestamp,
    ) -> TriggerRecord {
        TriggerRecord {
            trigger_id,
            tenant_id,
            creator_user_id: user("user-a"),
            agent_id: Some(AgentId::new("agent-a").expect("valid agent")),
            project_id: Some(ProjectId::new("project-a").expect("valid project")),
            name: "daily summary".to_string(),
            source: TriggerSourceKind::Schedule,
            schedule: TriggerSchedule::cron("0 8 * * *").expect("valid cron"),
            completion_policy: TriggerCompletionPolicy::Recurring,
            prompt: "summarize unread mail".to_string(),
            state: TriggerState::Scheduled,
            next_run_at,
            last_run_at: None,
            last_fired_slot: None,
            last_status: None,
            active_fire_slot: None,
            active_run_ref: None,
            created_at: ts(1_704_067_000),
        }
    }

    #[test]
    fn worker_config_rejects_noop_or_unsupported_settings() {
        let config = TriggerPollerWorkerConfig {
            poll_interval: Duration::ZERO,
            ..TriggerPollerWorkerConfig::default()
        };
        assert!(matches!(
            config.validate(),
            Err(TriggerError::InvalidPollerConfig { .. })
        ));

        let config = TriggerPollerWorkerConfig {
            fires_per_tick: 0,
            ..TriggerPollerWorkerConfig::default()
        };
        assert!(matches!(
            config.validate(),
            Err(TriggerError::InvalidPollerConfig { .. })
        ));

        let config = TriggerPollerWorkerConfig {
            max_concurrent_fires_per_trigger: 2,
            ..TriggerPollerWorkerConfig::default()
        };
        assert!(matches!(
            config.validate(),
            Err(TriggerError::InvalidPollerConfig { .. })
        ));
    }

    #[test]
    fn worker_new_rejects_invalid_config() {
        let config = TriggerPollerWorkerConfig {
            fires_per_tick: 0,
            ..TriggerPollerWorkerConfig::default()
        };
        let result = TriggerPollerWorker::new(
            config,
            TriggerPollerWorkerDeps {
                repository: Arc::new(InMemoryTriggerRepository::default()),
                source_provider: Arc::new(crate::ScheduleTriggerSourceProvider),
                materializer: Arc::new(RecordingMaterializer::success("content:trigger-fire")),
                trusted_submitter: Arc::new(RecordingSubmitter::with_outcomes(Vec::new())),
                active_run_lookup: Arc::new(RecordingActiveRunLookup::default()),
            },
        );

        assert!(matches!(
            result,
            Err(TriggerError::InvalidPollerConfig { .. })
        ));
    }

    fn worker(
        repo: Arc<dyn TriggerRepository>,
        materializer: Arc<RecordingMaterializer>,
        submitter: Arc<RecordingSubmitter>,
        active_lookup: Arc<RecordingActiveRunLookup>,
    ) -> TriggerPollerWorker {
        worker_with_source_provider(
            repo,
            Arc::new(crate::ScheduleTriggerSourceProvider),
            materializer,
            submitter,
            active_lookup,
        )
    }

    fn worker_with_source_provider(
        repo: Arc<dyn TriggerRepository>,
        source_provider: Arc<dyn TriggerSourceProvider>,
        materializer: Arc<RecordingMaterializer>,
        submitter: Arc<RecordingSubmitter>,
        active_lookup: Arc<RecordingActiveRunLookup>,
    ) -> TriggerPollerWorker {
        worker_with_config(
            repo,
            source_provider,
            materializer,
            submitter,
            active_lookup,
            TriggerPollerWorkerConfig::default(),
        )
    }

    fn worker_with_config(
        repo: Arc<dyn TriggerRepository>,
        source_provider: Arc<dyn TriggerSourceProvider>,
        materializer: Arc<RecordingMaterializer>,
        submitter: Arc<RecordingSubmitter>,
        active_lookup: Arc<RecordingActiveRunLookup>,
        config: TriggerPollerWorkerConfig,
    ) -> TriggerPollerWorker {
        TriggerPollerWorker::new(
            config,
            TriggerPollerWorkerDeps {
                repository: repo,
                source_provider,
                materializer,
                trusted_submitter: submitter,
                active_run_lookup: active_lookup,
            },
        )
        .expect("valid worker")
    }

    #[tokio::test]
    async fn tick_processes_one_due_trigger_happy_path() {
        let repo = Arc::new(InMemoryTriggerRepository::default());
        let trigger_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
        let fire_slot = ts(1_704_067_200);
        let record = sample_record(trigger_id, tenant("tenant-a"), fire_slot);
        let expected_next_run_at = record
            .schedule
            .next_slot_after(fire_slot)
            .expect("next run")
            .expect("future run");
        repo.upsert_trigger(record.clone()).await.expect("insert");
        let run_id = TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5a").expect("run id");
        let submitter = Arc::new(RecordingSubmitter::with_outcomes(vec![Ok(
            TrustedTriggerFireSubmitOutcome::Accepted {
                run_id,
                submitted_at: ts(1_704_067_205),
            },
        )]));
        let materializer = Arc::new(RecordingMaterializer::success("content:trigger-fire"));
        let worker = worker(
            repo.clone(),
            materializer.clone(),
            submitter.clone(),
            Arc::new(RecordingActiveRunLookup::default()),
        );

        let report = worker.tick_once(fire_slot).await.expect("tick succeeds");

        assert_eq!(report.due_records, 1);
        assert_eq!(
            report.results.last().map(|result| &result.outcome),
            Some(&TriggerPollerFireOutcome::Submitted { run_id })
        );
        assert_eq!(materializer.fires().len(), 1);
        assert_eq!(submitter.requests().len(), 1);
        let request = submitter.requests().pop().expect("submit request");
        assert_eq!(request.fire.identity.trigger_id, trigger_id);
        assert_eq!(request.fire.identity.fire_slot, fire_slot);
        assert_eq!(request.fire.creator_user_id, record.creator_user_id);
        assert_eq!(request.fire.agent_id, record.agent_id);
        assert_eq!(request.fire.project_id, record.project_id);
        assert_eq!(request.content_ref.as_str(), "content:trigger-fire");

        let persisted = repo
            .get_trigger(tenant("tenant-a"), trigger_id)
            .await
            .expect("load")
            .expect("record present");
        assert_eq!(persisted.last_status, Some(TriggerRunStatus::Ok));
        assert_eq!(persisted.last_fired_slot, Some(fire_slot));
        assert_eq!(persisted.active_fire_slot, Some(fire_slot));
        assert_eq!(persisted.active_run_ref, Some(run_id));
        assert_eq!(persisted.next_run_at, expected_next_run_at);
    }

    #[tokio::test]
    async fn tick_persists_replayed_submit_with_original_run_ref() {
        let repo = Arc::new(InMemoryTriggerRepository::default());
        let trigger_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
        let fire_slot = ts(1_704_067_200);
        let record = sample_record(trigger_id, tenant("tenant-a"), fire_slot);
        let expected_next_run_at = record
            .schedule
            .next_slot_after(fire_slot)
            .expect("next run")
            .expect("future run");
        repo.upsert_trigger(record).await.expect("insert");
        let original_run_id =
            TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5a").expect("run id");
        let worker = worker(
            repo.clone(),
            Arc::new(RecordingMaterializer::success("content:trigger-fire")),
            Arc::new(RecordingSubmitter::with_outcomes(vec![Ok(
                TrustedTriggerFireSubmitOutcome::Replayed {
                    original_run_id,
                    replayed_at: ts(1_704_067_205),
                },
            )])),
            Arc::new(RecordingActiveRunLookup::default()),
        );

        let report = worker.tick_once(fire_slot).await.expect("tick succeeds");

        assert_eq!(
            report.results.last().map(|result| &result.outcome),
            Some(&TriggerPollerFireOutcome::Replayed { original_run_id })
        );
        let persisted = repo
            .get_trigger(tenant("tenant-a"), trigger_id)
            .await
            .expect("load")
            .expect("record present");
        assert_eq!(persisted.last_status, Some(TriggerRunStatus::Ok));
        assert_eq!(persisted.last_fired_slot, Some(fire_slot));
        assert_eq!(persisted.active_fire_slot, Some(fire_slot));
        assert_eq!(persisted.active_run_ref, Some(original_run_id));
        assert_eq!(persisted.next_run_at, expected_next_run_at);
    }

    #[tokio::test]
    async fn tick_skips_claim_race_already_active_without_materializing() {
        let trigger_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
        let fire_slot = ts(1_704_067_200);
        let active_run_ref =
            TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5a").expect("run id");
        let repository = Arc::new(ClaimRaceRepository::new(
            sample_record(trigger_id, tenant("tenant-a"), fire_slot),
            ClaimDueFireOutcome::AlreadyActive {
                active_fire_slot: Some(fire_slot),
                active_run_ref: Some(active_run_ref),
            },
        ));
        let materializer = Arc::new(RecordingMaterializer::success("content:trigger-fire"));
        let submitter = Arc::new(RecordingSubmitter::with_outcomes(Vec::new()));
        let worker = worker(
            repository,
            materializer.clone(),
            submitter.clone(),
            Arc::new(RecordingActiveRunLookup::default()),
        );

        let report = worker.tick_once(fire_slot).await.expect("tick succeeds");

        assert_eq!(report.due_records, 1);
        assert_eq!(
            report.results.last().map(|result| &result.outcome),
            Some(&TriggerPollerFireOutcome::SkippedAlreadyActive {
                active_fire_slot: fire_slot,
                active_run_ref: Some(active_run_ref)
            })
        );
        assert_eq!(materializer.fires().len(), 0);
        assert_eq!(submitter.requests().len(), 0);
    }

    #[tokio::test]
    async fn tick_skips_active_trigger_but_processes_other_due_trigger() {
        let repo = Arc::new(InMemoryTriggerRepository::default());
        let fire_slot = ts(1_704_067_200);
        let active_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
        let due_id = TriggerId::parse("01J00000000000000000000000").expect("ulid");
        let active_run_ref =
            TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5a").expect("run id");
        let mut active = sample_record(active_id, tenant("tenant-a"), fire_slot);
        active.active_fire_slot = Some(fire_slot);
        active.active_run_ref = Some(active_run_ref);
        let due = sample_record(due_id, tenant("tenant-a"), fire_slot);
        repo.upsert_trigger(active).await.expect("insert active");
        repo.upsert_trigger(due).await.expect("insert due");
        let due_run_ref = TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5b").expect("run id");
        let submitter = Arc::new(RecordingSubmitter::with_outcomes(vec![Ok(
            TrustedTriggerFireSubmitOutcome::Accepted {
                run_id: due_run_ref,
                submitted_at: fire_slot,
            },
        )]));
        let active_lookup = Arc::new(RecordingActiveRunLookup::with_state(
            TriggerActiveRunState::Nonterminal,
        ));
        let worker = worker(
            repo.clone(),
            Arc::new(RecordingMaterializer::success("content:trigger-fire")),
            submitter,
            active_lookup,
        );

        let report = worker.tick_once(fire_slot).await.expect("tick succeeds");

        assert_eq!(report.active_records, 1);
        assert_eq!(report.due_records, 1);
        assert!(
            report
                .results
                .iter()
                .any(|result| result.trigger_id == active_id
                    && matches!(
                        result.outcome,
                        TriggerPollerFireOutcome::SkippedAlreadyActive { .. }
                    ))
        );
        assert!(
            report
                .results
                .iter()
                .any(|result| result.trigger_id == due_id
                    && result.outcome
                        == TriggerPollerFireOutcome::Submitted {
                            run_id: due_run_ref
                        })
        );
    }

    #[tokio::test]
    async fn tick_clears_terminal_active_run() {
        let repo = Arc::new(InMemoryTriggerRepository::default());
        let trigger_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
        let fire_slot = ts(1_704_067_200);
        let run_id = TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5a").expect("run id");
        let mut record = sample_record(trigger_id, tenant("tenant-a"), ts(1_704_067_260));
        record.active_fire_slot = Some(fire_slot);
        record.active_run_ref = Some(run_id);
        repo.upsert_trigger(record).await.expect("insert active");
        let active_lookup = Arc::new(RecordingActiveRunLookup::with_state(
            TriggerActiveRunState::Terminal,
        ));
        let worker = worker(
            repo.clone(),
            Arc::new(RecordingMaterializer::success("content:trigger-fire")),
            Arc::new(RecordingSubmitter::with_outcomes(Vec::new())),
            active_lookup.clone(),
        );

        let report = worker.tick_once(fire_slot).await.expect("tick succeeds");

        assert_eq!(report.active_records, 1);
        assert_eq!(
            report.results.last().map(|result| &result.outcome),
            Some(&TriggerPollerFireOutcome::ClearedTerminalActive { run_id })
        );
        assert_eq!(
            active_lookup.requests(),
            vec![TriggerActiveRunStateRequest {
                tenant_id: tenant("tenant-a"),
                trigger_id,
                fire_slot,
                run_id,
            }]
        );
        let persisted = repo
            .get_trigger(tenant("tenant-a"), trigger_id)
            .await
            .expect("load")
            .expect("record present");
        assert_eq!(persisted.active_fire_slot, None);
        assert_eq!(persisted.active_run_ref, None);
    }

    #[tokio::test]
    async fn tick_active_cleanup_cursor_reaches_terminal_rows_after_blocked_page() {
        let repo = Arc::new(InMemoryTriggerRepository::default());
        let first_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
        let second_id = TriggerId::parse("01J00000000000000000000000").expect("ulid");
        let third_id = TriggerId::parse("01J00000000000000000000001").expect("ulid");
        let fourth_id = TriggerId::parse("01J00000000000000000000002").expect("ulid");
        let terminal_id = TriggerId::parse("01J00000000000000000000003").expect("ulid");
        let first_slot = ts(1_704_067_200);
        let second_slot = ts(1_704_067_260);
        let third_slot = ts(1_704_067_320);
        let fourth_slot = ts(1_704_067_380);
        let terminal_slot = ts(1_704_067_440);
        let first_run = TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5a").expect("run id");
        let second_run = TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5b").expect("run id");
        let third_run = TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5c").expect("run id");
        let fourth_run = TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5d").expect("run id");
        let terminal_run =
            TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5e").expect("run id");

        let mut first = sample_record(first_id, tenant("tenant-a"), ts(1_704_067_800));
        first.active_fire_slot = Some(first_slot);
        first.active_run_ref = Some(first_run);
        let mut second = sample_record(second_id, tenant("tenant-a"), ts(1_704_067_800));
        second.active_fire_slot = Some(second_slot);
        second.active_run_ref = Some(second_run);
        let mut third = sample_record(third_id, tenant("tenant-a"), ts(1_704_067_800));
        third.active_fire_slot = Some(third_slot);
        third.active_run_ref = Some(third_run);
        let mut fourth = sample_record(fourth_id, tenant("tenant-a"), ts(1_704_067_800));
        fourth.active_fire_slot = Some(fourth_slot);
        fourth.active_run_ref = Some(fourth_run);
        let mut terminal = sample_record(terminal_id, tenant("tenant-a"), ts(1_704_067_800));
        terminal.active_fire_slot = Some(terminal_slot);
        terminal.active_run_ref = Some(terminal_run);
        repo.upsert_trigger(first).await.expect("insert first");
        repo.upsert_trigger(second).await.expect("insert second");
        repo.upsert_trigger(third).await.expect("insert third");
        repo.upsert_trigger(fourth).await.expect("insert fourth");
        repo.upsert_trigger(terminal)
            .await
            .expect("insert terminal");

        let active_lookup = Arc::new(RecordingActiveRunLookup::with_results(vec![
            Ok(TriggerActiveRunState::Nonterminal),
            Ok(TriggerActiveRunState::Nonterminal),
            Ok(TriggerActiveRunState::Nonterminal),
            Ok(TriggerActiveRunState::Nonterminal),
            Ok(TriggerActiveRunState::Terminal),
        ]));
        let worker = worker_with_config(
            repo.clone(),
            Arc::new(crate::ScheduleTriggerSourceProvider),
            Arc::new(RecordingMaterializer::success("content:trigger-fire")),
            Arc::new(RecordingSubmitter::with_outcomes(Vec::new())),
            active_lookup.clone(),
            TriggerPollerWorkerConfig {
                fires_per_tick: 2,
                ..TriggerPollerWorkerConfig::default()
            },
        );

        let first_report = worker.tick_once(first_slot).await.expect("first tick");
        assert_eq!(first_report.active_records, 2);
        assert!(
            first_report
                .results
                .iter()
                .all(|result| result.trigger_id != terminal_id)
        );

        let second_report = worker.tick_once(second_slot).await.expect("second tick");
        assert_eq!(second_report.active_records, 2);
        assert!(
            second_report
                .results
                .iter()
                .all(|result| result.trigger_id != terminal_id)
        );

        let third_report = worker.tick_once(third_slot).await.expect("third tick");
        assert_eq!(third_report.active_records, 1);
        assert!(
            third_report
                .results
                .iter()
                .any(|result| result.trigger_id == terminal_id
                    && result.outcome
                        == TriggerPollerFireOutcome::ClearedTerminalActive {
                            run_id: terminal_run
                        })
        );
        assert_eq!(active_lookup.requests().len(), 5);
        let persisted = repo
            .get_trigger(tenant("tenant-a"), terminal_id)
            .await
            .expect("load terminal")
            .expect("terminal record");
        assert_eq!(persisted.active_fire_slot, None);
        assert_eq!(persisted.active_run_ref, None);
    }

    #[tokio::test]
    async fn tick_active_cleanup_cursor_wraps_to_start_when_page_is_empty() {
        let repo = Arc::new(InMemoryTriggerRepository::default());
        let first_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
        let second_id = TriggerId::parse("01J00000000000000000000000").expect("ulid");
        let third_id = TriggerId::parse("01J00000000000000000000001").expect("ulid");
        let first_slot = ts(1_704_067_200);
        let second_slot = ts(1_704_067_260);
        let third_slot = ts(1_704_067_320);
        let first_run = TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5a").expect("run id");
        let second_run = TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5b").expect("run id");
        let third_run = TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5c").expect("run id");

        let mut first = sample_record(first_id, tenant("tenant-a"), ts(1_704_067_800));
        first.active_fire_slot = Some(first_slot);
        first.active_run_ref = Some(first_run);
        let mut second = sample_record(second_id, tenant("tenant-a"), ts(1_704_067_800));
        second.active_fire_slot = Some(second_slot);
        second.active_run_ref = Some(second_run);
        let mut third = sample_record(third_id, tenant("tenant-a"), ts(1_704_067_800));
        third.active_fire_slot = Some(third_slot);
        third.active_run_ref = Some(third_run);
        repo.upsert_trigger(first).await.expect("insert first");
        repo.upsert_trigger(second).await.expect("insert second");
        repo.upsert_trigger(third).await.expect("insert third");

        let active_lookup = Arc::new(RecordingActiveRunLookup::with_results(vec![
            Ok(TriggerActiveRunState::Nonterminal),
            Ok(TriggerActiveRunState::Nonterminal),
            Ok(TriggerActiveRunState::Nonterminal),
            Ok(TriggerActiveRunState::Nonterminal),
            Ok(TriggerActiveRunState::Nonterminal),
        ]));
        let worker = worker_with_config(
            repo,
            Arc::new(crate::ScheduleTriggerSourceProvider),
            Arc::new(RecordingMaterializer::success("content:trigger-fire")),
            Arc::new(RecordingSubmitter::with_outcomes(Vec::new())),
            active_lookup.clone(),
            TriggerPollerWorkerConfig {
                fires_per_tick: 2,
                ..TriggerPollerWorkerConfig::default()
            },
        );

        let first_report = worker.tick_once(first_slot).await.expect("first tick");
        assert_eq!(first_report.active_records, 2);
        let second_report = worker.tick_once(second_slot).await.expect("second tick");
        assert_eq!(second_report.active_records, 1);

        let third_report = worker.tick_once(third_slot).await.expect("third tick");
        assert_eq!(third_report.active_records, 2);
        assert_eq!(
            third_report
                .results
                .iter()
                .map(|result| result.trigger_id)
                .collect::<Vec<_>>(),
            vec![first_id, second_id]
        );
        assert_eq!(
            active_lookup
                .requests()
                .into_iter()
                .map(|request| request.trigger_id)
                .collect::<Vec<_>>(),
            vec![first_id, second_id, third_id, first_id, second_id]
        );
    }

    #[tokio::test]
    async fn tick_fails_when_wrap_refetch_returns_backend_error() {
        let trigger_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
        let fire_slot = ts(1_704_067_200);
        let run_id = TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5a").expect("run id");
        let mut record = sample_record(trigger_id, tenant("tenant-a"), ts(1_704_067_800));
        record.active_fire_slot = Some(fire_slot);
        record.active_run_ref = Some(run_id);
        let repo = Arc::new(ActiveWrapRefetchErrorRepository::new(record));
        let worker = worker_with_config(
            repo.clone(),
            Arc::new(crate::ScheduleTriggerSourceProvider),
            Arc::new(RecordingMaterializer::success("content:trigger-fire")),
            Arc::new(RecordingSubmitter::with_outcomes(Vec::new())),
            Arc::new(RecordingActiveRunLookup::with_state(
                TriggerActiveRunState::Nonterminal,
            )),
            TriggerPollerWorkerConfig {
                fires_per_tick: 1,
                ..TriggerPollerWorkerConfig::default()
            },
        );

        let first_report = worker.tick_once(fire_slot).await.expect("first tick");
        assert_eq!(first_report.active_records, 1);

        let error = worker
            .tick_once(fire_slot)
            .await
            .expect_err("wrap refetch fails");
        assert!(matches!(error, TriggerError::Backend { .. }));
        assert_eq!(repo.active_scan_call_shapes(), vec![false, true, false]);
    }

    #[tokio::test]
    async fn tick_retries_active_page_when_clear_fails_before_advancing_cursor() {
        let first_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
        let second_id = TriggerId::parse("01J00000000000000000000000").expect("ulid");
        let third_id = TriggerId::parse("01J00000000000000000000001").expect("ulid");
        let first_slot = ts(1_704_067_200);
        let second_slot = ts(1_704_067_260);
        let third_slot = ts(1_704_067_320);
        let first_run = TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5a").expect("run id");
        let second_run = TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5b").expect("run id");
        let third_run = TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5c").expect("run id");

        let mut first = sample_record(first_id, tenant("tenant-a"), ts(1_704_067_800));
        first.active_fire_slot = Some(first_slot);
        first.active_run_ref = Some(first_run);
        let mut second = sample_record(second_id, tenant("tenant-a"), ts(1_704_067_800));
        second.active_fire_slot = Some(second_slot);
        second.active_run_ref = Some(second_run);
        let mut third = sample_record(third_id, tenant("tenant-a"), ts(1_704_067_800));
        third.active_fire_slot = Some(third_slot);
        third.active_run_ref = Some(third_run);

        let repo = Arc::new(ActiveClearFailsOnceRepository::new(
            vec![first, second, third],
            second_id,
        ));
        let active_lookup = Arc::new(RecordingActiveRunLookup::with_results(vec![
            Ok(TriggerActiveRunState::Terminal),
            Ok(TriggerActiveRunState::Terminal),
            Ok(TriggerActiveRunState::Terminal),
            Ok(TriggerActiveRunState::Terminal),
        ]));
        let worker = worker_with_config(
            repo.clone(),
            Arc::new(crate::ScheduleTriggerSourceProvider),
            Arc::new(RecordingMaterializer::success("content:trigger-fire")),
            Arc::new(RecordingSubmitter::with_outcomes(Vec::new())),
            active_lookup,
            TriggerPollerWorkerConfig {
                fires_per_tick: 2,
                ..TriggerPollerWorkerConfig::default()
            },
        );

        let first_error = worker.tick_once(first_slot).await.expect_err("clear fails");
        assert!(matches!(first_error, TriggerError::Backend { .. }));

        let second_report = worker.tick_once(second_slot).await.expect("retry tick");

        assert_eq!(second_report.active_records, 2);
        assert_eq!(
            repo.clear_requests(),
            vec![first_id, second_id, second_id, third_id]
        );
        assert!(
            second_report
                .results
                .iter()
                .any(|result| result.trigger_id == second_id
                    && result.outcome
                        == TriggerPollerFireOutcome::ClearedTerminalActive { run_id: second_run })
        );
        assert!(
            second_report
                .results
                .iter()
                .any(|result| result.trigger_id == third_id
                    && result.outcome
                        == TriggerPollerFireOutcome::ClearedTerminalActive { run_id: third_run })
        );
    }

    #[tokio::test]
    async fn tick_reports_terminal_active_clear_race() {
        let trigger_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
        let fire_slot = ts(1_704_067_200);
        let run_id = TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5a").expect("run id");
        let mut record = sample_record(trigger_id, tenant("tenant-a"), ts(1_704_067_260));
        record.active_fire_slot = Some(fire_slot);
        record.active_run_ref = Some(run_id);
        let worker = worker(
            Arc::new(ActiveClearRaceRepository {
                active_record: record,
            }),
            Arc::new(RecordingMaterializer::success("content:trigger-fire")),
            Arc::new(RecordingSubmitter::with_outcomes(Vec::new())),
            Arc::new(RecordingActiveRunLookup::with_state(
                TriggerActiveRunState::Terminal,
            )),
        );

        let report = worker.tick_once(fire_slot).await.expect("tick succeeds");

        assert_eq!(report.active_records, 1);
        assert_eq!(
            report.results.last().map(|result| &result.outcome),
            Some(&TriggerPollerFireOutcome::SkippedAlreadyCleared { run_id })
        );
    }

    #[tokio::test]
    async fn tick_clears_terminal_active_and_processes_due_trigger() {
        let repo = Arc::new(InMemoryTriggerRepository::default());
        let active_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
        let due_id = TriggerId::parse("01J00000000000000000000000").expect("ulid");
        let fire_slot = ts(1_704_067_200);
        let run_id = TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5a").expect("run id");
        let mut active = sample_record(active_id, tenant("tenant-a"), ts(1_704_067_260));
        active.active_fire_slot = Some(fire_slot);
        active.active_run_ref = Some(run_id);
        repo.upsert_trigger(active).await.expect("insert active");
        repo.upsert_trigger(sample_record(due_id, tenant("tenant-a"), fire_slot))
            .await
            .expect("insert due");
        let due_run_id = TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5b").expect("run id");
        let worker = worker(
            repo.clone(),
            Arc::new(RecordingMaterializer::success("content:trigger-fire")),
            Arc::new(RecordingSubmitter::with_outcomes(vec![Ok(
                TrustedTriggerFireSubmitOutcome::Accepted {
                    run_id: due_run_id,
                    submitted_at: fire_slot,
                },
            )])),
            Arc::new(RecordingActiveRunLookup::with_state(
                TriggerActiveRunState::Terminal,
            )),
        );

        let report = worker.tick_once(fire_slot).await.expect("tick succeeds");

        assert!(
            report
                .results
                .iter()
                .any(|result| result.trigger_id == active_id
                    && result.outcome
                        == TriggerPollerFireOutcome::ClearedTerminalActive { run_id })
        );
        assert!(
            report
                .results
                .iter()
                .any(|result| result.trigger_id == due_id
                    && result.outcome
                        == TriggerPollerFireOutcome::Submitted { run_id: due_run_id })
        );
    }

    #[tokio::test]
    async fn tick_reports_active_lookup_error_and_continues_to_due_triggers() {
        let repo = Arc::new(InMemoryTriggerRepository::default());
        let active_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
        let due_id = TriggerId::parse("01J00000000000000000000000").expect("ulid");
        let fire_slot = ts(1_704_067_200);
        let run_id = TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5a").expect("run id");
        let mut active = sample_record(active_id, tenant("tenant-a"), ts(1_704_067_260));
        active.active_fire_slot = Some(fire_slot);
        active.active_run_ref = Some(run_id);
        repo.upsert_trigger(active).await.expect("insert active");
        repo.upsert_trigger(sample_record(due_id, tenant("tenant-a"), fire_slot))
            .await
            .expect("insert due");
        let due_run_id = TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5b").expect("run id");
        let worker = worker(
            repo,
            Arc::new(RecordingMaterializer::success("content:trigger-fire")),
            Arc::new(RecordingSubmitter::with_outcomes(vec![Ok(
                TrustedTriggerFireSubmitOutcome::Accepted {
                    run_id: due_run_id,
                    submitted_at: fire_slot,
                },
            )])),
            Arc::new(RecordingActiveRunLookup::with_results(vec![Err(
                TriggerError::Backend {
                    reason: "turn state unavailable".to_string(),
                },
            )])),
        );

        let report = worker.tick_once(fire_slot).await.expect("tick succeeds");

        assert!(
            report
                .results
                .iter()
                .any(|result| result.trigger_id == active_id
                    && matches!(
                        result.outcome,
                        TriggerPollerFireOutcome::ActiveRunLookupFailed {
                            run_id: actual_run_id,
                            reason: TriggerPollerFailureReason::ActiveRunLookup,
                        } if actual_run_id == run_id
                    ))
        );
        assert!(
            report
                .results
                .iter()
                .any(|result| result.trigger_id == due_id
                    && result.outcome
                        == TriggerPollerFireOutcome::Submitted { run_id: due_run_id })
        );
    }

    #[tokio::test]
    async fn tick_retries_active_lookup_error_before_advancing_cursor() {
        let repo = Arc::new(InMemoryTriggerRepository::default());
        let failed_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
        let terminal_id = TriggerId::parse("01J00000000000000000000000").expect("ulid");
        let failed_slot = ts(1_704_067_200);
        let terminal_slot = ts(1_704_067_260);
        let failed_run = TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5a").expect("run id");
        let terminal_run =
            TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5b").expect("run id");
        let mut failed = sample_record(failed_id, tenant("tenant-a"), ts(1_704_067_800));
        failed.active_fire_slot = Some(failed_slot);
        failed.active_run_ref = Some(failed_run);
        let mut terminal = sample_record(terminal_id, tenant("tenant-a"), ts(1_704_067_800));
        terminal.active_fire_slot = Some(terminal_slot);
        terminal.active_run_ref = Some(terminal_run);
        repo.upsert_trigger(failed).await.expect("insert failed");
        repo.upsert_trigger(terminal)
            .await
            .expect("insert terminal");

        let active_lookup = Arc::new(RecordingActiveRunLookup::with_results(vec![
            Err(TriggerError::Backend {
                reason: "turn state unavailable".to_string(),
            }),
            Ok(TriggerActiveRunState::Terminal),
            Ok(TriggerActiveRunState::Terminal),
        ]));
        let worker = worker_with_config(
            repo.clone(),
            Arc::new(crate::ScheduleTriggerSourceProvider),
            Arc::new(RecordingMaterializer::success("content:trigger-fire")),
            Arc::new(RecordingSubmitter::with_outcomes(Vec::new())),
            active_lookup.clone(),
            TriggerPollerWorkerConfig {
                fires_per_tick: 2,
                ..TriggerPollerWorkerConfig::default()
            },
        );

        let first_report = worker.tick_once(failed_slot).await.expect("first tick");
        assert!(
            first_report
                .results
                .iter()
                .any(|result| result.trigger_id == failed_id
                    && matches!(
                        result.outcome,
                        TriggerPollerFireOutcome::ActiveRunLookupFailed { .. }
                    ))
        );
        assert!(
            first_report
                .results
                .iter()
                .any(|result| result.trigger_id == terminal_id
                    && result.outcome
                        == TriggerPollerFireOutcome::ClearedTerminalActive {
                            run_id: terminal_run
                        })
        );

        let second_report = worker.tick_once(terminal_slot).await.expect("second tick");
        assert_eq!(second_report.active_records, 1);
        assert!(
            second_report
                .results
                .iter()
                .any(|result| result.trigger_id == failed_id
                    && result.outcome
                        == TriggerPollerFireOutcome::ClearedTerminalActive { run_id: failed_run })
        );
        assert_eq!(
            active_lookup
                .requests()
                .into_iter()
                .map(|request| request.trigger_id)
                .collect::<Vec<_>>(),
            vec![failed_id, terminal_id, failed_id]
        );
    }

    #[tokio::test]
    async fn tick_replayed_submit_can_be_cleared_on_a_later_tick_without_stopping_due_work() {
        let repo = Arc::new(InMemoryTriggerRepository::default());
        let replayed_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
        let due_id = TriggerId::parse("01J00000000000000000000000").expect("ulid");
        let fire_slot = ts(1_704_067_200);
        let replayed_run_id =
            TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5a").expect("run id");
        repo.upsert_trigger(sample_record(replayed_id, tenant("tenant-a"), fire_slot))
            .await
            .expect("insert replayed candidate");
        let first_worker = worker(
            repo.clone(),
            Arc::new(RecordingMaterializer::success("content:trigger-fire")),
            Arc::new(RecordingSubmitter::with_outcomes(vec![Ok(
                TrustedTriggerFireSubmitOutcome::Replayed {
                    original_run_id: replayed_run_id,
                    replayed_at: ts(1_704_067_205),
                },
            )])),
            Arc::new(RecordingActiveRunLookup::default()),
        );

        let first_report = first_worker.tick_once(fire_slot).await.expect("first tick");
        assert_eq!(
            first_report.results.last().map(|result| &result.outcome),
            Some(&TriggerPollerFireOutcome::Replayed {
                original_run_id: replayed_run_id
            })
        );
        let persisted_after_replay = repo
            .get_trigger(tenant("tenant-a"), replayed_id)
            .await
            .expect("reload replayed")
            .expect("replayed record");
        assert_eq!(persisted_after_replay.active_fire_slot, Some(fire_slot));
        assert_eq!(persisted_after_replay.active_run_ref, Some(replayed_run_id));

        repo.upsert_trigger(sample_record(due_id, tenant("tenant-a"), fire_slot))
            .await
            .expect("insert later due");

        let second_due_run_id =
            TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5c").expect("run id");
        let second_worker = worker(
            repo.clone(),
            Arc::new(RecordingMaterializer::success("content:trigger-fire")),
            Arc::new(RecordingSubmitter::with_outcomes(vec![Ok(
                TrustedTriggerFireSubmitOutcome::Accepted {
                    run_id: second_due_run_id,
                    submitted_at: fire_slot,
                },
            )])),
            Arc::new(RecordingActiveRunLookup::with_results(vec![Ok(
                TriggerActiveRunState::Terminal,
            )])),
        );

        let second_report = second_worker
            .tick_once(fire_slot)
            .await
            .expect("second tick");

        assert_eq!(second_report.active_records, 1);
        assert_eq!(second_report.due_records, 1);
        assert!(
            second_report
                .results
                .iter()
                .any(|result| result.trigger_id == replayed_id
                    && result.outcome
                        == TriggerPollerFireOutcome::ClearedTerminalActive {
                            run_id: replayed_run_id,
                        })
        );
        assert!(
            second_report
                .results
                .iter()
                .any(|result| result.trigger_id == due_id
                    && result.outcome
                        == TriggerPollerFireOutcome::Submitted {
                            run_id: second_due_run_id
                        })
        );
        assert_eq!(
            repo.get_trigger(tenant("tenant-a"), replayed_id)
                .await
                .expect("reload replayed after cleanup")
                .expect("replayed record after cleanup")
                .active_fire_slot,
            None
        );
        assert_eq!(
            repo.get_trigger(tenant("tenant-a"), replayed_id)
                .await
                .expect("reload replayed after cleanup")
                .expect("replayed record after cleanup")
                .active_run_ref,
            None
        );
    }

    #[tokio::test]
    async fn tick_keeps_missing_active_run_blocked() {
        let repo = Arc::new(InMemoryTriggerRepository::default());
        let trigger_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
        let fire_slot = ts(1_704_067_200);
        let run_id = TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5a").expect("run id");
        let mut record = sample_record(trigger_id, tenant("tenant-a"), fire_slot);
        record.active_fire_slot = Some(fire_slot);
        record.active_run_ref = Some(run_id);
        repo.upsert_trigger(record).await.expect("insert active");
        let active_lookup = Arc::new(RecordingActiveRunLookup::with_state(
            TriggerActiveRunState::Missing,
        ));
        let worker = worker(
            repo.clone(),
            Arc::new(RecordingMaterializer::success("content:trigger-fire")),
            Arc::new(RecordingSubmitter::with_outcomes(Vec::new())),
            active_lookup.clone(),
        );

        let report = worker.tick_once(fire_slot).await.expect("tick succeeds");

        assert!(matches!(
            report.results.last().map(|result| &result.outcome),
            Some(TriggerPollerFireOutcome::SkippedAlreadyActive { .. })
        ));
        assert_eq!(
            active_lookup.requests(),
            vec![TriggerActiveRunStateRequest {
                tenant_id: tenant("tenant-a"),
                trigger_id,
                fire_slot,
                run_id,
            }]
        );
        let persisted = repo
            .get_trigger(tenant("tenant-a"), trigger_id)
            .await
            .expect("load")
            .expect("record present");
        assert_eq!(persisted.active_fire_slot, Some(fire_slot));
        assert_eq!(persisted.active_run_ref, Some(run_id));
    }

    #[tokio::test]
    async fn tick_keeps_claim_only_active_fire_blocked() {
        let repo = Arc::new(InMemoryTriggerRepository::default());
        let trigger_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
        let fire_slot = ts(1_704_067_200);
        let mut record = sample_record(trigger_id, tenant("tenant-a"), fire_slot);
        record.active_fire_slot = Some(fire_slot);
        record.active_run_ref = None;
        repo.upsert_trigger(record).await.expect("insert active");
        let materializer = Arc::new(RecordingMaterializer::success("content:trigger-fire"));
        let submitter = Arc::new(RecordingSubmitter::with_outcomes(Vec::new()));
        let active_lookup = Arc::new(RecordingActiveRunLookup::with_state(
            TriggerActiveRunState::Terminal,
        ));
        let worker = worker(
            repo.clone(),
            materializer.clone(),
            submitter.clone(),
            active_lookup.clone(),
        );

        let report = worker.tick_once(fire_slot).await.expect("tick succeeds");

        assert!(matches!(
            report.results.first().map(|result| &result.outcome),
            Some(TriggerPollerFireOutcome::SkippedAlreadyActive {
                active_fire_slot: _,
                active_run_ref: None
            })
        ));
        assert_eq!(materializer.fires().len(), 0);
        assert_eq!(submitter.requests().len(), 0);
        assert_eq!(active_lookup.requests().len(), 0);
        let persisted = repo
            .get_trigger(tenant("tenant-a"), trigger_id)
            .await
            .expect("load")
            .expect("record present");
        assert_eq!(persisted.active_fire_slot, Some(fire_slot));
        assert_eq!(persisted.active_run_ref, None);
    }

    #[tokio::test]
    async fn tick_retryable_submit_failure_clears_active_and_keeps_slot_retryable() {
        let repo = Arc::new(InMemoryTriggerRepository::default());
        let trigger_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
        let fire_slot = ts(1_704_067_200);
        repo.upsert_trigger(sample_record(trigger_id, tenant("tenant-a"), fire_slot))
            .await
            .expect("insert");
        let worker = worker(
            repo.clone(),
            Arc::new(RecordingMaterializer::success("content:trigger-fire")),
            Arc::new(RecordingSubmitter::with_outcomes(vec![Ok(
                TrustedTriggerFireSubmitOutcome::RetryableFailed {
                    reason: TrustedTriggerSubmitFailureReason::Retryable,
                },
            )])),
            Arc::new(RecordingActiveRunLookup::default()),
        );

        let report = worker.tick_once(fire_slot).await.expect("tick succeeds");

        assert!(matches!(
            report.results.last().map(|result| &result.outcome),
            Some(TriggerPollerFireOutcome::RetryableFailed {
                reason: TriggerPollerFailureReason::TrustedSubmitRetryable,
            })
        ));
        let persisted = repo
            .get_trigger(tenant("tenant-a"), trigger_id)
            .await
            .expect("load")
            .expect("record present");
        assert_eq!(persisted.last_status, Some(TriggerRunStatus::Error));
        assert_eq!(persisted.next_run_at, fire_slot);
        assert_eq!(persisted.active_fire_slot, None);
        assert_eq!(persisted.active_run_ref, None);
    }

    #[tokio::test]
    async fn tick_accepted_mark_fire_missing_reports_due_failure() {
        let trigger_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
        let fire_slot = ts(1_704_067_200);
        let mut claimed_record = sample_record(trigger_id, tenant("tenant-a"), fire_slot);
        claimed_record.active_fire_slot = Some(fire_slot);
        let run_id = TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5a").expect("run id");
        let worker = worker(
            Arc::new(AcceptedMissingRepository {
                claimed_record,
                fire_slot,
            }),
            Arc::new(RecordingMaterializer::success("content:trigger-fire")),
            Arc::new(RecordingSubmitter::with_outcomes(vec![Ok(
                TrustedTriggerFireSubmitOutcome::Accepted {
                    run_id,
                    submitted_at: fire_slot,
                },
            )])),
            Arc::new(RecordingActiveRunLookup::default()),
        );

        let report = worker.tick_once(fire_slot).await.expect("tick succeeds");

        assert!(report.results.iter().any(|result| {
            result.trigger_id == trigger_id
                && matches!(
                    &result.outcome,
                    TriggerPollerFireOutcome::DueFireFailed { reason }
                        if *reason == TriggerPollerFailureReason::Backend
                )
        }));
    }

    #[tokio::test]
    async fn tick_replayed_mark_fire_missing_reports_due_failure() {
        let trigger_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
        let fire_slot = ts(1_704_067_200);
        let mut claimed_record = sample_record(trigger_id, tenant("tenant-a"), fire_slot);
        claimed_record.active_fire_slot = Some(fire_slot);
        let original_run_id =
            TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5a").expect("run id");
        let worker = worker(
            Arc::new(ReplayedMissingRepository {
                claimed_record,
                fire_slot,
            }),
            Arc::new(RecordingMaterializer::success("content:trigger-fire")),
            Arc::new(RecordingSubmitter::with_outcomes(vec![Ok(
                TrustedTriggerFireSubmitOutcome::Replayed {
                    original_run_id,
                    replayed_at: fire_slot,
                },
            )])),
            Arc::new(RecordingActiveRunLookup::default()),
        );

        let report = worker.tick_once(fire_slot).await.expect("tick succeeds");

        assert!(report.results.iter().any(|result| {
            result.trigger_id == trigger_id
                && matches!(
                    &result.outcome,
                    TriggerPollerFireOutcome::DueFireFailed { reason }
                        if *reason == TriggerPollerFailureReason::Backend
                )
        }));
    }

    #[tokio::test]
    async fn tick_fails_when_active_trigger_list_returns_backend_error() {
        let worker = worker(
            Arc::new(ActiveListErrorRepository),
            Arc::new(RecordingMaterializer::success("content:trigger-fire")),
            Arc::new(RecordingSubmitter::with_outcomes(Vec::new())),
            Arc::new(RecordingActiveRunLookup::default()),
        );

        let error = worker
            .tick_once(ts(1_704_067_200))
            .await
            .expect_err("active list failure should abort tick");

        assert!(matches!(error, TriggerError::Backend { .. }));
    }

    #[tokio::test]
    async fn tick_reports_due_record_error_and_continues_to_later_due_trigger() {
        let failed_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
        let success_id = TriggerId::parse("01J00000000000000000000000").expect("ulid");
        let fire_slot = ts(1_704_067_200);
        let mut failed = sample_record(failed_id, tenant("tenant-a"), fire_slot);
        failed.active_fire_slot = Some(fire_slot);
        let mut success = sample_record(success_id, tenant("tenant-b"), fire_slot);
        success.active_fire_slot = Some(fire_slot);
        let success_run_id =
            TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5a").expect("run id");
        let worker = worker(
            Arc::new(DueErrorThenSuccessRepository {
                failed_record: failed,
                success_record: success,
                fire_slot,
            }),
            Arc::new(RecordingMaterializer::success("content:trigger-fire")),
            Arc::new(RecordingSubmitter::with_outcomes(vec![Ok(
                TrustedTriggerFireSubmitOutcome::Accepted {
                    run_id: success_run_id,
                    submitted_at: fire_slot,
                },
            )])),
            Arc::new(RecordingActiveRunLookup::default()),
        );

        let report = worker.tick_once(fire_slot).await.expect("tick succeeds");

        assert_eq!(report.due_records, 2);
        assert!(
            report
                .results
                .iter()
                .any(|result| result.trigger_id == failed_id
                    && matches!(
                        result.outcome,
                        TriggerPollerFireOutcome::DueFireFailed {
                            reason: TriggerPollerFailureReason::Backend,
                        }
                    ))
        );
        assert!(
            report
                .results
                .iter()
                .any(|result| result.trigger_id == success_id
                    && result.outcome
                        == TriggerPollerFireOutcome::Submitted {
                            run_id: success_run_id
                        })
        );
    }

    #[tokio::test]
    async fn tick_submitter_backend_error_clears_active_and_keeps_slot_retryable() {
        let repo = Arc::new(InMemoryTriggerRepository::default());
        let trigger_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
        let fire_slot = ts(1_704_067_200);
        repo.upsert_trigger(sample_record(trigger_id, tenant("tenant-a"), fire_slot))
            .await
            .expect("insert");
        let worker = worker(
            repo.clone(),
            Arc::new(RecordingMaterializer::success("content:trigger-fire")),
            Arc::new(RecordingSubmitter::with_outcomes(vec![Err(
                TriggerError::Backend {
                    reason: "turn submit unavailable".to_string(),
                },
            )])),
            Arc::new(RecordingActiveRunLookup::default()),
        );

        let report = worker.tick_once(fire_slot).await.expect("tick succeeds");

        assert!(matches!(
            report.results.last().map(|result| &result.outcome),
            Some(TriggerPollerFireOutcome::RetryableFailed {
                reason: TriggerPollerFailureReason::Backend,
            })
        ));
        let persisted = repo
            .get_trigger(tenant("tenant-a"), trigger_id)
            .await
            .expect("load")
            .expect("record present");
        assert_eq!(persisted.last_status, Some(TriggerRunStatus::Error));
        assert_eq!(persisted.next_run_at, fire_slot);
        assert_eq!(persisted.active_fire_slot, None);
        assert_eq!(persisted.active_run_ref, None);
    }

    #[tokio::test]
    async fn tick_permanent_submit_failure_advances_next_slot() {
        let repo = Arc::new(InMemoryTriggerRepository::default());
        let trigger_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
        let fire_slot = ts(1_704_067_200);
        let record = sample_record(trigger_id, tenant("tenant-a"), fire_slot);
        let expected_next_run_at = record
            .schedule
            .next_slot_after(fire_slot)
            .expect("next run")
            .expect("future run");
        repo.upsert_trigger(record).await.expect("insert");
        let worker = worker(
            repo.clone(),
            Arc::new(RecordingMaterializer::success("content:trigger-fire")),
            Arc::new(RecordingSubmitter::with_outcomes(vec![Ok(
                TrustedTriggerFireSubmitOutcome::PermanentFailed {
                    reason: TrustedTriggerSubmitFailureReason::Permanent,
                },
            )])),
            Arc::new(RecordingActiveRunLookup::default()),
        );

        let report = worker.tick_once(fire_slot).await.expect("tick succeeds");

        assert!(matches!(
            report.results.last().map(|result| &result.outcome),
            Some(TriggerPollerFireOutcome::PermanentFailed {
                reason: TriggerPollerFailureReason::TrustedSubmitPermanent,
            })
        ));
        let persisted = repo
            .get_trigger(tenant("tenant-a"), trigger_id)
            .await
            .expect("load")
            .expect("record present");
        assert_eq!(persisted.last_status, Some(TriggerRunStatus::Error));
        assert_eq!(persisted.next_run_at, expected_next_run_at);
        assert_eq!(persisted.active_fire_slot, None);
        assert_eq!(persisted.active_run_ref, None);
    }

    #[tokio::test]
    async fn tick_permanent_materialization_failure_advances_next_slot() {
        let repo = Arc::new(InMemoryTriggerRepository::default());
        let trigger_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
        let fire_slot = ts(1_704_067_200);
        let record = sample_record(trigger_id, tenant("tenant-a"), fire_slot);
        let expected_next_run_at = record
            .schedule
            .next_slot_after(fire_slot)
            .expect("next run")
            .expect("future run");
        repo.upsert_trigger(record).await.expect("insert");
        let worker = worker(
            repo.clone(),
            Arc::new(RecordingMaterializer::failure(
                TriggerError::InvalidMaterialization {
                    reason: "bad prompt content ref".to_string(),
                },
            )),
            Arc::new(RecordingSubmitter::with_outcomes(Vec::new())),
            Arc::new(RecordingActiveRunLookup::default()),
        );

        let report = worker.tick_once(fire_slot).await.expect("tick succeeds");

        assert!(matches!(
            report.results.last().map(|result| &result.outcome),
            Some(TriggerPollerFireOutcome::PermanentFailed {
                reason: TriggerPollerFailureReason::InvalidMaterialization,
            })
        ));
        let persisted = repo
            .get_trigger(tenant("tenant-a"), trigger_id)
            .await
            .expect("load")
            .expect("record present");
        assert_eq!(persisted.last_status, Some(TriggerRunStatus::Error));
        assert_eq!(persisted.next_run_at, expected_next_run_at);
        assert_eq!(persisted.active_fire_slot, None);
        assert_eq!(persisted.active_run_ref, None);
    }

    #[tokio::test]
    async fn tick_source_provider_none_persists_permanent_failure_with_next_slot() {
        let repo = Arc::new(InMemoryTriggerRepository::default());
        let trigger_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
        let fire_slot = ts(1_704_067_200);
        let record = sample_record(trigger_id, tenant("tenant-a"), fire_slot);
        let expected_next_run_at = record
            .schedule
            .next_slot_after(fire_slot)
            .expect("next run")
            .expect("future run");
        repo.upsert_trigger(record).await.expect("insert");
        let worker = worker_with_source_provider(
            repo.clone(),
            Arc::new(NullSourceProvider),
            Arc::new(RecordingMaterializer::success("content:trigger-fire")),
            Arc::new(RecordingSubmitter::with_outcomes(Vec::new())),
            Arc::new(RecordingActiveRunLookup::default()),
        );

        let report = worker.tick_once(fire_slot).await.expect("tick succeeds");

        assert!(matches!(
            report.results.last().map(|result| &result.outcome),
            Some(TriggerPollerFireOutcome::PermanentFailed {
                reason: TriggerPollerFailureReason::SourceNoFire,
            })
        ));
        let persisted = repo
            .get_trigger(tenant("tenant-a"), trigger_id)
            .await
            .expect("load")
            .expect("record present");
        assert_eq!(persisted.last_status, Some(TriggerRunStatus::Error));
        assert_eq!(persisted.next_run_at, expected_next_run_at);
        assert_eq!(persisted.active_fire_slot, None);
        assert_eq!(persisted.active_run_ref, None);
    }

    #[tokio::test]
    async fn tick_source_provider_not_found_persists_permanent_failure_with_next_slot() {
        let repo = Arc::new(InMemoryTriggerRepository::default());
        let trigger_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
        let fire_slot = ts(1_704_067_200);
        let record = sample_record(trigger_id, tenant("tenant-a"), fire_slot);
        let expected_next_run_at = record
            .schedule
            .next_slot_after(fire_slot)
            .expect("next run")
            .expect("future run");
        repo.upsert_trigger(record).await.expect("insert");
        let worker = worker_with_source_provider(
            repo.clone(),
            Arc::new(NotFoundSourceProvider),
            Arc::new(RecordingMaterializer::success("content:trigger-fire")),
            Arc::new(RecordingSubmitter::with_outcomes(Vec::new())),
            Arc::new(RecordingActiveRunLookup::default()),
        );

        let report = worker.tick_once(fire_slot).await.expect("tick succeeds");

        assert!(matches!(
            report.results.last().map(|result| &result.outcome),
            Some(TriggerPollerFireOutcome::PermanentFailed {
                reason: TriggerPollerFailureReason::NotFound,
            })
        ));
        let persisted = repo
            .get_trigger(tenant("tenant-a"), trigger_id)
            .await
            .expect("load")
            .expect("record present");
        assert_eq!(persisted.last_status, Some(TriggerRunStatus::Error));
        assert_eq!(persisted.next_run_at, expected_next_run_at);
        assert_eq!(persisted.active_fire_slot, None);
        assert_eq!(persisted.active_run_ref, None);
    }

    #[tokio::test]
    async fn tick_source_provider_errors_report_bounded_permanent_reasons() {
        let cases = vec![
            (
                TriggerError::InvalidTriggerId {
                    reason: "bad trigger".to_string(),
                },
                TriggerPollerFailureReason::InvalidTriggerId,
            ),
            (
                TriggerError::InvalidFireIdentityComponent {
                    label: "fire_slot".to_string(),
                    reason: "bad component".to_string(),
                },
                TriggerPollerFailureReason::InvalidFireIdentityComponent,
            ),
            (
                TriggerError::InvalidRecord {
                    reason: "bad record".to_string(),
                },
                TriggerPollerFailureReason::InvalidRecord,
            ),
            (
                TriggerError::InvalidPollerConfig {
                    reason: "bad config".to_string(),
                },
                TriggerPollerFailureReason::InvalidPollerConfig,
            ),
        ];

        for (error, expected_reason) in cases {
            let repo = Arc::new(InMemoryTriggerRepository::default());
            let trigger_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
            let fire_slot = ts(1_704_067_200);
            repo.upsert_trigger(sample_record(trigger_id, tenant("tenant-a"), fire_slot))
                .await
                .expect("insert");
            let worker = worker_with_source_provider(
                repo,
                Arc::new(ErrorSourceProvider::new(error)),
                Arc::new(RecordingMaterializer::success("content:trigger-fire")),
                Arc::new(RecordingSubmitter::with_outcomes(Vec::new())),
                Arc::new(RecordingActiveRunLookup::default()),
            );

            let report = worker.tick_once(fire_slot).await.expect("tick succeeds");

            assert!(matches!(
                report.results.last().map(|result| &result.outcome),
                Some(TriggerPollerFireOutcome::PermanentFailed { reason })
                    if *reason == expected_reason
            ));
        }
    }

    #[tokio::test]
    async fn tick_permanent_failure_without_next_slot_completes_trigger() {
        let repo = Arc::new(InMemoryTriggerRepository::default());
        let trigger_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
        let fire_slot = ymd_hms(9999, 12, 31, 8, 0, 0);
        repo.upsert_trigger(sample_record(trigger_id, tenant("tenant-a"), fire_slot))
            .await
            .expect("insert");
        let worker = worker(
            repo.clone(),
            Arc::new(RecordingMaterializer::success("content:trigger-fire")),
            Arc::new(RecordingSubmitter::with_outcomes(Vec::new())),
            Arc::new(RecordingActiveRunLookup::default()),
        );

        let report = worker.tick_once(fire_slot).await.expect("tick succeeds");

        assert!(matches!(
            report.results.last().map(|result| &result.outcome),
            Some(TriggerPollerFireOutcome::PermanentFailed {
                reason: TriggerPollerFailureReason::InvalidSchedule,
            })
        ));
        let persisted = repo
            .get_trigger(tenant("tenant-a"), trigger_id)
            .await
            .expect("load")
            .expect("record present");
        assert_eq!(persisted.state, TriggerState::Completed);
        assert_eq!(persisted.last_status, Some(TriggerRunStatus::Error));
        assert_eq!(persisted.active_fire_slot, None);
        assert_eq!(persisted.active_run_ref, None);
    }

    struct RecordingMaterializer {
        result: Mutex<Option<Result<TriggerInboundContentRef, TriggerError>>>,
        fires: Mutex<Vec<TriggerFire>>,
    }

    struct NullSourceProvider;

    struct NotFoundSourceProvider;

    struct ErrorSourceProvider {
        error: Mutex<Option<TriggerError>>,
    }

    impl ErrorSourceProvider {
        fn new(error: TriggerError) -> Self {
            Self {
                error: Mutex::new(Some(error)),
            }
        }
    }

    #[async_trait]
    impl TriggerSourceProvider for NullSourceProvider {
        async fn evaluate(
            &self,
            _record: &TriggerRecord,
            _now: Timestamp,
        ) -> Result<Option<TriggerFire>, TriggerError> {
            Ok(None)
        }
    }

    #[async_trait]
    impl TriggerSourceProvider for NotFoundSourceProvider {
        async fn evaluate(
            &self,
            _record: &TriggerRecord,
            _now: Timestamp,
        ) -> Result<Option<TriggerFire>, TriggerError> {
            Err(TriggerError::NotFound)
        }
    }

    #[async_trait]
    impl TriggerSourceProvider for ErrorSourceProvider {
        async fn evaluate(
            &self,
            _record: &TriggerRecord,
            _now: Timestamp,
        ) -> Result<Option<TriggerFire>, TriggerError> {
            Err(self
                .error
                .lock()
                .expect("error lock")
                .take()
                .expect("source provider error configured"))
        }
    }

    impl RecordingMaterializer {
        fn success(content_ref: &str) -> Self {
            Self {
                result: Mutex::new(Some(
                    Ok(TriggerInboundContentRef::new(content_ref).unwrap()),
                )),
                fires: Mutex::new(Vec::new()),
            }
        }

        fn failure(error: TriggerError) -> Self {
            Self {
                result: Mutex::new(Some(Err(error))),
                fires: Mutex::new(Vec::new()),
            }
        }

        fn fires(&self) -> Vec<TriggerFire> {
            self.fires.lock().expect("fires lock").clone()
        }
    }

    #[async_trait]
    impl TriggerPromptMaterializer for RecordingMaterializer {
        async fn materialize_prompt(
            &self,
            fire: TriggerFire,
        ) -> Result<TriggerInboundContentRef, TriggerError> {
            self.fires.lock().expect("fires lock").push(fire);
            self.result
                .lock()
                .expect("result lock")
                .take()
                .expect("materializer result configured")
        }
    }

    struct RecordingSubmitter {
        outcomes: Mutex<Vec<Result<TrustedTriggerFireSubmitOutcome, TriggerError>>>,
        requests: Mutex<Vec<TrustedTriggerSubmitRequest>>,
    }

    impl RecordingSubmitter {
        fn with_outcomes(
            outcomes: Vec<Result<TrustedTriggerFireSubmitOutcome, TriggerError>>,
        ) -> Self {
            Self {
                outcomes: Mutex::new(outcomes.into_iter().rev().collect()),
                requests: Mutex::new(Vec::new()),
            }
        }

        fn requests(&self) -> Vec<TrustedTriggerSubmitRequest> {
            self.requests.lock().expect("requests lock").clone()
        }
    }

    #[async_trait]
    impl TrustedTriggerFireSubmitter for RecordingSubmitter {
        async fn submit_trusted_trigger_fire(
            &self,
            request: TrustedTriggerSubmitRequest,
        ) -> Result<TrustedTriggerFireSubmitOutcome, TriggerError> {
            self.requests.lock().expect("requests lock").push(request);
            self.outcomes
                .lock()
                .expect("outcomes lock")
                .pop()
                .expect("submit outcome configured")
        }
    }

    #[derive(Default)]
    struct RecordingActiveRunLookup {
        results: Mutex<Vec<Result<TriggerActiveRunState, TriggerError>>>,
        requests: Mutex<Vec<TriggerActiveRunStateRequest>>,
    }

    impl RecordingActiveRunLookup {
        fn with_state(state: TriggerActiveRunState) -> Self {
            Self::with_results(vec![Ok(state)])
        }

        fn with_results(results: Vec<Result<TriggerActiveRunState, TriggerError>>) -> Self {
            Self {
                results: Mutex::new(results.into_iter().rev().collect()),
                requests: Mutex::new(Vec::new()),
            }
        }

        fn requests(&self) -> Vec<TriggerActiveRunStateRequest> {
            self.requests.lock().expect("requests lock").clone()
        }
    }

    #[async_trait]
    impl TriggerActiveRunLookup for RecordingActiveRunLookup {
        async fn active_run_state(
            &self,
            request: TriggerActiveRunStateRequest,
        ) -> Result<TriggerActiveRunState, TriggerError> {
            self.requests.lock().expect("requests lock").push(request);
            self.results.lock().expect("results lock").pop().expect(
                "RecordingActiveRunLookup: more active_run_state calls than configured outcomes",
            )
        }
    }

    struct ActiveListErrorRepository;

    #[async_trait]
    impl TriggerRepository for ActiveListErrorRepository {
        async fn upsert_trigger(&self, _record: TriggerRecord) -> Result<(), TriggerError> {
            unreachable!("active-list-error repository is read-only")
        }

        async fn get_trigger(
            &self,
            _tenant_id: TenantId,
            _trigger_id: TriggerId,
        ) -> Result<Option<TriggerRecord>, TriggerError> {
            unreachable!("active-list-error repository does not load records")
        }

        async fn list_triggers(
            &self,
            _tenant_id: TenantId,
        ) -> Result<Vec<TriggerRecord>, TriggerError> {
            unreachable!("active-list-error repository does not list tenant records")
        }

        async fn remove_trigger(
            &self,
            _tenant_id: TenantId,
            _trigger_id: TriggerId,
        ) -> Result<Option<TriggerRecord>, TriggerError> {
            unreachable!("active-list-error repository does not remove records")
        }

        async fn list_due_triggers(
            &self,
            _now: Timestamp,
            _limit: usize,
        ) -> Result<Vec<TriggerRecord>, TriggerError> {
            unreachable!("active-list-error should abort before due scan")
        }

        async fn list_active_triggers(
            &self,
            limit: usize,
        ) -> Result<Vec<TriggerRecord>, TriggerError> {
            self.list_active_triggers_after(None, limit).await
        }

        async fn list_active_triggers_after(
            &self,
            _after: Option<ActiveTriggerScanCursor>,
            _limit: usize,
        ) -> Result<Vec<TriggerRecord>, TriggerError> {
            Err(TriggerError::Backend {
                reason: "active list unavailable".to_string(),
            })
        }

        async fn claim_due_fire(
            &self,
            _request: ClaimDueFireRequest,
        ) -> Result<ClaimDueFireOutcome, TriggerError> {
            unreachable!("active-list-error repository should not claim fires")
        }

        async fn mark_fire_accepted(
            &self,
            _request: FireAcceptedRequest,
        ) -> Result<Option<TriggerRecord>, TriggerError> {
            unreachable!("active-list-error repository should not persist accepted fires")
        }

        async fn mark_fire_replayed(
            &self,
            _request: FireReplayedRequest,
        ) -> Result<Option<TriggerRecord>, TriggerError> {
            unreachable!("active-list-error repository should not persist replayed fires")
        }

        async fn mark_fire_retryable_failed(
            &self,
            _request: FireRetryableFailedRequest,
        ) -> Result<Option<TriggerRecord>, TriggerError> {
            unreachable!("active-list-error repository should not persist retryable failures")
        }

        async fn mark_fire_permanently_failed(
            &self,
            _request: FirePermanentFailedRequest,
        ) -> Result<Option<TriggerRecord>, TriggerError> {
            unreachable!("active-list-error repository should not persist permanent failures")
        }

        async fn mark_fire_terminally_failed(
            &self,
            _request: FireTerminalFailedRequest,
        ) -> Result<Option<TriggerRecord>, TriggerError> {
            unreachable!("active-list-error repository should not persist terminal failures")
        }

        async fn clear_active_fire(
            &self,
            _request: ClearActiveFireRequest,
        ) -> Result<Option<TriggerRecord>, TriggerError> {
            unreachable!("active-list-error repository should not clear active fires")
        }
    }

    struct ActiveWrapRefetchErrorRepository {
        record: TriggerRecord,
        active_scan_calls: Mutex<Vec<bool>>,
    }

    impl ActiveWrapRefetchErrorRepository {
        fn new(record: TriggerRecord) -> Self {
            Self {
                record,
                active_scan_calls: Mutex::new(Vec::new()),
            }
        }

        fn active_scan_call_shapes(&self) -> Vec<bool> {
            self.active_scan_calls
                .lock()
                .expect("active scan calls lock")
                .clone()
        }
    }

    #[async_trait]
    impl TriggerRepository for ActiveWrapRefetchErrorRepository {
        async fn upsert_trigger(&self, _record: TriggerRecord) -> Result<(), TriggerError> {
            unreachable!("active-wrap-refetch-error repository is read-only")
        }

        async fn get_trigger(
            &self,
            _tenant_id: TenantId,
            _trigger_id: TriggerId,
        ) -> Result<Option<TriggerRecord>, TriggerError> {
            unreachable!("active-wrap-refetch-error repository does not load records")
        }

        async fn list_triggers(
            &self,
            _tenant_id: TenantId,
        ) -> Result<Vec<TriggerRecord>, TriggerError> {
            unreachable!("active-wrap-refetch-error repository does not list tenant records")
        }

        async fn remove_trigger(
            &self,
            _tenant_id: TenantId,
            _trigger_id: TriggerId,
        ) -> Result<Option<TriggerRecord>, TriggerError> {
            unreachable!("active-wrap-refetch-error repository does not remove records")
        }

        async fn list_due_triggers(
            &self,
            _now: Timestamp,
            _limit: usize,
        ) -> Result<Vec<TriggerRecord>, TriggerError> {
            Ok(Vec::new())
        }

        async fn list_active_triggers(
            &self,
            limit: usize,
        ) -> Result<Vec<TriggerRecord>, TriggerError> {
            self.list_active_triggers_after(None, limit).await
        }

        async fn list_active_triggers_after(
            &self,
            after: Option<ActiveTriggerScanCursor>,
            _limit: usize,
        ) -> Result<Vec<TriggerRecord>, TriggerError> {
            let mut calls = self
                .active_scan_calls
                .lock()
                .expect("active scan calls lock");
            calls.push(after.is_some());
            match calls.len() {
                1 => Ok(vec![self.record.clone()]),
                2 => Ok(Vec::new()),
                3 => Err(TriggerError::Backend {
                    reason: "wrap refetch unavailable".to_string(),
                }),
                _ => unreachable!("unexpected active scan call"),
            }
        }

        async fn claim_due_fire(
            &self,
            _request: ClaimDueFireRequest,
        ) -> Result<ClaimDueFireOutcome, TriggerError> {
            unreachable!("active-wrap-refetch-error repository should not claim fires")
        }

        async fn mark_fire_accepted(
            &self,
            _request: FireAcceptedRequest,
        ) -> Result<Option<TriggerRecord>, TriggerError> {
            unreachable!("active-wrap-refetch-error repository should not persist accepted fires")
        }

        async fn mark_fire_replayed(
            &self,
            _request: FireReplayedRequest,
        ) -> Result<Option<TriggerRecord>, TriggerError> {
            unreachable!("active-wrap-refetch-error repository should not persist replayed fires")
        }

        async fn mark_fire_retryable_failed(
            &self,
            _request: FireRetryableFailedRequest,
        ) -> Result<Option<TriggerRecord>, TriggerError> {
            unreachable!(
                "active-wrap-refetch-error repository should not persist retryable failures"
            )
        }

        async fn mark_fire_permanently_failed(
            &self,
            _request: FirePermanentFailedRequest,
        ) -> Result<Option<TriggerRecord>, TriggerError> {
            unreachable!(
                "active-wrap-refetch-error repository should not persist permanent failures"
            )
        }

        async fn mark_fire_terminally_failed(
            &self,
            _request: FireTerminalFailedRequest,
        ) -> Result<Option<TriggerRecord>, TriggerError> {
            unreachable!(
                "active-wrap-refetch-error repository should not persist terminal failures"
            )
        }

        async fn clear_active_fire(
            &self,
            _request: ClearActiveFireRequest,
        ) -> Result<Option<TriggerRecord>, TriggerError> {
            unreachable!("active-wrap-refetch-error repository should not clear active fires")
        }
    }

    struct ActiveClearRaceRepository {
        active_record: TriggerRecord,
    }

    #[async_trait]
    impl TriggerRepository for ActiveClearRaceRepository {
        async fn upsert_trigger(&self, _record: TriggerRecord) -> Result<(), TriggerError> {
            unreachable!("active-clear-race repository is read-only")
        }

        async fn get_trigger(
            &self,
            _tenant_id: TenantId,
            _trigger_id: TriggerId,
        ) -> Result<Option<TriggerRecord>, TriggerError> {
            unreachable!("active-clear-race repository does not load records")
        }

        async fn list_triggers(
            &self,
            _tenant_id: TenantId,
        ) -> Result<Vec<TriggerRecord>, TriggerError> {
            unreachable!("active-clear-race repository does not list tenant records")
        }

        async fn remove_trigger(
            &self,
            _tenant_id: TenantId,
            _trigger_id: TriggerId,
        ) -> Result<Option<TriggerRecord>, TriggerError> {
            unreachable!("active-clear-race repository does not remove records")
        }

        async fn list_due_triggers(
            &self,
            _now: Timestamp,
            _limit: usize,
        ) -> Result<Vec<TriggerRecord>, TriggerError> {
            Ok(Vec::new())
        }

        async fn list_active_triggers(
            &self,
            limit: usize,
        ) -> Result<Vec<TriggerRecord>, TriggerError> {
            self.list_active_triggers_after(None, limit).await
        }

        async fn list_active_triggers_after(
            &self,
            after: Option<ActiveTriggerScanCursor>,
            _limit: usize,
        ) -> Result<Vec<TriggerRecord>, TriggerError> {
            if after.is_some() {
                return Ok(Vec::new());
            }
            Ok(vec![self.active_record.clone()])
        }

        async fn claim_due_fire(
            &self,
            _request: ClaimDueFireRequest,
        ) -> Result<ClaimDueFireOutcome, TriggerError> {
            unreachable!("active-clear-race repository should not claim fires")
        }

        async fn mark_fire_accepted(
            &self,
            _request: FireAcceptedRequest,
        ) -> Result<Option<TriggerRecord>, TriggerError> {
            unreachable!("active-clear-race repository should not persist accepted fires")
        }

        async fn mark_fire_replayed(
            &self,
            _request: FireReplayedRequest,
        ) -> Result<Option<TriggerRecord>, TriggerError> {
            unreachable!("active-clear-race repository should not persist replayed fires")
        }

        async fn mark_fire_retryable_failed(
            &self,
            _request: FireRetryableFailedRequest,
        ) -> Result<Option<TriggerRecord>, TriggerError> {
            unreachable!("active-clear-race repository should not persist retryable failures")
        }

        async fn mark_fire_permanently_failed(
            &self,
            _request: FirePermanentFailedRequest,
        ) -> Result<Option<TriggerRecord>, TriggerError> {
            unreachable!("active-clear-race repository should not persist permanent failures")
        }

        async fn mark_fire_terminally_failed(
            &self,
            _request: FireTerminalFailedRequest,
        ) -> Result<Option<TriggerRecord>, TriggerError> {
            unreachable!("active-clear-race repository should not persist terminal failures")
        }

        async fn clear_active_fire(
            &self,
            _request: ClearActiveFireRequest,
        ) -> Result<Option<TriggerRecord>, TriggerError> {
            Ok(None)
        }
    }

    struct ActiveClearFailsOnceRepository {
        records: Mutex<Vec<TriggerRecord>>,
        clear_requests: Mutex<Vec<TriggerId>>,
        fail_once_trigger_id: TriggerId,
        failed_once: Mutex<bool>,
    }

    impl ActiveClearFailsOnceRepository {
        fn new(records: Vec<TriggerRecord>, fail_once_trigger_id: TriggerId) -> Self {
            Self {
                records: Mutex::new(records),
                clear_requests: Mutex::new(Vec::new()),
                fail_once_trigger_id,
                failed_once: Mutex::new(false),
            }
        }

        fn clear_requests(&self) -> Vec<TriggerId> {
            self.clear_requests
                .lock()
                .expect("clear requests lock")
                .clone()
        }
    }

    #[async_trait]
    impl TriggerRepository for ActiveClearFailsOnceRepository {
        async fn upsert_trigger(&self, _record: TriggerRecord) -> Result<(), TriggerError> {
            unreachable!("active-clear-fails-once repository is read-only")
        }

        async fn get_trigger(
            &self,
            _tenant_id: TenantId,
            _trigger_id: TriggerId,
        ) -> Result<Option<TriggerRecord>, TriggerError> {
            unreachable!("active-clear-fails-once repository does not load records")
        }

        async fn list_triggers(
            &self,
            _tenant_id: TenantId,
        ) -> Result<Vec<TriggerRecord>, TriggerError> {
            unreachable!("active-clear-fails-once repository does not list tenant records")
        }

        async fn remove_trigger(
            &self,
            _tenant_id: TenantId,
            _trigger_id: TriggerId,
        ) -> Result<Option<TriggerRecord>, TriggerError> {
            unreachable!("active-clear-fails-once repository does not remove records")
        }

        async fn list_due_triggers(
            &self,
            _now: Timestamp,
            _limit: usize,
        ) -> Result<Vec<TriggerRecord>, TriggerError> {
            Ok(Vec::new())
        }

        async fn list_active_triggers(
            &self,
            limit: usize,
        ) -> Result<Vec<TriggerRecord>, TriggerError> {
            self.list_active_triggers_after(None, limit).await
        }

        async fn list_active_triggers_after(
            &self,
            after: Option<ActiveTriggerScanCursor>,
            limit: usize,
        ) -> Result<Vec<TriggerRecord>, TriggerError> {
            let mut records = self
                .records
                .lock()
                .expect("active records lock")
                .iter()
                .filter_map(|record| {
                    let active_fire_slot = record.active_fire_slot?;
                    Some((
                        active_fire_slot,
                        record.tenant_id.clone(),
                        record.trigger_id,
                        record.clone(),
                    ))
                })
                .filter(
                    |(active_fire_slot, tenant_id, trigger_id, _)| match after.as_ref() {
                        Some(cursor) => {
                            (*active_fire_slot, tenant_id, *trigger_id)
                                > (
                                    cursor.active_fire_slot(),
                                    cursor.tenant_id(),
                                    cursor.trigger_id(),
                                )
                        }
                        None => true,
                    },
                )
                .collect::<Vec<_>>();
            records.sort_by_key(|(active_fire_slot, tenant_id, trigger_id, _)| {
                (*active_fire_slot, tenant_id.clone(), *trigger_id)
            });
            records.truncate(limit);
            Ok(records
                .into_iter()
                .map(|(_, _, _, record)| record)
                .collect())
        }

        async fn claim_due_fire(
            &self,
            _request: ClaimDueFireRequest,
        ) -> Result<ClaimDueFireOutcome, TriggerError> {
            unreachable!("active-clear-fails-once repository should not claim fires")
        }

        async fn mark_fire_accepted(
            &self,
            _request: FireAcceptedRequest,
        ) -> Result<Option<TriggerRecord>, TriggerError> {
            unreachable!("active-clear-fails-once repository should not persist accepted fires")
        }

        async fn mark_fire_replayed(
            &self,
            _request: FireReplayedRequest,
        ) -> Result<Option<TriggerRecord>, TriggerError> {
            unreachable!("active-clear-fails-once repository should not persist replayed fires")
        }

        async fn mark_fire_retryable_failed(
            &self,
            _request: FireRetryableFailedRequest,
        ) -> Result<Option<TriggerRecord>, TriggerError> {
            unreachable!("active-clear-fails-once repository should not persist retryable failures")
        }

        async fn mark_fire_permanently_failed(
            &self,
            _request: FirePermanentFailedRequest,
        ) -> Result<Option<TriggerRecord>, TriggerError> {
            unreachable!("active-clear-fails-once repository should not persist permanent failures")
        }

        async fn mark_fire_terminally_failed(
            &self,
            _request: FireTerminalFailedRequest,
        ) -> Result<Option<TriggerRecord>, TriggerError> {
            unreachable!("active-clear-fails-once repository should not persist terminal failures")
        }

        async fn clear_active_fire(
            &self,
            request: ClearActiveFireRequest,
        ) -> Result<Option<TriggerRecord>, TriggerError> {
            self.clear_requests
                .lock()
                .expect("clear requests lock")
                .push(request.trigger_id);
            if request.trigger_id == self.fail_once_trigger_id {
                let mut failed_once = self.failed_once.lock().expect("failed-once lock");
                if !*failed_once {
                    *failed_once = true;
                    return Err(TriggerError::Backend {
                        reason: "clear failed once".to_string(),
                    });
                }
            }

            let mut records = self.records.lock().expect("active records lock");
            let Some(record) = records.iter_mut().find(|record| {
                record.tenant_id == request.tenant_id && record.trigger_id == request.trigger_id
            }) else {
                return Ok(None);
            };
            let updated = record.clone();
            record.active_fire_slot = None;
            record.active_run_ref = None;
            Ok(Some(updated))
        }
    }

    struct AcceptedMissingRepository {
        claimed_record: TriggerRecord,
        fire_slot: Timestamp,
    }

    #[async_trait]
    impl TriggerRepository for AcceptedMissingRepository {
        async fn upsert_trigger(&self, _record: TriggerRecord) -> Result<(), TriggerError> {
            unreachable!("accepted-missing repository is read-only")
        }

        async fn get_trigger(
            &self,
            _tenant_id: TenantId,
            _trigger_id: TriggerId,
        ) -> Result<Option<TriggerRecord>, TriggerError> {
            unreachable!("accepted-missing repository does not load records")
        }

        async fn list_triggers(
            &self,
            _tenant_id: TenantId,
        ) -> Result<Vec<TriggerRecord>, TriggerError> {
            unreachable!("accepted-missing repository does not list tenant records")
        }

        async fn remove_trigger(
            &self,
            _tenant_id: TenantId,
            _trigger_id: TriggerId,
        ) -> Result<Option<TriggerRecord>, TriggerError> {
            unreachable!("accepted-missing repository does not remove records")
        }

        async fn list_due_triggers(
            &self,
            _now: Timestamp,
            _limit: usize,
        ) -> Result<Vec<TriggerRecord>, TriggerError> {
            Ok(vec![self.claimed_record.clone()])
        }

        async fn list_active_triggers(
            &self,
            limit: usize,
        ) -> Result<Vec<TriggerRecord>, TriggerError> {
            self.list_active_triggers_after(None, limit).await
        }

        async fn list_active_triggers_after(
            &self,
            _after: Option<ActiveTriggerScanCursor>,
            _limit: usize,
        ) -> Result<Vec<TriggerRecord>, TriggerError> {
            Ok(Vec::new())
        }

        async fn claim_due_fire(
            &self,
            _request: ClaimDueFireRequest,
        ) -> Result<ClaimDueFireOutcome, TriggerError> {
            Ok(ClaimDueFireOutcome::Claimed(ClaimedTriggerFire {
                record: self.claimed_record.clone(),
                fire_slot: self.fire_slot,
            }))
        }

        async fn mark_fire_accepted(
            &self,
            _request: FireAcceptedRequest,
        ) -> Result<Option<TriggerRecord>, TriggerError> {
            Ok(None)
        }

        async fn mark_fire_replayed(
            &self,
            _request: FireReplayedRequest,
        ) -> Result<Option<TriggerRecord>, TriggerError> {
            unreachable!("accepted-missing repository should not persist replayed fires")
        }

        async fn mark_fire_retryable_failed(
            &self,
            _request: FireRetryableFailedRequest,
        ) -> Result<Option<TriggerRecord>, TriggerError> {
            unreachable!("accepted-missing repository should not persist retryable failures")
        }

        async fn mark_fire_permanently_failed(
            &self,
            _request: FirePermanentFailedRequest,
        ) -> Result<Option<TriggerRecord>, TriggerError> {
            unreachable!("accepted-missing repository should not persist permanent failures")
        }

        async fn mark_fire_terminally_failed(
            &self,
            _request: FireTerminalFailedRequest,
        ) -> Result<Option<TriggerRecord>, TriggerError> {
            unreachable!("accepted-missing repository should not persist terminal failures")
        }

        async fn clear_active_fire(
            &self,
            _request: ClearActiveFireRequest,
        ) -> Result<Option<TriggerRecord>, TriggerError> {
            unreachable!("accepted-missing repository should not clear active fires")
        }
    }

    struct ReplayedMissingRepository {
        claimed_record: TriggerRecord,
        fire_slot: Timestamp,
    }

    #[async_trait]
    impl TriggerRepository for ReplayedMissingRepository {
        async fn upsert_trigger(&self, _record: TriggerRecord) -> Result<(), TriggerError> {
            unreachable!("replayed-missing repository is read-only")
        }

        async fn get_trigger(
            &self,
            _tenant_id: TenantId,
            _trigger_id: TriggerId,
        ) -> Result<Option<TriggerRecord>, TriggerError> {
            unreachable!("replayed-missing repository does not load records")
        }

        async fn list_triggers(
            &self,
            _tenant_id: TenantId,
        ) -> Result<Vec<TriggerRecord>, TriggerError> {
            unreachable!("replayed-missing repository does not list tenant records")
        }

        async fn remove_trigger(
            &self,
            _tenant_id: TenantId,
            _trigger_id: TriggerId,
        ) -> Result<Option<TriggerRecord>, TriggerError> {
            unreachable!("replayed-missing repository does not remove records")
        }

        async fn list_due_triggers(
            &self,
            _now: Timestamp,
            _limit: usize,
        ) -> Result<Vec<TriggerRecord>, TriggerError> {
            Ok(vec![self.claimed_record.clone()])
        }

        async fn list_active_triggers(
            &self,
            limit: usize,
        ) -> Result<Vec<TriggerRecord>, TriggerError> {
            self.list_active_triggers_after(None, limit).await
        }

        async fn list_active_triggers_after(
            &self,
            _after: Option<ActiveTriggerScanCursor>,
            _limit: usize,
        ) -> Result<Vec<TriggerRecord>, TriggerError> {
            Ok(Vec::new())
        }

        async fn claim_due_fire(
            &self,
            _request: ClaimDueFireRequest,
        ) -> Result<ClaimDueFireOutcome, TriggerError> {
            Ok(ClaimDueFireOutcome::Claimed(ClaimedTriggerFire {
                record: self.claimed_record.clone(),
                fire_slot: self.fire_slot,
            }))
        }

        async fn mark_fire_accepted(
            &self,
            _request: FireAcceptedRequest,
        ) -> Result<Option<TriggerRecord>, TriggerError> {
            unreachable!("replayed-missing repository should not persist accepted fires")
        }

        async fn mark_fire_replayed(
            &self,
            _request: FireReplayedRequest,
        ) -> Result<Option<TriggerRecord>, TriggerError> {
            Ok(None)
        }

        async fn mark_fire_retryable_failed(
            &self,
            _request: FireRetryableFailedRequest,
        ) -> Result<Option<TriggerRecord>, TriggerError> {
            unreachable!("replayed-missing repository should not persist retryable failures")
        }

        async fn mark_fire_permanently_failed(
            &self,
            _request: FirePermanentFailedRequest,
        ) -> Result<Option<TriggerRecord>, TriggerError> {
            unreachable!("replayed-missing repository should not persist permanent failures")
        }

        async fn mark_fire_terminally_failed(
            &self,
            _request: FireTerminalFailedRequest,
        ) -> Result<Option<TriggerRecord>, TriggerError> {
            unreachable!("replayed-missing repository should not persist terminal failures")
        }

        async fn clear_active_fire(
            &self,
            _request: ClearActiveFireRequest,
        ) -> Result<Option<TriggerRecord>, TriggerError> {
            unreachable!("replayed-missing repository should not clear active fires")
        }
    }

    struct DueErrorThenSuccessRepository {
        failed_record: TriggerRecord,
        success_record: TriggerRecord,
        fire_slot: Timestamp,
    }

    #[async_trait]
    impl TriggerRepository for DueErrorThenSuccessRepository {
        async fn upsert_trigger(&self, _record: TriggerRecord) -> Result<(), TriggerError> {
            unreachable!("due-error repository is read-only")
        }

        async fn get_trigger(
            &self,
            _tenant_id: TenantId,
            _trigger_id: TriggerId,
        ) -> Result<Option<TriggerRecord>, TriggerError> {
            unreachable!("due-error repository does not load records")
        }

        async fn list_triggers(
            &self,
            _tenant_id: TenantId,
        ) -> Result<Vec<TriggerRecord>, TriggerError> {
            unreachable!("due-error repository does not list tenant records")
        }

        async fn remove_trigger(
            &self,
            _tenant_id: TenantId,
            _trigger_id: TriggerId,
        ) -> Result<Option<TriggerRecord>, TriggerError> {
            unreachable!("due-error repository does not remove records")
        }

        async fn list_due_triggers(
            &self,
            _now: Timestamp,
            _limit: usize,
        ) -> Result<Vec<TriggerRecord>, TriggerError> {
            Ok(vec![
                self.failed_record.clone(),
                self.success_record.clone(),
            ])
        }

        async fn list_active_triggers(
            &self,
            limit: usize,
        ) -> Result<Vec<TriggerRecord>, TriggerError> {
            self.list_active_triggers_after(None, limit).await
        }

        async fn list_active_triggers_after(
            &self,
            _after: Option<ActiveTriggerScanCursor>,
            _limit: usize,
        ) -> Result<Vec<TriggerRecord>, TriggerError> {
            Ok(Vec::new())
        }

        async fn claim_due_fire(
            &self,
            request: ClaimDueFireRequest,
        ) -> Result<ClaimDueFireOutcome, TriggerError> {
            if request.trigger_id == self.failed_record.trigger_id {
                return Err(TriggerError::Backend {
                    reason: "claim failed".to_string(),
                });
            }
            Ok(ClaimDueFireOutcome::Claimed(ClaimedTriggerFire {
                record: self.success_record.clone(),
                fire_slot: self.fire_slot,
            }))
        }

        async fn mark_fire_accepted(
            &self,
            _request: FireAcceptedRequest,
        ) -> Result<Option<TriggerRecord>, TriggerError> {
            Ok(Some(self.success_record.clone()))
        }

        async fn mark_fire_replayed(
            &self,
            _request: FireReplayedRequest,
        ) -> Result<Option<TriggerRecord>, TriggerError> {
            unreachable!("due-error repository should not persist replayed fires")
        }

        async fn mark_fire_retryable_failed(
            &self,
            _request: FireRetryableFailedRequest,
        ) -> Result<Option<TriggerRecord>, TriggerError> {
            unreachable!("due-error repository should not persist retryable failures")
        }

        async fn mark_fire_permanently_failed(
            &self,
            _request: FirePermanentFailedRequest,
        ) -> Result<Option<TriggerRecord>, TriggerError> {
            unreachable!("due-error repository should not persist permanent failures")
        }

        async fn mark_fire_terminally_failed(
            &self,
            _request: FireTerminalFailedRequest,
        ) -> Result<Option<TriggerRecord>, TriggerError> {
            unreachable!("due-error repository should not persist terminal failures")
        }

        async fn clear_active_fire(
            &self,
            _request: ClearActiveFireRequest,
        ) -> Result<Option<TriggerRecord>, TriggerError> {
            unreachable!("due-error repository should not clear active fires")
        }
    }

    struct ClaimRaceRepository {
        due_record: TriggerRecord,
        claim_outcome: Mutex<Option<ClaimDueFireOutcome>>,
    }

    impl ClaimRaceRepository {
        fn new(due_record: TriggerRecord, claim_outcome: ClaimDueFireOutcome) -> Self {
            Self {
                due_record,
                claim_outcome: Mutex::new(Some(claim_outcome)),
            }
        }
    }

    #[async_trait]
    impl TriggerRepository for ClaimRaceRepository {
        async fn upsert_trigger(&self, _record: TriggerRecord) -> Result<(), TriggerError> {
            unreachable!("claim-race repository is read-only")
        }

        async fn get_trigger(
            &self,
            _tenant_id: TenantId,
            _trigger_id: TriggerId,
        ) -> Result<Option<TriggerRecord>, TriggerError> {
            unreachable!("claim-race repository does not load records")
        }

        async fn list_triggers(
            &self,
            _tenant_id: TenantId,
        ) -> Result<Vec<TriggerRecord>, TriggerError> {
            unreachable!("claim-race repository does not list tenant records")
        }

        async fn remove_trigger(
            &self,
            _tenant_id: TenantId,
            _trigger_id: TriggerId,
        ) -> Result<Option<TriggerRecord>, TriggerError> {
            unreachable!("claim-race repository does not remove records")
        }

        async fn list_due_triggers(
            &self,
            _now: Timestamp,
            _limit: usize,
        ) -> Result<Vec<TriggerRecord>, TriggerError> {
            Ok(vec![self.due_record.clone()])
        }

        async fn list_active_triggers(
            &self,
            limit: usize,
        ) -> Result<Vec<TriggerRecord>, TriggerError> {
            self.list_active_triggers_after(None, limit).await
        }

        async fn list_active_triggers_after(
            &self,
            _after: Option<ActiveTriggerScanCursor>,
            _limit: usize,
        ) -> Result<Vec<TriggerRecord>, TriggerError> {
            Ok(Vec::new())
        }

        async fn claim_due_fire(
            &self,
            _request: ClaimDueFireRequest,
        ) -> Result<ClaimDueFireOutcome, TriggerError> {
            Ok(self
                .claim_outcome
                .lock()
                .expect("claim outcome lock")
                .take()
                .expect("claim outcome configured"))
        }

        async fn mark_fire_accepted(
            &self,
            _request: FireAcceptedRequest,
        ) -> Result<Option<TriggerRecord>, TriggerError> {
            unreachable!("claim-race repository should not persist accepted fires")
        }

        async fn mark_fire_replayed(
            &self,
            _request: FireReplayedRequest,
        ) -> Result<Option<TriggerRecord>, TriggerError> {
            unreachable!("claim-race repository should not persist replayed fires")
        }

        async fn mark_fire_retryable_failed(
            &self,
            _request: FireRetryableFailedRequest,
        ) -> Result<Option<TriggerRecord>, TriggerError> {
            unreachable!("claim-race repository should not persist retryable failures")
        }

        async fn mark_fire_permanently_failed(
            &self,
            _request: FirePermanentFailedRequest,
        ) -> Result<Option<TriggerRecord>, TriggerError> {
            unreachable!("claim-race repository should not persist permanent failures")
        }

        async fn mark_fire_terminally_failed(
            &self,
            _request: FireTerminalFailedRequest,
        ) -> Result<Option<TriggerRecord>, TriggerError> {
            unreachable!("claim-race repository should not persist terminal failures")
        }

        async fn clear_active_fire(
            &self,
            _request: ClearActiveFireRequest,
        ) -> Result<Option<TriggerRecord>, TriggerError> {
            unreachable!("claim-race repository should not clear active fires")
        }
    }
}
