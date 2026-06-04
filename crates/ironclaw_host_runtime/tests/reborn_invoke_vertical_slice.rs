mod support;

use support::legacy_capability_fixture_to_v2;

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_authorization::{GrantAuthorizer, TrustAwareCapabilityDispatchAuthorizer};
use ironclaw_dispatcher::{
    RuntimeAdapter, RuntimeAdapterRequest, RuntimeAdapterResult, RuntimeDispatcher,
};
use ironclaw_events::{InMemoryEventSink, RuntimeEventKind};
use ironclaw_extensions::{ExtensionManifest, ExtensionPackage, ExtensionRegistry, ManifestSource};
use ironclaw_filesystem::LocalFilesystem;
use ironclaw_host_api::*;
use ironclaw_host_runtime::{
    AttestedRaiseHook, AttestedRaiseRequest, CapabilitySurfaceVersion, DefaultHostRuntime,
    HostRuntime, RuntimeAttestedGate, RuntimeBlockedReason, RuntimeCapabilityOutcome,
    RuntimeCapabilityRequest, RuntimeFailureKind, RuntimeGateId,
};
use ironclaw_resources::{
    InMemoryResourceGovernor, ResourceAccount, ResourceGovernor, ResourceTally,
};
use ironclaw_run_state::{InMemoryRunStateStore, RunStateStore, RunStatus};
use ironclaw_trust::{
    AdminConfig, AdminEntry, AuthorityCeiling, EffectiveTrustClass, HostTrustAssignment,
    HostTrustPolicy, TrustDecision, TrustProvenance,
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
    let adapter = Arc::new(RecordingRuntimeAdapter::new(json!({"via":"host-runtime"})));
    let (registry, dispatcher, governor, events) = runtime_dispatcher_stack(Arc::clone(&adapter));
    let dispatcher: Arc<dyn CapabilityDispatcher> = Arc::new(dispatcher);
    let authorizer = Arc::new(CountingGrantAuthorizer::default());
    let run_state = Arc::new(InMemoryRunStateStore::new());
    let runtime = DefaultHostRuntime::new(
        Arc::clone(&registry),
        dispatcher,
        authorizer.clone(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
        local_test_runtime_policy(),
    )
    .with_trust_policy(Arc::new(local_manifest_trust_policy()))
    .with_run_state(run_state.clone());
    let context = execution_context(CapabilitySet {
        grants: vec![dispatch_grant()],
    });
    let scope = context.resource_scope.clone();
    let invocation_id = context.invocation_id;
    let estimate = ResourceEstimate {
        output_bytes: Some(4_096),
        ..ResourceEstimate::default()
    };
    let input = json!({"message":"through host runtime"});

    let outcome = runtime
        .invoke_capability(RuntimeCapabilityRequest::new(
            context,
            capability_id(),
            estimate.clone(),
            input.clone(),
            trust_decision(),
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
    assert_eq!(recorded.mounts, None);
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
    let adapter = Arc::new(RecordingRuntimeAdapter::new(json!({"must_not":"dispatch"})));
    let (registry, dispatcher, governor, events) = runtime_dispatcher_stack(Arc::clone(&adapter));
    let dispatcher: Arc<dyn CapabilityDispatcher> = Arc::new(dispatcher);
    let run_state = Arc::new(InMemoryRunStateStore::new());
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
            trust_decision(),
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

/// A stub raise hook that records the request and returns a fixed
/// `AttestedSigningRequired`. Proves `DefaultHostRuntime::invoke_capability`
/// routes `request_signature` to the injected hook instead of normal dispatch.
struct StubRaiseHook {
    seen: Arc<Mutex<Option<AttestedRaiseRequest>>>,
    expected_tx_hash: String,
}

#[async_trait]
impl AttestedRaiseHook for StubRaiseHook {
    async fn raise(&self, request: AttestedRaiseRequest) -> RuntimeCapabilityOutcome {
        let capability_id = request.capability_id.clone();
        *self.seen.lock().unwrap() = Some(request);
        RuntimeCapabilityOutcome::AttestedSigningRequired(RuntimeAttestedGate {
            gate_id: RuntimeGateId::new(),
            capability_id,
            expected_tx_hash: self.expected_tx_hash.clone(),
            reason: RuntimeBlockedReason::AttestedSigningRequired,
        })
    }
}

#[tokio::test]
async fn request_signature_invocation_routes_to_attested_raise_hook() {
    use ironclaw_host_runtime::{REQUEST_SIGNATURE_CAPABILITY_ID, builtin_first_party_package};
    use ironclaw_trust::HostTrustAssignment;

    let seen = Arc::new(Mutex::new(None));
    let hook = Arc::new(StubRaiseHook {
        seen: Arc::clone(&seen),
        expected_tx_hash: "deadbeef".to_string(),
    });

    let mut registry = ExtensionRegistry::new();
    registry
        .insert(builtin_first_party_package().expect("builtin package"))
        .expect("insert builtin package");
    let registry = Arc::new(registry);

    // A no-op dispatcher: routing to the hook must short-circuit BEFORE dispatch.
    let adapter = Arc::new(RecordingRuntimeAdapter::new(json!({"must_not":"dispatch"})));
    let filesystem = Arc::new(LocalFilesystem::new());
    let governor = Arc::new(InMemoryResourceGovernor::new());
    let dispatcher: Arc<dyn CapabilityDispatcher> = Arc::new(
        RuntimeDispatcher::from_arcs(Arc::clone(&registry), filesystem, governor)
            .with_runtime_adapter_arc(RuntimeKind::FirstParty, Arc::clone(&adapter)),
    );

    let trust_policy = HostTrustPolicy::new(vec![Box::new(AdminConfig::with_entries(vec![
        AdminEntry::for_local_manifest(
            PackageId::new("builtin").unwrap(),
            "/system/extensions/builtin/manifest.toml".to_string(),
            None,
            HostTrustAssignment::first_party(),
            vec![EffectKind::DispatchCapability],
            None,
        ),
    ]))])
    .unwrap();

    let runtime = DefaultHostRuntime::new(
        Arc::clone(&registry),
        dispatcher,
        Arc::new(GrantAuthorizer::new()),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
        local_test_runtime_policy(),
    )
    .with_trust_policy(Arc::new(trust_policy))
    .with_attested_raise_hook(hook);

    let capability_id = CapabilityId::new(REQUEST_SIGNATURE_CAPABILITY_ID).unwrap();
    let context = execution_context(CapabilitySet { grants: vec![] });
    let outcome = runtime
        .invoke_capability(RuntimeCapabilityRequest::new(
            context,
            capability_id.clone(),
            ResourceEstimate::default(),
            json!({"provider_hint":"custodial","signer_account":"abc","decoded":{}}),
            trust_decision(),
        ))
        .await
        .unwrap();

    match outcome {
        RuntimeCapabilityOutcome::AttestedSigningRequired(gate) => {
            assert_eq!(gate.capability_id, capability_id);
            assert_eq!(gate.expected_tx_hash, "deadbeef");
            assert_eq!(gate.reason, RuntimeBlockedReason::AttestedSigningRequired);
        }
        other => panic!("expected AttestedSigningRequired, got {other:?}"),
    }
    // The hook saw the invocation; the dispatcher was never reached.
    assert!(seen.lock().unwrap().is_some());
    assert_eq!(adapter.request_count(), 0);
}

#[tokio::test]
async fn request_signature_without_raise_hook_fails_closed() {
    use ironclaw_host_runtime::{REQUEST_SIGNATURE_CAPABILITY_ID, builtin_first_party_package};
    use ironclaw_trust::HostTrustAssignment;

    let mut registry = ExtensionRegistry::new();
    registry
        .insert(builtin_first_party_package().expect("builtin package"))
        .expect("insert builtin package");
    let registry = Arc::new(registry);

    let adapter = Arc::new(RecordingRuntimeAdapter::new(json!({"must_not":"dispatch"})));
    let filesystem = Arc::new(LocalFilesystem::new());
    let governor = Arc::new(InMemoryResourceGovernor::new());
    let dispatcher: Arc<dyn CapabilityDispatcher> = Arc::new(
        RuntimeDispatcher::from_arcs(Arc::clone(&registry), filesystem, governor)
            .with_runtime_adapter_arc(RuntimeKind::FirstParty, Arc::clone(&adapter)),
    );

    let trust_policy = HostTrustPolicy::new(vec![Box::new(AdminConfig::with_entries(vec![
        AdminEntry::for_local_manifest(
            PackageId::new("builtin").unwrap(),
            "/system/extensions/builtin/manifest.toml".to_string(),
            None,
            HostTrustAssignment::first_party(),
            vec![EffectKind::DispatchCapability],
            None,
        ),
    ]))])
    .unwrap();

    // No `.with_attested_raise_hook(..)`: a bare runtime must refuse rather than
    // pretend to raise a gate (fail-closed first-party handler).
    let runtime = DefaultHostRuntime::new(
        Arc::clone(&registry),
        dispatcher,
        Arc::new(GrantAuthorizer::new()),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
        local_test_runtime_policy(),
    )
    .with_trust_policy(Arc::new(trust_policy));

    let capability_id = CapabilityId::new(REQUEST_SIGNATURE_CAPABILITY_ID).unwrap();
    let context = execution_context(CapabilitySet { grants: vec![] });
    let outcome = runtime
        .invoke_capability(RuntimeCapabilityRequest::new(
            context,
            capability_id.clone(),
            ResourceEstimate::default(),
            json!({"provider_hint":"custodial","signer_account":"abc","decoded":{}}),
            trust_decision(),
        ))
        .await
        .unwrap();

    // Fail-closed: refuses (never AttestedSigningRequired, never Completed).
    assert!(
        matches!(outcome, RuntimeCapabilityOutcome::Failed(_)),
        "bare runtime without raise hook must fail closed, got {outcome:?}"
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
    requests: Mutex<Vec<RecordedRuntimeRequest>>,
}

impl RecordingRuntimeAdapter {
    fn new(output: Value) -> Self {
        Self {
            output,
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
impl RuntimeAdapter<LocalFilesystem, InMemoryResourceGovernor> for RecordingRuntimeAdapter {
    async fn dispatch_json(
        &self,
        request: RuntimeAdapterRequest<'_, LocalFilesystem, InMemoryResourceGovernor>,
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
        let usage = ResourceUsage {
            output_bytes: serde_json::to_vec(&output).unwrap().len() as u64,
            ..ResourceUsage::default()
        };
        let reservation = match request.resource_reservation {
            Some(reservation) => reservation,
            None => request
                .governor
                .reserve(request.scope, request.estimate)
                .map_err(|_| DispatchError::Wasm {
                    kind: RuntimeDispatchErrorKind::Resource,
                })?,
        };
        let output_bytes = usage.output_bytes;
        let receipt = request
            .governor
            .reconcile(reservation.id, usage.clone())
            .map_err(|_| DispatchError::Wasm {
                kind: RuntimeDispatchErrorKind::Resource,
            })?;
        Ok(RuntimeAdapterResult {
            output,
            usage,
            receipt,
            output_bytes,
        })
    }
}

#[derive(Default)]
struct CountingGrantAuthorizer {
    calls: AtomicUsize,
}

impl CountingGrantAuthorizer {
    fn call_count(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl TrustAwareCapabilityDispatchAuthorizer for CountingGrantAuthorizer {
    async fn authorize_dispatch_with_trust(
        &self,
        context: &ExecutionContext,
        descriptor: &CapabilityDescriptor,
        estimate: &ResourceEstimate,
        trust_decision: &TrustDecision,
    ) -> Decision {
        self.calls.fetch_add(1, Ordering::SeqCst);
        GrantAuthorizer::new()
            .authorize_dispatch_with_trust(context, descriptor, estimate, trust_decision)
            .await
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
    adapter: Arc<RecordingRuntimeAdapter>,
) -> (
    Arc<ExtensionRegistry>,
    RuntimeDispatcher<'static, LocalFilesystem, InMemoryResourceGovernor>,
    Arc<InMemoryResourceGovernor>,
    InMemoryEventSink,
) {
    let registry = Arc::new(registry_with_echo_capability());
    let filesystem = Arc::new(LocalFilesystem::new());
    let governor = Arc::new(InMemoryResourceGovernor::new());
    let events = InMemoryEventSink::new();
    let dispatcher =
        RuntimeDispatcher::from_arcs(Arc::clone(&registry), filesystem, Arc::clone(&governor))
            .with_runtime_adapter_arc(RuntimeKind::Wasm, adapter)
            .with_event_sink_arc(Arc::new(events.clone()));
    (registry, dispatcher, governor, events)
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
    )
    .unwrap()
}

fn execution_context(grants: CapabilitySet) -> ExecutionContext {
    ExecutionContext::local_default(
        UserId::new("user").unwrap(),
        ExtensionId::new("caller").unwrap(),
        RuntimeKind::Wasm,
        TrustClass::UserTrusted,
        grants,
        MountView::default(),
    )
    .unwrap()
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

fn trust_decision() -> TrustDecision {
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
