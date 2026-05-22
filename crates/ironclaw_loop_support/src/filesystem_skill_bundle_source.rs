use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_filesystem::{FileType, FilesystemError, RootFilesystem, ScopedFilesystem};
use ironclaw_host_api::{ResourceScope, ScopedPath};
use ironclaw_skills::{MAX_PROMPT_FILE_SIZE, SkillTrust, normalize_line_endings, parse_skill_md};
use ironclaw_turns::run_profile::{LoopRunContext, SkillVisibility};

use crate::{
    SkillBundleDescriptor, SkillBundleId, SkillBundleProvenance, SkillBundleSource,
    SkillBundleSourceError, SkillFilePath, SkillSourceKind, sort_skill_bundle_descriptors,
};

const DEFAULT_MAX_BUNDLE_FILE_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilesystemSkillBundleRoot {
    source_kind: SkillSourceKind,
    root: ScopedPath,
    trust: Option<SkillTrust>,
    visibility: Option<SkillVisibility>,
}

impl FilesystemSkillBundleRoot {
    pub fn new(
        source_kind: SkillSourceKind,
        root: ScopedPath,
        trust: Option<SkillTrust>,
        visibility: Option<SkillVisibility>,
    ) -> Self {
        Self {
            source_kind,
            root,
            trust,
            visibility,
        }
    }

    pub fn system(root: ScopedPath) -> Self {
        Self::new(
            SkillSourceKind::System,
            root,
            Some(SkillTrust::Trusted),
            Some(SkillVisibility::Visible),
        )
    }

    pub fn user(root: ScopedPath) -> Self {
        Self::new(
            SkillSourceKind::User,
            root,
            Some(SkillTrust::Trusted),
            Some(SkillVisibility::Visible),
        )
    }

    pub fn tenant_shared(root: ScopedPath) -> Self {
        Self::new(
            SkillSourceKind::TenantShared,
            root,
            Some(SkillTrust::Installed),
            Some(SkillVisibility::Visible),
        )
    }

    pub fn source_kind(&self) -> SkillSourceKind {
        self.source_kind
    }

    pub fn root(&self) -> &ScopedPath {
        &self.root
    }

    pub fn trust(&self) -> Option<&SkillTrust> {
        self.trust.as_ref()
    }

    pub fn visibility(&self) -> Option<&SkillVisibility> {
        self.visibility.as_ref()
    }
}

pub struct FilesystemSkillBundleSource<F> {
    filesystem: Arc<ScopedFilesystem<F>>,
    roots: Vec<FilesystemSkillBundleRoot>,
    max_skill_md_bytes: usize,
    max_bundle_file_bytes: usize,
}

impl<F> std::fmt::Debug for FilesystemSkillBundleSource<F> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FilesystemSkillBundleSource")
            .field("filesystem", &"<ScopedFilesystem>")
            .field("roots", &self.roots)
            .field("max_skill_md_bytes", &self.max_skill_md_bytes)
            .field("max_bundle_file_bytes", &self.max_bundle_file_bytes)
            .finish()
    }
}

impl<F> FilesystemSkillBundleSource<F>
where
    F: RootFilesystem,
{
    pub fn new(
        filesystem: Arc<ScopedFilesystem<F>>,
        roots: Vec<FilesystemSkillBundleRoot>,
    ) -> Self {
        Self {
            filesystem,
            roots,
            max_skill_md_bytes: MAX_PROMPT_FILE_SIZE as usize,
            max_bundle_file_bytes: DEFAULT_MAX_BUNDLE_FILE_BYTES,
        }
    }

    pub fn with_max_skill_md_bytes(mut self, max_skill_md_bytes: usize) -> Self {
        self.max_skill_md_bytes = max_skill_md_bytes;
        self
    }

    pub fn with_max_bundle_file_bytes(mut self, max_bundle_file_bytes: usize) -> Self {
        self.max_bundle_file_bytes = max_bundle_file_bytes;
        self
    }

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

        for entry in entries {
            if entry.file_type != FileType::Directory {
                continue;
            }
            let bundle_id = match SkillBundleId::new(root.source_kind(), &entry.name) {
                Ok(bundle_id) => bundle_id,
                Err(_) => continue,
            };
            let skill_md_path = bundle_scoped_path(root.root(), bundle_id.name(), "SKILL.md")?;
            match self
                .validate_bundle_manifest(scope, &skill_md_path, &bundle_id)
                .await
            {
                Ok(()) => {}
                Err(SkillBundleSourceError::FileNotFound) => continue,
                Err(error) => return Err(error),
            }

            descriptors.push(
                SkillBundleDescriptor::new(
                    bundle_id,
                    root.trust().cloned(),
                    root.visibility().copied(),
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
    ) -> Result<(), SkillBundleSourceError> {
        let skill_md = self
            .read_bounded(scope, skill_md_path, self.max_skill_md_bytes)
            .await?;
        let skill_md =
            String::from_utf8(skill_md).map_err(|_| SkillBundleSourceError::InvalidSkillBundle)?;
        let skill_md = normalize_line_endings(&skill_md);
        let parsed =
            parse_skill_md(&skill_md).map_err(|_| SkillBundleSourceError::InvalidSkillBundle)?;
        if parsed.manifest.name != bundle_id.name() {
            return Err(SkillBundleSourceError::InvalidSkillBundle);
        }
        Ok(())
    }

    async fn read_bounded(
        &self,
        scope: &ResourceScope,
        path: &ScopedPath,
        max_bytes: usize,
    ) -> Result<Vec<u8>, SkillBundleSourceError> {
        let stat = self
            .filesystem
            .stat(scope, path)
            .await
            .map_err(map_file_read_error)?;
        if stat.file_type != FileType::File {
            return Err(SkillBundleSourceError::FileNotFound);
        }
        if stat.len > max_bytes as u64 {
            return Err(SkillBundleSourceError::ContentTooLarge);
        }
        let bytes = self
            .filesystem
            .read_bytes(scope, path)
            .await
            .map_err(map_file_read_error)?;
        if bytes.len() > max_bytes {
            return Err(SkillBundleSourceError::ContentTooLarge);
        }
        Ok(bytes)
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
            .find(|root| root.source_kind() == bundle_id.source_kind())
            .ok_or(SkillBundleSourceError::BundleNotFound)?;
        let scope = resource_scope_for_run(run_context);
        if path.as_str() != "SKILL.md" {
            let skill_md_path = bundle_scoped_path(root.root(), bundle_id.name(), "SKILL.md")?;
            self.validate_bundle_manifest(&scope, &skill_md_path, bundle_id)
                .await
                .map_err(|error| match error {
                    SkillBundleSourceError::FileNotFound => SkillBundleSourceError::BundleNotFound,
                    other => other,
                })?;
        }
        let scoped_path = bundle_scoped_path(root.root(), bundle_id.name(), path.as_str())?;
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

fn bundle_scoped_path(
    root: &ScopedPath,
    bundle_name: &str,
    path: &str,
) -> Result<ScopedPath, SkillBundleSourceError> {
    ScopedPath::new(format!(
        "{}/{}/{}",
        root.as_str().trim_end_matches('/'),
        bundle_name,
        path
    ))
    .map_err(|_| SkillBundleSourceError::InvalidFilePath)
}

fn map_file_read_error(error: FilesystemError) -> SkillBundleSourceError {
    if is_not_found(&error) {
        return SkillBundleSourceError::FileNotFound;
    }
    map_filesystem_error(error)
}

fn map_filesystem_error(error: FilesystemError) -> SkillBundleSourceError {
    match error {
        FilesystemError::PermissionDenied { .. } => SkillBundleSourceError::PermissionDenied,
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
        _ => SkillBundleSourceError::Internal,
    }
}

fn is_not_found(error: &FilesystemError) -> bool {
    matches!(error, FilesystemError::NotFound { .. })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_filesystem::{CasExpectation, Entry, InMemoryBackend, RootFilesystem};
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
        );
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
    async fn filesystem_source_rejects_invalid_skill_md_frontmatter() {
        let (root, source) = mounted_source();
        write_root(
            &root,
            "/tenants/tenant-a/users/user-a/skills/bad-skill/SKILL.md",
            "not frontmatter",
        )
        .await;

        let error = source
            .list_skill_bundles(&run_context().await)
            .await
            .unwrap_err();
        assert_eq!(error, SkillBundleSourceError::InvalidSkillBundle);
    }

    #[tokio::test]
    async fn filesystem_source_rejects_manifest_name_mismatches() {
        let (root, source) = mounted_source();
        write_root(
            &root,
            "/tenants/tenant-a/users/user-a/skills/folder-name/SKILL.md",
            skill_md("manifest-name", "Mismatch"),
        )
        .await;

        let error = source
            .list_skill_bundles(&run_context().await)
            .await
            .unwrap_err();
        assert_eq!(error, SkillBundleSourceError::InvalidSkillBundle);
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
    async fn filesystem_source_enforces_bounded_skill_md_reads() {
        let (root, source) = mounted_source();
        let source = source.with_max_skill_md_bytes(4);
        write_root(
            &root,
            "/tenants/tenant-a/users/user-a/skills/local-review/SKILL.md",
            skill_md("local-review", "Local review"),
        )
        .await;

        let error = source
            .list_skill_bundles(&run_context().await)
            .await
            .unwrap_err();
        assert_eq!(error, SkillBundleSourceError::ContentTooLarge);
    }
}
