//! Container runtime abstraction.
//!
//! Defines the `ContainerRuntime` trait that both Docker and Kubernetes
//! backends implement. Consumers (SandboxManager, ContainerJobManager,
//! SandboxReaper) depend on `Arc<dyn ContainerRuntime>` instead of a
//! concrete backend.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::sandbox::capabilities::RuntimeCapabilities;
use crate::sandbox::error::SandboxError;

// ---------------------------------------------------------------------------
// Shared types
// ---------------------------------------------------------------------------

/// Specification for creating a workload (container or pod).
#[derive(Debug, Clone)]
pub struct WorkloadSpec {
    /// Human-readable name (used as container name / pod name).
    pub name: String,
    /// Container image.
    pub image: String,
    /// Command to run (CMD).
    pub command: Vec<String>,
    /// Environment variables as `KEY=VALUE` strings.
    pub env: Vec<String>,
    /// Working directory inside the workload.
    pub working_dir: String,
    /// User to run as (e.g. `"1000:1000"`).
    pub user: Option<String>,
    /// Labels for identification and reaper discovery.
    pub labels: HashMap<String, String>,
    /// Volume mounts.
    pub mounts: Vec<VolumeMount>,
    /// Small inline files to materialize inside the workload.
    pub inline_files: Vec<InlineFileMount>,
    /// Tmpfs mounts (path → options, e.g. `"/tmp" → "size=512M"`).
    pub tmpfs_mounts: HashMap<String, String>,
    /// Memory limit in bytes.
    pub memory_bytes: Option<i64>,
    /// CPU shares (relative weight).
    pub cpu_shares: Option<i64>,
    /// Network mode (`"bridge"`, `"host"`, etc.). Ignored on Kubernetes.
    pub network_mode: Option<String>,
    /// Extra `/etc/hosts` entries. Ignored on Kubernetes.
    pub extra_hosts: Vec<String>,
    /// Linux capabilities to drop.
    pub cap_drop: Vec<String>,
    /// Linux capabilities to add back.
    pub cap_add: Vec<String>,
    /// Security options (e.g. `"no-new-privileges:true"`).
    pub security_opts: Vec<String>,
    /// Whether the root filesystem is read-only.
    pub readonly_rootfs: bool,
    /// Whether the workload should be automatically removed on exit.
    pub auto_remove: bool,
}

impl Default for WorkloadSpec {
    fn default() -> Self {
        Self {
            name: String::new(),
            image: String::new(),
            command: Vec::new(),
            env: Vec::new(),
            working_dir: "/workspace".to_string(),
            user: Some("1000:1000".to_string()),
            labels: HashMap::new(),
            mounts: Vec::new(),
            inline_files: Vec::new(),
            tmpfs_mounts: HashMap::new(),
            memory_bytes: None,
            cpu_shares: None,
            network_mode: Some("bridge".to_string()),
            extra_hosts: Vec::new(),
            cap_drop: vec!["ALL".to_string()],
            cap_add: vec!["CHOWN".to_string()],
            security_opts: vec!["no-new-privileges:true".to_string()],
            readonly_rootfs: true,
            auto_remove: false,
        }
    }
}

/// A volume mount specification.
#[derive(Debug, Clone)]
pub struct VolumeMount {
    /// Host path or volume name.
    pub source: String,
    /// Path inside the workload.
    pub target: String,
    /// Whether the mount is read-only.
    pub read_only: bool,
}

impl VolumeMount {
    /// Format as a Docker bind string (`source:target:ro` or `source:target:rw`).
    pub fn as_bind_string(&self) -> String {
        let mode = if self.read_only { "ro" } else { "rw" };
        format!("{}:{}:{}", self.source, self.target, mode)
    }
}

/// A small inline file that should appear inside the workload filesystem.
#[derive(Debug, Clone)]
pub struct InlineFileMount {
    /// Absolute path inside the workload.
    pub target: String,
    /// UTF-8 file contents.
    pub contents: String,
    /// File mode written inside the workload when supported by the runtime.
    pub mode: i32,
}

/// Output from a workload execution or exec.
#[derive(Debug, Clone)]
pub struct WorkloadOutput {
    /// Exit code.
    pub exit_code: i64,
    /// Standard output.
    pub stdout: String,
    /// Standard error.
    pub stderr: String,
    /// How long the execution took.
    pub duration: Duration,
    /// Whether output was truncated.
    pub truncated: bool,
}

/// A discovered managed workload (for reaper listing).
#[derive(Debug, Clone)]
pub struct ManagedWorkload {
    /// Runtime-specific workload identifier (container ID or pod name).
    pub workload_id: String,
    /// The job ID from the workload label.
    pub job_id: Uuid,
    /// When the workload was created.
    pub created_at: DateTime<Utc>,
}

/// Format the workload creation time for the `ironclaw.created_at` label.
///
/// We use unix milliseconds because Kubernetes label values must avoid `:` and
/// `+`, which appear in RFC3339 timestamps.
pub fn format_workload_created_at_label(created_at: DateTime<Utc>) -> String {
    created_at.timestamp_millis().to_string()
}

/// Parse the workload creation time from the `ironclaw.created_at` label.
///
/// New writes use unix milliseconds. Historical workloads may still carry the
/// pre-fix RFC3339 encoding, so we accept both.
pub fn parse_workload_created_at_label(label: &str) -> Option<DateTime<Utc>> {
    let millis = if label.len() >= 12 && label.chars().all(|c| c.is_ascii_digit()) {
        label
            .parse::<i64>()
            .ok()
            .and_then(DateTime::from_timestamp_millis)
    } else {
        None
    };

    millis.or_else(|| {
        DateTime::parse_from_rfc3339(label)
            .ok()
            .map(|dt| dt.with_timezone(&Utc))
    })
}

/// Runtime readiness status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeStatus {
    /// Runtime is available and responsive.
    Available,
    /// Runtime binary/API not found.
    NotInstalled,
    /// Runtime found but not responding.
    NotRunning,
    /// Runtime is disabled by configuration.
    Disabled,
}

impl RuntimeStatus {
    pub fn is_ok(&self) -> bool {
        matches!(self, RuntimeStatus::Available)
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            RuntimeStatus::Available => "available",
            RuntimeStatus::NotInstalled => "not installed",
            RuntimeStatus::NotRunning => "not running",
            RuntimeStatus::Disabled => "disabled",
        }
    }
}

/// Result of a runtime readiness check.
pub struct RuntimeDetection {
    pub status: RuntimeStatus,
    pub runtime_name: &'static str,
    pub install_hint: String,
    pub start_hint: String,
}

/// Which container runtime backend is selected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeBackend {
    Docker,
    Kubernetes,
}

impl std::fmt::Display for RuntimeBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RuntimeBackend::Docker => write!(f, "docker"),
            RuntimeBackend::Kubernetes => write!(f, "kubernetes"),
        }
    }
}

impl std::str::FromStr for RuntimeBackend {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "docker" => Ok(RuntimeBackend::Docker),
            "kubernetes" | "k8s" => Ok(RuntimeBackend::Kubernetes),
            _ => Err(format!(
                "invalid container runtime '{}', expected 'docker' or 'kubernetes'",
                s
            )),
        }
    }
}

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

/// Abstraction over container runtimes (Docker, Kubernetes, future Podman).
///
/// Methods are capability-oriented: they describe *what* the caller wants
/// (create a workload, exec in it, list managed workloads) without leaking
/// Docker-specific semantics like `start_container` (Kubernetes pods start
/// on creation) or bind-mount strings.
#[async_trait::async_trait]
pub trait ContainerRuntime: Send + Sync {
    /// Human-readable name of this runtime backend.
    fn name(&self) -> &'static str;

    /// Canonical capability profile for this runtime backend.
    fn capabilities(&self) -> RuntimeCapabilities;

    // ── Readiness ──────────────────────────────────────────────────

    /// Check whether the runtime is available and responsive.
    async fn is_available(&self) -> bool;

    /// Full readiness detection with install/start hints.
    async fn detect(&self) -> RuntimeDetection;

    // ── Image management ───────────────────────────────────────────

    /// Check if a container image exists locally (or is pullable on K8s).
    async fn image_exists(&self, image: &str) -> bool;

    /// Pull a container image.
    async fn pull_image(&self, image: &str) -> Result<(), SandboxError>;

    /// Build a container image from a Dockerfile.
    /// Returns `Err` with `SandboxError::Config` on runtimes that do not
    /// support local builds (e.g. Kubernetes).
    async fn build_image(&self, image: &str, dockerfile_path: &Path) -> Result<(), SandboxError>;

    // ── Workload lifecycle ─────────────────────────────────────────

    /// Create and start a workload. Returns the runtime-specific workload ID
    /// (Docker container ID or Kubernetes pod name).
    async fn create_and_start_workload(&self, spec: &WorkloadSpec) -> Result<String, SandboxError>;

    /// Wait for a workload to finish and return its exit code.
    async fn wait_workload(&self, workload_id: &str) -> Result<i64, SandboxError>;

    /// Stop a running workload with a grace period.
    async fn stop_workload(
        &self,
        workload_id: &str,
        grace_period_secs: u32,
    ) -> Result<(), SandboxError>;

    /// Remove a workload (force-delete).
    async fn remove_workload(&self, workload_id: &str) -> Result<(), SandboxError>;

    // ── Execution ──────────────────────────────────────────────────

    /// Execute a command inside a running workload.
    async fn exec_in_workload(
        &self,
        workload_id: &str,
        command: &[&str],
        working_dir: &str,
        max_output: usize,
        timeout: Duration,
    ) -> Result<WorkloadOutput, SandboxError>;

    /// Wait until a workload is ready to accept exec or file upload requests.
    ///
    /// Docker workloads are ready immediately after `start`, so the default
    /// implementation is a no-op. Kubernetes uses this hook to wait for the
    /// pod to reach a running state before bootstrap operations begin.
    async fn wait_workload_ready(
        &self,
        _workload_id: &str,
        _timeout: Duration,
    ) -> Result<(), SandboxError> {
        Ok(())
    }

    /// Upload a compressed workspace archive into a running workload.
    ///
    /// Runtimes that only support host mounts can keep the default
    /// unsupported implementation.
    async fn upload_workspace_archive(
        &self,
        _workload_id: &str,
        _archive_gz: &[u8],
        _target_dir: &str,
    ) -> Result<(), SandboxError> {
        Err(SandboxError::Config {
            reason: "runtime does not support uploading workspace archives".to_string(),
        })
    }

    // ── Logs ───────────────────────────────────────────────────────

    /// Collect stdout and stderr from a workload.
    /// Returns `(stdout, stderr, truncated)`.
    async fn collect_logs(
        &self,
        workload_id: &str,
        max_output: usize,
    ) -> Result<(String, String, bool), SandboxError>;

    // ── Discovery ──────────────────────────────────────────────────

    /// List workloads managed by IronClaw (identified by a label key).
    async fn list_managed_workloads(
        &self,
        label_key: &str,
    ) -> Result<Vec<ManagedWorkload>, SandboxError>;

    // ── Networking ─────────────────────────────────────────────────

    /// The host/address workers should use to reach the orchestrator.
    ///
    /// - Docker: `"host.docker.internal"` (resolved via extra_hosts)
    /// - Kubernetes: cluster-DNS service name
    fn orchestrator_host(&self) -> &str;

    /// Whether workloads can reach the host-local HTTP proxy.
    ///
    /// Docker workloads can (via `host.docker.internal`). Kubernetes pods
    /// cannot reach a host-bound proxy port through the cluster Service DNS,
    /// so injecting `http_proxy`/`https_proxy` env vars would produce
    /// unreachable endpoints. Use K8s NetworkPolicies instead.
    fn supports_host_proxy(&self) -> bool {
        self.capabilities().supports_host_proxy()
    }

    /// Whether the runtime can bind-mount host paths into workloads.
    ///
    /// Docker supports this natively. Kubernetes uses ephemeral PVCs instead,
    /// so `VolumeMount.source` (the host path) is ignored and volumes start
    /// empty. Callers should warn when mounts carry meaningful host data that
    /// the runtime will silently discard.
    fn supports_bind_mounts(&self) -> bool {
        self.capabilities().supports_bind_mounts()
    }
}

// ---------------------------------------------------------------------------
// Factory
// ---------------------------------------------------------------------------

/// Resolve which runtime backend to use.
///
/// Precedence: `CONTAINER_RUNTIME` env var > `config_override` (from DB
/// settings) > compiled feature flags default.
///
/// When no env var, no override, and only one runtime feature is compiled
/// in, that runtime is chosen. When both are compiled and nothing is set,
/// Docker wins as the conservative default.
pub fn resolve_runtime_backend(config_override: Option<&str>) -> Result<RuntimeBackend, String> {
    let default_backend = default_backend_for_compiled_features()?;
    let requested = std::env::var("CONTAINER_RUNTIME")
        .ok()
        .or_else(|| config_override.map(|s| s.to_string()))
        .unwrap_or(default_backend);

    let backend: RuntimeBackend = requested.parse()?;

    match backend {
        RuntimeBackend::Docker => {
            #[cfg(not(feature = "docker"))]
            return Err(
                "CONTAINER_RUNTIME=docker but the 'docker' feature is not compiled in".to_string(),
            );
            #[cfg(feature = "docker")]
            Ok(RuntimeBackend::Docker)
        }
        RuntimeBackend::Kubernetes => {
            #[cfg(not(feature = "kubernetes"))]
            return Err(
                "CONTAINER_RUNTIME=kubernetes but the 'kubernetes' feature is not compiled in"
                    .to_string(),
            );
            #[cfg(feature = "kubernetes")]
            Ok(RuntimeBackend::Kubernetes)
        }
    }
}

/// Pick the default backend string based on compiled feature flags.
fn default_backend_for_compiled_features() -> Result<String, String> {
    #[cfg(all(feature = "docker", feature = "kubernetes"))]
    {
        Ok("docker".to_string())
    }
    #[cfg(all(feature = "docker", not(feature = "kubernetes")))]
    {
        Ok("docker".to_string())
    }
    #[cfg(all(feature = "kubernetes", not(feature = "docker")))]
    {
        Ok("kubernetes".to_string())
    }
    #[cfg(not(any(feature = "docker", feature = "kubernetes")))]
    {
        Err(
            "no container runtime feature compiled in (enable 'docker' or 'kubernetes')"
                .to_string(),
        )
    }
}

/// Connect to the resolved runtime backend, returning a trait object.
///
/// This is the canonical factory for obtaining an `Arc<dyn ContainerRuntime>`.
/// Both `SandboxManager` and `ContainerJobManager` should call this instead
/// of hard-coding a specific backend.
///
/// `config_override` is the DB-backed `container_runtime` setting (if any).
/// `namespace` is the Kubernetes namespace (only used when connecting to a
/// Kubernetes backend; ignored for Docker).
pub async fn connect_runtime(
    config_override: Option<&str>,
    namespace: &str,
) -> Result<Arc<dyn ContainerRuntime>, SandboxError> {
    let backend = resolve_runtime_backend(config_override)
        .map_err(|reason| SandboxError::Config { reason })?;
    connect_runtime_backend(backend, namespace).await
}

/// Connect to a specific runtime backend.
///
/// `namespace` is used only for `RuntimeBackend::Kubernetes`.
pub async fn connect_runtime_backend(
    backend: RuntimeBackend,
    namespace: &str,
) -> Result<Arc<dyn ContainerRuntime>, SandboxError> {
    match backend {
        RuntimeBackend::Docker => {
            #[cfg(feature = "docker")]
            {
                let _ = namespace;
                let rt = crate::sandbox::docker::DockerRuntime::connect().await?;
                Ok(Arc::new(rt) as Arc<dyn ContainerRuntime>)
            }
            #[cfg(not(feature = "docker"))]
            {
                let _ = namespace;
                Err(SandboxError::Config {
                    reason: "docker feature not compiled in".to_string(),
                })
            }
        }
        RuntimeBackend::Kubernetes => {
            #[cfg(feature = "kubernetes")]
            {
                let rt = crate::sandbox::kubernetes::KubernetesRuntime::connect(namespace).await?;
                Ok(Arc::new(rt) as Arc<dyn ContainerRuntime>)
            }
            #[cfg(not(feature = "kubernetes"))]
            {
                let _ = namespace;
                Err(SandboxError::Config {
                    reason: "kubernetes feature not compiled in".to_string(),
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox::{
        ConfigDelivery, NetworkIsolation, RuntimeCapabilities, RuntimeStage, WorkspaceDelivery,
    };

    #[test]
    fn runtime_backend_parse() {
        assert_eq!(
            "docker".parse::<RuntimeBackend>().unwrap(), // safety: test
            RuntimeBackend::Docker
        );
        assert_eq!(
            "kubernetes".parse::<RuntimeBackend>().unwrap(), // safety: test
            RuntimeBackend::Kubernetes
        );
        assert_eq!(
            "k8s".parse::<RuntimeBackend>().unwrap(), // safety: test
            RuntimeBackend::Kubernetes
        );
        assert!("podman".parse::<RuntimeBackend>().is_err());
    }

    #[test]
    fn runtime_backend_display() {
        assert_eq!(RuntimeBackend::Docker.to_string(), "docker");
        assert_eq!(RuntimeBackend::Kubernetes.to_string(), "kubernetes");
    }

    #[test]
    fn runtime_status_is_ok() {
        assert!(RuntimeStatus::Available.is_ok());
        assert!(!RuntimeStatus::NotInstalled.is_ok());
        assert!(!RuntimeStatus::NotRunning.is_ok());
        assert!(!RuntimeStatus::Disabled.is_ok());
    }

    #[test]
    fn volume_mount_bind_string() {
        let m = VolumeMount {
            source: "/host/path".to_string(),
            target: "/workspace".to_string(),
            read_only: true,
        };
        assert_eq!(m.as_bind_string(), "/host/path:/workspace:ro");

        let m2 = VolumeMount {
            source: "/host/path".to_string(),
            target: "/workspace".to_string(),
            read_only: false,
        };
        assert_eq!(m2.as_bind_string(), "/host/path:/workspace:rw");
    }

    #[test]
    fn workload_spec_defaults() {
        let spec = WorkloadSpec::default();
        assert_eq!(spec.working_dir, "/workspace");
        assert_eq!(spec.user, Some("1000:1000".to_string()));
        assert!(spec.readonly_rootfs);
        assert!(!spec.auto_remove);
        assert_eq!(spec.cap_drop, vec!["ALL"]);
        assert_eq!(spec.cap_add, vec!["CHOWN"]);
    }

    #[test]
    fn created_at_label_round_trips_as_unix_millis() {
        let created_at = DateTime::from_timestamp_millis(1_713_111_222_333)
            .expect("millis fixture should be valid"); // safety: test fixture
        let encoded = format_workload_created_at_label(created_at);
        assert!(encoded.chars().all(|c| c.is_ascii_digit()));
        assert_eq!(parse_workload_created_at_label(&encoded), Some(created_at));
    }

    #[test]
    fn created_at_label_parser_accepts_legacy_rfc3339() {
        let parsed = parse_workload_created_at_label("2024-01-15T10:30:45+00:00");
        assert!(parsed.is_some());
    }

    #[test]
    fn created_at_label_parser_rejects_invalid_values() {
        assert!(parse_workload_created_at_label("not-a-timestamp").is_none());
        assert!(parse_workload_created_at_label("1713111222").is_none());
    }

    #[test]
    fn runtime_capabilities_summary_fields_are_stable() {
        let caps = RuntimeCapabilities::new(
            RuntimeStage::Stage2ProjectBacked,
            WorkspaceDelivery::OrchestratorBootstrap,
            ConfigDelivery::ProjectedVolume,
            NetworkIsolation::KubernetesNativeControls,
            &["network policy required"],
        );

        assert_eq!(
            caps.summary_fields(),
            [
                ("stage", "stage2-project-backed"),
                ("workspace", "orchestrator-bootstrap"),
                ("config", "projected-volume"),
                ("network", "kubernetes-native-controls"),
            ]
        );
    }
}
