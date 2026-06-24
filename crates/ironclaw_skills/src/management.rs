use std::{
    collections::HashMap,
    sync::{Arc, LazyLock, Mutex, Weak},
};

use crate::parser::starts_with_frontmatter_delimiter;
use crate::validation::normalize_skill_identifier;
use crate::{
    MAX_PROMPT_FILE_SIZE, ParsedSkill, SkillOrigin, SkillParseError, normalize_line_endings,
    parse_skill_md, validate_skill_name,
};
use async_trait::async_trait;
use ironclaw_filesystem::{
    BackendCapabilities, DirEntry, FileStat, FileType, FilesystemError, RootFilesystem,
    ScopedFilesystem,
};
use ironclaw_host_api::{HostApiError, MountView, ResourceScope, ScopedPath, VirtualPath};

mod install_bundle;
#[cfg(test)]
mod tests;

pub use install_bundle::{
    MAX_INSTALL_BUNDLE_FILE_BYTES, MAX_INSTALL_BUNDLE_FILES, MAX_INSTALL_BUNDLE_TOTAL_BYTES,
    SkillInstallFile,
};

use install_bundle::{
    existing_skill_install_matches, install_metadata_source, installed_skill_source,
    publish_skill_install, read_install_metadata_bytes, validate_install_bundle_files,
};

pub(super) const USER_SKILLS_ROOT: &str = "/skills";
const SYSTEM_SKILLS_ROOT: &str = "/system/skills";
pub(super) const SKILL_FILE_NAME: &str = "SKILL.md";
const SKILL_SEARCH_ENTRY_SCAN_LIMIT: usize = 250;
type SkillMutationLock = Arc<tokio::sync::Mutex<()>>;

static SKILL_MUTATION_LOCKS: LazyLock<Mutex<HashMap<String, Weak<tokio::sync::Mutex<()>>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

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

    async fn list_dir_bounded(
        &self,
        path: &VirtualPath,
        max_entries: usize,
    ) -> Result<Vec<DirEntry>, FilesystemError> {
        self.inner.list_dir_bounded(path, max_entries).await
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
    /// Whether the skill participates in automatic activation (mirrors
    /// `SkillManifest::auto_activate`). `false` means explicit-mention only.
    pub auto_activate: bool,
    /// Who authored the skill (mirrors `SkillManifest::origin`). Lets callers
    /// tell machine-learned skills apart — e.g. to scope the global
    /// auto-activate-learned switch or badge a skill's origin in the UI.
    pub origin: SkillOrigin,
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

pub fn skill_summary_json(skill: &SkillSummary) -> serde_json::Value {
    serde_json::json!({
        "name": skill.name,
        "version": skill.version,
        "description": skill.description,
        "source": skill.source.as_str(),
        "keywords": skill.keywords,
        "tags": skill.tags,
        "requires_skills": skill.requires_skills,
        "auto_activate": skill.auto_activate,
        "origin": skill.origin.as_str(),
    })
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillInstallResult {
    pub name: String,
    pub scoped_path: String,
    pub source: SkillSource,
}

struct PreparedSkillInstall {
    content: String,
    parsed: ParsedSkill,
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
pub struct SkillContentRequest<'a> {
    pub name: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillContentResult {
    pub name: String,
    pub content: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SkillUpdateRequest<'a> {
    pub name: &'a str,
    pub content: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillUpdateResult {
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

    let prepared = prepare_install_content(request.content, request.name)?;
    if prepared.content.len() as u64 > MAX_PROMPT_FILE_SIZE {
        tracing::debug!(
            max_bytes = MAX_PROMPT_FILE_SIZE,
            "skill install rejected oversized persisted content"
        );
        return Err(SkillManagementError::new(
            SkillManagementErrorKind::Resource,
        ));
    }
    validate_install_bundle_files(request.files)?;

    let skill_name = prepared.parsed.manifest.name;
    let mutation_lock = skill_mutation_lock(&skill_name);
    let _mutation_guard = mutation_lock.lock().await;
    let skill_dir = skill_root_scoped_path(USER_SKILLS_ROOT, &skill_name)?;
    let skill_path = skill_scoped_path(USER_SKILLS_ROOT, &skill_name, SKILL_FILE_NAME)?;

    log_skill_filesystem_phase("stat_existing_dir", &skill_name, &skill_dir);
    if stat_optional(context, &skill_dir).await?.is_some() {
        if existing_skill_install_matches(
            context,
            &skill_name,
            &prepared.content,
            request.files,
            request.source,
            request.source_url,
        )
        .await?
        {
            tracing::debug!(
                skill_name = %skill_name,
                scoped_path = %skill_dir,
                "skill install matched existing skill directory"
            );
            return Ok(SkillInstallResult {
                name: skill_name.clone(),
                scoped_path: format!("{USER_SKILLS_ROOT}/{skill_name}/{SKILL_FILE_NAME}"),
                source: installed_skill_source(request.source),
            });
        }
        tracing::debug!(
            skill_name = %skill_name,
            scoped_path = %skill_dir,
            "skill install rejected existing skill directory"
        );
        return Err(SkillManagementError::new(
            SkillManagementErrorKind::Conflict,
        ));
    }

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

    publish_skill_install(
        context,
        &skill_name,
        &prepared.content,
        request.files,
        request.source,
        request.source_url,
    )
    .await?;
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

fn prepare_install_content(
    content: &str,
    requested_name: Option<&str>,
) -> Result<PreparedSkillInstall, SkillManagementError> {
    let normalized_content = normalize_line_endings(content);
    match parse_skill_md(&normalized_content) {
        Ok(parsed) => {
            validate_requested_name_matches_manifest(requested_name, &parsed.manifest.name)?;
            Ok(PreparedSkillInstall {
                content: normalized_content,
                parsed,
            })
        }
        Err(SkillParseError::MissingFrontmatter)
            if !starts_with_frontmatter_delimiter(&normalized_content) =>
        {
            let content = synthesize_install_frontmatter(&normalized_content, requested_name)?;
            let parsed = parse_skill_md(&content).map_err(|error| {
                tracing::debug!(%error, "skill install failed to parse synthesized SKILL.md content");
                skill_parse_error(error)
            })?;
            Ok(PreparedSkillInstall { content, parsed })
        }
        Err(error) => {
            tracing::debug!(%error, "skill install failed to parse SKILL.md content");
            Err(skill_parse_error(error))
        }
    }
}

fn validate_requested_name_matches_manifest(
    requested_name: Option<&str>,
    manifest_name: &str,
) -> Result<(), SkillManagementError> {
    if let Some(requested_name) = requested_name
        && normalize_skill_identifier(requested_name).as_deref() != Some(manifest_name)
    {
        tracing::debug!(
            requested_name,
            parsed_name = %manifest_name,
            "skill install rejected name mismatch"
        );
        return Err(SkillManagementError::new(
            SkillManagementErrorKind::InvalidInput,
        ));
    }
    Ok(())
}

fn synthesize_install_frontmatter(
    normalized_content: &str,
    requested_name: Option<&str>,
) -> Result<String, SkillManagementError> {
    let Some(requested_name) = requested_name else {
        let error = SkillParseError::MissingFrontmatter;
        tracing::debug!(%error, "skill install failed to parse SKILL.md content");
        return Err(skill_parse_error(error));
    };
    let Some(skill_name) = normalize_skill_identifier(requested_name) else {
        tracing::debug!(
            requested_name,
            "skill install rejected invalid requested name"
        );
        return Err(SkillManagementError::new(
            SkillManagementErrorKind::InvalidInput,
        ));
    };

    let mut rendered = format!("---\nname: {skill_name}\n---\n\n");
    rendered.push_str(normalized_content);
    Ok(rendered)
}

fn skill_parse_error(error: SkillParseError) -> SkillManagementError {
    SkillManagementError::with_reason(
        SkillManagementErrorKind::InvalidInput,
        format!("skill content failed to parse: {error}"),
    )
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
    let mutation_lock = skill_mutation_lock(request.name);
    let _mutation_guard = mutation_lock.lock().await;
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

pub async fn read_skill_content(
    context: &SkillManagementContext,
    request: SkillContentRequest<'_>,
) -> Result<SkillContentResult, SkillManagementError> {
    if !validate_skill_name(request.name) {
        return Err(SkillManagementError::new(
            SkillManagementErrorKind::InvalidInput,
        ));
    }
    let skill_path = skill_scoped_path(USER_SKILLS_ROOT, request.name, SKILL_FILE_NAME)?;
    let content = read_skill_file(context, &skill_path)
        .await?
        .ok_or_else(|| SkillManagementError::new(SkillManagementErrorKind::NotFound))?;
    Ok(SkillContentResult {
        name: request.name.to_string(),
        content,
    })
}

pub async fn update_skill(
    context: &SkillManagementContext,
    request: SkillUpdateRequest<'_>,
) -> Result<SkillUpdateResult, SkillManagementError> {
    if !validate_skill_name(request.name) {
        return Err(SkillManagementError::new(
            SkillManagementErrorKind::InvalidInput,
        ));
    }
    if request.content.len() as u64 > MAX_PROMPT_FILE_SIZE {
        return Err(SkillManagementError::new(
            SkillManagementErrorKind::Resource,
        ));
    }

    let normalized = normalize_line_endings(request.content);
    let parsed = parse_skill_md(&normalized).map_err(skill_parse_error)?;
    if parsed.manifest.name != request.name {
        return Err(SkillManagementError::with_reason(
            SkillManagementErrorKind::InvalidInput,
            format!("edited skill name must remain '{}'", request.name),
        ));
    }

    let mutation_lock = skill_mutation_lock(request.name);
    let _mutation_guard = mutation_lock.lock().await;
    let skill_path = skill_scoped_path(USER_SKILLS_ROOT, request.name, SKILL_FILE_NAME)?;
    if stat_optional(context, &skill_path).await?.is_none() {
        return Err(SkillManagementError::new(
            SkillManagementErrorKind::NotFound,
        ));
    }
    context
        .filesystem
        .write_file(&context.scope, &skill_path, normalized.as_bytes())
        .await
        .map_err(filesystem_error)?;
    Ok(SkillUpdateResult {
        name: request.name.to_string(),
    })
}

/// Write the machine-learned provenance sidecar (`.ironclaw-learned.json`) next
/// to a skill's `SKILL.md`. Called ONLY by the self-evolution learning sink (via
/// the composition `PortSkillWriter`), never by the human-edit facade — so a
/// human content edit never refreshes the baseline and therefore naturally
/// diverges from it. See `docs/plans/2026-06-19-skill-edit-preservation.md`.
pub async fn write_learned_provenance(
    context: &SkillManagementContext,
    skill_name: &str,
    provenance: &crate::LearnedSkillProvenance,
) -> Result<(), SkillManagementError> {
    let path = skill_scoped_path(
        USER_SKILLS_ROOT,
        skill_name,
        crate::LEARNED_PROVENANCE_FILE_NAME,
    )?;
    let bytes = provenance.to_pretty_json().map_err(|error| {
        SkillManagementError::with_reason(SkillManagementErrorKind::Resource, error.to_string())
    })?;
    context
        .filesystem
        .write_file(&context.scope, &path, &bytes)
        .await
        .map_err(filesystem_error)?;
    Ok(())
}

/// Read the machine-learned provenance sidecar for a skill, if present. `None`
/// means the skill was not authored by the learning sink (a human-built,
/// registry-installed, or never-learned skill) — which the gate treats as "not
/// auto-evolvable". A malformed sidecar also reads as `None` (fail safe).
pub async fn read_learned_provenance(
    context: &SkillManagementContext,
    skill_name: &str,
) -> Result<Option<crate::LearnedSkillProvenance>, SkillManagementError> {
    let path = skill_scoped_path(
        USER_SKILLS_ROOT,
        skill_name,
        crate::LEARNED_PROVENANCE_FILE_NAME,
    )?;
    if stat_optional(context, &path).await?.is_none() {
        return Ok(None);
    }
    let Some(bytes) = context
        .filesystem
        .read_bytes_bounded(&context.scope, &path, crate::MAX_LEARNED_PROVENANCE_BYTES)
        .await
        .map_err(filesystem_error)?
    else {
        return Ok(None);
    };
    Ok(crate::LearnedSkillProvenance::from_sidecar_bytes(&bytes))
}

/// Bounded probe count for the bundle check — far above any realistic skill
/// directory (`SKILL.md` + a few dotfile sidecars + a handful of bundle files).
const MAX_BUNDLE_PROBE_ENTRIES: usize = 64;

/// True iff the skill directory holds anything beyond `SKILL.md` and `.`-prefixed
/// dotfile sidecars — i.e. it is a multi-file bundle (a human-built
/// `references/`/`scripts/`/`assets/` skill, or files a human added to a learned
/// one). The self-evolution gate must not auto-overwrite a bundle's `SKILL.md`,
/// since the distillation cannot see the sibling files it may reference. A
/// missing directory reads as `false`.
pub async fn skill_is_bundle(
    context: &SkillManagementContext,
    skill_name: &str,
) -> Result<bool, SkillManagementError> {
    let skill_dir = skill_root_scoped_path(USER_SKILLS_ROOT, skill_name)?;
    let entries = match context
        .filesystem
        .list_dir_bounded(&context.scope, &skill_dir, MAX_BUNDLE_PROBE_ENTRIES)
        .await
    {
        Ok(entries) => entries,
        Err(FilesystemError::NotFound { .. }) => return Ok(false),
        Err(error) => return Err(filesystem_error(error)),
    };
    // Fail closed: a directory at the bounded-probe cap was truncated, so a
    // bundle sibling could exist beyond the probe window. Treat a capped probe
    // as a bundle so self-evolution never overwrites a `SKILL.md` whose sibling
    // files it could not see.
    let has_bundle_sibling = entries
        .iter()
        .any(|entry| entry.name != SKILL_FILE_NAME && !entry.name.starts_with('.'));
    Ok(has_bundle_sibling || entries.len() >= MAX_BUNDLE_PROBE_ENTRIES)
}

fn skill_mutation_lock(skill_name: &str) -> SkillMutationLock {
    let mut guard = SKILL_MUTATION_LOCKS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    guard.retain(|_, weak| weak.strong_count() > 0);
    if let Some(existing) = guard.get(skill_name).and_then(Weak::upgrade) {
        return existing;
    }

    let lock = Arc::new(tokio::sync::Mutex::new(()));
    guard.insert(skill_name.to_string(), Arc::downgrade(&lock));
    lock
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
    Ok(match result {
        Ok(entries) => entries,
        Err(FilesystemError::NotFound { .. }) => {
            tracing::debug!("skill management skill root not found");
            Vec::new()
        }
        Err(FilesystemError::PermissionDenied { .. }) => {
            tracing::debug!("skill management skill root permission denied");
            Vec::new()
        }
        Err(error) if is_unmounted_scoped_root(&error) => {
            tracing::debug!("skill management skill root is not mounted");
            Vec::new()
        }
        Err(error) => return Err(filesystem_error(error)),
    })
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
    let source = skill_source_with_install_metadata(context, path, source).await?;
    Ok(Some(SkillSummary {
        name: parsed.manifest.name,
        version: parsed.manifest.version,
        description: parsed.manifest.description,
        source,
        keywords: parsed.manifest.activation.keywords,
        tags: parsed.manifest.activation.tags,
        requires_skills: parsed.manifest.requires.skills,
        auto_activate: parsed.manifest.auto_activate,
        origin: parsed.manifest.origin,
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

async fn skill_source_with_install_metadata(
    context: &SkillManagementContext,
    skill_path: &ScopedPath,
    default_source: SkillSource,
) -> Result<SkillSource, SkillManagementError> {
    if default_source != SkillSource::User {
        return Ok(default_source);
    }
    let Some(bytes) = read_install_metadata_bytes(context, skill_path).await? else {
        return Ok(default_source);
    };
    Ok(install_metadata_source(default_source, &bytes))
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
