use std::collections::HashSet;

use ironclaw_skills::{ManagedSkillSource, SkillManagementError, SkillManagementErrorKind};

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
    let bundled_skills = bundled_reborn_skill_summaries()?;
    let bundled_names = bundled_skills
        .iter()
        .map(|skill| skill.name.clone())
        .collect::<HashSet<_>>();
    skills.retain(|skill| {
        !(skill.source == ManagedSkillSource::System && bundled_names.contains(&skill.name))
    });

    let existing_keys = skills
        .iter()
        .map(|skill| (skill.name.clone(), skill.source.as_str()))
        .collect::<HashSet<_>>();
    skills.extend(
        bundled_skills
            .into_iter()
            .filter(|skill| !existing_keys.contains(&(skill.name.clone(), skill.source.as_str()))),
    );
    skills.sort_by(|left, right| {
        left.name
            .cmp(&right.name)
            .then_with(|| left.source.as_str().cmp(right.source.as_str()))
    });
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

    #[tokio::test]
    async fn local_skill_list_prefers_user_skill_over_bundled_duplicate_name() {
        let dir = tempfile::tempdir().expect("tempdir");
        let storage_root = dir.path().join("local-dev");
        write_skill(&storage_root, "code-review");

        let result = list_reborn_local_skills("list-owner", &storage_root)
            .await
            .expect("list skills");

        let code_review_skills = result
            .iter()
            .filter(|skill| skill.name == "code-review")
            .collect::<Vec<_>>();
        assert_eq!(code_review_skills.len(), 2);
        assert!(
            code_review_skills
                .iter()
                .any(|skill| skill.source == ManagedSkillSource::User)
        );
        assert!(
            code_review_skills
                .iter()
                .any(|skill| skill.source == ManagedSkillSource::System)
        );

        let mut seen = std::collections::HashSet::new();
        for skill in result {
            assert!(
                seen.insert((skill.name.clone(), skill.source.as_str())),
                "duplicate skill entry for {} from {}",
                skill.name,
                skill.source.as_str()
            );
        }
    }

    #[tokio::test]
    async fn local_skill_list_prefers_embedded_bundled_summary_over_storage_system_skill() {
        let dir = tempfile::tempdir().expect("tempdir");
        let storage_root = dir.path().join("local-dev");
        write_system_skill(&storage_root, "code-review", "old system description");
        let bundled_code_review = bundled_reborn_skill_summaries()
            .expect("bundled summaries")
            .into_iter()
            .find(|skill| skill.name == "code-review")
            .expect("bundled code-review");

        let result = list_reborn_local_skills("list-owner", &storage_root)
            .await
            .expect("list skills");

        let system_code_reviews = result
            .iter()
            .filter(|skill| {
                skill.name == "code-review" && skill.source == ManagedSkillSource::System
            })
            .collect::<Vec<_>>();
        assert_eq!(system_code_reviews.len(), 1);
        assert_eq!(
            system_code_reviews[0].description,
            bundled_code_review.description
        );
        assert_ne!(system_code_reviews[0].description, "old system description");
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

    fn write_system_skill(storage_root: &std::path::Path, name: &str, description: &str) {
        let skill_dir = storage_root.join("system/skills").join(name);
        std::fs::create_dir_all(&skill_dir).expect("skill dir");
        std::fs::write(
            skill_dir.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: {description}\n---\nUse system.\n"),
        )
        .expect("skill file");
    }
}
