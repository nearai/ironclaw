use ironclaw_skills::{SkillManagementError, SkillManagementErrorKind};

use crate::{
    RebornBuildError,
    bundled_skills::bundled_reborn_skill_summaries,
    lifecycle::{RebornLocalSkillManagementError, build_existing_local_dev_skill_management_port},
};

pub async fn list_reborn_local_skills(
    owner_id: impl Into<String>,
    local_dev_storage_root: impl Into<std::path::PathBuf>,
) -> Result<Vec<ironclaw_skills::SkillSummary>, RebornSkillListError> {
    let mut skills =
        match build_existing_local_dev_skill_management_port(owner_id, local_dev_storage_root)? {
            Some(skill_management) => skill_management
                .list()
                .await
                .map_err(map_local_skill_management_error)?,
            None => Vec::new(),
        };
    let existing_names = skills
        .iter()
        .map(|skill| skill.name.clone())
        .collect::<std::collections::HashSet<_>>();
    skills.extend(
        bundled_reborn_skill_summaries()?
            .into_iter()
            .filter(|skill| !existing_names.contains(&skill.name)),
    );
    skills.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(skills)
}

#[derive(Debug, thiserror::Error)]
pub enum RebornSkillListError {
    #[error(transparent)]
    Build(#[from] RebornBuildError),
    #[error("skill list request rejected: {reason}")]
    InvalidRequest { reason: String },
    #[error("skill list access denied")]
    AccessDenied,
    #[error("skill list unavailable: {reason}")]
    Unavailable { reason: String },
}

fn map_local_skill_management_error(
    error: RebornLocalSkillManagementError,
) -> RebornSkillListError {
    match error {
        RebornLocalSkillManagementError::InvalidContext { reason } => {
            RebornSkillListError::InvalidRequest { reason }
        }
        RebornLocalSkillManagementError::Skill(error) => map_skill_management_error(error),
    }
}

fn map_skill_management_error(error: SkillManagementError) -> RebornSkillListError {
    match error.kind() {
        SkillManagementErrorKind::InvalidInput
        | SkillManagementErrorKind::NotFound
        | SkillManagementErrorKind::Conflict
        | SkillManagementErrorKind::InvalidSkill => RebornSkillListError::InvalidRequest {
            reason: error
                .reason()
                .unwrap_or("skill management request rejected")
                .to_string(),
        },
        SkillManagementErrorKind::FilesystemDenied => RebornSkillListError::AccessDenied,
        SkillManagementErrorKind::Resource => RebornSkillListError::Unavailable {
            reason: "skill management resource unavailable".to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_skills::ManagedSkillSource;

    #[tokio::test]
    async fn local_skill_list_lists_all_skills_from_reborn_storage() {
        let dir = tempfile::tempdir().expect("tempdir");
        let storage_root = dir.path().join("local-dev");
        for index in 0..55 {
            write_skill(&storage_root, &format!("list-skill-{index:02}"));
        }

        let result = list_reborn_local_skills("list-owner", &storage_root)
            .await
            .expect("list skills");

        assert!(result.iter().any(|skill| skill.name == "list-skill-54"));
        assert!(
            result
                .iter()
                .any(|skill| skill.name == "code-review"
                    && skill.source == ManagedSkillSource::System)
        );
        assert!(
            result
                .iter()
                .filter(|skill| skill.name.starts_with("list-skill-"))
                .all(|skill| skill.source == ManagedSkillSource::User)
        );
    }

    #[tokio::test]
    async fn local_skill_list_missing_storage_reports_bundled_without_creating_state() {
        let dir = tempfile::tempdir().expect("tempdir");
        let storage_root = dir.path().join("missing-local-dev");

        let result = list_reborn_local_skills("list-owner", &storage_root)
            .await
            .expect("list skills");

        assert!(
            result
                .iter()
                .any(|skill| skill.name == "code-review"
                    && skill.source == ManagedSkillSource::System)
        );
        assert!(!storage_root.exists());
    }

    #[tokio::test]
    async fn local_skill_list_rejects_non_directory_storage_root() {
        let dir = tempfile::tempdir().expect("tempdir");
        let storage_root = dir.path().join("local-dev");
        std::fs::write(&storage_root, "not a directory").expect("storage root file");

        let error = match list_reborn_local_skills("list-owner", &storage_root).await {
            Ok(_) => panic!("file storage root must fail"),
            Err(error) => error,
        };

        assert!(
            matches!(
                error,
                RebornSkillListError::Build(RebornBuildError::InvalidConfig { .. })
            ),
            "unexpected error: {error}"
        );
        assert!(
            error.to_string().contains("not a directory"),
            "unexpected error: {error}"
        );
    }

    #[tokio::test]
    async fn local_skill_list_rejects_invalid_owner_id() {
        let dir = tempfile::tempdir().expect("tempdir");
        let storage_root = dir.path().join("local-dev");
        std::fs::create_dir_all(&storage_root).expect("storage root");

        let error = match list_reborn_local_skills("list/owner", &storage_root).await {
            Ok(_) => panic!("invalid owner id must fail"),
            Err(error) => error,
        };

        assert!(
            matches!(
                error,
                RebornSkillListError::Build(RebornBuildError::InvalidConfig { .. })
            ),
            "unexpected error: {error}"
        );
        assert!(
            error.to_string().contains("slash") || error.to_string().contains("path"),
            "unexpected error: {error}"
        );
    }

    fn write_skill(storage_root: &std::path::Path, name: &str) {
        let skill_dir = storage_root.join("skills").join(name);
        std::fs::create_dir_all(&skill_dir).expect("skill dir");
        std::fs::write(
            skill_dir.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: list test\n---\nUse list.\n"),
        )
        .expect("skill file");
    }
}
