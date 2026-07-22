use std::{sync::Arc, time::Instant};

use async_trait::async_trait;
use ironclaw_capabilities::ProcessAuthorizationRemintPort;
use ironclaw_host_api::{CapabilityDispatcher, RuntimeKind};
use ironclaw_observability::live_latency_started_at;
use ironclaw_processes::{
    ProcessExecutionError, ProcessExecutionRequest, ProcessExecutionResult, ProcessExecutor,
};

struct ProcessLatencyFields {
    capability_id: String,
    runtime: String,
    tenant_id: String,
    user_id: String,
    agent_id: String,
    project_id: String,
    mission_id: String,
    thread_id: String,
    invocation_id: String,
}

impl ProcessLatencyFields {
    fn from_request(
        started_at: Option<Instant>,
        request: &ProcessExecutionRequest,
    ) -> Option<Self> {
        started_at?;
        Some(Self {
            capability_id: request.capability_id.to_string(),
            runtime: format!("{:?}", request.runtime),
            tenant_id: request.scope.tenant_id.as_str().to_string(),
            user_id: request.scope.user_id.as_str().to_string(),
            agent_id: request
                .scope
                .agent_id
                .as_ref()
                .map(|id| id.as_str().to_string())
                .unwrap_or_default(),
            project_id: request
                .scope
                .project_id
                .as_ref()
                .map(|id| id.as_str().to_string())
                .unwrap_or_default(),
            mission_id: request
                .scope
                .mission_id
                .as_ref()
                .map(|id| id.as_str().to_string())
                .unwrap_or_default(),
            thread_id: request
                .scope
                .thread_id
                .as_ref()
                .map(|id| id.as_str().to_string())
                .unwrap_or_default(),
            invocation_id: request.scope.invocation_id.to_string(),
        })
    }
}

fn trace_process_latency_ok(
    operation: &'static str,
    fields: Option<&ProcessLatencyFields>,
    started_at: Option<Instant>,
) {
    let (Some(fields), Some(started_at)) = (fields, started_at) else {
        return;
    };

    ironclaw_observability::live_latency_trace_ok!(
        "process_executor",
        operation,
        Some(started_at),
        capability_id = fields.capability_id.as_str(),
        runtime = fields.runtime.as_str(),
        tenant_id = fields.tenant_id.as_str(),
        user_id = fields.user_id.as_str(),
        agent_id = fields.agent_id.as_str(),
        project_id = fields.project_id.as_str(),
        mission_id = fields.mission_id.as_str(),
        thread_id = fields.thread_id.as_str(),
        invocation_id = fields.invocation_id.as_str(),
        "process execution operation completed",
    );
}

fn trace_process_latency_error<E: ?Sized>(
    operation: &'static str,
    fields: Option<&ProcessLatencyFields>,
    started_at: Option<Instant>,
    _error: &E,
) {
    let (Some(fields), Some(started_at)) = (fields, started_at) else {
        return;
    };

    ironclaw_observability::live_latency_trace_error!(
        "process_executor",
        operation,
        Some(started_at),
        "process_execution_error",
        capability_id = fields.capability_id.as_str(),
        runtime = fields.runtime.as_str(),
        tenant_id = fields.tenant_id.as_str(),
        user_id = fields.user_id.as_str(),
        agent_id = fields.agent_id.as_str(),
        project_id = fields.project_id.as_str(),
        mission_id = fields.mission_id.as_str(),
        thread_id = fields.thread_id.as_str(),
        invocation_id = fields.invocation_id.as_str(),
        "process execution operation failed",
    );
}

#[derive(Clone)]
pub(super) struct HostProcessExecutor {
    dispatch_executor: Arc<dyn ProcessExecutor>,
    process_sandbox_executor: Option<Arc<dyn ProcessExecutor>>,
}

impl HostProcessExecutor {
    pub(super) fn new(
        dispatch_executor: Arc<dyn ProcessExecutor>,
        process_sandbox_executor: Option<Arc<dyn ProcessExecutor>>,
    ) -> Self {
        Self {
            dispatch_executor,
            process_sandbox_executor,
        }
    }
}

#[async_trait]
impl ProcessExecutor for HostProcessExecutor {
    async fn execute(
        &self,
        request: ProcessExecutionRequest,
    ) -> Result<ProcessExecutionResult, ProcessExecutionError> {
        let started_at = live_latency_started_at();
        let fields = ProcessLatencyFields::from_request(started_at, &request);
        if is_process_sandbox_request(&request) {
            let Some(executor) = &self.process_sandbox_executor else {
                let error = ProcessExecutionError::new("missing_process_sandbox_executor");
                trace_process_latency_error(
                    "host_process_execute",
                    fields.as_ref(),
                    started_at,
                    &error,
                );
                return Err(error);
            };
            let result = executor.execute(request).await;
            match &result {
                Ok(_) => {
                    trace_process_latency_ok("host_process_execute", fields.as_ref(), started_at)
                }
                Err(error) => trace_process_latency_error(
                    "host_process_execute",
                    fields.as_ref(),
                    started_at,
                    error,
                ),
            }
            return result;
        }
        let result = self.dispatch_executor.execute(request).await;
        match &result {
            Ok(_) => trace_process_latency_ok("host_process_execute", fields.as_ref(), started_at),
            Err(error) => trace_process_latency_error(
                "host_process_execute",
                fields.as_ref(),
                started_at,
                error,
            ),
        }
        result
    }
}

fn is_process_sandbox_request(request: &ProcessExecutionRequest) -> bool {
    request.runtime == RuntimeKind::System
        && request.capability_id.as_str() == ironclaw_process_sandbox::PROCESS_SANDBOX_CAPABILITY_ID
}

#[derive(Clone)]
pub(super) struct RuntimeDispatchProcessExecutor {
    dispatcher: Arc<dyn CapabilityDispatcher>,
    reminter: Arc<dyn ProcessAuthorizationRemintPort>,
}

impl RuntimeDispatchProcessExecutor {
    pub(super) fn new(
        dispatcher: Arc<dyn CapabilityDispatcher>,
        reminter: Arc<dyn ProcessAuthorizationRemintPort>,
    ) -> Self {
        Self {
            dispatcher,
            reminter,
        }
    }
}

#[async_trait]
impl ProcessExecutor for RuntimeDispatchProcessExecutor {
    async fn execute(
        &self,
        request: ProcessExecutionRequest,
    ) -> Result<ProcessExecutionResult, ProcessExecutionError> {
        let started_at = live_latency_started_at();
        let fields = ProcessLatencyFields::from_request(started_at, &request);
        if request.cancellation.is_cancelled() {
            let error = ProcessExecutionError::new("cancelled");
            trace_process_latency_error(
                "runtime_dispatch_execute",
                fields.as_ref(),
                started_at,
                &error,
            );
            return Err(error);
        }
        let authorized = match self.reminter.remint(&request).await {
            Ok(authorized) => authorized,
            Err(error) => {
                let error_kind = error.kind();
                trace_process_latency_error(
                    "runtime_dispatch_execute",
                    fields.as_ref(),
                    started_at,
                    &error,
                );
                return Err(ProcessExecutionError::new(error_kind));
            }
        };
        let result = self
            .dispatcher
            .dispatch_json(authorized)
            .await
            .map_err(|error| {
                trace_process_latency_error(
                    "runtime_dispatch_execute",
                    fields.as_ref(),
                    started_at,
                    &error,
                );
                ProcessExecutionError::new(error.event_kind())
            })?;
        let result = Ok(ProcessExecutionResult {
            output: result.output,
        });
        trace_process_latency_ok("runtime_dispatch_execute", fields.as_ref(), started_at);
        result
    }
}

// Removed: dispatch_error_kind was a local copy of DispatchError::event_kind() from ironclaw_host_api.
// Call error.event_kind() directly instead.

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    use ironclaw_host_api::dispatch_test_support::TestDispatcher;
    use ironclaw_host_api::{
        ActivityId, Actor, AgentId, CapabilityDispatchResult, CapabilityId, CorrelationId,
        DispatchError, ExtensionId, InvocationId, InvocationOrigin, MountView,
        ProcessAuthorizedContinuation, ProcessAuthorizedInvocation, ProcessId, ProductKind,
        ProjectId, ReservationStatus, ResourceEstimate, ResourceReceipt, ResourceReservationId,
        ResourceScope, ResourceUsage, RuntimeDispatchErrorKind, RuntimeLane, TenantId, ThreadId,
        UserId,
    };
    use ironclaw_processes::{ProcessCancellationToken, ProcessStart, ProcessStore};
    use serde_json::json;

    #[derive(Default)]
    struct RecordingProcessExecutor {
        label: &'static str,
        calls: Arc<Mutex<Vec<CapabilityId>>>,
    }

    impl RecordingProcessExecutor {
        fn new(label: &'static str) -> Self {
            Self {
                label,
                calls: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn calls(&self) -> Vec<CapabilityId> {
            self.calls
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone()
        }
    }

    #[async_trait]
    impl ProcessExecutor for RecordingProcessExecutor {
        async fn execute(
            &self,
            request: ProcessExecutionRequest,
        ) -> Result<ProcessExecutionResult, ProcessExecutionError> {
            self.calls
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(request.capability_id);
            Ok(ProcessExecutionResult {
                output: json!({ "executor": self.label }),
            })
        }
    }

    fn dispatch_result() -> CapabilityDispatchResult {
        CapabilityDispatchResult {
            capability_id: CapabilityId::new("demo.background").unwrap(),
            provider: ExtensionId::new("demo").unwrap(),
            runtime: RuntimeKind::Script,
            output: json!({"ok": true}),
            display_preview: None,
            usage: ResourceUsage::default(),
            receipt: ResourceReceipt {
                id: ResourceReservationId::new(),
                scope: ResourceScope::system(),
                status: ReservationStatus::Reconciled,
                estimate: ResourceEstimate::default(),
                actual: Some(ResourceUsage::default()),
            },
        }
    }

    #[tokio::test]
    async fn host_process_executor_sends_sandbox_capability_to_configured_executor() {
        let dispatch = Arc::new(RecordingProcessExecutor::new("dispatch"));
        let sandbox = Arc::new(RecordingProcessExecutor::new("sandbox"));
        let executor = HostProcessExecutor::new(
            dispatch.clone() as Arc<dyn ProcessExecutor>,
            Some(sandbox.clone() as Arc<dyn ProcessExecutor>),
        );

        let result = executor
            .execute(sample_process_request(
                "system.process_sandbox.run",
                RuntimeKind::System,
            ))
            .await
            .unwrap();

        assert_eq!(result.output, json!({ "executor": "sandbox" }));
        assert!(dispatch.calls().is_empty());
        assert_eq!(sandbox.calls().len(), 1);
    }

    #[tokio::test]
    async fn host_process_executor_keeps_unrouted_processes_on_dispatch_executor() {
        let dispatch = Arc::new(RecordingProcessExecutor::new("dispatch"));
        let sandbox = Arc::new(RecordingProcessExecutor::new("sandbox"));
        let executor = HostProcessExecutor::new(
            dispatch.clone() as Arc<dyn ProcessExecutor>,
            Some(sandbox.clone() as Arc<dyn ProcessExecutor>),
        );

        let result = executor
            .execute(sample_process_request(
                "demo.background",
                RuntimeKind::Script,
            ))
            .await
            .unwrap();

        assert_eq!(result.output, json!({ "executor": "dispatch" }));
        assert_eq!(dispatch.calls().len(), 1);
        assert!(sandbox.calls().is_empty());
    }

    #[tokio::test]
    async fn host_process_executor_requires_system_runtime_and_sandbox_capability() {
        // safety: test executor calls are in-memory process executor calls, not DB writes.
        let dispatch = Arc::new(RecordingProcessExecutor::new("dispatch"));
        let sandbox = Arc::new(RecordingProcessExecutor::new("sandbox"));
        let executor = HostProcessExecutor::new(
            dispatch.clone() as Arc<dyn ProcessExecutor>,
            Some(sandbox.clone() as Arc<dyn ProcessExecutor>),
        );

        let sandbox_capability_wrong_runtime = executor
            .execute(sample_process_request(
                "system.process_sandbox.run",
                RuntimeKind::Script,
            ))
            .await
            .unwrap();
        let system_runtime_wrong_capability = executor
            .execute(sample_process_request(
                "demo.background",
                RuntimeKind::System,
            ))
            .await
            .unwrap();

        assert_eq!(
            sandbox_capability_wrong_runtime.output,
            json!({ "executor": "dispatch" })
        );
        assert_eq!(
            system_runtime_wrong_capability.output,
            json!({ "executor": "dispatch" })
        );
        assert_eq!(dispatch.calls().len(), 2);
        assert!(sandbox.calls().is_empty());
    }

    #[tokio::test]
    async fn host_process_executor_fails_sandbox_capability_when_unconfigured() {
        let dispatch = Arc::new(RecordingProcessExecutor::new("dispatch"));
        let executor = HostProcessExecutor::new(dispatch.clone() as Arc<dyn ProcessExecutor>, None);

        let error = executor
            .execute(sample_process_request(
                "system.process_sandbox.run",
                RuntimeKind::System,
            ))
            .await
            .unwrap_err();

        assert_eq!(error.kind, "missing_process_sandbox_executor");
        assert!(dispatch.calls().is_empty());
    }

    #[tokio::test]
    async fn runtime_dispatch_executor_returns_completed_result_after_dispatch() {
        // safety: test executor call is an in-memory process executor call, not a DB write.
        let cancellation = ProcessCancellationToken::new();
        let signal = cancellation.clone();
        let dispatcher = Arc::new(TestDispatcher::responding(move |_, _| {
            signal.cancel();
            Ok(dispatch_result())
        }));
        let mut request = sample_process_request("demo.background", RuntimeKind::Script);
        request.cancellation = cancellation;
        let executor = runtime_dispatch_executor(dispatcher.clone(), &request).await;

        let result = executor.execute(request).await.unwrap();

        assert_eq!(result.output, dispatch_result().output);
        assert_eq!(dispatcher.call_count(), 1);
    }

    #[tokio::test]
    async fn runtime_dispatch_executor_rejects_missing_authorized_continuation() {
        // safety: test dispatcher is in-memory and should not be called.
        let dispatcher = Arc::new(TestDispatcher::ok(dispatch_result()));
        let mut request = sample_process_request("demo.background", RuntimeKind::Script);
        request.authorized_continuation = None;
        let executor = runtime_dispatch_executor(dispatcher.clone(), &request).await;

        let error = executor.execute(request).await.unwrap_err();

        assert_eq!(error.kind, "missing_process_authorization");
        assert_eq!(dispatcher.call_count(), 0);
    }

    #[tokio::test]
    async fn runtime_dispatch_executor_preserves_authenticated_actor() {
        // safety: test dispatcher call is in-memory and does not execute an external capability.
        let dispatcher = Arc::new(TestDispatcher::ok(dispatch_result()));
        let mut request = sample_process_request("demo.background", RuntimeKind::Script);
        let actor = UserId::new("slack-alice").unwrap();
        request.authenticated_actor_user_id = Some(actor.clone());
        if let Some(continuation) = &mut request.authorized_continuation {
            continuation.invocation.actor = Actor::Sealed(actor.clone());
        }
        let expected_scope = request.scope.clone();
        let expected_capability_id = request.capability_id.clone();
        let executor = runtime_dispatch_executor(dispatcher.clone(), &request).await;

        executor.execute(request).await.unwrap();

        let calls = dispatcher.recorded();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].authenticated_actor_user_id, Some(actor));
        assert_eq!(calls[0].invocation.scope, expected_scope);
        assert_eq!(calls[0].invocation.capability, expected_capability_id);
    }

    #[tokio::test]
    async fn runtime_dispatch_executor_rejects_request_not_matching_persisted_authorization() {
        let dispatcher = Arc::new(TestDispatcher::ok(dispatch_result()));
        let persisted = sample_process_request("demo.background", RuntimeKind::Script);
        let mut request = persisted.clone();
        if let Some(continuation) = &mut request.authorized_continuation {
            continuation.invocation.actor = Actor::Sealed(UserId::new("forged-user").unwrap());
        }
        let executor = runtime_dispatch_executor(dispatcher.clone(), &persisted).await;

        let error = executor.execute(request).await.unwrap_err();

        assert_eq!(error.kind, "process_authorization_mismatch");
        assert_eq!(dispatcher.call_count(), 0);
    }

    #[test]
    fn dispatch_error_kind_maps_dispatch_variants() {
        let capability = CapabilityId::new("demo.background").unwrap();
        let provider = ExtensionId::new("demo").unwrap();

        let cases = [
            (
                DispatchError::UnknownCapability {
                    capability: capability.clone(),
                },
                "unknown_capability",
            ),
            (
                DispatchError::UnknownProvider {
                    capability: capability.clone(),
                    provider,
                },
                "unknown_provider",
            ),
            (
                DispatchError::RuntimeMismatch {
                    capability: capability.clone(),
                    descriptor_runtime: RuntimeKind::Script,
                    package_runtime: RuntimeKind::Wasm,
                },
                "runtime_mismatch",
            ),
            (
                DispatchError::MissingRuntimeBackend {
                    runtime: RuntimeKind::Script,
                },
                "missing_runtime_backend",
            ),
            (
                DispatchError::UnsupportedRuntime {
                    capability,
                    runtime: RuntimeKind::Script,
                },
                "unsupported_runtime",
            ),
            (
                DispatchError::Script {
                    kind: RuntimeDispatchErrorKind::Resource,
                    model_visible_cause: None,
                },
                "resource",
            ),
            (
                DispatchError::Mcp {
                    kind: RuntimeDispatchErrorKind::NetworkDenied,
                    model_visible_cause: None,
                },
                "network_denied",
            ),
            (
                DispatchError::Wasm {
                    kind: RuntimeDispatchErrorKind::OutputDecode,
                    model_visible_cause: None,
                },
                "output_decode",
            ),
        ];

        for (error, expected) in cases {
            assert_eq!(error.event_kind(), expected);
        }
    }

    fn sample_process_request(
        capability_id: &str,
        runtime: RuntimeKind,
    ) -> ProcessExecutionRequest {
        let process_id = ProcessId::new();
        let invocation_id = InvocationId::new();
        let scope = ResourceScope {
            tenant_id: TenantId::new("tenant").unwrap(),
            user_id: UserId::new("user").unwrap(),
            agent_id: Some(AgentId::new("agent").unwrap()),
            project_id: Some(ProjectId::new("project").unwrap()),
            mission_id: None,
            thread_id: Some(ThreadId::new("thread").unwrap()),
            invocation_id: InvocationId::new(),
        };
        let capability_id = CapabilityId::new(capability_id).unwrap();
        let estimate = ResourceEstimate::default();
        let authorized_continuation =
            RuntimeLane::from_runtime_kind(runtime).map(|lane| ProcessAuthorizedContinuation {
                invocation: ProcessAuthorizedInvocation {
                    activity_id: ActivityId::from_uuid(invocation_id.as_uuid()),
                    capability: capability_id.clone(),
                    scope: scope.clone(),
                    actor: Actor::System,
                    origin: InvocationOrigin::Product(ProductKind::new("test").unwrap()),
                    estimate: estimate.clone(),
                    correlation_id: CorrelationId::new(),
                    process_id,
                    parent_process_id: None,
                },
                lane,
                mounts: Some(MountView::default()),
                resource_reservation: None,
            });
        ProcessExecutionRequest {
            process_id,
            invocation_id,
            scope,
            authenticated_actor_user_id: None,
            extension_id: ExtensionId::new("system").unwrap(),
            capability_id,
            runtime,
            estimate,
            mounts: MountView::default(),
            resource_reservation: None,
            authorized_continuation,
            input: json!({}),
            cancellation: ProcessCancellationToken::new(),
        }
    }

    async fn runtime_dispatch_executor(
        dispatcher: Arc<TestDispatcher>,
        request: &ProcessExecutionRequest,
    ) -> RuntimeDispatchProcessExecutor {
        let store = Arc::new(ironclaw_processes::in_memory_backed_process_store());
        store
            .start(process_start_from_request(request))
            .await
            .unwrap();
        let process_store: Arc<dyn ProcessStore> = store;
        RuntimeDispatchProcessExecutor::new(
            dispatcher,
            ironclaw_capabilities::process_authorization_remint_port(process_store),
        )
    }

    fn process_start_from_request(request: &ProcessExecutionRequest) -> ProcessStart {
        ProcessStart {
            process_id: request.process_id,
            parent_process_id: request
                .authorized_continuation
                .as_ref()
                .and_then(|continuation| continuation.invocation.parent_process_id),
            invocation_id: request.invocation_id,
            scope: request.scope.clone(),
            authenticated_actor_user_id: request.authenticated_actor_user_id.clone(),
            extension_id: request.extension_id.clone(),
            capability_id: request.capability_id.clone(),
            runtime: request.runtime,
            grants: Default::default(),
            mounts: request.mounts.clone(),
            estimated_resources: request.estimate.clone(),
            resource_reservation_id: request
                .resource_reservation
                .as_ref()
                .map(|reservation| reservation.id),
            authorized_continuation: request.authorized_continuation.clone(),
            input: request.input.clone(),
        }
    }
}
