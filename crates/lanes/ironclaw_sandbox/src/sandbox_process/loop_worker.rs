//! Full-duplex canonical-loop worker sessions inside persistent user containers.

use std::collections::VecDeque;

use async_trait::async_trait;
use bollard::{
    container::LogOutput,
    exec::{CreateExecOptions, StartExecOptions, StartExecResults},
};
use futures_util::StreamExt;
use ironclaw_host_api::process::{
    MAX_SANDBOX_LOOP_WORKER_FRAME_BYTES, RuntimeProcessError, SandboxLoopWorkerSession,
    SandboxLoopWorkerStartRequest, SandboxLoopWorkerTransport,
};
use tokio::io::AsyncWriteExt;

use super::{
    CONTAINER_WORKSPACE_ROOT, RebornScopedSandboxCommandTransport, registry::SandboxActivityGuard,
    user_container, user_key::RebornSandboxUserKey,
};

const FRAME_HEADER_BYTES: usize = std::mem::size_of::<u32>();

pub(super) struct DockerLoopWorkerSession {
    docker: bollard::Docker,
    container_name: String,
    exec_id: String,
    process_id: i64,
    input: Option<std::pin::Pin<Box<dyn tokio::io::AsyncWrite + Send>>>,
    output: std::pin::Pin<
        Box<dyn futures_util::Stream<Item = Result<LogOutput, bollard::errors::Error>> + Send>,
    >,
    stdout: VecDeque<u8>,
    diagnostic: String,
    _activity: SandboxActivityGuard,
    terminated: bool,
}

impl DockerLoopWorkerSession {
    async fn terminate_inner(&mut self) -> Result<(), RuntimeProcessError> {
        if self.terminated {
            return Ok(());
        }
        self.input.take();
        terminate_process(&self.docker, &self.container_name, self.process_id).await?;
        self.terminated = true;
        Ok(())
    }
}

impl Drop for DockerLoopWorkerSession {
    fn drop(&mut self) {
        if self.terminated {
            return;
        }
        self.terminated = true;
        self.input.take();
        let process_id = self.process_id;
        let docker = self.docker.clone();
        let container_name = self.container_name.clone();
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                if terminate_process(&docker, &container_name, process_id)
                    .await
                    .is_err()
                {
                    tracing::debug!("failed to terminate dropped sandbox loop worker");
                }
            });
        }
    }
}

#[async_trait]
impl SandboxLoopWorkerSession for DockerLoopWorkerSession {
    async fn send(&mut self, frame: Vec<u8>) -> Result<(), RuntimeProcessError> {
        if frame.len() > MAX_SANDBOX_LOOP_WORKER_FRAME_BYTES {
            return Err(RuntimeProcessError::ExecutionFailed(
                "sandbox loop worker frame exceeds the byte limit".to_string(),
            ));
        }
        let input = self.input.as_mut().ok_or_else(|| {
            RuntimeProcessError::ExecutionFailed(
                "sandbox loop worker input is already closed".to_string(),
            )
        })?;
        let length = u32::try_from(frame.len()).map_err(|_| {
            RuntimeProcessError::ExecutionFailed(
                "sandbox loop worker frame length cannot be represented".to_string(),
            )
        })?;
        input.write_u32(length).await.map_err(pipe_write_error)?;
        input.write_all(&frame).await.map_err(pipe_write_error)?;
        input.flush().await.map_err(pipe_write_error)
    }

    async fn receive(&mut self) -> Result<Option<Vec<u8>>, RuntimeProcessError> {
        loop {
            if self.stdout.len() >= FRAME_HEADER_BYTES {
                let length = u32::from_be_bytes([
                    self.stdout[0],
                    self.stdout[1],
                    self.stdout[2],
                    self.stdout[3],
                ]) as usize;
                if length > MAX_SANDBOX_LOOP_WORKER_FRAME_BYTES {
                    return Err(RuntimeProcessError::ExecutionFailed(
                        "sandbox loop worker emitted an oversized frame".to_string(),
                    ));
                }
                if self.stdout.len() >= FRAME_HEADER_BYTES + length {
                    self.stdout.drain(..FRAME_HEADER_BYTES);
                    return Ok(Some(self.stdout.drain(..length).collect()));
                }
            }

            let Some(output) = self.output.next().await else {
                if !self.stdout.is_empty() {
                    return Err(RuntimeProcessError::ExecutionFailed(
                        "sandbox loop worker exited with a partial frame".to_string(),
                    ));
                }
                if !self.diagnostic.is_empty() {
                    return Err(worker_diagnostic_exit_error(self.diagnostic.len()));
                }
                let exit_code = self
                    .docker
                    .inspect_exec(&self.exec_id)
                    .await
                    .ok()
                    .and_then(|inspection| inspection.exit_code);
                return Err(RuntimeProcessError::ExecutionFailed(format!(
                    "sandbox loop worker exited without an outcome (exit code {exit_code:?})"
                )));
            };
            match output.map_err(|error| {
                RuntimeProcessError::ExecutionFailed(format!(
                    "sandbox loop worker output failed: {error}"
                ))
            })? {
                LogOutput::StdOut { message } => self.stdout.extend(message),
                LogOutput::StdErr { message } => {
                    let diagnostic = String::from_utf8_lossy(&message);
                    super::append_with_limit(&mut self.diagnostic, &diagnostic, 8 * 1024);
                }
                _ => {}
            }
        }
    }

    async fn terminate(&mut self) -> Result<(), RuntimeProcessError> {
        self.terminate_inner().await
    }
}

#[async_trait]
impl SandboxLoopWorkerTransport for RebornScopedSandboxCommandTransport {
    async fn start_loop_worker(
        &self,
        request: SandboxLoopWorkerStartRequest,
    ) -> Result<Box<dyn SandboxLoopWorkerSession>, RuntimeProcessError> {
        super::reject_nul("sandbox loop worker executable", &request.executable)?;
        for argument in &request.args {
            super::reject_nul("sandbox loop worker argument", argument)?;
        }
        let workdir = RebornScopedSandboxCommandTransport::resolve_container_workdir(
            request.workdir.as_deref(),
        )?;
        let key = RebornSandboxUserKey::from_scope(&request.scope);
        let (workspace, activity, lifecycle) =
            self.begin_user_workspace(&request.scope, &key).await?;
        let container_user = self
            .config
            .container_identity
            .container_user(&workspace)
            .await?;
        let mut binds = self
            .config
            .mount_sources
            .prepare_container_binds(&workspace, None)
            .await?
            .into_iter()
            .map(|bind| bind.into_docker_bind())
            .collect::<Vec<_>>();
        self.config.append_broker_binds(&mut binds)?;
        binds.sort();

        let resolved_image = self.resolve_worker_image().await?;
        let bundle = match self.managed_egress.as_ref() {
            Some(managed) => Some(
                managed
                    .ensure_bundle(
                        &self.docker,
                        &key,
                        &request.scope.tenant_id,
                        &request.scope.user_id,
                    )
                    .await?,
            ),
            None => None,
        };
        if let (Some(managed), Some(bundle)) = (self.managed_egress.as_ref(), bundle.as_ref()) {
            managed
                .set_invocation(bundle, &request.scope.invocation_id)
                .await?;
        }
        let launch = self.user_container_launch_config(
            &request.scope,
            &resolved_image,
            bundle.as_ref(),
            container_user,
            binds,
        )?;
        let container_name = user_container::ensure_user_container(self, &key, launch).await?;
        let env = self.config.command_env_for_bundle(
            Default::default(),
            &[],
            self.managed_egress.as_deref(),
            bundle.as_ref(),
        )?;

        let mut command = vec![request.executable];
        command.extend(request.args);
        let created = self
            .docker
            .create_exec(
                &container_name,
                CreateExecOptions {
                    attach_stdin: Some(true),
                    attach_stdout: Some(true),
                    attach_stderr: Some(true),
                    tty: Some(false),
                    cmd: Some(command),
                    privileged: Some(false),
                    working_dir: Some(workdir.into_string()),
                    env: Some(env),
                    ..Default::default()
                },
            )
            .await
            .map_err(|error| {
                RuntimeProcessError::ExecutionFailed(format!(
                    "sandbox loop worker exec create failed: {error}"
                ))
            })?;
        let exec_id = created.id;
        let started = self
            .docker
            .start_exec(
                &exec_id,
                Some(StartExecOptions {
                    detach: false,
                    tty: false,
                    output_capacity: Some(64 * 1024),
                }),
            )
            .await
            .map_err(|error| {
                RuntimeProcessError::ExecutionFailed(format!(
                    "sandbox loop worker exec start failed: {error}"
                ))
            })?;
        let StartExecResults::Attached { output, input } = started else {
            return Err(RuntimeProcessError::ExecutionFailed(
                "sandbox loop worker unexpectedly detached".to_string(),
            ));
        };
        let process_id = wait_for_exec_pid(&self.docker, &exec_id).await?;

        // The activity guard pins the container. The lifecycle gate is released
        // so host-authorized nested shell calls can execute in this same worker.
        drop(lifecycle);
        Ok(Box::new(DockerLoopWorkerSession {
            docker: self.docker.clone(),
            container_name,
            exec_id,
            process_id,
            input: Some(input),
            output,
            stdout: VecDeque::new(),
            diagnostic: String::new(),
            _activity: activity,
            terminated: false,
        }))
    }
}
fn pipe_write_error(error: std::io::Error) -> RuntimeProcessError {
    RuntimeProcessError::ExecutionFailed(format!("sandbox loop worker input failed: {error}"))
}

fn worker_diagnostic_exit_error(diagnostic_bytes: usize) -> RuntimeProcessError {
    tracing::debug!(
        diagnostic_bytes,
        "sandbox loop worker exited with diagnostics"
    );
    RuntimeProcessError::ExecutionFailed(
        "sandbox loop worker exited with internal diagnostics".to_string(),
    )
}

async fn wait_for_exec_pid(
    docker: &bollard::Docker,
    exec_id: &str,
) -> Result<i64, RuntimeProcessError> {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let inspection = docker.inspect_exec(exec_id).await.map_err(|error| {
            RuntimeProcessError::ExecutionFailed(format!(
                "sandbox loop worker exec inspect failed: {error}"
            ))
        })?;
        if let Some(pid) = inspection.pid {
            return Ok(pid);
        }
        if inspection.running == Some(false) {
            return Err(RuntimeProcessError::ExecutionFailed(
                "sandbox loop worker exited before exposing a process id".to_string(),
            ));
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(RuntimeProcessError::ExecutionFailed(
                "sandbox loop worker did not expose a process id".to_string(),
            ));
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}

async fn terminate_process(
    docker: &bollard::Docker,
    container_name: &str,
    process_id: i64,
) -> Result<(), RuntimeProcessError> {
    let command = format!("kill -TERM {process_id} 2>/dev/null || true");
    let created = docker
        .create_exec(
            container_name,
            CreateExecOptions {
                attach_stdout: Some(false),
                attach_stderr: Some(false),
                cmd: Some(vec!["sh".to_string(), "-c".to_string(), command]),
                privileged: Some(false),
                working_dir: Some(CONTAINER_WORKSPACE_ROOT.to_string()),
                ..Default::default()
            },
        )
        .await
        .map_err(|error| {
            RuntimeProcessError::ExecutionFailed(format!(
                "sandbox loop worker termination create failed: {error}"
            ))
        })?;
    docker
        .start_exec(
            &created.id,
            Some(StartExecOptions {
                detach: true,
                tty: false,
                output_capacity: None,
            }),
        )
        .await
        .map_err(|error| {
            RuntimeProcessError::ExecutionFailed(format!(
                "sandbox loop worker termination failed: {error}"
            ))
        })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_diagnostic_exit_error_never_contains_worker_output() {
        let secret = "api_key=sk-worker-secret";
        let error = worker_diagnostic_exit_error(secret.len());

        assert!(!error.to_string().contains(secret));
        assert_eq!(
            error.to_string(),
            "process execution failed: sandbox loop worker exited with internal diagnostics"
        );
    }
}
