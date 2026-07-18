//! Crate-tier RED-then-GREEN coverage for the harness-port-seam P1 Change 1
//! test-support constructor: `test_support::create_refreshing_capability_port_for_test`
//! must drive the REAL production capability-port factory
//! (`create_refreshing_capability_port`) with in-crate stub
//! doubles, not a hand-rebuilt wrap order.
//!
//! Stubs here are intentionally minimal: a `HostRuntime` that echoes back a
//! `VisibleCapability` per granted capability id (so `capability_id_filter`
//! narrowing is observable), and a shared input/result io double standing in
//! for production's `StagedCapabilityIo` (one object, both roles).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use ironclaw_approvals::test_support::{
    in_memory_backed_auto_approve_setting_store,
    in_memory_backed_capability_permission_override_store,
    in_memory_backed_persistent_approval_policy_store,
};
use ironclaw_authorization::in_memory_backed_capability_lease_store;
use ironclaw_host_api::{
    AgentId, CapabilityDescriptor, CapabilityId, EffectKind, ExtensionId, MountAlias, MountGrant,
    MountPermissions, MountView, PermissionMode, ProjectId, ProviderToolName, ResourceEstimate,
    RuntimeKind, TenantId, ThreadId, UserId, VirtualPath,
};
use ironclaw_host_runtime::RuntimeCapabilityOutcome;
use ironclaw_host_runtime::{
    CancelRuntimeWorkOutcome, CancelRuntimeWorkRequest, CapabilitySurfaceVersion, HostRuntime,
    HostRuntimeError, HostRuntimeHealth, HostRuntimeStatus, RuntimeCapabilityRequest,
    RuntimeCapabilityResumeRequest, RuntimeStatusRequest, VisibleCapability,
    VisibleCapabilityAccess, VisibleCapabilityRequest, VisibleCapabilitySurface,
};
use ironclaw_loop_host::{
    CapabilityResultWrite, CapabilityWriteResult, LoopCapabilityInputResolver,
    LoopCapabilityResultWriter,
};
use ironclaw_product_workflow::{
    ProjectCaller, ProjectService, ProjectServiceError, RebornAddMemberRequest,
    RebornCreateProjectRequest, RebornDeleteProjectRequest, RebornGetProjectRequest,
    RebornListMembersRequest, RebornListMembersResponse, RebornListProjectsRequest,
    RebornListProjectsResponse, RebornProjectInfo, RebornProjectMemberInfo, RebornProjectResponse,
    RebornProjectRole, RebornProjectState, RebornRemoveMemberRequest,
    RebornUpdateMemberRoleRequest, RebornUpdateProjectRequest,
};
use ironclaw_reborn_composition::test_support::{
    PROJECT_CREATE_CAPABILITY_ID, RESULT_READ_CAPABILITY_ID, RefreshingCapabilityPortTestParts,
    create_refreshing_capability_port_for_test,
};
use ironclaw_turns::run_profile::{
    AgentLoopHostError, AgentLoopHostErrorKind, CapabilityInputRef, CapabilityInvocation,
    CapabilityOutcome, InMemoryLoopHostMilestoneSink, InMemoryRunProfileResolver, LoopRunContext,
    ProviderToolCall, RegisterProviderToolCallRequest, RunProfileResolutionRequest,
};
use ironclaw_turns::{RunProfileResolver, TurnId, TurnRunId, TurnScope};

/// Echoes a runtime-visible capability for every grant in the request's
/// context, so `capability_id_filter` narrowing (applied upstream of this
/// stub, in `build_inner`) is directly observable in `tool_definitions()`.
/// Also records the `ExecutionContext` of every `invoke_capability` call and
/// the `provider_trust` map of every `visible_capabilities` call, so tests
/// can observe what `build_inner` actually resolved and handed to the host
/// runtime (mount overrides, extra provider trust) rather than just that the
/// port assembled.
struct StubHostRuntime {
    invocation_contexts: StdMutex<Vec<ironclaw_host_api::ExecutionContext>>,
    visible_provider_trust: StdMutex<Vec<BTreeMap<ExtensionId, ironclaw_trust::TrustDecision>>>,
}

impl StubHostRuntime {
    fn new() -> Self {
        Self {
            invocation_contexts: StdMutex::new(Vec::new()),
            visible_provider_trust: StdMutex::new(Vec::new()),
        }
    }

    fn invocation_contexts(&self) -> Vec<ironclaw_host_api::ExecutionContext> {
        self.invocation_contexts
            .lock()
            .expect("invocation contexts lock")
            .clone()
    }

    fn visible_provider_trust(&self) -> Vec<BTreeMap<ExtensionId, ironclaw_trust::TrustDecision>> {
        self.visible_provider_trust
            .lock()
            .expect("visible provider trust lock")
            .clone()
    }
}

#[async_trait]
impl HostRuntime for StubHostRuntime {
    async fn invoke_capability(
        &self,
        request: RuntimeCapabilityRequest,
    ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
        self.invocation_contexts
            .lock()
            .expect("invocation contexts lock")
            .push(request.context.clone());
        Ok(RuntimeCapabilityOutcome::Completed(Box::new(
            ironclaw_host_runtime::RuntimeCapabilityCompleted {
                capability_id: request.capability_id,
                output: serde_json::json!({"ok": true}),
                display_preview: None,
                usage: ironclaw_host_api::ResourceUsage::default(),
            },
        )))
    }

    async fn resume_capability(
        &self,
        _request: RuntimeCapabilityResumeRequest,
    ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
        Err(HostRuntimeError::unavailable(
            "stub host runtime does not resume capabilities",
        ))
    }

    async fn visible_capabilities(
        &self,
        request: VisibleCapabilityRequest,
    ) -> Result<VisibleCapabilitySurface, HostRuntimeError> {
        self.visible_provider_trust
            .lock()
            .expect("visible provider trust lock")
            .push(request.provider_trust.clone());
        let capabilities = request
            .context
            .grants
            .grants
            .iter()
            .map(|grant| VisibleCapability {
                descriptor: CapabilityDescriptor {
                    id: grant.capability.clone(),
                    provider: ExtensionId::new("builtin").expect("static provider id is valid"),
                    runtime: RuntimeKind::FirstParty,
                    trust_ceiling: ironclaw_host_api::TrustClass::UserTrusted,
                    description: format!("stub capability {}", grant.capability.as_str()),
                    parameters_schema: serde_json::json!({"type": "object", "properties": {}}),
                    effects: vec![EffectKind::ReadFilesystem],
                    default_permission: PermissionMode::Allow,
                    runtime_credentials: Vec::new(),
                    network_targets: Vec::new(),
                    resource_profile: None,
                },
                access: VisibleCapabilityAccess::Available,
                estimated_resources: ResourceEstimate::default(),
            })
            .collect();
        Ok(VisibleCapabilitySurface {
            version: CapabilitySurfaceVersion::new("stub-v1").expect("static version is valid"),
            capabilities,
        })
    }

    async fn cancel_work(
        &self,
        _request: CancelRuntimeWorkRequest,
    ) -> Result<CancelRuntimeWorkOutcome, HostRuntimeError> {
        Ok(CancelRuntimeWorkOutcome {
            cancelled: Vec::new(),
            already_terminal: Vec::new(),
            unsupported: Vec::new(),
        })
    }

    async fn runtime_status(
        &self,
        _request: RuntimeStatusRequest,
    ) -> Result<HostRuntimeStatus, HostRuntimeError> {
        Ok(HostRuntimeStatus {
            active_work: Vec::new(),
        })
    }

    async fn health(&self) -> Result<HostRuntimeHealth, HostRuntimeError> {
        Ok(HostRuntimeHealth::default())
    }
}

/// Stands in for production's `StagedCapabilityIo`: ONE object implementing
/// both `LoopCapabilityInputResolver` and `LoopCapabilityResultWriter`, so
/// `Arc::clone`s into both config fields let input-ref/result-ref correlate
/// through a single shared store (the invariant the config's shared-io
/// doc-comment pins).
struct SharedStubCapabilityIo {
    inputs: StdMutex<HashMap<String, serde_json::Value>>,
    results: StdMutex<HashMap<String, serde_json::Value>>,
    next_id: AtomicU64,
}

impl SharedStubCapabilityIo {
    fn new() -> Self {
        Self {
            inputs: StdMutex::new(HashMap::new()),
            results: StdMutex::new(HashMap::new()),
            next_id: AtomicU64::new(0),
        }
    }

    fn staged_result(&self, result_ref: &str) -> Option<serde_json::Value> {
        self.results
            .lock()
            .expect("results lock")
            .get(result_ref)
            .cloned()
    }
}

#[async_trait]
impl LoopCapabilityInputResolver for SharedStubCapabilityIo {
    async fn resolve_capability_input(
        &self,
        _run_context: &LoopRunContext,
        input_ref: &CapabilityInputRef,
    ) -> Result<serde_json::Value, AgentLoopHostError> {
        self.inputs
            .lock()
            .expect("inputs lock")
            .get(input_ref.as_str())
            .cloned()
            .ok_or_else(|| {
                AgentLoopHostError::new(AgentLoopHostErrorKind::Internal, "missing staged input")
            })
    }

    async fn register_provider_tool_call_input(
        &self,
        _run_context: &LoopRunContext,
        tool_call: &ProviderToolCall,
    ) -> Result<CapabilityInputRef, AgentLoopHostError> {
        let n = self.next_id.fetch_add(1, Ordering::SeqCst);
        let input_ref = CapabilityInputRef::new(format!("input:stub-run:{n}"))
            .map_err(|reason| AgentLoopHostError::new(AgentLoopHostErrorKind::Internal, reason))?;
        self.inputs
            .lock()
            .expect("inputs lock")
            .insert(input_ref.as_str().to_string(), tool_call.arguments.clone());
        Ok(input_ref)
    }
}

#[async_trait]
impl LoopCapabilityResultWriter for SharedStubCapabilityIo {
    async fn write_capability_result(
        &self,
        write: CapabilityResultWrite<'_>,
    ) -> Result<CapabilityWriteResult, AgentLoopHostError> {
        let n = self.next_id.fetch_add(1, Ordering::SeqCst);
        let result_ref = ironclaw_turns::LoopResultRef::new(format!("result:stub-run.{n}"))
            .map_err(|reason| AgentLoopHostError::new(AgentLoopHostErrorKind::Internal, reason))?;
        let byte_len = serde_json::to_vec(&write.output)
            .map(|bytes| bytes.len() as u64)
            .unwrap_or(0);
        self.results
            .lock()
            .expect("results lock")
            .insert(result_ref.as_str().to_string(), write.output);
        Ok(CapabilityWriteResult::without_output_digest(
            result_ref, byte_len,
        ))
    }
}

/// Always creates a single fixed project; enough for the `project_create`
/// synthetic capability's dispatch path — this test does not exercise
/// listing/reading/updating projects.
struct StubProjectService;

#[async_trait]
impl ProjectService for StubProjectService {
    async fn list_projects(
        &self,
        _caller: ProjectCaller,
        _request: RebornListProjectsRequest,
    ) -> Result<RebornListProjectsResponse, ProjectServiceError> {
        Err(ProjectServiceError::Unavailable)
    }

    async fn create_project(
        &self,
        caller: ProjectCaller,
        request: RebornCreateProjectRequest,
    ) -> Result<RebornProjectResponse, ProjectServiceError> {
        let now = chrono::Utc::now();
        Ok(RebornProjectResponse {
            project: RebornProjectInfo {
                project_id: format!("project-{}", caller.user_id.as_str()),
                name: request.name,
                description: request.description,
                icon: None,
                color: None,
                metadata: serde_json::Value::Null,
                state: RebornProjectState::Active,
                role: RebornProjectRole::Owner,
                created_at: now,
                updated_at: now,
            },
        })
    }

    async fn get_project(
        &self,
        _caller: ProjectCaller,
        _request: RebornGetProjectRequest,
    ) -> Result<RebornProjectResponse, ProjectServiceError> {
        Err(ProjectServiceError::NotFound)
    }

    async fn update_project(
        &self,
        _caller: ProjectCaller,
        _request: RebornUpdateProjectRequest,
    ) -> Result<RebornProjectResponse, ProjectServiceError> {
        Err(ProjectServiceError::NotFound)
    }

    async fn delete_project(
        &self,
        _caller: ProjectCaller,
        _request: RebornDeleteProjectRequest,
    ) -> Result<(), ProjectServiceError> {
        Err(ProjectServiceError::NotFound)
    }

    async fn list_members(
        &self,
        _caller: ProjectCaller,
        _request: RebornListMembersRequest,
    ) -> Result<RebornListMembersResponse, ProjectServiceError> {
        Err(ProjectServiceError::NotFound)
    }

    async fn add_member(
        &self,
        _caller: ProjectCaller,
        _request: RebornAddMemberRequest,
    ) -> Result<RebornProjectMemberInfo, ProjectServiceError> {
        Err(ProjectServiceError::NotFound)
    }

    async fn update_member_role(
        &self,
        _caller: ProjectCaller,
        _request: RebornUpdateMemberRoleRequest,
    ) -> Result<RebornProjectMemberInfo, ProjectServiceError> {
        Err(ProjectServiceError::NotFound)
    }

    async fn remove_member(
        &self,
        _caller: ProjectCaller,
        _request: RebornRemoveMemberRequest,
    ) -> Result<(), ProjectServiceError> {
        Err(ProjectServiceError::NotFound)
    }
}

async fn run_context(label: &str) -> LoopRunContext {
    let scope = TurnScope::new(
        TenantId::new(format!("tenant-{label}")).expect("tenant id"),
        Some(AgentId::new(format!("agent-{label}")).expect("agent id")),
        Some(ProjectId::new(format!("project-{label}")).expect("project id")),
        ThreadId::new(format!("thread-{label}")).expect("thread id"),
    );
    let resolved = InMemoryRunProfileResolver::default()
        .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
        .await
        .expect("profile resolves");
    LoopRunContext::new(scope, TurnId::new(), TurnRunId::new(), resolved)
}

fn test_parts(
    run_context: LoopRunContext,
    runtime: Arc<StubHostRuntime>,
    shared_io: Arc<SharedStubCapabilityIo>,
    capability_id_filter: Option<HashSet<CapabilityId>>,
    capability_execution_mount_overrides: HashMap<CapabilityId, ironclaw_host_api::MountView>,
    additional_provider_trust: BTreeMap<ExtensionId, ironclaw_trust::TrustDecision>,
) -> RefreshingCapabilityPortTestParts {
    RefreshingCapabilityPortTestParts {
        runtime,
        run_context,
        fallback_user_id: UserId::new("user-stub").expect("user id"),
        workspace_mounts: ironclaw_host_api::MountView::default(),
        skill_mounts: ironclaw_host_api::MountView::default(),
        memory_mounts: ironclaw_host_api::MountView::default(),
        system_extensions_lifecycle_mounts: ironclaw_host_api::MountView::default(),
        input_resolver: shared_io.clone(),
        result_writer: shared_io,
        milestone_sink: Arc::new(InMemoryLoopHostMilestoneSink::default()),
        skill_activation_source: None,
        project_service: Arc::new(StubProjectService),
        thread_service: Arc::new(ironclaw_threads::InMemorySessionThreadService::default()),
        trajectory_observer: None,
        outbound_preferences_facade: None,
        outbound_delivery_target_set_requires_approval: false,
        tool_permission_overrides: Arc::new(in_memory_backed_capability_permission_override_store()),
        auto_approve_settings: Arc::new(in_memory_backed_auto_approve_setting_store()),
        persistent_approval_policies: Arc::new(in_memory_backed_persistent_approval_policy_store()),
        approval_requests: Arc::new(ironclaw_run_state::in_memory_backed_approval_request_store()),
        capability_leases: Arc::new(in_memory_backed_capability_lease_store()),
        capability_execution_mount_overrides,
        additional_provider_trust,
        capability_id_filter,
        // Extension-lane seams (harness-port-seam P1 Change 3): None/empty =
        // the no-op surface this stub-runtime suite always ran with.
        extension_management: None,
        additional_capability_grants: Vec::new(),
    }
}

/// (i) the port builds and (ii) `tool_definitions()` includes the mandatory
/// `project_create` synthetic capability alongside the stub HostRuntime's
/// builtin-policy-derived capabilities.
#[tokio::test]
async fn port_builds_and_includes_synthetic_capabilities() {
    let shared_io = Arc::new(SharedStubCapabilityIo::new());
    let parts = test_parts(
        run_context("builds").await,
        Arc::new(StubHostRuntime::new()),
        shared_io,
        None,
        HashMap::new(),
        BTreeMap::new(),
    );
    let port = create_refreshing_capability_port_for_test(parts)
        .await
        .expect("port assembles through the real production factory");

    let definitions = port.tool_definitions().expect("tool definitions");
    assert!(
        definitions
            .iter()
            .any(|definition| definition.capability_id.as_str() == PROJECT_CREATE_CAPABILITY_ID),
        "synthetic project_create capability must be present: {definitions:?}"
    );
    assert!(
        definitions
            .iter()
            .any(|definition| definition.capability_id.as_str() == "builtin.echo"),
        "stub host-runtime builtin capability must be present: {definitions:?}"
    );
}

/// (iii) `capability_id_filter` narrows the FULL granted-capability set
/// (builtin AND any activated extension grants -- see the config field's
/// doc-comment), not a builtin-only subset. Synthetic capabilities are
/// unaffected, since they bypass the filtered grants entirely.
///
/// This test only exercises builtin grants because there is no cheap way to
/// inject an extension-grant fixture from this external integration-test
/// crate: `ExtensionCapabilitySurfaceSource`'s test constructors
/// (`from_surface`/`from_active_capabilities`) are `#[cfg(test)]`-gated to
/// the crate's own unit tests, and `RefreshingCapabilityPortTestParts`
/// does not expose `extension_surface_source` at all -- the test-support
/// constructor always wires `ExtensionCapabilitySurfaceSource::new(None)`
/// (empty extension surface). The whole-set semantic is pinned by the
/// doc-comment on `RefreshingCapabilityPortConfig::capability_id_filter`
/// and the inline comment at its `retain` call site in `build_inner`.
#[tokio::test]
async fn capability_id_filter_narrows_visible_surface() {
    let shared_io = Arc::new(SharedStubCapabilityIo::new());
    let mut filter = HashSet::new();
    filter.insert(CapabilityId::new("builtin.echo").expect("capability id"));
    let parts = test_parts(
        run_context("filter").await,
        Arc::new(StubHostRuntime::new()),
        shared_io,
        Some(filter),
        HashMap::new(),
        BTreeMap::new(),
    );
    let port = create_refreshing_capability_port_for_test(parts)
        .await
        .expect("port assembles");

    let definitions = port.tool_definitions().expect("tool definitions");
    let builtin_ids: Vec<&str> = definitions
        .iter()
        .map(|definition| definition.capability_id.as_str())
        .filter(|id| {
            id.starts_with("builtin.")
                && *id != PROJECT_CREATE_CAPABILITY_ID
                && *id != RESULT_READ_CAPABILITY_ID
        })
        .collect();
    assert_eq!(
        builtin_ids,
        vec!["builtin.echo"],
        "only the filtered-in builtin capability should remain: {definitions:?}"
    );
}

/// `Some(empty set)` is a real, distinct narrowing (zero granted builtin
/// capabilities) from `None` (no filtering at all) -- the tri-state fix's
/// core invariant. Synthetic capabilities (`project_create`) still bypass
/// the filter entirely, since they wrap the port directly.
#[tokio::test]
async fn capability_id_filter_some_empty_grants_zero_capabilities() {
    let shared_io = Arc::new(SharedStubCapabilityIo::new());
    let parts = test_parts(
        run_context("empty-filter").await,
        Arc::new(StubHostRuntime::new()),
        shared_io,
        Some(HashSet::new()),
        HashMap::new(),
        BTreeMap::new(),
    );
    let port = create_refreshing_capability_port_for_test(parts)
        .await
        .expect("port assembles");

    let definitions = port.tool_definitions().expect("tool definitions");
    let builtin_ids: Vec<&str> = definitions
        .iter()
        .map(|definition| definition.capability_id.as_str())
        .filter(|id| {
            id.starts_with("builtin.")
                && *id != PROJECT_CREATE_CAPABILITY_ID
                && *id != RESULT_READ_CAPABILITY_ID
        })
        .collect();
    assert!(
        builtin_ids.is_empty(),
        "Some(empty) must grant zero builtin capabilities: {definitions:?}"
    );
    assert!(
        definitions
            .iter()
            .any(|definition| definition.capability_id.as_str() == PROJECT_CREATE_CAPABILITY_ID),
        "synthetic project_create capability bypasses the filter entirely: {definitions:?}"
    );
    assert!(
        definitions
            .iter()
            .any(|definition| definition.capability_id.as_str() == RESULT_READ_CAPABILITY_ID),
        "synthetic result_read capability bypasses the filter entirely (issue #5838): {definitions:?}"
    );
}

/// (iv) input-ref/result-ref correlate through ONE shared io: register a
/// provider tool call for `project_create`, invoke it, and confirm the
/// result staged under the returned `result_ref` is readable back through
/// the SAME `SharedStubCapabilityIo` the config's `input_resolver` and
/// `result_writer` both point at.
#[tokio::test]
async fn input_and_result_refs_correlate_through_one_shared_io() {
    let shared_io = Arc::new(SharedStubCapabilityIo::new());
    let parts = test_parts(
        run_context("io").await,
        Arc::new(StubHostRuntime::new()),
        shared_io.clone(),
        None,
        HashMap::new(),
        BTreeMap::new(),
    );
    let port = create_refreshing_capability_port_for_test(parts)
        .await
        .expect("port assembles");

    let tool_call = ProviderToolCall {
        provider_id: "stub-provider".to_string(),
        provider_model_id: "stub-model".to_string(),
        turn_id: Some("turn-1".to_string()),
        id: "call-1".to_string(),
        name: ProviderToolName::new("builtin__project_create").expect("tool name"),
        arguments: serde_json::json!({"name": "My Project"}),
        response_reasoning: None,
        reasoning: None,
        signature: None,
    };
    let candidate = port
        .register_provider_tool_call(RegisterProviderToolCallRequest {
            tool_call,
            activity_id: None,
        })
        .await
        .expect("registers the project_create provider tool call");

    // The input the synthetic capability will read back is staged under
    // `candidate.input_ref` in `shared_io` -- resolve it directly to confirm
    // the SAME io object served the registration.
    let staged_input = shared_io
        .inputs
        .lock()
        .expect("inputs lock")
        .get(candidate.input_ref.as_str())
        .cloned();
    assert_eq!(
        staged_input,
        Some(serde_json::json!({"name": "My Project"}))
    );

    let outcome = port
        .invoke_capability(CapabilityInvocation {
            activity_id: candidate.activity_id,
            surface_version: candidate.surface_version,
            capability_id: candidate.capability_id,
            input_ref: candidate.input_ref,
            approval_resume: None,
            auth_resume: None,
        })
        .await
        .expect("invokes project_create");

    let CapabilityOutcome::Completed(message) = outcome else {
        panic!("expected a completed outcome, got {outcome:?}");
    };
    let staged_result = shared_io.staged_result(message.result_ref.as_str());
    assert!(
        staged_result.is_some(),
        "result_ref must resolve through the SAME shared io the input was staged in"
    );
}

fn mount_view(alias: &str, target: &str) -> MountView {
    MountView::new(vec![MountGrant::new(
        MountAlias::new(alias).expect("valid mount alias"),
        VirtualPath::new(target).expect("valid virtual path"),
        MountPermissions::read_write_list_delete(),
    )])
    .expect("valid mount view")
}

/// (v) `capability_execution_mount_overrides` (the review-flagged FIX C
/// coverage gap): a non-empty override for `builtin.echo` must be the mount
/// view the STUB host runtime actually sees on `ExecutionContext.mounts` when
/// that capability is invoked -- not just baked into port construction.
#[tokio::test]
async fn capability_execution_mount_overrides_reach_invocation_context() {
    let shared_io = Arc::new(SharedStubCapabilityIo::new());
    let runtime = Arc::new(StubHostRuntime::new());
    let echo_id = CapabilityId::new("builtin.echo").expect("capability id");
    let override_mounts = mount_view("/override-alias", "/projects/override-target");
    let mut overrides = HashMap::new();
    overrides.insert(echo_id.clone(), override_mounts.clone());
    let parts = test_parts(
        run_context("mount-override").await,
        runtime.clone(),
        shared_io,
        None,
        overrides,
        BTreeMap::new(),
    );
    let port = create_refreshing_capability_port_for_test(parts)
        .await
        .expect("port assembles");

    let tool_call = ProviderToolCall {
        provider_id: "stub-provider".to_string(),
        provider_model_id: "stub-model".to_string(),
        turn_id: Some("turn-1".to_string()),
        id: "call-1".to_string(),
        name: ProviderToolName::new("builtin__echo").expect("tool name"),
        arguments: serde_json::json!({}),
        response_reasoning: None,
        reasoning: None,
        signature: None,
    };
    let candidate = port
        .register_provider_tool_call(RegisterProviderToolCallRequest {
            tool_call,
            activity_id: None,
        })
        .await
        .expect("registers the builtin.echo provider tool call");

    port.invoke_capability(CapabilityInvocation {
        activity_id: candidate.activity_id,
        surface_version: candidate.surface_version,
        capability_id: candidate.capability_id,
        input_ref: candidate.input_ref,
        approval_resume: None,
        auth_resume: None,
    })
    .await
    .expect("invokes builtin.echo");

    let invocations = runtime.invocation_contexts();
    assert_eq!(invocations.len(), 1, "expected exactly one invocation");
    assert_eq!(
        invocations[0].mounts, override_mounts,
        "the override mount, not the default, must reach the invocation ExecutionContext"
    );
}

fn distinguishing_trust_decision() -> ironclaw_trust::TrustDecision {
    ironclaw_trust::TrustDecision {
        effective_trust: ironclaw_trust::EffectiveTrustClass::user_trusted(),
        authority_ceiling: ironclaw_trust::AuthorityCeiling {
            allowed_effects: vec![EffectKind::ReadFilesystem],
            max_resource_ceiling: None,
        },
        // `Bundled` is never produced by `visible_capability_request`
        // itself (which only ever inserts `AdminConfig`), so seeing it back
        // out proves the test's entry -- not the canonical helper's -- won.
        provenance: ironclaw_trust::TrustProvenance::Bundled,
        evaluated_at: chrono::Utc::now(),
    }
}

/// (vi) `additional_provider_trust` (the review-flagged FIX D coverage gap):
/// a non-empty entry must be observable in the `VisibleCapabilityRequest`
/// the STUB host runtime receives, and -- since `build_inner` merges it in
/// AFTER the canonical helper's base map -- an entry keyed by the same
/// provider the canonical helper already populates (`builtin`) must
/// overwrite the base value.
#[tokio::test]
async fn additional_provider_trust_is_forwarded_to_visible_request() {
    let shared_io = Arc::new(SharedStubCapabilityIo::new());
    let runtime = Arc::new(StubHostRuntime::new());
    let builtin_provider = ExtensionId::new("builtin").expect("provider id");
    let override_trust = distinguishing_trust_decision();
    let mut additional_provider_trust = BTreeMap::new();
    additional_provider_trust.insert(builtin_provider.clone(), override_trust.clone());
    let parts = test_parts(
        run_context("provider-trust").await,
        runtime.clone(),
        shared_io,
        None,
        HashMap::new(),
        additional_provider_trust,
    );
    let port = create_refreshing_capability_port_for_test(parts)
        .await
        .expect("port assembles");
    // Construction already triggers one `visible_capabilities` refresh; drive
    // a second one explicitly through the public port API so the assertion
    // exercises the same seam a real loop run would.
    port.visible_capabilities(ironclaw_turns::run_profile::VisibleCapabilityRequest)
        .await
        .expect("visible capabilities refresh");

    let observed = runtime.visible_provider_trust();
    assert!(
        !observed.is_empty(),
        "host runtime must have observed at least one visible-capabilities request"
    );
    for provider_trust in &observed {
        assert_eq!(
            provider_trust.get(&builtin_provider),
            Some(&override_trust),
            "additional_provider_trust must overwrite the canonical helper's base \
             `builtin` provider-trust entry, not merge alongside it"
        );
    }
}

/// Review follow-up: pins the "many" edge of all three collection-valued
/// config knobs together in one port build, since the five tests above each
/// exercise a single entry. `capability_id_filter` must keep BOTH listed ids
/// (dropping the rest); `capability_execution_mount_overrides` must resolve
/// EACH capability's invocation against its OWN mount, not the other's or the
/// default; `additional_provider_trust` must land BOTH entries in the
/// observed provider-trust map. See PR #5950 review.
#[tokio::test]
async fn multi_entry_collection_knobs_round_trip() {
    let shared_io = Arc::new(SharedStubCapabilityIo::new());
    let runtime = Arc::new(StubHostRuntime::new());

    let echo_id = CapabilityId::new("builtin.echo").expect("capability id");
    let time_id = CapabilityId::new("builtin.time").expect("capability id");
    let mut filter = HashSet::new();
    filter.insert(echo_id.clone());
    filter.insert(time_id.clone());

    let echo_mounts = mount_view("/echo-alias", "/projects/echo-target");
    let time_mounts = mount_view("/time-alias", "/projects/time-target");
    let mut overrides = HashMap::new();
    overrides.insert(echo_id.clone(), echo_mounts.clone());
    overrides.insert(time_id.clone(), time_mounts.clone());

    let builtin_provider = ExtensionId::new("builtin").expect("provider id");
    let other_provider = ExtensionId::new("other-provider").expect("provider id");
    // `builtin_trust` overrides the "builtin" provider used to invoke both
    // capabilities below, so its ceiling must cover both grants' effects
    // (`dispatch_capability`/`read_filesystem`/`write_filesystem` per
    // builtin_capability_policy.toml) or invocation is denied as untrusted.
    let mut builtin_trust = distinguishing_trust_decision();
    builtin_trust.authority_ceiling.allowed_effects = vec![
        EffectKind::DispatchCapability,
        EffectKind::ReadFilesystem,
        EffectKind::WriteFilesystem,
    ];
    let other_trust = distinguishing_trust_decision();
    let mut additional_provider_trust = BTreeMap::new();
    additional_provider_trust.insert(builtin_provider.clone(), builtin_trust.clone());
    additional_provider_trust.insert(other_provider.clone(), other_trust.clone());

    let parts = test_parts(
        run_context("multi-entry").await,
        runtime.clone(),
        shared_io,
        Some(filter),
        overrides,
        additional_provider_trust,
    );
    let port = create_refreshing_capability_port_for_test(parts)
        .await
        .expect("port assembles");

    // capability_id_filter: both listed builtin ids survive, everything else
    // (e.g. builtin.shell, builtin.read_file, ...) is dropped.
    let definitions = port.tool_definitions().expect("tool definitions");
    let mut builtin_ids: Vec<&str> = definitions
        .iter()
        .map(|definition| definition.capability_id.as_str())
        .filter(|id| {
            id.starts_with("builtin.")
                && *id != PROJECT_CREATE_CAPABILITY_ID
                && *id != RESULT_READ_CAPABILITY_ID
        })
        .collect();
    builtin_ids.sort_unstable();
    assert_eq!(
        builtin_ids,
        vec!["builtin.echo", "builtin.time"],
        "both filtered-in builtin capabilities should remain, and no others: {definitions:?}"
    );

    // capability_execution_mount_overrides: invoke both and check each one's
    // ExecutionContext carries ITS OWN override mount, not the other's.
    async fn invoke(
        port: &dyn ironclaw_turns::run_profile::LoopCapabilityPort,
        provider_tool_name: &str,
    ) {
        let tool_call = ProviderToolCall {
            provider_id: "stub-provider".to_string(),
            provider_model_id: "stub-model".to_string(),
            turn_id: Some("turn-1".to_string()),
            id: format!("call-{provider_tool_name}"),
            name: ProviderToolName::new(provider_tool_name).expect("tool name"),
            arguments: serde_json::json!({}),
            response_reasoning: None,
            reasoning: None,
            signature: None,
        };
        let candidate = port
            .register_provider_tool_call(RegisterProviderToolCallRequest {
                tool_call,
                activity_id: None,
            })
            .await
            .expect("registers the provider tool call");
        port.invoke_capability(CapabilityInvocation {
            activity_id: candidate.activity_id,
            surface_version: candidate.surface_version,
            capability_id: candidate.capability_id,
            input_ref: candidate.input_ref,
            approval_resume: None,
            auth_resume: None,
        })
        .await
        .expect("invokes capability");
    }
    invoke(port.as_ref(), "builtin__echo").await;
    invoke(port.as_ref(), "builtin__time").await;

    let invocations = runtime.invocation_contexts();
    assert_eq!(invocations.len(), 2, "expected exactly two invocations");
    assert_eq!(
        invocations[0].mounts, echo_mounts,
        "builtin.echo must resolve against its own override mount"
    );
    assert_eq!(
        invocations[1].mounts, time_mounts,
        "builtin.time must resolve against its own override mount, not builtin.echo's"
    );

    // additional_provider_trust: both entries land in the observed
    // visible_capabilities provider-trust map.
    port.visible_capabilities(ironclaw_turns::run_profile::VisibleCapabilityRequest)
        .await
        .expect("visible capabilities refresh");
    let observed = runtime.visible_provider_trust();
    let last = observed
        .last()
        .expect("host runtime must have observed at least one visible-capabilities request");
    assert_eq!(last.get(&builtin_provider), Some(&builtin_trust));
    assert_eq!(last.get(&other_provider), Some(&other_trust));
}
