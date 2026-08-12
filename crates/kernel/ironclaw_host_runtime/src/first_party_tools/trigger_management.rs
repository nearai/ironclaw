use std::{collections::HashMap, sync::Arc, time::Instant};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ironclaw_extension_registry::{CapabilityManifest, ExtensionError};
use ironclaw_host_api::{
    capability::{EffectKind, PermissionMode},
    dispatch::{DispatchInputIssue, DispatchInputIssueCode, RuntimeDispatchErrorKind},
    error::HostApiError,
    ids::CapabilityId,
    invocation::InvocationOrigin,
    resource::{ResourceScope, ResourceUsage},
};
use ironclaw_triggers::{
    ACTIVE_HOLD_LOOKUP_TIMEOUT, ActiveHoldProjection, ActiveHoldReason,
    MissingTriggerActiveRunLookup, TriggerActiveRunLookup, TriggerError, TriggerId, TriggerRecord,
    TriggerRecordValidationKind, TriggerRepository, TriggerRunRecord, TriggerSchedule,
    TriggerScheduleValidationKind, TriggerSourceKind, TriggerState, active_holds_for_records,
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

/// Grounding description for the read path (issue #7246): the model was
/// observed fabricating automation status ("your digest routine is running")
/// while the Automations page showed none. Mirrors the outbound
/// targets-list pattern — the description states the positive
/// check-before-assert rule tied to the exact claims the model must not
/// fabricate, and bridges the user vocabulary ("automation", "routine") to
/// this trigger capability.
const TRIGGER_LIST_DESCRIPTION: &str = "List the caller's scheduled routines \u{2014} the automations shown on the Automations page \u{2014} with each routine's state (scheduled, paused, or completed), schedule, next and last fire times, recent run history, and any active hold. This listing is the authoritative current state. Call this before answering any question about which routines or automations exist, and before saying one is running, paused, already set up, delivering results, or missing \u{2014} never report routine or automation status from conversation history or memory. An empty list means the caller has no routines: say exactly that instead of guessing.";

const TRIGGER_CREATE_DESCRIPTION: &str = "Create a scheduled routine. The prompt is the full task each fire performs, written for a future run with no memory of this conversation. Where results go: a bare \"send me\" or \"notify me\" means the surface the user is asking from — never ask which channel. From a channel conversation, default to the channel this conversation is on: pick its target id from builtin__outbound_delivery_targets_list while the user is present and write that as an explicit step in the prompt naming the destination by pinned id (e.g. \"then deliver the summary with builtin__outbound_deliver to chat:team-dm\" — never a description like \"my DM\" that a fire would have to look up). From the web app with no external destination named, there is no delivery step to write: the fire's final reply IS the delivery — it lands in the routine's own run thread automatically — so end the prompt with the reply itself and write no delivery step. Only when the user explicitly asks to be notified in the browser or on their devices does the catalog's browser-push target apply: pin its target id like any other destination. When the user names an external destination (\"send me this in my messaging app\", \"post it to the team channel\"), that IS a delivery step even from the web app: pin its target id the same way — reaching the user or anyone else on an external surface always goes through builtin__outbound_deliver with a pinned target id, never through integration messaging tools, which act as the user toward other people. Several destinations mean one delivery step each; a fire that makes no delivery call delivers nothing externally.";

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
            TRIGGER_LIST_DESCRIPTION,
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
            "Resume a caller-scoped paused trigger so it may fire on its stored schedule. Use only when the user explicitly asks to resume or enable that routine; listing to avoid duplicates or to ensure exactly one routine exists is read-only and must not resume it.",
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
    // `NoopTriggerCreateHook` below (#5886).
    insert_handlers_with_create_hook(
        registry,
        repository,
        Arc::new(NoopTriggerCreateHook),
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
            repository,
            create_hook,
            clock: Arc::new(SystemTriggerManagementClock),
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
            repository,
            create_hook: Arc::new(NoopTriggerCreateHook),
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
    active_run_lookup: Arc<dyn TriggerActiveRunLookup>,
}

#[async_trait]
impl FirstPartyCapabilityHandler for TriggerManagementToolHandler {
    async fn dispatch(
        &self,
        request: FirstPartyCapabilityRequest,
    ) -> Result<FirstPartyCapabilityResult, FirstPartyCapabilityError> {
        // Defense-in-depth backstop (issue #5505): a scheduled/automation origin
        // must never create, remove, pause, or resume a routine — that is
        // self-referential automation that could silence or reschedule itself.
        //
        // The PRIMARY structural guarantees live one layer up, in the runner's
        // capability surface (`ironclaw_turn_runner::runtime`):
        //   * a scheduled-trigger fire runs on the `scheduled_trigger` surface
        //     profile, whose `resolved CapabilitySurfacePolicy`
        //     (`SCHEDULED_TRIGGER_DENIED_CAPABILITY_IDS`) strips the four mutation
        //     capabilities before the model can see them (`trigger_list` stays);
        //   * a subagent runs on the `subagent_tools` surface, whose per-flavor
        //     tool allowlist (`BUILTIN_SUBAGENT_FLAVORS`) never includes any
        //     trigger capability, so a subagent cannot reach these ids at all.
        //
        // This origin check is the belt to those suspenders: a caller that
        // reaches dispatch directly (bypassing the model-visible surface) is
        // still refused. Residual it cannot close on its own: a subagent spawned
        // from a scheduled run inherits `product_context: None`, so its dispatch
        // origin is `LoopRun`, not `ScheduledLoopRun` — the scheduled lineage is
        // not visible here. The subagent *surface* exclusion above is what
        // protects that path today (and `spawn_subagent` is globally disabled
        // pending #4147); if a trigger capability is ever added to a subagent
        // flavor, the parent's `ScheduledTrigger`/`ScheduledLoopRun` lineage must
        // also be propagated onto spawned runs so this backstop catches them too.
        if origin_forbids_routine_mutation(request.origin.as_ref())
            && is_trigger_mutation(request.capability_id.as_str())
        {
            return Err(FirstPartyCapabilityError::with_safe_summary(
                RuntimeDispatchErrorKind::PolicyDenied,
                "scheduled automation cannot mutate routines",
            ));
        }
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

fn is_trigger_mutation(capability_id: &str) -> bool {
    matches!(
        capability_id,
        TRIGGER_CREATE_CAPABILITY_ID
            | TRIGGER_REMOVE_CAPABILITY_ID
            | TRIGGER_PAUSE_CAPABILITY_ID
            | TRIGGER_RESUME_CAPABILITY_ID
    )
}

/// Origins that must never mutate routines. Both the model-initiated scheduled
/// loop-run ([`InvocationOrigin::ScheduledLoopRun`]) and the non-model
/// routine/heartbeat ([`InvocationOrigin::Automation`]) are refused: a scheduled
/// routine editing routines is self-referential automation. This matches the
/// builtin descriptors' declared `origin_gate_matrix`, which sets
/// `automation = Forbidden` for every trigger-mutation capability — so an
/// `Automation`-origin caller is already denied at the authorization gate; the
/// runtime backstop refuses it too, independent of that gate having run.
///
/// Interactive `LoopRun` and direct-user `Product` origins are intentionally
/// *not* here: creating a routine is a normal thing for an interactive turn or a
/// settings action to do (subject to the per-capability gate).
fn origin_forbids_routine_mutation(origin: Option<&InvocationOrigin>) -> bool {
    matches!(
        origin,
        Some(InvocationOrigin::ScheduledLoopRun(_) | InvocationOrigin::Automation(_))
    )
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
        // Retired stored routing (spec §8): a routine delivers externally only
        // by calling `builtin.outbound_deliver` from its own prompt, so nothing
        // here ever seals a delivery route again. The field survives only to
        // read pre-removal rows until the boot migration rewrites them.
        delivery_target: None,
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
    // #7246/#7474 review: `limit: 0` would return an empty list while routines
    // exist, and the description tells the model an empty list proves absence.
    // Reject zero (schema declares `minimum: 1`) so empty is always evidence.
    if input.limit == Some(0) {
        return Err(input_error());
    }
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
    // automations service so both read surfaces stay in lockstep (#5886).
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
/// capability's `active_hold` wire object — same shape the automations service
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
