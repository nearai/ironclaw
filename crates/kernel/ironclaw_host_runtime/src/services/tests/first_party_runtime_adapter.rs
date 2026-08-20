use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ironclaw_filesystem::InMemoryBackend;
use ironclaw_host_api::{
    decision::RuntimeCredentialAuthRequirement,
    dispatch::{DispatchError, RuntimeDispatchErrorKind},
    ids::{ExtensionId, RunId, SecretHandle, UserId, VendorId},
    invocation::InvocationOrigin,
    resource::{ResourceEstimate, ResourceReservation, ResourceScope},
    runtime::RuntimeKind,
};
use serde_json::json;

use super::*;
use crate::invocation_services::{
    InvocationServices, InvocationServicesError, InvocationServicesResolutionRequest,
    InvocationServicesResolver,
};

#[tokio::test]
async fn reply_attachment_builtin_is_discoverable_but_default_handler_fails_closed() {
    let capability_id = ironclaw_host_api::ids::CapabilityId::new(
        crate::first_party_tools::ATTACH_WORKSPACE_FILE_TO_REPLY_CAPABILITY_ID,
    )
    .expect("reply attachment capability id");
    let package =
        crate::first_party_tools::builtin_first_party_package().expect("built-in package");
    assert!(
        package
            .capabilities
            .iter()
            .any(|descriptor| descriptor.id == capability_id),
        "reply attachment capability must be model-discoverable"
    );

    let registry = crate::first_party_tools::builtin_first_party_handlers(Arc::new(
        ironclaw_triggers::InMemoryTriggerRepository::default(),
    ))
    .expect("built-in handlers");
    let handler = registry
        .get(&capability_id)
        .expect("fail-closed reply attachment handler");
    let filesystem = Arc::new(InMemoryBackend::new());
    let target =
        ironclaw_host_api::path::VirtualPath::new("/projects/reply-attachment/report.txt").unwrap();
    filesystem
        .put(
            &target,
            ironclaw_filesystem::Entry::bytes(b"report".to_vec()),
            ironclaw_filesystem::CasExpectation::Absent,
        )
        .await
        .expect("seed workspace file");
    let mounts =
        ironclaw_host_api::mount::MountView::new(vec![ironclaw_host_api::mount::MountGrant::new(
            ironclaw_host_api::path::MountAlias::new("/workspace").unwrap(),
            ironclaw_host_api::path::VirtualPath::new("/projects/reply-attachment").unwrap(),
            ironclaw_host_api::mount::MountPermissions::read_only(),
        )])
        .unwrap();
    let mut request = crate::FirstPartyCapabilityRequest::request_for_test(
        capability_id,
        sample_scope(),
        json!({"path": "/workspace/report.txt"}),
        None,
    );
    request.run_id = Some(RunId::new());
    request.mounts = Some(mounts);
    request.services.filesystem = filesystem;

    let error = handler
        .dispatch(request)
        .await
        .expect_err("default reply attachment port must fail closed");
    assert_eq!(error.kind(), Some(RuntimeDispatchErrorKind::Backend));
}

#[tokio::test]
async fn first_party_handler_receives_authenticated_actor_distinct_from_subject_scope() {
    let descriptor = test_descriptor(RuntimeKind::FirstParty, Vec::new());
    let recorded = Arc::new(Mutex::new(None));
    let registry = Arc::new(FirstPartyCapabilityRegistry::new().with_handler(
        descriptor.id.clone(),
        Arc::new(RecordingActorFirstPartyHandler {
            recorded: Arc::clone(&recorded),
        }),
    ));
    let adapter = FirstPartyRuntimeAdapter::from_registry(
        registry,
        Arc::new(ConfiguredInvocationServicesResolver::new(
            Arc::new(DiskFilesystem::new()),
            None,
            Arc::new(HostProcessPort::new()),
            None,
        )),
    );
    let filesystem = DiskFilesystem::new();
    let governor = InMemoryResourceGovernor::new();
    let mut scope = sample_scope();
    scope.user_id = UserId::new("shared-subject").expect("valid subject user id");
    let package = test_package(WASM_MANIFEST, "test-wasm");
    let policy = policy_with(
        FilesystemBackendKind::HostWorkspace,
        ProcessBackendKind::LocalHost,
        NetworkMode::DirectLogged,
        SecretMode::ScrubbedEnv,
    );

    adapter
        .dispatch_json(RuntimeLaneRequest {
            run_id: None,
            origin: None,
            package: &package,
            descriptor: &descriptor,
            filesystem: &filesystem,
            governor: &governor,
            runtime_policy: &policy,
            capability_id: &descriptor.id,
            scope,
            authenticated_actor_user_id: Some(
                UserId::new("slack-alice").expect("valid authenticated actor user id"),
            ),
            estimate: ResourceEstimate::default(),
            mounts: None,
            resource_reservation: None,
            input: json!({}),
        })
        .await
        .expect("first-party dispatch succeeds");

    let recorded = recorded
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
        .expect("handler recorded the request");
    assert_eq!(recorded.0.user_id.as_str(), "shared-subject");
    assert_eq!(recorded.1.as_ref().map(UserId::as_str), Some("slack-alice"));
}

type RecordedActorRequest = (ironclaw_host_api::resource::ResourceScope, Option<UserId>);

struct RecordingActorFirstPartyHandler {
    recorded: Arc<Mutex<Option<RecordedActorRequest>>>,
}

#[tokio::test]
async fn first_party_adapter_forwards_scheduled_loop_origin_unchanged() {
    let descriptor = test_descriptor(RuntimeKind::FirstParty, Vec::new());
    let recorded = Arc::new(Mutex::new(None));
    let registry = Arc::new(FirstPartyCapabilityRegistry::new().with_handler(
        descriptor.id.clone(),
        Arc::new(RecordingOriginFirstPartyHandler {
            recorded: Arc::clone(&recorded),
        }),
    ));
    let adapter = FirstPartyRuntimeAdapter::from_registry(
        registry,
        Arc::new(ConfiguredInvocationServicesResolver::new(
            Arc::new(DiskFilesystem::new()),
            None,
            Arc::new(HostProcessPort::new()),
            None,
        )),
    );
    let filesystem = DiskFilesystem::new();
    let governor = InMemoryResourceGovernor::new();
    let package = test_package(WASM_MANIFEST, "test-wasm");
    let policy = policy_with(
        FilesystemBackendKind::HostWorkspace,
        ProcessBackendKind::LocalHost,
        NetworkMode::DirectLogged,
        SecretMode::ScrubbedEnv,
    );
    let run_id = RunId::new();
    let origin = InvocationOrigin::ScheduledLoopRun(run_id);

    adapter
        .dispatch_json(RuntimeLaneRequest {
            run_id: Some(run_id),
            origin: Some(origin.clone()),
            package: &package,
            descriptor: &descriptor,
            filesystem: &filesystem,
            governor: &governor,
            runtime_policy: &policy,
            capability_id: &descriptor.id,
            scope: sample_scope(),
            authenticated_actor_user_id: None,
            estimate: ResourceEstimate::default(),
            mounts: None,
            resource_reservation: None,
            input: json!({}),
        })
        .await
        .expect("first-party dispatch succeeds");

    assert_eq!(
        recorded
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone(),
        Some(origin),
        "the runtime adapter must preserve the scheduler-sealed origin"
    );
}

struct RecordingOriginFirstPartyHandler {
    recorded: Arc<Mutex<Option<InvocationOrigin>>>,
}

#[async_trait]
impl crate::FirstPartyCapabilityHandler for RecordingOriginFirstPartyHandler {
    async fn dispatch(
        &self,
        request: crate::FirstPartyCapabilityRequest,
    ) -> Result<crate::FirstPartyCapabilityResult, crate::FirstPartyCapabilityError> {
        *self
            .recorded
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = request.origin;
        Ok(crate::FirstPartyCapabilityResult::new(
            json!({"ok": true}),
            ironclaw_host_api::resource::ResourceUsage::default(),
        ))
    }
}

#[async_trait]
impl crate::FirstPartyCapabilityHandler for RecordingActorFirstPartyHandler {
    async fn dispatch(
        &self,
        request: crate::FirstPartyCapabilityRequest,
    ) -> Result<crate::FirstPartyCapabilityResult, crate::FirstPartyCapabilityError> {
        *self
            .recorded
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) =
            Some((request.scope, request.authenticated_actor_user_id));
        Ok(crate::FirstPartyCapabilityResult::new(
            json!({"ok": true}),
            ironclaw_host_api::resource::ResourceUsage::default(),
        ))
    }
}

#[tokio::test]
async fn first_party_adapter_maps_handler_auth_required_to_dispatch_auth_required() {
    let descriptor = test_descriptor(RuntimeKind::FirstParty, Vec::new());
    let registry = Arc::new(FirstPartyCapabilityRegistry::new().with_handler(
        descriptor.id.clone(),
        Arc::new(AuthRequiredFirstPartyHandler),
    ));
    let adapter = FirstPartyRuntimeAdapter::from_registry(
        registry,
        Arc::new(ConfiguredInvocationServicesResolver::new(
            Arc::new(DiskFilesystem::new()),
            None,
            Arc::new(HostProcessPort::new()),
            None,
        )),
    );
    let filesystem = DiskFilesystem::new();
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
        .dispatch_json(RuntimeLaneRequest {
            run_id: None,
            origin: None,
            package: &package,
            descriptor: &descriptor,
            filesystem: &filesystem,
            governor: &governor,
            runtime_policy: &policy,
            capability_id: &descriptor.id,
            scope,
            authenticated_actor_user_id: None,
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
            requirement,
        }) => {
            assert_eq!(capability, descriptor.id);
            assert!(
                requirement.required_secrets.is_empty(),
                "auth_required() handler yields no required handles; got {:?}",
                requirement.required_secrets
            );
            assert!(requirement.credential_requirements.is_empty());
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
        Arc::new(ConfiguredInvocationServicesResolver::new(
            Arc::new(DiskFilesystem::new()),
            None,
            Arc::new(HostProcessPort::new()),
            None,
        )),
    );
    let filesystem = DiskFilesystem::new();
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
        .dispatch_json(RuntimeLaneRequest {
            run_id: None,
            origin: None,
            package: &package,
            descriptor: &descriptor,
            filesystem: &filesystem,
            governor: &governor,
            runtime_policy: &policy,
            capability_id: &descriptor.id,
            scope,
            authenticated_actor_user_id: None,
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
        Arc::new(ConfiguredInvocationServicesResolver::new(
            Arc::new(DiskFilesystem::new()),
            None,
            Arc::new(HostProcessPort::new()),
            None,
        )),
    );
    let filesystem = DiskFilesystem::new();
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
        .dispatch_json(RuntimeLaneRequest {
            run_id: None,
            origin: None,
            package: &package,
            descriptor: &descriptor,
            filesystem: &filesystem,
            governor: &governor,
            runtime_policy: &policy,
            capability_id: &descriptor.id,
            scope,
            authenticated_actor_user_id: None,
            estimate: ResourceEstimate::default(),
            mounts: None,
            resource_reservation: None,
            input: json!({}),
        })
        .await;

    match result {
        Err(DispatchError::AuthRequired { requirement, .. }) => {
            assert_eq!(requirement.required_secrets, vec![handle]);
        }
        other => panic!("expected AuthRequired, got {other:?}"),
    }
}

#[tokio::test]
async fn first_party_adapter_forwards_credential_requirements_from_auth_required_handler() {
    let requirement = RuntimeCredentialAuthRequirement {
        provider: VendorId::new("google").unwrap(),
        setup: ironclaw_host_api::capability::RuntimeCredentialAccountSetup::OAuth {
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
        Arc::new(ConfiguredInvocationServicesResolver::new(
            Arc::new(DiskFilesystem::new()),
            None,
            Arc::new(HostProcessPort::new()),
            None,
        )),
    );
    let filesystem = DiskFilesystem::new();
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
        .dispatch_json(RuntimeLaneRequest {
            run_id: None,
            origin: None,
            package: &package,
            descriptor: &descriptor,
            filesystem: &filesystem,
            governor: &governor,
            runtime_policy: &policy,
            capability_id: &descriptor.id,
            scope,
            authenticated_actor_user_id: None,
            estimate: ResourceEstimate::default(),
            mounts: None,
            resource_reservation: None,
            input: json!({}),
        })
        .await;

    match result {
        Err(DispatchError::AuthRequired {
            requirement: auth_requirement,
            ..
        }) => {
            assert_eq!(auth_requirement.credential_requirements, vec![requirement]);
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
        Arc::new(ConfiguredInvocationServicesResolver::new(
            Arc::new(DiskFilesystem::new()),
            None,
            Arc::new(HostProcessPort::new()),
            None,
        )),
    );
    let filesystem = DiskFilesystem::new();
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
        .dispatch_json(RuntimeLaneRequest {
            run_id: None,
            origin: None,
            package: &package,
            descriptor: &descriptor,
            filesystem: &filesystem,
            governor: &governor,
            runtime_policy: &policy,
            capability_id: &descriptor.id,
            scope,
            authenticated_actor_user_id: None,
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
        scope: ironclaw_host_api::resource::ResourceScope,
        estimate: ironclaw_host_api::resource::ResourceEstimate,
    ) -> Result<ironclaw_resources::ReservationOutcome, ironclaw_resources::ResourceError> {
        self.inner.reserve_with_outcome(scope, estimate)
    }

    fn reserve_with_id_and_outcome(
        &self,
        scope: ironclaw_host_api::resource::ResourceScope,
        estimate: ironclaw_host_api::resource::ResourceEstimate,
        reservation_id: ironclaw_host_api::ids::ResourceReservationId,
    ) -> Result<ironclaw_resources::ReservationOutcome, ironclaw_resources::ResourceError> {
        self.inner
            .reserve_with_id_and_outcome(scope, estimate, reservation_id)
    }

    fn reconcile(
        &self,
        reservation_id: ironclaw_host_api::ids::ResourceReservationId,
        _actual: ironclaw_host_api::resource::ResourceUsage,
    ) -> Result<ironclaw_host_api::resource::ResourceReceipt, ironclaw_resources::ResourceError>
    {
        Err(ironclaw_resources::ResourceError::UnknownReservation { id: reservation_id })
    }

    fn validate_reservation(
        &self,
        reservation: &ironclaw_host_api::resource::ResourceReservation,
    ) -> Result<(), ironclaw_resources::ResourceError> {
        self.inner.validate_reservation(reservation)
    }

    fn release(
        &self,
        reservation_id: ironclaw_host_api::ids::ResourceReservationId,
    ) -> Result<ironclaw_host_api::resource::ResourceReceipt, ironclaw_resources::ResourceError>
    {
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
        Arc::new(ConfiguredInvocationServicesResolver::new(
            Arc::new(DiskFilesystem::new()),
            None,
            Arc::new(HostProcessPort::new()),
            None,
        )),
    );
    let filesystem = DiskFilesystem::new();
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
        .dispatch_json(RuntimeLaneRequest {
            run_id: None,
            origin: None,
            package: &package,
            descriptor: &descriptor,
            filesystem: &filesystem,
            governor: &governor,
            runtime_policy: &policy,
            capability_id: &descriptor.id,
            scope,
            authenticated_actor_user_id: None,
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

// Test double for issue #7714: reconcile always fails, and the first release
// attempt fails too (the storage-error shape seen when the governor's journal
// is starved). Records every release argument so a test can prove the retry
// carries the same reservation id.
struct ReleaseFailsOnceGovernor {
    inner: InMemoryResourceGovernor,
    released: Mutex<Vec<ironclaw_host_api::ids::ResourceReservationId>>,
}

impl ReleaseFailsOnceGovernor {
    fn new() -> Self {
        Self {
            inner: InMemoryResourceGovernor::new(),
            released: Mutex::new(Vec::new()),
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    fn release_attempts(&self) -> Vec<ironclaw_host_api::ids::ResourceReservationId> {
        self.released.lock().expect("release log").clone()
    }
}

impl ResourceGovernor for ReleaseFailsOnceGovernor {
    fn set_limit(
        &self,
        account: ResourceAccount,
        limits: ironclaw_resources::ResourceLimits,
    ) -> Result<(), ironclaw_resources::ResourceError> {
        self.inner.set_limit(account, limits)
    }

    fn reserve_with_outcome(
        &self,
        scope: ironclaw_host_api::resource::ResourceScope,
        estimate: ironclaw_host_api::resource::ResourceEstimate,
    ) -> Result<ironclaw_resources::ReservationOutcome, ironclaw_resources::ResourceError> {
        self.inner.reserve_with_outcome(scope, estimate)
    }

    fn reserve_with_id_and_outcome(
        &self,
        scope: ironclaw_host_api::resource::ResourceScope,
        estimate: ironclaw_host_api::resource::ResourceEstimate,
        reservation_id: ironclaw_host_api::ids::ResourceReservationId,
    ) -> Result<ironclaw_resources::ReservationOutcome, ironclaw_resources::ResourceError> {
        self.inner
            .reserve_with_id_and_outcome(scope, estimate, reservation_id)
    }

    fn reconcile(
        &self,
        reservation_id: ironclaw_host_api::ids::ResourceReservationId,
        _actual: ironclaw_host_api::resource::ResourceUsage,
    ) -> Result<ironclaw_host_api::resource::ResourceReceipt, ironclaw_resources::ResourceError>
    {
        Err(ironclaw_resources::ResourceError::UnknownReservation { id: reservation_id })
    }

    fn validate_reservation(
        &self,
        reservation: &ironclaw_host_api::resource::ResourceReservation,
    ) -> Result<(), ironclaw_resources::ResourceError> {
        self.inner.validate_reservation(reservation)
    }

    fn release(
        &self,
        reservation_id: ironclaw_host_api::ids::ResourceReservationId,
    ) -> Result<ironclaw_host_api::resource::ResourceReceipt, ironclaw_resources::ResourceError>
    {
        let mut released = self.released.lock().expect("release log");
        released.push(reservation_id);
        if released.len() == 1 {
            return Err(ironclaw_resources::ResourceError::Storage {
                reason: "governor journal unavailable".to_string(),
            });
        }
        drop(released);
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

/// Regression for issue #7714: a release that fails after a reconcile failure
/// used to be logged and forgotten, leaking the reservation permanently. The
/// deferred queue must retry it on the next dispatch, with the same id.
#[tokio::test]
async fn first_party_adapter_retries_a_failed_reservation_release_on_the_next_dispatch() {
    let descriptor = test_descriptor(RuntimeKind::FirstParty, Vec::new());
    let registry = Arc::new(
        FirstPartyCapabilityRegistry::new()
            .with_handler(descriptor.id.clone(), Arc::new(SucceedingFirstPartyHandler)),
    );
    let adapter = FirstPartyRuntimeAdapter::from_registry(
        registry,
        Arc::new(ConfiguredInvocationServicesResolver::new(
            Arc::new(DiskFilesystem::new()),
            None,
            Arc::new(HostProcessPort::new()),
            None,
        )),
    );
    let filesystem = DiskFilesystem::new();
    let governor = ReleaseFailsOnceGovernor::new();
    let scope = sample_scope();
    let tenant_account = ResourceAccount::tenant(scope.tenant_id.clone());
    let package = test_package(WASM_MANIFEST, "test-wasm");
    let policy = policy_with(
        FilesystemBackendKind::HostWorkspace,
        ProcessBackendKind::LocalHost,
        NetworkMode::DirectLogged,
        SecretMode::ScrubbedEnv,
    );
    let lane_request = || RuntimeLaneRequest {
        run_id: None,
        origin: None,
        package: &package,
        descriptor: &descriptor,
        filesystem: &filesystem,
        governor: &governor,
        runtime_policy: &policy,
        capability_id: &descriptor.id,
        scope: scope.clone(),
        authenticated_actor_user_id: None,
        estimate: ResourceEstimate::default(),
        mounts: None,
        resource_reservation: None,
        input: json!({}),
    };

    adapter
        .dispatch_json(lane_request())
        .await
        .expect_err("reconcile failure must fail the dispatch");
    let leaked = governor.release_attempts();
    assert_eq!(leaked.len(), 1, "first release attempt must have happened");

    // A later dispatch is the retry seam.
    adapter
        .dispatch_json(lane_request())
        .await
        .expect_err("reconcile failure must fail the second dispatch too");

    // The retried release succeeds against the in-memory governor, which
    // rejects an unknown or already-released id — so a successful second
    // attempt is proof the reservation was still held and is now settled.
    let attempts = governor.release_attempts();
    assert_eq!(
        attempts.len(),
        3,
        "retry plus the second dispatch's own release, got {attempts:?}"
    );
    assert_eq!(
        attempts[1], attempts[0],
        "the retry must release the same reservation id, got {attempts:?}"
    );
    assert_eq!(
        governor.inner.reserved_for(&tenant_account),
        ResourceTally::default(),
        "no reservation may remain held, got {attempts:?}"
    );
}

/// Resolver that always fails, so a test can drive the service-resolution
/// cleanup branch without depending on which backend combinations happen to be
/// unsupported.
struct FailingInvocationServicesResolver;

impl InvocationServicesResolver for FailingInvocationServicesResolver {
    fn resolve(
        &self,
        _request: InvocationServicesResolutionRequest<'_>,
    ) -> Result<InvocationServices, InvocationServicesError> {
        Err(InvocationServicesError::UnsupportedFilesystemBackend {
            backend: FilesystemBackendKind::HostWorkspace,
        })
    }
}

/// Reserves through the governor and hands back the prepared reservation a
/// dispatch would carry.
fn prepared_reservation<G>(governor: &G, scope: &ResourceScope) -> ResourceReservation
where
    G: ResourceGovernor,
{
    governor
        .reserve_with_outcome(scope.clone(), ResourceEstimate::default())
        .expect("prepared reservation")
        .reservation
}

/// Regression for issue #7714: a prepared reservation abandoned because policy
/// planning failed used to be released best-effort and forgotten. When that
/// release fails the hold leaks permanently, so it must land in the deferred
/// queue like every other cleanup path.
#[tokio::test]
async fn first_party_adapter_defers_a_failed_release_after_planner_failure() {
    // `Network` effect against `NetworkMode::Deny` is what makes planning fail.
    let descriptor = test_descriptor(RuntimeKind::FirstParty, vec![EffectKind::Network]);
    let registry = Arc::new(
        FirstPartyCapabilityRegistry::new()
            .with_handler(descriptor.id.clone(), Arc::new(SucceedingFirstPartyHandler)),
    );
    let adapter = FirstPartyRuntimeAdapter::from_registry(
        registry,
        Arc::new(ConfiguredInvocationServicesResolver::new(
            Arc::new(DiskFilesystem::new()),
            None,
            Arc::new(HostProcessPort::new()),
            None,
        )),
    );
    let filesystem = DiskFilesystem::new();
    let governor = ReleaseFailsOnceGovernor::new();
    let scope = sample_scope();
    let tenant_account = ResourceAccount::tenant(scope.tenant_id.clone());
    let package = test_package(WASM_MANIFEST, "test-wasm");
    let policy = policy_with(
        FilesystemBackendKind::HostWorkspace,
        ProcessBackendKind::LocalHost,
        NetworkMode::Deny,
        SecretMode::ScrubbedEnv,
    );
    let reservation = prepared_reservation(&governor, &scope);
    let request = |reservation: Option<ResourceReservation>| RuntimeLaneRequest {
        run_id: None,
        origin: None,
        package: &package,
        descriptor: &descriptor,
        filesystem: &filesystem,
        governor: &governor,
        runtime_policy: &policy,
        capability_id: &descriptor.id,
        scope: scope.clone(),
        authenticated_actor_user_id: None,
        estimate: ResourceEstimate::default(),
        mounts: None,
        resource_reservation: reservation,
        input: json!({}),
    };

    adapter
        .dispatch_json(request(Some(reservation.clone())))
        .await
        .expect_err("planning must fail");
    assert_eq!(
        governor.release_attempts(),
        vec![reservation.id],
        "the abandoned reservation must at least be attempted"
    );

    // The next dispatch is the retry seam; it carries no reservation of its
    // own, so any further release attempt is the deferred retry.
    adapter
        .dispatch_json(request(None))
        .await
        .expect_err("planning must fail again");
    assert_eq!(
        governor.release_attempts(),
        vec![reservation.id, reservation.id],
        "the failed release must be retried with the same reservation id"
    );
    assert_eq!(
        governor.inner.reserved_for(&tenant_account),
        ResourceTally::default(),
        "no reservation may remain held"
    );
}

/// Same contract as the planner path, for the service-resolution cleanup
/// branch (issue #7714).
#[tokio::test]
async fn first_party_adapter_defers_a_failed_release_after_service_resolution_failure() {
    let descriptor = test_descriptor(RuntimeKind::FirstParty, Vec::new());
    let registry = Arc::new(
        FirstPartyCapabilityRegistry::new()
            .with_handler(descriptor.id.clone(), Arc::new(SucceedingFirstPartyHandler)),
    );
    let adapter = FirstPartyRuntimeAdapter::from_registry(
        registry,
        Arc::new(FailingInvocationServicesResolver),
    );
    let filesystem = DiskFilesystem::new();
    let governor = ReleaseFailsOnceGovernor::new();
    let scope = sample_scope();
    let tenant_account = ResourceAccount::tenant(scope.tenant_id.clone());
    let package = test_package(WASM_MANIFEST, "test-wasm");
    let policy = policy_with(
        FilesystemBackendKind::HostWorkspace,
        ProcessBackendKind::LocalHost,
        NetworkMode::DirectLogged,
        SecretMode::ScrubbedEnv,
    );
    let reservation = prepared_reservation(&governor, &scope);
    let request = |reservation: Option<ResourceReservation>| RuntimeLaneRequest {
        run_id: None,
        origin: None,
        package: &package,
        descriptor: &descriptor,
        filesystem: &filesystem,
        governor: &governor,
        runtime_policy: &policy,
        capability_id: &descriptor.id,
        scope: scope.clone(),
        authenticated_actor_user_id: None,
        estimate: ResourceEstimate::default(),
        mounts: None,
        resource_reservation: reservation,
        input: json!({}),
    };

    adapter
        .dispatch_json(request(Some(reservation.clone())))
        .await
        .expect_err("service resolution must fail");
    assert_eq!(governor.release_attempts(), vec![reservation.id]);

    adapter
        .dispatch_json(request(None))
        .await
        .expect_err("service resolution must fail again");
    assert_eq!(
        governor.release_attempts(),
        vec![reservation.id, reservation.id],
        "the failed release must be retried with the same reservation id"
    );
    assert_eq!(
        governor.inner.reserved_for(&tenant_account),
        ResourceTally::default(),
        "no reservation may remain held"
    );
}

/// Handler that records it was entered, then blocks forever. Lets a test drive
/// the adapter to the `catch_unwind().await` suspend point (the reservation is
/// already taken) and then cancel the future to exercise the cancellation path.
struct BlockingFirstPartyHandler {
    entered: Arc<std::sync::atomic::AtomicBool>,
}

#[async_trait]
impl crate::FirstPartyCapabilityHandler for BlockingFirstPartyHandler {
    async fn dispatch(
        &self,
        _request: crate::FirstPartyCapabilityRequest,
    ) -> Result<crate::FirstPartyCapabilityResult, crate::FirstPartyCapabilityError> {
        self.entered
            .store(true, std::sync::atomic::Ordering::SeqCst);
        // Block forever; the test cancels the dispatch future via timeout.
        std::future::pending::<()>().await;
        unreachable!("pending future never resolves")
    }
}

/// Regression test for the permanent resource-reservation leak.
///
/// The adapter reserves *before* awaiting `handler.dispatch().catch_unwind()`.
/// Before the `ReservationGuard` fix, cancelling the dispatch future mid-await
/// (the turn scheduler does this on user cancel / lease expiry / heartbeat-store
/// timeout) left the reservation in `reserved_by_account` forever — the governor
/// has no TTL/sweep, so the per-scope budget leaked permanently. With the guard,
/// dropping the future runs `Drop`, releasing the reservation.
///
/// We force the cancellation deterministically: the handler signals it was
/// entered (proving the reservation was taken) and then blocks forever; the
/// dispatch future is wrapped in a short `tokio::time::timeout`, whose elapse
/// drops the future at the suspended await.
#[tokio::test]
async fn first_party_adapter_releases_reservation_when_dispatch_future_is_cancelled() {
    let entered = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let descriptor = test_descriptor(RuntimeKind::FirstParty, Vec::new());
    let registry = Arc::new(FirstPartyCapabilityRegistry::new().with_handler(
        descriptor.id.clone(),
        Arc::new(BlockingFirstPartyHandler {
            entered: Arc::clone(&entered),
        }),
    ));
    let adapter = FirstPartyRuntimeAdapter::from_registry(
        registry,
        Arc::new(ConfiguredInvocationServicesResolver::new(
            Arc::new(DiskFilesystem::new()),
            None,
            Arc::new(HostProcessPort::new()),
            None,
        )),
    );
    let filesystem = DiskFilesystem::new();
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
    // Non-zero estimate so the held reservation is observable in the tally.
    let estimate = ResourceEstimate::default().set_output_bytes(128);

    let dispatch = adapter.dispatch_json(RuntimeLaneRequest {
        run_id: None,
        origin: None,
        package: &package,
        descriptor: &descriptor,
        filesystem: &filesystem,
        governor: &governor,
        runtime_policy: &policy,
        capability_id: &descriptor.id,
        scope,
        authenticated_actor_user_id: None,
        estimate,
        mounts: None,
        resource_reservation: None,
        input: json!({}),
    });

    // The handler blocks forever, so the timeout elapses and drops the dispatch
    // future at the await — the cancellation the turn scheduler performs.
    let outcome = tokio::time::timeout(Duration::from_millis(100), dispatch).await;
    assert!(
        outcome.is_err(),
        "the blocking handler must not complete; the timeout must cancel the dispatch future"
    );
    assert!(
        entered.load(std::sync::atomic::Ordering::SeqCst),
        "the handler must have been entered, proving the reservation was taken before the await"
    );

    // The dropped future's `ReservationGuard::drop` must have released the
    // reservation; the per-scope reserved tally returns to baseline.
    assert_eq!(
        governor.reserved_for(&tenant_account),
        ResourceTally::default(),
        "cancelling the dispatch future mid-await must release the reservation, not leak it"
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
            usage: ironclaw_host_api::resource::ResourceUsage::default(),
        })
    }
}

/// Handler that returns `Err(FirstPartyCapabilityError::Dispatch)` with
/// accountable usage, simulating a handler that consumed some resources
/// before failing. Used to exercise the `account_failed` path when the
/// handler error carries usage that `has_accountable_effects` considers
/// accountable (non-zero `output_bytes`).
struct DispatchFailingWithUsageHandler;

#[async_trait]
impl crate::FirstPartyCapabilityHandler for DispatchFailingWithUsageHandler {
    async fn dispatch(
        &self,
        _request: crate::FirstPartyCapabilityRequest,
    ) -> Result<crate::FirstPartyCapabilityResult, crate::FirstPartyCapabilityError> {
        let usage = ironclaw_host_api::resource::ResourceUsage::default().set_output_bytes(64);
        Err(
            crate::FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::OperationFailed)
                .with_usage(usage),
        )
    }
}

/// Regression test for the `account_failed` reconcile-failure branch when the
/// handler returns `Err` WITH accountable usage.
///
/// When `governor.reconcile` fails (simulated by `ReconcileFailingGovernor`):
///   (a) The adapter must return the **original** handler error
///       (`DispatchError::FirstParty { OperationFailed }`) — NOT the
///       `Resource` accounting error that `first_party_resource_error` produces.
///   (b) The reservation must be released (reserved tally returns to baseline),
///       because `account_failed` calls `governor.release` after a reconcile
///       failure.
#[tokio::test]
async fn first_party_adapter_preserves_handler_error_when_account_failed_reconcile_fails() {
    let descriptor = test_descriptor(RuntimeKind::FirstParty, Vec::new());
    let registry = Arc::new(FirstPartyCapabilityRegistry::new().with_handler(
        descriptor.id.clone(),
        Arc::new(DispatchFailingWithUsageHandler),
    ));
    let adapter = FirstPartyRuntimeAdapter::from_registry(
        registry,
        Arc::new(ConfiguredInvocationServicesResolver::new(
            Arc::new(DiskFilesystem::new()),
            None,
            Arc::new(HostProcessPort::new()),
            None,
        )),
    );
    let filesystem = DiskFilesystem::new();
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
        .dispatch_json(RuntimeLaneRequest {
            run_id: None,
            origin: None,
            package: &package,
            descriptor: &descriptor,
            filesystem: &filesystem,
            governor: &governor,
            runtime_policy: &policy,
            capability_id: &descriptor.id,
            scope,
            authenticated_actor_user_id: None,
            estimate: ResourceEstimate::default(),
            mounts: None,
            resource_reservation: None,
            input: json!({}),
        })
        .await;

    // (a) Must return the original handler error — NOT DispatchError::FirstParty{Resource}.
    assert!(
        matches!(
            result,
            Err(DispatchError::FirstParty {
                kind: RuntimeDispatchErrorKind::OperationFailed,
                ..
            })
        ),
        "adapter must preserve the original handler DispatchError kind when account_failed \
         reconcile fails; got {result:?}"
    );

    // (b) The reservation must not leak: release() is called by account_failed
    // after a reconcile failure, so the reserved tally returns to baseline.
    assert_eq!(
        governor.inner.reserved_for(&tenant_account),
        ResourceTally::default(),
        "reservation must be released when account_failed reconcile fails"
    );
}
