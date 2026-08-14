//! Interactive Docker stdio for the experimental ACP harness executor.
//!
//! This is deliberately narrower than the command transport: one pinned image,
//! one typed workspace bind, an explicit environment allowlist, and no Docker
//! flag escape hatch. The caller must terminate the returned session.

use std::{path::PathBuf, pin::Pin};

use bollard::{
    Docker,
    container::{
        AttachContainerOptions, Config, CreateContainerOptions, LogOutput, RemoveContainerOptions,
        StartContainerOptions,
    },
    models::HostConfig,
};
use futures_util::{Sink, Stream, StreamExt};
use ironclaw_host_api::ids::ThreadId;
use ironclaw_host_api::process::RuntimeProcessError;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

use super::connect_docker_with_retry;

const CONTAINER_WORKSPACE: &str = "/workspace";
const MAX_MEMORY_BYTES: i64 = 4 * 1024 * 1024 * 1024;
const MAX_PIDS: i64 = 512;
const CLEANUP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

pub type HarnessLineSink = Pin<Box<dyn Sink<String, Error = std::io::Error> + Send + 'static>>;
pub type HarnessLineStream = Pin<Box<dyn Stream<Item = std::io::Result<String>> + Send + 'static>>;

/// Opaque, reusable launch template. Credential values remain inside the lane;
/// the loop tier can only request a container for a typed thread id.
pub struct HarnessContainerTemplate {
    workspace_root: PathBuf,
    image: String,
    environment: Vec<String>,
    max_protocol_line_bytes: usize,
}

impl HarnessContainerTemplate {
    pub fn new(
        workspace_root: PathBuf,
        image: String,
        environment: Vec<String>,
        max_protocol_line_bytes: usize,
    ) -> Result<Self, RuntimeProcessError> {
        HarnessContainerConfig::validate(&image, &environment, max_protocol_line_bytes)?;
        Ok(Self {
            workspace_root,
            image,
            environment,
            max_protocol_line_bytes,
        })
    }

    pub fn workspace_for_thread(&self, thread_id: &ThreadId) -> PathBuf {
        let digest = Sha256::digest(thread_id.as_str().as_bytes());
        self.workspace_root.join(hex::encode(digest))
    }

    pub async fn start_for_thread(
        &self,
        thread_id: &ThreadId,
    ) -> Result<HarnessContainerSession, RuntimeProcessError> {
        HarnessContainerConfig::new(
            self.workspace_for_thread(thread_id),
            self.image.clone(),
            self.environment.clone(),
            self.max_protocol_line_bytes,
        )?
        .start()
        .await
    }
}

/// Trusted launch data assembled by the composition root.
///
/// `environment` contains already-resolved developer credentials and therefore
/// intentionally has no `Debug`, `Serialize`, or accessor API.
pub struct HarnessContainerConfig {
    workspace: PathBuf,
    image: String,
    environment: Vec<String>,
    max_protocol_line_bytes: usize,
}

impl HarnessContainerConfig {
    pub fn new(
        workspace: PathBuf,
        image: String,
        environment: Vec<String>,
        max_protocol_line_bytes: usize,
    ) -> Result<Self, RuntimeProcessError> {
        Self::validate(&image, &environment, max_protocol_line_bytes)?;
        Ok(Self {
            workspace,
            image,
            environment,
            max_protocol_line_bytes,
        })
    }

    fn validate(
        image: &str,
        environment: &[String],
        max_protocol_line_bytes: usize,
    ) -> Result<(), RuntimeProcessError> {
        if image.trim().is_empty() {
            return Err(invalid("harness image must not be empty"));
        }
        if max_protocol_line_bytes == 0 {
            return Err(invalid(
                "harness protocol line limit must be greater than zero",
            ));
        }
        if environment.iter().any(|entry| {
            entry.as_bytes().contains(&0)
                || !entry.contains('=')
                || entry.starts_with('=')
                || entry.starts_with("PATH=")
                || entry.starts_with("HOME=")
        }) {
            return Err(invalid("harness environment contains an invalid entry"));
        }
        Ok(())
    }

    pub async fn start(self) -> Result<HarnessContainerSession, RuntimeProcessError> {
        tokio::fs::create_dir_all(&self.workspace)
            .await
            .map_err(|error| invalid(format!("harness workspace could not be created: {error}")))?;
        let workspace = tokio::fs::canonicalize(&self.workspace)
            .await
            .map_err(|error| {
                invalid(format!("harness workspace could not be resolved: {error}"))
            })?;
        let workspace = workspace
            .to_str()
            .ok_or_else(|| invalid("harness workspace path is not valid UTF-8"))?;
        if workspace.contains(':') || workspace.contains('\n') || workspace.contains('\r') {
            return Err(invalid(
                "harness workspace path is not safe for a Docker bind",
            ));
        }

        let docker = connect_docker_with_retry().await?;
        let container_name = format!("ironclaw-harness-{}", uuid::Uuid::new_v4());
        let launch = Config {
            image: Some(self.image),
            working_dir: Some(CONTAINER_WORKSPACE.to_string()),
            env: Some(self.environment),
            open_stdin: Some(true),
            stdin_once: Some(false),
            attach_stdin: Some(true),
            attach_stdout: Some(true),
            attach_stderr: Some(true),
            tty: Some(false),
            host_config: Some(HostConfig {
                binds: Some(vec![format!("{workspace}:{CONTAINER_WORKSPACE}:rw")]),
                auto_remove: Some(false),
                network_mode: Some("bridge".to_string()),
                cap_drop: Some(vec!["ALL".to_string()]),
                security_opt: Some(vec!["no-new-privileges:true".to_string()]),
                readonly_rootfs: Some(true),
                pids_limit: Some(MAX_PIDS),
                memory: Some(MAX_MEMORY_BYTES),
                tmpfs: Some(
                    [(
                        "/tmp".to_string(),
                        "rw,nosuid,nodev,noexec,size=256m".to_string(),
                    )]
                    .into_iter()
                    .collect(),
                ),
                ..Default::default()
            }),
            ..Default::default()
        };
        let created = docker
            .create_container(
                Some(CreateContainerOptions {
                    name: container_name,
                    platform: None,
                }),
                launch,
            )
            .await
            .map_err(|error| invalid(format!("harness container create failed: {error}")))?;
        let container_id = created.id;

        let attached = match docker
            .attach_container(
                &container_id,
                Some(AttachContainerOptions::<String> {
                    stdin: Some(true),
                    stdout: Some(true),
                    stderr: Some(true),
                    stream: Some(true),
                    logs: Some(false),
                    ..Default::default()
                }),
            )
            .await
        {
            Ok(attached) => attached,
            Err(error) => {
                remove_best_effort(&docker, &container_id).await;
                return Err(invalid(format!("harness container attach failed: {error}")));
            }
        };
        if let Err(error) = docker
            .start_container(&container_id, None::<StartContainerOptions<String>>)
            .await
        {
            remove_best_effort(&docker, &container_id).await;
            return Err(invalid(format!("harness container start failed: {error}")));
        }

        let outgoing = Box::pin(futures_util::sink::unfold(
            attached.input,
            async move |mut input, mut line: String| {
                line.push('\n');
                input.write_all(line.as_bytes()).await?;
                input.flush().await?;
                Ok::<_, std::io::Error>(input)
            },
        ));
        let incoming = Box::pin(futures_util::stream::unfold(
            (attached.output, Vec::<u8>::new()),
            move |(mut output, mut buffered)| async move {
                loop {
                    if let Some(newline) = buffered.iter().position(|byte| *byte == b'\n') {
                        if newline > self.max_protocol_line_bytes {
                            return Some((
                                Err(std::io::Error::new(
                                    std::io::ErrorKind::InvalidData,
                                    "ACP protocol line exceeded configured limit",
                                )),
                                (output, Vec::new()),
                            ));
                        }
                        let mut line = buffered.drain(..=newline).collect::<Vec<_>>();
                        if line.last() == Some(&b'\n') {
                            line.pop();
                        }
                        if line.last() == Some(&b'\r') {
                            line.pop();
                        }
                        let decoded = String::from_utf8(line).map_err(|_| {
                            std::io::Error::new(
                                std::io::ErrorKind::InvalidData,
                                "ACP adapter emitted non-UTF-8 stdout",
                            )
                        });
                        return Some((decoded, (output, buffered)));
                    }
                    if buffered.len() > self.max_protocol_line_bytes {
                        return Some((
                            Err(std::io::Error::new(
                                std::io::ErrorKind::InvalidData,
                                "ACP protocol line exceeded configured limit",
                            )),
                            (output, Vec::new()),
                        ));
                    }
                    match output.next().await {
                        Some(Ok(LogOutput::StdOut { message }))
                        | Some(Ok(LogOutput::Console { message })) => {
                            buffered.extend_from_slice(&message);
                        }
                        Some(Ok(LogOutput::StdErr { .. })) => {
                            tracing::debug!("ACP harness adapter emitted stderr output");
                        }
                        Some(Ok(LogOutput::StdIn { .. })) => {}
                        Some(Err(error)) => {
                            return Some((
                                Err(std::io::Error::other(format!(
                                    "ACP container stream failed: {error}"
                                ))),
                                (output, Vec::new()),
                            ));
                        }
                        None if buffered.is_empty() => return None,
                        None => {
                            let decoded =
                                String::from_utf8(std::mem::take(&mut buffered)).map_err(|_| {
                                    std::io::Error::new(
                                        std::io::ErrorKind::InvalidData,
                                        "ACP adapter emitted non-UTF-8 stdout",
                                    )
                                });
                            return Some((decoded, (output, buffered)));
                        }
                    }
                }
            },
        ));

        Ok(HarnessContainerSession {
            docker,
            container_id: Some(container_id),
            outgoing: Some(outgoing),
            incoming: Some(incoming),
        })
    }
}

#[must_use = "harness containers must be explicitly terminated"]
pub struct HarnessContainerSession {
    docker: Docker,
    container_id: Option<String>,
    outgoing: Option<HarnessLineSink>,
    incoming: Option<HarnessLineStream>,
}

impl HarnessContainerSession {
    pub fn take_transport(
        &mut self,
    ) -> Result<(HarnessLineSink, HarnessLineStream), RuntimeProcessError> {
        let outgoing = self
            .outgoing
            .take()
            .ok_or_else(|| invalid("harness container transport was already taken"))?;
        let incoming = self
            .incoming
            .take()
            .ok_or_else(|| invalid("harness container transport was already taken"))?;
        Ok((outgoing, incoming))
    }

    pub async fn terminate(mut self) -> Result<(), RuntimeProcessError> {
        let container_id = self
            .container_id
            .take()
            .ok_or_else(|| invalid("ACP harness container was already removed"))?;
        match tokio::time::timeout(CLEANUP_TIMEOUT, remove(&self.docker, &container_id)).await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(_)) => {
                self.container_id = Some(container_id);
                Err(invalid("ACP harness container removal failed"))
            }
            Err(_) => {
                self.container_id = Some(container_id);
                Err(invalid("timed out removing ACP harness container"))
            }
        }
    }
}

impl Drop for HarnessContainerSession {
    fn drop(&mut self) {
        let Some(container_id) = self.container_id.take() else {
            return;
        };
        let docker = self.docker.clone();
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            tracing::debug!("could not schedule dropped ACP harness container cleanup");
            return;
        };
        let _cleanup = runtime.spawn(async move {
            match tokio::time::timeout(CLEANUP_TIMEOUT, remove(&docker, &container_id)).await {
                Ok(Ok(())) => {}
                Ok(Err(_)) => {
                    tracing::debug!("failed to remove dropped ACP harness container");
                }
                Err(_) => {
                    tracing::debug!("timed out removing dropped ACP harness container");
                }
            }
        });
    }
}

async fn remove_best_effort(docker: &Docker, container_id: &str) {
    if let Err(error) = remove(docker, container_id).await {
        tracing::debug!(
            ?error,
            "best-effort removal of ACP harness container failed"
        );
    }
}

async fn remove(docker: &Docker, container_id: &str) -> Result<(), bollard::errors::Error> {
    docker
        .remove_container(
            container_id,
            Some(RemoveContainerOptions {
                force: true,
                ..Default::default()
            }),
        )
        .await
}

fn invalid(reason: impl Into<String>) -> RuntimeProcessError {
    RuntimeProcessError::ExecutionFailed(reason.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_config_rejects_ambient_process_environment_entries() {
        let error = HarnessContainerConfig::new(
            PathBuf::from("workspace"),
            "image:dev".to_string(),
            vec!["HOME=/host/home".to_string()],
            1024,
        )
        .err()
        .expect("ambient HOME must be rejected");
        assert!(error.to_string().contains("invalid entry"));
    }
}
