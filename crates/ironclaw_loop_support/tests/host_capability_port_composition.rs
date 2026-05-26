use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use ironclaw_host_api::{
    CapabilityDescriptor, CapabilityId, CapabilitySet, ExecutionContext, ExtensionId, InvocationId,
    MountAlias, MountGrant, MountPermissions, MountView, PermissionMode, ResourceEstimate,
    ResourceUsage, RuntimeKind, ThreadId, TrustClass, UserId, VirtualPath,
};
use ironclaw_host_runtime::{
    CancelRuntimeWorkOutcome, CancelRuntimeWorkRequest, CapabilitySurfaceVersion, HostRuntime,
    HostRuntimeError, HostRuntimeHealth, HostRuntimeStatus, RuntimeCapabilityCompleted,
    RuntimeCapabilityOutcome, RuntimeCapabilityRequest, RuntimeCapabilityResumeRequest,
    RuntimeStatusRequest, SurfaceKind, VisibleCapability, VisibleCapabilityAccess,
    VisibleCapabilityRequest as HostVisibleCapabilityRequest,
    VisibleCapabilitySurface as HostVisibleCapabilitySurface,
};
use ironclaw_loop_support::{
    HostRuntimeLoopCapabilityPortFactory, LoopCapabilityInputResolver, LoopCapabilityResultWriter,
};
use ironclaw_trust::{AuthorityCeiling, EffectiveTrustClass, TrustDecision, TrustProvenance};
use ironclaw_turns::{
    InMemoryRunProfileResolver, LoopResultRef, RunProfileResolutionRequest, RunProfileResolver,
    TurnId, TurnRunId, TurnScope,
    run_profile::{
        AgentLoopHostError, AgentLoopHostErrorKind, CapabilityInputRef, CapabilityInvocation,
        InMemoryLoopHostMilestoneSink, LoopCapabilityPort, LoopRunContext, ProviderToolCall,
        VisibleCapabilityRequest,
    },
};

#[test]
fn host_capability_port_composition_rejects_external_raw_construction() {
    let workspace_root = workspace_root();
    let mut offenders = Vec::new();
    visit_rs_files(&workspace_root, &mut |path| {
        if should_skip(path) {
            return;
        }
        let src = std::fs::read_to_string(path).unwrap_or_default();
        if src.contains("HostRuntimeLoopCapabilityPort::new(")
            || src.contains("HostRuntimeLoopCapabilityPort {")
        {
            offenders.push(path.display().to_string());
        }
    });

    assert!(
        offenders.is_empty(),
        "HostRuntimeLoopCapabilityPort must be constructed only inside ironclaw_loop_support; offenders: {offenders:#?}"
    );
}

#[tokio::test]
async fn host_capability_port_composition_factory_builds_loop_capability_port() {
    let thread_id = ThreadId::new("thread-loop-support-factory").unwrap();
    let mut context = ExecutionContext::local_default(
        UserId::new("user-loop-support-factory").unwrap(),
        ExtensionId::new("loop-support-factory").unwrap(),
        RuntimeKind::Wasm,
        TrustClass::UserTrusted,
        CapabilitySet::default(),
        MountView::default(),
    )
    .unwrap();
    context.thread_id = Some(thread_id.clone());
    context.resource_scope.thread_id = Some(thread_id.clone());
    let run_context = loop_run_context(&context, thread_id).await;
    let visible_request =
        HostVisibleCapabilityRequest::new(context, SurfaceKind::new("agent_loop").unwrap());

    let factory = HostRuntimeLoopCapabilityPortFactory::new(
        Arc::new(EmptyHostRuntime),
        visible_request,
        Arc::new(UnusedInputResolver),
        Arc::new(UnusedResultWriter),
        Arc::new(InMemoryLoopHostMilestoneSink::default()),
    );
    let port: Arc<dyn LoopCapabilityPort> = factory.for_run_context(run_context);

    let surface = port
        .visible_capabilities(VisibleCapabilityRequest)
        .await
        .unwrap();

    assert_eq!(surface.version.as_str(), "factory-empty:v1");
    assert_eq!(surface.descriptors.len(), 1);
    assert_eq!(
        surface.descriptors[0].capability_id.as_str(),
        "ironclaw.loop.capability_info"
    );
}

#[tokio::test]
async fn visible_capability_request_rejects_caller_supplied_mounts() {
    let thread_id = ThreadId::new("thread-caller-mount-rejection").unwrap();
    let mut context = ExecutionContext::local_default(
        UserId::new("user-caller-mount-rejection").unwrap(),
        ExtensionId::new("loop-support-mounts").unwrap(),
        RuntimeKind::Wasm,
        TrustClass::UserTrusted,
        CapabilitySet::default(),
        MountView::new(vec![MountGrant::new(
            MountAlias::new("/workspace").unwrap(),
            VirtualPath::new("/projects/demo").unwrap(),
            MountPermissions::read_write(),
        )])
        .unwrap(),
    )
    .unwrap();
    context.thread_id = Some(thread_id.clone());
    context.resource_scope.thread_id = Some(thread_id.clone());
    let run_context = loop_run_context(&context, thread_id).await;
    let visible_request =
        HostVisibleCapabilityRequest::new(context, SurfaceKind::new("agent_loop").unwrap());

    let factory = HostRuntimeLoopCapabilityPortFactory::new(
        Arc::new(EmptyHostRuntime),
        visible_request,
        Arc::new(UnusedInputResolver),
        Arc::new(UnusedResultWriter),
        Arc::new(InMemoryLoopHostMilestoneSink::default()),
    );
    let port: Arc<dyn LoopCapabilityPort> = factory.for_run_context(run_context);

    let err = port
        .visible_capabilities(VisibleCapabilityRequest)
        .await
        .expect_err("caller-supplied mounts must be rejected");

    assert_eq!(
        err.kind,
        AgentLoopHostErrorKind::Unauthorized,
        "expected Unauthorized for caller-supplied mounts, got {:?}",
        err.kind
    );
}

#[tokio::test]
async fn factory_stages_provider_tool_call_arguments_without_custom_resolver_override() {
    let thread_id = ThreadId::new("thread-provider-tool-input").unwrap();
    let mut context = ExecutionContext::local_default(
        UserId::new("user-provider-tool-input").unwrap(),
        ExtensionId::new("loop-support-provider-tool-input").unwrap(),
        RuntimeKind::Wasm,
        TrustClass::UserTrusted,
        CapabilitySet::default(),
        MountView::default(),
    )
    .unwrap();
    context.thread_id = Some(thread_id.clone());
    context.resource_scope.thread_id = Some(thread_id.clone());
    let run_context = loop_run_context(&context, thread_id).await;
    let visible_request =
        HostVisibleCapabilityRequest::new(context, SurfaceKind::new("agent_loop").unwrap())
            .with_provider_trust(BTreeMap::from([(
                ExtensionId::new("demo").unwrap(),
                dispatch_trust_decision(),
            )]));

    let runtime = Arc::new(SingleToolHostRuntime::default());
    let factory = HostRuntimeLoopCapabilityPortFactory::new(
        runtime.clone(),
        visible_request,
        Arc::new(UnusedInputResolver),
        Arc::new(UnusedResultWriter),
        Arc::new(InMemoryLoopHostMilestoneSink::default()),
    );
    let port: Arc<dyn LoopCapabilityPort> = factory.for_run_context(run_context);

    port.visible_capabilities(VisibleCapabilityRequest)
        .await
        .expect("surface should snapshot provider tools");
    let arguments = serde_json::json!({
        "message": "hello\nfrom provider\r\n\twith tab"
    });
    let candidate = port
        .register_provider_tool_call(ProviderToolCall {
            provider_id: "provider".to_string(),
            provider_model_id: "model".to_string(),
            turn_id: Some("turn_1".to_string()),
            id: "call_1".to_string(),
            name: "demo__echo".to_string(),
            arguments: arguments.clone(),
            response_reasoning: None,
            reasoning: None,
            signature: None,
        })
        .await
        .expect("provider tool call should stage input");

    assert_eq!(
        candidate.capability_id,
        CapabilityId::new("demo.echo").unwrap()
    );
    assert!(
        candidate
            .input_ref
            .as_str()
            .starts_with("input:provider-tool-")
    );
    assert_eq!(
        candidate
            .provider_replay
            .clone()
            .expect("provider replay")
            .arguments,
        arguments
    );
    let outcome = port
        .invoke_capability(CapabilityInvocation {
            surface_version: candidate.surface_version,
            capability_id: candidate.capability_id,
            input_ref: candidate.input_ref,
        })
        .await
        .expect("staged provider input should invoke");
    assert!(
        matches!(
            outcome,
            ironclaw_turns::run_profile::CapabilityOutcome::Completed(_)
        ),
        "expected completed provider invocation, got {outcome:?}"
    );
    assert_eq!(runtime.take_requests()[0].input, arguments);
}

fn workspace_root() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .expect("crate lives under workspace crates directory")
}

async fn loop_run_context(context: &ExecutionContext, thread_id: ThreadId) -> LoopRunContext {
    let resolved = InMemoryRunProfileResolver::default()
        .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
        .await
        .unwrap();
    LoopRunContext::new(
        TurnScope::new(
            context.tenant_id.clone(),
            context.agent_id.clone(),
            context.project_id.clone(),
            thread_id,
        ),
        TurnId::new(),
        TurnRunId::new(),
        resolved,
    )
}

struct EmptyHostRuntime;

#[async_trait]
impl HostRuntime for EmptyHostRuntime {
    async fn invoke_capability(
        &self,
        _request: RuntimeCapabilityRequest,
    ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
        Err(HostRuntimeError::unavailable("not used in this test"))
    }

    async fn resume_capability(
        &self,
        _request: RuntimeCapabilityResumeRequest,
    ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
        Err(HostRuntimeError::unavailable("not used in this test"))
    }

    async fn visible_capabilities(
        &self,
        _request: HostVisibleCapabilityRequest,
    ) -> Result<HostVisibleCapabilitySurface, HostRuntimeError> {
        Ok(HostVisibleCapabilitySurface {
            version: CapabilitySurfaceVersion::new("factory-empty:v1").unwrap(),
            capabilities: Vec::new(),
        })
    }

    async fn cancel_work(
        &self,
        _request: CancelRuntimeWorkRequest,
    ) -> Result<CancelRuntimeWorkOutcome, HostRuntimeError> {
        Ok(CancelRuntimeWorkOutcome::default())
    }

    async fn runtime_status(
        &self,
        _request: RuntimeStatusRequest,
    ) -> Result<HostRuntimeStatus, HostRuntimeError> {
        Ok(HostRuntimeStatus::default())
    }

    async fn health(&self) -> Result<HostRuntimeHealth, HostRuntimeError> {
        Ok(HostRuntimeHealth::default())
    }
}

#[derive(Default)]
struct SingleToolHostRuntime {
    requests: Mutex<Vec<RuntimeCapabilityRequest>>,
}

impl SingleToolHostRuntime {
    fn take_requests(&self) -> Vec<RuntimeCapabilityRequest> {
        self.requests.lock().expect("requests lock").clone()
    }
}

#[async_trait]
impl HostRuntime for SingleToolHostRuntime {
    async fn invoke_capability(
        &self,
        request: RuntimeCapabilityRequest,
    ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
        self.requests
            .lock()
            .expect("requests lock")
            .push(request.clone());
        Ok(RuntimeCapabilityOutcome::Completed(Box::new(
            RuntimeCapabilityCompleted {
                capability_id: request.capability_id,
                output: serde_json::json!({"ok": true}),
                usage: ResourceUsage::default(),
            },
        )))
    }

    async fn resume_capability(
        &self,
        _request: RuntimeCapabilityResumeRequest,
    ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
        Err(HostRuntimeError::unavailable("not used in this test"))
    }

    async fn visible_capabilities(
        &self,
        _request: HostVisibleCapabilityRequest,
    ) -> Result<HostVisibleCapabilitySurface, HostRuntimeError> {
        Ok(HostVisibleCapabilitySurface {
            version: CapabilitySurfaceVersion::new("factory-single-tool:v1").unwrap(),
            capabilities: vec![VisibleCapability {
                descriptor: CapabilityDescriptor {
                    id: CapabilityId::new("demo.echo").unwrap(),
                    provider: ExtensionId::new("demo").unwrap(),
                    runtime: RuntimeKind::Wasm,
                    trust_ceiling: TrustClass::UserTrusted,
                    description: "Echo input".to_string(),
                    parameters_schema: serde_json::json!({
                        "type": "object",
                        "properties": {
                            "message": { "type": "string" }
                        }
                    }),
                    effects: Vec::new(),
                    default_permission: PermissionMode::Allow,
                    runtime_credentials: Vec::new(),
                    resource_profile: None,
                },
                access: VisibleCapabilityAccess::Available,
                estimated_resources: ResourceEstimate::default(),
            }],
        })
    }

    async fn cancel_work(
        &self,
        _request: CancelRuntimeWorkRequest,
    ) -> Result<CancelRuntimeWorkOutcome, HostRuntimeError> {
        Ok(CancelRuntimeWorkOutcome::default())
    }

    async fn runtime_status(
        &self,
        _request: RuntimeStatusRequest,
    ) -> Result<HostRuntimeStatus, HostRuntimeError> {
        Ok(HostRuntimeStatus::default())
    }

    async fn health(&self) -> Result<HostRuntimeHealth, HostRuntimeError> {
        Ok(HostRuntimeHealth::default())
    }
}

struct UnusedInputResolver;

#[async_trait]
impl LoopCapabilityInputResolver for UnusedInputResolver {
    async fn resolve_capability_input(
        &self,
        _run_context: &LoopRunContext,
        _input_ref: &ironclaw_turns::run_profile::CapabilityInputRef,
    ) -> Result<serde_json::Value, AgentLoopHostError> {
        Err(AgentLoopHostError::new(
            AgentLoopHostErrorKind::InvalidInvocation,
            "not used in this test",
        ))
    }
}

struct UnusedResultWriter;

#[async_trait]
impl LoopCapabilityResultWriter for UnusedResultWriter {
    async fn write_capability_result(
        &self,
        _run_context: &LoopRunContext,
        _input_ref: &CapabilityInputRef,
        _invocation_id: InvocationId,
        _capability_id: &CapabilityId,
        _output: serde_json::Value,
    ) -> Result<LoopResultRef, AgentLoopHostError> {
        LoopResultRef::new("result:factory").map_err(|_| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::Internal,
                "result ref could not be represented",
            )
        })
    }
}

fn dispatch_trust_decision() -> TrustDecision {
    TrustDecision {
        effective_trust: EffectiveTrustClass::user_trusted(),
        authority_ceiling: AuthorityCeiling {
            allowed_effects: vec![ironclaw_host_api::EffectKind::DispatchCapability],
            max_resource_ceiling: None,
        },
        provenance: TrustProvenance::Default,
        evaluated_at: chrono::Utc::now(),
    }
}

fn visit_rs_files(root: &Path, visit: &mut impl FnMut(&Path)) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            visit_rs_files(&path, visit);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            visit(&path);
        }
    }
}

fn should_skip(path: &Path) -> bool {
    path.components().any(|component| {
        let name = component.as_os_str();
        name == "ironclaw_loop_support" || name == "tests" || name == "target"
    })
}
