//! Crate-tier RED-then-GREEN coverage for the harness-port-seam P1 Change 1
//! test-support constructor: `test_support::create_refreshing_local_dev_capability_port_for_test`
//! must drive the REAL production capability-port factory
//! (`create_refreshing_local_dev_capability_port`) with in-crate stub
//! doubles, not a hand-rebuilt wrap order.
//!
//! Stubs here are intentionally minimal: a `HostRuntime` that echoes back a
//! `VisibleCapability` per granted capability id (so `capability_id_filter`
//! narrowing is observable), and a shared input/result io double standing in
//! for production's `LocalDevCapabilityIo` (one object, both roles).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use ironclaw_approvals::{
    InMemoryAutoApproveSettingStore, InMemoryCapabilityPermissionOverrideStore,
    InMemoryPersistentApprovalPolicyStore,
};
use ironclaw_authorization::InMemoryCapabilityLeaseStore;
use ironclaw_host_api::{
    AgentId, CapabilityDescriptor, CapabilityId, EffectKind, ExtensionId, PermissionMode,
    ProjectId, ProviderToolName, ResourceEstimate, RuntimeKind, TenantId, ThreadId, UserId,
};
use ironclaw_host_runtime::RuntimeCapabilityOutcome;
use ironclaw_host_runtime::{
    CancelRuntimeWorkOutcome, CancelRuntimeWorkRequest, CapabilitySurfaceVersion, HostRuntime,
    HostRuntimeError, HostRuntimeHealth, HostRuntimeStatus, RuntimeCapabilityRequest,
    RuntimeCapabilityResumeRequest, RuntimeStatusRequest, VisibleCapability,
    VisibleCapabilityAccess, VisibleCapabilityRequest, VisibleCapabilitySurface,
};
use ironclaw_loop_support::{
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
    RefreshingLocalDevCapabilityPortTestParts, create_refreshing_local_dev_capability_port_for_test,
};
use ironclaw_run_state::InMemoryApprovalRequestStore;
use ironclaw_turns::run_profile::{
    AgentLoopHostError, AgentLoopHostErrorKind, CapabilityInputRef, CapabilityInvocation,
    CapabilityOutcome, InMemoryLoopHostMilestoneSink, InMemoryRunProfileResolver, LoopRunContext,
    ProviderToolCall, RegisterProviderToolCallRequest, RunProfileResolutionRequest,
};
use ironclaw_turns::{RunProfileResolver, TurnId, TurnRunId, TurnScope};

/// Echoes a runtime-visible capability for every grant in the request's
/// context, so `capability_id_filter` narrowing (applied upstream of this
/// stub, in `build_inner`) is directly observable in `tool_definitions()`.
struct StubHostRuntime;

#[async_trait]
impl HostRuntime for StubHostRuntime {
    async fn invoke_capability(
        &self,
        _request: RuntimeCapabilityRequest,
    ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
        Err(HostRuntimeError::unavailable(
            "stub host runtime does not execute capabilities",
        ))
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

/// Stands in for production's `LocalDevCapabilityIo`: ONE object implementing
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
    shared_io: Arc<SharedStubCapabilityIo>,
    capability_id_filter: HashSet<CapabilityId>,
) -> RefreshingLocalDevCapabilityPortTestParts {
    RefreshingLocalDevCapabilityPortTestParts {
        runtime: Arc::new(StubHostRuntime),
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
        trajectory_observer: None,
        outbound_preferences_facade: None,
        outbound_delivery_target_set_requires_approval: false,
        tool_permission_overrides: Arc::new(InMemoryCapabilityPermissionOverrideStore::new()),
        auto_approve_settings: Arc::new(InMemoryAutoApproveSettingStore::new()),
        persistent_approval_policies: Arc::new(InMemoryPersistentApprovalPolicyStore::new()),
        approval_requests: Arc::new(InMemoryApprovalRequestStore::new()),
        capability_leases: Arc::new(InMemoryCapabilityLeaseStore::new()),
        capability_execution_mount_overrides: HashMap::new(),
        additional_provider_trust: BTreeMap::new(),
        capability_id_filter,
    }
}

/// (i) the port builds and (ii) `tool_definitions()` includes the mandatory
/// `project_create` synthetic capability alongside the stub HostRuntime's
/// builtin-policy-derived capabilities.
#[tokio::test]
async fn port_builds_and_includes_synthetic_capabilities() {
    let shared_io = Arc::new(SharedStubCapabilityIo::new());
    let parts = test_parts(run_context("builds").await, shared_io, HashSet::new());
    let port = create_refreshing_local_dev_capability_port_for_test(parts)
        .await
        .expect("port assembles through the real production factory");

    let definitions = port.tool_definitions().expect("tool definitions");
    assert!(
        definitions
            .iter()
            .any(|definition| definition.capability_id.as_str() == "builtin.project_create"),
        "synthetic project_create capability must be present: {definitions:?}"
    );
    assert!(
        definitions
            .iter()
            .any(|definition| definition.capability_id.as_str() == "builtin.echo"),
        "stub host-runtime builtin capability must be present: {definitions:?}"
    );
}

/// (iii) `capability_id_filter` narrows the builtin-policy-derived surface
/// (synthetic capabilities are unaffected, since they bypass the filtered
/// grants entirely).
#[tokio::test]
async fn capability_id_filter_narrows_visible_surface() {
    let shared_io = Arc::new(SharedStubCapabilityIo::new());
    let mut filter = HashSet::new();
    filter.insert(CapabilityId::new("builtin.echo").expect("capability id"));
    let parts = test_parts(run_context("filter").await, shared_io, filter);
    let port = create_refreshing_local_dev_capability_port_for_test(parts)
        .await
        .expect("port assembles");

    let definitions = port.tool_definitions().expect("tool definitions");
    let builtin_ids: Vec<&str> = definitions
        .iter()
        .map(|definition| definition.capability_id.as_str())
        .filter(|id| id.starts_with("builtin.") && *id != "builtin.project_create")
        .collect();
    assert_eq!(
        builtin_ids,
        vec!["builtin.echo"],
        "only the filtered-in builtin capability should remain: {definitions:?}"
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
    let parts = test_parts(run_context("io").await, shared_io.clone(), HashSet::new());
    let port = create_refreshing_local_dev_capability_port_for_test(parts)
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
