use std::{path::PathBuf, sync::Arc};

use ironclaw_filesystem::LocalFilesystem;
use ironclaw_host_api::{HostPath, UserId, VirtualPath};
use ironclaw_product_workflow::ProductWorkflowError;

use crate::{
    RebornBuildError, lifecycle::RebornLocalSkillManagementPort,
    local_dev_mounts::skill_management_mount_view,
};

#[derive(Clone)]
pub struct RebornLocalSkillCatalog {
    skill_management: Option<Arc<RebornLocalSkillManagementPort>>,
}

impl RebornLocalSkillCatalog {
    fn empty() -> Self {
        Self {
            skill_management: None,
        }
    }

    pub(crate) fn new(skill_management: Arc<RebornLocalSkillManagementPort>) -> Self {
        Self {
            skill_management: Some(skill_management),
        }
    }

    pub async fn list(&self) -> Result<RebornSkillListResult, RebornSkillCatalogError> {
        let Some(skill_management) = &self.skill_management else {
            return Ok(RebornSkillListResult::empty());
        };
        let skills = skill_management
            .list()
            .await
            .map_err(RebornSkillCatalogError::from)?
            .into_iter()
            .map(RebornSkillSummary::from)
            .collect::<Vec<_>>();
        Ok(RebornSkillListResult::new(skills))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebornSkillListResult {
    pub skills: Vec<RebornSkillSummary>,
    pub count: usize,
}

impl RebornSkillListResult {
    fn empty() -> Self {
        Self::new(Vec::new())
    }

    fn new(skills: Vec<RebornSkillSummary>) -> Self {
        let count = skills.len();
        Self { skills, count }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebornSkillSummary {
    pub name: String,
    pub version: String,
    pub description: String,
    pub source: RebornSkillSource,
    pub keywords: Vec<String>,
    pub tags: Vec<String>,
    pub requires_skills: Vec<String>,
}

impl From<ironclaw_skills::SkillSummary> for RebornSkillSummary {
    fn from(skill: ironclaw_skills::SkillSummary) -> Self {
        Self {
            name: skill.name,
            version: skill.version,
            description: skill.description,
            source: RebornSkillSource::from(skill.source),
            keywords: skill.keywords,
            tags: skill.tags,
            requires_skills: skill.requires_skills,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RebornSkillSource {
    System,
    User,
    Installed,
}

impl RebornSkillSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::User => "user",
            Self::Installed => "installed",
        }
    }
}

impl From<ironclaw_skills::ManagedSkillSource> for RebornSkillSource {
    fn from(source: ironclaw_skills::ManagedSkillSource) -> Self {
        match source {
            ironclaw_skills::ManagedSkillSource::System => Self::System,
            ironclaw_skills::ManagedSkillSource::User => Self::User,
            ironclaw_skills::ManagedSkillSource::Installed => Self::Installed,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RebornSkillCatalogError {
    #[error("skill catalog request rejected: {reason}")]
    InvalidRequest { reason: String },
    #[error("skill catalog access denied")]
    AccessDenied,
    #[error("skill catalog unavailable: {reason}")]
    Unavailable { reason: String },
}

impl From<ProductWorkflowError> for RebornSkillCatalogError {
    fn from(error: ProductWorkflowError) -> Self {
        match error {
            ProductWorkflowError::InvalidBindingRequest { reason } => {
                Self::InvalidRequest { reason }
            }
            ProductWorkflowError::BindingAccessDenied => Self::AccessDenied,
            ProductWorkflowError::Transient { reason } => Self::Unavailable { reason },
            other => Self::Unavailable {
                reason: other.to_string(),
            },
        }
    }
}

pub fn build_reborn_local_skill_catalog(
    owner_id: impl Into<String>,
    local_dev_storage_root: impl Into<PathBuf>,
) -> Result<RebornLocalSkillCatalog, RebornBuildError> {
    let owner_id = owner_id.into();
    let local_dev_storage_root = local_dev_storage_root.into();
    if !local_dev_storage_root
        .try_exists()
        .map_err(|error| RebornBuildError::InvalidConfig {
            reason: format!("local-dev skill storage root could not be inspected: {error}"),
        })?
    {
        return Ok(RebornLocalSkillCatalog::empty());
    }
    if !local_dev_storage_root.is_dir() {
        return Err(RebornBuildError::InvalidConfig {
            reason: "local-dev skill storage root is not a directory".to_string(),
        });
    }

    let mut filesystem = LocalFilesystem::new();
    filesystem.mount_local(
        VirtualPath::new("/projects")?,
        HostPath::from_path_buf(local_dev_storage_root),
    )?;
    let owner_user_id = UserId::new(owner_id).map_err(|error| RebornBuildError::InvalidConfig {
        reason: error.to_string(),
    })?;
    let skill_management = Arc::new(RebornLocalSkillManagementPort::new(
        owner_user_id,
        Arc::new(filesystem),
        skill_management_mount_view()?,
    ));
    Ok(RebornLocalSkillCatalog::new(skill_management))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn local_skill_catalog_lists_all_skills_from_reborn_storage() {
        let dir = tempfile::tempdir().expect("tempdir");
        let storage_root = dir.path().join("local-dev");
        for index in 0..55 {
            write_skill(&storage_root, &format!("catalog-skill-{index:02}"));
        }

        let catalog =
            build_reborn_local_skill_catalog("catalog-owner", &storage_root).expect("catalog");
        let result = catalog.list().await.expect("list skills");

        assert_eq!(result.count, 55);
        assert_eq!(result.skills.len(), 55);
        assert!(
            result
                .skills
                .iter()
                .any(|skill| skill.name == "catalog-skill-54")
        );
        assert!(
            result
                .skills
                .iter()
                .all(|skill| skill.source == RebornSkillSource::User)
        );
    }

    #[tokio::test]
    async fn local_skill_catalog_missing_storage_is_empty_without_creating_state() {
        let dir = tempfile::tempdir().expect("tempdir");
        let storage_root = dir.path().join("missing-local-dev");

        let catalog =
            build_reborn_local_skill_catalog("catalog-owner", &storage_root).expect("catalog");
        let result = catalog.list().await.expect("list skills");

        assert_eq!(result.count, 0);
        assert!(result.skills.is_empty());
        assert!(!storage_root.exists());
    }

    fn write_skill(storage_root: &std::path::Path, name: &str) {
        let skill_dir = storage_root.join("skills").join(name);
        std::fs::create_dir_all(&skill_dir).expect("skill dir");
        std::fs::write(
            skill_dir.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: catalog test\n---\nUse catalog.\n"),
        )
        .expect("skill file");
    }
}
