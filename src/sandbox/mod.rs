//! Docker execution sandbox for secure command execution.
//!
//! This module provides a complete sandboxing solution for running untrusted commands:
//! - **Container isolation**: Commands run in ephemeral Docker containers
//! - **Network proxy**: All network traffic goes through a validating proxy
//! - **Credential injection**: Secrets are injected by the proxy, never exposed in containers
//! - **Resource limits**: Memory, CPU, and timeout enforcement
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
//! │  │  • Coordinates container creation and execution                     │    │
//! │  │  • Manages proxy lifecycle                                          │    │
//! │  │  • Enforces resource limits                                         │    │
//! │  └─────────────────────────────────────────────────────────────────────┘    │
//! │           │                              │                                   │
//! │           ▼                              ▼                                   │
//! │  ┌──────────────────┐          ┌───────────────────┐                        │
//! │  │   Container      │          │   Network Proxy   │                        │
//! │  │   Runner         │          │                   │                        │
//! │  │                  │          │  • Allowlist      │                        │
//! │  │  • Create        │◀────────▶│  • Credentials    │                        │
//! │  │  • Execute       │          │  • Logging        │                        │
//! │  │  • Cleanup       │          │                   │                        │
//! │  └──────────────────┘          └───────────────────┘                        │
//! │           │                              │                                   │
//! │           ▼                              ▼                                   │
//! │  ┌──────────────────┐          ┌───────────────────┐                        │
//! │  │     Docker       │          │     Internet      │                        │
//! │  │                  │          │   (allowed hosts) │                        │
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
pub mod container;
pub mod error;
pub mod manager;
pub mod proxy;

pub use config::{
    CredentialLocation, CredentialMapping, ResourceLimits, SandboxConfig, SandboxPolicy,
};
pub use container::{ContainerOutput, ContainerRunner, connect_docker};
pub use error::{Result, SandboxError};
pub use manager::{ExecOutput, SandboxManager, SandboxManagerBuilder};
pub use proxy::{
    CredentialResolver, DefaultPolicyDecider, DomainAllowlist, EnvCredentialResolver, HttpProxy,
    NetworkDecision, NetworkPolicyDecider, NetworkProxyBuilder, NetworkRequest,
};

/// Default allowlist getter (re-export for convenience).
pub fn default_allowlist() -> Vec<String> {
    config::default_allowlist()
}

/// Default credential mappings getter (re-export for convenience).
pub fn default_credential_mappings() -> Vec<CredentialMapping> {
    config::default_credential_mappings()
}
