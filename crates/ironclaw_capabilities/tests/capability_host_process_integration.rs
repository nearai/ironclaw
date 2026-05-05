use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use async_trait::async_trait;
use ironclaw_authorization::*;
use ironclaw_capabilities::*;
use ironclaw_host_api::*;
use ironclaw_processes::*;
use ironclaw_run_state::*;
use serde_json::json;

mod support;
use support::*;

#[tokio::test]
async fn capability_host_spawn_runs_background_process_through_process_host() {
    let registry = registry_with_echo_capability();
    let dispatcher = RecordingDispatcher::default();
    let run_state = InMemoryRunStateStore::new();
    let process_services = ProcessServices::in_memory();
    let executor = Arc::new(RecordingSuccessExecutor::default());
    let process_manager = process_services.background_manager(Arc::clone(&executor));
    let process_host = process_services
        .host()
        .with_poll_interval(Duration::from_millis(5));
    let authorizer = SpawnOnlyAuthorizer;
    let host = CapabilityHost::new(&registry, &dispatcher, &authorizer)
        .with_run_state(&run_state)
        .with_process_manager(&process_manager);
    let parent_process_id = ProcessId::new();
    let context = execution_context_with_mounts_and_parent(
        CapabilitySet {
            grants: vec![spawn_grant()],
        },
        scoped_mounts(),
        Some(parent_process_id),
    );
    let scope = context.resource_scope.clone();
    let invocation_id = context.invocation_id;
    let mounts = context.mounts.clone();
    let grants = context.grants.clone();
    let estimate = ResourceEstimate {
        output_bytes: Some(2_048),
        process_count: Some(1),
        ..ResourceEstimate::default()
    };
    let input = json!({"message":"background"});

    let spawned = host
        .spawn_json(CapabilitySpawnRequest {
            context: context.clone(),
            capability_id: capability_id(),
            estimate: estimate.clone(),
            input: input.clone(),
            trust_decision: trust_decision(),
        })
        .await
        .unwrap();

    assert!(!dispatcher.has_request());
    assert_eq!(spawned.process.status, ProcessStatus::Running);
    assert_eq!(spawned.process.parent_process_id, Some(parent_process_id));
    assert_eq!(spawned.process.invocation_id, invocation_id);
    assert_eq!(spawned.process.scope, scope);
    assert_eq!(spawned.process.extension_id, extension_id());
    assert_eq!(spawned.process.capability_id, capability_id());
    assert_eq!(spawned.process.runtime, RuntimeKind::Wasm);
    assert_eq!(spawned.process.grants, grants);
    assert_eq!(spawned.process.mounts, mounts);
    assert_eq!(spawned.process.estimated_resources, estimate);

    let process_id = spawned.process.process_id;
    let result = process_host.await_result(&scope, process_id).await.unwrap();
    assert_eq!(result.status, ProcessStatus::Completed);
    assert_eq!(result.output, Some(json!({"process":"done"})));
    assert_eq!(
        process_host
            .status(&scope, process_id)
            .await
            .unwrap()
            .unwrap()
            .status,
        ProcessStatus::Completed
    );
    assert_eq!(
        process_host.output(&scope, process_id).await.unwrap(),
        Some(json!({"process":"done"}))
    );
    let execution = executor.take_request();
    assert_eq!(execution.process_id, process_id);
    assert_eq!(execution.invocation_id, invocation_id);
    assert_eq!(execution.scope, scope);
    assert_eq!(execution.extension_id, extension_id());
    assert_eq!(execution.capability_id, capability_id());
    assert_eq!(execution.runtime, RuntimeKind::Wasm);
    assert_eq!(execution.estimate, estimate);
    assert_eq!(execution.input, input);
    assert_eq!(
        run_state
            .get(&scope, invocation_id)
            .await
            .unwrap()
            .unwrap()
            .status,
        RunStatus::Completed
    );
}

#[tokio::test]
async fn capability_spawn_process_host_hides_cross_scope_status_and_output() {
    let registry = registry_with_echo_capability();
    let dispatcher = RecordingDispatcher::default();
    let process_services = ProcessServices::in_memory();
    let executor = Arc::new(RecordingSuccessExecutor::default());
    let process_manager = process_services.background_manager(Arc::clone(&executor));
    let process_host = process_services
        .host()
        .with_poll_interval(Duration::from_millis(5));
    let authorizer = SpawnOnlyAuthorizer;
    let host = CapabilityHost::new(&registry, &dispatcher, &authorizer)
        .with_process_manager(&process_manager);
    let context = execution_context(CapabilitySet {
        grants: vec![spawn_grant()],
    });
    let scope = context.resource_scope.clone();
    let wrong_user_scope = scope_for_user_with_invocation("other-user", context.invocation_id);
    let wrong_project_scope =
        scope_with_project_mission_thread(&scope, "other-project", "other-mission", "other-thread");

    let spawned = host
        .spawn_json(CapabilitySpawnRequest {
            context,
            capability_id: capability_id(),
            estimate: ResourceEstimate::default(),
            input: json!({"message":"private"}),
            trust_decision: trust_decision(),
        })
        .await
        .unwrap();
    let process_id = spawned.process.process_id;
    process_host.await_result(&scope, process_id).await.unwrap();

    assert_process_hidden(&process_host, &wrong_user_scope, process_id).await;
    assert_process_hidden(&process_host, &wrong_project_scope, process_id).await;
    assert_eq!(
        process_host.output(&scope, process_id).await.unwrap(),
        Some(json!({"process":"done"}))
    );
}

#[tokio::test]
async fn capability_host_spawn_fails_closed_on_unsupported_obligations_before_process_start() {
    let registry = registry_with_echo_capability();
    let dispatcher = RecordingDispatcher::default();
    let run_state = InMemoryRunStateStore::new();
    let process_services = ProcessServices::in_memory();
    let executor = Arc::new(RecordingSuccessExecutor::default());
    let process_manager = process_services.background_manager(Arc::clone(&executor));
    let host = CapabilityHost::new(&registry, &dispatcher, &SpawnObligatingAuthorizer)
        .with_run_state(&run_state)
        .with_process_manager(&process_manager);
    let context = execution_context(CapabilitySet {
        grants: vec![spawn_grant()],
    });
    let scope = context.resource_scope.clone();
    let invocation_id = context.invocation_id;

    let err = host
        .spawn_json(CapabilitySpawnRequest {
            context,
            capability_id: capability_id(),
            estimate: ResourceEstimate::default(),
            input: json!({"message":"must not spawn"}),
            trust_decision: trust_decision(),
        })
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        CapabilityInvocationError::UnsupportedObligations { .. }
    ));
    assert!(!dispatcher.has_request());
    assert!(executor.take_request_opt().is_none());
    assert!(
        process_services
            .process_store()
            .records_for_scope(&scope)
            .await
            .unwrap()
            .is_empty()
    );
    let run = run_state.get(&scope, invocation_id).await.unwrap().unwrap();
    assert_eq!(run.status, RunStatus::Failed);
    assert_eq!(run.error_kind.as_deref(), Some("UnsupportedObligations"));
}

#[derive(Default)]
struct RecordingSuccessExecutor {
    request: Mutex<Option<ProcessExecutionRequest>>,
}

impl RecordingSuccessExecutor {
    fn take_request(&self) -> ProcessExecutionRequest {
        self.take_request_opt().unwrap()
    }

    fn take_request_opt(&self) -> Option<ProcessExecutionRequest> {
        self.request.lock().unwrap().take()
    }
}

#[async_trait]
impl ProcessExecutor for RecordingSuccessExecutor {
    async fn execute(
        &self,
        request: ProcessExecutionRequest,
    ) -> Result<ProcessExecutionResult, ProcessExecutionError> {
        *self.request.lock().unwrap() = Some(request);
        Ok(ProcessExecutionResult {
            output: json!({"process":"done"}),
        })
    }
}

struct SpawnOnlyAuthorizer;

#[async_trait]
impl TrustAwareCapabilityDispatchAuthorizer for SpawnOnlyAuthorizer {
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

struct SpawnObligatingAuthorizer;

#[async_trait]
impl TrustAwareCapabilityDispatchAuthorizer for SpawnObligatingAuthorizer {
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
            obligations: Obligations::new(vec![Obligation::AuditBefore]).unwrap(),
        }
    }
}

fn execution_context_with_mounts_and_parent(
    grants: CapabilitySet,
    mounts: MountView,
    parent_process_id: Option<ProcessId>,
) -> ExecutionContext {
    let mut context = ExecutionContext::local_default(
        UserId::new("user").unwrap(),
        ExtensionId::new("caller").unwrap(),
        RuntimeKind::Wasm,
        TrustClass::UserTrusted,
        grants,
        mounts,
    )
    .unwrap();
    context.process_id = parent_process_id;
    context.validate().unwrap();
    context
}

fn scoped_mounts() -> MountView {
    MountView::new(vec![MountGrant::new(
        MountAlias::new("/workspace").unwrap(),
        VirtualPath::new("/projects/project-a").unwrap(),
        MountPermissions::read_only(),
    )])
    .unwrap()
}

fn scope_for_user_with_invocation(user: &str, invocation_id: InvocationId) -> ResourceScope {
    ResourceScope::local_default(UserId::new(user).unwrap(), invocation_id).unwrap()
}

fn scope_with_project_mission_thread(
    scope: &ResourceScope,
    project: &str,
    mission: &str,
    thread: &str,
) -> ResourceScope {
    let mut scope = scope.clone();
    scope.project_id = Some(ProjectId::new(project).unwrap());
    scope.mission_id = Some(MissionId::new(mission).unwrap());
    scope.thread_id = Some(ThreadId::new(thread).unwrap());
    scope
}

async fn assert_process_hidden(
    process_host: &ProcessHost<'_>,
    scope: &ResourceScope,
    process_id: ProcessId,
) {
    assert!(
        process_host
            .status(scope, process_id)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        process_host
            .result(scope, process_id)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        process_host
            .output(scope, process_id)
            .await
            .unwrap()
            .is_none()
    );
}
