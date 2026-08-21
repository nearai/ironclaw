use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};

use super::{registry, user_key::RebornSandboxUserKey, worker_spec};
use bollard::{
    Docker,
    container::{
        Config, CreateContainerOptions, InspectContainerOptions, ListContainersOptions,
        LogsOptions, RemoveContainerOptions, StartContainerOptions,
    },
    image::CreateImageOptions,
    models::{HealthConfig, HealthStatusEnum, HostConfig, HostConfigLogConfig},
    network::{
        ConnectNetworkOptions, CreateNetworkOptions, DisconnectNetworkOptions,
        InspectNetworkOptions, ListNetworksOptions,
    },
};
use futures_util::StreamExt;
use ironclaw_common::env_helpers::env_or_override;
use ironclaw_host_api::{
    action::NetworkPolicy,
    ids::{InvocationId, TenantId, UserId},
    process::RuntimeProcessError,
};
use sha2::{Digest, Sha256};
pub(super) const PROXY_LABEL_PREFIX: &str = "ironclaw.proxy";
pub(super) const NETWORK_LABEL_PREFIX: &str = "ironclaw.network";
pub(super) const DEFAULT_PROXY_IMAGE: &str =
    "ironsh/iron-proxy@sha256:c4628019c24f4cc8d77564a26b7c9cedb00accee6f93d06270e85fb8f9c6a7da";
pub(super) const PROXY_IMAGE_ENV: &str = "IRONCLAW_REBORN_SANDBOX_PROXY_IMAGE";
const PROXY_CONFIG_PATH: &str = "/run/ironclaw-proxy/proxy.yaml";
const PROXY_MATERIAL_ROOT: &str = "/run/ironclaw-proxy";
const PROXY_INVOCATION_ID_PATH: &str = "/run/ironclaw-proxy/invocation-id";
const SHARED_UPSTREAM_NETWORK_NAME: &str = "ironclaw-reborn-sandbox-upstream";
const SHARED_UPSTREAM_LABEL_ROLE: &str = "ironclaw.network.role";
const SHARED_UPSTREAM_ROLE: &str = "shared-proxy-upstream-v1";
const ISOLATED_GATEWAY_OPTION: &str = "com.docker.network.bridge.gateway_mode_ipv4";
const ISOLATED_GATEWAY_MODE: &str = "isolated";
const PROXY_TUNNEL_PORT: u16 = 3128;
const PROXY_AUDIT_CAPTURE_BYTES: usize = 4 * 1024 * 1024;
const PROXY_AUDIT_ROTATE_BYTES: u64 = 8 * 1024 * 1024;
const PROXY_AUDIT_DIR_BUDGET_BYTES: u64 = 256 * 1024 * 1024;
/// Parity with `ironclaw_network`'s private-address classifier: every range
/// that classifier rejects must appear here, so an allowlisted hostname
/// resolving into a non-public address is denied by the proxy exactly as the
/// canonical host enforcer would deny it (DNS-rebinding parity). This is
/// proxy configuration data handed to the sidecar, not an in-crate address
/// check — runtime crates must never classify addresses themselves.
const DENIED_UPSTREAM_CIDRS: &[&str] = &[
    "0.0.0.0/8",
    "10.0.0.0/8",
    "100.64.0.0/10",
    "127.0.0.0/8",
    "169.254.0.0/16",
    "172.16.0.0/12",
    "192.0.2.0/24",
    "192.168.0.0/16",
    "198.18.0.0/15",
    "198.51.100.0/24",
    "203.0.113.0/24",
    "224.0.0.0/4",
    "240.0.0.0/4",
    "::/128",
    "::1/128",
    "::ffff:0:0/96",
    "2001:db8::/32",
    "fc00::/7",
    "fe80::/10",
    "ff00::/8",
];
#[derive(Debug, Clone)]
pub(super) struct ManagedEgressConfig {
    proxy_image: String,
    policy: NetworkPolicy,
    material_root: PathBuf,
}

impl ManagedEgressConfig {
    pub(super) fn from_policy(
        policy: NetworkPolicy,
        material_root: PathBuf,
    ) -> Result<Self, RuntimeProcessError> {
        if policy.allowed_targets.is_empty() {
            return Err(RuntimeProcessError::ExecutionFailed(
                "sandbox managed egress policy must contain at least one allowed target"
                    .to_string(),
            ));
        }
        if !policy.deny_private_ip_ranges {
            return Err(RuntimeProcessError::ExecutionFailed(
                "sandbox managed egress policy must deny private IP ranges".to_string(),
            ));
        }
        reject_wildcard_targets(&policy)?;
        let proxy_image = configured_proxy_image()?;
        Ok(Self {
            proxy_image,
            policy,
            material_root,
        })
    }
}

pub(super) fn configured_proxy_image() -> Result<String, RuntimeProcessError> {
    let image = env_or_override(PROXY_IMAGE_ENV).unwrap_or_else(|| DEFAULT_PROXY_IMAGE.to_string());
    let Some((_, digest)) = image.rsplit_once("@sha256:") else {
        return Err(RuntimeProcessError::ExecutionFailed(
            "sandbox proxy image must be pinned by sha256 digest".to_string(),
        ));
    };
    if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(RuntimeProcessError::ExecutionFailed(
            "sandbox proxy image has an invalid sha256 digest".to_string(),
        ));
    }
    Ok(image)
}

async fn resolve_proxy_image(
    docker: &Docker,
    configured_image: &str,
) -> Result<String, RuntimeProcessError> {
    match docker.inspect_image(configured_image).await {
        Ok(inspected) => immutable_image_id(inspected.id),
        Err(error) if docker_status(&error) == Some(404) => {
            let mut pull = docker.create_image(
                Some(CreateImageOptions {
                    from_image: configured_image,
                    ..Default::default()
                }),
                None,
                None,
            );
            while let Some(progress) = pull.next().await {
                progress.map_err(|error| {
                    RuntimeProcessError::ExecutionFailed(format!(
                        "sandbox proxy image pull failed: {error}"
                    ))
                })?;
            }
            let inspected = docker
                .inspect_image(configured_image)
                .await
                .map_err(|error| {
                    RuntimeProcessError::ExecutionFailed(format!(
                        "sandbox proxy image could not be resolved after pull: {error}"
                    ))
                })?;
            immutable_image_id(inspected.id)
        }
        Err(error) => Err(RuntimeProcessError::ExecutionFailed(format!(
            "sandbox proxy image could not be resolved: {error}"
        ))),
    }
}

fn immutable_image_id(image_id: Option<String>) -> Result<String, RuntimeProcessError> {
    image_id.ok_or_else(|| {
        RuntimeProcessError::ExecutionFailed(
            "sandbox proxy image resolved without an immutable image id".to_string(),
        )
    })
}

pub(super) struct ManagedEgressRuntime {
    proxy_image: String,
    policy: NetworkPolicy,
    posture: String,
    material_root: PathBuf,
    upstream_gate: tokio::sync::Mutex<()>,
}

impl std::fmt::Debug for ManagedEgressRuntime {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ManagedEgressRuntime")
            .field("proxy_image", &self.proxy_image)
            .field("posture", &self.posture)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone)]
pub(super) struct ManagedEgressBundle {
    pub(super) network_name: String,
    pub(super) proxy_ip: String,
    proxy_host: String,
    pub(super) posture: String,
    invocation_id_path: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ManagedNetworkStatus {
    Missing,
    Compatible,
    Incompatible,
}

impl ManagedEgressRuntime {
    pub(super) async fn connect(
        docker: &Docker,
        config: ManagedEgressConfig,
    ) -> Result<Arc<Self>, RuntimeProcessError> {
        let proxy_image = resolve_proxy_image(docker, &config.proxy_image).await?;
        let material_root = config.material_root;
        create_material_directory(&material_root, 0o711).await?;
        let posture = proxy_posture(&proxy_image, &config.policy, &material_root)?;
        Ok(Arc::new(Self {
            proxy_image,
            policy: config.policy,
            posture,
            material_root,
            upstream_gate: tokio::sync::Mutex::new(()),
        }))
    }

    pub(super) async fn ensure_bundle(
        &self,
        docker: &Docker,
        key: &RebornSandboxUserKey,
        tenant_id: &TenantId,
        user_id: &UserId,
    ) -> Result<ManagedEgressBundle, RuntimeProcessError> {
        match self
            .ensure_bundle_inner(docker, key, tenant_id, user_id)
            .await
        {
            Ok(bundle) => Ok(bundle),
            Err(setup_error) => {
                match self
                    .rollback_provisioned_bundle(docker, key, &key.container_name())
                    .await
                {
                    Ok(()) => Err(setup_error),
                    Err(cleanup_error) => Err(RuntimeProcessError::ExecutionFailed(format!(
                        "{setup_error}; managed-egress provisioning rollback failed: {cleanup_error}"
                    ))),
                }
            }
        }
    }

    async fn ensure_bundle_inner(
        &self,
        docker: &Docker,
        key: &RebornSandboxUserKey,
        tenant_id: &TenantId,
        user_id: &UserId,
    ) -> Result<ManagedEgressBundle, RuntimeProcessError> {
        let network_name = key.network_name();
        let proxy_name = key.proxy_name();
        let upstream_network_name = SHARED_UPSTREAM_NETWORK_NAME;
        let proxy_material_root = self.material_root.join(&proxy_name);
        create_material_directory(&proxy_material_root, 0o711).await?;
        let invocation_id_path = proxy_material_root.join("invocation-id");
        if !tokio::fs::try_exists(&invocation_id_path)
            .await
            .map_err(|error| {
                RuntimeProcessError::ExecutionFailed(format!(
                    "sandbox proxy invocation marker check failed: {error}"
                ))
            })?
        {
            // The proxy drops DAC_OVERRIDE and may not share the host process
            // UID. This is attribution metadata, not a credential, so it must
            // be world-readable across the read-only bind mount.
            write_atomic_material_file(&invocation_id_path, b"unassigned").await?;
        }
        let network_labels = registry::build_user_container_launch_labels(
            NETWORK_LABEL_PREFIX,
            tenant_id,
            user_id,
            "internal-bridge-v1",
            &self.posture,
        );
        let upstream_network_labels = HashMap::from([(
            SHARED_UPSTREAM_LABEL_ROLE.to_string(),
            SHARED_UPSTREAM_ROLE.to_string(),
        )]);
        let proxy_labels = registry::build_user_container_launch_labels(
            PROXY_LABEL_PREFIX,
            tenant_id,
            user_id,
            &self.proxy_image,
            &self.posture,
        );

        match managed_network_status(docker, &network_name, &network_labels, true).await? {
            ManagedNetworkStatus::Compatible => {}
            ManagedNetworkStatus::Missing => {
                create_bridge_network(docker, &network_name, network_labels, true).await?;
            }
            ManagedNetworkStatus::Incompatible => {
                remove_user_container_if_present(docker, &key.container_name()).await?;
                self.preserve_proxy_audit(docker, &proxy_name, &proxy_name)
                    .await?;
                remove_proxy_if_present(docker, &proxy_name).await?;
                remove_network_if_present(docker, &network_name).await?;
                create_bridge_network(docker, &network_name, network_labels, true).await?;
            }
        }
        {
            let _upstream = self.upstream_gate.lock().await;
            match managed_network_status(
                docker,
                upstream_network_name,
                &upstream_network_labels,
                false,
            )
            .await?
            {
                ManagedNetworkStatus::Compatible => {}
                ManagedNetworkStatus::Missing => {
                    create_bridge_network(
                        docker,
                        upstream_network_name,
                        upstream_network_labels,
                        false,
                    )
                    .await?;
                }
                ManagedNetworkStatus::Incompatible => {
                    return Err(RuntimeProcessError::ExecutionFailed(
                        "sandbox shared proxy upstream network is incompatible; remove \
                         'ironclaw-reborn-sandbox-upstream' before retrying"
                            .to_string(),
                    ));
                }
            }
        }

        let proxy_ip = match inspect_proxy(docker, &proxy_name).await? {
            Some(inspected)
                if proxy_is_compatible(
                    &inspected,
                    &proxy_labels,
                    &network_name,
                    upstream_network_name,
                ) && proxy_is_ready(&inspected) =>
            {
                proxy_ip_on_network(&inspected, &network_name)?
            }
            Some(inspected)
                if proxy_is_compatible(
                    &inspected,
                    &proxy_labels,
                    &network_name,
                    upstream_network_name,
                ) =>
            {
                self.preserve_proxy_audit(docker, &proxy_name, &proxy_name)
                    .await?;
                remove_proxy_if_present(docker, &proxy_name).await?;
                self.create_proxy(
                    docker,
                    &proxy_name,
                    &network_name,
                    upstream_network_name,
                    proxy_labels,
                    &proxy_material_root,
                )
                .await?
            }
            Some(_) => {
                remove_user_container_if_present(docker, &key.container_name()).await?;
                self.preserve_proxy_audit(docker, &proxy_name, &proxy_name)
                    .await?;
                remove_proxy_if_present(docker, &proxy_name).await?;
                self.create_proxy(
                    docker,
                    &proxy_name,
                    &network_name,
                    upstream_network_name,
                    proxy_labels,
                    &proxy_material_root,
                )
                .await?
            }
            None => {
                self.create_proxy(
                    docker,
                    &proxy_name,
                    &network_name,
                    upstream_network_name,
                    proxy_labels,
                    &proxy_material_root,
                )
                .await?
            }
        };
        Ok(ManagedEgressBundle {
            network_name,
            proxy_ip,
            proxy_host: proxy_name,
            posture: self.posture.clone(),
            invocation_id_path,
        })
    }

    /// Updates attribution while the per-user lifecycle gate is held.
    ///
    /// `ironclaw-exec` terminates and reaps every descendant on normal exit,
    /// signal, and timeout before the gate can advance to another invocation.
    /// The marker therefore cannot be observed by a surviving prior command.
    pub(super) async fn set_invocation(
        &self,
        bundle: &ManagedEgressBundle,
        invocation_id: &InvocationId,
    ) -> Result<(), RuntimeProcessError> {
        // The capability-dropped proxy may not share the host process UID.
        // Attribution metadata must remain readable across that boundary.
        let invocation_id = invocation_id.to_string();
        write_atomic_material_file(&bundle.invocation_id_path, invocation_id.as_bytes()).await
    }

    async fn create_proxy(
        &self,
        docker: &Docker,
        proxy_name: &str,
        network_name: &str,
        upstream_network_name: &str,
        labels: HashMap<String, String>,
        proxy_material_root: &std::path::Path,
    ) -> Result<String, RuntimeProcessError> {
        let proxy_config_path = proxy_material_root.join("proxy.yaml");
        // The config binds listeners to this proxy's private-network address,
        // which Docker assigns at start. Remove any retained config so the
        // container's wait loop cannot exec against a previous address, then
        // write the real one once the address is known: a rendered config is
        // therefore never stale, and no endpoint IP has to be requested (the
        // daemon rejects caller-specified IPs on auto-subnet networks).
        remove_material_file(&proxy_config_path).await?;
        let binds = vec![docker_readonly_bind(
            proxy_material_root,
            PROXY_MATERIAL_ROOT,
        )?];
        let config = Config {
            image: Some(self.proxy_image.clone()),
            entrypoint: Some(vec!["/bin/sh".to_string(), "-c".to_string()]),
            // The host writes this config after start (the bound address is
            // only known then) by renaming into the bind-mounted directory. A
            // `stat`-only guard can observe the new dentry before the file is
            // openable, so require a successful open before exec — otherwise
            // iron-proxy exits ENOENT and the proxy never becomes healthy.
            cmd: Some(vec![format!(
                "while [ ! -s {PROXY_CONFIG_PATH} ] || ! cat {PROXY_CONFIG_PATH} >/dev/null 2>&1; do sleep 0.05; done; exec /usr/local/bin/iron-proxy -config {PROXY_CONFIG_PATH}"
            )]),
            labels: Some(labels),
            healthcheck: Some(HealthConfig {
                test: Some(vec![
                    "CMD-SHELL".to_string(),
                    format!(
                        "address=$(sed -n 's/^  tunnel_listen: \"\\([^\"]*\\)\"/\\1/p' {PROXY_CONFIG_PATH}); host=${{address%:*}}; port=${{address##*:}}; nc -z \"$host\" \"$port\""
                    ),
                ]),
                interval: Some(100_000_000),
                timeout: Some(1_000_000_000),
                retries: Some(20),
                ..Default::default()
            }),
            host_config: Some(HostConfig {
                network_mode: Some(network_name.to_string()),
                auto_remove: Some(false),
                cap_drop: Some(vec!["ALL".to_string()]),
                cap_add: Some(vec!["NET_BIND_SERVICE".to_string()]),
                security_opt: Some(vec!["no-new-privileges:true".to_string()]),
                readonly_rootfs: Some(true),
                pids_limit: Some(128),
                memory: Some(128 * 1024 * 1024),
                binds: Some(binds),
                tmpfs: Some(HashMap::from([(
                    "/tmp".to_string(),
                    "rw,noexec,nosuid,nodev,size=16m".to_string(),
                )])),
                log_config: Some(HostConfigLogConfig {
                    typ: Some(worker_spec::DOCKER_WORKER_LOG_DRIVER.to_string()),
                    config: Some(HashMap::from([
                        (
                            "max-size".to_string(),
                            worker_spec::DOCKER_WORKER_LOG_MAX_SIZE.to_string(),
                        ),
                        (
                            "max-file".to_string(),
                            worker_spec::DOCKER_WORKER_LOG_MAX_FILES.to_string(),
                        ),
                    ])),
                }),
                ..Default::default()
            }),
            attach_stdout: Some(false),
            attach_stderr: Some(false),
            open_stdin: Some(false),
            networking_config: None,
            ..Default::default()
        };
        docker
            .create_container(
                Some(CreateContainerOptions {
                    name: proxy_name.to_string(),
                    platform: None,
                }),
                config,
            )
            .await
            .map_err(|error| {
                RuntimeProcessError::ExecutionFailed(format!(
                    "sandbox proxy container create failed: {error}"
                ))
            })?;
        if let Err(error) = docker
            .connect_network(
                upstream_network_name,
                ConnectNetworkOptions {
                    container: proxy_name,
                    endpoint_config: Default::default(),
                },
            )
            .await
        {
            let _ = remove_proxy_if_present(docker, proxy_name).await;
            return Err(RuntimeProcessError::ExecutionFailed(format!(
                "sandbox proxy upstream network attach failed: {error}"
            )));
        }
        if let Err(error) = start_proxy_container(docker, proxy_name).await {
            let _ = remove_proxy_if_present(docker, proxy_name).await;
            return Err(error);
        }
        let inspected = inspect_proxy(docker, proxy_name).await?.ok_or_else(|| {
            RuntimeProcessError::ExecutionFailed(
                "sandbox proxy disappeared immediately after start".to_string(),
            )
        })?;
        let proxy_ip = match proxy_ip_on_network(&inspected, network_name) {
            Ok(proxy_ip) => proxy_ip,
            Err(error) => {
                let _ = remove_proxy_if_present(docker, proxy_name).await;
                return Err(error);
            }
        };
        let proxy_config = render_proxy_config(&self.policy, &proxy_ip)?;
        if let Err(error) =
            write_atomic_material_file(&proxy_config_path, proxy_config.as_bytes()).await
        {
            let _ = remove_proxy_if_present(docker, proxy_name).await;
            return Err(error);
        }
        if let Err(error) = wait_proxy_ready(docker, proxy_name).await {
            let detail = proxy_failure_detail(docker, proxy_name).await;
            let _ = remove_proxy_if_present(docker, proxy_name).await;
            return Err(RuntimeProcessError::ExecutionFailed(format!(
                "{error}{detail}"
            )));
        }
        Ok(proxy_ip)
    }
    pub(super) fn user_environment(
        &self,
        bundle: &ManagedEgressBundle,
    ) -> Result<Vec<String>, RuntimeProcessError> {
        let proxy_url =
            url::Url::parse(&format!("http://{}:{PROXY_TUNNEL_PORT}", bundle.proxy_host))
                .map_err(|error| {
                    RuntimeProcessError::ExecutionFailed(format!(
                        "sandbox proxy URL construction failed: {error}"
                    ))
                })?
                .to_string();
        Ok([
            ("IRONCLAW_REBORN_NETWORK_MODE", "brokered"),
            ("IRONCLAW_REBORN_HTTP_PROXY", proxy_url.as_str()),
            ("http_proxy", proxy_url.as_str()),
            ("https_proxy", proxy_url.as_str()),
            ("HTTP_PROXY", proxy_url.as_str()),
            ("HTTPS_PROXY", proxy_url.as_str()),
            ("NO_PROXY", ""),
            ("no_proxy", ""),
        ]
        .into_iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect())
    }

    pub(super) async fn reconcile_orphaned_bundles(
        &self,
        docker: &Docker,
        live_users: &HashSet<RebornSandboxUserKey>,
    ) -> Result<(), RuntimeProcessError> {
        let proxy_tenant_label = registry::label_tenant(PROXY_LABEL_PREFIX);
        let proxy_user_label = registry::label_user(PROXY_LABEL_PREFIX);
        let proxy_filters = HashMap::from([(
            "label".to_string(),
            vec![proxy_tenant_label.clone(), proxy_user_label.clone()],
        )]);
        let proxies = docker
            .list_containers(Some(ListContainersOptions {
                all: true,
                filters: proxy_filters,
                ..Default::default()
            }))
            .await
            .map_err(|error| {
                RuntimeProcessError::ExecutionFailed(format!(
                    "sandbox proxy reconciliation failed: {error}"
                ))
            })?;
        for proxy in proxies {
            let key = proxy.labels.as_ref().and_then(|labels| {
                managed_resource_key(labels, &proxy_tenant_label, &proxy_user_label)
            });
            if key.as_ref().is_some_and(|key| live_users.contains(key)) {
                continue;
            }
            if let Some(id) = proxy.id.as_deref() {
                let audit_name = key
                    .as_ref()
                    .map(RebornSandboxUserKey::proxy_name)
                    .unwrap_or_else(|| id.to_string());
                self.preserve_proxy_audit(docker, id, &audit_name).await?;
                remove_proxy_if_present(docker, id).await?;
            }
            if let Some(key) = key {
                remove_material_directory(&self.material_root.join(key.proxy_name())).await?;
            }
        }

        let network_tenant_label = registry::label_tenant(NETWORK_LABEL_PREFIX);
        let network_user_label = registry::label_user(NETWORK_LABEL_PREFIX);
        let network_filters = HashMap::from([(
            "label".to_string(),
            vec![network_tenant_label.clone(), network_user_label.clone()],
        )]);
        let networks = docker
            .list_networks(Some(ListNetworksOptions {
                filters: network_filters,
            }))
            .await
            .map_err(|error| {
                RuntimeProcessError::ExecutionFailed(format!(
                    "sandbox network reconciliation failed: {error}"
                ))
            })?;
        for network in networks {
            let key = network.labels.as_ref().and_then(|labels| {
                managed_resource_key(labels, &network_tenant_label, &network_user_label)
            });
            if key.as_ref().is_some_and(|key| live_users.contains(key)) {
                continue;
            }
            if let Some(id) = network.id.as_deref() {
                remove_network_if_present(docker, id).await?;
            }
        }
        Ok(())
    }

    /// Drains the proxy container's structured audit log into a durable
    /// per-user directory under `<material_root>/audit/` before the container
    /// (and with it Docker's `json-file` log) is deleted. Capture keeps the
    /// most recent [`PROXY_AUDIT_CAPTURE_BYTES`]; the destination rotates
    /// once past [`PROXY_AUDIT_ROTATE_BYTES`], bounding each user's disk usage
    /// while preserving recent egress evidence across suspension and retention.
    async fn preserve_proxy_audit(
        &self,
        docker: &Docker,
        container: &str,
        audit_name: &str,
    ) -> Result<(), RuntimeProcessError> {
        let mut logs = docker.logs(
            container,
            Some(LogsOptions::<String> {
                stdout: true,
                stderr: true,
                timestamps: true,
                tail: "all".to_string(),
                ..Default::default()
            }),
        );
        let mut captured: Vec<u8> = Vec::new();
        while let Some(chunk) = logs.next().await {
            match chunk {
                Ok(output) => {
                    captured.extend_from_slice(&output.into_bytes());
                    if captured.len() > PROXY_AUDIT_CAPTURE_BYTES {
                        let excess = captured.len() - PROXY_AUDIT_CAPTURE_BYTES;
                        captured.drain(..excess);
                    }
                }
                Err(error) if docker_status(&error) == Some(404) => return Ok(()),
                Err(error) => {
                    return Err(RuntimeProcessError::ExecutionFailed(format!(
                        "sandbox proxy audit log read failed: {error}"
                    )));
                }
            }
        }
        if captured.is_empty() {
            return Ok(());
        }
        let audit_dir = self.material_root.join("audit").join(audit_name);
        create_material_directory(&audit_dir, 0o700).await?;
        let audit_path = audit_dir.join("proxy.log");
        match tokio::fs::metadata(&audit_path).await {
            Ok(metadata) if metadata.len() > PROXY_AUDIT_ROTATE_BYTES => {
                let rotated = audit_dir.join("proxy.log.1");
                match tokio::fs::remove_file(&rotated).await {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => {
                        return Err(RuntimeProcessError::ExecutionFailed(format!(
                            "sandbox proxy rotated audit log cleanup failed: {error}"
                        )));
                    }
                }
                tokio::fs::rename(&audit_path, &rotated)
                    .await
                    .map_err(|error| {
                        RuntimeProcessError::ExecutionFailed(format!(
                            "sandbox proxy audit log rotation failed: {error}"
                        ))
                    })?;
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(RuntimeProcessError::ExecutionFailed(format!(
                    "sandbox proxy audit log metadata failed: {error}"
                )));
            }
        }
        let mut options = tokio::fs::OpenOptions::new();
        options.create(true).append(true);
        #[cfg(unix)]
        options.mode(0o600);
        let mut file = options.open(&audit_path).await.map_err(|error| {
            RuntimeProcessError::ExecutionFailed(format!(
                "sandbox proxy audit log open failed: {error}"
            ))
        })?;
        tokio::io::AsyncWriteExt::write_all(&mut file, &captured)
            .await
            .map_err(|error| {
                RuntimeProcessError::ExecutionFailed(format!(
                    "sandbox proxy audit log write failed: {error}"
                ))
            })?;
        tokio::io::AsyncWriteExt::flush(&mut file)
            .await
            .map_err(|error| {
                RuntimeProcessError::ExecutionFailed(format!(
                    "sandbox proxy audit log flush failed: {error}"
                ))
            })?;
        file.sync_data().await.map_err(|error| {
            RuntimeProcessError::ExecutionFailed(format!(
                "sandbox proxy audit log sync failed: {error}"
            ))
        })?;
        enforce_audit_budget(&audit_dir, PROXY_AUDIT_DIR_BUDGET_BYTES).await?;
        Ok(())
    }

    pub(super) async fn remove_proxy(
        &self,
        docker: &Docker,
        key: &RebornSandboxUserKey,
    ) -> Result<(), RuntimeProcessError> {
        let proxy_name = key.proxy_name();
        self.preserve_proxy_audit(docker, &proxy_name, &proxy_name)
            .await?;
        remove_proxy_if_present(docker, &proxy_name).await?;
        remove_material_directory(&self.material_root.join(proxy_name)).await
    }

    pub(super) async fn suspend_bundle(
        &self,
        docker: &Docker,
        key: &RebornSandboxUserKey,
    ) -> Result<(), RuntimeProcessError> {
        self.remove_proxy(docker, key).await
    }

    pub(super) async fn rollback_provisioned_bundle(
        &self,
        docker: &Docker,
        key: &RebornSandboxUserKey,
        user_container_name: &str,
    ) -> Result<(), RuntimeProcessError> {
        self.suspend_bundle(docker, key).await?;
        if !container_attached_to_network(docker, user_container_name, &key.network_name()).await? {
            remove_network_if_present(docker, &key.network_name()).await?;
        }
        Ok(())
    }

    pub(super) async fn remove_bundle(
        &self,
        docker: &Docker,
        key: &RebornSandboxUserKey,
        user_container_name: &str,
    ) -> Result<(), RuntimeProcessError> {
        self.suspend_bundle(docker, key).await?;
        disconnect_container_if_attached(docker, &key.network_name(), user_container_name).await?;
        remove_network_if_present(docker, &key.network_name()).await
    }
}

fn managed_resource_key(
    labels: &HashMap<String, String>,
    tenant_label: &str,
    user_label: &str,
) -> Option<RebornSandboxUserKey> {
    let tenant = TenantId::new(labels.get(tenant_label)?).ok()?;
    let user = UserId::new(labels.get(user_label)?).ok()?;
    Some(RebornSandboxUserKey::from_tenant_user(&tenant, &user))
}

pub(super) async fn ensure_user_container_attached(
    docker: &Docker,
    key: &RebornSandboxUserKey,
    user_container_name: &str,
) -> Result<(), RuntimeProcessError> {
    let network_name = key.network_name();
    if container_attached_to_network(docker, user_container_name, &network_name).await? {
        return Ok(());
    }
    docker
        .connect_network(
            &network_name,
            ConnectNetworkOptions {
                container: user_container_name,
                endpoint_config: Default::default(),
            },
        )
        .await
        .map_err(|error| {
            RuntimeProcessError::ExecutionFailed(format!(
                "sandbox user container private-network attach failed: {error}"
            ))
        })
}

async fn create_bridge_network(
    docker: &Docker,
    name: &str,
    labels: HashMap<String, String>,
    internal: bool,
) -> Result<(), RuntimeProcessError> {
    docker
        .create_network(CreateNetworkOptions {
            name: name.to_string(),
            check_duplicate: true,
            driver: "bridge".to_string(),
            internal,
            attachable: false,
            ingress: false,
            ipam: Default::default(),
            enable_ipv6: false,
            options: if internal {
                HashMap::from([(
                    ISOLATED_GATEWAY_OPTION.to_string(),
                    ISOLATED_GATEWAY_MODE.to_string(),
                )])
            } else {
                HashMap::new()
            },
            labels,
        })
        .await
        .map_err(network_create_error)?;
    Ok(())
}

fn network_create_error(error: bollard::errors::Error) -> RuntimeProcessError {
    let detail = error.to_string();
    let normalized = detail.to_ascii_lowercase();
    if normalized.contains("non-overlapping ipv4 address pool")
        || normalized.contains("all predefined address pools have been fully subnetted")
    {
        return RuntimeProcessError::ExecutionFailed(format!(
            "sandbox private network address pool is exhausted; configure Docker \
             default-address-pools in /etc/docker/daemon.json before enabling user sandboxes: \
             {detail}"
        ));
    }
    RuntimeProcessError::ExecutionFailed(format!("sandbox private network create failed: {detail}"))
}

async fn managed_network_status(
    docker: &Docker,
    name: &str,
    expected_labels: &HashMap<String, String>,
    expected_internal: bool,
) -> Result<ManagedNetworkStatus, RuntimeProcessError> {
    match docker
        .inspect_network(name, None::<InspectNetworkOptions<&str>>)
        .await
    {
        Ok(network) => {
            let compatible = network.internal == Some(expected_internal)
                && network.enable_ipv6 != Some(true)
                && (!expected_internal
                    || network.options.as_ref().is_some_and(|options| {
                        options.get(ISOLATED_GATEWAY_OPTION).map(String::as_str)
                            == Some(ISOLATED_GATEWAY_MODE)
                    }))
                && network.labels.as_ref().is_some_and(|labels| {
                    expected_labels
                        .iter()
                        .filter(|(key, _)| !key.ends_with(".created_at"))
                        .all(|(key, value)| labels.get(key) == Some(value))
                });
            Ok(if compatible {
                ManagedNetworkStatus::Compatible
            } else {
                ManagedNetworkStatus::Incompatible
            })
        }
        Err(error) if docker_status(&error) == Some(404) => Ok(ManagedNetworkStatus::Missing),
        Err(error) => Err(RuntimeProcessError::ExecutionFailed(format!(
            "sandbox private network inspect failed: {error}"
        ))),
    }
}

async fn inspect_proxy(
    docker: &Docker,
    name: &str,
) -> Result<Option<bollard::models::ContainerInspectResponse>, RuntimeProcessError> {
    match docker
        .inspect_container(name, None::<InspectContainerOptions>)
        .await
    {
        Ok(inspected) => Ok(Some(inspected)),
        Err(error) if docker_status(&error) == Some(404) => Ok(None),
        Err(error) => Err(RuntimeProcessError::ExecutionFailed(format!(
            "sandbox proxy inspect failed: {error}"
        ))),
    }
}

fn proxy_is_compatible(
    inspected: &bollard::models::ContainerInspectResponse,
    expected_labels: &HashMap<String, String>,
    network_name: &str,
    upstream_network_name: &str,
) -> bool {
    let labels = inspected
        .config
        .as_ref()
        .and_then(|config| config.labels.as_ref());
    let labels_match = labels.is_some_and(|labels| {
        expected_labels
            .iter()
            .filter(|(key, _)| !key.ends_with(".created_at"))
            .all(|(key, value)| labels.get(key) == Some(value))
    });
    labels_match
        && inspected
            .network_settings
            .as_ref()
            .and_then(|settings| settings.networks.as_ref())
            .is_some_and(|networks| {
                networks.contains_key(network_name)
                    && networks.contains_key(upstream_network_name)
                    && !networks.contains_key("bridge")
            })
}

fn proxy_ip_on_network(
    inspected: &bollard::models::ContainerInspectResponse,
    network_name: &str,
) -> Result<String, RuntimeProcessError> {
    inspected
        .network_settings
        .as_ref()
        .and_then(|settings| settings.networks.as_ref())
        .and_then(|networks| networks.get(network_name))
        .and_then(|endpoint| endpoint.ip_address.as_deref())
        .filter(|ip| !ip.is_empty())
        .map(str::to_string)
        .ok_or_else(|| {
            RuntimeProcessError::ExecutionFailed(
                "sandbox proxy has no address on its private network".to_string(),
            )
        })
}

fn proxy_is_ready(inspected: &bollard::models::ContainerInspectResponse) -> bool {
    let Some(state) = inspected.state.as_ref() else {
        return false;
    };
    state.running == Some(true)
        && state.paused != Some(true)
        && state.restarting != Some(true)
        && matches!(
            state.health.as_ref().and_then(|health| health.status),
            Some(HealthStatusEnum::HEALTHY)
        )
}

async fn start_proxy_container(docker: &Docker, name: &str) -> Result<(), RuntimeProcessError> {
    docker
        .start_container(name, None::<StartContainerOptions<String>>)
        .await
        .map_err(|error| {
            RuntimeProcessError::ExecutionFailed(format!("sandbox proxy start failed: {error}"))
        })
}

/// Bounded startup diagnostics for a proxy that never became healthy. Raw
/// proxy output can contain request data, so model-visible errors expose only
/// Docker's container state and exit code.
async fn proxy_failure_detail(docker: &Docker, name: &str) -> String {
    if let Ok(Some(inspected)) = inspect_proxy(docker, name).await
        && let Some(state) = inspected.state
    {
        return format!(
            " (status {:?}, exit code {:?})",
            state.status, state.exit_code
        );
    }
    " (container diagnostics unavailable)".to_string()
}

async fn wait_proxy_ready(docker: &Docker, name: &str) -> Result<(), RuntimeProcessError> {
    for _ in 0..100 {
        let Some(inspected) = inspect_proxy(docker, name).await? else {
            break;
        };
        let state = inspected.state.as_ref();
        if matches!(
            state
                .and_then(|state| state.health.as_ref())
                .and_then(|health| health.status),
            Some(HealthStatusEnum::HEALTHY)
        ) {
            return Ok(());
        }
        if !state.and_then(|state| state.running).unwrap_or(false) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    Err(RuntimeProcessError::ExecutionFailed(
        "sandbox proxy failed readiness".to_string(),
    ))
}
async fn remove_proxy_if_present(docker: &Docker, name: &str) -> Result<(), RuntimeProcessError> {
    match docker
        .remove_container(
            name,
            Some(RemoveContainerOptions {
                force: true,
                ..Default::default()
            }),
        )
        .await
    {
        Ok(()) => Ok(()),
        Err(error) if docker_status(&error) == Some(404) => Ok(()),
        Err(error) => Err(RuntimeProcessError::ExecutionFailed(format!(
            "sandbox proxy removal failed: {error}"
        ))),
    }
}

async fn remove_user_container_if_present(
    docker: &Docker,
    name: &str,
) -> Result<(), RuntimeProcessError> {
    match docker
        .remove_container(
            name,
            Some(RemoveContainerOptions {
                force: true,
                ..Default::default()
            }),
        )
        .await
    {
        Ok(()) => Ok(()),
        Err(error) if docker_status(&error) == Some(404) => Ok(()),
        Err(error) => Err(RuntimeProcessError::ExecutionFailed(format!(
            "sandbox user container removal for egress recycle failed: {error}"
        ))),
    }
}

async fn disconnect_container_if_attached(
    docker: &Docker,
    network_name: &str,
    container_name: &str,
) -> Result<(), RuntimeProcessError> {
    if !container_attached_to_network(docker, container_name, network_name).await? {
        return Ok(());
    }
    docker
        .disconnect_network(
            network_name,
            DisconnectNetworkOptions {
                container: container_name,
                force: true,
            },
        )
        .await
        .map_err(|error| {
            RuntimeProcessError::ExecutionFailed(format!(
                "sandbox user container private-network detach failed: {error}"
            ))
        })
}

async fn container_attached_to_network(
    docker: &Docker,
    container_name: &str,
    network_name: &str,
) -> Result<bool, RuntimeProcessError> {
    match docker
        .inspect_container(container_name, None::<InspectContainerOptions>)
        .await
    {
        Ok(inspected) => Ok(inspected
            .network_settings
            .and_then(|settings| settings.networks)
            .is_some_and(|networks| networks.contains_key(network_name))),
        Err(error) if docker_status(&error) == Some(404) => Ok(false),
        Err(error) => Err(RuntimeProcessError::ExecutionFailed(format!(
            "sandbox user container network inspection failed: {error}"
        ))),
    }
}

async fn remove_network_if_present(docker: &Docker, name: &str) -> Result<(), RuntimeProcessError> {
    match docker.remove_network(name).await {
        Ok(()) => Ok(()),
        Err(error) if docker_status(&error) == Some(404) => Ok(()),
        Err(error) => Err(RuntimeProcessError::ExecutionFailed(format!(
            "sandbox private network removal failed: {error}"
        ))),
    }
}

/// Rejects wildcard host patterns: iron-proxy globs match the apex and
/// subdomains at any depth, which is strictly wider than
/// [`ironclaw_network::target_matches_pattern`]'s exactly-one-label wildcard.
/// A pattern that cannot be represented exactly fails closed.
fn reject_wildcard_targets(policy: &NetworkPolicy) -> Result<(), RuntimeProcessError> {
    if policy
        .allowed_targets
        .iter()
        .any(|target| target.host_pattern.starts_with('*'))
    {
        return Err(RuntimeProcessError::ExecutionFailed(
            "sandbox managed egress proxy cannot represent wildcard host patterns exactly \
             (proxy globs also match the apex and deeper subdomains); enumerate exact \
             hostnames instead"
                .to_string(),
        ));
    }
    Ok(())
}

pub(super) fn proxy_posture(
    proxy_image: &str,
    policy: &NetworkPolicy,
    material_root: &std::path::Path,
) -> Result<String, RuntimeProcessError> {
    let policy_json = serde_json::to_vec(policy).map_err(|error| {
        RuntimeProcessError::ExecutionFailed(format!(
            "sandbox managed egress policy could not be serialized: {error}"
        ))
    })?;
    let mut hasher = Sha256::new();
    hasher.update(b"ironclaw-managed-egress-v3\0");
    hasher.update(proxy_image.as_bytes());
    hasher.update([0]);
    hasher.update(material_root.as_os_str().as_encoded_bytes());
    hasher.update([0]);
    hasher.update(policy_json);
    Ok(hex::encode(hasher.finalize()))
}

pub(super) fn render_proxy_config(
    policy: &NetworkPolicy,
    proxy_ip: &str,
) -> Result<String, RuntimeProcessError> {
    if policy.allowed_targets.is_empty() || !policy.deny_private_ip_ranges {
        return Err(RuntimeProcessError::ExecutionFailed(
            "sandbox managed egress policy is not fail-closed".to_string(),
        ));
    }
    if policy
        .allowed_targets
        .iter()
        .any(|target| target.scheme.is_some() || target.port.is_some())
    {
        return Err(RuntimeProcessError::ExecutionFailed(
            "sandbox managed egress proxy cannot preserve scheme- or port-specific targets"
                .to_string(),
        ));
    }
    if policy.max_egress_bytes.is_some() {
        return Err(RuntimeProcessError::ExecutionFailed(
            "sandbox managed egress proxy cannot preserve request-estimate byte ceilings for opaque TLS tunnels"
                .to_string(),
        ));
    }
    reject_wildcard_targets(policy)?;
    let dns_listen =
        serde_json::to_string(&format!("{proxy_ip}:53")).map_err(proxy_config_error)?;
    let http_listen =
        serde_json::to_string(&format!("{proxy_ip}:80")).map_err(proxy_config_error)?;
    let https_listen =
        serde_json::to_string(&format!("{proxy_ip}:443")).map_err(proxy_config_error)?;
    let tunnel_listen =
        serde_json::to_string(&format!("{proxy_ip}:3128")).map_err(proxy_config_error)?;
    let mut config = format!(
        "dns:\n  listen: {dns_listen}\n  proxy_ip: {}\nproxy:\n  http_listen: {http_listen}\n  https_listen: {https_listen}\n  tunnel_listen: {tunnel_listen}\n  upstream_deny_cidrs:\n",
        serde_json::to_string(proxy_ip).map_err(proxy_config_error)?,
    );
    for cidr in DENIED_UPSTREAM_CIDRS {
        config.push_str("    - ");
        config.push_str(cidr);
        config.push('\n');
    }
    config.push_str("tls:\n  mode: \"sni-only\"\ntransforms:\n  - name: secrets\n    config:\n      secrets:\n        - source:\n            type: file\n            path: \"");
    config.push_str(PROXY_INVOCATION_ID_PATH);
    config.push_str("\"\n            ttl: \"-1ns\"\n            failure_ttl: \"1ms\"\n          rules:\n            - host: \"*\"\n          inject:\n            header: \"X-Ironclaw-Invocation-Id\"\n            require: true\n  - name: annotate\n    config:\n      annotations:\n        - rules:\n            - host: \"*\"\n          headers: [\"X-Ironclaw-Invocation-Id\"]\n  - name: header_allowlist\n    config:\n      headers: [\"Authorization\", \"Content-Type\", \"Accept\", \"User-Agent\", \"Range\", \"If-None-Match\", \"If-Modified-Since\"]\n  - name: allowlist\n    config:\n      domains:\n");
    for target in &policy.allowed_targets {
        let quoted = serde_json::to_string(&target.host_pattern).map_err(proxy_config_error)?;
        config.push_str("        - ");
        config.push_str(&quoted);
        config.push('\n');
    }
    config.push_str("log:\n  level: info\n");
    Ok(config)
}

fn proxy_config_error(error: serde_json::Error) -> RuntimeProcessError {
    RuntimeProcessError::ExecutionFailed(format!("sandbox proxy config rendering failed: {error}"))
}

fn docker_status(error: &bollard::errors::Error) -> Option<u16> {
    match error {
        bollard::errors::Error::DockerResponseServerError { status_code, .. } => Some(*status_code),
        _ => None,
    }
}

fn docker_readonly_bind(
    host_path: &std::path::Path,
    container_path: &str,
) -> Result<String, RuntimeProcessError> {
    let host_path = host_path.to_str().ok_or_else(|| {
        RuntimeProcessError::ExecutionFailed(
            "sandbox managed-egress material path is not valid UTF-8".to_string(),
        )
    })?;
    if host_path.contains(':') || container_path.contains(':') {
        return Err(RuntimeProcessError::ExecutionFailed(
            "sandbox managed-egress bind path contains ':'".to_string(),
        ));
    }
    Ok(format!("{host_path}:{container_path}:ro"))
}

async fn write_atomic_material_file(
    path: &std::path::Path,
    contents: &[u8],
) -> Result<(), RuntimeProcessError> {
    let path = path.to_path_buf();
    let contents = contents.to_vec();
    tokio::task::spawn_blocking(move || {
        use std::io::Write as _;

        let parent = path.parent().ok_or_else(|| {
            RuntimeProcessError::ExecutionFailed(
                "sandbox managed-egress material path has no parent".to_string(),
            )
        })?;
        let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(|error| {
            RuntimeProcessError::ExecutionFailed(format!(
                "sandbox managed-egress temporary material file create failed: {error}"
            ))
        })?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            temporary
                .as_file()
                .set_permissions(std::fs::Permissions::from_mode(0o644))
                .map_err(|error| {
                    RuntimeProcessError::ExecutionFailed(format!(
                        "sandbox managed-egress material permissions failed: {error}"
                    ))
                })?;
        }
        temporary.write_all(&contents).map_err(|error| {
            RuntimeProcessError::ExecutionFailed(format!(
                "sandbox managed-egress material file write failed: {error}"
            ))
        })?;
        temporary.as_file().sync_all().map_err(|error| {
            RuntimeProcessError::ExecutionFailed(format!(
                "sandbox managed-egress material file sync failed: {error}"
            ))
        })?;
        temporary.persist(&path).map_err(|error| {
            RuntimeProcessError::ExecutionFailed(format!(
                "sandbox managed-egress material file commit failed: {}",
                error.error
            ))
        })?;
        #[cfg(unix)]
        std::fs::File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|error| {
                RuntimeProcessError::ExecutionFailed(format!(
                    "sandbox managed-egress material directory sync failed: {error}"
                ))
            })?;
        Ok(())
    })
    .await
    .map_err(|error| {
        RuntimeProcessError::ExecutionFailed(format!(
            "sandbox managed-egress material writer task failed: {error}"
        ))
    })?
}

async fn create_material_directory(
    path: &std::path::Path,
    unix_mode: u32,
) -> Result<(), RuntimeProcessError> {
    tokio::fs::create_dir_all(path).await.map_err(|error| {
        RuntimeProcessError::ExecutionFailed(format!(
            "sandbox managed-egress material directory create failed: {error}"
        ))
    })?;
    #[cfg(unix)]
    tokio::fs::set_permissions(path, {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::Permissions::from_mode(unix_mode)
    })
    .await
    .map_err(|error| {
        RuntimeProcessError::ExecutionFailed(format!(
            "sandbox managed-egress material directory permissions failed: {error}"
        ))
    })?;
    #[cfg(not(unix))]
    let _ = unix_mode;
    Ok(())
}
async fn enforce_audit_budget(
    audit_dir: &std::path::Path,
    budget_bytes: u64,
) -> Result<(), RuntimeProcessError> {
    let mut entries = Vec::new();
    let mut dir = tokio::fs::read_dir(audit_dir).await.map_err(|error| {
        RuntimeProcessError::ExecutionFailed(format!(
            "sandbox proxy audit directory scan failed: {error}"
        ))
    })?;
    while let Some(entry) = dir.next_entry().await.map_err(|error| {
        RuntimeProcessError::ExecutionFailed(format!(
            "sandbox proxy audit directory scan failed: {error}"
        ))
    })? {
        let metadata = entry.metadata().await.map_err(|error| {
            RuntimeProcessError::ExecutionFailed(format!(
                "sandbox proxy audit entry metadata failed: {error}"
            ))
        })?;
        if !metadata.is_file() {
            continue;
        }
        let modified = metadata.modified().map_err(|error| {
            RuntimeProcessError::ExecutionFailed(format!(
                "sandbox proxy audit entry modification time failed: {error}"
            ))
        })?;
        entries.push((modified, metadata.len(), entry.path()));
    }
    let mut total: u64 = entries.iter().map(|(_, len, _)| len).sum();
    if total <= budget_bytes {
        return Ok(());
    }
    entries.sort_by_key(|(modified, _, _)| *modified);
    for (_, len, path) in entries {
        if total <= budget_bytes {
            break;
        }
        tokio::fs::remove_file(&path).await.map_err(|error| {
            RuntimeProcessError::ExecutionFailed(format!(
                "sandbox proxy audit budget enforcement failed: {error}"
            ))
        })?;
        total = total.saturating_sub(len);
    }
    Ok(())
}

async fn remove_material_file(path: &std::path::Path) -> Result<(), RuntimeProcessError> {
    match tokio::fs::remove_file(path).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(RuntimeProcessError::ExecutionFailed(format!(
            "sandbox managed-egress material file cleanup failed: {error}"
        ))),
    }
}
async fn remove_material_directory(path: &std::path::Path) -> Result<(), RuntimeProcessError> {
    match tokio::fs::remove_dir_all(path).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(RuntimeProcessError::ExecutionFailed(format!(
            "sandbox managed-egress material cleanup failed: {error}"
        ))),
    }
}
#[cfg(test)]
mod tests {
    use ironclaw_common::env_helpers::{lock_env, remove_runtime_env, set_runtime_env};
    use ironclaw_host_api::action::{NetworkScheme, NetworkTargetPattern};

    use super::*;

    fn policy() -> NetworkPolicy {
        NetworkPolicy {
            allowed_targets: vec![
                NetworkTargetPattern {
                    scheme: None,
                    host_pattern: "github.com".to_string(),
                    port: None,
                },
                NetworkTargetPattern {
                    scheme: None,
                    host_pattern: "objects.githubusercontent.com".to_string(),
                    port: None,
                },
            ],
            deny_private_ip_ranges: true,
            max_egress_bytes: None,
        }
    }

    #[test]
    fn renders_default_deny_allowlist_without_management_api() {
        let rendered = render_proxy_config(&policy(), "172.28.10.2").unwrap();
        assert!(rendered.contains("proxy_ip: \"172.28.10.2\""));
        assert!(rendered.contains("listen: \"172.28.10.2:53\""));
        assert!(rendered.contains("http_listen: \"172.28.10.2:80\""));
        assert!(rendered.contains("https_listen: \"172.28.10.2:443\""));
        assert!(rendered.contains("tunnel_listen: \"172.28.10.2:3128\""));
        assert!(rendered.contains("- \"github.com\""));
        assert!(rendered.contains("- \"objects.githubusercontent.com\""));
        assert!(rendered.contains("upstream_deny_cidrs:"));
        for cidr in [
            "10.0.0.0/8",
            "127.0.0.0/8",
            "169.254.0.0/16",
            "192.0.2.0/24",
            "198.51.100.0/24",
            "203.0.113.0/24",
            "::ffff:0:0/96",
            "2001:db8::/32",
            "fc00::/7",
        ] {
            assert!(rendered.contains(cidr), "missing denied CIDR {cidr}");
        }
        assert!(!rendered.contains("management:"));
        assert!(rendered.contains(PROXY_INVOCATION_ID_PATH));
        assert!(rendered.contains("X-Ironclaw-Invocation-Id"));
        assert!(
            rendered.contains("ttl: \"-1ns\""),
            "attribution source must bypass the proxy cache on every request"
        );
        assert!(rendered.contains("name: header_allowlist"));
    }

    #[test]
    fn proxy_image_override_requires_a_full_sha256_digest() {
        let _guard = lock_env();
        remove_runtime_env(PROXY_IMAGE_ENV);
        assert_eq!(configured_proxy_image().unwrap(), DEFAULT_PROXY_IMAGE);

        for rejected in [
            "ironsh/iron-proxy:latest",
            "ironsh/iron-proxy@sha256:deadbeef",
            "ironsh/iron-proxy@sha256:gggggggggggggggggggggggggggggggggggggggggggggggggggggggggggggggg",
            "ironsh/iron-proxy@sha512:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        ] {
            set_runtime_env(PROXY_IMAGE_ENV, rejected);
            assert!(
                configured_proxy_image().is_err(),
                "proxy image override must reject {rejected}"
            );
        }

        let accepted = "ironsh/iron-proxy@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        set_runtime_env(PROXY_IMAGE_ENV, accepted);
        assert_eq!(configured_proxy_image().unwrap(), accepted);
        remove_runtime_env(PROXY_IMAGE_ENV);
    }

    #[test]
    fn renderer_rejects_non_fail_closed_or_unrepresentable_policy() {
        let mut open = policy();
        open.deny_private_ip_ranges = false;
        assert!(render_proxy_config(&open, "172.28.10.2").is_err());

        let mut byte_limited = policy();
        byte_limited.max_egress_bytes = Some(1024);
        assert!(render_proxy_config(&byte_limited, "172.28.10.2").is_err());

        open.deny_private_ip_ranges = true;
        open.allowed_targets.clear();
        assert!(render_proxy_config(&open, "172.28.10.2").is_err());

        let mut constrained = policy();
        constrained.allowed_targets[0].scheme = Some(NetworkScheme::Https);
        assert!(render_proxy_config(&constrained, "172.28.10.2").is_err());
        constrained.allowed_targets[0].scheme = None;
        constrained.allowed_targets[0].port = Some(443);
        assert!(render_proxy_config(&constrained, "172.28.10.2").is_err());

        let mut wildcard = policy();
        wildcard.allowed_targets[1].host_pattern = "*.githubusercontent.com".to_string();
        let error = render_proxy_config(&wildcard, "172.28.10.2").unwrap_err();
        assert!(
            error.to_string().contains("wildcard host patterns"),
            "wildcard must fail closed: {error}"
        );
        assert!(
            ManagedEgressConfig::from_policy(wildcard, PathBuf::from("/tmp/egress")).is_err(),
            "profile construction must reject wildcard targets"
        );
    }

    #[test]
    fn posture_binds_image_policy_and_material_root() {
        let image = "sha256:proxy-image";
        let base = proxy_posture(image, &policy(), std::path::Path::new("/a")).unwrap();
        assert_eq!(
            base,
            proxy_posture(image, &policy(), std::path::Path::new("/a")).unwrap()
        );
        assert_ne!(
            base,
            proxy_posture(image, &policy(), std::path::Path::new("/b")).unwrap()
        );
    }

    #[tokio::test]
    async fn atomic_material_writer_replaces_a_proxy_readable_marker() {
        let directory = tempfile::tempdir().unwrap();
        let marker = directory.path().join("invocation-id");

        write_atomic_material_file(&marker, b"first").await.unwrap();
        write_atomic_material_file(&marker, b"second")
            .await
            .unwrap();

        assert_eq!(tokio::fs::read(&marker).await.unwrap(), b"second");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;

            let mode = tokio::fs::metadata(marker)
                .await
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o644);
        }
    }

    #[tokio::test]
    async fn material_directories_apply_the_requested_proxy_boundary_mode() {
        let directory = tempfile::tempdir().unwrap();
        let proxy_material = directory.path().join("proxy");
        create_material_directory(&proxy_material, 0o711)
            .await
            .unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;

            let mode = tokio::fs::metadata(proxy_material)
                .await
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o711);
        }
    }

    #[test]
    fn adoption_requires_a_running_healthy_unpaused_proxy() {
        let inspected = |status, paused| bollard::models::ContainerInspectResponse {
            state: Some(bollard::models::ContainerState {
                running: Some(true),
                paused: Some(paused),
                restarting: Some(false),
                health: Some(bollard::models::Health {
                    status: Some(status),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        };

        assert!(proxy_is_ready(&inspected(HealthStatusEnum::HEALTHY, false)));
        assert!(!proxy_is_ready(&inspected(
            HealthStatusEnum::UNHEALTHY,
            false
        )));
        assert!(!proxy_is_ready(&inspected(HealthStatusEnum::HEALTHY, true)));
    }

    #[test]
    fn address_pool_exhaustion_has_operator_action() {
        let error = bollard::errors::Error::DockerResponseServerError {
            status_code: 500,
            message:
                "could not find an available, non-overlapping IPv4 address pool among the defaults"
                    .to_string(),
        };

        let RuntimeProcessError::ExecutionFailed(reason) = network_create_error(error) else {
            panic!("network creation must be model-visible");
        };
        assert!(reason.contains("default-address-pools"));
        assert!(reason.contains("/etc/docker/daemon.json"));
    }

    #[tokio::test]
    async fn audit_budget_removes_oldest_files_first() {
        let directory = tempfile::tempdir().unwrap();
        for (name, age_secs) in [("old.log", 300), ("mid.log", 200), ("new.log", 100)] {
            let path = directory.path().join(name);
            std::fs::write(&path, vec![0u8; 100]).unwrap();
            let mtime = std::time::SystemTime::now() - Duration::from_secs(age_secs);
            let file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
            file.set_times(std::fs::FileTimes::new().set_modified(mtime))
                .unwrap();
        }

        enforce_audit_budget(directory.path(), 250).await.unwrap();

        assert!(!directory.path().join("old.log").exists());
        assert!(directory.path().join("mid.log").exists());
        assert!(directory.path().join("new.log").exists());

        enforce_audit_budget(directory.path(), 250).await.unwrap();
        assert!(directory.path().join("mid.log").exists());
        assert!(directory.path().join("new.log").exists());
    }
}
