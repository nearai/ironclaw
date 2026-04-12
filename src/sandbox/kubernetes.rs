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
    Container, EnvVar, Pod, PodSpec, SecurityContext, VolumeMount as K8sVolumeMount,
};
use k8s_openapi::apimachinery::pkg::api::resource::Quantity;
use kube::Client;
use kube::api::{Api, DeleteParams, LogParams, PostParams};
use uuid::Uuid;

use crate::sandbox::error::SandboxError;
use crate::sandbox::runtime::{
    ContainerRuntime, ManagedWorkload, RuntimeDetection, RuntimeStatus, WorkloadOutput,
    WorkloadSpec, parse_workload_created_at_label,
};

const LABEL_MANAGED_BY: &str = "app.kubernetes.io/managed-by";
const MANAGED_BY_VALUE: &str = "ironclaw";

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
}

/// Build a Kubernetes Pod spec from a `WorkloadSpec`.
///
/// Extracted as a free function so unit tests can call it without constructing
/// a live `KubernetesRuntime` (which requires a real `kube::Client`).
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

    let container = Container {
        name: "worker".to_string(),
        image: Some(spec.image.clone()),
        command: if spec.command.is_empty() {
            None
        } else {
            Some(spec.command.clone())
        },
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

    let mut volumes: Vec<k8s_openapi::api::core::v1::Volume> = spec
        .mounts
        .iter()
        .enumerate()
        .map(|(i, m)| k8s_openapi::api::core::v1::Volume {
            name: format!("vol-{i}"),
            host_path: Some(k8s_openapi::api::core::v1::HostPathVolumeSource {
                path: m.source.clone(),
                type_: Some("DirectoryOrCreate".to_string()),
            }),
            ..Default::default()
        })
        .collect();

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
}

#[async_trait::async_trait]
impl ContainerRuntime for KubernetesRuntime {
    fn name(&self) -> &'static str {
        "kubernetes"
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
        let pod = self.build_pod(spec);
        let api = self.pods_api();

        let created = api
            .create(&PostParams::default(), &pod)
            .await
            .map_err(|e| SandboxError::ContainerCreationFailed {
                reason: format!("pod creation failed: {e}"),
            })?;

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
            Ok(_) => Ok(()),
            Err(kube::Error::Api(e)) if e.code == 404 => Ok(()),
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
                    1
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
                None => continue,
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
        assert_eq!(
            container.command.as_ref().unwrap(), // test
            &["sleep".to_string(), "30".to_string()]
        );

        let env = container.env.as_ref().unwrap(); // test
        assert_eq!(env.len(), 2);
        assert_eq!(env[0].name, "FOO");
        assert_eq!(env[0].value.as_deref(), Some("bar"));
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
}
