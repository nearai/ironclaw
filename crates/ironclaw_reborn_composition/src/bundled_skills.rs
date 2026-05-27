use std::collections::HashSet;
use std::fs;
use std::hash::Hasher;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use ironclaw_filesystem::{
    CasExpectation, Entry, FileType, FilesystemError, LocalFilesystem, RootFilesystem,
};
use ironclaw_host_api::{HostPath, VirtualPath};
use ironclaw_loop_support::SkillFilePath;
use ironclaw_skills::{ManagedSkillSource, SkillSummary};
use serde::{Deserialize, Serialize};
use tokio::time::sleep;

use crate::RebornBuildError;

const EMBEDDED_REBORN_SKILL_SUMMARIES_JSON: &str = include_str!(concat!(
    env!("OUT_DIR"),
    "/embedded_reborn_skill_summaries.json"
));
const EMBEDDED_REBORN_SKILL_BUNDLES_JSON: &str = include_str!(concat!(
    env!("OUT_DIR"),
    "/embedded_reborn_skill_bundles.json"
));
const BUNDLED_MARKER_FILE: &str = ".ironclaw-reborn-bundled.json";
const BUNDLED_INSTALL_LOCK_FILE: &str = ".ironclaw-reborn-bundled.lock";
const BUNDLED_MARKER_OWNER: &str = "ironclaw_reborn_composition_bundled_skill";
const BUNDLED_INSTALL_LOCK_TIMEOUT: Duration = Duration::from_secs(30);
const BUNDLED_INSTALL_LOCK_RETRY: Duration = Duration::from_millis(25);
const SYSTEM_SKILLS_ROOT: &str = "/projects/system/skills";

#[derive(Debug, Deserialize)]
struct EmbeddedRebornSkillSummary {
    name: String,
    version: String,
    description: String,
    keywords: Vec<String>,
    tags: Vec<String>,
    requires_skills: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct EmbeddedRebornSkillBundle {
    name: String,
    files: Vec<EmbeddedRebornSkillFile>,
}

#[derive(Debug, Deserialize)]
struct EmbeddedRebornSkillFile {
    path: String,
    bytes: Vec<u8>,
}

#[derive(Debug, Deserialize, Serialize)]
struct BundledSkillMarker {
    owner: String,
    format: u8,
    content_hash: String,
}

pub(crate) async fn ensure_bundled_reborn_skills_installed(
    local_dev_storage_root: &Path,
) -> Result<(), RebornBuildError> {
    let bundled_skills = embedded_reborn_skill_bundles()?;
    let filesystem = local_dev_storage_filesystem(local_dev_storage_root)?;
    let system_skills_root = system_skills_root_path()?;
    create_dir_all(&filesystem, &system_skills_root).await?;
    let install_lock = BundledSkillInstallLock::acquire(&filesystem, &system_skills_root).await?;

    let result = async {
        let bundled_names = bundled_skills
            .iter()
            .map(|skill| skill.name.as_str())
            .collect::<HashSet<_>>();
        remove_stale_managed_skills(&filesystem, &system_skills_root, &bundled_names).await?;

        for skill in bundled_skills {
            install_bundled_skill(&filesystem, &system_skills_root, skill).await?;
        }
        Ok(())
    }
    .await;

    let release_result = install_lock.release(&filesystem).await;
    match (result, release_result) {
        (Err(error), _) => Err(error),
        (Ok(()), Err(error)) => Err(error),
        (Ok(()), Ok(())) => Ok(()),
    }
}

pub(crate) fn bundled_reborn_skill_summaries() -> Result<Vec<SkillSummary>, RebornBuildError> {
    Ok(embedded_reborn_skill_summaries()?
        .into_iter()
        .map(|skill| SkillSummary {
            name: skill.name,
            version: skill.version,
            description: skill.description,
            source: ManagedSkillSource::System,
            keywords: skill.keywords,
            tags: skill.tags,
            requires_skills: skill.requires_skills,
        })
        .collect())
}

fn embedded_reborn_skill_summaries() -> Result<Vec<EmbeddedRebornSkillSummary>, RebornBuildError> {
    serde_json::from_str(EMBEDDED_REBORN_SKILL_SUMMARIES_JSON).map_err(|error| {
        invalid_config(format!(
            "failed to parse embedded Reborn skill summaries: {error}"
        ))
    })
}

fn embedded_reborn_skill_bundles() -> Result<Vec<EmbeddedRebornSkillBundle>, RebornBuildError> {
    serde_json::from_str(EMBEDDED_REBORN_SKILL_BUNDLES_JSON).map_err(|error| {
        invalid_config(format!(
            "failed to parse embedded Reborn skill bundles: {error}"
        ))
    })
}

fn local_dev_storage_filesystem(
    local_dev_storage_root: &Path,
) -> Result<LocalFilesystem, RebornBuildError> {
    let storage_root = prepare_local_dev_storage_root(local_dev_storage_root)?;
    let mut filesystem = LocalFilesystem::new();
    filesystem
        .mount_local(
            VirtualPath::new("/projects")?,
            HostPath::from_path_buf(storage_root),
        )
        .map_err(invalid_config)?;
    Ok(filesystem)
}

fn prepare_local_dev_storage_root(
    local_dev_storage_root: &Path,
) -> Result<PathBuf, RebornBuildError> {
    reject_existing_symlink(local_dev_storage_root, "local-dev skill storage root")?;
    fs::create_dir_all(local_dev_storage_root).map_err(invalid_config)?;
    reject_existing_symlink(local_dev_storage_root, "local-dev skill storage root")?;
    let metadata = fs::metadata(local_dev_storage_root).map_err(invalid_config)?;
    if !metadata.is_dir() {
        return Err(invalid_config(format!(
            "local-dev skill storage root is not a directory: {}",
            local_dev_storage_root.display()
        )));
    }
    local_dev_storage_root
        .canonicalize()
        .map_err(invalid_config)
}

fn reject_existing_symlink(path: &Path, label: &str) -> Result<(), RebornBuildError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(invalid_config(format!(
            "{label} must not be a symlink: {}",
            path.display()
        ))),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(invalid_config(error)),
    }
}

fn system_skills_root_path() -> Result<VirtualPath, RebornBuildError> {
    VirtualPath::new(SYSTEM_SKILLS_ROOT).map_err(invalid_config)
}

struct BundledSkillInstallLock {
    path: VirtualPath,
}

impl BundledSkillInstallLock {
    async fn acquire(
        filesystem: &dyn RootFilesystem,
        system_skills_root: &VirtualPath,
    ) -> Result<Self, RebornBuildError> {
        let path = child_path(system_skills_root, BUNDLED_INSTALL_LOCK_FILE)?;
        let started_at = Instant::now();
        loop {
            match filesystem
                .put(
                    &path,
                    Entry::bytes(format!("{:?}", started_at).into_bytes()),
                    CasExpectation::Absent,
                )
                .await
            {
                Ok(_) => return Ok(Self { path }),
                Err(error)
                    if matches!(error, FilesystemError::VersionMismatch { .. })
                        && started_at.elapsed() < BUNDLED_INSTALL_LOCK_TIMEOUT =>
                {
                    sleep(BUNDLED_INSTALL_LOCK_RETRY).await;
                }
                Err(FilesystemError::VersionMismatch { .. }) => {
                    return Err(invalid_config(format!(
                        "timed out waiting for bundled skill install lock: {}",
                        path
                    )));
                }
                Err(error) => return Err(invalid_config(error)),
            }
        }
    }

    async fn release(self, filesystem: &dyn RootFilesystem) -> Result<(), RebornBuildError> {
        delete_if_exists(filesystem, &self.path).await
    }
}

async fn remove_stale_managed_skills(
    filesystem: &dyn RootFilesystem,
    system_skills_root: &VirtualPath,
    bundled_names: &HashSet<&str>,
) -> Result<(), RebornBuildError> {
    let entries = filesystem
        .list_dir(system_skills_root)
        .await
        .map_err(invalid_config)?;
    for entry in entries {
        if entry.file_type != FileType::Directory {
            continue;
        }
        if bundled_names.contains(entry.name.as_str())
            || read_managed_marker(filesystem, &entry.path)
                .await?
                .is_none()
        {
            continue;
        }
        filesystem.delete(&entry.path).await.map_err(|error| {
            invalid_config(format!(
                "failed to remove stale bundled skill {}: {error}",
                entry.name
            ))
        })?;
    }
    Ok(())
}

async fn install_bundled_skill(
    filesystem: &dyn RootFilesystem,
    system_skills_root: &VirtualPath,
    skill: EmbeddedRebornSkillBundle,
) -> Result<(), RebornBuildError> {
    let skill_dir = child_path(system_skills_root, &skill.name)?;
    let content_hash = bundled_skill_hash(&skill);
    if path_exists(filesystem, &skill_dir).await? {
        let Some(marker) = read_managed_marker(filesystem, &skill_dir).await? else {
            tracing::warn!(
                skill_name = %skill.name,
                path = %skill_dir,
                "skipping bundled Reborn skill because an unmanaged system skill already exists"
            );
            return Ok(());
        };
        if marker.content_hash == content_hash {
            return Ok(());
        }
        filesystem.delete(&skill_dir).await.map_err(|error| {
            invalid_config(format!(
                "failed to remove changed bundled skill {}: {error}",
                skill.name
            ))
        })?;
    }

    if let Err(error) = write_bundled_skill_dir(filesystem, &skill_dir, &skill, &content_hash).await
    {
        let cleanup_result = delete_if_exists(filesystem, &skill_dir).await;
        if let Err(cleanup_error) = cleanup_result {
            return Err(invalid_config(format!(
                "failed to install bundled skill {}; cleanup failed after {error}: {cleanup_error}",
                skill.name
            )));
        }
        return Err(error);
    }
    Ok(())
}

async fn write_bundled_skill_dir(
    filesystem: &dyn RootFilesystem,
    skill_dir: &VirtualPath,
    skill: &EmbeddedRebornSkillBundle,
    content_hash: &str,
) -> Result<(), RebornBuildError> {
    for file in &skill.files {
        let relative_path = validated_bundle_file_path(&file.path)?;
        let target = bundle_file_path(skill_dir, &relative_path)?;
        filesystem
            .put(
                &target,
                Entry::bytes(file.bytes.clone()),
                CasExpectation::Any,
            )
            .await
            .map_err(|error| {
                invalid_config(format!(
                    "failed to write bundled skill file {}: {error}",
                    target
                ))
            })?;
    }
    write_marker(filesystem, skill_dir, content_hash).await
}

async fn read_managed_marker(
    filesystem: &dyn RootFilesystem,
    skill_dir: &VirtualPath,
) -> Result<Option<BundledSkillMarker>, RebornBuildError> {
    let marker_path = child_path(skill_dir, BUNDLED_MARKER_FILE)?;
    let Some(entry) = filesystem.get(&marker_path).await.map_err(invalid_config)? else {
        return Ok(None);
    };
    let Some(marker) = serde_json::from_slice::<BundledSkillMarker>(&entry.entry.body).ok() else {
        return Ok(None);
    };
    Ok((marker.owner == BUNDLED_MARKER_OWNER).then_some(marker))
}

async fn write_marker(
    filesystem: &dyn RootFilesystem,
    skill_dir: &VirtualPath,
    content_hash: &str,
) -> Result<(), RebornBuildError> {
    let marker = BundledSkillMarker {
        owner: BUNDLED_MARKER_OWNER.to_string(),
        format: 1,
        content_hash: content_hash.to_string(),
    };
    let marker_path = child_path(skill_dir, BUNDLED_MARKER_FILE)?;
    let bytes = serde_json::to_vec_pretty(&marker).map_err(invalid_config)?;
    filesystem
        .put(&marker_path, Entry::bytes(bytes), CasExpectation::Any)
        .await
        .map(|_| ())
        .map_err(|error| {
            invalid_config(format!(
                "failed to write bundled skill marker {}: {error}",
                marker_path
            ))
        })
}

async fn create_dir_all(
    filesystem: &dyn RootFilesystem,
    path: &VirtualPath,
) -> Result<(), RebornBuildError> {
    filesystem
        .create_dir_all(path)
        .await
        .map_err(invalid_config)
}

async fn path_exists(
    filesystem: &dyn RootFilesystem,
    path: &VirtualPath,
) -> Result<bool, RebornBuildError> {
    match filesystem.stat(path).await {
        Ok(_) => Ok(true),
        Err(FilesystemError::NotFound { .. }) => Ok(false),
        Err(error) => Err(invalid_config(error)),
    }
}

async fn delete_if_exists(
    filesystem: &dyn RootFilesystem,
    path: &VirtualPath,
) -> Result<(), RebornBuildError> {
    match filesystem.delete(path).await {
        Ok(()) => Ok(()),
        Err(FilesystemError::NotFound { .. }) => Ok(()),
        Err(error) => Err(invalid_config(error)),
    }
}

fn child_path(parent: &VirtualPath, child: &str) -> Result<VirtualPath, RebornBuildError> {
    VirtualPath::new(format!(
        "{}/{}",
        parent.as_str().trim_end_matches('/'),
        child
    ))
    .map_err(invalid_config)
}

fn bundle_file_path(
    skill_dir: &VirtualPath,
    relative_path: &Path,
) -> Result<VirtualPath, RebornBuildError> {
    let relative_path = relative_path
        .to_str()
        .ok_or_else(|| invalid_config("bundled skill file path must be UTF-8"))?
        .replace('\\', "/");
    child_path(skill_dir, &relative_path)
}

fn validated_bundle_file_path(path: &str) -> Result<PathBuf, RebornBuildError> {
    let path = SkillFilePath::new(path)
        .map_err(|error| invalid_config(format!("invalid bundled skill file path: {error}")))?;
    Ok(Path::new(path.as_str()).to_path_buf())
}

fn bundled_skill_hash(skill: &EmbeddedRebornSkillBundle) -> String {
    let mut hasher = StableFnv64::default();
    hasher.write(skill.name.as_bytes());
    for file in &skill.files {
        hasher.write(file.path.as_bytes());
        hasher.write(&[0]);
        hasher.write(&file.bytes);
        hasher.write(&[0]);
    }
    format!("{:016x}", hasher.finish())
}

#[derive(Default)]
struct StableFnv64(u64);

impl Hasher for StableFnv64 {
    fn finish(&self) -> u64 {
        if self.0 == 0 {
            0xcbf29ce484222325
        } else {
            self.0
        }
    }

    fn write(&mut self, bytes: &[u8]) {
        let mut hash = self.finish();
        for byte in bytes {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x100000001b3);
        }
        self.0 = hash;
    }
}

fn invalid_config(reason: impl std::fmt::Display) -> RebornBuildError {
    RebornBuildError::InvalidConfig {
        reason: reason.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn bundled_reborn_skills_include_current_repo_bundles_and_assets() {
        let dir = tempfile::tempdir().expect("tempdir");
        let local_dev_root = dir.path().join("local-dev");

        ensure_bundled_reborn_skills_installed(&local_dev_root)
            .await
            .expect("install bundled skills");

        assert!(
            local_dev_root
                .join("system/skills/code-review/SKILL.md")
                .is_file()
        );
        assert!(
            local_dev_root
                .join("system/skills/portfolio/scripts/backtest_strategy.py")
                .is_file()
        );
    }

    #[tokio::test]
    async fn bundled_reborn_skills_do_not_overwrite_unmanaged_system_skills() {
        let dir = tempfile::tempdir().expect("tempdir");
        let local_dev_root = dir.path().join("local-dev");
        let skill_dir = local_dev_root.join("system/skills/code-review");
        fs::create_dir_all(&skill_dir).expect("mkdir");
        fs::write(skill_dir.join("SKILL.md"), "operator-owned").expect("write");

        ensure_bundled_reborn_skills_installed(&local_dev_root)
            .await
            .expect("install bundled skills");

        assert_eq!(
            fs::read_to_string(skill_dir.join("SKILL.md")).expect("read"),
            "operator-owned"
        );
    }

    #[tokio::test]
    async fn bundled_reborn_skills_skip_unchanged_managed_dirs() {
        let dir = tempfile::tempdir().expect("tempdir");
        let local_dev_root = dir.path().join("local-dev");
        let skill_md = local_dev_root.join("system/skills/code-review/SKILL.md");

        ensure_bundled_reborn_skills_installed(&local_dev_root)
            .await
            .expect("install bundled skills");
        let first_modified = fs::metadata(&skill_md)
            .expect("metadata")
            .modified()
            .expect("modified");

        ensure_bundled_reborn_skills_installed(&local_dev_root)
            .await
            .expect("install bundled skills");

        assert_eq!(
            fs::metadata(&skill_md)
                .expect("metadata")
                .modified()
                .expect("modified"),
            first_modified
        );
    }

    #[tokio::test]
    async fn bundled_reborn_skills_replace_changed_managed_dirs() {
        let dir = tempfile::tempdir().expect("tempdir");
        let local_dev_root = dir.path().join("local-dev");
        let skill_dir = local_dev_root.join("system/skills/code-review");
        let skill_md = skill_dir.join("SKILL.md");

        ensure_bundled_reborn_skills_installed(&local_dev_root)
            .await
            .expect("install bundled skills");
        let bundled_skill_md = fs::read_to_string(&skill_md).expect("read bundled skill");
        fs::write(&skill_md, "old managed skill").expect("write old skill");
        fs::write(skill_dir.join("OLD_SENTINEL"), "old").expect("write old sentinel");
        write_marker_file(&skill_dir, "stale-content-hash");

        ensure_bundled_reborn_skills_installed(&local_dev_root)
            .await
            .expect("replace bundled skills");

        assert_eq!(
            fs::read_to_string(&skill_md).expect("read replaced skill"),
            bundled_skill_md
        );
        assert!(!skill_dir.join("OLD_SENTINEL").exists());
        assert_no_bundle_scratch_dirs(&local_dev_root.join("system/skills"));
    }

    #[tokio::test]
    async fn bundled_reborn_skills_remove_stale_managed_dirs() {
        let dir = tempfile::tempdir().expect("tempdir");
        let local_dev_root = dir.path().join("local-dev");
        let system_skills_root = local_dev_root.join("system/skills");
        let obsolete_dir = system_skills_root.join("obsolete-managed");
        let operator_dir = system_skills_root.join("operator-owned");
        fs::create_dir_all(&obsolete_dir).expect("obsolete dir");
        fs::write(obsolete_dir.join("SKILL.md"), "obsolete").expect("obsolete skill");
        write_marker_file(&obsolete_dir, "obsolete-hash");
        fs::create_dir_all(&operator_dir).expect("operator dir");
        fs::write(operator_dir.join("SKILL.md"), "operator").expect("operator skill");
        fs::write(
            operator_dir.join(BUNDLED_MARKER_FILE),
            r#"{"owner":"operator","format":1,"content_hash":"operator-hash"}"#,
        )
        .expect("operator marker");

        ensure_bundled_reborn_skills_installed(&local_dev_root)
            .await
            .expect("install bundled skills");

        assert!(!obsolete_dir.exists());
        assert!(operator_dir.join("SKILL.md").is_file());
    }

    fn assert_no_bundle_scratch_dirs(system_skills_root: &Path) {
        for entry in fs::read_dir(system_skills_root).expect("read system skills") {
            let entry = entry.expect("system skill entry");
            let name = entry.file_name().to_string_lossy().to_string();
            assert!(
                !name.contains(".tmp-") && !name.contains(".previous-"),
                "unexpected bundled skill scratch dir: {name}"
            );
        }
    }

    fn write_marker_file(skill_dir: &Path, content_hash: &str) {
        let marker = BundledSkillMarker {
            owner: BUNDLED_MARKER_OWNER.to_string(),
            format: 1,
            content_hash: content_hash.to_string(),
        };
        let bytes = serde_json::to_vec_pretty(&marker).expect("marker json");
        fs::write(skill_dir.join(BUNDLED_MARKER_FILE), bytes).expect("write marker");
    }
}
