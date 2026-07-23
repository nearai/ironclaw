use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ironclaw_host_api::{
    DispatchError, ExtensionId, InvocationOrigin, ResourceEstimate, RunId,
    RuntimeCredentialAuthRequirement, RuntimeDispatchErrorKind, RuntimeKind, SecretHandle, UserId,
    VendorId,
};
use serde_json::json;

use super::*;

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

type RecordedActorRequest = (ironclaw_host_api::ResourceScope, Option<UserId>);

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
            ironclaw_host_api::ResourceUsage::default(),
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
            ironclaw_host_api::ResourceUsage::default(),
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
        provider: VendorId::new("google").unwrap(),
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

    fn validate_reservation(
        &self,
        reservation: &ironclaw_host_api::ResourceReservation,
    ) -> Result<(), ironclaw_resources::ResourceError> {
        self.inner.validate_reservation(reservation)
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
            usage: ironclaw_host_api::ResourceUsage::default(),
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
        let usage = ironclaw_host_api::ResourceUsage::default().set_output_bytes(64);
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
