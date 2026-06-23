use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use ironclaw_host_api::{ProjectId, TenantId, UserId};
use serde::{Deserialize, Serialize};
use serde_json::{Value as JsonValue, json};

use crate::{
    BlockWorkflowRunInput, ClaimRunnableWorkflowRunsInput, CreateOrGetWorkflowRunInput,
    CreateOrGetWorkflowRunOutcome, FindLatestWorkflowEventForProviderInput, GetGithubIssueInput,
    GetPullRequestInput, GithubChecksChangedPayload, GithubCommentRef, GithubIssueBlockKind,
    GithubIssueBlockState, GithubIssueCandidateSelector, GithubIssueClosedPayload,
    GithubIssueProviderSnapshot, GithubIssueWorkflowConfig, GithubIssueWorkflowConfigSource,
    GithubIssueWorkflowError, GithubIssueWorkflowEventType, GithubIssueWorkflowPolicy,
    GithubIssueWorkflowPolicyPorts, GithubIssueWorkflowPollerConfig, GithubIssueWorkflowPort,
    GithubIssueWorkflowRepository, GithubIssueWorkflowRun, GithubProviderRef,
    GithubPullRequestCheckSnapshot, GithubPullRequestRef, GithubPullRequestSnapshot,
    GithubPullRequestUpdatedPayload, GithubRepositorySelector, GithubReviewCommentCreatedPayload,
    GithubReviewCommentSnapshot, LeaseReleaseOutcome, ListActiveWorkflowRunsForRepositoryInput,
    ListIssueCommentsInput, ListPullRequestChecksInput, ListPullRequestReviewCommentsInput,
    RecordWorkflowEventInput, RecordWorkflowEventOutcome, ReleaseWorkflowRunLeaseInput,
    SearchGithubIssuesInput, StageTurnSubmitter, WorkflowClock, WorkflowConfigAccessRequest,
    WorkflowEventEnvelope, WorkflowEventSourceKind, WorkflowProjectAccess, WorkflowWorkerId,
    WorkflowWorkspaceManager, checks_failed_key, checks_succeeded_key, issue_binding_ref,
    issue_changed_key, issue_closed_key, issue_discovered_key, pr_updated_key,
    primary_pr_binding_ref, review_comment_created_key,
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

struct LifecycleEventRecord {
    event_type: GithubIssueWorkflowEventType,
    provider: GithubProviderRef,
    provider_updated_at: Option<DateTime<Utc>>,
    idempotency_key: crate::WorkflowIdempotencyKey,
    payload: JsonValue,
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
                let repository_result = async {
                    self.discover_repository(&workflow_config, repository, &mut outcome)
                        .await?;
                    self.refresh_active_runs_for_repository(
                        &workflow_config,
                        repository,
                        &mut outcome,
                    )
                    .await
                }
                .await;
                if let Err(error) = repository_result {
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
        let query = open_bug_query(repository, &workflow_config.candidate_selector);
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
            if !workflow_config
                .candidate_selector
                .allows_author_login(snapshot.author_login.as_deref())
            {
                continue;
            }
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
                    provider_account_ref: Some(workflow_config.provider_account_ref.clone()),
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

    async fn refresh_active_runs_for_repository(
        &self,
        workflow_config: &GithubIssueWorkflowConfig,
        repository: &GithubRepositorySelector,
        outcome: &mut GithubIssueWorkflowPollerTickOutcome,
    ) -> Result<(), GithubIssueWorkflowError> {
        let active_runs = self
            .ports
            .repository()
            .list_active_workflow_runs_for_repository(ListActiveWorkflowRunsForRepositoryInput {
                tenant_id: workflow_config.tenant_id.clone(),
                repository: repository.clone(),
                limit: workflow_config.max_active_runs_per_repo as usize,
            })
            .await?;

        for run in active_runs {
            self.refresh_active_run(workflow_config, run, outcome)
                .await?;
        }

        Ok(())
    }

    async fn refresh_active_run(
        &self,
        workflow_config: &GithubIssueWorkflowConfig,
        run: GithubIssueWorkflowRun,
        outcome: &mut GithubIssueWorkflowPollerTickOutcome,
    ) -> Result<(), GithubIssueWorkflowError> {
        if run.event_cursor == 0 && run.workflow_state.primary_pr.is_none() {
            return Ok(());
        }

        let provider_account_ref = run
            .provider_account_ref
            .clone()
            .unwrap_or_else(|| workflow_config.provider_account_ref.clone());
        let issue_snapshot = self
            .ports
            .github_port()
            .get_issue(GetGithubIssueInput {
                provider_account_ref: provider_account_ref.clone(),
                owner: run.issue_ref.owner.clone(),
                repo: run.issue_ref.repo.clone(),
                number: run.issue_ref.number,
            })
            .await?;
        ensure_snapshot_matches_request(
            &GithubRepositorySelector {
                owner: run.issue_ref.owner.clone(),
                repo: run.issue_ref.repo.clone(),
            },
            run.issue_ref.number,
            &issue_snapshot,
        )?;

        if let Some(primary_pr) = run.workflow_state.primary_pr.clone() {
            let pull_request = self
                .ports
                .github_port()
                .get_pull_request(GetPullRequestInput {
                    provider_account_ref: provider_account_ref.clone(),
                    owner: primary_pr.owner.clone(),
                    repo: primary_pr.repo.clone(),
                    number: primary_pr.number,
                })
                .await?;
            ensure_pull_request_matches_ref(&primary_pr, &pull_request)?;
            self.refresh_pull_request_event(&run, &pull_request, outcome)
                .await?;
            self.refresh_pull_request_checks(
                &run,
                &provider_account_ref,
                &pull_request.pull_request,
                outcome,
            )
            .await?;
            self.refresh_pull_request_review_comments(
                &run,
                &provider_account_ref,
                &pull_request.pull_request,
                outcome,
            )
            .await?;
        }

        if issue_snapshot.state != "closed" {
            return Ok(());
        }
        let provider = issue_binding_ref(&run.issue_ref).provider_ref;
        self.record_lifecycle_event(
            &run,
            LifecycleEventRecord {
                event_type: GithubIssueWorkflowEventType::GithubIssueClosed,
                provider,
                provider_updated_at: issue_snapshot.updated_at,
                idempotency_key: issue_closed_key(&run.issue_ref, issue_snapshot.updated_at),
                payload: serde_json::to_value(GithubIssueClosedPayload {
                    issue: run.issue_ref.clone(),
                    closed_at: issue_snapshot.updated_at,
                })
                .map_err(poller_serde_error)?,
            },
            outcome,
        )
        .await
    }

    async fn refresh_pull_request_event(
        &self,
        run: &GithubIssueWorkflowRun,
        snapshot: &GithubPullRequestSnapshot,
        outcome: &mut GithubIssueWorkflowPollerTickOutcome,
    ) -> Result<(), GithubIssueWorkflowError> {
        let provider = primary_pr_binding_ref(&snapshot.pull_request).provider_ref;
        if !self
            .should_record_provider_event(
                &run.workflow_run_id,
                &[
                    GithubIssueWorkflowEventType::GithubPullRequestOpened,
                    GithubIssueWorkflowEventType::GithubPullRequestUpdated,
                ],
                &provider,
                snapshot.updated_at,
            )
            .await?
        {
            outcome.events_deduped += 1;
            return Ok(());
        }

        self.record_lifecycle_event(
            run,
            LifecycleEventRecord {
                event_type: GithubIssueWorkflowEventType::GithubPullRequestUpdated,
                provider,
                provider_updated_at: snapshot.updated_at,
                idempotency_key: pr_updated_key(&snapshot.pull_request, snapshot.updated_at),
                payload: serde_json::to_value(GithubPullRequestUpdatedPayload {
                    pull_request: snapshot.pull_request.clone(),
                    state: snapshot.state.clone(),
                    merged: snapshot.merged,
                    draft: snapshot.draft,
                })
                .map_err(poller_serde_error)?,
            },
            outcome,
        )
        .await
    }

    async fn refresh_pull_request_checks(
        &self,
        run: &GithubIssueWorkflowRun,
        provider_account_ref: &crate::GithubProviderAccountRef,
        pull_request: &GithubPullRequestRef,
        outcome: &mut GithubIssueWorkflowPollerTickOutcome,
    ) -> Result<(), GithubIssueWorkflowError> {
        let checks = self
            .ports
            .github_port()
            .list_pull_request_checks(ListPullRequestChecksInput {
                provider_account_ref: provider_account_ref.clone(),
                owner: pull_request.owner.clone(),
                repo: pull_request.repo.clone(),
                pull_request_number: pull_request.number,
                head_sha: pull_request.head_sha.clone(),
                limit: 100,
            })
            .await?;
        let has_failure = checks.iter().any(|check| check.conclusion.is_failure());
        for check in checks.iter().filter(|check| check.conclusion.is_failure()) {
            let provider = check_provider_ref(pull_request, check);
            self.record_lifecycle_event(
                run,
                LifecycleEventRecord {
                    event_type: GithubIssueWorkflowEventType::GithubChecksFailed,
                    provider,
                    provider_updated_at: check.completed_at,
                    idempotency_key: checks_failed_key(&check.head_sha, &check.suite_or_run_id),
                    payload: serde_json::to_value(GithubChecksChangedPayload {
                        pull_request: Some(pull_request.clone()),
                        head_sha: check.head_sha.clone(),
                        suite_or_run_id: check.suite_or_run_id.clone(),
                        conclusion: check.conclusion.as_provider_str().to_string(),
                    })
                    .map_err(poller_serde_error)?,
                },
                outcome,
            )
            .await?;
        }
        if !has_failure
            && !checks.is_empty()
            && checks.iter().all(|check| check.conclusion.is_success())
        {
            let head_sha = checks[0].head_sha.clone();
            self.record_lifecycle_event(
                run,
                LifecycleEventRecord {
                    event_type: GithubIssueWorkflowEventType::GithubChecksSucceeded,
                    provider: aggregate_checks_provider_ref(pull_request, &head_sha),
                    provider_updated_at: checks.iter().filter_map(|check| check.completed_at).max(),
                    idempotency_key: checks_succeeded_key(&head_sha, "aggregate"),
                    payload: serde_json::to_value(GithubChecksChangedPayload {
                        pull_request: Some(pull_request.clone()),
                        head_sha,
                        suite_or_run_id: "aggregate".to_string(),
                        conclusion: "success".to_string(),
                    })
                    .map_err(poller_serde_error)?,
                },
                outcome,
            )
            .await?;
        }

        Ok(())
    }

    async fn refresh_pull_request_review_comments(
        &self,
        run: &GithubIssueWorkflowRun,
        provider_account_ref: &crate::GithubProviderAccountRef,
        pull_request: &GithubPullRequestRef,
        outcome: &mut GithubIssueWorkflowPollerTickOutcome,
    ) -> Result<(), GithubIssueWorkflowError> {
        let comments = self
            .ports
            .github_port()
            .list_pull_request_review_comments(ListPullRequestReviewCommentsInput {
                provider_account_ref: provider_account_ref.clone(),
                owner: pull_request.owner.clone(),
                repo: pull_request.repo.clone(),
                pull_request_number: pull_request.number,
                since: run
                    .workflow_state
                    .last_provider_watermarks
                    .reviews_updated_at,
                limit: 100,
            })
            .await?;
        for comment in comments {
            if comment.body.contains("ironclaw:github-bug-workflow") {
                continue;
            }
            self.record_review_comment_event(run, pull_request, comment, outcome)
                .await?;
        }

        Ok(())
    }

    async fn record_review_comment_event(
        &self,
        run: &GithubIssueWorkflowRun,
        pull_request: &GithubPullRequestRef,
        comment: GithubReviewCommentSnapshot,
        outcome: &mut GithubIssueWorkflowPollerTickOutcome,
    ) -> Result<(), GithubIssueWorkflowError> {
        let provider = review_comment_provider_ref(pull_request, &comment.comment);
        let comment_identity = comment
            .comment
            .node_id
            .as_deref()
            .unwrap_or(comment.comment.url.as_str());
        self.record_lifecycle_event(
            run,
            LifecycleEventRecord {
                event_type: GithubIssueWorkflowEventType::GithubReviewCommentCreated,
                provider,
                provider_updated_at: Some(comment.updated_at),
                idempotency_key: review_comment_created_key(comment_identity),
                payload: serde_json::to_value(GithubReviewCommentCreatedPayload {
                    pull_request: Some(pull_request.clone()),
                    comment: comment.comment,
                })
                .map_err(poller_serde_error)?,
            },
            outcome,
        )
        .await
    }

    async fn record_lifecycle_event(
        &self,
        run: &GithubIssueWorkflowRun,
        record: LifecycleEventRecord,
        outcome: &mut GithubIssueWorkflowPollerTickOutcome,
    ) -> Result<(), GithubIssueWorkflowError> {
        let event_type = record.event_type;
        let event_outcome = self
            .ports
            .repository()
            .record_workflow_event(RecordWorkflowEventInput {
                workflow_run_id: run.workflow_run_id.clone(),
                workflow_event_type: event_type.clone(),
                envelope: WorkflowEventEnvelope {
                    source_kind: WorkflowEventSourceKind::Poller,
                    source_delivery_id: None,
                    provider: record.provider,
                    observed_at: self.ports.clock().now(),
                    provider_updated_at: record.provider_updated_at,
                    idempotency_key: record.idempotency_key,
                    payload_schema: event_payload_schema(&event_type).to_string(),
                    payload: record.payload,
                },
            })
            .await?;
        match event_outcome {
            RecordWorkflowEventOutcome::Recorded { .. } => outcome.events_recorded += 1,
            RecordWorkflowEventOutcome::Duplicate { .. }
            | RecordWorkflowEventOutcome::Superseded { .. } => outcome.events_deduped += 1,
        }
        Ok(())
    }

    async fn should_record_changed_event(
        &self,
        workflow_run_id: &crate::GithubIssueWorkflowRunId,
        provider: &GithubProviderRef,
        provider_updated_at: Option<DateTime<Utc>>,
    ) -> Result<bool, GithubIssueWorkflowError> {
        self.should_record_provider_event(
            workflow_run_id,
            &[
                GithubIssueWorkflowEventType::GithubIssueDiscovered,
                GithubIssueWorkflowEventType::GithubIssueChanged,
            ],
            provider,
            provider_updated_at,
        )
        .await
    }

    async fn should_record_provider_event(
        &self,
        workflow_run_id: &crate::GithubIssueWorkflowRunId,
        workflow_event_types: &[GithubIssueWorkflowEventType],
        provider: &GithubProviderRef,
        provider_updated_at: Option<DateTime<Utc>>,
    ) -> Result<bool, GithubIssueWorkflowError> {
        let latest = self
            .ports
            .repository()
            .find_latest_workflow_event_for_provider(FindLatestWorkflowEventForProviderInput {
                workflow_run_id: workflow_run_id.clone(),
                workflow_event_types: workflow_event_types.to_vec(),
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

fn open_bug_query(
    repository: &GithubRepositorySelector,
    selector: &GithubIssueCandidateSelector,
) -> String {
    let mut query = format!(
        "repo:{}/{} is:issue state:open",
        repository.owner, repository.repo
    );
    for label in &selector.labels {
        query.push_str(" label:");
        query.push_str(&github_search_value(label));
    }
    query
}

fn github_search_value(value: &str) -> String {
    if value
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.'))
    {
        return value.to_string();
    }

    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

fn event_payload_schema(event_type: &GithubIssueWorkflowEventType) -> &'static str {
    match event_type {
        GithubIssueWorkflowEventType::GithubIssueDiscovered => "github.issue.discovered.v1",
        GithubIssueWorkflowEventType::GithubIssueChanged => "github.issue.changed.v1",
        GithubIssueWorkflowEventType::GithubIssueClosed => "github.issue.closed.v1",
        GithubIssueWorkflowEventType::GithubPullRequestOpened => "github.pr.opened.v1",
        GithubIssueWorkflowEventType::GithubPullRequestUpdated => "github.pr.updated.v1",
        GithubIssueWorkflowEventType::GithubChecksChanged => "github.checks.changed.v1",
        GithubIssueWorkflowEventType::GithubChecksFailed => "github.checks.failed.v1",
        GithubIssueWorkflowEventType::GithubChecksSucceeded => "github.checks.succeeded.v1",
        GithubIssueWorkflowEventType::GithubReviewCommentCreated => {
            "github.review_comment.created.v1"
        }
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
            "author_login": snapshot.author_login,
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

fn ensure_pull_request_matches_ref(
    expected: &GithubPullRequestRef,
    snapshot: &GithubPullRequestSnapshot,
) -> Result<(), GithubIssueWorkflowError> {
    let actual = &snapshot.pull_request;
    if actual.owner == expected.owner
        && actual.repo == expected.repo
        && actual.number == expected.number
    {
        return Ok(());
    }

    Err(GithubIssueWorkflowError::ProviderRead {
        reason: format!(
            "GitHub provider returned PR {}/{}#{} while reading workflow PR {}/{}#{}",
            actual.owner,
            actual.repo,
            actual.number,
            expected.owner,
            expected.repo,
            expected.number
        ),
    })
}

fn check_provider_ref(
    pull_request: &GithubPullRequestRef,
    check: &GithubPullRequestCheckSnapshot,
) -> GithubProviderRef {
    GithubProviderRef {
        system: "github".to_string(),
        resource_type: "check_run".to_string(),
        owner: pull_request.owner.clone(),
        repo: pull_request.repo.clone(),
        provider_id: format!("{}:{}", check.head_sha, check.suite_or_run_id),
        provider_url: check.details_url.clone(),
    }
}

fn aggregate_checks_provider_ref(
    pull_request: &GithubPullRequestRef,
    head_sha: &str,
) -> GithubProviderRef {
    GithubProviderRef {
        system: "github".to_string(),
        resource_type: "check_suite".to_string(),
        owner: pull_request.owner.clone(),
        repo: pull_request.repo.clone(),
        provider_id: format!("{head_sha}:aggregate"),
        provider_url: Some(pull_request.url.clone()),
    }
}

fn review_comment_provider_ref(
    pull_request: &GithubPullRequestRef,
    comment: &GithubCommentRef,
) -> GithubProviderRef {
    GithubProviderRef {
        system: "github".to_string(),
        resource_type: "review_comment".to_string(),
        owner: pull_request.owner.clone(),
        repo: pull_request.repo.clone(),
        provider_id: comment
            .node_id
            .clone()
            .unwrap_or_else(|| comment.url.clone()),
        provider_url: Some(comment.url.clone()),
    }
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

fn poller_serde_error(error: serde_json::Error) -> GithubIssueWorkflowError {
    GithubIssueWorkflowError::Policy {
        reason: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::{github_search_value, open_bug_query};
    use crate::{GithubIssueCandidateSelector, GithubRepositorySelector};

    #[test]
    fn open_bug_query_uses_configured_candidate_labels() {
        let repository = GithubRepositorySelector::new("near", "ironclaw").expect("repository");
        let query = open_bug_query(
            &repository,
            &GithubIssueCandidateSelector {
                labels: vec!["bug".to_string(), "good first issue".to_string()],
                allowed_author_logins: Vec::new(),
            },
        );

        assert_eq!(
            query,
            "repo:near/ironclaw is:issue state:open label:bug label:\"good first issue\""
        );
    }

    #[test]
    fn github_search_value_escapes_quoted_labels() {
        assert_eq!(github_search_value("bug"), "bug");
        assert_eq!(
            github_search_value("needs \"care\""),
            "\"needs \\\"care\\\"\""
        );
    }
}
