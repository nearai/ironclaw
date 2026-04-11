//! Docker backend for the `ContainerRuntime` trait.
//!
//! Wraps bollard API calls behind the runtime abstraction so that
//! `SandboxManager`, `ContainerJobManager`, and `SandboxReaper` can
//! operate against any compiled-in backend.

use std::path::Path;
use std::time::Duration;

use bollard::Docker;
use bollard::container::{
    Config, CreateContainerOptions, LogOutput, LogsOptions, RemoveContainerOptions,
    StartContainerOptions, StopContainerOptions, WaitContainerOptions,
};
use bollard::exec::{CreateExecOptions, StartExecResults};
use bollard::models::HostConfig;
use chrono::{DateTime, Utc};
use futures::StreamExt;
use uuid::Uuid;

use crate::sandbox::container::connect_docker;
use crate::sandbox::error::SandboxError;
use crate::sandbox::runtime::{
    ContainerRuntime, ManagedWorkload, RuntimeDetection, RuntimeStatus, WorkloadOutput,
    WorkloadSpec,
};

/// Append `text` into `buffer` up to `limit` bytes without breaking UTF-8.
/// Returns `true` when truncation occurred.
fn append_with_limit(buffer: &mut String, text: &str, limit: usize) -> bool {
    if text.is_empty() {
        return false;
    }
    if buffer.len() >= limit {
        return true;
    }
    let remaining = limit - buffer.len();
    if text.len() <= remaining {
        buffer.push_str(text);
        return false;
    }
    let end = crate::util::floor_char_boundary(text, remaining);
    buffer.push_str(&text[..end]);
    true
}

/// Docker implementation of `ContainerRuntime`.
pub struct DockerRuntime {
    docker: Docker,
}

impl DockerRuntime {
    /// Create a new `DockerRuntime` from an existing bollard connection.
    pub fn new(docker: Docker) -> Self {
        Self { docker }
    }

    /// Connect to the Docker daemon and return a runtime instance.
    pub async fn connect() -> Result<Self, SandboxError> {
        let docker = connect_docker().await?;
        Ok(Self::new(docker))
    }

    /// Get a reference to the underlying bollard client.
    pub fn inner(&self) -> &Docker {
        &self.docker
    }
}

#[async_trait::async_trait]
impl ContainerRuntime for DockerRuntime {
    fn name(&self) -> &'static str {
        "docker"
    }

    // ── Readiness ──────────────────────────────────────────────────

    async fn is_available(&self) -> bool {
        self.docker.ping().await.is_ok()
    }

    async fn detect(&self) -> RuntimeDetection {
        let detection = crate::sandbox::detect::check_docker().await;
        RuntimeDetection {
            status: match detection.status {
                crate::sandbox::detect::DockerStatus::Available => RuntimeStatus::Available,
                crate::sandbox::detect::DockerStatus::NotInstalled => RuntimeStatus::NotInstalled,
                crate::sandbox::detect::DockerStatus::NotRunning => RuntimeStatus::NotRunning,
                crate::sandbox::detect::DockerStatus::Disabled => RuntimeStatus::Disabled,
            },
            runtime_name: "docker",
            install_hint: detection.platform.install_hint().to_string(),
            start_hint: detection.platform.start_hint().to_string(),
        }
    }

    // ── Image management ───────────────────────────────────────────

    async fn image_exists(&self, image: &str) -> bool {
        self.docker.inspect_image(image).await.is_ok()
    }

    async fn pull_image(&self, image: &str) -> Result<(), SandboxError> {
        use bollard::image::CreateImageOptions;

        tracing::info!("Pulling image: {}", image);

        let options = CreateImageOptions {
            from_image: image.to_string(),
            ..Default::default()
        };

        let mut stream = self.docker.create_image(Some(options), None, None);

        while let Some(result) = stream.next().await {
            match result {
                Ok(info) => {
                    if let Some(status) = info.status {
                        tracing::debug!("Pull status: {}", status);
                    }
                }
                Err(e) => {
                    return Err(SandboxError::ContainerCreationFailed {
                        reason: format!("image pull failed: {}", e),
                    });
                }
            }
        }

        tracing::info!("Successfully pulled image: {}", image);
        Ok(())
    }

    async fn build_image(&self, image: &str, dockerfile_path: &Path) -> Result<(), SandboxError> {
        use tokio::io::AsyncBufReadExt;
        use tokio::process::Command;

        const MAX_STDERR_CAPTURE: usize = 4096;

        let canonical =
            dockerfile_path
                .canonicalize()
                .map_err(|e| SandboxError::ContainerCreationFailed {
                    reason: format!(
                        "cannot resolve Dockerfile path '{}': {}",
                        dockerfile_path.display(),
                        e
                    ),
                })?;

        let context_dir =
            canonical
                .parent()
                .ok_or_else(|| SandboxError::ContainerCreationFailed {
                    reason: format!(
                        "Dockerfile path '{}' has no parent directory",
                        canonical.display()
                    ),
                })?;

        tracing::info!("Building image from {}: {}", canonical.display(), image);

        let mut child = Command::new("docker")
            .arg("build")
            .arg("-f")
            .arg(&canonical)
            .arg("-t")
            .arg(image)
            .arg(".")
            .current_dir(context_dir)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| SandboxError::ContainerCreationFailed {
                reason: format!("failed to run docker build: {}", e),
            })?;

        let mut stdout_lines = tokio::io::BufReader::new(child.stdout.take().ok_or_else(|| {
            SandboxError::ContainerCreationFailed {
                reason: "stdout pipe missing".to_string(),
            }
        })?)
        .lines();
        let mut stderr_lines = tokio::io::BufReader::new(child.stderr.take().ok_or_else(|| {
            SandboxError::ContainerCreationFailed {
                reason: "stderr pipe missing".to_string(),
            }
        })?)
        .lines();

        let mut stderr_capture = String::new();
        let mut stdout_done = false;
        let mut stderr_done = false;

        while !stdout_done || !stderr_done {
            tokio::select! {
                line = stdout_lines.next_line(), if !stdout_done => {
                    match line {
                        Ok(Some(line)) => tracing::info!("[docker build] {}", line),
                        Ok(None) => stdout_done = true,
                        Err(e) => {
                            tracing::warn!("Error reading docker build stdout: {}", e);
                            stdout_done = true;
                        }
                    }
                },
                line = stderr_lines.next_line(), if !stderr_done => {
                    match line {
                        Ok(Some(line)) => {
                            tracing::info!("[docker build] {}", line);
                            if stderr_capture.len() < MAX_STDERR_CAPTURE {
                                stderr_capture.push_str(&line);
                                stderr_capture.push('\n');
                            }
                        }
                        Ok(None) => stderr_done = true,
                        Err(e) => {
                            tracing::warn!("Error reading docker build stderr: {}", e);
                            stderr_done = true;
                        }
                    }
                },
            }
        }

        let status = child
            .wait()
            .await
            .map_err(|e| SandboxError::ContainerCreationFailed {
                reason: format!("docker build wait failed: {}", e),
            })?;

        if !status.success() {
            let code = status
                .code()
                .map_or("unknown".to_string(), |c| c.to_string());
            return Err(SandboxError::ContainerCreationFailed {
                reason: format!(
                    "docker build failed (exit {}): {}",
                    code,
                    stderr_capture.trim_end()
                ),
            });
        }

        tracing::info!("Successfully built image: {}", image);
        Ok(())
    }

    // ── Workload lifecycle ─────────────────────────────────────────

    async fn create_and_start_workload(&self, spec: &WorkloadSpec) -> Result<String, SandboxError> {
        let binds: Vec<String> = spec.mounts.iter().map(|m| m.as_bind_string()).collect();

        let host_config = HostConfig {
            binds: if binds.is_empty() { None } else { Some(binds) },
            memory: spec.memory_bytes,
            cpu_shares: spec.cpu_shares,
            auto_remove: Some(spec.auto_remove),
            network_mode: spec.network_mode.clone(),
            extra_hosts: if spec.extra_hosts.is_empty() {
                None
            } else {
                Some(spec.extra_hosts.clone())
            },
            cap_drop: if spec.cap_drop.is_empty() {
                None
            } else {
                Some(spec.cap_drop.clone())
            },
            cap_add: if spec.cap_add.is_empty() {
                None
            } else {
                Some(spec.cap_add.clone())
            },
            security_opt: if spec.security_opts.is_empty() {
                None
            } else {
                Some(spec.security_opts.clone())
            },
            readonly_rootfs: Some(spec.readonly_rootfs),
            tmpfs: if spec.tmpfs_mounts.is_empty() {
                None
            } else {
                Some(spec.tmpfs_mounts.clone())
            },
            ..Default::default()
        };

        let config = Config {
            image: Some(spec.image.clone()),
            cmd: if spec.command.is_empty() {
                None
            } else {
                Some(spec.command.clone())
            },
            working_dir: Some(spec.working_dir.clone()),
            env: if spec.env.is_empty() {
                None
            } else {
                Some(spec.env.clone())
            },
            host_config: Some(host_config),
            user: spec.user.clone(),
            labels: if spec.labels.is_empty() {
                None
            } else {
                Some(spec.labels.clone())
            },
            ..Default::default()
        };

        let options = CreateContainerOptions {
            name: spec.name.clone(),
            ..Default::default()
        };

        let response = self
            .docker
            .create_container(Some(options), config)
            .await
            .map_err(|e| SandboxError::ContainerCreationFailed {
                reason: e.to_string(),
            })?;

        let container_id = response.id;

        self.docker
            .start_container(&container_id, None::<StartContainerOptions<String>>)
            .await
            .map_err(|e| SandboxError::ContainerStartFailed {
                reason: e.to_string(),
            })?;

        Ok(container_id)
    }

    async fn wait_workload(&self, workload_id: &str) -> Result<i64, SandboxError> {
        let mut wait_stream = self.docker.wait_container(
            workload_id,
            Some(WaitContainerOptions {
                condition: "not-running",
            }),
        );

        match wait_stream.next().await {
            Some(Ok(response)) => Ok(response.status_code),
            Some(Err(e)) => Err(SandboxError::ExecutionFailed {
                reason: format!("wait failed: {}", e),
            }),
            None => Err(SandboxError::ExecutionFailed {
                reason: "workload wait stream ended unexpectedly".to_string(),
            }),
        }
    }

    async fn stop_workload(
        &self,
        workload_id: &str,
        grace_period_secs: u32,
    ) -> Result<(), SandboxError> {
        self.docker
            .stop_container(
                workload_id,
                Some(StopContainerOptions {
                    t: grace_period_secs as i64,
                }),
            )
            .await
            .map_err(|e| SandboxError::ExecutionFailed {
                reason: format!("stop failed: {}", e),
            })
    }

    async fn remove_workload(&self, workload_id: &str) -> Result<(), SandboxError> {
        self.docker
            .remove_container(
                workload_id,
                Some(RemoveContainerOptions {
                    force: true,
                    ..Default::default()
                }),
            )
            .await
            .map_err(|e| SandboxError::ExecutionFailed {
                reason: format!("remove failed: {}", e),
            })
    }

    // ── Execution ──────────────────────────────────────────────────

    async fn exec_in_workload(
        &self,
        workload_id: &str,
        command: &[&str],
        working_dir: &str,
        max_output: usize,
        timeout: Duration,
    ) -> Result<WorkloadOutput, SandboxError> {
        let start_time = std::time::Instant::now();

        let exec = self
            .docker
            .create_exec(
                workload_id,
                CreateExecOptions {
                    cmd: Some(command.to_vec()),
                    attach_stdout: Some(true),
                    attach_stderr: Some(true),
                    working_dir: Some(working_dir),
                    ..Default::default()
                },
            )
            .await
            .map_err(|e| SandboxError::ExecutionFailed {
                reason: format!("exec create failed: {}", e),
            })?;

        let result =
            tokio::time::timeout(timeout, async { self.run_exec(&exec.id, max_output).await })
                .await;

        match result {
            Ok(Ok(mut output)) => {
                output.duration = start_time.elapsed();
                Ok(output)
            }
            Ok(Err(e)) => Err(e),
            Err(_) => Err(SandboxError::Timeout(timeout)),
        }
    }

    // ── Logs ───────────────────────────────────────────────────────

    async fn collect_logs(
        &self,
        workload_id: &str,
        max_output: usize,
    ) -> Result<(String, String, bool), SandboxError> {
        let options = LogsOptions::<String> {
            stdout: true,
            stderr: true,
            follow: false,
            ..Default::default()
        };

        let mut stream = self.docker.logs(workload_id, Some(options));

        let mut stdout = String::new();
        let mut stderr = String::new();
        let mut truncated = false;
        let half_max = max_output / 2;

        while let Some(result) = stream.next().await {
            match result {
                Ok(LogOutput::StdOut { message }) => {
                    let text = String::from_utf8_lossy(&message);
                    truncated |= append_with_limit(&mut stdout, &text, half_max);
                }
                Ok(LogOutput::StdErr { message }) => {
                    let text = String::from_utf8_lossy(&message);
                    truncated |= append_with_limit(&mut stderr, &text, half_max);
                }
                Ok(_) => {}
                Err(e) => {
                    tracing::warn!("Error reading workload logs: {}", e);
                }
            }
        }

        Ok((stdout, stderr, truncated))
    }

    // ── Discovery ──────────────────────────────────────────────────

    async fn list_managed_workloads(
        &self,
        label_key: &str,
    ) -> Result<Vec<ManagedWorkload>, SandboxError> {
        use bollard::container::ListContainersOptions;
        use std::collections::HashMap;

        let mut filters = HashMap::new();
        filters.insert("label", vec![label_key]);

        let options = ListContainersOptions {
            all: true,
            filters,
            ..Default::default()
        };

        let summaries = self
            .docker
            .list_containers(Some(options))
            .await
            .map_err(|e| SandboxError::ExecutionFailed {
                reason: format!("list containers failed: {}", e),
            })?;

        let mut result = Vec::new();

        for summary in summaries {
            let container_id = match summary.id {
                Some(id) => id,
                None => continue,
            };

            let labels = summary.labels.unwrap_or_default();

            let job_id = match labels.get(label_key).and_then(|s| s.parse::<Uuid>().ok()) {
                Some(id) => id,
                None => {
                    tracing::warn!(
                        container_id = %&container_id[..12.min(container_id.len())],
                        label_key = %label_key,
                        "Managed workload missing valid job_id label"
                    );
                    continue;
                }
            };

            let created_at = match labels
                .get("ironclaw.created_at")
                .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                .map(|dt| dt.with_timezone(&Utc))
                .or_else(|| {
                    summary
                        .created
                        .and_then(|ts| DateTime::from_timestamp(ts, 0))
                }) {
                Some(ts) => ts,
                None => {
                    tracing::warn!(
                        container_id = %&container_id[..12.min(container_id.len())],
                        "Could not determine creation time for workload, skipping"
                    );
                    continue;
                }
            };

            result.push(ManagedWorkload {
                workload_id: container_id,
                job_id,
                created_at,
            });
        }

        Ok(result)
    }

    // ── Networking ─────────────────────────────────────────────────

    fn orchestrator_host(&self) -> &str {
        "host.docker.internal"
    }
}

impl DockerRuntime {
    /// Run an exec and collect output (internal helper).
    async fn run_exec(
        &self,
        exec_id: &str,
        max_output: usize,
    ) -> Result<WorkloadOutput, SandboxError> {
        let start_result = self.docker.start_exec(exec_id, None).await.map_err(|e| {
            SandboxError::ExecutionFailed {
                reason: format!("exec start failed: {}", e),
            }
        })?;

        let mut stdout = String::new();
        let mut stderr = String::new();
        let mut truncated = false;
        let half_max = max_output / 2;

        if let StartExecResults::Attached { mut output, .. } = start_result {
            while let Some(result) = output.next().await {
                match result {
                    Ok(LogOutput::StdOut { message }) => {
                        let text = String::from_utf8_lossy(&message);
                        truncated |= append_with_limit(&mut stdout, &text, half_max);
                    }
                    Ok(LogOutput::StdErr { message }) => {
                        let text = String::from_utf8_lossy(&message);
                        truncated |= append_with_limit(&mut stderr, &text, half_max);
                    }
                    Ok(_) => {}
                    Err(e) => {
                        tracing::warn!("Error reading exec output: {}", e);
                    }
                }
            }
        }

        let inspect =
            self.docker
                .inspect_exec(exec_id)
                .await
                .map_err(|e| SandboxError::ExecutionFailed {
                    reason: format!("exec inspect failed: {}", e),
                })?;

        let exit_code = inspect.exit_code.unwrap_or(-1);

        Ok(WorkloadOutput {
            exit_code,
            stdout,
            stderr,
            duration: Duration::ZERO,
            truncated,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_with_limit_truncates_on_utf8_boundary() {
        let mut out = String::new();
        let truncated = append_with_limit(&mut out, "ab🙂cd", 5);
        assert!(truncated);
        assert_eq!(out, "ab");
    }

    #[test]
    fn append_with_limit_marks_truncated_when_full() {
        let mut out = "abc".to_string();
        let truncated = append_with_limit(&mut out, "z", 3);
        assert!(truncated);
        assert_eq!(out, "abc");
    }

    #[test]
    fn append_with_limit_appends_without_truncation() {
        let mut out = String::new();
        let truncated = append_with_limit(&mut out, "hello", 10);
        assert!(!truncated);
        assert_eq!(out, "hello");
    }
}
