use std::{collections::HashMap, sync::Arc, time::Instant};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ironclaw_extensions::{CapabilityManifest, ExtensionError};
use ironclaw_host_api::{
    CapabilityId, DispatchInputIssue, DispatchInputIssueCode, EffectKind, HostApiError,
    PermissionMode, ResourceScope, ResourceUsage, RuntimeDispatchErrorKind,
};
use ironclaw_triggers::{
    ACTIVE_HOLD_LOOKUP_TIMEOUT, ActiveHoldProjection, ActiveHoldReason,
    MissingTriggerActiveRunLookup, NoopTriggerCreationLifecycle, SystemTriggerCreationClock,
    TriggerActiveRunLookup, TriggerCreateRequest, TriggerCreateSchedule, TriggerCreateScheduleKind,
    TriggerCreationClock, TriggerCreationError, TriggerCreationService, TriggerError, TriggerId,
    TriggerRecord, TriggerRecordValidationKind, TriggerRepository, TriggerRunRecord,
    TriggerScheduleValidationKind, TriggerState, active_holds_for_records,
};
use serde::Deserialize;
use serde_json::{Value, json};

#[cfg(test)]
use ironclaw_triggers::{TriggerSchedule, TriggerSourceKind};

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

pub use ironclaw_triggers::TriggerCreateLifecycle as TriggerCreateHook;
#[cfg(any(test, feature = "test-support"))]
pub use ironclaw_triggers::TriggerCreationClock as TriggerManagementClock;

const TRIGGER_CREATE_DESCRIPTION: &str = "Create a caller-scoped scheduled trigger (one-time or recurring). The prompt is the full task each fire performs. If delivery_target_id is set, never put a send, post, or deliver-results step for that result in the prompt; each fire's final reply is delivered automatically to that target. Do not tell the prompt to send results back to the requesting user. Asks like 'send me the result' are delivery routing, not a task step: pass delivery_target_id with an id from builtin__outbound_delivery_targets_list and keep every send-to-requester step, even one with a pinned conversation id, out of the prompt. Put messaging in the prompt only when messaging someone else is itself the task; pin that third-party recipient, resolved while the user is present. Without delivery_target_id, the user's default outbound target applies at fire time; builtin__outbound_delivery_target_set changes that user-wide default.";

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
    // Compatibility wrapper: supplies `MissingTriggerActiveRunLookup`, so
    // callers through this path never project an `active_hold`, mirroring
    // `NoopTriggerCreationLifecycle` below (#5886).
    insert_handlers_with_create_hook(
        registry,
        repository,
        Arc::new(NoopTriggerCreationLifecycle),
        Arc::new(MissingTriggerActiveRunLookup),
    )
}

pub(super) fn insert_handlers_with_create_hook(
    registry: &mut FirstPartyCapabilityRegistry,
    repository: Arc<dyn TriggerRepository>,
    create_hook: Arc<dyn TriggerCreateHook>,
    active_run_lookup: Arc<dyn TriggerActiveRunLookup>,
) -> Result<(), HostApiError> {
    insert_trigger_handlers(
        registry,
        Arc::new(TriggerManagementToolHandler {
            creation_service: TriggerCreationService::new(Arc::clone(&repository), create_hook),
            repository,
            clock: Arc::new(SystemTriggerCreationClock),
            active_run_lookup,
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
            creation_service: TriggerCreationService::with_clock(
                Arc::clone(&repository),
                Arc::new(NoopTriggerCreationLifecycle),
                Arc::clone(&clock),
            ),
            repository,
            clock,
            active_run_lookup: Arc::new(MissingTriggerActiveRunLookup),
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

struct TriggerManagementToolHandler {
    creation_service: TriggerCreationService,
    repository: Arc<dyn TriggerRepository>,
    clock: Arc<dyn TriggerCreationClock>,
    active_run_lookup: Arc<dyn TriggerActiveRunLookup>,
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
                create_trigger(&self.creation_service, &request.scope, request.input).await?
            }
            TRIGGER_LIST_CAPABILITY_ID => {
                list_triggers(
                    &*self.repository,
                    &*self.active_run_lookup,
                    &request.scope,
                    request.input,
                    self.clock.now(),
                )
                .await?
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

impl TriggerScheduleInput {
    fn into_create_schedule(self) -> TriggerCreateSchedule {
        match self {
            Self::Cron {
                expression,
                timezone,
            } => TriggerCreateSchedule::Cron {
                expression,
                timezone,
            },
            Self::Once { at, timezone } => TriggerCreateSchedule::Once { at, timezone },
        }
    }

    #[cfg(test)]
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TriggerCreateInput {
    name: String,
    prompt: String,
    schedule: TriggerScheduleInput,
    /// Optional per-trigger outbound delivery target id (from the outbound
    /// delivery target capabilities). Host-validated before persistence.
    #[serde(default)]
    delivery_target_id: Option<String>,
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
    creation_service: &TriggerCreationService,
    scope: &ResourceScope,
    input: Value,
) -> Result<Value, FirstPartyCapabilityError> {
    let input: TriggerCreateInput = TriggerCreateInput::deserialize(&input)
        .map_err(|error| trigger_create_shape_error(&input, error))?;
    let delivery_target = match input.delivery_target_id {
        Some(raw) => Some(
            ironclaw_triggers::TriggerDeliveryTargetId::new(raw).map_err(trigger_record_error)?,
        ),
        None => None,
    };
    let record = creation_service
        .create(TriggerCreateRequest {
            scope: scope.clone(),
            name: input.name,
            prompt: input.prompt,
            schedule: input.schedule.into_create_schedule(),
            delivery_target,
        })
        .await
        .map_err(map_trigger_creation_error)?;
    Ok(json!({
        "trigger": trigger_output(&record, &[], None),
    }))
}

async fn list_triggers(
    repository: &dyn TriggerRepository,
    active_run_lookup: &dyn TriggerActiveRunLookup,
    scope: &ResourceScope,
    input: Value,
    now: DateTime<Utc>,
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
    // Reason/elapsed-occurrence derivation and lookup batching live in
    // `ironclaw_triggers::active_holds_for_records`, shared with the
    // automations facade so both read surfaces stay in lockstep (#5886).
    let mut holds: HashMap<TriggerId, Value> =
        active_holds_for_records(active_run_lookup, &records, now, ACTIVE_HOLD_LOOKUP_TIMEOUT)
            .await
            .into_iter()
            .map(|(trigger_id, hold)| (trigger_id, active_hold_json(hold)))
            .collect();
    let output = records
        .into_iter()
        .map(|record| {
            let runs = runs_by_trigger
                .remove(&record.trigger_id)
                .unwrap_or_default();
            let hold = holds.remove(&record.trigger_id);
            trigger_output(&record, &runs, hold)
        })
        .collect::<Vec<_>>();
    Ok(json!({ "triggers": output }))
}

/// Maps the crate-neutral hold projection (`ironclaw_triggers`) to this
/// capability's `active_hold` wire object — same shape the automations facade
/// maps to `RebornAutomationActiveHold`, just JSON instead of a typed DTO
/// (#5886).
fn active_hold_json(hold: ActiveHoldProjection) -> Value {
    let reason = match hold.reason {
        ActiveHoldReason::Approval => "approval",
        ActiveHoldReason::Auth => "auth",
        ActiveHoldReason::InProgress => "in_progress",
        ActiveHoldReason::Other => "other",
    };
    json!({
        "reason": reason,
        "since": hold.since,
        "elapsed_occurrences": hold.elapsed_occurrences,
        "elapsed_occurrences_capped": hold.elapsed_occurrences_capped,
    })
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
    let input: TriggerStateInput = serde_json::from_value(input).map_err(|error| {
        tracing::debug!(%error, "failed to deserialize trigger state input");
        input_error()
    })?;
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
        "trigger": updated.as_ref().map(|record| trigger_output(record, &[], None)),
    }))
}

fn trigger_output(
    record: &TriggerRecord,
    recent_runs: &[TriggerRunRecord],
    active_hold: Option<Value>,
) -> Value {
    let is_enabled = record.state == TriggerState::Scheduled;
    let has_active_fire = record.has_active_fire();
    let mut output = json!({
        "trigger_id": record.trigger_id.to_string(),
        "agent_id": record.agent_id.as_ref().map(|id| id.as_str()),
        "project_id": record.project_id.as_ref().map(|id| id.as_str()),
        "name": record.name,
        "source": record.source,
        "schedule": record.schedule,
        "delivery_target_id": record.delivery_target.as_ref().map(|target| target.as_str()),
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
    });
    // `active_hold` is omitted entirely (not null) when there is no live hold
    // to report — Missing/Terminal active-run states and lookup failures both
    // resolve to `None` upstream (#5886).
    if let Some(hold) = active_hold {
        output["active_hold"] = hold;
    }
    output
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
    if let Some(value) = root.get("delivery_target_id")
        && !value.is_null()
        && !value.is_string()
    {
        issues.push(type_mismatch("delivery_target_id", "string"));
    }
    unexpected_fields(
        root,
        &["name", "prompt", "schedule", "delivery_target_id"],
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
    kind: TriggerCreateScheduleKind,
    error: TriggerError,
) -> FirstPartyCapabilityError {
    let issue = match error {
        TriggerError::InvalidSchedule {
            kind: TriggerScheduleValidationKind::InvalidTimezone,
            ..
        } => invalid_value("schedule.timezone").expected("valid IANA timezone name"),
        TriggerError::InvalidSchedule { .. } => match kind {
            TriggerCreateScheduleKind::Cron => invalid_value("schedule.expression")
                .expected("five-, six-, or seven-field cron with at least one-minute cadence"),
            TriggerCreateScheduleKind::Once => invalid_value("schedule.at").expected(
                "YYYY-MM-DDTHH:MM:SS or RFC3339 with an offset matching the selected timezone",
            ),
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
        TriggerError::InvalidRecord {
            kind: TriggerRecordValidationKind::DeliveryTargetInvalid,
            ..
        } => invalid_trigger_input(vec![invalid_value("delivery_target_id").expected(
            "an outbound delivery target id available to this caller (from \
             builtin__outbound_delivery_targets_list)",
        )]),
        other => invalid_trigger_input(vec![
            invalid_value("trigger").expected(trigger_error_kind(&other)),
        ]),
    }
}

fn trigger_next_run_error(
    kind: TriggerCreateScheduleKind,
    _error: TriggerError,
) -> FirstPartyCapabilityError {
    let issue = match kind {
        TriggerCreateScheduleKind::Cron => invalid_value("schedule.expression")
            .expected("cron expression with at least one future fire time"),
        TriggerCreateScheduleKind::Once => {
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
    lifecycle_error: TriggerError,
    error: TriggerError,
) -> FirstPartyCapabilityError {
    tracing::warn!(
        runtime_dispatch_error_kind = %RuntimeDispatchErrorKind::Backend,
        repository_operation,
        lifecycle_error_kind = trigger_error_kind(&lifecycle_error),
        trigger_error_kind = trigger_error_kind(&error),
        error_kind = "trigger_create_rollback_failed",
        "trigger management capability create hook rollback failed"
    );
    FirstPartyCapabilityError::with_safe_summary(
        RuntimeDispatchErrorKind::Backend,
        "trigger create rollback failed after hook error",
    )
}

fn map_trigger_creation_error(error: TriggerCreationError) -> FirstPartyCapabilityError {
    match error {
        TriggerCreationError::InvalidSchedule { kind, source } => {
            trigger_schedule_error(kind, source)
        }
        TriggerCreationError::NoFutureFireTime { kind, source } => {
            trigger_next_run_error(kind, source)
        }
        TriggerCreationError::InvalidDeliveryTarget { source }
        | TriggerCreationError::InvalidRecord { source } => trigger_record_error(source),
        TriggerCreationError::Repository { operation, source } => {
            trigger_repository_error(operation, source)
        }
        TriggerCreationError::Lifecycle { operation, source } => {
            trigger_create_hook_error(operation, source)
        }
        TriggerCreationError::Rollback {
            operation,
            lifecycle_error,
            source,
        } => trigger_create_rollback_error(operation, lifecycle_error, source),
    }
}

fn trigger_error_kind(error: &TriggerError) -> &'static str {
    match error {
        TriggerError::InvalidTriggerId { .. } => "invalid_trigger_id",
        TriggerError::InvalidFireIdentityComponent { .. } => "invalid_fire_identity_component",
        TriggerError::InvalidRecord { .. } => "invalid_record",
        TriggerError::InvalidPollerConfig { .. } => "invalid_poller_config",
        TriggerError::InvalidSchedule { .. } => "invalid_schedule",
        TriggerError::InvalidMaterialization { .. } => "invalid_materialization",
        TriggerError::BlockedMaterialization { .. } => "blocked_materialization",
        TriggerError::Backend { .. } => "backend",
        TriggerError::NotFound => "not_found",
    }
}

fn elapsed_usage_with_bytes(started: Instant, output_bytes: u64) -> ResourceUsage {
    ResourceUsage::default()
        .set_wall_clock_ms(started.elapsed().as_millis().try_into().unwrap_or(u64::MAX))
        .set_output_bytes(output_bytes)
}

#[cfg(test)]
mod tests;
