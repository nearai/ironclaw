use std::sync::Arc;

use ironclaw_host_api::UserId;
use ironclaw_product_workflow::{AutomationProductFacade, RebornAutomationState};
use ironclaw_triggers::{InMemoryTriggerRepository, TriggerId, TriggerRepository, TriggerState};

use crate::automation::RebornAutomationProductFacade;

use super::{caller, make_record, now};

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

    let facade = RebornAutomationProductFacade::new(repo.clone());

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

    let facade = RebornAutomationProductFacade::new(repo);
    let response = facade
        .pause_automation(c, trigger_id.to_string())
        .await
        .expect("pause wrong-scope automation");

    assert!(!response.updated);
    assert!(response.automation.is_none());
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

    let facade = RebornAutomationProductFacade::new(repo.clone());
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

    let facade = RebornAutomationProductFacade::new(repo.clone());
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

    let facade = RebornAutomationProductFacade::new(repo);
    let response = facade
        .resume_automation(c, trigger_id.to_string())
        .await
        .expect("resume completed automation");

    assert!(!response.updated);
    assert!(response.automation.is_none());
}

#[tokio::test]
async fn pause_automation_rejects_invalid_automation_id_as_bad_request() {
    let facade = RebornAutomationProductFacade::new(Arc::new(InMemoryTriggerRepository::default()));

    let error = facade
        .pause_automation(caller(), "not a trigger id".to_string())
        .await
        .expect_err("invalid automation id should be rejected");

    assert_eq!(error.status_code, 400);
}

#[tokio::test]
async fn delete_automation_rejects_invalid_automation_id_as_bad_request() {
    let facade = RebornAutomationProductFacade::new(Arc::new(InMemoryTriggerRepository::default()));

    let error = facade
        .delete_automation(caller(), "not a trigger id".to_string())
        .await
        .expect_err("invalid automation id should be rejected");

    assert_eq!(error.status_code, 400);
}
