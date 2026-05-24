use std::sync::Arc;

use crate::{MAX_PROMPT_FILE_SIZE, normalize_line_endings, parse_skill_md, validate_skill_name};
use async_trait::async_trait;
use ironclaw_filesystem::{
    BackendCapabilities, DirEntry, FileStat, FileType, FilesystemError, FilesystemOperation,
    RootFilesystem, ScopedFilesystem,
};
use ironclaw_host_api::{HostApiError, MountView, ResourceScope, ScopedPath, VirtualPath};

const USER_SKILLS_ROOT: &str = "/skills";
const SYSTEM_SKILLS_ROOT: &str = "/system/skills";
const SKILL_FILE_NAME: &str = "SKILL.md";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillManagementErrorKind {
    InvalidInput,
    FilesystemDenied,
    NotFound,
    Conflict,
    Resource,
    InvalidSkill,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillManagementError {
    kind: SkillManagementErrorKind,
}

impl SkillManagementError {
    pub fn new(kind: SkillManagementErrorKind) -> Self {
        Self { kind }
    }

    pub fn kind(&self) -> SkillManagementErrorKind {
        self.kind
    }
}

#[derive(Clone)]
pub struct SkillManagementContext {
    filesystem: Arc<ScopedFilesystem<SkillManagementRootFilesystem>>,
    scope: ResourceScope,
}

impl SkillManagementContext {
    pub fn new(
        filesystem: Arc<dyn RootFilesystem>,
        mounts: MountView,
        scope: ResourceScope,
    ) -> Self {
        Self {
            filesystem: Arc::new(ScopedFilesystem::with_fixed_view(
                Arc::new(SkillManagementRootFilesystem { inner: filesystem }),
                mounts,
            )),
            scope,
        }
    }
}

#[derive(Clone)]
struct SkillManagementRootFilesystem {
    inner: Arc<dyn RootFilesystem>,
}

#[async_trait]
impl RootFilesystem for SkillManagementRootFilesystem {
    fn capabilities(&self) -> BackendCapabilities {
        self.inner.capabilities()
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
        self.inner.read_file_bounded(path, max_bytes).await
    }

    async fn write_file(&self, path: &VirtualPath, bytes: &[u8]) -> Result<(), FilesystemError> {
        self.inner.write_file(path, bytes).await
    }

    async fn create_dir_all(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        self.inner.create_dir_all(path).await
    }

    async fn delete(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        self.inner.delete(path).await
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillSummary {
    pub name: String,
    pub version: String,
    pub description: String,
    pub source: SkillSource,
    pub keywords: Vec<String>,
    pub tags: Vec<String>,
    pub requires_skills: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillSource {
    System,
    User,
}

impl SkillSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::User => "user",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SkillInstallRequest<'a> {
    pub name: Option<&'a str>,
    pub content: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillInstallResult {
    pub name: String,
    pub scoped_path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SkillRemoveRequest<'a> {
    pub name: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillRemoveResult {
    pub name: String,
}

pub async fn list_skills(
    context: &SkillManagementContext,
) -> Result<Vec<SkillSummary>, SkillManagementError> {
    let mut skills = Vec::new();
    skills.extend(list_skill_root(context, SYSTEM_SKILLS_ROOT, SkillSource::System).await?);
    skills.extend(list_skill_root(context, USER_SKILLS_ROOT, SkillSource::User).await?);
    Ok(skills)
}

pub async fn install_skill(
    context: &SkillManagementContext,
    request: SkillInstallRequest<'_>,
) -> Result<SkillInstallResult, SkillManagementError> {
    if request.content.len() as u64 > MAX_PROMPT_FILE_SIZE {
        return Err(SkillManagementError::new(
            SkillManagementErrorKind::Resource,
        ));
    }

    let normalized = normalize_line_endings(request.content);
    let parsed = parse_skill_md(&normalized)
        .map_err(|_| SkillManagementError::new(SkillManagementErrorKind::InvalidInput))?;
    if let Some(requested_name) = request.name
        && requested_name != parsed.manifest.name
    {
        return Err(SkillManagementError::new(
            SkillManagementErrorKind::InvalidInput,
        ));
    }

    let skill_name = parsed.manifest.name;
    let skill_dir = skill_root_scoped_path(USER_SKILLS_ROOT, &skill_name)?;
    let skill_path = skill_scoped_path(USER_SKILLS_ROOT, &skill_name, SKILL_FILE_NAME)?;

    if stat_optional(context, &skill_path).await?.is_some() {
        return Err(SkillManagementError::new(
            SkillManagementErrorKind::Conflict,
        ));
    }

    context
        .filesystem
        .create_dir_all(&context.scope, &skill_dir)
        .await
        .or_else(|error| match error {
            FilesystemError::Unsupported {
                operation: FilesystemOperation::CreateDirAll,
                ..
            } => Ok(()),
            other => Err(other),
        })
        .map_err(filesystem_error)?;
    context
        .filesystem
        .write_file(&context.scope, &skill_path, normalized.as_bytes())
        .await
        .map_err(filesystem_error)?;

    Ok(SkillInstallResult {
        name: skill_name.clone(),
        scoped_path: format!("{USER_SKILLS_ROOT}/{skill_name}/{SKILL_FILE_NAME}"),
    })
}

pub async fn remove_skill(
    context: &SkillManagementContext,
    request: SkillRemoveRequest<'_>,
) -> Result<SkillRemoveResult, SkillManagementError> {
    if !validate_skill_name(request.name) {
        return Err(SkillManagementError::new(
            SkillManagementErrorKind::InvalidInput,
        ));
    }
    let skill_dir = skill_root_scoped_path(USER_SKILLS_ROOT, request.name)?;
    let skill_path = skill_scoped_path(USER_SKILLS_ROOT, request.name, SKILL_FILE_NAME)?;
    if stat_optional(context, &skill_path).await?.is_none() {
        return Err(SkillManagementError::new(
            SkillManagementErrorKind::NotFound,
        ));
    }
    context
        .filesystem
        .delete(&context.scope, &skill_dir)
        .await
        .map_err(filesystem_error)?;
    Ok(SkillRemoveResult {
        name: request.name.to_string(),
    })
}

async fn list_skill_root(
    context: &SkillManagementContext,
    scoped_root: &str,
    source: SkillSource,
) -> Result<Vec<SkillSummary>, SkillManagementError> {
    let root = ScopedPath::new(scoped_root)
        .map_err(|_| SkillManagementError::new(SkillManagementErrorKind::InvalidInput))?;
    let entries = match context.filesystem.list_dir(&context.scope, &root).await {
        Ok(entries) => entries,
        Err(FilesystemError::NotFound { .. }) => return Ok(Vec::new()),
        Err(FilesystemError::PermissionDenied { .. }) => return Ok(Vec::new()),
        Err(error) if is_unmounted_scoped_root(&error) => return Ok(Vec::new()),
        Err(error) => return Err(filesystem_error(error)),
    };

    let mut skills = Vec::new();
    for entry in entries {
        if entry.file_type != FileType::Directory {
            continue;
        }
        let name = entry.name.as_str();
        if !validate_skill_name(name) {
            continue;
        }
        let skill_path = skill_scoped_path(scoped_root, name, SKILL_FILE_NAME)?;
        if let Some(skill) = read_skill_summary(context, &skill_path, source).await? {
            skills.push(skill);
        }
    }
    Ok(skills)
}

async fn read_skill_summary(
    context: &SkillManagementContext,
    path: &ScopedPath,
    source: SkillSource,
) -> Result<Option<SkillSummary>, SkillManagementError> {
    let Some(content) = read_skill_file(context, path).await? else {
        return Ok(None);
    };
    let parsed = parse_skill_md(&content)
        .map_err(|_| SkillManagementError::new(SkillManagementErrorKind::InvalidSkill))?;
    Ok(Some(SkillSummary {
        name: parsed.manifest.name,
        version: parsed.manifest.version,
        description: parsed.manifest.description,
        source,
        keywords: parsed.manifest.activation.keywords,
        tags: parsed.manifest.activation.tags,
        requires_skills: parsed.manifest.requires.skills,
    }))
}

fn skill_root_scoped_path(root: &str, name: &str) -> Result<ScopedPath, SkillManagementError> {
    skill_scoped_path(root, name, "")
}

fn is_unmounted_scoped_root(error: &FilesystemError) -> bool {
    matches!(
        error,
        FilesystemError::Contract(HostApiError::InvalidMount { reason, .. })
            if reason == "no mount alias matches scoped path"
    )
}

fn skill_scoped_path(
    root: &str,
    name: &str,
    file_name: &str,
) -> Result<ScopedPath, SkillManagementError> {
    if !validate_skill_name(name) || file_name.contains('/') || file_name.contains('\\') {
        return Err(SkillManagementError::new(
            SkillManagementErrorKind::InvalidInput,
        ));
    }
    let path = if file_name.is_empty() {
        format!("{}/{}", root.trim_end_matches('/'), name)
    } else {
        format!("{}/{}/{}", root.trim_end_matches('/'), name, file_name)
    };
    ScopedPath::new(path)
        .map_err(|_| SkillManagementError::new(SkillManagementErrorKind::InvalidInput))
}

async fn stat_optional(
    context: &SkillManagementContext,
    path: &ScopedPath,
) -> Result<Option<ironclaw_filesystem::FileStat>, SkillManagementError> {
    match context.filesystem.stat(&context.scope, path).await {
        Ok(stat) => Ok(Some(stat)),
        Err(FilesystemError::NotFound { .. }) => Ok(None),
        Err(error) => Err(filesystem_error(error)),
    }
}

async fn read_skill_file(
    context: &SkillManagementContext,
    path: &ScopedPath,
) -> Result<Option<String>, SkillManagementError> {
    let stat = match stat_optional(context, path).await? {
        Some(stat) => stat,
        None => return Ok(None),
    };
    if stat.file_type != FileType::File || stat.sensitive {
        return Ok(None);
    }
    let Some(bytes) = context
        .filesystem
        .read_bytes_bounded(&context.scope, path, MAX_PROMPT_FILE_SIZE as usize)
        .await
        .map_err(filesystem_error)?
    else {
        return Err(SkillManagementError::new(
            SkillManagementErrorKind::Resource,
        ));
    };
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|_| SkillManagementError::new(SkillManagementErrorKind::InvalidSkill))
}

fn filesystem_error(error: FilesystemError) -> SkillManagementError {
    match error {
        FilesystemError::Contract(_) => {
            SkillManagementError::new(SkillManagementErrorKind::InvalidInput)
        }
        FilesystemError::PermissionDenied { .. }
        | FilesystemError::MountNotFound { .. }
        | FilesystemError::PathOutsideMount { .. }
        | FilesystemError::SymlinkEscape { .. }
        | FilesystemError::MountConflict { .. } => {
            SkillManagementError::new(SkillManagementErrorKind::FilesystemDenied)
        }
        FilesystemError::NotFound { .. } => {
            SkillManagementError::new(SkillManagementErrorKind::NotFound)
        }
        FilesystemError::Backend { .. } => {
            SkillManagementError::new(SkillManagementErrorKind::InvalidSkill)
        }
        FilesystemError::Unsupported { .. }
        | FilesystemError::VersionMismatch { .. }
        | FilesystemError::IndexConflict { .. } => {
            SkillManagementError::new(SkillManagementErrorKind::FilesystemDenied)
        }
        _ => SkillManagementError::new(SkillManagementErrorKind::FilesystemDenied),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_filesystem::InMemoryBackend;
    use ironclaw_host_api::{MountAlias, MountGrant, MountPermissions};

    #[tokio::test]
    async fn install_list_and_remove_user_skills_through_scoped_mounts() {
        let filesystem = Arc::new(InMemoryBackend::default());
        write_file(
            filesystem.as_ref(),
            "/projects/system/skills/system-helper/SKILL.md",
            skill_md(
                "system-helper",
                "system skill description",
                "SYSTEM_SKILL_PROMPT",
            ),
        )
        .await;
        let context = skill_management_context(filesystem.clone(), skill_mounts());

        let installed = install_skill(
            &context,
            SkillInstallRequest {
                name: None,
                content: &skill_md(
                    "local-helper",
                    "local skill description",
                    "LOCAL_SKILL_PROMPT",
                ),
            },
        )
        .await
        .unwrap();
        assert_eq!(installed.name, "local-helper");
        assert_eq!(
            installed.scoped_path,
            "/skills/local-helper/SKILL.md".to_string()
        );

        let listed = list_skills(&context).await.unwrap();
        assert_eq!(listed.len(), 2);
        assert!(
            listed
                .iter()
                .any(|skill| skill.name == "system-helper" && skill.source == SkillSource::System)
        );
        assert!(
            listed
                .iter()
                .any(|skill| skill.name == "local-helper" && skill.source == SkillSource::User)
        );

        let removed = remove_skill(
            &context,
            SkillRemoveRequest {
                name: "local-helper",
            },
        )
        .await
        .unwrap();
        assert_eq!(removed.name, "local-helper");
        assert_eq!(list_skills(&context).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn install_rejects_name_mismatch() {
        let filesystem = Arc::new(InMemoryBackend::default());
        let context = skill_management_context(filesystem, skill_mounts());

        let error = install_skill(
            &context,
            SkillInstallRequest {
                name: Some("expected"),
                content: &skill_md("actual", "description", "PROMPT"),
            },
        )
        .await
        .unwrap_err();

        assert_eq!(error.kind(), SkillManagementErrorKind::InvalidInput);
    }

    #[tokio::test]
    async fn list_treats_unmounted_optional_skill_root_as_empty() {
        let filesystem = Arc::new(InMemoryBackend::default());
        write_file(
            filesystem.as_ref(),
            "/projects/skills/local-helper/SKILL.md",
            skill_md("local-helper", "local skill description", "PROMPT"),
        )
        .await;
        let context = skill_management_context(filesystem, user_skill_mounts());

        let listed = list_skills(&context).await.unwrap();

        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "local-helper");
        assert_eq!(listed[0].source, SkillSource::User);
    }

    #[tokio::test]
    async fn remove_rejects_system_skill() {
        let filesystem = Arc::new(InMemoryBackend::default());
        write_file(
            filesystem.as_ref(),
            "/projects/system/skills/system-helper/SKILL.md",
            skill_md("system-helper", "system skill description", "PROMPT"),
        )
        .await;
        let context = skill_management_context(filesystem, skill_mounts());

        let error = remove_skill(
            &context,
            SkillRemoveRequest {
                name: "system-helper",
            },
        )
        .await
        .unwrap_err();

        assert_eq!(error.kind(), SkillManagementErrorKind::NotFound);
    }

    async fn write_file(root: &InMemoryBackend, path: &str, body: String) {
        root.write_file(&VirtualPath::new(path).unwrap(), body.as_bytes())
            .await
            .unwrap();
    }

    fn skill_mounts() -> MountView {
        MountView::new(vec![
            MountGrant::new(
                MountAlias::new("/skills").unwrap(),
                VirtualPath::new("/projects/skills").unwrap(),
                MountPermissions::read_write_list_delete(),
            ),
            MountGrant::new(
                MountAlias::new("/system/skills").unwrap(),
                VirtualPath::new("/projects/system/skills").unwrap(),
                MountPermissions::read_only(),
            ),
        ])
        .unwrap()
    }

    fn user_skill_mounts() -> MountView {
        MountView::new(vec![MountGrant::new(
            MountAlias::new("/skills").unwrap(),
            VirtualPath::new("/projects/skills").unwrap(),
            MountPermissions::read_write_list_delete(),
        )])
        .unwrap()
    }

    fn skill_management_context(
        filesystem: Arc<InMemoryBackend>,
        mounts: MountView,
    ) -> SkillManagementContext {
        let filesystem: Arc<dyn RootFilesystem> = filesystem;
        SkillManagementContext::new(filesystem, mounts, ResourceScope::system())
    }

    fn skill_md(name: &str, description: &str, prompt: &str) -> String {
        format!("---\nname: {name}\ndescription: {description}\n---\n{prompt}\n")
    }
}
