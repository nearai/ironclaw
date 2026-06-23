use std::{sync::Arc, time::Instant};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ironclaw_extensions::{CapabilityManifest, ExtensionError};
use ironclaw_host_api::{
    CapabilityId, DispatchInputIssue, DispatchInputIssueCode, EffectKind, HostApiError,
    PermissionMode, ResourceScope, ResourceUsage, RuntimeDispatchErrorKind,
};
use ironclaw_triggers::{
    TriggerError, TriggerId, TriggerRecord, TriggerRecordValidationKind, TriggerRepository,
    TriggerRunRecord, TriggerSchedule, TriggerScheduleValidationKind, TriggerSourceKind,
    TriggerState,
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
pub const TRIGGER_PAUSE_CAPABILITY_ID: &str = "builtin.trigger_pause";
pub const TRIGGER_RESUME_CAPABILITY_ID: &str = "builtin.trigger_resume";

const TRIGGER_CREATE_DESCRIPTION: &str = "Create a caller-scoped scheduled trigger (one-time or recurring). If the user asks for routine or trigger results to be sent through an outbound product or channel, use the visible outbound delivery target capabilities to select that delivery target before creating the trigger; delivery routing is not encoded in this input.";

pub(super) fn manifests() -> Result<Vec<CapabilityManifest>, ExtensionError> {
    Ok(vec![
        first_party_capability_manifest(
            TRIGGER_CREATE_CAPABILITY_ID,
            TRIGGER_CREATE_DESCRIPTION,
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
        first_party_capability_manifest(
            TRIGGER_PAUSE_CAPABILITY_ID,
            "Pause a caller-scoped scheduled trigger so it remains retained but does not fire",
            vec![EffectKind::DispatchCapability, EffectKind::ExternalWrite],
            PermissionMode::Ask,
            resource_profile(),
        )?,
        first_party_capability_manifest(
            TRIGGER_RESUME_CAPABILITY_ID,
            "Resume a caller-scoped paused trigger so it may fire on its stored schedule",
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
    registry.insert_handler(
        CapabilityId::new(TRIGGER_REMOVE_CAPABILITY_ID)?,
        handler.clone(),
    );
    registry.insert_handler(
        CapabilityId::new(TRIGGER_PAUSE_CAPABILITY_ID)?,
        handler.clone(),
    );
    registry.insert_handler(CapabilityId::new(TRIGGER_RESUME_CAPABILITY_ID)?, handler);
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
            TRIGGER_PAUSE_CAPABILITY_ID => {
                set_trigger_state(
                    &*self.repository,
                    &request.scope,
                    request.input,
                    TriggerState::Paused,
                )
                .await?
            }
            TRIGGER_RESUME_CAPABILITY_ID => {
                set_trigger_state(
                    &*self.repository,
                    &request.scope,
                    request.input,
                    TriggerState::Scheduled,
                )
                .await?
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
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum TriggerScheduleInput {
    Cron {
        expression: String,
        timezone: String,
    },
    Once {
        at: String,
        timezone: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TriggerScheduleInputKind {
    Cron,
    Once,
}

impl TriggerScheduleInput {
    fn kind(&self) -> TriggerScheduleInputKind {
        match self {
            Self::Cron { .. } => TriggerScheduleInputKind::Cron,
            Self::Once { .. } => TriggerScheduleInputKind::Once,
        }
    }

    fn into_schedule(self) -> Result<TriggerSchedule, TriggerError> {
        match self {
            Self::Cron {
                expression,
                timezone,
            } => TriggerSchedule::cron_with_timezone(expression, timezone),
            Self::Once { at, timezone } => TriggerSchedule::once_from_local(&at, &timezone),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TriggerCreateInput {
    name: String,
    prompt: String,
    schedule: TriggerScheduleInput,
}

#[derive(Deserialize)]
struct TriggerRemoveInput {
    trigger_id: String,
}

#[derive(Deserialize)]
struct TriggerStateInput {
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
    let input: TriggerCreateInput = TriggerCreateInput::deserialize(&input)
        .map_err(|error| trigger_create_shape_error(&input, error))?;
    let schedule_kind = input.schedule.kind();
    let schedule = input
        .schedule
        .into_schedule()
        .map_err(|error| trigger_schedule_error(schedule_kind, error))?;
    let next_run_at = next_run_at_for_schedule(&schedule, now)
        .map_err(|error| trigger_next_run_error(schedule_kind, error))?;
    let record = TriggerRecord {
        trigger_id: TriggerId::new(),
        tenant_id: scope.tenant_id.clone(),
        creator_user_id: scope.user_id.clone(),
        agent_id: scope.agent_id.clone(),
        project_id: scope.project_id.clone(),
        name: input.name,
        source: TriggerSourceKind::Schedule,
        schedule,
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
    record.validate().map_err(trigger_record_error)?;
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
            &[],
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

async fn set_trigger_state(
    repository: &dyn TriggerRepository,
    scope: &ResourceScope,
    input: Value,
    state: TriggerState,
) -> Result<Value, FirstPartyCapabilityError> {
    let input: TriggerStateInput = serde_json::from_value(input).map_err(|_| input_error())?;
    let trigger_id = TriggerId::parse(&input.trigger_id).map_err(trigger_input_error)?;
    let updated = repository
        .set_scoped_trigger_state(
            scope.tenant_id.clone(),
            scope.user_id.clone(),
            scope.agent_id.clone(),
            scope.project_id.clone(),
            trigger_id,
            state,
        )
        .await
        .map_err(|error| trigger_repository_error("set_scoped_trigger_state", error))?;
    Ok(json!({
        "updated": updated.is_some(),
        "trigger": updated.as_ref().map(|record| trigger_output(record, &[])),
    }))
}

fn trigger_output(record: &TriggerRecord, recent_runs: &[TriggerRunRecord]) -> Value {
    let is_enabled = record.state == TriggerState::Scheduled;
    let has_active_fire = record.has_active_fire();
    json!({
        "trigger_id": record.trigger_id.to_string(),
        "agent_id": record.agent_id.as_ref().map(|id| id.as_str()),
        "project_id": record.project_id.as_ref().map(|id| id.as_str()),
        "name": record.name,
        "source": record.source,
        "schedule": record.schedule,
        "state": record.state,
        "next_run_at": record.next_run_at,
        "last_run_at": record.last_run_at,
        "last_status": record.last_status,
        "recent_runs": recent_runs.iter().map(trigger_run_output).collect::<Vec<_>>(),
        // Model-facing trigger status: `is_active` means the trigger is enabled
        // to fire. In-flight run state is exposed separately as `has_active_fire`.
        "is_enabled": is_enabled,
        "is_active": is_enabled,
        "has_active_fire": has_active_fire,
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
) -> Result<DateTime<Utc>, TriggerError> {
    schedule.next_slot_after(now).and_then(|next| {
        next.ok_or_else(|| TriggerError::InvalidSchedule {
            kind: TriggerScheduleValidationKind::NoFutureFireTime,
            reason: "schedule has no future fire time".to_string(),
        })
    })
}

fn trigger_create_shape_error(
    input: &Value,
    _error: serde_json::Error,
) -> FirstPartyCapabilityError {
    invalid_trigger_input(classify_trigger_create_shape(input))
}

fn classify_trigger_create_shape(input: &Value) -> Vec<DispatchInputIssue> {
    let Some(root) = input.as_object() else {
        return vec![type_mismatch("input", "object")];
    };

    let mut issues = Vec::new();
    required_string(root, "name", "name", "string", &mut issues);
    required_string(root, "prompt", "prompt", "string", &mut issues);
    unexpected_fields(
        root,
        &["name", "prompt", "schedule"],
        "unexpected_field",
        &mut issues,
    );

    let Some(schedule) = root.get("schedule") else {
        issues.push(missing_required("schedule").expected("object with kind"));
        return issues;
    };
    let Some(schedule) = schedule.as_object() else {
        issues.push(type_mismatch("schedule", "object"));
        return issues;
    };

    match schedule.get("kind") {
        None | Some(Value::Null) => {
            issues.push(missing_required("schedule.kind").expected("cron or once"));
        }
        Some(Value::String(kind)) if kind == "cron" => {
            schedule_variant_shape_issues(
                schedule,
                &["kind", "expression", "timezone"],
                &[
                    ("expression", "schedule.expression", "cron expression"),
                    ("timezone", "schedule.timezone", "IANA timezone name"),
                ],
                &mut issues,
            );
        }
        Some(Value::String(kind)) if kind == "once" => {
            schedule_variant_shape_issues(
                schedule,
                &["kind", "at", "timezone"],
                &[
                    ("at", "schedule.at", "YYYY-MM-DDTHH:MM:SS"),
                    ("timezone", "schedule.timezone", "IANA timezone name"),
                ],
                &mut issues,
            );
        }
        Some(Value::String(_)) => {
            issues.push(invalid_value("schedule.kind").expected("cron or once"));
        }
        Some(_) => issues.push(type_mismatch("schedule.kind", "string")),
    }

    if issues.is_empty() {
        issues.push(invalid_value("input").expected("valid trigger_create input"));
    }
    issues
}

fn schedule_variant_shape_issues(
    schedule: &serde_json::Map<String, Value>,
    allowed_fields: &[&str],
    required_strings: &[(&'static str, &'static str, &'static str)],
    issues: &mut Vec<DispatchInputIssue>,
) {
    unexpected_fields(
        schedule,
        allowed_fields,
        "schedule.unexpected_field",
        issues,
    );
    for (field, path, expected) in required_strings {
        required_string(schedule, field, path, expected, issues);
    }
}

fn unexpected_fields(
    object: &serde_json::Map<String, Value>,
    allowed: &[&str],
    path: &'static str,
    issues: &mut Vec<DispatchInputIssue>,
) {
    for field in object.keys() {
        if !allowed.contains(&field.as_str()) {
            issues.push(unexpected_field(path));
        }
    }
}

fn required_string(
    object: &serde_json::Map<String, Value>,
    field: &'static str,
    path: &'static str,
    expected: &'static str,
    issues: &mut Vec<DispatchInputIssue>,
) {
    match object.get(field) {
        None | Some(Value::Null) => issues.push(missing_required(path).expected(expected)),
        Some(Value::String(_)) => {}
        Some(_) => issues.push(type_mismatch(path, "string")),
    }
}

fn missing_required(path: impl Into<String>) -> DispatchInputIssue {
    DispatchInputIssue::new(path, DispatchInputIssueCode::MissingRequired)
}

fn unexpected_field(path: impl Into<String>) -> DispatchInputIssue {
    DispatchInputIssue::new(path, DispatchInputIssueCode::UnexpectedField)
}

fn type_mismatch(path: impl Into<String>, expected: &'static str) -> DispatchInputIssue {
    DispatchInputIssue::new(path, DispatchInputIssueCode::TypeMismatch).expected(expected)
}

fn invalid_value(path: impl Into<String>) -> DispatchInputIssue {
    DispatchInputIssue::new(path, DispatchInputIssueCode::InvalidValue)
}

fn invalid_trigger_input(issues: Vec<DispatchInputIssue>) -> FirstPartyCapabilityError {
    let issue_paths = issues
        .iter()
        .map(|issue| issue.path.as_str())
        .collect::<Vec<_>>();
    tracing::debug!(
        runtime_dispatch_error_kind = %RuntimeDispatchErrorKind::InputEncode,
        issue_count = issues.len(),
        issue_paths = ?issue_paths,
        "trigger management capability input validation failed"
    );
    FirstPartyCapabilityError::invalid_input_issues(
        "trigger_create input failed validation",
        issues,
    )
}

fn trigger_schedule_error(
    kind: TriggerScheduleInputKind,
    error: TriggerError,
) -> FirstPartyCapabilityError {
    let issue = match error {
        TriggerError::InvalidSchedule {
            kind: TriggerScheduleValidationKind::InvalidTimezone,
            ..
        } => invalid_value("schedule.timezone").expected("valid IANA timezone name"),
        TriggerError::InvalidSchedule { .. } => match kind {
            TriggerScheduleInputKind::Cron => invalid_value("schedule.expression")
                .expected("five-, six-, or seven-field cron with at least one-minute cadence"),
            TriggerScheduleInputKind::Once => invalid_value("schedule.at")
                .expected("YYYY-MM-DDTHH:MM:SS valid in the selected timezone"),
        },
        other => invalid_value("schedule").expected(trigger_error_kind(&other)),
    };
    invalid_trigger_input(vec![issue])
}

fn trigger_record_error(error: TriggerError) -> FirstPartyCapabilityError {
    match error {
        TriggerError::InvalidRecord {
            kind: TriggerRecordValidationKind::NameEmpty,
            ..
        } => invalid_trigger_input(vec![
            invalid_value("name").expected("non-empty trigger name"),
        ]),
        TriggerError::InvalidRecord {
            kind: TriggerRecordValidationKind::PromptEmpty,
            ..
        } => invalid_trigger_input(vec![
            invalid_value("prompt").expected("non-empty trigger prompt"),
        ]),
        TriggerError::InvalidRecord {
            kind: TriggerRecordValidationKind::NameTooLong,
            ..
        } => invalid_trigger_input(vec![
            invalid_value("name").expected("trigger name within the allowed byte limit"),
        ]),
        TriggerError::InvalidRecord {
            kind: TriggerRecordValidationKind::PromptTooLong,
            ..
        } => invalid_trigger_input(vec![
            invalid_value("prompt").expected("trigger prompt within the allowed byte limit"),
        ]),
        other => invalid_trigger_input(vec![
            invalid_value("trigger").expected(trigger_error_kind(&other)),
        ]),
    }
}

fn trigger_next_run_error(
    kind: TriggerScheduleInputKind,
    _error: TriggerError,
) -> FirstPartyCapabilityError {
    let issue = match kind {
        TriggerScheduleInputKind::Cron => invalid_value("schedule.expression")
            .expected("cron expression with at least one future fire time"),
        TriggerScheduleInputKind::Once => {
            invalid_value("schedule.at").expected("future local datetime")
        }
    };
    invalid_trigger_input(vec![issue])
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
            TriggerError::InvalidSchedule {
                kind: TriggerScheduleValidationKind::NoFutureFireTime,
                ..
            }
        ));
    }

    #[test]
    fn trigger_create_input_rejects_missing_timezone() {
        let input = serde_json::json!({
            "name": "daily",
            "prompt": "check mail",
            "schedule": { "kind": "cron", "expression": "0 9 * * *" }  // missing timezone
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
            "schedule": { "kind": "cron", "expression": "0 9 * * *", "timezone": "Not/A/Timezone" }
        });
        let parsed: TriggerCreateInput = serde_json::from_value(input).expect("deserialize");
        let result = parsed.schedule.into_schedule();
        assert!(result.is_err(), "invalid timezone must be rejected");
    }

    #[test]
    fn trigger_create_input_accepts_cron_schedule() {
        let input = serde_json::json!({
            "name": "daily",
            "prompt": "check mail",
            "schedule": { "kind": "cron", "expression": "0 9 * * *", "timezone": "America/Los_Angeles" }
        });
        let parsed: TriggerCreateInput = serde_json::from_value(input).expect("deserialize");
        let schedule = parsed
            .schedule
            .into_schedule()
            .expect("valid cron schedule accepted");
        match &schedule {
            TriggerSchedule::Cron { timezone, .. } => {
                assert_eq!(timezone, "America/Los_Angeles");
            }
            TriggerSchedule::Once { .. } => panic!("expected Cron"),
        }
    }

    #[test]
    fn trigger_create_input_rejects_missing_schedule() {
        let input = serde_json::json!({
            "name": "daily",
            "prompt": "check mail"
        });
        let result: Result<TriggerCreateInput, _> = serde_json::from_value(input);
        assert!(
            result.is_err(),
            "omitting schedule must fail deserialization"
        );
    }

    #[test]
    fn trigger_create_input_accepts_once_schedule_and_persists_as_utc() {
        // 2099-06-24T17:00:00 UTC is unambiguous and in the future
        let input = serde_json::json!({
            "name": "one-off reminder",
            "prompt": "remind me about the meeting",
            "schedule": { "kind": "once", "at": "2099-06-24T17:00:00", "timezone": "UTC" }
        });
        let parsed: TriggerCreateInput =
            serde_json::from_value(input).expect("deserialize one-shot input");
        let schedule = parsed
            .schedule
            .into_schedule()
            .expect("valid once schedule accepted");
        match &schedule {
            TriggerSchedule::Once { at, timezone } => {
                assert_eq!(timezone, "UTC");
                // Wall-clock 17:00:00 UTC → stored UTC timestamp must match
                assert_eq!(at.to_rfc3339(), "2099-06-24T17:00:00+00:00");
            }
            TriggerSchedule::Cron { .. } => panic!("expected Once"),
        }
    }

    #[test]
    fn trigger_create_input_rejects_dst_ambiguous_time() {
        // 2026-11-01T01:30:00 in America/New_York occurs twice (DST fall-back overlap)
        let input = serde_json::json!({
            "name": "ambiguous",
            "prompt": "test",
            "schedule": { "kind": "once", "at": "2026-11-01T01:30:00", "timezone": "America/New_York" }
        });
        let parsed: TriggerCreateInput = serde_json::from_value(input).expect("deserialize");
        let result = parsed.schedule.into_schedule();
        assert!(
            result.is_err(),
            "DST-ambiguous time must be rejected as input error"
        );
    }

    #[test]
    fn trigger_create_input_rejects_dst_gap_time() {
        // 2026-03-08T02:30:00 in America/New_York does not exist (DST spring-forward gap)
        let input = serde_json::json!({
            "name": "dst-gap",
            "prompt": "test",
            "schedule": { "kind": "once", "at": "2026-03-08T02:30:00", "timezone": "America/New_York" }
        });
        let parsed: TriggerCreateInput = serde_json::from_value(input).expect("deserialize");
        let result = parsed.schedule.into_schedule();
        assert!(
            result.is_err(),
            "DST-gap time must be rejected as input error"
        );
    }
}
