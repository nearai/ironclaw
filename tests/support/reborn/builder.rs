//! `RebornIntegrationHarness` — the integration test tier that runs the full
//! internal Reborn stack and intercepts the model at the vendor-SDK seam.
//!
//! Unlike `RebornBinaryE2EHarness` (which swaps the whole `HostManagedModelGateway`
//! with `RebornTraceReplayModelGateway`), this tier wires the REAL
//! `LlmProviderModelGateway` over the REAL `ironclaw_llm` decorator chain
//! (`apply_decorator_chain`, hermetic passthrough) and only scripts the raw
//! provider underneath via `TraceLlm`. A turn therefore exercises model-profile
//! resolution, `CompletionRequest`/tool-definition assembly, and the
//! retry/routing/circuit/cache decorators for real.
//!
//! Slice 1 scope: InMemory storage, single text reply, `build → submit_turn →
//! assert_reply_contains`. A future `StorageMode::LibSql` variant is a
//! non-breaking addition (the builder defaults to InMemory directly today rather
//! than introducing a one-variant enum).

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use ironclaw_filesystem::InMemoryBackend;
use ironclaw_llm::{LlmProvider, SessionConfig, apply_decorator_chain, create_session_manager};
use ironclaw_loop_support::{
    EmptyUserProfileSource, HostManagedModelGateway, JsonSpawnSubagentInputCodec,
    SubagentSpawnLimits,
};
use ironclaw_product_adapters::{ProductInboundAck, ProductTriggerReason, ProductWorkflow};
use ironclaw_product_workflow::{
    ConversationBindingService, DefaultInboundTurnService, DefaultProductWorkflow,
    IdempotencyLedger, InboundTurnService, ProductConversationRouteKind, ResolveBindingRequest,
    ResolvedBinding,
};
use ironclaw_reborn::loop_exit_applier::{
    LoopExitEvidencePort, ThreadCheckpointLoopExitEvidencePort,
};
use ironclaw_reborn::model_gateway::{LlmModelProfilePolicy, LlmProviderModelGateway};
use ironclaw_reborn::runtime::{
    DefaultPlannedRuntimeConfig, DefaultPlannedRuntimeParts, RuntimeTurnStateStore,
    build_default_planned_runtime,
};
use ironclaw_reborn::subagent::{
    flavors::StaticSubagentDefinitionResolver, gate_resolution::BoundedSubagentGateResolutionStore,
    goal_store::InMemoryBoundedSubagentGoalStore,
};
use ironclaw_threads::{SessionThreadService, ThreadScope};
use ironclaw_turns::run_profile::{InMemoryLoopHostMilestoneSink, ModelProfileId};
use ironclaw_turns::{
    FilesystemTurnStateStore, GetRunStateRequest, InMemoryCheckpointStateStore,
    LoopCheckpointStore, TurnRunId, TurnScope, TurnStateStore, TurnStatus,
};

use super::filesystem::BlockingTurnStatePutFilesystem;
use super::harness::{
    EmptyIdentityContextSource, HarnessCapabilityMode, HarnessCapabilityRecorder,
    HarnessTurnBackend, HarnessTurnStorageBackend, HostRuntimeCapabilityHarness,
    RecordingTestCapabilityPort, scoped_turns_fs, test_product_scope,
};
use super::reply::RebornScriptedReply;
use super::scripted_provider::{SCRIPTED_MODEL_NAME, scripted_trace_llm};
use super::session_thread::RebornThreadHarness;
use super::test_adapter::{RebornTestIngress, RebornTestProductAdapter};

type HarnessResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// The actor/user that submits turns. Reused at binding-probe time and submit
/// time so both resolve to the same conversation binding (and thread).
const HARNESS_ACTOR_ID: &str = "host-user";
/// Model profile the planned runtime requests; the gateway policy permits it.
const INTERACTIVE_MODEL_PROFILE: &str = "interactive_model";

/// Selects the capability backend the integration harness wires.
enum RebornCapabilityBackend {
    /// Echo recorder: records capability invocations, executes nothing. Default —
    /// a text-only turn invokes no tool.
    Echo,
    /// Real first-party tool runtime (`builtin.http` + friends) with the recording
    /// `RuntimeHttpEgress` (scripted body, no network) — the §3.7 Tier-2 capture.
    BuiltinHttpTools,
}

/// Builder for [`RebornIntegrationHarness`]. The script is fixed at build time
/// (no post-build mutation), matching the existing harness's construction-time
/// queue.
pub struct RebornIntegrationHarnessBuilder {
    conversation_id: String,
    replies: Vec<RebornScriptedReply>,
    capability: RebornCapabilityBackend,
}

impl RebornIntegrationHarnessBuilder {
    /// Set the scripted model replies (consumed in order at the raw-provider seam).
    pub fn script(mut self, replies: impl IntoIterator<Item = RebornScriptedReply>) -> Self {
        self.replies = replies.into_iter().collect();
        self
    }

    /// Use the real first-party tool runtime so scripted tool calls execute through
    /// `RuntimeHttpEgress`, captured at the recording egress (no network). Required
    /// for tool-calling tests; a text-only turn needs only the default echo backend.
    pub fn with_builtin_http_tools(mut self) -> Self {
        self.capability = RebornCapabilityBackend::BuiltinHttpTools;
        self
    }

    /// Build the harness: apply hermetic env, wire the real model gateway over
    /// the scripted provider, and start the planned runtime.
    pub async fn build(self) -> HarnessResult<RebornIntegrationHarness> {
        apply_hermetic_env();

        // --- product workflow + binding -------------------------------------
        let adapter = RebornTestProductAdapter::new("reborn-itest", "itest-install")?;
        let ingress = RebornTestIngress::new(adapter);
        let scope = test_product_scope(
            "tenant-itest",
            "host-user",
            "agent-itest",
            Some("project-itest"),
        );
        let product_harness =
            super::product_workflow::RebornProductWorkflowHarness::filesystem_temp(scope)?;

        let probe = ingress.verified_text_envelope_with_trigger(
            "binding-probe",
            HARNESS_ACTOR_ID,
            &self.conversation_id,
            "hi",
            ProductTriggerReason::DirectChat,
        )?;
        let binding = product_harness
            .binding_service()?
            .resolve_binding(binding_request(&probe))
            .await?;
        let thread_scope = thread_scope_from_binding(&binding)?;
        let turn_scope = TurnScope::new_with_owner(
            binding.tenant_id.clone(),
            binding.agent_id.clone(),
            binding.project_id.clone(),
            binding.thread_id.clone(),
            binding.subject_user_id.clone(),
        );

        // --- durable stores (InMemory) --------------------------------------
        let thread_harness = RebornThreadHarness::filesystem_temp(thread_scope.clone())?;
        let turn_root = Arc::new(tempfile::tempdir()?);
        let turn_backend: Arc<HarnessTurnStorageBackend> =
            Arc::new(BlockingTurnStatePutFilesystem::new(InMemoryBackend::new()));
        let turn_store: Arc<FilesystemTurnStateStore<HarnessTurnBackend>> = Arc::new(
            FilesystemTurnStateStore::new(scoped_turns_fs(turn_backend, &binding)?),
        );
        let checkpoint_state_store = Arc::new(InMemoryCheckpointStateStore::default());
        let loop_checkpoint_store: Arc<dyn LoopCheckpointStore> = turn_store.clone();
        let milestone_sink = Arc::new(InMemoryLoopHostMilestoneSink::default());

        // --- real model gateway over the scripted raw provider --------------
        let raw: Arc<dyn LlmProvider> = Arc::new(scripted_trace_llm(self.replies));
        let session = create_session_manager(SessionConfig::default()).await;
        let llm_config = ironclaw_llm::testing::nearai_test_config(SCRIPTED_MODEL_NAME);
        let provider = apply_decorator_chain(raw, &llm_config, session).await?;
        let model_profile_id = ModelProfileId::new(INTERACTIVE_MODEL_PROFILE)
            .map_err(|reason| format!("invalid model profile id: {reason}"))?;
        let policy = LlmModelProfilePolicy::new().allow_model_profile(model_profile_id, None);
        let model_gateway: Arc<dyn HostManagedModelGateway> =
            Arc::new(LlmProviderModelGateway::new(provider, policy));

        // --- capability surface ---------------------------------------------
        // Echo by default (records, executes nothing — a text reply invokes no
        // tool). `with_builtin_http_tools` swaps in the real first-party tool
        // runtime so tool calls execute through `RuntimeHttpEgress`, captured at
        // the recording egress (§3.6/§3.7). Both backends flow through the shared
        // `HarnessCapabilityMode::into_parts` wiring (single mechanism). The echo
        // arm surfaces the port's own allowlist (not `CapabilityAllowSet::All`);
        // benign because a text-only turn invokes no tool.
        let capability_mode = match self.capability {
            RebornCapabilityBackend::Echo => {
                HarnessCapabilityMode::Recording(RecordingTestCapabilityPort::echo())
            }
            RebornCapabilityBackend::BuiltinHttpTools => HarnessCapabilityMode::HostRuntime(
                Arc::new(HostRuntimeCapabilityHarness::core_builtin_tools().await?),
            ),
        };
        let (
            capability_factory,
            capability_surface_resolver,
            capability_input_resolver,
            capability_result_writer,
            capability_recorder,
        ) = capability_mode.into_parts(milestone_sink.clone())?;

        // --- loop-exit evidence (plain; no gates/blocks in slice 1) ---------
        let turn_state_for_evidence: Arc<dyn TurnStateStore> = turn_store.clone();
        let loop_exit_evidence: Arc<dyn LoopExitEvidencePort> =
            Arc::new(ThreadCheckpointLoopExitEvidencePort::new_with_thread_scope(
                thread_harness.service.clone(),
                turn_state_for_evidence,
                Arc::clone(&loop_checkpoint_store),
                thread_scope.clone(),
            ));

        // --- planned runtime composition ------------------------------------
        let turn_state_for_runtime: Arc<dyn RuntimeTurnStateStore> = turn_store.clone();
        let composition = build_default_planned_runtime(DefaultPlannedRuntimeParts {
            turn_state: turn_state_for_runtime,
            thread_service: thread_harness.service.clone() as Arc<dyn SessionThreadService>,
            thread_scope: thread_scope.clone(),
            model_gateway,
            checkpoint_state_store,
            loop_checkpoint_store,
            milestone_sink,
            capability_factory,
            capability_surface_resolver,
            capability_result_writer,
            subagent_goal_store: Arc::new(InMemoryBoundedSubagentGoalStore::new()),
            subagent_gate_store: Arc::new(BoundedSubagentGateResolutionStore::new()),
            subagent_definition_resolver: Arc::new(StaticSubagentDefinitionResolver),
            subagent_spawn_input_codec: Arc::new(JsonSpawnSubagentInputCodec::new(
                capability_input_resolver,
            )),
            subagent_spawn_limits: SubagentSpawnLimits::default(),
            loop_exit_evidence,
            config: DefaultPlannedRuntimeConfig {
                poll_interval: Duration::from_millis(10),
                ..DefaultPlannedRuntimeConfig::default()
            },
            model_route_resolver: None,
            cancellation_factory: None,
            skill_context_source: None,
            input_queue: None,
            identity_context_source: Arc::new(EmptyIdentityContextSource),
            user_profile_source: Arc::new(EmptyUserProfileSource),
            model_policy_guard: None,
            model_budget_accountant: None,
            safety_context: None,
            hook_dispatcher_builder_factory: None,
            communication_context_provider: None,
            hook_security_audit_sink: None,
            turn_event_sink: None,
            attachment_read_port: None,
            scheduler_wake_wiring: None,
        })?;

        // --- product workflow over the coordinator --------------------------
        let binding_service: Arc<dyn ConversationBindingService> =
            Arc::new(product_harness.binding_service()?);
        let inbound: Arc<dyn InboundTurnService> = Arc::new(DefaultInboundTurnService::new(
            Arc::clone(&binding_service),
            thread_harness.service_instance()?,
            composition.coordinator.clone(),
        ));
        let ledger: Arc<dyn IdempotencyLedger> = Arc::new(product_harness.idempotency_ledger());
        let workflow = DefaultProductWorkflow::new(inbound, ledger, binding_service);

        Ok(RebornIntegrationHarness {
            ingress,
            workflow,
            conversation_id: self.conversation_id,
            binding,
            turn_scope,
            turn_store,
            thread_harness,
            scheduler_handle: Some(composition.scheduler_handle),
            event_seq: AtomicU64::new(1),
            capability_recorder,
            _product_harness: product_harness,
            _turn_root: turn_root,
        })
    }
}

/// Full-stack Reborn integration harness with a scripted raw provider beneath
/// the real decorator chain. See module docs.
pub struct RebornIntegrationHarness {
    ingress: RebornTestIngress,
    workflow: DefaultProductWorkflow,
    conversation_id: String,
    binding: ResolvedBinding,
    turn_scope: TurnScope,
    turn_store: Arc<FilesystemTurnStateStore<HarnessTurnBackend>>,
    thread_harness: RebornThreadHarness,
    scheduler_handle: Option<ironclaw_host_runtime::TurnRunSchedulerHandle>,
    event_seq: AtomicU64,
    capability_recorder: HarnessCapabilityRecorder,
    _product_harness: super::product_workflow::RebornProductWorkflowHarness,
    _turn_root: Arc<tempfile::TempDir>,
}

impl RebornIntegrationHarness {
    /// Default harness: InMemory storage, hermetic env, real decorator chain.
    pub fn test_default() -> RebornIntegrationHarnessBuilder {
        Self::builder("conv-itest")
    }

    /// Builder for a specific conversation id.
    pub fn builder(conversation_id: impl Into<String>) -> RebornIntegrationHarnessBuilder {
        RebornIntegrationHarnessBuilder {
            conversation_id: conversation_id.into(),
            replies: Vec::new(),
            capability: RebornCapabilityBackend::Echo,
        }
    }

    /// Submit a user turn and wait for it to complete.
    pub async fn submit_turn(&self, text: &str) -> HarnessResult<TurnRunId> {
        let event_id = format!("evt-{}", self.event_seq.fetch_add(1, Ordering::Relaxed));
        let envelope = self.ingress.verified_text_envelope_with_trigger(
            &event_id,
            HARNESS_ACTOR_ID,
            &self.conversation_id,
            text,
            ProductTriggerReason::DirectChat,
        )?;
        let ack = self.workflow.accept_inbound(envelope).await?;
        let run_id = match ack {
            ProductInboundAck::Accepted {
                submitted_run_id, ..
            } => submitted_run_id,
            other => return Err(format!("expected accepted inbound ack, got {other:?}").into()),
        };
        self.wait_for_completion(run_id).await?;
        Ok(run_id)
    }

    /// Assert the finalized assistant reply in thread history contains `text`.
    ///
    /// (Co-located with the harness fields it reads. When the `assert_*` family
    /// grows — `assert_capability_denied`/`assert_capability_order`, design §3.3 —
    /// it can move to a dedicated `assertions.rs` with deliberate field accessors.)
    pub async fn assert_reply_contains(&self, text: &str) -> HarnessResult<()> {
        self.thread_harness
            .assert_final_reply(self.binding.thread_id.clone(), text)
            .await
            .map_err(Into::into)
    }

    /// Assert the named capability was invoked through the real capability path
    /// (proves the scripted tool call actually ran the tool).
    pub async fn assert_tool_invoked(&self, capability_id: &str) -> HarnessResult<()> {
        let invocations = self.capability_recorder.invocations();
        if invocations
            .iter()
            .any(|invocation| invocation.capability_id.as_str() == capability_id)
        {
            return Ok(());
        }
        let seen: Vec<&str> = invocations
            .iter()
            .map(|invocation| invocation.capability_id.as_str())
            .collect();
        Err(format!("capability {capability_id:?} was not invoked; saw {seen:?}").into())
    }

    /// Assert a tool HTTP egress request was captured (Tier-2) whose URL contains
    /// `url_substr` — the proof that the tool crossed `RuntimeHttpEgress`.
    pub async fn assert_egress_request_matching(&self, url_substr: &str) -> HarnessResult<()> {
        let requests = self.capability_recorder.runtime_http_requests();
        if requests
            .iter()
            .any(|request| request.url.contains(url_substr))
        {
            return Ok(());
        }
        let seen: Vec<&str> = requests
            .iter()
            .map(|request| request.url.as_str())
            .collect();
        Err(format!(
            "no captured runtime HTTP egress request matching {url_substr:?}; saw {seen:?}"
        )
        .into())
    }

    async fn wait_for_completion(&self, run_id: TurnRunId) -> HarnessResult<()> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        loop {
            let state = self
                .turn_store
                .get_run_state(GetRunStateRequest {
                    scope: self.turn_scope.clone(),
                    run_id,
                })
                .await?;
            if state.status == TurnStatus::Completed {
                return Ok(());
            }
            if state.status.is_terminal() {
                return Err(format!(
                    "run reached terminal status {:?} before Completed; failure={:?}",
                    state.status, state.failure
                )
                .into());
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(format!(
                    "timed out waiting for Completed; last status={:?} failure={:?}",
                    state.status, state.failure
                )
                .into());
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
}

impl Drop for RebornIntegrationHarness {
    fn drop(&mut self) {
        // Scheduler shutdown is async and cannot run from Drop; dropping the
        // handle closes the command channel and the supervisor task exits.
        let _ = self.scheduler_handle.take();
    }
}

/// Hermetic env baked unconditionally so every test form inherits it and a
/// developer `.env` can never reach a vendor (design §2/§4.1). The chain itself
/// reads the explicit passthrough `LlmConfig`, so the LLM env vars are belt-and-
/// suspenders; keychain disable + UTC are genuinely load-bearing for hermeticity.
///
/// Applied exactly once per process via [`OnceLock`]: the values are constant,
/// and `cargo test` runs `#[tokio::test]`s in parallel threads within one binary
/// — a per-call `set_var`/`remove_var` would be a data race (and is `unsafe`
/// under edition 2024). Once-init runs before any concurrent `build()` mutates
/// or reads the environment.
fn apply_hermetic_env() {
    static HERMETIC_ENV: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    HERMETIC_ENV.get_or_init(|| {
        // SAFETY: `get_or_init` guarantees this closure runs exactly once across
        // all threads, so there is no concurrent env mutation/read.
        unsafe {
            std::env::set_var("IRONCLAW_DISABLE_OS_KEYCHAIN", "1");
            std::env::set_var("TZ", "UTC");
            std::env::set_var("LLM_MAX_RETRIES", "0");
            std::env::remove_var("NEARAI_CHEAP_MODEL");
            std::env::remove_var("NEARAI_FALLBACK_MODEL");
            std::env::remove_var("LLM_CIRCUIT_BREAKER_THRESHOLD");
            std::env::remove_var("LLM_RESPONSE_CACHE_ENABLED");
        }
    });
}

/// Assemble a `ResolveBindingRequest` from a verified inbound envelope. Slice 1
/// is DirectChat-only, so the route kind is `Direct`.
fn binding_request(
    envelope: &ironclaw_product_adapters::ProductInboundEnvelope,
) -> ResolveBindingRequest {
    ResolveBindingRequest {
        adapter_id: envelope.adapter_id().clone(),
        installation_id: envelope.installation_id().clone(),
        external_actor_ref: envelope.external_actor_ref().clone(),
        external_conversation_ref: envelope.external_conversation_ref().clone(),
        external_event_id: envelope.external_event_id().clone(),
        route_kind: ProductConversationRouteKind::Direct,
        auth_claim: envelope.auth_claim().clone(),
    }
}

fn thread_scope_from_binding(binding: &ResolvedBinding) -> HarnessResult<ThreadScope> {
    Ok(ThreadScope {
        tenant_id: binding.tenant_id.clone(),
        agent_id: binding
            .agent_id
            .clone()
            .ok_or("resolved binding missing agent id")?,
        project_id: binding.project_id.clone(),
        owner_user_id: binding.subject_user_id.clone(),
        mission_id: None,
    })
}
