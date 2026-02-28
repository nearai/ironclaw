//! Error types for the Docker execution sandbox.

use std::time::Duration;

/// Errors that can occur in the sandbox system.
#[derive(Debug, thiserror::Error)]
pub enum SandboxError {
    /// Container runtime (Docker/Podman) is not available or not running.
    #[error("Container runtime not available: {reason}")]
    DockerNotAvailable { reason: String },

    /// Failed to create container.
    #[error("Container creation failed: {reason}")]
    ContainerCreationFailed { reason: String },

    /// Failed to start container.
    #[error("Container start failed: {reason}")]
    ContainerStartFailed { reason: String },

    /// Command execution failed inside container.
    #[error("Execution failed: {reason}")]
    ExecutionFailed { reason: String },

    /// Command timed out.
    #[error("Command timed out after {0:?}")]
    Timeout(Duration),

    /// Container resource limit exceeded.
    #[error("Resource limit exceeded: {resource} limit of {limit}")]
    ResourceLimitExceeded { resource: String, limit: String },

    /// Network proxy error.
    #[error("Proxy error: {reason}")]
    ProxyError { reason: String },

    /// Network request blocked by policy.
    #[error("Network request blocked: {reason}")]
    NetworkBlocked { reason: String },

    /// Credential injection failed.
    #[error("Credential injection failed for {domain}: {reason}")]
    CredentialInjectionFailed { domain: String, reason: String },

    /// Docker API error.
    #[error("Docker API error: {0}")]
    Docker(#[from] bollard::errors::Error),

    /// I/O error.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Configuration error.
    #[error("Configuration error: {reason}")]
    Config { reason: String },
}

/// Result type for sandbox operations.
pub type Result<T> = std::result::Result<T, SandboxError>;
