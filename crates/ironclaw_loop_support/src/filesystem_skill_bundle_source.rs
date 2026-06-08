use std::{collections::HashSet, sync::Arc};

use async_trait::async_trait;
use ironclaw_filesystem::{FileType, FilesystemError, RootFilesystem, ScopedFilesystem};
use ironclaw_host_api::{ResourceScope, ScopedPath, TenantId};
use ironclaw_skills::{
    INSTALL_METADATA_FILE_NAME, InstalledSkillMetadata, MAX_INSTALL_METADATA_BYTES,
    MAX_PROMPT_FILE_SIZE, SkillTrust, parse_skill_md,
};
use ironclaw_turns::run_profile::{LoopRunContext, SkillVisibility};
use parking_lot::Mutex;
use tracing::debug;

use crate::{
    SkillBundleDescriptor, SkillBundleId, SkillBundleProvenance, SkillBundleSource,
    SkillBundleSourceError, SkillFilePath, SkillSourceKind, sort_skill_bundle_descriptors,
};

const DEFAULT_MAX_BUNDLE_FILE_BYTES: usize = 256 * 1024;
const DEFAULT_MAX_BUNDLES_PER_ROOT: usize = 100;
/// One scoped filesystem root that can contain portable skill bundle folders.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilesystemSkillBundleRoot {
    source_kind: SkillSourceKind,
    root: ScopedPath,
    tenant_id: Option<TenantId>,
    trust: Option<SkillTrust>,
    visibility: Option<SkillVisibility>,
}

impl FilesystemSkillBundleRoot {
    /// Creates a skill bundle root with caller-supplied source metadata.
    pub fn new(
        source_kind: SkillSourceKind,
        root: ScopedPath,
        trust: Option<SkillTrust>,
        visibility: Option<SkillVisibility>,
    ) -> Self {
        Self {
            source_kind,
            root,
            tenant_id: None,
            trust,
            visibility,
        }
    }

    /// Creates a built-in system skill root.
    pub fn system(root: ScopedPath) -> Self {
        Self::new(
            SkillSourceKind::System,
            root,
            Some(SkillTrust::Trusted),
            Some(SkillVisibility::Visible),
        )
    }

    /// Creates a user-owned skill root.
    pub fn user(root: ScopedPath) -> Self {
        Self::new(
            SkillSourceKind::User,
            root,
            Some(SkillTrust::Trusted),
            Some(SkillVisibility::Visible),
        )
    }

    /// Creates a tenant-shared skill root.
    pub fn tenant_shared(root: ScopedPath, tenant_id: TenantId) -> Self {
        let mut root = Self::new(
            SkillSourceKind::TenantShared,
            root,
            Some(SkillTrust::Installed),
            Some(SkillVisibility::Visible),
        );
        root.tenant_id = Some(tenant_id);
        root
    }

    /// Returns the descriptor source kind for bundles discovered under this root.
    pub fn source_kind(&self) -> SkillSourceKind {
        self.source_kind
    }

    /// Returns the scoped filesystem path for this root.
    pub fn root(&self) -> &ScopedPath {
        &self.root
    }

    /// Returns the tenant that owns this root when the root is tenant-scoped.
    pub fn tenant_id(&self) -> Option<&TenantId> {
        self.tenant_id.as_ref()
    }

    /// Returns trust metadata applied to bundles discovered under this root.
    pub fn trust(&self) -> Option<&SkillTrust> {
        self.trust.as_ref()
    }

    /// Returns visibility metadata applied to bundles discovered under this root.
    pub fn visibility(&self) -> Option<&SkillVisibility> {
        self.visibility.as_ref()
    }
}

/// Filesystem-backed skill bundle source over host-approved scoped roots.
pub struct FilesystemSkillBundleSource<F> {
    filesystem: Arc<ScopedFilesystem<F>>,
    roots: Vec<FilesystemSkillBundleRoot>,
    validated_manifests: Mutex<HashSet<ScopedPath>>,
    max_skill_md_bytes: usize,
    max_bundle_file_bytes: usize,
    max_bundles_per_root: usize,
}

impl<F> std::fmt::Debug for FilesystemSkillBundleSource<F> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FilesystemSkillBundleSource")
            .field("filesystem", &"<ScopedFilesystem>")
            .field("roots", &self.roots)
            .field(
                "validated_manifests",
                &self.validated_manifests.lock().len(),
            )
            .field("max_skill_md_bytes", &self.max_skill_md_bytes)
            .field("max_bundle_file_bytes", &self.max_bundle_file_bytes)
            .field("max_bundles_per_root", &self.max_bundles_per_root)
            .finish()
    }
}

impl<F> FilesystemSkillBundleSource<F>
where
    F: RootFilesystem,
{
    /// Creates a filesystem skill bundle source.
    ///
    /// Each source kind must appear at most once because bundle ids intentionally
    /// expose source kind and name, not raw host root provenance.
    pub fn new(
        filesystem: Arc<ScopedFilesystem<F>>,
        roots: Vec<FilesystemSkillBundleRoot>,
    ) -> Result<Self, SkillBundleSourceError> {
        ensure_unique_source_kinds(&roots)?;
        Ok(Self {
            filesystem,
            roots,
            validated_manifests: Mutex::new(HashSet::new()),
            max_skill_md_bytes: MAX_PROMPT_FILE_SIZE as usize,
            max_bundle_file_bytes: DEFAULT_MAX_BUNDLE_FILE_BYTES,
            max_bundles_per_root: DEFAULT_MAX_BUNDLES_PER_ROOT,
        })
    }

    /// Overrides the maximum allowed `SKILL.md` manifest size in bytes.
    pub fn with_max_skill_md_bytes(mut self, max_skill_md_bytes: usize) -> Self {
        self.max_skill_md_bytes = max_skill_md_bytes;
        self
    }

    /// Overrides the maximum allowed supporting file size in bytes.
    pub fn with_max_bundle_file_bytes(mut self, max_bundle_file_bytes: usize) -> Self {
        self.max_bundle_file_bytes = max_bundle_file_bytes;
        self
    }

    /// Overrides the maximum number of bundle directories scanned per root.
    pub fn with_max_bundles_per_root(mut self, max_bundles_per_root: usize) -> Self {
        self.max_bundles_per_root = max_bundles_per_root;
        self
    }

    /// Returns the configured filesystem roots.
    pub fn roots(&self) -> &[FilesystemSkillBundleRoot] {
        &self.roots
    }

    async fn list_root(
        &self,
        scope: &ResourceScope,
        root: &FilesystemSkillBundleRoot,
        descriptors: &mut Vec<SkillBundleDescriptor>,
    ) -> Result<(), SkillBundleSourceError> {
        let entries = match self.filesystem.list_dir(scope, root.root()).await {
            Ok(entries) => entries,
            Err(error) if is_not_found(&error) => return Ok(()),
            Err(error) => return Err(map_filesystem_error(error)),
        };

        let mut directory_entries = Vec::new();
        for entry in entries {
            if entry.file_type == FileType::Directory {
                directory_entries.push(entry);
                if directory_entries.len() > self.max_bundles_per_root {
                    return Err(SkillBundleSourceError::BundleScanLimitExceeded);
                }
            }
        }

        let skill_md_file = SkillFilePath::skill_md();
        for entry in directory_entries {
            let bundle_id = match SkillBundleId::new(root.source_kind(), &entry.name) {
                Ok(bundle_id) => bundle_id,
                Err(_) => {
                    debug!(
                        bundle_name = %entry.name,
                        source_kind = %root.source_kind(),
                        "skipping invalid skill bundle directory name"
                    );
                    continue;
                }
            };
            let skill_md_path = bundle_scoped_path(root.root(), &bundle_id, &skill_md_file)?;
            let description = match self
                .validate_bundle_manifest(scope, &skill_md_path, &bundle_id)
                .await
            {
                Ok(description) => description,
                Err(error) if is_skippable_manifest_error(&error) => {
                    debug!(
                        bundle_id = %bundle_id,
                        error = ?error,
                        "skipping invalid skill bundle manifest"
                    );
                    continue;
                }
                Err(error) => return Err(error),
            };
            self.validated_manifests.lock().insert(skill_md_path);
            let trust = self.bundle_trust(scope, root, &bundle_id).await?;

            descriptors.push(
                SkillBundleDescriptor::new(
                    bundle_id,
                    trust,
                    root.visibility().copied(),
                    description,
                )
                .with_provenance(SkillBundleProvenance::new(root.source_kind())),
            );
        }

        Ok(())
    }

    async fn validate_bundle_manifest(
        &self,
        scope: &ResourceScope,
        skill_md_path: &ScopedPath,
        bundle_id: &SkillBundleId,
    ) -> Result<String, SkillBundleSourceError> {
        let skill_md = self
            .read_bounded(scope, skill_md_path, self.max_skill_md_bytes)
            .await?;
        let skill_md = String::from_utf8(skill_md)
            .map_err(|_| SkillBundleSourceError::BundleUtf8DecodeFailed)?;
        let parsed =
            parse_skill_md(&skill_md).map_err(|_| SkillBundleSourceError::ManifestParseFailed)?;
        if parsed.manifest.name != bundle_id.name() {
            return Err(SkillBundleSourceError::InvalidSkillBundle);
        }
        let description = parsed.manifest.description;
        if description.trim().is_empty() {
            return Err(SkillBundleSourceError::InvalidSkillBundle);
        }
        Ok(description)
    }

    async fn bundle_trust(
        &self,
        scope: &ResourceScope,
        root: &FilesystemSkillBundleRoot,
        bundle_id: &SkillBundleId,
    ) -> Result<Option<SkillTrust>, SkillBundleSourceError> {
        let default_trust = root.trust().cloned();
        if default_trust != Some(SkillTrust::Trusted) {
            return Ok(default_trust);
        }
        let metadata_path = bundle_scoped_path(
            root.root(),
            bundle_id,
            &SkillFilePath::new(INSTALL_METADATA_FILE_NAME)?,
        )?;
        let bytes = match self
            .filesystem
            .read_bytes_bounded(scope, &metadata_path, MAX_INSTALL_METADATA_BYTES)
            .await
        {
            Ok(Some(bytes)) => bytes,
            Ok(None) => return Ok(Some(SkillTrust::Installed)),
            Err(error) if is_not_found(&error) => return Ok(default_trust),
            Err(error) => return Err(map_file_read_error(error)),
        };
        if InstalledSkillMetadata::sidecar_bytes_mark_installed(&bytes) {
            Ok(Some(SkillTrust::Installed))
        } else {
            Ok(default_trust)
        }
    }

    async fn read_bounded(
        &self,
        scope: &ResourceScope,
        path: &ScopedPath,
        max_bytes: usize,
    ) -> Result<Vec<u8>, SkillBundleSourceError> {
        self.filesystem
            .read_bytes_bounded(scope, path, max_bytes)
            .await
            .map_err(map_file_read_error)?
            .ok_or(SkillBundleSourceError::ContentTooLarge)
    }
}

#[async_trait]
impl<F> SkillBundleSource for FilesystemSkillBundleSource<F>
where
    F: RootFilesystem + 'static,
{
    async fn list_skill_bundles(
        &self,
        run_context: &LoopRunContext,
    ) -> Result<Vec<SkillBundleDescriptor>, SkillBundleSourceError> {
        let scope = resource_scope_for_run(run_context);
        let mut descriptors = Vec::new();
        for root in &self.roots {
            if !root_visible_for_run(root, run_context) {
                continue;
            }
            self.list_root(&scope, root, &mut descriptors).await?;
        }
        sort_skill_bundle_descriptors(&mut descriptors);
        Ok(descriptors)
    }

    async fn read_skill_bundle_file(
        &self,
        run_context: &LoopRunContext,
        bundle_id: &SkillBundleId,
        path: &SkillFilePath,
    ) -> Result<Vec<u8>, SkillBundleSourceError> {
        let root = self
            .roots
            .iter()
            .filter(|root| root_visible_for_run(root, run_context))
            .find(|root| root.source_kind() == bundle_id.source_kind())
            .ok_or(SkillBundleSourceError::BundleNotFound)?;
        let scope = resource_scope_for_run(run_context);
        let skill_md_path = bundle_scoped_path(root.root(), bundle_id, &SkillFilePath::skill_md())?;
        if path.as_str() != SkillFilePath::skill_md().as_str()
            && !self.validated_manifests.lock().contains(&skill_md_path)
        {
            // Re-validates manifests before uncached reads for security defense-in-depth.
            self.validate_bundle_manifest(&scope, &skill_md_path, bundle_id)
                .await
                .map_err(|error| match error {
                    SkillBundleSourceError::FileNotFound => SkillBundleSourceError::BundleNotFound,
                    other => other,
                })?;
            let mut validated_manifests = self.validated_manifests.lock();
            if !validated_manifests.contains(&skill_md_path) {
                validated_manifests.insert(skill_md_path.clone());
            }
        }
        let scoped_path = bundle_scoped_path(root.root(), bundle_id, path)?;
        self.read_bounded(&scope, &scoped_path, self.max_bundle_file_bytes)
            .await
    }
}

fn resource_scope_for_run(run_context: &LoopRunContext) -> ResourceScope {
    let mut scope = run_context.scope.to_resource_scope();
    if let Some(actor) = run_context.actor() {
        scope.user_id = actor.user_id.clone();
    }
    scope
}

fn root_visible_for_run(root: &FilesystemSkillBundleRoot, run_context: &LoopRunContext) -> bool {
    match root.source_kind() {
        SkillSourceKind::TenantShared => root
            .tenant_id()
            .is_some_and(|tenant_id| tenant_id == &run_context.scope.tenant_id),
        SkillSourceKind::System | SkillSourceKind::User => true,
    }
}

fn bundle_scoped_path(
    root: &ScopedPath,
    bundle_id: &SkillBundleId,
    path: &SkillFilePath,
) -> Result<ScopedPath, SkillBundleSourceError> {
    ScopedPath::new(format!(
        "{}/{}/{}",
        root.as_str().trim_end_matches('/'),
        bundle_id.name(),
        path.as_str()
    ))
    .map_err(|_| SkillBundleSourceError::InvalidFilePath)
}

fn ensure_unique_source_kinds(
    roots: &[FilesystemSkillBundleRoot],
) -> Result<(), SkillBundleSourceError> {
    let mut seen = HashSet::new();
    for root in roots {
        if !seen.insert(root.source_kind()) {
            return Err(SkillBundleSourceError::DuplicateSourceKind);
        }
    }
    Ok(())
}

fn map_file_read_error(error: FilesystemError) -> SkillBundleSourceError {
    if is_not_found(&error) {
        return SkillBundleSourceError::FileNotFound;
    }
    map_filesystem_error(error)
}

fn map_filesystem_error(error: FilesystemError) -> SkillBundleSourceError {
    if !is_not_found(&error) {
        tracing::warn!(
            component = "filesystem_skill_bundle_source",
            operation = "map_filesystem_error",
            error = %error,
            error_debug = ?error,
            "filesystem skill bundle error mapped to safe source error"
        );
    }
    match error {
        FilesystemError::PermissionDenied { .. } => SkillBundleSourceError::PermissionDenied,
        // Kept for API completeness: some callers route filesystem errors through
        // this general mapper without first applying the list/read-specific
        // not-found semantics.
        FilesystemError::NotFound { .. } => SkillBundleSourceError::BundleNotFound,
        FilesystemError::Unsupported { .. } => SkillBundleSourceError::SourceUnavailable,
        FilesystemError::Contract(_)
        | FilesystemError::MountNotFound { .. }
        | FilesystemError::PathOutsideMount { .. }
        | FilesystemError::SymlinkEscape { .. }
        | FilesystemError::MountConflict { .. }
        | FilesystemError::Backend { .. }
        | FilesystemError::VersionMismatch { .. }
        | FilesystemError::IndexConflict { .. }
        | FilesystemError::DescriptorOverclaims { .. }
        | FilesystemError::SerializeIndexed { .. }
        | FilesystemError::DeserializeIndexed { .. }
        | FilesystemError::CorruptRecordVersion { .. }
        | FilesystemError::IndexSpecMissingAfterUpsert { .. }
        | FilesystemError::BackendInfrastructure { .. } => SkillBundleSourceError::Internal,
        // FilesystemError is #[non_exhaustive], so a wildcard remains required.
        _ => SkillBundleSourceError::Internal,
    }
}

fn is_not_found(error: &FilesystemError) -> bool {
    matches!(error, FilesystemError::NotFound { .. })
}

fn is_skippable_manifest_error(error: &SkillBundleSourceError) -> bool {
    matches!(
        error,
        SkillBundleSourceError::FileNotFound
            | SkillBundleSourceError::InvalidSkillBundle
            | SkillBundleSourceError::BundleUtf8DecodeFailed
            | SkillBundleSourceError::ManifestParseFailed
            | SkillBundleSourceError::ContentTooLarge
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_filesystem::{
        BackendCapabilities, CasExpectation, DirEntry, Entry, FileStat, InMemoryBackend,
        RecordVersion, RootFilesystem,
    };
    use ironclaw_host_api::{
        AgentId, MountAlias, MountGrant, MountPermissions, MountView, ProjectId, TenantId,
        ThreadId, UserId, VirtualPath,
    };
    use ironclaw_turns::{
        RunProfileResolutionRequest, RunProfileResolver, TurnActor, TurnId, TurnRunId, TurnScope,
        run_profile::InMemoryRunProfileResolver,
    };

    fn skill_md(name: &str, description: &str) -> String {
        format!("---\nname: {name}\ndescription: {description}\n---\nUse the {name} skill.\n")
    }

    async fn run_context() -> LoopRunContext {
        let tenant_id = TenantId::new("tenant-a").unwrap();
        let agent_id = AgentId::new("agent-a").unwrap();
        let project_id = ProjectId::new("project-a").unwrap();
        let thread_id = ThreadId::new("thread-a").unwrap();
        let user_id = UserId::new("user-a").unwrap();
        let scope = TurnScope::new(tenant_id, Some(agent_id), Some(project_id), thread_id);
        let resolved = InMemoryRunProfileResolver::default()
            .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
            .await
            .unwrap();
        LoopRunContext::new(scope, TurnId::new(), TurnRunId::new(), resolved)
            .with_actor(TurnActor::new(user_id))
    }

    fn mounted_source() -> (
        Arc<InMemoryBackend>,
        FilesystemSkillBundleSource<InMemoryBackend>,
    ) {
        let root = Arc::new(InMemoryBackend::default());
        let view = MountView::new(vec![
            MountGrant::new(
                MountAlias::new("/system/skills").unwrap(),
                VirtualPath::new("/system/skills").unwrap(),
                MountPermissions::read_only(),
            ),
            MountGrant::new(
                MountAlias::new("/skills").unwrap(),
                VirtualPath::new("/tenants/tenant-a/users/user-a/skills").unwrap(),
                MountPermissions::read_write_list_delete(),
            ),
        ])
        .unwrap();
        let filesystem = Arc::new(ScopedFilesystem::with_fixed_view(Arc::clone(&root), view));
        let source = FilesystemSkillBundleSource::new(
            filesystem,
            vec![
                FilesystemSkillBundleRoot::system(ScopedPath::new("/system/skills").unwrap()),
                FilesystemSkillBundleRoot::user(ScopedPath::new("/skills").unwrap()),
            ],
        )
        .unwrap();
        (root, source)
    }

    async fn write_root(root: &InMemoryBackend, path: &str, bytes: impl Into<Vec<u8>>) {
        root.put(
            &VirtualPath::new(path).unwrap(),
            Entry::bytes(bytes.into()),
            CasExpectation::Any,
        )
        .await
        .unwrap();
    }

    struct GrowingReadBackend {
        inner: InMemoryBackend,
        growing_path: VirtualPath,
    }

    impl GrowingReadBackend {
        fn new(growing_path: VirtualPath) -> Self {
            Self {
                inner: InMemoryBackend::default(),
                growing_path,
            }
        }
    }

    #[async_trait]
    impl RootFilesystem for GrowingReadBackend {
        fn capabilities(&self) -> BackendCapabilities {
            self.inner.capabilities()
        }

        async fn put(
            &self,
            path: &VirtualPath,
            entry: Entry,
            cas: CasExpectation,
        ) -> Result<RecordVersion, FilesystemError> {
            self.inner.put(path, entry, cas).await
        }

        async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
            self.inner.list_dir(path).await
        }

        async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
            self.inner.stat(path).await
        }

        async fn read_file_bounded(
            &self,
            path: &VirtualPath,
            max_bytes: usize,
        ) -> Result<Option<Vec<u8>>, FilesystemError> {
            if path == &self.growing_path {
                return Ok(None);
            }
            self.inner.read_file_bounded(path, max_bytes).await
        }
    }

    #[tokio::test]
    async fn filesystem_source_lists_valid_skill_bundles_in_deterministic_source_order() {
        let (root, source) = mounted_source();
        write_root(
            &root,
            "/tenants/tenant-a/users/user-a/skills/local-review/SKILL.md",
            skill_md("local-review", "Local review"),
        )
        .await;
        write_root(
            &root,
            "/system/skills/code-review/SKILL.md",
            skill_md("code-review", "System review"),
        )
        .await;
        write_root(
            &root,
            "/tenants/tenant-a/users/user-a/skills/code-review/SKILL.md",
            skill_md("code-review", "User review"),
        )
        .await;
        write_root(
            &root,
            "/tenants/tenant-a/users/user-a/skills/no-manifest/README.md",
            "not a skill",
        )
        .await;

        let descriptors = source
            .list_skill_bundles(&run_context().await)
            .await
            .unwrap();
        let ids: Vec<String> = descriptors
            .iter()
            .map(|descriptor| descriptor.id().to_string())
            .collect();
        assert_eq!(
            ids,
            vec![
                "system:code-review",
                "user:code-review",
                "user:local-review"
            ]
        );
        assert_eq!(descriptors[0].trust(), Some(&SkillTrust::Trusted));
        assert_eq!(descriptors[1].trust(), Some(&SkillTrust::Trusted));
        assert_eq!(descriptors[0].visibility(), Some(&SkillVisibility::Visible));
        let descriptions = descriptors
            .iter()
            .map(SkillBundleDescriptor::description)
            .collect::<Vec<_>>();
        assert_eq!(
            descriptions,
            vec!["System review", "User review", "Local review"]
        );
    }

    #[tokio::test]
    async fn filesystem_source_downgrades_url_installed_user_bundle_metadata() {
        let (root, source) = mounted_source();
        write_root(
            &root,
            "/tenants/tenant-a/users/user-a/skills/remote-review/SKILL.md",
            skill_md("remote-review", "Remote review"),
        )
        .await;
        write_root(
            &root,
            "/tenants/tenant-a/users/user-a/skills/remote-review/.ironclaw-install.json",
            br#"{"source":"installed_url","source_url":"https://example.test/SKILL.md"}"#.to_vec(),
        )
        .await;

        let descriptors = source
            .list_skill_bundles(&run_context().await)
            .await
            .unwrap();

        assert_eq!(descriptors.len(), 1);
        assert_eq!(descriptors[0].id().name(), "remote-review");
        assert_eq!(descriptors[0].trust(), Some(&SkillTrust::Installed));
    }

    #[tokio::test]
    async fn filesystem_source_fails_closed_on_malformed_install_metadata() {
        let (root, source) = mounted_source();
        write_root(
            &root,
            "/tenants/tenant-a/users/user-a/skills/remote-review/SKILL.md",
            skill_md("remote-review", "Remote review"),
        )
        .await;
        write_root(
            &root,
            "/tenants/tenant-a/users/user-a/skills/remote-review/.ironclaw-install.json",
            "not json",
        )
        .await;

        let descriptors = source
            .list_skill_bundles(&run_context().await)
            .await
            .unwrap();

        assert_eq!(descriptors.len(), 1);
        assert_eq!(descriptors[0].id().name(), "remote-review");
        assert_eq!(descriptors[0].trust(), Some(&SkillTrust::Installed));
    }

    #[tokio::test]
    async fn filesystem_source_reads_bundle_relative_supporting_files() {
        let (root, source) = mounted_source();
        write_root(
            &root,
            "/tenants/tenant-a/users/user-a/skills/local-review/SKILL.md",
            skill_md("local-review", "Local review"),
        )
        .await;
        write_root(
            &root,
            "/tenants/tenant-a/users/user-a/skills/local-review/references/policy.md",
            "policy text",
        )
        .await;

        let bytes = source
            .read_skill_bundle_file(
                &run_context().await,
                &SkillBundleId::new(SkillSourceKind::User, "local-review").unwrap(),
                &SkillFilePath::new("references/policy.md").unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(bytes, b"policy text");
    }

    #[tokio::test]
    async fn filesystem_source_skips_directories_without_skill_md() {
        let (root, source) = mounted_source();
        write_root(
            &root,
            "/tenants/tenant-a/users/user-a/skills/no-manifest/README.md",
            "not a skill",
        )
        .await;

        let descriptors = source
            .list_skill_bundles(&run_context().await)
            .await
            .unwrap();
        assert!(descriptors.is_empty());
    }

    #[tokio::test]
    async fn filesystem_source_rejects_reads_from_directories_without_skill_md() {
        let (root, source) = mounted_source();
        write_root(
            &root,
            "/tenants/tenant-a/users/user-a/skills/no-manifest/references/policy.md",
            "not a skill",
        )
        .await;

        let error = source
            .read_skill_bundle_file(
                &run_context().await,
                &SkillBundleId::new(SkillSourceKind::User, "no-manifest").unwrap(),
                &SkillFilePath::new("references/policy.md").unwrap(),
            )
            .await
            .unwrap_err();
        assert_eq!(error, SkillBundleSourceError::BundleNotFound);
    }

    #[tokio::test]
    async fn filesystem_source_skips_invalid_skill_md_frontmatter() {
        let (root, source) = mounted_source();
        write_root(
            &root,
            "/tenants/tenant-a/users/user-a/skills/bad-skill/SKILL.md",
            "not frontmatter",
        )
        .await;

        let descriptors = source
            .list_skill_bundles(&run_context().await)
            .await
            .unwrap();
        assert!(descriptors.is_empty());
    }

    #[tokio::test]
    async fn filesystem_source_skips_empty_skill_descriptions() {
        let (root, source) = mounted_source();
        write_root(
            &root,
            "/tenants/tenant-a/users/user-a/skills/empty-description/SKILL.md",
            skill_md("empty-description", ""),
        )
        .await;
        write_root(
            &root,
            "/tenants/tenant-a/users/user-a/skills/blank-description/SKILL.md",
            skill_md("blank-description", "   "),
        )
        .await;
        write_root(
            &root,
            "/tenants/tenant-a/users/user-a/skills/valid-skill/SKILL.md",
            skill_md("valid-skill", "Valid skill"),
        )
        .await;

        let descriptors = source
            .list_skill_bundles(&run_context().await)
            .await
            .unwrap();
        let ids = descriptors
            .iter()
            .map(|descriptor| descriptor.id().to_string())
            .collect::<Vec<_>>();
        assert_eq!(ids, vec!["user:valid-skill"]);
    }

    #[tokio::test]
    async fn filesystem_source_skips_non_utf8_skill_md() {
        let (root, source) = mounted_source();
        write_root(
            &root,
            "/tenants/tenant-a/users/user-a/skills/bad-skill/SKILL.md",
            vec![0xff, 0xfe, 0xfd],
        )
        .await;

        let descriptors = source
            .list_skill_bundles(&run_context().await)
            .await
            .unwrap();
        assert!(descriptors.is_empty());
    }

    #[tokio::test]
    async fn filesystem_source_maps_manifest_permission_denied() {
        let root = Arc::new(InMemoryBackend::default());
        write_root(
            &root,
            "/tenants/tenant-a/users/user-a/skills/local-review/SKILL.md",
            skill_md("local-review", "Local review"),
        )
        .await;
        let view = MountView::new(vec![MountGrant::new(
            MountAlias::new("/skills").unwrap(),
            VirtualPath::new("/tenants/tenant-a/users/user-a/skills").unwrap(),
            MountPermissions {
                read: false,
                write: false,
                list: true,
                delete: false,
                execute: false,
            },
        )])
        .unwrap();
        let filesystem = Arc::new(ScopedFilesystem::with_fixed_view(Arc::clone(&root), view));
        let source = FilesystemSkillBundleSource::new(
            filesystem,
            vec![FilesystemSkillBundleRoot::user(
                ScopedPath::new("/skills").unwrap(),
            )],
        )
        .unwrap();

        let error = source
            .list_skill_bundles(&run_context().await)
            .await
            .unwrap_err();
        assert_eq!(error, SkillBundleSourceError::PermissionDenied);
    }

    #[tokio::test]
    async fn filesystem_source_skips_manifest_name_mismatches() {
        let (root, source) = mounted_source();
        write_root(
            &root,
            "/tenants/tenant-a/users/user-a/skills/folder-name/SKILL.md",
            skill_md("manifest-name", "Mismatch"),
        )
        .await;

        let descriptors = source
            .list_skill_bundles(&run_context().await)
            .await
            .unwrap();
        assert!(descriptors.is_empty());
    }

    #[tokio::test]
    async fn filesystem_source_skips_bad_manifest_without_hiding_valid_bundles() {
        let (root, source) = mounted_source();
        write_root(
            &root,
            "/tenants/tenant-a/users/user-a/skills/bad-skill/SKILL.md",
            "not frontmatter",
        )
        .await;
        write_root(
            &root,
            "/tenants/tenant-a/users/user-a/skills/good-skill/SKILL.md",
            skill_md("good-skill", "Good skill"),
        )
        .await;

        let descriptors = source
            .list_skill_bundles(&run_context().await)
            .await
            .unwrap();
        let ids: Vec<String> = descriptors
            .iter()
            .map(|descriptor| descriptor.id().to_string())
            .collect();
        assert_eq!(ids, vec!["user:good-skill"]);
    }

    #[tokio::test]
    async fn filesystem_source_skips_invalid_bundle_folder_names() {
        let (root, source) = mounted_source();
        write_root(
            &root,
            "/tenants/tenant-a/users/user-a/skills/valid-skill/SKILL.md",
            skill_md("valid-skill", "Valid skill"),
        )
        .await;
        for invalid_name in [
            "has space",
            "-starts-with-dash",
            ".starts-with-dot",
            "ümlaut",
        ] {
            write_root(
                &root,
                &format!("/tenants/tenant-a/users/user-a/skills/{invalid_name}/SKILL.md"),
                skill_md(invalid_name, "Invalid skill"),
            )
            .await;
        }

        let descriptors = source
            .list_skill_bundles(&run_context().await)
            .await
            .unwrap();
        let ids: Vec<String> = descriptors
            .iter()
            .map(|descriptor| descriptor.id().to_string())
            .collect();
        assert_eq!(ids, vec!["user:valid-skill"]);
    }

    #[test]
    fn filesystem_source_rejects_path_traversal_in_file_path() {
        for invalid in [
            "../../etc/passwd",
            "../sibling/SKILL.md",
            "..",
            "references/../SKILL.md",
        ] {
            assert_eq!(
                SkillFilePath::new(invalid).unwrap_err(),
                SkillBundleSourceError::InvalidFilePath
            );
        }
    }

    #[tokio::test]
    async fn filesystem_source_returns_bundle_not_found_when_no_root_matches_source_kind() {
        let (_root, source) = mounted_source();

        let error = source
            .read_skill_bundle_file(
                &run_context().await,
                &SkillBundleId::new(SkillSourceKind::TenantShared, "shared-review").unwrap(),
                &SkillFilePath::new("SKILL.md").unwrap(),
            )
            .await
            .unwrap_err();
        assert_eq!(error, SkillBundleSourceError::BundleNotFound);
    }

    #[tokio::test]
    async fn filesystem_source_list_skill_bundles_aborts_on_root_error() {
        let root = Arc::new(InMemoryBackend::default());
        write_root(
            &root,
            "/tenants/tenant-a/users/user-a/skills/local-review/SKILL.md",
            skill_md("local-review", "Local review"),
        )
        .await;
        let view = MountView::new(vec![
            MountGrant::new(
                MountAlias::new("/system/skills").unwrap(),
                VirtualPath::new("/system/skills").unwrap(),
                MountPermissions::none(),
            ),
            MountGrant::new(
                MountAlias::new("/skills").unwrap(),
                VirtualPath::new("/tenants/tenant-a/users/user-a/skills").unwrap(),
                MountPermissions::read_write_list_delete(),
            ),
        ])
        .unwrap();
        let filesystem = Arc::new(ScopedFilesystem::with_fixed_view(root, view));
        let source = FilesystemSkillBundleSource::new(
            filesystem,
            vec![
                FilesystemSkillBundleRoot::system(ScopedPath::new("/system/skills").unwrap()),
                FilesystemSkillBundleRoot::user(ScopedPath::new("/skills").unwrap()),
            ],
        )
        .unwrap();

        let error = source
            .list_skill_bundles(&run_context().await)
            .await
            .unwrap_err();
        assert_eq!(error, SkillBundleSourceError::PermissionDenied);
    }

    #[tokio::test]
    async fn filesystem_source_filters_tenant_shared_roots_by_tenant() {
        let root = Arc::new(InMemoryBackend::default());
        write_root(
            &root,
            "/tenant-shared/tenant-a/skills/team-review/SKILL.md",
            skill_md("team-review", "Team review"),
        )
        .await;
        let view = MountView::new(vec![MountGrant::new(
            MountAlias::new("/tenant-shared/skills").unwrap(),
            VirtualPath::new("/tenant-shared/tenant-a/skills").unwrap(),
            MountPermissions::read_only(),
        )])
        .unwrap();
        let filesystem = Arc::new(ScopedFilesystem::with_fixed_view(root, view));
        let source = FilesystemSkillBundleSource::new(
            filesystem,
            vec![FilesystemSkillBundleRoot::tenant_shared(
                ScopedPath::new("/tenant-shared/skills").unwrap(),
                TenantId::new("tenant-b").unwrap(),
            )],
        )
        .unwrap();

        let descriptors = source
            .list_skill_bundles(&run_context().await)
            .await
            .unwrap();
        assert!(descriptors.is_empty());
    }

    #[tokio::test]
    async fn filesystem_source_rejects_duplicate_source_kind_roots() {
        let root = Arc::new(InMemoryBackend::default());
        let view = MountView::new(vec![MountGrant::new(
            MountAlias::new("/skills").unwrap(),
            VirtualPath::new("/tenants/tenant-a/users/user-a/skills").unwrap(),
            MountPermissions::read_write_list_delete(),
        )])
        .unwrap();
        let filesystem = Arc::new(ScopedFilesystem::with_fixed_view(root, view));

        let error = FilesystemSkillBundleSource::new(
            filesystem,
            vec![
                FilesystemSkillBundleRoot::user(ScopedPath::new("/skills").unwrap()),
                FilesystemSkillBundleRoot::user(ScopedPath::new("/skills/other").unwrap()),
            ],
        )
        .unwrap_err();
        assert_eq!(error, SkillBundleSourceError::DuplicateSourceKind);
    }

    #[tokio::test]
    async fn filesystem_source_enforces_bundle_scan_limit() {
        let (root, source) = mounted_source();
        let source = source.with_max_bundles_per_root(1);
        write_root(
            &root,
            "/tenants/tenant-a/users/user-a/skills/alpha/SKILL.md",
            skill_md("alpha", "Alpha"),
        )
        .await;
        write_root(
            &root,
            "/tenants/tenant-a/users/user-a/skills/beta/SKILL.md",
            skill_md("beta", "Beta"),
        )
        .await;

        let error = source
            .list_skill_bundles(&run_context().await)
            .await
            .unwrap_err();
        assert_eq!(error, SkillBundleSourceError::BundleScanLimitExceeded);
    }

    #[tokio::test]
    async fn filesystem_source_enforces_bounded_reads() {
        let (root, source) = mounted_source();
        let source = source.with_max_bundle_file_bytes(4);
        write_root(
            &root,
            "/tenants/tenant-a/users/user-a/skills/local-review/SKILL.md",
            skill_md("local-review", "Local review"),
        )
        .await;
        write_root(
            &root,
            "/tenants/tenant-a/users/user-a/skills/local-review/references/large.md",
            "too large",
        )
        .await;

        let error = source
            .read_skill_bundle_file(
                &run_context().await,
                &SkillBundleId::new(SkillSourceKind::User, "local-review").unwrap(),
                &SkillFilePath::new("references/large.md").unwrap(),
            )
            .await
            .unwrap_err();
        assert_eq!(error, SkillBundleSourceError::ContentTooLarge);
    }

    #[tokio::test]
    async fn filesystem_source_returns_content_too_large_when_file_grows_between_stat_and_read() {
        let growing_path = VirtualPath::new(
            "/tenants/tenant-a/users/user-a/skills/local-review/references/large.md",
        )
        .unwrap();
        let root = Arc::new(GrowingReadBackend::new(growing_path.clone()));
        root.put(
            &VirtualPath::new("/tenants/tenant-a/users/user-a/skills/local-review/SKILL.md")
                .unwrap(),
            Entry::bytes(skill_md("local-review", "Local review").into_bytes()),
            CasExpectation::Any,
        )
        .await
        .unwrap();
        root.put(
            &growing_path,
            Entry::bytes(b"small before read".to_vec()),
            CasExpectation::Any,
        )
        .await
        .unwrap();
        let view = MountView::new(vec![MountGrant::new(
            MountAlias::new("/skills").unwrap(),
            VirtualPath::new("/tenants/tenant-a/users/user-a/skills").unwrap(),
            MountPermissions::read_write_list_delete(),
        )])
        .unwrap();
        let filesystem = Arc::new(ScopedFilesystem::with_fixed_view(Arc::clone(&root), view));
        let source = FilesystemSkillBundleSource::new(
            filesystem,
            vec![FilesystemSkillBundleRoot::user(
                ScopedPath::new("/skills").unwrap(),
            )],
        )
        .unwrap();

        let error = source
            .read_skill_bundle_file(
                &run_context().await,
                &SkillBundleId::new(SkillSourceKind::User, "local-review").unwrap(),
                &SkillFilePath::new("references/large.md").unwrap(),
            )
            .await
            .unwrap_err();
        assert_eq!(error, SkillBundleSourceError::ContentTooLarge);
    }

    #[tokio::test]
    async fn filesystem_source_enforces_bounded_skill_md_reads() {
        let (root, source) = mounted_source();
        let source = source.with_max_skill_md_bytes(4);
        write_root(
            &root,
            "/tenants/tenant-a/users/user-a/skills/local-review/SKILL.md",
            skill_md("local-review", "Local review"),
        )
        .await;

        let descriptors = source
            .list_skill_bundles(&run_context().await)
            .await
            .unwrap();
        assert!(descriptors.is_empty());
    }
}
