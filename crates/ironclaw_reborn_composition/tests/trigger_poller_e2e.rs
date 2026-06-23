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
use ironclaw_host_api::{
    AgentId, CapabilityGrant, CapabilityGrantId, CapabilityId, CapabilitySet, EffectKind,
    ExecutionContext, ExtensionId, GrantConstraints, MountView, NetworkPolicy, Principal,
    ResourceEstimate, RuntimeKind, TenantId, TrustClass, UserId,
};
use ironclaw_host_runtime::{
    RuntimeCapabilityOutcome, RuntimeCapabilityRequest, TRIGGER_CREATE_CAPABILITY_ID,
};
use ironclaw_loop_support::{
    HostManagedModelError, HostManagedModelGateway, HostManagedModelRequest,
    HostManagedModelResponse,
};
use ironclaw_reborn_composition::{
    RebornCompositionProfile, RebornLocalRuntimeProfileOptions, RebornRuntime,
    RebornRuntimeIdentity, RebornRuntimeInput, TriggerPollerSettings, build_reborn_runtime,
    local_runtime_build_input_with_options,
};
use ironclaw_triggers::{
    TRIGGER_TRUSTED_ADAPTER_INSTALLATION_ID, TRIGGER_TRUSTED_ADAPTER_KIND,
    TRIGGER_TRUSTED_EXTERNAL_ACTOR_NAMESPACE, TriggerId, TriggerPollerWorkerConfig, TriggerRecord,
    TriggerRepository, TriggerRunStatus, TriggerSchedule, TriggerSourceKind, TriggerState,
};
use ironclaw_trust::{AuthorityCeiling, EffectiveTrustClass, TrustDecision, TrustProvenance};
use serde_json::{Value, json};
use tokio::sync::Mutex as TokioMutex;

const TENANT: &str = "trigger-e2e-tenant";
const USER: &str = "trigger-e2e-owner";
const AGENT: &str = "trigger-e2e-agent";
const TRIGGER_PROMPT: &str = "trigger-e2e-prompt-marker-do-not-rephrase";

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
    let host_home_root = root.path().join("host-home");
    std::fs::create_dir_all(&host_home_root).expect("host home root");
    let input = local_runtime_build_input_with_options(
        RebornCompositionProfile::LocalDevYolo,
        USER,
        root.path().join("local-dev"),
        RebornLocalRuntimeProfileOptions {
            confirm_host_access: true,
        },
    )
    .expect("local-yolo runtime input")
    .with_local_dev_confirmed_host_home_root(host_home_root);

    let input = RebornRuntimeInput::from_services(input)
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

async fn invoke_trigger_create(runtime: &RebornRuntime, input: Value) -> Value {
    let host_runtime = runtime
        .services()
        .host_runtime
        .as_deref()
        .expect("runtime exposes host runtime");
    let outcome = host_runtime
        .invoke_capability(RuntimeCapabilityRequest::new(
            trigger_management_execution_context(),
            CapabilityId::new(TRIGGER_CREATE_CAPABILITY_ID).expect("capability id"),
            ResourceEstimate::default(),
            input,
            trigger_management_trust_decision(),
        ))
        .await
        .expect("trigger create invocation completes");
    let RuntimeCapabilityOutcome::Completed(completed) = outcome else {
        panic!("expected trigger create to complete, got {outcome:?}");
    };
    completed.output
}

fn trigger_management_execution_context() -> ExecutionContext {
    let tenant_id = TenantId::new(TENANT).expect("tenant id");
    let user_id = UserId::new(USER).expect("user id");
    let agent_id = AgentId::new(AGENT).expect("agent id");
    let extension_id = ExtensionId::new("trigger-e2e-caller").expect("extension id");
    let mut context = ExecutionContext::local_default(
        user_id.clone(),
        extension_id.clone(),
        RuntimeKind::FirstParty,
        TrustClass::UserTrusted,
        CapabilitySet {
            grants: vec![CapabilityGrant {
                id: CapabilityGrantId::new(),
                capability: CapabilityId::new(TRIGGER_CREATE_CAPABILITY_ID).expect("capability id"),
                grantee: Principal::Extension(extension_id),
                issued_by: Principal::HostRuntime,
                constraints: GrantConstraints {
                    allowed_effects: vec![
                        EffectKind::DispatchCapability,
                        EffectKind::ExternalWrite,
                    ],
                    mounts: MountView::default(),
                    network: NetworkPolicy::default(),
                    secrets: Vec::new(),
                    resource_ceiling: None,
                    expires_at: None,
                    max_invocations: None,
                },
            }],
        },
        MountView::default(),
    )
    .expect("execution context");
    context.tenant_id = tenant_id.clone();
    context.agent_id = Some(agent_id.clone());
    context.project_id = None;
    context.resource_scope.tenant_id = tenant_id;
    context.resource_scope.agent_id = Some(agent_id);
    context.resource_scope.project_id = None;
    context
}

fn trigger_management_trust_decision() -> TrustDecision {
    TrustDecision {
        effective_trust: EffectiveTrustClass::user_trusted(),
        authority_ceiling: AuthorityCeiling {
            allowed_effects: vec![EffectKind::DispatchCapability, EffectKind::ExternalWrite],
            max_resource_ceiling: None,
        },
        provenance: TrustProvenance::AdminConfig,
        evaluated_at: Utc::now(),
    }
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
    // ref must match the trusted trigger constants used for trigger fires.
    pairing
        .pair_external_actor(
            tenant_id.clone(),
            AdapterKind::new(TRIGGER_TRUSTED_ADAPTER_KIND).expect("adapter kind"),
            AdapterInstallationId::new(TRIGGER_TRUSTED_ADAPTER_INSTALLATION_ID)
                .expect("installation id"),
            ExternalActorRef::new(TRIGGER_TRUSTED_EXTERNAL_ACTOR_NAMESPACE, user_id.as_str())
                .expect("actor ref"),
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
        // One-shot: fires once, then becomes Completed via clear_active_fire.
        schedule: TriggerSchedule::once(Utc::now() - chrono::Duration::seconds(120), "UTC")
            .expect("valid once schedule"),
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
    // and for clear_active_fire to run (which transitions state to Completed for
    // CompleteAfterFirstFire triggers). The first-pass loop breaks as soon as the
    // claim+prompt are seen; the settle may still be in flight.
    let final_record = wait_for_settled(
        &repo,
        &tenant_id,
        record.trigger_id,
        Duration::from_secs(5),
        |r| {
            r.last_fired_slot.is_some()
                && r.last_run_at.is_some()
                && r.state == TriggerState::Completed
        },
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
    assert!(
        final_record.last_fired_slot.is_some(),
        "once schedule: last_fired_slot should be set after fire — record: {final_record:?}",
    );
    assert!(
        final_record.last_run_at.is_some(),
        "once schedule: last_run_at should be set after fire — record: {final_record:?}",
    );
    assert_eq!(
        final_record.last_status,
        Some(TriggerRunStatus::Ok),
        "once schedule: last_status should be Ok after fire — record: {final_record:?}",
    );
    assert_eq!(
        final_record.state,
        TriggerState::Completed,
        "once schedule: state must be Completed after clear_active_fire — record: {final_record:?}",
    );
}

#[tokio::test]
async fn builtin_trigger_create_pairs_creator_and_poller_submits_turn() {
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

    let created = invoke_trigger_create(
        &runtime,
        json!({
            "name": "trigger-e2e-created-by-tool",
            "prompt": TRIGGER_PROMPT,
            "schedule": { "kind": "cron", "expression": "* * * * *", "timezone": "UTC" }
        }),
    )
    .await;
    assert_eq!(
        created["trigger"]["name"],
        json!("trigger-e2e-created-by-tool")
    );
    assert_eq!(created["trigger"]["state"], json!("scheduled"));
    assert!(created["trigger"]["last_status"].is_null());
    assert!(created["trigger"]["prompt"].is_null());
    assert!(created["trigger"]["tenant_id"].is_null());
    assert!(created["trigger"]["creator_user_id"].is_null());

    let repo = runtime
        .trigger_repository()
        .expect("local-dev runtime exposes trigger repository");
    let tenant_id = TenantId::new(TENANT).expect("tenant id");
    let user_id = UserId::new(USER).expect("user id");
    let trigger_id = TriggerId::parse(
        created["trigger"]["trigger_id"]
            .as_str()
            .expect("created trigger id"),
    )
    .expect("valid trigger id");

    let mut record = repo
        .get_trigger(tenant_id.clone(), trigger_id)
        .await
        .expect("get created trigger")
        .expect("created trigger persisted");
    assert_eq!(record.prompt, TRIGGER_PROMPT);
    assert_eq!(record.creator_user_id, user_id);
    assert_eq!(record.name, "trigger-e2e-created-by-tool");

    let original_next_run_at = record.next_run_at;
    record.next_run_at = Utc::now() - chrono::Duration::seconds(120);
    repo.upsert_trigger(record.clone())
        .await
        .expect("make created trigger due");

    let deadline = Instant::now() + Duration::from_secs(15);
    let mut record_was_mutated = false;
    let mut prompt_seen = false;

    while Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(100)).await;

        let current = repo
            .get_trigger(tenant_id.clone(), trigger_id)
            .await
            .expect("get trigger")
            .expect("record present");

        let mutated = current.last_fired_slot.is_some()
            || current.last_run_at.is_some()
            || current.last_status.is_some()
            || current.active_fire_slot.is_some();

        if mutated {
            record_was_mutated = true;
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

    let settled = wait_for_settled(&repo, &tenant_id, trigger_id, Duration::from_secs(5), |r| {
        r.last_fired_slot.is_some() && r.next_run_at > original_next_run_at
    })
    .await;

    runtime.shutdown().await.expect("runtime shutdown");

    let captured_contents = recording_gateway.captured_message_contents().await;
    assert!(
        record_was_mutated,
        "poller did not mutate trigger created through builtin.trigger_create — record: {settled:?}",
    );
    assert!(
        prompt_seen,
        "LLM gateway never received trigger prompt for builtin-created trigger — \
         captured_messages: {captured_contents:?}"
    );
    assert_eq!(settled.last_status, Some(TriggerRunStatus::Ok));
    assert!(
        settled.last_run_at.is_some(),
        "builtin-created trigger should record last_run_at after poller fire"
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
            AdapterKind::new(TRIGGER_TRUSTED_ADAPTER_KIND).expect("adapter kind"),
            AdapterInstallationId::new(TRIGGER_TRUSTED_ADAPTER_INSTALLATION_ID)
                .expect("installation id"),
            ExternalActorRef::new(TRIGGER_TRUSTED_EXTERNAL_ACTOR_NAMESPACE, user_id.as_str())
                .expect("actor ref"),
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

    // Seed a past-due one-shot trigger. An unpaired external actor is a
    // permanent pre-submit failure: the trusted trigger submitter must not send
    // a turn, and the one-shot is completed so it cannot re-fire the same slot
    // forever.
    let fire_at = Utc::now() - chrono::Duration::seconds(120);
    let record = TriggerRecord {
        trigger_id,
        tenant_id: tenant_id.clone(),
        creator_user_id: user_id,
        agent_id: Some(agent_id),
        project_id: None,
        name: "trigger-e2e-unpaired".to_string(),
        source: TriggerSourceKind::Schedule,
        schedule: TriggerSchedule::once(fire_at, "UTC").expect("valid once schedule"),
        prompt: TRIGGER_PROMPT.to_string(),
        state: TriggerState::Scheduled,
        next_run_at: fire_at,
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

    // The one-shot trigger records the permanent pre-submit failure and stops
    // retrying the already-past slot.
    assert_eq!(
        current.state,
        TriggerState::Completed,
        "unpaired one-shot trigger must be completed after permanent pre-submit failure — \
         state: {:?}, last_status: {:?}",
        current.state,
        current.last_status
    );
    assert_eq!(
        current.last_status,
        Some(TriggerRunStatus::Error),
        "unpaired one-shot trigger must record an error status — record: {current:?}"
    );
    assert_eq!(
        current.active_fire_slot, None,
        "completed failed one-shot trigger must not keep an active fire — record: {current:?}"
    );
    assert_eq!(
        current.active_run_ref, None,
        "completed failed one-shot trigger must not have an active run — record: {current:?}"
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
            AdapterKind::new(TRIGGER_TRUSTED_ADAPTER_KIND).expect("adapter kind"),
            AdapterInstallationId::new(TRIGGER_TRUSTED_ADAPTER_INSTALLATION_ID)
                .expect("installation id"),
            ExternalActorRef::new(TRIGGER_TRUSTED_EXTERNAL_ACTOR_NAMESPACE, user_id.as_str())
                .expect("actor ref"),
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
        // Every minute — recurring cron stays Scheduled after each fire.
        schedule: TriggerSchedule::cron("* * * * *").expect("valid cron expression"),
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
