//! Canonical planned-loop placement inside the persistent user sandbox.

use std::{num::NonZeroU32, sync::Arc};

use async_trait::async_trait;
use ironclaw_host_api::{
    ids::InvocationId,
    process::{RuntimeProcessError, SandboxLoopWorkerStartRequest, SandboxLoopWorkerTransport},
};
use ironclaw_loop_contracts::{
    AgentLoopDriver, AgentLoopDriverDescriptor, AgentLoopDriverError, AgentLoopDriverHost,
    AgentLoopDriverResumeRequest, AgentLoopDriverRunRequest, AgentLoopHostError, LoopExit,
};
use ironclaw_loop_host::{
    LoopWorkerFailure, LoopWorkerInvocation, LoopWorkerOutcome, LoopWorkerSettings,
    read_worker_bootstrap, remote_host_from_stdio, serve_loop_worker,
};

use crate::{
    app_loop_family::build_loop_family_registry_with_overrides,
    driver_registry::{DriverKind, DriverRegistry, LoopDriverRegistryKey},
    planned_driver::PlannedDriver,
    planned_driver_factory::{
        DefaultPlannedDriverRegistrationError, planned_driver_descriptor,
        planned_driver_requirements,
    },
    turn_runner::sanitized_driver_failure,
};

pub const LOOP_WORKER_EXECUTABLE: &str = "/usr/local/bin/ironclaw-loop-worker";

pub struct SandboxedPlannedDriver {
    descriptor: AgentLoopDriverDescriptor,
    transport: Arc<dyn SandboxLoopWorkerTransport>,
    settings: LoopWorkerSettings,
}

impl SandboxedPlannedDriver {
    pub fn new(
        transport: Arc<dyn SandboxLoopWorkerTransport>,
        default_iteration_limit: Option<NonZeroU32>,
        model_availability_attempts: Option<NonZeroU32>,
    ) -> Result<Self, AgentLoopDriverError> {
        let descriptor = planned_driver_descriptor()
            .map_err(|reason| AgentLoopDriverError::InvalidRequest { reason })?;
        Ok(Self {
            descriptor,
            transport,
            settings: LoopWorkerSettings {
                default_iteration_limit: default_iteration_limit.map(NonZeroU32::get),
                model_availability_attempts: model_availability_attempts.map(NonZeroU32::get),
            },
        })
    }

    async fn invoke(
        &self,
        invocation: LoopWorkerInvocation,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
    ) -> Result<LoopExit, AgentLoopDriverError> {
        let context = host.run_context();
        let mut scope = context.scope.to_resource_scope();
        if let Some(actor) = context.actor() {
            scope.user_id = actor.user_id.clone();
        }
        scope.thread_id = Some(context.thread_id.clone());
        scope.invocation_id = InvocationId::from_uuid(context.run_id.as_uuid());

        let mut session = self
            .transport
            .start_loop_worker(SandboxLoopWorkerStartRequest {
                scope,
                executable: LOOP_WORKER_EXECUTABLE.to_string(),
                args: Vec::new(),
                workdir: Some("/workspace".to_string()),
            })
            .await
            .map_err(worker_process_transport_error)?;
        let outcome = serve_loop_worker(session.as_mut(), host, invocation, self.settings).await;
        let cleanup = session.terminate().await;
        if let Err(error) = cleanup {
            return Err(worker_process_transport_error(error));
        }
        match outcome.map_err(worker_host_transport_error)? {
            LoopWorkerOutcome::Exit(exit) => Ok(exit),
            LoopWorkerOutcome::Failed(failure) => {
                let sanitized = sanitized_driver_failure(&failure.kind, failure.detail.as_deref())
                    .ok_or_else(|| AgentLoopDriverError::Failed {
                        reason_kind: "driver_failed".to_string(),
                        detail: None,
                    })?;
                Err(AgentLoopDriverError::Failed {
                    reason_kind: sanitized.category().to_string(),
                    detail: sanitized.detail().map(str::to_string),
                })
            }
        }
    }
}
fn worker_process_transport_error(error: RuntimeProcessError) -> AgentLoopDriverError {
    let error_kind = match error {
        RuntimeProcessError::Timeout(_) => "timeout",
        RuntimeProcessError::ExecutionFailed(_) => "execution_failed",
    };
    tracing::debug!(error_kind, "sandbox loop worker process transport failed");
    unavailable_worker_error()
}

fn worker_host_transport_error(error: AgentLoopHostError) -> AgentLoopDriverError {
    tracing::debug!(
        error_kind = error.kind.as_str(),
        "sandbox loop worker host transport failed"
    );
    unavailable_worker_error()
}

fn unavailable_worker_error() -> AgentLoopDriverError {
    AgentLoopDriverError::Failed {
        reason_kind: "sandbox_loop_worker_unavailable".to_string(),
        detail: None,
    }
}

pub fn register_sandboxed_default_planned_driver(
    registry: &mut DriverRegistry,
    transport: Arc<dyn SandboxLoopWorkerTransport>,
    default_iteration_limit: Option<NonZeroU32>,
    model_availability_attempts: Option<NonZeroU32>,
) -> Result<LoopDriverRegistryKey, DefaultPlannedDriverRegistrationError> {
    let driver = Arc::new(SandboxedPlannedDriver::new(
        transport,
        default_iteration_limit,
        model_availability_attempts,
    )?);
    registry
        .register_driver(
            driver,
            planned_driver_requirements(),
            DriverKind::Production,
        )
        .map_err(Into::into)
}

#[async_trait]
impl AgentLoopDriver for SandboxedPlannedDriver {
    fn descriptor(&self) -> AgentLoopDriverDescriptor {
        self.descriptor.clone()
    }

    async fn run(
        &self,
        request: AgentLoopDriverRunRequest,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
    ) -> Result<LoopExit, AgentLoopDriverError> {
        self.invoke(LoopWorkerInvocation::Run(request), host).await
    }

    async fn resume(
        &self,
        request: AgentLoopDriverResumeRequest,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
    ) -> Result<LoopExit, AgentLoopDriverError> {
        self.invoke(LoopWorkerInvocation::Resume(request), host)
            .await
    }
}

/// Canonical loop-worker entrypoint used by the sandbox worker image.
pub async fn run_loop_worker_stdio() -> Result<(), String> {
    let mut stdin = tokio::io::stdin();
    let bootstrap = read_worker_bootstrap(&mut stdin)
        .await
        .map_err(|error| error.to_string())?;
    let remote_host = remote_host_from_stdio(&bootstrap).map_err(|error| error.to_string())?;
    let registry = build_loop_family_registry_with_overrides(
        bootstrap
            .settings
            .default_iteration_limit
            .and_then(NonZeroU32::new),
        bootstrap
            .settings
            .model_availability_attempts
            .and_then(NonZeroU32::new),
    )
    .map_err(|error| error.to_string())?;
    let driver =
        PlannedDriver::default_from_registry(&registry).map_err(|error| error.to_string())?;
    let outcome = match bootstrap.invocation {
        LoopWorkerInvocation::Run(request) => driver.run(request, &remote_host).await,
        LoopWorkerInvocation::Resume(request) => driver.resume(request, &remote_host).await,
    };
    let outcome = match outcome {
        Ok(exit) => LoopWorkerOutcome::Exit(exit),
        Err(error) => LoopWorkerOutcome::Failed(worker_failure(&error)),
    };
    remote_host
        .write_outcome(outcome)
        .await
        .map_err(|error| error.to_string())
}

fn worker_failure(error: &AgentLoopDriverError) -> LoopWorkerFailure {
    match error {
        AgentLoopDriverError::InvalidRequest { reason } => LoopWorkerFailure {
            kind: "driver_invalid_request".to_string(),
            detail: Some(ironclaw_loop_host::scrub_model_visible_detail(reason)),
        },
        AgentLoopDriverError::Unavailable { reason } => LoopWorkerFailure {
            kind: "driver_unavailable".to_string(),
            detail: Some(ironclaw_loop_host::scrub_model_visible_detail(reason)),
        },
        AgentLoopDriverError::Failed {
            reason_kind,
            detail,
        } => LoopWorkerFailure {
            kind: reason_kind.clone(),
            detail: detail
                .as_deref()
                .map(ironclaw_loop_host::scrub_model_visible_detail),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct UnusedTransport;

    #[async_trait]
    impl SandboxLoopWorkerTransport for UnusedTransport {
        async fn start_loop_worker(
            &self,
            _request: SandboxLoopWorkerStartRequest,
        ) -> Result<
            Box<dyn ironclaw_host_api::process::SandboxLoopWorkerSession>,
            ironclaw_host_api::process::RuntimeProcessError,
        > {
            Err(
                ironclaw_host_api::process::RuntimeProcessError::ExecutionFailed(
                    "unused test transport".to_string(),
                ),
            )
        }
    }

    struct HangingSession {
        terminated: Arc<std::sync::atomic::AtomicBool>,
    }

    #[async_trait]
    impl ironclaw_host_api::process::SandboxLoopWorkerSession for HangingSession {
        async fn send(
            &mut self,
            _frame: Vec<u8>,
        ) -> Result<(), ironclaw_host_api::process::RuntimeProcessError> {
            Ok(())
        }

        async fn receive(
            &mut self,
        ) -> Result<Option<Vec<u8>>, ironclaw_host_api::process::RuntimeProcessError> {
            std::future::pending().await
        }

        async fn terminate(
            &mut self,
        ) -> Result<(), ironclaw_host_api::process::RuntimeProcessError> {
            self.terminated
                .store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }
    }

    struct HangingTransport {
        terminated: Arc<std::sync::atomic::AtomicBool>,
    }

    #[async_trait]
    impl SandboxLoopWorkerTransport for HangingTransport {
        async fn start_loop_worker(
            &self,
            _request: SandboxLoopWorkerStartRequest,
        ) -> Result<
            Box<dyn ironclaw_host_api::process::SandboxLoopWorkerSession>,
            ironclaw_host_api::process::RuntimeProcessError,
        > {
            Ok(Box::new(HangingSession {
                terminated: Arc::clone(&self.terminated),
            }))
        }
    }

    #[tokio::test]
    async fn cancelled_silent_worker_is_terminated_after_grace_period() {
        use chrono::Utc;
        use ironclaw_agent_loop::test_support::{MockAgentLoopDriverHost, test_run_context};
        use ironclaw_loop_contracts::{LoopCancelReasonKind, LoopCancellationSignal};

        let terminated = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let driver = SandboxedPlannedDriver::new(
            Arc::new(HangingTransport {
                terminated: Arc::clone(&terminated),
            }),
            None,
            None,
        )
        .expect("driver");
        let mut context = test_run_context("sandbox-worker-cancellation");
        context.resolved_run_profile.loop_driver = driver.descriptor();
        let signal = LoopCancellationSignal {
            reason_kind: LoopCancelReasonKind::UserRequested,
            requested_at: Utc::now(),
        };
        let (host, _checkpoints) = MockAgentLoopDriverHost::builder()
            .run_context(context.clone())
            .cancellation_signal(signal)
            .build();

        let result = driver
            .run(
                AgentLoopDriverRunRequest {
                    turn_id: context.turn_id,
                    run_id: context.run_id,
                    resolved_run_profile: context.resolved_run_profile,
                },
                &host,
            )
            .await;

        assert!(matches!(
            result,
            Err(AgentLoopDriverError::Failed {
                reason_kind,
                detail: None,
            }) if reason_kind == "sandbox_loop_worker_unavailable"
        ));
        assert!(terminated.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[test]
    fn sandbox_worker_receives_resolved_family_overrides() {
        let driver = SandboxedPlannedDriver::new(
            Arc::new(UnusedTransport),
            NonZeroU32::new(7),
            NonZeroU32::new(2),
        )
        .expect("driver");

        assert_eq!(driver.settings.default_iteration_limit, Some(7));
        assert_eq!(driver.settings.model_availability_attempts, Some(2));
    }

    #[test]
    fn worker_failure_preserves_canonical_reason_and_scrubbed_detail() {
        let failure = worker_failure(&AgentLoopDriverError::Failed {
            reason_kind: "model_credentials_unavailable".to_string(),
            detail: Some("provider api_key=sk-secretvalue unavailable".to_string()),
        });

        assert_eq!(failure.kind, "model_credentials_unavailable");
        let detail = failure.detail.expect("safe detail");
        assert!(!detail.contains("sk-secretvalue"));
    }
}
