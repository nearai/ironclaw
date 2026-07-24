use chrono::{Datelike, TimeZone};
use serde_json::{from_value, json, to_value};

use super::*;

fn ts(seconds: i64) -> Timestamp {
    Utc.timestamp_opt(seconds, 0)
        .single()
        .expect("valid timestamp")
}

fn tenant(value: &str) -> TenantId {
    TenantId::new(value).expect("valid tenant")
}

fn poison_in_memory_repo(repo: &InMemoryTriggerRepository) {
    let poison_repo = repo.clone();
    let _ = std::panic::catch_unwind(move || {
        let _guard = poison_repo.state.lock().expect("lock before poison");
        panic!("poison trigger repository mutex");
    });
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
        prompt: "summarize unread mail".to_string(),
        delivery_target: None,
        state: TriggerState::Scheduled,
        next_run_at,
        last_run_at: None,
        last_fired_slot: None,
        last_status: None,
        active_fire_slot: None,
        active_run_ref: None,
        created_at: ts(1_704_067_200),
    }
}

#[test]
fn cron_schedule_accepts_minute_cadence_and_computes_next_slot() {
    let schedule = TriggerSchedule::cron("*/5 * * * *").expect("minute cadence is valid");
    let next = schedule
        .next_slot_after(Utc.with_ymd_and_hms(2026, 5, 30, 12, 3, 0).unwrap())
        .expect("next slot")
        .expect("future slot");
    assert_eq!(next, Utc.with_ymd_and_hms(2026, 5, 30, 12, 5, 0).unwrap());
}

#[test]
fn cron_schedule_rejects_exhausted_finite_year() {
    let past_year = Utc::now().year() - 1;
    let error = TriggerSchedule::cron(format!("0 0 8 * * * {past_year}"))
        .expect_err("exhausted finite cron rejected");
    assert!(
        error
            .to_string()
            .contains("cron expression has no upcoming fire time"),
        "unexpected error: {error}"
    );
}

#[test]
fn cron_schedule_rejects_wrong_field_count() {
    let error = TriggerSchedule::cron("0 8 * *").expect_err("cron field count rejected");
    assert!(
        error
            .to_string()
            .contains("expected 5, 6, or 7 cron fields"),
        "unexpected error: {error}"
    );
}

#[test]
fn trigger_id_parse_rejects_invalid_ulid() {
    let error = TriggerId::parse("not-a-ulid").expect_err("malformed ulid rejected");
    assert!(
        error.to_string().contains("invalid trigger id"),
        "unexpected error: {error}"
    );
}

#[test]
fn public_fire_id_wrappers_validate_hex_accessors_and_serde_round_trip() {
    let route_value = "a".repeat(64);
    let event_value = "b".repeat(64);
    let route = TriggerRouteThreadId::new(route_value.clone()).expect("valid route id");
    let event = TriggerExternalEventId::new(event_value.clone()).expect("valid event id");

    assert_eq!(route.as_str(), route_value);
    assert_eq!(route.as_ref(), route_value);
    assert_eq!(route.to_string(), route_value);
    assert_eq!(event.as_str(), event_value);
    assert_eq!(event.as_ref(), event_value);
    assert_eq!(event.to_string(), event_value);
    assert!(TriggerRouteThreadId::new("route-1").is_err());
    assert!(TriggerExternalEventId::new("event-1").is_err());
    assert_eq!(to_value(&route).unwrap(), json!(route_value));
    assert_eq!(to_value(&event).unwrap(), json!(event_value));
    assert_eq!(
        from_value::<TriggerRouteThreadId>(json!(route_value)).unwrap(),
        route
    );
    assert_eq!(
        from_value::<TriggerExternalEventId>(json!(event_value)).unwrap(),
        event
    );
    assert!(matches!(
        TriggerRouteThreadId::new("route-1"),
        Err(TriggerError::InvalidFireIdentityComponent { .. })
    ));
    assert!(matches!(
        TriggerExternalEventId::new("event-1"),
        Err(TriggerError::InvalidFireIdentityComponent { .. })
    ));
}

#[test]
fn cron_schedule_rejects_sub_minute_seconds_fields() {
    for expression in [
        "*/30 * * * * *",
        "1 * * * * *",
        "0/15 * * * * * *",
        "00/15 * * * * *",
    ] {
        let error = TriggerSchedule::cron(expression).expect_err("sub-minute cron rejected");
        assert!(
            error.to_string().contains("second-level cadence"),
            "unexpected error: {error}"
        );
    }
}

#[test]
fn cron_schedule_accepts_zero_and_zero_padded_seconds_fields() {
    for expression in ["0 0 * * * *", "00 0 * * * *"] {
        TriggerSchedule::cron(expression).expect("zero seconds accepted");
    }
}

#[test]
fn cron_schedule_accepts_far_future_recurring_dates() {
    TriggerSchedule::cron("0 8 31 12 *").expect("annual schedule accepted");
}

#[test]
fn trigger_enums_serialize_as_snake_case() {
    assert_eq!(
        to_value(TriggerSourceKind::Schedule).unwrap(),
        json!("schedule")
    );
    assert_eq!(
        to_value(TriggerState::Scheduled).unwrap(),
        json!("scheduled")
    );
    assert_eq!(to_value(TriggerRunStatus::Ok).unwrap(), json!("ok"));
    assert_eq!(
        from_value::<TriggerRunStatus>(json!("error")).unwrap(),
        TriggerRunStatus::Error
    );
    assert!(from_value::<TriggerRunStatus>(json!("timed_out")).is_err());
    assert!(from_value::<TriggerRunStatus>(json!("approval_blocked")).is_err());
}

#[test]
fn fire_identity_is_stable_domain_separated_and_tenant_scoped() {
    let trigger_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
    let slot = Utc.with_ymd_and_hms(2026, 5, 30, 8, 0, 0).unwrap();
    let first = TriggerFireIdentity::new(tenant("tenant-a"), trigger_id, slot);
    let second = TriggerFireIdentity::new(tenant("tenant-a"), trigger_id, slot);
    let other_slot = TriggerFireIdentity::new(
        tenant("tenant-a"),
        trigger_id,
        slot + chrono::Duration::minutes(1),
    );
    let other_tenant = TriggerFireIdentity::new(tenant("tenant-b"), trigger_id, slot);

    assert_eq!(first, second);
    assert_ne!(
        first.route_thread_id.as_str(),
        first.external_event_id.as_str()
    );
    assert_ne!(first.route_thread_id, other_slot.route_thread_id);
    assert_ne!(first.external_event_id, other_slot.external_event_id);
    assert_ne!(first.route_thread_id, other_tenant.route_thread_id);
}

#[test]
fn fire_identity_length_prefixing_avoids_component_boundary_collisions() {
    let slot = Utc.with_ymd_and_hms(2026, 5, 30, 8, 0, 0).unwrap();
    let trigger_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
    let similar_trigger_id = TriggerId::parse("01J00000000000000000000000").expect("ulid");
    let short_tenant = TriggerFireIdentity::new(tenant("ab"), trigger_id, slot);
    let prefix_tenant = TriggerFireIdentity::new(tenant("a"), similar_trigger_id, slot);

    assert_ne!(short_tenant.route_thread_id, prefix_tenant.route_thread_id);
    assert_eq!(short_tenant.route_thread_id.as_str().len(), 64);
    assert_eq!(short_tenant.external_event_id.as_str().len(), 64);
}

#[tokio::test]
async fn schedule_provider_emits_due_fire_only() {
    let trigger_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
    let mut record = sample_record(trigger_id, tenant("tenant-a"), ts(1_704_067_200));
    record.delivery_target = Some(
        TriggerDeliveryTargetId::new("slack:personal-dm:T123:user-a").expect("delivery target"),
    );
    let provider = ScheduleTriggerSourceProvider;

    assert!(
        provider
            .evaluate(&record, ts(1_704_067_199))
            .await
            .expect("not due")
            .is_none()
    );
    let fire = provider
        .evaluate(&record, ts(1_704_067_200))
        .await
        .expect("due")
        .expect("fire");
    assert_eq!(fire.identity.trigger_id, trigger_id);
    assert_eq!(fire.identity.fire_slot, record.next_run_at);
    assert_eq!(fire.prompt, record.prompt);
    // Per-trigger delivery routing must survive record -> fire so the
    // delivery layer can honor it without re-reading the record.
    assert_eq!(fire.delivery_target, record.delivery_target);
}

#[test]
fn trigger_delivery_target_id_is_opaque_and_validated() {
    let target = TriggerDeliveryTargetId::new("slack:personal-dm:T123:user-a")
        .expect("valid delivery target id");
    assert_eq!(target.as_str(), "slack:personal-dm:T123:user-a");
    assert_eq!(
        to_value(&target).unwrap(),
        json!("slack:personal-dm:T123:user-a")
    );
    assert_eq!(
        from_value::<TriggerDeliveryTargetId>(json!("slack:personal-dm:T123:user-a")).unwrap(),
        target
    );
    assert!(TriggerDeliveryTargetId::new("x".repeat(512)).is_ok());

    assert!(parse_trigger_delivery_target_id("").is_err());
    assert!(parse_trigger_delivery_target_id(" target").is_err());
    assert!(parse_trigger_delivery_target_id("target\nid").is_err());
    assert!(parse_trigger_delivery_target_id("target:\u{200b}hidden").is_err());
    assert!(parse_trigger_delivery_target_id("x".repeat(513)).is_err());
    assert!(from_value::<TriggerDeliveryTargetId>(json!("")).is_err());
}

#[test]
fn trigger_inbound_content_ref_is_opaque_validated_materialization_output() {
    let content_ref =
        TriggerInboundContentRef::new("content:trigger-fire-01").expect("valid content ref");

    assert_eq!(content_ref.as_str(), "content:trigger-fire-01");
    assert_eq!(content_ref.as_ref(), "content:trigger-fire-01");
    assert_eq!(content_ref.to_string(), "content:trigger-fire-01");
    assert_eq!(
        to_value(&content_ref).unwrap(),
        json!("content:trigger-fire-01")
    );
    assert_eq!(
        from_value::<TriggerInboundContentRef>(json!("content:trigger-fire-01")).unwrap(),
        content_ref
    );
    assert!(TriggerInboundContentRef::new("x".repeat(512)).is_ok());

    assert!(TriggerInboundContentRef::new("").is_err());
    assert!(TriggerInboundContentRef::new("content:\ntrigger").is_err());
    assert!(TriggerInboundContentRef::new("x".repeat(513)).is_err());

    assert!(from_value::<TriggerInboundContentRef>(json!("")).is_err());
    assert!(from_value::<TriggerInboundContentRef>(json!("content:\ntrigger")).is_err());
    assert!(from_value::<TriggerInboundContentRef>(json!("x".repeat(513))).is_err());
}

#[tokio::test]
async fn prompt_materializer_port_receives_fire_and_returns_materialized_prompt() {
    struct RecordingMaterializer;

    #[async_trait]
    impl TriggerPromptMaterializer for RecordingMaterializer {
        async fn materialize_prompt(
            &self,
            fire: TriggerFire,
        ) -> Result<TriggerMaterializedPrompt, TriggerError> {
            assert_eq!(fire.creator_user_id, user("user-a"));
            assert_eq!(fire.agent_id, Some(AgentId::new("agent-a").unwrap()));
            assert_eq!(fire.project_id, Some(ProjectId::new("project-a").unwrap()));
            assert_eq!(fire.prompt, "summarize unread mail");
            let content_ref = TriggerInboundContentRef::new(format!(
                "content:{}",
                fire.identity.external_event_id
            ))?;
            Ok(TriggerMaterializedPrompt::for_fire(&fire, content_ref))
        }
    }

    let trigger_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
    let record = sample_record(trigger_id, tenant("tenant-a"), ts(1_704_067_200));
    let fire = ScheduleTriggerSourceProvider
        .evaluate(&record, ts(1_704_067_200))
        .await
        .expect("due")
        .expect("fire");

    let materialized = RecordingMaterializer
        .materialize_prompt(fire.clone())
        .await
        .expect("materialized");

    assert_eq!(
        materialized.content_ref().as_str(),
        format!("content:{}", fire.identity.external_event_id)
    );
    assert_eq!(
        materialized.trusted_inbound_binding().external_event_id(),
        fire.identity.external_event_id().as_str()
    );
}

#[tokio::test]
async fn schedule_provider_uses_state_as_fire_gate() {
    let trigger_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
    let mut record = sample_record(trigger_id, tenant("tenant-a"), ts(1_704_067_200));
    let provider = ScheduleTriggerSourceProvider;

    assert!(
        provider
            .evaluate(&record, ts(1_704_067_200))
            .await
            .expect("scheduled state remains due")
            .is_some()
    );

    record.state = TriggerState::Paused;
    assert!(
        provider
            .evaluate(&record, ts(1_704_067_200))
            .await
            .expect("paused state is not due")
            .is_none()
    );

    record.state = TriggerState::Completed;
    assert!(
        provider
            .evaluate(&record, ts(1_704_067_200))
            .await
            .expect("completed state is not due")
            .is_none()
    );
}

#[tokio::test]
async fn schedule_provider_rejects_invalid_record() {
    let mut record = sample_record(
        TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid"),
        tenant("tenant-a"),
        ts(1_704_067_200),
    );
    record.prompt.clear();

    let error = ScheduleTriggerSourceProvider
        .evaluate(&record, ts(1_704_067_200))
        .await
        .expect_err("invalid record rejected");
    assert!(
        error
            .to_string()
            .contains("trigger prompt must not be empty"),
        "unexpected error: {error}"
    );
}

#[tokio::test]
async fn in_memory_repository_lists_and_removes_scoped_records() {
    let repo = InMemoryTriggerRepository::default();
    let due = sample_record(
        TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid"),
        tenant("tenant-a"),
        ts(1_704_067_200),
    );
    let later = sample_record(
        TriggerId::parse("01J00000000000000000000000").expect("ulid"),
        tenant("tenant-a"),
        ts(1_704_067_260),
    );
    let other_tenant = sample_record(
        TriggerId::parse("01J00000000000000000000001").expect("ulid"),
        tenant("tenant-b"),
        ts(1_704_067_200),
    );
    let other_tenant_id = other_tenant.trigger_id;
    repo.upsert_trigger(due.clone()).await.expect("insert due");
    repo.upsert_trigger(later.clone())
        .await
        .expect("insert later");
    repo.upsert_trigger(other_tenant)
        .await
        .expect("insert other tenant");

    let due_records = repo
        .list_due_triggers(ts(1_704_067_200), 10)
        .await
        .expect("list due");
    assert_eq!(
        due_records
            .iter()
            .map(|record| record.trigger_id)
            .collect::<Vec<_>>(),
        vec![due.trigger_id, other_tenant_id]
    );

    let tenant_records = repo
        .list_triggers(tenant("tenant-a"))
        .await
        .expect("list tenant");
    assert_eq!(
        tenant_records
            .iter()
            .map(|record| record.trigger_id)
            .collect::<Vec<_>>(),
        vec![due.trigger_id, later.trigger_id]
    );

    let removed = repo
        .remove_trigger(tenant("tenant-a"), due.trigger_id)
        .await
        .expect("remove")
        .expect("record removed");
    assert_eq!(removed.trigger_id, due.trigger_id);
    assert!(
        repo.get_trigger(tenant("tenant-a"), due.trigger_id)
            .await
            .expect("lookup")
            .is_none()
    );
}

#[tokio::test]
async fn in_memory_repository_remove_missing_key_returns_none() {
    let repo = InMemoryTriggerRepository::default();
    assert!(
        repo.remove_trigger(
            tenant("tenant-a"),
            TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid")
        )
        .await
        .expect("remove missing")
        .is_none()
    );
}

#[tokio::test]
async fn in_memory_repository_rejects_invalid_record_on_upsert() {
    let repo = InMemoryTriggerRepository::default();
    let mut record = sample_record(
        TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid"),
        tenant("tenant-a"),
        ts(1_704_067_200),
    );
    record.name.clear();
    assert!(matches!(
        repo.upsert_trigger(record).await,
        Err(TriggerError::InvalidRecord { .. })
    ));

    let mut record = sample_record(
        TriggerId::parse("01J00000000000000000000000").expect("ulid"),
        tenant("tenant-a"),
        ts(1_704_067_200),
    );
    record.prompt.clear();
    assert!(matches!(
        repo.upsert_trigger(record).await,
        Err(TriggerError::InvalidRecord { .. })
    ));

    let mut record = sample_record(
        TriggerId::parse("01J00000000000000000000001").expect("ulid"),
        tenant("tenant-a"),
        ts(1_704_067_200),
    );
    record.name = "x".repeat(MAX_TRIGGER_NAME_BYTES + 1);
    assert!(matches!(
        repo.upsert_trigger(record).await,
        Err(TriggerError::InvalidRecord { .. })
    ));

    let mut record = sample_record(
        TriggerId::parse("01J00000000000000000000002").expect("ulid"),
        tenant("tenant-a"),
        ts(1_704_067_200),
    );
    record.prompt = "x".repeat(MAX_TRIGGER_PROMPT_BYTES + 1);
    assert!(matches!(
        repo.upsert_trigger(record).await,
        Err(TriggerError::InvalidRecord { .. })
    ));
}

#[tokio::test]
async fn in_memory_repository_list_due_triggers_handles_zero_limit() {
    let repo = InMemoryTriggerRepository::default();
    repo.upsert_trigger(sample_record(
        TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid"),
        tenant("tenant-a"),
        ts(1_704_067_200),
    ))
    .await
    .expect("insert due");

    let due_records = repo
        .list_due_triggers(ts(1_704_067_200), 0)
        .await
        .expect("list due");
    assert!(due_records.is_empty());
}

#[tokio::test]
async fn in_memory_repository_list_due_triggers_truncates_to_limit_one() {
    let repo = InMemoryTriggerRepository::default();
    let first = sample_record(
        TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid"),
        tenant("tenant-a"),
        ts(1_704_067_200),
    );
    let mut second = sample_record(
        TriggerId::parse("01J00000000000000000000000").expect("ulid"),
        tenant("tenant-a"),
        ts(1_704_067_260),
    );
    second.created_at = ts(1_704_067_201);
    repo.upsert_trigger(first.clone())
        .await
        .expect("insert first");
    repo.upsert_trigger(second).await.expect("insert second");

    let due_records = repo
        .list_due_triggers(ts(1_704_067_260), 1)
        .await
        .expect("list due");
    assert_eq!(due_records.len(), 1);
    assert_eq!(due_records[0].trigger_id, first.trigger_id);
}

#[tokio::test]
async fn in_memory_repository_list_due_triggers_orders_same_slot_by_tenant_then_trigger_id() {
    let repo = InMemoryTriggerRepository::default();
    let due_slot = ts(1_704_067_200);
    let tenant_a_high = sample_record(
        TriggerId::parse("01J00000000000000000000000").expect("ulid"),
        tenant("tenant-a"),
        due_slot,
    );
    let tenant_b_low = sample_record(
        TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid"),
        tenant("tenant-b"),
        due_slot,
    );
    let tenant_a_low = sample_record(
        TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZY").expect("ulid"),
        tenant("tenant-a"),
        due_slot,
    );
    repo.upsert_trigger(tenant_b_low.clone())
        .await
        .expect("insert tenant b");
    repo.upsert_trigger(tenant_a_high.clone())
        .await
        .expect("insert tenant a high");
    repo.upsert_trigger(tenant_a_low.clone())
        .await
        .expect("insert tenant a low");

    let due_records = repo
        .list_due_triggers(due_slot, 10)
        .await
        .expect("list due");

    assert_eq!(
        due_records
            .iter()
            .map(|record| (record.tenant_id.clone(), record.trigger_id))
            .collect::<Vec<_>>(),
        vec![
            (tenant_a_low.tenant_id.clone(), tenant_a_low.trigger_id),
            (tenant_a_high.tenant_id.clone(), tenant_a_high.trigger_id),
            (tenant_b_low.tenant_id.clone(), tenant_b_low.trigger_id),
        ]
    );
}

#[tokio::test]
async fn in_memory_repository_list_due_triggers_clamps_large_limit() {
    let repo = InMemoryTriggerRepository::default();
    for _ in 0..=MAX_DUE_TRIGGER_POLL_LIMIT {
        repo.upsert_trigger(sample_record(
            TriggerId::new(),
            tenant("tenant-a"),
            ts(1_704_067_200),
        ))
        .await
        .expect("insert due");
    }

    let due_records = repo
        .list_due_triggers(ts(1_704_067_200), MAX_DUE_TRIGGER_POLL_LIMIT + 10)
        .await
        .expect("list due");
    assert_eq!(due_records.len(), MAX_DUE_TRIGGER_POLL_LIMIT);
}

#[tokio::test]
async fn in_memory_repository_running_history_does_not_overwrite_terminal_history() {
    let repo = InMemoryTriggerRepository::default();
    let tenant_id = tenant("tenant-a");
    let trigger_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
    let fire_slot = ts(1_704_067_200);
    let run_id = TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5a").expect("valid run");
    let completed_at = fire_slot + chrono::Duration::seconds(30);
    let later_submitted_at = fire_slot + chrono::Duration::seconds(45);

    repo.complete_run_history(
        &tenant_id,
        trigger_id,
        fire_slot,
        Some(run_id),
        TriggerRunHistoryStatus::Ok,
        completed_at,
    )
    .expect("seed terminal history");

    repo.upsert_running_run_history(
        &tenant_id,
        trigger_id,
        fire_slot,
        run_id,
        Some(ThreadId::new("01890f0f-test-7000-8000-000000000001").expect("valid thread id")),
        later_submitted_at,
    )
    .expect("late running upsert is ignored");

    let runs = repo
        .list_trigger_run_history(tenant_id, trigger_id, 10)
        .await
        .expect("list run history");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].status, TriggerRunHistoryStatus::Ok);
    assert_eq!(runs[0].submitted_at, completed_at);
    assert_eq!(runs[0].completed_at, Some(completed_at));
}

#[test]
fn in_memory_repository_returns_backend_error_when_mutex_is_poisoned() {
    let repo = InMemoryTriggerRepository::default();
    let poison_repo = repo.clone();
    let _ = std::panic::catch_unwind(move || {
        let _guard = poison_repo.state.lock().expect("lock before poison");
        panic!("poison trigger repository mutex");
    });

    let error = repo
        .lock_state()
        .expect_err("poisoned mutex maps to backend");
    assert!(matches!(error, TriggerError::Backend { .. }));
}

#[tokio::test]
async fn in_memory_repository_claim_due_fire_returns_backend_error_when_mutex_is_poisoned() {
    let repo = InMemoryTriggerRepository::default();
    poison_in_memory_repo(&repo);

    let error = repo
        .claim_due_fire(ClaimDueFireRequest {
            tenant_id: tenant("tenant-a"),
            trigger_id: TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid"),
            fire_slot: ts(1_704_067_200),
            now: ts(1_704_067_200),
        })
        .await
        .expect_err("poisoned mutex maps to backend through claim API");
    assert!(matches!(error, TriggerError::Backend { .. }));
}

#[tokio::test]
async fn in_memory_repository_mark_fire_accepted_returns_backend_error_when_mutex_is_poisoned() {
    let repo = InMemoryTriggerRepository::default();
    poison_in_memory_repo(&repo);

    let fire_slot = ts(1_704_067_200);
    let error = repo
        .mark_fire_accepted(FireAcceptedRequest {
            tenant_id: tenant("tenant-a"),
            trigger_id: TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid"),
            fire_slot,
            run_id: TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5a").expect("valid run"),
            thread_id: ThreadId::new("01890f0f-test-7000-8000-000000000002")
                .expect("valid thread id"),
            submitted_at: fire_slot,
        })
        .await
        .expect_err("poisoned mutex maps to backend through accepted-result API");
    assert!(matches!(error, TriggerError::Backend { .. }));
}

#[tokio::test]
async fn in_memory_repository_mark_fire_replayed_returns_backend_error_when_mutex_is_poisoned() {
    let repo = InMemoryTriggerRepository::default();
    poison_in_memory_repo(&repo);

    let fire_slot = ts(1_704_067_200);
    let error = repo
        .mark_fire_replayed(FireReplayedRequest {
            tenant_id: tenant("tenant-a"),
            trigger_id: TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid"),
            fire_slot,
            original_run_id: TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5a")
                .expect("valid run"),
            thread_id: None,
            replayed_at: fire_slot,
        })
        .await
        .expect_err("poisoned mutex maps to backend through replayed-result API");
    assert!(matches!(error, TriggerError::Backend { .. }));
}

#[tokio::test]
async fn in_memory_repository_mark_fire_retryable_failed_returns_backend_error_when_mutex_is_poisoned()
 {
    let repo = InMemoryTriggerRepository::default();
    poison_in_memory_repo(&repo);

    let fire_slot = ts(1_704_067_200);
    let error = repo
        .mark_fire_retryable_failed(FireRetryableFailedRequest {
            tenant_id: tenant("tenant-a"),
            trigger_id: TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid"),
            fire_slot,
        })
        .await
        .expect_err("poisoned mutex maps to backend through retryable-failure API");
    assert!(matches!(error, TriggerError::Backend { .. }));
}

#[tokio::test]
async fn in_memory_repository_mark_fire_permanently_failed_returns_backend_error_when_mutex_is_poisoned()
 {
    let repo = InMemoryTriggerRepository::default();
    poison_in_memory_repo(&repo);

    let fire_slot = ts(1_704_067_200);
    let error = repo
        .mark_fire_permanently_failed(FirePermanentFailedRequest {
            tenant_id: tenant("tenant-a"),
            trigger_id: TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid"),
            fire_slot,
            next_run_at: ts(1_704_067_260),
        })
        .await
        .expect_err("poisoned mutex maps to backend through permanent-failure API");
    assert!(matches!(error, TriggerError::Backend { .. }));
}

#[tokio::test]
async fn fire_once_trigger_completes_after_clear_active_fire() {
    let repo = InMemoryTriggerRepository::default();
    let trigger_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
    let fire_slot = ts(1_704_067_200);
    let run_id = TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5a").expect("valid run");
    // Insert a fire-once trigger with an active fire already in progress.
    let mut record = sample_record(trigger_id, tenant("tenant-a"), fire_slot);
    record.schedule = TriggerSchedule::once(fire_slot, "UTC").expect("valid once");
    record.active_fire_slot = Some(fire_slot);
    record.active_run_ref = Some(run_id);
    repo.upsert_trigger(record).await.expect("insert");

    // Clearing the active fire must transition state to Completed for fire-once.
    let cleared = repo
        .clear_active_fire(ClearActiveFireRequest {
            tenant_id: tenant("tenant-a"),
            trigger_id,
            fire_slot,
            run_id,
            status: TriggerRunHistoryStatus::Ok,
        })
        .await
        .expect("clear_active_fire succeeds")
        .expect("record returned");
    assert_eq!(cleared.state, TriggerState::Completed);
    assert_eq!(cleared.active_fire_slot, None);
    assert_eq!(cleared.active_run_ref, None);

    // Persisted record must also be Completed.
    let persisted = repo
        .get_trigger(tenant("tenant-a"), trigger_id)
        .await
        .expect("load")
        .expect("record present");
    assert_eq!(persisted.state, TriggerState::Completed);
}

#[tokio::test]
async fn fire_once_trigger_not_due_after_completing() {
    let repo = InMemoryTriggerRepository::default();
    let trigger_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
    let fire_slot = ts(1_704_067_200);
    let run_id = TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5a").expect("valid run");
    // Insert fire-once trigger with an active fire.
    let mut record = sample_record(trigger_id, tenant("tenant-a"), fire_slot);
    record.schedule = TriggerSchedule::once(fire_slot, "UTC").expect("valid once");
    record.active_fire_slot = Some(fire_slot);
    record.active_run_ref = Some(run_id);
    repo.upsert_trigger(record).await.expect("insert");

    // Clear the fire to move trigger to Completed.
    repo.clear_active_fire(ClearActiveFireRequest {
        tenant_id: tenant("tenant-a"),
        trigger_id,
        fire_slot,
        run_id,
        status: TriggerRunHistoryStatus::Ok,
    })
    .await
    .expect("clear_active_fire succeeds");

    // Trigger must not appear in the due list — is_due_at requires state == Scheduled.
    let due_records = repo
        .list_due_triggers(fire_slot, 10)
        .await
        .expect("list due");
    assert!(
        due_records.iter().all(|r| r.trigger_id != trigger_id),
        "completed fire-once trigger must not be due"
    );
}

#[tokio::test]
async fn recurring_trigger_reschedules_after_clear_active_fire() {
    // Regression guard: clear_active_fire must NOT transition Recurring triggers to Completed.
    let repo = InMemoryTriggerRepository::default();
    let trigger_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
    let fire_slot = ts(1_704_067_200);
    let next_slot = ts(1_704_067_260);
    let run_id = TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5a").expect("valid run");
    let mut record = sample_record(trigger_id, tenant("tenant-a"), next_slot);
    record.active_fire_slot = Some(fire_slot);
    record.active_run_ref = Some(run_id);
    repo.upsert_trigger(record).await.expect("insert");

    let cleared = repo
        .clear_active_fire(ClearActiveFireRequest {
            tenant_id: tenant("tenant-a"),
            trigger_id,
            fire_slot,
            run_id,
            status: TriggerRunHistoryStatus::Ok,
        })
        .await
        .expect("clear_active_fire succeeds")
        .expect("record returned");

    // Recurring triggers must stay Scheduled so the next slot can fire.
    assert_eq!(cleared.state, TriggerState::Scheduled);
    assert_eq!(cleared.active_fire_slot, None);
    assert_eq!(cleared.active_run_ref, None);
}

#[test]
fn cron_schedule_rejects_invalid_timezone() {
    let error = TriggerSchedule::cron_with_timezone("0 9 * * *", "Not/A/Timezone")
        .expect_err("invalid timezone rejected");
    assert!(
        error.to_string().contains("invalid timezone"),
        "unexpected error: {error}"
    );
}

#[test]
fn elapsed_occurrences_between_counts_cron_slots_exactly() {
    let schedule = TriggerSchedule::cron("* * * * *").expect("valid cron");
    let after = Utc.with_ymd_and_hms(2025, 6, 1, 9, 0, 0).unwrap();
    let now = Utc.with_ymd_and_hms(2025, 6, 1, 9, 16, 0).unwrap();
    let elapsed = schedule
        .elapsed_occurrences_between(after, now, 99)
        .expect("count");
    assert_eq!(
        elapsed,
        ElapsedOccurrenceCount {
            count: 16,
            capped: false
        }
    );
}

#[test]
fn elapsed_occurrences_between_reports_cap_truncation() {
    let schedule = TriggerSchedule::cron("* * * * *").expect("valid cron");
    let after = Utc.with_ymd_and_hms(2025, 6, 1, 9, 0, 0).unwrap();
    let now = Utc.with_ymd_and_hms(2025, 6, 1, 10, 0, 0).unwrap();
    let elapsed = schedule
        .elapsed_occurrences_between(after, now, 5)
        .expect("count");
    assert_eq!(
        elapsed,
        ElapsedOccurrenceCount {
            count: 5,
            capped: true
        }
    );
}

#[test]
fn elapsed_occurrences_between_exact_cap_window_is_not_capped() {
    let schedule = TriggerSchedule::cron("* * * * *").expect("valid cron");
    let after = Utc.with_ymd_and_hms(2025, 6, 1, 9, 0, 0).unwrap();
    let now = Utc.with_ymd_and_hms(2025, 6, 1, 9, 5, 0).unwrap();
    let elapsed = schedule
        .elapsed_occurrences_between(after, now, 5)
        .expect("count");
    assert_eq!(
        elapsed,
        ElapsedOccurrenceCount {
            count: 5,
            capped: false
        }
    );
}

#[test]
fn elapsed_occurrences_between_zero_window_and_once_schedules_elapse_nothing() {
    let after = Utc.with_ymd_and_hms(2025, 6, 1, 9, 0, 0).unwrap();
    let cron = TriggerSchedule::cron("* * * * *").expect("valid cron");
    let none = cron
        .elapsed_occurrences_between(after, after, 99)
        .expect("count");
    assert_eq!(
        none,
        ElapsedOccurrenceCount {
            count: 0,
            capped: false
        }
    );
    let once = TriggerSchedule::once(Utc.with_ymd_and_hms(2025, 6, 1, 12, 0, 0).unwrap(), "UTC")
        .expect("valid once");
    let now = Utc.with_ymd_and_hms(2025, 6, 2, 9, 0, 0).unwrap();
    let elapsed = once
        .elapsed_occurrences_between(after, now, 99)
        .expect("count");
    assert_eq!(
        elapsed,
        ElapsedOccurrenceCount {
            count: 0,
            capped: false
        }
    );
}

#[test]
fn once_schedule_next_slot_after_returns_some_when_future() {
    let at = Utc.with_ymd_and_hms(2025, 6, 1, 12, 0, 0).unwrap();
    let schedule = TriggerSchedule::once(at, "UTC").expect("valid once");
    let before = Utc.with_ymd_and_hms(2025, 6, 1, 11, 0, 0).unwrap();
    let next = schedule
        .next_slot_after(before)
        .expect("next slot")
        .expect("some");
    assert_eq!(next, at);
}

#[test]
fn once_schedule_next_slot_after_returns_none_when_past() {
    let at = Utc.with_ymd_and_hms(2025, 6, 1, 12, 0, 0).unwrap();
    let schedule = TriggerSchedule::once(at, "UTC").expect("valid once");
    let after = Utc.with_ymd_and_hms(2025, 6, 1, 13, 0, 0).unwrap();
    let next = schedule.next_slot_after(after).expect("next slot");
    assert!(next.is_none());
}

#[test]
fn once_schedule_next_slot_after_returns_none_when_equal() {
    let at = Utc.with_ymd_and_hms(2025, 6, 1, 12, 0, 0).unwrap();
    let schedule = TriggerSchedule::once(at, "UTC").expect("valid once");
    let next = schedule.next_slot_after(at).expect("next slot");
    assert!(next.is_none());
}

#[test]
fn once_schedule_rejects_invalid_timezone() {
    let at = Utc.with_ymd_and_hms(2025, 6, 1, 12, 0, 0).unwrap();
    let error = TriggerSchedule::once(at, "Not/A/Timezone").expect_err("invalid tz rejected");
    assert!(
        error.to_string().contains("invalid timezone"),
        "unexpected error: {error}"
    );
}

#[test]
fn cron_schedule_evaluates_in_named_timezone() {
    // "0 9 * * *" = 9am local time
    // America/New_York is UTC-5 in winter (no DST in January)
    // 9am New York = 14:00 UTC
    let schedule = TriggerSchedule::cron_with_timezone("0 9 * * *", "America/New_York")
        .expect("valid schedule");
    let after = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(); // midnight UTC, Jan 1
    let next = schedule
        .next_slot_after(after)
        .expect("next slot")
        .expect("future slot");
    // 9am NY on 2026-01-01 = 14:00 UTC (EST = UTC-5)
    assert_eq!(next, Utc.with_ymd_and_hms(2026, 1, 1, 14, 0, 0).unwrap());
}

#[test]
fn cron_schedule_dst_spring_forward_does_not_panic() {
    // US clocks spring forward 2nd Sunday of March at 2am
    // 2026-03-08 is the second Sunday in March 2026
    // "0 2 * * *" = 2am NY — this hour is skipped on spring-forward day
    let schedule = TriggerSchedule::cron_with_timezone("0 2 * * *", "America/New_York")
        .expect("valid schedule");
    // just before spring forward: 2026-03-08 06:59:00 UTC = 1:59am EST
    let before_gap = Utc.with_ymd_and_hms(2026, 3, 8, 6, 59, 0).unwrap();
    // Should not panic; result may be None or next day
    let _ = schedule
        .next_slot_after(before_gap)
        .expect("no error during DST gap");
}

#[test]
fn once_from_local_valid_time_converts_to_utc() {
    // 2026-01-15 09:00:00 America/New_York = EST = UTC-5 => 14:00:00 UTC
    let schedule = TriggerSchedule::once_from_local("2026-01-15T09:00:00", "America/New_York")
        .expect("valid local time");
    let expected = Utc.with_ymd_and_hms(2026, 1, 15, 14, 0, 0).unwrap();
    match schedule {
        TriggerSchedule::Once { at, .. } => assert_eq!(at, expected),
        _ => panic!("expected Once schedule"),
    }
}

#[test]
fn once_from_local_ambiguous_dst_overlap_rejected() {
    // 2026-11-01 01:30:00 America/New_York is ambiguous (clocks fall back at 2am)
    let error = TriggerSchedule::once_from_local("2026-11-01T01:30:00", "America/New_York")
        .expect_err("ambiguous DST time rejected");
    assert!(
        error.to_string().contains("ambiguous"),
        "unexpected error: {error}"
    );
}

#[test]
fn once_from_local_dst_gap_rejected() {
    // 2026-03-08 02:30:00 America/New_York is in the DST gap (clocks spring forward from 2am to 3am)
    let error = TriggerSchedule::once_from_local("2026-03-08T02:30:00", "America/New_York")
        .expect_err("DST gap time rejected");
    assert!(
        error.to_string().contains("does not exist"),
        "unexpected error: {error}"
    );
}

#[test]
fn once_from_local_rejects_malformed_datetime() {
    let error = TriggerSchedule::once_from_local("not-a-date", "America/New_York")
        .expect_err("malformed datetime must be rejected");
    match error {
        TriggerError::InvalidSchedule { reason, .. } => {
            assert!(
                reason.contains("not-a-date"),
                "reason should name the bad input: {reason}"
            );
        }
        other => panic!("expected InvalidSchedule, got {other:?}"),
    }
}
#[test]
fn once_schedule_storage_round_trip_uses_schedule_at_column() {
    let at = Utc.with_ymd_and_hms(2026, 6, 15, 10, 0, 0).unwrap();
    let schedule = TriggerSchedule::once(at, "UTC").expect("valid once");
    let (kind, expression, schedule_at) = schedule.to_storage();
    assert_eq!(kind, "once");
    assert_eq!(expression, ""); // schedule_expression is empty for Once
    assert!(schedule_at.is_some(), "Once must populate schedule_at");
    // Round-trip via from_storage
    let restored = TriggerSchedule::from_storage(kind, expression, schedule_at.as_deref(), "UTC")
        .expect("from_storage round-trip");
    assert_eq!(restored, schedule);
}

#[tokio::test]
async fn exhausted_finite_cron_transitions_to_completed_on_clear_active_fire() {
    let repo = InMemoryTriggerRepository::default();
    let trigger_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
    let fire_slot = ts(1_704_067_200);
    let run_id = TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5a").expect("valid run");
    let mut record = sample_record(trigger_id, tenant("tenant-a"), fire_slot);
    record.schedule = TriggerSchedule::once(fire_slot, "UTC").expect("valid once");
    record.active_fire_slot = Some(fire_slot);
    record.active_run_ref = Some(run_id);
    repo.upsert_trigger(record).await.expect("insert");

    let cleared = repo
        .clear_active_fire(ClearActiveFireRequest {
            tenant_id: tenant("tenant-a"),
            trigger_id,
            fire_slot,
            run_id,
            status: TriggerRunHistoryStatus::Ok,
        })
        .await
        .expect("clear succeeds")
        .expect("record returned");

    assert_eq!(
        cleared.state,
        TriggerState::Completed,
        "exhausted schedule must transition to Completed"
    );
}
