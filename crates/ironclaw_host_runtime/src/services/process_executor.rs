use std::{sync::Arc, time::Instant};

use async_trait::async_trait;
use ironclaw_host_api::{CapabilityDispatchRequest, CapabilityDispatcher, RuntimeKind};
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
}

impl RuntimeDispatchProcessExecutor {
    pub(super) fn new(dispatcher: Arc<dyn CapabilityDispatcher>) -> Self {
        Self { dispatcher }
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
        let result = self
            .dispatcher
            .dispatch_json(CapabilityDispatchRequest {
                capability_id: request.capability_id,
                scope: request.scope,
                estimate: request.estimate,
                mounts: Some(request.mounts),
                resource_reservation: request.resource_reservation,
                input: request.input,
            })
            .await
            .map_err(|error| ProcessExecutionError::new(error.event_kind()))?;
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

    use ironclaw_host_api::{
        AgentId, CapabilityDispatchResult, CapabilityId, DispatchError, ExtensionId, InvocationId,
        MountView, ProcessId, ProjectId, ReservationStatus, ResourceEstimate, ResourceReceipt,
        ResourceReservationId, ResourceScope, ResourceUsage, RuntimeDispatchErrorKind, TenantId,
        ThreadId, UserId,
    };
    use ironclaw_processes::ProcessCancellationToken;
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

    #[derive(Clone)]
    struct CancellingDispatcher {
        cancellation: ProcessCancellationToken,
    }

    #[async_trait]
    impl CapabilityDispatcher for CancellingDispatcher {
        async fn dispatch_json(
            &self,
            request: CapabilityDispatchRequest,
        ) -> Result<CapabilityDispatchResult, DispatchError> {
            self.cancellation.cancel();
            Ok(CapabilityDispatchResult {
                capability_id: request.capability_id,
                provider: ExtensionId::new("demo").unwrap(),
                runtime: RuntimeKind::Script,
                output: json!({"ok": true}),
                display_preview: None,
                usage: ResourceUsage::default(),
                receipt: ResourceReceipt {
                    id: ResourceReservationId::new(),
                    scope: request.scope,
                    status: ReservationStatus::Reconciled,
                    estimate: ResourceEstimate::default(),
                    actual: Some(ResourceUsage::default()),
                },
            })
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
    async fn runtime_dispatch_executor_returns_cancelled_after_dispatch() {
        // safety: test executor call is an in-memory process executor call, not a DB write.
        let cancellation = ProcessCancellationToken::new();
        let dispatcher = Arc::new(CancellingDispatcher {
            cancellation: cancellation.clone(),
        });
        let executor = RuntimeDispatchProcessExecutor::new(dispatcher);
        let mut request = sample_process_request("demo.background", RuntimeKind::Script);
        request.cancellation = cancellation;

        let error = executor.execute(request).await.unwrap_err();

        assert_eq!(error.kind, "cancelled");
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
                },
                "resource",
            ),
            (
                DispatchError::Mcp {
                    kind: RuntimeDispatchErrorKind::NetworkDenied,
                },
                "network_denied",
            ),
            (
                DispatchError::Wasm {
                    kind: RuntimeDispatchErrorKind::OutputDecode,
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
        ProcessExecutionRequest {
            process_id: ProcessId::new(),
            invocation_id: InvocationId::new(),
            scope: ResourceScope {
                tenant_id: TenantId::new("tenant").unwrap(),
                user_id: UserId::new("user").unwrap(),
                agent_id: Some(AgentId::new("agent").unwrap()),
                project_id: Some(ProjectId::new("project").unwrap()),
                mission_id: None,
                thread_id: Some(ThreadId::new("thread").unwrap()),
                invocation_id: InvocationId::new(),
            },
            extension_id: ExtensionId::new("system").unwrap(),
            capability_id: CapabilityId::new(capability_id).unwrap(),
            runtime,
            estimate: ResourceEstimate::default(),
            mounts: MountView::default(),
            resource_reservation: None,
            input: json!({}),
            cancellation: ProcessCancellationToken::new(),
        }
    }
}
