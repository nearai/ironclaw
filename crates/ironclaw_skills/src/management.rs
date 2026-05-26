use std::{
    collections::HashMap,
    sync::{Arc, OnceLock},
};

use crate::{MAX_PROMPT_FILE_SIZE, normalize_line_endings, parse_skill_md, validate_skill_name};
use ironclaw_filesystem::{
    DirEntry, FileType, FilesystemError, FilesystemOperation, RootFilesystem, ScopedFilesystem,
};
use ironclaw_host_api::{HostApiError, MountView, ResourceScope, ScopedPath};
use tokio::sync::{Mutex, OwnedMutexGuard};

const USER_SKILLS_ROOT: &str = "/skills";
const SYSTEM_SKILLS_ROOT: &str = "/system/skills";
const SKILL_FILE_NAME: &str = "SKILL.md";
const SKILL_SEARCH_ENTRY_SCAN_LIMIT: usize = 250;
static SKILL_WRITE_LOCKS: OnceLock<Mutex<HashMap<String, Arc<Mutex<()>>>>> = OnceLock::new();

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
    reason: Option<String>,
}

impl SkillManagementError {
    pub fn new(kind: SkillManagementErrorKind) -> Self {
        Self { kind, reason: None }
    }

    pub fn with_reason(kind: SkillManagementErrorKind, reason: impl Into<String>) -> Self {
        Self {
            kind,
            reason: Some(reason.into()),
        }
    }

    pub fn kind(&self) -> SkillManagementErrorKind {
        self.kind
    }

    pub fn reason(&self) -> Option<&str> {
        self.reason.as_deref()
    }
}

#[derive(Clone)]
pub struct SkillManagementContext {
    filesystem: Arc<ScopedFilesystem<dyn RootFilesystem>>,
    scope: ResourceScope,
    write_lock_namespace: String,
}

impl SkillManagementContext {
    pub fn new(
        filesystem: Arc<dyn RootFilesystem>,
        mounts: MountView,
        scope: ResourceScope,
    ) -> Self {
        let write_lock_namespace = skill_write_lock_namespace(&mounts, &scope);
        Self {
            filesystem: Arc::new(ScopedFilesystem::with_fixed_view(filesystem, mounts)),
            scope,
            write_lock_namespace,
        }
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SkillSearchRequest<'a> {
    pub query: &'a str,
    pub limit: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillSearchResult {
    pub skills: Vec<SkillSummary>,
    pub truncated: bool,
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
    fields(query_bytes = request.query.len(), limit = request.limit)
)]
pub async fn search_skills(
    context: &SkillManagementContext,
    request: SkillSearchRequest<'_>,
) -> Result<SkillSearchResult, SkillManagementError> {
    let normalized_query = request.query.trim().to_lowercase();
    let mut skills = Vec::new();
    let mut remaining_entries = SKILL_SEARCH_ENTRY_SCAN_LIMIT;
    let mut truncated = collect_matching_skill_root(
        context,
        SYSTEM_SKILLS_ROOT,
        SkillSource::System,
        &normalized_query,
        request.limit,
        &mut remaining_entries,
        &mut skills,
    )
    .await?;
    if !truncated {
        truncated = collect_matching_skill_root(
            context,
            USER_SKILLS_ROOT,
            SkillSource::User,
            &normalized_query,
            request.limit,
            &mut remaining_entries,
            &mut skills,
        )
        .await?;
    }
    tracing::debug!(
        skill_count = skills.len(),
        truncated,
        "skill management searched skills"
    );
    Ok(SkillSearchResult { skills, truncated })
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
    let parsed = parse_skill_md(&normalized).map_err(|error| {
        tracing::debug!(%error, "skill install failed to parse SKILL.md content");
        SkillManagementError::with_reason(
            SkillManagementErrorKind::InvalidInput,
            format!("skill content failed to parse: {error}"),
        )
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

    let skill_name = parsed.manifest.name;
    let skill_dir = skill_root_scoped_path(USER_SKILLS_ROOT, &skill_name)?;
    let skill_path = skill_scoped_path(USER_SKILLS_ROOT, &skill_name, SKILL_FILE_NAME)?;
    let _guard = skill_write_lock(context, &skill_path).await;

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
        "skill install completed"
    );

    Ok(SkillInstallResult {
        name: skill_name.clone(),
        scoped_path: format!("{USER_SKILLS_ROOT}/{skill_name}/{SKILL_FILE_NAME}"),
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
    let _guard = skill_write_lock(context, &skill_path).await;
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

async fn skill_write_lock(
    context: &SkillManagementContext,
    skill_path: &ScopedPath,
) -> OwnedMutexGuard<()> {
    let key = format!("{}:{}", context.write_lock_namespace, skill_path);
    let lock = {
        let registry = SKILL_WRITE_LOCKS.get_or_init(|| Mutex::new(HashMap::new()));
        let mut locks = registry.lock().await;
        locks
            .entry(key)
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    };
    lock.lock_owned().await
}

fn skill_write_lock_namespace(mounts: &MountView, scope: &ResourceScope) -> String {
    let user_root = ScopedPath::new(USER_SKILLS_ROOT)
        .ok()
        .and_then(|path| mounts.resolve(&path).ok())
        .map(|path| path.as_str().to_string())
        .unwrap_or_else(|| USER_SKILLS_ROOT.to_string());
    format!(
        "tenant={};user={};agent={};project={};root={}",
        scope.tenant_id.as_str(),
        scope.user_id.as_str(),
        scope.agent_id.as_ref().map(|id| id.as_str()).unwrap_or(""),
        scope
            .project_id
            .as_ref()
            .map(|id| id.as_str())
            .unwrap_or(""),
        user_root,
    )
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
    let entries = list_skill_root_entries(context, scoped_root).await?;

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

async fn collect_matching_skill_root(
    context: &SkillManagementContext,
    scoped_root: &str,
    source: SkillSource,
    normalized_query: &str,
    limit: usize,
    remaining_entries: &mut usize,
    skills: &mut Vec<SkillSummary>,
) -> Result<bool, SkillManagementError> {
    if skills.len() >= limit || *remaining_entries == 0 {
        return Ok(true);
    }
    let fetch_limit = remaining_entries.saturating_add(1);
    let mut entries = list_skill_root_entries_bounded(context, scoped_root, fetch_limit).await?;
    let root_truncated = entries.len() > *remaining_entries;
    entries.truncate(*remaining_entries);

    for entry in entries {
        *remaining_entries -= 1;
        if entry.file_type != FileType::Directory {
            continue;
        }
        let name = entry.name.as_str();
        if !validate_skill_name(name) {
            continue;
        }
        if skills.len() >= limit {
            return Ok(true);
        }
        let skill_path = skill_scoped_path(scoped_root, name, SKILL_FILE_NAME)?;
        let Some(skill) = read_skill_summary(context, &skill_path, source).await? else {
            continue;
        };
        if !skill_matches_query(&skill, normalized_query) {
            continue;
        }
        skills.push(skill);
    }
    Ok(root_truncated)
}

async fn list_skill_root_entries(
    context: &SkillManagementContext,
    scoped_root: &str,
) -> Result<Vec<DirEntry>, SkillManagementError> {
    list_skill_root_entries_with(context, scoped_root, None).await
}

async fn list_skill_root_entries_bounded(
    context: &SkillManagementContext,
    scoped_root: &str,
    max_entries: usize,
) -> Result<Vec<DirEntry>, SkillManagementError> {
    list_skill_root_entries_with(context, scoped_root, Some(max_entries)).await
}

async fn list_skill_root_entries_with(
    context: &SkillManagementContext,
    scoped_root: &str,
    max_entries: Option<usize>,
) -> Result<Vec<DirEntry>, SkillManagementError> {
    let root = ScopedPath::new(scoped_root).map_err(|error| {
        SkillManagementError::with_reason(
            SkillManagementErrorKind::InvalidInput,
            format!("invalid skill root path: {error}"),
        )
    })?;
    let result = match max_entries {
        Some(max_entries) => {
            context
                .filesystem
                .list_dir_bounded(&context.scope, &root, max_entries)
                .await
        }
        None => context.filesystem.list_dir(&context.scope, &root).await,
    };
    match result {
        Ok(entries) => Ok(entries),
        Err(FilesystemError::NotFound { .. }) => {
            tracing::debug!("skill management skill root not found");
            Ok(Vec::new())
        }
        Err(FilesystemError::PermissionDenied { .. }) => {
            tracing::debug!("skill management skill root permission denied");
            Ok(Vec::new())
        }
        Err(error) if is_unmounted_scoped_root(&error) => {
            tracing::debug!("skill management skill root is not mounted");
            Ok(Vec::new())
        }
        Err(error) => Err(filesystem_error(error)),
    }
}

fn skill_matches_query(skill: &SkillSummary, normalized_query: &str) -> bool {
    normalized_query.is_empty()
        || skill.name.to_lowercase().contains(normalized_query)
        || skill.description.to_lowercase().contains(normalized_query)
}

async fn read_skill_summary(
    context: &SkillManagementContext,
    path: &ScopedPath,
    source: SkillSource,
) -> Result<Option<SkillSummary>, SkillManagementError> {
    let Some(content) = read_skill_file(context, path).await? else {
        return Ok(None);
    };
    let parsed = parse_skill_md(&content).map_err(|error| {
        tracing::debug!(
            scoped_path = %path,
            %error,
            "skill management failed to parse skill summary"
        );
        SkillManagementError::with_reason(
            SkillManagementErrorKind::InvalidSkill,
            format!("skill summary failed to parse: {error}"),
        )
    })?;
    tracing::debug!(
        scoped_path = %path,
        skill_name = %parsed.manifest.name,
        "skill management parsed skill summary"
    );
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
    ScopedPath::new(path).map_err(|error| {
        SkillManagementError::with_reason(
            SkillManagementErrorKind::InvalidInput,
            format!("invalid skill path: {error}"),
        )
    })
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
    use ironclaw_host_api::{MountAlias, MountGrant, MountPermissions, VirtualPath};

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
    async fn install_preserves_parse_error_context() {
        let filesystem = Arc::new(InMemoryBackend::default());
        let context = skill_management_context(filesystem, skill_mounts());

        let error = install_skill(
            &context,
            SkillInstallRequest {
                name: None,
                content: "---\nname: broken\ndescription: [\n---\nPROMPT",
            },
        )
        .await
        .unwrap_err();

        assert_eq!(error.kind(), SkillManagementErrorKind::InvalidInput);
        assert!(
            error
                .reason()
                .expect("parse failure should carry context")
                .contains("skill content failed to parse")
        );
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
    async fn search_skills_returns_bounded_matches_with_truncation() {
        let filesystem = Arc::new(InMemoryBackend::default());
        for index in 0..4 {
            write_file(
                filesystem.as_ref(),
                &format!("/projects/skills/local-{index}/SKILL.md"),
                skill_md(
                    &format!("local-{index}"),
                    "shared search description",
                    "PROMPT",
                ),
            )
            .await;
        }
        let context = skill_management_context(filesystem, skill_mounts());

        let result = search_skills(
            &context,
            SkillSearchRequest {
                query: "shared",
                limit: 2,
            },
        )
        .await
        .unwrap();

        assert_eq!(result.skills.len(), 2);
        assert!(result.truncated);
    }

    #[tokio::test]
    async fn search_skills_zero_limit_returns_empty_truncated_result() {
        let filesystem = Arc::new(InMemoryBackend::default());
        write_file(
            filesystem.as_ref(),
            "/projects/skills/local-helper/SKILL.md",
            skill_md("local-helper", "shared search description", "PROMPT"),
        )
        .await;
        let context = skill_management_context(filesystem, skill_mounts());

        let result = search_skills(
            &context,
            SkillSearchRequest {
                query: "",
                limit: 0,
            },
        )
        .await
        .unwrap();

        assert!(result.skills.is_empty());
        assert!(result.truncated);
    }

    #[tokio::test]
    async fn search_skills_empty_query_returns_all_matching_skills() {
        let filesystem = Arc::new(InMemoryBackend::default());
        for name in ["alpha-helper", "beta-helper"] {
            write_file(
                filesystem.as_ref(),
                &format!("/projects/skills/{name}/SKILL.md"),
                skill_md(name, "shared search description", "PROMPT"),
            )
            .await;
        }
        let context = skill_management_context(filesystem, skill_mounts());

        let result = search_skills(
            &context,
            SkillSearchRequest {
                query: "",
                limit: 10,
            },
        )
        .await
        .unwrap();

        let names = result
            .skills
            .iter()
            .map(|skill| skill.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["alpha-helper", "beta-helper"]);
        assert!(!result.truncated);
    }

    #[tokio::test]
    async fn search_skills_propagates_filesystem_error() {
        let context = skill_management_context(Arc::new(FailingListBackend), skill_mounts());

        let error = search_skills(
            &context,
            SkillSearchRequest {
                query: "",
                limit: 10,
            },
        )
        .await
        .unwrap_err();

        assert_eq!(error.kind(), SkillManagementErrorKind::InvalidSkill);
    }

    #[tokio::test]
    async fn search_skills_stops_after_entry_scan_budget() {
        let filesystem = Arc::new(InMemoryBackend::default());
        for index in 0..=SKILL_SEARCH_ENTRY_SCAN_LIMIT {
            write_file(
                filesystem.as_ref(),
                &format!("/projects/skills/local-{index:03}/SKILL.md"),
                skill_md(
                    &format!("local-{index:03}"),
                    "non matching description",
                    "PROMPT",
                ),
            )
            .await;
        }
        let context = skill_management_context(filesystem, skill_mounts());

        let result = search_skills(
            &context,
            SkillSearchRequest {
                query: "absent-query",
                limit: 50,
            },
        )
        .await
        .unwrap();

        assert!(result.skills.is_empty());
        assert!(result.truncated);
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
        filesystem: Arc<dyn RootFilesystem>,
        mounts: MountView,
    ) -> SkillManagementContext {
        SkillManagementContext::new(filesystem, mounts, ResourceScope::system())
    }

    fn skill_md(name: &str, description: &str, prompt: &str) -> String {
        format!("---\nname: {name}\ndescription: {description}\n---\n{prompt}\n")
    }

    struct FailingListBackend;

    #[async_trait::async_trait]
    impl RootFilesystem for FailingListBackend {
        async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
            Err(FilesystemError::Backend {
                path: path.clone(),
                operation: FilesystemOperation::ListDir,
                reason: "list failed".to_string(),
            })
        }

        async fn list_dir_bounded(
            &self,
            path: &VirtualPath,
            _max_entries: usize,
        ) -> Result<Vec<DirEntry>, FilesystemError> {
            Err(FilesystemError::Backend {
                path: path.clone(),
                operation: FilesystemOperation::ListDir,
                reason: "bounded list failed".to_string(),
            })
        }

        async fn stat(
            &self,
            path: &VirtualPath,
        ) -> Result<ironclaw_filesystem::FileStat, FilesystemError> {
            Err(FilesystemError::Backend {
                path: path.clone(),
                operation: FilesystemOperation::Stat,
                reason: "stat failed".to_string(),
            })
        }
    }
}
