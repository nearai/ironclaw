use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use ironclaw_host_api::{ProjectId, TenantId, UserId};
use serde::{Deserialize, Serialize};
use serde_json::{Value as JsonValue, json};

use crate::{
    BlockWorkflowRunInput, ClaimRunnableWorkflowRunsInput, CreateOrGetWorkflowRunInput,
    CreateOrGetWorkflowRunOutcome, FindLatestWorkflowEventForProviderInput, GetGithubIssueInput,
    GithubIssueBlockKind, GithubIssueBlockState, GithubIssueProviderSnapshot,
    GithubIssueWorkflowConfig, GithubIssueWorkflowConfigSource, GithubIssueWorkflowError,
    GithubIssueWorkflowEventType, GithubIssueWorkflowPolicy, GithubIssueWorkflowPolicyPorts,
    GithubIssueWorkflowPollerConfig, GithubIssueWorkflowPort, GithubIssueWorkflowRepository,
    GithubProviderRef, GithubRepositorySelector, LeaseReleaseOutcome, ListIssueCommentsInput,
    RecordWorkflowEventInput, RecordWorkflowEventOutcome, ReleaseWorkflowRunLeaseInput,
    SearchGithubIssuesInput, StageTurnSubmitter, WorkflowClock, WorkflowConfigAccessRequest,
    WorkflowEventEnvelope, WorkflowEventSourceKind, WorkflowProjectAccess, WorkflowWorkerId,
    WorkflowWorkspaceManager, issue_binding_ref, issue_changed_key, issue_discovered_key,
};

const DEFAULT_WORKFLOW_POLICY_KEY: &str = "github-bug-workflow";

pub trait GithubIssueWorkflowPollerPorts: Send + Sync {
    type Clock: WorkflowClock + ?Sized;
    type ConfigSource: GithubIssueWorkflowConfigSource + ?Sized;
    type GithubPort: GithubIssueWorkflowPort + ?Sized;
    type ProjectAccess: WorkflowProjectAccess + ?Sized;
    type Repository: GithubIssueWorkflowRepository + ?Sized;
    type StageTurnSubmitter: StageTurnSubmitter + ?Sized;
    type WorkspaceManager: WorkflowWorkspaceManager + ?Sized;

    fn clock(&self) -> Arc<Self::Clock>;
    fn config_source(&self) -> Arc<Self::ConfigSource>;
    fn github_port(&self) -> Arc<Self::GithubPort>;
    fn project_access(&self) -> Arc<Self::ProjectAccess>;
    fn repository(&self) -> Arc<Self::Repository>;
    fn stage_turn_submitter(&self) -> Arc<Self::StageTurnSubmitter>;
    fn workspace_manager(&self) -> Arc<Self::WorkspaceManager>;
    fn worker_id(&self) -> WorkflowWorkerId;
}

#[derive(Debug)]
pub struct GithubIssueWorkflowPoller<P> {
    ports: P,
    config: GithubIssueWorkflowPollerConfig,
    workflow_policy_key: String,
    workflow_policy_version: String,
}

impl<P> GithubIssueWorkflowPoller<P>
where
    P: GithubIssueWorkflowPollerPorts,
{
    pub fn new(
        ports: P,
        config: GithubIssueWorkflowPollerConfig,
        workflow_policy_version: impl Into<String>,
    ) -> Self {
        Self {
            ports,
            config,
            workflow_policy_key: DEFAULT_WORKFLOW_POLICY_KEY.to_string(),
            workflow_policy_version: workflow_policy_version.into(),
        }
    }

    pub fn ports(&self) -> &P {
        &self.ports
    }

    pub async fn tick_once(
        &self,
    ) -> Result<GithubIssueWorkflowPollerTickOutcome, GithubIssueWorkflowError> {
        let mut outcome = GithubIssueWorkflowPollerTickOutcome::default();
        if !self.config.enabled {
            outcome.disabled = true;
            return Ok(outcome);
        }

        let configs = self
            .ports
            .config_source()
            .list_enabled_workflow_configs()
            .await?;
        outcome.configs_loaded = configs.len();

        let mut repos_remaining = self.config.max_repos_per_tick;
        let mut claim_tenants = Vec::new();
        for workflow_config in configs {
            workflow_config.validate()?;
            if repos_remaining == 0 {
                break;
            }
            if let Err(error) = self.assert_config_access(&workflow_config).await {
                outcome
                    .blocked_configs
                    .push(blocked_config(&workflow_config, None, &error));
                continue;
            }

            let mut config_blocked = false;
            for repository in &workflow_config.repositories {
                if repos_remaining == 0 {
                    break;
                }
                repos_remaining -= 1;
                if let Err(error) = self
                    .discover_repository(&workflow_config, repository, &mut outcome)
                    .await
                {
                    outcome.blocked_configs.push(blocked_config(
                        &workflow_config,
                        Some(repository),
                        &error,
                    ));
                    config_blocked = true;
                    break;
                }
            }

            if !config_blocked
                && !claim_tenants
                    .iter()
                    .any(|tenant_id| tenant_id == &workflow_config.tenant_id)
            {
                claim_tenants.push(workflow_config.tenant_id);
            }
        }

        self.tick_runnable_runs(&claim_tenants, &mut outcome)
            .await?;
        Ok(outcome)
    }

    async fn assert_config_access(
        &self,
        workflow_config: &GithubIssueWorkflowConfig,
    ) -> Result<(), GithubIssueWorkflowError> {
        self.ports
            .project_access()
            .assert_workflow_config_access(WorkflowConfigAccessRequest {
                tenant_id: workflow_config.tenant_id.clone(),
                creator_user_id: workflow_config.owner_user_id.clone(),
                project_id: workflow_config.project_id.clone(),
                repositories: workflow_config.repositories.clone(),
                provider_account_ref: workflow_config.provider_account_ref.clone(),
            })
            .await
    }

    async fn discover_repository(
        &self,
        workflow_config: &GithubIssueWorkflowConfig,
        repository: &GithubRepositorySelector,
        outcome: &mut GithubIssueWorkflowPollerTickOutcome,
    ) -> Result<(), GithubIssueWorkflowError> {
        let query = open_bug_query(repository);
        let hits = self
            .ports
            .github_port()
            .search_open_bug_issues(SearchGithubIssuesInput {
                provider_account_ref: workflow_config.provider_account_ref.clone(),
                owner: repository.owner.clone(),
                repo: repository.repo.clone(),
                query,
                limit: self.config.max_issues_per_repo_per_tick,
            })
            .await?;
        outcome.repositories_scanned += 1;

        for hit in hits
            .into_iter()
            .take(self.config.max_issues_per_repo_per_tick)
        {
            outcome.issues_seen += 1;
            let snapshot = self
                .ports
                .github_port()
                .get_issue(GetGithubIssueInput {
                    provider_account_ref: workflow_config.provider_account_ref.clone(),
                    owner: repository.owner.clone(),
                    repo: repository.repo.clone(),
                    number: hit.number,
                })
                .await?;
            ensure_snapshot_matches_request(repository, hit.number, &snapshot)?;
            let issue_ref = snapshot.issue_ref();
            let comments = self
                .ports
                .github_port()
                .list_issue_comments(ListIssueCommentsInput {
                    issue: issue_ref.clone(),
                })
                .await?;
            let run_outcome = self
                .ports
                .repository()
                .create_or_get_workflow_run(CreateOrGetWorkflowRunInput {
                    tenant_id: workflow_config.tenant_id.clone(),
                    creator_user_id: workflow_config.owner_user_id.clone(),
                    agent_id: None,
                    project_id: Some(workflow_config.project_id.clone()),
                    issue_ref: issue_ref.clone(),
                    workflow_policy_key: self.workflow_policy_key.clone(),
                    workflow_policy_version: self.workflow_policy_version.clone(),
                    now: self.ports.clock().now(),
                })
                .await?;

            let (run, event_type) = match run_outcome {
                CreateOrGetWorkflowRunOutcome::Created { run } => {
                    (run, GithubIssueWorkflowEventType::GithubIssueDiscovered)
                }
                CreateOrGetWorkflowRunOutcome::Existing { run } => {
                    let provider = issue_binding_ref(&issue_ref).provider_ref;
                    if !self
                        .should_record_changed_event(
                            &run.workflow_run_id,
                            &provider,
                            snapshot.updated_at,
                        )
                        .await?
                    {
                        outcome.events_deduped += 1;
                        continue;
                    }
                    (run, GithubIssueWorkflowEventType::GithubIssueChanged)
                }
            };

            let provider = issue_binding_ref(&issue_ref).provider_ref;
            let event_outcome = self
                .ports
                .repository()
                .record_workflow_event(RecordWorkflowEventInput {
                    workflow_run_id: run.workflow_run_id,
                    workflow_event_type: event_type.clone(),
                    envelope: WorkflowEventEnvelope {
                        source_kind: WorkflowEventSourceKind::Poller,
                        source_delivery_id: None,
                        provider,
                        observed_at: self.ports.clock().now(),
                        provider_updated_at: snapshot.updated_at,
                        idempotency_key: match event_type {
                            GithubIssueWorkflowEventType::GithubIssueDiscovered => {
                                issue_discovered_key(&issue_ref)
                            }
                            GithubIssueWorkflowEventType::GithubIssueChanged => {
                                issue_changed_key(&issue_ref, snapshot.updated_at)
                            }
                            _ => {
                                return Err(GithubIssueWorkflowError::Policy {
                                    reason: "poller can only emit issue discovery/change events"
                                        .to_string(),
                                });
                            }
                        },
                        payload_schema: event_payload_schema(&event_type).to_string(),
                        payload: issue_event_payload(&snapshot, comments.len()),
                    },
                })
                .await?;
            match event_outcome {
                RecordWorkflowEventOutcome::Recorded { .. } => outcome.events_recorded += 1,
                RecordWorkflowEventOutcome::Duplicate { .. }
                | RecordWorkflowEventOutcome::Superseded { .. } => outcome.events_deduped += 1,
            }
        }

        Ok(())
    }

    async fn should_record_changed_event(
        &self,
        workflow_run_id: &crate::GithubIssueWorkflowRunId,
        provider: &GithubProviderRef,
        provider_updated_at: Option<DateTime<Utc>>,
    ) -> Result<bool, GithubIssueWorkflowError> {
        let latest = self
            .ports
            .repository()
            .find_latest_workflow_event_for_provider(FindLatestWorkflowEventForProviderInput {
                workflow_run_id: workflow_run_id.clone(),
                workflow_event_types: vec![
                    GithubIssueWorkflowEventType::GithubIssueDiscovered,
                    GithubIssueWorkflowEventType::GithubIssueChanged,
                ],
                provider: provider.clone(),
            })
            .await?;

        let Some(latest) = latest else {
            return Ok(true);
        };

        Ok(match (latest.provider_updated_at, provider_updated_at) {
            (Some(latest_updated_at), Some(current_updated_at)) => {
                current_updated_at > latest_updated_at
            }
            (None, Some(_)) => true,
            _ => false,
        })
    }

    async fn tick_runnable_runs(
        &self,
        tenants: &[TenantId],
        outcome: &mut GithubIssueWorkflowPollerTickOutcome,
    ) -> Result<(), GithubIssueWorkflowError> {
        let mut remaining = self.config.max_runnable_runs_per_tick;
        let lease_duration = chrono_lease_duration(self.config.lease_duration)?;
        for tenant_id in tenants {
            if remaining == 0 {
                break;
            }
            let now = self.ports.clock().now();
            let claimed = self
                .ports
                .repository()
                .claim_runnable_workflow_runs(ClaimRunnableWorkflowRunsInput {
                    tenant_id: tenant_id.clone(),
                    worker_id: self.ports.worker_id(),
                    now,
                    lease_expires_at: now + lease_duration,
                    limit: remaining,
                })
                .await?;
            remaining = remaining.saturating_sub(claimed.len());
            outcome.runnable_runs_claimed += claimed.len();

            for run in claimed {
                self.tick_claimed_run(run, outcome).await?;
            }
        }
        Ok(())
    }

    async fn tick_claimed_run(
        &self,
        run: crate::GithubIssueWorkflowRun,
        outcome: &mut GithubIssueWorkflowPollerTickOutcome,
    ) -> Result<(), GithubIssueWorkflowError> {
        let policy = GithubIssueWorkflowPolicy::new(
            self.policy_ports(),
            self.workflow_policy_version.clone(),
        );
        match policy.tick(run.clone()).await {
            Ok(policy_outcome) => {
                outcome.policy_ticks += 1;
                let release = self
                    .ports
                    .repository()
                    .release_workflow_run_lease(ReleaseWorkflowRunLeaseInput {
                        workflow_run_id: policy_outcome.run.workflow_run_id,
                        worker_id: self.ports.worker_id(),
                        now: self.ports.clock().now(),
                    })
                    .await?;
                if matches!(release, LeaseReleaseOutcome::Released { .. }) {
                    outcome.leases_released += 1;
                }
                Ok(())
            }
            Err(error) => {
                let reason = error.to_string();
                let kind = run_block_kind(&error);
                self.ports
                    .repository()
                    .block_workflow_run(BlockWorkflowRunInput {
                        workflow_run_id: run.workflow_run_id.clone(),
                        worker_id: self.ports.worker_id(),
                        active_block: GithubIssueBlockState {
                            kind: kind.clone(),
                            reason: reason.clone(),
                            blocked_at: self.ports.clock().now(),
                        },
                        now: self.ports.clock().now(),
                    })
                    .await?;
                outcome
                    .blocked_runs
                    .push(GithubIssueWorkflowPollerBlockedRun {
                        workflow_run_id: run.workflow_run_id,
                        kind,
                        reason,
                    });
                Ok(())
            }
        }
    }

    fn policy_ports(&self) -> PollerPolicyPorts<P> {
        PollerPolicyPorts {
            clock: self.ports.clock(),
            github_port: self.ports.github_port(),
            project_access: self.ports.project_access(),
            repository: self.ports.repository(),
            stage_turn_submitter: self.ports.stage_turn_submitter(),
            workspace_manager: self.ports.workspace_manager(),
            worker_id: self.ports.worker_id(),
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GithubIssueWorkflowPollerTickOutcome {
    pub disabled: bool,
    pub configs_loaded: usize,
    pub repositories_scanned: usize,
    pub issues_seen: usize,
    pub events_recorded: usize,
    pub events_deduped: usize,
    pub runnable_runs_claimed: usize,
    pub policy_ticks: usize,
    pub leases_released: usize,
    pub blocked_configs: Vec<GithubIssueWorkflowPollerBlockedConfig>,
    pub blocked_runs: Vec<GithubIssueWorkflowPollerBlockedRun>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GithubIssueWorkflowPollerBlockedConfig {
    pub tenant_id: TenantId,
    pub project_id: ProjectId,
    pub creator_user_id: UserId,
    pub repository: Option<GithubRepositorySelector>,
    pub kind: GithubIssueWorkflowPollerBlockKind,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GithubIssueWorkflowPollerBlockedRun {
    pub workflow_run_id: crate::GithubIssueWorkflowRunId,
    pub kind: GithubIssueBlockKind,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GithubIssueWorkflowPollerBlockKind {
    ProjectAccessDenied,
    ProviderRateLimited,
    ProviderReadFailed,
    WorkflowPolicy,
    Repository,
}

struct PollerPolicyPorts<P>
where
    P: GithubIssueWorkflowPollerPorts,
{
    clock: Arc<P::Clock>,
    github_port: Arc<P::GithubPort>,
    project_access: Arc<P::ProjectAccess>,
    repository: Arc<P::Repository>,
    stage_turn_submitter: Arc<P::StageTurnSubmitter>,
    workspace_manager: Arc<P::WorkspaceManager>,
    worker_id: WorkflowWorkerId,
}

impl<P> GithubIssueWorkflowPolicyPorts for PollerPolicyPorts<P>
where
    P: GithubIssueWorkflowPollerPorts,
{
    type Clock = P::Clock;
    type GithubPort = P::GithubPort;
    type ProjectAccess = P::ProjectAccess;
    type Repository = P::Repository;
    type StageTurnSubmitter = P::StageTurnSubmitter;
    type WorkspaceManager = P::WorkspaceManager;

    fn clock(&self) -> Arc<Self::Clock> {
        self.clock.clone()
    }

    fn github_port(&self) -> Arc<Self::GithubPort> {
        self.github_port.clone()
    }

    fn project_access(&self) -> Arc<Self::ProjectAccess> {
        self.project_access.clone()
    }

    fn repository(&self) -> Arc<Self::Repository> {
        self.repository.clone()
    }

    fn stage_turn_submitter(&self) -> Arc<Self::StageTurnSubmitter> {
        self.stage_turn_submitter.clone()
    }

    fn workspace_manager(&self) -> Arc<Self::WorkspaceManager> {
        self.workspace_manager.clone()
    }

    fn worker_id(&self) -> WorkflowWorkerId {
        self.worker_id.clone()
    }
}

fn open_bug_query(repository: &GithubRepositorySelector) -> String {
    format!(
        "repo:{}/{} is:issue state:open label:bug",
        repository.owner, repository.repo
    )
}

fn event_payload_schema(event_type: &GithubIssueWorkflowEventType) -> &'static str {
    match event_type {
        GithubIssueWorkflowEventType::GithubIssueDiscovered => "github.issue.discovered.v1",
        GithubIssueWorkflowEventType::GithubIssueChanged => "github.issue.changed.v1",
        _ => "github.issue.unknown.v1",
    }
}

fn issue_event_payload(snapshot: &GithubIssueProviderSnapshot, comment_count: usize) -> JsonValue {
    let issue = snapshot.issue_ref();
    json!({
        "issue": issue,
        "provider_snapshot": {
            "title": snapshot.title,
            "state": snapshot.state,
            "labels": snapshot.labels,
            "updated_at": snapshot.updated_at,
            "comment_count": comment_count,
            "body_present": !snapshot.body.is_empty(),
        }
    })
}

fn ensure_snapshot_matches_request(
    repository: &GithubRepositorySelector,
    requested_number: u64,
    snapshot: &GithubIssueProviderSnapshot,
) -> Result<(), GithubIssueWorkflowError> {
    if snapshot.owner == repository.owner
        && snapshot.repo == repository.repo
        && snapshot.number == requested_number
    {
        return Ok(());
    }

    Err(GithubIssueWorkflowError::ProviderRead {
        reason: format!(
            "GitHub provider returned issue {}/{}#{} while reading configured issue {}/{}#{}",
            snapshot.owner,
            snapshot.repo,
            snapshot.number,
            repository.owner,
            repository.repo,
            requested_number
        ),
    })
}

fn blocked_config(
    workflow_config: &GithubIssueWorkflowConfig,
    repository: Option<&GithubRepositorySelector>,
    error: &GithubIssueWorkflowError,
) -> GithubIssueWorkflowPollerBlockedConfig {
    GithubIssueWorkflowPollerBlockedConfig {
        tenant_id: workflow_config.tenant_id.clone(),
        project_id: workflow_config.project_id.clone(),
        creator_user_id: workflow_config.owner_user_id.clone(),
        repository: repository.cloned(),
        kind: config_block_kind(error),
        reason: error.to_string(),
    }
}

fn config_block_kind(error: &GithubIssueWorkflowError) -> GithubIssueWorkflowPollerBlockKind {
    match error {
        GithubIssueWorkflowError::PolicyDenied { .. } => {
            GithubIssueWorkflowPollerBlockKind::ProjectAccessDenied
        }
        GithubIssueWorkflowError::ProviderRateLimited { .. } => {
            GithubIssueWorkflowPollerBlockKind::ProviderRateLimited
        }
        GithubIssueWorkflowError::ProviderRead { .. } => {
            GithubIssueWorkflowPollerBlockKind::ProviderReadFailed
        }
        GithubIssueWorkflowError::Repository { .. } => {
            GithubIssueWorkflowPollerBlockKind::Repository
        }
        _ => GithubIssueWorkflowPollerBlockKind::WorkflowPolicy,
    }
}

fn run_block_kind(error: &GithubIssueWorkflowError) -> GithubIssueBlockKind {
    match error {
        GithubIssueWorkflowError::ProviderRateLimited { .. } => GithubIssueBlockKind::RateLimited,
        _ => GithubIssueBlockKind::RecoveryRequired,
    }
}

fn chrono_lease_duration(
    duration: std::time::Duration,
) -> Result<Duration, GithubIssueWorkflowError> {
    Duration::from_std(duration).map_err(|_| GithubIssueWorkflowError::InvalidConfig {
        reason: "poller lease_duration is too large".to_string(),
    })
}
