use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::{
    CapabilityDispatchRequest, CapabilityDispatcher, DispatchError, RuntimeKind,
};
use ironclaw_processes::{
    ProcessExecutionError, ProcessExecutionRequest, ProcessExecutionResult, ProcessExecutor,
};

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
        if is_process_sandbox_request(&request) {
            let Some(executor) = &self.process_sandbox_executor else {
                return Err(ProcessExecutionError::new(
                    "missing_process_sandbox_executor",
                ));
            };
            return executor.execute(request).await;
        }
        self.dispatch_executor.execute(request).await
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
        if request.cancellation.is_cancelled() {
            return Err(ProcessExecutionError::new("cancelled"));
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
            .map_err(|error| ProcessExecutionError::new(dispatch_error_kind(&error)))?;
        if request.cancellation.is_cancelled() {
            return Err(ProcessExecutionError::new("cancelled"));
        }
        Ok(ProcessExecutionResult {
            output: result.output,
        })
    }
}

fn dispatch_error_kind(error: &DispatchError) -> &'static str {
    match error {
        DispatchError::UnknownCapability { .. } => "unknown_capability",
        DispatchError::UnknownProvider { .. } => "unknown_provider",
        DispatchError::RuntimeMismatch { .. } => "runtime_mismatch",
        DispatchError::MissingRuntimeBackend { .. } => "missing_runtime_backend",
        DispatchError::UnsupportedRuntime { .. } => "unsupported_runtime",
        DispatchError::Mcp { kind }
        | DispatchError::Script { kind }
        | DispatchError::Wasm { kind } => kind.event_kind(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    use ironclaw_host_api::{
        AgentId, CapabilityId, ExtensionId, InvocationId, MountView, ProcessId, ProjectId,
        ResourceEstimate, ResourceScope, TenantId, ThreadId, UserId,
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
