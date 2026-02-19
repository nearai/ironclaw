//! Error types for sidecar management.

use thiserror::Error;

/// Result type for sidecar operations.
pub type Result<T> = std::result::Result<T, SidecarError>;

/// Errors that can occur during sidecar management.
#[derive(Debug, Error)]
pub enum SidecarError {
    /// Docker is not available.
    #[error("Docker not available: {reason}")]
    DockerNotAvailable {
        /// Reason why Docker is unavailable.
        reason: String,
    },

    /// Failed to pull the image.
    #[error("Failed to pull image '{image}': {reason}")]
    ImagePullFailed {
        /// Image name.
        image: String,
        /// Reason for failure.
        reason: String,
    },

    /// Failed to create the container.
    #[error("Failed to create container '{name}': {reason}")]
    ContainerCreationFailed {
        /// Container name.
        name: String,
        /// Reason for failure.
        reason: String,
    },

    /// Failed to start the container.
    #[error("Failed to start container '{name}': {reason}")]
    ContainerStartFailed {
        /// Container name.
        name: String,
        /// Reason for failure.
        reason: String,
    },

    /// Container failed health check within timeout.
    #[error("Container '{name}' failed health check within {timeout:?}: {reason}")]
    HealthCheckFailed {
        /// Container name.
        name: String,
        /// Timeout duration.
        timeout: std::time::Duration,
        /// Reason for failure.
        reason: String,
    },

    /// Container stopped unexpectedly.
    #[error("Container '{name}' stopped unexpectedly")]
    ContainerStopped {
        /// Container name.
        name: String,
    },

    /// Failed to connect to Docker socket.
    #[error("Docker socket connection failed: {reason}")]
    SocketConnectionFailed {
        /// Reason for failure.
        reason: String,
    },

    /// Configuration error.
    #[error("Sidecar configuration error: {reason}")]
    Config {
        /// Reason for error.
        reason: String,
    },

    /// I/O error.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// HTTP client error during health check.
    #[error("HTTP error during health check: {0}")]
    Http(String),
}
