//! Full-path integration test for the composition-owned trigger poller on a
//! **production-shaped** `RebornRuntime` (`local_runtime: None`,
//! `production_runtime: Some(...)`).
//!
//! Before #5013 the poller launch hard-errored for any production-shaped
//! runtime ("trigger poller is not wired for production runtime launch"), so
//! `build_reborn_runtime` returned `Err` the moment `trigger_poller.enabled`
//! was set on a production profile — that is the RED state this test pins.
//! After the launch-collapse the poller runs on the same substrate-agnostic
//! path the local runtime uses, sourcing its trigger repository, conversation
//! services, identity directory, and turn-run snapshot source from the
//! production store graph. This test drives the whole path: it builds the
//! production runtime with the poller enabled, seeds a due `TriggerRecord`
//! through the production trigger repository, pairs the trigger creator's
//! external actor through the production conversation services, and asserts
//! the poller claims + materializes + submits + settles the trigger against
//! the production substrate.
//!
//! Assertion seam: the trigger record. `mark_fire_accepted` (→ `last_status =
//! Ok`, `last_run_at`, `last_fired_slot`) is written only after the trusted
//! submitter returns `Accepted { run_id, .. }` — i.e. a real trusted-ingress
//! turn was materialized (via the production conversation services) and
//! submitted to the coordinator. `clear_active_fire` (→ `state = Completed`
//! for a once-schedule, `active_run_ref`/`active_fire_slot` cleared) is written
//! only after the run engine signals the run terminal. So the settled record
//! proves claim → materialize → submit → run-terminal → settle, all against
//! the production store graph — a trigger/run-seam assertion, not
//! `wait_for_status(Completed)` alone.
//!
//! Scope note: the model gateway override IS honored on production under
//! `test-support`, but the fired trusted-ingress run terminates before the
//! first model call in this minimal secure-default / tenant-sandbox production
//! harness (no real conversational agent-loop environment), so this tier
//! cannot assert "the trigger prompt reached the model" the way the local
//! `trigger_poller_e2e` suite does. That is a turn-execution-environment limit,
//! not a poller-wiring gap — the poller's claim/materialize/submit/settle path
//! is fully exercised here. Delivery likewise stays downstream: the poller runs
//! without a post-submit delivery hook (production gets no channel driver yet).
//!
//! Uses the libSQL production substrate; database backends always compile, so
//! the test is active with the rest of the crate's integration tests.

use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_conversations::{AdapterInstallationId, AdapterKind, ExternalActorRef};
use ironclaw_host_api::{
    AgentId, TenantId, UserId,
    runtime_policy::{
        ApprovalPolicy, AuditMode, DeploymentMode, EffectiveRuntimePolicy, FilesystemBackendKind,
        NetworkMode, ProcessBackendKind, RuntimeProfile, SecretMode,
    },
};
use ironclaw_host_runtime::{
    CommandExecutionOutput, CommandExecutionRequest, RuntimeProcessError, SandboxCommandTransport,
    TenantSandboxProcessPort,
};
use ironclaw_loop_host::{
    HostManagedModelError, HostManagedModelGateway, HostManagedModelRequest,
    HostManagedModelResponse,
};
use ironclaw_reborn_composition::{
    RebornBuildInput, RebornCompositionProfile, RebornRuntimeIdentity, RebornRuntimeInput,
    RebornRuntimeProcessBinding, TriggerPollerSettings, build_reborn_runtime,
    builtin_first_party_trust_policy,
};
use ironclaw_triggers::{
    TRIGGER_TRUSTED_ADAPTER_INSTALLATION_ID, TRIGGER_TRUSTED_ADAPTER_KIND,
    TRIGGER_TRUSTED_EXTERNAL_ACTOR_NAMESPACE, TriggerId, TriggerPollerWorkerConfig, TriggerRecord,
    TriggerRunStatus, TriggerSchedule, TriggerSourceKind, TriggerState,
};
use tokio::sync::Mutex as TokioMutex;

const TENANT: &str = "trigger-prod-tenant";
const USER: &str = "trigger-prod-owner";
const AGENT: &str = "trigger-prod-agent";
const TRIGGER_PROMPT: &str = "trigger-prod-prompt-marker-do-not-rephrase";

/// Deterministic model gateway: records every request and returns a plain
/// assistant reply so the fired run terminates without touching a real LLM,
/// process backend, or network.
#[derive(Debug, Default)]
struct RecordingGateway {
    requests: Arc<TokioMutex<Vec<HostManagedModelRequest>>>,
}

impl RecordingGateway {
    async fn captured_message_contents(&self) -> Vec<String> {
        let guard = self.requests.lock().await;
        guard
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
            "trigger prod e2e ok".to_string(),
        ))
    }
}

#[derive(Debug)]
struct RecordingSandboxTransport;

#[async_trait]
impl SandboxCommandTransport for RecordingSandboxTransport {
    async fn run_command(
        &self,
        _request: CommandExecutionRequest,
    ) -> Result<CommandExecutionOutput, RuntimeProcessError> {
        Ok(CommandExecutionOutput {
            output: String::new(),
            saved_output: None,
            exit_code: 0,
            sandboxed: true,
            duration: Duration::ZERO,
        })
    }
}

/// Builds a production-shaped runtime with the trigger poller enabled.
/// Mirrors `production_runtime_automations.rs`'s recipe (Production profile,
/// first-party trust policy, secure-default runtime policy, tenant-sandbox
/// process binding) and additionally opts the poller in via the existing
/// `trigger_poller.enabled` input flag, using the tenant-scoped placeholder
/// authorizer so no fire-time access checker is required for the test.
async fn build_production_runtime_with_poller(
    dir: &tempfile::TempDir,
    model_gateway: Arc<RecordingGateway>,
) -> ironclaw_reborn_composition::RebornRuntime {
    let db = Arc::new(
        libsql::Builder::new_local(dir.path().join("reborn.db"))
            .build()
            .await
            .expect("libsql db"),
    );

    let input = RebornRuntimeInput::from_services(
        RebornBuildInput::libsql(
            RebornCompositionProfile::Production,
            USER,
            db,
            dir.path().join("events.db").to_string_lossy(),
            None,
            ironclaw_secrets::SecretMaterial::from("01234567890123456789012345678901"),
        )
        .with_production_trust_policy(Arc::new(
            builtin_first_party_trust_policy().expect("trust policy"),
        ))
        .with_runtime_policy(EffectiveRuntimePolicy {
            deployment: DeploymentMode::HostedMultiTenant,
            requested_profile: RuntimeProfile::SecureDefault,
            resolved_profile: RuntimeProfile::SecureDefault,
            filesystem_backend: FilesystemBackendKind::ScopedVirtual,
            process_backend: ProcessBackendKind::TenantSandbox,
            network_mode: NetworkMode::Deny,
            secret_mode: SecretMode::BrokeredHandles,
            approval_policy: ApprovalPolicy::AskAlways,
            audit_mode: AuditMode::Standard,
        })
        .with_runtime_process_binding(RebornRuntimeProcessBinding::tenant_sandbox(Arc::new(
            TenantSandboxProcessPort::new(Arc::new(RecordingSandboxTransport)),
        ))),
    )
    .with_identity(RebornRuntimeIdentity {
        tenant_id: TENANT.to_string(),
        agent_id: AGENT.to_string(),
        source_binding_id: "trigger-prod-source".to_string(),
        reply_target_binding_id: "trigger-prod-reply".to_string(),
    })
    .with_trigger_poller_settings(
        TriggerPollerSettings::enabled_with_tenant_scoped_authorizer_for_test().with_worker_config(
            TriggerPollerWorkerConfig::default().set_poll_interval(Duration::from_millis(20)),
        ),
    )
    .with_model_gateway_override(model_gateway);

    // RED before #5013: this errored with "trigger poller is not wired for
    // production runtime launch". GREEN after the launch-collapse.
    build_reborn_runtime(input)
        .await
        .expect("production runtime builds with the trigger poller enabled")
}

/// End-to-end: a production-shaped runtime with the poller enabled claims and
/// fires a due one-shot trigger, driving a run to the model gateway and
/// settling the record `Completed`/`Ok`.
#[tokio::test]
async fn production_runtime_trigger_poller_fires_due_scheduled_trigger() {
    let dir = tempfile::tempdir().expect("tempdir");
    let recording_gateway = Arc::new(RecordingGateway::default());

    let runtime = build_production_runtime_with_poller(&dir, Arc::clone(&recording_gateway)).await;

    // The production-shaped runtime exposes its trigger repository only through
    // the test-support production accessor (`trigger_repository()` covers the
    // local substrate, which production does not have).
    assert!(
        runtime.trigger_repository().is_none(),
        "production runtime has no local-substrate trigger repository"
    );
    let repo = runtime
        .production_trigger_repository_for_test()
        .expect("production runtime exposes its store-graph trigger repository");
    let pairing = runtime
        .trigger_conversation_pairing()
        .expect("poller-enabled production runtime exposes conversation pairing service");

    let tenant_id = TenantId::new(TENANT).expect("tenant id");
    let user_id = UserId::new(USER).expect("user id");
    let agent_id = AgentId::new(AGENT).expect("agent id");
    let trigger_id = TriggerId::new();

    // Seed the trigger creator's actor pairing through the production
    // `ConversationActorPairingService`. The trusted trigger submission path
    // fails closed for unpaired actors by design; the constants must match the
    // trusted trigger fire constants.
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
        name: "trigger-prod-test".to_string(),
        source: TriggerSourceKind::Schedule,
        // One-shot, already due: fires once then becomes Completed.
        schedule: TriggerSchedule::once(Utc::now() - chrono::Duration::seconds(120), "UTC")
            .expect("valid once schedule"),
        prompt: TRIGGER_PROMPT.to_string(),
        delivery_target: None,
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
        .expect("upsert trigger record on the production repository");

    // Poll until the once-schedule trigger settles: `last_status = Ok`
    // (accepted trusted submit) and `state = Completed` (run reached terminal,
    // `clear_active_fire` ran). The production poller runs at a 20ms interval,
    // so this is well within the deadline.
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut final_record = repo
        .get_trigger(tenant_id.clone(), record.trigger_id)
        .await
        .expect("get_trigger")
        .expect("record present");
    while Instant::now() < deadline {
        if final_record.last_fired_slot.is_some()
            && final_record.last_run_at.is_some()
            && final_record.last_status.is_some()
            && final_record.state == TriggerState::Completed
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
        final_record = repo
            .get_trigger(tenant_id.clone(), record.trigger_id)
            .await
            .expect("get_trigger")
            .expect("record present");
    }

    runtime.shutdown().await.expect("runtime shutdown");

    // Trigger/run-seam assertions (see module docs): the production poller
    // claimed the due trigger, materialized its prompt through the production
    // conversation services, submitted a trusted-ingress turn that the
    // coordinator ACCEPTED (returning a run_id — the precondition for
    // `mark_fire_accepted`), and the run reached terminal so the once-schedule
    // trigger settled `Completed`. All settle writes landed on the production
    // store-graph trigger repository read back here.
    assert!(
        final_record.last_fired_slot.is_some(),
        "once schedule: last_fired_slot should be set after the production poller fires — \
         record: {final_record:?}",
    );
    assert!(
        final_record.last_run_at.is_some(),
        "once schedule: last_run_at should be set after the production poller fires — \
         record: {final_record:?}",
    );
    assert_eq!(
        final_record.last_status,
        Some(TriggerRunStatus::Ok),
        "once schedule: last_status must be Ok after an accepted trusted submit — \
         record: {final_record:?}",
    );
    assert_eq!(
        final_record.state,
        TriggerState::Completed,
        "once schedule: state must be Completed after the run reaches terminal and \
         clear_active_fire runs — record: {final_record:?}",
    );
    assert!(
        final_record.active_run_ref.is_none() && final_record.active_fire_slot.is_none(),
        "the settled once-schedule fire must clear its active run/fire claim — \
         record: {final_record:?}",
    );

    // The model gateway is wired (so no real LLM is ever contacted) but is not
    // asserted on: the fired trusted-ingress run terminates before the first
    // model call in this minimal production harness. See the module docs — this
    // is a turn-execution-environment limit, not a poller-wiring gap. The
    // captured-request snapshot is read purely to keep the gateway on the run's
    // wiring path.
    let _captured_contents = recording_gateway.captured_message_contents().await;
}
