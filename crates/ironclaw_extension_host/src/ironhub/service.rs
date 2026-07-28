use std::collections::HashMap;
use std::sync::{Arc, LazyLock};
use std::time::Instant;

use chrono::{DateTime, Utc};
use ironclaw_host_api::{
    CapabilityId, ExtensionId, InstallationState, InvocationId, NetworkMethod, ResourceScope,
    RuntimeHttpEgress, RuntimeHttpEgressError, RuntimeHttpEgressRequest, RuntimeHttpEgressResponse,
    RuntimeKind, TrustClass,
};
use ironclaw_host_runtime::{
    BUILTIN_FIRST_PARTY_PROVIDER, HostRuntimeHttpEgressPort, HostRuntimeHttpEgressRequest,
};
use ironclaw_product::{
    LifecyclePackageId, LifecyclePackageKind, LifecyclePackageRef, LifecycleProductPayload,
    LifecycleProductResponse, LifecycleProductSurfaceContext,
};
use ironclaw_skills::{
    ManagedSkillSource, ScopedSkillManagementError, ScopedSkillManagementPort,
    SkillManagementErrorKind,
};
use tokio::sync::Mutex as AsyncMutex;

use crate::ExtensionLifecycleManager;

use super::catalog::{
    catalog, classify, classify_gate_and_digest, compact_skill_summary, compact_tool_summary,
    entry_matches, invalid, network_policy_for_url, sha256_hex, skill_summary, tool_summary,
    validate_artifact, validate_artifact_url, validate_hub_name, validate_manifest,
    verify_signed_manifest,
};
use super::model::{
    DEFAULT_IRONHUB_MANIFEST_URL, IronHubArtifact, IronHubCommand, IronHubCommandError,
    IronHubEntryKind, IronHubInstallOptions, IronHubManifest, IronHubPhase, IronHubProvenance,
    IronHubResponse, MANIFEST_CACHE_MAX_ENTRIES, MANIFEST_CACHE_TTL, MAX_MANIFEST_BYTES,
    MAX_METADATA_BYTES, MAX_SIGNED_MANIFEST_BYTES, MAX_WASM_BYTES,
};
use super::package::ironhub_tool_package;

struct CachedManifest {
    manifest: Arc<IronHubManifest>,
    fetched_at: Instant,
}

static MANIFEST_CACHE: LazyLock<std::sync::Mutex<HashMap<String, CachedManifest>>> =
    LazyLock::new(|| std::sync::Mutex::new(HashMap::new()));
static MANIFEST_FETCH_LOCKS: LazyLock<std::sync::Mutex<HashMap<String, Arc<AsyncMutex<()>>>>> =
    LazyLock::new(|| std::sync::Mutex::new(HashMap::new()));
static MANIFEST_LAST_SEEN: LazyLock<std::sync::Mutex<HashMap<String, DateTime<Utc>>>> =
    LazyLock::new(|| std::sync::Mutex::new(HashMap::new()));
static INSTALL_LOCKS: LazyLock<std::sync::Mutex<HashMap<String, Arc<AsyncMutex<()>>>>> =
    LazyLock::new(|| std::sync::Mutex::new(HashMap::new()));

pub trait RebornIronHubRuntime {
    fn ironhub_skill_management(&self) -> Arc<ScopedSkillManagementPort>;
    fn ironhub_extension_management(&self) -> Arc<ExtensionLifecycleManager>;
    fn ironhub_host_runtime_http_egress(&self) -> Option<HostRuntimeHttpEgressPort>;
    fn ironhub_surface_context(&self) -> LifecycleProductSurfaceContext;
}

pub async fn execute_reborn_ironhub_command(
    runtime: &impl RebornIronHubRuntime,
    command: IronHubCommand,
) -> Result<IronHubResponse, IronHubCommandError> {
    let egress = runtime
        .ironhub_host_runtime_http_egress()
        .ok_or(IronHubCommandError::RuntimeHttpEgressUnavailable)?;
    let context = runtime.ironhub_surface_context();
    let scope = ResourceScope {
        tenant_id: context.tenant_id,
        user_id: context.user_id,
        agent_id: context.agent_id,
        project_id: context.project_id,
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    };
    let service = IronHubService::new_with_host_egress(
        runtime.ironhub_skill_management(),
        runtime.ironhub_extension_management(),
        egress,
        scope,
        ironhub_command_capability_id(&command)?,
    );
    service.execute(command).await
}

pub async fn execute_reborn_ironhub_service_command(
    skill_management: Arc<ScopedSkillManagementPort>,
    extension_management: Arc<ExtensionLifecycleManager>,
    runtime_http_egress: Arc<dyn RuntimeHttpEgress>,
    scope: ResourceScope,
    command: IronHubCommand,
) -> Result<IronHubResponse, IronHubCommandError> {
    let capability_id = ironhub_command_capability_id(&command)?;
    IronHubService::new_with_runtime_egress(
        skill_management,
        extension_management,
        runtime_http_egress,
        scope,
        capability_id,
    )
    .execute(command)
    .await
}

enum IronHubEgress {
    Host {
        port: HostRuntimeHttpEgressPort,
        capability_id: CapabilityId,
    },
    Runtime {
        egress: Arc<dyn RuntimeHttpEgress>,
        capability_id: CapabilityId,
    },
}

impl IronHubEgress {
    fn capability_id(&self) -> CapabilityId {
        match self {
            Self::Host { capability_id, .. } | Self::Runtime { capability_id, .. } => {
                capability_id.clone()
            }
        }
    }

    async fn execute(
        &self,
        request: RuntimeHttpEgressRequest,
    ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
        match self {
            Self::Host { port, .. } => {
                let extension_id =
                    ExtensionId::new(BUILTIN_FIRST_PARTY_PROVIDER).map_err(|error| {
                        RuntimeHttpEgressError::Request {
                            reason: format!("invalid builtin provider id: {error}"),
                            request_bytes: 0,
                            response_bytes: 0,
                        }
                    })?;
                port.execute(HostRuntimeHttpEgressRequest {
                    extension_id,
                    trust: TrustClass::FirstParty,
                    request,
                    credentials: Vec::new(),
                })
                .await
            }
            Self::Runtime { egress, .. } => egress.execute(request).await,
        }
    }
}

pub(crate) struct IronHubService {
    skill_management: Arc<ScopedSkillManagementPort>,
    extension_management: Arc<ExtensionLifecycleManager>,
    egress: IronHubEgress,
    scope: ResourceScope,
    manifest_url: String,
    verify_keys: &'static [(&'static str, &'static str)],
}

impl IronHubService {
    fn new(
        skill_management: Arc<ScopedSkillManagementPort>,
        extension_management: Arc<ExtensionLifecycleManager>,
        egress: IronHubEgress,
        scope: ResourceScope,
    ) -> Self {
        Self {
            skill_management,
            extension_management,
            egress,
            scope,
            manifest_url: resolve_manifest_url(),
            verify_keys: super::model::MANIFEST_VERIFY_KEYS,
        }
    }

    pub(crate) fn new_with_runtime_egress(
        skill_management: Arc<ScopedSkillManagementPort>,
        extension_management: Arc<ExtensionLifecycleManager>,
        egress: Arc<dyn RuntimeHttpEgress>,
        scope: ResourceScope,
        capability_id: CapabilityId,
    ) -> Self {
        Self::new(
            skill_management,
            extension_management,
            IronHubEgress::Runtime {
                egress,
                capability_id,
            },
            scope,
        )
    }

    fn new_with_host_egress(
        skill_management: Arc<ScopedSkillManagementPort>,
        extension_management: Arc<ExtensionLifecycleManager>,
        port: HostRuntimeHttpEgressPort,
        scope: ResourceScope,
        capability_id: CapabilityId,
    ) -> Self {
        Self::new(
            skill_management,
            extension_management,
            IronHubEgress::Host {
                port,
                capability_id,
            },
            scope,
        )
    }

    pub(crate) async fn execute(
        &self,
        command: IronHubCommand,
    ) -> Result<IronHubResponse, IronHubCommandError> {
        match command {
            IronHubCommand::Search { query } => self.search(&query).await,
            IronHubCommand::List { kind } => self.list(kind).await,
            IronHubCommand::Info { name, kind } => self.info(&name, kind).await,
            IronHubCommand::Install { name, options } => self.install(&name, options).await,
        }
    }

    async fn search(&self, query: &str) -> Result<IronHubResponse, IronHubCommandError> {
        let manifest = self.fetch_manifest_cached().await?;
        let query = query.trim().to_ascii_lowercase();
        let mut entries = manifest
            .tools
            .iter()
            .filter(|entry| entry_matches(&entry.name, &entry.description, &query))
            .map(compact_tool_summary)
            .collect::<Vec<_>>();
        entries.extend(
            manifest
                .skills
                .iter()
                .filter(|entry| entry_matches(&entry.name, &entry.description, &query))
                .map(compact_skill_summary),
        );
        IronHubResponse::discovered_catalog(entries)
    }

    async fn list(
        &self,
        kind: Option<IronHubEntryKind>,
    ) -> Result<IronHubResponse, IronHubCommandError> {
        let manifest = self.fetch_manifest_cached().await?;
        let mut entries = Vec::new();
        if kind != Some(IronHubEntryKind::Skill) {
            entries.extend(manifest.tools.iter().map(compact_tool_summary));
        }
        if kind != Some(IronHubEntryKind::Tool) {
            entries.extend(manifest.skills.iter().map(compact_skill_summary));
        }
        IronHubResponse::discovered_catalog(entries)
    }

    async fn info(
        &self,
        name: &str,
        hint: Option<IronHubEntryKind>,
    ) -> Result<IronHubResponse, IronHubCommandError> {
        validate_hub_name(name)?;
        let manifest = self.fetch_manifest_cached().await?;
        let entry = match classify(&manifest, name, hint)? {
            IronHubEntryKind::Tool => tool_summary(
                manifest
                    .find_tool(name)
                    .ok_or_else(|| catalog("tool not found"))?,
            ),
            IronHubEntryKind::Skill => skill_summary(
                manifest
                    .find_skill(name)
                    .ok_or_else(|| catalog("skill not found"))?,
            ),
        };
        Ok(IronHubResponse::discovered(vec![entry]))
    }

    async fn install(
        &self,
        name: &str,
        options: IronHubInstallOptions,
    ) -> Result<IronHubResponse, IronHubCommandError> {
        validate_hub_name(name)?;
        let manifest = self.fetch_manifest_cached().await?;
        let (kind, provenance, artifact_digest) =
            classify_gate_and_digest(&manifest, name, options.kind, &options)?;
        let lock_key = format!("{}:{name}", kind.as_str());
        let lock = install_lock(&lock_key);
        let result = async {
            let _guard = lock.lock().await;
            let lifecycle = match kind {
                IronHubEntryKind::Skill => {
                    let entry = manifest
                        .find_skill(name)
                        .ok_or_else(|| catalog("skill not found"))?;
                    let content = self
                        .download_verified(&entry.skill_md, MAX_METADATA_BYTES)
                        .await?;
                    let content = String::from_utf8(content).map_err(|error| {
                        IronHubCommandError::Install {
                            reason: format!("skill markdown is not UTF-8: {error}"),
                        }
                    })?;
                    let installed = self
                        .install_skill(
                            entry.name.as_str(),
                            &content,
                            &entry.skill_md.url,
                            options.force,
                        )
                        .await?;
                    LifecycleProductResponse {
                        package_ref: Some(
                            LifecyclePackageRef::new(
                                LifecyclePackageKind::Skill,
                                installed.name.as_str(),
                            )
                            .map_err(|error| invalid(error.to_string()))?,
                        ),
                        phase: InstallationState::Installed,
                        blockers: Vec::new(),
                        message: None,
                        payload: Some(LifecycleProductPayload::SkillInstall {
                            installed: true,
                            name: LifecyclePackageId::new(installed.name)
                                .map_err(|error| invalid(error.to_string()))?,
                        }),
                    }
                }
                IronHubEntryKind::Tool => {
                    let entry = manifest
                        .find_tool(name)
                        .ok_or_else(|| catalog("tool not found"))?;
                    let wasm = self.download_verified(&entry.wasm, MAX_WASM_BYTES).await?;
                    let capabilities = self
                        .download_verified(&entry.capabilities, MAX_METADATA_BYTES)
                        .await?;
                    let reserved = self
                        .extension_management
                        .reserved_bundled_extension_ids()
                        .await;
                    let package = ironhub_tool_package(entry, wasm, capabilities, &reserved)?;
                    self.extension_management
                        .install_registry_package(
                            package,
                            options.force,
                            &self.scope.user_id,
                            &self.scope,
                        )
                        .await?
                }
            };
            let entry = match kind {
                IronHubEntryKind::Tool => tool_summary(
                    manifest
                        .find_tool(name)
                        .ok_or_else(|| catalog("tool not found"))?,
                ),
                IronHubEntryKind::Skill => skill_summary(
                    manifest
                        .find_skill(name)
                        .ok_or_else(|| catalog("skill not found"))?,
                ),
            };
            Ok(IronHubResponse {
                phase: IronHubPhase::Installed,
                total_entries: 1,
                returned_entries: 1,
                truncated: false,
                message: Some(install_message(
                    kind,
                    name,
                    &entry_version(&manifest, kind, name)?,
                    provenance,
                    &artifact_digest,
                )),
                entries: vec![entry],
                lifecycle: Some(lifecycle),
            })
        }
        .await;
        drop(lock);
        evict_idle_async_locks(&INSTALL_LOCKS);
        result
    }

    async fn install_skill(
        &self,
        name: &str,
        content: &str,
        source_url: &str,
        force: bool,
    ) -> Result<ironclaw_skills::SkillInstallResult, IronHubCommandError> {
        let first = self
            .skill_management
            .install_from_url_for_scope(self.scope.clone(), Some(name), content, source_url)
            .await;
        let Err(error) = first else {
            return first.map_err(skill_install_error);
        };
        if !force || !is_skill_conflict(&error) {
            return Err(skill_install_error(error));
        }
        let previous = self
            .skill_management
            .read_content_for_scope(self.scope.clone(), name)
            .await
            .map_err(skill_install_error)?;
        let previous_source_url = match previous.source {
            ManagedSkillSource::Installed => Some(previous.source_url.as_deref().ok_or_else(|| {
                IronHubCommandError::Install {
                    reason: format!(
                        "cannot force-replace installed skill '{name}' because its source URL is unavailable"
                    ),
                }
            })?),
            ManagedSkillSource::User => None,
            ManagedSkillSource::System => {
                return Err(IronHubCommandError::Install {
                    reason: format!("cannot force-replace system skill '{name}'"),
                });
            }
        };
        self.skill_management
            .remove_for_scope(self.scope.clone(), name)
            .await
            .map_err(skill_install_error)?;
        match self
            .skill_management
            .install_from_url_for_scope(self.scope.clone(), Some(name), content, source_url)
            .await
        {
            Ok(result) => Ok(result),
            Err(original_error) => {
                let restore = match previous_source_url {
                    Some(source_url) => {
                        self.skill_management
                            .install_from_url_for_scope(
                                self.scope.clone(),
                                Some(name),
                                &previous.content,
                                source_url,
                            )
                            .await
                    }
                    None => {
                        self.skill_management
                            .install_for_scope(self.scope.clone(), Some(name), &previous.content)
                            .await
                    }
                };
                if let Err(restore_error) = restore {
                    return Err(IronHubCommandError::Install {
                        reason: format!(
                            "forced skill replacement failed ({original_error}); previous skill restoration also failed ({restore_error})"
                        ),
                    });
                }
                Err(skill_install_error(original_error))
            }
        }
    }

    async fn fetch_manifest_cached(&self) -> Result<Arc<IronHubManifest>, IronHubCommandError> {
        let now = Instant::now();
        if let Some(hit) = manifest_cache_get(&self.manifest_url, now) {
            return Ok(hit);
        }
        let lock = manifest_fetch_lock(&self.manifest_url);
        let result = async {
            let _guard = lock.lock().await;
            let now = Instant::now();
            if let Some(hit) = manifest_cache_get(&self.manifest_url, now) {
                return Ok(hit);
            }
            let manifest = Arc::new(self.fetch_manifest().await?);
            manifest_cache_put(&self.manifest_url, Arc::clone(&manifest), now);
            Ok(manifest)
        }
        .await;
        drop(lock);
        evict_idle_async_locks(&MANIFEST_FETCH_LOCKS);
        result
    }

    async fn fetch_manifest(&self) -> Result<IronHubManifest, IronHubCommandError> {
        validate_artifact_url("hub-manifest", "manifest_url", &self.manifest_url)?;
        let envelope = self
            .download_url(&self.manifest_url, MAX_SIGNED_MANIFEST_BYTES)
            .await?;
        let bytes = if self.verify_keys == super::model::MANIFEST_VERIFY_KEYS {
            verify_signed_manifest(&envelope)
        } else {
            super::catalog::verify_signed_manifest_with_keys(&envelope, self.verify_keys)
        }
        .map_err(|reason| IronHubCommandError::Catalog {
            reason: format!("signed manifest verification failed: {reason}"),
        })?;
        if bytes.len() > usize::try_from(MAX_MANIFEST_BYTES).unwrap_or(usize::MAX) {
            return Err(catalog("manifest exceeds size cap"));
        }
        let manifest: IronHubManifest =
            serde_json::from_slice(&bytes).map_err(|error| IronHubCommandError::Catalog {
                reason: format!("manifest parse failed: {error}"),
            })?;
        validate_manifest(&manifest)?;
        enforce_manifest_monotonic(&self.manifest_url, &manifest)?;
        Ok(manifest)
    }

    async fn download_verified(
        &self,
        artifact: &IronHubArtifact,
        max_bytes: u64,
    ) -> Result<Vec<u8>, IronHubCommandError> {
        validate_artifact(artifact, max_bytes)?;
        let bytes = self
            .download_url(&artifact.url, artifact.size_bytes)
            .await?;
        if u64::try_from(bytes.len()).unwrap_or(u64::MAX) != artifact.size_bytes {
            return Err(IronHubCommandError::Install {
                reason: format!(
                    "size mismatch for {}: expected {} bytes, got {}",
                    artifact.url,
                    artifact.size_bytes,
                    bytes.len()
                ),
            });
        }
        let actual = sha256_hex(&bytes);
        if !actual.eq_ignore_ascii_case(&artifact.sha256) {
            return Err(IronHubCommandError::Install {
                reason: format!(
                    "checksum mismatch for {}: expected {}, got {}",
                    artifact.url, artifact.sha256, actual
                ),
            });
        }
        Ok(bytes)
    }

    async fn download_url(
        &self,
        url: &str,
        max_bytes: u64,
    ) -> Result<Vec<u8>, IronHubCommandError> {
        let request = RuntimeHttpEgressRequest {
            runtime: RuntimeKind::FirstParty,
            scope: self.scope.clone(),
            capability_id: self.egress.capability_id(),
            method: NetworkMethod::Get,
            url: url.to_string(),
            headers: Vec::new(),
            body: Vec::new(),
            network_policy: network_policy_for_url(url, max_bytes)?,
            credential_injections: Vec::new(),
            response_body_limit: Some(max_bytes),
            save_body_to: None,
            timeout_ms: Some(30_000),
        };
        let response =
            self.egress
                .execute(request)
                .await
                .map_err(|error| IronHubCommandError::Catalog {
                    reason: error.stable_runtime_reason().to_string(),
                })?;
        if !(200..300).contains(&response.status) {
            return Err(catalog(format!(
                "download returned HTTP {}",
                response.status
            )));
        }
        if response.body.len() > usize::try_from(max_bytes).unwrap_or(usize::MAX) {
            return Err(catalog("download exceeded response size cap"));
        }
        Ok(response.body)
    }
}

#[cfg(test)]
pub(crate) fn configure_test_catalog(
    mut service: IronHubService,
    manifest_url: impl Into<String>,
    verify_keys: &'static [(&'static str, &'static str)],
) -> IronHubService {
    service.manifest_url = manifest_url.into();
    service.verify_keys = verify_keys;
    service
}

#[cfg(test)]
pub(crate) fn clear_test_manifest_cache(url: &str) {
    MANIFEST_CACHE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .remove(url);
}

#[cfg(test)]
pub(crate) fn test_manifest_fetch_lock_exists(url: &str) -> bool {
    MANIFEST_FETCH_LOCKS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .contains_key(url)
}

#[cfg(test)]
pub(crate) fn test_install_lock_exists(key: &str) -> bool {
    INSTALL_LOCKS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .contains_key(key)
}

fn ironhub_command_capability_id(
    command: &IronHubCommand,
) -> Result<CapabilityId, IronHubCommandError> {
    let value = match command {
        IronHubCommand::Search { .. } | IronHubCommand::List { .. } => {
            super::capabilities::IRONHUB_SEARCH_CAPABILITY_ID
        }
        IronHubCommand::Info { .. } => super::capabilities::IRONHUB_INFO_CAPABILITY_ID,
        IronHubCommand::Install { .. } => super::capabilities::IRONHUB_INSTALL_CAPABILITY_ID,
    };
    CapabilityId::new(value).map_err(|error| invalid(error.to_string()))
}

fn entry_version(
    manifest: &IronHubManifest,
    kind: IronHubEntryKind,
    name: &str,
) -> Result<String, IronHubCommandError> {
    match kind {
        IronHubEntryKind::Tool => manifest
            .find_tool(name)
            .map(|entry| entry.version.clone())
            .ok_or_else(|| catalog("tool not found")),
        IronHubEntryKind::Skill => manifest
            .find_skill(name)
            .map(|entry| entry.version.clone())
            .ok_or_else(|| catalog("skill not found")),
    }
}

fn is_skill_conflict(error: &ScopedSkillManagementError) -> bool {
    matches!(
        error,
        ScopedSkillManagementError::Skill(error)
            if error.kind() == SkillManagementErrorKind::Conflict
    )
}

fn skill_install_error(error: ScopedSkillManagementError) -> IronHubCommandError {
    IronHubCommandError::Install {
        reason: error.to_string(),
    }
}

fn resolve_manifest_url() -> String {
    std::env::var("IRONHUB_MANIFEST_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_IRONHUB_MANIFEST_URL.to_string())
}

fn manifest_cache_get(url: &str, now: Instant) -> Option<Arc<IronHubManifest>> {
    let mut guard = MANIFEST_CACHE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    guard.retain(|_, entry| now.duration_since(entry.fetched_at) <= MANIFEST_CACHE_TTL);
    guard.get(url).map(|entry| Arc::clone(&entry.manifest))
}

fn manifest_cache_put(url: &str, manifest: Arc<IronHubManifest>, now: Instant) {
    let mut guard = MANIFEST_CACHE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    guard.retain(|_, entry| now.duration_since(entry.fetched_at) <= MANIFEST_CACHE_TTL);
    if guard.len() >= MANIFEST_CACHE_MAX_ENTRIES
        && let Some(victim) = guard.keys().next().cloned()
    {
        guard.remove(&victim);
    }
    guard.insert(
        url.to_string(),
        CachedManifest {
            manifest,
            fetched_at: now,
        },
    );
}

fn manifest_fetch_lock(url: &str) -> Arc<AsyncMutex<()>> {
    let mut guard = MANIFEST_FETCH_LOCKS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    guard
        .entry(url.to_string())
        .or_insert_with(|| Arc::new(AsyncMutex::new(())))
        .clone()
}

fn evict_idle_async_locks(locks: &std::sync::Mutex<HashMap<String, Arc<AsyncMutex<()>>>>) {
    let mut guard = locks
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    // Once an operation releases its async guard and drops its local Arc,
    // the map is the sole owner. Remove those idle entries; live/waiting
    // operations retain an Arc and therefore keep their shared lock.
    guard.retain(|_, lock| Arc::strong_count(lock) > 1);
}

fn enforce_manifest_monotonic(
    url: &str,
    manifest: &IronHubManifest,
) -> Result<(), IronHubCommandError> {
    let generated_at = DateTime::parse_from_rfc3339(&manifest.generated_at)
        .map_err(|error| catalog(format!("manifest generated_at is not RFC3339: {error}")))?
        .with_timezone(&Utc);
    let mut guard = MANIFEST_LAST_SEEN
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(previous) = guard.get(url)
        && generated_at < *previous
    {
        return Err(catalog(format!(
            "signed manifest replay rejected: generated_at {} is older than last seen {}",
            generated_at.to_rfc3339(),
            previous.to_rfc3339()
        )));
    }
    if !guard.contains_key(url) && guard.len() >= MANIFEST_CACHE_MAX_ENTRIES {
        return Err(catalog(
            "manifest replay tracking capacity exceeded; refusing untracked manifest URL",
        ));
    }
    guard.insert(url.to_string(), generated_at);
    Ok(())
}

fn install_lock(key: &str) -> Arc<AsyncMutex<()>> {
    let mut guard = INSTALL_LOCKS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    guard
        .entry(key.to_string())
        .or_insert_with(|| Arc::new(AsyncMutex::new(())))
        .clone()
}

fn install_message(
    kind: IronHubEntryKind,
    name: &str,
    version: &str,
    provenance: IronHubProvenance,
    artifact_digest: &str,
) -> String {
    format!(
        "installed {} '{}' {} from IronHub; provenance={}, artifact_digest={}",
        kind.as_str(),
        name,
        version,
        provenance.as_wire(),
        artifact_digest
    )
}
