//! Engine-v2 project converter.
//!
//! Project state lived in v1 `memory_documents` under two user-visible/system
//! layouts. Both deserialize through the compatibility DTO and converge on the
//! canonical Reborn [`ProjectRepository`]. A repeated source/project id must be
//! exact; divergent source or target state fails without overwriting it.

use std::collections::BTreeMap;

use ironclaw_host_api::ProjectId;
use ironclaw_projects::{ProjectError, ProjectRecord, ProjectState};
use serde_json::json;

use crate::error::MigrationError;
use crate::options::MigrationOptions;
use crate::report::{Domain, LossReason, MigrationReport};
use crate::source::V1Source;
use crate::target::RebornTarget;
use crate::v2_model;

pub(crate) async fn run(
    source: &V1Source,
    target: &RebornTarget,
    options: &MigrationOptions,
    report: &mut MigrationReport,
) -> Result<(), MigrationError> {
    let mut projects: BTreeMap<String, (String, ProjectRecord)> = BTreeMap::new();

    for document in source.project_documents().await? {
        let Some(slug) = project_slug(&document.path) else {
            continue;
        };
        let source_id = format!("project:{}:{}", slug, document.path);
        let project = match serde_json::from_str::<v2_model::Project>(&document.content) {
            Ok(project) => project,
            Err(error) => {
                report.record_loss(
                    Domain::Project,
                    source_id,
                    "*",
                    LossReason::Unparseable,
                    format!("engine-v2 project JSON could not be parsed: {error}"),
                );
                continue;
            }
        };
        let owner_raw = if project.user_id.is_empty() {
            document.user_id.as_str()
        } else {
            project.user_id.as_str()
        };
        let Some(owner_user_id) =
            report.valid_user_id(Domain::Project, &source_id, "user_id", owner_raw)
        else {
            continue;
        };
        let project_id =
            ProjectId::new(project.id.to_string()).map_err(|error| MigrationError::ReadSource {
                domain: source_id.clone(),
                reason: format!("engine-v2 project UUID is not a valid Reborn ProjectId: {error}"),
            })?;
        let updated_at = project.updated_at.unwrap_or(project.created_at);
        let record = ProjectRecord {
            project_id: project_id.clone(),
            tenant_id: target.tenant_id.clone(),
            owner_user_id,
            name: project.name,
            description: project.description,
            icon: None,
            color: None,
            metadata: json!({
                "legacy_engine_v2": {
                    "goals": project.goals,
                    "metrics": project.metrics,
                    "metadata": project.metadata,
                    "workspace_path": project.workspace_path,
                }
            }),
            state: ProjectState::Active,
            created_at: project.created_at,
            updated_at,
        };
        if let Err(error) = record.validate() {
            report.record_loss(
                Domain::Project,
                source_id,
                "*",
                LossReason::Unparseable,
                format!("engine-v2 project cannot satisfy the Reborn project contract: {error}"),
            );
            continue;
        }

        let key = project_id.as_str().to_string();
        if let Some((existing_source, existing)) = projects.get(&key) {
            if existing != &record {
                return Err(MigrationError::WriteTarget {
                    domain: format!("project {key}"),
                    reason: format!(
                        "source documents {existing_source} and {} contain divergent state for the same project id",
                        document.path
                    ),
                });
            }
            continue;
        }
        projects.insert(key, (document.path, record));
    }

    for (source_id, record) in projects.into_values() {
        if !options.dry_run {
            compare_and_create(target, &source_id, record).await?;
        }
        report.stats.projects = report.stats.projects.saturating_add(1);
    }
    Ok(())
}

async fn compare_and_create(
    target: &RebornTarget,
    source_id: &str,
    record: ProjectRecord,
) -> Result<(), MigrationError> {
    if let Some(existing) = target
        .project_repo
        .get_project(&record.tenant_id, &record.project_id)
        .await
        .map_err(|error| project_write_error(source_id, "read deterministic target slot", error))?
    {
        return if existing == record {
            Ok(())
        } else {
            Err(project_conflict(source_id, &record.project_id))
        };
    }

    match target.project_repo.create_project(record.clone()).await {
        Ok(()) => Ok(()),
        Err(ProjectError::AlreadyExists) => {
            let existing = target
                .project_repo
                .get_project(&record.tenant_id, &record.project_id)
                .await
                .map_err(|error| {
                    project_write_error(source_id, "reconcile concurrent create", error)
                })?;
            match existing {
                Some(existing) if existing == record => Ok(()),
                Some(_) => Err(project_conflict(source_id, &record.project_id)),
                None => Err(MigrationError::WriteTarget {
                    domain: format!("project {source_id}"),
                    reason: "project vanished while reconciling a concurrent create".to_string(),
                }),
            }
        }
        Err(error) => Err(project_write_error(source_id, "create project", error)),
    }
}

fn project_conflict(source_id: &str, project_id: &ProjectId) -> MigrationError {
    MigrationError::WriteTarget {
        domain: format!("project {source_id}"),
        reason: format!(
            "project id {} already contains divergent state; refusing to overwrite",
            project_id.as_str()
        ),
    }
}

fn project_write_error(source_id: &str, operation: &str, error: ProjectError) -> MigrationError {
    MigrationError::WriteTarget {
        domain: format!("project {source_id}"),
        reason: format!("{operation}: {error}"),
    }
}

fn project_slug(path: &str) -> Option<&str> {
    let segments: Vec<_> = path.trim_matches('/').split('/').collect();
    match segments.as_slice() {
        ["projects", slug, ".project.json"] if !slug.is_empty() => Some(slug),
        [".system", "engine", "projects", slug, "project.json"] if !slug.is_empty() => Some(slug),
        ["engine", "projects", slug, "project.json"] if !slug.is_empty() => Some(slug),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::project_slug;

    #[test]
    fn recognizes_both_project_document_layouts() {
        assert_eq!(project_slug("projects/alpha/.project.json"), Some("alpha"));
        assert_eq!(
            project_slug(".system/engine/projects/beta/project.json"),
            Some("beta")
        );
        assert_eq!(
            project_slug("engine/projects/legacy/project.json"),
            Some("legacy")
        );
    }

    #[test]
    fn rejects_mission_and_near_match_paths() {
        assert_eq!(
            project_slug(".system/engine/projects/beta/missions/x/mission.json"),
            None
        );
        assert_eq!(project_slug("projects/alpha/project.json"), None);
        assert_eq!(project_slug("projects//.project.json"), None);
    }
}
