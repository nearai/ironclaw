use chrono::{Datelike, TimeZone};

use super::*;

/// Duplicate-delivery contract: the stored trigger prompt is replayed to a
/// fresh model at fire time, so the description must teach that each
/// fire's final reply is delivered by the host (otherwise the fired model
/// both calls a messaging capability and emits a final reply, delivering
/// the result twice), while messaging-as-task automations ("send Firat a
/// joke every morning") stay expressible with recipients pinned at
/// creation time instead of guessed at fire time.
#[test]
fn trigger_create_description_teaches_task_only_prompt_and_host_owned_delivery() {
    assert!(
        TRIGGER_CREATE_DESCRIPTION.contains(
            "If delivery_target_id is set, never put a send, post, or deliver-results step"
        ),
        "trigger_create description must front-load the no-duplicate-delivery rule: {TRIGGER_CREATE_DESCRIPTION}"
    );
    assert!(
        TRIGGER_CREATE_DESCRIPTION.contains("delivered automatically"),
        "trigger_create description must state host-owned result delivery: {TRIGGER_CREATE_DESCRIPTION}"
    );
    assert!(
        TRIGGER_CREATE_DESCRIPTION.contains("full task each fire performs"),
        "trigger_create description must say the prompt is the task, not routing: {TRIGGER_CREATE_DESCRIPTION}"
    );
    assert!(
        TRIGGER_CREATE_DESCRIPTION
            .contains("Do not tell the prompt to send results back to the requesting user"),
        "trigger_create description must forbid result self-delivery phrasing in the stored prompt: {TRIGGER_CREATE_DESCRIPTION}"
    );
    assert!(
        TRIGGER_CREATE_DESCRIPTION.contains("resolved while the user is present"),
        "trigger_create description must require creation-time recipient pinning: {TRIGGER_CREATE_DESCRIPTION}"
    );
    // Laundering guard: a live QA fire executed a duplicate user-identity
    // send because the creating model set delivery_target_id AND pinned
    // the requester's own DM into the prompt as if it were a third-party
    // recipient. The description must say receiving results is routing,
    // never a prompt step — pinned conversation id or not.
    assert!(
        TRIGGER_CREATE_DESCRIPTION.contains("delivery routing, not a task step"),
        "trigger_create description must frame 'send me the result' as routing: {TRIGGER_CREATE_DESCRIPTION}"
    );
    assert!(
        TRIGGER_CREATE_DESCRIPTION.contains("even one with a pinned conversation id"),
        "trigger_create description must forbid laundering a self-send behind a pinned id: {TRIGGER_CREATE_DESCRIPTION}"
    );
    assert!(
        TRIGGER_CREATE_DESCRIPTION.contains("pass delivery_target_id with an id from")
            && TRIGGER_CREATE_DESCRIPTION.contains("builtin__outbound_delivery_targets_list"),
        "trigger_create description must teach per-trigger delivery routing: {TRIGGER_CREATE_DESCRIPTION}"
    );
    assert!(
        TRIGGER_CREATE_DESCRIPTION
            .contains("inherits the current source run's authorized delivery route")
            && TRIGGER_CREATE_DESCRIPTION.contains("only when no source route exists")
            && TRIGGER_CREATE_DESCRIPTION.contains("never prompt parsing"),
        "trigger_create description must explain trusted source-route inheritance and fallback: {TRIGGER_CREATE_DESCRIPTION}"
    );
}

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
    assert!(
        matches!(
            result,
            Err(TriggerError::InvalidSchedule {
                kind: TriggerScheduleValidationKind::InvalidTimezone,
                ..
            })
        ),
        "expected InvalidSchedule(InvalidTimezone) error, got {result:?}"
    );
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
        matches!(
            result,
            Err(TriggerError::InvalidSchedule {
                kind: TriggerScheduleValidationKind::AmbiguousDateTime,
                ..
            })
        ),
        "expected InvalidSchedule(AmbiguousDateTime) error, got {result:?}"
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
        matches!(
            result,
            Err(TriggerError::InvalidSchedule {
                kind: TriggerScheduleValidationKind::NonexistentDateTime,
                ..
            })
        ),
        "expected InvalidSchedule(NonexistentDateTime) error, got {result:?}"
    );
}

// -- active_hold projection (#5886) --------------------------------
//
// The reason/elapsed-occurrence derivation and lookup-batching contract
// itself is covered in `ironclaw_triggers::worker::ports::tests` (the
// owning crate); these tests cover only this capability's wire mapping and
// wiring into `active_holds_for_records`.

use ironclaw_triggers::{
    ActiveHoldReason, BlockedActiveRunKind, TriggerActiveRunState, active_hold_projection,
};

fn test_record(active_fire_slot: Option<DateTime<Utc>>) -> TriggerRecord {
    use ironclaw_host_api::{AgentId, ProjectId, TenantId, UserId};
    let now = Utc::now();
    TriggerRecord {
        trigger_id: TriggerId::new(),
        tenant_id: TenantId::new("tenant-alpha").expect("valid tenant"),
        creator_user_id: UserId::new("user-alpha").expect("valid user"),
        agent_id: Some(AgentId::new("agent-alpha").expect("valid agent")),
        project_id: Some(ProjectId::new("project-alpha").expect("valid project")),
        name: "daily".to_string(),
        source: TriggerSourceKind::Schedule,
        schedule: TriggerSchedule::Cron {
            expression: "0 9 * * *".to_string(),
            timezone: "UTC".to_string(),
        },
        prompt: "check mail".to_string(),
        delivery_target: None,
        state: TriggerState::Scheduled,
        next_run_at: now,
        last_run_at: None,
        last_fired_slot: None,
        last_status: None,
        active_fire_slot,
        active_run_ref: active_fire_slot.map(|_| ironclaw_turns::TurnRunId::new()),
        created_at: now,
    }
}

#[test]
fn active_hold_json_maps_blocked_approval() {
    let now = Utc::now();
    let record = test_record(Some(now - chrono::Duration::days(3)));
    let projection = active_hold_projection(
        &record,
        Some(TriggerActiveRunState::Blocked {
            kind: BlockedActiveRunKind::Approval,
        }),
        now,
    )
    .expect("blocked state yields a hold");
    let hold = active_hold_json(projection);
    assert_eq!(hold["reason"], "approval");
    assert_eq!(hold["since"], json!(record.active_fire_slot));
    assert!(hold["elapsed_occurrences"].as_u64().is_some());
}

#[test]
fn active_hold_json_maps_nonterminal_to_in_progress() {
    let now = Utc::now();
    let record = test_record(None);
    let projection = active_hold_projection(&record, Some(TriggerActiveRunState::Nonterminal), now)
        .expect("nonterminal state yields a hold");
    let hold = active_hold_json(projection);
    assert_eq!(hold["reason"], "in_progress");
    assert!(hold["since"].is_null());
    assert!(hold["elapsed_occurrences"].is_null());
}

#[test]
fn active_hold_json_maps_claimed_but_unaccepted_to_other() {
    // No `active_run_ref` yet (claimed but not accepted) — `run_state:
    // None` must resolve to `Other`, matching the shared derivation
    // contract (#5886).
    let now = Utc::now();
    let record = test_record(Some(now));
    let projection = active_hold_projection(&record, None, now)
        .expect("claimed-but-unaccepted fire yields a hold");
    assert_eq!(projection.reason, ActiveHoldReason::Other);
    assert_eq!(active_hold_json(projection)["reason"], "other");
}

#[test]
fn trigger_output_omits_active_hold_key_when_none() {
    let record = test_record(None);
    let output = trigger_output(&record, &[], None);
    assert!(output.get("active_hold").is_none());
}

#[test]
fn trigger_output_includes_active_hold_when_present() {
    let record = test_record(Some(Utc::now()));
    let hold = json!({"reason": "auth", "since": null, "elapsed_occurrences": null, "elapsed_occurrences_capped": false});
    let output = trigger_output(&record, &[], Some(hold));
    assert_eq!(output["active_hold"]["reason"], "auth");
}

// `active_holds_for_records`'s lookup-error degrade and
// claimed-but-unaccepted skip-lookup behavior are pinned directly against
// the shared function in `ironclaw_triggers::worker::ports::tests`
// (`active_holds_for_records_degrades_on_lookup_error` and
// `active_holds_for_records_skips_lookup_for_claimed_but_unaccepted`); no
// duplicate coverage here (#5886).

// -- routine self-mutation origin backstop (#5505) ------------------------
//
// Drives the real handler `dispatch` (the caller that gates the side effect),
// not `origin_forbids_routine_mutation` in isolation, per the "test through the
// caller" rule. The guard runs before input parsing, so the mutation-denial
// cases pass an empty body deliberately.

use std::sync::Mutex;

use ironclaw_host_api::{InvocationId, InvocationOrigin, ProductKind, RoutineId, RunId, UserId};
use ironclaw_triggers::{InMemoryTriggerRepository, TriggerDeliveryTargetId};

const MUTATION_CAPABILITIES: &[&str] = &[
    TRIGGER_CREATE_CAPABILITY_ID,
    TRIGGER_REMOVE_CAPABILITY_ID,
    TRIGGER_PAUSE_CAPABILITY_ID,
    TRIGGER_RESUME_CAPABILITY_ID,
];

fn once_create_input(name: &str) -> Value {
    json!({
        "name": name,
        "prompt": "remind me later",
        "schedule": {"kind": "once", "at": "2999-01-01T00:00:00", "timezone": "UTC"},
    })
}

fn origin_test_handler(create_hook: Arc<dyn TriggerCreateHook>) -> TriggerManagementToolHandler {
    TriggerManagementToolHandler {
        repository: Arc::new(InMemoryTriggerRepository::default()),
        create_hook,
        clock: Arc::new(SystemTriggerManagementClock),
        active_run_lookup: Arc::new(MissingTriggerActiveRunLookup),
    }
}

async fn dispatch_with_origin(
    handler: &TriggerManagementToolHandler,
    origin: Option<InvocationOrigin>,
    capability_id: &str,
    input: Value,
) -> Result<FirstPartyCapabilityResult, FirstPartyCapabilityError> {
    let scope = ResourceScope::local_default(
        UserId::new("trigger-origin-user").expect("user"),
        InvocationId::new(),
    )
    .expect("scope");
    let mut request = FirstPartyCapabilityRequest::request_for_test(
        CapabilityId::new(capability_id).expect("capability id"),
        scope,
        input,
        None,
    );
    request.origin = origin;
    handler.dispatch(request).await
}

fn assert_routine_mutation_denied(error: FirstPartyCapabilityError, capability_id: &str) {
    match error {
        FirstPartyCapabilityError::Dispatch {
            kind, safe_summary, ..
        } => {
            assert_eq!(
                kind,
                RuntimeDispatchErrorKind::PolicyDenied,
                "{capability_id} must be policy-denied"
            );
            assert_eq!(
                safe_summary.as_deref(),
                Some("scheduled automation cannot mutate routines"),
                "{capability_id} denial summary",
            );
        }
        other => panic!("expected policy-denied dispatch error for {capability_id}, got {other:?}"),
    }
}

#[tokio::test]
async fn scheduled_loop_run_origin_is_denied_every_routine_mutation() {
    let handler = origin_test_handler(Arc::new(NoopTriggerCreateHook));
    for capability_id in MUTATION_CAPABILITIES {
        let error = dispatch_with_origin(
            &handler,
            Some(InvocationOrigin::ScheduledLoopRun(RunId::new())),
            capability_id,
            json!({}),
        )
        .await
        .expect_err("a scheduled loop-run must not mutate routines");
        assert_routine_mutation_denied(error, capability_id);
    }
}

#[tokio::test]
async fn automation_origin_is_denied_every_routine_mutation() {
    // Broadened backstop: a routine/heartbeat `Automation` origin is refused too,
    // matching the descriptors' `automation = Forbidden` origin-gate matrix.
    let handler = origin_test_handler(Arc::new(NoopTriggerCreateHook));
    for capability_id in MUTATION_CAPABILITIES {
        let error = dispatch_with_origin(
            &handler,
            Some(InvocationOrigin::Automation(
                RoutineId::new("nightly").expect("routine"),
            )),
            capability_id,
            json!({}),
        )
        .await
        .expect_err("an automation origin must not mutate routines");
        assert_routine_mutation_denied(error, capability_id);
    }
}

#[tokio::test]
async fn interactive_and_product_origins_may_create_a_routine() {
    // The backstop must not be over-broad: an interactive loop turn and a direct
    // product action can still create a routine.
    for origin in [
        InvocationOrigin::LoopRun(RunId::new()),
        InvocationOrigin::Product(ProductKind::new("settings").expect("product")),
    ] {
        let handler = origin_test_handler(Arc::new(NoopTriggerCreateHook));
        let kind = origin.kind();
        let result = dispatch_with_origin(
            &handler,
            Some(origin),
            TRIGGER_CREATE_CAPABILITY_ID,
            once_create_input("allowed-origin-routine"),
        )
        .await
        .unwrap_or_else(|error| panic!("{kind} create must be allowed, got {error:?}"));
        assert_eq!(
            result.output["trigger"]["name"],
            json!("allowed-origin-routine"),
            "{kind} create should persist the routine"
        );
    }
}

#[tokio::test]
async fn scheduled_origin_may_still_list_routines() {
    // Read-only `trigger_list` is never denied — only the four mutations are.
    let handler = origin_test_handler(Arc::new(NoopTriggerCreateHook));
    let result = dispatch_with_origin(
        &handler,
        Some(InvocationOrigin::ScheduledLoopRun(RunId::new())),
        TRIGGER_LIST_CAPABILITY_ID,
        json!({}),
    )
    .await
    .expect("a scheduled origin may still list routines");
    assert!(
        result.output["triggers"].is_array(),
        "trigger_list must return a triggers array under a scheduled origin"
    );
}

// -- delivery-target precedence: explicit wins, implicit not consulted ------

/// A create hook that always accepts an explicit target and records whether the
/// implicit (source-run) resolver was consulted. The implicit path returns a
/// DIFFERENT id so a precedence regression would be observable in the persisted
/// record, not merely in the "was it called" flag.
#[derive(Default)]
struct ExplicitWinsCreateHook {
    implicit_consulted: Arc<Mutex<bool>>,
    validated_targets: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl TriggerCreateHook for ExplicitWinsCreateHook {
    async fn resolve_implicit_delivery_target(
        &self,
        _scope: &ResourceScope,
        _run_id: Option<RunId>,
    ) -> Result<Option<TriggerDeliveryTargetId>, TriggerError> {
        *self.implicit_consulted.lock().expect("implicit lock") = true;
        Ok(Some(
            ironclaw_triggers::parse_trigger_delivery_target_id("implicit-source-target")
                .expect("implicit target"),
        ))
    }

    async fn validate_delivery_target(
        &self,
        _scope: &ResourceScope,
        target: &TriggerDeliveryTargetId,
    ) -> Result<(), TriggerError> {
        self.validated_targets
            .lock()
            .expect("validated lock")
            .push(target.as_str().to_string());
        Ok(())
    }

    async fn after_trigger_persisted(&self, _record: &TriggerRecord) -> Result<(), TriggerError> {
        Ok(())
    }
}

#[tokio::test]
async fn explicit_delivery_target_wins_over_source_run_context() {
    // Both inputs present: an explicit `delivery_target_id` AND a source run
    // context (`run_id`/origin). The explicit target must win, and the implicit
    // source-run resolver must never be consulted (the precedence rule the
    // `resolve_delivery_target` default encodes, exercised through `dispatch`).
    let hook = Arc::new(ExplicitWinsCreateHook::default());
    let handler = origin_test_handler(hook.clone());
    let scope = ResourceScope::local_default(
        UserId::new("explicit-target-user").expect("user"),
        InvocationId::new(),
    )
    .expect("scope");
    let mut request = FirstPartyCapabilityRequest::request_for_test(
        CapabilityId::new(TRIGGER_CREATE_CAPABILITY_ID).expect("capability id"),
        scope,
        json!({
            "name": "explicit-wins",
            "prompt": "deliver here",
            "schedule": {"kind": "once", "at": "2999-01-01T00:00:00", "timezone": "UTC"},
            "delivery_target_id": "explicit-chosen-target",
        }),
        None,
    );
    let run_id = RunId::new();
    request.run_id = Some(run_id);
    request.origin = Some(InvocationOrigin::LoopRun(run_id));

    let result = handler
        .dispatch(request)
        .await
        .expect("explicit-target create succeeds");

    assert_eq!(
        result.output["trigger"]["delivery_target_id"],
        json!("explicit-chosen-target"),
        "the explicit delivery target must be persisted, not the implicit source target",
    );
    assert!(
        !*hook.implicit_consulted.lock().expect("implicit lock"),
        "the implicit source-run resolver must not be consulted when an explicit target is given",
    );
    assert_eq!(
        hook.validated_targets
            .lock()
            .expect("validated lock")
            .as_slice(),
        &["explicit-chosen-target".to_string()],
        "only the explicit target is validated",
    );
}
