#![cfg(any(feature = "libsql", feature = "postgres"))]

use chrono::{TimeZone, Utc};
use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId, Timestamp, UserId};
use ironclaw_triggers::{
    ActiveTriggerScanCursor, ClearActiveFireRequest, InMemoryTriggerRepository,
    TriggerCompletionPolicy, TriggerError, TriggerId, TriggerRecord, TriggerRepository,
    TriggerRunStatus, TriggerSchedule, TriggerSourceKind, TriggerState,
};
use ironclaw_turns::TurnRunId;

#[cfg(feature = "libsql")]
use {
    ironclaw_triggers::LibSqlTriggerRepository, libsql::params, std::sync::Arc, tempfile::tempdir,
};

#[cfg(feature = "postgres")]
use ironclaw_triggers::PostgresTriggerRepository;

fn ts(seconds: i64) -> Timestamp {
    Utc.timestamp_opt(seconds, 0).single().expect("valid ts")
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
        created_at: ts(1_704_067_200),
    }
}

async fn assert_round_trip_and_scoped_isolation(repo: &impl TriggerRepository) {
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

    repo.upsert_trigger(due.clone()).await.expect("insert due");
    repo.upsert_trigger(later.clone())
        .await
        .expect("insert later");
    repo.upsert_trigger(other_tenant.clone())
        .await
        .expect("insert other tenant");

    let fetched = repo
        .get_trigger(tenant("tenant-a"), due.trigger_id)
        .await
        .expect("get trigger")
        .expect("record present");
    assert_eq!(fetched, due);

    assert!(
        repo.get_trigger(tenant("tenant-b"), due.trigger_id)
            .await
            .expect("wrong-tenant lookup")
            .is_none()
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

    let mut other_agent = sample_record(
        TriggerId::parse("01J00000000000000000000002").expect("ulid"),
        tenant("tenant-a"),
        ts(1_704_067_320),
    );
    other_agent.agent_id = Some(AgentId::new("agent-b").expect("valid agent"));
    repo.upsert_trigger(other_agent.clone())
        .await
        .expect("insert other agent");

    let first_scoped_record = repo
        .list_scoped_triggers(
            tenant("tenant-a"),
            user("user-a"),
            Some(AgentId::new("agent-a").expect("valid agent")),
            Some(ProjectId::new("project-a").expect("valid project")),
            1,
        )
        .await
        .expect("list first scoped trigger");
    assert_eq!(
        first_scoped_record
            .iter()
            .map(|record| record.trigger_id)
            .collect::<Vec<_>>(),
        vec![due.trigger_id]
    );

    let scoped_records = repo
        .list_scoped_triggers(
            tenant("tenant-a"),
            user("user-a"),
            Some(AgentId::new("agent-a").expect("valid agent")),
            Some(ProjectId::new("project-a").expect("valid project")),
            10,
        )
        .await
        .expect("list scoped triggers");
    assert_eq!(
        scoped_records
            .iter()
            .map(|record| record.trigger_id)
            .collect::<Vec<_>>(),
        vec![due.trigger_id, later.trigger_id]
    );

    assert!(
        repo.list_scoped_triggers(
            tenant("tenant-a"),
            user("user-a"),
            Some(AgentId::new("agent-c").expect("valid agent")),
            Some(ProjectId::new("project-a").expect("valid project")),
            10,
        )
        .await
        .expect("list other scoped triggers")
        .is_empty()
    );

    assert_eq!(
        repo.remove_scoped_trigger(
            tenant("tenant-a"),
            user("user-a"),
            Some(AgentId::new("agent-c").expect("valid agent")),
            Some(ProjectId::new("project-a").expect("valid project")),
            later.trigger_id,
        )
        .await
        .expect("wrong-scope scoped remove"),
        None
    );
    assert!(
        repo.get_trigger(tenant("tenant-a"), later.trigger_id)
            .await
            .expect("lookup after wrong-scope scoped remove")
            .is_some()
    );
    let scoped_removed = repo
        .remove_scoped_trigger(
            tenant("tenant-a"),
            user("user-a"),
            Some(AgentId::new("agent-a").expect("valid agent")),
            Some(ProjectId::new("project-a").expect("valid project")),
            later.trigger_id,
        )
        .await
        .expect("matching-scope scoped remove")
        .expect("scoped removed record");
    assert_eq!(scoped_removed.trigger_id, later.trigger_id);
    assert!(
        repo.get_trigger(tenant("tenant-a"), later.trigger_id)
            .await
            .expect("lookup scoped removed")
            .is_none()
    );

    let removed = repo
        .remove_trigger(tenant("tenant-a"), due.trigger_id)
        .await
        .expect("remove trigger")
        .expect("removed record");
    assert_eq!(removed.trigger_id, due.trigger_id);
    assert!(
        repo.get_trigger(tenant("tenant-a"), due.trigger_id)
            .await
            .expect("lookup removed")
            .is_none()
    );
    assert!(
        repo.get_trigger(tenant("tenant-b"), other_tenant.trigger_id)
            .await
            .expect("lookup other tenant")
            .is_some()
    );
    assert_eq!(
        repo.remove_trigger(tenant("tenant-a"), other_tenant.trigger_id)
            .await
            .expect("wrong-tenant remove"),
        None
    );
    assert!(
        repo.get_trigger(tenant("tenant-b"), other_tenant.trigger_id)
            .await
            .expect("other tenant remains")
            .is_some()
    );
    assert!(
        repo.remove_trigger(tenant("tenant-a"), due.trigger_id)
            .await
            .expect("remove missing trigger")
            .is_none()
    );
}

async fn assert_round_trip_preserves_optional_run_metadata_and_completion_policy(
    repo: &impl TriggerRepository,
) {
    let mut record = sample_record(
        TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid"),
        tenant("tenant-a"),
        ts(1_704_067_260),
    );
    record.completion_policy = TriggerCompletionPolicy::CompleteAfterFirstFire;
    record.last_run_at = Some(ts(1_704_067_200));
    record.last_fired_slot = Some(ts(1_704_067_140));
    record.last_status = Some(TriggerRunStatus::Error);
    record.active_fire_slot = Some(ts(1_704_067_260));
    record.active_run_ref = Some(TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5a").unwrap());

    repo.upsert_trigger(record.clone())
        .await
        .expect("insert record with run metadata");

    let fetched = repo
        .get_trigger(tenant("tenant-a"), record.trigger_id)
        .await
        .expect("get trigger")
        .expect("record present");

    assert_eq!(fetched, record);
}

async fn assert_round_trip_preserves_null_optional_scope_fields(repo: &impl TriggerRepository) {
    let mut record = sample_record(
        TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid"),
        tenant("tenant-a"),
        ts(1_704_067_260),
    );
    record.agent_id = None;
    record.project_id = None;

    repo.upsert_trigger(record.clone())
        .await
        .expect("insert record with null optional fields");

    let fetched = repo
        .get_trigger(tenant("tenant-a"), record.trigger_id)
        .await
        .expect("get trigger")
        .expect("record present");

    assert_eq!(fetched, record);

    let scoped_null_records = repo
        .list_scoped_triggers(tenant("tenant-a"), user("user-a"), None, None, 10)
        .await
        .expect("list null-scoped triggers");
    assert_eq!(
        scoped_null_records
            .iter()
            .map(|record| record.trigger_id)
            .collect::<Vec<_>>(),
        vec![record.trigger_id]
    );

    assert!(
        repo.list_scoped_triggers(
            tenant("tenant-a"),
            user("user-a"),
            Some(AgentId::new("agent-a").expect("valid agent")),
            None,
            10,
        )
        .await
        .expect("list nonmatching scoped triggers")
        .is_empty()
    );

    assert_eq!(
        repo.remove_scoped_trigger(
            tenant("tenant-a"),
            user("user-a"),
            None,
            None,
            TriggerId::parse("01J00000000000000000000009").expect("ulid"),
        )
        .await
        .expect("missing null-scoped remove"),
        None
    );

    let scoped_null_removed = repo
        .remove_scoped_trigger(
            tenant("tenant-a"),
            user("user-a"),
            None,
            None,
            record.trigger_id,
        )
        .await
        .expect("matching null-scoped remove")
        .expect("null-scoped removed record");
    assert_eq!(scoped_null_removed.trigger_id, record.trigger_id);
    assert!(
        repo.get_trigger(tenant("tenant-a"), record.trigger_id)
            .await
            .expect("lookup removed null-scoped record")
            .is_none()
    );
}

async fn assert_upsert_preserves_original_created_at(repo: &impl TriggerRepository) {
    let trigger_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
    let tenant_id = tenant("tenant-a");
    let original_created_at = ts(1_704_067_200);
    let mut record = sample_record(trigger_id, tenant_id.clone(), ts(1_704_067_260));
    record.created_at = original_created_at;

    repo.upsert_trigger(record.clone())
        .await
        .expect("insert record");

    let mut update = record;
    update.name = "renamed trigger".to_string();
    update.created_at = ts(1_704_067_900);
    repo.upsert_trigger(update)
        .await
        .expect("update existing record");

    let fetched = repo
        .get_trigger(tenant_id, trigger_id)
        .await
        .expect("get trigger")
        .expect("record present");

    assert_eq!(fetched.name, "renamed trigger");
    assert_eq!(fetched.created_at, original_created_at);
}

async fn assert_due_query_clamps_limit_and_respects_state_gate(repo: &impl TriggerRepository) {
    let due_slot = ts(1_704_067_200);
    let future = sample_record(
        TriggerId::parse("01J00000000000000000000002").expect("ulid"),
        tenant("tenant-future"),
        ts(1_704_067_320),
    );
    let paused = {
        let mut record = sample_record(
            TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZY").expect("ulid"),
            tenant("tenant-paused"),
            due_slot,
        );
        record.state = TriggerState::Paused;
        record
    };
    let completed = {
        let mut record = sample_record(
            TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZX").expect("ulid"),
            tenant("tenant-completed"),
            due_slot,
        );
        record.state = TriggerState::Completed;
        record
    };
    let active_claim = {
        let mut record = sample_record(
            TriggerId::parse("01J00000000000000000000004").expect("ulid"),
            tenant("tenant-active-claim"),
            due_slot,
        );
        record.active_fire_slot = Some(due_slot);
        record
    };
    let active_run_claim = {
        let mut record = sample_record(
            TriggerId::parse("01J00000000000000000000005").expect("ulid"),
            tenant("tenant-active-run"),
            due_slot,
        );
        record.active_fire_slot = Some(due_slot);
        record.active_run_ref =
            Some(TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5a").expect("valid run"));
        record
    };
    repo.upsert_trigger(paused.clone())
        .await
        .expect("insert paused");
    repo.upsert_trigger(future.clone())
        .await
        .expect("insert future");
    repo.upsert_trigger(completed.clone())
        .await
        .expect("insert completed");
    repo.upsert_trigger(active_claim.clone())
        .await
        .expect("insert active claim");
    repo.upsert_trigger(active_run_claim.clone())
        .await
        .expect("insert active run claim");

    let small_a = sample_record(
        TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid"),
        tenant("tenant-a"),
        due_slot,
    );
    let small_b = sample_record(
        TriggerId::parse("01J00000000000000000000000").expect("ulid"),
        tenant("tenant-b"),
        due_slot,
    );
    let small_c = sample_record(
        TriggerId::parse("01J00000000000000000000000").expect("ulid"),
        tenant("tenant-c"),
        due_slot,
    );
    let same_tenant_low = sample_record(
        TriggerId::parse("01J00000000000000000000001").expect("ulid"),
        tenant("tenant-d"),
        due_slot,
    );
    let same_tenant_high = sample_record(
        TriggerId::parse("01J00000000000000000000003").expect("ulid"),
        tenant("tenant-d"),
        due_slot,
    );
    repo.upsert_trigger(small_b.clone())
        .await
        .expect("insert small_b");
    repo.upsert_trigger(small_c.clone())
        .await
        .expect("insert small_c");
    repo.upsert_trigger(small_a.clone())
        .await
        .expect("insert small_a");
    repo.upsert_trigger(same_tenant_high.clone())
        .await
        .expect("insert same_tenant_high");
    repo.upsert_trigger(same_tenant_low.clone())
        .await
        .expect("insert same_tenant_low");

    let ordered_due_records = repo
        .list_due_triggers(due_slot, 5)
        .await
        .expect("list due ordered");
    assert_eq!(
        ordered_due_records
            .iter()
            .map(|record| (record.tenant_id.clone(), record.trigger_id))
            .collect::<Vec<_>>(),
        vec![
            (small_a.tenant_id.clone(), small_a.trigger_id),
            (small_b.tenant_id.clone(), small_b.trigger_id),
            (small_c.tenant_id.clone(), small_c.trigger_id),
            (
                same_tenant_low.tenant_id.clone(),
                same_tenant_low.trigger_id
            ),
            (
                same_tenant_high.tenant_id.clone(),
                same_tenant_high.trigger_id
            ),
        ]
    );

    for index in 0..127 {
        let record = sample_record(
            TriggerId::parse("01Z00000000000000000000000").expect("ulid"),
            tenant(&format!("tenant-z-{index:03}")),
            due_slot,
        );
        repo.upsert_trigger(record).await.expect("insert filler");
    }

    assert!(
        repo.list_due_triggers(due_slot, 0)
            .await
            .expect("zero limit")
            .is_empty()
    );

    let due_records = repo
        .list_due_triggers(due_slot, 128 + 10)
        .await
        .expect("list due");
    assert_eq!(due_records.len(), 128);
    assert!(
        !due_records
            .iter()
            .any(|record| record.tenant_id == future.tenant_id),
        "future scheduled record must not be returned as due"
    );
    assert!(
        !due_records
            .iter()
            .any(|record| record.tenant_id == paused.tenant_id),
        "paused record must not be returned as due"
    );
    assert!(
        !due_records
            .iter()
            .any(|record| record.tenant_id == completed.tenant_id),
        "completed record must not be returned as due"
    );
    assert!(
        !due_records
            .iter()
            .any(|record| record.tenant_id == active_claim.tenant_id),
        "active fire claim must not be returned as due"
    );
    assert!(
        !due_records
            .iter()
            .any(|record| record.tenant_id == active_run_claim.tenant_id),
        "active run claim must not be returned as due"
    );
}

async fn assert_active_query_lists_active_records_in_deterministic_order(
    repo: &impl TriggerRepository,
) {
    let early_slot = ts(1_704_067_200);
    let later_slot = ts(1_704_067_260);
    let inactive = sample_record(
        TriggerId::parse("01J00000000000000000000001").expect("ulid"),
        tenant("tenant-inactive"),
        early_slot,
    );
    let inactive_trigger_id = inactive.trigger_id;
    let blocked_oldest_a = {
        let mut record = sample_record(
            TriggerId::parse("01J00000000000000000000002").expect("ulid"),
            tenant("tenant-blocked-a"),
            later_slot,
        );
        record.active_fire_slot = Some(early_slot);
        record
    };
    let blocked_oldest_b = {
        let mut record = sample_record(
            TriggerId::parse("01J00000000000000000000003").expect("ulid"),
            tenant("tenant-blocked-b"),
            later_slot,
        );
        record.active_fire_slot = Some(early_slot);
        record
    };
    let blocked_oldest_c = {
        let mut record = sample_record(
            TriggerId::parse("01J00000000000000000000004").expect("ulid"),
            tenant("tenant-blocked-c"),
            later_slot,
        );
        record.active_fire_slot = Some(early_slot);
        record
    };
    let active_terminal_later_a = {
        let mut record = sample_record(
            TriggerId::parse("01J00000000000000000000005").expect("ulid"),
            tenant("tenant-terminal-a"),
            later_slot,
        );
        record.active_fire_slot = Some(later_slot);
        record.active_run_ref =
            Some(TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5c").expect("valid run"));
        record
    };
    let active_terminal_later_b = {
        let mut record = sample_record(
            TriggerId::parse("01J00000000000000000000006").expect("ulid"),
            tenant("tenant-terminal-b"),
            later_slot,
        );
        record.active_fire_slot = Some(later_slot);
        record.active_run_ref =
            Some(TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5d").expect("valid run"));
        record
    };
    let mut overflow_records = Vec::new();
    for index in 0..126 {
        let mut record = sample_record(
            TriggerId::new(),
            tenant(&format!("tenant-z-overflow-{index:03}")),
            later_slot,
        );
        record.active_fire_slot = Some(later_slot);
        overflow_records.push(record);
    }

    repo.upsert_trigger(inactive)
        .await
        .expect("insert inactive");
    repo.upsert_trigger(blocked_oldest_a.clone())
        .await
        .expect("insert blocked oldest a");
    repo.upsert_trigger(blocked_oldest_b.clone())
        .await
        .expect("insert blocked oldest b");
    repo.upsert_trigger(blocked_oldest_c.clone())
        .await
        .expect("insert blocked oldest c");
    repo.upsert_trigger(active_terminal_later_a.clone())
        .await
        .expect("insert active terminal later a");
    repo.upsert_trigger(active_terminal_later_b.clone())
        .await
        .expect("insert active terminal later b");
    for record in &overflow_records {
        repo.upsert_trigger(record.clone())
            .await
            .expect("insert overflow active");
    }

    assert!(
        repo.list_active_triggers(0)
            .await
            .expect("zero active limit")
            .is_empty()
    );

    let first_page = repo
        .list_active_triggers(3)
        .await
        .expect("list first active page");
    assert_eq!(
        first_page
            .iter()
            .map(|record| (record.tenant_id.clone(), record.trigger_id))
            .collect::<Vec<_>>(),
        vec![
            (
                blocked_oldest_a.tenant_id.clone(),
                blocked_oldest_a.trigger_id,
            ),
            (
                blocked_oldest_b.tenant_id.clone(),
                blocked_oldest_b.trigger_id,
            ),
            (
                blocked_oldest_c.tenant_id.clone(),
                blocked_oldest_c.trigger_id,
            ),
        ]
    );

    let cursor =
        ActiveTriggerScanCursor::from_active_record(&first_page[2]).expect("active cursor");
    assert!(
        repo.list_active_triggers_after(Some(cursor.clone()), 0)
            .await
            .expect("list active cursor with zero limit")
            .is_empty()
    );
    let second_page = repo
        .list_active_triggers_after(Some(cursor.clone()), 3)
        .await
        .expect("list second active page");
    assert_eq!(
        second_page
            .iter()
            .map(|record| (record.tenant_id.clone(), record.trigger_id))
            .collect::<Vec<_>>(),
        vec![
            (
                active_terminal_later_a.tenant_id.clone(),
                active_terminal_later_a.trigger_id,
            ),
            (
                active_terminal_later_b.tenant_id.clone(),
                active_terminal_later_b.trigger_id,
            ),
            (
                overflow_records[0].tenant_id.clone(),
                overflow_records[0].trigger_id,
            ),
        ]
    );
    let cursor_at_last =
        ActiveTriggerScanCursor::from_active_record(overflow_records.last().expect("overflow row"))
            .expect("last active cursor");
    assert!(
        repo.list_active_triggers_after(Some(cursor_at_last), 3)
            .await
            .expect("list after last active row")
            .is_empty(),
        "cursor at the last active row must return an empty page"
    );

    let active = repo
        .list_active_triggers(128 + 10)
        .await
        .expect("list active triggers");
    assert_eq!(active.len(), 128);
    assert!(
        active
            .iter()
            .all(|record| record.active_fire_slot.is_some()),
        "active query must only return rows with an active fire slot"
    );
    assert!(
        active
            .iter()
            .all(|record| record.trigger_id != inactive_trigger_id),
        "inactive rows must not appear in the active cleanup query"
    );
    assert_eq!(
        active
            .iter()
            .take(6)
            .map(|record| (record.tenant_id.clone(), record.trigger_id))
            .collect::<Vec<_>>(),
        vec![
            (
                blocked_oldest_a.tenant_id.clone(),
                blocked_oldest_a.trigger_id,
            ),
            (
                blocked_oldest_b.tenant_id.clone(),
                blocked_oldest_b.trigger_id,
            ),
            (
                blocked_oldest_c.tenant_id.clone(),
                blocked_oldest_c.trigger_id,
            ),
            (
                active_terminal_later_a.tenant_id.clone(),
                active_terminal_later_a.trigger_id,
            ),
            (
                active_terminal_later_b.tenant_id.clone(),
                active_terminal_later_b.trigger_id,
            ),
            (
                overflow_records[0].tenant_id.clone(),
                overflow_records[0].trigger_id,
            ),
        ]
    );
    assert!(
        active
            .iter()
            .any(|record| record.trigger_id == active_terminal_later_a.trigger_id),
        "later terminal active rows should still be reachable"
    );

    let limited = repo
        .list_active_triggers(1)
        .await
        .expect("list active limited");
    assert_eq!(limited.len(), 1);
    assert_eq!(limited[0].trigger_id, blocked_oldest_a.trigger_id);
}

async fn assert_active_query_paginates_same_slot_same_tenant_by_trigger_id(
    repo: &impl TriggerRepository,
) {
    let active_slot = ts(1_704_067_260);
    let tenant_id = tenant("tenant-tie");
    let first = {
        let mut record = sample_record(
            TriggerId::parse("01J00000000000000000000000").expect("ulid"),
            tenant_id.clone(),
            ts(1_704_067_800),
        );
        record.active_fire_slot = Some(active_slot);
        record
    };
    let second = {
        let mut record = sample_record(
            TriggerId::parse("01J00000000000000000000001").expect("ulid"),
            tenant_id.clone(),
            ts(1_704_067_800),
        );
        record.active_fire_slot = Some(active_slot);
        record
    };
    let third = {
        let mut record = sample_record(
            TriggerId::parse("01J00000000000000000000002").expect("ulid"),
            tenant_id,
            ts(1_704_067_800),
        );
        record.active_fire_slot = Some(active_slot);
        record
    };

    repo.upsert_trigger(first.clone())
        .await
        .expect("insert first tie row");
    repo.upsert_trigger(second.clone())
        .await
        .expect("insert second tie row");
    repo.upsert_trigger(third.clone())
        .await
        .expect("insert third tie row");

    let first_page = repo
        .list_active_triggers(2)
        .await
        .expect("list first tie page");
    assert_eq!(
        first_page
            .iter()
            .map(|record| record.trigger_id)
            .collect::<Vec<_>>(),
        vec![first.trigger_id, second.trigger_id]
    );
    let cursor =
        ActiveTriggerScanCursor::from_active_record(&first_page[1]).expect("tie page cursor");
    let second_page = repo
        .list_active_triggers_after(Some(cursor), 2)
        .await
        .expect("list second tie page");
    assert_eq!(
        second_page
            .iter()
            .map(|record| record.trigger_id)
            .collect::<Vec<_>>(),
        vec![third.trigger_id]
    );
}

async fn assert_rejects_validation_failures_before_persistence(repo: &impl TriggerRepository) {
    let trigger_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
    let tenant_id = tenant("tenant-a");
    let next_run_at = ts(1_704_067_200);

    let mut name_error = sample_record(trigger_id, tenant_id.clone(), next_run_at);
    name_error.name.clear();
    assert!(matches!(
        repo.upsert_trigger(name_error).await,
        Err(TriggerError::InvalidRecord { .. })
    ));

    let mut prompt_error = sample_record(trigger_id, tenant_id.clone(), next_run_at);
    prompt_error.prompt.clear();
    assert!(matches!(
        repo.upsert_trigger(prompt_error).await,
        Err(TriggerError::InvalidRecord { .. })
    ));

    let mut schedule_error = sample_record(trigger_id, tenant_id, next_run_at);
    schedule_error.schedule = TriggerSchedule::Cron {
        expression: "*/30 * * * * *".to_string(),
        timezone: "UTC".to_string(),
    };
    assert!(matches!(
        repo.upsert_trigger(schedule_error).await,
        Err(TriggerError::InvalidSchedule { .. })
    ));

    assert!(
        repo.list_triggers(tenant("tenant-a"))
            .await
            .expect("list after failures")
            .is_empty()
    );
}

async fn assert_persists_trigger_state_fire_gate(repo: &impl TriggerRepository) {
    let trigger_id = TriggerId::parse("01J00000000000000000000000").expect("ulid");
    let tenant_id = tenant("tenant-a");
    let mut record = sample_record(trigger_id, tenant_id.clone(), ts(1_704_067_200));
    record.state = TriggerState::Paused;

    repo.upsert_trigger(record.clone())
        .await
        .expect("insert paused");

    let fetched = repo
        .get_trigger(tenant_id.clone(), trigger_id)
        .await
        .expect("get paused")
        .expect("paused record");
    assert_eq!(fetched.state, TriggerState::Paused);
    assert_eq!(fetched.schedule, record.schedule);
    assert!(
        repo.list_due_triggers(ts(1_704_067_200), 10)
            .await
            .expect("list due")
            .is_empty()
    );

    record.state = TriggerState::Scheduled;
    repo.upsert_trigger(record.clone())
        .await
        .expect("reactivate");
    let due_records = repo
        .list_due_triggers(ts(1_704_067_200), 10)
        .await
        .expect("list due after reactivation");
    assert_eq!(due_records.len(), 1);
    assert_eq!(due_records[0].state, TriggerState::Scheduled);
    assert_eq!(due_records[0].trigger_id, trigger_id);
}

#[cfg(feature = "libsql")]
async fn build_libsql_repo_with_db() -> (
    tempfile::TempDir,
    Arc<libsql::Database>,
    LibSqlTriggerRepository,
) {
    let dir = tempdir().expect("tempdir");
    let db_path = dir.path().join("triggers.db");
    let db = Arc::new(
        libsql::Builder::new_local(db_path.display().to_string())
            .build()
            .await
            .expect("build libsql db"),
    );
    let repo = LibSqlTriggerRepository::new(db.clone());
    repo.run_migrations().await.expect("run migrations");
    (dir, db, repo)
}

#[cfg(feature = "libsql")]
async fn build_libsql_repo() -> (tempfile::TempDir, LibSqlTriggerRepository) {
    let (dir, _db, repo) = build_libsql_repo_with_db().await;
    (dir, repo)
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_repository_contract_parity() {
    let (_dir, repo) = build_libsql_repo().await;
    assert_round_trip_and_scoped_isolation(&repo).await;

    let (_dir, repo) = build_libsql_repo().await;
    assert_round_trip_preserves_optional_run_metadata_and_completion_policy(&repo).await;

    let (_dir, repo) = build_libsql_repo().await;
    assert_round_trip_preserves_null_optional_scope_fields(&repo).await;

    let (_dir, repo) = build_libsql_repo().await;
    assert_upsert_preserves_original_created_at(&repo).await;

    let (_dir, repo) = build_libsql_repo().await;
    assert_due_query_clamps_limit_and_respects_state_gate(&repo).await;

    let (_dir, repo) = build_libsql_repo().await;
    assert_active_query_lists_active_records_in_deterministic_order(&repo).await;

    let (_dir, repo) = build_libsql_repo().await;
    assert_active_query_paginates_same_slot_same_tenant_by_trigger_id(&repo).await;

    let (_dir, repo) = build_libsql_repo().await;
    assert_rejects_validation_failures_before_persistence(&repo).await;

    let (_dir, repo) = build_libsql_repo().await;
    assert_persists_trigger_state_fire_gate(&repo).await;
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_repository_run_migrations_is_idempotent() {
    let dir = tempdir().expect("tempdir");
    let db_path = dir.path().join("triggers.db");
    let db = Arc::new(
        libsql::Builder::new_local(db_path.display().to_string())
            .build()
            .await
            .expect("build libsql db"),
    );
    let repo = LibSqlTriggerRepository::new(db);

    repo.run_migrations().await.expect("first run migrations");
    repo.run_migrations().await.expect("second run migrations");
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_repository_rejects_malformed_persisted_rows() {
    let (_dir, db, repo) = build_libsql_repo_with_db().await;
    let trigger_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
    let tenant_id = tenant("tenant-a");
    let record = sample_record(trigger_id, tenant_id.clone(), ts(1_704_067_260));

    repo.upsert_trigger(record).await.expect("insert record");

    let conn = db.connect().expect("connect raw libsql");
    for (column, value, expected_field, read_mode) in malformed_row_cases() {
        conn.execute(
            &format!(
                "UPDATE trigger_records SET {column} = ?1 WHERE tenant_id = ?2 AND trigger_id = ?3"
            ),
            params![value, tenant_id.as_str(), trigger_id.to_string()],
        )
        .await
        .expect("corrupt persisted row");

        assert_malformed_row_error(
            &repo,
            tenant_id.clone(),
            trigger_id,
            expected_field,
            read_mode,
        )
        .await;

        conn.execute("DELETE FROM trigger_records", ())
            .await
            .expect("clear malformed row");
        repo.upsert_trigger(sample_record(
            trigger_id,
            tenant_id.clone(),
            ts(1_704_067_260),
        ))
        .await
        .expect("restore valid row");
    }
}

#[cfg(feature = "postgres")]
#[tokio::test]
async fn postgres_repository_contract_parity() {
    let Some((_container, pool)) = postgres_pool_or_skip().await else {
        return;
    };
    let repo = PostgresTriggerRepository::new(pool.clone());
    repo.run_migrations().await.expect("run migrations");
    assert_round_trip_and_scoped_isolation(&repo).await;

    clear_postgres_triggers(&pool).await;
    assert_round_trip_preserves_optional_run_metadata_and_completion_policy(&repo).await;

    clear_postgres_triggers(&pool).await;
    assert_round_trip_preserves_null_optional_scope_fields(&repo).await;

    clear_postgres_triggers(&pool).await;
    assert_upsert_preserves_original_created_at(&repo).await;

    clear_postgres_triggers(&pool).await;
    assert_due_query_clamps_limit_and_respects_state_gate(&repo).await;

    clear_postgres_triggers(&pool).await;
    assert_active_query_lists_active_records_in_deterministic_order(&repo).await;

    clear_postgres_triggers(&pool).await;
    assert_active_query_paginates_same_slot_same_tenant_by_trigger_id(&repo).await;

    clear_postgres_triggers(&pool).await;
    assert_rejects_validation_failures_before_persistence(&repo).await;

    clear_postgres_triggers(&pool).await;
    assert_persists_trigger_state_fire_gate(&repo).await;
}

#[cfg(feature = "postgres")]
#[tokio::test]
async fn postgres_repository_run_migrations_is_idempotent() {
    let Some((_container, pool)) = postgres_pool_or_skip().await else {
        return;
    };
    let repo = PostgresTriggerRepository::new(pool);

    repo.run_migrations().await.expect("first run migrations");
    repo.run_migrations().await.expect("second run migrations");
}

#[cfg(feature = "postgres")]
#[tokio::test]
async fn postgres_repository_rejects_malformed_persisted_rows() {
    let Some((_container, pool)) = postgres_pool_or_skip().await else {
        return;
    };
    let repo = PostgresTriggerRepository::new(pool.clone());
    repo.run_migrations().await.expect("run migrations");
    let trigger_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
    let tenant_id = tenant("tenant-a");
    let record = sample_record(trigger_id, tenant_id.clone(), ts(1_704_067_260));

    repo.upsert_trigger(record).await.expect("insert record");

    let client = pool.get().await.expect("postgres connection");
    for (column, value, expected_field, read_mode) in malformed_row_cases() {
        client
            .execute(
                &format!(
                    "UPDATE trigger_records SET {column} = $1 WHERE tenant_id = $2 AND trigger_id = $3"
                ),
                &[&value, &tenant_id.as_str(), &trigger_id.to_string()],
            )
            .await
            .expect("corrupt persisted row");

        assert_malformed_row_error(
            &repo,
            tenant_id.clone(),
            trigger_id,
            expected_field,
            read_mode,
        )
        .await;

        client
            .execute("DELETE FROM trigger_records", &[])
            .await
            .expect("clear malformed row");
        repo.upsert_trigger(sample_record(
            trigger_id,
            tenant_id.clone(),
            ts(1_704_067_260),
        ))
        .await
        .expect("restore valid row");
    }
}

#[derive(Clone, Copy)]
enum ReadMode {
    Get,
    List,
    Due,
    Remove,
}

fn malformed_row_cases() -> Vec<(&'static str, &'static str, &'static str, ReadMode)> {
    use ReadMode::{Due, Get, List, Remove};

    [
        ("trigger_id", "not-a-ulid", "invalid length", List),
        ("tenant_id", "", "tenant_id", Due),
        ("creator_user_id", "", "creator_user_id", Remove),
        ("creator_user_id", "", "creator_user_id", Get),
        ("agent_id", "", "agent_id", Get),
        ("project_id", "", "project_id", Get),
        ("name", "", "name", Get),
        ("name", "   ", "name", Get),
        ("source", "webhook", "source", Get),
        ("schedule_expression", "*/30 * * * * *", "schedule", Get),
        ("state", "unknown", "state", Get),
        ("completion_policy", "once", "completion_policy", Get),
        ("prompt", "", "prompt", Get),
        ("prompt", "\t  ", "prompt", Get),
        ("next_run_at", "not-a-timestamp", "next_run_at", Get),
        ("last_run_at", "not-a-timestamp", "last_run_at", Get),
        ("last_fired_slot", "not-a-timestamp", "last_fired_slot", Get),
        (
            "active_fire_slot",
            "not-a-timestamp",
            "active_fire_slot",
            Get,
        ),
        ("active_run_ref", "not-a-uuid", "active_run_ref", Get),
        (
            "active_run_ref",
            "01890f0f-9b6f-7a85-9e5b-9f21a93c4f5a",
            "active_run_ref",
            Get,
        ),
        ("last_status", "timed_out", "last_status", Get),
        ("created_at", "not-a-timestamp", "created_at", Get),
    ]
    .into()
}

async fn assert_malformed_row_error(
    repo: &impl TriggerRepository,
    tenant_id: TenantId,
    trigger_id: TriggerId,
    expected_field: &str,
    read_mode: ReadMode,
) {
    let error = match read_mode {
        ReadMode::Get => repo.get_trigger(tenant_id.clone(), trigger_id).await,
        ReadMode::List => repo
            .list_triggers(tenant_id.clone())
            .await
            .map(|records| records.first().cloned()),
        ReadMode::Due => repo
            .list_due_triggers(ts(1_704_067_260), 10)
            .await
            .map(|records| records.first().cloned()),
        ReadMode::Remove => repo.remove_trigger(tenant_id.clone(), trigger_id).await,
    }
    .expect_err("malformed row must fail hydration");
    assert!(
        if expected_field == "invalid length" {
            matches!(
                error,
                TriggerError::InvalidTriggerId { ref reason } if reason.contains(expected_field)
            )
        } else if expected_field == "schedule" {
            matches!(error, TriggerError::InvalidSchedule { .. })
        } else {
            matches!(
                error,
                TriggerError::InvalidRecord { ref reason } if reason.contains(expected_field)
            )
        },
        "expected malformed row to report {expected_field}, got {error:?}"
    );
}

#[cfg(feature = "postgres")]
async fn postgres_pool_or_skip() -> Option<(
    testcontainers_modules::testcontainers::ContainerAsync<
        testcontainers_modules::postgres::Postgres,
    >,
    deadpool_postgres::Pool,
)> {
    if std::env::var("IRONCLAW_SKIP_POSTGRES_TESTS").is_ok() {
        eprintln!(
            "skipping Postgres trigger repository tests: IRONCLAW_SKIP_POSTGRES_TESTS is set"
        );
        return None;
    }

    // Test-only bootstrap: production composition must pass a constructed pool
    // into PostgresTriggerRepository and keep URL parsing out of this crate.
    let (container, database_url) = start_postgres_container().await?;
    let config: tokio_postgres::Config = database_url
        .parse()
        .expect("testcontainer database URL must parse");
    let manager = deadpool_postgres::Manager::new(config, tokio_postgres::NoTls);
    let pool = deadpool_postgres::Pool::builder(manager)
        .max_size(4)
        .build()
        .expect("Postgres pool must build");
    if let Err(error) = pool.get().await {
        eprintln!("skipping Postgres trigger repository tests: database unavailable ({error})");
        return None;
    }
    Some((container, pool))
}

#[cfg(feature = "postgres")]
async fn start_postgres_container() -> Option<(
    testcontainers_modules::testcontainers::ContainerAsync<
        testcontainers_modules::postgres::Postgres,
    >,
    String,
)> {
    use testcontainers_modules::testcontainers::{ImageExt, runners::AsyncRunner};

    let image = testcontainers_modules::postgres::Postgres::default()
        .with_db_name("ironclaw_test")
        .with_user("postgres")
        .with_password("postgres")
        .with_tag("16-alpine");

    let container = match image.start().await {
        Ok(container) => container,
        Err(error) => {
            eprintln!(
                "skipping Postgres trigger repository tests: docker/testcontainers unavailable ({error})"
            );
            return None;
        }
    };
    let host = match container.get_host().await {
        Ok(host) => host,
        Err(error) => {
            eprintln!(
                "skipping Postgres trigger repository tests: could not resolve container host ({error})"
            );
            return None;
        }
    };
    let port = match container.get_host_port_ipv4(5432).await {
        Ok(port) => port,
        Err(error) => {
            eprintln!(
                "skipping Postgres trigger repository tests: could not resolve container port ({error})"
            );
            return None;
        }
    };
    Some((
        container,
        format!("postgres://postgres:postgres@{host}:{port}/ironclaw_test"),
    ))
}

#[cfg(feature = "postgres")]
async fn clear_postgres_triggers(pool: &deadpool_postgres::Pool) {
    pool.get()
        .await
        .expect("postgres connection")
        .execute("DELETE FROM trigger_records", &[])
        .await
        .expect("clear trigger records");
}

// ---------------------------------------------------------------------------
// Timezone round-trip parity (Comment 3a)
// ---------------------------------------------------------------------------

async fn assert_round_trip_preserves_named_timezone(repo: &impl TriggerRepository) {
    let trigger_id = TriggerId::parse("01J00000000000000000000099").expect("ulid");
    let tenant_id = tenant("tenant-tz");
    let next_run_at = ts(1_704_067_200);

    let mut record = sample_record(trigger_id, tenant_id.clone(), next_run_at);
    record.schedule =
        TriggerSchedule::cron_with_timezone("0 9 * * *", "America/New_York").expect("valid tz");

    repo.upsert_trigger(record.clone())
        .await
        .expect("insert record with named timezone");

    let fetched = repo
        .get_trigger(tenant_id, trigger_id)
        .await
        .expect("get trigger")
        .expect("record present");

    assert_eq!(
        fetched.schedule, record.schedule,
        "named timezone must survive a full round-trip"
    );
    match &fetched.schedule {
        TriggerSchedule::Cron { timezone, .. } => {
            assert_eq!(
                timezone, "America/New_York",
                "timezone must be preserved verbatim"
            );
        }
    }
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_timezone_round_trip() {
    let (_dir, repo) = build_libsql_repo().await;
    assert_round_trip_preserves_named_timezone(&repo).await;
}

#[cfg(feature = "postgres")]
#[tokio::test]
async fn postgres_timezone_round_trip() {
    let Some((_container, pool)) = postgres_pool_or_skip().await else {
        return;
    };
    let repo = PostgresTriggerRepository::new(pool.clone());
    repo.run_migrations().await.expect("run migrations");
    assert_round_trip_preserves_named_timezone(&repo).await;
}

// ---------------------------------------------------------------------------
// Migration regression: legacy row without schedule_timezone gets "UTC" (Comment 3b)
//
// Simulates a pre-migration table that lacks the schedule_timezone column.
// The libsql migration adds the column via ALTER TABLE ... ADD COLUMN with a
// NOT NULL DEFAULT 'UTC' — so any row inserted before the migration exists
// must read back with timezone == "UTC" after migration runs.
//
// Postgres already uses ADD COLUMN IF NOT EXISTS (idempotent SQL) and the
// NOT NULL DEFAULT 'UTC' fills existing rows identically; we cover it via the
// postgres_timezone_round_trip test above (which seeds through upsert_trigger,
// not a pre-migration raw insert, so it tests the migration-complete state).
// A true pre-migration raw-insert scenario would require running without
// run_migrations first, which the postgres harness does not support without
// a separate DDL setup step; that coverage is deferred. See comment below.
// ---------------------------------------------------------------------------

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_utc_backfill_on_legacy_row_without_schedule_timezone() {
    // Build the database without the schedule_timezone column — simulate the
    // schema state before the migration that adds it.
    let dir = tempdir().expect("tempdir");
    let db_path = dir.path().join("triggers-legacy.db");
    let db = Arc::new(
        libsql::Builder::new_local(db_path.display().to_string())
            .build()
            .await
            .expect("build libsql db"),
    );

    // Create the table WITHOUT schedule_timezone (pre-migration schema).
    let conn = db.connect().expect("raw libsql connect for schema setup");
    conn.execute(
        "CREATE TABLE IF NOT EXISTS trigger_records (
            trigger_id TEXT NOT NULL,
            tenant_id TEXT NOT NULL,
            creator_user_id TEXT NOT NULL,
            agent_id TEXT,
            project_id TEXT,
            name TEXT NOT NULL,
            source TEXT NOT NULL,
            schedule_expression TEXT NOT NULL,
            completion_policy TEXT NOT NULL,
            prompt TEXT NOT NULL,
            state TEXT NOT NULL,
            next_run_at TEXT NOT NULL,
            last_run_at TEXT,
            last_fired_slot TEXT,
            last_status TEXT,
            active_fire_slot TEXT,
            active_run_ref TEXT,
            created_at TEXT NOT NULL,
            PRIMARY KEY (tenant_id, trigger_id)
        )",
        (),
    )
    .await
    .expect("create pre-migration table");

    // Insert a legacy row that has no schedule_timezone column value.
    conn.execute(
        "INSERT INTO trigger_records (
            trigger_id, tenant_id, creator_user_id, name, source,
            schedule_expression, completion_policy, prompt, state,
            next_run_at, created_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            "01J00000000000000000000098",
            "tenant-migration",
            "user-a",
            "legacy trigger",
            "schedule",
            "0 8 * * *",
            "recurring",
            "daily task",
            "scheduled",
            "2024-01-01T00:00:00Z",
            "2024-01-01T00:00:00Z",
        ],
    )
    .await
    .expect("insert legacy row without schedule_timezone");

    // Run migrations — this adds schedule_timezone NOT NULL DEFAULT 'UTC',
    // which backfills the existing row with "UTC".
    let repo = LibSqlTriggerRepository::new(db);
    repo.run_migrations()
        .await
        .expect("migration must succeed on pre-existing table");

    // Read back the legacy row and assert timezone was backfilled to "UTC".
    let trigger_id = TriggerId::parse("01J00000000000000000000098").expect("ulid");
    let tenant_id = TenantId::new("tenant-migration").expect("valid tenant");
    let fetched = repo
        .get_trigger(tenant_id, trigger_id)
        .await
        .expect("get trigger after migration")
        .expect("legacy row must be readable after migration");

    match &fetched.schedule {
        TriggerSchedule::Cron { timezone, .. } => {
            assert_eq!(
                timezone, "UTC",
                "legacy row without schedule_timezone must read back as UTC after migration"
            );
        }
    }
}

mod fire_claim_contract {
    use super::*;

    use ironclaw_triggers::{
        ClaimDueFireOutcome, ClaimDueFireRequest, FireAcceptedRequest, FirePermanentFailedRequest,
        FireReplayedRequest, FireRetryableFailedRequest, FireTerminalFailedRequest,
        TriggerRunHistoryStatus,
    };

    async fn assert_fire_claim_and_update_contract(repo: &impl TriggerRepository) {
        let trigger_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
        let tenant_id = tenant("tenant-a");
        let fire_slot = ts(1_704_067_200);
        let accepted_at = ts(1_704_067_205);
        let mut record = sample_record(trigger_id, tenant_id.clone(), fire_slot);
        let expected_next_run_at = record
            .schedule
            .next_slot_after(fire_slot)
            .expect("next slot calculation")
            .expect("future slot");
        record.last_status = Some(TriggerRunStatus::Error);
        repo.upsert_trigger(record.clone())
            .await
            .expect("insert record");

        let claimed = repo
            .claim_due_fire(ClaimDueFireRequest {
                tenant_id: tenant_id.clone(),
                trigger_id,
                fire_slot,
                now: fire_slot,
            })
            .await
            .expect("claim due fire");
        let ClaimDueFireOutcome::Claimed(claimed) = claimed else {
            panic!("record should be claimable, got {claimed:?}");
        };
        assert_eq!(claimed.record.active_fire_slot, Some(fire_slot));
        assert_eq!(claimed.record.active_run_ref, None);

        let persisted = repo
            .get_trigger(tenant_id.clone(), trigger_id)
            .await
            .expect("reload claimed record")
            .expect("record present");
        assert_eq!(persisted.active_fire_slot, Some(fire_slot));
        assert_eq!(persisted.active_run_ref, None);
        assert_eq!(persisted.last_status, Some(TriggerRunStatus::Error));

        let accepted_run_id =
            TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5a").expect("valid run");
        let accepted = repo
            .mark_fire_accepted(FireAcceptedRequest {
                tenant_id: tenant_id.clone(),
                trigger_id,
                fire_slot,
                run_id: accepted_run_id,
                thread_id: ThreadId::new("01890f0f-aa01-7000-8000-000000000001")
                    .expect("valid thread id"),
                submitted_at: accepted_at,
                next_run_at: expected_next_run_at,
            })
            .await
            .expect("mark accepted")
            .expect("accepted fire should persist");
        assert_eq!(accepted.last_run_at, Some(accepted_at));
        assert_eq!(accepted.last_fired_slot, Some(fire_slot));
        assert_eq!(accepted.last_status, Some(TriggerRunStatus::Ok));
        assert_eq!(accepted.active_fire_slot, Some(fire_slot));
        assert_eq!(accepted.active_run_ref, Some(accepted_run_id));
        assert_eq!(accepted.next_run_at, expected_next_run_at);

        let accepted_again = repo
            .mark_fire_accepted(FireAcceptedRequest {
                tenant_id: tenant_id.clone(),
                trigger_id,
                fire_slot,
                run_id: accepted_run_id,
                thread_id: ThreadId::new("01890f0f-aa01-7000-8000-000000000001")
                    .expect("valid thread id"),
                submitted_at: ts(1_704_067_206),
                next_run_at: expected_next_run_at,
            })
            .await
            .expect("idempotent accepted result")
            .expect("accepted result returns existing record");
        assert_eq!(accepted_again, accepted);

        let different_accepted_run_id =
            TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5c").expect("valid run");
        let error = repo
            .mark_fire_accepted(FireAcceptedRequest {
                tenant_id: tenant_id.clone(),
                trigger_id,
                fire_slot,
                run_id: different_accepted_run_id,
                thread_id: ThreadId::new("01890f0f-aa01-7000-8000-000000000002")
                    .expect("valid thread id"),
                submitted_at: accepted_at,
                next_run_at: expected_next_run_at,
            })
            .await
            .expect_err("different accepted run id must not rewrite active_run_ref");
        assert_error_contains(error, "must not rewrite an existing active_run_ref");

        let error = repo
            .mark_fire_retryable_failed(FireRetryableFailedRequest {
                tenant_id: tenant_id.clone(),
                trigger_id,
                fire_slot,
            })
            .await
            .expect_err("stale retryable failure must not clear accepted run ref");
        assert_error_contains(error, "must not clear an accepted active_run_ref");

        let replayed_trigger_id = TriggerId::parse("01J00000000000000000000006").expect("ulid");
        let replayed_tenant_id = tenant("tenant-replayed");
        let replayed_record =
            sample_record(replayed_trigger_id, replayed_tenant_id.clone(), fire_slot);
        repo.upsert_trigger(replayed_record)
            .await
            .expect("insert replayed record");
        let replayed_claim = repo
            .claim_due_fire(ClaimDueFireRequest {
                tenant_id: replayed_tenant_id.clone(),
                trigger_id: replayed_trigger_id,
                fire_slot,
                now: fire_slot,
            })
            .await
            .expect("claim replayed record");
        assert!(matches!(replayed_claim, ClaimDueFireOutcome::Claimed(_)));

        let replayed_run_id =
            TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5b").expect("valid run");
        let replayed = repo
            .mark_fire_replayed(FireReplayedRequest {
                tenant_id: replayed_tenant_id.clone(),
                trigger_id: replayed_trigger_id,
                fire_slot,
                original_run_id: replayed_run_id,
                thread_id: None,
                replayed_at: accepted_at,
                next_run_at: expected_next_run_at,
            })
            .await
            .expect("mark replayed")
            .expect("replayed fire should persist");
        assert_eq!(replayed.last_run_at, Some(accepted_at));
        assert_eq!(replayed.last_fired_slot, Some(fire_slot));
        assert_eq!(replayed.last_status, Some(TriggerRunStatus::Ok));
        assert_eq!(replayed.active_fire_slot, Some(fire_slot));
        assert_eq!(replayed.active_run_ref, Some(replayed_run_id));
        assert_eq!(replayed.next_run_at, expected_next_run_at);

        let replayed_again = repo
            .mark_fire_replayed(FireReplayedRequest {
                tenant_id: replayed_tenant_id.clone(),
                trigger_id: replayed_trigger_id,
                fire_slot,
                original_run_id: replayed_run_id,
                thread_id: None,
                replayed_at: ts(1_704_067_207),
                next_run_at: expected_next_run_at,
            })
            .await
            .expect("idempotent replayed result")
            .expect("replayed result returns existing record");
        assert_eq!(replayed_again, replayed);

        let different_replayed_run_id =
            TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5d").expect("valid run");
        let error = repo
            .mark_fire_replayed(FireReplayedRequest {
                tenant_id: replayed_tenant_id.clone(),
                trigger_id: replayed_trigger_id,
                fire_slot,
                original_run_id: different_replayed_run_id,
                thread_id: None,
                replayed_at: accepted_at,
                next_run_at: expected_next_run_at,
            })
            .await
            .expect_err("different replayed run id must not rewrite active_run_ref");
        assert_error_contains(error, "must not rewrite an existing active_run_ref");

        let error = repo
            .mark_fire_permanently_failed(FirePermanentFailedRequest {
                tenant_id: replayed_tenant_id,
                trigger_id: replayed_trigger_id,
                fire_slot,
                next_run_at: expected_next_run_at,
            })
            .await
            .expect_err("stale permanent failure must not clear replayed run ref");
        assert_error_contains(error, "must not clear an accepted active_run_ref");

        let failure_previous_run_at = ts(1_704_066_900);
        let failure_previous_slot = ts(1_704_066_840);
        let retryable_trigger_id = TriggerId::parse("01J00000000000000000000004").expect("ulid");
        let retryable_tenant_id = tenant("tenant-retryable");
        let mut retryable_record =
            sample_record(retryable_trigger_id, retryable_tenant_id.clone(), fire_slot);
        retryable_record.last_run_at = Some(failure_previous_run_at);
        retryable_record.last_fired_slot = Some(failure_previous_slot);
        repo.upsert_trigger(retryable_record)
            .await
            .expect("insert retryable record");
        let retryable_claim = repo
            .claim_due_fire(ClaimDueFireRequest {
                tenant_id: retryable_tenant_id.clone(),
                trigger_id: retryable_trigger_id,
                fire_slot,
                now: fire_slot,
            })
            .await
            .expect("claim retryable record");
        assert!(matches!(retryable_claim, ClaimDueFireOutcome::Claimed(_)));

        let retryable_failed = repo
            .mark_fire_retryable_failed(FireRetryableFailedRequest {
                tenant_id: retryable_tenant_id,
                trigger_id: retryable_trigger_id,
                fire_slot,
            })
            .await
            .expect("mark retryable failed")
            .expect("retryable failure should persist");
        assert_eq!(retryable_failed.last_run_at, Some(failure_previous_run_at));
        assert_eq!(
            retryable_failed.last_fired_slot,
            Some(failure_previous_slot)
        );
        assert_eq!(retryable_failed.last_status, Some(TriggerRunStatus::Error));
        assert_eq!(retryable_failed.active_fire_slot, None);
        assert_eq!(retryable_failed.active_run_ref, None);
        assert_eq!(retryable_failed.next_run_at, fire_slot);

        let permanent_trigger_id = TriggerId::parse("01J00000000000000000000005").expect("ulid");
        let permanent_tenant_id = tenant("tenant-permanent");
        let mut permanent_record =
            sample_record(permanent_trigger_id, permanent_tenant_id.clone(), fire_slot);
        permanent_record.last_run_at = Some(failure_previous_run_at);
        permanent_record.last_fired_slot = Some(failure_previous_slot);
        repo.upsert_trigger(permanent_record)
            .await
            .expect("insert permanent-failure record");
        let permanent_claim = repo
            .claim_due_fire(ClaimDueFireRequest {
                tenant_id: permanent_tenant_id.clone(),
                trigger_id: permanent_trigger_id,
                fire_slot,
                now: fire_slot,
            })
            .await
            .expect("claim permanent-failure record");
        assert!(matches!(permanent_claim, ClaimDueFireOutcome::Claimed(_)));

        let permanent_failed = repo
            .mark_fire_permanently_failed(FirePermanentFailedRequest {
                tenant_id: permanent_tenant_id,
                trigger_id: permanent_trigger_id,
                fire_slot,
                next_run_at: expected_next_run_at,
            })
            .await
            .expect("mark permanently failed")
            .expect("permanent failure should persist");
        assert_eq!(permanent_failed.last_run_at, Some(failure_previous_run_at));
        assert_eq!(
            permanent_failed.last_fired_slot,
            Some(failure_previous_slot)
        );
        assert_eq!(permanent_failed.last_status, Some(TriggerRunStatus::Error));
        assert_eq!(permanent_failed.active_fire_slot, None);
        assert_eq!(permanent_failed.active_run_ref, None);
        assert!(permanent_failed.next_run_at > fire_slot);

        let terminal_trigger_id = TriggerId::parse("01J00000000000000000000006").expect("ulid");
        let terminal_tenant_id = tenant("tenant-terminal");
        let mut terminal_record =
            sample_record(terminal_trigger_id, terminal_tenant_id.clone(), fire_slot);
        terminal_record.last_run_at = Some(failure_previous_run_at);
        terminal_record.last_fired_slot = Some(failure_previous_slot);
        repo.upsert_trigger(terminal_record)
            .await
            .expect("insert terminal-failure record");
        let terminal_claim = repo
            .claim_due_fire(ClaimDueFireRequest {
                tenant_id: terminal_tenant_id.clone(),
                trigger_id: terminal_trigger_id,
                fire_slot,
                now: fire_slot,
            })
            .await
            .expect("claim terminal-failure record");
        assert!(matches!(terminal_claim, ClaimDueFireOutcome::Claimed(_)));

        let terminal_failed = repo
            .mark_fire_terminally_failed(FireTerminalFailedRequest {
                tenant_id: terminal_tenant_id,
                trigger_id: terminal_trigger_id,
                fire_slot,
            })
            .await
            .expect("mark terminally failed")
            .expect("terminal failure should persist");
        assert_eq!(terminal_failed.last_run_at, Some(failure_previous_run_at));
        assert_eq!(terminal_failed.last_fired_slot, Some(failure_previous_slot));
        assert_eq!(terminal_failed.last_status, Some(TriggerRunStatus::Error));
        assert_eq!(terminal_failed.state, TriggerState::Completed);
        assert_eq!(terminal_failed.active_fire_slot, None);
        assert_eq!(terminal_failed.active_run_ref, None);
        assert_eq!(terminal_failed.next_run_at, fire_slot);
    }

    async fn assert_fire_result_rejects_invalid_next_run_at(repo: &impl TriggerRepository) {
        let fire_slot = ts(1_704_067_200);
        let stale_fire_slot = ts(1_704_067_140);

        let early_claim_trigger_id = TriggerId::parse("01J0000000000000000000000D").expect("ulid");
        let early_claim_tenant_id = tenant("tenant-early-claim");
        repo.upsert_trigger(sample_record(
            early_claim_trigger_id,
            early_claim_tenant_id.clone(),
            fire_slot,
        ))
        .await
        .expect("insert early-claim record");
        assert!(
            repo.claim_due_fire(ClaimDueFireRequest {
                tenant_id: early_claim_tenant_id,
                trigger_id: early_claim_trigger_id,
                fire_slot,
                now: ts(1_704_067_199),
            })
            .await
            .expect("early claim")
            .matches_not_due(),
            "scheduled row must not be claimable before the requested fire slot is due"
        );

        let stale_accepted_trigger_id =
            TriggerId::parse("01J00000000000000000000010").expect("ulid");
        let stale_accepted_tenant_id = tenant("tenant-stale-accepted");
        let mut stale_accepted_record = sample_record(
            stale_accepted_trigger_id,
            stale_accepted_tenant_id.clone(),
            fire_slot,
        );
        stale_accepted_record.active_fire_slot = Some(fire_slot);
        repo.upsert_trigger(stale_accepted_record.clone())
            .await
            .expect("insert stale accepted record");
        assert!(
            repo.mark_fire_accepted(FireAcceptedRequest {
                tenant_id: stale_accepted_tenant_id.clone(),
                trigger_id: stale_accepted_trigger_id,
                fire_slot: stale_fire_slot,
                run_id: TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f62")
                    .expect("valid run"),
                thread_id: ThreadId::new("01890f0f-bb01-7000-8000-000000000001")
                    .expect("valid thread id"),
                submitted_at: fire_slot,
                next_run_at: ts(1_704_067_260),
            })
            .await
            .expect("stale accepted update")
            .is_none()
        );
        let reloaded = repo
            .get_trigger(stale_accepted_tenant_id, stale_accepted_trigger_id)
            .await
            .expect("reload stale accepted")
            .expect("record present");
        assert_eq!(reloaded, stale_accepted_record);

        let stale_replayed_trigger_id =
            TriggerId::parse("01J00000000000000000000011").expect("ulid");
        let stale_replayed_tenant_id = tenant("tenant-stale-replayed");
        let mut stale_replayed_record = sample_record(
            stale_replayed_trigger_id,
            stale_replayed_tenant_id.clone(),
            fire_slot,
        );
        stale_replayed_record.active_fire_slot = Some(fire_slot);
        repo.upsert_trigger(stale_replayed_record.clone())
            .await
            .expect("insert stale replayed record");
        assert!(
            repo.mark_fire_replayed(FireReplayedRequest {
                tenant_id: stale_replayed_tenant_id.clone(),
                trigger_id: stale_replayed_trigger_id,
                fire_slot: stale_fire_slot,
                original_run_id: TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f63")
                    .expect("valid run"),
                thread_id: None,
                replayed_at: fire_slot,
                next_run_at: ts(1_704_067_260),
            })
            .await
            .expect("stale replayed update")
            .is_none()
        );
        let reloaded = repo
            .get_trigger(stale_replayed_tenant_id, stale_replayed_trigger_id)
            .await
            .expect("reload stale replayed")
            .expect("record present");
        assert_eq!(reloaded, stale_replayed_record);

        let stale_retryable_trigger_id =
            TriggerId::parse("01J00000000000000000000012").expect("ulid");
        let stale_retryable_tenant_id = tenant("tenant-stale-retryable");
        let mut stale_retryable_record = sample_record(
            stale_retryable_trigger_id,
            stale_retryable_tenant_id.clone(),
            fire_slot,
        );
        stale_retryable_record.active_fire_slot = Some(fire_slot);
        repo.upsert_trigger(stale_retryable_record.clone())
            .await
            .expect("insert stale retryable record");
        assert!(
            repo.mark_fire_retryable_failed(FireRetryableFailedRequest {
                tenant_id: stale_retryable_tenant_id.clone(),
                trigger_id: stale_retryable_trigger_id,
                fire_slot: stale_fire_slot,
            })
            .await
            .expect("stale retryable update")
            .is_none()
        );
        let reloaded = repo
            .get_trigger(stale_retryable_tenant_id, stale_retryable_trigger_id)
            .await
            .expect("reload stale retryable")
            .expect("record present");
        assert_eq!(reloaded, stale_retryable_record);

        let stale_permanent_trigger_id =
            TriggerId::parse("01J00000000000000000000013").expect("ulid");
        let stale_permanent_tenant_id = tenant("tenant-stale-permanent");
        let mut stale_permanent_record = sample_record(
            stale_permanent_trigger_id,
            stale_permanent_tenant_id.clone(),
            fire_slot,
        );
        stale_permanent_record.active_fire_slot = Some(fire_slot);
        repo.upsert_trigger(stale_permanent_record.clone())
            .await
            .expect("insert stale permanent record");
        assert!(
            repo.mark_fire_permanently_failed(FirePermanentFailedRequest {
                tenant_id: stale_permanent_tenant_id.clone(),
                trigger_id: stale_permanent_trigger_id,
                fire_slot: stale_fire_slot,
                next_run_at: ts(1_704_067_260),
            })
            .await
            .expect("stale permanent update")
            .is_none()
        );
        let reloaded = repo
            .get_trigger(stale_permanent_tenant_id, stale_permanent_trigger_id)
            .await
            .expect("reload stale permanent")
            .expect("record present");
        assert_eq!(reloaded, stale_permanent_record);

        let stale_terminal_trigger_id =
            TriggerId::parse("01J00000000000000000000014").expect("ulid");
        let stale_terminal_tenant_id = tenant("tenant-stale-terminal");
        let mut stale_terminal_record = sample_record(
            stale_terminal_trigger_id,
            stale_terminal_tenant_id.clone(),
            fire_slot,
        );
        stale_terminal_record.active_fire_slot = Some(fire_slot);
        repo.upsert_trigger(stale_terminal_record.clone())
            .await
            .expect("insert stale terminal record");
        assert!(
            repo.mark_fire_terminally_failed(FireTerminalFailedRequest {
                tenant_id: stale_terminal_tenant_id.clone(),
                trigger_id: stale_terminal_trigger_id,
                fire_slot: stale_fire_slot,
            })
            .await
            .expect("stale terminal update")
            .is_none()
        );
        let reloaded = repo
            .get_trigger(stale_terminal_tenant_id, stale_terminal_trigger_id)
            .await
            .expect("reload stale terminal")
            .expect("record present");
        assert_eq!(reloaded, stale_terminal_record);

        let accepted_trigger_id = TriggerId::parse("01J0000000000000000000000E").expect("ulid");
        let accepted_tenant_id = tenant("tenant-invalid-accepted");
        let mut accepted_record =
            sample_record(accepted_trigger_id, accepted_tenant_id.clone(), fire_slot);
        accepted_record.active_fire_slot = Some(fire_slot);
        repo.upsert_trigger(accepted_record)
            .await
            .expect("insert invalid accepted record");
        let error = repo
            .mark_fire_accepted(FireAcceptedRequest {
                tenant_id: accepted_tenant_id,
                trigger_id: accepted_trigger_id,
                fire_slot,
                run_id: TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f60")
                    .expect("valid run"),
                thread_id: ThreadId::new("01890f0f-cc01-7000-8000-000000000001")
                    .expect("valid thread id"),
                submitted_at: fire_slot,
                next_run_at: fire_slot,
            })
            .await
            .expect_err("accepted result rejects non-future next_run_at");
        assert_error_contains(error, "must be after the claimed fire slot");

        let replayed_trigger_id = TriggerId::parse("01J0000000000000000000000F").expect("ulid");
        let replayed_tenant_id = tenant("tenant-invalid-replayed");
        let mut replayed_record =
            sample_record(replayed_trigger_id, replayed_tenant_id.clone(), fire_slot);
        replayed_record.active_fire_slot = Some(fire_slot);
        repo.upsert_trigger(replayed_record)
            .await
            .expect("insert invalid replayed record");
        let error = repo
            .mark_fire_replayed(FireReplayedRequest {
                tenant_id: replayed_tenant_id,
                trigger_id: replayed_trigger_id,
                fire_slot,
                original_run_id: TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f61")
                    .expect("valid run"),
                thread_id: None,
                replayed_at: fire_slot,
                next_run_at: fire_slot,
            })
            .await
            .expect_err("replayed result rejects non-future next_run_at");
        assert_error_contains(error, "must be after the claimed fire slot");

        let retryable_trigger_id = TriggerId::parse("01J00000000000000000000007").expect("ulid");
        let retryable_tenant_id = tenant("tenant-invalid-retryable");
        let mut retryable_record =
            sample_record(retryable_trigger_id, retryable_tenant_id.clone(), fire_slot);
        retryable_record.active_fire_slot = Some(fire_slot);
        retryable_record.next_run_at = ts(1_704_067_260);
        repo.upsert_trigger(retryable_record)
            .await
            .expect("insert invalid retryable record");
        let error = repo
            .mark_fire_retryable_failed(FireRetryableFailedRequest {
                tenant_id: retryable_tenant_id,
                trigger_id: retryable_trigger_id,
                fire_slot,
            })
            .await
            .expect_err("retryable failure rejects advanced next_run_at");
        assert_error_contains(error, "at or before the failed fire slot");

        let permanent_trigger_id = TriggerId::parse("01J00000000000000000000008").expect("ulid");
        let permanent_tenant_id = tenant("tenant-invalid-permanent");
        let mut permanent_record =
            sample_record(permanent_trigger_id, permanent_tenant_id.clone(), fire_slot);
        permanent_record.active_fire_slot = Some(fire_slot);
        repo.upsert_trigger(permanent_record)
            .await
            .expect("insert invalid permanent record");
        let error = repo
            .mark_fire_permanently_failed(FirePermanentFailedRequest {
                tenant_id: permanent_tenant_id,
                trigger_id: permanent_trigger_id,
                fire_slot,
                next_run_at: fire_slot,
            })
            .await
            .expect_err("permanent failure rejects non-future next_run_at");
        assert_error_contains(error, "must be after the claimed fire slot");
    }

    async fn assert_fire_clear_contract(repo: &impl TriggerRepository) {
        let trigger_id = TriggerId::parse("01J00000000000000000000016").expect("ulid");
        let tenant_id = tenant("tenant-clear");
        let fire_slot = ts(1_704_067_200);
        let run_id = TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f66").expect("valid run");
        let mut active_record = sample_record(trigger_id, tenant_id.clone(), fire_slot);
        active_record.active_fire_slot = Some(fire_slot);
        active_record.active_run_ref = Some(run_id);
        repo.upsert_trigger(active_record.clone())
            .await
            .expect("insert active record");

        let wrong_run_clear = repo
            .clear_active_fire(ClearActiveFireRequest {
                tenant_id: tenant_id.clone(),
                trigger_id,
                fire_slot,
                run_id: TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f67")
                    .expect("valid run"),
                status: TriggerRunHistoryStatus::Ok,
            })
            .await
            .expect("clear with wrong run ref");
        assert!(
            wrong_run_clear.is_none(),
            "mismatched run ref must not clear"
        );
        let reloaded = repo
            .get_trigger(tenant_id.clone(), trigger_id)
            .await
            .expect("reload mismatched clear")
            .expect("record present");
        assert_eq!(reloaded, active_record);

        let wrong_slot_clear = repo
            .clear_active_fire(ClearActiveFireRequest {
                tenant_id: tenant_id.clone(),
                trigger_id,
                fire_slot: fire_slot + chrono::Duration::minutes(1),
                run_id,
                status: TriggerRunHistoryStatus::Ok,
            })
            .await
            .expect("clear with wrong fire slot");
        assert!(
            wrong_slot_clear.is_none(),
            "mismatched fire slot must not clear"
        );
        let reloaded = repo
            .get_trigger(tenant_id.clone(), trigger_id)
            .await
            .expect("reload wrong-slot clear")
            .expect("record present");
        assert_eq!(reloaded, active_record);

        let wrong_tenant_clear = repo
            .clear_active_fire(ClearActiveFireRequest {
                tenant_id: tenant("tenant-clear-other"),
                trigger_id,
                fire_slot,
                run_id,
                status: TriggerRunHistoryStatus::Ok,
            })
            .await
            .expect("clear with wrong tenant");
        assert!(
            wrong_tenant_clear.is_none(),
            "mismatched tenant must not clear"
        );
        let reloaded = repo
            .get_trigger(tenant_id.clone(), trigger_id)
            .await
            .expect("reload wrong-tenant clear")
            .expect("record present");
        assert_eq!(reloaded, active_record);

        let cleared = repo
            .clear_active_fire(ClearActiveFireRequest {
                tenant_id: tenant_id.clone(),
                trigger_id,
                fire_slot,
                run_id,
                status: TriggerRunHistoryStatus::Ok,
            })
            .await
            .expect("clear active fire")
            .expect("active fire should clear");
        let mut expected = active_record;
        expected.active_fire_slot = None;
        expected.active_run_ref = None;
        assert_eq!(cleared, expected);

        let persisted = repo
            .get_trigger(tenant_id, trigger_id)
            .await
            .expect("reload cleared record")
            .expect("record present");
        assert_eq!(persisted, expected);
    }

    async fn assert_fire_claim_exclusions_and_active_gate_contract(repo: &impl TriggerRepository) {
        let fire_slot = ts(1_704_067_200);
        let tenant_id = tenant("tenant-a");

        let base_trigger_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
        let base = sample_record(base_trigger_id, tenant_id.clone(), fire_slot);
        repo.upsert_trigger(base.clone())
            .await
            .expect("insert base");

        let paused = {
            let mut record = sample_record(
                TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZY").expect("ulid"),
                tenant("tenant-paused"),
                fire_slot,
            );
            record.state = TriggerState::Paused;
            record
        };
        repo.upsert_trigger(paused.clone())
            .await
            .expect("insert paused");

        let paused_active = {
            let mut record = sample_record(
                TriggerId::parse("01J0000000000000000000000B").expect("ulid"),
                tenant("tenant-paused-active"),
                fire_slot,
            );
            record.state = TriggerState::Paused;
            record.active_fire_slot = Some(fire_slot);
            record.active_run_ref =
                Some(TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5f").expect("valid run"));
            record
        };
        repo.upsert_trigger(paused_active.clone())
            .await
            .expect("insert paused active");

        let completed = {
            let mut record = sample_record(
                TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZX").expect("ulid"),
                tenant("tenant-completed"),
                fire_slot,
            );
            record.state = TriggerState::Completed;
            record
        };
        repo.upsert_trigger(completed.clone())
            .await
            .expect("insert completed");

        let future = sample_record(
            TriggerId::parse("01J00000000000000000000002").expect("ulid"),
            tenant("tenant-future"),
            ts(1_704_067_320),
        );
        repo.upsert_trigger(future.clone())
            .await
            .expect("insert future");

        assert!(
            repo.claim_due_fire(ClaimDueFireRequest {
                tenant_id: tenant_id.clone(),
                trigger_id: base_trigger_id,
                fire_slot: ts(1_704_067_260),
                now: fire_slot,
            })
            .await
            .expect("wrong fire slot claim")
            .matches_not_due(),
            "wrong fire slot must not be claimable"
        );

        assert!(
            repo.claim_due_fire(ClaimDueFireRequest {
                tenant_id: tenant_id.clone(),
                trigger_id: TriggerId::parse("01J00000000000000000000009").expect("ulid"),
                fire_slot,
                now: fire_slot,
            })
            .await
            .expect("missing row claim")
            .matches_not_found(),
            "missing row must not be claimable"
        );

        assert!(
            repo.claim_due_fire(ClaimDueFireRequest {
                tenant_id: paused.tenant_id.clone(),
                trigger_id: paused.trigger_id,
                fire_slot,
                now: fire_slot,
            })
            .await
            .expect("paused claim")
            .matches_not_due(),
            "paused row must not be claimable"
        );

        assert!(
            repo.claim_due_fire(ClaimDueFireRequest {
                tenant_id: paused_active.tenant_id.clone(),
                trigger_id: paused_active.trigger_id,
                fire_slot,
                now: fire_slot,
            })
            .await
            .expect("paused active claim")
            .matches_not_due(),
            "paused row with stale active metadata must not be claimable"
        );

        assert!(
            repo.claim_due_fire(ClaimDueFireRequest {
                tenant_id: completed.tenant_id.clone(),
                trigger_id: completed.trigger_id,
                fire_slot,
                now: fire_slot,
            })
            .await
            .expect("completed claim")
            .matches_not_due(),
            "completed row must not be claimable"
        );

        assert!(
            repo.claim_due_fire(ClaimDueFireRequest {
                tenant_id: future.tenant_id.clone(),
                trigger_id: future.trigger_id,
                fire_slot: future.next_run_at,
                now: fire_slot,
            })
            .await
            .expect("future claim")
            .matches_not_due(),
            "future next_run_at must not be claimable"
        );

        let claimed = repo
            .claim_due_fire(ClaimDueFireRequest {
                tenant_id: tenant_id.clone(),
                trigger_id: base_trigger_id,
                fire_slot,
                now: fire_slot,
            })
            .await
            .expect("claim base row");
        let ClaimDueFireOutcome::Claimed(claimed) = claimed else {
            panic!("base row should be claimable, got {claimed:?}");
        };
        assert_eq!(claimed.record.active_fire_slot, Some(fire_slot));
        assert_eq!(claimed.record.active_run_ref, None);

        let mut active_fire = claimed.record.clone();
        active_fire.last_status = Some(TriggerRunStatus::Error);
        repo.upsert_trigger(active_fire)
            .await
            .expect("persist active row with error status");
        assert!(
            repo.claim_due_fire(ClaimDueFireRequest {
                tenant_id: tenant_id.clone(),
                trigger_id: base_trigger_id,
                fire_slot,
                now: fire_slot,
            })
            .await
            .expect("active fire slot claim")
            .matches_already_active(Some(fire_slot), None),
            "active fire slot must block a second claim"
        );

        let active_run_ref =
            Some(TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5a").expect("valid run"));
        let mut active_run = claimed.record.clone();
        active_run.active_run_ref = active_run_ref;
        repo.upsert_trigger(active_run)
            .await
            .expect("persist active row with run ref");
        assert!(
            repo.claim_due_fire(ClaimDueFireRequest {
                tenant_id: tenant_id.clone(),
                trigger_id: base_trigger_id,
                fire_slot,
                now: fire_slot,
            })
            .await
            .expect("active run ref claim")
            .matches_already_active(Some(fire_slot), active_run_ref),
            "active run ref must block a second claim"
        );

        let run_only_trigger_id = TriggerId::parse("01J0000000000000000000000A").expect("ulid");
        let run_only_tenant_id = tenant("tenant-run-only");
        let mut run_only =
            sample_record(run_only_trigger_id, run_only_tenant_id.clone(), fire_slot);
        run_only.active_fire_slot = None;
        run_only.active_run_ref =
            Some(TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5e").expect("valid run"));
        assert!(run_only.has_active_fire());
        let error = repo
            .upsert_trigger(run_only)
            .await
            .expect_err("active_run_ref without fire slot must be rejected");
        assert_error_contains(error, "active_run_ref requires active_fire_slot");

        assert!(
            repo.get_trigger(run_only_tenant_id, run_only_trigger_id)
                .await
                .expect("run-only row lookup")
                .is_none(),
            "invalid run-ref-only row must not be persisted"
        );

        let mut status_only = sample_record(
            TriggerId::parse("01J00000000000000000000003").expect("ulid"),
            tenant("tenant-status"),
            fire_slot,
        );
        status_only.last_status = Some(TriggerRunStatus::Error);
        repo.upsert_trigger(status_only.clone())
            .await
            .expect("insert status-only record");
        let status_claim = repo
            .claim_due_fire(ClaimDueFireRequest {
                tenant_id: status_only.tenant_id.clone(),
                trigger_id: status_only.trigger_id,
                fire_slot,
                now: ts(1_704_067_260),
            })
            .await
            .expect("status-only claim");
        let ClaimDueFireOutcome::Claimed(status_claim) = status_claim else {
            panic!("status-only row should still be claimable, got {status_claim:?}");
        };
        assert_eq!(status_claim.record.active_fire_slot, Some(fire_slot));
        assert_eq!(status_claim.record.active_run_ref, None);
        assert_eq!(
            status_claim.record.last_status,
            Some(TriggerRunStatus::Error)
        );
    }

    async fn assert_durable_claim_is_atomic<R>(repo: std::sync::Arc<R>)
    where
        R: TriggerRepository + 'static,
    {
        let trigger_id = TriggerId::parse("01J0000000000000000000000C").expect("ulid");
        let tenant_id = tenant("tenant-atomic");
        let fire_slot = ts(1_704_067_200);
        let now = fire_slot;
        repo.upsert_trigger(sample_record(trigger_id, tenant_id.clone(), fire_slot))
            .await
            .expect("insert atomic record");

        let first_repo = repo.clone();
        let second_repo = repo.clone();
        let first_tenant_id = tenant_id.clone();
        let second_tenant_id = tenant_id;
        let first = async move {
            tokio::task::yield_now().await;
            first_repo
                .claim_due_fire(ClaimDueFireRequest {
                    tenant_id: first_tenant_id,
                    trigger_id,
                    fire_slot,
                    now,
                })
                .await
        };
        let second = async move {
            tokio::task::yield_now().await;
            second_repo
                .claim_due_fire(ClaimDueFireRequest {
                    tenant_id: second_tenant_id,
                    trigger_id,
                    fire_slot,
                    now,
                })
                .await
        };

        let (first, second) = tokio::join!(first, second);
        let outcomes = [first.expect("first claim"), second.expect("second claim")];

        let claimed = outcomes
            .iter()
            .find_map(|outcome| match outcome {
                ClaimDueFireOutcome::Claimed(claimed) => Some(claimed.clone()),
                _ => None,
            })
            .expect("one poller must claim the fire");
        let already_active_count = outcomes
            .iter()
            .filter(|outcome| {
                matches!(
                    outcome,
                    ClaimDueFireOutcome::AlreadyActive {
                        active_fire_slot: Some(slot),
                        active_run_ref: None,
                    } if *slot == fire_slot
                )
            })
            .count();

        assert_eq!(
            already_active_count, 1,
            "one poller must observe the active claim"
        );
        assert_eq!(claimed.fire_slot, fire_slot);
        assert_eq!(claimed.record.active_fire_slot, Some(fire_slot));
        assert_eq!(claimed.record.active_run_ref, None);

        let persisted = repo
            .get_trigger(tenant("tenant-atomic"), trigger_id)
            .await
            .expect("reload atomic record")
            .expect("record present");
        assert_eq!(persisted.active_fire_slot, Some(fire_slot));
        assert_eq!(persisted.active_run_ref, None);
    }

    async fn assert_mark_fire_accepted_is_idempotent_under_concurrency<R>(
        repo: std::sync::Arc<R>,
        trigger_id: TriggerId,
        tenant_id: TenantId,
    ) where
        R: TriggerRepository + 'static,
    {
        let fire_slot = ts(1_704_067_200);
        let accepted_at = ts(1_704_067_205);
        let record = sample_record(trigger_id, tenant_id.clone(), fire_slot);
        let next_run_at = record
            .schedule
            .next_slot_after(fire_slot)
            .expect("next slot calculation")
            .expect("future slot");
        repo.upsert_trigger(record).await.expect("insert record");
        assert!(matches!(
            repo.claim_due_fire(ClaimDueFireRequest {
                tenant_id: tenant_id.clone(),
                trigger_id,
                fire_slot,
                now: fire_slot,
            })
            .await
            .expect("claim fire"),
            ClaimDueFireOutcome::Claimed(_)
        ));

        let run_id = TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f64").expect("valid run");
        let request = FireAcceptedRequest {
            tenant_id: tenant_id.clone(),
            trigger_id,
            fire_slot,
            run_id,
            thread_id: ThreadId::new("01890f0f-dd01-7000-8000-000000000001")
                .expect("valid thread id"),
            submitted_at: accepted_at,
            next_run_at,
        };
        let first_repo = repo.clone();
        let second_repo = repo.clone();
        let first_request = request.clone();
        let second_request = request;
        let first = async move {
            tokio::task::yield_now().await;
            first_repo.mark_fire_accepted(first_request).await
        };
        let second = async move {
            tokio::task::yield_now().await;
            second_repo.mark_fire_accepted(second_request).await
        };

        let (first, second) = tokio::join!(first, second);
        let first = first
            .expect("first accepted result")
            .expect("first accepted record");
        let second = second
            .expect("second accepted result")
            .expect("second accepted record");
        assert_eq!(first, second);
        assert_eq!(first.active_fire_slot, Some(fire_slot));
        assert_eq!(first.active_run_ref, Some(run_id));
        assert_eq!(first.last_run_at, Some(accepted_at));
        assert_eq!(first.last_fired_slot, Some(fire_slot));
        assert_eq!(first.last_status, Some(TriggerRunStatus::Ok));

        let persisted = repo
            .get_trigger(tenant_id, trigger_id)
            .await
            .expect("reload accepted result")
            .expect("record present");
        assert_eq!(persisted, first);
    }

    async fn assert_mark_fire_replayed_is_idempotent_under_concurrency<R>(
        repo: std::sync::Arc<R>,
        trigger_id: TriggerId,
        tenant_id: TenantId,
    ) where
        R: TriggerRepository + 'static,
    {
        let fire_slot = ts(1_704_067_200);
        let replayed_at = ts(1_704_067_205);
        let record = sample_record(trigger_id, tenant_id.clone(), fire_slot);
        let next_run_at = record
            .schedule
            .next_slot_after(fire_slot)
            .expect("next slot calculation")
            .expect("future slot");
        repo.upsert_trigger(record).await.expect("insert record");
        assert!(matches!(
            repo.claim_due_fire(ClaimDueFireRequest {
                tenant_id: tenant_id.clone(),
                trigger_id,
                fire_slot,
                now: fire_slot,
            })
            .await
            .expect("claim fire"),
            ClaimDueFireOutcome::Claimed(_)
        ));

        let original_run_id =
            TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f65").expect("valid run");
        let request = FireReplayedRequest {
            tenant_id: tenant_id.clone(),
            trigger_id,
            fire_slot,
            original_run_id,
            thread_id: None,
            replayed_at,
            next_run_at,
        };
        let first_repo = repo.clone();
        let second_repo = repo.clone();
        let first_request = request.clone();
        let second_request = request;
        let first = async move {
            tokio::task::yield_now().await;
            first_repo.mark_fire_replayed(first_request).await
        };
        let second = async move {
            tokio::task::yield_now().await;
            second_repo.mark_fire_replayed(second_request).await
        };

        let (first, second) = tokio::join!(first, second);
        let first = first
            .expect("first replayed result")
            .expect("first replayed record");
        let second = second
            .expect("second replayed result")
            .expect("second replayed record");
        assert_eq!(first, second);
        assert_eq!(first.active_fire_slot, Some(fire_slot));
        assert_eq!(first.active_run_ref, Some(original_run_id));
        assert_eq!(first.last_run_at, Some(replayed_at));
        assert_eq!(first.last_fired_slot, Some(fire_slot));
        assert_eq!(first.last_status, Some(TriggerRunStatus::Ok));

        let persisted = repo
            .get_trigger(tenant_id, trigger_id)
            .await
            .expect("reload replayed result")
            .expect("record present");
        assert_eq!(persisted, first);
    }

    trait ClaimDueFireOutcomeAssertions {
        fn matches_not_found(&self) -> bool;
        fn matches_not_due(&self) -> bool;
        fn matches_already_active(
            &self,
            active_fire_slot: Option<Timestamp>,
            active_run_ref: Option<TurnRunId>,
        ) -> bool;
    }

    impl ClaimDueFireOutcomeAssertions for ClaimDueFireOutcome {
        fn matches_not_found(&self) -> bool {
            matches!(self, Self::NotFound)
        }

        fn matches_not_due(&self) -> bool {
            matches!(self, Self::NotDue { .. })
        }

        fn matches_already_active(
            &self,
            active_fire_slot: Option<Timestamp>,
            active_run_ref: Option<TurnRunId>,
        ) -> bool {
            matches!(
                self,
                Self::AlreadyActive {
                    active_fire_slot: actual_fire_slot,
                    active_run_ref: actual_run_ref,
                } if *actual_fire_slot == active_fire_slot && *actual_run_ref == active_run_ref
            )
        }
    }

    async fn assert_durable_fire_claim_contract(repo: &impl TriggerRepository) {
        assert_fire_claim_and_update_contract(repo).await;
        assert_fire_claim_exclusions_and_active_gate_contract(repo).await;
        assert_fire_result_rejects_invalid_next_run_at(repo).await;
        assert_fire_clear_contract(repo).await;
        assert_run_history_lifecycle_contract(repo).await;
        assert_run_history_retention_contract(repo).await;
    }

    async fn assert_run_history_lifecycle_contract(repo: &impl TriggerRepository) {
        let trigger_id = TriggerId::parse("01J00000000000000000000030").expect("ulid");
        let tenant_id = tenant("tenant-run-history");
        let fire_slot = ts(1_704_067_200);
        let claim_now = ts(1_704_067_203);
        let submitted_at = ts(1_704_067_205);
        let record = sample_record(trigger_id, tenant_id.clone(), fire_slot);
        let expected_next_run_at = record
            .schedule
            .next_slot_after(fire_slot)
            .expect("next slot calculation")
            .expect("future slot");
        repo.upsert_trigger(record).await.expect("insert record");

        let claimed = repo
            .claim_due_fire(ClaimDueFireRequest {
                tenant_id: tenant_id.clone(),
                trigger_id,
                fire_slot,
                now: claim_now,
            })
            .await
            .expect("claim fire");
        assert!(matches!(claimed, ClaimDueFireOutcome::Claimed(_)));

        let runs = repo
            .list_trigger_run_history(tenant_id.clone(), trigger_id, 10)
            .await
            .expect("list claimed run history");
        assert_eq!(runs.len(), 1);
        // Pre-acceptance: thread_id is None — no canonical thread exists yet.
        assert_eq!(runs[0].tenant_id, tenant_id);
        assert_eq!(runs[0].trigger_id, trigger_id);
        assert_eq!(runs[0].fire_slot, fire_slot);
        assert_eq!(runs[0].run_id, None);
        assert_eq!(
            runs[0].thread_id, None,
            "claim-time run must have no canonical thread"
        );
        assert_eq!(runs[0].status, TriggerRunHistoryStatus::Running);
        assert_eq!(runs[0].submitted_at, claim_now);
        assert_eq!(runs[0].completed_at, None);

        let run_id = TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f80").expect("valid run");
        let canonical_thread_id =
            ThreadId::new("01890f0f-c000-7000-8000-000000000001").expect("valid thread id");
        repo.mark_fire_accepted(FireAcceptedRequest {
            tenant_id: tenant_id.clone(),
            trigger_id,
            fire_slot,
            run_id,
            thread_id: canonical_thread_id.clone(),
            submitted_at,
            next_run_at: expected_next_run_at,
        })
        .await
        .expect("mark accepted")
        .expect("accepted fire should persist");

        let runs = repo
            .list_trigger_run_history(tenant_id.clone(), trigger_id, 10)
            .await
            .expect("list accepted run history");
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].run_id, Some(run_id));
        assert_eq!(runs[0].status, TriggerRunHistoryStatus::Running);
        assert_eq!(runs[0].submitted_at, submitted_at);
        assert_eq!(runs[0].completed_at, None);
        // After acceptance, thread_id must be Some(canonical UUID).
        assert_eq!(
            runs[0].thread_id,
            Some(canonical_thread_id.clone()),
            "thread_id must be set to the canonical UUID at fire acceptance"
        );

        repo.clear_active_fire(ClearActiveFireRequest {
            tenant_id: tenant_id.clone(),
            trigger_id,
            fire_slot,
            run_id,
            status: TriggerRunHistoryStatus::Ok,
        })
        .await
        .expect("clear active fire")
        .expect("active fire should clear");

        let runs = repo
            .list_trigger_run_history(tenant_id.clone(), trigger_id, 10)
            .await
            .expect("list cleared run history");
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].run_id, Some(run_id));
        assert_eq!(runs[0].status, TriggerRunHistoryStatus::Ok);
        assert_eq!(runs[0].submitted_at, submitted_at);
        assert!(runs[0].completed_at.is_some());

        let empty_runs = repo
            .list_trigger_run_history(tenant_id.clone(), trigger_id, 0)
            .await
            .expect("zero limit returns empty run history");
        assert!(empty_runs.is_empty());

        let second_fire_slot = expected_next_run_at;
        let second_claim_now = second_fire_slot + chrono::Duration::seconds(3);
        let second_submitted_at = second_fire_slot + chrono::Duration::seconds(5);
        let second_run_id =
            TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f81").expect("valid run");
        let second_next_run_at = second_fire_slot + chrono::Duration::days(1);
        let second_claimed = repo
            .claim_due_fire(ClaimDueFireRequest {
                tenant_id: tenant_id.clone(),
                trigger_id,
                fire_slot: second_fire_slot,
                now: second_claim_now,
            })
            .await
            .expect("claim second fire");
        assert!(matches!(second_claimed, ClaimDueFireOutcome::Claimed(_)));
        repo.mark_fire_accepted(FireAcceptedRequest {
            tenant_id: tenant_id.clone(),
            trigger_id,
            fire_slot: second_fire_slot,
            run_id: second_run_id,
            thread_id: ThreadId::new("01890f0f-c000-7000-8000-000000000002")
                .expect("valid thread id"),
            submitted_at: second_submitted_at,
            next_run_at: second_next_run_at,
        })
        .await
        .expect("mark second fire accepted")
        .expect("second accepted fire should persist");

        let newest_limited_runs = repo
            .list_trigger_run_history(tenant_id.clone(), trigger_id, 1)
            .await
            .expect("list bounded run history");
        assert_eq!(newest_limited_runs.len(), 1);
        assert_eq!(newest_limited_runs[0].fire_slot, second_fire_slot);
        assert_eq!(newest_limited_runs[0].run_id, Some(second_run_id));
        assert_eq!(
            newest_limited_runs[0].status,
            TriggerRunHistoryStatus::Running
        );

        let other_trigger_id = TriggerId::parse("01J00000000000000000000032").expect("ulid");
        let batch_runs = repo
            .list_trigger_run_history_batch(tenant_id.clone(), &[trigger_id, other_trigger_id], 1)
            .await
            .expect("list batched run history");
        assert_eq!(
            batch_runs
                .get(&trigger_id)
                .expect("trigger history present")[0]
                .fire_slot,
            second_fire_slot
        );
        assert!(
            batch_runs
                .get(&other_trigger_id)
                .map(Vec::is_empty)
                .unwrap_or(true)
        );

        let other_trigger_runs = repo
            .list_trigger_run_history(tenant_id.clone(), other_trigger_id, 10)
            .await
            .expect("list other trigger history");
        assert!(other_trigger_runs.is_empty());

        let other_tenant_runs = repo
            .list_trigger_run_history(tenant("tenant-run-history-other"), trigger_id, 10)
            .await
            .expect("list other tenant history");
        assert!(other_tenant_runs.is_empty());

        let failed_trigger_id = TriggerId::parse("01J00000000000000000000031").expect("ulid");
        let failed_tenant_id = tenant("tenant-run-history-failed");
        repo.upsert_trigger(sample_record(
            failed_trigger_id,
            failed_tenant_id.clone(),
            fire_slot,
        ))
        .await
        .expect("insert failed record");
        let failed_claim = repo
            .claim_due_fire(ClaimDueFireRequest {
                tenant_id: failed_tenant_id.clone(),
                trigger_id: failed_trigger_id,
                fire_slot,
                now: claim_now,
            })
            .await
            .expect("claim failed fire");
        assert!(matches!(failed_claim, ClaimDueFireOutcome::Claimed(_)));

        repo.mark_fire_terminally_failed(FireTerminalFailedRequest {
            tenant_id: failed_tenant_id.clone(),
            trigger_id: failed_trigger_id,
            fire_slot,
        })
        .await
        .expect("mark terminal failure")
        .expect("terminal failure should persist");

        let runs = repo
            .list_trigger_run_history(failed_tenant_id, failed_trigger_id, 10)
            .await
            .expect("list failed run history");
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].run_id, None);
        assert_eq!(runs[0].status, TriggerRunHistoryStatus::Error);
        assert!(runs[0].completed_at.is_some());

        let missing_history_trigger_id =
            TriggerId::parse("01J00000000000000000000034").expect("ulid");
        let missing_history_tenant_id = tenant("tenant-run-history-missing-running-row");
        let missing_history_fire_slot = ts(1_704_067_300);
        let missing_history_run_id =
            TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f82").expect("valid run");
        let mut missing_history_record = sample_record(
            missing_history_trigger_id,
            missing_history_tenant_id.clone(),
            missing_history_fire_slot,
        );
        missing_history_record.active_fire_slot = Some(missing_history_fire_slot);
        missing_history_record.active_run_ref = Some(missing_history_run_id);
        repo.upsert_trigger(missing_history_record)
            .await
            .expect("insert active record without run history row");

        repo.clear_active_fire(ClearActiveFireRequest {
            tenant_id: missing_history_tenant_id.clone(),
            trigger_id: missing_history_trigger_id,
            fire_slot: missing_history_fire_slot,
            run_id: missing_history_run_id,
            status: TriggerRunHistoryStatus::Ok,
        })
        .await
        .expect("clear active fire without running history row")
        .expect("active fire should clear");

        let runs = repo
            .list_trigger_run_history(missing_history_tenant_id, missing_history_trigger_id, 10)
            .await
            .expect("list inserted completion run history");
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].run_id, Some(missing_history_run_id));
        assert_eq!(runs[0].status, TriggerRunHistoryStatus::Ok);
        assert_eq!(
            runs[0].submitted_at,
            runs[0].completed_at.expect("completion timestamp"),
            "completion-only run-history rows must use completed_at as fallback submitted_at"
        );

        // Replay thread-id semantics: a replay carrying the canonical thread id
        // persists it, and a later replay WITHOUT a resolved scope must not
        // clobber the stored canonical id back to None (which would regress the
        // Automations panel chat link to a 404).
        let replay_thread_trigger_id =
            TriggerId::parse("01J00000000000000000000035").expect("ulid");
        let replay_thread_tenant_id = tenant("tenant-run-history-replay-thread");
        let replay_thread_record = sample_record(
            replay_thread_trigger_id,
            replay_thread_tenant_id.clone(),
            fire_slot,
        );
        let replay_next_run_at = replay_thread_record
            .schedule
            .next_slot_after(fire_slot)
            .expect("next slot calculation")
            .expect("future slot");
        repo.upsert_trigger(replay_thread_record)
            .await
            .expect("insert replay-thread record");
        let replay_thread_claim = repo
            .claim_due_fire(ClaimDueFireRequest {
                tenant_id: replay_thread_tenant_id.clone(),
                trigger_id: replay_thread_trigger_id,
                fire_slot,
                now: claim_now,
            })
            .await
            .expect("claim replay-thread fire");
        assert!(matches!(
            replay_thread_claim,
            ClaimDueFireOutcome::Claimed(_)
        ));

        let replay_thread_run_id =
            TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f83").expect("valid run");
        let replay_canonical_thread_id =
            ThreadId::new("01890f0f-c000-7000-8000-000000000003").expect("valid thread id");
        repo.mark_fire_replayed(FireReplayedRequest {
            tenant_id: replay_thread_tenant_id.clone(),
            trigger_id: replay_thread_trigger_id,
            fire_slot,
            original_run_id: replay_thread_run_id,
            thread_id: Some(replay_canonical_thread_id.clone()),
            replayed_at: submitted_at,
            next_run_at: replay_next_run_at,
        })
        .await
        .expect("mark replayed with canonical thread id")
        .expect("replayed fire should persist");

        let runs = repo
            .list_trigger_run_history(
                replay_thread_tenant_id.clone(),
                replay_thread_trigger_id,
                10,
            )
            .await
            .expect("list replayed run history");
        assert_eq!(runs.len(), 1);
        assert_eq!(
            runs[0].thread_id,
            Some(replay_canonical_thread_id.clone()),
            "replay must persist the canonical thread id when the submit outcome carries it"
        );

        repo.mark_fire_replayed(FireReplayedRequest {
            tenant_id: replay_thread_tenant_id.clone(),
            trigger_id: replay_thread_trigger_id,
            fire_slot,
            original_run_id: replay_thread_run_id,
            thread_id: None,
            replayed_at: ts(1_704_067_209),
            next_run_at: replay_next_run_at,
        })
        .await
        .expect("idempotent replay without resolved scope")
        .expect("replayed result returns existing record");

        let runs = repo
            .list_trigger_run_history(replay_thread_tenant_id, replay_thread_trigger_id, 10)
            .await
            .expect("list run history after scopeless replay");
        assert_eq!(runs.len(), 1);
        assert_eq!(
            runs[0].thread_id,
            Some(replay_canonical_thread_id),
            "a replay without a resolved scope must not clobber the stored canonical thread id"
        );
    }

    async fn assert_run_history_retention_contract(repo: &impl TriggerRepository) {
        let trigger_id = TriggerId::parse("01J00000000000000000000033").expect("ulid");
        let tenant_id = tenant("tenant-run-history-retention");
        let base_fire_slot = ts(1_704_067_200);
        repo.upsert_trigger(sample_record(trigger_id, tenant_id.clone(), base_fire_slot))
            .await
            .expect("insert retention record");

        for offset in 0..=500 {
            let fire_slot = base_fire_slot + chrono::Duration::minutes(offset);
            let run_id = TurnRunId::new();
            let mut active_record = sample_record(trigger_id, tenant_id.clone(), fire_slot);
            active_record.active_fire_slot = Some(fire_slot);
            active_record.active_run_ref = Some(run_id);
            repo.upsert_trigger(active_record)
                .await
                .expect("upsert active retention record");
            repo.clear_active_fire(ClearActiveFireRequest {
                tenant_id: tenant_id.clone(),
                trigger_id,
                fire_slot,
                run_id,
                status: TriggerRunHistoryStatus::Ok,
            })
            .await
            .expect("clear retention fire")
            .expect("active fire should clear");
        }

        let retained = repo
            .list_trigger_run_history(tenant_id, trigger_id, 501)
            .await
            .expect("list retained run history");
        assert_eq!(retained.len(), 500);
        assert_eq!(
            retained.first().expect("newest retained").fire_slot,
            base_fire_slot + chrono::Duration::minutes(500)
        );
        assert_eq!(
            retained.last().expect("oldest retained").fire_slot,
            base_fire_slot + chrono::Duration::minutes(1)
        );
    }

    async fn seed_persisted_run_history(repo: &impl TriggerRepository) -> (TenantId, TriggerId) {
        let trigger_id = TriggerId::parse("01J00000000000000000000040").expect("ulid");
        let tenant_id = tenant("tenant-malformed-run-history");
        let fire_slot = ts(1_704_067_200);
        let record = sample_record(trigger_id, tenant_id.clone(), fire_slot);
        let expected_next_run_at = record
            .schedule
            .next_slot_after(fire_slot)
            .expect("next slot calculation")
            .expect("future slot");
        repo.upsert_trigger(record).await.expect("insert record");

        let claim_now = fire_slot + chrono::Duration::seconds(3);
        let claimed = repo
            .claim_due_fire(ClaimDueFireRequest {
                tenant_id: tenant_id.clone(),
                trigger_id,
                fire_slot,
                now: claim_now,
            })
            .await
            .expect("claim fire");
        assert!(matches!(claimed, ClaimDueFireOutcome::Claimed(_)));

        let run_id = TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f90").expect("valid run");
        repo.mark_fire_accepted(FireAcceptedRequest {
            tenant_id: tenant_id.clone(),
            trigger_id,
            fire_slot,
            run_id,
            thread_id: ThreadId::new("01890f0f-ee01-7000-8000-000000000001")
                .expect("valid thread id"),
            submitted_at: fire_slot + chrono::Duration::seconds(5),
            next_run_at: expected_next_run_at,
        })
        .await
        .expect("mark fire accepted")
        .expect("accepted fire should persist");

        (tenant_id, trigger_id)
    }

    fn malformed_run_history_cases() -> Vec<(&'static str, &'static str, &'static str)> {
        vec![
            ("fire_slot", "not-a-timestamp", "fire_slot"),
            ("run_id", "not-a-uuid", "run_id"),
            ("thread_id", "not/a/valid/thread-id", "thread"),
            ("status", "timed_out", "status"),
            ("submitted_at", "not-a-timestamp", "submitted_at"),
            ("completed_at", "not-a-timestamp", "completed_at"),
        ]
    }

    async fn assert_malformed_run_history_hydration_errors(
        repo: &impl TriggerRepository,
        tenant_id: TenantId,
        trigger_id: TriggerId,
        expected: &str,
    ) {
        let error = repo
            .list_trigger_run_history(tenant_id.clone(), trigger_id, 10)
            .await
            .expect_err("malformed run history row must fail single-trigger hydration");
        assert_error_contains(error, expected);

        let other_trigger_id = TriggerId::parse("01J00000000000000000000041").expect("ulid");
        let error = repo
            .list_trigger_run_history_batch(tenant_id, &[trigger_id, other_trigger_id], 10)
            .await
            .expect_err("malformed run history row must fail batched hydration");
        assert_error_contains(error, expected);
    }

    fn assert_error_contains(error: TriggerError, expected: &str) {
        assert!(
            error.to_string().contains(expected),
            "expected error to contain {expected:?}, got {error}"
        );
    }

    #[tokio::test]
    async fn in_memory_repository_fire_claim_contract() {
        let repo = InMemoryTriggerRepository::default();
        assert_fire_claim_and_update_contract(&repo).await;
        assert_fire_claim_exclusions_and_active_gate_contract(&repo).await;
        assert_fire_result_rejects_invalid_next_run_at(&repo).await;
        assert_fire_clear_contract(&repo).await;
        assert_run_history_lifecycle_contract(&repo).await;
        assert_run_history_retention_contract(&repo).await;
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn libsql_repository_fire_claim_contract() {
        let (_dir, repo) = build_libsql_repo().await;
        assert_durable_fire_claim_contract(&repo).await;
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn libsql_repository_rejects_malformed_persisted_run_history_rows() {
        for (column, value, expected) in malformed_run_history_cases() {
            let (_dir, db, repo) = build_libsql_repo_with_db().await;
            let (tenant_id, trigger_id) = seed_persisted_run_history(&repo).await;
            let conn = db.connect().expect("connect raw libsql");
            conn.execute(
                &format!(
                    "UPDATE trigger_run_history SET {column} = ?1 WHERE tenant_id = ?2 AND trigger_id = ?3"
                ),
                libsql::params![value, tenant_id.as_str(), trigger_id.to_string()],
            )
            .await
            .expect("corrupt persisted run history row");

            assert_malformed_run_history_hydration_errors(&repo, tenant_id, trigger_id, expected)
                .await;
        }
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn libsql_repository_fire_claim_is_atomic() {
        let (_dir, repo) = build_libsql_repo().await;
        assert_durable_claim_is_atomic(std::sync::Arc::new(repo)).await;
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn libsql_repository_mark_fire_accepted_is_idempotent_under_concurrency() {
        let (_dir, repo) = build_libsql_repo().await;
        assert_mark_fire_accepted_is_idempotent_under_concurrency(
            std::sync::Arc::new(repo),
            TriggerId::parse("01J00000000000000000000014").expect("ulid"),
            tenant("tenant-accepted-concurrent"),
        )
        .await;
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn libsql_repository_mark_fire_replayed_is_idempotent_under_concurrency() {
        let (_dir, repo) = build_libsql_repo().await;
        assert_mark_fire_replayed_is_idempotent_under_concurrency(
            std::sync::Arc::new(repo),
            TriggerId::parse("01J00000000000000000000015").expect("ulid"),
            tenant("tenant-replayed-concurrent"),
        )
        .await;
    }

    #[cfg(feature = "postgres")]
    #[tokio::test]
    async fn postgres_repository_fire_claim_contract() {
        let Some((_container, pool)) = postgres_pool_or_skip().await else {
            return;
        };
        let repo = PostgresTriggerRepository::new(pool.clone());
        repo.run_migrations().await.expect("run migrations");
        assert_durable_fire_claim_contract(&repo).await;
        clear_postgres_triggers(&pool).await;
    }

    #[cfg(feature = "postgres")]
    #[tokio::test]
    async fn postgres_repository_rejects_malformed_persisted_run_history_rows() {
        let Some((_container, pool)) = postgres_pool_or_skip().await else {
            return;
        };
        let repo = PostgresTriggerRepository::new(pool.clone());
        repo.run_migrations().await.expect("run migrations");
        let client = pool.get().await.expect("postgres connection");

        for (column, value, expected) in malformed_run_history_cases() {
            client
                .execute("DELETE FROM trigger_run_history", &[])
                .await
                .expect("clear trigger run history");
            client
                .execute("DELETE FROM trigger_records", &[])
                .await
                .expect("clear trigger records");

            let (tenant_id, trigger_id) = seed_persisted_run_history(&repo).await;
            client
                .execute(
                    &format!(
                        "UPDATE trigger_run_history SET {column} = $1 WHERE tenant_id = $2 AND trigger_id = $3"
                    ),
                    &[&value, &tenant_id.as_str(), &trigger_id.to_string()],
                )
                .await
                .expect("corrupt persisted run history row");

            assert_malformed_run_history_hydration_errors(&repo, tenant_id, trigger_id, expected)
                .await;
        }

        client
            .execute("DELETE FROM trigger_run_history", &[])
            .await
            .expect("clear trigger run history");
        clear_postgres_triggers(&pool).await;
    }

    #[cfg(feature = "postgres")]
    #[tokio::test]
    async fn postgres_repository_fire_claim_is_atomic() {
        let Some((_container, pool)) = postgres_pool_or_skip().await else {
            return;
        };
        let repo = PostgresTriggerRepository::new(pool.clone());
        repo.run_migrations().await.expect("run migrations");
        assert_durable_claim_is_atomic(std::sync::Arc::new(repo)).await;
        clear_postgres_triggers(&pool).await;
    }

    #[cfg(feature = "postgres")]
    #[tokio::test]
    async fn postgres_repository_mark_fire_accepted_is_idempotent_under_concurrency() {
        let Some((_container, pool)) = postgres_pool_or_skip().await else {
            return;
        };
        let repo = PostgresTriggerRepository::new(pool.clone());
        repo.run_migrations().await.expect("run migrations");
        assert_mark_fire_accepted_is_idempotent_under_concurrency(
            std::sync::Arc::new(repo),
            TriggerId::parse("01J00000000000000000000016").expect("ulid"),
            tenant("tenant-postgres-accepted-concurrent"),
        )
        .await;
        clear_postgres_triggers(&pool).await;
    }

    #[cfg(feature = "postgres")]
    #[tokio::test]
    async fn postgres_repository_mark_fire_replayed_is_idempotent_under_concurrency() {
        let Some((_container, pool)) = postgres_pool_or_skip().await else {
            return;
        };
        let repo = PostgresTriggerRepository::new(pool.clone());
        repo.run_migrations().await.expect("run migrations");
        assert_mark_fire_replayed_is_idempotent_under_concurrency(
            std::sync::Arc::new(repo),
            TriggerId::parse("01J00000000000000000000017").expect("ulid"),
            tenant("tenant-postgres-replayed-concurrent"),
        )
        .await;
        clear_postgres_triggers(&pool).await;
    }
}

// ---------------------------------------------------------------------------
// find_trigger_run_by_thread_id contract
// ---------------------------------------------------------------------------

mod find_trigger_run_by_thread_id_contract {
    use super::*;

    use ironclaw_triggers::{ClaimDueFireOutcome, ClaimDueFireRequest, FireAcceptedRequest};

    fn thread_id(value: &str) -> ThreadId {
        ThreadId::new(value).expect("valid thread id")
    }

    /// Seeds a trigger record and marks one fire as accepted with the given
    /// `thread_id`, returning `(trigger_id, fire_slot)`.
    async fn seed_accepted_run(
        repo: &impl TriggerRepository,
        trigger_id: TriggerId,
        tenant_id: TenantId,
        run_thread_id: ThreadId,
    ) -> Timestamp {
        let fire_slot = ts(1_704_067_200);
        let record = sample_record(trigger_id, tenant_id.clone(), fire_slot);
        repo.upsert_trigger(record).await.expect("upsert");
        let claimed = repo
            .claim_due_fire(ClaimDueFireRequest {
                tenant_id: tenant_id.clone(),
                trigger_id,
                fire_slot,
                now: fire_slot,
            })
            .await
            .expect("claim due fire");
        assert!(
            matches!(claimed, ClaimDueFireOutcome::Claimed(_)),
            "seed_accepted_run: claim must succeed"
        );
        let next_run_at = ts(fire_slot.timestamp() + 3600);
        repo.mark_fire_accepted(FireAcceptedRequest {
            tenant_id,
            trigger_id,
            fire_slot,
            run_id: TurnRunId::new(),
            thread_id: run_thread_id,
            submitted_at: fire_slot,
            next_run_at,
        })
        .await
        .expect("mark fire accepted");
        fire_slot
    }

    async fn assert_find_trigger_run_by_thread_id_contract(repo: &impl TriggerRepository) {
        let tenant_a = tenant("tenant-a");
        let tenant_b = tenant("tenant-b");
        let trigger_id_a = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
        let trigger_id_b = TriggerId::parse("01J00000000000000000000000").expect("ulid");
        let t1 = thread_id("01890f0f-test-7000-8000-000000000001");
        let t2 = thread_id("01890f0f-test-7000-8000-000000000002");
        let unknown = thread_id("01890f0f-test-7000-8000-999999999999");

        // Seed trigger-a (tenant-a) with thread t1.
        let _ = seed_accepted_run(repo, trigger_id_a, tenant_a.clone(), t1.clone()).await;
        // Seed trigger-b (tenant-b) with thread t2.
        let _ = seed_accepted_run(repo, trigger_id_b, tenant_b.clone(), t2.clone()).await;

        // Found: correct tenant + thread_id.
        let result = repo
            .find_trigger_run_by_thread_id(tenant_a.clone(), &t1)
            .await
            .expect("find by known thread_id")
            .expect("run record must be present");
        assert_eq!(result.0.trigger_id, trigger_id_a);
        assert_eq!(
            result.1.thread_id.as_ref().map(|t| t.as_str()),
            Some(t1.as_str())
        );

        // Not found: correct tenant, wrong thread_id (unknown).
        let not_found = repo
            .find_trigger_run_by_thread_id(tenant_a.clone(), &unknown)
            .await
            .expect("find by unknown thread_id must not error");
        assert!(not_found.is_none(), "unknown thread_id must return None");

        // Tenant isolation: searching tenant_b for thread t1 (which lives in tenant_a)
        // must return None.
        let cross_tenant = repo
            .find_trigger_run_by_thread_id(tenant_b.clone(), &t1)
            .await
            .expect("cross-tenant find must not error");
        assert!(
            cross_tenant.is_none(),
            "thread_id from another tenant must not be visible"
        );

        // Run rows without a thread_id (pre-acceptance rows) must not be
        // findable.  We test this via a fresh trigger that has been claimed but
        // NOT accepted (no thread_id row yet).
        let trigger_id_c = TriggerId::parse("01J00000000000000000000003").expect("ulid");
        let fire_slot_c = ts(1_704_067_300);
        let record_c = sample_record(trigger_id_c, tenant_a.clone(), fire_slot_c);
        repo.upsert_trigger(record_c)
            .await
            .expect("upsert trigger-c");
        // Do not call mark_fire_accepted — no thread_id row exists.
        let t_none = thread_id("01890f0f-test-7000-8000-000000000003");
        let pre_accept = repo
            .find_trigger_run_by_thread_id(tenant_a.clone(), &t_none)
            .await
            .expect("pre-acceptance find must not error");
        assert!(
            pre_accept.is_none(),
            "pre-acceptance trigger (no thread_id row) must return None"
        );
    }

    // In-memory backend.
    #[tokio::test]
    async fn in_memory_find_trigger_run_by_thread_id() {
        let repo = InMemoryTriggerRepository::default();
        assert_find_trigger_run_by_thread_id_contract(&repo).await;
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn libsql_find_trigger_run_by_thread_id() {
        let (_dir, repo) = super::build_libsql_repo().await;
        assert_find_trigger_run_by_thread_id_contract(&repo).await;
    }

    #[cfg(feature = "postgres")]
    #[tokio::test]
    async fn postgres_find_trigger_run_by_thread_id() {
        let Some((_container, pool)) = super::postgres_pool_or_skip().await else {
            return;
        };
        let repo = PostgresTriggerRepository::new(pool.clone());
        repo.run_migrations().await.expect("run migrations");
        assert_find_trigger_run_by_thread_id_contract(&repo).await;
        super::clear_postgres_triggers(&pool).await;
    }
}
