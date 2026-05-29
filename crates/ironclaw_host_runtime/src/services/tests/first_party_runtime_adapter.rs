use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::{
    DispatchError, ResourceEstimate, RuntimeDispatchErrorKind, RuntimeKind, SecretHandle,
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
        }) => {
            assert_eq!(capability, descriptor.id);
            assert!(
                required_secrets.is_empty(),
                "auth_required() handler yields no required handles; got {required_secrets:?}"
            );
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
                kind: RuntimeDispatchErrorKind::Backend
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
