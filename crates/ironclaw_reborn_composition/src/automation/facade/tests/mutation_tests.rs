use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::UserId;
use ironclaw_product_workflow::{
    AutomationCreateRequest, AutomationCreateSchedule, AutomationName, AutomationProductFacade,
    RebornAutomationSource, RebornAutomationState,
};
use ironclaw_triggers::{
    InMemoryTriggerRepository, TriggerCreateLifecycle, TriggerError, TriggerId, TriggerRecord,
    TriggerRepository, TriggerState,
};

use super::{caller, facade_over, make_record, now};

fn automation_name(value: &str) -> AutomationName {
    AutomationName::new(value).expect("valid automation name")
}

fn cron_create_request(expression: &str, timezone: &str) -> AutomationCreateRequest {
    AutomationCreateRequest {
        name: automation_name("Daily status"),
        prompt: "Generate a daily status".to_string(),
        schedule: AutomationCreateSchedule::Cron {
            expression: expression.to_string(),
            timezone: timezone.to_string(),
        },
    }
}

fn once_create_request(at: impl Into<String>, timezone: &str) -> AutomationCreateRequest {
    AutomationCreateRequest {
        name: automation_name("Follow up"),
        prompt: "Check deployment".to_string(),
        schedule: AutomationCreateSchedule::Once {
            at: at.into(),
            timezone: timezone.to_string(),
        },
    }
}

#[tokio::test]
async fn create_automation_persists_exact_caller_scope_and_reads_back() {
    let repo = Arc::new(InMemoryTriggerRepository::default());
    let c = caller();
    let facade = facade_over(repo.clone());

    let created = facade
        .create_automation(c.clone(), cron_create_request("0 9 * * *", "UTC"))
        .await
        .expect("create automation");

    assert_eq!(created.name, "Daily status");
    assert_eq!(created.state, RebornAutomationState::Scheduled);
    assert!(matches!(
        created.source,
        RebornAutomationSource::Schedule { ref cron, ref timezone }
            if cron == "0 9 * * *" && timezone == "UTC"
    ));
    let records = repo
        .list_scoped_triggers(
            c.tenant_id.clone(),
            c.user_id.clone(),
            Some(c.agent_id.clone()),
            c.project_id.clone(),
            10,
            &[],
        )
        .await
        .expect("list created automation");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].trigger_id.to_string(), created.automation_id);
    assert_eq!(records[0].prompt, "Generate a daily status");
}

#[tokio::test]
async fn create_automation_accepts_future_one_time_schedule() {
    let repo = Arc::new(InMemoryTriggerRepository::default());
    let facade = facade_over(repo);
    let future = (chrono::Utc::now() + chrono::Duration::days(2))
        .format("%Y-%m-%dT%H:%M:%S")
        .to_string();

    let created = facade
        .create_automation(caller(), once_create_request(future, "UTC"))
        .await
        .expect("create one-time automation");

    assert!(matches!(
        created.source,
        RebornAutomationSource::Once { .. }
    ));
    assert!(created.next_run_at.is_some());
}

#[tokio::test]
async fn create_automation_normalizes_rfc3339_and_reads_back_caller_scope() {
    let repo = Arc::new(InMemoryTriggerRepository::default());
    let c = caller();
    let facade = facade_over(repo.clone());

    let created = facade
        .create_automation(
            c.clone(),
            once_create_request("2099-06-25T01:00:00+08:00", "Asia/Shanghai"),
        )
        .await
        .expect("create RFC3339 automation");
    assert!(matches!(
        created.source,
        RebornAutomationSource::Once { ref at, ref timezone }
            if at == "2099-06-24T17:00:00+00:00"
                && timezone == "Asia/Shanghai"
    ));

    let records = repo
        .list_scoped_triggers(
            c.tenant_id,
            c.user_id,
            Some(c.agent_id),
            c.project_id,
            10,
            &[],
        )
        .await
        .expect("list RFC3339 automation");
    assert_eq!(records.len(), 1);
}

#[tokio::test]
async fn create_automation_rejects_rfc3339_offset_mismatch_without_write() {
    let repo = Arc::new(InMemoryTriggerRepository::default());
    let facade = facade_over(repo.clone());

    let error = facade
        .create_automation(
            caller(),
            once_create_request("2099-06-25T01:00:00+09:00", "Asia/Shanghai"),
        )
        .await
        .expect_err("mismatched RFC3339 offset rejected");
    assert_eq!(error.status_code, 400);
    assert!(!error.retryable);
    assert!(
        repo.list_triggers(caller().tenant_id)
            .await
            .expect("list triggers")
            .is_empty()
    );
}

#[tokio::test]
async fn create_automation_maps_invalid_schedule_to_bad_request_without_write() {
    let repo = Arc::new(InMemoryTriggerRepository::default());
    let facade = facade_over(repo.clone());

    let error = facade
        .create_automation(caller(), cron_create_request("0 8 * *", "UTC"))
        .await
        .expect_err("invalid cron rejected");

    assert_eq!(error.status_code, 400);
    assert!(!error.retryable);
    assert!(
        repo.list_triggers(caller().tenant_id)
            .await
            .expect("list triggers")
            .is_empty()
    );
}

#[derive(Debug)]
struct FailingCreateLifecycle;

#[async_trait]
impl TriggerCreateLifecycle for FailingCreateLifecycle {
    async fn after_trigger_persisted(&self, _record: &TriggerRecord) -> Result<(), TriggerError> {
        Err(TriggerError::Backend {
            reason: "pairing unavailable".to_string(),
        })
    }
}

#[tokio::test]
async fn create_automation_maps_pairing_failure_to_503_and_rolls_back() {
    let repo = Arc::new(InMemoryTriggerRepository::default());
    let facade =
        facade_over(repo.clone()).with_creation_lifecycle(Arc::new(FailingCreateLifecycle));

    let error = facade
        .create_automation(
            caller(),
            once_create_request("2099-06-25T01:00:00+08:00", "Asia/Shanghai"),
        )
        .await
        .expect_err("pairing failure returned");

    assert_eq!(error.status_code, 503);
    assert!(error.retryable);
    assert!(
        repo.list_triggers(caller().tenant_id)
            .await
            .expect("list triggers")
            .is_empty()
    );
}

#[tokio::test]
async fn pause_and_resume_update_scoped_trigger_state() {
    let repo = Arc::new(InMemoryTriggerRepository::default());
    let c = caller();
    let trigger_id = TriggerId::new();
    repo.upsert_trigger(make_record(
        trigger_id,
        &c,
        TriggerState::Scheduled,
        "Daily task",
        "0 9 * * *",
    ))
    .await
    .expect("upsert trigger");

    let facade = facade_over(repo.clone());

    let paused = facade
        .pause_automation(c.clone(), trigger_id.to_string())
        .await
        .expect("pause automation");
    assert!(paused.updated);
    assert_eq!(
        paused.automation.expect("paused automation").state,
        RebornAutomationState::Paused
    );
    assert!(
        repo.list_due_triggers(now(), 10)
            .await
            .expect("list due while paused")
            .is_empty(),
        "paused automation must not be eligible to fire"
    );

    let resumed = facade
        .resume_automation(c, trigger_id.to_string())
        .await
        .expect("resume automation");
    assert!(resumed.updated);
    assert_eq!(
        resumed.automation.expect("resumed automation").state,
        RebornAutomationState::Scheduled
    );
    assert_eq!(
        repo.list_due_triggers(now(), 10)
            .await
            .expect("list due after resume")
            .len(),
        1,
        "resumed automation should be eligible again when its next slot is due"
    );
}

#[tokio::test]
async fn pause_automation_returns_not_updated_for_wrong_scope() {
    let repo = Arc::new(InMemoryTriggerRepository::default());
    let c = caller();
    let mut other_caller = caller();
    other_caller.user_id = UserId::new("other-user").expect("valid user id");
    let trigger_id = TriggerId::new();
    repo.upsert_trigger(make_record(
        trigger_id,
        &other_caller,
        TriggerState::Scheduled,
        "Other task",
        "0 10 * * *",
    ))
    .await
    .expect("upsert trigger");

    let facade = facade_over(repo);
    let response = facade
        .pause_automation(c, trigger_id.to_string())
        .await
        .expect("pause wrong-scope automation");

    assert!(!response.updated);
    assert!(response.automation.is_none());
}

#[tokio::test]
async fn rename_automation_updates_scoped_trigger_name() {
    let repo = Arc::new(InMemoryTriggerRepository::default());
    let c = caller();
    let trigger_id = TriggerId::new();
    repo.upsert_trigger(make_record(
        trigger_id,
        &c,
        TriggerState::Scheduled,
        "Original task",
        "0 9 * * *",
    ))
    .await
    .expect("upsert trigger");

    let facade = facade_over(repo.clone());
    let response = facade
        .rename_automation(
            c.clone(),
            trigger_id.to_string(),
            automation_name("Inbox sweep"),
        )
        .await
        .expect("rename automation");

    assert!(response.updated);
    assert_eq!(
        response.automation.expect("renamed automation").name,
        "Inbox sweep"
    );
    assert_eq!(
        repo.get_trigger(c.tenant_id, trigger_id)
            .await
            .expect("get renamed trigger")
            .expect("record")
            .name,
        "Inbox sweep"
    );
}

#[tokio::test]
async fn rename_automation_returns_not_updated_for_wrong_scope() {
    let repo = Arc::new(InMemoryTriggerRepository::default());
    let c = caller();
    let mut other_caller = caller();
    other_caller.user_id = UserId::new("other-user").expect("valid user id");
    let trigger_id = TriggerId::new();
    repo.upsert_trigger(make_record(
        trigger_id,
        &other_caller,
        TriggerState::Scheduled,
        "Other task",
        "0 10 * * *",
    ))
    .await
    .expect("upsert trigger");

    let facade = facade_over(repo.clone());
    let response = facade
        .rename_automation(c, trigger_id.to_string(), automation_name("Wrong scope"))
        .await
        .expect("rename wrong-scope automation");

    assert!(!response.updated);
    assert!(response.automation.is_none());
    assert_eq!(
        repo.get_trigger(other_caller.tenant_id, trigger_id)
            .await
            .expect("get original trigger")
            .expect("record")
            .name,
        "Other task"
    );
}

#[tokio::test]
async fn delete_automation_removes_scoped_trigger() {
    let repo = Arc::new(InMemoryTriggerRepository::default());
    let c = caller();
    let trigger_id = TriggerId::new();
    repo.upsert_trigger(make_record(
        trigger_id,
        &c,
        TriggerState::Scheduled,
        "Delete me",
        "0 9 * * *",
    ))
    .await
    .expect("upsert trigger");

    let facade = facade_over(repo.clone());
    let response = facade
        .delete_automation(c.clone(), trigger_id.to_string())
        .await
        .expect("delete automation");

    assert!(response.updated);
    assert!(response.automation.is_none());
    assert!(
        repo.list_scoped_triggers(
            c.tenant_id,
            c.user_id,
            Some(c.agent_id),
            c.project_id,
            10,
            &[]
        )
        .await
        .expect("list scoped triggers")
        .is_empty()
    );
}

#[tokio::test]
async fn delete_automation_returns_not_updated_for_wrong_scope() {
    let repo = Arc::new(InMemoryTriggerRepository::default());
    let c = caller();
    let mut other_caller = caller();
    other_caller.user_id = UserId::new("other-user").expect("valid user id");
    let trigger_id = TriggerId::new();
    repo.upsert_trigger(make_record(
        trigger_id,
        &other_caller,
        TriggerState::Scheduled,
        "Other task",
        "0 10 * * *",
    ))
    .await
    .expect("upsert trigger");

    let facade = facade_over(repo.clone());
    let response = facade
        .delete_automation(c, trigger_id.to_string())
        .await
        .expect("delete wrong-scope automation");

    assert!(!response.updated);
    assert!(response.automation.is_none());
    assert_eq!(
        repo.list_scoped_triggers(
            other_caller.tenant_id,
            other_caller.user_id,
            Some(other_caller.agent_id),
            other_caller.project_id,
            10,
            &[],
        )
        .await
        .expect("list other scoped triggers")
        .len(),
        1
    );
}

#[tokio::test]
async fn resume_automation_does_not_reopen_completed_trigger() {
    let repo = Arc::new(InMemoryTriggerRepository::default());
    let c = caller();
    let trigger_id = TriggerId::new();
    repo.upsert_trigger(make_record(
        trigger_id,
        &c,
        TriggerState::Completed,
        "Finished task",
        "0 11 * * *",
    ))
    .await
    .expect("upsert completed trigger");

    let facade = facade_over(repo);
    let response = facade
        .resume_automation(c, trigger_id.to_string())
        .await
        .expect("resume completed automation");

    assert!(!response.updated);
    assert!(response.automation.is_none());
}

#[tokio::test]
async fn pause_automation_rejects_invalid_automation_id_as_bad_request() {
    let facade = facade_over(Arc::new(InMemoryTriggerRepository::default()));

    let error = facade
        .pause_automation(caller(), "not a trigger id".to_string())
        .await
        .expect_err("invalid automation id should be rejected");

    assert_eq!(error.status_code, 400);
}

#[tokio::test]
async fn rename_automation_rejects_invalid_automation_id_as_bad_request() {
    let facade = facade_over(Arc::new(InMemoryTriggerRepository::default()));

    let error = facade
        .rename_automation(
            caller(),
            "not a trigger id".to_string(),
            automation_name("New name"),
        )
        .await
        .expect_err("invalid automation id should be rejected");

    assert_eq!(error.status_code, 400);
}

#[tokio::test]
async fn delete_automation_rejects_invalid_automation_id_as_bad_request() {
    let facade = facade_over(Arc::new(InMemoryTriggerRepository::default()));

    let error = facade
        .delete_automation(caller(), "not a trigger id".to_string())
        .await
        .expect_err("invalid automation id should be rejected");

    assert_eq!(error.status_code, 400);
}
