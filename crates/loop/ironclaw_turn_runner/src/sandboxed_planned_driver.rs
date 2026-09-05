//! Canonical planned-loop placement inside the persistent user sandbox.

use std::{num::NonZeroU32, sync::Arc};

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
    WorkerContentVisibility, read_worker_bootstrap, remote_host_from_stdio, serve_loop_worker,
};

pub const LOOP_WORKER_EXECUTABLE: &str = "/usr/local/bin/ironclaw-loop-worker";

pub const PI_LOOP_WORKER_EXECUTABLE: &str = "/usr/local/bin/ironclaw-pi-worker";
pub(crate) const PI_CHECKPOINT_SCHEMA_ID: &str = "pi_worker_session";

pub(crate) struct PiRunProfileResolver {
    pub(crate) inner: Arc<dyn ironclaw_loop_contracts::RunProfileResolver>,
}

#[async_trait]
impl ironclaw_loop_contracts::RunProfileResolver for PiRunProfileResolver {
    async fn resolve_run_profile(
        &self,
        request: ironclaw_loop_contracts::RunProfileResolutionRequest,
    ) -> Result<
        ironclaw_loop_contracts::ResolvedRunProfile,
        ironclaw_loop_contracts::RunProfileResolutionError,
    > {
        let mut profile = self.inner.resolve_run_profile(request).await?;
        if profile.loop_driver.id.as_str()
            == crate::planned_driver_factory::PLANNED_DRIVER_DEFAULT_ID
        {
            let schema = ironclaw_loop_contracts::CheckpointSchemaId::new(PI_CHECKPOINT_SCHEMA_ID)
                .map_err(|reason| {
                    ironclaw_loop_contracts::RunProfileResolutionError::InvalidRequest { reason }
                })?;
            let version = ironclaw_host_api::turn::RunProfileVersion::new(1);
            profile.loop_driver.checkpoint_schema_id = Some(schema.clone());
            profile.loop_driver.checkpoint_schema_version = Some(version);
            profile.checkpoint_schema_id = schema;
            profile.checkpoint_schema_version = version;
        }
        Ok(profile)
    }
}

/// Which loop-worker binary the sandboxed driver launches, and how much of
/// the run's own transcript the worker may see. Pi is the default and runs
/// content-resolved; the explicit Rust worker remains content-blind. See
/// `docs/internal/reborn/2026-09-pi-loop-worker-plan.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LoopWorkerKind {
    Rust,
    #[default]
    Pi,
}

impl LoopWorkerKind {
    pub fn executable(self) -> &'static str {
        match self {
            Self::Rust => LOOP_WORKER_EXECUTABLE,
            Self::Pi => PI_LOOP_WORKER_EXECUTABLE,
        }
    }

    pub fn content_visibility(self) -> WorkerContentVisibility {
        match self {
            Self::Rust => WorkerContentVisibility::Blind,
            Self::Pi => WorkerContentVisibility::Resolved,
        }
    }

    /// Case-insensitive `IRONCLAW_REBORN_SANDBOX_LOOP_WORKER_KIND` value.
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "rust" => Some(Self::Rust),
            "pi" => Some(Self::Pi),
            _ => None,
        }
    }
}

pub struct SandboxedPlannedDriver {
    descriptor: AgentLoopDriverDescriptor,
    transport: Arc<dyn SandboxLoopWorkerTransport>,
    kind: LoopWorkerKind,
    settings: LoopWorkerSettings,
}

impl SandboxedPlannedDriver {
    pub fn new(
        transport: Arc<dyn SandboxLoopWorkerTransport>,
        kind: LoopWorkerKind,
        default_iteration_limit: Option<NonZeroU32>,
        model_availability_attempts: Option<NonZeroU32>,
    ) -> Result<Self, AgentLoopDriverError> {
        let descriptor = planned_driver_descriptor()
            .and_then(|descriptor| match kind {
                LoopWorkerKind::Rust => Ok(descriptor),
                LoopWorkerKind::Pi => descriptor.with_checkpoint_schema(
                    PI_CHECKPOINT_SCHEMA_ID,
                    ironclaw_host_api::turn::RunProfileVersion::new(1),
                ),
            })
            .map_err(|reason| AgentLoopDriverError::InvalidRequest { reason })?;
        Ok(Self {
            descriptor,
            transport,
            kind,
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
                executable: self.kind.executable().to_string(),
                args: Vec::new(),
                workdir: Some("/workspace".to_string()),
            })
            .await
            .map_err(worker_process_transport_error)?;
        // The run host owns the same content resolver and instruction store as
        // its prompt port. The transport independently gates resolved visibility.
        let content = host.worker_content_port();
        let outcome = serve_loop_worker(
            session.as_mut(),
            host,
            invocation,
            self.settings,
            content,
            self.kind.content_visibility(),
        )
        .await;
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
    kind: LoopWorkerKind,
    default_iteration_limit: Option<NonZeroU32>,
    model_availability_attempts: Option<NonZeroU32>,
) -> Result<LoopDriverRegistryKey, DefaultPlannedDriverRegistrationError> {
    let driver = Arc::new(SandboxedPlannedDriver::new(
        transport,
        kind,
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
            LoopWorkerKind::Rust,
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
            LoopWorkerKind::Rust,
            NonZeroU32::new(7),
            NonZeroU32::new(2),
        )
        .expect("driver");

        assert_eq!(driver.settings.default_iteration_limit, Some(7));
        assert_eq!(driver.settings.model_availability_attempts, Some(2));
    }

    #[test]
    fn worker_kind_selects_executable_and_visibility() {
        assert_eq!(
            LoopWorkerKind::default().executable(),
            PI_LOOP_WORKER_EXECUTABLE
        );
        assert_eq!(
            LoopWorkerKind::Rust.executable(),
            "/usr/local/bin/ironclaw-loop-worker"
        );
        assert_eq!(LoopWorkerKind::Pi.executable(), PI_LOOP_WORKER_EXECUTABLE);
        assert_eq!(
            LoopWorkerKind::Rust.content_visibility(),
            WorkerContentVisibility::Blind
        );
        assert_eq!(
            LoopWorkerKind::Pi.content_visibility(),
            WorkerContentVisibility::Resolved
        );
    }

    #[test]
    fn worker_kind_parse_accepts_case_insensitive_values_and_rejects_others() {
        assert_eq!(LoopWorkerKind::parse("rust"), Some(LoopWorkerKind::Rust));
        assert_eq!(LoopWorkerKind::parse("RUST"), Some(LoopWorkerKind::Rust));
        assert_eq!(LoopWorkerKind::parse(" pi "), Some(LoopWorkerKind::Pi));
        assert_eq!(LoopWorkerKind::parse("Pi"), Some(LoopWorkerKind::Pi));
        assert_eq!(LoopWorkerKind::parse("node"), None);
        assert_eq!(LoopWorkerKind::parse(""), None);
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
