//! Sidecar lifecycle management using Docker.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use bollard::Docker;
use bollard::container::{
    Config, CreateContainerOptions, RemoveContainerOptions, StartContainerOptions,
};
use bollard::image::CreateImageOptions;
use bollard::models::{HostConfig, PortBinding};
use futures::StreamExt;
use tokio::sync::RwLock;

use crate::sandbox::container::connect_docker;
use crate::sidecar::config::{HealthCheck, SidecarConfig, SidecarEndpoint};
use crate::sidecar::error::{Result, SidecarError};

/// State of a sidecar container.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidecarState {
    /// Not started yet.
    NotStarted,
    /// Currently starting up.
    Starting,
    /// Running and healthy.
    Ready,
    /// Running but not yet healthy.
    Unhealthy,
    /// Stopped or removed.
    Stopped,
    /// Failed to start.
    Failed,
}

/// Manages a single Docker sidecar container.
///
/// Provides lazy initialization (start on first request), health checks,
/// and clean shutdown. Thread-safe for concurrent access.
pub struct SidecarManager {
    config: SidecarConfig,
    docker: RwLock<Option<Docker>>,
    container_id: RwLock<Option<String>>,
    state: RwLock<SidecarState>,
    initialized: AtomicBool,
    /// Reusable HTTP client for health checks (avoids per-request allocation).
    http_client: reqwest::Client,
}

impl SidecarManager {
    /// Create a new sidecar manager.
    pub fn new(config: SidecarConfig) -> Self {
        let http_client = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self {
            config,
            docker: RwLock::new(None),
            container_id: RwLock::new(None),
            state: RwLock::new(SidecarState::NotStarted),
            initialized: AtomicBool::new(false),
            http_client,
        }
    }

    /// Get the current state.
    pub async fn state(&self) -> SidecarState {
        *self.state.read().await
    }

    /// Check if the sidecar is ready.
    pub async fn is_ready(&self) -> bool {
        *self.state.read().await == SidecarState::Ready
    }

    /// Get the endpoint for this sidecar.
    pub fn endpoint(&self) -> Option<SidecarEndpoint> {
        self.config.primary_endpoint()
    }

    /// Ensure the sidecar is running and healthy.
    ///
    /// This is the main entry point for lazy initialization.
    /// If the sidecar is already ready, returns immediately.
    /// Otherwise, starts the container and waits for health check.
    ///
    /// Uses a single write-lock scope to atomically check-and-set state,
    /// preventing TOCTOU races where two concurrent callers both enter `start()`.
    pub async fn ensure_ready(&self) -> Result<SidecarEndpoint> {
        // Connect to Docker if needed
        if !self.initialized.load(Ordering::SeqCst) {
            self.initialize().await?;
        }

        // Atomically check state and decide action under a single write lock
        let action = {
            let mut state = self.state.write().await;
            match *state {
                SidecarState::Ready => None, // already ready
                SidecarState::Starting => Some("wait"),
                SidecarState::NotStarted | SidecarState::Stopped | SidecarState::Unhealthy => {
                    *state = SidecarState::Starting;
                    Some("start")
                }
                SidecarState::Failed => {
                    return Err(SidecarError::ContainerStartFailed {
                        name: self.config.container_name(),
                        reason: "previous start attempt failed".to_string(),
                    });
                }
            }
        };

        match action {
            None => self.endpoint().ok_or(SidecarError::Config {
                reason: "no ports configured".to_string(),
            }),
            Some("wait") => self.wait_for_ready().await,
            Some("start") => self.do_start().await,
            _ => unreachable!(),
        }
    }

    /// Initialize connection to Docker.
    async fn initialize(&self) -> Result<()> {
        if self.initialized.load(Ordering::SeqCst) {
            return Ok(());
        }

        let docker = connect_docker()
            .await
            .map_err(|e| SidecarError::DockerNotAvailable {
                reason: e.to_string(),
            })?;

        *self.docker.write().await = Some(docker);
        self.initialized.store(true, Ordering::SeqCst);

        tracing::debug!("Sidecar '{}' initialized", self.config.name);
        Ok(())
    }

    /// Start the container and wait for it to become healthy.
    /// Caller must have already set state to `Starting` atomically.
    async fn do_start(&self) -> Result<SidecarEndpoint> {
        let docker = self
            .docker
            .read()
            .await
            .clone()
            .ok_or(SidecarError::DockerNotAvailable {
                reason: "not initialized".to_string(),
            })?;

        let container_name = self.config.container_name();

        // Remove existing container if present (from previous run)
        let _ = docker
            .remove_container(
                &container_name,
                Some(RemoveContainerOptions {
                    force: true,
                    ..Default::default()
                }),
            )
            .await;

        // Pull image if needed
        if self.config.auto_pull {
            self.pull_image(&docker).await?;
        }

        // Create container
        let container_id = self.create_container(&docker).await?;

        // Start container
        docker
            .start_container(&container_id, None::<StartContainerOptions<String>>)
            .await
            .map_err(|e| SidecarError::ContainerStartFailed {
                name: container_name.clone(),
                reason: e.to_string(),
            })?;

        *self.container_id.write().await = Some(container_id.clone());

        tracing::info!("Started sidecar container: {}", container_name);

        // Wait for health check
        let endpoint = self.wait_for_ready().await?;

        let mut state = self.state.write().await;
        *state = SidecarState::Ready;

        tracing::info!("Sidecar '{}' ready at {}", self.config.name, endpoint);

        Ok(endpoint)
    }

    /// Pull the Docker image.
    async fn pull_image(&self, docker: &Docker) -> Result<()> {
        // Check if image exists locally
        if docker.inspect_image(&self.config.image).await.is_ok() {
            tracing::debug!("Image '{}' exists locally", self.config.image);
            return Ok(());
        }

        tracing::info!("Pulling image: {}", self.config.image);

        let options = CreateImageOptions {
            from_image: self.config.image.clone(),
            ..Default::default()
        };

        let mut stream = docker.create_image(Some(options), None, None);

        while let Some(result) = stream.next().await {
            match result {
                Ok(info) => {
                    if let Some(status) = info.status {
                        tracing::trace!("Pull status: {}", status);
                    }
                }
                Err(e) => {
                    return Err(SidecarError::ImagePullFailed {
                        image: self.config.image.clone(),
                        reason: e.to_string(),
                    });
                }
            }
        }

        tracing::info!("Pulled image: {}", self.config.image);
        Ok(())
    }

    /// Create the container with configured options.
    async fn create_container(&self, docker: &Docker) -> Result<String> {
        let container_name = self.config.container_name();

        // Build port bindings
        let mut port_bindings = std::collections::HashMap::new();
        for (host_port, container_port) in &self.config.ports {
            port_bindings.insert(
                format!("{}/tcp", container_port),
                Some(vec![PortBinding {
                    host_ip: Some("127.0.0.1".to_string()),
                    host_port: Some(host_port.clone()),
                }]),
            );
        }

        // Build environment
        let env: Vec<String> = self
            .config
            .env
            .iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect();

        // Build volume mounts
        let binds: Vec<String> = self
            .config
            .volumes
            .iter()
            .map(|(host, container, opts)| format!("{}:{}:{}", host, container, opts))
            .collect();

        // Exposed ports (bollard expects HashMap<String, HashMap<(), ()>>)
        let exposed_ports: std::collections::HashMap<String, std::collections::HashMap<(), ()>> =
            self.config
                .ports
                .iter()
                .map(|(_, container_port)| {
                    (
                        format!("{}/tcp", container_port),
                        std::collections::HashMap::new(),
                    )
                })
                .collect();

        let host_config = HostConfig {
            port_bindings: Some(port_bindings),
            binds: if binds.is_empty() { None } else { Some(binds) },
            extra_hosts: if self.config.extra_hosts.is_empty() {
                None
            } else {
                Some(self.config.extra_hosts.clone())
            },
            network_mode: Some(self.config.network_mode.clone()),
            // Auto-remove on stop (but we handle removal explicitly for control)
            auto_remove: Some(false),
            ..Default::default()
        };

        let config = Config {
            image: Some(self.config.image.clone()),
            env: if env.is_empty() { None } else { Some(env) },
            exposed_ports: Some(exposed_ports),
            host_config: Some(host_config),
            // Container stays running
            cmd: None,
            entrypoint: None,
            ..Default::default()
        };

        let options = CreateContainerOptions {
            name: container_name.clone(),
            ..Default::default()
        };

        let response = docker
            .create_container(Some(options), config)
            .await
            .map_err(|e| SidecarError::ContainerCreationFailed {
                name: container_name.clone(),
                reason: e.to_string(),
            })?;

        Ok(response.id)
    }

    /// Wait for the container to pass health check.
    async fn wait_for_ready(&self) -> Result<SidecarEndpoint> {
        let endpoint = self.config.primary_endpoint().ok_or(SidecarError::Config {
            reason: "no ports configured".to_string(),
        })?;

        // If no health check, assume ready immediately
        if matches!(self.config.health_check, HealthCheck::None) {
            return Ok(endpoint);
        }

        let start = std::time::Instant::now();
        let timeout = self.config.startup_timeout;
        let interval = self.config.health_poll_interval;

        while start.elapsed() < timeout {
            // Check if container is still running
            if let Some(docker) = self.docker.read().await.as_ref()
                && let Some(container_id) = self.container_id.read().await.as_ref()
                && let Ok(info) = docker.inspect_container(container_id, None).await
                && info.state.is_some_and(|s| s.running != Some(true))
            {
                let mut state = self.state.write().await;
                *state = SidecarState::Failed;
                return Err(SidecarError::ContainerStopped {
                    name: self.config.name.clone(),
                });
            }

            // Perform health check
            match self.check_health(&endpoint).await {
                Ok(true) => return Ok(endpoint),
                Ok(false) => {
                    // Not healthy yet, wait and retry
                    tokio::time::sleep(interval).await;
                }
                Err(e) => {
                    tracing::trace!("Health check error: {}", e);
                    tokio::time::sleep(interval).await;
                }
            }
        }

        let mut state = self.state.write().await;
        *state = SidecarState::Failed;

        Err(SidecarError::HealthCheckFailed {
            name: self.config.name.clone(),
            timeout,
            reason: "timeout waiting for health check".to_string(),
        })
    }

    /// Perform the health check.
    async fn check_health(&self, endpoint: &SidecarEndpoint) -> Result<bool> {
        match &self.config.health_check {
            HealthCheck::None => Ok(true),
            HealthCheck::Http { path, port: _ } => {
                let url = format!(
                    "http://127.0.0.1:{}/{}",
                    endpoint.port,
                    path.trim_start_matches('/')
                );
                self.http_health_check(&url).await
            }
            HealthCheck::Tcp { port: _ } => {
                let addr = format!("127.0.0.1:{}", endpoint.port);
                self.tcp_health_check(&addr).await
            }
            HealthCheck::Command { cmd: _ } => {
                // For command health checks, we'd need exec support
                // For now, just return true (can be extended later)
                tracing::warn!("Command health check not yet implemented, assuming healthy");
                Ok(true)
            }
        }
    }

    /// HTTP health check.
    async fn http_health_check(&self, url: &str) -> Result<bool> {
        let response = self.http_client.get(url).send().await;

        match response {
            Ok(resp) => Ok(resp.status().is_success()),
            Err(e) => {
                // Connection refused is expected during startup
                if e.is_connect() {
                    Ok(false)
                } else {
                    Err(SidecarError::Http(e.to_string()))
                }
            }
        }
    }

    /// TCP health check.
    async fn tcp_health_check(&self, addr: &str) -> Result<bool> {
        use tokio::net::TcpStream;

        match tokio::time::timeout(Duration::from_secs(2), TcpStream::connect(addr)).await {
            Ok(Ok(_)) => Ok(true),
            Ok(Err(_)) => Ok(false),
            Err(_) => Ok(false), // Timeout
        }
    }

    /// Shutdown the sidecar (stop and remove container).
    pub async fn shutdown(&self) {
        let docker = self.docker.read().await.clone();
        let container_id = self.container_id.read().await.clone();

        if let (Some(docker), Some(id)) = (docker, container_id) {
            if !self.config.keep_on_shutdown {
                tracing::info!("Stopping sidecar container: {}", self.config.name);

                let _ = docker
                    .remove_container(
                        &id,
                        Some(RemoveContainerOptions {
                            force: true,
                            ..Default::default()
                        }),
                    )
                    .await;

                tracing::info!("Stopped sidecar container: {}", self.config.name);
            } else {
                tracing::info!(
                    "Keeping sidecar container running (keep_on_shutdown=true): {}",
                    self.config.name
                );
            }
        }

        let mut state = self.state.write().await;
        *state = SidecarState::Stopped;
        self.initialized.store(false, Ordering::SeqCst);
    }

    /// Restart the sidecar (stop, remove, start fresh).
    pub async fn restart(&self) -> Result<SidecarEndpoint> {
        self.shutdown().await;

        let mut state = self.state.write().await;
        *state = SidecarState::NotStarted;
        drop(state);

        self.ensure_ready().await
    }

    /// Get the container ID if running.
    pub async fn container_id(&self) -> Option<String> {
        self.container_id.read().await.clone()
    }

    /// Get the configuration.
    pub fn config(&self) -> &SidecarConfig {
        &self.config
    }
}

impl Drop for SidecarManager {
    fn drop(&mut self) {
        if self.initialized.load(Ordering::SeqCst) {
            tracing::warn!(
                "SidecarManager '{}' dropped without shutdown(), container may remain running",
                self.config.name
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_state_transitions() {
        let config = SidecarConfig {
            name: "test".to_string(),
            ports: vec![("8080".to_string(), "80".to_string())],
            ..Default::default()
        };

        let manager = SidecarManager::new(config);

        // Initial state
        assert_eq!(
            tokio_test::block_on(manager.state()),
            SidecarState::NotStarted
        );
    }

    #[test]
    fn test_endpoint_extraction() {
        let config = SidecarConfig::browserless(9222, None);
        let manager = SidecarManager::new(config);

        let endpoint = manager.endpoint().unwrap();
        assert_eq!(endpoint.port, 9222);
        assert_eq!(endpoint.http_url(), "http://127.0.0.1:9222");
    }
}
