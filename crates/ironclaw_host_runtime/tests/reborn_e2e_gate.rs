use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_approvals::LeaseApproval;
use ironclaw_authorization::{
    CapabilityLeaseStatus, CapabilityLeaseStore, GrantAuthorizer, InMemoryCapabilityLeaseStore,
    TrustAwareCapabilityDispatchAuthorizer,
};
use ironclaw_events::{
    DurableEventLog, EventStreamKey, InMemoryDurableEventLog, InMemoryEventSink, ReadScope,
    RuntimeEventKind,
};
use ironclaw_extensions::{ExtensionManifest, ExtensionPackage, ExtensionRegistry};
use ironclaw_filesystem::LocalFilesystem;
use ironclaw_host_api::*;
use ironclaw_host_runtime::{
    CapabilitySurfaceVersion, HostHttpEgressService, HostRuntime, HostRuntimeServices,
    NetworkObligationPolicyStore, RuntimeCapabilityOutcome, RuntimeCapabilityRequest,
    RuntimeCapabilityResumeRequest, RuntimeFailureKind, RuntimeSecretInjectionStore,
    RuntimeStatusRequest, SurfaceKind,
};
use ironclaw_network::{
    NetworkHttpEgress, NetworkHttpError, NetworkHttpRequest, NetworkHttpResponse, NetworkUsage,
};
use ironclaw_processes::{InMemoryProcessResultStore, InMemoryProcessStore, ProcessServices};
use ironclaw_resources::{InMemoryResourceGovernor, ResourceAccount, ResourceTally};
use ironclaw_run_state::{
    InMemoryApprovalRequestStore, InMemoryRunStateStore, RunStateStore, RunStatus,
};
use ironclaw_scripts::{
    ScriptBackend, ScriptBackendOutput, ScriptBackendRequest, ScriptRuntime, ScriptRuntimeConfig,
};
use ironclaw_secrets::{InMemorySecretStore, SecretMaterial};
use ironclaw_trust::{
    AdminConfig, AdminEntry, AuthorityCeiling, EffectiveTrustClass, HostTrustAssignment,
    HostTrustPolicy, TrustDecision, TrustProvenance,
};
use serde_json::json;

#[tokio::test]
async fn reborn_e2e_gate_invokes_script_through_host_runtime_with_status_events_and_resources() {
    let governor = Arc::new(InMemoryResourceGovernor::new());
    let run_state = Arc::new(InMemoryRunStateStore::new());
    let event_log = Arc::new(InMemoryDurableEventLog::new());
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(SCRIPT_MANIFEST)),
        Arc::new(LocalFilesystem::new()),
        Arc::clone(&governor),
        Arc::new(GrantAuthorizer::new()),
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_trust_policy(Arc::new(local_manifest_trust_policy()))
    .with_run_state(Arc::clone(&run_state))
    .with_script_runtime(Arc::new(ScriptRuntime::new(
        ScriptRuntimeConfig::for_testing(),
        EchoScriptBackend,
    )))
    .with_durable_event_log(Arc::clone(&event_log));
    let runtime = services.host_runtime();
    let context = execution_context_with_dispatch_grant();
    let scope = context.resource_scope.clone();
    let invocation_id = context.invocation_id;

    let surface = runtime
        .visible_capabilities(ironclaw_host_runtime::VisibleCapabilityRequest::new(
            scope.clone(),
            context.correlation_id,
            SurfaceKind::new("gateway-smoke").unwrap(),
        ))
        .await
        .unwrap();
    assert_eq!(surface.version.as_str(), "surface-v1");
    assert_eq!(surface.descriptors.len(), 1);
    assert_eq!(surface.descriptors[0].id, script_capability_id());

    let health = runtime.health().await.unwrap();
    assert!(health.ready);
    assert!(health.missing_runtime_backends.is_empty());

    let status_before = runtime
        .runtime_status(RuntimeStatusRequest::new(
            scope.clone(),
            CorrelationId::new(),
        ))
        .await
        .unwrap();
    assert!(status_before.active_work.is_empty());

    let input = json!({
        "message": "reborn e2e happy path",
        "secret_sentinel": "SECRET_REBORN_E2E_GATE_SHOULD_NOT_LEAK",
        "host_path_sentinel": "/private/tmp/reborn-e2e-gate"
    });
    let outcome = runtime
        .invoke_capability(RuntimeCapabilityRequest::new(
            context,
            script_capability_id(),
            ResourceEstimate {
                output_bytes: Some(4096),
                ..ResourceEstimate::default()
            },
            input.clone(),
            trust_decision_with_dispatch_authority(),
        ))
        .await
        .unwrap();

    match outcome {
        RuntimeCapabilityOutcome::Completed(completed) => {
            assert_eq!(completed.capability_id, script_capability_id());
            assert_eq!(completed.output, input);
            assert!(completed.usage.output_bytes > 0);
        }
        other => panic!("expected completed outcome, got {other:?}"),
    }

    let run = run_state.get(&scope, invocation_id).await.unwrap().unwrap();
    assert_eq!(run.status, RunStatus::Completed);
    let tenant_account = ResourceAccount::tenant(scope.tenant_id.clone());
    assert_eq!(
        governor.reserved_for(&tenant_account),
        ResourceTally::default()
    );
    assert!(governor.usage_for(&tenant_account).output_bytes > 0);

    let status_after = runtime
        .runtime_status(RuntimeStatusRequest::new(
            scope.clone(),
            CorrelationId::new(),
        ))
        .await
        .unwrap();
    assert!(status_after.active_work.is_empty());

    let replay = event_log
        .read_after_cursor(
            &EventStreamKey::from_scope(&scope),
            &ReadScope::any(),
            None,
            10,
        )
        .await
        .unwrap();
    let kinds = replay
        .entries
        .iter()
        .map(|entry| entry.record.kind)
        .collect::<Vec<_>>();
    assert_eq!(
        kinds,
        vec![
            RuntimeEventKind::DispatchRequested,
            RuntimeEventKind::RuntimeSelected,
            RuntimeEventKind::DispatchSucceeded,
        ]
    );
    let serialized = serde_json::to_string(&replay).unwrap();
    for forbidden in [
        "SECRET_REBORN_E2E_GATE_SHOULD_NOT_LEAK",
        "/private/tmp/reborn-e2e-gate",
    ] {
        assert!(
            !serialized.contains(forbidden),
            "durable Reborn E2E event replay leaked {forbidden}: {serialized}"
        );
    }
}

#[tokio::test]
async fn reborn_e2e_gate_blocks_for_approval_resumes_once_and_rejects_replay() {
    let fixture = approval_resume_fixture();
    let runtime = fixture.services.host_runtime();
    let context = execution_context_without_grants();
    let scope = context.resource_scope.clone();
    let invocation_id = context.invocation_id;
    let input = json!({"message": "approval resume through Reborn E2E gate"});

    let gate = block_for_approval(&runtime, context.clone(), input.clone()).await;
    let blocked_run = fixture
        .run_state
        .get(&scope, invocation_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(blocked_run.status, RunStatus::BlockedApproval);
    assert_eq!(
        blocked_run.approval_request_id,
        Some(gate.approval_request_id)
    );

    let lease =
        approve_dispatch_for_services(&fixture.services, &scope, gate.approval_request_id).await;

    let resumed = runtime
        .resume_capability(RuntimeCapabilityResumeRequest::new(
            context.clone(),
            gate.approval_request_id,
            script_capability_id(),
            ResourceEstimate::default(),
            input.clone(),
            trust_decision_with_dispatch_authority(),
        ))
        .await
        .unwrap();
    match resumed {
        RuntimeCapabilityOutcome::Completed(completed) => {
            assert_eq!(completed.capability_id, script_capability_id());
            assert_eq!(completed.output, input);
        }
        other => panic!("expected completed approval resume, got {other:?}"),
    }
    assert_eq!(
        fixture
            .capability_leases
            .get(&scope, lease.grant.id)
            .await
            .unwrap()
            .status,
        CapabilityLeaseStatus::Consumed
    );
    assert_event_kinds(
        &fixture.events,
        &[
            RuntimeEventKind::DispatchRequested,
            RuntimeEventKind::RuntimeSelected,
            RuntimeEventKind::DispatchSucceeded,
        ],
    );

    let replay = runtime
        .resume_capability(RuntimeCapabilityResumeRequest::new(
            context,
            gate.approval_request_id,
            script_capability_id(),
            ResourceEstimate::default(),
            json!({"message": "approval resume through Reborn E2E gate"}),
            trust_decision_with_dispatch_authority(),
        ))
        .await
        .unwrap();
    assert_failed_outcome(replay, RuntimeFailureKind::Authorization);
    assert_eq!(
        fixture.events.events().len(),
        3,
        "replayed approval resume must fail before a second runtime dispatch"
    );
}

#[tokio::test]
async fn reborn_e2e_gate_fails_unsupported_obligations_before_runtime_events_or_success() {
    let run_state = Arc::new(InMemoryRunStateStore::new());
    let events = InMemoryEventSink::new();
    let governor = Arc::new(InMemoryResourceGovernor::new());
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(SCRIPT_MANIFEST)),
        Arc::new(LocalFilesystem::new()),
        Arc::clone(&governor),
        Arc::new(ObligatingAuthorizer),
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_trust_policy(Arc::new(local_manifest_trust_policy()))
    .with_run_state(Arc::clone(&run_state))
    .with_script_runtime(Arc::new(ScriptRuntime::new(
        ScriptRuntimeConfig::for_testing(),
        EchoScriptBackend,
    )))
    .with_event_sink(Arc::new(events.clone()));
    let runtime = services.host_runtime();
    let context = execution_context_with_dispatch_grant();
    let scope = context.resource_scope.clone();
    let invocation_id = context.invocation_id;

    let outcome = runtime
        .invoke_capability(RuntimeCapabilityRequest::new(
            context,
            script_capability_id(),
            ResourceEstimate::default(),
            json!({"message": "unsupported obligation"}),
            trust_decision_with_dispatch_authority(),
        ))
        .await
        .unwrap();

    assert_failed_outcome(outcome, RuntimeFailureKind::Backend);
    assert!(events.events().is_empty());
    let run = run_state.get(&scope, invocation_id).await.unwrap().unwrap();
    assert_eq!(run.status, RunStatus::Failed);
    assert_eq!(run.error_kind.as_deref(), Some("ObligationFailed"));
    let tenant_account = ResourceAccount::tenant(scope.tenant_id.clone());
    assert_eq!(
        governor.reserved_for(&tenant_account),
        ResourceTally::default()
    );
    assert_eq!(
        governor.usage_for(&tenant_account),
        ResourceTally::default()
    );
}

#[test]
fn reborn_e2e_gate_host_http_consumes_staged_policy_and_secret_once() {
    let network = RecordingNetwork::ok(NetworkHttpResponse {
        status: 200,
        headers: vec![],
        body: br#"{"ok":true}"#.to_vec(),
        usage: NetworkUsage {
            request_bytes: 5,
            response_bytes: 11,
            resolved_ip: None,
        },
    });
    let network_recorder = network.requests.clone();
    let policy_store = Arc::new(NetworkObligationPolicyStore::new());
    let secret_injections = Arc::new(RuntimeSecretInjectionStore::new());
    let scope = sample_scope(InvocationId::new());
    let capability_id = script_capability_id();
    let handle = SecretHandle::new("api-token").unwrap();
    let staged_policy = sample_policy();
    policy_store.insert(&scope, &capability_id, staged_policy.clone());
    secret_injections
        .insert(
            &scope,
            &capability_id,
            &handle,
            SecretMaterial::from("sk-reborn-e2e-staged-secret"),
        )
        .unwrap();
    let service = HostHttpEgressService::new(network, InMemorySecretStore::new())
        .with_network_policy_store(Arc::clone(&policy_store))
        .with_secret_injection_store(Arc::clone(&secret_injections));

    let request = RuntimeHttpEgressRequest {
        runtime: RuntimeKind::Script,
        scope: scope.clone(),
        capability_id: capability_id.clone(),
        method: NetworkMethod::Post,
        url: "https://api.example.test/v1/run".to_string(),
        headers: vec![],
        body: b"hello".to_vec(),
        network_policy: caller_supplied_policy(),
        credential_injections: vec![RuntimeCredentialInjection {
            handle: handle.clone(),
            source: RuntimeCredentialSource::StagedObligation {
                capability_id: capability_id.clone(),
            },
            target: RuntimeCredentialTarget::Header {
                name: "authorization".to_string(),
                prefix: Some("Bearer ".to_string()),
            },
            required: true,
        }],
        response_body_limit: Some(4096),
        timeout_ms: None,
    };

    let response = service
        .execute(request.clone())
        .expect("host HTTP egress should use staged Reborn policy and secret material");
    assert_eq!(response.status, 200);
    let recorded = network_recorder.lock().unwrap();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].policy, staged_policy);
    assert_eq!(
        recorded[0]
            .headers
            .iter()
            .find(|(name, _)| name == "authorization"),
        Some(&(
            "authorization".to_string(),
            "Bearer sk-reborn-e2e-staged-secret".to_string()
        ))
    );
    drop(recorded);
    assert!(
        secret_injections
            .take(&scope, &capability_id, &handle)
            .unwrap()
            .is_none(),
        "staged secret material must be consumed exactly once"
    );
    assert_eq!(
        policy_store.get(&scope, &capability_id),
        Some(staged_policy),
        "host egress must leave staged network policy for invocation/process lifecycle cleanup"
    );

    let replay = service
        .execute(request)
        .expect_err("consumed staged secret must not be reusable");
    assert!(matches!(replay, RuntimeHttpEgressError::Credential { .. }));
    assert_eq!(
        network_recorder.lock().unwrap().len(),
        1,
        "replay must fail before a second outbound transport attempt"
    );
}

type InMemoryServices = HostRuntimeServices<
    LocalFilesystem,
    InMemoryResourceGovernor,
    InMemoryProcessStore,
    InMemoryProcessResultStore,
>;

struct ApprovalFixture {
    services: InMemoryServices,
    run_state: Arc<InMemoryRunStateStore>,
    capability_leases: Arc<InMemoryCapabilityLeaseStore>,
    events: InMemoryEventSink,
}

fn approval_resume_fixture() -> ApprovalFixture {
    let run_state = Arc::new(InMemoryRunStateStore::new());
    let approval_requests = Arc::new(InMemoryApprovalRequestStore::new());
    let capability_leases = Arc::new(InMemoryCapabilityLeaseStore::new());
    let events = InMemoryEventSink::new();
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_manifest(SCRIPT_MANIFEST)),
        Arc::new(LocalFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(ApprovalThenGrantAuthorizer),
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_trust_policy(Arc::new(local_manifest_trust_policy()))
    .with_run_state(Arc::clone(&run_state))
    .with_approval_requests(approval_requests)
    .with_capability_leases(Arc::clone(&capability_leases))
    .with_script_runtime(Arc::new(ScriptRuntime::new(
        ScriptRuntimeConfig::for_testing(),
        EchoScriptBackend,
    )))
    .with_event_sink(Arc::new(events.clone()));

    ApprovalFixture {
        services,
        run_state,
        capability_leases,
        events,
    }
}

async fn block_for_approval(
    runtime: &impl HostRuntime,
    context: ExecutionContext,
    input: serde_json::Value,
) -> ironclaw_host_runtime::RuntimeApprovalGate {
    let outcome = runtime
        .invoke_capability(RuntimeCapabilityRequest::new(
            context,
            script_capability_id(),
            ResourceEstimate::default(),
            input,
            trust_decision_with_dispatch_authority(),
        ))
        .await
        .unwrap();
    match outcome {
        RuntimeCapabilityOutcome::ApprovalRequired(gate) => gate,
        other => panic!("expected approval gate, got {other:?}"),
    }
}

async fn approve_dispatch_for_services(
    services: &InMemoryServices,
    scope: &ResourceScope,
    approval_request_id: ApprovalRequestId,
) -> ironclaw_authorization::CapabilityLease {
    services
        .approval_resolver()
        .expect("approval resolver should be configured")
        .approve_dispatch(
            scope,
            approval_request_id,
            LeaseApproval {
                issued_by: Principal::HostRuntime,
                allowed_effects: vec![EffectKind::DispatchCapability],
                mounts: MountView::default(),
                network: NetworkPolicy::default(),
                secrets: Vec::new(),
                resource_ceiling: None,
                expires_at: None,
                max_invocations: Some(1),
            },
        )
        .await
        .unwrap()
}

struct ApprovalThenGrantAuthorizer;

#[async_trait]
impl TrustAwareCapabilityDispatchAuthorizer for ApprovalThenGrantAuthorizer {
    async fn authorize_dispatch_with_trust(
        &self,
        context: &ExecutionContext,
        descriptor: &CapabilityDescriptor,
        estimate: &ResourceEstimate,
        trust_decision: &TrustDecision,
    ) -> Decision {
        if context.grants.grants.is_empty() {
            Decision::RequireApproval {
                request: ApprovalRequest {
                    id: ApprovalRequestId::new(),
                    correlation_id: context.correlation_id,
                    requested_by: Principal::Extension(context.extension_id.clone()),
                    action: Box::new(Action::Dispatch {
                        capability: descriptor.id.clone(),
                        estimated_resources: estimate.clone(),
                    }),
                    invocation_fingerprint: None,
                    reason: "approval required".to_string(),
                    reusable_scope: None,
                },
            }
        } else {
            GrantAuthorizer::new()
                .authorize_dispatch_with_trust(context, descriptor, estimate, trust_decision)
                .await
        }
    }
}

struct ObligatingAuthorizer;

#[async_trait]
impl TrustAwareCapabilityDispatchAuthorizer for ObligatingAuthorizer {
    async fn authorize_dispatch_with_trust(
        &self,
        _context: &ExecutionContext,
        _descriptor: &CapabilityDescriptor,
        _estimate: &ResourceEstimate,
        _trust_decision: &TrustDecision,
    ) -> Decision {
        Decision::Allow {
            obligations: Obligations::new(vec![Obligation::AuditBefore]).unwrap(),
        }
    }
}

struct EchoScriptBackend;

impl ScriptBackend for EchoScriptBackend {
    fn execute(&self, request: ScriptBackendRequest) -> Result<ScriptBackendOutput, String> {
        let value = serde_json::from_str(&request.stdin_json).map_err(|error| error.to_string())?;
        Ok(ScriptBackendOutput::json(value))
    }
}

#[derive(Clone)]
struct RecordingNetwork {
    response: Result<NetworkHttpResponse, NetworkHttpError>,
    requests: Arc<Mutex<Vec<NetworkHttpRequest>>>,
}

impl RecordingNetwork {
    fn ok(response: NetworkHttpResponse) -> Self {
        Self {
            response: Ok(response),
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl NetworkHttpEgress for RecordingNetwork {
    fn execute(
        &self,
        request: NetworkHttpRequest,
    ) -> Result<NetworkHttpResponse, NetworkHttpError> {
        self.requests.lock().unwrap().push(request);
        self.response.clone()
    }
}

fn registry_with_manifest(manifest: &str) -> ExtensionRegistry {
    let mut registry = ExtensionRegistry::new();
    let manifest = ExtensionManifest::parse(manifest).unwrap();
    let package = ExtensionPackage::from_manifest(
        manifest,
        VirtualPath::new("/system/extensions/script").unwrap(),
    )
    .unwrap();
    registry.insert(package).unwrap();
    registry
}

fn execution_context_with_dispatch_grant() -> ExecutionContext {
    let mut grants = CapabilitySet::default();
    grants.grants.push(dispatch_grant());
    ExecutionContext::local_default(
        UserId::new("user").unwrap(),
        ExtensionId::new("caller").unwrap(),
        RuntimeKind::Script,
        TrustClass::UserTrusted,
        grants,
        MountView::default(),
    )
    .unwrap()
}

fn execution_context_without_grants() -> ExecutionContext {
    ExecutionContext::local_default(
        UserId::new("user").unwrap(),
        ExtensionId::new("caller").unwrap(),
        RuntimeKind::Script,
        TrustClass::UserTrusted,
        CapabilitySet::default(),
        MountView::default(),
    )
    .unwrap()
}

fn dispatch_grant() -> CapabilityGrant {
    CapabilityGrant {
        id: CapabilityGrantId::new(),
        capability: script_capability_id(),
        grantee: Principal::Extension(ExtensionId::new("caller").unwrap()),
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

fn local_manifest_trust_policy() -> HostTrustPolicy {
    HostTrustPolicy::new(vec![Box::new(AdminConfig::with_entries(vec![
        AdminEntry::for_local_manifest(
            PackageId::new("script").unwrap(),
            "/system/extensions/script/manifest.toml".to_string(),
            None,
            HostTrustAssignment::user_trusted(),
            vec![EffectKind::DispatchCapability],
            None,
        ),
    ]))])
    .unwrap()
}

fn trust_decision_with_dispatch_authority() -> TrustDecision {
    TrustDecision {
        effective_trust: EffectiveTrustClass::user_trusted(),
        authority_ceiling: AuthorityCeiling {
            allowed_effects: vec![EffectKind::DispatchCapability],
            max_resource_ceiling: None,
        },
        provenance: TrustProvenance::Default,
        evaluated_at: Utc::now(),
    }
}

fn sample_scope(invocation_id: InvocationId) -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new("tenant1").unwrap(),
        user_id: UserId::new("user1").unwrap(),
        agent_id: None,
        project_id: None,
        mission_id: None,
        thread_id: None,
        invocation_id,
    }
}

fn sample_policy() -> NetworkPolicy {
    NetworkPolicy {
        allowed_targets: vec![NetworkTargetPattern {
            scheme: Some(NetworkScheme::Https),
            host_pattern: "api.example.test".to_string(),
            port: None,
        }],
        deny_private_ip_ranges: true,
        max_egress_bytes: Some(4096),
    }
}

fn caller_supplied_policy() -> NetworkPolicy {
    NetworkPolicy {
        allowed_targets: vec![NetworkTargetPattern {
            scheme: Some(NetworkScheme::Https),
            host_pattern: "caller.example.test".to_string(),
            port: None,
        }],
        deny_private_ip_ranges: false,
        max_egress_bytes: Some(1),
    }
}

fn script_capability_id() -> CapabilityId {
    CapabilityId::new("script.echo").unwrap()
}

fn assert_event_kinds(events: &InMemoryEventSink, expected: &[RuntimeEventKind]) {
    let actual = events
        .events()
        .into_iter()
        .map(|event| event.kind)
        .collect::<Vec<_>>();
    assert_eq!(actual, expected);
}

fn assert_failed_outcome(outcome: RuntimeCapabilityOutcome, expected: RuntimeFailureKind) {
    match outcome {
        RuntimeCapabilityOutcome::Failed(failure) => assert_eq!(failure.kind, expected),
        other => panic!("expected failed outcome {expected:?}, got {other:?}"),
    }
}

const SCRIPT_MANIFEST: &str = r#"
id = "script"
name = "Script Echo"
version = "0.1.0"
description = "Script echo test extension"
trust = "third_party"

[runtime]
kind = "script"
runner = "sandboxed_process"
command = "echo-script"
args = []

[[capabilities]]
id = "script.echo"
description = "Echo text through script runtime"
effects = ["dispatch_capability"]
default_permission = "allow"
parameters_schema = { type = "object" }
"#;
