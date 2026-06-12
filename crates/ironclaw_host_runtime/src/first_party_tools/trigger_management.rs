use std::{sync::Arc, time::Instant};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ironclaw_extensions::{CapabilityManifest, ExtensionError};
use ironclaw_host_api::{
    CapabilityId, EffectKind, HostApiError, PermissionMode, ResourceScope, ResourceUsage,
    RuntimeDispatchErrorKind,
};
use ironclaw_triggers::{
    TriggerCompletionPolicy, TriggerError, TriggerId, TriggerRecord, TriggerRepository,
    TriggerRunRecord, TriggerSchedule, TriggerSourceKind, TriggerState,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    FirstPartyCapabilityError, FirstPartyCapabilityHandler, FirstPartyCapabilityRegistry,
    FirstPartyCapabilityRequest, FirstPartyCapabilityResult,
};

use super::{
    FIRST_PARTY_MAX_OUTPUT_BYTES, bounded_input_size, bounded_output_bytes,
    first_party_capability_manifest, input_error, resource_profile,
};

const TRIGGER_LIST_MAX_LIMIT: usize = 100;
const TRIGGER_RUN_HISTORY_DEFAULT_LIMIT: usize = 25;
const TRIGGER_RUN_HISTORY_MAX_LIMIT: usize = 100;

pub const TRIGGER_CREATE_CAPABILITY_ID: &str = "builtin.trigger_create";
pub const TRIGGER_LIST_CAPABILITY_ID: &str = "builtin.trigger_list";
pub const TRIGGER_REMOVE_CAPABILITY_ID: &str = "builtin.trigger_remove";

pub(super) fn manifests() -> Result<Vec<CapabilityManifest>, ExtensionError> {
    Ok(vec![
        first_party_capability_manifest(
            TRIGGER_CREATE_CAPABILITY_ID,
            "Create a caller-scoped scheduled trigger",
            vec![EffectKind::DispatchCapability, EffectKind::ExternalWrite],
            PermissionMode::Ask,
            resource_profile(),
        )?,
        first_party_capability_manifest(
            TRIGGER_LIST_CAPABILITY_ID,
            "List scheduled triggers owned by the current caller scope",
            vec![EffectKind::DispatchCapability],
            PermissionMode::Allow,
            resource_profile(),
        )?,
        first_party_capability_manifest(
            TRIGGER_REMOVE_CAPABILITY_ID,
            "Remove a caller-scoped scheduled trigger",
            vec![EffectKind::DispatchCapability, EffectKind::ExternalWrite],
            PermissionMode::Ask,
            resource_profile(),
        )?,
    ])
}

pub(super) fn insert_handlers(
    registry: &mut FirstPartyCapabilityRegistry,
    repository: Arc<dyn TriggerRepository>,
) -> Result<(), HostApiError> {
    insert_handlers_with_create_hook(registry, repository, Arc::new(NoopTriggerCreateHook))
}

pub(super) fn insert_handlers_with_create_hook(
    registry: &mut FirstPartyCapabilityRegistry,
    repository: Arc<dyn TriggerRepository>,
    create_hook: Arc<dyn TriggerCreateHook>,
) -> Result<(), HostApiError> {
    insert_trigger_handlers(
        registry,
        Arc::new(TriggerManagementToolHandler {
            repository,
            create_hook,
            clock: Arc::new(SystemTriggerManagementClock),
        }),
    )
}

#[cfg(any(test, feature = "test-support"))]
pub(super) fn insert_handlers_with_clock(
    registry: &mut FirstPartyCapabilityRegistry,
    repository: Arc<dyn TriggerRepository>,
    clock: Arc<dyn TriggerManagementClock>,
) -> Result<(), HostApiError> {
    insert_trigger_handlers(
        registry,
        Arc::new(TriggerManagementToolHandler {
            repository,
            create_hook: Arc::new(NoopTriggerCreateHook),
            clock,
        }),
    )
}

fn insert_trigger_handlers(
    registry: &mut FirstPartyCapabilityRegistry,
    handler: Arc<TriggerManagementToolHandler>,
) -> Result<(), HostApiError> {
    registry.insert_handler(
        CapabilityId::new(TRIGGER_CREATE_CAPABILITY_ID)?,
        handler.clone(),
    );
    registry.insert_handler(
        CapabilityId::new(TRIGGER_LIST_CAPABILITY_ID)?,
        handler.clone(),
    );
    registry.insert_handler(CapabilityId::new(TRIGGER_REMOVE_CAPABILITY_ID)?, handler);
    Ok(())
}

#[cfg(any(test, feature = "test-support"))]
#[doc(hidden)]
pub trait TriggerManagementClock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
}

#[cfg(not(any(test, feature = "test-support")))]
trait TriggerManagementClock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
}

#[async_trait]
pub trait TriggerCreateHook: Send + Sync {
    async fn after_trigger_persisted(&self, record: &TriggerRecord) -> Result<(), TriggerError>;
}

#[derive(Debug)]
struct NoopTriggerCreateHook;

#[async_trait]
impl TriggerCreateHook for NoopTriggerCreateHook {
    async fn after_trigger_persisted(&self, _record: &TriggerRecord) -> Result<(), TriggerError> {
        Ok(())
    }
}

#[derive(Debug)]
struct SystemTriggerManagementClock;

impl TriggerManagementClock for SystemTriggerManagementClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

struct TriggerManagementToolHandler {
    repository: Arc<dyn TriggerRepository>,
    create_hook: Arc<dyn TriggerCreateHook>,
    clock: Arc<dyn TriggerManagementClock>,
}

#[async_trait]
impl FirstPartyCapabilityHandler for TriggerManagementToolHandler {
    async fn dispatch(
        &self,
        request: FirstPartyCapabilityRequest,
    ) -> Result<FirstPartyCapabilityResult, FirstPartyCapabilityError> {
        bounded_input_size(request.capability_id.as_str(), &request.input)?;
        let started = Instant::now();
        let output = match request.capability_id.as_str() {
            TRIGGER_CREATE_CAPABILITY_ID => {
                create_trigger(
                    &*self.repository,
                    &*self.create_hook,
                    &request.scope,
                    request.input,
                    self.clock.now(),
                )
                .await?
            }
            TRIGGER_LIST_CAPABILITY_ID => {
                list_triggers(&*self.repository, &request.scope, request.input).await?
            }
            TRIGGER_REMOVE_CAPABILITY_ID => {
                remove_trigger(&*self.repository, &request.scope, request.input).await?
            }
            _ => {
                return Err(FirstPartyCapabilityError::new(
                    RuntimeDispatchErrorKind::UndeclaredCapability,
                ));
            }
        };
        let output_bytes = bounded_output_bytes(&output, FIRST_PARTY_MAX_OUTPUT_BYTES)?;
        Ok(FirstPartyCapabilityResult::new(
            output,
            elapsed_usage_with_bytes(started, output_bytes),
        ))
    }
}

#[derive(Deserialize)]
struct TriggerCreateInput {
    name: String,
    prompt: String,
    cron: String,
    timezone: String,
}

#[derive(Deserialize)]
struct TriggerRemoveInput {
    trigger_id: String,
}

#[derive(Deserialize)]
struct TriggerListInput {
    limit: Option<usize>,
    run_limit: Option<usize>,
}

async fn create_trigger(
    repository: &dyn TriggerRepository,
    create_hook: &dyn TriggerCreateHook,
    scope: &ResourceScope,
    input: Value,
    now: DateTime<Utc>,
) -> Result<Value, FirstPartyCapabilityError> {
    let input: TriggerCreateInput = serde_json::from_value(input).map_err(|_| input_error())?;
    let schedule = TriggerSchedule::cron_with_timezone(input.cron, input.timezone)
        .map_err(trigger_input_error)?;
    let next_run_at = next_run_at_for_schedule(&schedule, now)?;
    let record = TriggerRecord {
        trigger_id: TriggerId::new(),
        tenant_id: scope.tenant_id.clone(),
        creator_user_id: scope.user_id.clone(),
        agent_id: scope.agent_id.clone(),
        project_id: scope.project_id.clone(),
        name: input.name,
        source: TriggerSourceKind::Schedule,
        schedule,
        completion_policy: TriggerCompletionPolicy::Recurring,
        prompt: input.prompt,
        state: TriggerState::Scheduled,
        next_run_at,
        last_run_at: None,
        last_fired_slot: None,
        last_status: None,
        active_fire_slot: None,
        active_run_ref: None,
        created_at: now,
    };
    record.validate().map_err(trigger_input_error)?;
    repository
        .upsert_trigger(record.clone())
        .await
        .map_err(|error| trigger_repository_error("upsert_trigger", error))?;
    if let Err(error) = create_hook.after_trigger_persisted(&record).await {
        let hook_error = trigger_create_hook_error("after_trigger_persisted", error);
        if let Err(remove_error) = repository
            .remove_trigger(record.tenant_id.clone(), record.trigger_id)
            .await
        {
            return Err(trigger_create_rollback_error(
                "remove_trigger",
                remove_error,
            ));
        }
        return Err(hook_error);
    }
    Ok(json!({
        "trigger": trigger_output(&record, &[]),
    }))
}

async fn list_triggers(
    repository: &dyn TriggerRepository,
    scope: &ResourceScope,
    input: Value,
) -> Result<Value, FirstPartyCapabilityError> {
    let input: TriggerListInput = serde_json::from_value(input).map_err(|_| input_error())?;
    let limit = input
        .limit
        .unwrap_or(TRIGGER_LIST_MAX_LIMIT)
        .min(TRIGGER_LIST_MAX_LIMIT);
    let run_limit = input
        .run_limit
        .unwrap_or(TRIGGER_RUN_HISTORY_DEFAULT_LIMIT)
        .min(TRIGGER_RUN_HISTORY_MAX_LIMIT);
    let records = repository
        .list_scoped_triggers(
            scope.tenant_id.clone(),
            scope.user_id.clone(),
            scope.agent_id.clone(),
            scope.project_id.clone(),
            limit,
        )
        .await
        .map_err(|error| trigger_repository_error("list_scoped_triggers", error))?;
    let trigger_ids = records
        .iter()
        .map(|record| record.trigger_id)
        .collect::<Vec<_>>();
    let mut runs_by_trigger = repository
        .list_trigger_run_history_batch(scope.tenant_id.clone(), &trigger_ids, run_limit)
        .await
        .map_err(|error| trigger_repository_error("list_trigger_run_history_batch", error))?;
    let output = records
        .into_iter()
        .map(|record| {
            let runs = runs_by_trigger
                .remove(&record.trigger_id)
                .unwrap_or_default();
            trigger_output(&record, &runs)
        })
        .collect::<Vec<_>>();
    Ok(json!({ "triggers": output }))
}

async fn remove_trigger(
    repository: &dyn TriggerRepository,
    scope: &ResourceScope,
    input: Value,
) -> Result<Value, FirstPartyCapabilityError> {
    let input: TriggerRemoveInput = serde_json::from_value(input).map_err(|_| input_error())?;
    let trigger_id = TriggerId::parse(&input.trigger_id).map_err(trigger_input_error)?;
    let removed = repository
        .remove_scoped_trigger(
            scope.tenant_id.clone(),
            scope.user_id.clone(),
            scope.agent_id.clone(),
            scope.project_id.clone(),
            trigger_id,
        )
        .await
        .map_err(|error| trigger_repository_error("remove_scoped_trigger", error))?;
    Ok(json!({
        "removed": removed.is_some(),
        "trigger": removed.as_ref().map(trigger_remove_output),
    }))
}

fn trigger_output(record: &TriggerRecord, recent_runs: &[TriggerRunRecord]) -> Value {
    json!({
        "trigger_id": record.trigger_id.to_string(),
        "agent_id": record.agent_id.as_ref().map(|id| id.as_str()),
        "project_id": record.project_id.as_ref().map(|id| id.as_str()),
        "name": record.name,
        "source": record.source,
        "schedule": record.schedule,
        "completion_policy": record.completion_policy,
        "state": record.state,
        "next_run_at": record.next_run_at,
        "last_run_at": record.last_run_at,
        "last_status": record.last_status,
        "recent_runs": recent_runs.iter().map(trigger_run_output).collect::<Vec<_>>(),
        "is_active": record.has_active_fire(),
        "created_at": record.created_at,
    })
}

fn trigger_run_output(run: &TriggerRunRecord) -> Value {
    json!({
        "fire_slot": run.fire_slot,
        "run_id": run.run_id.as_ref().map(ToString::to_string),
        "thread_id": run.thread_id.as_ref().map(|t| t.as_str()),
        "status": run.status,
        "submitted_at": run.submitted_at,
        "completed_at": run.completed_at,
    })
}

fn trigger_remove_output(record: &TriggerRecord) -> Value {
    json!({
        "trigger_id": record.trigger_id.to_string(),
        "name": record.name,
    })
}

fn next_run_at_for_schedule(
    schedule: &TriggerSchedule,
    now: DateTime<Utc>,
) -> Result<DateTime<Utc>, FirstPartyCapabilityError> {
    schedule
        .next_slot_after(now)
        .map_err(trigger_input_error)?
        .ok_or_else(input_error)
}

fn trigger_input_error(error: TriggerError) -> FirstPartyCapabilityError {
    tracing::debug!(
        runtime_dispatch_error_kind = %RuntimeDispatchErrorKind::InputEncode,
        trigger_error_kind = trigger_error_kind(&error),
        "trigger management capability input validation failed"
    );
    input_error()
}

fn trigger_repository_error(
    repository_operation: &'static str,
    error: TriggerError,
) -> FirstPartyCapabilityError {
    tracing::debug!(
        runtime_dispatch_error_kind = %RuntimeDispatchErrorKind::Backend,
        repository_operation,
        trigger_error_kind = trigger_error_kind(&error),
        "trigger management capability repository operation failed"
    );
    FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::Backend)
}

fn trigger_create_hook_error(
    hook_operation: &'static str,
    error: TriggerError,
) -> FirstPartyCapabilityError {
    tracing::debug!(
        runtime_dispatch_error_kind = %RuntimeDispatchErrorKind::Backend,
        hook_operation,
        trigger_error_kind = trigger_error_kind(&error),
        "trigger management capability create hook failed"
    );
    FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::Backend)
}

fn trigger_create_rollback_error(
    repository_operation: &'static str,
    error: TriggerError,
) -> FirstPartyCapabilityError {
    tracing::warn!(
        runtime_dispatch_error_kind = %RuntimeDispatchErrorKind::Backend,
        repository_operation,
        trigger_error_kind = trigger_error_kind(&error),
        error_kind = "trigger_create_rollback_failed",
        "trigger management capability create hook rollback failed"
    );
    FirstPartyCapabilityError::with_safe_summary(
        RuntimeDispatchErrorKind::Backend,
        "trigger create rollback failed after hook error",
    )
}

fn trigger_error_kind(error: &TriggerError) -> &'static str {
    match error {
        TriggerError::InvalidTriggerId { .. } => "invalid_trigger_id",
        TriggerError::InvalidFireIdentityComponent { .. } => "invalid_fire_identity_component",
        TriggerError::InvalidRecord { .. } => "invalid_record",
        TriggerError::InvalidPollerConfig { .. } => "invalid_poller_config",
        TriggerError::InvalidSchedule { .. } => "invalid_schedule",
        TriggerError::InvalidMaterialization { .. } => "invalid_materialization",
        TriggerError::Backend { .. } => "backend",
        TriggerError::NotFound => "not_found",
    }
}

fn elapsed_usage_with_bytes(started: Instant, output_bytes: u64) -> ResourceUsage {
    ResourceUsage {
        wall_clock_ms: started.elapsed().as_millis().try_into().unwrap_or(u64::MAX),
        output_bytes,
        ..ResourceUsage::default()
    }
}

#[cfg(test)]
mod tests {
    use chrono::{Datelike, TimeZone};

    use super::*;

    #[test]
    fn next_run_at_for_schedule_rejects_schedule_with_no_future_slot() {
        let future_year = Utc::now().year() + 1;
        let schedule = TriggerSchedule::cron(format!("0 0 8 * * * {future_year}"))
            .expect("future finite schedule is valid");
        let after_schedule_expires = Utc
            .with_ymd_and_hms(future_year + 1, 1, 1, 0, 0, 0)
            .unwrap();

        let error = next_run_at_for_schedule(&schedule, after_schedule_expires)
            .expect_err("exhausted schedule rejected");

        assert!(matches!(
            error,
            FirstPartyCapabilityError::Dispatch {
                kind: RuntimeDispatchErrorKind::InputEncode,
                ..
            }
        ));
    }

    #[test]
    fn trigger_create_input_rejects_missing_timezone() {
        let input = serde_json::json!({
            "name": "daily",
            "prompt": "check mail",
            "cron": "0 9 * * *"
        });
        let result: Result<TriggerCreateInput, _> = serde_json::from_value(input);
        assert!(
            result.is_err(),
            "missing timezone must fail deserialization"
        );
    }

    #[test]
    fn trigger_create_input_rejects_invalid_timezone() {
        let input = serde_json::json!({
            "name": "daily",
            "prompt": "check mail",
            "cron": "0 9 * * *",
            "timezone": "Not/A/Timezone"
        });
        let parsed: TriggerCreateInput = serde_json::from_value(input).expect("deserialize");
        let result = TriggerSchedule::cron_with_timezone(parsed.cron, parsed.timezone);
        assert!(result.is_err(), "invalid timezone must be rejected");
        let error_msg = result.unwrap_err().to_string();
        assert!(
            error_msg.contains("invalid timezone"),
            "error should name the problem: {error_msg}"
        );
    }

    #[test]
    fn trigger_create_input_accepts_valid_timezone() {
        let input = serde_json::json!({
            "name": "daily",
            "prompt": "check mail",
            "cron": "0 9 * * *",
            "timezone": "America/Los_Angeles"
        });
        let parsed: TriggerCreateInput = serde_json::from_value(input).expect("deserialize");
        let schedule = TriggerSchedule::cron_with_timezone(parsed.cron, &parsed.timezone)
            .expect("valid timezone accepted");
        match &schedule {
            TriggerSchedule::Cron { timezone, .. } => {
                assert_eq!(timezone, "America/Los_Angeles");
            }
        }
    }
}
