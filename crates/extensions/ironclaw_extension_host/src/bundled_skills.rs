use std::collections::HashSet;
use std::hash::Hasher;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use ironclaw_filesystem::{CasExpectation, Entry, FileType, FilesystemError, RootFilesystem};
use ironclaw_host_api::path::VirtualPath;
use ironclaw_loop_host::SkillFilePath;
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
const BUNDLED_MARKER_OWNER: &str = "ironclaw_composition_bundled_skill";
const BUNDLED_INSTALL_LOCK_TIMEOUT: Duration = Duration::from_secs(30);
const BUNDLED_INSTALL_LOCK_RETRY: Duration = Duration::from_millis(25);
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

/// Install the bundled skills into ANY skill root, on any filesystem backend.
///
/// Assembly owns filesystem construction, host-path containment, and mount selection. The marker,
/// install lock, and stale-skill removal stay idempotent across boots and safe when several instances
/// share one database.
pub async fn ensure_bundled_reborn_skills_installed_in(
    filesystem: &dyn RootFilesystem,
    system_skills_root: &VirtualPath,
) -> Result<(), RebornBuildError> {
    let bundled_skills = embedded_reborn_skill_bundles()?;
    let install_lock = BundledSkillInstallLock::acquire(filesystem, system_skills_root).await?;
    let result = async {
        let bundled_names = bundled_skills
            .iter()
            .map(|skill| skill.name.as_str())
            .collect::<HashSet<_>>();
        remove_stale_managed_skills(filesystem, system_skills_root, &bundled_names).await?;

        for skill in bundled_skills {
            install_bundled_skill(filesystem, system_skills_root, skill).await?;
        }
        Ok(())
    }
    .await;

    let release_result = install_lock.release(filesystem).await;
    match (result, release_result) {
        (Err(error), _) => Err(error),
        (Ok(()), Err(error)) => Err(error),
        (Ok(()), Ok(())) => Ok(()),
    }
}

pub fn bundled_reborn_skill_summaries() -> Result<Vec<SkillSummary>, RebornBuildError> {
    // Which bundled skills ship a `scripts/` directory, read from the embedded bundles rather than
    // assumed. Reporting `has_scripts: false` for any such bundle would tell the Skills page it is
    // prose-only.
    let skills_with_scripts = embedded_reborn_skill_bundles()?
        .into_iter()
        .filter(|bundle| {
            bundle
                .files
                .iter()
                .any(|file| file.path.starts_with("scripts/") || file.path.contains("/scripts/"))
        })
        .map(|bundle| bundle.name)
        .collect::<HashSet<_>>();
    Ok(embedded_reborn_skill_summaries()?
        .into_iter()
        .map(|skill| SkillSummary {
            has_scripts: skills_with_scripts.contains(&skill.name),
            name: skill.name,
            version: skill.version,
            description: skill.description,
            source: ManagedSkillSource::System,
            keywords: skill.keywords,
            tags: skill.tags,
            requires_skills: skill.requires_skills,
            auto_activate: true,
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
    use ironclaw_filesystem::InMemoryBackend;

    use super::*;

    const ARCHIVED_RUNTIME_SKILLS: &[&str] = &[
        "ceo-setup",
        "code-review",
        "commitment-setup",
        "content-creator-setup",
        "developer-setup",
        "github",
        "github-workflow",
        "linear",
        "llm-council",
        "local-test",
        "new-project",
        "parallel-pr-review",
        "plan-mode",
        "portfolio",
        "project-setup",
        "trader-setup",
        "web-ui-test",
    ];

    #[test]
    fn archived_runtime_skills_are_preserved_but_not_bundled() {
        let bundled_names = embedded_reborn_skill_bundles()
            .expect("parse embedded skills")
            .into_iter()
            .map(|skill| skill.name)
            .collect::<HashSet<_>>();
        let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .find(|path| path.join("Cargo.lock").is_file() && path.join("skills").is_dir())
            .expect("repository root");

        for name in ARCHIVED_RUNTIME_SKILLS {
            assert!(
                !bundled_names.contains(*name),
                "archived runtime skill {name} must not be model-triggerable"
            );
            assert!(
                repo_root
                    .join("docs/internal/archived-skills")
                    .join(name)
                    .join("SKILL.md")
                    .is_file(),
                "archived runtime skill {name} must remain available for parity restoration"
            );
        }
    }
    /// Zero-legacy gate for embedded skill guidance: the Reborn binary embeds
    /// the repo `skills/` directory, so a skill teaching the retired v1
    /// automation tools (`routine_create` / `routine_list`) misdirects every
    /// Reborn automation conversation its keywords match. The automation
    /// advisor must teach the Reborn capability surface instead.
    ///
    /// Delivery is explicit-prompt-step-only now (`builtin__outbound_deliver`,
    /// per `crates/contracts/ironclaw_loop_contracts/prompts/delivery.md`): the skill must not
    /// resurrect the retired `delivery_target_id` routing field or claim
    /// external delivery happens automatically to a stored target.
    #[test]
    fn embedded_skills_teach_reborn_trigger_tools_not_retired_v1_routines() {
        let bundles = embedded_reborn_skill_bundles().expect("embedded bundles parse");
        let mut routine_advisor_skill_md = None;
        for bundle in &bundles {
            for file in &bundle.files {
                let Ok(content) = std::str::from_utf8(&file.bytes) else {
                    continue;
                };
                assert!(
                    !content.contains("routine_create") && !content.contains("routine_list"),
                    "embedded skill `{}` file `{}` references retired v1 routine tools",
                    bundle.name,
                    file.path
                );
                if bundle.name == "routine-advisor" && file.path == "SKILL.md" {
                    routine_advisor_skill_md = Some(content.to_string());
                }
            }
        }
        let skill_md = routine_advisor_skill_md.expect("routine-advisor SKILL.md is embedded");
        assert!(
            skill_md.contains("builtin__trigger_create"),
            "routine-advisor must teach the Reborn trigger_create capability"
        );
        assert!(
            skill_md.contains("builtin__outbound_delivery_targets_list"),
            "routine-advisor must teach delivery-target selection"
        );
        assert!(
            // Not `contains("builtin__outbound_deliver")` alone: that's a literal
            // prefix of `builtin__outbound_delivery_targets_list`, already proven
            // present above, so that check alone can never fail. Assert the
            // call-site phrase instead.
            skill_md.contains("call `builtin__outbound_deliver`"),
            "routine-advisor must teach the explicit outbound-delivery tool"
        );
        assert!(
            skill_md.contains("delivery as an explicit prompt step"),
            "routine-advisor must frame delivery as an explicit prompt-authored step, not a \
             stored routing target"
        );
        assert!(
            skill_md.contains("delivers nothing externally"),
            "routine-advisor must state that a fire with no delivery call delivers nothing \
             externally (successor to the dropped 'delivery routing, not a task step' pin)"
        );
        assert!(
            skill_md.contains("builtin__notification_channels_set"),
            "routine-advisor must teach the background-run notification channel tool"
        );
        assert!(
            !skill_md.contains("delivery_target_id"),
            "routine-advisor must not resurrect the retired delivery_target_id parameter"
        );
        assert!(
            !skill_md.contains("delivered automatically"),
            "routine-advisor must not claim external delivery happens automatically"
        );
    }

    #[tokio::test]
    async fn bundled_reborn_skills_include_current_repo_bundles() {
        let filesystem = InMemoryBackend::new();
        let system_skills_root = test_system_skills_root();

        ensure_bundled_reborn_skills_installed_in(&filesystem, &system_skills_root)
            .await
            .expect("install bundled skills");

        assert!(
            filesystem
                .stat(&test_skill_path("coding/SKILL.md"))
                .await
                .is_ok()
        );
        assert!(
            filesystem
                .stat(&test_skill_path("routine-advisor/SKILL.md"))
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn exact_system_skills_root_does_not_append_the_system_namespace() {
        let filesystem = InMemoryBackend::new();
        let system_skills_root =
            VirtualPath::new("/system/skills/exact-system-skills").expect("valid root");

        ensure_bundled_reborn_skills_installed_in(&filesystem, &system_skills_root)
            .await
            .expect("install bundled skills at the exact system skills root");

        assert!(
            filesystem
                .stat(
                    &VirtualPath::new("/system/skills/exact-system-skills/coding/SKILL.md")
                        .expect("valid skill path"),
                )
                .await
                .is_ok()
        );
        for appended_namespace in ["system", "skills"] {
            assert!(
                matches!(
                    filesystem
                        .stat(
                            &VirtualPath::new(format!(
                                "/system/skills/exact-system-skills/{appended_namespace}"
                            ))
                            .expect("valid path"),
                        )
                        .await,
                    Err(FilesystemError::NotFound { .. })
                ),
                "an exact system skills root must not receive another {appended_namespace} namespace"
            );
        }
    }

    #[tokio::test]
    async fn bundled_reborn_skills_do_not_overwrite_unmanaged_system_skills() {
        let filesystem = InMemoryBackend::new();
        let system_skills_root = test_system_skills_root();
        let skill_md = test_skill_path("coding/SKILL.md");
        filesystem
            .put(
                &skill_md,
                Entry::bytes(b"operator-owned".to_vec()),
                CasExpectation::Any,
            )
            .await
            .expect("write operator skill");

        ensure_bundled_reborn_skills_installed_in(&filesystem, &system_skills_root)
            .await
            .expect("install bundled skills");

        assert_eq!(
            bundled_skill_file(&filesystem, "coding/SKILL.md").await,
            b"operator-owned"
        );
    }

    #[tokio::test]
    async fn bundled_reborn_skills_skip_unchanged_managed_dirs() {
        let filesystem = InMemoryBackend::new();
        let system_skills_root = test_system_skills_root();
        let skill_md = test_skill_path("coding/SKILL.md");

        ensure_bundled_reborn_skills_installed_in(&filesystem, &system_skills_root)
            .await
            .expect("install bundled skills");
        let first_version = filesystem
            .get(&skill_md)
            .await
            .expect("read skill")
            .expect("bundled skill exists")
            .version;

        ensure_bundled_reborn_skills_installed_in(&filesystem, &system_skills_root)
            .await
            .expect("install bundled skills");

        assert_eq!(
            filesystem
                .get(&skill_md)
                .await
                .expect("read skill")
                .expect("bundled skill exists")
                .version,
            first_version
        );
    }

    #[tokio::test]
    async fn bundled_reborn_skills_replace_changed_managed_dirs() {
        let filesystem = InMemoryBackend::new();
        let system_skills_root = test_system_skills_root();
        let skill_dir = test_skill_path("coding");
        let skill_md = test_skill_path("coding/SKILL.md");

        ensure_bundled_reborn_skills_installed_in(&filesystem, &system_skills_root)
            .await
            .expect("install bundled skills");
        let bundled_skill_md = bundled_skill_file(&filesystem, "coding/SKILL.md").await;
        filesystem
            .put(
                &skill_md,
                Entry::bytes(b"old managed skill".to_vec()),
                CasExpectation::Any,
            )
            .await
            .expect("write old skill");
        filesystem
            .put(
                &test_skill_path("coding/OLD_SENTINEL"),
                Entry::bytes(b"old".to_vec()),
                CasExpectation::Any,
            )
            .await
            .expect("write old sentinel");
        write_marker_file(&filesystem, &skill_dir, "stale-content-hash").await;

        ensure_bundled_reborn_skills_installed_in(&filesystem, &system_skills_root)
            .await
            .expect("replace bundled skills");

        assert_eq!(
            bundled_skill_file(&filesystem, "coding/SKILL.md").await,
            bundled_skill_md
        );
        assert!(matches!(
            filesystem
                .stat(&test_skill_path("coding/OLD_SENTINEL"))
                .await,
            Err(FilesystemError::NotFound { .. })
        ));
        assert_no_bundle_scratch_dirs(&filesystem, &system_skills_root).await;
    }

    #[tokio::test]
    async fn bundled_reborn_skills_remove_stale_managed_dirs() {
        let filesystem = InMemoryBackend::new();
        let system_skills_root = test_system_skills_root();
        let obsolete_dir = test_skill_path("obsolete-managed");
        filesystem
            .put(
                &test_skill_path("obsolete-managed/SKILL.md"),
                Entry::bytes(b"obsolete".to_vec()),
                CasExpectation::Any,
            )
            .await
            .expect("obsolete skill");
        write_marker_file(&filesystem, &obsolete_dir, "obsolete-hash").await;
        filesystem
            .put(
                &test_skill_path("operator-owned/SKILL.md"),
                Entry::bytes(b"operator".to_vec()),
                CasExpectation::Any,
            )
            .await
            .expect("operator skill");
        filesystem
            .put(
                &test_skill_path(&format!("operator-owned/{BUNDLED_MARKER_FILE}")),
                Entry::bytes(
                    br#"{"owner":"operator","format":1,"content_hash":"operator-hash"}"#.to_vec(),
                ),
                CasExpectation::Any,
            )
            .await
            .expect("operator marker");

        ensure_bundled_reborn_skills_installed_in(&filesystem, &system_skills_root)
            .await
            .expect("install bundled skills");

        assert!(matches!(
            filesystem.stat(&obsolete_dir).await,
            Err(FilesystemError::NotFound { .. })
        ));
        assert!(
            filesystem
                .stat(&test_skill_path("operator-owned/SKILL.md"))
                .await
                .is_ok()
        );
    }

    fn test_system_skills_root() -> VirtualPath {
        VirtualPath::new("/system/skills").expect("valid system skills root")
    }

    fn test_skill_path(relative: &str) -> VirtualPath {
        VirtualPath::new(format!("/system/skills/{relative}")).expect("valid system skill path")
    }

    async fn bundled_skill_file(filesystem: &InMemoryBackend, relative: &str) -> Vec<u8> {
        filesystem
            .get(&test_skill_path(relative))
            .await
            .expect("read bundled skill")
            .expect("bundled skill exists")
            .entry
            .body
    }

    async fn assert_no_bundle_scratch_dirs(
        filesystem: &InMemoryBackend,
        system_skills_root: &VirtualPath,
    ) {
        for entry in filesystem
            .list_dir(system_skills_root)
            .await
            .expect("read system skills")
        {
            let name = entry.name;
            assert!(
                !name.contains(".tmp-") && !name.contains(".previous-"),
                "unexpected bundled skill scratch dir: {name}"
            );
        }
    }

    async fn write_marker_file(
        filesystem: &InMemoryBackend,
        skill_dir: &VirtualPath,
        content_hash: &str,
    ) {
        let marker = BundledSkillMarker {
            owner: BUNDLED_MARKER_OWNER.to_string(),
            format: 1,
            content_hash: content_hash.to_string(),
        };
        let bytes = serde_json::to_vec_pretty(&marker).expect("marker json");
        filesystem
            .put(
                &child_path(skill_dir, BUNDLED_MARKER_FILE).expect("valid marker path"),
                Entry::bytes(bytes),
                CasExpectation::Any,
            )
            .await
            .expect("write marker");
    }
}
