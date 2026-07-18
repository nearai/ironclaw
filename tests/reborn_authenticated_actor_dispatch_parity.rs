use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use ironclaw_authorization::GrantAuthorizer;
use ironclaw_extensions::ExtensionRegistry;
use ironclaw_filesystem::DiskFilesystem;
use ironclaw_host_api::{
    CapabilityGrant, CapabilityGrantId, CapabilityId, CapabilitySet, EffectKind, ExecutionContext,
    ExtensionId, GrantConstraints, MountView, NetworkPolicy, PackageId, PackageSource, Principal,
    ProviderToolName, ResourceScope, ResourceUsage, RuntimeKind, ThreadId, TrustClass, UserId,
};
use ironclaw_host_runtime::{
    BUILTIN_FIRST_PARTY_PROVIDER, CapabilitySurfacePolicy, CapabilitySurfaceVersion,
    ECHO_CAPABILITY_ID, FirstPartyCapabilityError, FirstPartyCapabilityHandler,
    FirstPartyCapabilityRegistry, FirstPartyCapabilityRequest, FirstPartyCapabilityResult,
    HostRuntimeServices, SurfaceKind, VisibleCapabilityRequest as HostVisibleCapabilityRequest,
    builtin_first_party_package,
};
use ironclaw_loop_host::{
    CapabilityResultWrite, CapabilityWriteResult, HostRuntimeLoopCapabilityPortFactory,
    LoopCapabilityInputResolver, LoopCapabilityResultWriter, loop_driver_execution_extension_id,
};
use ironclaw_processes::ProcessServices;
use ironclaw_resources::InMemoryResourceGovernor;
use ironclaw_trust::{AdminConfig, AdminEntry, HostTrustAssignment, HostTrustPolicy, TrustPolicy};
use ironclaw_turns::{
    InMemoryRunProfileResolver, LoopResultRef, RunProfileResolutionRequest, RunProfileResolver,
    TurnActor, TurnId, TurnRunId, TurnScope,
    run_profile::{
        AgentLoopHostError, AgentLoopHostErrorKind, CapabilityInvocation, CapabilityOutcome,
        InMemoryLoopHostMilestoneSink, LoopCapabilityPort, LoopRunContext, ProviderToolCall,
        RegisterProviderToolCallRequest, VisibleCapabilityRequest,
    },
};
use serde_json::json;

#[tokio::test]
async fn loop_run_dispatch_preserves_authenticated_actor_distinct_from_shared_subject() {
    let capability_id = CapabilityId::new(ECHO_CAPABILITY_ID).unwrap();
    let package = builtin_first_party_package().unwrap();
    let trust_policy = Arc::new(first_party_trust_policy());
    let provider_trust = trust_policy
        .evaluate(
            &package
                .trust_policy_input(
                    PackageSource::LocalManifest {
                        path: "/system/extensions/builtin/manifest.toml".to_string(),
                    },
                    package.manifest_digest(),
                    None,
                )
                .unwrap(),
        )
        .unwrap();
    let mut registry = ExtensionRegistry::new();
    registry.insert(package).unwrap();
    let recorded = Arc::new(Mutex::new(None));
    let handlers = FirstPartyCapabilityRegistry::new().with_handler(
        capability_id.clone(),
        Arc::new(RecordingActorHandler {
            recorded: Arc::clone(&recorded),
        }),
    );
    let runtime = Arc::new(
        HostRuntimeServices::new(
            Arc::new(registry),
            Arc::new(DiskFilesystem::new()),
            Arc::new(InMemoryResourceGovernor::new()),
            Arc::new(GrantAuthorizer::new()),
            ProcessServices::in_memory(),
            CapabilitySurfaceVersion::new("actor-parity-v1").unwrap(),
        )
        .with_first_party_capabilities(Arc::new(handlers))
        .with_trust_policy(trust_policy)
        .host_runtime_for_local_testing(),
    );

    let thread_id = ThreadId::new("thread-actor-dispatch-parity").unwrap();
    let mut context = ExecutionContext::local_default(
        UserId::new("shared-subject").unwrap(),
        ExtensionId::new("actor-parity-bootstrap").unwrap(),
        RuntimeKind::FirstParty,
        TrustClass::UserTrusted,
        CapabilitySet::default(),
        MountView::default(),
    )
    .unwrap();
    context.thread_id = Some(thread_id.clone());
    context.resource_scope.thread_id = Some(thread_id.clone());
    let run_context = loop_run_context(&context, thread_id)
        .await
        .with_actor(TurnActor::new(UserId::new("slack-alice").unwrap()));
    let loop_driver_extension = loop_driver_execution_extension_id(&run_context).unwrap();
    context.extension_id = loop_driver_extension.clone();
    context
        .grants
        .grants
        .push(dispatch_grant(capability_id.clone(), loop_driver_extension));
    let visible_request = HostVisibleCapabilityRequest::new(
        context,
        SurfaceKind::new("authenticated_actor_parity").unwrap(),
    )
    .with_policy(CapabilitySurfacePolicy::allow_all())
    .with_provider_trust(BTreeMap::from([(
        ExtensionId::new(BUILTIN_FIRST_PARTY_PROVIDER).unwrap(),
        provider_trust,
    )]));
    let port: Arc<dyn LoopCapabilityPort> = HostRuntimeLoopCapabilityPortFactory::new(
        runtime,
        visible_request,
        Arc::new(UnusedInputResolver),
        Arc::new(ResultWriter),
        Arc::new(InMemoryLoopHostMilestoneSink::default()),
    )
    .for_run_context(run_context);

    let surface = port
        .visible_capabilities(VisibleCapabilityRequest)
        .await
        .expect("real host runtime publishes the built-in echo capability");
    assert!(
        surface
            .descriptors
            .iter()
            .any(|descriptor| descriptor.capability_id == capability_id),
        "built-in echo must be visible through the real host runtime: {surface:?}"
    );
    let candidate = port
        .register_provider_tool_call(RegisterProviderToolCallRequest::new(ProviderToolCall {
            provider_id: "provider".to_string(),
            provider_model_id: "model".to_string(),
            turn_id: Some("turn-actor-parity".to_string()),
            id: "call-actor-parity".to_string(),
            name: ProviderToolName::new("builtin__echo").unwrap(),
            arguments: json!({"message": "hello"}),
            response_reasoning: None,
            reasoning: None,
            signature: None,
        }))
        .await
        .expect("provider tool call is registered");
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
        .expect("real first-party dispatch succeeds");

    assert!(matches!(outcome, CapabilityOutcome::Completed(_)));
    let (subject, actor) = recorded
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
        .expect("recording first-party handler receives one request");
    assert_eq!(subject.user_id.as_str(), "shared-subject");
    assert_eq!(actor.as_ref().map(UserId::as_str), Some("slack-alice"));
}

type RecordedActorRequest = (ResourceScope, Option<UserId>);

struct RecordingActorHandler {
    recorded: Arc<Mutex<Option<RecordedActorRequest>>>,
}

#[async_trait]
impl FirstPartyCapabilityHandler for RecordingActorHandler {
    async fn dispatch(
        &self,
        request: FirstPartyCapabilityRequest,
    ) -> Result<FirstPartyCapabilityResult, FirstPartyCapabilityError> {
        *self
            .recorded
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) =
            Some((request.scope, request.authenticated_actor_user_id));
        Ok(FirstPartyCapabilityResult::new(
            json!({"ok": true}),
            ResourceUsage::default(),
        ))
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
            "provider-call staging should own this input",
        ))
    }
}

struct ResultWriter;

#[async_trait]
impl LoopCapabilityResultWriter for ResultWriter {
    async fn write_capability_result(
        &self,
        _write: CapabilityResultWrite<'_>,
    ) -> Result<CapabilityWriteResult, AgentLoopHostError> {
        Ok(CapabilityWriteResult::without_output_digest(
            LoopResultRef::new("result:actor-parity").unwrap(),
            0,
        ))
    }
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

fn dispatch_grant(
    capability_id: CapabilityId,
    loop_driver_extension: ExtensionId,
) -> CapabilityGrant {
    CapabilityGrant {
        id: CapabilityGrantId::new(),
        capability: capability_id,
        grantee: Principal::Extension(loop_driver_extension),
        issued_by: Principal::HostRuntime,
        constraints: GrantConstraints {
            allowed_effects: vec![EffectKind::DispatchCapability],
            mounts: MountView::default(),
            network: NetworkPolicy::default(),
            secrets: Vec::new(),
            resource_ceiling: None,
            expires_at: None,
            max_invocations: None,
        },
    }
}

fn first_party_trust_policy() -> HostTrustPolicy {
    HostTrustPolicy::new(vec![Box::new(AdminConfig::with_entries(vec![
        AdminEntry::for_local_manifest(
            PackageId::new(BUILTIN_FIRST_PARTY_PROVIDER).unwrap(),
            "/system/extensions/builtin/manifest.toml".to_string(),
            None,
            HostTrustAssignment::first_party(),
            vec![EffectKind::DispatchCapability],
            None,
        ),
    ]))])
    .unwrap()
}
