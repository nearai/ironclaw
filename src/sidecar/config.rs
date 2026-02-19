//! Configuration types for sidecar management.

use std::time::Duration;

/// Configuration for a Docker sidecar container.
#[derive(Debug, Clone)]
pub struct SidecarConfig {
    /// Unique name for this sidecar (used in container naming).
    pub name: String,
    /// Docker image to run.
    pub image: String,
    /// Port mappings: (host_port, container_port).
    pub ports: Vec<(String, String)>,
    /// Environment variables: (name, value).
    pub env: Vec<(String, String)>,
    /// Volume mounts: (host_path, container_path, options).
    /// Options: "ro", "rw", etc.
    pub volumes: Vec<(String, String, String)>,
    /// Health check configuration.
    pub health_check: HealthCheck,
    /// Time to wait for health check to pass.
    pub startup_timeout: Duration,
    /// Interval between health check polls.
    pub health_poll_interval: Duration,
    /// Whether to auto-pull the image if not present.
    pub auto_pull: bool,
    /// Whether to keep the container running on shutdown (for debugging).
    pub keep_on_shutdown: bool,
    /// Extra hosts to add to /etc/hosts (for host resolution).
    pub extra_hosts: Vec<String>,
    /// Network mode (default: bridge).
    pub network_mode: String,
}

impl Default for SidecarConfig {
    fn default() -> Self {
        Self {
            name: String::new(),
            image: String::new(),
            ports: Vec::new(),
            env: Vec::new(),
            volumes: Vec::new(),
            health_check: HealthCheck::None,
            startup_timeout: Duration::from_secs(60),
            health_poll_interval: Duration::from_millis(500),
            auto_pull: true,
            keep_on_shutdown: false,
            extra_hosts: Vec::new(),
            network_mode: "bridge".to_string(),
        }
    }
}

impl SidecarConfig {
    /// Create a config for Browserless Chromium.
    pub fn browserless(port: u16, token: Option<String>) -> Self {
        let mut env = vec![
            ("MAX_CONCURRENT_SESSIONS".to_string(), "4".to_string()),
            ("MAX_QUEUE_LENGTH".to_string(), "10".to_string()),
        ];
        if let Some(t) = token {
            env.push(("TOKEN".to_string(), t));
        }

        Self {
            name: "ironclaw-browserless".to_string(),
            image: "ghcr.io/browserless/chromium:latest".to_string(),
            ports: vec![(port.to_string(), "3000".to_string())],
            env,
            volumes: Vec::new(),
            health_check: HealthCheck::Http {
                path: "/".to_string(),
                port: 3000,
            },
            startup_timeout: Duration::from_secs(90),
            ..Default::default()
        }
    }

    /// Get the primary endpoint URL for this sidecar.
    /// Returns the first mapped port as an HTTP URL.
    /// Returns `None` if no ports are configured or port strings are not valid numbers.
    pub fn primary_endpoint(&self) -> Option<SidecarEndpoint> {
        let (host_port, container_port) = self.ports.first()?;
        let port = host_port.parse().ok()?;
        let container_port = container_port.parse().ok()?;
        Some(SidecarEndpoint {
            host: "127.0.0.1".to_string(),
            port,
            container_port,
        })
    }

    /// Generate container name with prefix.
    pub fn container_name(&self) -> String {
        format!("ironclaw-sidecar-{}", self.name)
    }
}

/// Health check configuration for a sidecar.
#[derive(Debug, Clone, Default)]
pub enum HealthCheck {
    /// No health check (assume ready after start).
    #[default]
    None,
    /// HTTP health check on a path.
    Http {
        /// Path to check (e.g., "/" or "/health").
        path: String,
        /// Port inside the container.
        port: u16,
    },
    /// TCP health check (connection test).
    Tcp {
        /// Port inside the container.
        port: u16,
    },
    /// Custom command to run inside the container.
    Command {
        /// Command to execute (exit 0 = healthy).
        cmd: Vec<String>,
    },
}

/// Represents a sidecar endpoint.
#[derive(Debug, Clone)]
pub struct SidecarEndpoint {
    /// Host address (usually 127.0.0.1).
    pub host: String,
    /// Port on the host.
    pub port: u16,
    /// Port inside the container.
    pub container_port: u16,
}

impl SidecarEndpoint {
    /// Get the HTTP URL for this endpoint.
    pub fn http_url(&self) -> String {
        format!("http://{}:{}", self.host, self.port)
    }

    /// Get the WebSocket URL for this endpoint.
    pub fn ws_url(&self) -> String {
        format!("ws://{}:{}", self.host, self.port)
    }
}

impl std::fmt::Display for SidecarEndpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.host, self.port)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_browserless_config() {
        let config = SidecarConfig::browserless(9222, Some("mytoken".to_string()));

        assert_eq!(config.name, "ironclaw-browserless");
        assert_eq!(config.image, "ghcr.io/browserless/chromium:latest");
        assert_eq!(config.ports.len(), 1);
        assert!(config.env.iter().any(|(k, _)| k == "TOKEN"));
    }

    #[test]
    fn test_primary_endpoint() {
        let config = SidecarConfig {
            name: "test".to_string(),
            ports: vec![("8080".to_string(), "80".to_string())],
            ..Default::default()
        };

        let endpoint = config.primary_endpoint().unwrap();
        assert_eq!(endpoint.port, 8080);
        assert_eq!(endpoint.http_url(), "http://127.0.0.1:8080");
    }

    #[test]
    fn test_container_name() {
        let config = SidecarConfig {
            name: "my-service".to_string(),
            ..Default::default()
        };

        assert_eq!(config.container_name(), "ironclaw-sidecar-my-service");
    }
}
