use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::ResourceScope;
use thiserror::Error;

use crate::{
    TriggerDeliveryTargetId, TriggerError, TriggerId, TriggerRecord, TriggerRepository,
    TriggerSchedule, TriggerScheduleValidationKind, TriggerSourceKind, TriggerState,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriggerCreateScheduleKind {
    Cron,
    Once,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TriggerCreateSchedule {
    Cron {
        expression: String,
        timezone: String,
    },
    Once {
        at: String,
        timezone: String,
    },
}

impl TriggerCreateSchedule {
    pub fn kind(&self) -> TriggerCreateScheduleKind {
        match self {
            Self::Cron { .. } => TriggerCreateScheduleKind::Cron,
            Self::Once { .. } => TriggerCreateScheduleKind::Once,
        }
    }

    fn into_schedule(self) -> Result<TriggerSchedule, TriggerError> {
        match self {
            Self::Cron {
                expression,
                timezone,
            } => TriggerSchedule::cron_with_timezone(expression, timezone),
            Self::Once { at, timezone } => TriggerSchedule::once_from_input(&at, &timezone),
        }
    }
}

#[derive(Debug, Clone)]
pub struct TriggerCreateRequest {
    pub scope: ResourceScope,
    pub name: String,
    pub prompt: String,
    pub schedule: TriggerCreateSchedule,
    pub delivery_target: Option<TriggerDeliveryTargetId>,
}

#[derive(Debug, Error)]
pub enum TriggerCreationError {
    #[error("invalid {kind:?} trigger schedule: {source}")]
    InvalidSchedule {
        kind: TriggerCreateScheduleKind,
        #[source]
        source: TriggerError,
    },
    #[error("{kind:?} trigger schedule has no future fire time: {source}")]
    NoFutureFireTime {
        kind: TriggerCreateScheduleKind,
        #[source]
        source: TriggerError,
    },
    #[error("invalid trigger delivery target: {source}")]
    InvalidDeliveryTarget {
        #[source]
        source: TriggerError,
    },
    #[error("invalid trigger record: {source}")]
    InvalidRecord {
        #[source]
        source: TriggerError,
    },
    #[error("trigger repository operation {operation} failed: {source}")]
    Repository {
        operation: &'static str,
        #[source]
        source: TriggerError,
    },
    #[error("trigger creation lifecycle operation {operation} failed: {source}")]
    Lifecycle {
        operation: &'static str,
        #[source]
        source: TriggerError,
    },
    #[error(
        "trigger creation rollback operation {operation} failed after lifecycle error {lifecycle_error}: {source}"
    )]
    Rollback {
        operation: &'static str,
        lifecycle_error: TriggerError,
        #[source]
        source: TriggerError,
    },
}

pub trait TriggerCreationClock: Send + Sync {
    fn now(&self) -> chrono::DateTime<chrono::Utc>;
}

#[derive(Debug)]
pub struct SystemTriggerCreationClock;

impl TriggerCreationClock for SystemTriggerCreationClock {
    fn now(&self) -> chrono::DateTime<chrono::Utc> {
        chrono::Utc::now()
    }
}

#[async_trait]
pub trait TriggerCreateLifecycle: Send + Sync {
    async fn validate_delivery_target(
        &self,
        scope: &ResourceScope,
        target: &TriggerDeliveryTargetId,
    ) -> Result<(), TriggerError> {
        let _ = (scope, target);
        Err(TriggerError::InvalidRecord {
            kind: crate::TriggerRecordValidationKind::DeliveryTargetInvalid,
            reason: "per-trigger delivery targets are not supported by this host".to_string(),
        })
    }

    async fn after_trigger_persisted(&self, record: &TriggerRecord) -> Result<(), TriggerError>;
}

#[derive(Debug)]
pub struct NoopTriggerCreationLifecycle;

#[async_trait]
impl TriggerCreateLifecycle for NoopTriggerCreationLifecycle {
    async fn after_trigger_persisted(&self, _record: &TriggerRecord) -> Result<(), TriggerError> {
        Ok(())
    }
}

#[async_trait]
trait TriggerCreationStore: Send + Sync {
    async fn upsert_trigger(&self, record: TriggerRecord) -> Result<(), TriggerError>;

    async fn remove_trigger(
        &self,
        tenant_id: ironclaw_host_api::TenantId,
        trigger_id: TriggerId,
    ) -> Result<Option<TriggerRecord>, TriggerError>;
}

struct RepositoryTriggerCreationStore {
    repository: Arc<dyn TriggerRepository>,
}

#[async_trait]
impl TriggerCreationStore for RepositoryTriggerCreationStore {
    async fn upsert_trigger(&self, record: TriggerRecord) -> Result<(), TriggerError> {
        self.repository.upsert_trigger(record).await
    }

    async fn remove_trigger(
        &self,
        tenant_id: ironclaw_host_api::TenantId,
        trigger_id: TriggerId,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        self.repository.remove_trigger(tenant_id, trigger_id).await
    }
}

#[derive(Clone)]
pub struct TriggerCreationService {
    store: Arc<dyn TriggerCreationStore>,
    lifecycle: Arc<dyn TriggerCreateLifecycle>,
    clock: Arc<dyn TriggerCreationClock>,
    operation_timeout: Option<std::time::Duration>,
}

impl std::fmt::Debug for TriggerCreationService {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TriggerCreationService")
            .field("store", &"Arc<dyn TriggerCreationStore>")
            .field("lifecycle", &"Arc<dyn TriggerCreateLifecycle>")
            .field("clock", &"Arc<dyn TriggerCreationClock>")
            .field("operation_timeout", &self.operation_timeout)
            .finish()
    }
}

impl TriggerCreationService {
    pub fn new(
        repository: Arc<dyn TriggerRepository>,
        lifecycle: Arc<dyn TriggerCreateLifecycle>,
    ) -> Self {
        Self::with_clock(repository, lifecycle, Arc::new(SystemTriggerCreationClock))
    }

    pub fn with_clock(
        repository: Arc<dyn TriggerRepository>,
        lifecycle: Arc<dyn TriggerCreateLifecycle>,
        clock: Arc<dyn TriggerCreationClock>,
    ) -> Self {
        Self {
            store: Arc::new(RepositoryTriggerCreationStore { repository }),
            lifecycle,
            clock,
            operation_timeout: None,
        }
    }

    pub fn with_operation_timeout(mut self, timeout: std::time::Duration) -> Self {
        self.operation_timeout = Some(timeout);
        self
    }

    pub async fn create(
        &self,
        request: TriggerCreateRequest,
    ) -> Result<TriggerRecord, TriggerCreationError> {
        let now = self.clock.now();
        let schedule_kind = request.schedule.kind();
        let schedule = request.schedule.into_schedule().map_err(|source| {
            TriggerCreationError::InvalidSchedule {
                kind: schedule_kind,
                source,
            }
        })?;
        let next_run_at = schedule
            .next_slot_after(now)
            .and_then(|next| {
                next.ok_or_else(|| TriggerError::InvalidSchedule {
                    kind: TriggerScheduleValidationKind::NoFutureFireTime,
                    reason: "schedule has no future fire time".to_string(),
                })
            })
            .map_err(|source| TriggerCreationError::NoFutureFireTime {
                kind: schedule_kind,
                source,
            })?;

        if let Some(target) = request.delivery_target.as_ref() {
            self.lifecycle
                .validate_delivery_target(&request.scope, target)
                .await
                .map_err(|source| TriggerCreationError::InvalidDeliveryTarget { source })?;
        }

        let record = TriggerRecord {
            trigger_id: TriggerId::new(),
            tenant_id: request.scope.tenant_id,
            creator_user_id: request.scope.user_id,
            agent_id: request.scope.agent_id,
            project_id: request.scope.project_id,
            name: request.name,
            source: TriggerSourceKind::Schedule,
            schedule,
            prompt: request.prompt,
            delivery_target: request.delivery_target,
            state: TriggerState::Scheduled,
            next_run_at,
            last_run_at: None,
            last_fired_slot: None,
            last_status: None,
            active_fire_slot: None,
            active_run_ref: None,
            created_at: now,
        };
        record
            .validate()
            .map_err(|source| TriggerCreationError::InvalidRecord { source })?;
        self.run_operation(self.store.upsert_trigger(record.clone()))
            .await
            .map_err(|source| TriggerCreationError::Repository {
                operation: "upsert_trigger",
                source,
            })?;

        if let Err(lifecycle_error) = self
            .run_operation(self.lifecycle.after_trigger_persisted(&record))
            .await
        {
            if let Err(source) = self
                .run_operation(
                    self.store
                        .remove_trigger(record.tenant_id.clone(), record.trigger_id),
                )
                .await
            {
                return Err(TriggerCreationError::Rollback {
                    operation: "remove_trigger",
                    lifecycle_error,
                    source,
                });
            }
            return Err(TriggerCreationError::Lifecycle {
                operation: "after_trigger_persisted",
                source: lifecycle_error,
            });
        }

        Ok(record)
    }

    async fn run_operation<T>(
        &self,
        operation: impl std::future::Future<Output = Result<T, TriggerError>>,
    ) -> Result<T, TriggerError> {
        match self.operation_timeout {
            Some(timeout) => tokio::time::timeout(timeout, operation)
                .await
                .map_err(|_| TriggerError::Backend {
                    reason: "trigger creation operation timed out".to_string(),
                })?,
            None => operation.await,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use chrono::{TimeZone as _, Utc};
    use ironclaw_host_api::{AgentId, InvocationId, ProjectId, TenantId, UserId};

    use super::*;
    use crate::{InMemoryTriggerRepository, TriggerRecordValidationKind};

    #[derive(Debug)]
    struct FixedClock(chrono::DateTime<Utc>);

    impl TriggerCreationClock for FixedClock {
        fn now(&self) -> chrono::DateTime<Utc> {
            self.0
        }
    }

    #[derive(Debug, Default)]
    struct RecordingLifecycle {
        records: Mutex<Vec<TriggerRecord>>,
        fail_after_persist: bool,
    }

    #[async_trait]
    impl TriggerCreateLifecycle for RecordingLifecycle {
        async fn after_trigger_persisted(
            &self,
            record: &TriggerRecord,
        ) -> Result<(), TriggerError> {
            self.records
                .lock()
                .map_err(|error| TriggerError::Backend {
                    reason: format!("recording lifecycle lock poisoned: {error}"),
                })?
                .push(record.clone());
            if self.fail_after_persist {
                Err(TriggerError::Backend {
                    reason: "pairing unavailable".to_string(),
                })
            } else {
                Ok(())
            }
        }
    }

    fn scope() -> ResourceScope {
        ResourceScope {
            tenant_id: TenantId::new("tenant-a").expect("tenant"),
            user_id: UserId::new("user-a").expect("user"),
            agent_id: Some(AgentId::new("agent-a").expect("agent")),
            project_id: Some(ProjectId::new("project-a").expect("project")),
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        }
    }

    fn service(
        repository: Arc<dyn TriggerRepository>,
        lifecycle: Arc<dyn TriggerCreateLifecycle>,
    ) -> TriggerCreationService {
        TriggerCreationService::with_clock(
            repository,
            lifecycle,
            Arc::new(FixedClock(
                Utc.with_ymd_and_hms(2026, 7, 16, 8, 0, 0)
                    .single()
                    .expect("fixed time"),
            )),
        )
    }

    fn cron_request(expression: &str, timezone: &str) -> TriggerCreateRequest {
        TriggerCreateRequest {
            scope: scope(),
            name: "Daily summary".to_string(),
            prompt: "Generate a summary".to_string(),
            schedule: TriggerCreateSchedule::Cron {
                expression: expression.to_string(),
                timezone: timezone.to_string(),
            },
            delivery_target: None,
        }
    }

    #[tokio::test]
    async fn creates_caller_scoped_cron_trigger_and_runs_lifecycle() {
        let repository = Arc::new(InMemoryTriggerRepository::default());
        let lifecycle = Arc::new(RecordingLifecycle::default());
        let record = service(repository.clone(), lifecycle.clone())
            .create(cron_request("0 9 * * *", "UTC"))
            .await
            .expect("create cron trigger");

        assert_eq!(record.tenant_id, scope().tenant_id);
        assert_eq!(record.creator_user_id, scope().user_id);
        assert_eq!(record.agent_id, scope().agent_id);
        assert_eq!(record.project_id, scope().project_id);
        assert_eq!(
            record.created_at,
            Utc.with_ymd_and_hms(2026, 7, 16, 8, 0, 0).unwrap()
        );
        assert_eq!(
            record.next_run_at,
            Utc.with_ymd_and_hms(2026, 7, 16, 9, 0, 0).unwrap()
        );
        assert!(
            repository
                .get_trigger(record.tenant_id.clone(), record.trigger_id)
                .await
                .expect("read trigger")
                .is_some()
        );
        assert_eq!(lifecycle.records.lock().expect("records").len(), 1);
    }

    #[tokio::test]
    async fn creates_future_one_time_trigger() {
        let repository = Arc::new(InMemoryTriggerRepository::default());
        let record = service(repository, Arc::new(RecordingLifecycle::default()))
            .create(TriggerCreateRequest {
                scope: scope(),
                name: "Follow up".to_string(),
                prompt: "Check the deployment".to_string(),
                schedule: TriggerCreateSchedule::Once {
                    at: "2026-07-16T18:00:00".to_string(),
                    timezone: "Asia/Shanghai".to_string(),
                },
                delivery_target: None,
            })
            .await
            .expect("create one-time trigger");

        assert_eq!(
            record.next_run_at,
            Utc.with_ymd_and_hms(2026, 7, 16, 10, 0, 0).unwrap()
        );
    }

    #[tokio::test]
    async fn creates_rfc3339_one_time_trigger() {
        let repository = Arc::new(InMemoryTriggerRepository::default());
        let record = service(repository, Arc::new(RecordingLifecycle::default()))
            .create(TriggerCreateRequest {
                scope: scope(),
                name: "Follow up".to_string(),
                prompt: "Check the deployment".to_string(),
                schedule: TriggerCreateSchedule::Once {
                    at: "2026-07-16T18:00:00+08:00".to_string(),
                    timezone: "Asia/Shanghai".to_string(),
                },
                delivery_target: None,
            })
            .await
            .expect("create RFC3339 one-time trigger");

        assert_eq!(
            record.next_run_at,
            Utc.with_ymd_and_hms(2026, 7, 16, 10, 0, 0).unwrap()
        );
    }

    #[tokio::test]
    async fn rejects_invalid_schedule_variants_before_persistence() {
        let cases = [
            cron_request("0 9 * * *", "Not/A/Timezone"),
            cron_request("0 8 * *", "UTC"),
            cron_request("*/10 * * * * *", "UTC"),
            TriggerCreateRequest {
                scope: scope(),
                name: "DST overlap".to_string(),
                prompt: "Check something".to_string(),
                schedule: TriggerCreateSchedule::Once {
                    at: "2026-11-01T01:30:00".to_string(),
                    timezone: "America/New_York".to_string(),
                },
                delivery_target: None,
            },
            TriggerCreateRequest {
                scope: scope(),
                name: "Past".to_string(),
                prompt: "Check something".to_string(),
                schedule: TriggerCreateSchedule::Once {
                    at: "2026-07-16T07:59:00".to_string(),
                    timezone: "UTC".to_string(),
                },
                delivery_target: None,
            },
            TriggerCreateRequest {
                scope: scope(),
                name: "Offset mismatch".to_string(),
                prompt: "Check something".to_string(),
                schedule: TriggerCreateSchedule::Once {
                    at: "2026-07-16T18:00:00+09:00".to_string(),
                    timezone: "Asia/Shanghai".to_string(),
                },
                delivery_target: None,
            },
            TriggerCreateRequest {
                scope: scope(),
                name: "Past RFC3339".to_string(),
                prompt: "Check something".to_string(),
                schedule: TriggerCreateSchedule::Once {
                    at: "2026-07-16T07:59:00Z".to_string(),
                    timezone: "UTC".to_string(),
                },
                delivery_target: None,
            },
            TriggerCreateRequest {
                scope: scope(),
                name: "Equal-now RFC3339".to_string(),
                prompt: "Check something".to_string(),
                schedule: TriggerCreateSchedule::Once {
                    at: "2026-07-16T08:00:00Z".to_string(),
                    timezone: "UTC".to_string(),
                },
                delivery_target: None,
            },
            TriggerCreateRequest {
                scope: scope(),
                name: "Invalid RFC3339 timezone".to_string(),
                prompt: "Check something".to_string(),
                schedule: TriggerCreateSchedule::Once {
                    at: "2026-07-16T18:00:00+08:00".to_string(),
                    timezone: "Not/A/Timezone".to_string(),
                },
                delivery_target: None,
            },
            TriggerCreateRequest {
                scope: scope(),
                name: "Malformed RFC3339".to_string(),
                prompt: "Check something".to_string(),
                schedule: TriggerCreateSchedule::Once {
                    at: "not-a-date+08:00".to_string(),
                    timezone: "Asia/Shanghai".to_string(),
                },
                delivery_target: None,
            },
        ];

        for request in cases {
            let repository = Arc::new(InMemoryTriggerRepository::default());
            let error = service(repository.clone(), Arc::new(RecordingLifecycle::default()))
                .create(request)
                .await
                .expect_err("invalid schedule rejected");
            assert!(matches!(
                error,
                TriggerCreationError::InvalidSchedule { .. }
                    | TriggerCreationError::NoFutureFireTime { .. }
            ));
            assert!(
                repository
                    .list_triggers(scope().tenant_id)
                    .await
                    .expect("list triggers")
                    .is_empty()
            );
        }
    }

    #[tokio::test]
    async fn rejects_invalid_record_before_persistence() {
        let repository = Arc::new(InMemoryTriggerRepository::default());
        let mut request = cron_request("0 9 * * *", "UTC");
        request.prompt = " ".to_string();
        let error = service(repository.clone(), Arc::new(RecordingLifecycle::default()))
            .create(request)
            .await
            .expect_err("empty prompt rejected");

        assert!(matches!(
            error,
            TriggerCreationError::InvalidRecord {
                source: TriggerError::InvalidRecord {
                    kind: TriggerRecordValidationKind::PromptEmpty,
                    ..
                }
            }
        ));
        assert!(
            repository
                .list_triggers(scope().tenant_id)
                .await
                .expect("list triggers")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn lifecycle_failure_rolls_back_persisted_trigger() {
        let repository = Arc::new(InMemoryTriggerRepository::default());
        let lifecycle = Arc::new(RecordingLifecycle {
            records: Mutex::new(Vec::new()),
            fail_after_persist: true,
        });
        let error = service(repository.clone(), lifecycle)
            .create(cron_request("0 9 * * *", "UTC"))
            .await
            .expect_err("lifecycle failure returned");

        assert!(matches!(error, TriggerCreationError::Lifecycle { .. }));
        assert!(
            repository
                .list_triggers(scope().tenant_id)
                .await
                .expect("list triggers")
                .is_empty()
        );
    }

    #[derive(Debug)]
    struct SlowLifecycle;

    #[async_trait]
    impl TriggerCreateLifecycle for SlowLifecycle {
        async fn after_trigger_persisted(
            &self,
            _record: &TriggerRecord,
        ) -> Result<(), TriggerError> {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            Ok(())
        }
    }

    #[tokio::test]
    async fn lifecycle_timeout_rolls_back_persisted_trigger() {
        let repository = Arc::new(InMemoryTriggerRepository::default());
        let service = service(repository.clone(), Arc::new(SlowLifecycle))
            .with_operation_timeout(std::time::Duration::from_millis(1));

        let error = service
            .create(cron_request("0 9 * * *", "UTC"))
            .await
            .expect_err("lifecycle timeout returned");

        assert!(matches!(error, TriggerCreationError::Lifecycle { .. }));
        assert!(
            repository
                .list_triggers(scope().tenant_id)
                .await
                .expect("list triggers")
                .is_empty()
        );
    }

    #[derive(Debug, Default)]
    struct RollbackFailingStore {
        persisted: Mutex<Option<TriggerRecord>>,
    }

    #[async_trait]
    impl TriggerCreationStore for RollbackFailingStore {
        async fn upsert_trigger(&self, record: TriggerRecord) -> Result<(), TriggerError> {
            *self
                .persisted
                .lock()
                .map_err(|error| TriggerError::Backend {
                    reason: format!("store lock poisoned: {error}"),
                })? = Some(record);
            Ok(())
        }

        async fn remove_trigger(
            &self,
            _tenant_id: TenantId,
            _trigger_id: TriggerId,
        ) -> Result<Option<TriggerRecord>, TriggerError> {
            Err(TriggerError::Backend {
                reason: "rollback unavailable".to_string(),
            })
        }
    }

    #[tokio::test]
    async fn rollback_failure_is_distinct_and_keeps_persisted_evidence() {
        let store = Arc::new(RollbackFailingStore::default());
        let service = TriggerCreationService {
            store: store.clone(),
            lifecycle: Arc::new(RecordingLifecycle {
                records: Mutex::new(Vec::new()),
                fail_after_persist: true,
            }),
            clock: Arc::new(FixedClock(
                Utc.with_ymd_and_hms(2026, 7, 16, 8, 0, 0).unwrap(),
            )),
            operation_timeout: None,
        };
        let error = service
            .create(cron_request("0 9 * * *", "UTC"))
            .await
            .expect_err("rollback failure returned");

        assert!(matches!(error, TriggerCreationError::Rollback { .. }));
        assert!(store.persisted.lock().expect("persisted").is_some());
    }
}
