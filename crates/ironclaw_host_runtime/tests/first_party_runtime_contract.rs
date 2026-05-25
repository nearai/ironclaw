use std::{
    panic::AssertUnwindSafe,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use chrono::Utc;
use futures_util::FutureExt;
use ironclaw_authorization::GrantAuthorizer;
use ironclaw_events::{InMemoryEventSink, RuntimeEventKind};
use ironclaw_extensions::{
    CapabilityManifest, CapabilityVisibility, ExtensionManifest, ExtensionPackage,
    ExtensionRegistry, ExtensionRuntime, MANIFEST_SCHEMA_VERSION, ManifestSource,
};
use ironclaw_filesystem::LocalFilesystem;
use ironclaw_host_api::*;
use ironclaw_host_runtime::{
    CapabilitySurfaceVersion, FirstPartyCapabilityError, FirstPartyCapabilityHandler,
    FirstPartyCapabilityRegistry, FirstPartyCapabilityRequest, FirstPartyCapabilityResult,
    HostRuntime, HostRuntimeServices, ProductionWiringComponent, ProductionWiringConfig,
    ProductionWiringIssueKind, RuntimeCapabilityOutcome, RuntimeCapabilityRequest,
    RuntimeFailureKind,
};
use ironclaw_network::{
    NetworkHttpEgress, NetworkHttpError, NetworkHttpRequest, NetworkHttpResponse, NetworkUsage,
};
use ironclaw_resources::{InMemoryResourceGovernor, ResourceAccount, ResourceTally};
use ironclaw_run_state::{InMemoryRunStateStore, RunStateStore, RunStatus};
use ironclaw_secrets::{InMemorySecretStore, SecretMaterial, SecretStore};
use ironclaw_trust::{
    AdminConfig, AdminEntry, AuthorityCeiling, EffectiveTrustClass, HostTrustAssignment,
    HostTrustPolicy, TrustDecision, TrustProvenance,
};
use serde_json::{Value, json};

#[tokio::test]
async fn host_runtime_invokes_first_party_handler_through_capability_host() {
    let handler = Arc::new(RecordingFirstPartyHandler::new(
        json!({"via":"first-party"}),
    ));
    let first_party =
        FirstPartyCapabilityRegistry::new().with_handler(capability_id(), Arc::clone(&handler));
    let events = InMemoryEventSink::new();
    let run_state = Arc::new(InMemoryRunStateStore::new());
    let governor = Arc::new(InMemoryResourceGovernor::new());
    let runtime = HostRuntimeServices::new(
        Arc::new(first_party_registry()),
        Arc::new(LocalFilesystem::new()),
        Arc::clone(&governor),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_first_party_capabilities(Arc::new(first_party))
    .with_trust_policy(Arc::new(first_party_trust_policy()))
    .with_run_state(Arc::clone(&run_state))
    .with_event_sink(Arc::new(events.clone()))
    .host_runtime_for_local_testing();
    let context = execution_context(CapabilitySet {
        grants: vec![dispatch_grant()],
    });
    let scope = context.resource_scope.clone();
    let invocation_id = context.invocation_id;
    let estimate = ResourceEstimate {
        output_bytes: Some(1024),
        ..ResourceEstimate::default()
    };
    let input = json!({"message":"status"});

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
        panic!("expected completed first-party invocation, got {outcome:?}");
    };
    assert_eq!(completed.capability_id, capability_id());
    assert_eq!(completed.output, json!({"via":"first-party"}));
    assert!(completed.usage.output_bytes > 0);

    let recorded = handler.take_request();
    assert_eq!(recorded.capability_id, capability_id());
    assert_eq!(recorded.scope, scope);
    assert_eq!(recorded.estimate, estimate);
    assert_eq!(recorded.mounts, None);
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
async fn first_party_handler_uses_staged_secret_through_production_host_egress() {
    let handle = SecretHandle::new("api-token").unwrap();
    let handler = Arc::new(HttpFirstPartyHandler {
        handle: handle.clone(),
    });
    let first_party =
        FirstPartyCapabilityRegistry::new().with_handler(capability_id(), Arc::clone(&handler));
    let secret_store = Arc::new(InMemorySecretStore::new());
    let network = RecordingNetworkHttpEgress::default();
    let network_recorder = network.requests.clone();
    let runtime = HostRuntimeServices::new(
        Arc::new(first_party_registry_with_effects(vec![
            EffectKind::DispatchCapability,
            EffectKind::Network,
            EffectKind::UseSecret,
        ])),
        Arc::new(LocalFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_secret_store(Arc::clone(&secret_store))
    .with_first_party_capabilities(Arc::new(first_party))
    .try_with_host_http_egress(network)
    .unwrap()
    .with_trust_policy(Arc::new(first_party_trust_policy()))
    .host_runtime_for_local_testing();
    let context = execution_context(CapabilitySet {
        grants: vec![dispatch_grant_with_secret(&handle)],
    });
    let scope = context.resource_scope.clone();
    stage_http_secret(&secret_store, &scope, &handle).await;

    let outcome = runtime
        .invoke_capability(RuntimeCapabilityRequest::new(
            context,
            capability_id(),
            ResourceEstimate {
                network_egress_bytes: Some(100),
                output_bytes: Some(1024),
                ..ResourceEstimate::default()
            },
            json!({"url":"https://api.example.test/v1/native"}),
            trust_decision(),
        ))
        .await
        .unwrap();

    let RuntimeCapabilityOutcome::Completed(completed) = outcome else {
        panic!("expected completed first-party HTTP fixture, got {outcome:?}");
    };
    assert_eq!(completed.output["status"], json!(200));
    let requests = network_recorder.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].url, "https://api.example.test/v1/native");
    assert_eq!(
        requests[0]
            .headers
            .iter()
            .find(|(name, _)| name == "authorization"),
        Some(&(
            "authorization".to_string(),
            "Bearer sk-first-party-staged-secret".to_string()
        ))
    );
}

#[tokio::test]
async fn first_party_handler_maps_egress_error_codes_to_dispatch_errors() {
    let cases = [
        (
            RuntimeHttpEgressError::Credential {
                reason: "missing staged credential".to_string(),
            },
            RuntimeFailureKind::Backend,
        ),
        (
            RuntimeHttpEgressError::Request {
                reason: "request denied".to_string(),
                request_bytes: 11,
                response_bytes: 0,
            },
            RuntimeFailureKind::InvalidInput,
        ),
        (
            RuntimeHttpEgressError::Network {
                reason: "network blocked".to_string(),
                request_bytes: 11,
                response_bytes: 0,
            },
            RuntimeFailureKind::Network,
        ),
        (
            RuntimeHttpEgressError::Response {
                reason: "bad response".to_string(),
                request_bytes: 11,
                response_bytes: 22,
            },
            RuntimeFailureKind::OperationFailed,
        ),
        (
            RuntimeHttpEgressError::Response {
                reason: RUNTIME_HTTP_REASON_RESPONSE_BODY_LIMIT_EXCEEDED.to_string(),
                request_bytes: 11,
                response_bytes: 4097,
            },
            RuntimeFailureKind::OutputTooLarge,
        ),
    ];

    for (error, expected_kind) in cases {
        let handle = SecretHandle::new("api-token").unwrap();
        let secret_store = Arc::new(InMemorySecretStore::new());
        let runtime = http_first_party_services(&handle)
            .with_secret_store(Arc::clone(&secret_store))
            .with_runtime_http_egress(Arc::new(FailingRuntimeHttpEgress { error }))
            .with_trust_policy(Arc::new(first_party_trust_policy()))
            .host_runtime_for_local_testing();
        let context = execution_context(CapabilitySet {
            grants: vec![dispatch_grant_with_secret(&handle)],
        });
        stage_http_secret(&secret_store, &context.resource_scope, &handle).await;

        let outcome = invoke_http_fixture(
            &runtime,
            context,
            json!({"url":"https://api.example.test/v1/native"}),
        )
        .await;

        let RuntimeCapabilityOutcome::Failed(failure) = outcome else {
            panic!("expected failed first-party HTTP fixture, got {outcome:?}");
        };
        assert_eq!(failure.kind, expected_kind, "{failure:?}");
    }

    let handle = SecretHandle::new("api-token").unwrap();
    let secret_store = Arc::new(InMemorySecretStore::new());
    let runtime = http_first_party_services(&handle)
        .with_secret_store(Arc::clone(&secret_store))
        .with_trust_policy(Arc::new(first_party_trust_policy()))
        .host_runtime_for_local_testing();
    let context = execution_context(CapabilitySet {
        grants: vec![dispatch_grant_with_secret(&handle)],
    });
    stage_http_secret(&secret_store, &context.resource_scope, &handle).await;
    let outcome = invoke_http_fixture(
        &runtime,
        context,
        json!({"url":"https://api.example.test/v1/native"}),
    )
    .await;
    let RuntimeCapabilityOutcome::Failed(failure) = outcome else {
        panic!("expected missing egress to fail, got {outcome:?}");
    };
    assert_eq!(failure.kind, RuntimeFailureKind::Network);

    let handle = SecretHandle::new("api-token").unwrap();
    let secret_store = Arc::new(InMemorySecretStore::new());
    let runtime = http_first_party_services(&handle)
        .with_secret_store(Arc::clone(&secret_store))
        .with_runtime_http_egress(Arc::new(UnreachableRuntimeHttpEgress))
        .with_trust_policy(Arc::new(first_party_trust_policy()))
        .host_runtime_for_local_testing();
    let context = execution_context(CapabilitySet {
        grants: vec![dispatch_grant_with_secret(&handle)],
    });
    stage_http_secret(&secret_store, &context.resource_scope, &handle).await;
    let outcome = invoke_http_fixture(&runtime, context, json!({"missing_url": true})).await;
    let RuntimeCapabilityOutcome::Failed(failure) = outcome else {
        panic!("expected missing URL to fail, got {outcome:?}");
    };
    assert_eq!(failure.kind, RuntimeFailureKind::InvalidInput);
}

#[tokio::test]
async fn first_party_handler_rejects_non_string_url_input() {
    for input in [json!({"url": 123}), json!({"url": true})] {
        let handle = SecretHandle::new("api-token").unwrap();
        let secret_store = Arc::new(InMemorySecretStore::new());
        let runtime = http_first_party_services(&handle)
            .with_secret_store(Arc::clone(&secret_store))
            .with_runtime_http_egress(Arc::new(UnreachableRuntimeHttpEgress))
            .with_trust_policy(Arc::new(first_party_trust_policy()))
            .host_runtime_for_local_testing();
        let context = execution_context(CapabilitySet {
            grants: vec![dispatch_grant_with_secret(&handle)],
        });
        stage_http_secret(&secret_store, &context.resource_scope, &handle).await;

        let outcome = invoke_http_fixture(&runtime, context, input).await;

        let RuntimeCapabilityOutcome::Failed(failure) = outcome else {
            panic!("expected non-string URL to fail, got {outcome:?}");
        };
        assert_eq!(failure.kind, RuntimeFailureKind::InvalidInput);
    }
}

#[test]
fn http_error_kind_maps_all_reason_codes() {
    let cases = [
        (
            RuntimeHttpEgressReasonCode::CredentialUnavailable,
            RuntimeDispatchErrorKind::Client,
        ),
        (
            RuntimeHttpEgressReasonCode::RequestDenied,
            RuntimeDispatchErrorKind::InputEncode,
        ),
        (
            RuntimeHttpEgressReasonCode::NetworkError,
            RuntimeDispatchErrorKind::NetworkDenied,
        ),
        (
            RuntimeHttpEgressReasonCode::ResponseError,
            RuntimeDispatchErrorKind::OperationFailed,
        ),
        (
            RuntimeHttpEgressReasonCode::ResponseBodyLimitExceeded,
            RuntimeDispatchErrorKind::OutputTooLarge,
        ),
    ];

    for (reason, expected_kind) in cases {
        assert_eq!(http_error_kind(reason), expected_kind);
    }
}

#[tokio::test]
async fn production_wiring_rejects_first_party_registry_without_declared_handler() {
    let services = HostRuntimeServices::new(
        Arc::new(first_party_registry()),
        Arc::new(LocalFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_first_party_capabilities(Arc::new(FirstPartyCapabilityRegistry::new()));

    let report = services
        .validate_production_wiring(&ProductionWiringConfig::new([RuntimeKind::FirstParty]))
        .expect_err("empty first-party registry must not satisfy declared first-party capability");

    assert!(
        report.contains(
            ProductionWiringComponent::FirstPartyRuntime,
            ProductionWiringIssueKind::Missing
        ),
        "missing first-party handler coverage should be reported: {report:?}"
    );
}

#[tokio::test]
async fn host_runtime_health_reports_missing_first_party_backend_for_empty_registry() {
    let runtime = HostRuntimeServices::new(
        Arc::new(first_party_registry()),
        Arc::new(LocalFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_first_party_capabilities(Arc::new(FirstPartyCapabilityRegistry::new()))
    .host_runtime_for_local_testing();

    let health = runtime.health().await.unwrap();

    assert!(
        !health.ready,
        "first-party backend must be unready without handler coverage"
    );
    assert_eq!(
        health.missing_runtime_backends,
        vec![RuntimeKind::FirstParty]
    );
}

#[tokio::test]
async fn first_party_handler_error_reconciles_reported_usage_after_side_effect() {
    let handler = Arc::new(FailingFirstPartyHandler::new(ResourceUsage {
        network_egress_bytes: 77,
        ..ResourceUsage::default()
    }));
    let first_party =
        FirstPartyCapabilityRegistry::new().with_handler(capability_id(), Arc::clone(&handler));
    let governor = Arc::new(InMemoryResourceGovernor::new());
    let runtime = HostRuntimeServices::new(
        Arc::new(first_party_registry()),
        Arc::new(LocalFilesystem::new()),
        Arc::clone(&governor),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_first_party_capabilities(Arc::new(first_party))
    .with_trust_policy(Arc::new(first_party_trust_policy()))
    .host_runtime_for_local_testing();
    let context = execution_context(CapabilitySet {
        grants: vec![dispatch_grant()],
    });
    let account = ResourceAccount::tenant(context.resource_scope.tenant_id.clone());

    let outcome = runtime
        .invoke_capability(RuntimeCapabilityRequest::new(
            context,
            capability_id(),
            ResourceEstimate {
                network_egress_bytes: Some(100),
                ..ResourceEstimate::default()
            },
            json!({"message":"status"}),
            trust_decision(),
        ))
        .await
        .unwrap();

    let RuntimeCapabilityOutcome::Failed(failure) = outcome else {
        panic!("expected failed first-party invocation, got {outcome:?}");
    };
    assert_eq!(failure.kind, RuntimeFailureKind::Backend);
    assert_eq!(governor.usage_for(&account).network_egress_bytes, 77);
    assert_eq!(governor.reserved_for(&account), ResourceTally::default());
}

#[tokio::test]
async fn first_party_handler_panic_fails_closed_and_releases_reservation() {
    let handler = Arc::new(PanickingFirstPartyHandler);
    let first_party =
        FirstPartyCapabilityRegistry::new().with_handler(capability_id(), Arc::clone(&handler));
    let events = InMemoryEventSink::new();
    let run_state = Arc::new(InMemoryRunStateStore::new());
    let governor = Arc::new(InMemoryResourceGovernor::new());
    let runtime = HostRuntimeServices::new(
        Arc::new(first_party_registry()),
        Arc::new(LocalFilesystem::new()),
        Arc::clone(&governor),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_first_party_capabilities(Arc::new(first_party))
    .with_trust_policy(Arc::new(first_party_trust_policy()))
    .with_run_state(Arc::clone(&run_state))
    .with_event_sink(Arc::new(events.clone()))
    .host_runtime_for_local_testing();
    let context = execution_context(CapabilitySet {
        grants: vec![dispatch_grant()],
    });
    let scope = context.resource_scope.clone();
    let invocation_id = context.invocation_id;
    let account = ResourceAccount::tenant(scope.tenant_id.clone());

    let outcome = AssertUnwindSafe(runtime.invoke_capability(RuntimeCapabilityRequest::new(
        context,
        capability_id(),
        ResourceEstimate {
            network_egress_bytes: Some(100),
            ..ResourceEstimate::default()
        },
        json!({"message":"status"}),
        trust_decision(),
    )))
    .catch_unwind()
    .await
    .expect("first-party handler panic must be translated into stable failed outcome")
    .unwrap();

    let RuntimeCapabilityOutcome::Failed(failure) = outcome else {
        panic!("expected handler panic to fail closed, got {outcome:?}");
    };
    assert_eq!(failure.kind, RuntimeFailureKind::Backend);
    assert_eq!(governor.reserved_for(&account), ResourceTally::default());
    let run = run_state.get(&scope, invocation_id).await.unwrap().unwrap();
    assert_eq!(run.status, RunStatus::Failed);
    assert_event_kinds(
        &events,
        &[
            RuntimeEventKind::DispatchRequested,
            RuntimeEventKind::RuntimeSelected,
            RuntimeEventKind::DispatchFailed,
        ],
    );
}

#[tokio::test]
async fn first_party_missing_handler_fails_closed_without_side_effect_handler() {
    let first_party = FirstPartyCapabilityRegistry::new();
    let events = InMemoryEventSink::new();
    let runtime = HostRuntimeServices::new(
        Arc::new(first_party_registry()),
        Arc::new(LocalFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_first_party_capabilities(Arc::new(first_party))
    .with_trust_policy(Arc::new(first_party_trust_policy()))
    .with_event_sink(Arc::new(events.clone()))
    .host_runtime_for_local_testing();
    let context = execution_context(CapabilitySet {
        grants: vec![dispatch_grant()],
    });

    let outcome = runtime
        .invoke_capability(RuntimeCapabilityRequest::new(
            context,
            capability_id(),
            ResourceEstimate::default(),
            json!({"message":"status"}),
            trust_decision(),
        ))
        .await
        .unwrap();

    let RuntimeCapabilityOutcome::Failed(failure) = outcome else {
        panic!("expected missing first-party handler to fail closed, got {outcome:?}");
    };
    assert_eq!(failure.capability_id, capability_id());
    assert_eq!(failure.kind, RuntimeFailureKind::Backend);
    assert_eq!(
        failure.message.as_deref(),
        Some("dispatch failed: UndeclaredCapability")
    );
    assert_event_kinds(
        &events,
        &[
            RuntimeEventKind::DispatchRequested,
            RuntimeEventKind::RuntimeSelected,
            RuntimeEventKind::DispatchFailed,
        ],
    );
}

#[derive(Clone)]
struct RecordedFirstPartyRequest {
    capability_id: CapabilityId,
    scope: ResourceScope,
    estimate: ResourceEstimate,
    mounts: Option<MountView>,
    input: Value,
}

struct RecordingFirstPartyHandler {
    output: Value,
    requests: Mutex<Vec<RecordedFirstPartyRequest>>,
}

impl RecordingFirstPartyHandler {
    fn new(output: Value) -> Self {
        Self {
            output,
            requests: Mutex::new(Vec::new()),
        }
    }

    fn take_request(&self) -> RecordedFirstPartyRequest {
        self.requests.lock().unwrap().remove(0)
    }
}

struct FailingFirstPartyHandler {
    usage: ResourceUsage,
}

impl FailingFirstPartyHandler {
    fn new(usage: ResourceUsage) -> Self {
        Self { usage }
    }
}

struct PanickingFirstPartyHandler;

struct HttpFirstPartyHandler {
    handle: SecretHandle,
}

#[async_trait]
impl FirstPartyCapabilityHandler for FailingFirstPartyHandler {
    async fn dispatch(
        &self,
        _request: FirstPartyCapabilityRequest,
    ) -> Result<FirstPartyCapabilityResult, FirstPartyCapabilityError> {
        Err(
            FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::Backend)
                .with_usage(self.usage.clone()),
        )
    }
}

#[async_trait]
impl FirstPartyCapabilityHandler for PanickingFirstPartyHandler {
    async fn dispatch(
        &self,
        _request: FirstPartyCapabilityRequest,
    ) -> Result<FirstPartyCapabilityResult, FirstPartyCapabilityError> {
        panic!("first-party handler panic")
    }
}

#[async_trait]
impl FirstPartyCapabilityHandler for RecordingFirstPartyHandler {
    async fn dispatch(
        &self,
        request: FirstPartyCapabilityRequest,
    ) -> Result<FirstPartyCapabilityResult, FirstPartyCapabilityError> {
        self.requests
            .lock()
            .unwrap()
            .push(RecordedFirstPartyRequest {
                capability_id: request.capability_id.clone(),
                scope: request.scope.clone(),
                estimate: request.estimate.clone(),
                mounts: request.mounts.clone(),
                input: request.input.clone(),
            });
        Ok(FirstPartyCapabilityResult::new(
            self.output.clone(),
            ResourceUsage::default(),
        ))
    }
}

fn first_party_registry() -> ExtensionRegistry {
    first_party_registry_with_effects(vec![EffectKind::DispatchCapability])
}

fn first_party_http_registry() -> ExtensionRegistry {
    first_party_registry_with_effects(vec![
        EffectKind::DispatchCapability,
        EffectKind::Network,
        EffectKind::UseSecret,
    ])
}

fn first_party_registry_with_effects(effects: Vec<EffectKind>) -> ExtensionRegistry {
    let package = ExtensionPackage::from_manifest(
        ExtensionManifest {
            schema_version: MANIFEST_SCHEMA_VERSION.to_string(),
            id: provider_id(),
            name: "Host".to_string(),
            version: "0.1.0".to_string(),
            description: "Host-owned first-party capabilities".to_string(),
            source: ManifestSource::HostBundled,
            requested_trust: RequestedTrustClass::FirstPartyRequested,
            descriptor_trust_default: TrustClass::Sandbox,
            runtime: ExtensionRuntime::FirstParty {
                service: "host".to_string(),
            },
            host_apis: Vec::new(),
            capabilities: vec![CapabilityManifest {
                id: capability_id(),
                implements: Vec::new(),
                description: "Reports host status".to_string(),
                effects,
                default_permission: PermissionMode::Allow,
                visibility: CapabilityVisibility::Model,
                input_schema_ref: CapabilityProfileSchemaRef::new(
                    "schemas/host/status.input.v1.json",
                )
                .unwrap(),
                output_schema_ref: CapabilityProfileSchemaRef::new(
                    "schemas/host/status.output.v1.json",
                )
                .unwrap(),
                prompt_doc_ref: Some(
                    CapabilityProfileSchemaRef::new("prompts/host/status.md").unwrap(),
                ),
                required_host_ports: Vec::new(),
                runtime_credentials: Vec::new(),
                resource_profile: None,
            }],
        },
        VirtualPath::new("/system/extensions/host").unwrap(),
    )
    .unwrap();
    let mut registry = ExtensionRegistry::new();
    registry.insert(package).unwrap();
    registry
}

#[async_trait]
impl FirstPartyCapabilityHandler for HttpFirstPartyHandler {
    async fn dispatch(
        &self,
        request: FirstPartyCapabilityRequest,
    ) -> Result<FirstPartyCapabilityResult, FirstPartyCapabilityError> {
        let egress = request
            .services
            .runtime_http_egress
            .as_ref()
            .ok_or_else(|| FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::NetworkDenied))?
            .clone();
        let url = request
            .input
            .get("url")
            .and_then(Value::as_str)
            .ok_or_else(|| FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::InputEncode))?
            .to_string();
        let response = egress
            .execute(RuntimeHttpEgressRequest {
                runtime: RuntimeKind::FirstParty,
                scope: request.scope.clone(),
                capability_id: request.capability_id.clone(),
                method: NetworkMethod::Get,
                url,
                headers: vec![],
                body: Vec::new(),
                network_policy: NetworkPolicy::default(),
                credential_injections: vec![RuntimeCredentialInjection {
                    handle: self.handle.clone(),
                    // Production first-party egress must use StagedObligation;
                    // SecretStoreLease is only for test/legacy paths.
                    source: RuntimeCredentialSource::StagedObligation {
                        capability_id: request.capability_id,
                    },
                    target: RuntimeCredentialTarget::Header {
                        name: "authorization".to_string(),
                        prefix: Some("Bearer ".to_string()),
                    },
                    required: true,
                }],
                response_body_limit: Some(4096),
                timeout_ms: Some(1000),
            })
            .map_err(|error| {
                FirstPartyCapabilityError::new(http_error_kind(error.reason_code()))
            })?;
        Ok(FirstPartyCapabilityResult::new(
            json!({"status": response.status}),
            ResourceUsage {
                network_egress_bytes: response.request_bytes,
                output_bytes: 14,
                ..ResourceUsage::default()
            },
        ))
    }
}

// Keep in sync with production error mapping if one is introduced.
fn http_error_kind(reason: RuntimeHttpEgressReasonCode) -> RuntimeDispatchErrorKind {
    match reason {
        RuntimeHttpEgressReasonCode::CredentialUnavailable => RuntimeDispatchErrorKind::Client,
        RuntimeHttpEgressReasonCode::RequestDenied => RuntimeDispatchErrorKind::InputEncode,
        RuntimeHttpEgressReasonCode::NetworkError => RuntimeDispatchErrorKind::NetworkDenied,
        RuntimeHttpEgressReasonCode::ResponseError => RuntimeDispatchErrorKind::OperationFailed,
        RuntimeHttpEgressReasonCode::ResponseBodyLimitExceeded => {
            RuntimeDispatchErrorKind::OutputTooLarge
        }
    }
}

fn http_first_party_services(
    handle: &SecretHandle,
) -> HostRuntimeServices<
    LocalFilesystem,
    InMemoryResourceGovernor,
    ironclaw_processes::InMemoryProcessStore,
    ironclaw_processes::InMemoryProcessResultStore,
> {
    let handler = Arc::new(HttpFirstPartyHandler {
        handle: handle.clone(),
    });
    let first_party =
        FirstPartyCapabilityRegistry::new().with_handler(capability_id(), Arc::clone(&handler));

    HostRuntimeServices::new(
        Arc::new(first_party_http_registry()),
        Arc::new(LocalFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_first_party_capabilities(Arc::new(first_party))
}

async fn invoke_http_fixture(
    runtime: &impl HostRuntime,
    context: ExecutionContext,
    input: Value,
) -> RuntimeCapabilityOutcome {
    runtime
        .invoke_capability(RuntimeCapabilityRequest::new(
            context,
            capability_id(),
            ResourceEstimate {
                network_egress_bytes: Some(100),
                output_bytes: Some(1024),
                ..ResourceEstimate::default()
            },
            input,
            trust_decision(),
        ))
        .await
        .unwrap()
}

fn execution_context(grants: CapabilitySet) -> ExecutionContext {
    ExecutionContext::local_default(
        UserId::new("user").unwrap(),
        ExtensionId::new("caller").unwrap(),
        RuntimeKind::FirstParty,
        TrustClass::FirstParty,
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

fn dispatch_grant_with_secret(handle: &SecretHandle) -> CapabilityGrant {
    CapabilityGrant {
        id: CapabilityGrantId::new(),
        capability: capability_id(),
        grantee: Principal::Extension(ExtensionId::new("caller").unwrap()),
        issued_by: Principal::HostRuntime,
        constraints: GrantConstraints {
            allowed_effects: vec![
                EffectKind::DispatchCapability,
                EffectKind::Network,
                EffectKind::UseSecret,
            ],
            mounts: MountView::default(),
            network: test_network_policy(),
            secrets: vec![handle.clone()],
            resource_ceiling: None,
            expires_at: None,
            max_invocations: None,
        },
    }
}

fn first_party_trust_policy() -> HostTrustPolicy {
    HostTrustPolicy::new(vec![Box::new(AdminConfig::with_entries(vec![
        AdminEntry::for_local_manifest(
            PackageId::new("host").unwrap(),
            "/system/extensions/host/manifest.toml".to_string(),
            None,
            HostTrustAssignment::first_party(),
            vec![
                EffectKind::DispatchCapability,
                EffectKind::Network,
                EffectKind::UseSecret,
            ],
            None,
        ),
    ]))])
    .unwrap()
}

fn test_network_policy() -> NetworkPolicy {
    NetworkPolicy {
        allowed_targets: vec![NetworkTargetPattern {
            scheme: Some(NetworkScheme::Https),
            host_pattern: "api.example.test".to_string(),
            port: None,
        }],
        deny_private_ip_ranges: true,
        max_egress_bytes: Some(10_000),
    }
}

async fn stage_http_secret(
    secret_store: &InMemorySecretStore,
    scope: &ResourceScope,
    handle: &SecretHandle,
) {
    secret_store
        .put(
            scope.clone(),
            handle.clone(),
            SecretMaterial::from("sk-first-party-staged-secret"),
        )
        .await
        .unwrap();
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

fn provider_id() -> ExtensionId {
    ExtensionId::new("host").unwrap()
}

fn capability_id() -> CapabilityId {
    CapabilityId::new("host.status").unwrap()
}

fn assert_event_kinds(events: &InMemoryEventSink, expected: &[RuntimeEventKind]) {
    let actual = events
        .events()
        .into_iter()
        .map(|event| event.kind)
        .collect::<Vec<_>>();
    assert_eq!(actual, expected);
}

#[derive(Debug, Clone, Default)]
struct RecordingNetworkHttpEgress {
    requests: Arc<Mutex<Vec<NetworkHttpRequest>>>,
}

struct FailingRuntimeHttpEgress {
    error: RuntimeHttpEgressError,
}

struct UnreachableRuntimeHttpEgress;

impl RuntimeHttpEgress for FailingRuntimeHttpEgress {
    fn execute(
        &self,
        _request: RuntimeHttpEgressRequest,
    ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
        Err(self.error.clone())
    }
}

impl RuntimeHttpEgress for UnreachableRuntimeHttpEgress {
    fn execute(
        &self,
        _request: RuntimeHttpEgressRequest,
    ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
        panic!("runtime HTTP egress should not be called before URL validation")
    }
}

impl NetworkHttpEgress for RecordingNetworkHttpEgress {
    fn execute(
        &self,
        request: NetworkHttpRequest,
    ) -> Result<NetworkHttpResponse, NetworkHttpError> {
        self.requests.lock().unwrap().push(request.clone());
        Ok(NetworkHttpResponse {
            status: 200,
            headers: Vec::new(),
            body: b"ok".to_vec(),
            usage: NetworkUsage {
                request_bytes: request.body.len() as u64,
                response_bytes: 2,
                resolved_ip: None,
            },
        })
    }
}
