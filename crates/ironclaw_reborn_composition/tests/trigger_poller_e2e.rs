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
use chrono::{TimeZone, Utc};
use ironclaw_conversations::{AdapterInstallationId, AdapterKind, ExternalActorRef};
use ironclaw_host_api::{
    AgentId, CapabilityGrant, CapabilityGrantId, CapabilityId, CapabilitySet, EffectKind,
    ExecutionContext, ExtensionId, GrantConstraints, MountView, NetworkPolicy, Principal,
    ProviderToolName, ResourceEstimate, RuntimeKind, TenantId, TrustClass, UserId,
};
use ironclaw_host_runtime::{
    RuntimeCapabilityOutcome, RuntimeCapabilityRequest, TRIGGER_CREATE_CAPABILITY_ID,
    TRIGGER_PAUSE_CAPABILITY_ID,
};
use ironclaw_loop_support::{
    HostManagedModelError, HostManagedModelGateway, HostManagedModelRequest,
    HostManagedModelResponse,
};
use ironclaw_reborn::runtime::ToolDisclosureMode;
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
use ironclaw_turns::run_profile::{
    LoopCapabilityPort, ProviderToolCall, RegisterProviderToolCallRequest,
};
use serde_json::{Value, json};
use tokio::sync::Mutex as TokioMutex;

const TENANT: &str = "trigger-e2e-tenant";
const USER: &str = "trigger-e2e-owner";
const AGENT: &str = "trigger-e2e-agent";
const TRIGGER_PROMPT: &str = "trigger-e2e-prompt-marker-do-not-rephrase";
/// Name of the trigger the fired run's capability call attempts to create.
/// Issue #5505's fix strips `builtin.trigger_create` from the model-visible
/// surface for a `scheduled_trigger` fire, so this name must NEVER appear in
/// the trigger repository after the fire settles.
const SELF_CREATE_MARKER_TRIGGER_NAME: &str = "self-created-by-fire-should-not-exist";
/// Mirrors the production capability-id -> provider-tool-name transform
/// (`provider_tool_name_base` in `ironclaw_loop_support::capability_port`,
/// private to that crate) so this test tracks the `TRIGGER_*_CAPABILITY_ID`
/// constants automatically instead of hardcoding their mapped results. Only
/// the `.` -> `__` replacement is needed for these capability ids (all of the
/// form `"builtin.trigger_*"`) — the full transform also maps other
/// non-alphanumeric/`_`/`-` characters to `_`, but none of these ids have
/// any. Referenced directly rather than discovered via `tool_definitions()`
/// because on a `scheduled_trigger` surface the fix removes every mutator
/// from `tool_definitions()` entirely — there would be nothing to look up.
fn provider_tool_name_for_capability_id(capability_id: &str) -> String {
    capability_id.replace('.', "__")
}

/// Shared by every scripted `HostManagedModelGateway` test double in this
/// file that records raw requests (`RecordingGateway`,
/// `SingleMutatorAttemptGateway`) — flattens every captured request's message
/// contents into one list.
async fn captured_message_contents(
    requests: &Arc<TokioMutex<Vec<HostManagedModelRequest>>>,
) -> Vec<String> {
    let guard = requests.lock().await;
    guard
        .iter()
        .flat_map(|req| req.messages.iter().map(|m| m.content.clone()))
        .collect()
}

#[derive(Debug, Default)]
struct RecordingGateway {
    requests: Arc<TokioMutex<Vec<HostManagedModelRequest>>>,
}

impl RecordingGateway {
    async fn captured_message_contents(&self) -> Vec<String> {
        captured_message_contents(&self.requests).await
    }

    async fn request_count_containing(&self, needle: &str) -> usize {
        let snapshot = self.requests.lock().await;
        snapshot
            .iter()
            .filter(|req| req.messages.iter().any(|m| m.content.contains(needle)))
            .count()
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

/// Model gateway for T0-5505-E2E: on the fired run's first turn, registers a
/// single provider tool call against the run's actual composed
/// `LoopCapabilityPort` (every subsequent turn just replies plainly, so the
/// run terminates after at most one capability-call round trip). Shared by
/// the `trigger_create` denial test below and
/// `scheduled_trigger_fire_can_pause_another_trigger_it_manages` — both
/// attempt exactly one mutator capability and differ only in which
/// `ProviderToolCall` they set via [`Self::set_tool_call`] before the fire.
///
/// `CapabilitySurfaceDenyFilter::register_provider_tool_call` resolves the
/// provider tool call to a capability id via its OWN
/// `provider_tool_call_capability_ids` override (delegating to `inner` so
/// deferred/disclosed tools still resolve — see #5149's progressive tool
/// disclosure), then scope-checks the resolved id against its deny set
/// before ever building a candidate — the exact seam a production
/// LLM-provider-backed gateway uses to turn a raw tool-call response into a
/// `CapabilityCallCandidate`, so registering through it here exercises the
/// *real* `PerSurfaceCapabilityDenyDecorator` / `CapabilitySurfaceDenyFilter`
/// composition chain, not a stand-in. If registration is denied, there is
/// nothing to dispatch and a plain reply is returned instead so the run
/// still terminates cleanly; if permitted, the candidate is forwarded so the
/// loop actually dispatches it for real.
#[derive(Default)]
struct SingleMutatorAttemptGateway {
    requests: Arc<TokioMutex<Vec<HostManagedModelRequest>>>,
    call_count: TokioMutex<usize>,
    tool_call: TokioMutex<Option<ProviderToolCall>>,
    /// `None` until the first turn runs. `Some(Ok(()))` if the registration
    /// was accepted (capability was visible + permitted on this run's
    /// surface), `Some(Err(safe_summary))` if it was denied before a
    /// candidate could even be built.
    outcome: TokioMutex<Option<Result<(), String>>>,
}

impl SingleMutatorAttemptGateway {
    async fn captured_message_contents(&self) -> Vec<String> {
        captured_message_contents(&self.requests).await
    }

    async fn set_tool_call(&self, tool_call: ProviderToolCall) {
        *self.tool_call.lock().await = Some(tool_call);
    }

    async fn outcome(&self) -> Option<Result<(), String>> {
        self.outcome.lock().await.clone()
    }

    /// The `trigger_create` provider tool call attempt, shaped for its real
    /// input schema (see
    /// `ironclaw_host_runtime::first_party_tools::trigger_management`'s
    /// `TriggerCreateInput`) — attempts to create a second trigger named
    /// `SELF_CREATE_MARKER_TRIGGER_NAME`.
    fn create_tool_call() -> ProviderToolCall {
        ProviderToolCall {
            provider_id: "trigger-e2e-provider".to_string(),
            provider_model_id: "trigger-e2e-model".to_string(),
            turn_id: Some("trigger-e2e-self-create-turn".to_string()),
            id: format!("trigger-e2e-mutator-call-{TRIGGER_CREATE_CAPABILITY_ID}"),
            name: ProviderToolName::new(provider_tool_name_for_capability_id(
                TRIGGER_CREATE_CAPABILITY_ID,
            ))
            .expect("mutator provider tool name is valid"),
            arguments: json!({
                "name": SELF_CREATE_MARKER_TRIGGER_NAME,
                "prompt": "run-time action steps -- this trigger must never be created",
                "schedule": { "kind": "cron", "expression": "* * * * *", "timezone": "UTC" },
            }),
            response_reasoning: None,
            reasoning: None,
            signature: None,
        }
    }

    /// The `trigger_pause` provider tool call attempt against `target_trigger_id`,
    /// shaped for its real input schema (see
    /// `ironclaw_host_runtime::first_party_tools::trigger_management`'s
    /// `TriggerStateInput`).
    fn pause_tool_call(target_trigger_id: TriggerId) -> ProviderToolCall {
        ProviderToolCall {
            provider_id: "trigger-e2e-provider".to_string(),
            provider_model_id: "trigger-e2e-model".to_string(),
            turn_id: Some("trigger-e2e-pause-other-turn".to_string()),
            id: format!("trigger-e2e-mutator-call-{TRIGGER_PAUSE_CAPABILITY_ID}"),
            name: ProviderToolName::new(provider_tool_name_for_capability_id(
                TRIGGER_PAUSE_CAPABILITY_ID,
            ))
            .expect("pause provider tool name is valid"),
            arguments: json!({ "trigger_id": target_trigger_id.to_string() }),
            response_reasoning: None,
            reasoning: None,
            signature: None,
        }
    }
}

#[async_trait]
impl HostManagedModelGateway for SingleMutatorAttemptGateway {
    async fn stream_model(
        &self,
        request: HostManagedModelRequest,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        // Only reachable if the driver host ever omits the capability port
        // from the model request (it should not — the composed loop always
        // wires one). Recording + a plain reply keeps this branch harmless
        // either way instead of silently no-op'ing the test.
        self.requests.lock().await.push(request);
        Ok(HostManagedModelResponse::assistant_reply(
            "trigger e2e ok (no capability port on request)".to_string(),
        ))
    }

    async fn stream_model_with_capabilities(
        &self,
        request: HostManagedModelRequest,
        capabilities: Arc<dyn LoopCapabilityPort>,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        self.requests.lock().await.push(request);
        let is_first_call = {
            let mut count = self.call_count.lock().await;
            *count += 1;
            *count == 1
        };

        if !is_first_call {
            return Ok(HostManagedModelResponse::assistant_reply(
                "trigger e2e ok".to_string(),
            ));
        }

        let tool_call = self
            .tool_call
            .lock()
            .await
            .clone()
            .expect("test sets a tool call before triggering the fire");
        let registration = capabilities
            .register_provider_tool_call(RegisterProviderToolCallRequest::new(tool_call))
            .await;
        match registration {
            Ok(candidate) => {
                *self.outcome.lock().await = Some(Ok(()));
                // Permitted — forward the candidate so the loop actually
                // dispatches it, letting repository-side assertions catch a
                // regression too.
                Ok(HostManagedModelResponse::capability_calls(
                    vec![candidate],
                    "",
                ))
            }
            Err(error) => {
                *self.outcome.lock().await = Some(Err(error.safe_summary.clone()));
                Ok(HostManagedModelResponse::assistant_reply(
                    "capability unavailable; continuing".to_string(),
                ))
            }
        }
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

fn current_minute_slot() -> chrono::DateTime<Utc> {
    let now_seconds = Utc::now().timestamp();
    let minute_seconds = now_seconds - now_seconds.rem_euclid(60);
    Utc.timestamp_opt(minute_seconds, 0)
        .single()
        .expect("valid minute timestamp")
}

/// Shared runtime builder. Every test passes the `TriggerPollerSettings` it
/// wants; identity, runtime policy, and model-gateway override are shared.
///
/// Generic over the concrete gateway type (rather than
/// `Arc<dyn HostManagedModelGateway>`) so every call site — the existing
/// `RecordingGateway` ones and `SingleMutatorAttemptGateway`'s — passes its
/// `Arc<Concrete>` unchanged; the unsized coercion to
/// `Arc<dyn HostManagedModelGateway>` happens once, here, instead of at every
/// call site.
async fn build_runtime_with<G: HostManagedModelGateway + 'static>(
    root: &tempfile::TempDir,
    model_gateway: Arc<G>,
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
        .with_model_gateway_override(model_gateway);

    build_reborn_runtime(input).await.expect("runtime builds")
}

/// Same as [`build_runtime_with`], but with an explicit
/// [`ToolDisclosureMode`] instead of the implicit `Off` default. Kept
/// separate (rather than adding a parameter to `build_runtime_with`) because
/// `build_runtime_with` has 8 existing call sites for concerns unrelated to
/// tool disclosure — churning all of them to thread through a mode they never
/// vary would be unnecessary here.
async fn build_runtime_with_tool_disclosure<G: HostManagedModelGateway + 'static>(
    root: &tempfile::TempDir,
    model_gateway: Arc<G>,
    trigger_poller: TriggerPollerSettings,
    tool_disclosure: ToolDisclosureMode,
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
        .with_model_gateway_override(model_gateway)
        .with_tool_disclosure(tool_disclosure);

    build_reborn_runtime(input).await.expect("runtime builds")
}

async fn invoke_trigger_create(runtime: &RebornRuntime, input: Value) -> Value {
    // The Tools-settings global auto-approve switch is authoritative for
    // first-party tool dispatch; turn it on for the trigger management
    // scope so the create call (and the poller-submitted turn that shares the
    // same tenant/user) exercise the dispatch path instead of stopping at the
    // per-tool approval gate.
    let auto_approve = runtime
        .services()
        .local_dev_auto_approve_settings_for_test()
        .expect("local-dev exposes auto-approve settings for test");
    let auto_approve_scope = trigger_management_execution_context().resource_scope;
    auto_approve
        .set(ironclaw_approvals::AutoApproveSettingInput {
            updated_by: Principal::User(auto_approve_scope.user_id.clone()),
            scope: auto_approve_scope,
            enabled: true,
        })
        .await
        .expect("enable global auto-approve for trigger management dispatch");

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
async fn builtin_created_recurring_trigger_fires_again_after_first_run_settles() {
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
            "name": "trigger-e2e-created-by-tool-fires-twice",
            "prompt": TRIGGER_PROMPT,
            "schedule": { "kind": "cron", "expression": "* * * * *", "timezone": "UTC" }
        }),
    )
    .await;

    let repo = runtime
        .trigger_repository()
        .expect("local-dev runtime exposes trigger repository");
    let tenant_id = TenantId::new(TENANT).expect("tenant id");
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
    let first_due_slot = current_minute_slot() - chrono::Duration::minutes(1);
    let second_due_slot = record
        .schedule
        .next_slot_after(first_due_slot)
        .expect("valid recurring schedule")
        .expect("recurring schedule should have a second slot");
    record.next_run_at = first_due_slot;
    repo.upsert_trigger(record.clone())
        .await
        .expect("make first recurring slot due");

    let second = wait_for_settled(
        &repo,
        &tenant_id,
        trigger_id,
        Duration::from_secs(15),
        |r| {
            r.last_fired_slot
                .map(|slot| slot >= second_due_slot)
                .unwrap_or(false)
                && r.last_run_at.is_some()
                && r.last_status == Some(TriggerRunStatus::Ok)
                && r.active_fire_slot.is_none()
                && r.active_run_ref.is_none()
                && r.next_run_at > second_due_slot
        },
    )
    .await;

    runtime.shutdown().await.expect("runtime shutdown");

    let request_count = recording_gateway
        .request_count_containing(TRIGGER_PROMPT)
        .await;
    assert!(
        request_count >= 2,
        "recurring trigger should submit once per due slot — requests containing prompt: {request_count}"
    );
    assert_eq!(
        second.state,
        TriggerState::Scheduled,
        "recurring trigger must remain Scheduled after the second fire — record: {second:?}"
    );
    assert_eq!(
        second.last_status,
        Some(TriggerRunStatus::Ok),
        "second fire should settle successfully — record: {second:?}"
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

    // Seed a past-due one-shot trigger. An unpaired external actor blocks
    // trusted trigger materialization before any turn can be submitted. This
    // is retryable: the trigger records the failed attempt, clears the active
    // claim, and remains Scheduled until the actor is paired.
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

    // The one-shot trigger records the blocked pre-submit failure and remains
    // retryable instead of completing the already-past slot.
    assert_eq!(
        current.state,
        TriggerState::Scheduled,
        "unpaired one-shot trigger must remain Scheduled after blocked pre-submit failure — \
         state: {:?}, last_status: {:?}",
        current.state,
        current.last_status
    );
    assert_eq!(
        current.last_status,
        Some(TriggerRunStatus::Error),
        "unpaired trigger must record the retryable failure — record: {current:?}"
    );
    assert_eq!(
        current.active_fire_slot, None,
        "blocked failed one-shot trigger must not keep an active fire — record: {current:?}"
    );
    assert_eq!(
        current.active_run_ref, None,
        "blocked failed one-shot trigger must not have an active run — record: {current:?}"
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

/// T0-5505-E2E: end-to-end proof that issue #5505's fix composes through the
/// real Reborn runtime — a scheduled-trigger fire resolves the dedicated
/// `scheduled_trigger` capability surface, and a fired run that tries to
/// create a *second* trigger cannot, because `trigger_create` is stripped
/// from that surface (`builtin.trigger_list`, `builtin.trigger_remove`,
/// `builtin.trigger_pause`, `builtin.trigger_resume`, and firing itself all
/// stay intact — see `scheduled_trigger_fire_can_pause_another_trigger_it_manages`
/// below for a full-path proof that a permitted mutator actually dispatches).
///
/// Unlike the other tests in this file, the fired run's model gateway
/// (`SingleMutatorAttemptGateway`, set to attempt `trigger_create`) does not
/// just record requests — on the fired run's first turn it registers a real
/// `builtin.trigger_create` provider tool call against the run's actual
/// composed `LoopCapabilityPort` (the exact seam a native provider tool-call
/// response goes through in production), attempting to create a second
/// trigger named `SELF_CREATE_MARKER_TRIGGER_NAME`. See
/// `SingleMutatorAttemptGateway`'s doc comment for why this exercises the
/// real `PerSurfaceCapabilityDenyDecorator` / `CapabilitySurfaceDenyFilter`
/// chain instead of a stand-in.
#[tokio::test]
async fn scheduled_trigger_fire_cannot_invoke_trigger_mutators() {
    scheduled_trigger_denies_create_with_tool_disclosure(ToolDisclosureMode::Off).await;
}

/// Same coverage as `scheduled_trigger_fire_cannot_invoke_trigger_mutators`,
/// but with the runtime built under `ToolDisclosureMode::Bridged` instead of
/// the default `Off`.
///
/// This is not a redundant copy of the `Off` variant above. PR #5515
/// self-review: the deny decorator (`PerSurfaceCapabilityDenyDecorator` /
/// `CapabilitySurfaceDenyFilter`) is deliberately wired in `runtime.rs`
/// *after* the conditional `ToolDisclosureCapabilityDecorator` so the
/// `trigger_create` denial stays outermost — and therefore still wins —
/// even when bridged tool disclosure is enabled. Before this test,
/// `Bridged` had exactly one usage anywhere in the repo (an unrelated
/// system-prompt test), so nothing exercised that decorator-ordering
/// composition end-to-end; a decorator-order or bridged-disclosure
/// regression could have re-exposed `trigger_create` without any
/// whole-path test failing. Keep this alongside the `Off` variant rather
/// than folding it in — it pins the composition order, not just the deny
/// outcome.
#[tokio::test]
async fn scheduled_trigger_fire_cannot_invoke_trigger_mutators_with_bridged_disclosure() {
    scheduled_trigger_denies_create_with_tool_disclosure(ToolDisclosureMode::Bridged).await;
}

async fn scheduled_trigger_denies_create_with_tool_disclosure(tool_disclosure: ToolDisclosureMode) {
    let root = tempfile::tempdir().expect("tempdir");
    let gateway = Arc::new(SingleMutatorAttemptGateway::default());
    gateway
        .set_tool_call(SingleMutatorAttemptGateway::create_tool_call())
        .await;

    let runtime = build_runtime_with_tool_disclosure(
        &root,
        Arc::clone(&gateway),
        TriggerPollerSettings::enabled_with_tenant_scoped_authorizer_for_test().with_worker_config(
            TriggerPollerWorkerConfig {
                poll_interval: Duration::from_millis(20),
                ..Default::default()
            },
        ),
        tool_disclosure,
    )
    .await;

    // Create the one legitimate trigger through the builtin tool (direct
    // dispatch — bypasses the model, so `gateway` is not invoked yet).
    let created = invoke_trigger_create(
        &runtime,
        json!({
            "name": "trigger-e2e-self-create-guard",
            "prompt": TRIGGER_PROMPT,
            "schedule": { "kind": "cron", "expression": "* * * * *", "timezone": "UTC" }
        }),
    )
    .await;
    assert_eq!(
        created["trigger"]["name"],
        json!("trigger-e2e-self-create-guard")
    );

    let repo = runtime
        .trigger_repository()
        .expect("local-dev runtime exposes trigger repository");
    let tenant_id = TenantId::new(TENANT).expect("tenant id");
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
    record.next_run_at = Utc::now() - chrono::Duration::seconds(120);
    repo.upsert_trigger(record.clone())
        .await
        .expect("make created trigger due");

    // Wait for the fire to settle. This is the model's ONLY turn where a
    // capability call can be attempted (`invoke_trigger_create` above never
    // touched the model), so once `last_status` is set, the create
    // self-attempt inside `gateway` has already run to completion.
    let settled = wait_for_settled(
        &repo,
        &tenant_id,
        trigger_id,
        Duration::from_secs(15),
        |r| r.last_fired_slot.is_some() && r.last_run_at.is_some() && r.last_status.is_some(),
    )
    .await;

    runtime.shutdown().await.expect("runtime shutdown");

    let captured_contents = gateway.captured_message_contents().await;
    let outcome = gateway.outcome().await;

    assert!(
        outcome.is_some(),
        "the fired run must have attempted the trigger_create registration — \
         captured_messages: {captured_contents:?}"
    );

    // Core assertion, mechanism-level: the surface must deny the
    // `trigger_create` registration. `trigger_create` is the id on the
    // scheduled_trigger deny set (the fix's `PerSurfaceCapabilityDenyDecorator`
    // / `CapabilitySurfaceDenyFilter`), so `register_provider_tool_call`'s own
    // scope check on the resolved capability id fails closed before a
    // candidate is ever built — see `SingleMutatorAttemptGateway`'s doc
    // comment for the exact call chain.
    //
    // GUARD AGAINST FALSE-PASS: pre-fix (or if the production deny constant
    // ever drops `trigger_create`), the scheduled_trigger capability surface
    // would not deny it, so the scope check above would pass and
    // `capabilities.register_provider_tool_call(...)` inside the gateway
    // would return `Ok(candidate)` — with a REAL, run-scoped staged input,
    // because `register_provider_tool_call` is the exact path a native
    // provider tool call uses to stage its arguments through the run's real
    // `LocalDevCapabilityIo`. The loop would then actually dispatch it, and
    // the marker trigger asserted absent below WOULD exist.
    const DENIED_SUMMARY: &str = "provider tool call targets a disabled capability";
    assert_eq!(
        outcome,
        Some(Err(DENIED_SUMMARY.to_string())),
        "expected the scheduled_trigger surface to deny the trigger_create \
         registration attempt made from inside the fired run"
    );

    // The denial must not otherwise break the fire: the original trigger
    // still settles Ok, exactly like the happy-path tests above.
    assert_eq!(
        settled.last_status,
        Some(TriggerRunStatus::Ok),
        "the original trigger must still settle Ok — the fix blocks only \
         trigger_create, not the fire itself — record: {settled:?}"
    );

    // Belt-and-suspenders behavioral check straight against the repository:
    // regardless of how the denial surfaced, no second trigger was ever
    // persisted, and the only trigger that exists is the original.
    let all_triggers = repo
        .list_triggers(tenant_id)
        .await
        .expect("list triggers after fire settles");
    assert!(
        all_triggers
            .iter()
            .all(|trigger| trigger.name != SELF_CREATE_MARKER_TRIGGER_NAME),
        "a scheduled-trigger fire must not be able to create a second trigger — \
         found triggers: {all_triggers:?}"
    );
    assert_eq!(
        all_triggers.len(),
        1,
        "exactly the original trigger should exist after the fire settles — \
         found triggers: {all_triggers:?}"
    );
    assert_eq!(all_triggers[0].trigger_id, trigger_id);
    assert_eq!(
        all_triggers[0].state,
        TriggerState::Scheduled,
        "the trigger_create denial must not otherwise disturb the original \
         trigger's state: {:?}",
        all_triggers[0]
    );
}

/// T0-5505-E2E (pause/resume/remove permit decision): a scheduled-trigger
/// fire may need to manage the trigger fleet — pausing, resuming, or
/// removing a trigger — to complete its task, so unlike `trigger_create`,
/// `builtin.trigger_pause` is NOT denied on the `scheduled_trigger`
/// capability surface. This proves the permit decision holds through the
/// real composed pipeline, not just the mechanism-level
/// `scheduled_trigger_surface_excludes_create_interactive_includes_it` unit
/// test in `ironclaw_reborn::runtime` — `register_provider_tool_call`
/// resolves and scope-checks the capability id, and a genuine
/// `PerSurfaceCapabilityDenyDecorator` regression that started denying
/// `trigger_pause` again would fail this test's registration assertion.
/// `trigger_remove`/`trigger_resume` share the exact same
/// `CapabilitySurfaceDenyFilter::permits` code path as `trigger_pause` (all
/// three are simply absent from `SCHEDULED_TRIGGER_DENIED_CAPABILITY_IDS`),
/// so one full-path proof is representative; duplicating this test per
/// mutator would not exercise a different mechanism.
///
/// Targets a *different* trigger than the one that fires — a fired trigger
/// mutating its own record mid-run is a distinct, more involved scenario
/// (interaction with the poller's own settle write) that this test
/// deliberately does not attempt.
#[tokio::test]
async fn scheduled_trigger_fire_can_pause_another_trigger_it_manages() {
    let root = tempfile::tempdir().expect("tempdir");
    let gateway = Arc::new(SingleMutatorAttemptGateway::default());

    let runtime = build_runtime_with(
        &root,
        Arc::clone(&gateway),
        TriggerPollerSettings::enabled_with_tenant_scoped_authorizer_for_test().with_worker_config(
            TriggerPollerWorkerConfig {
                poll_interval: Duration::from_millis(20),
                ..Default::default()
            },
        ),
    )
    .await;

    // The trigger that fires and does the managing.
    let firer = invoke_trigger_create(
        &runtime,
        json!({
            "name": "trigger-e2e-pause-firer",
            "prompt": TRIGGER_PROMPT,
            "schedule": { "kind": "cron", "expression": "* * * * *", "timezone": "UTC" }
        }),
    )
    .await;
    let firer_trigger_id = TriggerId::parse(
        firer["trigger"]["trigger_id"]
            .as_str()
            .expect("created firer trigger id"),
    )
    .expect("valid firer trigger id");

    // A separate trigger the firer's fire manages (pauses).
    let managed = invoke_trigger_create(
        &runtime,
        json!({
            "name": "trigger-e2e-pause-managed",
            "prompt": TRIGGER_PROMPT,
            "schedule": { "kind": "cron", "expression": "* * * * *", "timezone": "UTC" }
        }),
    )
    .await;
    let managed_trigger_id = TriggerId::parse(
        managed["trigger"]["trigger_id"]
            .as_str()
            .expect("created managed trigger id"),
    )
    .expect("valid managed trigger id");

    let repo = runtime
        .trigger_repository()
        .expect("local-dev runtime exposes trigger repository");
    let tenant_id = TenantId::new(TENANT).expect("tenant id");

    let mut firer_record = repo
        .get_trigger(tenant_id.clone(), firer_trigger_id)
        .await
        .expect("get firer trigger")
        .expect("firer trigger persisted");
    firer_record.next_run_at = Utc::now() - chrono::Duration::seconds(120);
    repo.upsert_trigger(firer_record)
        .await
        .expect("make firer trigger due");

    gateway
        .set_tool_call(SingleMutatorAttemptGateway::pause_tool_call(
            managed_trigger_id,
        ))
        .await;

    let settled = wait_for_settled(
        &repo,
        &tenant_id,
        firer_trigger_id,
        Duration::from_secs(15),
        |r| r.last_fired_slot.is_some() && r.last_run_at.is_some() && r.last_status.is_some(),
    )
    .await;

    runtime.shutdown().await.expect("runtime shutdown");

    assert_eq!(
        gateway.outcome().await,
        Some(Ok(())),
        "expected the scheduled_trigger surface to permit the trigger_pause \
         registration attempt made from inside the fired run"
    );
    assert_eq!(
        settled.last_status,
        Some(TriggerRunStatus::Ok),
        "the firer trigger must still settle Ok — record: {settled:?}"
    );

    let managed_after = repo
        .get_trigger(tenant_id, managed_trigger_id)
        .await
        .expect("get managed trigger after fire settles")
        .expect("managed trigger still exists");
    assert_eq!(
        managed_after.state,
        TriggerState::Paused,
        "the permitted trigger_pause dispatch must have actually paused the \
         managed trigger: {managed_after:?}"
    );
}
