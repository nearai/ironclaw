use std::collections::HashSet;
use std::fs;
use std::hash::Hasher;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use ironclaw_loop_support::SkillFilePath;
use ironclaw_skills::{ManagedSkillSource, SkillSummary};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::RebornBuildError;

const EMBEDDED_REBORN_SKILLS_JSON: &str =
    include_str!(concat!(env!("OUT_DIR"), "/embedded_reborn_skills.json"));
const BUNDLED_MARKER_FILE: &str = ".ironclaw-reborn-bundled.json";
const BUNDLED_INSTALL_LOCK_DIR: &str = ".ironclaw-reborn-bundled.lock";
const BUNDLED_MARKER_OWNER: &str = "ironclaw_reborn_composition_bundled_skill";
const BUNDLED_INSTALL_LOCK_TIMEOUT: Duration = Duration::from_secs(30);
const BUNDLED_INSTALL_LOCK_RETRY: Duration = Duration::from_millis(25);

#[derive(Debug, Deserialize)]
struct EmbeddedRebornSkill {
    name: String,
    version: String,
    description: String,
    keywords: Vec<String>,
    tags: Vec<String>,
    requires_skills: Vec<String>,
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

pub(crate) fn ensure_bundled_reborn_skills_installed(
    local_dev_storage_root: &Path,
) -> Result<(), RebornBuildError> {
    let bundled_skills = embedded_reborn_skills()?;
    let system_skills_root = prepare_system_skills_root(local_dev_storage_root)?;
    let _install_lock = BundledSkillInstallLock::acquire(&system_skills_root)?;

    let bundled_names = bundled_skills
        .iter()
        .map(|skill| skill.name.as_str())
        .collect::<HashSet<_>>();
    remove_stale_managed_skills(&system_skills_root, &bundled_names)?;

    for skill in bundled_skills {
        install_bundled_skill(&system_skills_root, skill)?;
    }
    Ok(())
}

pub(crate) fn bundled_reborn_skill_summaries() -> Result<Vec<SkillSummary>, RebornBuildError> {
    Ok(embedded_reborn_skills()?
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

fn embedded_reborn_skills() -> Result<Vec<EmbeddedRebornSkill>, RebornBuildError> {
    serde_json::from_str(EMBEDDED_REBORN_SKILLS_JSON)
        .map_err(|error| invalid_config(format!("failed to parse embedded Reborn skills: {error}")))
}

fn prepare_system_skills_root(local_dev_storage_root: &Path) -> Result<PathBuf, RebornBuildError> {
    fs::create_dir_all(local_dev_storage_root).map_err(invalid_config)?;
    ensure_real_directory(local_dev_storage_root, "local-dev skill storage root")?;
    let storage_root = local_dev_storage_root
        .canonicalize()
        .map_err(invalid_config)?;

    let system_root = local_dev_storage_root.join("system");
    ensure_real_directory(&system_root, "local-dev system skill storage root")?;
    let system_skills_root = system_root.join("skills");
    ensure_real_directory(
        &system_skills_root,
        "local-dev system skill storage skills root",
    )?;

    let canonical_system_skills_root = system_skills_root.canonicalize().map_err(invalid_config)?;
    if !canonical_system_skills_root.starts_with(&storage_root) {
        return Err(invalid_config(format!(
            "local-dev system skills root escapes storage root: {}",
            system_skills_root.display()
        )));
    }
    Ok(canonical_system_skills_root)
}

fn ensure_real_directory(path: &Path, label: &str) -> Result<(), RebornBuildError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(invalid_config(format!(
            "{label} must not be a symlink: {}",
            path.display()
        ))),
        Ok(metadata) if !metadata.is_dir() => Err(invalid_config(format!(
            "{label} is not a directory: {}",
            path.display()
        ))),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => {
            if let Err(error) = fs::create_dir(path)
                && error.kind() != ErrorKind::AlreadyExists
            {
                return Err(invalid_config(error));
            }
            ensure_real_directory(path, label)
        }
        Err(error) => Err(invalid_config(error)),
    }
}

struct BundledSkillInstallLock {
    path: PathBuf,
}

impl BundledSkillInstallLock {
    fn acquire(system_skills_root: &Path) -> Result<Self, RebornBuildError> {
        let path = system_skills_root.join(BUNDLED_INSTALL_LOCK_DIR);
        let started_at = Instant::now();
        loop {
            match fs::create_dir(&path) {
                Ok(()) => return Ok(Self { path }),
                Err(error)
                    if error.kind() == ErrorKind::AlreadyExists
                        && started_at.elapsed() < BUNDLED_INSTALL_LOCK_TIMEOUT =>
                {
                    thread::sleep(BUNDLED_INSTALL_LOCK_RETRY);
                }
                Err(error) if error.kind() == ErrorKind::AlreadyExists => {
                    return Err(invalid_config(format!(
                        "timed out waiting for bundled skill install lock: {}",
                        path.display()
                    )));
                }
                Err(error) => return Err(invalid_config(error)),
            }
        }
    }
}

impl Drop for BundledSkillInstallLock {
    fn drop(&mut self) {
        let _ = fs::remove_dir(&self.path);
    }
}

fn remove_stale_managed_skills(
    system_skills_root: &Path,
    bundled_names: &HashSet<&str>,
) -> Result<(), RebornBuildError> {
    let entries = fs::read_dir(system_skills_root).map_err(invalid_config)?;
    for entry in entries {
        let entry = entry.map_err(invalid_config)?;
        if !entry.file_type().map_err(invalid_config)?.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if bundled_names.contains(name.as_str()) || read_managed_marker(&entry.path()).is_none() {
            continue;
        }
        fs::remove_dir_all(entry.path()).map_err(|error| {
            invalid_config(format!(
                "failed to remove stale bundled skill {name}: {error}"
            ))
        })?;
    }
    Ok(())
}

fn install_bundled_skill(
    system_skills_root: &Path,
    skill: EmbeddedRebornSkill,
) -> Result<(), RebornBuildError> {
    let skill_dir = system_skills_root.join(&skill.name);
    let content_hash = bundled_skill_hash(&skill);
    if skill_dir.exists() {
        let Some(marker) = read_managed_marker(&skill_dir) else {
            tracing::warn!(
                skill_name = %skill.name,
                path = %skill_dir.display(),
                "skipping bundled Reborn skill because an unmanaged system skill already exists"
            );
            return Ok(());
        };
        if marker.content_hash == content_hash {
            return Ok(());
        }
    }

    let staging_dir = system_skills_root.join(format!(".{}.tmp-{}", skill.name, Uuid::new_v4()));
    if staging_dir.exists() {
        fs::remove_dir_all(&staging_dir).map_err(invalid_config)?;
    }
    write_bundled_skill_dir(&staging_dir, &skill, &content_hash)?;
    replace_skill_dir(&skill_dir, &staging_dir, &skill.name)
}

fn write_bundled_skill_dir(
    staging_dir: &Path,
    skill: &EmbeddedRebornSkill,
    content_hash: &str,
) -> Result<(), RebornBuildError> {
    fs::create_dir_all(staging_dir).map_err(invalid_config)?;
    for file in &skill.files {
        let relative_path = validated_bundle_file_path(&file.path)?;
        let target = staging_dir.join(relative_path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(invalid_config)?;
        }
        fs::write(&target, &file.bytes).map_err(|error| {
            invalid_config(format!(
                "failed to write bundled skill file {}: {error}",
                target.display()
            ))
        })?;
    }
    write_marker(staging_dir, content_hash)
}

fn replace_skill_dir(
    skill_dir: &Path,
    staging_dir: &Path,
    skill_name: &str,
) -> Result<(), RebornBuildError> {
    if !skill_dir.exists() {
        return fs::rename(staging_dir, skill_dir).map_err(invalid_config);
    }

    let backup_dir = skill_dir.with_file_name(format!(".{skill_name}.previous-{}", Uuid::new_v4()));
    fs::rename(skill_dir, &backup_dir).map_err(invalid_config)?;
    if let Err(error) = fs::rename(staging_dir, skill_dir) {
        if let Err(restore_error) = fs::rename(&backup_dir, skill_dir) {
            return Err(invalid_config(format!(
                "failed to replace bundled skill {skill_name}: {error}; restore failed: {restore_error}"
            )));
        }
        return Err(invalid_config(format!(
            "failed to replace bundled skill {skill_name}: {error}"
        )));
    }
    fs::remove_dir_all(&backup_dir).map_err(invalid_config)
}

fn read_managed_marker(skill_dir: &Path) -> Option<BundledSkillMarker> {
    let marker_path = skill_dir.join(BUNDLED_MARKER_FILE);
    let bytes = fs::read(marker_path).ok()?;
    let marker = serde_json::from_slice::<BundledSkillMarker>(&bytes).ok()?;
    (marker.owner == BUNDLED_MARKER_OWNER).then_some(marker)
}

fn write_marker(skill_dir: &Path, content_hash: &str) -> Result<(), RebornBuildError> {
    let marker = BundledSkillMarker {
        owner: BUNDLED_MARKER_OWNER.to_string(),
        format: 1,
        content_hash: content_hash.to_string(),
    };
    let marker_path = skill_dir.join(BUNDLED_MARKER_FILE);
    let bytes = serde_json::to_vec_pretty(&marker).map_err(invalid_config)?;
    fs::write(&marker_path, bytes).map_err(|error| {
        invalid_config(format!(
            "failed to write bundled skill marker {}: {error}",
            marker_path.display()
        ))
    })
}

fn validated_bundle_file_path(path: &str) -> Result<PathBuf, RebornBuildError> {
    let path = SkillFilePath::new(path)
        .map_err(|error| invalid_config(format!("invalid bundled skill file path: {error}")))?;
    Ok(Path::new(path.as_str()).to_path_buf())
}

fn bundled_skill_hash(skill: &EmbeddedRebornSkill) -> String {
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

    #[test]
    fn bundled_reborn_skills_include_current_repo_bundles_and_assets() {
        let dir = tempfile::tempdir().expect("tempdir");
        let local_dev_root = dir.path().join("local-dev");

        ensure_bundled_reborn_skills_installed(&local_dev_root).expect("install bundled skills");

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

    #[test]
    fn bundled_reborn_skills_do_not_overwrite_unmanaged_system_skills() {
        let dir = tempfile::tempdir().expect("tempdir");
        let local_dev_root = dir.path().join("local-dev");
        let skill_dir = local_dev_root.join("system/skills/code-review");
        fs::create_dir_all(&skill_dir).expect("mkdir");
        fs::write(skill_dir.join("SKILL.md"), "operator-owned").expect("write");

        ensure_bundled_reborn_skills_installed(&local_dev_root).expect("install bundled skills");

        assert_eq!(
            fs::read_to_string(skill_dir.join("SKILL.md")).expect("read"),
            "operator-owned"
        );
    }

    #[test]
    fn bundled_reborn_skills_skip_unchanged_managed_dirs() {
        let dir = tempfile::tempdir().expect("tempdir");
        let local_dev_root = dir.path().join("local-dev");
        let skill_md = local_dev_root.join("system/skills/code-review/SKILL.md");

        ensure_bundled_reborn_skills_installed(&local_dev_root).expect("install bundled skills");
        let first_modified = fs::metadata(&skill_md)
            .expect("metadata")
            .modified()
            .expect("modified");

        ensure_bundled_reborn_skills_installed(&local_dev_root).expect("install bundled skills");

        assert_eq!(
            fs::metadata(&skill_md)
                .expect("metadata")
                .modified()
                .expect("modified"),
            first_modified
        );
    }

    #[test]
    fn bundled_reborn_skills_replace_changed_managed_dirs() {
        let dir = tempfile::tempdir().expect("tempdir");
        let local_dev_root = dir.path().join("local-dev");
        let skill_dir = local_dev_root.join("system/skills/code-review");
        let skill_md = skill_dir.join("SKILL.md");

        ensure_bundled_reborn_skills_installed(&local_dev_root).expect("install bundled skills");
        let bundled_skill_md = fs::read_to_string(&skill_md).expect("read bundled skill");
        fs::write(&skill_md, "old managed skill").expect("write old skill");
        fs::write(skill_dir.join("OLD_SENTINEL"), "old").expect("write old sentinel");
        write_marker(&skill_dir, "stale-content-hash").expect("write stale marker");

        ensure_bundled_reborn_skills_installed(&local_dev_root).expect("replace bundled skills");

        assert_eq!(
            fs::read_to_string(&skill_md).expect("read replaced skill"),
            bundled_skill_md
        );
        assert!(!skill_dir.join("OLD_SENTINEL").exists());
        assert_no_bundle_scratch_dirs(&local_dev_root.join("system/skills"));
    }

    #[test]
    fn bundled_reborn_skills_remove_stale_managed_dirs() {
        let dir = tempfile::tempdir().expect("tempdir");
        let local_dev_root = dir.path().join("local-dev");
        let system_skills_root = local_dev_root.join("system/skills");
        let obsolete_dir = system_skills_root.join("obsolete-managed");
        let operator_dir = system_skills_root.join("operator-owned");
        fs::create_dir_all(&obsolete_dir).expect("obsolete dir");
        fs::write(obsolete_dir.join("SKILL.md"), "obsolete").expect("obsolete skill");
        write_marker(&obsolete_dir, "obsolete-hash").expect("obsolete marker");
        fs::create_dir_all(&operator_dir).expect("operator dir");
        fs::write(operator_dir.join("SKILL.md"), "operator").expect("operator skill");
        fs::write(
            operator_dir.join(BUNDLED_MARKER_FILE),
            r#"{"owner":"operator","format":1,"content_hash":"operator-hash"}"#,
        )
        .expect("operator marker");

        ensure_bundled_reborn_skills_installed(&local_dev_root).expect("install bundled skills");

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
}
