use ironclaw_filesystem::{FileType, FilesystemError, FilesystemOperation, RootFilesystem};
use ironclaw_host_api::{MountPermissions, MountView, ScopedPath, VirtualPath};
use ironclaw_skills::{
    MAX_PROMPT_FILE_SIZE, normalize_line_endings, parse_skill_md, validate_skill_name,
};

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

#[derive(Clone, Copy)]
pub struct SkillManagementContext<'a> {
    filesystem: &'a dyn RootFilesystem,
    mounts: &'a MountView,
}

impl<'a> SkillManagementContext<'a> {
    pub fn new(filesystem: &'a dyn RootFilesystem, mounts: &'a MountView) -> Self {
        Self { filesystem, mounts }
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
    context: SkillManagementContext<'_>,
) -> Result<Vec<SkillSummary>, SkillManagementError> {
    let mut skills = Vec::new();
    skills.extend(list_skill_root(&context, SYSTEM_SKILLS_ROOT, SkillSource::System).await?);
    skills.extend(list_skill_root(&context, USER_SKILLS_ROOT, SkillSource::User).await?);
    Ok(skills)
}

pub async fn install_skill(
    context: SkillManagementContext<'_>,
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
    let skill_dir = resolve_required_scoped_child(
        &context,
        USER_SKILLS_ROOT,
        &skill_name,
        FilesystemOperation::CreateDirAll,
    )?;
    let skill_path = resolve_required_scoped_child(
        &context,
        USER_SKILLS_ROOT,
        &format!("{skill_name}/{SKILL_FILE_NAME}"),
        FilesystemOperation::WriteFile,
    )?;

    if stat_optional(&context, &skill_path).await?.is_some() {
        return Err(SkillManagementError::new(
            SkillManagementErrorKind::Conflict,
        ));
    }

    context
        .filesystem
        .create_dir_all(&skill_dir)
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
        .write_file(&skill_path, normalized.as_bytes())
        .await
        .map_err(filesystem_error)?;

    Ok(SkillInstallResult {
        name: skill_name.clone(),
        scoped_path: format!("{USER_SKILLS_ROOT}/{skill_name}/{SKILL_FILE_NAME}"),
    })
}

pub async fn remove_skill(
    context: SkillManagementContext<'_>,
    request: SkillRemoveRequest<'_>,
) -> Result<SkillRemoveResult, SkillManagementError> {
    if !validate_skill_name(request.name) {
        return Err(SkillManagementError::new(
            SkillManagementErrorKind::InvalidInput,
        ));
    }
    let skill_dir = resolve_required_scoped_child(
        &context,
        USER_SKILLS_ROOT,
        request.name,
        FilesystemOperation::Delete,
    )?;
    let skill_path = resolve_required_scoped_child(
        &context,
        USER_SKILLS_ROOT,
        &format!("{}/{SKILL_FILE_NAME}", request.name),
        FilesystemOperation::ReadFile,
    )?;
    if stat_optional(&context, &skill_path).await?.is_none() {
        return Err(SkillManagementError::new(
            SkillManagementErrorKind::NotFound,
        ));
    }
    context
        .filesystem
        .delete(&skill_dir)
        .await
        .map_err(filesystem_error)?;
    Ok(SkillRemoveResult {
        name: request.name.to_string(),
    })
}

async fn list_skill_root(
    context: &SkillManagementContext<'_>,
    scoped_root: &str,
    source: SkillSource,
) -> Result<Vec<SkillSummary>, SkillManagementError> {
    let Some(root) =
        resolve_optional_scoped_path(context, scoped_root, FilesystemOperation::ListDir)?
    else {
        return Ok(Vec::new());
    };
    let entries = match context.filesystem.list_dir(&root).await {
        Ok(entries) => entries,
        Err(FilesystemError::NotFound { .. }) => return Ok(Vec::new()),
        Err(error) => return Err(filesystem_error(error)),
    };

    let mut skills = Vec::new();
    for entry in entries {
        if entry.file_type != FileType::Directory {
            continue;
        }
        let Some(name) = entry.path.as_str().rsplit('/').next() else {
            continue;
        };
        if !validate_skill_name(name) {
            continue;
        }
        let Some(skill_path) = child_virtual_path(&entry.path, SKILL_FILE_NAME)? else {
            continue;
        };
        if let Some(skill) = read_skill_summary(context, &skill_path, source).await? {
            skills.push(skill);
        }
    }
    Ok(skills)
}

async fn read_skill_summary(
    context: &SkillManagementContext<'_>,
    path: &VirtualPath,
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

fn resolve_required_scoped_child(
    context: &SkillManagementContext<'_>,
    root: &str,
    relative: &str,
    operation: FilesystemOperation,
) -> Result<VirtualPath, SkillManagementError> {
    validate_relative_skill_path(relative)?;
    let scoped = format!("{}/{}", root.trim_end_matches('/'), relative);
    let Some(path) = resolve_scoped_path(context, &scoped, operation)? else {
        return Err(SkillManagementError::new(
            SkillManagementErrorKind::FilesystemDenied,
        ));
    };
    Ok(path)
}

fn resolve_optional_scoped_path(
    context: &SkillManagementContext<'_>,
    scoped_path: &str,
    operation: FilesystemOperation,
) -> Result<Option<VirtualPath>, SkillManagementError> {
    resolve_scoped_path(context, scoped_path, operation)
}

fn resolve_scoped_path(
    context: &SkillManagementContext<'_>,
    scoped_path: &str,
    operation: FilesystemOperation,
) -> Result<Option<VirtualPath>, SkillManagementError> {
    let scoped = ScopedPath::new(scoped_path)
        .map_err(|_| SkillManagementError::new(SkillManagementErrorKind::InvalidInput))?;
    let (virtual_path, grant) = match context.mounts.resolve_with_grant(&scoped) {
        Ok(resolved) => resolved,
        Err(_) => return Ok(None),
    };
    if !operation_allowed(&grant.permissions, operation) {
        return Err(SkillManagementError::new(
            SkillManagementErrorKind::FilesystemDenied,
        ));
    }
    Ok(Some(virtual_path))
}

fn operation_allowed(permissions: &MountPermissions, operation: FilesystemOperation) -> bool {
    match operation {
        FilesystemOperation::ReadFile => permissions.read,
        FilesystemOperation::WriteFile | FilesystemOperation::AppendFile => permissions.write,
        FilesystemOperation::ListDir => permissions.list,
        FilesystemOperation::Stat => permissions.read || permissions.list,
        FilesystemOperation::Delete => permissions.delete,
        FilesystemOperation::CreateDirAll => permissions.write,
        _ => false,
    }
}

fn validate_relative_skill_path(path: &str) -> Result<(), SkillManagementError> {
    if path.is_empty()
        || path.starts_with('/')
        || path
            .split('/')
            .any(|segment| segment.is_empty() || segment == "." || segment == "..")
    {
        return Err(SkillManagementError::new(
            SkillManagementErrorKind::InvalidInput,
        ));
    }
    Ok(())
}

fn child_virtual_path(
    parent: &VirtualPath,
    child: &str,
) -> Result<Option<VirtualPath>, SkillManagementError> {
    if child.is_empty() || child.contains('/') {
        return Ok(None);
    }
    VirtualPath::new(format!(
        "{}/{}",
        parent.as_str().trim_end_matches('/'),
        child
    ))
    .map(Some)
    .map_err(|_| SkillManagementError::new(SkillManagementErrorKind::FilesystemDenied))
}

async fn stat_optional(
    context: &SkillManagementContext<'_>,
    path: &VirtualPath,
) -> Result<Option<ironclaw_filesystem::FileStat>, SkillManagementError> {
    match context.filesystem.stat(path).await {
        Ok(stat) => Ok(Some(stat)),
        Err(FilesystemError::NotFound { .. }) => Ok(None),
        Err(error) => Err(filesystem_error(error)),
    }
}

async fn read_skill_file(
    context: &SkillManagementContext<'_>,
    path: &VirtualPath,
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
        .read_file_bounded(path, MAX_PROMPT_FILE_SIZE as usize)
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
        let filesystem = InMemoryBackend::default();
        write_file(
            &filesystem,
            "/projects/system/skills/system-helper/SKILL.md",
            skill_md(
                "system-helper",
                "system skill description",
                "SYSTEM_SKILL_PROMPT",
            ),
        )
        .await;
        let mounts = skill_mounts();
        let context = SkillManagementContext::new(&filesystem, &mounts);

        let installed = install_skill(
            context,
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

        let listed = list_skills(context).await.unwrap();
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
            context,
            SkillRemoveRequest {
                name: "local-helper",
            },
        )
        .await
        .unwrap();
        assert_eq!(removed.name, "local-helper");
        assert_eq!(list_skills(context).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn install_rejects_name_mismatch() {
        let filesystem = InMemoryBackend::default();
        let mounts = skill_mounts();
        let context = SkillManagementContext::new(&filesystem, &mounts);

        let error = install_skill(
            context,
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
    async fn remove_rejects_system_skill() {
        let filesystem = InMemoryBackend::default();
        write_file(
            &filesystem,
            "/projects/system/skills/system-helper/SKILL.md",
            skill_md("system-helper", "system skill description", "PROMPT"),
        )
        .await;
        let mounts = skill_mounts();
        let context = SkillManagementContext::new(&filesystem, &mounts);

        let error = remove_skill(
            context,
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

    fn skill_md(name: &str, description: &str, prompt: &str) -> String {
        format!("---\nname: {name}\ndescription: {description}\n---\n{prompt}\n")
    }
}
