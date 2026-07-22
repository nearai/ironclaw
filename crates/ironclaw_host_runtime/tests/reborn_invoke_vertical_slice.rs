mod support;

use support::legacy_capability_fixture_to_v2;

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

use async_trait::async_trait;
use ironclaw_authorization::TrustAwareCapabilityDispatchAuthorizer;
use ironclaw_dispatcher::{
    BoundCapabilityAdapter, CapabilityDispatchRequest, ResolvedCapability, RuntimeAdapterResult,
    RuntimeDispatcher, ToolResolver,
};
use ironclaw_events::{InMemoryEventSink, RuntimeEventKind};
use ironclaw_extensions::{ExtensionManifest, ExtensionPackage, ExtensionRegistry, ManifestSource};
use ironclaw_host_api::*;
use ironclaw_host_runtime::{
    BuiltinObligationHandler, CapabilitySurfaceVersion, DefaultHostRuntime, HostRuntime,
    RuntimeCapabilityOutcome, RuntimeCapabilityRequest, RuntimeFailureKind,
};
use ironclaw_resources::{
    InMemoryResourceGovernor, ResourceAccount, ResourceGovernor, ResourceTally,
};
use ironclaw_run_state::{RunStateStore, RunStatus};
use ironclaw_trust::{
    AdminConfig, AdminEntry, HostTrustAssignment, HostTrustPolicy, TrustDecision,
};
use serde_json::{Value, json};

fn local_test_runtime_policy() -> ironclaw_host_api::runtime_policy::EffectiveRuntimePolicy {
    ironclaw_runtime_policy::resolve(ironclaw_runtime_policy::ResolveRequest::new(
        ironclaw_host_api::runtime_policy::DeploymentMode::LocalSingleUser,
        ironclaw_host_api::runtime_policy::RuntimeProfile::LocalDev,
    ))
    .unwrap()
}

#[tokio::test]
async fn default_host_runtime_invokes_through_runtime_dispatcher_with_resources_and_events() {
    let (registry, dispatcher, governor, events, adapter) =
        runtime_dispatcher_stack(json!({"via":"host-runtime"}));
    let dispatcher: Arc<dyn CapabilityDispatcher> = Arc::new(dispatcher);
    let expected_mounts = representative_mounts();
    let authorizer = Arc::new(MountingAuthorizer::new(expected_mounts.clone()));
    let run_state = Arc::new(ironclaw_run_state::in_memory_backed_run_state_store());
    let runtime = DefaultHostRuntime::new(
        Arc::clone(&registry),
        dispatcher,
        authorizer.clone(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
        local_test_runtime_policy(),
    )
    .with_trust_policy(Arc::new(local_manifest_trust_policy()))
    .with_run_state(run_state.clone())
    .with_obligation_handler(Arc::new(BuiltinObligationHandler::new()));
    let mut context = execution_context(CapabilitySet {
        grants: vec![dispatch_grant()],
    });
    context.mounts = expected_mounts.clone();
    let scope = context.resource_scope.clone();
    let invocation_id = context.invocation_id;
    let estimate = ResourceEstimate::default().set_output_bytes(4_096);
    let input = json!({"message":"through host runtime"});

    let outcome = runtime
        .invoke_capability(RuntimeCapabilityRequest::new(
            context,
            capability_id(),
            estimate.clone(),
            input.clone(),
        ))
        .await
        .unwrap();

    let RuntimeCapabilityOutcome::Completed(completed) = outcome else {
        panic!("expected completed host-runtime outcome, got {outcome:?}");
    };
    assert_eq!(completed.capability_id, capability_id());
    assert_eq!(completed.output, json!({"via":"host-runtime"}));
    assert!(completed.usage.output_bytes > 0);
    assert_eq!(authorizer.call_count(), 1);

    let recorded = adapter.take_request();
    assert_eq!(recorded.capability_id, capability_id());
    assert_eq!(recorded.scope, scope);
    assert_eq!(recorded.estimate, estimate);
    assert_eq!(recorded.mounts, Some(expected_mounts));
    assert_eq!(recorded.resource_reservation, None);
    assert_eq!(recorded.input, input);

    let run = run_state.get(&scope, invocation_id).await.unwrap().unwrap();
    assert_eq!(run.status, RunStatus::Completed);

    let tenant_account = ResourceAccount::tenant(scope.tenant_id.clone());
    assert_eq!(
        governor.reserved_for(&tenant_account),
        ResourceTally::default()
    );
    assert!(governor.usage_for(&tenant_account).output_bytes > 0);
    assert_event_kinds(
        &events,
        &[
            RuntimeEventKind::DispatchRequested,
            RuntimeEventKind::RuntimeSelected,
            RuntimeEventKind::DispatchSucceeded,
        ],
    );
}

#[tokio::test]
async fn default_host_runtime_fails_unsupported_obligations_before_runtime_dispatch() {
    let (registry, dispatcher, governor, events, adapter) =
        runtime_dispatcher_stack(json!({"must_not":"dispatch"}));
    let dispatcher: Arc<dyn CapabilityDispatcher> = Arc::new(dispatcher);
    let run_state = Arc::new(ironclaw_run_state::in_memory_backed_run_state_store());
    let runtime = DefaultHostRuntime::new(
        Arc::clone(&registry),
        dispatcher,
        Arc::new(ObligatingAuthorizer),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
        local_test_runtime_policy(),
    )
    .with_run_state(run_state.clone());
    let context = execution_context(CapabilitySet {
        grants: vec![dispatch_grant()],
    });
    let scope = context.resource_scope.clone();
    let invocation_id = context.invocation_id;

    let outcome = runtime
        .invoke_capability(RuntimeCapabilityRequest::new(
            context,
            capability_id(),
            ResourceEstimate::default(),
            json!({"message":"obligation"}),
        ))
        .await
        .unwrap();

    let RuntimeCapabilityOutcome::Failed(failure) = outcome else {
        panic!("expected failed host-runtime outcome, got {outcome:?}");
    };
    assert_eq!(failure.capability_id, capability_id());
    assert_eq!(failure.kind, RuntimeFailureKind::Authorization);
    let message = failure
        .message
        .expect("failure should carry stable message");
    assert!(message.contains("unsupported authorization obligations"));
    assert!(!message.contains('/'));
    assert_eq!(adapter.request_count(), 0);
    assert!(events.events().is_empty());

    let run = run_state.get(&scope, invocation_id).await.unwrap().unwrap();
    assert_eq!(run.status, RunStatus::Failed);
    assert_eq!(run.error_kind.as_deref(), Some("UnsupportedObligations"));
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

#[derive(Clone)]
struct RecordedRuntimeRequest {
    capability_id: CapabilityId,
    scope: ResourceScope,
    estimate: ResourceEstimate,
    mounts: Option<MountView>,
    resource_reservation: Option<ResourceReservation>,
    input: Value,
}

struct RecordingRuntimeAdapter {
    output: Value,
    governor: Arc<InMemoryResourceGovernor>,
    requests: Mutex<Vec<RecordedRuntimeRequest>>,
}

impl RecordingRuntimeAdapter {
    fn new(output: Value, governor: Arc<InMemoryResourceGovernor>) -> Self {
        Self {
            output,
            governor,
            requests: Mutex::new(Vec::new()),
        }
    }

    fn take_request(&self) -> RecordedRuntimeRequest {
        self.requests.lock().unwrap().remove(0)
    }

    fn request_count(&self) -> usize {
        self.requests.lock().unwrap().len()
    }
}

#[async_trait]
impl BoundCapabilityAdapter for RecordingRuntimeAdapter {
    async fn dispatch_json(
        &self,
        request: CapabilityDispatchRequest,
    ) -> Result<RuntimeAdapterResult, DispatchError> {
        self.requests.lock().unwrap().push(RecordedRuntimeRequest {
            capability_id: request.capability_id.clone(),
            scope: request.scope.clone(),
            estimate: request.estimate.clone(),
            mounts: request.mounts.clone(),
            resource_reservation: request.resource_reservation.clone(),
            input: request.input.clone(),
        });
        let output = self.output.clone();
        let usage = ResourceUsage::default()
            .set_output_bytes(serde_json::to_vec(&output).unwrap().len() as u64);
        let reservation = match request.resource_reservation {
            Some(reservation) => reservation,
            None => self
                .governor
                .reserve(request.scope, request.estimate)
                .map_err(|_| DispatchError::Wasm {
                    kind: RuntimeDispatchErrorKind::Resource,
                    model_visible_cause: None,
                })?,
        };
        let output_bytes = usage.output_bytes;
        let receipt = self
            .governor
            .reconcile(reservation.id, usage.clone())
            .map_err(|_| DispatchError::Wasm {
                kind: RuntimeDispatchErrorKind::Resource,
                model_visible_cause: None,
            })?;
        Ok(RuntimeAdapterResult {
            output,
            display_preview: None,
            usage,
            receipt,
            output_bytes,
        })
    }
}

struct SingleCapabilityResolver {
    capability_id: CapabilityId,
    resolved: ResolvedCapability,
}

impl ToolResolver for SingleCapabilityResolver {
    fn resolve(&self, capability_id: &CapabilityId) -> Option<ResolvedCapability> {
        (capability_id == &self.capability_id).then(|| self.resolved.clone())
    }
}

struct MountingAuthorizer {
    calls: AtomicUsize,
    mounts: MountView,
}

impl MountingAuthorizer {
    fn new(mounts: MountView) -> Self {
        Self {
            calls: AtomicUsize::new(0),
            mounts,
        }
    }

    fn call_count(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl TrustAwareCapabilityDispatchAuthorizer for MountingAuthorizer {
    async fn authorize_dispatch_with_trust(
        &self,
        _context: &ExecutionContext,
        _descriptor: &CapabilityDescriptor,
        _estimate: &ResourceEstimate,
        _trust_decision: &TrustDecision,
    ) -> Decision {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Decision::Allow {
            obligations: Obligations::new(vec![Obligation::UseScopedMounts {
                mounts: self.mounts.clone(),
            }])
            .unwrap(),
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

fn runtime_dispatcher_stack(
    output: Value,
) -> (
    Arc<ExtensionRegistry>,
    RuntimeDispatcher<'static, InMemoryResourceGovernor>,
    Arc<InMemoryResourceGovernor>,
    InMemoryEventSink,
    Arc<RecordingRuntimeAdapter>,
) {
    let registry = Arc::new(registry_with_echo_capability());
    let governor = Arc::new(InMemoryResourceGovernor::new());
    let events = InMemoryEventSink::new();
    let adapter = Arc::new(RecordingRuntimeAdapter::new(output, Arc::clone(&governor)));
    let resolver: Arc<dyn ToolResolver> = Arc::new(SingleCapabilityResolver {
        capability_id: capability_id(),
        resolved: ResolvedCapability {
            provider: ExtensionId::new("echo").unwrap(),
            runtime: RuntimeKind::Wasm,
            adapter: Arc::clone(&adapter) as Arc<dyn BoundCapabilityAdapter>,
        },
    });
    let dispatcher = RuntimeDispatcher::from_arcs(resolver, Arc::clone(&governor))
        .with_event_sink_arc(Arc::new(events.clone()));
    (registry, dispatcher, governor, events, adapter)
}

fn registry_with_echo_capability() -> ExtensionRegistry {
    let manifest = parse_manifest(ECHO_MANIFEST);
    let package = ExtensionPackage::from_manifest(
        manifest,
        VirtualPath::new("/system/extensions/echo").unwrap(),
    )
    .unwrap();
    let mut registry = ExtensionRegistry::new();
    registry.insert(package).unwrap();
    registry
}

fn parse_manifest(manifest: &str) -> ExtensionManifest {
    let manifest = legacy_capability_fixture_to_v2(manifest);
    ExtensionManifest::parse(
        &manifest,
        ManifestSource::InstalledLocal,
        &HostPortCatalog::empty(),
        &capability_provider_contracts(),
    )
    .unwrap()
}

fn execution_context(grants: CapabilitySet) -> ExecutionContext {
    let mut context = ExecutionContext::local_default(
        UserId::new("user").unwrap(),
        ExtensionId::new("caller").unwrap(),
        RuntimeKind::Wasm,
        TrustClass::UserTrusted,
        grants,
        MountView::default(),
    )
    .unwrap();
    context.run_id = Some(RunId::new());
    context
}

fn dispatch_grant() -> CapabilityGrant {
    CapabilityGrant {
        id: CapabilityGrantId::new(),
        capability: capability_id(),
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

fn representative_mounts() -> MountView {
    MountView::new(vec![MountGrant::new(
        MountAlias::new("/workspace").unwrap(),
        VirtualPath::new("/projects/project-a").unwrap(),
        MountPermissions {
            read: true,
            write: true,
            delete: false,
            list: true,
            execute: false,
        },
    )])
    .unwrap()
}

fn local_manifest_trust_policy() -> HostTrustPolicy {
    HostTrustPolicy::new(vec![Box::new(AdminConfig::with_entries(vec![
        AdminEntry::for_local_manifest(
            PackageId::new("echo").unwrap(),
            "/system/extensions/echo/manifest.toml".to_string(),
            None,
            HostTrustAssignment::user_trusted(),
            vec![EffectKind::DispatchCapability],
            None,
        ),
    ]))])
    .unwrap()
}

fn capability_id() -> CapabilityId {
    CapabilityId::new("echo.say").unwrap()
}

fn assert_event_kinds(events: &InMemoryEventSink, expected: &[RuntimeEventKind]) {
    let actual = events
        .events()
        .into_iter()
        .map(|event| event.kind)
        .collect::<Vec<_>>();
    assert_eq!(actual, expected);
}

const ECHO_MANIFEST: &str = r#"
id = "echo"
name = "Echo"
version = "0.1.0"
description = "Echo test extension"
trust = "third_party"

[runtime]
kind = "wasm"
module = "echo.wasm"

[[capabilities]]
id = "echo.say"
description = "Echoes input"
effects = ["dispatch_capability"]
default_permission = "allow"
parameters_schema = {}
"#;

fn capability_provider_contracts() -> ironclaw_extensions::HostApiContractRegistry {
    let mut contracts = ironclaw_extensions::HostApiContractRegistry::new();
    contracts
        .register(std::sync::Arc::new(
            ironclaw_extensions::CapabilityProviderHostApiContract::new()
                .expect("capability provider contract"),
        ))
        .expect("register capability provider contract");
    contracts
}
