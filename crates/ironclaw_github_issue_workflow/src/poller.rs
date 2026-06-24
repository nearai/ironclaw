use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use ironclaw_host_api::{ProjectId, TenantId, UserId};
use serde::{Deserialize, Serialize};
use serde_json::{Value as JsonValue, json};
use tracing::debug;

use crate::{
    BlockWorkflowRunInput, ClaimRunnableWorkflowRunsInput, CreateOrGetWorkflowRunInput,
    CreateOrGetWorkflowRunOutcome, FailStageRunInput, FindLatestWorkflowEventForProviderInput,
    GetGithubIssueInput, GetPullRequestInput, GetStageRunInput, GithubChecksChangedPayload,
    GithubCommentRef, GithubIssueBlockKind, GithubIssueBlockState, GithubIssueCandidateSelector,
    GithubIssueClosedPayload, GithubIssueCommentSnapshot, GithubIssueProviderSnapshot,
    GithubIssueProviderSnapshotSummary, GithubIssueWorkflowConfig, GithubIssueWorkflowConfigSource,
    GithubIssueWorkflowError, GithubIssueWorkflowEventType, GithubIssueWorkflowPolicy,
    GithubIssueWorkflowPolicyPorts, GithubIssueWorkflowPollerConfig, GithubIssueWorkflowPort,
    GithubIssueWorkflowRepository, GithubIssueWorkflowRun, GithubIssueWorkflowRunStatus,
    GithubProviderRef, GithubPullRequestCheckSnapshot, GithubPullRequestRef,
    GithubPullRequestSnapshot, GithubPullRequestUpdatedPayload, GithubRepositorySelector,
    GithubReviewCommentCreatedPayload, GithubReviewCommentSnapshot, LeaseReleaseOutcome,
    ListActiveWorkflowRunsForRepositoryInput, ListIssueCommentsInput, ListPullRequestChecksInput,
    ListPullRequestReviewCommentsInput, ProviderContentSummary, RecordWorkflowEventInput,
    RecordWorkflowEventOutcome, ReleaseWorkflowRunLeaseInput, SearchGithubIssuesInput,
    StageTurnSubmitter, WorkflowClock, WorkflowConfigAccessRequest, WorkflowEventEnvelope,
    WorkflowEventSourceKind, WorkflowProjectAccess, WorkflowWorkerId, WorkflowWorkspaceManager,
    checks_failed_key, checks_succeeded_key, issue_binding_ref, issue_changed_key,
    issue_closed_key, issue_discovered_key, pr_updated_key, primary_pr_binding_ref,
    review_comment_created_key, stage_slug,
};

const DEFAULT_WORKFLOW_POLICY_KEY: &str = "github-bug-workflow";
// Per-text cap for an issue body or a single comment embedded in the engineered
// snapshot. Raised from the original conservative 12k so a long bug report (with
// stack traces / repro steps) survives into the model context; the downstream
// event replay is capped at ~100KB total, so this stays well under that ceiling
// even with several comments attached.
const MAX_PROVIDER_CONTENT_SUMMARY_CHARS: usize = 64_000;
const MAX_PROVIDER_COMMENT_SUMMARIES: usize = 5;
const WORKFLOW_CLAIM_COMMENT_PREFIX: &str = "<!-- ironclaw:github-bug-workflow:claim:";
/// Upper bound on the number of policy transitions a single `tick_claimed_run`
/// drains for one claimed run before yielding the lease. Each policy tick
/// advances exactly one event; draining collapses already-recorded queued
/// events into one tick so a multi-stage burst does not wait a full poll
/// interval per boundary. The bound caps how long a single run holds its lease
/// so one busy run cannot starve the rest of the tenant.
const MAX_DRAINED_TRANSITIONS_PER_TICK: usize = 32;

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

    #[tracing::instrument(
        skip_all,
        fields(worker_id = %self.ports.worker_id(), policy_key = %self.workflow_policy_key)
    )]
    pub async fn tick_once(
        &self,
    ) -> Result<GithubIssueWorkflowPollerTickOutcome, GithubIssueWorkflowError> {
        let mut outcome = GithubIssueWorkflowPollerTickOutcome::default();
        if !self.config.enabled {
            outcome.disabled = true;
            debug!("github issue workflow poller is disabled; skipping tick");
            return Ok(outcome);
        }

        let configs = self
            .ports
            .config_source()
            .list_enabled_workflow_configs()
            .await?;
        outcome.configs_loaded = configs.len();
        debug!(
            configs_loaded = outcome.configs_loaded,
            "poller tick started"
        );

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
        debug!(
            repositories_scanned = outcome.repositories_scanned,
            issues_seen = outcome.issues_seen,
            events_recorded = outcome.events_recorded,
            events_deduped = outcome.events_deduped,
            runnable_runs_claimed = outcome.runnable_runs_claimed,
            policy_ticks = outcome.policy_ticks,
            leases_released = outcome.leases_released,
            stale_stages_failed = outcome.stale_stages_failed,
            blocked_configs = outcome.blocked_configs.len(),
            blocked_runs = outcome.blocked_runs.len(),
            "poller tick completed"
        );
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

    #[tracing::instrument(
        skip_all,
        fields(
            owner = %repository.owner,
            repo = %repository.repo,
            account_id = %workflow_config.provider_account_ref.account_id,
        )
    )]
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
        debug!(hits = hits.len(), "discovered open bug issue candidates");

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
                debug!(
                    issue = hit.number,
                    author_login = snapshot.author_login.as_deref().unwrap_or("<none>"),
                    outcome = "skipped",
                    "author allowlist denied issue candidate"
                );
                continue;
            }
            let issue_ref = snapshot.issue_ref();
            let comments = self
                .ports
                .github_port()
                .list_issue_comments(ListIssueCommentsInput {
                    provider_account_ref: workflow_config.provider_account_ref.clone(),
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
                    debug!(
                        issue = hit.number,
                        workflow_run_id = %run.workflow_run_id,
                        outcome = "discovered",
                        "created workflow run for newly discovered issue"
                    );
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
                        debug!(
                            issue = hit.number,
                            workflow_run_id = %run.workflow_run_id,
                            outcome = "deduped",
                            "issue change already recorded; skipping"
                        );
                        continue;
                    }
                    debug!(
                        issue = hit.number,
                        workflow_run_id = %run.workflow_run_id,
                        outcome = "changed",
                        "issue changed since last observation"
                    );
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
                        payload: issue_event_payload(&snapshot, &comments),
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
            if !claimed.is_empty() {
                debug!(
                    tenant = %tenant_id,
                    worker_id = %self.ports.worker_id(),
                    claimed = claimed.len(),
                    "claimed runnable workflow runs"
                );
            }

            for run in claimed {
                self.tick_claimed_run(run, outcome).await?;
            }
        }
        Ok(())
    }

    #[tracing::instrument(
        skip_all,
        fields(
            workflow_run_id = %run.workflow_run_id,
            issue = run.issue_ref.number,
            worker_id = %self.ports.worker_id(),
        )
    )]
    async fn tick_claimed_run(
        &self,
        run: crate::GithubIssueWorkflowRun,
        outcome: &mut GithubIssueWorkflowPollerTickOutcome,
    ) -> Result<(), GithubIssueWorkflowError> {
        debug!(
            event_cursor = run.event_cursor,
            active_stage = run.active_stage_run_id.is_some(),
            "ticking claimed workflow run"
        );
        let policy = GithubIssueWorkflowPolicy::new(
            self.policy_ports(),
            self.workflow_policy_version.clone(),
        );
        // DRAIN: a single policy tick advances exactly one event, but the stage
        // result sink may have queued several events for this run (e.g. a stage
        // completion that immediately submits the next stage which completes
        // again). Re-tick the same run while it keeps reporting progress so a
        // multi-transition burst collapses into one claimed tick instead of
        // waiting a full poll interval per boundary. We stop when: a tick makes
        // no progress (no new event after the cursor), the run reaches a
        // terminal status, or the per-tick guard is hit (bounding lease-hold
        // time so one busy run cannot starve the tenant). The lease is held the
        // whole time — `advance_run_cursor` fails with NotLeaseOwner if it is
        // lost, which surfaces as the Err arm below.
        let mut run = run;
        let mut drained = 0usize;
        loop {
            match policy.tick(run.clone()).await {
                Ok(policy_outcome) => {
                    run = policy_outcome.run;
                    // A tick that advanced no event is a cursor probe, not a
                    // transition: stop draining and do NOT count it, so
                    // `policy_ticks` stays a count of real transitions rather
                    // than being inflated by the trailing empty probe every
                    // drain performs (a single-event run is 1 tick, not 2).
                    if policy_outcome.processed_event_count == 0 {
                        break;
                    }
                    outcome.policy_ticks += 1;
                    drained += 1;
                    if run_is_terminal(&run.status) || drained >= MAX_DRAINED_TRANSITIONS_PER_TICK {
                        break;
                    }
                }
                Err(error) => {
                    return self
                        .block_run_after_policy_failure(run, error, outcome)
                        .await;
                }
            }
        }
        // The drain settled. If the run still has an active stage and is not
        // terminal, run the stuck-stage reconciler on that final state.
        if !run_is_terminal(&run.status) && run.active_stage_run_id.is_some() {
            debug!(
                active_stage = true,
                "claimed run settled with an active stage (no further events to drain)"
            );
            // Stuck-stage reconciler: a stage turn is fire-and-forget, so
            // a turn that dies without reporting a result strands
            // `active_stage_run_id` set forever while the poller silently
            // re-claims the run every tick. When a drained run still has an
            // active stage, check the STAGE-level heartbeat (the run lease is
            // renewed each tick and never goes stale, so it cannot detect
            // this). If the stage is stale — or was already failed by a prior
            // reconcile that crashed before blocking — fail the stage and
            // escalate the run to RecoveryRequired via the same block path the
            // policy-error arm uses.
            if let Some(stage_run_id) = run.active_stage_run_id.clone() {
                let now = self.ports.clock().now();
                let snapshot = self
                    .ports
                    .repository()
                    .get_stage_run(GetStageRunInput {
                        workflow_run_id: run.workflow_run_id.clone(),
                        stage_run_id: stage_run_id.clone(),
                    })
                    .await?;
                let stale_after = chrono_lease_duration(self.config.stage_stale_after)?;
                match snapshot {
                    Some(snapshot) => {
                        let is_stale =
                            snapshot.active && now - snapshot.last_heartbeat_at >= stale_after;
                        // Do NOT escalate an inactive-but-not-failed stage:
                        // that is work that SUCCEEDED whose run-pointer clear
                        // merely crashed mid-accept; escalating it would
                        // wrongly demand human recovery of completed work.
                        if is_stale || snapshot.failed {
                            self.ports
                                .repository()
                                .fail_stage_run(FailStageRunInput {
                                    workflow_run_id: run.workflow_run_id.clone(),
                                    stage_run_id: stage_run_id.clone(),
                                    now,
                                })
                                .await?;
                            let reason = format!(
                                "stage `{}` stalled with no progress for >= {}s",
                                stage_slug(&snapshot.stage),
                                self.config.stage_stale_after.as_secs(),
                            );
                            self.escalate_stuck_stage_to_recovery(&run, reason, now, outcome)
                                .await?;
                            debug!(
                                stage_run_id = %stage_run_id,
                                "reconciler escalated stale stage to RecoveryRequired"
                            );
                            // block_workflow_run already cleared the lease and
                            // the active-stage pointer, so skip lease release.
                            return Ok(());
                        }
                    }
                    // Orphan pointer: `active_stage_run_id` is set but NO stage
                    // row backs it — a crash between the run-pointer write and
                    // the stage-row write. There is no stage-level heartbeat to
                    // read, so staleness is measured from the run's `created_at`
                    // (the only stable anchor: the run's lease — and therefore
                    // its `updated_at`/`last_heartbeat_at` — is refreshed on
                    // every claim, so it never goes stale on a stuck run). A
                    // brief orphan window is normal during stage creation, so we
                    // escalate only after `stage_stale_after`. There is no row to
                    // fail; `block_workflow_run` clears the orphan pointer, so
                    // the run stops re-claiming as a permanent no-op.
                    None => {
                        if now - run.created_at >= stale_after {
                            let reason = format!(
                                "active stage pointer `{}` has no backing stage row for >= {}s \
                                 (orphan pointer; recovery required)",
                                stage_run_id,
                                self.config.stage_stale_after.as_secs(),
                            );
                            self.escalate_stuck_stage_to_recovery(&run, reason, now, outcome)
                                .await?;
                            debug!(
                                stage_run_id = %stage_run_id,
                                "reconciler escalated orphan active-stage pointer to RecoveryRequired"
                            );
                            // block_workflow_run already cleared the lease and
                            // the orphan pointer, so skip lease release.
                            return Ok(());
                        }
                        debug!(
                            stage_run_id = %stage_run_id,
                            "orphan active-stage pointer is still within the stale window; leaving it"
                        );
                    }
                }
            }
        }
        let release = self
            .ports
            .repository()
            .release_workflow_run_lease(ReleaseWorkflowRunLeaseInput {
                workflow_run_id: run.workflow_run_id,
                worker_id: self.ports.worker_id(),
                now: self.ports.clock().now(),
            })
            .await?;
        if matches!(release, LeaseReleaseOutcome::Released { .. }) {
            outcome.leases_released += 1;
        }
        Ok(())
    }

    /// Block a claimed run after its policy tick failed, mirroring the original
    /// inline error arm. Used by the drain loop so a failure at any drained
    /// transition blocks the run instead of looping.
    async fn block_run_after_policy_failure(
        &self,
        run: crate::GithubIssueWorkflowRun,
        error: GithubIssueWorkflowError,
        outcome: &mut GithubIssueWorkflowPollerTickOutcome,
    ) -> Result<(), GithubIssueWorkflowError> {
        let reason = error.to_string();
        let kind = run_block_kind(&error);
        debug!(
            kind = ?kind,
            reason = %reason,
            "blocking workflow run after policy tick failure"
        );
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

    /// Escalate a stuck stage (stale stage row OR orphan pointer with no row) to
    /// a `RecoveryRequired` block. Shared by both stuck-stage reconciler arms so
    /// they record the outcome identically: block the run (which also clears the
    /// lease and the active-stage pointer), count it, and surface the blocked run
    /// to the tick outcome.
    async fn escalate_stuck_stage_to_recovery(
        &self,
        run: &crate::GithubIssueWorkflowRun,
        reason: String,
        now: DateTime<Utc>,
        outcome: &mut GithubIssueWorkflowPollerTickOutcome,
    ) -> Result<(), GithubIssueWorkflowError> {
        self.ports
            .repository()
            .block_workflow_run(BlockWorkflowRunInput {
                workflow_run_id: run.workflow_run_id.clone(),
                worker_id: self.ports.worker_id(),
                active_block: GithubIssueBlockState {
                    kind: GithubIssueBlockKind::RecoveryRequired,
                    reason: reason.clone(),
                    blocked_at: now,
                },
                now,
            })
            .await?;
        outcome.stale_stages_failed += 1;
        outcome
            .blocked_runs
            .push(GithubIssueWorkflowPollerBlockedRun {
                workflow_run_id: run.workflow_run_id.clone(),
                kind: GithubIssueBlockKind::RecoveryRequired,
                reason,
            });
        Ok(())
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

/// Wake channel that lets a stage-result producer re-tick the poller
/// immediately at a stage boundary instead of waiting a full poll interval.
///
/// Mirrors the turn-runner wake pattern: a `tokio::sync::Notify` shared between
/// a [`GithubIssueWorkflowPollerWakeSender`] (handed to the stage-result sink)
/// and a [`GithubIssueWorkflowPollerWakeReceiver`] (selected against in the
/// poller loop). Wake delivery is best-effort and edge-triggered: a wake that
/// arrives while the poller is mid-tick coalesces into the single pending
/// permit `Notify` holds, so it is safe to over-fire and the interval remains
/// the safety-net fallback.
#[derive(Debug, Clone)]
pub struct GithubIssueWorkflowPollerWakeSender {
    notify: Arc<tokio::sync::Notify>,
}

impl GithubIssueWorkflowPollerWakeSender {
    /// Signal the poller that a run may have newly drainable events.
    pub fn wake(&self) {
        self.notify.notify_one();
    }
}

/// Receiver half of the poller wake channel. Held by the poller loop.
#[derive(Debug, Clone)]
pub struct GithubIssueWorkflowPollerWakeReceiver {
    notify: Arc<tokio::sync::Notify>,
}

impl GithubIssueWorkflowPollerWakeReceiver {
    /// Construct a connected wake sender/receiver pair.
    pub fn channel() -> (
        GithubIssueWorkflowPollerWakeSender,
        GithubIssueWorkflowPollerWakeReceiver,
    ) {
        let notify = Arc::new(tokio::sync::Notify::new());
        (
            GithubIssueWorkflowPollerWakeSender {
                notify: Arc::clone(&notify),
            },
            Self { notify },
        )
    }

    /// Wait for a wake signal. The caller pairs this with an interval fallback
    /// via `select!` so a missed/dropped wake still recovers on the next tick.
    pub async fn notified(&self) {
        self.notify.notified().await;
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
    pub stale_stages_failed: usize,
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

fn issue_event_payload(
    snapshot: &GithubIssueProviderSnapshot,
    comments: &[GithubIssueCommentSnapshot],
) -> JsonValue {
    let issue = snapshot.issue_ref();
    let provider_snapshot = GithubIssueProviderSnapshotSummary {
        title: snapshot.title.clone(),
        state: snapshot.state.clone(),
        author_login: snapshot.author_login.clone(),
        labels: snapshot.labels.clone(),
        updated_at: snapshot.updated_at,
        comment_count: comments.len(),
        body_present: !snapshot.body.is_empty(),
        content_summaries: provider_content_summaries(snapshot, comments),
    };
    json!({
        "issue": issue,
        "provider_snapshot": provider_snapshot,
    })
}

fn provider_content_summaries(
    snapshot: &GithubIssueProviderSnapshot,
    comments: &[GithubIssueCommentSnapshot],
) -> Vec<ProviderContentSummary> {
    // +1 issue summary, +1 possible elision marker.
    let mut summaries = Vec::with_capacity(MAX_PROVIDER_COMMENT_SUMMARIES + 2);
    summaries.push(ProviderContentSummary {
        source_ref: format!(
            "github:issue:{}/{}#{}",
            snapshot.owner, snapshot.repo, snapshot.number
        ),
        author: snapshot.author_login.clone(),
        summary: format!(
            "Issue title: {}\n\nIssue body:\n{}",
            snapshot.title,
            truncate_provider_text(&snapshot.body, MAX_PROVIDER_CONTENT_SUMMARY_CHARS)
        ),
        trust: "untrusted_provider_content".to_string(),
    });

    // Drop the workflow's own claim comments, then keep up to the cap. Count
    // the total non-claim comments first so we can tell the model how many were
    // elided rather than silently truncating the conversation (mirrors how
    // `truncate_provider_text` marks a truncated body).
    let included: Vec<&GithubIssueCommentSnapshot> = comments
        .iter()
        .filter(|comment| {
            !comment
                .body
                .trim_start()
                .starts_with(WORKFLOW_CLAIM_COMMENT_PREFIX)
        })
        .collect();
    let total_comments = included.len();
    summaries.extend(
        included
            .iter()
            .take(MAX_PROVIDER_COMMENT_SUMMARIES)
            .map(|comment| ProviderContentSummary {
                source_ref: comment.comment.url.clone(),
                author: Some(comment.author_login.clone()),
                summary: format!(
                    "Issue comment updated at {}:\n{}",
                    comment.updated_at.to_rfc3339(),
                    truncate_provider_text(&comment.body, MAX_PROVIDER_CONTENT_SUMMARY_CHARS)
                ),
                trust: "untrusted_provider_content".to_string(),
            }),
    );
    if total_comments > MAX_PROVIDER_COMMENT_SUMMARIES {
        let dropped = total_comments - MAX_PROVIDER_COMMENT_SUMMARIES;
        summaries.push(ProviderContentSummary {
            source_ref: format!(
                "github:issue:{}/{}#{}:comments-elided",
                snapshot.owner, snapshot.repo, snapshot.number
            ),
            author: None,
            summary: format!("[{dropped} additional comments not shown]"),
            trust: "untrusted_provider_content".to_string(),
        });
    }

    summaries
}

fn truncate_provider_text(text: &str, max_chars: usize) -> String {
    let text = text.trim();
    if text.chars().count() <= max_chars {
        return text.to_string();
    }

    let truncated = text.chars().take(max_chars).collect::<String>();
    format!("{truncated}\n[truncated after {max_chars} characters]")
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

/// Whether a run has reached a terminal status. The drain loop stops re-ticking
/// a run once it is terminal (the policy itself short-circuits a terminal tick,
/// but stopping here avoids a redundant final no-op tick).
fn run_is_terminal(status: &GithubIssueWorkflowRunStatus) -> bool {
    matches!(
        status,
        GithubIssueWorkflowRunStatus::Succeeded
            | GithubIssueWorkflowRunStatus::Failed
            | GithubIssueWorkflowRunStatus::Cancelled
    )
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
    use chrono::{TimeZone, Utc};

    use super::{
        MAX_PROVIDER_COMMENT_SUMMARIES, github_search_value, open_bug_query,
        provider_content_summaries,
    };
    use crate::{
        GithubCommentRef, GithubIssueCandidateSelector, GithubIssueCommentSnapshot,
        GithubIssueProviderSnapshot, GithubRepositorySelector,
    };

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

    #[test]
    fn provider_content_summaries_marks_dropped_comments() {
        let snapshot = GithubIssueProviderSnapshot {
            owner: "near".to_string(),
            repo: "ironclaw".to_string(),
            number: 7,
            node_id: None,
            url: "https://github.com/near/ironclaw/issues/7".to_string(),
            default_branch: "main".to_string(),
            title: "Bug".to_string(),
            body: "body".to_string(),
            state: "open".to_string(),
            author_login: Some("reporter".to_string()),
            labels: vec!["bug".to_string()],
            updated_at: None,
        };
        // Two more comments than the cap so the elision marker must appear.
        let comment_count = MAX_PROVIDER_COMMENT_SUMMARIES + 2;
        let comments: Vec<GithubIssueCommentSnapshot> = (0..comment_count)
            .map(|index| GithubIssueCommentSnapshot {
                comment: GithubCommentRef {
                    node_id: Some(format!("c{index}")),
                    url: format!("https://github.com/near/ironclaw/issues/7#c{index}"),
                },
                body: format!("comment {index}"),
                author_login: "octocat".to_string(),
                created_at: Utc.timestamp_opt(1, 0).unwrap(),
                updated_at: Utc.timestamp_opt(2, 0).unwrap(),
            })
            .collect();

        let summaries = provider_content_summaries(&snapshot, &comments);

        // 1 issue + cap comments + 1 elision marker.
        assert_eq!(summaries.len(), 1 + MAX_PROVIDER_COMMENT_SUMMARIES + 1);
        let marker = summaries.last().expect("a summary is present");
        assert_eq!(marker.summary, "[2 additional comments not shown]");
        assert!(marker.source_ref.ends_with(":comments-elided"));
    }

    #[test]
    fn provider_content_summaries_has_no_marker_when_under_cap() {
        let snapshot = GithubIssueProviderSnapshot {
            owner: "near".to_string(),
            repo: "ironclaw".to_string(),
            number: 8,
            node_id: None,
            url: "https://github.com/near/ironclaw/issues/8".to_string(),
            default_branch: "main".to_string(),
            title: "Bug".to_string(),
            body: "body".to_string(),
            state: "open".to_string(),
            author_login: Some("reporter".to_string()),
            labels: Vec::new(),
            updated_at: None,
        };
        let comments = vec![GithubIssueCommentSnapshot {
            comment: GithubCommentRef {
                node_id: Some("c0".to_string()),
                url: "https://github.com/near/ironclaw/issues/8#c0".to_string(),
            },
            body: "only comment".to_string(),
            author_login: "octocat".to_string(),
            created_at: Utc.timestamp_opt(1, 0).unwrap(),
            updated_at: Utc.timestamp_opt(2, 0).unwrap(),
        }];

        let summaries = provider_content_summaries(&snapshot, &comments);

        // 1 issue + 1 comment, no elision marker.
        assert_eq!(summaries.len(), 2);
        assert!(
            summaries
                .iter()
                .all(|summary| !summary.summary.contains("additional comments not shown"))
        );
    }
}
