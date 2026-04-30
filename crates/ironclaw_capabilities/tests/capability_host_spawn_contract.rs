use std::sync::Mutex;

use async_trait::async_trait;
use ironclaw_approvals::*;
use ironclaw_authorization::*;
use ironclaw_capabilities::*;
use ironclaw_host_api::*;
use ironclaw_processes::*;
use ironclaw_run_state::*;
use serde_json::json;

mod support;
use support::*;

#[tokio::test]
async fn capability_host_blocks_spawn_for_approval_without_starting_process() {
    let registry = registry_with_echo_capability();
    let dispatcher = RecordingDispatcher::default();
    let process_manager = RecordingProcessManager::default();
    let run_state = InMemoryRunStateStore::new();
    let approval_requests = InMemoryApprovalRequestStore::new();
    let host = CapabilityHost::new(&registry, &dispatcher, &SpawnApprovalAuthorizer)
        .with_process_manager(&process_manager)
        .with_run_state(&run_state)
        .with_approval_requests(&approval_requests);
    let context = execution_context(CapabilitySet::default());
    let scope = context.resource_scope.clone();
    let invocation_id = context.invocation_id;
    let estimate = ResourceEstimate::default();
    let input = json!({"message": "background approval"});

    let err = host
        .spawn_json(CapabilitySpawnRequest {
            context,
            capability_id: capability_id(),
            estimate: estimate.clone(),
            input: input.clone(),
            trust_decision: trust_decision(),
        })
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        CapabilityInvocationError::AuthorizationRequiresApproval { .. }
    ));
    assert!(!dispatcher.has_request());
    assert!(!process_manager.has_start());
    let run = run_state.get(&scope, invocation_id).await.unwrap().unwrap();
    assert_eq!(run.status, RunStatus::BlockedApproval);
    let approval_id = run.approval_request_id.unwrap();
    let approval = approval_requests
        .get(&scope, approval_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(approval.status, ApprovalStatus::Pending);
    assert_eq!(
        approval.request.invocation_fingerprint,
        Some(
            InvocationFingerprint::for_spawn(&scope, &capability_id(), &estimate, &input).unwrap()
        )
    );
}

#[tokio::test]
async fn capability_host_resumes_approved_spawn_and_consumes_matching_lease() {
    let registry = registry_with_echo_capability();
    let dispatcher = RecordingDispatcher::default();
    let process_manager = RecordingProcessManager::default();
    let run_state = InMemoryRunStateStore::new();
    let approval_requests = InMemoryApprovalRequestStore::new();
    let leases = InMemoryCapabilityLeaseStore::new();
    let block_host = CapabilityHost::new(&registry, &dispatcher, &SpawnApprovalAuthorizer)
        .with_process_manager(&process_manager)
        .with_run_state(&run_state)
        .with_approval_requests(&approval_requests);
    let context = execution_context(CapabilitySet::default());
    let scope = context.resource_scope.clone();
    let invocation_id = context.invocation_id;
    let estimate = ResourceEstimate::default();
    let input = json!({"message": "approved background"});

    block_host
        .spawn_json(CapabilitySpawnRequest {
            context: context.clone(),
            capability_id: capability_id(),
            estimate: estimate.clone(),
            input: input.clone(),
            trust_decision: trust_decision(),
        })
        .await
        .unwrap_err();
    let approval_id = run_state
        .get(&scope, invocation_id)
        .await
        .unwrap()
        .unwrap()
        .approval_request_id
        .unwrap();
    let lease = ApprovalResolver::new(&approval_requests, &leases)
        .approve_spawn(
            &scope,
            approval_id,
            LeaseApproval {
                issued_by: Principal::HostRuntime,
                allowed_effects: vec![EffectKind::DispatchCapability, EffectKind::SpawnProcess],
                mounts: MountView::default(),
                network: NetworkPolicy::default(),
                secrets: Vec::new(),
                resource_ceiling: None,
                expires_at: None,
                max_invocations: Some(1),
            },
        )
        .await
        .unwrap();

    let resume_authorizer = GrantAuthorizer::new();
    let resume_host = CapabilityHost::new(&registry, &dispatcher, &resume_authorizer)
        .with_process_manager(&process_manager)
        .with_run_state(&run_state)
        .with_approval_requests(&approval_requests)
        .with_capability_leases(&leases);
    let result = resume_host
        .resume_spawn_json(CapabilityResumeRequest {
            context: context.clone(),
            approval_request_id: approval_id,
            capability_id: capability_id(),
            estimate,
            input,
            trust_decision: trust_decision(),
        })
        .await
        .unwrap();

    assert!(!dispatcher.has_request());
    let start = process_manager.take_start();
    assert_eq!(start.scope, context.resource_scope);
    assert_eq!(start.capability_id, capability_id());
    assert!(
        start
            .grants
            .grants
            .iter()
            .any(|grant| grant.id == lease.grant.id),
        "resumed spawned process must inherit the approved lease grant"
    );
    assert_eq!(result.process.process_id, start.process_id);
    let run = run_state.get(&scope, invocation_id).await.unwrap().unwrap();
    assert_eq!(run.status, RunStatus::Completed);
    let consumed = leases.get(&scope, lease.grant.id).await.unwrap();
    assert_eq!(consumed.status, CapabilityLeaseStatus::Consumed);
}

#[tokio::test]
async fn capability_host_denies_spawn_when_trust_ceiling_omits_spawn_effect() {
    let registry = registry_with_echo_capability();
    let dispatcher = RecordingDispatcher::default();
    let process_manager = RecordingProcessManager::default();
    let authorizer = GrantAuthorizer::new();
    let host = CapabilityHost::new(&registry, &dispatcher, &authorizer)
        .with_process_manager(&process_manager);
    let context = execution_context(CapabilitySet {
        grants: vec![spawn_grant()],
    });

    let err = host
        .spawn_json(CapabilitySpawnRequest {
            context,
            capability_id: capability_id(),
            estimate: ResourceEstimate::default(),
            input: json!({"message": "blocked spawn"}),
            trust_decision: trust_decision_with_effects(vec![EffectKind::DispatchCapability]),
        })
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        CapabilityInvocationError::AuthorizationDenied {
            reason: DenyReason::PolicyDenied,
            ..
        }
    ));
    assert!(!dispatcher.has_request());
    assert!(!process_manager.has_start());
}

#[tokio::test]
async fn capability_host_returns_spawn_result_when_run_completion_fails_after_spawn() {
    let registry = registry_with_echo_capability();
    let dispatcher = RecordingDispatcher::default();
    let process_manager = RecordingProcessManager::default();
    let run_state = FailCompleteRunStateStore::new();
    let authorizer = SpawnAuthorizer;
    let host = CapabilityHost::new(&registry, &dispatcher, &authorizer)
        .with_process_manager(&process_manager)
        .with_run_state(&run_state);
    let context = execution_context(CapabilitySet {
        grants: vec![dispatch_grant()],
    });

    let result = host
        .spawn_json(CapabilitySpawnRequest {
            context: context.clone(),
            capability_id: capability_id(),
            estimate: ResourceEstimate::default(),
            input: json!({"message": "background"}),
            trust_decision: trust_decision(),
        })
        .await
        .unwrap();

    assert!(!dispatcher.has_request());
    let start = process_manager.take_start();
    assert_eq!(result.process.process_id, start.process_id);
}

#[tokio::test]
async fn capability_host_spawns_authorized_process_without_dispatching_inline() {
    let registry = registry_with_echo_capability();
    let dispatcher = RecordingDispatcher::default();
    let process_manager = RecordingProcessManager::default();
    let authorizer = SpawnAuthorizer;
    let host = CapabilityHost::new(&registry, &dispatcher, &authorizer)
        .with_process_manager(&process_manager);
    let context = execution_context(CapabilitySet {
        grants: vec![dispatch_grant()],
    });

    let result = host
        .spawn_json(CapabilitySpawnRequest {
            context: context.clone(),
            capability_id: capability_id(),
            estimate: ResourceEstimate::default(),
            input: json!({"message": "background"}),
            trust_decision: trust_decision(),
        })
        .await
        .unwrap();

    assert!(!dispatcher.has_request());
    let start = process_manager.take_start();
    assert_eq!(start.scope, context.resource_scope);
    assert_eq!(start.capability_id, capability_id());
    assert_eq!(start.extension_id, ExtensionId::new("echo").unwrap());
    assert_eq!(start.runtime, RuntimeKind::Wasm);
    assert_eq!(start.input, json!({"message": "background"}));
    assert_eq!(result.process.process_id, start.process_id);
}

#[derive(Default)]
struct RecordingProcessManager {
    start: Mutex<Option<ProcessStart>>,
}

impl RecordingProcessManager {
    fn take_start(&self) -> ProcessStart {
        self.start
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
            .unwrap()
    }

    fn has_start(&self) -> bool {
        self.start
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_some()
    }
}

struct FailCompleteRunStateStore {
    inner: InMemoryRunStateStore,
}

impl FailCompleteRunStateStore {
    fn new() -> Self {
        Self {
            inner: InMemoryRunStateStore::new(),
        }
    }
}

#[async_trait]
impl RunStateStore for FailCompleteRunStateStore {
    async fn start(&self, start: RunStart) -> Result<RunRecord, RunStateError> {
        self.inner.start(start).await
    }

    async fn block_approval(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
        approval: ApprovalRequest,
    ) -> Result<RunRecord, RunStateError> {
        self.inner
            .block_approval(scope, invocation_id, approval)
            .await
    }

    async fn block_auth(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
        error_kind: String,
    ) -> Result<RunRecord, RunStateError> {
        self.inner
            .block_auth(scope, invocation_id, error_kind)
            .await
    }

    async fn complete(
        &self,
        _scope: &ResourceScope,
        _invocation_id: InvocationId,
    ) -> Result<RunRecord, RunStateError> {
        Err(RunStateError::Filesystem(
            "complete transition unavailable".to_string(),
        ))
    }

    async fn fail(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
        error_kind: String,
    ) -> Result<RunRecord, RunStateError> {
        self.inner.fail(scope, invocation_id, error_kind).await
    }

    async fn get(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
    ) -> Result<Option<RunRecord>, RunStateError> {
        self.inner.get(scope, invocation_id).await
    }

    async fn records_for_scope(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<RunRecord>, RunStateError> {
        self.inner.records_for_scope(scope).await
    }
}

#[async_trait]
impl ProcessManager for RecordingProcessManager {
    async fn spawn(&self, start: ProcessStart) -> Result<ProcessRecord, ProcessError> {
        *self
            .start
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(start.clone());
        Ok(ProcessRecord {
            process_id: start.process_id,
            parent_process_id: start.parent_process_id,
            invocation_id: start.invocation_id,
            scope: start.scope,
            extension_id: start.extension_id,
            capability_id: start.capability_id,
            runtime: start.runtime,
            status: ProcessStatus::Running,
            grants: start.grants,
            mounts: start.mounts,
            estimated_resources: start.estimated_resources,
            resource_reservation_id: start.resource_reservation_id,
            error_kind: None,
        })
    }
}

struct SpawnApprovalAuthorizer;

#[async_trait]
impl TrustAwareCapabilityDispatchAuthorizer for SpawnApprovalAuthorizer {
    async fn authorize_dispatch_with_trust(
        &self,
        _context: &ExecutionContext,
        _descriptor: &CapabilityDescriptor,
        _estimate: &ResourceEstimate,
        _trust_decision: &ironclaw_trust::TrustDecision,
    ) -> Decision {
        Decision::Deny {
            reason: DenyReason::MissingGrant,
        }
    }

    async fn authorize_spawn_with_trust(
        &self,
        context: &ExecutionContext,
        _descriptor: &CapabilityDescriptor,
        estimate: &ResourceEstimate,
        _trust_decision: &ironclaw_trust::TrustDecision,
    ) -> Decision {
        Decision::RequireApproval {
            request: ApprovalRequest {
                id: ApprovalRequestId::new(),
                correlation_id: context.correlation_id,
                requested_by: Principal::Extension(context.extension_id.clone()),
                action: Box::new(Action::SpawnCapability {
                    capability: capability_id(),
                    estimated_resources: estimate.clone(),
                }),
                invocation_fingerprint: None,
                reason: "spawn approval required".to_string(),
                reusable_scope: None,
            },
        }
    }
}

struct SpawnAuthorizer;

#[async_trait]
impl TrustAwareCapabilityDispatchAuthorizer for SpawnAuthorizer {
    async fn authorize_dispatch_with_trust(
        &self,
        _context: &ExecutionContext,
        _descriptor: &CapabilityDescriptor,
        _estimate: &ResourceEstimate,
        _trust_decision: &ironclaw_trust::TrustDecision,
    ) -> Decision {
        Decision::Deny {
            reason: DenyReason::MissingGrant,
        }
    }

    async fn authorize_spawn_with_trust(
        &self,
        _context: &ExecutionContext,
        _descriptor: &CapabilityDescriptor,
        _estimate: &ResourceEstimate,
        _trust_decision: &ironclaw_trust::TrustDecision,
    ) -> Decision {
        Decision::Allow {
            obligations: Obligations::empty(),
        }
    }
}
