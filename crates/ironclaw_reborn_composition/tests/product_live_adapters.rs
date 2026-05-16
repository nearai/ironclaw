use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ironclaw_host_api::{
    AgentId, CapabilityId, CapabilitySet, ExecutionContext, ExtensionId, RuntimeKind, TenantId,
    ThreadId, TrustClass, UserId,
};
use ironclaw_host_runtime::{
    CapabilitySurfacePolicy, SurfaceKind, VisibleCapabilityRequest as HostVisibleCapabilityRequest,
};
use ironclaw_loop_support::{
    HostIdentityContextBuildError, HostIdentityContextCandidate, HostIdentityContextSource,
    HostInputBatch, HostInputEnvelope, HostInputQueue, HostInputQueueError, HostManagedModelError,
    HostManagedModelErrorKind, HostManagedModelGateway, HostManagedModelRequest,
    HostManagedModelResponse, LoopCapabilityInputResolver, LoopCapabilityResultWriter,
    ProductLiveCancellationProbe, RunCancellationFactory, RunCancellationHandle,
    verify_product_live_cancellation_probe,
};
use ironclaw_reborn::loop_exit_applier::ThreadCheckpointLoopExitEvidencePort;
use ironclaw_reborn::model_routes::{ModelSelectionMode, ModelSlot};
use ironclaw_reborn::runtime::{
    DefaultPlannedRuntimeConfig, DefaultPlannedRuntimeParts, build_product_live_planned_runtime,
};
use ironclaw_reborn_composition::{
    ProductLiveCapabilityIo, ProductLiveModelRouteSettings, ProductLivePlannedRuntimeAdapterConfig,
    ProductLivePlannedRuntimeAdapterError, ProductLivePlannedRuntimeAdapters,
    ProductLiveVisibleCapabilityRequestConfig, RebornBuildInput, RebornServices,
    build_reborn_services, capability_allowlist, visible_capability_request_for_run,
};
use ironclaw_threads::{InMemorySessionThreadService, ThreadScope};
use ironclaw_trust::EffectiveTrustClass;
use ironclaw_turns::{
    CheckpointStateStore, InMemoryCheckpointStateStore, InMemoryLoopCheckpointStore,
    InMemoryTurnStateStore, LoopCheckpointStore, LoopResultRef, RunProfileResolutionRequest,
    RunProfileResolver, TurnId, TurnRunId, TurnScope, TurnStateStore,
    run_profile::{
        AgentLoopHostError, CapabilityInputRef, InMemoryLoopHostMilestoneSink,
        InstructionSafetyContext, LoopCancelReasonKind, LoopModelBudgetAccountant,
        LoopModelPolicyGuard, LoopRunContext, NoOpBudgetAccountant, NoOpPolicyGuard, PromptMode,
        VisibleCapabilityRequest,
    },
};

#[tokio::test]
async fn capability_io_resolves_staged_inputs_and_materializes_run_scoped_results() {
    let io = ProductLiveCapabilityIo::default();
    let run_context = loop_run_context("capability-io").await;
    let input_ref = io
        .stage_input(&run_context, serde_json::json!({ "text": "hello" }))
        .unwrap();

    let resolved = io
        .resolve_capability_input(&run_context, &input_ref)
        .await
        .unwrap();
    assert_eq!(resolved, serde_json::json!({ "text": "hello" }));

    let result_ref = io
        .write_capability_result(
            &run_context,
            &capability_id("demo.echo"),
            serde_json::json!({ "reply": "hello" }),
        )
        .await
        .unwrap();

    assert!(
        result_ref
            .as_str()
            .starts_with(&format!("result:{}.", run_context.run_id)),
        "result refs must be scoped to the loop run: {}",
        result_ref.as_str()
    );
    assert_eq!(
        io.result_for_ref(&run_context, &result_ref).unwrap(),
        serde_json::json!({ "reply": "hello" })
    );
}

#[tokio::test]
async fn capability_io_rejects_cross_run_input_and_result_refs() {
    let io = ProductLiveCapabilityIo::default();
    let first_run = loop_run_context("capability-io-first").await;
    let second_run = loop_run_context("capability-io-second").await;
    let input_ref = io
        .stage_input(&first_run, serde_json::json!({ "text": "first" }))
        .unwrap();

    let input_error = io
        .resolve_capability_input(&second_run, &input_ref)
        .await
        .expect_err("cross-run input refs must fail closed");
    assert_eq!(
        input_error.kind,
        ironclaw_turns::run_profile::AgentLoopHostErrorKind::ScopeMismatch
    );

    let result_ref = io
        .write_capability_result(
            &first_run,
            &capability_id("demo.echo"),
            serde_json::json!({ "reply": "first" }),
        )
        .await
        .unwrap();
    let result_error = io
        .result_for_ref(&second_run, &result_ref)
        .expect_err("cross-run result refs must fail closed");
    assert_eq!(
        result_error.kind,
        ironclaw_turns::run_profile::AgentLoopHostErrorKind::ScopeMismatch
    );
}

#[tokio::test]
async fn visible_capability_request_builder_scopes_context_to_loop_run() {
    let run_context = loop_run_context("visible-builder").await;
    let request = visible_capability_request_for_run(
        &run_context,
        ProductLiveVisibleCapabilityRequestConfig::new(
            UserId::new("user-visible-builder").unwrap(),
            ExtensionId::new("planned-driver").unwrap(),
            RuntimeKind::FirstParty,
            TrustClass::System,
            SurfaceKind::new("agent_loop").unwrap(),
            CapabilitySurfacePolicy::allow_all(),
        )
        .with_grants(CapabilitySet::default())
        .with_provider_trust(
            ExtensionId::new("demo").unwrap(),
            EffectiveTrustClass::user_trusted(),
        ),
    )
    .unwrap();

    assert_eq!(request.context.tenant_id, run_context.scope.tenant_id);
    assert_eq!(request.context.agent_id, run_context.scope.agent_id);
    assert_eq!(request.context.project_id, run_context.scope.project_id);
    assert_eq!(
        request.context.thread_id.as_ref(),
        Some(&run_context.thread_id)
    );
    assert_eq!(
        request.context.resource_scope.thread_id.as_ref(),
        Some(&run_context.thread_id)
    );
    assert!(
        request
            .provider_trust
            .contains_key(&ExtensionId::new("demo").unwrap())
    );
}

#[tokio::test]
async fn adapter_bundle_requires_host_runtime_facade() {
    let result = ProductLivePlannedRuntimeAdapters::from_services(
        &RebornServices::disabled(),
        adapter_config(),
    );

    assert!(matches!(
        result,
        Err(ProductLivePlannedRuntimeAdapterError::MissingHostRuntime)
    ));
}

#[tokio::test]
async fn adapter_bundle_wires_required_product_live_components() {
    let root = tempfile::tempdir().unwrap();
    let services = build_reborn_services(RebornBuildInput::local_dev(
        "adapter-test-owner",
        root.path().join("local-dev"),
    ))
    .await
    .unwrap();
    let adapters =
        ProductLivePlannedRuntimeAdapters::from_services(&services, adapter_config()).unwrap();

    let route = adapters
        .model_route_resolver
        .resolve_model_route(ModelSlot::Default)
        .unwrap();
    assert_eq!(route.route().provider_id(), "nearai");
    assert_eq!(route.route().model_id(), "qwen3-coder");
    assert_eq!(route.policy_mode(), ModelSelectionMode::ManagedOnly);

    let context = loop_run_context("adapter-config").await;
    let allow_set = adapters
        .capability_surface_resolver
        .resolve(&context)
        .await
        .unwrap();
    assert!(allow_set.permits(&capability_id("demo.allowed")));
    assert!(!allow_set.permits(&capability_id("demo.denied")));

    let readiness = verify_product_live_cancellation_probe(adapters.cancellation_factory.as_ref())
        .expect("turn-state cancellation factory should expose a live probe");
    assert_eq!(
        readiness,
        ironclaw_loop_support::ProductLiveCancellationReadiness::ExternallyControllable
    );

    let capability_port = adapters
        .capability_factory
        .create_capability_port(&context)
        .await
        .unwrap();
    let visible = capability_port
        .visible_capabilities(VisibleCapabilityRequest)
        .await
        .unwrap();
    assert!(
        !visible.version.as_str().is_empty(),
        "host-runtime capability facade should supply a concrete surface version"
    );
}

#[tokio::test]
async fn adapter_bundle_satisfies_product_live_runtime_readiness_gate() {
    let root = tempfile::tempdir().unwrap();
    let services = build_reborn_services(RebornBuildInput::local_dev(
        "runtime-gate-owner",
        root.path().join("local-dev"),
    ))
    .await
    .unwrap();
    let thread_service = Arc::new(InMemorySessionThreadService::default());
    let turn_state = Arc::new(InMemoryTurnStateStore::default());
    let checkpoint_state_store = Arc::new(InMemoryCheckpointStateStore::default());
    let loop_checkpoint_store = Arc::new(InMemoryLoopCheckpointStore::default());
    let milestone_sink = Arc::new(InMemoryLoopHostMilestoneSink::default());
    let thread_scope = thread_scope("runtime-gate");
    let adapters =
        ProductLivePlannedRuntimeAdapters::from_services(&services, adapter_config()).unwrap();

    let turn_state_for_evidence: Arc<dyn TurnStateStore> = turn_state.clone();
    let loop_checkpoint_for_evidence: Arc<dyn LoopCheckpointStore> = loop_checkpoint_store.clone();
    let composition = build_product_live_planned_runtime(DefaultPlannedRuntimeParts {
        turn_state,
        thread_service: Arc::clone(&thread_service),
        thread_scope: thread_scope.clone(),
        model_gateway: Arc::new(StubModelGateway),
        checkpoint_state_store: checkpoint_state_store as Arc<dyn CheckpointStateStore>,
        loop_checkpoint_store,
        milestone_sink,
        capability_factory: adapters.capability_factory,
        capability_surface_resolver: adapters.capability_surface_resolver,
        loop_exit_evidence: Arc::new(ThreadCheckpointLoopExitEvidencePort::new_with_thread_scope(
            thread_service,
            turn_state_for_evidence,
            loop_checkpoint_for_evidence,
            thread_scope,
        )),
        config: DefaultPlannedRuntimeConfig::default(),
        model_route_resolver: Some(adapters.model_route_resolver),
        cancellation_factory: Some(adapters.cancellation_factory),
        skill_context_source: None,
        input_queue: Some(adapters.input_queue),
        identity_context_source: adapters.identity_context_source,
        model_policy_guard: Some(adapters.model_policy_guard),
        model_budget_accountant: Some(adapters.model_budget_accountant),
        safety_context: Some(adapters.safety_context),
    })
    .expect("adapter bundle should satisfy the product-live readiness gate");

    let profile = composition
        .run_profile_resolver
        .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
        .await
        .unwrap();
    assert_eq!(profile.profile_id.as_str(), "reborn-planned-default");
}

#[tokio::test]
async fn model_route_settings_wire_default_and_mission_slots() {
    let root = tempfile::tempdir().unwrap();
    let services = build_reborn_services(RebornBuildInput::local_dev(
        "route-settings-owner",
        root.path().join("local-dev"),
    ))
    .await
    .unwrap();
    let settings = ProductLiveModelRouteSettings::new("nearai", "qwen3-coder")
        .unwrap()
        .with_mission_route("openrouter", "anthropic/claude-sonnet-4")
        .unwrap();
    let adapters = ProductLivePlannedRuntimeAdapters::from_services(
        &services,
        ProductLivePlannedRuntimeAdapterConfig {
            model_routes: settings,
            ..adapter_config()
        },
    )
    .unwrap();

    let mission = adapters
        .model_route_resolver
        .resolve_model_route(ModelSlot::Mission)
        .unwrap();
    assert_eq!(mission.route().provider_id(), "openrouter");
    assert_eq!(mission.route().model_id(), "anthropic/claude-sonnet-4");
}

fn adapter_config() -> ProductLivePlannedRuntimeAdapterConfig {
    ProductLivePlannedRuntimeAdapterConfig {
        visible_capability_request: host_visible_capability_request("adapter-config"),
        capability_input_resolver: Arc::new(UnusedCapabilityIo),
        capability_result_writer: Arc::new(UnusedCapabilityIo),
        capability_allow_set: capability_allowlist([capability_id("demo.allowed")]),
        model_routes: ProductLiveModelRouteSettings::new("nearai", "qwen3-coder").unwrap(),
        cancellation_factory: Arc::new(ReadyRunCancellationFactory::default()),
        input_queue: Arc::new(EmptyInputQueue),
        identity_context_source: Arc::new(EmptyIdentityContextSource),
        model_policy_guard: Arc::new(NoOpPolicyGuard) as Arc<dyn LoopModelPolicyGuard>,
        model_budget_accountant: Arc::new(NoOpBudgetAccountant)
            as Arc<dyn LoopModelBudgetAccountant>,
        safety_context: InstructionSafetyContext::new(
            "policy:adapter-test",
            "adapter test safety policy",
        )
        .unwrap(),
        milestone_sink: None,
    }
}

async fn loop_run_context(label: &str) -> LoopRunContext {
    let context = host_visible_capability_request(label).context;
    let resolved = ironclaw_reborn::planned_driver_factory::default_planned_run_profile_resolver()
        .unwrap()
        .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
        .await
        .unwrap();
    LoopRunContext::new(
        TurnScope::new(
            context.tenant_id,
            context.agent_id,
            context.project_id,
            context.thread_id.unwrap(),
        ),
        TurnId::new(),
        TurnRunId::new(),
        resolved,
    )
}

fn host_visible_capability_request(label: &str) -> HostVisibleCapabilityRequest {
    let mut context = ExecutionContext::local_default(
        UserId::new(format!("user-{label}")).unwrap(),
        ExtensionId::new("adapter-test").unwrap(),
        RuntimeKind::Wasm,
        TrustClass::UserTrusted,
        CapabilitySet::default(),
        ironclaw_host_api::MountView::default(),
    )
    .unwrap();
    let thread_id = ThreadId::new(format!("thread-{label}")).unwrap();
    context.thread_id = Some(thread_id.clone());
    context.resource_scope.thread_id = Some(thread_id);
    HostVisibleCapabilityRequest::new(context, SurfaceKind::new("agent_loop").unwrap())
}

fn thread_scope(label: &str) -> ThreadScope {
    ThreadScope {
        tenant_id: TenantId::new(format!("tenant-{label}")).unwrap(),
        agent_id: AgentId::new(format!("agent-{label}")).unwrap(),
        project_id: None,
        owner_user_id: None,
        mission_id: None,
    }
}

fn capability_id(value: &str) -> CapabilityId {
    CapabilityId::new(value).unwrap()
}

struct EmptyInputQueue;

#[async_trait]
impl HostInputQueue for EmptyInputQueue {
    async fn next_after(
        &self,
        _run_id: TurnRunId,
        after: ironclaw_turns::run_profile::LoopInputCursorToken,
        _limit: usize,
    ) -> Result<HostInputBatch, HostInputQueueError> {
        Ok(HostInputBatch {
            inputs: Vec::<HostInputEnvelope>::new(),
            next_cursor: after,
        })
    }

    async fn ack_consumed(
        &self,
        _run_id: TurnRunId,
        _tokens: Vec<ironclaw_turns::run_profile::LoopInputAckToken>,
    ) -> Result<(), HostInputQueueError> {
        Ok(())
    }
}

struct EmptyIdentityContextSource;

#[async_trait]
impl HostIdentityContextSource for EmptyIdentityContextSource {
    async fn load_identity_candidates(
        &self,
        _run_context: &LoopRunContext,
        _mode: PromptMode,
    ) -> Result<Vec<HostIdentityContextCandidate>, HostIdentityContextBuildError> {
        Ok(Vec::new())
    }
}

struct UnusedCapabilityIo;

#[async_trait]
impl LoopCapabilityInputResolver for UnusedCapabilityIo {
    async fn resolve_capability_input(
        &self,
        _run_context: &LoopRunContext,
        _input_ref: &CapabilityInputRef,
    ) -> Result<serde_json::Value, AgentLoopHostError> {
        Ok(serde_json::json!({}))
    }
}

#[async_trait]
impl LoopCapabilityResultWriter for UnusedCapabilityIo {
    async fn write_capability_result(
        &self,
        _run_context: &LoopRunContext,
        _capability_id: &CapabilityId,
        _output: serde_json::Value,
    ) -> Result<LoopResultRef, AgentLoopHostError> {
        Ok(LoopResultRef::new("result:adapter-test").unwrap())
    }
}

struct StubModelGateway;

#[async_trait]
impl HostManagedModelGateway for StubModelGateway {
    async fn stream_model(
        &self,
        _request: HostManagedModelRequest,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        Err(HostManagedModelError::safe(
            HostManagedModelErrorKind::Unavailable,
            "model gateway not exercised by adapter readiness test",
        ))
    }
}

#[derive(Default)]
struct ReadyRunCancellationFactory {
    handles: Arc<Mutex<HashMap<TurnRunId, RunCancellationHandle>>>,
}

#[async_trait]
impl RunCancellationFactory for ReadyRunCancellationFactory {
    async fn handle_for_run(
        &self,
        _scope: &TurnScope,
        run_id: TurnRunId,
    ) -> Result<RunCancellationHandle, AgentLoopHostError> {
        let handle = RunCancellationHandle::default();
        self.handles
            .lock()
            .expect("ready cancellation lock poisoned")
            .insert(run_id, handle.clone());
        Ok(handle)
    }

    fn product_live_cancellation_probe(&self) -> Option<Box<dyn ProductLiveCancellationProbe>> {
        Some(Box::new(ReadyCancellationProbe {
            handle: RunCancellationHandle::default(),
        }))
    }
}

struct ReadyCancellationProbe {
    handle: RunCancellationHandle,
}

impl ProductLiveCancellationProbe for ReadyCancellationProbe {
    fn request_cancellation(
        &self,
        reason_kind: LoopCancelReasonKind,
    ) -> Result<(), AgentLoopHostError> {
        self.handle.request(reason_kind);
        Ok(())
    }

    fn is_cancellation_observed(&self) -> Result<bool, AgentLoopHostError> {
        Ok(self.handle.is_requested())
    }
}
