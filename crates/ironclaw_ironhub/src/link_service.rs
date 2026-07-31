use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ironclaw_filesystem::{CasExpectation, Entry, FilesystemError, RootFilesystem};
use ironclaw_host_api::{
    ids::{CapabilityId, InvocationId},
    path::VirtualPath,
    product_surface::ProductSurfaceCaller,
    resource::ResourceScope,
};
use ironclaw_product::{
    IronhubInstallDeliveryRequest, IronhubInstallDeliveryResult, IronhubLinkError,
    IronhubLinkService, IronhubRegisterRequest,
};
use ironclaw_skills::ScopedSkillManagementPort;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use ironclaw_extension_host::ExtensionLifecycleManager;

use super::agent_link::{InstallDelivery, IronhubSharedKey, RegisterChallenge, verify_signature};
use super::model::{IronHubCommand, IronHubCommandError, IronHubInstallOptions, IronHubPhase};
use super::service::IronHubService;

const IRONHUB_STATE_ROOT: &str = "/system/settings/ironhub";
const MAX_LINK_ID_BYTES: usize = 256;
const MAX_NONCE_BYTES: usize = 512;
const MANIFEST_CAS_RETRIES: usize = 32;
const MAX_TIMESTAMP_DRIFT_SECS: u64 = 300;
const INSTALL_CAPABILITY_ID: &str = "builtin.ironhub_install";

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IronhubLinkBuildError {
    #[error("invalid IronHub link configuration")]
    InvalidConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IronhubLinkStateError {
    #[error("IronHub install request was replayed")]
    NonceReplay,
    #[error("IronHub private manifest was replayed or downgraded")]
    ManifestReplay,
    #[error("invalid IronHub durable state input")]
    InvalidInput,
    #[error("IronHub durable state is unavailable")]
    Unavailable,
}

#[derive(Clone)]
pub struct IronhubLinkStateStore {
    filesystem: Arc<dyn RootFilesystem>,
}

pub struct RebornIronhubLinkService {
    skill_management: Arc<ScopedSkillManagementPort>,
    extension_management: Arc<ExtensionLifecycleManager>,
    egress: Arc<dyn ironclaw_host_api::http::RuntimeHttpEgress>,
    state: Arc<IronhubLinkStateStore>,
    shared_key: IronhubSharedKey,
    install_capability: CapabilityId,
    manifest_url: crate::IronhubManifestUrl,
}

impl RebornIronhubLinkService {
    pub fn new(
        skill_management: Arc<ScopedSkillManagementPort>,
        extension_management: Arc<ExtensionLifecycleManager>,
        runtime_http_egress: Arc<dyn ironclaw_host_api::http::RuntimeHttpEgress>,
        state: Arc<IronhubLinkStateStore>,
        shared_key: IronhubSharedKey,
    ) -> Result<Self, IronhubLinkBuildError> {
        let install_capability = CapabilityId::new(INSTALL_CAPABILITY_ID)
            .map_err(|_| IronhubLinkBuildError::InvalidConfig)?;
        Ok(Self {
            skill_management,
            extension_management,
            egress: runtime_http_egress,
            state,
            shared_key,
            install_capability,
            manifest_url: crate::IronhubManifestUrl::default(),
        })
    }

    pub fn with_manifest_url(mut self, manifest_url: crate::IronhubManifestUrl) -> Self {
        self.manifest_url = manifest_url;
        self
    }

    fn install_service(&self, scope: ResourceScope) -> IronHubService {
        let service = IronHubService::new_with_runtime_egress(
            Arc::clone(&self.skill_management),
            Arc::clone(&self.extension_management),
            Arc::clone(&self.egress),
            scope,
            self.install_capability.clone(),
        );
        #[cfg(test)]
        let service = super::service::configure_test_catalog(
            service,
            super::model::DEFAULT_IRONHUB_MANIFEST_URL,
            super::tests::test_manifest_verify_keys(),
        );
        service
            .with_manifest_url(self.manifest_url.as_str().to_string())
            .with_link_state(Arc::clone(&self.state))
    }
}

impl IronhubLinkStateStore {
    pub fn new(filesystem: Arc<dyn RootFilesystem>) -> Self {
        Self { filesystem }
    }

    pub async fn consume_install_nonce(
        &self,
        caller: &ProductSurfaceCaller,
        nonce: &str,
        consumed_at: DateTime<Utc>,
    ) -> Result<(), IronhubLinkStateError> {
        if nonce.is_empty() || nonce.len() > MAX_NONCE_BYTES || nonce.chars().any(char::is_control)
        {
            return Err(IronhubLinkStateError::InvalidInput);
        }
        let path = nonce_path(caller, nonce)?;
        let record = ConsumedNonce { consumed_at };
        let body = serde_json::to_vec(&record).map_err(|error| {
            tracing::debug!(%error, "failed to serialize consumed IronHub nonce");
            IronhubLinkStateError::Unavailable
        })?;
        match self
            .filesystem
            .put(&path, Entry::bytes(body), CasExpectation::Absent)
            .await
        {
            Ok(_) => Ok(()),
            Err(FilesystemError::VersionMismatch { .. }) => Err(IronhubLinkStateError::NonceReplay),
            Err(error) => {
                tracing::debug!(%error, "failed to persist consumed IronHub nonce");
                Err(IronhubLinkStateError::Unavailable)
            }
        }
    }

    pub async fn record_private_manifest(
        &self,
        catalog_host: &str,
        signed_repo: &str,
        generated_at: DateTime<Utc>,
        signed_manifest_digest: &str,
    ) -> Result<(), IronhubLinkStateError> {
        let catalog_host = canonical_host(catalog_host)?;
        if signed_repo.trim().is_empty() || signed_repo.len() > 1024 {
            return Err(IronhubLinkStateError::InvalidInput);
        }
        let path = manifest_path(&catalog_host, signed_repo)?;
        let desired = PrivateManifestState {
            catalog_host,
            signed_repo: signed_repo.to_string(),
            generated_at,
            signed_manifest_digest: signed_manifest_digest.to_ascii_lowercase(),
        };

        for _ in 0..MANIFEST_CAS_RETRIES {
            let current = self.filesystem.get(&path).await.map_err(|error| {
                tracing::debug!(%error, "failed to read durable IronHub manifest state");
                IronhubLinkStateError::Unavailable
            })?;
            let cas = match current {
                Some(versioned) => {
                    let prior: PrivateManifestState = serde_json::from_slice(&versioned.entry.body)
                        .map_err(|error| {
                            tracing::debug!(
                                %error,
                                "failed to decode durable IronHub manifest state"
                            );
                            IronhubLinkStateError::Unavailable
                        })?;
                    if prior.catalog_host != desired.catalog_host
                        || prior.signed_repo != desired.signed_repo
                    {
                        return Err(IronhubLinkStateError::Unavailable);
                    }
                    if desired.generated_at <= prior.generated_at {
                        return Err(IronhubLinkStateError::ManifestReplay);
                    }
                    CasExpectation::Version(versioned.version)
                }
                None => CasExpectation::Absent,
            };
            let body = serde_json::to_vec(&desired).map_err(|error| {
                tracing::debug!(%error, "failed to serialize durable IronHub manifest state");
                IronhubLinkStateError::Unavailable
            })?;
            match self.filesystem.put(&path, Entry::bytes(body), cas).await {
                Ok(_) => return Ok(()),
                Err(FilesystemError::VersionMismatch { .. }) => {
                    tokio::task::yield_now().await;
                }
                Err(error) => {
                    tracing::debug!(%error, "failed to persist durable IronHub manifest state");
                    return Err(IronhubLinkStateError::Unavailable);
                }
            }
        }
        Err(IronhubLinkStateError::Unavailable)
    }

    pub async fn record_public_manifest(
        &self,
        manifest_url: &str,
        generated_at: DateTime<Utc>,
        signed_manifest_digest: &str,
    ) -> Result<(), IronhubLinkStateError> {
        let path = public_manifest_path(manifest_url)?;
        let desired = PublicManifestState {
            manifest_url: manifest_url.to_string(),
            generated_at,
            signed_manifest_digest: signed_manifest_digest.to_ascii_lowercase(),
        };
        for _ in 0..MANIFEST_CAS_RETRIES {
            let current = self.filesystem.get(&path).await.map_err(|error| {
                tracing::debug!(%error, "failed to read public IronHub manifest replay state");
                IronhubLinkStateError::Unavailable
            })?;
            let cas = match current {
                Some(versioned) => {
                    let prior: PublicManifestState = serde_json::from_slice(&versioned.entry.body)
                        .map_err(|error| {
                            tracing::debug!(
                                %error,
                                "failed to decode public IronHub manifest replay state"
                            );
                            IronhubLinkStateError::Unavailable
                        })?;
                    if desired.manifest_url != prior.manifest_url
                        || desired.generated_at < prior.generated_at
                        || (desired.generated_at == prior.generated_at
                            && desired.signed_manifest_digest != prior.signed_manifest_digest)
                    {
                        return Err(IronhubLinkStateError::ManifestReplay);
                    }
                    if desired == prior {
                        return Ok(());
                    }
                    CasExpectation::Version(versioned.version)
                }
                None => CasExpectation::Absent,
            };
            let body = serde_json::to_vec(&desired).map_err(|error| {
                tracing::debug!(%error, "failed to serialize public IronHub manifest replay state");
                IronhubLinkStateError::Unavailable
            })?;
            match self.filesystem.put(&path, Entry::bytes(body), cas).await {
                Ok(_) => return Ok(()),
                Err(FilesystemError::VersionMismatch { .. }) => tokio::task::yield_now().await,
                Err(error) => {
                    tracing::debug!(%error, "failed to persist public IronHub manifest replay state");
                    return Err(IronhubLinkStateError::Unavailable);
                }
            }
        }
        Err(IronhubLinkStateError::Unavailable)
    }
}

#[async_trait]
impl IronhubLinkService for RebornIronhubLinkService {
    async fn register(&self, request: IronhubRegisterRequest) -> Result<(), IronhubLinkError> {
        authenticate_register(&self.shared_key, &request)
    }

    async fn deliver_install(
        &self,
        caller: ProductSurfaceCaller,
        request: IronhubInstallDeliveryRequest,
    ) -> Result<IronhubInstallDeliveryResult, IronhubLinkError> {
        authenticate_install(&self.shared_key, &request)?;
        self.state
            .consume_install_nonce(&caller, &request.nonce, Utc::now())
            .await
            .map_err(map_state_error)?;

        let scope = ResourceScope {
            tenant_id: caller.tenant_id,
            user_id: caller.user_id,
            agent_id: caller.agent_id,
            project_id: caller.project_id,
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        };
        let response = self
            .install_service(scope)
            .execute(IronHubCommand::Install {
                name: request.slug.clone(),
                options: IronHubInstallOptions {
                    kind: None,
                    force: false,
                    acknowledge_unverified: false,
                    expected_version: Some(request.version),
                    expected_artifact_digest: Some(request.artifact_digest),
                    private_manifest_url: request.private_manifest_url,
                },
            })
            .await
            .map_err(map_install_error)?;

        Ok(IronhubInstallDeliveryResult {
            installed: response.phase == IronHubPhase::Installed,
            slug: request.slug,
            message: response.message.unwrap_or_default(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ConsumedNonce {
    consumed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct PrivateManifestState {
    catalog_host: String,
    signed_repo: String,
    generated_at: DateTime<Utc>,
    signed_manifest_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct PublicManifestState {
    manifest_url: String,
    generated_at: DateTime<Utc>,
    signed_manifest_digest: String,
}

fn nonce_path(
    caller: &ProductSurfaceCaller,
    nonce: &str,
) -> Result<VirtualPath, IronhubLinkStateError> {
    let mut hasher = Sha256::new();
    hash_length_prefixed(&mut hasher, caller.tenant_id.as_str().as_bytes());
    hash_length_prefixed(&mut hasher, caller.user_id.as_str().as_bytes());
    if let Some(agent_id) = &caller.agent_id {
        hash_length_prefixed(&mut hasher, agent_id.as_str().as_bytes());
    } else {
        hash_length_prefixed(&mut hasher, &[]);
    }
    if let Some(project_id) = &caller.project_id {
        hash_length_prefixed(&mut hasher, project_id.as_str().as_bytes());
    } else {
        hash_length_prefixed(&mut hasher, &[]);
    }
    hash_length_prefixed(&mut hasher, nonce.as_bytes());
    let digest = hex::encode(hasher.finalize());
    state_path(&format!("install-nonces/{digest}.json"))
}

fn manifest_path(
    catalog_host: &str,
    signed_repo: &str,
) -> Result<VirtualPath, IronhubLinkStateError> {
    let mut hasher = Sha256::new();
    hash_length_prefixed(&mut hasher, catalog_host.as_bytes());
    hash_length_prefixed(&mut hasher, signed_repo.as_bytes());
    let digest = hex::encode(hasher.finalize());
    state_path(&format!("private-manifests/{digest}.json"))
}

fn public_manifest_path(manifest_url: &str) -> Result<VirtualPath, IronhubLinkStateError> {
    if manifest_url.is_empty() || manifest_url.len() > 4096 {
        return Err(IronhubLinkStateError::InvalidInput);
    }
    let digest = hex::encode(Sha256::digest(manifest_url.as_bytes()));
    state_path(&format!("public-manifests/{digest}.json"))
}

fn hash_length_prefixed(hasher: &mut Sha256, field: &[u8]) {
    hasher.update(u64::try_from(field.len()).unwrap_or(u64::MAX).to_be_bytes());
    hasher.update(field);
}

fn state_path(suffix: &str) -> Result<VirtualPath, IronhubLinkStateError> {
    VirtualPath::new(format!("{IRONHUB_STATE_ROOT}/{suffix}"))
        .map_err(|_| IronhubLinkStateError::Unavailable)
}

fn canonical_host(host: &str) -> Result<String, IronhubLinkStateError> {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    if host.is_empty() || host.chars().any(char::is_control) {
        return Err(IronhubLinkStateError::InvalidInput);
    }
    Ok(host)
}

fn authenticate_register(
    shared_key: &IronhubSharedKey,
    request: &IronhubRegisterRequest,
) -> Result<(), IronhubLinkError> {
    for (field, value) in [
        ("uid", request.uid.as_str()),
        ("aid", request.aid.as_str()),
        ("nonce", request.nonce.as_str()),
    ] {
        if value.is_empty()
            || value.len()
                > if field == "nonce" {
                    MAX_NONCE_BYTES
                } else {
                    MAX_LINK_ID_BYTES
                }
            || value.contains(':')
            || value.chars().any(char::is_control)
        {
            return Err(IronhubLinkError::InvalidInput {
                reason: format!("invalid register {field}"),
            });
        }
    }
    if !timestamp_fresh(request.ts) {
        return Err(IronhubLinkError::StaleTimestamp);
    }
    let challenge = RegisterChallenge {
        uid: &request.uid,
        aid: &request.aid,
        ts: request.ts,
        nonce: &request.nonce,
    };
    if verify_signature(shared_key, &challenge.payload(), &request.sig) {
        Ok(())
    } else {
        Err(IronhubLinkError::InvalidSignature)
    }
}

fn authenticate_install(
    shared_key: &IronhubSharedKey,
    request: &IronhubInstallDeliveryRequest,
) -> Result<(), IronhubLinkError> {
    if !timestamp_fresh(request.ts) {
        return Err(IronhubLinkError::StaleTimestamp);
    }
    let delivery = InstallDelivery {
        slug: &request.slug,
        version: &request.version,
        uid: &request.uid,
        aid: &request.aid,
        ts: request.ts,
        nonce: &request.nonce,
        artifact_digest: &request.artifact_digest,
        private_manifest_url: request.private_manifest_url.as_deref(),
    };
    if verify_signature(shared_key, &delivery.payload(), &request.sig) {
        Ok(())
    } else {
        Err(IronhubLinkError::InvalidSignature)
    }
}

fn timestamp_fresh(ts: u64) -> bool {
    let Ok(ts) = i64::try_from(ts) else {
        return false;
    };
    Utc::now().timestamp().abs_diff(ts) <= MAX_TIMESTAMP_DRIFT_SECS
}

fn map_state_error(error: IronhubLinkStateError) -> IronhubLinkError {
    match error {
        IronhubLinkStateError::NonceReplay | IronhubLinkStateError::ManifestReplay => {
            IronhubLinkError::Replay
        }
        IronhubLinkStateError::InvalidInput => IronhubLinkError::InvalidInput {
            reason: "invalid durable replay state input".to_string(),
        },
        IronhubLinkStateError::Unavailable => IronhubLinkError::Unavailable,
    }
}

fn map_install_error(error: IronHubCommandError) -> IronhubLinkError {
    match error {
        IronHubCommandError::InvalidInput { reason } | IronHubCommandError::Catalog { reason } => {
            IronhubLinkError::InvalidInput { reason }
        }
        IronHubCommandError::RuntimeHttpEgressUnavailable => IronhubLinkError::Install {
            reason: "runtime HTTP egress is unavailable".to_string(),
        },
        IronHubCommandError::Install { reason } => IronhubLinkError::Install { reason },
        IronHubCommandError::Product(_) => IronhubLinkError::Install {
            reason: "extension lifecycle failed".to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use hmac::{Hmac, KeyInit, Mac};
    use ironclaw_filesystem::{
        Fault, FaultInjecting, FileStat, FileType, FilesystemOperation, InMemoryBackend,
        RecordVersion,
    };
    use sha2::Sha256;

    use super::*;

    const SHARED_KEY: &str = "ihub_sk_LinkServiceTestKey0000000000000000000000000";

    fn shared_filesystem() -> Arc<dyn RootFilesystem> {
        Arc::new(InMemoryBackend::new())
    }

    fn shared_key() -> IronhubSharedKey {
        IronhubSharedKey::new(SHARED_KEY).expect("test shared key")
    }

    fn caller(user_id: &str) -> ProductSurfaceCaller {
        ProductSurfaceCaller::new(
            ironclaw_host_api::ids::TenantId::new("tenant").expect("tenant"),
            ironclaw_host_api::ids::UserId::new(user_id).expect("user"),
            Some(ironclaw_host_api::ids::AgentId::new("agent").expect("agent")),
            None,
        )
    }

    struct AlwaysCasConflictFilesystem;

    #[async_trait]
    impl RootFilesystem for AlwaysCasConflictFilesystem {
        async fn put(
            &self,
            path: &VirtualPath,
            _entry: Entry,
            _cas: CasExpectation,
        ) -> Result<RecordVersion, FilesystemError> {
            Err(FilesystemError::VersionMismatch {
                path: path.clone(),
                expected: None,
                found: Some(RecordVersion::from_backend(1)),
            })
        }

        async fn get(
            &self,
            _path: &VirtualPath,
        ) -> Result<Option<ironclaw_filesystem::VersionedEntry>, FilesystemError> {
            Ok(None)
        }

        async fn list_dir(
            &self,
            _path: &VirtualPath,
        ) -> Result<Vec<ironclaw_filesystem::DirEntry>, FilesystemError> {
            Ok(Vec::new())
        }

        async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
            Ok(FileStat {
                path: path.clone(),
                file_type: FileType::File,
                len: 0,
                modified: None,
                sensitive: false,
            })
        }
    }

    fn sign(payload: &str) -> String {
        let mut mac = Hmac::<Sha256>::new_from_slice(SHARED_KEY.as_bytes()).expect("HMAC key");
        mac.update(payload.as_bytes());
        hex::encode(mac.finalize().into_bytes())
    }

    fn register_request(ts: u64) -> IronhubRegisterRequest {
        let mut request = IronhubRegisterRequest {
            uid: "user-1".to_string(),
            aid: "agent-1".to_string(),
            ts,
            nonce: "register-nonce".to_string(),
            sig: String::new(),
        };
        let challenge = RegisterChallenge {
            uid: &request.uid,
            aid: &request.aid,
            ts: request.ts,
            nonce: &request.nonce,
        };
        request.sig = sign(&challenge.payload());
        request
    }

    fn install_request(ts: u64) -> IronhubInstallDeliveryRequest {
        let mut request = IronhubInstallDeliveryRequest {
            slug: "fixture".to_string(),
            version: "1.0.0".to_string(),
            uid: "user-1".to_string(),
            aid: "agent-1".to_string(),
            ts,
            nonce: "install-nonce".to_string(),
            artifact_digest: format!("sha256:{}", "a".repeat(64)),
            sig: String::new(),
            private_manifest_url: Some("https://hub.ironclaw.com/private/manifest".to_string()),
        };
        request.sig = sign(
            &InstallDelivery {
                slug: &request.slug,
                version: &request.version,
                uid: &request.uid,
                aid: &request.aid,
                ts: request.ts,
                nonce: &request.nonce,
                artifact_digest: &request.artifact_digest,
                private_manifest_url: request.private_manifest_url.as_deref(),
            }
            .payload(),
        );
        request
    }

    #[test]
    fn register_authentication_rejects_stale_timestamp_before_hmac() {
        let request = register_request(1);
        assert!(matches!(
            authenticate_register(&shared_key(), &request),
            Err(IronhubLinkError::StaleTimestamp)
        ));
    }

    #[test]
    fn register_authentication_rejects_bad_hmac() {
        let mut request = register_request(
            u64::try_from(Utc::now().timestamp()).expect("current positive timestamp"),
        );
        request.sig = "00".to_string();
        assert!(matches!(
            authenticate_register(&shared_key(), &request),
            Err(IronhubLinkError::InvalidSignature)
        ));
    }

    #[test]
    fn register_authentication_rejects_ambiguous_delimited_fields_before_hmac() {
        let mut request = register_request(
            u64::try_from(Utc::now().timestamp()).expect("current positive timestamp"),
        );
        request.uid = "user:alternate".to_string();

        assert!(matches!(
            authenticate_register(&shared_key(), &request),
            Err(IronhubLinkError::InvalidInput { .. })
        ));
    }

    #[test]
    fn authentication_validates_all_bounded_fields_and_install_signatures() {
        let now = u64::try_from(Utc::now().timestamp()).expect("positive timestamp");
        assert!(authenticate_register(&shared_key(), &register_request(now)).is_ok());

        for (field, value) in [
            ("uid", "".to_string()),
            ("aid", "x".repeat(MAX_LINK_ID_BYTES + 1)),
            ("nonce", "x".repeat(MAX_NONCE_BYTES + 1)),
            ("nonce", "line\nbreak".to_string()),
        ] {
            let mut request = register_request(now);
            match field {
                "uid" => request.uid = value,
                "aid" => request.aid = value,
                "nonce" => request.nonce = value,
                _ => unreachable!("fixed test field"),
            }
            assert!(matches!(
                authenticate_register(&shared_key(), &request),
                Err(IronhubLinkError::InvalidInput { .. })
            ));
        }

        let valid = install_request(now);
        assert!(authenticate_install(&shared_key(), &valid).is_ok());
        let mut bad_signature = valid.clone();
        bad_signature.sig = "00".to_string();
        assert!(matches!(
            authenticate_install(&shared_key(), &bad_signature),
            Err(IronhubLinkError::InvalidSignature)
        ));
        let stale = install_request(1);
        assert!(matches!(
            authenticate_install(&shared_key(), &stale),
            Err(IronhubLinkError::StaleTimestamp)
        ));
        assert!(!timestamp_fresh(u64::MAX));
    }

    #[test]
    fn link_errors_map_to_stable_product_errors() {
        assert!(matches!(
            map_state_error(IronhubLinkStateError::NonceReplay),
            IronhubLinkError::Replay
        ));
        assert!(matches!(
            map_state_error(IronhubLinkStateError::ManifestReplay),
            IronhubLinkError::Replay
        ));
        assert!(matches!(
            map_state_error(IronhubLinkStateError::InvalidInput),
            IronhubLinkError::InvalidInput { .. }
        ));
        assert!(matches!(
            map_state_error(IronhubLinkStateError::Unavailable),
            IronhubLinkError::Unavailable
        ));
        for error in [
            IronHubCommandError::InvalidInput {
                reason: "bad input".to_string(),
            },
            IronHubCommandError::Catalog {
                reason: "bad catalog".to_string(),
            },
        ] {
            assert!(matches!(
                map_install_error(error),
                IronhubLinkError::InvalidInput { .. }
            ));
        }
        assert!(matches!(
            map_install_error(IronHubCommandError::RuntimeHttpEgressUnavailable),
            IronhubLinkError::Install { .. }
        ));
        assert!(matches!(
            map_install_error(IronHubCommandError::Install {
                reason: "failed".to_string(),
            }),
            IronhubLinkError::Install { .. }
        ));
        assert!(matches!(
            map_install_error(IronHubCommandError::Product(
                ironclaw_product::ProductSurfaceFailure::InvalidBindingRequest {
                    reason: "failed".to_string(),
                },
            )),
            IronhubLinkError::Install { .. }
        ));
    }

    #[tokio::test]
    async fn nonce_is_single_use_across_store_reconstruction() {
        let filesystem = shared_filesystem();
        let first = IronhubLinkStateStore::new(Arc::clone(&filesystem));
        let caller = caller("user-a");
        first
            .consume_install_nonce(&caller, "one-shot", Utc::now())
            .await
            .expect("first consumption");

        let reconstructed = IronhubLinkStateStore::new(filesystem);
        assert_eq!(
            reconstructed
                .consume_install_nonce(&caller, "one-shot", Utc::now())
                .await,
            Err(IronhubLinkStateError::NonceReplay)
        );
    }

    #[tokio::test]
    async fn nonce_consumption_is_scoped_to_authenticated_caller() {
        let store = IronhubLinkStateStore::new(shared_filesystem());
        store
            .consume_install_nonce(&caller("user-a"), "shared-nonce", Utc::now())
            .await
            .expect("first caller consumes nonce");
        store
            .consume_install_nonce(&caller("user-b"), "shared-nonce", Utc::now())
            .await
            .expect("another caller has an independent nonce namespace");
        let project_caller = ProductSurfaceCaller::new(
            ironclaw_host_api::ids::TenantId::new("tenant").expect("tenant"),
            ironclaw_host_api::ids::UserId::new("user-c").expect("user"),
            None,
            Some(ironclaw_host_api::ids::ProjectId::new("project").expect("project")),
        );
        store
            .consume_install_nonce(&project_caller, "shared-nonce", Utc::now())
            .await
            .expect("project-only caller has a stable nonce namespace");
    }

    #[tokio::test]
    async fn durable_state_rejects_invalid_inputs_and_maps_backend_failures() {
        let store = IronhubLinkStateStore::new(shared_filesystem());
        for nonce in ["", "line\nbreak"] {
            assert_eq!(
                store
                    .consume_install_nonce(&caller("user"), nonce, Utc::now())
                    .await,
                Err(IronhubLinkStateError::InvalidInput)
            );
        }
        let oversized_nonce = "x".repeat(MAX_NONCE_BYTES + 1);
        assert_eq!(
            store
                .consume_install_nonce(&caller("user"), &oversized_nonce, Utc::now())
                .await,
            Err(IronhubLinkStateError::InvalidInput)
        );
        assert_eq!(
            store
                .record_private_manifest("", "repo", Utc::now(), "digest")
                .await,
            Err(IronhubLinkStateError::InvalidInput)
        );
        assert_eq!(
            store
                .record_private_manifest("host", "", Utc::now(), "digest")
                .await,
            Err(IronhubLinkStateError::InvalidInput)
        );
        assert_eq!(
            store.record_public_manifest("", Utc::now(), "digest").await,
            Err(IronhubLinkStateError::InvalidInput)
        );
        let oversized_url = "x".repeat(4097);
        assert_eq!(
            store
                .record_public_manifest(&oversized_url, Utc::now(), "digest")
                .await,
            Err(IronhubLinkStateError::InvalidInput)
        );
        assert_eq!(
            store
                .record_private_manifest("bad\nhost", "repo", Utc::now(), "digest")
                .await,
            Err(IronhubLinkStateError::InvalidInput)
        );

        for operation in [
            FilesystemOperation::ReadFile,
            FilesystemOperation::WriteFile,
        ] {
            let filesystem: Arc<dyn RootFilesystem> = Arc::new(
                FaultInjecting::new(InMemoryBackend::new())
                    .with_fault(Fault::on(operation).backend("state unavailable")),
            );
            let unavailable = IronhubLinkStateStore::new(filesystem);
            let result = unavailable
                .record_private_manifest("catalog.example", "org/repo", Utc::now(), "digest")
                .await;
            assert_eq!(result, Err(IronhubLinkStateError::Unavailable));
            assert_eq!(
                unavailable
                    .record_public_manifest(
                        "https://catalog.example/manifest",
                        Utc::now(),
                        "digest",
                    )
                    .await,
                Err(IronhubLinkStateError::Unavailable)
            );
        }

        let filesystem: Arc<dyn RootFilesystem> = Arc::new(
            FaultInjecting::new(InMemoryBackend::new())
                .with_fault(Fault::on(FilesystemOperation::WriteFile).backend("nonce unavailable")),
        );
        let unavailable = IronhubLinkStateStore::new(filesystem);
        assert_eq!(
            unavailable
                .consume_install_nonce(&caller("user"), "nonce", Utc::now())
                .await,
            Err(IronhubLinkStateError::Unavailable)
        );

        let conflict_store = IronhubLinkStateStore::new(Arc::new(AlwaysCasConflictFilesystem));
        assert_eq!(
            conflict_store
                .record_private_manifest("catalog.example", "org/repo", Utc::now(), "digest")
                .await,
            Err(IronhubLinkStateError::Unavailable)
        );
        assert_eq!(
            conflict_store
                .record_public_manifest("https://catalog.example/manifest", Utc::now(), "digest")
                .await,
            Err(IronhubLinkStateError::Unavailable)
        );
    }

    #[tokio::test]
    async fn durable_manifest_state_rejects_corruption_and_accepts_monotonic_updates() {
        let filesystem = Arc::new(InMemoryBackend::new());
        let store = IronhubLinkStateStore::new(filesystem.clone());
        let generated_at = Utc::now();
        store
            .record_private_manifest("catalog.example", "org/repo", generated_at, "digest-a")
            .await
            .expect("initial private state");
        store
            .record_private_manifest(
                "catalog.example",
                "org/repo",
                generated_at + chrono::Duration::seconds(1),
                "digest-b",
            )
            .await
            .expect("newer private state");

        let private_path = manifest_path("catalog.example", "corrupt/repo").expect("state path");
        filesystem
            .put(
                &private_path,
                Entry::bytes(b"{}".to_vec()),
                CasExpectation::Absent,
            )
            .await
            .expect("seed corrupt private state");
        assert_eq!(
            store
                .record_private_manifest("catalog.example", "corrupt/repo", Utc::now(), "digest")
                .await,
            Err(IronhubLinkStateError::Unavailable)
        );

        let mismatch_path = manifest_path("catalog.example", "mismatch/repo").expect("state path");
        let mismatched = PrivateManifestState {
            catalog_host: "other.example".to_string(),
            signed_repo: "other/repo".to_string(),
            generated_at,
            signed_manifest_digest: "digest".to_string(),
        };
        filesystem
            .put(
                &mismatch_path,
                Entry::bytes(serde_json::to_vec(&mismatched).expect("state JSON")),
                CasExpectation::Absent,
            )
            .await
            .expect("seed mismatched private state");
        assert_eq!(
            store
                .record_private_manifest("catalog.example", "mismatch/repo", Utc::now(), "digest")
                .await,
            Err(IronhubLinkStateError::Unavailable)
        );

        let public_url = "https://catalog.example/public-manifest";
        store
            .record_public_manifest(public_url, generated_at, "digest-a")
            .await
            .expect("initial public state");
        store
            .record_public_manifest(
                public_url,
                generated_at + chrono::Duration::seconds(1),
                "digest-b",
            )
            .await
            .expect("newer public state");

        let corrupt_public_url = "https://catalog.example/corrupt-manifest";
        let public_path = public_manifest_path(corrupt_public_url).expect("state path");
        filesystem
            .put(
                &public_path,
                Entry::bytes(b"{}".to_vec()),
                CasExpectation::Absent,
            )
            .await
            .expect("seed corrupt public state");
        assert_eq!(
            store
                .record_public_manifest(corrupt_public_url, Utc::now(), "digest")
                .await,
            Err(IronhubLinkStateError::Unavailable)
        );
    }

    #[tokio::test]
    async fn private_manifest_key_ignores_rotating_url_token() {
        let filesystem = shared_filesystem();
        let first = IronhubLinkStateStore::new(Arc::clone(&filesystem));
        let generated_at = Utc::now();
        first
            .record_private_manifest("Catalog.Example.", "org/repo", generated_at, "digest-a")
            .await
            .expect("first manifest");

        let reconstructed = IronhubLinkStateStore::new(filesystem);
        assert_eq!(
            reconstructed
                .record_private_manifest("catalog.example", "org/repo", generated_at, "digest-a",)
                .await,
            Err(IronhubLinkStateError::ManifestReplay)
        );
    }

    #[tokio::test]
    async fn private_manifest_rejects_downgrade() {
        let store = IronhubLinkStateStore::new(shared_filesystem());
        let newer = Utc::now();
        store
            .record_private_manifest("catalog.example", "org/repo", newer, "digest-new")
            .await
            .expect("new manifest");

        assert_eq!(
            store
                .record_private_manifest(
                    "catalog.example",
                    "org/repo",
                    newer - chrono::Duration::seconds(1),
                    "digest-old",
                )
                .await,
            Err(IronhubLinkStateError::ManifestReplay)
        );
    }

    #[tokio::test]
    async fn public_manifest_replay_state_survives_store_reconstruction() {
        let filesystem = shared_filesystem();
        let first = IronhubLinkStateStore::new(Arc::clone(&filesystem));
        let manifest_url = "https://hub.ironclaw.com/api/catalog/manifest.json";
        let generated_at = Utc::now();
        first
            .record_public_manifest(manifest_url, generated_at, "digest-new")
            .await
            .expect("first manifest");

        let reconstructed = IronhubLinkStateStore::new(filesystem);
        reconstructed
            .record_public_manifest(manifest_url, generated_at, "digest-new")
            .await
            .expect("identical manifest remains idempotent");
        assert_eq!(
            reconstructed
                .record_public_manifest(
                    manifest_url,
                    generated_at - chrono::Duration::seconds(1),
                    "digest-old",
                )
                .await,
            Err(IronhubLinkStateError::ManifestReplay)
        );
        assert_eq!(
            reconstructed
                .record_public_manifest(manifest_url, generated_at, "digest-conflict")
                .await,
            Err(IronhubLinkStateError::ManifestReplay)
        );
    }
}
