use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{Duration as ChronoDuration, Utc};
use ironclaw_approvals::{ApprovalResolver, LeaseApproval};
use ironclaw_authorization::*;
use ironclaw_capabilities::*;
use ironclaw_dispatcher::{
    RuntimeAdapter, RuntimeAdapterRequest, RuntimeAdapterResult, RuntimeDispatcher,
};
use ironclaw_events::{InMemoryEventSink, RuntimeEventKind};
use ironclaw_filesystem::LocalFilesystem;
use ironclaw_host_api::*;
use ironclaw_resources::*;
use ironclaw_run_state::*;
use serde_json::{Value, json};

mod support;
use support::*;

#[tokio::test]
async fn capability_host_invokes_through_runtime_dispatcher_and_completes_run() {
    let adapter = Arc::new(RecordingRuntimeAdapter::new(
        json!({"via":"runtime-dispatcher"}),
    ));
    let (registry, dispatcher, governor, events) = runtime_dispatcher_stack(Arc::clone(&adapter));
    let run_state = InMemoryRunStateStore::new();
    let authorizer = GrantAuthorizer::new();
    let host =
        CapabilityHost::new(registry.as_ref(), &dispatcher, &authorizer).with_run_state(&run_state);
    let context = execution_context(CapabilitySet {
        grants: vec![dispatch_grant()],
    });
    let scope = context.resource_scope.clone();
    let invocation_id = context.invocation_id;
    let estimate = ResourceEstimate {
        output_bytes: Some(4_096),
        ..ResourceEstimate::default()
    };
    let input = json!({"message":"authorized"});

    let result = host
        .invoke_json(CapabilityInvocationRequest {
            context,
            capability_id: capability_id(),
            estimate: estimate.clone(),
            input: input.clone(),
            trust_decision: trust_decision(),
        })
        .await
        .unwrap();

    assert_eq!(result.dispatch.output, json!({"via":"runtime-dispatcher"}));
    let recorded = adapter.take_request();
    assert_eq!(recorded.capability_id, capability_id());
    assert_eq!(recorded.scope, scope);
    assert_eq!(recorded.estimate, estimate);
    assert_eq!(recorded.input, input);
    assert_eq!(recorded.mounts, None);
    assert_eq!(recorded.resource_reservation, None);
    assert_eq!(
        run_state
            .get(&recorded.scope, invocation_id)
            .await
            .unwrap()
            .unwrap()
            .status,
        RunStatus::Completed
    );
    assert_eq!(
        governor.reserved_for(&ResourceAccount::tenant(recorded.scope.tenant_id.clone())),
        ResourceTally::default()
    );
    assert!(
        governor
            .usage_for(&ResourceAccount::tenant(recorded.scope.tenant_id.clone()))
            .output_bytes
            > 0
    );
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
async fn capability_host_blocks_then_resumes_approved_dispatch_through_runtime_dispatcher() {
    let adapter = Arc::new(RecordingRuntimeAdapter::new(json!({"approved":true})));
    let (registry, dispatcher, _governor, events) = runtime_dispatcher_stack(Arc::clone(&adapter));
    let run_state = InMemoryRunStateStore::new();
    let approval_requests = InMemoryApprovalRequestStore::new();
    let leases = InMemoryCapabilityLeaseStore::new();
    let block_host = CapabilityHost::new(registry.as_ref(), &dispatcher, &ApprovalAuthorizer)
        .with_run_state(&run_state)
        .with_approval_requests(&approval_requests);
    let context = execution_context(CapabilitySet::default());
    let scope = context.resource_scope.clone();
    let invocation_id = context.invocation_id;
    let estimate = ResourceEstimate {
        output_bytes: Some(1_024),
        ..ResourceEstimate::default()
    };
    let input = json!({"message":"approved"});

    let err = block_host
        .invoke_json(CapabilityInvocationRequest {
            context: context.clone(),
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
    assert_eq!(adapter.request_count(), 0);
    let blocked = run_state.get(&scope, invocation_id).await.unwrap().unwrap();
    assert_eq!(blocked.status, RunStatus::BlockedApproval);
    let approval_id = blocked.approval_request_id.unwrap();
    let lease = approve_dispatch(&approval_requests, &leases, &scope, approval_id, None)
        .await
        .unwrap();

    let resume_authorizer = GrantAuthorizer::new();
    let resume_host = CapabilityHost::new(registry.as_ref(), &dispatcher, &resume_authorizer)
        .with_run_state(&run_state)
        .with_approval_requests(&approval_requests)
        .with_capability_leases(&leases);
    let result = resume_host
        .resume_json(CapabilityResumeRequest {
            context: context.clone(),
            approval_request_id: approval_id,
            capability_id: capability_id(),
            estimate: estimate.clone(),
            input: input.clone(),
            trust_decision: trust_decision(),
        })
        .await
        .unwrap();

    assert_eq!(result.dispatch.output, json!({"approved":true}));
    assert_eq!(adapter.request_count(), 1);
    let recorded = adapter.take_request();
    assert_eq!(recorded.scope, scope);
    assert_eq!(recorded.estimate, estimate);
    assert_eq!(recorded.input, input);
    assert_eq!(
        run_state
            .get(&scope, invocation_id)
            .await
            .unwrap()
            .unwrap()
            .status,
        RunStatus::Completed
    );
    assert_eq!(
        leases.get(&scope, lease.grant.id).await.unwrap().status,
        CapabilityLeaseStatus::Consumed
    );
    assert_event_kinds(
        &events,
        &[
            RuntimeEventKind::DispatchRequested,
            RuntimeEventKind::RuntimeSelected,
            RuntimeEventKind::DispatchSucceeded,
        ],
    );

    let second_err = resume_host
        .resume_json(CapabilityResumeRequest {
            context,
            approval_request_id: approval_id,
            capability_id: capability_id(),
            estimate: ResourceEstimate {
                output_bytes: Some(1_024),
                ..ResourceEstimate::default()
            },
            input: json!({"message":"approved"}),
            trust_decision: trust_decision(),
        })
        .await
        .unwrap_err();
    assert!(matches!(
        second_err,
        CapabilityInvocationError::ResumeNotBlocked {
            status: RunStatus::Completed,
            ..
        }
    ));
    assert_eq!(adapter.request_count(), 0);
}

#[tokio::test]
async fn capability_host_rejects_resume_from_wrong_user_scope_without_dispatch_or_lease_claim() {
    let adapter = Arc::new(RecordingRuntimeAdapter::new(json!({"must_not":"dispatch"})));
    let (registry, dispatcher, _governor, _events) = runtime_dispatcher_stack(Arc::clone(&adapter));
    let run_state = InMemoryRunStateStore::new();
    let approval_requests = InMemoryApprovalRequestStore::new();
    let leases = InMemoryCapabilityLeaseStore::new();
    let block_host = CapabilityHost::new(registry.as_ref(), &dispatcher, &ApprovalAuthorizer)
        .with_run_state(&run_state)
        .with_approval_requests(&approval_requests);
    let context = execution_context(CapabilitySet::default());
    let scope = context.resource_scope.clone();
    let invocation_id = context.invocation_id;
    let estimate = ResourceEstimate::default();
    let input = json!({"message":"scoped"});

    block_host
        .invoke_json(CapabilityInvocationRequest {
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
    let lease = approve_dispatch(&approval_requests, &leases, &scope, approval_id, None)
        .await
        .unwrap();
    let wrong_context = context_for_user_with_invocation("other-user", invocation_id);

    let resume_authorizer = GrantAuthorizer::new();
    let resume_host = CapabilityHost::new(registry.as_ref(), &dispatcher, &resume_authorizer)
        .with_run_state(&run_state)
        .with_approval_requests(&approval_requests)
        .with_capability_leases(&leases);
    let err = resume_host
        .resume_json(CapabilityResumeRequest {
            context: wrong_context.clone(),
            approval_request_id: approval_id,
            capability_id: capability_id(),
            estimate,
            input,
            trust_decision: trust_decision(),
        })
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        CapabilityInvocationError::RunState(error)
            if matches!(*error, RunStateError::UnknownInvocation { .. })
    ));
    assert_eq!(adapter.request_count(), 0);
    assert!(
        approval_requests
            .get(&wrong_context.resource_scope, approval_id)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        leases
            .get(&wrong_context.resource_scope, lease.grant.id)
            .await
            .is_none()
    );
    assert_eq!(
        leases.get(&scope, lease.grant.id).await.unwrap().status,
        CapabilityLeaseStatus::Active
    );
    let original_run = run_state.get(&scope, invocation_id).await.unwrap().unwrap();
    assert_eq!(original_run.status, RunStatus::BlockedApproval);
}

#[tokio::test]
async fn capability_host_rejects_expired_approval_lease_before_dispatch() {
    let adapter = Arc::new(RecordingRuntimeAdapter::new(json!({"must_not":"dispatch"})));
    let (registry, dispatcher, _governor, _events) = runtime_dispatcher_stack(Arc::clone(&adapter));
    let run_state = InMemoryRunStateStore::new();
    let approval_requests = InMemoryApprovalRequestStore::new();
    let leases = InMemoryCapabilityLeaseStore::new();
    let block_host = CapabilityHost::new(registry.as_ref(), &dispatcher, &ApprovalAuthorizer)
        .with_run_state(&run_state)
        .with_approval_requests(&approval_requests);
    let context = execution_context(CapabilitySet::default());
    let scope = context.resource_scope.clone();
    let invocation_id = context.invocation_id;
    let estimate = ResourceEstimate::default();
    let input = json!({"message":"expired lease"});

    block_host
        .invoke_json(CapabilityInvocationRequest {
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
    let lease = approve_dispatch(
        &approval_requests,
        &leases,
        &scope,
        approval_id,
        Some(Utc::now() - ChronoDuration::seconds(1)),
    )
    .await
    .unwrap();

    let resume_authorizer = GrantAuthorizer::new();
    let resume_host = CapabilityHost::new(registry.as_ref(), &dispatcher, &resume_authorizer)
        .with_run_state(&run_state)
        .with_approval_requests(&approval_requests)
        .with_capability_leases(&leases);
    let err = resume_host
        .resume_json(CapabilityResumeRequest {
            context,
            approval_request_id: approval_id,
            capability_id: capability_id(),
            estimate,
            input,
            trust_decision: trust_decision(),
        })
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        CapabilityInvocationError::ApprovalLeaseMissing { .. }
    ));
    assert_eq!(adapter.request_count(), 0);
    assert_eq!(
        leases.get(&scope, lease.grant.id).await.unwrap().status,
        CapabilityLeaseStatus::Active
    );
    let run = run_state.get(&scope, invocation_id).await.unwrap().unwrap();
    assert_eq!(run.status, RunStatus::Failed);
    assert_eq!(run.error_kind.as_deref(), Some("ApprovalLeaseMissing"));
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

fn runtime_dispatcher_stack(
    adapter: Arc<RecordingRuntimeAdapter>,
) -> (
    Arc<ironclaw_extensions::ExtensionRegistry>,
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

async fn approve_dispatch(
    approval_requests: &InMemoryApprovalRequestStore,
    leases: &InMemoryCapabilityLeaseStore,
    scope: &ResourceScope,
    approval_id: ApprovalRequestId,
    expires_at: Option<Timestamp>,
) -> Result<CapabilityLease, ironclaw_approvals::ApprovalResolutionError> {
    ApprovalResolver::new(approval_requests, leases)
        .approve_dispatch(
            scope,
            approval_id,
            LeaseApproval {
                issued_by: Principal::HostRuntime,
                allowed_effects: vec![EffectKind::DispatchCapability],
                mounts: MountView::default(),
                network: NetworkPolicy::default(),
                secrets: Vec::new(),
                resource_ceiling: None,
                expires_at,
                max_invocations: Some(1),
            },
        )
        .await
}

fn context_for_user_with_invocation(user: &str, invocation_id: InvocationId) -> ExecutionContext {
    let user_id = UserId::new(user).unwrap();
    let resource_scope = ResourceScope::local_default(user_id.clone(), invocation_id).unwrap();
    let mut context = ExecutionContext::local_default(
        user_id,
        ExtensionId::new("caller").unwrap(),
        RuntimeKind::Wasm,
        TrustClass::UserTrusted,
        CapabilitySet::default(),
        MountView::default(),
    )
    .unwrap();
    context.invocation_id = invocation_id;
    context.tenant_id = resource_scope.tenant_id.clone();
    context.user_id = resource_scope.user_id.clone();
    context.agent_id = resource_scope.agent_id.clone();
    context.project_id = resource_scope.project_id.clone();
    context.mission_id = resource_scope.mission_id.clone();
    context.thread_id = resource_scope.thread_id.clone();
    context.resource_scope = resource_scope;
    context.validate().unwrap();
    context
}

fn assert_event_kinds(events: &InMemoryEventSink, expected: &[RuntimeEventKind]) {
    let actual = events
        .events()
        .into_iter()
        .map(|event| event.kind)
        .collect::<Vec<_>>();
    assert_eq!(actual, expected);
}
