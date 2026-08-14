//! Process placement for external ACP agents.
//!
//! The harness executor depends only on [`AgentPlacement`]. Host and Docker
//! launch policy stays behind this boundary, while both implementations expose
//! the same line-oriented ACP transport and cleanup contract.

use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    pin::Pin,
    time::Duration,
};

use async_trait::async_trait;
use futures_util::{Sink, SinkExt, Stream, StreamExt};
use ironclaw_host_api::ids::ThreadId;
use ironclaw_sandbox::sandbox_process::{HarnessContainerSession, HarnessContainerTemplate};
use sha2::{Digest, Sha256};
use tokio::{
    io::{AsyncReadExt, AsyncWrite},
    process::{Child, Command},
    task::JoinHandle,
};
use tokio_util::codec::{FramedRead, FramedWrite, LinesCodec, LinesCodecError};

const CLEANUP_TIMEOUT: Duration = Duration::from_secs(10);

pub type AgentLineSink = Pin<Box<dyn Sink<String, Error = std::io::Error> + Send + 'static>>;
pub type AgentLineStream = Pin<Box<dyn Stream<Item = std::io::Result<String>> + Send + 'static>>;

/// Placement-owned process handle. The executor can use and terminate it, but
/// cannot observe whether it is backed by a host process or a container.
#[must_use = "agent processes must be explicitly terminated"]
pub struct AgentProcess {
    workspace: PathBuf,
    working_directory: PathBuf,
    outgoing: Option<AgentLineSink>,
    incoming: Option<AgentLineStream>,
    control: Option<Box<dyn AgentProcessControl>>,
}

impl AgentProcess {
    fn new(
        workspace: PathBuf,
        working_directory: PathBuf,
        outgoing: AgentLineSink,
        incoming: AgentLineStream,
        control: Box<dyn AgentProcessControl>,
    ) -> Self {
        Self {
            workspace,
            working_directory,
            outgoing: Some(outgoing),
            incoming: Some(incoming),
            control: Some(control),
        }
    }

    /// Persistent host path shared by successive processes for this thread.
    pub fn workspace(&self) -> &Path {
        &self.workspace
    }

    /// Absolute workspace path as seen by the placed process.
    pub fn working_directory(&self) -> &Path {
        &self.working_directory
    }

    pub fn take_transport(
        &mut self,
    ) -> Result<(AgentLineSink, AgentLineStream), AgentPlacementError> {
        let outgoing = self
            .outgoing
            .take()
            .ok_or(AgentPlacementError::TransportAlreadyTaken)?;
        let incoming = self
            .incoming
            .take()
            .ok_or(AgentPlacementError::TransportAlreadyTaken)?;
        Ok((outgoing, incoming))
    }

    pub async fn terminate(mut self) -> Result<(), AgentPlacementError> {
        self.outgoing.take();
        self.incoming.take();
        let control = self
            .control
            .take()
            .ok_or(AgentPlacementError::AlreadyTerminated)?;
        control.terminate().await
    }
}

#[async_trait]
trait AgentProcessControl: Send {
    async fn terminate(self: Box<Self>) -> Result<(), AgentPlacementError>;
}

#[async_trait]
pub trait AgentPlacement: Send + Sync {
    fn workspace_for_thread(&self, thread_id: &ThreadId) -> PathBuf;

    async fn spawn(&self, thread_id: &ThreadId) -> Result<AgentProcess, AgentPlacementError>;
}

/// Runs the configured adapter directly on the host in a thread-scoped working
/// directory. Only explicitly supplied environment entries are inherited.
pub struct HostAgentPlacement {
    workspace_root: PathBuf,
    program: OsString,
    arguments: Vec<OsString>,
    environment: Vec<(OsString, OsString)>,
    max_protocol_line_bytes: usize,
}

impl HostAgentPlacement {
    pub fn new(
        workspace_root: PathBuf,
        program: OsString,
        arguments: Vec<OsString>,
        environment: Vec<String>,
        max_protocol_line_bytes: usize,
    ) -> Result<Self, AgentPlacementError> {
        if program.is_empty() {
            return Err(AgentPlacementError::InvalidConfig(
                "harness command must not be empty",
            ));
        }
        let environment = validate_environment(environment, max_protocol_line_bytes)?;
        Ok(Self {
            workspace_root,
            program,
            arguments,
            environment,
            max_protocol_line_bytes,
        })
    }
}

#[async_trait]
impl AgentPlacement for HostAgentPlacement {
    fn workspace_for_thread(&self, thread_id: &ThreadId) -> PathBuf {
        thread_workspace(&self.workspace_root, thread_id)
    }

    async fn spawn(&self, thread_id: &ThreadId) -> Result<AgentProcess, AgentPlacementError> {
        let workspace = prepare_workspace(self.workspace_for_thread(thread_id)).await?;
        let home = workspace.join(".home");
        tokio::fs::create_dir_all(&home)
            .await
            .map_err(AgentPlacementError::Workspace)?;

        let mut command = Command::new(&self.program);
        command
            .args(&self.arguments)
            .current_dir(&workspace)
            .env_clear()
            .env("HOME", home)
            .env("PATH", trusted_host_path())
            .envs(self.environment.iter().cloned())
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        let mut child = command.spawn().map_err(AgentPlacementError::Spawn)?;
        let stdin = child
            .stdin
            .take()
            .ok_or(AgentPlacementError::MissingPipe("stdin"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or(AgentPlacementError::MissingPipe("stdout"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or(AgentPlacementError::MissingPipe("stderr"))?;

        let outgoing = boxed_line_sink(stdin);
        let incoming = Box::pin(
            FramedRead::new(
                stdout,
                LinesCodec::new_with_max_length(self.max_protocol_line_bytes),
            )
            .map(|line| line.map_err(codec_error)),
        );
        let stderr_task = tokio::spawn(async move {
            let mut stderr = stderr;
            let mut buffer = [0_u8; 4096];
            loop {
                match stderr.read(&mut buffer).await {
                    Ok(0) => break,
                    Ok(_) => tracing::debug!("ACP harness adapter emitted stderr output"),
                    Err(_) => {
                        tracing::debug!("ACP harness adapter stderr stream failed");
                        break;
                    }
                }
            }
        });

        Ok(AgentProcess::new(
            workspace.clone(),
            workspace,
            outgoing,
            incoming,
            Box::new(HostProcessControl { child, stderr_task }),
        ))
    }
}

/// Runs the configured adapter through the existing hardened Docker sandbox
/// lane while presenting the same process contract as host placement.
pub struct DockerAgentPlacement {
    template: HarnessContainerTemplate,
}

impl DockerAgentPlacement {
    pub fn new(
        workspace_root: PathBuf,
        image: String,
        environment: Vec<String>,
        max_protocol_line_bytes: usize,
    ) -> Result<Self, AgentPlacementError> {
        let template = HarnessContainerTemplate::new(
            workspace_root,
            image,
            environment,
            max_protocol_line_bytes,
        )
        .map_err(|error| AgentPlacementError::Docker(error.to_string()))?;
        Ok(Self { template })
    }
}

#[async_trait]
impl AgentPlacement for DockerAgentPlacement {
    fn workspace_for_thread(&self, thread_id: &ThreadId) -> PathBuf {
        self.template.workspace_for_thread(thread_id)
    }

    async fn spawn(&self, thread_id: &ThreadId) -> Result<AgentProcess, AgentPlacementError> {
        let workspace = self.template.workspace_for_thread(thread_id);
        let mut session = self
            .template
            .start_for_thread(thread_id)
            .await
            .map_err(|error| AgentPlacementError::Docker(error.to_string()))?;
        let (outgoing, incoming) = session
            .take_transport()
            .map_err(|error| AgentPlacementError::Docker(error.to_string()))?;
        Ok(AgentProcess::new(
            workspace,
            PathBuf::from("/workspace"),
            outgoing,
            incoming,
            Box::new(DockerProcessControl { session }),
        ))
    }
}

struct HostProcessControl {
    child: Child,
    stderr_task: JoinHandle<()>,
}

#[async_trait]
impl AgentProcessControl for HostProcessControl {
    async fn terminate(mut self: Box<Self>) -> Result<(), AgentPlacementError> {
        let running = self
            .child
            .try_wait()
            .map_err(AgentPlacementError::Cleanup)?
            .is_none();
        if running {
            self.child
                .start_kill()
                .map_err(AgentPlacementError::Cleanup)?;
        }
        let wait = tokio::time::timeout(CLEANUP_TIMEOUT, self.child.wait()).await;
        self.stderr_task.abort();
        match wait {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(error)) => Err(AgentPlacementError::Cleanup(error)),
            Err(_) => Err(AgentPlacementError::CleanupTimeout),
        }
    }
}

impl Drop for HostProcessControl {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
        self.stderr_task.abort();
    }
}

struct DockerProcessControl {
    session: HarnessContainerSession,
}

#[async_trait]
impl AgentProcessControl for DockerProcessControl {
    async fn terminate(self: Box<Self>) -> Result<(), AgentPlacementError> {
        let DockerProcessControl { session } = *self;
        session
            .terminate()
            .await
            .map_err(|error| AgentPlacementError::Docker(error.to_string()))
    }
}

fn boxed_line_sink(writer: impl AsyncWrite + Send + Unpin + 'static) -> AgentLineSink {
    Box::pin(SinkExt::<String>::sink_map_err(
        FramedWrite::new(writer, LinesCodec::new()),
        codec_error,
    ))
}

fn codec_error(error: LinesCodecError) -> std::io::Error {
    match error {
        LinesCodecError::Io(error) => error,
        LinesCodecError::MaxLineLengthExceeded => std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "ACP protocol line exceeded configured limit",
        ),
    }
}

fn validate_environment(
    environment: Vec<String>,
    max_protocol_line_bytes: usize,
) -> Result<Vec<(OsString, OsString)>, AgentPlacementError> {
    if max_protocol_line_bytes == 0 {
        return Err(AgentPlacementError::InvalidConfig(
            "harness protocol line limit must be greater than zero",
        ));
    }
    environment
        .into_iter()
        .map(|entry| {
            if entry.as_bytes().contains(&0) {
                return Err(AgentPlacementError::InvalidConfig(
                    "harness environment contains an invalid entry",
                ));
            }
            let (name, value) = entry
                .split_once('=')
                .ok_or(AgentPlacementError::InvalidConfig(
                    "harness environment contains an invalid entry",
                ))?;
            if name.is_empty() || matches!(name, "PATH" | "HOME") {
                return Err(AgentPlacementError::InvalidConfig(
                    "harness environment contains an invalid entry",
                ));
            }
            Ok((OsString::from(name), OsString::from(value)))
        })
        .collect()
}

async fn prepare_workspace(workspace: PathBuf) -> Result<PathBuf, AgentPlacementError> {
    tokio::fs::create_dir_all(&workspace)
        .await
        .map_err(AgentPlacementError::Workspace)?;
    tokio::fs::canonicalize(workspace)
        .await
        .map_err(AgentPlacementError::Workspace)
}

fn thread_workspace(root: &Path, thread_id: &ThreadId) -> PathBuf {
    let digest = Sha256::digest(thread_id.as_str().as_bytes());
    root.join(hex::encode(digest))
}

fn trusted_host_path() -> OsString {
    std::env::var_os("PATH").unwrap_or_else(|| OsString::from("/usr/local/bin:/usr/bin:/bin"))
}

#[derive(Debug, thiserror::Error)]
pub enum AgentPlacementError {
    #[error("invalid agent placement: {0}")]
    InvalidConfig(&'static str),
    #[error("agent workspace preparation failed: {0}")]
    Workspace(std::io::Error),
    #[error("agent process spawn failed: {0}")]
    Spawn(std::io::Error),
    #[error("agent process did not expose {0}")]
    MissingPipe(&'static str),
    #[error("agent process transport was already taken")]
    TransportAlreadyTaken,
    #[error("agent process was already terminated")]
    AlreadyTerminated,
    #[error("agent process cleanup failed: {0}")]
    Cleanup(std::io::Error),
    #[error("agent process cleanup timed out")]
    CleanupTimeout,
    #[error("Docker agent placement failed: {0}")]
    Docker(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_placement_rejects_ambient_environment_overrides() {
        let error = HostAgentPlacement::new(
            PathBuf::from("workspace"),
            OsString::from("agent"),
            Vec::new(),
            vec!["HOME=/host/home".to_string()],
            1024,
        )
        .err()
        .expect("ambient HOME must be rejected");
        assert!(error.to_string().contains("invalid entry"));
    }
}
