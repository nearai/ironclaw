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
