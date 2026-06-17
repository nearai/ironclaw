use std::collections::HashMap;
use std::sync::{Arc, LazyLock};
use std::time::Instant;

use chrono::{DateTime, Utc};
use ironclaw_common::hashing::sha256_hex;
use ironclaw_host_api::{
    CapabilityId, ExtensionId, InvocationId, NetworkMethod, ResourceScope, RuntimeHttpEgress,
    RuntimeHttpEgressError, RuntimeHttpEgressRequest, RuntimeHttpEgressResponse, RuntimeKind,
    TrustClass, UserId,
};
use ironclaw_host_runtime::{
    BUILTIN_FIRST_PARTY_PROVIDER, HostRuntimeHttpEgressPort, HostRuntimeHttpEgressRequest,
};
use ironclaw_product_workflow::{
    LifecyclePackageId, LifecyclePackageKind, LifecyclePhase, LifecycleProductPayload,
    LifecycleProductResponse,
};
use tokio::sync::Mutex as AsyncMutex;

use crate::extension_lifecycle::RebornLocalExtensionManagementPort;
use crate::factory::RebornServices;
use crate::lifecycle::{
    RebornLocalSkillManagementError, RebornLocalSkillManagementPort, response_with_payload,
};

#[cfg(not(test))]
use super::catalog::verify_signed_manifest;
use super::catalog::{
    classify, classify_gate_and_digest, entry_matches, network_policy_for_url, package_ref,
    skill_summary, tool_summary, validate_artifact, validate_artifact_url, validate_hub_name,
};
use super::errors::{catalog_error, invalid_input, product_error};
use super::model::{
    DEFAULT_IRONHUB_MANIFEST_URL, IronHubArtifact, IronHubCommand, IronHubCommandError,
    IronHubEntryKind, IronHubInstallOptions, IronHubManifest, IronHubProvenance,
    MANIFEST_CACHE_MAX_ENTRIES, MANIFEST_CACHE_TTL, MAX_MANIFEST_BYTES, MAX_METADATA_BYTES,
    MAX_SIGNED_MANIFEST_BYTES, MAX_WASM_BYTES,
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

pub async fn execute_reborn_ironhub_command(
    services: &RebornServices,
    command: IronHubCommand,
) -> Result<LifecycleProductResponse, IronHubCommandError> {
    let local_runtime = services
        .local_runtime
        .as_ref()
        .ok_or(IronHubCommandError::LocalRuntimeUnavailable)?;
    let extension_management = local_runtime
        .extension_management
        .as_ref()
        .ok_or(IronHubCommandError::LocalRuntimeUnavailable)?;
    let host_runtime_http_egress = local_runtime
        .host_runtime_http_egress
        .as_ref()
        .ok_or(IronHubCommandError::RuntimeHttpEgressUnavailable)?;
    let scope = ResourceScope::local_default(
        UserId::new("reborn-cli").map_err(invalid_input)?,
        InvocationId::new(),
    )
    .map_err(invalid_input)?;
    let capability_id = CapabilityId::new("builtin.ironhub_fetch").map_err(invalid_input)?;
    let service = IronHubService::new_with_host_egress(
        Arc::clone(&local_runtime.skill_management),
        Arc::clone(extension_management),
        host_runtime_http_egress.clone(),
        capability_id,
        scope,
    );
    service.execute(command).await
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
                port.execute(HostRuntimeHttpEgressRequest {
                    extension_id: ExtensionId::new(BUILTIN_FIRST_PARTY_PROVIDER).map_err(
                        |error| RuntimeHttpEgressError::Request {
                            reason: format!("invalid builtin provider id: {error}"),
                            request_bytes: 0,
                            response_bytes: 0,
                        },
                    )?,
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
    skill_management: Arc<RebornLocalSkillManagementPort>,
    extension_management: Arc<RebornLocalExtensionManagementPort>,
    egress: IronHubEgress,
    scope: ResourceScope,
    manifest_url: String,
    #[cfg(test)]
    manifest_verify_keys: &'static [(&'static str, &'static str)],
}

impl IronHubService {
    pub(crate) fn new_with_host_egress(
        skill_management: Arc<RebornLocalSkillManagementPort>,
        extension_management: Arc<RebornLocalExtensionManagementPort>,
        host_runtime_http_egress: HostRuntimeHttpEgressPort,
        capability_id: CapabilityId,
        scope: ResourceScope,
    ) -> Self {
        Self::new(
            skill_management,
            extension_management,
            IronHubEgress::Host {
                port: host_runtime_http_egress,
                capability_id,
            },
            scope,
        )
    }

    pub(crate) fn new_with_runtime_egress(
        skill_management: Arc<RebornLocalSkillManagementPort>,
        extension_management: Arc<RebornLocalExtensionManagementPort>,
        runtime_http_egress: Arc<dyn RuntimeHttpEgress>,
        capability_id: CapabilityId,
        scope: ResourceScope,
    ) -> Self {
        Self::new(
            skill_management,
            extension_management,
            IronHubEgress::Runtime {
                egress: runtime_http_egress,
                capability_id,
            },
            scope,
        )
    }

    fn new(
        skill_management: Arc<RebornLocalSkillManagementPort>,
        extension_management: Arc<RebornLocalExtensionManagementPort>,
        egress: IronHubEgress,
        scope: ResourceScope,
    ) -> Self {
        Self {
            skill_management,
            extension_management,
            egress,
            scope,
            manifest_url: resolve_manifest_url(),
            #[cfg(test)]
            manifest_verify_keys: super::model::MANIFEST_VERIFY_KEYS,
        }
    }

    #[cfg(test)]
    pub(crate) fn with_manifest_url(mut self, manifest_url: impl Into<String>) -> Self {
        self.manifest_url = manifest_url.into();
        self
    }

    #[cfg(test)]
    pub(crate) fn with_manifest_verify_keys(
        mut self,
        manifest_verify_keys: &'static [(&'static str, &'static str)],
    ) -> Self {
        self.manifest_verify_keys = manifest_verify_keys;
        self
    }

    pub(crate) async fn execute(
        &self,
        command: IronHubCommand,
    ) -> Result<LifecycleProductResponse, IronHubCommandError> {
        match command {
            IronHubCommand::Search { query } => self.search(&query).await,
            IronHubCommand::List { kind } => self.list(kind).await,
            IronHubCommand::Info { name, kind } => self.info(&name, kind).await,
            IronHubCommand::Install { name, options } => self.install(&name, options).await,
        }
    }

    async fn search(&self, query: &str) -> Result<LifecycleProductResponse, IronHubCommandError> {
        let manifest = self.fetch_manifest_cached().await?;
        let query = query.trim().to_ascii_lowercase();
        let tools = manifest
            .tools
            .iter()
            .filter(|entry| entry_matches(&entry.name, &entry.description, &query))
            .map(tool_summary)
            .collect::<Result<Vec<_>, _>>()?;
        let skills = manifest
            .skills
            .iter()
            .filter(|entry| entry_matches(&entry.name, &entry.description, &query))
            .map(skill_summary)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(response_with_payload(
            None,
            LifecyclePhase::Discovered,
            LifecycleProductPayload::CatalogSearch {
                count: tools.len() + skills.len(),
                tools,
                skills,
            },
        ))
    }

    async fn list(
        &self,
        kind: Option<IronHubEntryKind>,
    ) -> Result<LifecycleProductResponse, IronHubCommandError> {
        let manifest = self.fetch_manifest_cached().await?;
        match kind {
            Some(IronHubEntryKind::Skill) => {
                let skills = manifest
                    .skills
                    .iter()
                    .map(skill_summary)
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(response_with_payload(
                    None,
                    LifecyclePhase::Discovered,
                    LifecycleProductPayload::SkillSearch {
                        count: skills.len(),
                        limit: skills.len(),
                        truncated: false,
                        skills,
                    },
                ))
            }
            None => {
                let tools = manifest
                    .tools
                    .iter()
                    .map(tool_summary)
                    .collect::<Result<Vec<_>, _>>()?;
                let skills = manifest
                    .skills
                    .iter()
                    .map(skill_summary)
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(response_with_payload(
                    None,
                    LifecyclePhase::Discovered,
                    LifecycleProductPayload::CatalogSearch {
                        count: tools.len() + skills.len(),
                        tools,
                        skills,
                    },
                ))
            }
            Some(IronHubEntryKind::Tool) => {
                let tools = manifest
                    .tools
                    .iter()
                    .map(tool_summary)
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(response_with_payload(
                    None,
                    LifecyclePhase::Discovered,
                    LifecycleProductPayload::ExtensionSearch {
                        count: tools.len(),
                        extensions: tools,
                    },
                ))
            }
        }
    }

    async fn info(
        &self,
        name: &str,
        hint: Option<IronHubEntryKind>,
    ) -> Result<LifecycleProductResponse, IronHubCommandError> {
        validate_hub_name(name)?;
        let manifest = self.fetch_manifest_cached().await?;
        let kind = classify(&manifest, name, hint)?;
        let response = match kind {
            IronHubEntryKind::Tool => {
                let tool = manifest
                    .find_tool(name)
                    .ok_or_else(|| catalog_error("tool not found"))?;
                response_with_payload(
                    Some(package_ref(LifecyclePackageKind::Extension, &tool.name)?),
                    LifecyclePhase::Discovered,
                    LifecycleProductPayload::ExtensionSearch {
                        extensions: vec![tool_summary(tool)?],
                        count: 1,
                    },
                )
            }
            IronHubEntryKind::Skill => {
                let skill = manifest
                    .find_skill(name)
                    .ok_or_else(|| catalog_error("skill not found"))?;
                response_with_payload(
                    Some(package_ref(LifecyclePackageKind::Skill, &skill.name)?),
                    LifecyclePhase::Discovered,
                    LifecycleProductPayload::SkillSearch {
                        skills: vec![skill_summary(skill)?],
                        count: 1,
                        limit: 1,
                        truncated: false,
                    },
                )
            }
        };
        Ok(response)
    }

    async fn install(
        &self,
        name: &str,
        options: IronHubInstallOptions,
    ) -> Result<LifecycleProductResponse, IronHubCommandError> {
        validate_hub_name(name)?;
        let manifest = self.fetch_manifest_cached().await?;
        let (kind, provenance, artifact_digest) =
            classify_gate_and_digest(&manifest, name, options.kind, &options)?;
        let lock_key = format!("{}:{name}", kind.as_str());
        let lock = install_lock(&lock_key);
        let _guard = lock.lock().await;
        match kind {
            IronHubEntryKind::Skill => {
                let entry = manifest
                    .find_skill(name)
                    .ok_or_else(|| catalog_error("skill not found"))?;
                let content = self
                    .download_verified(&entry.skill_md, MAX_METADATA_BYTES)
                    .await?;
                let content =
                    String::from_utf8(content).map_err(|error| IronHubCommandError::Install {
                        reason: format!("skill markdown is not UTF-8: {error}"),
                    })?;
                let mut result = self
                    .skill_management
                    .install_from_url(Some(&entry.name), &content, &entry.skill_md.url)
                    .await;
                if options.force && matches!(result, Err(ref error) if is_skill_conflict(error)) {
                    self.skill_management
                        .remove_if_installed(&entry.name)
                        .await
                        .map_err(|error| IronHubCommandError::Install {
                            reason: error.to_string(),
                        })?;
                    result = self
                        .skill_management
                        .install_from_url(Some(&entry.name), &content, &entry.skill_md.url)
                        .await;
                }
                let result = result.map_err(|error| IronHubCommandError::Install {
                    reason: error.to_string(),
                })?;
                let mut response = response_with_payload(
                    Some(package_ref(LifecyclePackageKind::Skill, &result.name)?),
                    LifecyclePhase::Installed,
                    LifecycleProductPayload::SkillInstall {
                        installed: true,
                        name: LifecyclePackageId::new(result.name).map_err(product_error)?,
                    },
                );
                response.message = Some(install_message(
                    IronHubEntryKind::Skill,
                    name,
                    entry.version.as_str(),
                    provenance,
                    &artifact_digest,
                ));
                Ok(response)
            }
            IronHubEntryKind::Tool => {
                let entry = manifest
                    .find_tool(name)
                    .ok_or_else(|| catalog_error("tool not found"))?;
                let wasm = self.download_verified(&entry.wasm, MAX_WASM_BYTES).await?;
                let capabilities = self
                    .download_verified(&entry.capabilities, MAX_METADATA_BYTES)
                    .await?;
                let package = ironhub_tool_package(entry, &wasm, &capabilities)?;
                let mut response = self
                    .extension_management
                    .install_available_package(&package, options.force)
                    .await
                    .map_err(IronHubCommandError::Product)?;
                response.message = Some(install_message(
                    IronHubEntryKind::Tool,
                    name,
                    entry.version.as_str(),
                    provenance,
                    &artifact_digest,
                ));
                Ok(response)
            }
        }
    }

    async fn fetch_manifest_cached(&self) -> Result<Arc<IronHubManifest>, IronHubCommandError> {
        let now = Instant::now();
        if let Some(hit) = manifest_cache_get(&self.manifest_url, now) {
            return Ok(hit);
        }
        let fetch_lock = manifest_fetch_lock(&self.manifest_url);
        let _fetch_guard = fetch_lock.lock().await;
        let now = Instant::now();
        if let Some(hit) = manifest_cache_get(&self.manifest_url, now) {
            return Ok(hit);
        }
        let manifest = Arc::new(self.fetch_manifest().await?);
        manifest_cache_put(&self.manifest_url, Arc::clone(&manifest), now);
        Ok(manifest)
    }

    async fn fetch_manifest(&self) -> Result<IronHubManifest, IronHubCommandError> {
        validate_artifact_url("hub-manifest", "manifest_url", &self.manifest_url)?;
        let envelope = self
            .download_url(&self.manifest_url, MAX_SIGNED_MANIFEST_BYTES)
            .await?;
        #[cfg(not(test))]
        let verified_manifest = verify_signed_manifest(&envelope);
        #[cfg(test)]
        let verified_manifest =
            super::catalog::verify_signed_manifest_with_keys(&envelope, self.manifest_verify_keys);
        let bytes = verified_manifest.map_err(|reason| IronHubCommandError::Catalog {
            reason: format!("signed manifest verification failed: {reason}"),
        })?;
        if bytes.len() > usize::try_from(MAX_MANIFEST_BYTES).unwrap_or(usize::MAX) {
            return Err(IronHubCommandError::Catalog {
                reason: "manifest exceeds size cap".to_string(),
            });
        }
        let manifest: IronHubManifest =
            serde_json::from_slice(&bytes).map_err(|error| IronHubCommandError::Catalog {
                reason: format!("manifest parse failed: {error}"),
            })?;
        enforce_manifest_monotonic(&self.manifest_url, &manifest)?;
        Ok(manifest)
    }

    async fn download_verified(
        &self,
        artifact: &IronHubArtifact,
        max_bytes: u64,
    ) -> Result<Vec<u8>, IronHubCommandError> {
        validate_artifact(artifact, max_bytes)?;
        let bytes = self.download_url(&artifact.url, max_bytes).await?;
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
            return Err(IronHubCommandError::Catalog {
                reason: format!("download returned HTTP {}", response.status),
            });
        }
        if response.body.len() > usize::try_from(max_bytes).unwrap_or(usize::MAX) {
            return Err(IronHubCommandError::Catalog {
                reason: "download exceeds size cap".to_string(),
            });
        }
        Ok(response.body)
    }
}

fn is_skill_conflict(error: &RebornLocalSkillManagementError) -> bool {
    matches!(
        error,
        RebornLocalSkillManagementError::Skill(error)
            if error.kind() == ironclaw_skills::SkillManagementErrorKind::Conflict
    )
}

fn resolve_manifest_url() -> String {
    std::env::var("IRONHUB_MANIFEST_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_IRONHUB_MANIFEST_URL.to_string())
}

fn manifest_cache_get(url: &str, now: Instant) -> Option<Arc<IronHubManifest>> {
    let guard = MANIFEST_CACHE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let entry = guard.get(url)?;
    (now.duration_since(entry.fetched_at) <= MANIFEST_CACHE_TTL)
        .then(|| Arc::clone(&entry.manifest))
}

fn manifest_cache_put(url: &str, manifest: Arc<IronHubManifest>, now: Instant) {
    let mut guard = MANIFEST_CACHE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if guard.len() >= MANIFEST_CACHE_MAX_ENTRIES && !guard.contains_key(url) {
        guard.retain(|_, entry| now.duration_since(entry.fetched_at) <= MANIFEST_CACHE_TTL);
        if guard.len() >= MANIFEST_CACHE_MAX_ENTRIES
            && let Some(victim) = guard.keys().next().cloned()
        {
            guard.remove(&victim);
        }
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

fn enforce_manifest_monotonic(
    url: &str,
    manifest: &IronHubManifest,
) -> Result<(), IronHubCommandError> {
    let generated_at = DateTime::parse_from_rfc3339(&manifest.generated_at)
        .map_err(|error| IronHubCommandError::Catalog {
            reason: format!("manifest generated_at is not RFC3339: {error}"),
        })?
        .with_timezone(&Utc);
    let mut guard = MANIFEST_LAST_SEEN
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(previous) = guard.get(url)
        && generated_at < *previous
    {
        return Err(IronHubCommandError::Catalog {
            reason: format!(
                "signed manifest replay rejected: generated_at {} is older than last seen {}",
                generated_at.to_rfc3339(),
                previous.to_rfc3339()
            ),
        });
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
