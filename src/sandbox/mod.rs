//! Container execution sandbox for secure command execution.
//!
//! This module provides a complete sandboxing solution for running untrusted commands:
//! - **Container isolation**: Commands run in ephemeral containers (Docker or Kubernetes)
//! - **Network proxy**: All network traffic goes through a validating proxy
//! - **Credential injection**: Secrets are injected by the proxy, never exposed in containers
//! - **Resource limits**: Memory, CPU, and timeout enforcement
//!
//! # Kubernetes caveats
//!
//! - **One-shot sandboxes fail closed** on Kubernetes because the runtime cannot
//!   provide the host bind mounts and host-local proxy required by the sandboxed
//!   policies. Use Docker for sandboxed command execution until a K8s-native
//!   replacement exists.
//! - **Worker jobs with host-backed mounts** (project workspaces, per-job MCP JSON)
//!   are rejected on Kubernetes for the same reason. Callers must avoid those paths
//!   or implement explicit data/config delivery through the orchestrator.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────────────────┐
//! │                           Sandbox System                                     │
//! │                                                                              │
//! │  ┌─────────────────────────────────────────────────────────────────────┐    │
//! │  │                        SandboxManager                                │    │
//! │  │                                                                      │    │
//! │  │  • Coordinates workload creation and execution                      │    │
//! │  │  • Manages proxy lifecycle                                          │    │
//! │  │  • Enforces resource limits                                         │    │
//! │  └─────────────────────────────────────────────────────────────────────┘    │
//! │           │                              │                                   │
//! │           ▼                              ▼                                   │
//! │  ┌──────────────────┐          ┌───────────────────┐                        │
//! │  │  Container       │          │   Network Proxy   │                        │
//! │  │  Runtime         │          │                   │                        │
//! │  │  (Docker or K8s) │          │  • Allowlist      │                        │
//! │  │  • Create        │◀────────▶│  • Credentials    │                        │
//! │  │  • Execute       │          │  • Logging        │                        │
//! │  │  • Cleanup       │          │                   │                        │
//! │  └──────────────────┘          └───────────────────┘                        │
//! │           │                              │                                   │
//! │           ▼                              ▼                                   │
//! │  ┌──────────────────┐          ┌───────────────────┐                        │
//! │  │  Docker / K8s    │          │     Internet      │                        │
//! │  │  API             │          │   (allowed hosts) │                        │
//! │  └──────────────────┘          └───────────────────┘                        │
//! └─────────────────────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Sandbox Policies
//!
//! | Policy | Filesystem | Network | Use Case |
//! |--------|------------|---------|----------|
//! | `ReadOnly` | Read workspace | Proxied | Explore code, fetch docs |
//! | `WorkspaceWrite` | Read/write workspace | Proxied | Build software, run tests |
//! | `FullAccess` | Full host | Full | Direct execution (no sandbox) |
//!
//! # Example
//!
//! ```rust,no_run
//! use ironclaw::sandbox::{SandboxManager, SandboxManagerBuilder, SandboxPolicy};
//! use std::collections::HashMap;
//! use std::path::Path;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let manager = SandboxManagerBuilder::new()
//!     .enabled(true)
//!     .policy(SandboxPolicy::WorkspaceWrite)
//!     .build();
//!
//! manager.initialize().await?;
//!
//! let result = manager.execute(
//!     "cargo build --release",
//!     Path::new("/workspace/my-project"),
//!     HashMap::new(),
//! ).await?;
//!
//! println!("Exit code: {}", result.exit_code);
//! println!("Output: {}", result.output);
//!
//! manager.shutdown().await;
//! # Ok(())
//! # }
//! ```
//!
//! # Security Properties
//!
//! - **No credentials in containers**: Environment variables with secrets never enter containers
//! - **Network isolation**: All traffic routes through the proxy (validated domains only)
//! - **Non-root execution**: Containers run as UID 1000
//! - **Read-only root**: Container filesystem is read-only (except workspace mount)
//! - **Capability dropping**: All Linux capabilities dropped, only essential ones added back
//! - **Auto-cleanup**: Containers are removed after execution (--rm + explicit cleanup)
//! - **Timeout enforcement**: Commands are killed after the timeout

pub mod config;
#[cfg(feature = "docker")]
pub mod container;
#[cfg(feature = "docker")]
pub mod detect;
#[cfg(feature = "docker")]
pub mod docker;
pub mod error;
#[cfg(feature = "kubernetes")]
pub mod kubernetes;
pub mod manager;
pub mod proxy;
pub mod runtime;

pub use config::{ResourceLimits, SandboxConfig, SandboxPolicy};
#[cfg(feature = "docker")]
pub use container::{ContainerOutput, ContainerRunner, connect_docker};
#[cfg(feature = "docker")]
pub use detect::{DockerDetection, DockerStatus, Platform, check_docker};
pub use error::{Result, SandboxError};
pub use manager::{ExecOutput, SandboxManager, SandboxManagerBuilder};
pub use proxy::{
    CredentialResolver, DefaultPolicyDecider, DomainAllowlist, EnvCredentialResolver, HttpProxy,
    NetworkDecision, NetworkPolicyDecider, NetworkProxyBuilder, NetworkRequest,
};
pub use runtime::{
    ContainerRuntime, ManagedWorkload, RuntimeBackend, RuntimeDetection, RuntimeStatus,
    VolumeMount, WorkloadOutput, WorkloadSpec, connect_runtime, resolve_runtime_backend,
};

/// Default allowlist getter (re-export for convenience).
pub fn default_allowlist() -> Vec<String> {
    config::default_allowlist()
}

/// Default credential mappings getter (re-export for convenience).
pub fn default_credential_mappings() -> Vec<crate::secrets::CredentialMapping> {
    config::default_credential_mappings()
}
