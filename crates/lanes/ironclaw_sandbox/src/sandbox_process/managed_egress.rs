use std::{
    collections::{BTreeMap, HashMap, HashSet},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};

use super::{
    ca::SandboxCertificateAuthority, registry, user_key::RebornSandboxUserKey, worker_spec,
};
use bollard::{
    Docker,
    container::{
        Config, CreateContainerOptions, InspectContainerOptions, ListContainersOptions,
        LogsOptions, RemoveContainerOptions, RestartContainerOptions, StartContainerOptions,
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
    process::{RuntimeProcessError, SandboxCommandCredential},
};
use secrecy::ExposeSecret;
use serde::Serialize;
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;
pub(super) const PROXY_LABEL_PREFIX: &str = "ironclaw.proxy";
pub(super) const NETWORK_LABEL_PREFIX: &str = "ironclaw.network";
pub(super) const DEFAULT_PROXY_IMAGE: &str =
    "ironsh/iron-proxy@sha256:c4628019c24f4cc8d77564a26b7c9cedb00accee6f93d06270e85fb8f9c6a7da";
pub(super) const PROXY_IMAGE_ENV: &str = "IRONCLAW_REBORN_SANDBOX_PROXY_IMAGE";
const PROXY_CONFIG_PATH: &str = "/run/ironclaw-proxy/proxy.yaml";
const PROXY_MATERIAL_ROOT: &str = "/run/ironclaw-proxy";
const PROXY_INVOCATION_ID_PATH: &str = "/run/ironclaw-proxy/invocation-id";
const PROXY_CREDENTIAL_BUNDLE_PATH: &str = "/run/ironclaw-proxy/credentials.json";
const PROXY_CA_CERT_PATH: &str = "/run/ironclaw-proxy/ca.crt";
const PROXY_CA_KEY_PATH: &str = "/run/ironclaw-proxy/ca.key";
pub(super) const USER_PROXY_CA_PATH: &str = "/run/ironclaw/egress-ca.crt";
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
    ca: Arc<SandboxCertificateAuthority>,
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
        let ca = Arc::new(SandboxCertificateAuthority::generate()?);
        Ok(Self {
            proxy_image,
            policy,
            material_root,
            ca,
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
    ca: Arc<SandboxCertificateAuthority>,
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
    pub(super) ca_cert_path: PathBuf,
    invocation_id_path: PathBuf,
}

#[cfg(test)]
impl ManagedEgressBundle {
    pub(super) fn test_bundle(material_root: &std::path::Path) -> Self {
        Self {
            network_name: "test-network".to_string(),
            proxy_ip: "172.28.10.2".to_string(),
            proxy_host: "test-proxy".to_string(),
            posture: "test-posture".to_string(),
            ca_cert_path: material_root.join("ca.crt"),
            invocation_id_path: material_root.join("invocation-id"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ManagedNetworkStatus {
    Missing,
    Compatible,
    Incompatible,
}

#[cfg(test)]
impl ManagedEgressRuntime {
    pub(super) fn test_runtime(
        policy: NetworkPolicy,
        material_root: PathBuf,
    ) -> Result<Arc<Self>, RuntimeProcessError> {
        Ok(Arc::new(Self {
            proxy_image: "sha256:test-proxy".to_string(),
            policy,
            posture: "test-posture".to_string(),
            ca: Arc::new(SandboxCertificateAuthority::generate()?),
            material_root,
            upstream_gate: tokio::sync::Mutex::new(()),
        }))
    }
}

impl ManagedEgressRuntime {
    pub(super) async fn connect(
        docker: &Docker,
        config: ManagedEgressConfig,
    ) -> Result<Arc<Self>, RuntimeProcessError> {
        let proxy_image = resolve_proxy_image(docker, &config.proxy_image).await?;
        let material_root = config.material_root;
        create_material_directory(&material_root, 0o711).await?;
        let posture = proxy_posture(
            &proxy_image,
            &config.policy,
            config.ca.root_certificate_pem().as_bytes(),
        )?;
        Ok(Arc::new(Self {
            proxy_image,
            policy: config.policy,
            posture,
            material_root,
            ca: config.ca,
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
        let ca_cert_path = proxy_material_root.join("ca.crt");
        let ca_key_path = proxy_material_root.join("ca.key");
        write_atomic_material_file_if_changed(
            &ca_cert_path,
            self.ca.root_certificate_pem().as_bytes(),
            0o644,
        )
        .await?;
        let ca_key = self.ca.proxy_private_key_pem();
        write_atomic_material_file_if_changed(
            &ca_key_path,
            ca_key.expose_secret().as_bytes(),
            0o600,
        )
        .await?;
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
            ca_cert_path,
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
    pub(super) async fn configure_credentials(
        &self,
        docker: &Docker,
        bundle: &ManagedEgressBundle,
        credentials: &[SandboxCommandCredential],
    ) -> Result<(), RuntimeProcessError> {
        self.configure_credentials_with_restart(bundle, credentials, || {
            restart_proxy_container(docker, &bundle.proxy_host)
        })
        .await
    }

    async fn configure_credentials_with_restart<F, Fut>(
        &self,
        bundle: &ManagedEgressBundle,
        credentials: &[SandboxCommandCredential],
        restart: F,
    ) -> Result<(), RuntimeProcessError>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<(), RuntimeProcessError>>,
    {
        let material_root = bundle.ca_cert_path.parent().ok_or_else(|| {
            RuntimeProcessError::ExecutionFailed(
                "sandbox proxy credential material path has no parent".to_string(),
            )
        })?;
        let configured = async {
            remove_credential_material(material_root).await?;
            let mut credential_bundle = BTreeMap::new();
            let mut placeholder_values = HashSet::with_capacity(credentials.len());
            let mut rendered = Vec::with_capacity(credentials.len());
            for credential in credentials {
                if !self.policy.allowed_targets.iter().any(|target| {
                    target
                        .host_pattern
                        .eq_ignore_ascii_case(&credential.approved_host)
                }) {
                    return Err(RuntimeProcessError::ExecutionFailed(
                        "sandbox credential host is outside the approved network policy"
                            .to_string(),
                    ));
                }
                let header_prefix = credential.header_prefix.as_deref().unwrap_or_default();
                let placeholder_value = format!("{header_prefix}{}", credential.placeholder);
                if credential.placeholder.trim().is_empty()
                    || credential.header_name.trim().is_empty()
                    || placeholder_value.trim().is_empty()
                    || credential.placeholder == credential.expose_secret()
                {
                    return Err(RuntimeProcessError::ExecutionFailed(
                        "sandbox credential replacement rule is invalid".to_string(),
                    ));
                }
                if !placeholder_values.insert(placeholder_value.clone()) {
                    return Err(RuntimeProcessError::ExecutionFailed(
                        "sandbox credential bundle contains a duplicate placeholder".to_string(),
                    ));
                }
                let bundle_key = credential.credential_key.as_str().to_string();
                let authorized_value =
                    Zeroizing::new(format!("{header_prefix}{}", credential.expose_secret()));
                if credential_bundle
                    .insert(bundle_key.clone(), authorized_value)
                    .is_some()
                {
                    return Err(RuntimeProcessError::ExecutionFailed(
                        "sandbox credential bundle contains a duplicate handle".to_string(),
                    ));
                }
                rendered.push(ProxyCredentialRule {
                    bundle_key,
                    approved_host: credential.approved_host.clone(),
                    target_header: credential.header_name.clone(),
                    placeholder_value,
                });
            }
            let config =
                render_proxy_config_inner(&self.policy, &bundle.proxy_ip, true, &rendered)?;
            let bundle_json =
                Zeroizing::new(serde_json::to_vec(&credential_bundle).map_err(|error| {
                    RuntimeProcessError::ExecutionFailed(format!(
                        "sandbox proxy credential bundle rendering failed: {error}"
                    ))
                })?);
            write_proxy_credential_material(material_root, &bundle_json, config.as_bytes()).await?;
            restart().await
        }
        .await;
        if let Err(configure_error) = configured {
            return match remove_credential_material(material_root).await {
                Ok(()) => Err(configure_error),
                Err(cleanup_error) => Err(RuntimeProcessError::ExecutionFailed(format!(
                    "{configure_error}; sandbox proxy credential rollback failed: {cleanup_error}"
                ))),
            };
        }
        Ok(())
    }

    pub(super) async fn clear_credentials(
        &self,
        docker: &Docker,
        bundle: &ManagedEgressBundle,
    ) -> Result<(), RuntimeProcessError> {
        let material_root = bundle.ca_cert_path.parent().ok_or_else(|| {
            RuntimeProcessError::ExecutionFailed(
                "sandbox proxy credential material path has no parent".to_string(),
            )
        })?;
        // Delete the bundle before asking Docker to reload. A failed or hung
        // restart must not extend the lifetime of credential material on disk.
        // If deletion itself fails, still reload the uncredentialed config so
        // the proxy stops referencing the stranded file.
        let cleanup_result = remove_credential_material(material_root).await;
        let clear_result = async {
            let config = render_proxy_config_with_ca(&self.policy, &bundle.proxy_ip)?;
            write_bind_mounted_proxy_config(&material_root.join("proxy.yaml"), config.as_bytes())
                .await?;
            restart_proxy_container(docker, &bundle.proxy_host).await
        }
        .await;
        match (clear_result, cleanup_result) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(clear_error), Ok(())) => Err(clear_error),
            (Ok(()), Err(cleanup_error)) => Err(cleanup_error),
            (Err(clear_error), Err(cleanup_error)) => Err(RuntimeProcessError::ExecutionFailed(
                format!("{clear_error}; sandbox proxy credential cleanup failed: {cleanup_error}"),
            )),
        }
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
                        "address=$(sed -n 's/^  tunnel_listen: //p' {PROXY_CONFIG_PATH} | tr -d '\"'); host=${{address%:*}}; port=${{address##*:}}; nc -z \"$host\" \"$port\""
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
                // The proxy runs as container root after dropping every
                // capability. Keep only low-port binding plus read/search
                // override so it can read 0600 host-owned CA/token files
                // across Docker's host-UID boundary; the only host bind is
                // its own read-only per-user material directory.
                cap_add: Some(vec![
                    "NET_BIND_SERVICE".to_string(),
                    "DAC_READ_SEARCH".to_string(),
                ]),
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
        let proxy_config = render_proxy_config_with_ca(&self.policy, &proxy_ip)?;
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
    pub(super) fn user_ca_bind(
        &self,
        bundle: &ManagedEgressBundle,
    ) -> Result<String, RuntimeProcessError> {
        docker_readonly_bind(&bundle.ca_cert_path, USER_PROXY_CA_PATH)
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
            ("SSL_CERT_FILE", USER_PROXY_CA_PATH),
            ("NODE_EXTRA_CA_CERTS", USER_PROXY_CA_PATH),
            ("REQUESTS_CA_BUNDLE", USER_PROXY_CA_PATH),
            ("CURL_CA_BUNDLE", USER_PROXY_CA_PATH),
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

    /// Stops the idle proxy without invalidating the persistent worker's CA
    /// file bind mount. Docker Desktop pins file binds to the source inode, so
    /// the CA material must outlive every suspension of the user container.
    pub(super) async fn suspend_bundle(
        &self,
        docker: &Docker,
        key: &RebornSandboxUserKey,
    ) -> Result<(), RuntimeProcessError> {
        let proxy_name = key.proxy_name();
        self.preserve_proxy_audit(docker, &proxy_name, &proxy_name)
            .await?;
        remove_proxy_if_present(docker, &proxy_name).await?;
        remove_credential_material(&self.material_root.join(proxy_name)).await
    }

    pub(super) async fn rollback_provisioned_bundle(
        &self,
        docker: &Docker,
        key: &RebornSandboxUserKey,
        user_container_name: &str,
    ) -> Result<(), RuntimeProcessError> {
        self.remove_proxy(docker, key).await?;
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
        self.remove_proxy(docker, key).await?;
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

async fn restart_proxy_container(docker: &Docker, name: &str) -> Result<(), RuntimeProcessError> {
    docker
        .restart_container(name, Some(RestartContainerOptions { t: 10 }))
        .await
        .map_err(|error| {
            RuntimeProcessError::ExecutionFailed(format!(
                "sandbox proxy credential reload failed: {error}"
            ))
        })?;
    if let Err(error) = wait_proxy_ready(docker, name).await {
        let detail = proxy_failure_detail(docker, name).await;
        return Err(RuntimeProcessError::ExecutionFailed(format!(
            "{error}{detail}"
        )));
    }
    Ok(())
}

async fn remove_credential_material(
    material_root: &std::path::Path,
) -> Result<(), RuntimeProcessError> {
    let mut entries = tokio::fs::read_dir(material_root).await.map_err(|error| {
        RuntimeProcessError::ExecutionFailed(format!(
            "sandbox proxy credential material scan failed: {error}"
        ))
    })?;
    while let Some(entry) = entries.next_entry().await.map_err(|error| {
        RuntimeProcessError::ExecutionFailed(format!(
            "sandbox proxy credential material scan failed: {error}"
        ))
    })? {
        let filename = entry.file_name();
        if filename.to_str().is_some_and(|name| {
            name == "credentials.json"
                || (name.starts_with("credential-") && name.ends_with(".secret"))
        }) {
            tokio::fs::remove_file(entry.path())
                .await
                .map_err(|error| {
                    RuntimeProcessError::ExecutionFailed(format!(
                        "sandbox proxy credential material cleanup failed: {error}"
                    ))
                })?;
        }
    }
    Ok(())
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
    ca_certificate: &[u8],
) -> Result<String, RuntimeProcessError> {
    let policy_json = serde_json::to_vec(policy).map_err(|error| {
        RuntimeProcessError::ExecutionFailed(format!(
            "sandbox managed egress policy could not be serialized: {error}"
        ))
    })?;
    let mut hasher = Sha256::new();
    hasher.update(b"ironclaw-managed-egress-v5\0");
    hasher.update(proxy_image.as_bytes());
    hasher.update([0]);
    hasher.update(ca_certificate);
    hasher.update([0]);
    hasher.update(policy_json);
    Ok(hex::encode(hasher.finalize()))
}

#[derive(Debug, Serialize)]
struct ProxyConfig {
    dns: ProxyDnsConfig,
    proxy: ProxyListenerConfig,
    tls: ProxyTlsConfig,
    transforms: Vec<ProxyTransform>,
    log: ProxyLogConfig,
}

#[derive(Debug, Serialize)]
struct ProxyDnsConfig {
    listen: String,
    proxy_ip: String,
}

#[derive(Debug, Serialize)]
struct ProxyListenerConfig {
    http_listen: String,
    https_listen: String,
    tunnel_listen: String,
    upstream_deny_cidrs: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
enum ProxyTlsConfig {
    SniOnly { mode: String },
    Mitm { ca_cert: String, ca_key: String },
}

#[derive(Debug, Serialize)]
#[serde(tag = "name", content = "config", rename_all = "snake_case")]
enum ProxyTransform {
    Allowlist(ProxyAllowlistTransform),
    HeaderAllowlist(ProxyHeaderAllowlistTransform),
    Secrets(ProxySecretsTransform),
    Annotate(ProxyAnnotateTransform),
}

#[derive(Debug, Serialize)]
struct ProxyAllowlistTransform {
    domains: Vec<String>,
}

#[derive(Debug, Serialize)]
struct ProxyHeaderAllowlistTransform {
    headers: Vec<String>,
}

#[derive(Debug, Serialize)]
struct ProxySecretsTransform {
    secrets: Vec<ProxySecret>,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
enum ProxySecret {
    Inject {
        source: ProxySecretSource,
        rules: Vec<ProxyRule>,
        inject: ProxyInjectOperation,
    },
    Replace {
        source: ProxySecretSource,
        replace: ProxyReplaceOperation,
        rules: Vec<ProxyRule>,
    },
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ProxySecretSource {
    File {
        path: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        json_key: Option<String>,
        ttl: String,
        failure_ttl: String,
    },
}

#[derive(Debug, Serialize)]
struct ProxyRule {
    host: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    methods: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    paths: Vec<String>,
}

#[derive(Debug, Serialize)]
struct ProxyInjectOperation {
    header: String,
    require: bool,
}

#[derive(Debug, Serialize)]
struct ProxyReplaceOperation {
    proxy_value: String,
    match_headers: Vec<String>,
    require: bool,
}

#[derive(Debug, Serialize)]
struct ProxyAnnotateTransform {
    annotations: Vec<ProxyAnnotation>,
}

#[derive(Debug, Serialize)]
struct ProxyAnnotation {
    rules: Vec<ProxyRule>,
    headers: Vec<String>,
}

#[derive(Debug, Serialize)]
struct ProxyLogConfig {
    level: String,
}

struct ProxyCredentialRule {
    bundle_key: String,
    approved_host: String,
    target_header: String,
    placeholder_value: String,
}

pub(super) fn render_proxy_config(
    policy: &NetworkPolicy,
    proxy_ip: &str,
) -> Result<String, RuntimeProcessError> {
    render_proxy_config_inner(policy, proxy_ip, false, &[])
}

fn render_proxy_config_with_ca(
    policy: &NetworkPolicy,
    proxy_ip: &str,
) -> Result<String, RuntimeProcessError> {
    render_proxy_config_inner(policy, proxy_ip, true, &[])
}

fn render_proxy_config_inner(
    policy: &NetworkPolicy,
    proxy_ip: &str,
    intercept_tls: bool,
    credentials: &[ProxyCredentialRule],
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

    let mut headers = [
        "Authorization",
        "Content-Type",
        "Accept",
        "User-Agent",
        "Range",
        "If-None-Match",
        "If-Modified-Since",
    ]
    .into_iter()
    .map(str::to_string)
    .collect::<Vec<_>>();
    let mut normalized_headers = headers
        .iter()
        .map(|header| header.to_ascii_lowercase())
        .collect::<HashSet<_>>();
    for credential in credentials {
        if normalized_headers.insert(credential.target_header.to_ascii_lowercase()) {
            headers.push(credential.target_header.clone());
        }
    }

    let wildcard_rule = || ProxyRule {
        host: "*".to_string(),
        methods: Vec::new(),
        paths: Vec::new(),
    };
    let mut secrets = Vec::with_capacity(credentials.len() + 1);
    secrets.push(ProxySecret::Inject {
        source: ProxySecretSource::File {
            path: PROXY_INVOCATION_ID_PATH.to_string(),
            json_key: None,
            ttl: "-1ns".to_string(),
            failure_ttl: "1ms".to_string(),
        },
        rules: vec![wildcard_rule()],
        inject: ProxyInjectOperation {
            header: "X-Ironclaw-Invocation-Id".to_string(),
            require: true,
        },
    });
    let methods = ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"];
    for credential in credentials {
        secrets.push(ProxySecret::Replace {
            source: ProxySecretSource::File {
                path: PROXY_CREDENTIAL_BUNDLE_PATH.to_string(),
                json_key: Some(credential.bundle_key.clone()),
                ttl: "-1ns".to_string(),
                failure_ttl: "1ms".to_string(),
            },
            replace: ProxyReplaceOperation {
                proxy_value: credential.placeholder_value.clone(),
                match_headers: vec![credential.target_header.clone()],
                require: true,
            },
            rules: vec![ProxyRule {
                host: credential.approved_host.clone(),
                methods: methods.into_iter().map(str::to_string).collect(),
                paths: vec!["/*".to_string()],
            }],
        });
    }

    let config = ProxyConfig {
        dns: ProxyDnsConfig {
            listen: format!("{proxy_ip}:53"),
            proxy_ip: proxy_ip.to_string(),
        },
        proxy: ProxyListenerConfig {
            http_listen: format!("{proxy_ip}:80"),
            https_listen: format!("{proxy_ip}:443"),
            tunnel_listen: format!("{proxy_ip}:3128"),
            upstream_deny_cidrs: DENIED_UPSTREAM_CIDRS
                .iter()
                .map(|cidr| (*cidr).to_string())
                .collect(),
        },
        tls: if intercept_tls {
            ProxyTlsConfig::Mitm {
                ca_cert: PROXY_CA_CERT_PATH.to_string(),
                ca_key: PROXY_CA_KEY_PATH.to_string(),
            }
        } else {
            ProxyTlsConfig::SniOnly {
                mode: "sni-only".to_string(),
            }
        },
        transforms: vec![
            ProxyTransform::Allowlist(ProxyAllowlistTransform {
                domains: policy
                    .allowed_targets
                    .iter()
                    .map(|target| target.host_pattern.clone())
                    .collect(),
            }),
            ProxyTransform::HeaderAllowlist(ProxyHeaderAllowlistTransform { headers }),
            ProxyTransform::Secrets(ProxySecretsTransform { secrets }),
            ProxyTransform::Annotate(ProxyAnnotateTransform {
                annotations: vec![ProxyAnnotation {
                    rules: vec![wildcard_rule()],
                    headers: vec!["X-Ironclaw-Invocation-Id".to_string()],
                }],
            }),
        ],
        log: ProxyLogConfig {
            level: "info".to_string(),
        },
    };
    serde_yaml::to_string(&config).map_err(proxy_config_error)
}

fn proxy_config_error(error: serde_yaml::Error) -> RuntimeProcessError {
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
    write_atomic_material_file_with_mode(path.to_path_buf(), contents.to_vec(), 0o644).await
}

async fn write_atomic_private_material_file(
    path: &std::path::Path,
    contents: &[u8],
) -> Result<(), RuntimeProcessError> {
    write_atomic_material_file_with_mode(
        path.to_path_buf(),
        Zeroizing::new(contents.to_vec()),
        0o600,
    )
    .await
}

/// Keeps an existing file bind mount valid when the material has not changed.
///
/// Docker Desktop pins file bind mounts to the source inode. Replacing an
/// unchanged CA file makes that mount disappear from a running user container.
/// A changed CA still uses the atomic writer; its posture change recreates the
/// dependent containers before they execute another command.
async fn write_atomic_material_file_if_changed(
    path: &std::path::Path,
    contents: &[u8],
    mode: u32,
) -> Result<(), RuntimeProcessError> {
    match tokio::fs::read(path).await {
        Ok(existing) => {
            let existing = Zeroizing::new(existing);
            if existing.as_slice() == contents {
                #[cfg(unix)]
                tokio::fs::set_permissions(path, {
                    use std::os::unix::fs::PermissionsExt as _;
                    std::fs::Permissions::from_mode(mode)
                })
                .await
                .map_err(|error| {
                    RuntimeProcessError::ExecutionFailed(format!(
                        "sandbox managed-egress material permissions failed: {error}"
                    ))
                })?;
                #[cfg(not(unix))]
                let _ = mode;
                return Ok(());
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(RuntimeProcessError::ExecutionFailed(format!(
                "sandbox managed-egress material read failed: {error}"
            )));
        }
    }
    write_atomic_material_file_with_mode(
        path.to_path_buf(),
        Zeroizing::new(contents.to_vec()),
        mode,
    )
    .await
}

/// Rewrites the existing proxy config without replacing its inode.
///
/// Docker Desktop can retain a stale directory-bind dentry across an atomic
/// rename. The proxy reads this file only at startup, and the lifecycle gate
/// serializes updates with restarts, so an in-place write followed by `sync_all`
/// gives the container one stable mounted file to reopen.
async fn write_bind_mounted_proxy_config(
    path: &std::path::Path,
    contents: &[u8],
) -> Result<(), RuntimeProcessError> {
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(path)
        .await
        .map_err(|error| {
            RuntimeProcessError::ExecutionFailed(format!(
                "sandbox proxy config open failed: {error}"
            ))
        })?;
    tokio::io::AsyncWriteExt::write_all(&mut file, contents)
        .await
        .map_err(|error| {
            RuntimeProcessError::ExecutionFailed(format!(
                "sandbox proxy config write failed: {error}"
            ))
        })?;
    file.sync_all().await.map_err(|error| {
        RuntimeProcessError::ExecutionFailed(format!("sandbox proxy config sync failed: {error}"))
    })
}

async fn write_proxy_credential_material(
    material_root: &std::path::Path,
    bundle_json: &[u8],
    proxy_config: &[u8],
) -> Result<(), RuntimeProcessError> {
    let result = async {
        write_atomic_private_material_file(&material_root.join("credentials.json"), bundle_json)
            .await?;
        write_bind_mounted_proxy_config(&material_root.join("proxy.yaml"), proxy_config).await
    }
    .await;
    if let Err(write_error) = result {
        return match remove_credential_material(material_root).await {
            Ok(()) => Err(write_error),
            Err(cleanup_error) => Err(RuntimeProcessError::ExecutionFailed(format!(
                "{write_error}; sandbox proxy credential material rollback failed: {cleanup_error}"
            ))),
        };
    }
    Ok(())
}

/// Apply POSIX permission bits to a freshly created material file.
///
/// Windows has no POSIX permission model, so the non-unix build is a no-op.
/// It still *takes* `mode`, which is the point: a `#[cfg(unix)]` block around
/// the call site left the parameter unused on Windows and `-D warnings`
/// rejected the build. Consuming it on every platform keeps one signature and
/// needs no lint suppression.
#[cfg(unix)]
fn apply_material_mode(file: &std::fs::File, mode: u32) -> Result<(), RuntimeProcessError> {
    use std::os::unix::fs::PermissionsExt as _;
    file.set_permissions(std::fs::Permissions::from_mode(mode))
        .map_err(|error| {
            RuntimeProcessError::ExecutionFailed(format!(
                "sandbox managed-egress material permissions failed: {error}"
            ))
        })
}

#[cfg(not(unix))]
fn apply_material_mode(_file: &std::fs::File, _mode: u32) -> Result<(), RuntimeProcessError> {
    Ok(())
}

async fn write_atomic_material_file_with_mode<C>(
    path: PathBuf,
    contents: C,
    mode: u32,
) -> Result<(), RuntimeProcessError>
where
    C: AsRef<[u8]> + Send + 'static,
{
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
        apply_material_mode(temporary.as_file(), mode)?;
        temporary.write_all(contents.as_ref()).map_err(|error| {
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

    /// The managed-egress writer hands the credential material's permission
    /// bits to `apply_material_mode` on every platform. Before this existed
    /// the call site was a `#[cfg(unix)]` block, which left `mode` unused on
    /// Windows and made `-D warnings` fail the whole build there.
    ///
    /// Asserting the mode actually lands (rather than that the call merely
    /// returns `Ok`) is the point: material files carry proxy credentials, so
    /// a writer that silently stopped restricting them would be a real leak,
    /// not a style regression.
    #[cfg(unix)]
    #[test]
    fn apply_material_mode_restricts_the_file_to_its_owner() {
        use std::os::unix::fs::PermissionsExt as _;

        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("material");
        let file = std::fs::File::create(&path).expect("create material file");

        apply_material_mode(&file, 0o600).expect("apply mode");

        let mode = std::fs::metadata(&path)
            .expect("material metadata")
            .permissions()
            .mode();
        assert_eq!(
            mode & 0o777,
            0o600,
            "credential material must be owner-only; got {:o}",
            mode & 0o777
        );
    }

    /// Windows has no POSIX permission model, so the non-unix build is a
    /// deliberate no-op -- but it must still accept `mode`, because consuming
    /// the parameter on every platform is what removes the need for a lint
    /// suppression at the call site.
    #[cfg(not(unix))]
    #[test]
    fn apply_material_mode_is_a_noop_off_unix() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("material");
        let file = std::fs::File::create(&path).expect("create material file");

        apply_material_mode(&file, 0o600).expect("apply mode must succeed off unix");
    }

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

    fn test_runtime(material_root: PathBuf) -> ManagedEgressRuntime {
        ManagedEgressRuntime {
            proxy_image: "sha256:test-proxy".to_string(),
            policy: policy(),
            posture: "test-posture".to_string(),
            ca: Arc::new(SandboxCertificateAuthority::generate().unwrap()),
            material_root,
            upstream_gate: tokio::sync::Mutex::new(()),
        }
    }

    fn test_bundle(material_root: &std::path::Path) -> ManagedEgressBundle {
        ManagedEgressBundle {
            network_name: "test-network".to_string(),
            proxy_ip: "172.28.10.2".to_string(),
            proxy_host: "test-proxy".to_string(),
            posture: "test-posture".to_string(),
            ca_cert_path: material_root.join("ca.crt"),
            invocation_id_path: material_root.join("invocation-id"),
        }
    }

    fn test_credential(
        placeholder: &str,
        header_name: &str,
        secret: &str,
    ) -> SandboxCommandCredential {
        SandboxCommandCredential::new(
            ironclaw_host_api::ids::SecretHandle::new("atlas_runtime_token").unwrap(),
            "ATLAS_TOKEN".to_string(),
            placeholder.to_string(),
            "github.com".to_string(),
            header_name.to_string(),
            Some("Bearer ".to_string()),
            secret.to_string(),
        )
    }

    #[test]
    fn renders_parseable_default_deny_allowlist_without_management_api() {
        let rendered = render_proxy_config(&policy(), "172.28.10.2").unwrap();
        let parsed: serde_yaml::Value = serde_yaml::from_str(&rendered).unwrap();

        assert_eq!(parsed["dns"]["proxy_ip"].as_str(), Some("172.28.10.2"));
        assert_eq!(parsed["dns"]["listen"].as_str(), Some("172.28.10.2:53"));
        assert_eq!(
            parsed["proxy"]["http_listen"].as_str(),
            Some("172.28.10.2:80")
        );
        assert_eq!(
            parsed["proxy"]["https_listen"].as_str(),
            Some("172.28.10.2:443")
        );
        assert_eq!(
            parsed["proxy"]["tunnel_listen"].as_str(),
            Some("172.28.10.2:3128")
        );
        let denied = parsed["proxy"]["upstream_deny_cidrs"]
            .as_sequence()
            .unwrap();
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
            assert!(
                denied.iter().any(|value| value.as_str() == Some(cidr)),
                "missing denied CIDR {cidr}"
            );
        }
        assert!(parsed.get("management").is_none());
        assert_eq!(parsed["tls"]["mode"].as_str(), Some("sni-only"));
    }

    #[test]
    fn credential_renderer_preserves_transform_order_and_bundle_source() {
        let credentials = [ProxyCredentialRule {
            bundle_key: "atlas_runtime_token".to_string(),
            approved_host: "github.com".to_string(),
            target_header: "Authorization".to_string(),
            placeholder_value: "Bearer icsbx_test_placeholder".to_string(),
        }];

        let rendered =
            render_proxy_config_inner(&policy(), "172.28.10.2", true, &credentials).unwrap();
        let parsed: serde_yaml::Value = serde_yaml::from_str(&rendered).unwrap();
        let transforms = parsed["transforms"].as_sequence().unwrap();
        let names = transforms
            .iter()
            .map(|transform| transform["name"].as_str().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            ["allowlist", "header_allowlist", "secrets", "annotate"]
        );

        let header_allowlist = transforms[1]["config"]["headers"].as_sequence().unwrap();
        assert_eq!(
            header_allowlist
                .iter()
                .filter(|header| header.as_str() == Some("Authorization"))
                .count(),
            1,
            "a credential header already in the base allowlist must not be duplicated"
        );
        let secrets = transforms[2]["config"]["secrets"].as_sequence().unwrap();
        assert_eq!(
            secrets[0]["source"]["path"].as_str(),
            Some(PROXY_INVOCATION_ID_PATH)
        );
        assert_eq!(
            secrets[0]["inject"]["header"].as_str(),
            Some("X-Ironclaw-Invocation-Id")
        );
        assert_eq!(
            secrets[1]["source"]["path"].as_str(),
            Some(PROXY_CREDENTIAL_BUNDLE_PATH)
        );
        assert_eq!(
            secrets[1]["source"]["json_key"].as_str(),
            Some("atlas_runtime_token")
        );
        assert_eq!(
            secrets[1]["replace"]["proxy_value"].as_str(),
            Some("Bearer icsbx_test_placeholder")
        );
        assert_eq!(secrets[1]["rules"][0]["host"].as_str(), Some("github.com"));
        assert_eq!(parsed["tls"]["ca_cert"].as_str(), Some(PROXY_CA_CERT_PATH));
        assert_eq!(parsed["tls"]["ca_key"].as_str(), Some(PROXY_CA_KEY_PATH));
    }

    #[test]
    fn typed_renderer_round_trips_yaml_sensitive_values_exactly() {
        let host = "api.atlas.test\"quoted\nline";
        let header = "X-Atlas-\"\n\u{1}";
        let placeholder = "Bearer placeholder\"\n\u{2}";
        let policy = NetworkPolicy {
            allowed_targets: vec![NetworkTargetPattern {
                scheme: None,
                host_pattern: host.to_string(),
                port: None,
            }],
            deny_private_ip_ranges: true,
            max_egress_bytes: None,
        };
        let credentials = [ProxyCredentialRule {
            bundle_key: "atlas\"key\n".to_string(),
            approved_host: host.to_string(),
            target_header: header.to_string(),
            placeholder_value: placeholder.to_string(),
        }];

        let rendered =
            render_proxy_config_inner(&policy, "172.28.10.2", true, &credentials).unwrap();
        let parsed: serde_yaml::Value = serde_yaml::from_str(&rendered).unwrap();
        let transforms = parsed["transforms"].as_sequence().unwrap();
        let secrets = transforms[2]["config"]["secrets"].as_sequence().unwrap();

        assert_eq!(transforms[0]["config"]["domains"][0].as_str(), Some(host));
        assert_eq!(
            transforms[1]["config"]["headers"]
                .as_sequence()
                .unwrap()
                .last()
                .and_then(serde_yaml::Value::as_str),
            Some(header)
        );
        assert_eq!(
            secrets[1]["source"]["json_key"].as_str(),
            Some("atlas\"key\n")
        );
        assert_eq!(secrets[1]["rules"][0]["host"].as_str(), Some(host));
        assert_eq!(
            secrets[1]["replace"]["proxy_value"].as_str(),
            Some(placeholder)
        );
        assert_eq!(
            secrets[1]["replace"]["match_headers"][0].as_str(),
            Some(header)
        );
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
    fn posture_binds_image_policy_and_deployment_identity() {
        let image = "sha256:proxy-image";
        let base = proxy_posture(image, &policy(), b"ca-a").unwrap();
        assert_eq!(base, proxy_posture(image, &policy(), b"ca-a").unwrap());
        assert_ne!(base, proxy_posture(image, &policy(), b"ca-b").unwrap());
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
    async fn bind_mounted_proxy_config_rewrite_preserves_the_mounted_inode() {
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt as _;

        let directory = tempfile::tempdir().unwrap();
        let config = directory.path().join("proxy.yaml");
        write_atomic_material_file(&config, b"first").await.unwrap();

        #[cfg(unix)]
        let original_inode = tokio::fs::metadata(&config).await.unwrap().ino();

        write_bind_mounted_proxy_config(&config, b"second")
            .await
            .unwrap();

        assert_eq!(tokio::fs::read(&config).await.unwrap(), b"second");
        #[cfg(unix)]
        assert_eq!(
            tokio::fs::metadata(&config).await.unwrap().ino(),
            original_inode,
            "Docker Desktop bind mounts track the original config inode"
        );
    }

    #[tokio::test]
    async fn unchanged_ca_material_preserves_the_file_bind_inode() {
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt as _;

        let directory = tempfile::tempdir().unwrap();
        let certificate = directory.path().join("ca.crt");
        write_atomic_material_file_if_changed(&certificate, b"same-root", 0o644)
            .await
            .unwrap();

        #[cfg(unix)]
        let original_inode = tokio::fs::metadata(&certificate).await.unwrap().ino();

        #[cfg(unix)]
        tokio::fs::set_permissions(&certificate, {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::Permissions::from_mode(0o666)
        })
        .await
        .unwrap();

        write_atomic_material_file_if_changed(&certificate, b"same-root", 0o644)
            .await
            .unwrap();

        assert_eq!(tokio::fs::read(&certificate).await.unwrap(), b"same-root");
        #[cfg(unix)]
        assert_eq!(
            tokio::fs::metadata(&certificate).await.unwrap().ino(),
            original_inode,
            "an unchanged CA must remain visible through an existing file bind mount"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            assert_eq!(
                tokio::fs::metadata(&certificate)
                    .await
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o644,
                "unchanged material must have its required mode repaired"
            );
        }
    }

    #[tokio::test]
    async fn credential_cleanup_removes_bundle_and_legacy_material() {
        let directory = tempfile::tempdir().unwrap();
        let bundle = directory.path().join("credentials.json");
        let legacy = directory.path().join("credential-0.secret");
        let marker = directory.path().join("invocation-id");
        let certificate = directory.path().join("ca.crt");
        let private_key = directory.path().join("ca.key");
        write_atomic_private_material_file(&bundle, br#"{"atlas":"secret"}"#)
            .await
            .unwrap();
        write_atomic_private_material_file(&legacy, b"legacy")
            .await
            .unwrap();
        write_atomic_material_file(&marker, b"keep").await.unwrap();
        write_atomic_material_file(&certificate, b"certificate")
            .await
            .unwrap();
        write_atomic_private_material_file(&private_key, b"private-key")
            .await
            .unwrap();

        remove_credential_material(directory.path()).await.unwrap();

        assert!(!bundle.exists());
        assert!(!legacy.exists());
        assert!(marker.exists());
        assert_eq!(tokio::fs::read(&certificate).await.unwrap(), b"certificate");
        assert_eq!(tokio::fs::read(&private_key).await.unwrap(), b"private-key");
    }

    #[tokio::test]
    async fn failed_credential_write_removes_bundle() {
        let directory = tempfile::tempdir().unwrap();
        tokio::fs::create_dir(directory.path().join("proxy.yaml"))
            .await
            .unwrap();

        let result = write_proxy_credential_material(
            directory.path(),
            br#"{"atlas_runtime_token":"Bearer secret"}"#,
            b"proxy config",
        )
        .await;

        assert!(result.is_err());
        assert!(!directory.path().join("credentials.json").exists());
    }

    #[tokio::test]
    async fn failed_proxy_restart_removes_configured_credential_bundle() {
        let directory = tempfile::tempdir().unwrap();
        let runtime = test_runtime(directory.path().to_path_buf());
        let bundle = test_bundle(directory.path());
        let credential = test_credential("icsbx_test_placeholder", "Authorization", "secret");
        write_atomic_material_file(&directory.path().join("proxy.yaml"), b"base")
            .await
            .unwrap();

        let result = runtime
            .configure_credentials_with_restart(&bundle, &[credential], || async {
                Err(RuntimeProcessError::ExecutionFailed(
                    "test proxy restart failure".to_string(),
                ))
            })
            .await;

        assert!(result.is_err());
        assert!(!directory.path().join("credentials.json").exists());
    }

    #[tokio::test]
    async fn invalid_credential_replacement_rules_fail_before_material_is_written() {
        let directory = tempfile::tempdir().unwrap();
        let runtime = test_runtime(directory.path().to_path_buf());
        let bundle = test_bundle(directory.path());
        let invalid = [
            test_credential("", "Authorization", "secret"),
            test_credential("   ", "Authorization", "secret"),
            test_credential("same-value", "Authorization", "same-value"),
            test_credential("icsbx_test_placeholder", "", "secret"),
        ];

        for credential in invalid {
            let result = runtime
                .configure_credentials_with_restart(&bundle, &[credential], || async { Ok(()) })
                .await;
            assert!(result.is_err());
            assert!(!directory.path().join("credentials.json").exists());
        }
    }

    #[tokio::test]
    async fn duplicate_credential_placeholders_fail_before_material_is_written() {
        let directory = tempfile::tempdir().unwrap();
        let runtime = test_runtime(directory.path().to_path_buf());
        let bundle = test_bundle(directory.path());
        let credentials = [
            test_credential("icsbx_duplicate", "Authorization", "first-secret"),
            test_credential("icsbx_duplicate", "X-Api-Key", "second-secret"),
        ];

        let result = runtime
            .configure_credentials_with_restart(&bundle, &credentials, || async { Ok(()) })
            .await;

        assert!(result.is_err());
        assert!(!directory.path().join("credentials.json").exists());
    }

    #[tokio::test]
    async fn configured_secret_exists_only_in_the_private_bundle() {
        const RAW_SECRET_SENTINEL: &str = "raw-secret-sentinel-for-test";

        let directory = tempfile::tempdir().unwrap();
        let runtime = test_runtime(directory.path().to_path_buf());
        let bundle = test_bundle(directory.path());
        let credential = test_credential(
            "icsbx_test_placeholder",
            "Authorization",
            RAW_SECRET_SENTINEL,
        );
        write_atomic_material_file(&directory.path().join("proxy.yaml"), b"base")
            .await
            .unwrap();

        runtime
            .configure_credentials_with_restart(
                &bundle,
                std::slice::from_ref(&credential),
                || async { Ok(()) },
            )
            .await
            .unwrap();

        let proxy_config = tokio::fs::read_to_string(directory.path().join("proxy.yaml"))
            .await
            .unwrap();
        let credential_bundle =
            tokio::fs::read_to_string(directory.path().join("credentials.json"))
                .await
                .unwrap();
        assert!(!proxy_config.contains(RAW_SECRET_SENTINEL));
        assert!(!format!("{runtime:?}").contains(RAW_SECRET_SENTINEL));
        assert!(!format!("{credential:?}").contains(RAW_SECRET_SENTINEL));
        assert!(credential_bundle.contains(RAW_SECRET_SENTINEL));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;

            let mode = tokio::fs::metadata(directory.path().join("credentials.json"))
                .await
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
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
