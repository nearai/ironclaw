//! Transport abstraction for the per-project sandbox.
//!
//! `SandboxTransport` is the seam between the host's `MountBackend`
//! implementation and the actual JSON-RPC channel into the daemon. The real
//! implementation in [`DockerTransport`](super::docker_transport::DockerTransport)
//! pipes NDJSON over `docker exec -i`. Tests can implement the trait with
//! an in-process channel that mirrors the daemon's behavior — that's how
//! `ContainerizedFilesystemBackend` is unit-tested without spinning up
//! Docker.
//!
//! Each call goes through `dispatch(&Request) -> Result<Response, MountError>`.
//! The transport is responsible for serializing concurrent calls (v1
//! requirement: one in-flight per container) and for surfacing IPC failures
//! as [`MountError::Backend`].

use async_trait::async_trait;
use ironclaw_engine::MountError;

use super::protocol::{Request, Response};

/// Trait implemented by anything that can dispatch a JSON-RPC request to
/// a sandbox daemon and read the matching response. Implementations are
/// expected to be `Arc<dyn ...>`-shareable across the per-project mount
/// table.
#[async_trait]
pub trait SandboxTransport: Send + Sync + std::fmt::Debug {
    /// Dispatch a request and wait for the matching response. Errors here
    /// are infrastructure failures (container down, daemon crashed, IPC
    /// broken) and surface to the engine as [`MountError::Backend`].
    /// Tool-level failures are returned via `Response::error` instead.
    async fn dispatch(&self, request: Request) -> Result<Response, MountError>;
}
