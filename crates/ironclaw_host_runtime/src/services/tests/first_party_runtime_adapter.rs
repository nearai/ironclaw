use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::{DispatchError, ResourceEstimate, RuntimeKind};
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

    assert!(matches!(
        result,
        Err(DispatchError::AuthRequired { capability, .. }) if capability == descriptor.id
    ));
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
