use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::{
    DispatchError, ExtensionId, ResourceEstimate, RuntimeCredentialAccountProviderId,
    RuntimeCredentialAuthRequirement, RuntimeDispatchErrorKind, RuntimeKind, SecretHandle,
};
use serde_json::json;

use super::*;

#[tokio::test]
async fn first_party_adapter_maps_handler_auth_required_to_dispatch_auth_required() {
    let descriptor = test_descriptor(RuntimeKind::FirstParty, Vec::new());
    let registry = Arc::new(FirstPartyCapabilityRegistry::new().with_handler(
        descriptor.id.clone(),
        Arc::new(AuthRequiredFirstPartyHandler),
    ));
    let adapter = FirstPartyRuntimeAdapter::from_registry(
        registry,
        Arc::new(LocalInvocationServicesResolver::new(
            Arc::new(LocalFilesystem::new()),
            None,
            Arc::new(LocalHostProcessPort::new()),
            None,
        )),
    );
    let filesystem = LocalFilesystem::new();
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope();
    let package = test_package(WASM_MANIFEST, "test-wasm");
    let policy = policy_with(
        FilesystemBackendKind::HostWorkspace,
        ProcessBackendKind::LocalHost,
        NetworkMode::DirectLogged,
        SecretMode::ScrubbedEnv,
    );

    let result = adapter
        .dispatch_json(RuntimeAdapterRequest {
            package: &package,
            descriptor: &descriptor,
            filesystem: &filesystem,
            governor: &governor,
            runtime_policy: &policy,
            capability_id: &descriptor.id,
            scope,
            estimate: ResourceEstimate::default(),
            mounts: None,
            resource_reservation: None,
            input: json!({}),
        })
        .await;

    // required_secrets must be forwarded, not silently dropped.
    match result {
        Err(DispatchError::AuthRequired {
            capability,
            required_secrets,
            credential_requirements,
        }) => {
            assert_eq!(capability, descriptor.id);
            assert!(
                required_secrets.is_empty(),
                "auth_required() handler yields no required handles; got {required_secrets:?}"
            );
            assert!(credential_requirements.is_empty());
        }
        other => panic!("expected AuthRequired, got {other:?}"),
    }
}

#[tokio::test]
async fn first_party_adapter_releases_reservation_when_handler_returns_auth_required() {
    let descriptor = test_descriptor(RuntimeKind::FirstParty, Vec::new());
    let registry = Arc::new(FirstPartyCapabilityRegistry::new().with_handler(
        descriptor.id.clone(),
        Arc::new(AuthRequiredFirstPartyHandler),
    ));
    let adapter = FirstPartyRuntimeAdapter::from_registry(
        registry,
        Arc::new(LocalInvocationServicesResolver::new(
            Arc::new(LocalFilesystem::new()),
            None,
            Arc::new(LocalHostProcessPort::new()),
            None,
        )),
    );
    let filesystem = LocalFilesystem::new();
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope();
    let tenant_account = ResourceAccount::tenant(scope.tenant_id.clone());
    let package = test_package(WASM_MANIFEST, "test-wasm");
    let policy = policy_with(
        FilesystemBackendKind::HostWorkspace,
        ProcessBackendKind::LocalHost,
        NetworkMode::DirectLogged,
        SecretMode::ScrubbedEnv,
    );

    let result = adapter
        .dispatch_json(RuntimeAdapterRequest {
            package: &package,
            descriptor: &descriptor,
            filesystem: &filesystem,
            governor: &governor,
            runtime_policy: &policy,
            capability_id: &descriptor.id,
            scope,
            estimate: ResourceEstimate::default(),
            mounts: None,
            resource_reservation: None,
            input: json!({}),
        })
        .await;

    assert!(matches!(result, Err(DispatchError::AuthRequired { .. })));
    assert_eq!(
        governor.reserved_for(&tenant_account),
        ResourceTally::default(),
        "reservation must be released when handler returns AuthRequired"
    );
}

#[tokio::test]
async fn first_party_adapter_forwards_required_secrets_from_auth_required_handler() {
    let handle = SecretHandle::new("google-access-token").unwrap();
    let descriptor = test_descriptor(RuntimeKind::FirstParty, Vec::new());
    let registry = Arc::new(FirstPartyCapabilityRegistry::new().with_handler(
        descriptor.id.clone(),
        Arc::new(AuthRequiredWithSecretsHandler {
            handle: handle.clone(),
        }),
    ));
    let adapter = FirstPartyRuntimeAdapter::from_registry(
        registry,
        Arc::new(LocalInvocationServicesResolver::new(
            Arc::new(LocalFilesystem::new()),
            None,
            Arc::new(LocalHostProcessPort::new()),
            None,
        )),
    );
    let filesystem = LocalFilesystem::new();
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope();
    let package = test_package(WASM_MANIFEST, "test-wasm");
    let policy = policy_with(
        FilesystemBackendKind::HostWorkspace,
        ProcessBackendKind::LocalHost,
        NetworkMode::DirectLogged,
        SecretMode::ScrubbedEnv,
    );

    let result = adapter
        .dispatch_json(RuntimeAdapterRequest {
            package: &package,
            descriptor: &descriptor,
            filesystem: &filesystem,
            governor: &governor,
            runtime_policy: &policy,
            capability_id: &descriptor.id,
            scope,
            estimate: ResourceEstimate::default(),
            mounts: None,
            resource_reservation: None,
            input: json!({}),
        })
        .await;

    match result {
        Err(DispatchError::AuthRequired {
            required_secrets, ..
        }) => {
            assert_eq!(required_secrets, vec![handle]);
        }
        other => panic!("expected AuthRequired, got {other:?}"),
    }
}

#[tokio::test]
async fn first_party_adapter_forwards_credential_requirements_from_auth_required_handler() {
    let requirement = RuntimeCredentialAuthRequirement {
        provider: RuntimeCredentialAccountProviderId::new("google").unwrap(),
        setup: ironclaw_host_api::RuntimeCredentialAccountSetup::OAuth {
            scopes: vec!["https://www.googleapis.com/auth/gmail.readonly".to_string()],
        },
        requester_extension: ExtensionId::new("gmail").unwrap(),
        provider_scopes: vec!["https://www.googleapis.com/auth/gmail.readonly".to_string()],
    };
    let descriptor = test_descriptor(RuntimeKind::FirstParty, Vec::new());
    let registry = Arc::new(FirstPartyCapabilityRegistry::new().with_handler(
        descriptor.id.clone(),
        Arc::new(AuthRequiredWithCredentialRequirementsHandler {
            requirement: requirement.clone(),
        }),
    ));
    let adapter = FirstPartyRuntimeAdapter::from_registry(
        registry,
        Arc::new(LocalInvocationServicesResolver::new(
            Arc::new(LocalFilesystem::new()),
            None,
            Arc::new(LocalHostProcessPort::new()),
            None,
        )),
    );
    let filesystem = LocalFilesystem::new();
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope();
    let package = test_package(WASM_MANIFEST, "test-wasm");
    let policy = policy_with(
        FilesystemBackendKind::HostWorkspace,
        ProcessBackendKind::LocalHost,
        NetworkMode::DirectLogged,
        SecretMode::ScrubbedEnv,
    );

    let result = adapter
        .dispatch_json(RuntimeAdapterRequest {
            package: &package,
            descriptor: &descriptor,
            filesystem: &filesystem,
            governor: &governor,
            runtime_policy: &policy,
            capability_id: &descriptor.id,
            scope,
            estimate: ResourceEstimate::default(),
            mounts: None,
            resource_reservation: None,
            input: json!({}),
        })
        .await;

    match result {
        Err(DispatchError::AuthRequired {
            credential_requirements,
            ..
        }) => {
            assert_eq!(credential_requirements, vec![requirement]);
        }
        other => panic!("expected AuthRequired, got {other:?}"),
    }
}

#[tokio::test]
async fn first_party_adapter_maps_panicking_handler_to_backend() {
    let descriptor = test_descriptor(RuntimeKind::FirstParty, Vec::new());
    let registry = Arc::new(
        FirstPartyCapabilityRegistry::new()
            .with_handler(descriptor.id.clone(), Arc::new(PanicOnDispatchHandler)),
    );
    let adapter = FirstPartyRuntimeAdapter::from_registry(
        registry,
        Arc::new(LocalInvocationServicesResolver::new(
            Arc::new(LocalFilesystem::new()),
            None,
            Arc::new(LocalHostProcessPort::new()),
            None,
        )),
    );
    let filesystem = LocalFilesystem::new();
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope();
    let tenant_account = ResourceAccount::tenant(scope.tenant_id.clone());
    let package = test_package(WASM_MANIFEST, "test-wasm");
    let policy = policy_with(
        FilesystemBackendKind::HostWorkspace,
        ProcessBackendKind::LocalHost,
        NetworkMode::DirectLogged,
        SecretMode::ScrubbedEnv,
    );

    let result = adapter
        .dispatch_json(RuntimeAdapterRequest {
            package: &package,
            descriptor: &descriptor,
            filesystem: &filesystem,
            governor: &governor,
            runtime_policy: &policy,
            capability_id: &descriptor.id,
            scope,
            estimate: ResourceEstimate::default(),
            mounts: None,
            resource_reservation: None,
            input: json!({}),
        })
        .await;

    assert!(
        matches!(
            result,
            Err(DispatchError::FirstParty {
                kind: RuntimeDispatchErrorKind::Backend,
                ..
            })
        ),
        "panicking handler must be contained as Backend, got {result:?}"
    );
    assert_eq!(
        governor.reserved_for(&tenant_account),
        ResourceTally::default(),
        "reservation must be released when handler panics"
    );
}

struct AuthRequiredFirstPartyHandler;

#[async_trait]
impl crate::FirstPartyCapabilityHandler for AuthRequiredFirstPartyHandler {
    async fn dispatch(
        &self,
        _request: crate::FirstPartyCapabilityRequest,
    ) -> Result<crate::FirstPartyCapabilityResult, crate::FirstPartyCapabilityError> {
        Err(crate::FirstPartyCapabilityError::auth_required())
    }
}

struct AuthRequiredWithSecretsHandler {
    handle: SecretHandle,
}

#[async_trait]
impl crate::FirstPartyCapabilityHandler for AuthRequiredWithSecretsHandler {
    async fn dispatch(
        &self,
        _request: crate::FirstPartyCapabilityRequest,
    ) -> Result<crate::FirstPartyCapabilityResult, crate::FirstPartyCapabilityError> {
        Err(crate::FirstPartyCapabilityError::auth_required_with(vec![
            self.handle.clone(),
        ]))
    }
}

struct AuthRequiredWithCredentialRequirementsHandler {
    requirement: RuntimeCredentialAuthRequirement,
}

#[async_trait]
impl crate::FirstPartyCapabilityHandler for AuthRequiredWithCredentialRequirementsHandler {
    async fn dispatch(
        &self,
        _request: crate::FirstPartyCapabilityRequest,
    ) -> Result<crate::FirstPartyCapabilityResult, crate::FirstPartyCapabilityError> {
        Err(
            crate::FirstPartyCapabilityError::auth_required_for_credentials(vec![
                self.requirement.clone(),
            ]),
        )
    }
}

struct PanicOnDispatchHandler;

#[async_trait]
impl crate::FirstPartyCapabilityHandler for PanicOnDispatchHandler {
    async fn dispatch(
        &self,
        _request: crate::FirstPartyCapabilityRequest,
    ) -> Result<crate::FirstPartyCapabilityResult, crate::FirstPartyCapabilityError> {
        panic!("handler panic must be contained at the adapter boundary")
    }
}

// Test double: reconcile always fails with UnknownReservation.
// Used to verify the reconcile-failure path in FirstPartyRuntimeAdapter
// releases the reservation and returns DispatchError::FirstParty { Resource }.
struct ReconcileFailingGovernor {
    inner: InMemoryResourceGovernor,
}

impl ReconcileFailingGovernor {
    fn new() -> Self {
        Self {
            inner: InMemoryResourceGovernor::new(),
        }
    }
}

impl ResourceGovernor for ReconcileFailingGovernor {
    fn set_limit(
        &self,
        account: ResourceAccount,
        limits: ironclaw_resources::ResourceLimits,
    ) -> Result<(), ironclaw_resources::ResourceError> {
        self.inner.set_limit(account, limits)
    }

    fn reserve_with_outcome(
        &self,
        scope: ironclaw_host_api::ResourceScope,
        estimate: ironclaw_host_api::ResourceEstimate,
    ) -> Result<ironclaw_resources::ReservationOutcome, ironclaw_resources::ResourceError> {
        self.inner.reserve_with_outcome(scope, estimate)
    }

    fn reserve_with_id_and_outcome(
        &self,
        scope: ironclaw_host_api::ResourceScope,
        estimate: ironclaw_host_api::ResourceEstimate,
        reservation_id: ironclaw_host_api::ResourceReservationId,
    ) -> Result<ironclaw_resources::ReservationOutcome, ironclaw_resources::ResourceError> {
        self.inner
            .reserve_with_id_and_outcome(scope, estimate, reservation_id)
    }

    fn reconcile(
        &self,
        reservation_id: ironclaw_host_api::ResourceReservationId,
        _actual: ironclaw_host_api::ResourceUsage,
    ) -> Result<ironclaw_host_api::ResourceReceipt, ironclaw_resources::ResourceError> {
        Err(ironclaw_resources::ResourceError::UnknownReservation { id: reservation_id })
    }

    fn release(
        &self,
        reservation_id: ironclaw_host_api::ResourceReservationId,
    ) -> Result<ironclaw_host_api::ResourceReceipt, ironclaw_resources::ResourceError> {
        self.inner.release(reservation_id)
    }

    fn account_snapshot(
        &self,
        account: &ResourceAccount,
    ) -> Result<Option<ironclaw_resources::AccountSnapshot>, ironclaw_resources::ResourceError>
    {
        self.inner.account_snapshot(account)
    }
}

#[tokio::test]
async fn first_party_adapter_releases_reservation_when_reconcile_fails_after_success() {
    let descriptor = test_descriptor(RuntimeKind::FirstParty, Vec::new());
    let registry = Arc::new(
        FirstPartyCapabilityRegistry::new()
            .with_handler(descriptor.id.clone(), Arc::new(SucceedingFirstPartyHandler)),
    );
    let adapter = FirstPartyRuntimeAdapter::from_registry(
        registry,
        Arc::new(LocalInvocationServicesResolver::new(
            Arc::new(LocalFilesystem::new()),
            None,
            Arc::new(LocalHostProcessPort::new()),
            None,
        )),
    );
    let filesystem = LocalFilesystem::new();
    let governor = ReconcileFailingGovernor::new();
    let scope = sample_scope();
    let tenant_account = ResourceAccount::tenant(scope.tenant_id.clone());
    let package = test_package(WASM_MANIFEST, "test-wasm");
    let policy = policy_with(
        FilesystemBackendKind::HostWorkspace,
        ProcessBackendKind::LocalHost,
        NetworkMode::DirectLogged,
        SecretMode::ScrubbedEnv,
    );

    let result = adapter
        .dispatch_json(RuntimeAdapterRequest {
            package: &package,
            descriptor: &descriptor,
            filesystem: &filesystem,
            governor: &governor,
            runtime_policy: &policy,
            capability_id: &descriptor.id,
            scope,
            estimate: ResourceEstimate::default(),
            mounts: None,
            resource_reservation: None,
            input: json!({}),
        })
        .await;

    assert!(
        matches!(
            result,
            Err(DispatchError::FirstParty {
                kind: RuntimeDispatchErrorKind::Resource,
                ..
            })
        ),
        "reconcile failure must produce FirstParty{{Resource}}, got {result:?}"
    );
    assert_eq!(
        governor.inner.reserved_for(&tenant_account),
        ResourceTally::default(),
        "reservation must be released after reconcile failure"
    );
}

struct SucceedingFirstPartyHandler;

#[async_trait]
impl crate::FirstPartyCapabilityHandler for SucceedingFirstPartyHandler {
    async fn dispatch(
        &self,
        _request: crate::FirstPartyCapabilityRequest,
    ) -> Result<crate::FirstPartyCapabilityResult, crate::FirstPartyCapabilityError> {
        Ok(crate::FirstPartyCapabilityResult {
            output: serde_json::json!({"ok": true}),
            display_preview: None,
            usage: ironclaw_host_api::ResourceUsage::default(),
        })
    }
}
