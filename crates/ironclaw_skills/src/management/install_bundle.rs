use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet},
};

use ironclaw_filesystem::{FileType, FilesystemError, FilesystemOperation};
use ironclaw_host_api::{path::ScopedPath, registry_package::RegistryPackageProvenance};

use crate::{
    INSTALL_METADATA_FILE_NAME, InstalledSkillMetadata, MAX_INSTALL_METADATA_BYTES,
    MAX_PROMPT_FILE_SIZE, normalize_safe_relative_path, validate_skill_name,
};

use super::{
    SKILL_FILE_NAME, SkillInstallSource, SkillManagementContext, SkillManagementError,
    SkillManagementErrorKind, SkillSource, USER_SKILLS_ROOT, filesystem_error,
    log_skill_filesystem_phase, scoped_sibling, skill_mutation_lock, skill_root_scoped_path,
    skill_scoped_path, stat_optional,
};

pub const MAX_INSTALL_BUNDLE_FILES: usize = 256;
pub const MAX_INSTALL_BUNDLE_FILE_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_INSTALL_BUNDLE_TOTAL_BYTES: usize = 20 * 1024 * 1024;
const MAX_SKILL_SNAPSHOT_FILES: usize = MAX_INSTALL_BUNDLE_FILES + 2;
const MAX_SKILL_SNAPSHOT_ENTRIES: usize = MAX_SKILL_SNAPSHOT_FILES * 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ExistingRegistryInstallMatch {
    Match,
    AdoptReceipt,
    Conflict,
}
const MAX_SKILL_SNAPSHOT_TOTAL_BYTES: usize =
    MAX_INSTALL_BUNDLE_TOTAL_BYTES + MAX_PROMPT_FILE_SIZE as usize + MAX_INSTALL_METADATA_BYTES;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SkillInstallFile<'a> {
    pub relative_path: &'a str,
    pub contents: &'a [u8],
}

pub(crate) struct SkillBundleSnapshot {
    files: Vec<OwnedSkillBundleFile>,
    source: SkillSource,
}

struct OwnedSkillBundleFile {
    relative_path: String,
    contents: Vec<u8>,
}

pub(crate) async fn capture_skill_bundle(
    context: &SkillManagementContext,
    skill_name: &str,
) -> Result<SkillBundleSnapshot, SkillManagementError> {
    if !validate_skill_name(skill_name) {
        return Err(SkillManagementError::new(
            SkillManagementErrorKind::InvalidInput,
        ));
    }
    let mutation_lock = skill_mutation_lock(skill_name);
    let _mutation_guard = mutation_lock.lock().await;
    capture_skill_bundle_locked(context, skill_name).await
}

pub(super) async fn capture_skill_bundle_locked(
    context: &SkillManagementContext,
    skill_name: &str,
) -> Result<SkillBundleSnapshot, SkillManagementError> {
    let skill_dir = skill_root_scoped_path(USER_SKILLS_ROOT, skill_name)?;
    let mut files = Vec::new();
    let mut stack = vec![(skill_dir, String::new())];
    let mut entry_count = 0usize;
    let mut total_bytes = 0usize;

    while let Some((dir_path, relative_prefix)) = stack.pop() {
        let remaining_entries = MAX_SKILL_SNAPSHOT_ENTRIES
            .checked_sub(entry_count)
            .ok_or_else(|| SkillManagementError::new(SkillManagementErrorKind::Resource))?;
        if remaining_entries == 0 {
            return Err(SkillManagementError::new(
                SkillManagementErrorKind::Resource,
            ));
        }
        let entries = context
            .filesystem
            .list_dir_bounded(
                &context.scope,
                &dir_path,
                remaining_entries.saturating_add(1),
            )
            .await
            .map_err(filesystem_error)?;
        if entries.len() > remaining_entries {
            return Err(SkillManagementError::new(
                SkillManagementErrorKind::Resource,
            ));
        }
        entry_count = entry_count
            .checked_add(entries.len())
            .ok_or_else(|| SkillManagementError::new(SkillManagementErrorKind::Resource))?;

        for entry in entries {
            let relative_path = if relative_prefix.is_empty() {
                entry.name.clone()
            } else {
                format!("{relative_prefix}{}", entry.name)
            };
            let relative_path = normalize_snapshot_relative_path(&relative_path)?;
            let entry_path = scoped_child(&dir_path, &entry.name)?;
            match entry.file_type {
                FileType::File => {
                    if files.len() >= MAX_SKILL_SNAPSHOT_FILES {
                        return Err(SkillManagementError::new(
                            SkillManagementErrorKind::Resource,
                        ));
                    }
                    let contents = context
                        .filesystem
                        .read_bytes_bounded(
                            &context.scope,
                            &entry_path,
                            MAX_INSTALL_BUNDLE_FILE_BYTES,
                        )
                        .await
                        .map_err(filesystem_error)?
                        .ok_or_else(|| {
                            SkillManagementError::new(SkillManagementErrorKind::Resource)
                        })?;
                    total_bytes = total_bytes.saturating_add(contents.len());
                    if total_bytes > MAX_SKILL_SNAPSHOT_TOTAL_BYTES {
                        return Err(SkillManagementError::new(
                            SkillManagementErrorKind::Resource,
                        ));
                    }
                    files.push(OwnedSkillBundleFile {
                        relative_path,
                        contents,
                    });
                }
                FileType::Directory => {
                    stack.push((entry_path, format!("{relative_path}/")));
                }
                FileType::Symlink | FileType::Other => {
                    return Err(SkillManagementError::new(
                        SkillManagementErrorKind::InvalidSkill,
                    ));
                }
            }
        }
    }

    if !files
        .iter()
        .any(|file| file.relative_path == SKILL_FILE_NAME)
    {
        return Err(SkillManagementError::new(
            SkillManagementErrorKind::NotFound,
        ));
    }
    files.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    let source = files
        .iter()
        .find(|file| file.relative_path == INSTALL_METADATA_FILE_NAME)
        .map(|file| install_metadata_source(SkillSource::User, &file.contents))
        .unwrap_or(SkillSource::User);
    Ok(SkillBundleSnapshot { files, source })
}

pub(crate) async fn restore_skill_bundle(
    context: &SkillManagementContext,
    skill_name: &str,
    snapshot: SkillBundleSnapshot,
) -> Result<SkillSource, SkillManagementError> {
    if !validate_skill_name(skill_name) {
        return Err(SkillManagementError::new(
            SkillManagementErrorKind::InvalidInput,
        ));
    }
    let mutation_lock = skill_mutation_lock(skill_name);
    let _mutation_guard = mutation_lock.lock().await;
    restore_skill_bundle_locked(context, skill_name, snapshot).await
}

pub(super) async fn restore_skill_bundle_locked(
    context: &SkillManagementContext,
    skill_name: &str,
    snapshot: SkillBundleSnapshot,
) -> Result<SkillSource, SkillManagementError> {
    let skill_dir = skill_root_scoped_path(USER_SKILLS_ROOT, skill_name)?;
    if stat_optional(context, &skill_dir).await?.is_some() {
        return Err(SkillManagementError::new(
            SkillManagementErrorKind::Conflict,
        ));
    }

    let result: Result<(), SkillManagementError> = async {
        create_dir_all(context, skill_name, "restore_create_dir_all", &skill_dir).await?;
        for file in &snapshot.files {
            let relative_path = normalize_snapshot_relative_path(&file.relative_path)?;
            let file_path = skill_bundle_file_scoped_path(skill_name, &relative_path)?;
            if let Some(parent) = scoped_parent(&file_path)? {
                create_dir_all(context, skill_name, "restore_create_parent", &parent).await?;
            }
            log_skill_filesystem_phase("restore_write_file", skill_name, &file_path);
            context
                .filesystem
                .write_file(&context.scope, &file_path, &file.contents)
                .await
                .map_err(filesystem_error)?;
        }
        Ok(())
    }
    .await;

    if let Err(error) = result {
        if let Err(cleanup_error) = cleanup_partial_install(context, skill_name, &skill_dir).await {
            let original_kind = error.kind();
            return Err(SkillManagementError::with_reason(
                original_kind,
                format!(
                    "skill bundle restore failed with {error:?}; partial cleanup also failed with {cleanup_error:?}"
                ),
            ));
        }
        return Err(error);
    }
    Ok(snapshot.source)
}

pub(super) async fn publish_skill_install(
    context: &SkillManagementContext,
    skill_name: &str,
    normalized_content: &str,
    files: &[SkillInstallFile<'_>],
    source: SkillInstallSource,
    source_url: Option<&str>,
    registry_provenance: Option<&RegistryPackageProvenance>,
) -> Result<(), SkillManagementError> {
    let skill_dir = skill_root_scoped_path(USER_SKILLS_ROOT, skill_name)?;
    let skill_path = skill_scoped_path(USER_SKILLS_ROOT, skill_name, SKILL_FILE_NAME)?;

    let result = async {
        create_dir_all(context, skill_name, "create_dir_all", &skill_dir).await?;
        for file in files {
            let relative_path = normalize_install_relative_path(file.relative_path)?;
            let file_path = skill_bundle_file_scoped_path(skill_name, &relative_path)?;
            if let Some(parent) = scoped_parent(&file_path)? {
                create_dir_all(context, skill_name, "create_bundle_parent", &parent).await?;
            }
            log_skill_filesystem_phase("write_bundle_file", skill_name, &file_path);
            context
                .filesystem
                .write_file(&context.scope, &file_path, file.contents)
                .await
                .map_err(|error| {
                    log_skill_filesystem_phase("write_bundle_file_failed", skill_name, &file_path);
                    filesystem_error(error)
                })?;
        }
        if source == SkillInstallSource::InstalledUrl {
            let metadata_path =
                skill_bundle_file_scoped_path(skill_name, INSTALL_METADATA_FILE_NAME)?;
            let metadata = install_metadata_bytes(source_url, registry_provenance)?;
            log_skill_filesystem_phase("write_install_metadata", skill_name, &metadata_path);
            context
                .filesystem
                .write_file(&context.scope, &metadata_path, &metadata)
                .await
                .map_err(|error| {
                    log_skill_filesystem_phase(
                        "write_install_metadata_failed",
                        skill_name,
                        &metadata_path,
                    );
                    filesystem_error(error)
                })?;
        }
        log_skill_filesystem_phase("write_file", skill_name, &skill_path);
        context
            .filesystem
            .write_file(&context.scope, &skill_path, normalized_content.as_bytes())
            .await
            .map_err(|error| {
                log_skill_filesystem_phase("write_file_failed", skill_name, &skill_path);
                filesystem_error(error)
            })?;
        Ok(())
    }
    .await;

    if let Err(error) = result {
        cleanup_partial_install(context, skill_name, &skill_dir).await?;
        return Err(error);
    }
    Ok(())
}

pub(super) async fn existing_skill_install_matches(
    context: &SkillManagementContext,
    skill_name: &str,
    normalized_content: &str,
    files: &[SkillInstallFile<'_>],
    source: SkillInstallSource,
    source_url: Option<&str>,
    registry_provenance: Option<&RegistryPackageProvenance>,
) -> Result<ExistingRegistryInstallMatch, SkillManagementError> {
    let skill_dir = skill_root_scoped_path(USER_SKILLS_ROOT, skill_name)?;
    let expected_files = expected_install_files(
        normalized_content,
        files,
        source,
        source_url,
        registry_provenance,
    )?;
    existing_files_match_expected(context, &skill_dir, expected_files).await
}

fn expected_install_files<'a>(
    normalized_content: &'a str,
    files: &'a [SkillInstallFile<'a>],
    source: SkillInstallSource,
    source_url: Option<&str>,
    registry_provenance: Option<&RegistryPackageProvenance>,
) -> Result<BTreeMap<String, Cow<'a, [u8]>>, SkillManagementError> {
    let mut expected = BTreeMap::from([(
        SKILL_FILE_NAME.to_string(),
        Cow::Borrowed(normalized_content.as_bytes()),
    )]);
    if source == SkillInstallSource::InstalledUrl {
        expected.insert(
            INSTALL_METADATA_FILE_NAME.to_string(),
            Cow::Owned(install_metadata_bytes(source_url, registry_provenance)?),
        );
    }
    for file in files {
        let relative_path = normalize_install_relative_path(file.relative_path)?;
        expected.insert(relative_path, Cow::Borrowed(file.contents));
    }
    Ok(expected)
}

async fn existing_files_match_expected(
    context: &SkillManagementContext,
    skill_dir: &ScopedPath,
    mut expected_files: BTreeMap<String, Cow<'_, [u8]>>,
) -> Result<ExistingRegistryInstallMatch, SkillManagementError> {
    let mut adopt_receipt = false;
    let expected_dirs = expected_directory_prefixes(expected_files.keys());
    let expected_entry_count = expected_files.len().saturating_add(expected_dirs.len());
    let mut stack = vec![(skill_dir.clone(), String::new())];
    while let Some((dir_path, relative_prefix)) = stack.pop() {
        let entries = context
            .filesystem
            .list_dir_bounded(
                &context.scope,
                &dir_path,
                expected_entry_count.saturating_add(1),
            )
            .await
            .map_err(filesystem_error)?;
        if entries.len() > expected_entry_count {
            return Ok(ExistingRegistryInstallMatch::Conflict);
        }
        for entry in entries {
            let relative_path = if relative_prefix.is_empty() {
                entry.name.clone()
            } else {
                format!("{relative_prefix}{}", entry.name)
            };
            match entry.file_type {
                FileType::File => {
                    let Some(expected_contents) = expected_files.remove(&relative_path) else {
                        return Ok(ExistingRegistryInstallMatch::Conflict);
                    };
                    let file_path = scoped_child(&dir_path, &entry.name)?;
                    let existing_contents = if relative_path == INSTALL_METADATA_FILE_NAME {
                        context
                            .filesystem
                            .read_bytes_bounded(
                                &context.scope,
                                &file_path,
                                MAX_INSTALL_METADATA_BYTES,
                            )
                            .await
                            .map_err(filesystem_error)?
                    } else {
                        read_existing_file_bytes(context, &file_path, expected_contents.len())
                            .await?
                    };
                    let Some(existing_contents) = existing_contents else {
                        return Ok(ExistingRegistryInstallMatch::Conflict);
                    };
                    if existing_contents != expected_contents.as_ref()
                        && relative_path == INSTALL_METADATA_FILE_NAME
                    {
                        match install_metadata_match(&existing_contents, expected_contents.as_ref())
                        {
                            ExistingRegistryInstallMatch::Match => {}
                            ExistingRegistryInstallMatch::AdoptReceipt => adopt_receipt = true,
                            ExistingRegistryInstallMatch::Conflict => {
                                return Ok(ExistingRegistryInstallMatch::Conflict);
                            }
                        }
                    } else if existing_contents != expected_contents.as_ref() {
                        return Ok(ExistingRegistryInstallMatch::Conflict);
                    }
                }
                FileType::Directory => {
                    if !expected_dirs.contains(&relative_path) {
                        return Ok(ExistingRegistryInstallMatch::Conflict);
                    }
                    stack.push((
                        scoped_child(&dir_path, &entry.name)?,
                        format!("{relative_path}/"),
                    ));
                }
                FileType::Symlink | FileType::Other => {
                    return Ok(ExistingRegistryInstallMatch::Conflict);
                }
            }
        }
    }
    if !expected_files.is_empty() {
        return Ok(ExistingRegistryInstallMatch::Conflict);
    }
    Ok(if adopt_receipt {
        ExistingRegistryInstallMatch::AdoptReceipt
    } else {
        ExistingRegistryInstallMatch::Match
    })
}

fn install_metadata_match(existing: &[u8], expected: &[u8]) -> ExistingRegistryInstallMatch {
    let (Ok(existing), Ok(expected)) = (
        serde_json::from_slice::<InstalledSkillMetadata>(existing),
        serde_json::from_slice::<InstalledSkillMetadata>(expected),
    ) else {
        return ExistingRegistryInstallMatch::Conflict;
    };
    if existing.source != expected.source
        || existing.source_url != expected.source_url
        || existing.source_subdir != expected.source_subdir
    {
        return ExistingRegistryInstallMatch::Conflict;
    }
    match (&existing.registry_provenance, &expected.registry_provenance) {
        (Some(existing), Some(expected)) if existing.same_package_identity(expected) => {
            ExistingRegistryInstallMatch::Match
        }
        (None, Some(_)) => ExistingRegistryInstallMatch::AdoptReceipt,
        (None, None) => ExistingRegistryInstallMatch::Match,
        _ => ExistingRegistryInstallMatch::Conflict,
    }
}

pub(super) async fn adopt_registry_receipt(
    context: &SkillManagementContext,
    skill_name: &str,
    source_url: Option<&str>,
    provenance: &RegistryPackageProvenance,
) -> Result<(), SkillManagementError> {
    let metadata_path = skill_bundle_file_scoped_path(skill_name, INSTALL_METADATA_FILE_NAME)?;
    let metadata = install_metadata_bytes(source_url, Some(provenance))?;
    context
        .filesystem
        .write_file(&context.scope, &metadata_path, &metadata)
        .await
        .map_err(filesystem_error)
}

fn expected_directory_prefixes<'a>(
    paths: impl IntoIterator<Item = &'a String>,
) -> BTreeSet<String> {
    let mut dirs = BTreeSet::new();
    for path in paths {
        let mut current = path.as_str();
        while let Some((parent, _)) = current.rsplit_once('/') {
            dirs.insert(parent.to_string());
            current = parent;
        }
    }
    dirs
}

pub(super) fn validate_install_bundle_files(
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

pub(super) fn installed_skill_source(source: SkillInstallSource) -> SkillSource {
    match source {
        SkillInstallSource::User => SkillSource::User,
        SkillInstallSource::InstalledUrl => SkillSource::Installed,
    }
}

pub(super) fn install_metadata_source(default_source: SkillSource, bytes: &[u8]) -> SkillSource {
    if default_source == SkillSource::User
        && InstalledSkillMetadata::sidecar_bytes_mark_installed(bytes)
    {
        SkillSource::Installed
    } else {
        default_source
    }
}

pub(super) async fn read_install_metadata_bytes(
    context: &SkillManagementContext,
    skill_path: &ScopedPath,
) -> Result<Option<Vec<u8>>, SkillManagementError> {
    let Some(metadata_path) = scoped_sibling(skill_path, INSTALL_METADATA_FILE_NAME)? else {
        return Ok(None);
    };
    match context
        .filesystem
        .read_bytes_bounded(&context.scope, &metadata_path, MAX_INSTALL_METADATA_BYTES)
        .await
    {
        Ok(Some(bytes)) => Ok(Some(bytes)),
        Ok(None) => {
            tracing::warn!(
                scoped_path = %metadata_path,
                max_bytes = MAX_INSTALL_METADATA_BYTES,
                "skill install metadata sidecar exceeded bounded read limit; treating as installed"
            );
            Ok(Some(Vec::new()))
        }
        Err(FilesystemError::NotFound { .. }) => Ok(None),
        Err(error) => Err(filesystem_error(error)),
    }
}

pub(super) async fn read_install_metadata_bytes_strict(
    context: &SkillManagementContext,
    skill_path: &ScopedPath,
) -> Result<Option<Vec<u8>>, SkillManagementError> {
    let Some(metadata_path) = scoped_sibling(skill_path, INSTALL_METADATA_FILE_NAME)? else {
        return Ok(None);
    };
    match context
        .filesystem
        .read_bytes_bounded(&context.scope, &metadata_path, MAX_INSTALL_METADATA_BYTES)
        .await
    {
        Ok(Some(bytes)) => Ok(Some(bytes)),
        Ok(None) => Err(SkillManagementError::with_reason(
            SkillManagementErrorKind::Resource,
            format!(
                "skill install metadata at {metadata_path} exceeds the {MAX_INSTALL_METADATA_BYTES}-byte limit"
            ),
        )),
        Err(FilesystemError::NotFound { .. }) => Ok(None),
        Err(error) => Err(filesystem_error(error)),
    }
}

fn install_metadata_bytes(
    source_url: Option<&str>,
    registry_provenance: Option<&RegistryPackageProvenance>,
) -> Result<Vec<u8>, SkillManagementError> {
    let metadata = registry_provenance
        .cloned()
        .map(|provenance| InstalledSkillMetadata::installed_registry(source_url, provenance))
        .unwrap_or_else(|| InstalledSkillMetadata::installed_url(source_url));
    let bytes = metadata.to_pretty_json().map_err(|error| {
        SkillManagementError::with_reason(
            SkillManagementErrorKind::InvalidInput,
            format!("failed to serialize skill install metadata: {error}"),
        )
    })?;
    if bytes.len() > MAX_INSTALL_METADATA_BYTES {
        return Err(SkillManagementError::new(
            SkillManagementErrorKind::Resource,
        ));
    }
    Ok(bytes)
}

fn skill_bundle_file_scoped_path(
    skill_name: &str,
    relative_path: &str,
) -> Result<ScopedPath, SkillManagementError> {
    ScopedPath::new(format!(
        "{}/{}/{}",
        USER_SKILLS_ROOT.trim_end_matches('/'),
        skill_name,
        relative_path
    ))
    .map_err(|_| SkillManagementError::new(SkillManagementErrorKind::InvalidInput))
}

fn normalize_install_relative_path(path: &str) -> Result<String, SkillManagementError> {
    let normalized = normalize_snapshot_relative_path(path)?;
    if path.contains("://")
        || normalized == SKILL_FILE_NAME
        || normalized == INSTALL_METADATA_FILE_NAME
    {
        return Err(SkillManagementError::new(
            SkillManagementErrorKind::InvalidInput,
        ));
    }
    Ok(normalized)
}

fn normalize_snapshot_relative_path(path: &str) -> Result<String, SkillManagementError> {
    if path.is_empty()
        || path.starts_with('/')
        || path.contains('\\')
        || path.contains('\0')
        || path.chars().any(char::is_control)
    {
        return Err(SkillManagementError::new(
            SkillManagementErrorKind::InvalidInput,
        ));
    }

    let normalized = normalize_safe_relative_path(std::path::Path::new(path))
        .map_err(|_| SkillManagementError::new(SkillManagementErrorKind::InvalidInput))?;
    normalized
        .to_str()
        .map(str::to_string)
        .ok_or_else(|| SkillManagementError::new(SkillManagementErrorKind::InvalidInput))
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

fn scoped_child(parent: &ScopedPath, name: &str) -> Result<ScopedPath, SkillManagementError> {
    if name.contains('/') || name.contains('\\') || name.chars().any(char::is_control) {
        return Err(SkillManagementError::new(
            SkillManagementErrorKind::InvalidInput,
        ));
    }
    ScopedPath::new(format!("{}/{name}", parent.as_str().trim_end_matches('/')))
        .map_err(|_| SkillManagementError::new(SkillManagementErrorKind::InvalidInput))
}

async fn create_dir_all(
    context: &SkillManagementContext,
    skill_name: &str,
    phase: &'static str,
    path: &ScopedPath,
) -> Result<(), SkillManagementError> {
    log_skill_filesystem_phase(phase, skill_name, path);
    context
        .filesystem
        .create_dir_all(&context.scope, path)
        .await
        .or_else(|error| match error {
            FilesystemError::Unsupported {
                operation: FilesystemOperation::CreateDirAll,
                ..
            } => {
                log_skill_filesystem_phase("create_dir_all_unsupported", skill_name, path);
                Ok(())
            }
            other => Err(other),
        })
        .map_err(|error| {
            log_skill_filesystem_phase("create_dir_all_failed", skill_name, path);
            filesystem_error(error)
        })
}

async fn cleanup_partial_install(
    context: &SkillManagementContext,
    skill_name: &str,
    skill_dir: &ScopedPath,
) -> Result<(), SkillManagementError> {
    log_skill_filesystem_phase("cleanup_partial_install", skill_name, skill_dir);
    if let Err(error) = context.filesystem.delete(&context.scope, skill_dir).await {
        tracing::debug!(
            skill_name,
            scoped_path = %skill_dir,
            error = ?error,
            "skill install failed to clean up partial bundle"
        );
        return Err(filesystem_error(error));
    }
    Ok(())
}

async fn read_existing_file_bytes(
    context: &SkillManagementContext,
    path: &ScopedPath,
    expected_len: usize,
) -> Result<Option<Vec<u8>>, SkillManagementError> {
    match context
        .filesystem
        .read_bytes_bounded(&context.scope, path, expected_len.saturating_add(1))
        .await
    {
        Ok(Some(bytes)) if bytes.len() == expected_len => Ok(Some(bytes)),
        Ok(Some(_)) | Ok(None) | Err(FilesystemError::NotFound { .. }) => Ok(None),
        Err(error) => Err(filesystem_error(error)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_install_relative_path_rejects_injection_vectors() {
        for path in [
            r"nested\file.txt",
            "nested/\0file.txt",
            "nested/\nfile.txt",
            "https://example.com/file.txt",
        ] {
            assert!(normalize_install_relative_path(path).is_err());
        }
    }

    #[test]
    fn normalize_snapshot_path_preserves_reserved_files_without_allowing_escape() {
        assert_eq!(
            normalize_snapshot_relative_path(SKILL_FILE_NAME).expect("skill path"),
            SKILL_FILE_NAME
        );
        assert_eq!(
            normalize_snapshot_relative_path(INSTALL_METADATA_FILE_NAME).expect("metadata path"),
            INSTALL_METADATA_FILE_NAME
        );
        for path in ["/absolute", "../escape", r"nested\escape", "nested/\nfile"] {
            assert!(normalize_snapshot_relative_path(path).is_err());
        }
    }
}
