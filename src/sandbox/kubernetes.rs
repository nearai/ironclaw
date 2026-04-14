//! Kubernetes backend for the `ContainerRuntime` trait.
//!
//! Maps workload lifecycle operations to Kubernetes Pod API calls.
//! Workers run as individual Pods in a configurable namespace, reaching the
//! orchestrator via cluster-DNS (`<service>.<namespace>.svc.cluster.local`).

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use futures::StreamExt;
use k8s_openapi::api::core::v1::{
    ConfigMap, ConfigMapProjection, Container, EnvVar, EphemeralVolumeSource, KeyToPath,
    PersistentVolumeClaimSpec, PersistentVolumeClaimTemplate, Pod, PodSpec, ProjectedVolumeSource,
    SecurityContext, Volume as K8sVolume, VolumeMount as K8sVolumeMount, VolumeProjection,
    VolumeResourceRequirements,
};
use k8s_openapi::apimachinery::pkg::api::resource::Quantity;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use kube::Client;
use kube::api::{Api, DeleteParams, LogParams, PostParams};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use uuid::Uuid;

use crate::sandbox::capabilities::{
    RuntimeCapabilities, kubernetes_runtime_capabilities_with_controls,
};
use crate::sandbox::error::SandboxError;
use crate::sandbox::kubernetes_policy::KubernetesIsolationReadiness;
use crate::sandbox::runtime::{
    ContainerRuntime, ManagedWorkload, RuntimeDetection, RuntimeStatus, WorkloadCommandMode,
    WorkloadOutput, WorkloadSpec, parse_workload_created_at_label,
};

const LABEL_MANAGED_BY: &str = "app.kubernetes.io/managed-by";
const MANAGED_BY_VALUE: &str = "ironclaw";

fn inline_config_key(index: usize) -> String {
    format!("file-{index}")
}

/// Kubernetes implementation of `ContainerRuntime`.
pub struct KubernetesRuntime {
    client: Client,
    namespace: String,
    orchestrator_service: String,
}

impl KubernetesRuntime {
    /// Connect using the default kubeconfig / in-cluster config.
    ///
    /// `namespace` is the Kubernetes namespace for worker pods (resolved by
    /// the config layer from DB setting / env var / default).
    pub async fn connect(namespace: &str) -> Result<Self, SandboxError> {
        let client = Client::try_default()
            .await
            .map_err(|e| SandboxError::Runtime {
                reason: format!("failed to create Kubernetes client: {}", e),
            })?;

        let orchestrator_service = std::env::var("IRONCLAW_K8S_ORCHESTRATOR_SERVICE")
            .unwrap_or_else(|_| format!("ironclaw-orchestrator.{namespace}.svc.cluster.local"));

        Ok(Self {
            client,
            namespace: namespace.to_string(),
            orchestrator_service,
        })
    }

    fn pods_api(&self) -> Api<Pod> {
        Api::namespaced(self.client.clone(), &self.namespace)
    }

    fn build_pod(&self, spec: &WorkloadSpec) -> Pod {
        build_pod_spec(&self.namespace, spec)
    }

    fn inline_config_map_name(&self, workload_name: &str) -> String {
        format!("{workload_name}-inline-config")
    }

    async fn create_inline_config_map(&self, spec: &WorkloadSpec) -> Result<(), SandboxError> {
        if spec.inline_files.is_empty() {
            return Ok(());
        }

        let config_maps: Api<ConfigMap> = Api::namespaced(self.client.clone(), &self.namespace);
        let name = self.inline_config_map_name(&spec.name);
        let data = spec
            .inline_files
            .iter()
            .enumerate()
            .map(|(index, file)| (inline_config_key(index), file.contents.clone()))
            .collect();
        let config_map = ConfigMap {
            metadata: ObjectMeta {
                name: Some(name.clone()),
                namespace: Some(self.namespace.clone()),
                labels: Some(BTreeMap::from([(
                    LABEL_MANAGED_BY.to_string(),
                    MANAGED_BY_VALUE.to_string(),
                )])),
                ..Default::default()
            },
            data: Some(data),
            ..Default::default()
        };

        config_maps
            .create(&PostParams::default(), &config_map)
            .await
            .map_err(|e| SandboxError::ContainerCreationFailed {
                reason: format!("inline config map creation failed: {e}"),
            })?;

        Ok(())
    }

    async fn remove_inline_config_map(&self, workload_id: &str) -> Result<(), SandboxError> {
        let config_maps: Api<ConfigMap> = Api::namespaced(self.client.clone(), &self.namespace);
        let name = self.inline_config_map_name(workload_id);

        match config_maps.delete(&name, &DeleteParams::default()).await {
            Ok(_) => Ok(()),
            Err(kube::Error::Api(e)) if e.code == 404 => Ok(()),
            Err(e) => Err(SandboxError::ExecutionFailed {
                reason: format!("inline config map delete failed: {e}"),
            }),
        }
    }
}

/// Build a Kubernetes Pod spec from a `WorkloadSpec`.
///
/// Extracted as a free function so unit tests can call it without constructing
/// a live `KubernetesRuntime` (which requires a real `kube::Client`).
///
/// **Volume semantics**: `WorkloadSpec.mounts` are converted to ephemeral PVCs
/// using the cluster's default `StorageClass`. These volumes start empty —
/// `VolumeMount.source` (the host path) is intentionally ignored because
/// `hostPath` volumes are a security risk and not available on managed K8s.
/// Workers that need project data should fetch it via the orchestrator API
/// rather than relying on filesystem mounts.
fn build_pod_spec(namespace: &str, spec: &WorkloadSpec) -> Pod {
    let mut labels = BTreeMap::new();
    labels.insert(LABEL_MANAGED_BY.to_string(), MANAGED_BY_VALUE.to_string());
    for (k, v) in &spec.labels {
        labels.insert(k.clone(), v.clone());
    }

    let env_vars: Vec<EnvVar> = spec
        .env
        .iter()
        .filter_map(|e| {
            let (key, value) = e.split_once('=')?;
            Some(EnvVar {
                name: key.to_string(),
                value: Some(value.to_string()),
                ..Default::default()
            })
        })
        .collect();

    let mut volume_mounts: Vec<K8sVolumeMount> = spec
        .mounts
        .iter()
        .enumerate()
        .map(|(i, m)| K8sVolumeMount {
            name: format!("vol-{i}"),
            mount_path: m.target.clone(),
            read_only: Some(m.read_only),
            ..Default::default()
        })
        .collect();

    if !spec.inline_files.is_empty() {
        for (index, file) in spec.inline_files.iter().enumerate() {
            volume_mounts.push(K8sVolumeMount {
                name: "inline-config".to_string(),
                mount_path: file.target.clone(),
                read_only: Some(true),
                sub_path: Some(inline_config_key(index)),
                ..Default::default()
            });
        }
    }

    for (i, path) in spec.tmpfs_mounts.keys().enumerate() {
        volume_mounts.push(K8sVolumeMount {
            name: format!("tmpfs-{i}"),
            mount_path: path.clone(),
            ..Default::default()
        });
    }

    let mut resources_limits = BTreeMap::new();
    if let Some(mem) = spec.memory_bytes {
        resources_limits.insert("memory".to_string(), Quantity(format!("{mem}")));
    }
    if let Some(cpu) = spec.cpu_shares {
        // Docker cpu_shares is a relative weight (default 1024 = 1 CPU core).
        // Convert to Kubernetes millicores: shares / 1024 * 1000.
        let millicores = (cpu as f64 / 1024.0 * 1000.0).round() as i64;
        let millicores = millicores.max(1);
        resources_limits.insert("cpu".to_string(), Quantity(format!("{millicores}m")));
    }

    let capabilities = if !spec.cap_drop.is_empty() || !spec.cap_add.is_empty() {
        Some(k8s_openapi::api::core::v1::Capabilities {
            drop: if spec.cap_drop.is_empty() {
                None
            } else {
                Some(spec.cap_drop.clone())
            },
            add: if spec.cap_add.is_empty() {
                None
            } else {
                Some(spec.cap_add.clone())
            },
        })
    } else {
        None
    };

    let security_context = SecurityContext {
        read_only_root_filesystem: Some(spec.readonly_rootfs),
        allow_privilege_escalation: Some(
            !spec
                .security_opts
                .iter()
                .any(|s| s.contains("no-new-privileges")),
        ),
        run_as_user: spec
            .user
            .as_ref()
            .and_then(|u| u.split(':').next().and_then(|uid| uid.parse::<i64>().ok())),
        run_as_group: spec
            .user
            .as_ref()
            .and_then(|u| u.split(':').nth(1).and_then(|gid| gid.parse::<i64>().ok())),
        capabilities,
        ..Default::default()
    };

    let (command, args) = match spec.command_mode {
        WorkloadCommandMode::ReplaceEntrypoint => {
            let command = if spec.command.is_empty() {
                None
            } else {
                Some(spec.command.clone())
            };
            (command, None)
        }
        WorkloadCommandMode::AppendToEntrypoint => {
            let args = if spec.command.is_empty() {
                None
            } else {
                Some(spec.command.clone())
            };
            (None, args)
        }
    };

    let container = Container {
        name: "worker".to_string(),
        image: Some(spec.image.clone()),
        command,
        args,
        working_dir: Some(spec.working_dir.clone()),
        env: if env_vars.is_empty() {
            None
        } else {
            Some(env_vars)
        },
        volume_mounts: if volume_mounts.is_empty() {
            None
        } else {
            Some(volume_mounts)
        },
        resources: if resources_limits.is_empty() {
            None
        } else {
            Some(k8s_openapi::api::core::v1::ResourceRequirements {
                limits: Some(resources_limits),
                ..Default::default()
            })
        },
        security_context: Some(security_context),
        ..Default::default()
    };

    let mut volumes: Vec<K8sVolume> = spec
        .mounts
        .iter()
        .enumerate()
        .map(|(i, _m)| {
            let mut requests = BTreeMap::new();
            requests.insert("storage".to_string(), Quantity("1Gi".to_string()));

            k8s_openapi::api::core::v1::Volume {
                name: format!("vol-{i}"),
                ephemeral: Some(EphemeralVolumeSource {
                    volume_claim_template: Some(PersistentVolumeClaimTemplate {
                        metadata: None,
                        spec: PersistentVolumeClaimSpec {
                            access_modes: Some(vec!["ReadWriteOnce".to_string()]),
                            resources: Some(VolumeResourceRequirements {
                                requests: Some(requests),
                                ..Default::default()
                            }),
                            ..Default::default()
                        },
                    }),
                }),
                ..Default::default()
            }
        })
        .collect();

    if !spec.inline_files.is_empty() {
        let projected_sources = spec
            .inline_files
            .iter()
            .enumerate()
            .map(|(index, file)| KeyToPath {
                key: inline_config_key(index),
                path: inline_config_key(index),
                mode: Some(file.mode),
            })
            .collect();
        volumes.push(K8sVolume {
            name: "inline-config".to_string(),
            projected: Some(ProjectedVolumeSource {
                sources: Some(vec![VolumeProjection {
                    config_map: Some(ConfigMapProjection {
                        items: Some(projected_sources),
                        name: format!("{}-inline-config", spec.name),
                        optional: Some(false),
                    }),
                    ..Default::default()
                }]),
                ..Default::default()
            }),
            ..Default::default()
        });
    }

    for (i, (_path, opts)) in spec.tmpfs_mounts.iter().enumerate() {
        let size_limit = if opts.is_empty() {
            None
        } else {
            opts.split(',')
                .find_map(|part| part.strip_prefix("size="))
                .map(|s| Quantity(s.to_string()))
        };
        volumes.push(k8s_openapi::api::core::v1::Volume {
            name: format!("tmpfs-{i}"),
            empty_dir: Some(k8s_openapi::api::core::v1::EmptyDirVolumeSource {
                medium: Some("Memory".to_string()),
                size_limit,
            }),
            ..Default::default()
        });
    }

    Pod {
        metadata: k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta {
            name: Some(spec.name.clone()),
            namespace: Some(namespace.to_string()),
            labels: Some(labels),
            ..Default::default()
        },
        spec: Some(PodSpec {
            containers: vec![container],
            restart_policy: Some("Never".to_string()),
            automount_service_account_token: Some(false),
            volumes: if volumes.is_empty() {
                None
            } else {
                Some(volumes)
            },
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// Extract the real exit code from a K8s exec `Status`.
///
/// When a command exits non-zero, Kubernetes may include the exit code in
/// `status.details.causes` with `reason: "ExitCode"` and `message: "<code>"`.
/// Falls back to 1 when the cause is absent or unparseable.
fn extract_exit_code_from_status(
    status: &k8s_openapi::apimachinery::pkg::apis::meta::v1::Status,
) -> i64 {
    if let Some(details) = &status.details
        && let Some(causes) = &details.causes
    {
        for cause in causes {
            if cause.reason.as_deref() == Some("ExitCode")
                && let Some(msg) = &cause.message
                && let Ok(code) = msg.parse::<i64>()
            {
                return code;
            }
        }
    }
    1
}

impl KubernetesRuntime {
    /// Wait until a pod reaches a terminal phase or a timeout.
    async fn wait_for_phase(
        &self,
        pod_name: &str,
        timeout: Duration,
    ) -> Result<String, SandboxError> {
        use kube::runtime::watcher;
        use kube::runtime::watcher::Event;

        let api = self.pods_api();
        let config = watcher::Config::default().fields(&format!("metadata.name={pod_name}"));

        let mut stream = watcher::watcher(api, config).boxed();

        let deadline = tokio::time::Instant::now() + timeout;

        loop {
            tokio::select! {
                _ = tokio::time::sleep_until(deadline) => {
                    return Err(SandboxError::Timeout(timeout));
                }
                event = stream.next() => {
                    match event {
                        Some(Ok(Event::Apply(pod) | Event::InitApply(pod))) => {
                            if let Some(status) = &pod.status
                                && let Some(phase) = &status.phase
                            {
                                match phase.as_str() {
                                    "Succeeded" | "Failed" => return Ok(phase.clone()),
                                    _ => continue,
                                }
                            }
                        }
                        Some(Ok(Event::Delete(_))) => {
                            return Err(SandboxError::Runtime {
                                reason: format!("pod {pod_name} was deleted while waiting"),
                            });
                        }
                        Some(Ok(Event::Init | Event::InitDone)) => continue,
                        Some(Err(e)) => {
                            return Err(SandboxError::Runtime {
                                reason: format!("watch error for pod {pod_name}: {e}"),
                            });
                        }
                        None => {
                            return Err(SandboxError::Runtime {
                                reason: format!("watch stream ended for pod {pod_name}"),
                            });
                        }
                    }
                }
            }
        }
    }

    async fn wait_for_running(
        &self,
        pod_name: &str,
        timeout: Duration,
    ) -> Result<(), SandboxError> {
        let api = self.pods_api();
        let deadline = tokio::time::Instant::now() + timeout;

        loop {
            if tokio::time::Instant::now() >= deadline {
                return Err(SandboxError::Timeout(timeout));
            }

            let pod = api.get(pod_name).await.map_err(|e| SandboxError::Runtime {
                reason: format!("failed to inspect pod {pod_name}: {e}"),
            })?;

            if let Some(status) = &pod.status
                && let Some(phase) = &status.phase
            {
                match phase.as_str() {
                    "Running" => return Ok(()),
                    "Succeeded" | "Failed" => {
                        return Err(SandboxError::ExecutionFailed {
                            reason: format!(
                                "pod {pod_name} reached terminal phase {phase} before workspace bootstrap"
                            ),
                        });
                    }
                    _ => {}
                }
            }

            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }
}

#[async_trait::async_trait]
impl ContainerRuntime for KubernetesRuntime {
    fn name(&self) -> &'static str {
        "kubernetes"
    }

    fn capabilities(&self) -> RuntimeCapabilities {
        let readiness = KubernetesIsolationReadiness::from_env();
        kubernetes_runtime_capabilities_with_controls(
            readiness.native_network_controls_enabled(),
            readiness.projected_runtime_config_enabled(),
        )
    }

    // ── Readiness ──────────────────────────────────────────────────

    async fn is_available(&self) -> bool {
        let api: Api<Pod> = Api::namespaced(self.client.clone(), &self.namespace);
        api.list(&kube::api::ListParams::default().limit(1))
            .await
            .is_ok()
    }

    async fn detect(&self) -> RuntimeDetection {
        let available = self.is_available().await;
        RuntimeDetection {
            status: if available {
                RuntimeStatus::Available
            } else {
                RuntimeStatus::NotRunning
            },
            runtime_name: "kubernetes",
            install_hint: "Install kubectl and ensure a valid kubeconfig is available.".to_string(),
            start_hint: "Check that the Kubernetes cluster is reachable (`kubectl cluster-info`)."
                .to_string(),
        }
    }

    // ── Image management ───────────────────────────────────────────

    async fn image_exists(&self, _image: &str) -> bool {
        // Kubernetes pulls images at pod creation; we can't check the node
        // image cache from the API server. Return true to skip pull-before-run.
        true
    }

    async fn pull_image(&self, _image: &str) -> Result<(), SandboxError> {
        // No-op: Kubernetes handles image pulls via imagePullPolicy.
        Ok(())
    }

    async fn build_image(&self, _image: &str, _dockerfile_path: &Path) -> Result<(), SandboxError> {
        Err(SandboxError::Config {
            reason: "Kubernetes runtime does not support local image builds. \
                     Push images to a container registry and reference them by tag."
                .to_string(),
        })
    }

    // ── Workload lifecycle ─────────────────────────────────────────

    async fn create_and_start_workload(&self, spec: &WorkloadSpec) -> Result<String, SandboxError> {
        self.create_inline_config_map(spec).await?;
        let pod = self.build_pod(spec);
        let api = self.pods_api();

        let created = match api.create(&PostParams::default(), &pod).await {
            Ok(created) => created,
            Err(e) => {
                let _ = self.remove_inline_config_map(&spec.name).await;
                return Err(SandboxError::ContainerCreationFailed {
                    reason: format!("pod creation failed: {e}"),
                });
            }
        };

        let pod_name = created.metadata.name.unwrap_or_else(|| spec.name.clone());

        tracing::debug!(pod = %pod_name, namespace = %self.namespace, "Created Kubernetes pod");

        Ok(pod_name)
    }

    async fn wait_workload(&self, workload_id: &str) -> Result<i64, SandboxError> {
        let phase = self
            .wait_for_phase(workload_id, Duration::from_secs(3600))
            .await?;

        match phase.as_str() {
            "Succeeded" => Ok(0),
            "Failed" => {
                let api = self.pods_api();
                if let Ok(pod) = api.get(workload_id).await
                    && let Some(status) = pod.status
                    && let Some(statuses) = status.container_statuses
                    && let Some(cs) = statuses.first()
                    && let Some(terminated) = &cs.state.as_ref().and_then(|s| s.terminated.as_ref())
                {
                    return Ok(terminated.exit_code.into());
                }
                Ok(1)
            }
            other => Err(SandboxError::Runtime {
                reason: format!("unexpected pod phase: {other}"),
            }),
        }
    }

    async fn stop_workload(
        &self,
        workload_id: &str,
        grace_period_secs: u32,
    ) -> Result<(), SandboxError> {
        let api = self.pods_api();
        let dp = DeleteParams {
            grace_period_seconds: Some(grace_period_secs),
            ..Default::default()
        };

        api.delete(workload_id, &dp)
            .await
            .map_err(|e| SandboxError::ExecutionFailed {
                reason: format!("pod deletion failed: {e}"),
            })?;

        Ok(())
    }

    async fn remove_workload(&self, workload_id: &str) -> Result<(), SandboxError> {
        let api = self.pods_api();
        let dp = DeleteParams {
            grace_period_seconds: Some(0),
            ..Default::default()
        };

        match api.delete(workload_id, &dp).await {
            Ok(_) => {
                self.remove_inline_config_map(workload_id).await?;
                Ok(())
            }
            Err(kube::Error::Api(e)) if e.code == 404 => {
                self.remove_inline_config_map(workload_id).await?;
                Ok(())
            }
            Err(e) => Err(SandboxError::ExecutionFailed {
                reason: format!("pod force-delete failed: {e}"),
            }),
        }
    }

    // ── Execution ──────────────────────────────────────────────────

    async fn exec_in_workload(
        &self,
        workload_id: &str,
        command: &[&str],
        _working_dir: &str,
        max_output: usize,
        timeout: Duration,
    ) -> Result<WorkloadOutput, SandboxError> {
        use kube::api::AttachParams;
        use tokio::io::AsyncReadExt;

        let start_time = std::time::Instant::now();
        let api = self.pods_api();

        let ap = AttachParams {
            container: Some("worker".to_string()),
            stdout: true,
            stderr: true,
            stdin: false,
            tty: false,
            ..Default::default()
        };

        let cmd_strs: Vec<String> = command.iter().map(|s| s.to_string()).collect();

        let exec_result = tokio::time::timeout(timeout, async {
            let mut attached = api.exec(workload_id, cmd_strs, &ap).await.map_err(|e| {
                SandboxError::ExecutionFailed {
                    reason: format!("pod exec failed: {e}"),
                }
            })?;

            let mut stdout_buf = Vec::new();
            let mut stderr_buf = Vec::new();
            let half_max = max_output / 2;

            if let Some(mut reader) = attached.stdout() {
                let mut buf = vec![0u8; 4096];
                while stdout_buf.len() < half_max {
                    match reader.read(&mut buf).await {
                        Ok(0) => break,
                        Ok(n) => stdout_buf.extend_from_slice(&buf[..n]),
                        Err(_) => break,
                    }
                }
            }

            if let Some(mut reader) = attached.stderr() {
                let mut buf = vec![0u8; 4096];
                while stderr_buf.len() < half_max {
                    match reader.read(&mut buf).await {
                        Ok(0) => break,
                        Ok(n) => stderr_buf.extend_from_slice(&buf[..n]),
                        Err(_) => break,
                    }
                }
            }

            let status_future =
                attached
                    .take_status()
                    .ok_or_else(|| SandboxError::ExecutionFailed {
                        reason: "pod exec did not return a status stream".to_string(),
                    })?;
            let exit_code: i64 = if let Some(status) = status_future.await {
                if status.status == Some("Success".to_string()) {
                    0
                } else {
                    extract_exit_code_from_status(&status)
                }
            } else {
                1
            };

            let truncated = stdout_buf.len() >= half_max || stderr_buf.len() >= half_max;

            Ok::<WorkloadOutput, SandboxError>(WorkloadOutput {
                exit_code,
                stdout: String::from_utf8_lossy(&stdout_buf).to_string(),
                stderr: String::from_utf8_lossy(&stderr_buf).to_string(),
                duration: start_time.elapsed(),
                truncated,
            })
        })
        .await;

        match exec_result {
            Ok(result) => result,
            Err(_) => Err(SandboxError::Timeout(timeout)),
        }
    }

    async fn wait_workload_ready(
        &self,
        workload_id: &str,
        timeout: Duration,
    ) -> Result<(), SandboxError> {
        self.wait_for_running(workload_id, timeout).await
    }

    async fn upload_workspace_archive(
        &self,
        workload_id: &str,
        archive_gz: &[u8],
        target_dir: &str,
    ) -> Result<(), SandboxError> {
        use kube::api::AttachParams;

        let api = self.pods_api();
        let ap = AttachParams {
            container: Some("worker".to_string()),
            stdout: true,
            stderr: true,
            stdin: true,
            tty: false,
            ..Default::default()
        };

        let cmd = vec![
            "sh".to_string(),
            "-lc".to_string(),
            format!("mkdir -p {target_dir} && tar -xzf - -C {target_dir}"),
        ];

        let mut attached =
            api.exec(workload_id, cmd, &ap)
                .await
                .map_err(|e| SandboxError::ExecutionFailed {
                    reason: format!("workspace upload exec failed: {e}"),
                })?;

        if let Some(mut stdin) = attached.stdin() {
            stdin
                .write_all(archive_gz)
                .await
                .map_err(|e| SandboxError::ExecutionFailed {
                    reason: format!("failed to stream workspace archive into pod: {e}"),
                })?;
            stdin
                .shutdown()
                .await
                .map_err(|e| SandboxError::ExecutionFailed {
                    reason: format!("failed to finish workspace archive stream: {e}"),
                })?;
        } else {
            return Err(SandboxError::ExecutionFailed {
                reason: "workspace upload exec did not expose stdin".to_string(),
            });
        }

        let mut stderr_buf = Vec::new();
        if let Some(mut stderr) = attached.stderr() {
            let mut buf = vec![0u8; 4096];
            loop {
                match stderr.read(&mut buf).await {
                    Ok(0) => break,
                    Ok(n) => stderr_buf.extend_from_slice(&buf[..n]),
                    Err(e) => {
                        return Err(SandboxError::ExecutionFailed {
                            reason: format!("failed to read workspace upload stderr: {e}"),
                        });
                    }
                }
            }
        }

        let status = attached
            .take_status()
            .ok_or_else(|| SandboxError::ExecutionFailed {
                reason: "workspace upload exec did not return a status stream".to_string(),
            })?
            .await;

        match status {
            Some(status) if status.status == Some("Success".to_string()) => Ok(()),
            Some(status) => Err(SandboxError::ExecutionFailed {
                reason: format!(
                    "workspace upload failed with exit code {}: {}",
                    extract_exit_code_from_status(&status),
                    String::from_utf8_lossy(&stderr_buf).trim()
                ),
            }),
            None => Err(SandboxError::ExecutionFailed {
                reason: "workspace upload exec returned no status".to_string(),
            }),
        }
    }

    // ── Logs ───────────────────────────────────────────────────────

    async fn collect_logs(
        &self,
        workload_id: &str,
        max_output: usize,
    ) -> Result<(String, String, bool), SandboxError> {
        let api = self.pods_api();

        let params = LogParams {
            container: Some("worker".to_string()),
            limit_bytes: Some(max_output as i64),
            ..Default::default()
        };

        let logs =
            api.logs(workload_id, &params)
                .await
                .map_err(|e| SandboxError::ExecutionFailed {
                    reason: format!("pod log retrieval failed: {e}"),
                })?;

        let truncated = logs.len() >= max_output;
        // Kubernetes pod logs combine stdout/stderr; we put it all in stdout.
        Ok((logs, String::new(), truncated))
    }

    // ── Discovery ──────────────────────────────────────────────────

    async fn list_managed_workloads(
        &self,
        label_key: &str,
    ) -> Result<Vec<ManagedWorkload>, SandboxError> {
        use kube::api::ListParams;

        let api = self.pods_api();
        let lp = ListParams::default().labels(&format!("{LABEL_MANAGED_BY}={MANAGED_BY_VALUE}"));

        let pod_list = api
            .list(&lp)
            .await
            .map_err(|e| SandboxError::ExecutionFailed {
                reason: format!("pod list failed: {e}"),
            })?;

        let mut result = Vec::new();

        for pod in pod_list {
            let pod_name = match &pod.metadata.name {
                Some(n) => n.clone(),
                None => continue,
            };

            let labels = pod.metadata.labels.unwrap_or_default();

            let job_id = match labels.get(label_key).and_then(|s| s.parse::<Uuid>().ok()) {
                Some(id) => id,
                None => continue,
            };

            let created_at = match labels
                .get("ironclaw.created_at")
                .and_then(|s| parse_workload_created_at_label(s))
                .or_else(|| pod.metadata.creation_timestamp.as_ref().map(|ts| ts.0))
            {
                Some(ts) => ts,
                None => {
                    tracing::warn!(
                        pod_name = %pod_name,
                        "Could not determine creation time for workload, skipping"
                    );
                    continue;
                }
            };

            result.push(ManagedWorkload {
                workload_id: pod_name,
                job_id,
                created_at,
            });
        }

        Ok(result)
    }

    // ── Networking ─────────────────────────────────────────────────

    fn orchestrator_host(&self) -> &str {
        &self.orchestrator_service
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn build_pod_basic_structure() {
        let spec = WorkloadSpec {
            name: "test-pod".to_string(),
            image: "worker:latest".to_string(),
            command: vec!["sleep".to_string(), "30".to_string()],
            command_mode: WorkloadCommandMode::AppendToEntrypoint,
            env: vec!["FOO=bar".to_string(), "BAZ=qux".to_string()],
            ..Default::default()
        };

        let pod = build_pod_spec("test-ns", &spec);

        assert_eq!(pod.metadata.name.as_deref(), Some("test-pod"));
        assert_eq!(pod.metadata.namespace.as_deref(), Some("test-ns"));

        let labels = pod.metadata.labels.as_ref().unwrap(); // test
        assert_eq!(
            labels.get(LABEL_MANAGED_BY).map(|s| s.as_str()),
            Some(MANAGED_BY_VALUE)
        );

        let pod_spec = pod.spec.as_ref().unwrap(); // test
        assert_eq!(pod_spec.restart_policy.as_deref(), Some("Never"));

        let container = &pod_spec.containers[0];
        assert_eq!(container.name, "worker");
        assert_eq!(container.image.as_deref(), Some("worker:latest"));
        assert!(container.command.is_none());
        assert_eq!(
            container.args.as_ref().unwrap(), // test
            &["sleep".to_string(), "30".to_string()]
        );

        let env = container.env.as_ref().unwrap(); // test
        assert_eq!(env.len(), 2);
        assert_eq!(env[0].name, "FOO");
        assert_eq!(env[0].value.as_deref(), Some("bar"));
    }

    #[test]
    fn build_pod_replace_entrypoint_uses_command() {
        let spec = WorkloadSpec {
            name: "shell-pod".to_string(),
            image: "worker:latest".to_string(),
            command: vec!["sh".to_string(), "-c".to_string(), "echo hi".to_string()],
            command_mode: WorkloadCommandMode::ReplaceEntrypoint,
            ..Default::default()
        };

        let pod = build_pod_spec("test-ns", &spec);
        let container = &pod.spec.as_ref().unwrap().containers[0]; // test

        assert_eq!(
            container.command.as_ref().unwrap(), // test
            &["sh".to_string(), "-c".to_string(), "echo hi".to_string()]
        );
        assert!(container.args.is_none());
    }

    #[test]
    fn build_pod_security_context() {
        let spec = WorkloadSpec {
            name: "sec-pod".to_string(),
            image: "worker:v1".to_string(),
            user: Some("1000:2000".to_string()),
            readonly_rootfs: true,
            security_opts: vec!["no-new-privileges:true".to_string()],
            ..Default::default()
        };

        let pod = build_pod_spec("default", &spec);
        let container = &pod.spec.as_ref().unwrap().containers[0]; // test
        let sc = container.security_context.as_ref().unwrap(); // test

        assert_eq!(sc.run_as_user, Some(1000));
        assert_eq!(sc.run_as_group, Some(2000));
        assert_eq!(sc.read_only_root_filesystem, Some(true));
        assert_eq!(sc.allow_privilege_escalation, Some(false));
    }

    #[test]
    fn build_pod_tmpfs_emptydir() {
        let mut tmpfs = HashMap::new();
        tmpfs.insert("/tmp".to_string(), "size=64M".to_string());
        tmpfs.insert("/run".to_string(), String::new());

        let spec = WorkloadSpec {
            name: "tmpfs-pod".to_string(),
            image: "worker:v1".to_string(),
            tmpfs_mounts: tmpfs,
            ..Default::default()
        };

        let pod = build_pod_spec("default", &spec);
        let pod_spec = pod.spec.as_ref().unwrap(); // test
        let container = &pod_spec.containers[0];

        let vm = container.volume_mounts.as_ref().unwrap(); // test
        let tmpfs_vms: Vec<_> = vm.iter().filter(|v| v.name.starts_with("tmpfs-")).collect();
        assert_eq!(tmpfs_vms.len(), 2);

        let volumes = pod_spec.volumes.as_ref().unwrap(); // test
        let tmpfs_vols: Vec<_> = volumes
            .iter()
            .filter(|v| v.name.starts_with("tmpfs-"))
            .collect();
        assert_eq!(tmpfs_vols.len(), 2);

        for vol in &tmpfs_vols {
            let ed = vol.empty_dir.as_ref().unwrap(); // test
            assert_eq!(ed.medium.as_deref(), Some("Memory"));
        }

        let sized_vol = tmpfs_vols.iter().find(|v| {
            v.empty_dir
                .as_ref()
                .and_then(|ed| ed.size_limit.as_ref())
                .is_some()
        });
        assert!(
            sized_vol.is_some(),
            "one tmpfs volume should have a size_limit"
        );
    }

    #[test]
    fn build_pod_capabilities() {
        let spec = WorkloadSpec {
            name: "caps-pod".to_string(),
            image: "worker:v1".to_string(),
            cap_drop: vec!["ALL".to_string()],
            cap_add: vec!["CHOWN".to_string(), "NET_BIND_SERVICE".to_string()],
            ..Default::default()
        };

        let pod = build_pod_spec("default", &spec);
        let container = &pod.spec.as_ref().unwrap().containers[0]; // test
        let sc = container.security_context.as_ref().unwrap(); // test
        let caps = sc.capabilities.as_ref().unwrap(); // test

        assert_eq!(caps.drop.as_ref().unwrap(), &["ALL".to_string()]); // test
        assert_eq!(
            caps.add.as_ref().unwrap(), // test
            &["CHOWN".to_string(), "NET_BIND_SERVICE".to_string()]
        );
    }

    #[test]
    fn build_pod_bind_mounts_use_ephemeral_pvc() {
        let spec = WorkloadSpec {
            name: "mount-pod".to_string(),
            image: "worker:v1".to_string(),
            mounts: vec![
                crate::sandbox::runtime::VolumeMount {
                    source: "/host/workspace".to_string(),
                    target: "/workspace".to_string(),
                    read_only: false,
                },
                crate::sandbox::runtime::VolumeMount {
                    source: "/host/data".to_string(),
                    target: "/data".to_string(),
                    read_only: true,
                },
            ],
            ..Default::default()
        };

        let pod = build_pod_spec("default", &spec);
        let pod_spec = pod.spec.as_ref().unwrap(); // test
        let container = &pod_spec.containers[0];

        let vm = container.volume_mounts.as_ref().unwrap(); // test
        assert_eq!(vm.len(), 2);
        assert_eq!(vm[0].mount_path, "/workspace");
        assert_eq!(vm[0].read_only, Some(false));
        assert_eq!(vm[1].mount_path, "/data");
        assert_eq!(vm[1].read_only, Some(true));

        let volumes = pod_spec.volumes.as_ref().unwrap(); // test
        assert_eq!(volumes.len(), 2);

        for vol in volumes {
            assert!(vol.host_path.is_none(), "volumes must not use hostPath");
            let eph = vol
                .ephemeral
                .as_ref()
                .expect("volume should use ephemeral PVC"); // test
            let template = eph
                .volume_claim_template
                .as_ref()
                .expect("ephemeral volume must have a claim template"); // test
            let access = template.spec.access_modes.as_ref().unwrap(); // test
            assert_eq!(access, &["ReadWriteOnce".to_string()]);
            let resources = template.spec.resources.as_ref().unwrap(); // test
            let requests = resources.requests.as_ref().unwrap(); // test
            assert!(requests.contains_key("storage"), "PVC must request storage");
        }
    }

    #[test]
    fn build_pod_inline_files_use_projected_config_volume() {
        let spec = WorkloadSpec {
            name: "config-pod".to_string(),
            image: "worker:v1".to_string(),
            inline_files: vec![crate::sandbox::runtime::InlineFileMount {
                target: "/home/sandbox/.ironclaw/mcp-servers.json".to_string(),
                contents: "{\"ok\":true}".to_string(),
                mode: 0o444,
            }],
            ..Default::default()
        };

        let pod = build_pod_spec("default", &spec);
        let pod_spec = pod.spec.as_ref().unwrap();
        let container = &pod_spec.containers[0];

        let vm = container.volume_mounts.as_ref().unwrap();
        let inline_mount = vm
            .iter()
            .find(|mount| mount.name == "inline-config")
            .expect("inline config mount should exist");
        assert_eq!(
            inline_mount.mount_path,
            "/home/sandbox/.ironclaw/mcp-servers.json"
        );
        assert_eq!(inline_mount.read_only, Some(true));
        assert_eq!(inline_mount.sub_path.as_deref(), Some("file-0"));

        let volumes = pod_spec.volumes.as_ref().unwrap();
        let inline_volume = volumes
            .iter()
            .find(|volume| volume.name == "inline-config")
            .expect("inline config volume should exist");
        let projected = inline_volume
            .projected
            .as_ref()
            .expect("inline config should use a projected volume");
        let sources = projected
            .sources
            .as_ref()
            .expect("projected sources should exist");
        assert_eq!(sources.len(), 1);
        let config_map = sources[0]
            .config_map
            .as_ref()
            .expect("projected source should come from a config map");
        assert_eq!(config_map.name, "config-pod-inline-config");
        let items = config_map
            .items
            .as_ref()
            .expect("config map items should exist");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].key, "file-0");
        assert_eq!(items[0].path, "file-0");
        assert_eq!(items[0].mode, Some(0o444));
    }

    #[test]
    fn build_pod_cpu_shares_to_millicores() {
        let spec = WorkloadSpec {
            name: "cpu-pod".to_string(),
            image: "worker:v1".to_string(),
            cpu_shares: Some(512),
            ..Default::default()
        };

        let pod = build_pod_spec("default", &spec);
        let container = &pod.spec.as_ref().unwrap().containers[0]; // test
        let resources = container.resources.as_ref().unwrap(); // test
        let limits = resources.limits.as_ref().unwrap(); // test
        assert_eq!(limits.get("cpu").unwrap().0, "500m"); // test
    }

    #[test]
    fn extract_exit_code_from_status_with_cause() {
        use k8s_openapi::apimachinery::pkg::apis::meta::v1::{Status, StatusCause, StatusDetails};

        let status = Status {
            status: Some("Failure".to_string()),
            message: Some("command terminated with non-zero exit code".to_string()),
            details: Some(StatusDetails {
                causes: Some(vec![StatusCause {
                    reason: Some("ExitCode".to_string()),
                    message: Some("137".to_string()),
                    ..Default::default()
                }]),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert_eq!(extract_exit_code_from_status(&status), 137);
    }

    #[test]
    fn extract_exit_code_falls_back_to_1() {
        use k8s_openapi::apimachinery::pkg::apis::meta::v1::Status;

        let status = Status {
            status: Some("Failure".to_string()),
            ..Default::default()
        };
        assert_eq!(extract_exit_code_from_status(&status), 1);
    }
}
