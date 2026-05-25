use std::{path::Component, sync::Arc};

use crate::{MAX_PROMPT_FILE_SIZE, normalize_line_endings, parse_skill_md, validate_skill_name};
use async_trait::async_trait;
use ironclaw_filesystem::{
    BackendCapabilities, DirEntry, FileStat, FileType, FilesystemError, FilesystemOperation,
    RootFilesystem, ScopedFilesystem,
};
use ironclaw_host_api::{HostApiError, MountView, ResourceScope, ScopedPath, VirtualPath};
use serde::Deserialize;
use serde_json::json;

const USER_SKILLS_ROOT: &str = "/skills";
const SYSTEM_SKILLS_ROOT: &str = "/system/skills";
const SKILL_FILE_NAME: &str = "SKILL.md";
const INSTALL_METADATA_FILE_NAME: &str = ".ironclaw-install.json";
pub const MAX_INSTALL_BUNDLE_FILES: usize = 256;
const MAX_INSTALL_BUNDLE_FILE_BYTES: usize = 2 * 1024 * 1024;
const MAX_INSTALL_BUNDLE_TOTAL_BYTES: usize = 20 * 1024 * 1024;

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
    Installed,
}

impl SkillSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::User => "user",
            Self::Installed => "installed",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillInstallSource {
    User,
    InstalledUrl,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SkillInstallRequest<'a> {
    pub name: Option<&'a str>,
    pub content: &'a str,
    pub files: &'a [SkillInstallFile<'a>],
    pub source: SkillInstallSource,
    pub source_url: Option<&'a str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SkillInstallFile<'a> {
    pub relative_path: &'a str,
    pub contents: &'a [u8],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillInstallResult {
    pub name: String,
    pub scoped_path: String,
    pub source: SkillSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SkillRemoveRequest<'a> {
    pub name: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillRemoveResult {
    pub name: String,
}

#[tracing::instrument(level = "debug", skip(context))]
pub async fn list_skills(
    context: &SkillManagementContext,
) -> Result<Vec<SkillSummary>, SkillManagementError> {
    let mut skills = Vec::new();
    skills.extend(list_skill_root(context, SYSTEM_SKILLS_ROOT, SkillSource::System).await?);
    skills.extend(list_skill_root(context, USER_SKILLS_ROOT, SkillSource::User).await?);
    tracing::debug!(skill_count = skills.len(), "skill management listed skills");
    Ok(skills)
}

#[tracing::instrument(
    level = "debug",
    skip(context, request),
    fields(
        requested_name = request.name.unwrap_or("<none>"),
        content_bytes = request.content.len(),
    )
)]
pub async fn install_skill(
    context: &SkillManagementContext,
    request: SkillInstallRequest<'_>,
) -> Result<SkillInstallResult, SkillManagementError> {
    tracing::debug!("skill install started");
    if request.content.len() as u64 > MAX_PROMPT_FILE_SIZE {
        tracing::debug!(
            max_bytes = MAX_PROMPT_FILE_SIZE,
            "skill install rejected oversized content"
        );
        return Err(SkillManagementError::new(
            SkillManagementErrorKind::Resource,
        ));
    }

    let normalized = normalize_line_endings(request.content);
    let parsed = parse_skill_md(&normalized).map_err(|_| {
        tracing::debug!("skill install failed to parse SKILL.md content");
        SkillManagementError::new(SkillManagementErrorKind::InvalidInput)
    })?;
    if let Some(requested_name) = request.name
        && requested_name != parsed.manifest.name
    {
        tracing::debug!(
            requested_name,
            parsed_name = %parsed.manifest.name,
            "skill install rejected name mismatch"
        );
        return Err(SkillManagementError::new(
            SkillManagementErrorKind::InvalidInput,
        ));
    }
    validate_install_bundle_files(request.files)?;

    let skill_name = parsed.manifest.name;
    let skill_dir = skill_root_scoped_path(USER_SKILLS_ROOT, &skill_name)?;
    let skill_path = skill_scoped_path(USER_SKILLS_ROOT, &skill_name, SKILL_FILE_NAME)?;

    log_skill_filesystem_phase("stat_existing", &skill_name, &skill_path);
    if stat_optional(context, &skill_path).await?.is_some() {
        tracing::debug!(
            skill_name = %skill_name,
            scoped_path = %skill_path,
            "skill install rejected existing skill"
        );
        return Err(SkillManagementError::new(
            SkillManagementErrorKind::Conflict,
        ));
    }

    log_skill_filesystem_phase("create_dir_all", &skill_name, &skill_dir);
    context
        .filesystem
        .create_dir_all(&context.scope, &skill_dir)
        .await
        .or_else(|error| match error {
            FilesystemError::Unsupported {
                operation: FilesystemOperation::CreateDirAll,
                ..
            } => {
                log_skill_filesystem_phase("create_dir_all_unsupported", &skill_name, &skill_dir);
                Ok(())
            }
            other => Err(other),
        })
        .map_err(|error| {
            log_skill_filesystem_phase("create_dir_all_failed", &skill_name, &skill_dir);
            filesystem_error(error)
        })?;
    for file in request.files {
        let relative_path = normalize_install_relative_path(file.relative_path)?;
        let file_path =
            skill_bundle_file_scoped_path(USER_SKILLS_ROOT, &skill_name, &relative_path)?;
        if let Some(parent) = scoped_parent(&file_path)? {
            log_skill_filesystem_phase("create_bundle_parent", &skill_name, &parent);
            context
                .filesystem
                .create_dir_all(&context.scope, &parent)
                .await
                .or_else(|error| match error {
                    FilesystemError::Unsupported {
                        operation: FilesystemOperation::CreateDirAll,
                        ..
                    } => {
                        log_skill_filesystem_phase(
                            "create_bundle_parent_unsupported",
                            &skill_name,
                            &parent,
                        );
                        Ok(())
                    }
                    other => Err(other),
                })
                .map_err(|error| {
                    log_skill_filesystem_phase("create_bundle_parent_failed", &skill_name, &parent);
                    filesystem_error(error)
                })?;
        }
        log_skill_filesystem_phase("write_bundle_file", &skill_name, &file_path);
        context
            .filesystem
            .write_file(&context.scope, &file_path, file.contents)
            .await
            .map_err(|error| {
                log_skill_filesystem_phase("write_bundle_file_failed", &skill_name, &file_path);
                filesystem_error(error)
            })?;
    }
    if request.source == SkillInstallSource::InstalledUrl {
        let metadata_path = skill_bundle_file_scoped_path(
            USER_SKILLS_ROOT,
            &skill_name,
            INSTALL_METADATA_FILE_NAME,
        )?;
        let metadata = install_metadata_bytes(request.source_url)?;
        log_skill_filesystem_phase("write_install_metadata", &skill_name, &metadata_path);
        context
            .filesystem
            .write_file(&context.scope, &metadata_path, &metadata)
            .await
            .map_err(|error| {
                log_skill_filesystem_phase(
                    "write_install_metadata_failed",
                    &skill_name,
                    &metadata_path,
                );
                filesystem_error(error)
            })?;
    }
    log_skill_filesystem_phase("write_file", &skill_name, &skill_path);
    context
        .filesystem
        .write_file(&context.scope, &skill_path, normalized.as_bytes())
        .await
        .map_err(|error| {
            log_skill_filesystem_phase("write_file_failed", &skill_name, &skill_path);
            filesystem_error(error)
        })?;
    tracing::debug!(
        skill_name = %skill_name,
        scoped_path = %skill_path,
        bundle_file_count = request.files.len(),
        "skill install completed"
    );

    Ok(SkillInstallResult {
        name: skill_name.clone(),
        scoped_path: format!("{USER_SKILLS_ROOT}/{skill_name}/{SKILL_FILE_NAME}"),
        source: installed_skill_source(request.source),
    })
}

#[tracing::instrument(
    level = "debug",
    skip(context, request),
    fields(skill_name = %request.name)
)]
pub async fn remove_skill(
    context: &SkillManagementContext,
    request: SkillRemoveRequest<'_>,
) -> Result<SkillRemoveResult, SkillManagementError> {
    tracing::debug!("skill remove started");
    if !validate_skill_name(request.name) {
        tracing::debug!("skill remove rejected invalid name");
        return Err(SkillManagementError::new(
            SkillManagementErrorKind::InvalidInput,
        ));
    }
    let skill_dir = skill_root_scoped_path(USER_SKILLS_ROOT, request.name)?;
    let skill_path = skill_scoped_path(USER_SKILLS_ROOT, request.name, SKILL_FILE_NAME)?;
    log_skill_filesystem_phase("stat_existing", request.name, &skill_path);
    if stat_optional(context, &skill_path).await?.is_none() {
        tracing::debug!(
            scoped_path = %skill_path,
            "skill remove could not find installed skill"
        );
        return Err(SkillManagementError::new(
            SkillManagementErrorKind::NotFound,
        ));
    }
    log_skill_filesystem_phase("delete_dir", request.name, &skill_dir);
    context
        .filesystem
        .delete(&context.scope, &skill_dir)
        .await
        .map_err(|error| {
            log_skill_filesystem_phase("delete_dir_failed", request.name, &skill_dir);
            filesystem_error(error)
        })?;
    tracing::debug!("skill remove completed");
    Ok(SkillRemoveResult {
        name: request.name.to_string(),
    })
}

#[tracing::instrument(
    level = "debug",
    skip(context),
    fields(scoped_root = %scoped_root, source = source.as_str())
)]
async fn list_skill_root(
    context: &SkillManagementContext,
    scoped_root: &str,
    source: SkillSource,
) -> Result<Vec<SkillSummary>, SkillManagementError> {
    tracing::debug!("skill management listing skill root");
    let root = ScopedPath::new(scoped_root)
        .map_err(|_| SkillManagementError::new(SkillManagementErrorKind::InvalidInput))?;
    let entries = match context.filesystem.list_dir(&context.scope, &root).await {
        Ok(entries) => entries,
        Err(FilesystemError::NotFound { .. }) => {
            tracing::debug!("skill management skill root not found");
            return Ok(Vec::new());
        }
        Err(FilesystemError::PermissionDenied { .. }) => {
            tracing::debug!("skill management skill root permission denied");
            return Ok(Vec::new());
        }
        Err(error) if is_unmounted_scoped_root(&error) => {
            tracing::debug!("skill management skill root is not mounted");
            return Ok(Vec::new());
        }
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
    tracing::debug!(
        skill_count = skills.len(),
        "skill management listed skill root"
    );
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
    let parsed = parse_skill_md(&content).map_err(|_| {
        tracing::debug!(
            scoped_path = %path,
            "skill management failed to parse skill summary"
        );
        SkillManagementError::new(SkillManagementErrorKind::InvalidSkill)
    })?;
    tracing::debug!(
        scoped_path = %path,
        skill_name = %parsed.manifest.name,
        "skill management parsed skill summary"
    );
    let source = skill_source_with_install_metadata(context, path, source).await?;
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

#[derive(Debug, Deserialize)]
struct InstallMetadata {
    source: Option<String>,
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

fn skill_bundle_file_scoped_path(
    root: &str,
    name: &str,
    relative_path: &str,
) -> Result<ScopedPath, SkillManagementError> {
    if !validate_skill_name(name) {
        return Err(SkillManagementError::new(
            SkillManagementErrorKind::InvalidInput,
        ));
    }
    ScopedPath::new(format!(
        "{}/{}/{}",
        root.trim_end_matches('/'),
        name,
        relative_path
    ))
    .map_err(|_| SkillManagementError::new(SkillManagementErrorKind::InvalidInput))
}

fn validate_install_bundle_files(
    files: &[SkillInstallFile<'_>],
) -> Result<(), SkillManagementError> {
    if files.len() > MAX_INSTALL_BUNDLE_FILES {
        return Err(SkillManagementError::new(
            SkillManagementErrorKind::Resource,
        ));
    }
    let mut total_bytes = 0usize;
    for file in files {
        if file.contents.len() > MAX_INSTALL_BUNDLE_FILE_BYTES {
            return Err(SkillManagementError::new(
                SkillManagementErrorKind::Resource,
            ));
        }
        total_bytes = total_bytes
            .checked_add(file.contents.len())
            .ok_or_else(|| SkillManagementError::new(SkillManagementErrorKind::Resource))?;
        if total_bytes > MAX_INSTALL_BUNDLE_TOTAL_BYTES {
            return Err(SkillManagementError::new(
                SkillManagementErrorKind::Resource,
            ));
        }
        normalize_install_relative_path(file.relative_path)?;
    }
    Ok(())
}

fn normalize_install_relative_path(path: &str) -> Result<String, SkillManagementError> {
    if path.is_empty()
        || path.starts_with('/')
        || path.contains('\\')
        || path.contains('\0')
        || path.chars().any(char::is_control)
        || path.contains("://")
    {
        return Err(SkillManagementError::new(
            SkillManagementErrorKind::InvalidInput,
        ));
    }

    let mut parts = Vec::new();
    for component in std::path::Path::new(path).components() {
        match component {
            Component::Normal(part) => {
                let part = part.to_str().ok_or_else(|| {
                    SkillManagementError::new(SkillManagementErrorKind::InvalidInput)
                })?;
                if part.is_empty() {
                    return Err(SkillManagementError::new(
                        SkillManagementErrorKind::InvalidInput,
                    ));
                }
                parts.push(part);
            }
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(SkillManagementError::new(
                    SkillManagementErrorKind::InvalidInput,
                ));
            }
        }
    }

    if parts.is_empty() || parts == [SKILL_FILE_NAME] || parts == [INSTALL_METADATA_FILE_NAME] {
        return Err(SkillManagementError::new(
            SkillManagementErrorKind::InvalidInput,
        ));
    }
    Ok(parts.join("/"))
}

fn installed_skill_source(source: SkillInstallSource) -> SkillSource {
    match source {
        SkillInstallSource::User => SkillSource::User,
        SkillInstallSource::InstalledUrl => SkillSource::Installed,
    }
}

fn install_metadata_bytes(source_url: Option<&str>) -> Result<Vec<u8>, SkillManagementError> {
    serde_json::to_vec_pretty(&json!({
        "source": "installed_url",
        "source_url": source_url,
    }))
    .map_err(|_| SkillManagementError::new(SkillManagementErrorKind::InvalidInput))
}

async fn skill_source_with_install_metadata(
    context: &SkillManagementContext,
    skill_path: &ScopedPath,
    default_source: SkillSource,
) -> Result<SkillSource, SkillManagementError> {
    if default_source != SkillSource::User {
        return Ok(default_source);
    }
    let Some(metadata_path) = scoped_sibling(skill_path, INSTALL_METADATA_FILE_NAME)? else {
        return Ok(default_source);
    };
    let bytes = match context
        .filesystem
        .read_bytes_bounded(&context.scope, &metadata_path, 4096)
        .await
    {
        Ok(Some(bytes)) => bytes,
        Ok(None) | Err(FilesystemError::NotFound { .. }) => return Ok(default_source),
        Err(error) => return Err(filesystem_error(error)),
    };
    let Ok(metadata) = serde_json::from_slice::<InstallMetadata>(&bytes) else {
        return Ok(default_source);
    };
    if metadata.source.as_deref() == Some("installed_url") {
        Ok(SkillSource::Installed)
    } else {
        Ok(default_source)
    }
}

fn scoped_sibling(
    path: &ScopedPath,
    sibling: &str,
) -> Result<Option<ScopedPath>, SkillManagementError> {
    let Some((parent, _)) = path.as_str().rsplit_once('/') else {
        return Ok(None);
    };
    if parent.is_empty() {
        return Ok(None);
    }
    ScopedPath::new(format!("{parent}/{sibling}"))
        .map(Some)
        .map_err(|_| SkillManagementError::new(SkillManagementErrorKind::InvalidInput))
}

fn scoped_parent(path: &ScopedPath) -> Result<Option<ScopedPath>, SkillManagementError> {
    let Some((parent, _)) = path.as_str().rsplit_once('/') else {
        return Ok(None);
    };
    if parent.is_empty() || parent == USER_SKILLS_ROOT {
        return Ok(None);
    }
    ScopedPath::new(parent.to_string())
        .map(Some)
        .map_err(|_| SkillManagementError::new(SkillManagementErrorKind::InvalidInput))
}

async fn stat_optional(
    context: &SkillManagementContext,
    path: &ScopedPath,
) -> Result<Option<ironclaw_filesystem::FileStat>, SkillManagementError> {
    match context.filesystem.stat(&context.scope, path).await {
        Ok(stat) => Ok(Some(stat)),
        Err(FilesystemError::NotFound { .. }) => Ok(None),
        Err(error) => {
            tracing::debug!(scoped_path = %path, "skill management stat failed");
            Err(filesystem_error(error))
        }
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
        tracing::debug!(
            scoped_path = %path,
            file_type = ?stat.file_type,
            sensitive = stat.sensitive,
            "skill management skipped non-readable skill file"
        );
        return Ok(None);
    }
    let Some(bytes) = context
        .filesystem
        .read_bytes_bounded(&context.scope, path, MAX_PROMPT_FILE_SIZE as usize)
        .await
        .map_err(|error| {
            tracing::debug!(scoped_path = %path, "skill management failed to read skill file");
            filesystem_error(error)
        })?
    else {
        tracing::debug!(
            scoped_path = %path,
            max_bytes = MAX_PROMPT_FILE_SIZE,
            "skill management skill file exceeded read bound"
        );
        return Err(SkillManagementError::new(
            SkillManagementErrorKind::Resource,
        ));
    };
    let content = String::from_utf8(bytes).map_err(|_| {
        tracing::debug!(scoped_path = %path, "skill management skill file is not UTF-8");
        SkillManagementError::new(SkillManagementErrorKind::InvalidSkill)
    })?;
    Ok(Some(content))
}

fn log_skill_filesystem_phase(phase: &'static str, skill_name: &str, scoped_path: &ScopedPath) {
    tracing::debug!(
        phase,
        skill_name = %skill_name,
        scoped_path = %scoped_path,
        "skill management filesystem phase"
    );
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
                files: &[],
                source: SkillInstallSource::User,
                source_url: None,
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
                files: &[],
                source: SkillInstallSource::User,
                source_url: None,
            },
        )
        .await
        .unwrap_err();

        assert_eq!(error.kind(), SkillManagementErrorKind::InvalidInput);
    }

    #[tokio::test]
    async fn install_rejects_invalid_bundle_files() {
        let cases = [
            (
                "../escape.md",
                b"ok".as_slice(),
                SkillManagementErrorKind::InvalidInput,
            ),
            (
                "/absolute.md",
                b"ok".as_slice(),
                SkillManagementErrorKind::InvalidInput,
            ),
            (
                "SKILL.md",
                b"ok".as_slice(),
                SkillManagementErrorKind::InvalidInput,
            ),
            (
                ".ironclaw-install.json",
                b"ok".as_slice(),
                SkillManagementErrorKind::InvalidInput,
            ),
        ];

        for (relative_path, contents, expected) in cases {
            let filesystem = Arc::new(InMemoryBackend::default());
            let context = skill_management_context(filesystem, skill_mounts());

            let error = install_skill(
                &context,
                SkillInstallRequest {
                    name: None,
                    content: &skill_md("bundle-helper", "description", "PROMPT"),
                    files: &[SkillInstallFile {
                        relative_path,
                        contents,
                    }],
                    source: SkillInstallSource::User,
                    source_url: None,
                },
            )
            .await
            .unwrap_err();

            assert_eq!(error.kind(), expected);
        }

        let oversized = vec![b'x'; MAX_INSTALL_BUNDLE_FILE_BYTES + 1];
        let filesystem = Arc::new(InMemoryBackend::default());
        let context = skill_management_context(filesystem, skill_mounts());
        let error = install_skill(
            &context,
            SkillInstallRequest {
                name: None,
                content: &skill_md("oversized-helper", "description", "PROMPT"),
                files: &[SkillInstallFile {
                    relative_path: "references/large.bin",
                    contents: &oversized,
                }],
                source: SkillInstallSource::User,
                source_url: None,
            },
        )
        .await
        .unwrap_err();
        assert_eq!(error.kind(), SkillManagementErrorKind::Resource);

        let paths = (0..=MAX_INSTALL_BUNDLE_FILES)
            .map(|index| format!("references/{index}.md"))
            .collect::<Vec<_>>();
        let files = paths
            .iter()
            .map(|path| SkillInstallFile {
                relative_path: path.as_str(),
                contents: b"ok",
            })
            .collect::<Vec<_>>();
        let filesystem = Arc::new(InMemoryBackend::default());
        let context = skill_management_context(filesystem, skill_mounts());
        let error = install_skill(
            &context,
            SkillInstallRequest {
                name: None,
                content: &skill_md("too-many-helper", "description", "PROMPT"),
                files: &files,
                source: SkillInstallSource::User,
                source_url: None,
            },
        )
        .await
        .unwrap_err();
        assert_eq!(error.kind(), SkillManagementErrorKind::Resource);
    }

    #[tokio::test]
    async fn install_bundle_failure_does_not_publish_skill_md() {
        let inner = Arc::new(InMemoryBackend::default());
        let filesystem = Arc::new(FailingBundleWriteFilesystem {
            inner: inner.clone(),
        });
        let context = skill_management_context_with_root(filesystem, skill_mounts());

        let error = install_skill(
            &context,
            SkillInstallRequest {
                name: None,
                content: &skill_md("partial-helper", "description", "PROMPT"),
                files: &[SkillInstallFile {
                    relative_path: "scripts/run.py",
                    contents: b"print('nope')\n",
                }],
                source: SkillInstallSource::User,
                source_url: None,
            },
        )
        .await
        .unwrap_err();
        assert_eq!(error.kind(), SkillManagementErrorKind::InvalidSkill);

        match inner
            .read_file_bounded(
                &VirtualPath::new("/projects/skills/partial-helper/SKILL.md").unwrap(),
                1024,
            )
            .await
        {
            Ok(None) | Err(FilesystemError::NotFound { .. }) => {}
            Ok(Some(_)) => panic!(
                "SKILL.md should be written last so failed bundle writes do not publish a partial skill"
            ),
            Err(error) => panic!("unexpected filesystem error: {error:?}"),
        }
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

    #[derive(Clone)]
    struct FailingBundleWriteFilesystem {
        inner: Arc<InMemoryBackend>,
    }

    #[async_trait]
    impl RootFilesystem for FailingBundleWriteFilesystem {
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

        async fn write_file(
            &self,
            path: &VirtualPath,
            bytes: &[u8],
        ) -> Result<(), FilesystemError> {
            if path.as_str().ends_with("/scripts/run.py") {
                return Err(FilesystemError::Backend {
                    operation: FilesystemOperation::WriteFile,
                    path: path.clone(),
                    reason: "injected bundle write failure".to_string(),
                });
            }
            self.inner.write_file(path, bytes).await
        }

        async fn create_dir_all(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
            self.inner.create_dir_all(path).await
        }

        async fn delete(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
            self.inner.delete(path).await
        }
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

    fn skill_management_context_with_root(
        filesystem: Arc<dyn RootFilesystem>,
        mounts: MountView,
    ) -> SkillManagementContext {
        SkillManagementContext::new(filesystem, mounts, ResourceScope::system())
    }

    fn skill_md(name: &str, description: &str, prompt: &str) -> String {
        format!("---\nname: {name}\ndescription: {description}\n---\n{prompt}\n")
    }
}
