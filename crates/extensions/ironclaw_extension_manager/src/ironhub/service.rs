use std::collections::HashMap;
use std::sync::{Arc, LazyLock};
use std::time::Instant;

use chrono::{DateTime, Utc};
use futures::{StreamExt, TryStreamExt, stream};
use ironclaw_extension_contracts::lifecycle_id::LifecyclePackageId;
use ironclaw_extension_contracts::state::InstallationState;
use ironclaw_host_api::{
    action::NetworkMethod,
    http::{
        RuntimeHttpEgress, RuntimeHttpEgressError, RuntimeHttpEgressRequest,
        RuntimeHttpEgressResponse,
    },
    ids::{CapabilityId, InvocationId},
    resource::ResourceScope,
    runtime::RuntimeKind,
};
use ironclaw_product_contracts::lifecycle_service::LifecycleProductSurfaceContext;
use ironclaw_product_contracts::package_lifecycle::{
    LifecyclePackageKind, LifecyclePackageRef, LifecycleProductPayload, LifecycleProductResponse,
};
use ironclaw_skills::{
    ScopedSkillManagementError, ScopedSkillManagementPort, SkillManagementErrorKind,
};

use ironclaw_extension_host::ExtensionLifecycleManager;

use super::catalog::{
    CatalogOrigin, IronHubManifestSource, catalog, classify, classify_gate_and_digest,
    compact_skill_summary, compact_tool_summary, entry_matches, invalid,
    network_policy_for_url_from_origin, sha256_hex, skill_file_byte_cap, skill_summary,
    tool_summary, validate_artifact_for_origin, validate_artifact_url, validate_hub_name,
    validate_manifest, validate_private_manifest, validate_private_manifest_origin,
    verify_signed_manifest,
};
use super::link_service::{IronhubLinkStateError, IronhubLinkStateStore};
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

const MAX_SKILL_FILE_DOWNLOAD_CONCURRENCY: usize = 8;

pub trait RebornIronHubRuntime {
    fn ironhub_skill_management(&self) -> Arc<ScopedSkillManagementPort>;
    fn ironhub_extension_management(&self) -> Arc<ExtensionLifecycleManager>;
    fn ironhub_runtime_http_egress(&self) -> Option<Arc<dyn RuntimeHttpEgress>>;
    fn ironhub_surface_context(&self) -> LifecycleProductSurfaceContext;
    fn ironhub_link_state(&self) -> Arc<IronhubLinkStateStore>;
    fn ironhub_manifest_url(&self) -> IronhubManifestUrl;
}

pub async fn execute_reborn_ironhub_command(
    runtime: &impl RebornIronHubRuntime,
    command: IronHubCommand,
) -> Result<IronHubResponse, IronHubCommandError> {
    let egress = runtime
        .ironhub_runtime_http_egress()
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
    let service = IronHubService::new_with_runtime_egress(
        runtime.ironhub_skill_management(),
        runtime.ironhub_extension_management(),
        egress,
        scope,
        ironhub_command_capability_id(&command)?,
        runtime.ironhub_link_state(),
    )
    .with_manifest_url(runtime.ironhub_manifest_url().into_inner());
    service.execute(command).await
}

pub async fn execute_reborn_ironhub_service_command(
    skill_management: Arc<ScopedSkillManagementPort>,
    extension_management: Arc<ExtensionLifecycleManager>,
    runtime_http_egress: Arc<dyn RuntimeHttpEgress>,
    link_state: Arc<IronhubLinkStateStore>,
    manifest_url: IronhubManifestUrl,
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
        link_state,
    )
    .with_manifest_url(manifest_url.into_inner())
    .execute(command)
    .await
}

struct IronHubEgress {
    egress: Arc<dyn RuntimeHttpEgress>,
    capability_id: CapabilityId,
}

impl IronHubEgress {
    fn capability_id(&self) -> CapabilityId {
        self.capability_id.clone()
    }

    async fn execute(
        &self,
        request: RuntimeHttpEgressRequest,
    ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
        self.egress.execute(request).await
    }
}

pub(crate) struct IronHubService {
    skill_management: Arc<ScopedSkillManagementPort>,
    extension_management: Arc<ExtensionLifecycleManager>,
    egress: IronHubEgress,
    scope: ResourceScope,
    manifest_url: String,
    verify_keys: &'static [(&'static str, &'static str)],
    link_state: Arc<IronhubLinkStateStore>,
}

impl IronHubService {
    fn new(
        skill_management: Arc<ScopedSkillManagementPort>,
        extension_management: Arc<ExtensionLifecycleManager>,
        egress: IronHubEgress,
        scope: ResourceScope,
        link_state: Arc<IronhubLinkStateStore>,
    ) -> Self {
        Self {
            skill_management,
            extension_management,
            egress,
            scope,
            manifest_url: DEFAULT_IRONHUB_MANIFEST_URL.to_string(),
            verify_keys: super::model::MANIFEST_VERIFY_KEYS,
            link_state,
        }
    }

    pub(crate) fn new_with_runtime_egress(
        skill_management: Arc<ScopedSkillManagementPort>,
        extension_management: Arc<ExtensionLifecycleManager>,
        egress: Arc<dyn RuntimeHttpEgress>,
        scope: ResourceScope,
        capability_id: CapabilityId,
        link_state: Arc<IronhubLinkStateStore>,
    ) -> Self {
        Self::new(
            skill_management,
            extension_management,
            IronHubEgress {
                egress,
                capability_id,
            },
            scope,
            link_state,
        )
    }

    pub(super) fn with_verify_keys(
        mut self,
        verify_keys: &'static [(&'static str, &'static str)],
    ) -> Self {
        self.verify_keys = verify_keys;
        self
    }

    pub(super) fn with_manifest_url(mut self, manifest_url: String) -> Self {
        self.manifest_url = manifest_url;
        self
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
        let catalog_total = manifest.tools.len() + manifest.skills.len();
        IronHubResponse::discovered_catalog(entries, catalog_total)
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
        let catalog_total = manifest.tools.len() + manifest.skills.len();
        IronHubResponse::discovered_catalog(entries, catalog_total)
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
        let private_origin = options
            .private_manifest_url
            .as_deref()
            .map(|private_url| validate_private_manifest_origin(&self.manifest_url, private_url))
            .transpose()?;
        let (manifest, source) = match (
            options.private_manifest_url.as_deref(),
            private_origin.as_ref(),
        ) {
            (Some(private_url), Some(origin)) => (
                Arc::new(self.fetch_private_manifest(private_url, origin).await?),
                IronHubManifestSource::Private,
            ),
            (None, None) => (
                self.fetch_manifest_cached().await?,
                IronHubManifestSource::Public,
            ),
            _ => {
                return Err(catalog(
                    "private manifest source could not be validated against the catalog origin",
                ));
            }
        };
        let (kind, provenance, artifact_digest) =
            classify_gate_and_digest(&manifest, name, options.kind, &options, source)?;
        let lifecycle = match kind {
            IronHubEntryKind::Skill => {
                let entry = manifest
                    .find_skill(name)
                    .ok_or_else(|| catalog("skill not found"))?;
                let content = self
                    .download_verified(&entry.skill_md, MAX_METADATA_BYTES, private_origin.as_ref())
                    .await?;
                let content =
                    String::from_utf8(content).map_err(|error| IronHubCommandError::Install {
                        reason: format!("skill markdown is not UTF-8: {error}"),
                    })?;
                let source_url = private_origin
                    .as_ref()
                    .map(CatalogOrigin::redacted_source_url)
                    .unwrap_or_else(|| entry.skill_md.url.clone());
                let private_origin_ref = private_origin.as_ref();
                let files = stream::iter(entry.files.iter().cloned())
                    .map(|file| async move {
                        self.download_verified(
                            &file.artifact,
                            skill_file_byte_cap(),
                            private_origin_ref,
                        )
                        .await
                        .map(|contents| (file.path, contents))
                    })
                    .buffered(MAX_SKILL_FILE_DOWNLOAD_CONCURRENCY)
                    .try_collect::<Vec<_>>()
                    .await?;
                let installed = self
                    .install_skill(
                        entry.name.as_str(),
                        &content,
                        &files,
                        &source_url,
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
                // Digest-verified like every other artifact, and covered by
                // the same catalog signature.
                let manifest_artifact = entry.manifest.as_ref().ok_or_else(|| {
                    catalog(format!(
                        "'{}' publishes no extension manifest; its catalog entry predates \
                             published manifests, so refresh the catalog",
                        entry.name
                    ))
                })?;
                let tool_manifest = self
                    .download_verified(
                        manifest_artifact,
                        MAX_METADATA_BYTES,
                        private_origin.as_ref(),
                    )
                    .await?;
                let wasm = self
                    .download_verified(&entry.wasm, MAX_WASM_BYTES, private_origin.as_ref())
                    .await?;
                let capabilities = self
                    .download_verified(
                        &entry.capabilities,
                        MAX_METADATA_BYTES,
                        private_origin.as_ref(),
                    )
                    .await?;
                let mut schemas = Vec::with_capacity(entry.schemas.len());
                for (path, artifact) in &entry.schemas {
                    let content = self
                        .download_verified(artifact, MAX_METADATA_BYTES, private_origin.as_ref())
                        .await?;
                    schemas.push((path.clone(), content));
                }
                let mut prompts = Vec::with_capacity(entry.prompts.len());
                for (path, artifact) in &entry.prompts {
                    let content = self
                        .download_verified(artifact, MAX_METADATA_BYTES, private_origin.as_ref())
                        .await?;
                    prompts.push((path.clone(), content));
                }
                let reserved = self
                    .extension_management
                    .reserved_bundled_extension_ids()
                    .await;
                let package = ironhub_tool_package(
                    entry,
                    tool_manifest,
                    wasm,
                    capabilities,
                    schemas,
                    prompts,
                    &reserved,
                )?;
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
        let mut entry = match kind {
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
        entry.provenance = provenance;
        Ok(IronHubResponse {
            phase: IronHubPhase::Installed,
            total_entries: 1,
            returned_entries: 1,
            truncated: false,
            catalog_total: None,
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

    async fn install_skill(
        &self,
        name: &str,
        content: &str,
        files: &[(String, Vec<u8>)],
        source_url: &str,
        force: bool,
    ) -> Result<ironclaw_skills::SkillInstallResult, IronHubCommandError> {
        let bundle: Vec<ironclaw_skills::SkillInstallFile<'_>> = files
            .iter()
            .map(|(path, contents)| ironclaw_skills::SkillInstallFile {
                relative_path: path.as_str(),
                contents: contents.as_slice(),
            })
            .collect();
        let first = self
            .skill_management
            .install_from_url_for_scope(
                self.scope.clone(),
                Some(name),
                content,
                &bundle,
                source_url,
            )
            .await;
        let Err(error) = first else {
            return first.map_err(skill_install_error);
        };
        if !force || !is_skill_conflict(&error) {
            return Err(skill_install_error(error));
        }
        let previous = self
            .skill_management
            .capture_replacement_snapshot_for_scope(self.scope.clone(), name)
            .await
            .map_err(skill_install_error)?;
        self.skill_management
            .remove_for_scope(self.scope.clone(), name)
            .await
            .map_err(skill_install_error)?;
        match self
            .skill_management
            .install_from_url_for_scope(
                self.scope.clone(),
                Some(name),
                content,
                &bundle,
                source_url,
            )
            .await
        {
            Ok(result) => Ok(result),
            Err(original_error) => {
                let restore = self
                    .skill_management
                    .restore_replacement_snapshot(previous)
                    .await;
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
        // Duplicate cache misses may fetch concurrently. Durable manifest CAS
        // makes identical fetches idempotent, so no process-local mutex is held
        // across network or filesystem I/O.
        let manifest = Arc::new(self.fetch_manifest().await?);
        manifest_cache_put(&self.manifest_url, Arc::clone(&manifest), now);
        Ok(manifest)
    }

    async fn fetch_manifest(&self) -> Result<IronHubManifest, IronHubCommandError> {
        validate_artifact_url("hub-manifest", "manifest_url", &self.manifest_url)?;
        let envelope = self
            .download_url(&self.manifest_url, MAX_SIGNED_MANIFEST_BYTES, None)
            .await?;
        let bytes = self.verify_manifest_envelope(&envelope)?;
        if bytes.len() > usize::try_from(MAX_MANIFEST_BYTES).unwrap_or(usize::MAX) {
            return Err(catalog("manifest exceeds size cap"));
        }
        let manifest: IronHubManifest =
            serde_json::from_slice(&bytes).map_err(|error| IronHubCommandError::Catalog {
                reason: format!("manifest parse failed: {error}"),
            })?;
        validate_manifest(&manifest)?;
        let generated_at = DateTime::parse_from_rfc3339(&manifest.generated_at)
            .map_err(|error| catalog(format!("manifest generated_at is not RFC3339: {error}")))?
            .with_timezone(&Utc);
        self.link_state
            .record_public_manifest(&self.manifest_url, generated_at, &sha256_hex(&bytes))
            .await
            .map_err(map_link_state_error)?;
        Ok(manifest)
    }

    async fn fetch_private_manifest(
        &self,
        private_url: &str,
        origin: &CatalogOrigin,
    ) -> Result<IronHubManifest, IronHubCommandError> {
        let envelope = self
            .download_url(private_url, MAX_SIGNED_MANIFEST_BYTES, Some(origin))
            .await?;
        let bytes = self.verify_manifest_envelope(&envelope)?;
        if bytes.len() > usize::try_from(MAX_MANIFEST_BYTES).unwrap_or(usize::MAX) {
            return Err(catalog("private manifest exceeds size cap"));
        }
        let manifest: IronHubManifest =
            serde_json::from_slice(&bytes).map_err(|error| IronHubCommandError::Catalog {
                reason: format!("private manifest parse failed: {error}"),
            })?;
        validate_private_manifest(&manifest, origin)?;
        let generated_at = DateTime::parse_from_rfc3339(&manifest.generated_at)
            .map_err(|error| {
                catalog(format!(
                    "private manifest generated_at is not RFC3339: {error}"
                ))
            })?
            .with_timezone(&Utc);
        self.link_state
            .record_private_manifest(
                origin.host(),
                &manifest.repo,
                generated_at,
                &sha256_hex(&bytes),
            )
            .await
            .map_err(map_link_state_error)?;
        Ok(manifest)
    }

    fn verify_manifest_envelope(&self, envelope: &[u8]) -> Result<Vec<u8>, IronHubCommandError> {
        if self.verify_keys == super::model::MANIFEST_VERIFY_KEYS {
            verify_signed_manifest(envelope)
        } else {
            super::catalog::verify_signed_manifest_with_keys(envelope, self.verify_keys)
        }
        .map_err(|reason| IronHubCommandError::Catalog {
            reason: format!("signed manifest verification failed: {reason}"),
        })
    }

    async fn download_verified(
        &self,
        artifact: &IronHubArtifact,
        max_bytes: u64,
        origin: Option<&CatalogOrigin>,
    ) -> Result<Vec<u8>, IronHubCommandError> {
        validate_artifact_for_origin(artifact, max_bytes, origin)?;
        let bytes = self
            .download_url(&artifact.url, artifact.size_bytes, origin)
            .await?;
        if u64::try_from(bytes.len()).unwrap_or(u64::MAX) != artifact.size_bytes {
            return Err(IronHubCommandError::Install {
                reason: format!(
                    "artifact size mismatch: expected {} bytes, got {}",
                    artifact.size_bytes,
                    bytes.len()
                ),
            });
        }
        let actual = sha256_hex(&bytes);
        if !actual.eq_ignore_ascii_case(&artifact.sha256) {
            return Err(IronHubCommandError::Install {
                reason: format!(
                    "artifact checksum mismatch: expected {}, got {}",
                    artifact.sha256, actual
                ),
            });
        }
        Ok(bytes)
    }

    async fn download_url(
        &self,
        url: &str,
        max_bytes: u64,
        origin: Option<&CatalogOrigin>,
    ) -> Result<Vec<u8>, IronHubCommandError> {
        let request = RuntimeHttpEgressRequest {
            runtime: RuntimeKind::FirstParty,
            scope: self.scope.clone(),
            capability_id: self.egress.capability_id(),
            method: NetworkMethod::Get,
            url: url.to_string(),
            headers: Vec::new(),
            body: Vec::new(),
            network_policy: network_policy_for_url_from_origin(url, max_bytes, origin)?,
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

fn ironhub_command_capability_id(
    command: &IronHubCommand,
) -> Result<CapabilityId, IronHubCommandError> {
    let value = match command {
        IronHubCommand::Search { .. } | IronHubCommand::List { .. } => {
            super::IRONHUB_SEARCH_CAPABILITY_ID
        }
        IronHubCommand::Info { .. } => super::IRONHUB_INFO_CAPABILITY_ID,
        IronHubCommand::Install { .. } => super::IRONHUB_INSTALL_CAPABILITY_ID,
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

fn map_link_state_error(error: IronhubLinkStateError) -> IronHubCommandError {
    match error {
        IronhubLinkStateError::ManifestReplay => catalog("private signed manifest replay rejected"),
        IronhubLinkStateError::NonceReplay => invalid("install nonce was replayed"),
        IronhubLinkStateError::InvalidInput => invalid("invalid IronHub durable state input"),
        IronhubLinkStateError::Unavailable => {
            catalog("IronHub durable replay state is unavailable")
        }
    }
}

#[cfg(test)]
mod link_state_error_tests {
    use super::*;

    #[test]
    fn every_link_state_error_maps_to_a_stable_command_category() {
        assert!(matches!(
            map_link_state_error(IronhubLinkStateError::ManifestReplay),
            IronHubCommandError::Catalog { .. }
        ));
        assert!(matches!(
            map_link_state_error(IronhubLinkStateError::NonceReplay),
            IronHubCommandError::InvalidInput { .. }
        ));
        assert!(matches!(
            map_link_state_error(IronhubLinkStateError::InvalidInput),
            IronHubCommandError::InvalidInput { .. }
        ));
        assert!(matches!(
            map_link_state_error(IronhubLinkStateError::Unavailable),
            IronHubCommandError::Catalog { .. }
        ));
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IronhubManifestUrl(String);

impl IronhubManifestUrl {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_inner(self) -> String {
        self.0
    }
}

impl Default for IronhubManifestUrl {
    fn default() -> Self {
        Self(DEFAULT_IRONHUB_MANIFEST_URL.to_string())
    }
}

pub fn validated_manifest_url(value: &str) -> Result<IronhubManifestUrl, IronHubCommandError> {
    let value = value.trim();
    validate_artifact_url("hub-manifest", "manifest_url", value)?;
    Ok(IronhubManifestUrl(value.to_string()))
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
