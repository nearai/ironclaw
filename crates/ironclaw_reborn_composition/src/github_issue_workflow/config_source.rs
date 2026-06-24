//! Project-metadata-backed config source and project-access checker for the
//! GitHub issue workflow.
//!
//! [`ProjectMetadataGithubIssueWorkflowConfigSource`] reads the workflow's
//! enabled repositories/labels from a project's metadata via the
//! [`ProjectService`]; [`ProjectServiceWorkflowProjectAccess`] enforces
//! repo-allowability, credential-fit, and project access. The `Empty` /
//! `Unconfigured` variants fail closed when no source is wired.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_github_issue_workflow::{
    GithubIssueCandidateSelector, GithubIssueWorkflowConfig, GithubIssueWorkflowConfigSource,
    GithubIssueWorkflowError, GithubProviderAccountRef, GithubRepositorySelector,
    WorkflowConfigAccessRequest, WorkflowProjectAccess, WorkflowProjectAccessRequest,
};
use ironclaw_host_api::{ProjectId, TenantId, UserId};
use ironclaw_product_workflow::{
    ProjectCaller, ProjectService, ProjectServiceError, RebornGetProjectRequest,
    RebornProjectResponse,
};
use serde::Deserialize;
use serde_json::Value as JsonValue;

const PROJECT_METADATA_GITHUB_ISSUE_WORKFLOW_KEY: &str = "github_issue_workflow";
const DEFAULT_GITHUB_ISSUE_WORKFLOW_RUN_PROFILE: &str = "default";

pub(crate) fn project_metadata_github_issue_workflow_config_source(
    project_service: Arc<dyn ProjectService>,
    tenant_id: TenantId,
    owner_user_id: UserId,
    project_id: ProjectId,
    configured_provider_account_ref: GithubProviderAccountRef,
) -> Arc<dyn GithubIssueWorkflowConfigSource> {
    Arc::new(ProjectMetadataGithubIssueWorkflowConfigSource {
        project_service,
        tenant_id,
        owner_user_id,
        project_id,
        configured_provider_account_ref,
    })
}

pub(crate) fn project_service_github_issue_workflow_project_access(
    project_service: Arc<dyn ProjectService>,
    configured_provider_account_ref: GithubProviderAccountRef,
) -> Arc<dyn WorkflowProjectAccess> {
    Arc::new(ProjectServiceWorkflowProjectAccess {
        project_service,
        configured_provider_account_ref,
    })
}

pub(super) struct ProjectMetadataGithubIssueWorkflowConfigSource {
    pub(super) project_service: Arc<dyn ProjectService>,
    pub(super) tenant_id: TenantId,
    pub(super) owner_user_id: UserId,
    pub(super) project_id: ProjectId,
    pub(super) configured_provider_account_ref: GithubProviderAccountRef,
}

#[async_trait]
impl GithubIssueWorkflowConfigSource for ProjectMetadataGithubIssueWorkflowConfigSource {
    async fn list_enabled_workflow_configs(
        &self,
    ) -> Result<Vec<GithubIssueWorkflowConfig>, GithubIssueWorkflowError> {
        let response = self
            .project_service
            .get_project(
                ProjectCaller {
                    tenant_id: self.tenant_id.clone(),
                    user_id: self.owner_user_id.clone(),
                },
                RebornGetProjectRequest {
                    project_id: self.project_id.to_string(),
                },
            )
            .await
            .map_err(project_service_error_to_workflow_error)?;

        let Some(section) = project_metadata_workflow_section(&response.project.metadata)? else {
            return Ok(Vec::new());
        };
        if !section.enabled {
            return Ok(Vec::new());
        }

        let repositories = section
            .repositories
            .unwrap_or_default()
            .into_iter()
            .map(|repository| GithubRepositorySelector::new(repository.owner, repository.repo))
            .collect::<Result<Vec<_>, _>>()?;
        let mut candidate_selector = GithubIssueCandidateSelector::default();
        if let Some(labels) = section.labels {
            candidate_selector.labels = labels;
        }
        if let Some(allowed_author_logins) = section.allowed_author_logins {
            candidate_selector.allowed_author_logins = allowed_author_logins;
        }
        let config = GithubIssueWorkflowConfig {
            tenant_id: self.tenant_id.clone(),
            project_id: self.project_id.clone(),
            owner_user_id: self.owner_user_id.clone(),
            repositories,
            candidate_selector,
            max_active_runs_per_repo: section.max_active_runs_per_repo.unwrap_or(1),
            default_run_profile: section
                .default_run_profile
                .unwrap_or_else(|| DEFAULT_GITHUB_ISSUE_WORKFLOW_RUN_PROFILE.to_string()),
            provider_account_ref: self.configured_provider_account_ref.clone(),
        };
        config.validate()?;
        Ok(vec![config])
    }
}

pub(super) struct ProjectServiceWorkflowProjectAccess {
    pub(super) project_service: Arc<dyn ProjectService>,
    /// The composition-bound GitHub credential account. This is the single
    /// source of truth for "the project's bound credential": project metadata
    /// is user-editable and deliberately does NOT carry an account ref (the
    /// config source rejects one — see
    /// `project_metadata_config_source_rejects_untrusted_fields`), so the bound
    /// account is supplied here from trusted composition config and the
    /// forwarded request account is checked against it.
    pub(super) configured_provider_account_ref: GithubProviderAccountRef,
}

#[async_trait]
impl WorkflowProjectAccess for ProjectServiceWorkflowProjectAccess {
    async fn assert_workflow_config_access(
        &self,
        request: WorkflowConfigAccessRequest,
    ) -> Result<(), GithubIssueWorkflowError> {
        // Repo-allowability + credential-fit: the poller forwards the validated
        // config's repositories + provider account ref. Fail LOUD up front if a
        // misconfigured run would point at a repo the project never declared, or
        // an account that is not the project's bound credential — otherwise the
        // run discovers issues, claims, and burns Triage -> Implementation before
        // failing at the first provider write.

        // Structural validation first (well-formedness), so the metadata
        // comparison below never has to reason about empty/garbage selectors.
        if request.repositories.is_empty() {
            return Err(GithubIssueWorkflowError::PolicyDenied {
                reason: "workflow config access denied: no repositories configured".to_string(),
            });
        }
        for selector in &request.repositories {
            selector
                .validate()
                .map_err(|error| GithubIssueWorkflowError::PolicyDenied {
                    reason: format!(
                        "workflow config access denied: repository selector is not allowable: {error}"
                    ),
                })?;
        }
        request.provider_account_ref.validate().map_err(|error| {
            GithubIssueWorkflowError::PolicyDenied {
                reason: format!(
                    "workflow config access denied: provider account ref is invalid: {error}"
                ),
            }
        })?;

        // Credential-fit: the forwarded account must be the project's bound
        // GitHub credential account (composition config — project metadata does
        // not carry the account). A mismatch is the "wrong account id" footgun.
        if request.provider_account_ref != self.configured_provider_account_ref {
            return Err(GithubIssueWorkflowError::PolicyDenied {
                reason:
                    "workflow config access denied: configured provider account is not the project's bound credential account"
                        .to_string(),
            });
        }

        // Project access AND repo-allowability are both derived from the SAME
        // trusted `get_project` read: confirm the caller can reach the project,
        // then confirm every configured repo is within the project's declared
        // `github_issue_workflow.repositories` selector.
        let project = self
            .read_project(
                request.tenant_id,
                request.creator_user_id,
                request.project_id,
            )
            .await?;
        let declared = declared_repository_selectors(&project.project.metadata)?;
        for selector in &request.repositories {
            let allowed = declared
                .iter()
                .any(|repo| repo.owner == selector.owner && repo.repo == selector.repo);
            if !allowed {
                return Err(GithubIssueWorkflowError::PolicyDenied {
                    reason: format!(
                        "workflow config access denied: repository {}/{} is not in the project's declared repositories",
                        selector.owner, selector.repo
                    ),
                });
            }
        }
        Ok(())
    }

    async fn assert_workflow_project_access(
        &self,
        request: WorkflowProjectAccessRequest,
    ) -> Result<(), GithubIssueWorkflowError> {
        let Some(project_id) = request.project_id else {
            return Err(GithubIssueWorkflowError::PolicyDenied {
                reason: "workflow run has no project scope".to_string(),
            });
        };
        self.assert_project_access(request.tenant_id, request.creator_user_id, project_id)
            .await
    }
}

impl ProjectServiceWorkflowProjectAccess {
    async fn assert_project_access(
        &self,
        tenant_id: TenantId,
        creator_user_id: UserId,
        project_id: ProjectId,
    ) -> Result<(), GithubIssueWorkflowError> {
        self.read_project(tenant_id, creator_user_id, project_id)
            .await
            .map(|_| ())
    }

    /// Fetch the project through the trusted [`ProjectService`], mapping any
    /// access/backend failure to a sanitized workflow error (no host paths).
    /// Returns the full response so repo-allowability can be derived from the
    /// same read that proves project access.
    async fn read_project(
        &self,
        tenant_id: TenantId,
        creator_user_id: UserId,
        project_id: ProjectId,
    ) -> Result<RebornProjectResponse, GithubIssueWorkflowError> {
        self.project_service
            .get_project(
                ProjectCaller {
                    tenant_id,
                    user_id: creator_user_id,
                },
                RebornGetProjectRequest {
                    project_id: project_id.to_string(),
                },
            )
            .await
            .map_err(project_service_error_to_workflow_error)
    }
}

/// The repositories a project has DECLARED as allowable for the GitHub issue
/// workflow, derived from its `github_issue_workflow` metadata section. A
/// missing/disabled section yields an empty set, which fails every
/// repo-allowability check closed (no declaration => nothing is allowable).
fn declared_repository_selectors(
    metadata: &JsonValue,
) -> Result<Vec<ProjectMetadataGithubRepositorySelector>, GithubIssueWorkflowError> {
    let Some(section) = project_metadata_workflow_section(metadata)? else {
        return Ok(Vec::new());
    };
    if !section.enabled {
        return Ok(Vec::new());
    }
    Ok(section.repositories.unwrap_or_default())
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ProjectMetadataGithubIssueWorkflowSection {
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    repositories: Option<Vec<ProjectMetadataGithubRepositorySelector>>,
    #[serde(default)]
    labels: Option<Vec<String>>,
    #[serde(default)]
    allowed_author_logins: Option<Vec<String>>,
    #[serde(default)]
    max_active_runs_per_repo: Option<u32>,
    #[serde(default)]
    default_run_profile: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ProjectMetadataGithubRepositorySelector {
    owner: String,
    repo: String,
}

pub(super) fn project_metadata_workflow_section(
    metadata: &JsonValue,
) -> Result<Option<ProjectMetadataGithubIssueWorkflowSection>, GithubIssueWorkflowError> {
    let Some(metadata_object) = metadata.as_object() else {
        if metadata.is_null() {
            return Ok(None);
        }
        return Err(GithubIssueWorkflowError::InvalidConfig {
            reason: "project metadata must be an object or null".to_string(),
        });
    };
    let Some(section) = metadata_object.get(PROJECT_METADATA_GITHUB_ISSUE_WORKFLOW_KEY) else {
        return Ok(None);
    };
    if section.is_null() {
        return Ok(None);
    }
    serde_json::from_value(section.clone())
        .map(Some)
        .map_err(|error| GithubIssueWorkflowError::InvalidConfig {
            reason: format!(
                "project metadata `{PROJECT_METADATA_GITHUB_ISSUE_WORKFLOW_KEY}` is invalid: {error}"
            ),
        })
}

pub(super) fn project_service_error_to_workflow_error(
    error: ProjectServiceError,
) -> GithubIssueWorkflowError {
    match error {
        ProjectServiceError::NotFound | ProjectServiceError::Denied => {
            GithubIssueWorkflowError::PolicyDenied {
                reason: "workflow project is not accessible".to_string(),
            }
        }
        ProjectServiceError::InvalidInput { field } => GithubIssueWorkflowError::InvalidConfig {
            reason: format!("workflow project reference is invalid: {field}"),
        },
        ProjectServiceError::Conflict => GithubIssueWorkflowError::Repository {
            reason: "workflow project service reported a conflict".to_string(),
        },
        ProjectServiceError::Unavailable => GithubIssueWorkflowError::Repository {
            reason: "workflow project service is unavailable".to_string(),
        },
        ProjectServiceError::Internal => GithubIssueWorkflowError::Repository {
            reason: "workflow project service returned an internal error".to_string(),
        },
    }
}

pub(super) struct EmptyGithubIssueWorkflowConfigSource;

#[async_trait]
impl GithubIssueWorkflowConfigSource for EmptyGithubIssueWorkflowConfigSource {
    async fn list_enabled_workflow_configs(
        &self,
    ) -> Result<Vec<GithubIssueWorkflowConfig>, GithubIssueWorkflowError> {
        Ok(Vec::new())
    }
}

pub(super) struct UnconfiguredWorkflowProjectAccess;

#[async_trait]
impl WorkflowProjectAccess for UnconfiguredWorkflowProjectAccess {
    async fn assert_workflow_config_access(
        &self,
        _request: WorkflowConfigAccessRequest,
    ) -> Result<(), GithubIssueWorkflowError> {
        Err(GithubIssueWorkflowError::PolicyDenied {
            reason: "GitHub issue workflow project access checker is not configured".to_string(),
        })
    }

    async fn assert_workflow_project_access(
        &self,
        _request: WorkflowProjectAccessRequest,
    ) -> Result<(), GithubIssueWorkflowError> {
        Err(GithubIssueWorkflowError::PolicyDenied {
            reason: "GitHub issue workflow project access checker is not configured".to_string(),
        })
    }
}
