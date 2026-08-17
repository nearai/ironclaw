use std::sync::Arc;

use crate::{AutomationProductService, RebornAutomationState};
use ironclaw_host_api::{
    Timestamp,
    ids::{TenantId, UserId},
};
use ironclaw_triggers::AutomationName;
use ironclaw_triggers::{
    InMemoryTriggerRepository, TriggerId, TriggerManualFireOutcome, TriggerManualFireRunner,
    TriggerRepository, TriggerState,
};

use super::{caller, make_record, now, service_over};

struct RecordingManualFireRunner {
    outcome: TriggerManualFireOutcome,
    calls: std::sync::Mutex<Vec<(TenantId, TriggerId)>>,
}

impl RecordingManualFireRunner {
    fn new(outcome: TriggerManualFireOutcome) -> Self {
        Self {
            outcome,
            calls: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn call_count(&self) -> usize {
        self.calls.lock().expect("manual fire calls lock").len()
    }
}

#[async_trait::async_trait]
impl TriggerManualFireRunner for RecordingManualFireRunner {
    async fn run_manual_fire(
        &self,
        tenant_id: TenantId,
        trigger_id: TriggerId,
        _now: Timestamp,
    ) -> Result<TriggerManualFireOutcome, ironclaw_triggers::TriggerError> {
        self.calls
            .lock()
            .expect("manual fire calls lock")
            .push((tenant_id, trigger_id));
        Ok(self.outcome.clone())
    }
}

fn automation_name(value: &str) -> AutomationName {
    AutomationName::new(value).expect("valid automation name")
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

    let service = service_over(repo.clone());

    let paused = service
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

    let resume_started_at = now();
    let resumed = service
        .resume_automation(c, trigger_id.to_string())
        .await
        .expect("resume automation");
    assert!(resumed.updated);
    let resumed = resumed.automation.expect("resumed automation");
    assert_eq!(resumed.state, RebornAutomationState::Scheduled);
    assert!(
        resumed.next_run_at.expect("next run after resume") > resume_started_at,
        "resume must skip recurring slots missed while paused"
    );
    assert_eq!(
        repo.list_due_triggers(now(), 10)
            .await
            .expect("list due after resume")
            .len(),
        0,
        "resumed automation must not replay a slot missed while paused"
    );
}

#[tokio::test]
async fn run_automation_is_caller_scoped_before_manual_fire_dispatch() {
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
    let runner = Arc::new(RecordingManualFireRunner::new(
        TriggerManualFireOutcome::Submitted {
            run_id: ironclaw_turns::TurnRunId::new(),
        },
    ));
    let service = service_over(repo).with_manual_fire_runner(runner.clone());

    let response = service
        .run_automation(c, trigger_id.to_string())
        .await
        .expect("wrong-scope run is hidden");

    assert!(!response.updated);
    assert!(response.automation.is_none());
    assert_eq!(
        runner.call_count(),
        0,
        "foreign trigger must never be fired"
    );
}

#[tokio::test]
async fn run_automation_maps_active_and_paused_to_conflict() {
    for outcome in [
        TriggerManualFireOutcome::AlreadyActive {
            active_fire_slot: Some(now()),
            active_run_ref: None,
        },
        TriggerManualFireOutcome::Paused,
    ] {
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
        let service = service_over(repo)
            .with_manual_fire_runner(Arc::new(RecordingManualFireRunner::new(outcome)));

        let error = service
            .run_automation(c, trigger_id.to_string())
            .await
            .expect_err("manual fire conflict");
        assert_eq!(error.status_code, 409);
        assert_eq!(
            error.code,
            ironclaw_product_contracts::surface::ProductSurfaceErrorCode::Conflict
        );
    }
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

    let service = service_over(repo);
    let response = service
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

    let service = service_over(repo.clone());
    let response = service
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

    let service = service_over(repo.clone());
    let response = service
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

    let service = service_over(repo.clone());
    let response = service
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

    let service = service_over(repo.clone());
    let response = service
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

    let service = service_over(repo);
    let response = service
        .resume_automation(c, trigger_id.to_string())
        .await
        .expect("resume completed automation");

    assert!(!response.updated);
    assert!(response.automation.is_none());
}

#[tokio::test]
async fn pause_automation_rejects_invalid_automation_id_as_bad_request() {
    let service = service_over(Arc::new(InMemoryTriggerRepository::default()));

    let error = service
        .pause_automation(caller(), "not a trigger id".to_string())
        .await
        .expect_err("invalid automation id should be rejected");

    assert_eq!(error.status_code, 400);
}

#[tokio::test]
async fn rename_automation_rejects_invalid_automation_id_as_bad_request() {
    let service = service_over(Arc::new(InMemoryTriggerRepository::default()));

    let error = service
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
    let service = service_over(Arc::new(InMemoryTriggerRepository::default()));

    let error = service
        .delete_automation(caller(), "not a trigger id".to_string())
        .await
        .expect_err("invalid automation id should be rejected");

    assert_eq!(error.status_code, 400);
}
