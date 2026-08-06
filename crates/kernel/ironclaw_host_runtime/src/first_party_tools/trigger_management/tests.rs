use chrono::{Datelike, TimeZone};

use super::*;

/// Delivery is now a step the *prompt* owns, not a stored routing field: a
/// routine's fire delivers externally only by calling
/// `builtin__outbound_deliver` itself. The description must therefore teach
/// (a) the prompt is the whole task, written for a memory-less future run,
/// (b) any wanted delivery is an explicit prompt step naming its destination,
/// picked while the user is present, and (c) a fire that makes no delivery
/// call delivers nothing externally — its reply only lands in the routine's
/// own run thread. It must NOT resurrect the retired `delivery_target_id`
/// input, which no longer exists on this capability.
#[test]
fn trigger_create_description_teaches_prompt_owned_delivery_with_no_stored_target() {
    assert!(
        TRIGGER_CREATE_DESCRIPTION.contains("full task each fire performs"),
        "trigger_create description must say the prompt is the whole task: {TRIGGER_CREATE_DESCRIPTION}"
    );
    assert!(
        TRIGGER_CREATE_DESCRIPTION.contains("no memory of this conversation"),
        "trigger_create description must say the fire has no memory of this conversation: {TRIGGER_CREATE_DESCRIPTION}"
    );
    assert!(
        TRIGGER_CREATE_DESCRIPTION
            .contains("write that as an explicit step in the prompt naming the destination"),
        "trigger_create description must make delivery an explicit prompt step: {TRIGGER_CREATE_DESCRIPTION}"
    );
    assert!(
        TRIGGER_CREATE_DESCRIPTION.contains("builtin__outbound_deliver"),
        "trigger_create description must name the delivery tool the prompt should call: {TRIGGER_CREATE_DESCRIPTION}"
    );
    assert!(
        TRIGGER_CREATE_DESCRIPTION.contains("builtin__outbound_delivery_targets_list")
            && TRIGGER_CREATE_DESCRIPTION.contains("while the user is present"),
        "trigger_create description must require destinations be picked at creation time: {TRIGGER_CREATE_DESCRIPTION}"
    );
    assert!(
        TRIGGER_CREATE_DESCRIPTION
            .contains("a fire that makes no delivery call delivers nothing externally"),
        "trigger_create description must state the no-call/no-delivery rule: {TRIGGER_CREATE_DESCRIPTION}"
    );
    // The source-channel default (the agreed routing UX): a bare "send me X"
    // asked from a channel conversation means that channel — the description
    // must state the default so the model pins the origin channel's target
    // rather than leaving bare requests destination-less.
    assert!(
        TRIGGER_CREATE_DESCRIPTION.contains("default to the channel this conversation is on"),
        "trigger_create description must pin the source-channel default: {TRIGGER_CREATE_DESCRIPTION}"
    );
    // The retired stored-target field: a create call carrying it is now a
    // rejected unexpected field, so the description must never advertise it.
    assert!(
        !TRIGGER_CREATE_DESCRIPTION.contains("delivery_target_id"),
        "trigger_create description must not advertise the retired stored delivery target: {TRIGGER_CREATE_DESCRIPTION}"
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
    use ironclaw_host_api::ids::{AgentId, ProjectId, TenantId, UserId};
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

use ironclaw_host_api::{
    ids::{InvocationId, ProductKind, RoutineId, RunId, UserId},
    invocation::InvocationOrigin,
};
use ironclaw_triggers::InMemoryTriggerRepository;

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

// -- retired stored delivery target -----------------------------------------

#[tokio::test]
async fn trigger_create_rejects_the_retired_delivery_target_id_input() {
    // Routines no longer carry a stored delivery route: delivery is a step the
    // prompt performs by calling `builtin__outbound_deliver`. A create call
    // that still passes the retired field must be refused as an unexpected
    // field (never silently accepted-and-ignored, which would leave the caller
    // believing a route was sealed), and nothing may be persisted.
    let handler = origin_test_handler(Arc::new(NoopTriggerCreateHook));
    let run_id = RunId::new();
    let error = dispatch_with_origin(
        &handler,
        Some(InvocationOrigin::LoopRun(run_id)),
        TRIGGER_CREATE_CAPABILITY_ID,
        json!({
            "name": "retired-target-field",
            "prompt": "deliver here",
            "schedule": {"kind": "once", "at": "2999-01-01T00:00:00", "timezone": "UTC"},
            "delivery_target_id": "slack:personal-dm:T123:user-a",
        }),
    )
    .await
    .expect_err("a create carrying the retired stored-target field must be refused");

    match error {
        FirstPartyCapabilityError::Dispatch {
            kind,
            detail: Some(detail),
            ..
        } => {
            assert_eq!(
                kind,
                RuntimeDispatchErrorKind::InputEncode,
                "the retired field must fail input validation, not dispatch"
            );
            let ironclaw_host_api::dispatch::DispatchFailureDetail::InvalidInput { issues } =
                *detail
            else {
                panic!("expected an invalid-input detail, got {detail:?}");
            };
            assert!(
                issues.iter().any(|issue| issue.path == "unexpected_field"
                    && issue.code == DispatchInputIssueCode::UnexpectedField),
                "expected an unexpected_field issue for delivery_target_id, got {issues:?}"
            );
        }
        other => panic!("expected an invalid-input dispatch error, got {other:?}"),
    }

    let listed = dispatch_with_origin(
        &handler,
        Some(InvocationOrigin::LoopRun(run_id)),
        TRIGGER_LIST_CAPABILITY_ID,
        json!({}),
    )
    .await
    .expect("list after the rejected create");
    assert_eq!(
        listed.output["triggers"],
        json!([]),
        "a rejected create must persist nothing"
    );
}
