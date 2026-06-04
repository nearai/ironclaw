//! Full-path integration test for the composition-owned trigger poller.
//!
//! Drives a real `RebornRuntime` with the trigger poller enabled, seeds a
//! due `TriggerRecord` via the in-memory repository, and asserts that the
//! spawned background task (a) mutates the record and (b) causes the LLM
//! gateway to receive a request whose content includes the trigger prompt.

#![cfg(feature = "test-support")]

use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_conversations::{AdapterInstallationId, AdapterKind, ExternalActorRef};
use ironclaw_host_api::runtime_policy::{
    ApprovalPolicy, AuditMode, DeploymentMode, EffectiveRuntimePolicy, FilesystemBackendKind,
    NetworkMode, ProcessBackendKind, RuntimeProfile, SecretMode,
};
use ironclaw_host_api::{AgentId, TenantId, UserId};
use ironclaw_loop_support::{
    HostManagedModelError, HostManagedModelGateway, HostManagedModelRequest,
    HostManagedModelResponse,
};
use ironclaw_reborn_composition::{
    RebornBuildInput, RebornRuntime, RebornRuntimeIdentity, RebornRuntimeInput,
    TriggerPollerSettings, build_reborn_runtime,
};
use ironclaw_triggers::{
    TriggerCompletionPolicy, TriggerId, TriggerPollerWorkerConfig, TriggerRecord,
    TriggerRepository, TriggerRunStatus, TriggerSchedule, TriggerSourceKind, TriggerState,
};
use tokio::sync::Mutex as TokioMutex;

const TENANT: &str = "trigger-e2e-tenant";
const USER: &str = "trigger-e2e-owner";
const AGENT: &str = "trigger-e2e-agent";
const TRIGGER_PROMPT: &str = "trigger-e2e-prompt-marker-do-not-rephrase";

fn local_dev_runtime_policy() -> EffectiveRuntimePolicy {
    EffectiveRuntimePolicy {
        deployment: DeploymentMode::LocalSingleUser,
        requested_profile: RuntimeProfile::LocalDev,
        resolved_profile: RuntimeProfile::LocalDev,
        filesystem_backend: FilesystemBackendKind::HostWorkspace,
        process_backend: ProcessBackendKind::LocalHost,
        network_mode: NetworkMode::DirectLogged,
        secret_mode: SecretMode::ScrubbedEnv,
        approval_policy: ApprovalPolicy::AskDestructive,
        audit_mode: AuditMode::LocalMinimal,
    }
}

#[derive(Debug, Default)]
struct RecordingGateway {
    requests: Arc<TokioMutex<Vec<HostManagedModelRequest>>>,
}

impl RecordingGateway {
    async fn captured_message_contents(&self) -> Vec<String> {
        let snapshot = self.requests.lock().await.clone();
        snapshot
            .iter()
            .flat_map(|req| req.messages.iter().map(|m| m.content.clone()))
            .collect()
    }
}

#[async_trait]
impl HostManagedModelGateway for RecordingGateway {
    async fn stream_model(
        &self,
        request: HostManagedModelRequest,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        self.requests.lock().await.push(request);
        Ok(HostManagedModelResponse::assistant_reply(
            "trigger e2e ok".to_string(),
        ))
    }
}

/// Poll `repo` until `predicate` returns `true` or `deadline` elapses.
///
/// Returns the last record seen. If the predicate is satisfied before the
/// deadline, the returned record satisfies the predicate. If the deadline
/// elapses, the returned record is the last one read (which may not satisfy
/// the predicate — callers should then let the existing assertions fail with
/// the diagnostic they already carry).
///
/// Used by the happy-path and Recurring tests to wait for the settle writes
/// (step 3: `mark_fire_accepted` / `mark_fire_replayed`) to become visible
/// after the first-pass loop breaks on `record_was_mutated && prompt_seen`.
async fn wait_for_settled<F>(
    repo: &Arc<dyn TriggerRepository>,
    tenant_id: &TenantId,
    trigger_id: TriggerId,
    deadline: Duration,
    mut predicate: F,
) -> TriggerRecord
where
    F: FnMut(&TriggerRecord) -> bool,
{
    let stop = Instant::now() + deadline;
    let mut last: Option<TriggerRecord> = None;
    while Instant::now() < stop {
        let current = repo
            .get_trigger(tenant_id.clone(), trigger_id)
            .await
            .expect("get_trigger")
            .expect("record present");
        if predicate(&current) {
            return current;
        }
        last = Some(current);
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    last.expect("at least one read should have succeeded in wait_for_settled")
}

/// Shared runtime builder. Every test passes the `TriggerPollerSettings` it
/// wants; identity, runtime policy, and model-gateway override are shared.
async fn build_runtime_with(
    root: &tempfile::TempDir,
    recording_gateway: Arc<RecordingGateway>,
    trigger_poller: TriggerPollerSettings,
) -> RebornRuntime {
    let input =
        RebornRuntimeInput::from_services(
            RebornBuildInput::local_dev(USER, root.path().join("local-dev"))
                .with_runtime_policy(local_dev_runtime_policy()),
        )
        .with_identity(RebornRuntimeIdentity {
            tenant_id: TENANT.to_string(),
            agent_id: AGENT.to_string(),
            source_binding_id: "trigger-e2e-source".to_string(),
            reply_target_binding_id: "trigger-e2e-reply".to_string(),
        })
        .with_trigger_poller_settings(trigger_poller)
        .with_model_gateway_override(
            Arc::clone(&recording_gateway) as Arc<dyn HostManagedModelGateway>
        );

    build_reborn_runtime(input).await.expect("runtime builds")
}

#[tokio::test]
async fn trigger_poller_drives_trusted_ingress_for_due_scheduled_trigger() {
    let root = tempfile::tempdir().expect("tempdir");
    let recording_gateway = Arc::new(RecordingGateway {
        requests: Arc::new(TokioMutex::new(Vec::new())),
    });

    let runtime = build_runtime_with(
        &root,
        Arc::clone(&recording_gateway),
        TriggerPollerSettings::enabled_with_tenant_scoped_authorizer_for_test().with_worker_config(
            TriggerPollerWorkerConfig {
                poll_interval: Duration::from_millis(20),
                ..Default::default()
            },
        ),
    )
    .await;

    let repo = runtime
        .trigger_repository()
        .expect("local-dev runtime exposes trigger repository");
    let pairing = runtime
        .trigger_conversation_pairing()
        .expect("trigger poller runtime exposes conversation pairing service");

    let tenant_id = TenantId::new(TENANT).expect("tenant id");
    let user_id = UserId::new(USER).expect("user id");
    let agent_id = AgentId::new(AGENT).expect("agent id");
    let trigger_id = TriggerId::new();

    // Seed the trigger creator's actor pairing through the production
    // `ConversationActorPairingService` API. The trusted trigger
    // submission path fails closed for unpaired actors by design; in
    // production, onboarding establishes this pairing before any trigger
    // can be created. The adapter kind / installation id / external actor
    // ref must match the values
    // `crates/ironclaw_reborn_composition/src/trigger_poller_trusted_submit.rs`
    // hardcodes for trigger fires.
    pairing
        .pair_external_actor(
            tenant_id.clone(),
            AdapterKind::new("trigger").expect("adapter kind"),
            AdapterInstallationId::new("reborn-trigger-poller").expect("installation id"),
            ExternalActorRef::new("user", user_id.as_str()).expect("actor ref"),
            user_id.clone(),
        )
        .await
        .expect("pair external actor for trigger creator");

    let record = TriggerRecord {
        trigger_id,
        tenant_id: tenant_id.clone(),
        creator_user_id: user_id,
        agent_id: Some(agent_id),
        project_id: None,
        name: "trigger-e2e-test".to_string(),
        source: TriggerSourceKind::Schedule,
        schedule: TriggerSchedule::cron("* * * * *").expect("valid cron expression"),
        completion_policy: TriggerCompletionPolicy::CompleteAfterFirstFire,
        prompt: TRIGGER_PROMPT.to_string(),
        state: TriggerState::Scheduled,
        next_run_at: Utc::now() - chrono::Duration::seconds(120),
        last_run_at: None,
        last_fired_slot: None,
        last_status: None,
        active_fire_slot: None,
        active_run_ref: None,
        created_at: Utc::now(),
    };

    repo.upsert_trigger(record.clone())
        .await
        .expect("upsert trigger record");

    let deadline = Instant::now() + Duration::from_secs(15);
    let mut record_was_mutated = false;
    let mut prompt_seen = false;

    while Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(100)).await;

        let current = repo
            .get_trigger(tenant_id.clone(), record.trigger_id)
            .await
            .expect("get_trigger")
            .expect("record present");

        let mutated = current.last_fired_slot.is_some()
            || current.last_run_at.is_some()
            || current.last_status.is_some()
            || current.active_fire_slot.is_some()
            || current.state == TriggerState::Completed;

        if mutated {
            record_was_mutated = true;
            // Only poll the gateway after the record was touched.
            let contents = recording_gateway.captured_message_contents().await;
            if contents
                .iter()
                .any(|content| content.contains(TRIGGER_PROMPT))
            {
                prompt_seen = true;
            }
        }

        if record_was_mutated && prompt_seen {
            break;
        }
    }

    // Wait for the settle writes (mark_fire_accepted sets last_fired_slot, last_run_at)
    // to become visible. The first-pass loop breaks as soon as the claim+prompt are seen;
    // the settle may still be in flight.
    let final_record = wait_for_settled(
        &repo,
        &tenant_id,
        record.trigger_id,
        Duration::from_secs(5),
        |r| r.last_fired_slot.is_some() && r.last_run_at.is_some(),
    )
    .await;

    runtime.shutdown().await.expect("runtime shutdown");

    // Final snapshot for diagnostics, once. Taken after shutdown so a request
    // submitted between snapshot and shutdown completion cannot be invisible.
    let captured_contents = recording_gateway.captured_message_contents().await;

    assert!(
        record_was_mutated,
        "poller did not mutate trigger record within 15s — record: {final_record:?}"
    );
    assert!(
        prompt_seen,
        "LLM gateway never received a request containing the trigger prompt within 15s \
         — captured_messages: {captured_contents:?}"
    );
    // CompleteAfterFirstFire: the settle write (mark_fire_accepted) records last_fired_slot
    // and last_run_at. `state` transitions to `Completed` only when the terminal-failure
    // path runs (`mark_fire_terminally_failed`); the policy field is currently stored but
    // never consulted by `mark_fire_accepted` — see issue #4420 for the production gap.
    // Once that is fixed, tighten this to `assert_eq!(state, TriggerState::Completed, ...)`.
    assert!(
        final_record.last_fired_slot.is_some(),
        "CompleteAfterFirstFire policy: last_fired_slot should be set after fire — record: {final_record:?}",
    );
    assert!(
        final_record.last_run_at.is_some(),
        "CompleteAfterFirstFire policy: last_run_at should be set after fire — record: {final_record:?}",
    );
    assert_eq!(
        final_record.last_status,
        Some(TriggerRunStatus::Ok),
        "CompleteAfterFirstFire policy: last_status should be Ok after fire — record: {final_record:?}",
    );
}

#[tokio::test]
async fn trigger_conversation_pairing_returns_none_when_poller_disabled() {
    let root = tempfile::tempdir().expect("tempdir");
    let recording_gateway = Arc::new(RecordingGateway {
        requests: Arc::new(TokioMutex::new(Vec::new())),
    });

    // Use the default settings (enabled: false) — do NOT call
    // with_trigger_poller_settings with enabled: true.
    let runtime = build_runtime_with(
        &root,
        Arc::clone(&recording_gateway),
        TriggerPollerSettings::default(),
    )
    .await;

    // The trigger repository is built regardless of poller state.
    assert!(
        runtime.trigger_repository().is_some(),
        "trigger repository should be present even when poller is disabled"
    );

    // When the poller is disabled, no conversation pairing service is wired.
    assert!(
        runtime.trigger_conversation_pairing().is_none(),
        "trigger_conversation_pairing should be None when poller is disabled"
    );

    runtime.shutdown().await.expect("runtime shutdown");
}

#[tokio::test]
async fn trigger_poller_does_not_fire_trigger_with_future_next_run_at() {
    let root = tempfile::tempdir().expect("tempdir");
    let recording_gateway = Arc::new(RecordingGateway {
        requests: Arc::new(TokioMutex::new(Vec::new())),
    });

    let runtime = build_runtime_with(
        &root,
        Arc::clone(&recording_gateway),
        TriggerPollerSettings::enabled_with_tenant_scoped_authorizer_for_test().with_worker_config(
            TriggerPollerWorkerConfig {
                poll_interval: Duration::from_millis(20),
                ..Default::default()
            },
        ),
    )
    .await;

    let repo = runtime
        .trigger_repository()
        .expect("local-dev runtime exposes trigger repository");
    let pairing = runtime
        .trigger_conversation_pairing()
        .expect("trigger poller runtime exposes conversation pairing service");

    let tenant_id = TenantId::new(TENANT).expect("tenant id");
    let user_id = UserId::new(USER).expect("user id");
    let agent_id = AgentId::new(AGENT).expect("agent id");
    let trigger_id = TriggerId::new();

    pairing
        .pair_external_actor(
            tenant_id.clone(),
            AdapterKind::new("trigger").expect("adapter kind"),
            AdapterInstallationId::new("reborn-trigger-poller").expect("installation id"),
            ExternalActorRef::new("user", user_id.as_str()).expect("actor ref"),
            user_id.clone(),
        )
        .await
        .expect("pair external actor for trigger creator");

    // Seed a trigger that is NOT due — next_run_at is one hour in the future.
    let record = TriggerRecord {
        trigger_id,
        tenant_id: tenant_id.clone(),
        creator_user_id: user_id,
        agent_id: Some(agent_id),
        project_id: None,
        name: "trigger-e2e-future".to_string(),
        source: TriggerSourceKind::Schedule,
        schedule: TriggerSchedule::cron("* * * * *").expect("valid cron expression"),
        completion_policy: TriggerCompletionPolicy::CompleteAfterFirstFire,
        prompt: TRIGGER_PROMPT.to_string(),
        state: TriggerState::Scheduled,
        next_run_at: Utc::now() + chrono::Duration::seconds(3600),
        last_run_at: None,
        last_fired_slot: None,
        last_status: None,
        active_fire_slot: None,
        active_run_ref: None,
        created_at: Utc::now(),
    };

    repo.upsert_trigger(record.clone())
        .await
        .expect("upsert trigger record");

    // Sleep for ~500ms — 25 poll cycles at 20ms. Generous margin.
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Clone the repo handle before shutdown (Arc is cheap to clone).
    let repo_after = repo.clone();

    runtime.shutdown().await.expect("runtime shutdown");

    // Snapshot captured_contents AFTER shutdown so a request submitted between
    // snapshot and shutdown completion cannot produce a false-green result.
    // The recording_gateway Arc is independent of the runtime.
    let captured_contents = recording_gateway.captured_message_contents().await;

    let current = repo_after
        .get_trigger(tenant_id.clone(), record.trigger_id)
        .await
        .expect("get_trigger")
        .expect("record present");

    assert!(
        current.last_fired_slot.is_none(),
        "poller should not have fired a future trigger — last_fired_slot: {:?}",
        current.last_fired_slot
    );
    assert!(
        current.last_run_at.is_none(),
        "poller should not have run a future trigger — last_run_at: {:?}",
        current.last_run_at
    );
    assert!(
        current.last_status.is_none(),
        "poller should not have set a status on a future trigger — last_status: {:?}",
        current.last_status
    );
    assert!(
        current.active_fire_slot.is_none(),
        "poller should not have set active_fire_slot on a future trigger — active_fire_slot: {:?}",
        current.active_fire_slot
    );
    assert_eq!(
        current.state,
        TriggerState::Scheduled,
        "future trigger should remain Scheduled — state: {:?}",
        current.state
    );
    assert!(
        captured_contents.is_empty(),
        "LLM gateway should not have received any requests for a future trigger"
    );
}

#[tokio::test]
async fn trigger_poller_does_not_submit_turn_for_unpaired_actor() {
    let root = tempfile::tempdir().expect("tempdir");
    let recording_gateway = Arc::new(RecordingGateway {
        requests: Arc::new(TokioMutex::new(Vec::new())),
    });

    let runtime = build_runtime_with(
        &root,
        Arc::clone(&recording_gateway),
        TriggerPollerSettings::enabled_with_tenant_scoped_authorizer_for_test().with_worker_config(
            TriggerPollerWorkerConfig {
                poll_interval: Duration::from_millis(20),
                ..Default::default()
            },
        ),
    )
    .await;

    let repo = runtime
        .trigger_repository()
        .expect("local-dev runtime exposes trigger repository");

    // Intentionally do NOT call pair_external_actor — the actor is unpaired.

    let tenant_id = TenantId::new(TENANT).expect("tenant id");
    let user_id = UserId::new(USER).expect("user id");
    let agent_id = AgentId::new(AGENT).expect("agent id");
    let trigger_id = TriggerId::new();

    // Seed a past-due trigger.
    let record = TriggerRecord {
        trigger_id,
        tenant_id: tenant_id.clone(),
        creator_user_id: user_id,
        agent_id: Some(agent_id),
        project_id: None,
        name: "trigger-e2e-unpaired".to_string(),
        source: TriggerSourceKind::Schedule,
        schedule: TriggerSchedule::cron("* * * * *").expect("valid cron expression"),
        completion_policy: TriggerCompletionPolicy::CompleteAfterFirstFire,
        prompt: TRIGGER_PROMPT.to_string(),
        state: TriggerState::Scheduled,
        next_run_at: Utc::now() - chrono::Duration::seconds(120),
        last_run_at: None,
        last_fired_slot: None,
        last_status: None,
        active_fire_slot: None,
        active_run_ref: None,
        created_at: Utc::now(),
    };

    repo.upsert_trigger(record.clone())
        .await
        .expect("upsert trigger record");

    // Sleep for ~1s — 50 poll cycles at 20ms — to give the poller multiple chances.
    tokio::time::sleep(Duration::from_secs(1)).await;

    // Clone the repo handle before shutdown (Arc is cheap to clone).
    let repo_after = repo.clone();

    runtime.shutdown().await.expect("runtime shutdown");

    // Snapshot captured_contents AFTER shutdown so a request submitted between
    // snapshot and shutdown completion cannot produce a false-green result.
    // The recording_gateway Arc is independent of the runtime.
    let captured_contents = recording_gateway.captured_message_contents().await;

    let current = repo_after
        .get_trigger(tenant_id.clone(), record.trigger_id)
        .await
        .expect("get_trigger")
        .expect("record present");

    // Safety guarantee: no turn was ever submitted to the LLM gateway.
    assert!(
        captured_contents.is_empty(),
        "LLM gateway should not have received any requests for an unpaired actor — \
         captured: {:?}",
        captured_contents
    );

    // The trigger must not be marked Completed (failure-closed behavior).
    assert_ne!(
        current.state,
        TriggerState::Completed,
        "unpaired trigger must not be marked Completed — state: {:?}, last_status: {:?}",
        current.state,
        current.last_status
    );
}

#[tokio::test]
async fn trigger_poller_fires_recurring_trigger_and_leaves_it_scheduled() {
    let root = tempfile::tempdir().expect("tempdir");
    let recording_gateway = Arc::new(RecordingGateway {
        requests: Arc::new(TokioMutex::new(Vec::new())),
    });

    let runtime = build_runtime_with(
        &root,
        Arc::clone(&recording_gateway),
        TriggerPollerSettings::enabled_with_tenant_scoped_authorizer_for_test().with_worker_config(
            TriggerPollerWorkerConfig {
                poll_interval: Duration::from_millis(20),
                ..Default::default()
            },
        ),
    )
    .await;

    let repo = runtime
        .trigger_repository()
        .expect("local-dev runtime exposes trigger repository");
    let pairing = runtime
        .trigger_conversation_pairing()
        .expect("trigger poller runtime exposes conversation pairing service");

    let tenant_id = TenantId::new(TENANT).expect("tenant id");
    let user_id = UserId::new(USER).expect("user id");
    let agent_id = AgentId::new(AGENT).expect("agent id");
    let trigger_id = TriggerId::new();

    // Pair the external actor — same as the happy-path test.
    pairing
        .pair_external_actor(
            tenant_id.clone(),
            AdapterKind::new("trigger").expect("adapter kind"),
            AdapterInstallationId::new("reborn-trigger-poller").expect("installation id"),
            ExternalActorRef::new("user", user_id.as_str()).expect("actor ref"),
            user_id.clone(),
        )
        .await
        .expect("pair external actor for trigger creator");

    let original_next_run_at = Utc::now() - chrono::Duration::seconds(120);

    let record = TriggerRecord {
        trigger_id,
        tenant_id: tenant_id.clone(),
        creator_user_id: user_id,
        agent_id: Some(agent_id),
        project_id: None,
        name: "trigger-e2e-recurring".to_string(),
        source: TriggerSourceKind::Schedule,
        // Every minute — already at MIN_FIRE_CADENCE.
        schedule: TriggerSchedule::cron("* * * * *").expect("valid cron expression"),
        completion_policy: TriggerCompletionPolicy::Recurring,
        prompt: TRIGGER_PROMPT.to_string(),
        state: TriggerState::Scheduled,
        next_run_at: original_next_run_at,
        last_run_at: None,
        last_fired_slot: None,
        last_status: None,
        active_fire_slot: None,
        active_run_ref: None,
        created_at: Utc::now(),
    };

    repo.upsert_trigger(record.clone())
        .await
        .expect("upsert trigger record");

    let deadline = Instant::now() + Duration::from_secs(15);
    let mut record_was_mutated = false;
    let mut prompt_seen = false;

    while Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(100)).await;

        let current = repo
            .get_trigger(tenant_id.clone(), record.trigger_id)
            .await
            .expect("get_trigger")
            .expect("record present");

        let mutated = current.last_fired_slot.is_some()
            || current.last_run_at.is_some()
            || current.last_status.is_some()
            || current.active_fire_slot.is_some();

        if mutated {
            record_was_mutated = true;
            // Only poll the gateway after the record was touched.
            let contents = recording_gateway.captured_message_contents().await;
            if contents
                .iter()
                .any(|content| content.contains(TRIGGER_PROMPT))
            {
                prompt_seen = true;
            }
        }

        if record_was_mutated && prompt_seen {
            break;
        }
    }

    // Wait for the settle writes (mark_fire_replayed sets last_fired_slot, advances
    // next_run_at) to become visible. The first-pass loop breaks as soon as the
    // claim+prompt are seen; the settle may still be in flight.
    let settled = wait_for_settled(
        &repo,
        &tenant_id,
        record.trigger_id,
        Duration::from_secs(5),
        |r| r.last_fired_slot.is_some() && r.next_run_at > original_next_run_at,
    )
    .await;

    runtime.shutdown().await.expect("runtime shutdown");

    // Final snapshot for diagnostics, once. Taken after shutdown so a request
    // submitted between snapshot and shutdown completion cannot be invisible.
    let captured_contents = recording_gateway.captured_message_contents().await;

    assert!(
        record_was_mutated,
        "poller did not mutate recurring trigger record within 15s — record: {settled:?}"
    );
    assert!(
        prompt_seen,
        "LLM gateway never received a request for recurring trigger within 15s \
         — captured_messages: {captured_contents:?}"
    );

    // Recurring triggers must remain Scheduled (not Completed) after firing.
    assert_eq!(
        settled.state,
        TriggerState::Scheduled,
        "recurring trigger should remain Scheduled after fire — state: {:?}",
        settled.state
    );
    assert!(
        settled.last_fired_slot.is_some(),
        "recurring trigger should have last_fired_slot set after fire"
    );
    assert!(
        settled.last_run_at.is_some(),
        "recurring trigger should have last_run_at set after fire"
    );
    assert!(
        settled.next_run_at > original_next_run_at,
        "recurring trigger next_run_at should have advanced — original: {:?}, current: {:?}",
        original_next_run_at,
        settled.next_run_at
    );
}
