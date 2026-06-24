//! Reusable, feature-gated scaffolding for driving the GitHub issue workflow
//! poller end-to-end in tests.
//!
//! Compiled only under `cfg(test)` or the `test-support` feature, so it ships
//! zero bytes in production binaries. Downstream crates (e.g. the storage
//! crate's hermetic E2E) depend on this crate with `features = ["test-support"]`
//! to drive the real [`crate::GithubIssueWorkflowPoller`] over a durable
//! repository.
//!
//! ## Stage completion is injected BETWEEN ticks, not inside `submit_stage_turn`
//!
//! [`RecordingStageTurnSubmitter`] only records the submitted request — it does
//! NOT write the stage result. A scripted submitter that injected the
//! `StageCompleted` result *synchronously inside* `submit_stage_turn` would
//! bump the run row's version mid-tick (via `accept_stage_result`), which then
//! breaks the policy's own subsequent `advance_event_cursor_and_transition`
//! optimistic version CAS in the same tick. Instead, the driver calls
//! [`complete_active_stage`] between `tick_once` calls, when the run is at a
//! stable version, mirroring how a real agent reports its result out-of-band
//! before the next poll.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, TimeZone, Utc};
use serde_json::{Value as JsonValue, json};
use tokio::sync::Mutex;

use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId, UserId};
use ironclaw_turns::TurnRunId;

use crate::{
    AcceptStageResultInput, AcceptStageResultOutcome, CreateDraftPullRequestInput,
    CreateIssueCommentInput, CreateOrGetWorkflowRunInput, CreateOrGetWorkflowRunOutcome,
    GetAuthenticatedWorkflowActorInput, GetGithubIssueInput, GetPullRequestInput, GetStageRunInput,
    GithubActorSnapshot, GithubCommentRef, GithubIssueCommentSnapshot, GithubIssueProviderSnapshot,
    GithubIssueRef, GithubIssueSearchHit, GithubIssueStage, GithubIssueWorkflowConfig,
    GithubIssueWorkflowConfigSource, GithubIssueWorkflowError, GithubIssueWorkflowEventType,
    GithubIssueWorkflowPollerConfig, GithubIssueWorkflowPollerPorts, GithubIssueWorkflowPort,
    GithubIssueWorkflowRepository, GithubIssueWorkflowRun, GithubIssueWorkspaceSession,
    GithubIssueWorkspaceSessionId, GithubProviderAccountRef, GithubPullRequestCheckSnapshot,
    GithubPullRequestRef, GithubPullRequestSnapshot, GithubRepositorySelector,
    GithubReviewCommentSnapshot, ListIssueCommentsInput, ListPullRequestChecksInput,
    ListPullRequestReviewCommentsInput, ListPullRequestsInput, PrepareWorkflowWorkspaceOutcome,
    PrepareWorkflowWorkspaceRequest, PublishWorkflowWorkspaceOutcome,
    PublishWorkflowWorkspaceRequest, RecordWorkflowEventInput, SearchGithubIssuesInput,
    StageCompletedPayload, StageTurnSubmitter, SubmitStageTurnOutcome, SubmitStageTurnRequest,
    VerifyWorkflowWorkspaceOutcome, VerifyWorkflowWorkspaceRequest, WorkflowClock,
    WorkflowConfigAccessRequest, WorkflowEventEnvelope, WorkflowEventSourceKind,
    WorkflowProjectAccess, WorkflowProjectAccessRequest, WorkflowWorkerId,
    WorkflowWorkspaceManager, WorkflowWorkspaceMountRef, WorkflowWorkspaceRef, issue_binding_ref,
    stage_result_reported_key,
};

/// The default working branch the [`PublishingWorkspaceManager`] publishes.
pub const TEST_WORKING_BRANCH: &str = "ironclaw/fix-42";
/// The draft PR number the [`RecordingGithubPort`] returns by default.
pub const TEST_DRAFT_PR_NUMBER: u64 = 4242;

/// A fixed timestamp helper for deterministic tests.
pub fn fixed_time(seconds: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(seconds, 0)
        .single()
        .expect("valid timestamp")
}

/// A provider account ref so claim-path ops fail-closed checks are satisfied.
pub fn provider_account_ref() -> GithubProviderAccountRef {
    GithubProviderAccountRef {
        provider: "github".to_string(),
        account_id: "github-test-support".to_string(),
    }
}

/// The schema version string for a stage's result envelope.
pub fn schema_version(stage: &GithubIssueStage) -> &'static str {
    match stage {
        GithubIssueStage::Triage => "triage.v1",
        GithubIssueStage::Planning => "planning.v1",
        GithubIssueStage::Implementation => "implementation.v1",
        GithubIssueStage::PrSynthesis => "pr_synthesis.v1",
        GithubIssueStage::CiRepair => "ci_repair.v1",
        GithubIssueStage::ReviewResponse => "review_response.v1",
    }
}

/// A schema-valid stage result envelope for each stage. The payloads satisfy
/// the per-stage required fields enforced by `validate_stage_result`.
pub fn stage_result(stage: &GithubIssueStage) -> JsonValue {
    let payload = match stage {
        GithubIssueStage::Triage => json!({
            "is_reproducible": true,
            "suspected_area": "the failing module",
            "risk": "low",
            "recommended_next_stage": "planning"
        }),
        GithubIssueStage::Planning => json!({
            "plan_items": ["inspect the failing path", "add a regression test"],
            "files_to_inspect_or_change": ["src/lib.rs"],
            "test_strategy": "unit test reproducing the bug then asserting the fix",
            "confidence": 0.8
        }),
        GithubIssueStage::Implementation => json!({
            "changed_files": ["src/lib.rs"],
            "commands_run": ["cargo test"],
            "test_evidence": ["tests passed"],
            "pr_ready": true
        }),
        GithubIssueStage::PrSynthesis => json!({
            "title": "Fix bug 42",
            "body": "This fixes bug 42.",
            "branch_name": TEST_WORKING_BRANCH,
            "base_branch": "main",
            "head_sha": "head-sha-42"
        }),
        GithubIssueStage::CiRepair => json!({
            "failing_checks": ["clippy"],
            "diagnosis": "fixed the lint",
            "changed_files": ["src/lib.rs"],
            "commands_run": ["cargo clippy"]
        }),
        GithubIssueStage::ReviewResponse => json!({
            "addressed_comments": ["comment-node-1"],
            "remaining_comments": [],
            "commands_run": ["cargo test"]
        }),
    };
    json!({
        "outcome": "completed",
        "summary": "stage completed",
        "evidence": [],
        "next_actions": [],
        "payload": payload
    })
}

/// Build a single-repository, single-account workflow config.
pub fn workflow_config(owner: &str, repo: &str) -> GithubIssueWorkflowConfig {
    GithubIssueWorkflowConfig {
        tenant_id: TenantId::new("tenant-test-support").expect("tenant"),
        project_id: ProjectId::new("project-test-support").expect("project"),
        owner_user_id: UserId::new("user-test-support").expect("user"),
        repositories: vec![GithubRepositorySelector {
            owner: owner.to_string(),
            repo: repo.to_string(),
        }],
        candidate_selector: Default::default(),
        max_active_runs_per_repo: 1,
        default_run_profile: "default".to_string(),
        provider_account_ref: provider_account_ref(),
    }
}

/// A discoverable open bug issue snapshot for the given repository.
pub fn issue_snapshot(owner: &str, repo: &str, number: u64) -> GithubIssueProviderSnapshot {
    GithubIssueProviderSnapshot {
        owner: owner.to_string(),
        repo: repo.to_string(),
        number,
        node_id: Some(format!("issue-node-{repo}-{number}")),
        url: format!("https://github.com/{owner}/{repo}/issues/{number}"),
        default_branch: "main".to_string(),
        title: format!("Bug {number}"),
        body: format!("body for issue {number}"),
        state: "open".to_string(),
        author_login: Some("core-dev".to_string()),
        labels: vec!["bug".to_string()],
        updated_at: Some(fixed_time(100)),
    }
}

fn issue_hit(snapshot: &GithubIssueProviderSnapshot) -> GithubIssueSearchHit {
    GithubIssueSearchHit {
        owner: snapshot.owner.clone(),
        repo: snapshot.repo.clone(),
        number: snapshot.number,
        node_id: snapshot.node_id.clone(),
        url: snapshot.url.clone(),
        default_branch: snapshot.default_branch.clone(),
        updated_at: snapshot.updated_at,
    }
}

/// A settable, deterministic clock.
#[derive(Debug)]
pub struct TestClock {
    now: std::sync::Mutex<DateTime<Utc>>,
}

impl TestClock {
    pub fn new(now: DateTime<Utc>) -> Self {
        Self {
            now: std::sync::Mutex::new(now),
        }
    }

    pub fn set(&self, now: DateTime<Utc>) {
        *self.now.lock().expect("clock lock") = now;
    }
}

impl WorkflowClock for TestClock {
    fn now(&self) -> DateTime<Utc> {
        *self.now.lock().expect("clock lock")
    }
}

/// Lists a fixed set of enabled workflow configs.
#[derive(Debug)]
pub struct TestConfigSource {
    configs: Vec<GithubIssueWorkflowConfig>,
}

impl TestConfigSource {
    pub fn new(configs: Vec<GithubIssueWorkflowConfig>) -> Self {
        Self { configs }
    }
}

#[async_trait]
impl GithubIssueWorkflowConfigSource for TestConfigSource {
    async fn list_enabled_workflow_configs(
        &self,
    ) -> Result<Vec<GithubIssueWorkflowConfig>, GithubIssueWorkflowError> {
        Ok(self.configs.clone())
    }
}

/// A GitHub port that serves seeded issues and records created draft PRs.
#[derive(Debug, Default)]
pub struct RecordingGithubPort {
    search_results: Mutex<HashMap<(String, String), Vec<GithubIssueSearchHit>>>,
    issue_snapshots: Mutex<HashMap<(String, String, u64), GithubIssueProviderSnapshot>>,
    created_prs: Mutex<Vec<CreateDraftPullRequestInput>>,
}

impl RecordingGithubPort {
    pub fn new() -> Self {
        Self::default()
    }

    /// Seed a discoverable open bug issue (search hit + get_issue snapshot).
    pub async fn seed_issue(&self, snapshot: GithubIssueProviderSnapshot) {
        let key = (snapshot.owner.clone(), snapshot.repo.clone());
        self.search_results
            .lock()
            .await
            .entry(key)
            .or_default()
            .push(issue_hit(&snapshot));
        self.issue_snapshots.lock().await.insert(
            (
                snapshot.owner.clone(),
                snapshot.repo.clone(),
                snapshot.number,
            ),
            snapshot,
        );
    }

    /// The draft PRs created via `create_draft_pull_request`, in order.
    pub async fn created_prs(&self) -> Vec<CreateDraftPullRequestInput> {
        self.created_prs.lock().await.clone()
    }
}

#[async_trait]
impl GithubIssueWorkflowPort for RecordingGithubPort {
    async fn search_open_bug_issues(
        &self,
        input: SearchGithubIssuesInput,
    ) -> Result<Vec<GithubIssueSearchHit>, GithubIssueWorkflowError> {
        Ok(self
            .search_results
            .lock()
            .await
            .get(&(input.owner, input.repo))
            .cloned()
            .unwrap_or_default())
    }

    async fn get_issue(
        &self,
        input: GetGithubIssueInput,
    ) -> Result<GithubIssueProviderSnapshot, GithubIssueWorkflowError> {
        self.issue_snapshots
            .lock()
            .await
            .get(&(input.owner, input.repo, input.number))
            .cloned()
            .ok_or_else(|| GithubIssueWorkflowError::ProviderRead {
                reason: "missing seeded issue snapshot".to_string(),
            })
    }

    async fn get_authenticated_workflow_actor(
        &self,
        _input: GetAuthenticatedWorkflowActorInput,
    ) -> Result<GithubActorSnapshot, GithubIssueWorkflowError> {
        Ok(GithubActorSnapshot {
            login: "ironclaw-bot".to_string(),
            node_id: Some("actor-node-test-support".to_string()),
        })
    }

    async fn list_issue_comments(
        &self,
        _input: ListIssueCommentsInput,
    ) -> Result<Vec<GithubIssueCommentSnapshot>, GithubIssueWorkflowError> {
        Ok(Vec::new())
    }

    async fn create_issue_comment(
        &self,
        _input: CreateIssueCommentInput,
    ) -> Result<GithubCommentRef, GithubIssueWorkflowError> {
        Ok(GithubCommentRef {
            node_id: Some("created-comment-node-test-support".to_string()),
            url: "https://github.com/test/test/issues/42#issuecomment-created".to_string(),
        })
    }

    async fn list_pull_requests(
        &self,
        _input: ListPullRequestsInput,
    ) -> Result<Vec<GithubPullRequestSnapshot>, GithubIssueWorkflowError> {
        Ok(Vec::new())
    }

    async fn get_pull_request(
        &self,
        input: GetPullRequestInput,
    ) -> Result<GithubPullRequestSnapshot, GithubIssueWorkflowError> {
        // Return a benign open draft snapshot so a PrOpen run's poll-phase PR
        // refresh (after restart/replay) succeeds and the run stays PrOpen
        // rather than escalating or erroring.
        Ok(GithubPullRequestSnapshot {
            pull_request: GithubPullRequestRef {
                owner: input.owner,
                repo: input.repo,
                number: input.number,
                node_id: Some(format!("pr-node-{}", input.number)),
                url: format!("https://github.com/test/test/pull/{}", input.number),
                head_branch: TEST_WORKING_BRANCH.to_string(),
                head_sha: Some("head-sha-42".to_string()),
            },
            title: "Fix bug 42".to_string(),
            body: "Draft body".to_string(),
            state: "open".to_string(),
            draft: true,
            merged: false,
            updated_at: Some(fixed_time(100)),
        })
    }

    async fn list_pull_request_checks(
        &self,
        _input: ListPullRequestChecksInput,
    ) -> Result<Vec<GithubPullRequestCheckSnapshot>, GithubIssueWorkflowError> {
        Ok(Vec::new())
    }

    async fn list_pull_request_review_comments(
        &self,
        _input: ListPullRequestReviewCommentsInput,
    ) -> Result<Vec<GithubReviewCommentSnapshot>, GithubIssueWorkflowError> {
        Ok(Vec::new())
    }

    async fn create_draft_pull_request(
        &self,
        input: CreateDraftPullRequestInput,
    ) -> Result<GithubPullRequestRef, GithubIssueWorkflowError> {
        let head_branch = input.head_branch.clone();
        let owner = input.owner.clone();
        let repo = input.repo.clone();
        self.created_prs.lock().await.push(input);
        Ok(GithubPullRequestRef {
            owner,
            repo,
            number: TEST_DRAFT_PR_NUMBER,
            node_id: Some(format!("pr-node-{TEST_DRAFT_PR_NUMBER}")),
            url: format!("https://github.com/test/test/pull/{TEST_DRAFT_PR_NUMBER}"),
            head_branch,
            head_sha: Some("head-sha-42".to_string()),
        })
    }
}

/// Allows every config and run; the hermetic E2E is not exercising the
/// project-access gate (covered by composition tests).
#[derive(Debug, Default)]
pub struct AllowAllProjectAccess;

#[async_trait]
impl WorkflowProjectAccess for AllowAllProjectAccess {
    async fn assert_workflow_config_access(
        &self,
        _request: WorkflowConfigAccessRequest,
    ) -> Result<(), GithubIssueWorkflowError> {
        Ok(())
    }

    async fn assert_workflow_project_access(
        &self,
        _request: WorkflowProjectAccessRequest,
    ) -> Result<(), GithubIssueWorkflowError> {
        Ok(())
    }
}

/// A workspace manager that prepares a working branch and reports changes to
/// publish (so the PrSynthesis stage opens a draft PR).
#[derive(Debug, Default)]
pub struct PublishingWorkspaceManager;

#[async_trait]
impl WorkflowWorkspaceManager for PublishingWorkspaceManager {
    async fn prepare_workspace(
        &self,
        request: PrepareWorkflowWorkspaceRequest,
    ) -> Result<PrepareWorkflowWorkspaceOutcome, GithubIssueWorkflowError> {
        let workspace_session_id = GithubIssueWorkspaceSessionId::from_trusted(
            "workspace-session-test-support".to_string(),
        )
        .expect("workspace session id");
        Ok(PrepareWorkflowWorkspaceOutcome {
            session: GithubIssueWorkspaceSession {
                workspace_session_id: workspace_session_id.clone(),
                workflow_run_id: request.workflow_run_id,
                repository: GithubRepositorySelector {
                    owner: request.issue.owner,
                    repo: request.issue.repo,
                },
                base_branch: request.base_branch,
                base_sha: None,
                working_branch: TEST_WORKING_BRANCH.to_string(),
                current_head_sha: Some("head-sha-42".to_string()),
                workspace_ref: WorkflowWorkspaceRef {
                    thread_id: Some(
                        ThreadId::new("workspace-thread-test-support").expect("thread"),
                    ),
                    workspace_session_id: Some(workspace_session_id),
                    turn_run_id: Some(TurnRunId::new()),
                },
                mount_ref: WorkflowWorkspaceMountRef {
                    mount_id: "workspace-mount-test-support".to_string(),
                    alias: "/workspace".to_string(),
                },
                created_at: request.requested_at,
            },
        })
    }

    async fn publish_workspace(
        &self,
        request: PublishWorkflowWorkspaceRequest,
    ) -> Result<PublishWorkflowWorkspaceOutcome, GithubIssueWorkflowError> {
        Ok(PublishWorkflowWorkspaceOutcome {
            working_branch: TEST_WORKING_BRANCH.to_string(),
            base_branch: request.base_branch,
            head_sha: "head-sha-42".to_string(),
            has_changes: true,
        })
    }

    async fn verify_workspace(
        &self,
        _request: VerifyWorkflowWorkspaceRequest,
    ) -> Result<VerifyWorkflowWorkspaceOutcome, GithubIssueWorkflowError> {
        Ok(VerifyWorkflowWorkspaceOutcome {
            ran: true,
            passed: true,
            exit_code: Some(0),
            command_label: "test-support verification".to_string(),
            stdout_tail: String::new(),
            stderr_tail: String::new(),
        })
    }
}

/// Records submitted stage turns but does NOT report results. The driver
/// reports results out-of-band via [`complete_active_stage`] (see module docs
/// for why synchronous injection would be incorrect).
#[derive(Debug, Default)]
pub struct RecordingStageTurnSubmitter {
    requests: Mutex<Vec<SubmitStageTurnRequest>>,
}

impl RecordingStageTurnSubmitter {
    pub async fn request_count(&self) -> usize {
        self.requests.lock().await.len()
    }
}

#[async_trait]
impl StageTurnSubmitter for RecordingStageTurnSubmitter {
    async fn submit_stage_turn(
        &self,
        request: SubmitStageTurnRequest,
    ) -> Result<SubmitStageTurnOutcome, GithubIssueWorkflowError> {
        let count = {
            let mut requests = self.requests.lock().await;
            requests.push(request);
            requests.len()
        };
        Ok(SubmitStageTurnOutcome::Submitted {
            thread_id: ThreadId::new(format!("thread-test-support-{count}")).expect("thread id"),
            turn_run_id: TurnRunId::new(),
        })
    }
}

/// Poller ports generic over the repository, so the same scaffolding drives
/// either an in-memory repository or a durable `Arc<dyn ...>` (e.g. libSQL).
pub struct TestPollerPorts<R: GithubIssueWorkflowRepository + ?Sized> {
    pub repository: Arc<R>,
    pub config_source: Arc<TestConfigSource>,
    pub github: Arc<RecordingGithubPort>,
    pub project_access: Arc<AllowAllProjectAccess>,
    pub stage_turns: Arc<RecordingStageTurnSubmitter>,
    pub workspace: Arc<PublishingWorkspaceManager>,
    pub clock: Arc<TestClock>,
    pub worker_id: WorkflowWorkerId,
}

impl<R: GithubIssueWorkflowRepository + ?Sized> TestPollerPorts<R> {
    /// Build poller ports over `repository`, listing `configs`, serving issues
    /// from `github`. Reuse the same `github` instance across a restart to
    /// accumulate created-PR assertions.
    pub fn new(
        repository: Arc<R>,
        configs: Vec<GithubIssueWorkflowConfig>,
        github: Arc<RecordingGithubPort>,
    ) -> Self {
        Self {
            repository,
            config_source: Arc::new(TestConfigSource::new(configs)),
            github,
            project_access: Arc::new(AllowAllProjectAccess),
            stage_turns: Arc::new(RecordingStageTurnSubmitter::default()),
            workspace: Arc::new(PublishingWorkspaceManager),
            clock: Arc::new(TestClock::new(fixed_time(1000))),
            worker_id: WorkflowWorkerId::from_trusted("test-support-worker".to_string())
                .expect("worker id"),
        }
    }
}

impl<R: GithubIssueWorkflowRepository + ?Sized> GithubIssueWorkflowPollerPorts
    for TestPollerPorts<R>
{
    type Clock = TestClock;
    type ConfigSource = TestConfigSource;
    type GithubPort = RecordingGithubPort;
    type ProjectAccess = AllowAllProjectAccess;
    type Repository = R;
    type StageTurnSubmitter = RecordingStageTurnSubmitter;
    type WorkspaceManager = PublishingWorkspaceManager;

    fn clock(&self) -> Arc<Self::Clock> {
        self.clock.clone()
    }

    fn config_source(&self) -> Arc<Self::ConfigSource> {
        self.config_source.clone()
    }

    fn github_port(&self) -> Arc<Self::GithubPort> {
        self.github.clone()
    }

    fn project_access(&self) -> Arc<Self::ProjectAccess> {
        self.project_access.clone()
    }

    fn repository(&self) -> Arc<Self::Repository> {
        self.repository.clone()
    }

    fn stage_turn_submitter(&self) -> Arc<Self::StageTurnSubmitter> {
        self.stage_turns.clone()
    }

    fn workspace_manager(&self) -> Arc<Self::WorkspaceManager> {
        self.workspace.clone()
    }

    fn worker_id(&self) -> WorkflowWorkerId {
        self.worker_id.clone()
    }
}

/// A poller config with a generous `stage_stale_after` so the stuck-stage
/// reconciler never fires during a hermetic run.
pub fn poller_config() -> GithubIssueWorkflowPollerConfig {
    GithubIssueWorkflowPollerConfig {
        enabled: true,
        stage_stale_after: std::time::Duration::from_secs(86_400),
        ..GithubIssueWorkflowPollerConfig::default()
    }
}

/// Read the current workflow run for `issue` (idempotent get of the run the
/// poller created on discovery).
pub async fn load_run<R: GithubIssueWorkflowRepository + ?Sized>(
    repository: &R,
    config: &GithubIssueWorkflowConfig,
    issue: &GithubIssueRef,
    workflow_policy_version: &str,
    now: DateTime<Utc>,
) -> GithubIssueWorkflowRun {
    match repository
        .create_or_get_workflow_run(CreateOrGetWorkflowRunInput {
            tenant_id: config.tenant_id.clone(),
            creator_user_id: config.owner_user_id.clone(),
            agent_id: Some(AgentId::new("agent-test-support").expect("agent")),
            project_id: Some(config.project_id.clone()),
            provider_account_ref: Some(config.provider_account_ref.clone()),
            issue_ref: issue.clone(),
            workflow_policy_key: "github-bug-workflow".to_string(),
            workflow_policy_version: workflow_policy_version.to_string(),
            now,
        })
        .await
        .expect("load workflow run")
    {
        CreateOrGetWorkflowRunOutcome::Created { run }
        | CreateOrGetWorkflowRunOutcome::Existing { run } => run,
    }
}

/// Report a schema-valid result for the run's currently active stage, mirroring
/// what a real agent's `report_stage_result` would persist. Called BETWEEN
/// `tick_once` calls so it never mutates the run version mid-tick.
pub async fn complete_active_stage<R: GithubIssueWorkflowRepository + ?Sized>(
    repository: &R,
    run: &GithubIssueWorkflowRun,
    now: DateTime<Utc>,
) {
    let stage_run_id = run
        .active_stage_run_id
        .clone()
        .expect("run has an active stage to complete");
    let snapshot = repository
        .get_stage_run(GetStageRunInput {
            workflow_run_id: run.workflow_run_id.clone(),
            stage_run_id: stage_run_id.clone(),
        })
        .await
        .expect("get_stage_run")
        .expect("active stage row exists");
    let stage = snapshot.stage;
    let result = stage_result(&stage);

    match repository
        .accept_stage_result(AcceptStageResultInput {
            workflow_run_id: run.workflow_run_id.clone(),
            stage_run_id: stage_run_id.clone(),
            result: result.clone(),
            now,
        })
        .await
        .expect("accept_stage_result")
    {
        AcceptStageResultOutcome::Accepted { .. }
        | AcceptStageResultOutcome::NotActiveStage { .. } => {}
        AcceptStageResultOutcome::Terminal => {
            panic!("run unexpectedly terminal while completing stage {stage:?}")
        }
    }

    repository
        .record_workflow_event(RecordWorkflowEventInput {
            workflow_run_id: run.workflow_run_id.clone(),
            workflow_event_type: GithubIssueWorkflowEventType::StageCompleted,
            envelope: WorkflowEventEnvelope {
                source_kind: WorkflowEventSourceKind::WorkflowInternal,
                source_delivery_id: None,
                provider: issue_binding_ref(&run.issue_ref).provider_ref,
                observed_at: now,
                provider_updated_at: None,
                idempotency_key: stage_result_reported_key(&stage_run_id, schema_version(&stage)),
                payload_schema: "stage.completed.v1".to_string(),
                payload: serde_json::to_value(StageCompletedPayload {
                    stage_run_id,
                    stage: stage.clone(),
                    schema_version: schema_version(&stage).to_string(),
                    result,
                })
                .expect("serialize stage completed payload"),
            },
        })
        .await
        .expect("record stage completed event");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        CreateStageRunInput, CreateStageRunOutcome, InMemoryGithubIssueWorkflowRepository,
        ListWorkflowEventsAfterInput,
    };

    #[tokio::test]
    async fn complete_active_stage_records_stage_result_into_repository() {
        let repository = InMemoryGithubIssueWorkflowRepository::default();
        let config = workflow_config("nearai", "ironclaw");
        let snapshot = issue_snapshot("nearai", "ironclaw", 42);
        let issue = snapshot.issue_ref();

        // Create a run with an active Triage stage, as the poller would.
        let run = load_run(
            &repository,
            &config,
            &issue,
            "test-support-v1",
            fixed_time(1000),
        )
        .await;
        let run = match repository
            .create_stage_run(CreateStageRunInput {
                workflow_run_id: run.workflow_run_id.clone(),
                stage: GithubIssueStage::Triage,
                now: fixed_time(1000),
            })
            .await
            .expect("create stage run")
        {
            CreateStageRunOutcome::Created { run, .. }
            | CreateStageRunOutcome::ActiveStageExists { run, .. } => run,
            CreateStageRunOutcome::Terminal => panic!("new run should not be terminal"),
        };
        assert!(run.active_stage_run_id.is_some());

        // Drive the helper through its real interface.
        complete_active_stage(&repository, &run, fixed_time(2000)).await;

        let events = repository
            .list_workflow_events_after(ListWorkflowEventsAfterInput {
                workflow_run_id: run.workflow_run_id.clone(),
                after_sequence: 0,
                limit: 10,
            })
            .await
            .expect("list events");
        assert!(
            events
                .iter()
                .any(|event| event.workflow_event_type
                    == GithubIssueWorkflowEventType::StageCompleted),
            "complete_active_stage must record a StageCompleted event"
        );

        // The accepted stage row is now inactive.
        let stage = repository
            .get_stage_run(GetStageRunInput {
                workflow_run_id: run.workflow_run_id.clone(),
                stage_run_id: run.active_stage_run_id.clone().expect("active stage"),
            })
            .await
            .expect("get stage run")
            .expect("stage row exists");
        assert!(!stage.active);
    }
}
