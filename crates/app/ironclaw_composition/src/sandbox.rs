//! User-sandbox runtime construction.
//!
//! Provider-specific Docker and Railway setup stays behind this factory so the
//! rest of composition receives the same opaque runtime process binding.

use std::{path::PathBuf, sync::Arc};

use ironclaw_host_runtime::UserSandboxProcessPort;
use ironclaw_sandbox::{
    RailwayPreviewSandboxConfig, RailwayPreviewSandboxTransport, RebornSandboxConfig,
    RebornScopedSandboxCommandTransport,
};

use crate::{RebornBuildError, RebornRuntimeProcessBinding};

/// Connect the existing local Docker transport and return the complete runtime
/// process binding. Profile boot fails if the daemon is unavailable; no caller
/// receives an unsandboxed fallback or a provider assembly handle.
pub async fn build_local_docker_user_sandbox_binding(
    workspace_root: PathBuf,
    legacy_workspace_root: Option<PathBuf>,
    enable_loop_worker: bool,
) -> Result<RebornRuntimeProcessBinding, RebornBuildError> {
    let mut config = RebornSandboxConfig::new(workspace_root);
    if let Some(legacy_workspace_root) = legacy_workspace_root {
        config = config.with_legacy_workspace_root(legacy_workspace_root);
    }
    let config =
        config
            .with_managed_egress_proxy()
            .map_err(|error| RebornBuildError::InvalidConfig {
                reason: format!("user-sandbox managed egress configuration is invalid: {error}"),
            })?;
    let transport = Arc::new(
        RebornScopedSandboxCommandTransport::connect(config)
            .await
            .map_err(|error| RebornBuildError::InvalidConfig {
                reason: format!(
                    "user-sandbox process backend requires Docker and pinned worker/proxy images: {error}"
                ),
            })?,
    );
    Ok(assemble_local_user_sandbox_binding(
        transport,
        enable_loop_worker,
    ))
}

fn assemble_local_user_sandbox_binding<T>(
    transport: Arc<T>,
    enable_loop_worker: bool,
) -> RebornRuntimeProcessBinding
where
    T: ironclaw_host_api::process::SandboxCommandTransport
        + ironclaw_host_api::process::SandboxLoopWorkerTransport
        + 'static,
{
    let command_transport: Arc<dyn ironclaw_host_api::process::SandboxCommandTransport> =
        transport.clone();
    let process_port = Arc::new(UserSandboxProcessPort::new(command_transport));
    if enable_loop_worker {
        let loop_worker_transport: Arc<dyn ironclaw_host_api::process::SandboxLoopWorkerTransport> =
            transport;
        RebornRuntimeProcessBinding::user_sandbox_with_loop_worker(
            process_port,
            loop_worker_transport,
        )
    } else {
        RebornRuntimeProcessBinding::user_sandbox(process_port)
    }
}

/// Build the complete lazy Railway runtime process binding without exposing
/// Railway's transport configuration through composition's public API. The
/// first shell invocation provisions the caller's remote sandbox.
pub fn build_railway_user_sandbox_binding(
    project_id: String,
    environment_id: String,
    cli_path: Option<PathBuf>,
    idle_timeout_minutes: Option<u16>,
    worker_image: Option<String>,
) -> Result<RebornRuntimeProcessBinding, RebornBuildError> {
    let mut config = RailwayPreviewSandboxConfig::new(project_id, environment_id)
        .map_err(invalid_railway_config)?;
    if let Some(cli_path) = cli_path {
        config = config.with_cli_path(cli_path);
    }
    if let Some(minutes) = idle_timeout_minutes {
        config = config
            .with_idle_timeout_minutes(minutes)
            .map_err(invalid_railway_config)?;
    }
    if let Some(worker_image) = worker_image {
        config = config
            .with_worker_image(worker_image)
            .map_err(invalid_railway_config)?;
    }
    Ok(binding(Arc::new(RailwayPreviewSandboxTransport::new(
        config,
    ))))
}

fn invalid_railway_config(error: impl std::fmt::Display) -> RebornBuildError {
    RebornBuildError::InvalidConfig {
        reason: format!("Railway preview sandbox configuration is invalid: {error}"),
    }
}

fn binding(
    transport: Arc<dyn ironclaw_host_api::process::SandboxCommandTransport>,
) -> RebornRuntimeProcessBinding {
    RebornRuntimeProcessBinding::user_sandbox(Arc::new(UserSandboxProcessPort::new(transport)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use ironclaw_host_api::process::{
        CommandExecutionOutput, CommandExecutionRequest, RuntimeProcessError,
        SandboxLoopWorkerSession, SandboxLoopWorkerStartRequest,
    };

    struct UnusedTransport;

    #[async_trait]
    impl ironclaw_host_api::process::SandboxCommandTransport for UnusedTransport {
        async fn run_command(
            &self,
            _request: CommandExecutionRequest,
        ) -> Result<CommandExecutionOutput, RuntimeProcessError> {
            Err(RuntimeProcessError::ExecutionFailed(
                "unused test transport".to_string(),
            ))
        }
    }

    #[async_trait]
    impl ironclaw_host_api::process::SandboxLoopWorkerTransport for UnusedTransport {
        async fn start_loop_worker(
            &self,
            _request: SandboxLoopWorkerStartRequest,
        ) -> Result<Box<dyn SandboxLoopWorkerSession>, RuntimeProcessError> {
            Err(RuntimeProcessError::ExecutionFailed(
                "unused test transport".to_string(),
            ))
        }
    }

    #[test]
    fn loop_worker_switch_controls_the_resulting_process_binding() {
        for (enabled, expected) in [(false, false), (true, true)] {
            let binding = assemble_local_user_sandbox_binding(Arc::new(UnusedTransport), enabled);
            assert_eq!(binding.loop_worker_transport().is_some(), expected);
        }
    }
}
