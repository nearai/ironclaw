use std::time::Duration;

use ironclaw_host_api::{ProjectId, TenantId, UserId};
use serde::{Deserialize, Serialize};

use crate::GithubIssueWorkflowError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GithubIssueWorkflowConfig {
    pub tenant_id: TenantId,
    pub project_id: ProjectId,
    pub owner_user_id: UserId,
    pub repositories: Vec<GithubRepositorySelector>,
    pub candidate_selector: GithubIssueCandidateSelector,
    pub max_active_runs_per_repo: u32,
    pub default_run_profile: String,
    pub provider_account_ref: GithubProviderAccountRef,
}

impl GithubIssueWorkflowConfig {
    pub fn validate(&self) -> Result<(), GithubIssueWorkflowError> {
        if self.repositories.is_empty() {
            return Err(GithubIssueWorkflowError::InvalidConfig {
                reason: "repositories must not be empty".to_string(),
            });
        }
        for repository in &self.repositories {
            repository.validate()?;
        }
        self.candidate_selector.validate()?;
        if self.max_active_runs_per_repo == 0 {
            return Err(GithubIssueWorkflowError::InvalidConfig {
                reason: "max_active_runs_per_repo must be greater than zero".to_string(),
            });
        }
        validate_non_empty("default_run_profile", &self.default_run_profile)?;
        self.provider_account_ref.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GithubRepositorySelector {
    pub owner: String,
    pub repo: String,
}

impl GithubRepositorySelector {
    pub fn new(
        owner: impl Into<String>,
        repo: impl Into<String>,
    ) -> Result<Self, GithubIssueWorkflowError> {
        let selector = Self {
            owner: owner.into(),
            repo: repo.into(),
        };
        selector.validate()?;
        Ok(selector)
    }

    pub fn validate(&self) -> Result<(), GithubIssueWorkflowError> {
        validate_non_empty("repository owner", &self.owner)?;
        validate_non_empty("repository name", &self.repo)?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GithubIssueCandidateSelector {
    pub labels: Vec<String>,
    pub allowed_author_logins: Vec<String>,
}

impl GithubIssueCandidateSelector {
    pub fn validate(&self) -> Result<(), GithubIssueWorkflowError> {
        for label in &self.labels {
            validate_non_empty("candidate label", label)?;
        }
        for login in &self.allowed_author_logins {
            validate_non_empty("allowed author login", login)?;
        }
        Ok(())
    }

    pub fn allows_author_login(&self, author_login: Option<&str>) -> bool {
        if self.allowed_author_logins.is_empty() {
            return true;
        }

        let Some(author_login) = author_login
            .map(str::trim)
            .filter(|login| !login.is_empty())
        else {
            return false;
        };

        self.allowed_author_logins
            .iter()
            .any(|allowed_login| allowed_login.trim().eq_ignore_ascii_case(author_login))
    }
}

impl Default for GithubIssueCandidateSelector {
    fn default() -> Self {
        Self {
            labels: vec!["bug".to_string()],
            allowed_author_logins: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GithubProviderAccountRef {
    pub provider: String,
    pub account_id: String,
}

impl GithubProviderAccountRef {
    pub fn validate(&self) -> Result<(), GithubIssueWorkflowError> {
        validate_non_empty("provider", &self.provider)?;
        validate_non_empty("provider account", &self.account_id)?;
        Ok(())
    }
}

fn validate_non_empty(name: &'static str, value: &str) -> Result<(), GithubIssueWorkflowError> {
    if value.trim().is_empty() {
        return Err(GithubIssueWorkflowError::InvalidConfig {
            reason: format!("{name} must not be empty"),
        });
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GithubIssueWorkflowPollerConfig {
    pub enabled: bool,
    pub poll_interval: Duration,
    pub max_repos_per_tick: usize,
    pub max_issues_per_repo_per_tick: usize,
    pub max_runnable_runs_per_tick: usize,
    pub lease_duration: Duration,
}

impl Default for GithubIssueWorkflowPollerConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            poll_interval: Duration::from_secs(60),
            max_repos_per_tick: 20,
            max_issues_per_repo_per_tick: 10,
            max_runnable_runs_per_tick: 10,
            lease_duration: Duration::from_secs(300),
        }
    }
}
