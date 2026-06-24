mod pr_lifecycle_contract {
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex as StdMutex};

    use async_trait::async_trait;
    use chrono::{TimeZone, Utc};
    use ironclaw_github_issue_workflow::{
        AcceptStageResultInput, AcceptStageResultOutcome, CreateDraftPullRequestInput,
        CreateIssueCommentInput, CreateOrGetWorkflowRunInput, CreateOrGetWorkflowRunOutcome,
        GetAuthenticatedWorkflowActorInput, GithubActorSnapshot, GithubChecksChangedPayload,
        GithubCommentRef, GithubIssueClosedPayload, GithubIssueCommentSnapshot, GithubIssueRef,
        GithubIssueStage, GithubIssueWorkflowError, GithubIssueWorkflowEventType,
        GithubIssueWorkflowMode, GithubIssueWorkflowPolicy, GithubIssueWorkflowPolicyPorts,
        GithubIssueWorkflowPort, GithubIssueWorkflowRepository, GithubIssueWorkflowRun,
        GithubIssueWorkflowRunStatus, GithubIssueWorkspaceSession, GithubIssueWorkspaceSessionId,
        GithubProviderAccountRef, GithubProviderRef, GithubPullRequestRef,
        GithubPullRequestSnapshot, GithubPullRequestUpdatedPayload, GithubRepositorySelector,
        GithubReviewCommentCreatedPayload, InMemoryGithubIssueWorkflowRepository,
        ListIssueCommentsInput, ListPullRequestsInput, PrepareWorkflowWorkspaceOutcome,
        PrepareWorkflowWorkspaceRequest, ProviderActionKind, ProviderActionReconciliationStrategy,
        ProviderActionRunOutcome, ProviderActionStatus, RecordWorkflowEventInput,
        RecordWorkflowEventOutcome, RunDraftPullRequestProviderActionRequest,
        StageCompletedPayload, StageTurnSubmitter, SubmitStageTurnOutcome, SubmitStageTurnRequest,
        WorkflowClock, WorkflowEventEnvelope, WorkflowEventSourceKind, WorkflowProjectAccess,
        WorkflowProjectAccessRequest, WorkflowRunTransition, WorkflowWorkerId,
        WorkflowWorkspaceManager, WorkflowWorkspaceMountRef, WorkflowWorkspaceRef,
        checks_failed_key, issue_binding_ref, issue_closed_key, review_comment_created_key,
        stable_pr_marker, stage_result_reported_key,
    };
    use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId, UserId};
    use ironclaw_turns::TurnRunId;
    use serde_json::{Value as JsonValue, json};
    use tokio::sync::Mutex;

    fn fixed_time(seconds: i64) -> chrono::DateTime<Utc> {
        Utc.timestamp_opt(seconds, 0).unwrap()
    }

    fn tenant() -> TenantId {
        TenantId::new("tenant-pr-lifecycle").unwrap()
    }

    fn user() -> UserId {
        UserId::new("user-pr-lifecycle").unwrap()
    }

    fn agent() -> AgentId {
        AgentId::new("agent-pr-lifecycle").unwrap()
    }

    fn project() -> ProjectId {
        ProjectId::new("project-pr-lifecycle").unwrap()
    }

    fn worker() -> WorkflowWorkerId {
        WorkflowWorkerId::from_trusted("worker-pr-lifecycle".to_string()).unwrap()
    }

    fn provider_account_ref() -> GithubProviderAccountRef {
        GithubProviderAccountRef {
            provider: "github".to_string(),
            account_id: "github-pr-lifecycle".to_string(),
        }
    }

    fn issue() -> GithubIssueRef {
        GithubIssueRef {
            owner: "nearai".to_string(),
            repo: "ironclaw".to_string(),
            number: 42,
            node_id: Some("issue-node-42".to_string()),
            url: "https://github.com/nearai/ironclaw/issues/42".to_string(),
            default_branch: "main".to_string(),
        }
    }

    fn provider_ref(issue: &GithubIssueRef) -> GithubProviderRef {
        issue_binding_ref(issue).provider_ref
    }

    fn pr(number: u64, marker: Option<&str>) -> GithubPullRequestSnapshot {
        GithubPullRequestSnapshot {
            pull_request: GithubPullRequestRef {
                owner: "nearai".to_string(),
                repo: "ironclaw".to_string(),
                number,
                node_id: Some(format!("pr-node-{number}")),
                url: format!("https://github.com/nearai/ironclaw/pull/{number}"),
                head_branch: "ironclaw/fix-42".to_string(),
                head_sha: Some("head-sha-42".to_string()),
            },
            title: format!("Fix bug {number}"),
            body: marker
                .map(|marker| format!("{marker}\nDraft body"))
                .unwrap_or_else(|| "Draft body".to_string()),
            state: "open".to_string(),
            draft: true,
            merged: false,
            updated_at: Some(fixed_time(40)),
        }
    }

    fn stage_result(stage: GithubIssueStage) -> JsonValue {
        match stage {
            GithubIssueStage::Implementation => json!({
                "outcome": "completed",
                "summary": "implementation completed",
                "evidence": [],
                "next_actions": [],
                "payload": {
                    "changed_files": ["src/lib.rs"],
                    "commands_run": ["cargo test"],
                    "test_evidence": ["tests passed"],
                    "pr_ready": true
                }
            }),
            GithubIssueStage::PrSynthesis => json!({
                "outcome": "completed",
                "summary": "pr synthesized",
                "evidence": [],
                "next_actions": [],
                "payload": {
                    "title": "Fix bug 42",
                    "body": "This fixes bug 42.",
                    "branch_name": "ironclaw/fix-42",
                    "base_branch": "main",
                    "head_sha": "head-sha-42"
                }
            }),
            GithubIssueStage::CiRepair => json!({
                "outcome": "completed",
                "summary": "ci repaired",
                "evidence": [],
                "next_actions": [],
                "payload": {
                    "failing_checks": ["clippy"],
                    "diagnosis": "fixed",
                    "changed_files": ["src/lib.rs"],
                    "commands_run": ["cargo test"]
                }
            }),
            GithubIssueStage::ReviewResponse => json!({
                "outcome": "completed",
                "summary": "review addressed",
                "evidence": [],
                "next_actions": [],
                "payload": {
                    "addressed_comments": ["comment-node-1"],
                    "remaining_comments": [],
                    "commands_run": ["cargo test"]
                }
            }),
            _ => json!({
                "outcome": "completed",
                "summary": "stage completed",
                "evidence": [],
                "next_actions": [],
                "payload": {}
            }),
        }
    }

    fn schema_version(stage: GithubIssueStage) -> &'static str {
        match stage {
            GithubIssueStage::Triage => "triage.v1",
            GithubIssueStage::Planning => "planning.v1",
            GithubIssueStage::Implementation => "implementation.v1",
            GithubIssueStage::PrSynthesis => "pr_synthesis.v1",
            GithubIssueStage::CiRepair => "ci_repair.v1",
            GithubIssueStage::ReviewResponse => "review_response.v1",
        }
    }

    async fn create_claimed_run(
        repository: &InMemoryGithubIssueWorkflowRepository,
    ) -> GithubIssueWorkflowRun {
        let run = match repository
            .create_or_get_workflow_run(CreateOrGetWorkflowRunInput {
                tenant_id: tenant(),
                creator_user_id: user(),
                agent_id: Some(agent()),
                project_id: Some(project()),
                provider_account_ref: Some(provider_account_ref()),
                issue_ref: issue(),
                workflow_policy_key: "github-bug-workflow".to_string(),
                workflow_policy_version: "2026-06-22".to_string(),
                now: fixed_time(10),
            })
            .await
            .unwrap()
        {
            CreateOrGetWorkflowRunOutcome::Created { run }
            | CreateOrGetWorkflowRunOutcome::Existing { run } => run,
        };

        repository
            .claim_runnable_workflow_runs(
                ironclaw_github_issue_workflow::ClaimRunnableWorkflowRunsInput {
                    tenant_id: tenant(),
                    worker_id: worker(),
                    now: fixed_time(11),
                    lease_expires_at: fixed_time(71),
                    limit: 1,
                },
            )
            .await
            .unwrap()
            .pop()
            .unwrap_or(run)
    }

    async fn set_mode(
        repository: &InMemoryGithubIssueWorkflowRepository,
        run: GithubIssueWorkflowRun,
        mode: GithubIssueWorkflowMode,
        primary_pr: Option<GithubPullRequestRef>,
    ) -> GithubIssueWorkflowRun {
        let outcome = repository
            .advance_event_cursor_and_transition(
                ironclaw_github_issue_workflow::AdvanceWorkflowRunInput {
                    workflow_run_id: run.workflow_run_id.clone(),
                    worker_id: worker(),
                    expected_workflow_run_version: run.workflow_run_version,
                    expected_event_cursor: run.event_cursor,
                    next_event_cursor: run.event_cursor,
                    transition: WorkflowRunTransition {
                        mode: Some(mode),
                        primary_pr,
                        clear_active_block: true,
                        ..WorkflowRunTransition::default()
                    },
                    now: fixed_time(20),
                },
            )
            .await
            .unwrap();
        match outcome {
            ironclaw_github_issue_workflow::TransitionOutcome::Applied { run } => run,
            other => panic!("mode transition should apply: {other:?}"),
        }
    }

    /// Attach a prepared workspace session to a run, as the Implementation
    /// transition does in a real run. Required before PrSynthesis so the
    /// publish-workspace step has a session to push.
    async fn attach_workspace_session(
        repository: &InMemoryGithubIssueWorkflowRepository,
        run: GithubIssueWorkflowRun,
    ) -> GithubIssueWorkflowRun {
        let workspace_session_id =
            GithubIssueWorkspaceSessionId::from_trusted("workspace-session-pr".to_string())
                .unwrap();
        let session = GithubIssueWorkspaceSession {
            workspace_session_id: workspace_session_id.clone(),
            workflow_run_id: run.workflow_run_id.clone(),
            repository: GithubRepositorySelector {
                owner: run.issue_ref.owner.clone(),
                repo: run.issue_ref.repo.clone(),
            },
            base_branch: run.issue_ref.default_branch.clone(),
            base_sha: None,
            working_branch: "ironclaw/fix-42".to_string(),
            current_head_sha: Some("head-sha-42".to_string()),
            workspace_ref: WorkflowWorkspaceRef {
                thread_id: None,
                workspace_session_id: Some(workspace_session_id),
                turn_run_id: None,
            },
            mount_ref: WorkflowWorkspaceMountRef {
                mount_id: "workspace-mount-pr".to_string(),
                alias: "/workspace".to_string(),
            },
            created_at: fixed_time(20),
        };
        let outcome = repository
            .advance_event_cursor_and_transition(
                ironclaw_github_issue_workflow::AdvanceWorkflowRunInput {
                    workflow_run_id: run.workflow_run_id.clone(),
                    worker_id: worker(),
                    expected_workflow_run_version: run.workflow_run_version,
                    expected_event_cursor: run.event_cursor,
                    next_event_cursor: run.event_cursor,
                    transition: WorkflowRunTransition {
                        workspace_session: Some(session),
                        ..WorkflowRunTransition::default()
                    },
                    now: fixed_time(21),
                },
            )
            .await
            .unwrap();
        match outcome {
            ironclaw_github_issue_workflow::TransitionOutcome::Applied { run } => run,
            other => panic!("workspace session transition should apply: {other:?}"),
        }
    }

    async fn record_stage_completed(
        repository: &InMemoryGithubIssueWorkflowRepository,
        run: &GithubIssueWorkflowRun,
        stage: GithubIssueStage,
    ) -> GithubIssueWorkflowRun {
        let stage_run_id = run.active_stage_run_id.clone().unwrap_or_default();
        let result = stage_result(stage.clone());
        let accepted_run = match repository
            .accept_stage_result(AcceptStageResultInput {
                workflow_run_id: run.workflow_run_id.clone(),
                stage_run_id: stage_run_id.clone(),
                result: result.clone(),
                now: fixed_time(21),
            })
            .await
            .unwrap()
        {
            AcceptStageResultOutcome::Accepted { run }
            | AcceptStageResultOutcome::NotActiveStage { run } => run,
            AcceptStageResultOutcome::Terminal => run.clone(),
        };
        let outcome = repository
            .record_workflow_event(RecordWorkflowEventInput {
                workflow_run_id: run.workflow_run_id.clone(),
                workflow_event_type: GithubIssueWorkflowEventType::StageCompleted,
                envelope: WorkflowEventEnvelope {
                    source_kind: WorkflowEventSourceKind::WorkflowInternal,
                    source_delivery_id: None,
                    provider: provider_ref(&run.issue_ref),
                    observed_at: fixed_time(22),
                    provider_updated_at: None,
                    idempotency_key: stage_result_reported_key(
                        &stage_run_id,
                        schema_version(stage.clone()),
                    ),
                    payload_schema: "stage.completed.v1".to_string(),
                    payload: serde_json::to_value(StageCompletedPayload {
                        stage_run_id,
                        stage: stage.clone(),
                        schema_version: schema_version(stage).to_string(),
                        result,
                    })
                    .unwrap(),
                },
            })
            .await;
        let outcome = outcome.unwrap();
        assert!(matches!(
            outcome,
            RecordWorkflowEventOutcome::Recorded { .. }
        ));
        accepted_run
    }

    async fn record_pr_updated(
        repository: &InMemoryGithubIssueWorkflowRepository,
        run: &GithubIssueWorkflowRun,
        pull_request: GithubPullRequestRef,
        state: &str,
        merged: bool,
    ) {
        repository
            .record_workflow_event(RecordWorkflowEventInput {
                workflow_run_id: run.workflow_run_id.clone(),
                workflow_event_type: GithubIssueWorkflowEventType::GithubPullRequestUpdated,
                envelope: WorkflowEventEnvelope {
                    source_kind: WorkflowEventSourceKind::Poller,
                    source_delivery_id: None,
                    provider: provider_ref(&run.issue_ref),
                    observed_at: fixed_time(30),
                    provider_updated_at: Some(fixed_time(30)),
                    idempotency_key: ironclaw_github_issue_workflow::pr_updated_key(
                        &pull_request,
                        Some(fixed_time(30)),
                    ),
                    payload_schema: "github.pr.updated.v1".to_string(),
                    payload: serde_json::to_value(GithubPullRequestUpdatedPayload {
                        pull_request,
                        state: state.to_string(),
                        merged,
                        draft: false,
                    })
                    .unwrap(),
                },
            })
            .await
            .unwrap();
    }

    async fn record_checks_failed(
        repository: &InMemoryGithubIssueWorkflowRepository,
        run: &GithubIssueWorkflowRun,
        pull_request: GithubPullRequestRef,
    ) {
        record_checks_failed_with_suite(repository, run, pull_request, "clippy", fixed_time(31))
            .await;
    }

    async fn record_checks_failed_with_suite(
        repository: &InMemoryGithubIssueWorkflowRepository,
        run: &GithubIssueWorkflowRun,
        pull_request: GithubPullRequestRef,
        suite_or_run_id: &str,
        observed_at: chrono::DateTime<Utc>,
    ) {
        let head_sha = pull_request
            .head_sha
            .clone()
            .unwrap_or_else(|| "head-sha-42".to_string());
        repository
            .record_workflow_event(RecordWorkflowEventInput {
                workflow_run_id: run.workflow_run_id.clone(),
                workflow_event_type: GithubIssueWorkflowEventType::GithubChecksFailed,
                envelope: WorkflowEventEnvelope {
                    source_kind: WorkflowEventSourceKind::Poller,
                    source_delivery_id: None,
                    provider: provider_ref(&run.issue_ref),
                    observed_at,
                    provider_updated_at: Some(observed_at),
                    idempotency_key: checks_failed_key(&head_sha, suite_or_run_id),
                    payload_schema: "github.checks.failed.v1".to_string(),
                    payload: serde_json::to_value(GithubChecksChangedPayload {
                        pull_request: Some(pull_request),
                        head_sha,
                        suite_or_run_id: suite_or_run_id.to_string(),
                        conclusion: "failure".to_string(),
                    })
                    .unwrap(),
                },
            })
            .await
            .unwrap();
    }

    async fn record_review_comment(
        repository: &InMemoryGithubIssueWorkflowRepository,
        run: &GithubIssueWorkflowRun,
        pull_request: GithubPullRequestRef,
    ) {
        record_review_comment_with_id(
            repository,
            run,
            pull_request,
            "review-comment-node-1",
            fixed_time(32),
        )
        .await;
    }

    async fn record_review_comment_with_id(
        repository: &InMemoryGithubIssueWorkflowRepository,
        run: &GithubIssueWorkflowRun,
        pull_request: GithubPullRequestRef,
        comment_node_id: &str,
        observed_at: chrono::DateTime<Utc>,
    ) {
        let comment = GithubCommentRef {
            node_id: Some(comment_node_id.to_string()),
            url: format!("https://github.com/nearai/ironclaw/pull/12#discussion_{comment_node_id}"),
        };
        repository
            .record_workflow_event(RecordWorkflowEventInput {
                workflow_run_id: run.workflow_run_id.clone(),
                workflow_event_type: GithubIssueWorkflowEventType::GithubReviewCommentCreated,
                envelope: WorkflowEventEnvelope {
                    source_kind: WorkflowEventSourceKind::Poller,
                    source_delivery_id: None,
                    provider: provider_ref(&run.issue_ref),
                    observed_at,
                    provider_updated_at: Some(observed_at),
                    idempotency_key: review_comment_created_key(comment_node_id),
                    payload_schema: "github.review_comment.created.v1".to_string(),
                    payload: serde_json::to_value(GithubReviewCommentCreatedPayload {
                        pull_request: Some(pull_request),
                        comment,
                    })
                    .unwrap(),
                },
            })
            .await
            .unwrap();
    }

    async fn record_issue_closed(
        repository: &InMemoryGithubIssueWorkflowRepository,
        run: &GithubIssueWorkflowRun,
    ) {
        repository
            .record_workflow_event(RecordWorkflowEventInput {
                workflow_run_id: run.workflow_run_id.clone(),
                workflow_event_type: GithubIssueWorkflowEventType::GithubIssueClosed,
                envelope: WorkflowEventEnvelope {
                    source_kind: WorkflowEventSourceKind::Poller,
                    source_delivery_id: None,
                    provider: provider_ref(&run.issue_ref),
                    observed_at: fixed_time(33),
                    provider_updated_at: Some(fixed_time(33)),
                    idempotency_key: issue_closed_key(&run.issue_ref, Some(fixed_time(33))),
                    payload_schema: "github.issue.closed.v1".to_string(),
                    payload: serde_json::to_value(GithubIssueClosedPayload {
                        issue: run.issue_ref.clone(),
                        closed_at: Some(fixed_time(33)),
                    })
                    .unwrap(),
                },
            })
            .await
            .unwrap();
    }

    #[derive(Debug)]
    struct FakeClock {
        now: StdMutex<chrono::DateTime<Utc>>,
    }

    impl FakeClock {
        fn new(now: chrono::DateTime<Utc>) -> Self {
            Self {
                now: StdMutex::new(now),
            }
        }
    }

    impl WorkflowClock for FakeClock {
        fn now(&self) -> chrono::DateTime<Utc> {
            self.now.lock().unwrap().to_owned()
        }
    }

    #[derive(Debug)]
    struct FakeGithubPort {
        pull_requests: Mutex<Vec<GithubPullRequestSnapshot>>,
        created_prs: Mutex<Vec<CreateDraftPullRequestInput>>,
        create_pr_results: Mutex<VecDeque<Result<GithubPullRequestRef, GithubIssueWorkflowError>>>,
    }

    impl FakeGithubPort {
        fn new() -> Self {
            Self {
                pull_requests: Mutex::new(Vec::new()),
                created_prs: Mutex::new(Vec::new()),
                create_pr_results: Mutex::new(VecDeque::from([Ok(pr(12, None).pull_request)])),
            }
        }

        async fn set_pull_requests(&self, pull_requests: Vec<GithubPullRequestSnapshot>) {
            *self.pull_requests.lock().await = pull_requests;
        }

        async fn created_prs(&self) -> Vec<CreateDraftPullRequestInput> {
            self.created_prs.lock().await.clone()
        }
    }

    #[async_trait]
    impl GithubIssueWorkflowPort for FakeGithubPort {
        async fn get_authenticated_workflow_actor(
            &self,
            _input: GetAuthenticatedWorkflowActorInput,
        ) -> Result<GithubActorSnapshot, GithubIssueWorkflowError> {
            Ok(GithubActorSnapshot {
                login: "ironclaw-bot".to_string(),
                node_id: Some("actor-node-1".to_string()),
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
                node_id: Some("comment-node-1".to_string()),
                url: "https://github.com/nearai/ironclaw/issues/42#issuecomment-1".to_string(),
            })
        }

        async fn list_pull_requests(
            &self,
            _input: ListPullRequestsInput,
        ) -> Result<Vec<GithubPullRequestSnapshot>, GithubIssueWorkflowError> {
            Ok(self.pull_requests.lock().await.clone())
        }

        async fn create_draft_pull_request(
            &self,
            input: CreateDraftPullRequestInput,
        ) -> Result<GithubPullRequestRef, GithubIssueWorkflowError> {
            self.created_prs.lock().await.push(input);
            self.create_pr_results
                .lock()
                .await
                .pop_front()
                .unwrap_or_else(|| Ok(pr(12, None).pull_request))
        }
    }

    #[derive(Debug, Default)]
    struct FakeProjectAccess;

    #[async_trait]
    impl WorkflowProjectAccess for FakeProjectAccess {
        async fn assert_workflow_project_access(
            &self,
            _request: WorkflowProjectAccessRequest,
        ) -> Result<(), GithubIssueWorkflowError> {
            Ok(())
        }
    }

    #[derive(Debug, Default)]
    struct FakeWorkspaceManager;

    #[async_trait]
    impl WorkflowWorkspaceManager for FakeWorkspaceManager {
        async fn prepare_workspace(
            &self,
            request: PrepareWorkflowWorkspaceRequest,
        ) -> Result<PrepareWorkflowWorkspaceOutcome, GithubIssueWorkflowError> {
            let workspace_session_id =
                GithubIssueWorkspaceSessionId::from_trusted("workspace-session-pr".to_string())
                    .unwrap();
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
                    working_branch: "ironclaw/fix-42".to_string(),
                    current_head_sha: Some("head-sha-42".to_string()),
                    workspace_ref: WorkflowWorkspaceRef {
                        thread_id: Some(ThreadId::new("workspace-thread-pr").unwrap()),
                        workspace_session_id: Some(workspace_session_id),
                        turn_run_id: Some(TurnRunId::new()),
                    },
                    mount_ref: WorkflowWorkspaceMountRef {
                        mount_id: "workspace-mount-pr".to_string(),
                        alias: "/workspace".to_string(),
                    },
                    created_at: request.requested_at,
                },
            })
        }

        async fn publish_workspace(
            &self,
            request: ironclaw_github_issue_workflow::PublishWorkflowWorkspaceRequest,
        ) -> Result<
            ironclaw_github_issue_workflow::PublishWorkflowWorkspaceOutcome,
            GithubIssueWorkflowError,
        > {
            Ok(ironclaw_github_issue_workflow::PublishWorkflowWorkspaceOutcome {
                working_branch: "ironclaw/fix-42".to_string(),
                base_branch: request.base_branch,
                head_sha: "head-sha-42".to_string(),
                has_changes: true,
            })
        }
    }

    #[derive(Debug, Default)]
    struct FakeStageTurnSubmitter {
        requests: Mutex<Vec<SubmitStageTurnRequest>>,
    }

    impl FakeStageTurnSubmitter {
        async fn requests(&self) -> Vec<SubmitStageTurnRequest> {
            self.requests.lock().await.clone()
        }
    }

    #[async_trait]
    impl StageTurnSubmitter for FakeStageTurnSubmitter {
        async fn submit_stage_turn(
            &self,
            request: SubmitStageTurnRequest,
        ) -> Result<SubmitStageTurnOutcome, GithubIssueWorkflowError> {
            let request_count = {
                let mut requests = self.requests.lock().await;
                requests.push(request);
                requests.len()
            };
            Ok(SubmitStageTurnOutcome::Submitted {
                thread_id: ThreadId::new(format!("thread-pr-{request_count}")).unwrap(),
                turn_run_id: TurnRunId::new(),
            })
        }
    }

    #[derive(Debug)]
    struct FakePolicyPorts {
        repository: Arc<InMemoryGithubIssueWorkflowRepository>,
        github: Arc<FakeGithubPort>,
        stage_turns: Arc<FakeStageTurnSubmitter>,
        project_access: Arc<FakeProjectAccess>,
        workspace: Arc<FakeWorkspaceManager>,
        clock: Arc<FakeClock>,
        worker_id: WorkflowWorkerId,
    }

    impl GithubIssueWorkflowPolicyPorts for FakePolicyPorts {
        type Clock = FakeClock;
        type GithubPort = FakeGithubPort;
        type ProjectAccess = FakeProjectAccess;
        type Repository = InMemoryGithubIssueWorkflowRepository;
        type StageTurnSubmitter = FakeStageTurnSubmitter;
        type WorkspaceManager = FakeWorkspaceManager;

        fn clock(&self) -> Arc<Self::Clock> {
            self.clock.clone()
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

    fn policy() -> GithubIssueWorkflowPolicy<FakePolicyPorts> {
        GithubIssueWorkflowPolicy::new(
            FakePolicyPorts {
                repository: Arc::new(InMemoryGithubIssueWorkflowRepository::default()),
                github: Arc::new(FakeGithubPort::new()),
                stage_turns: Arc::new(FakeStageTurnSubmitter::default()),
                project_access: Arc::new(FakeProjectAccess),
                workspace: Arc::new(FakeWorkspaceManager),
                clock: Arc::new(FakeClock::new(fixed_time(40))),
                worker_id: worker(),
            },
            "pr-lifecycle-v1",
        )
    }

    #[tokio::test]
    async fn implementation_ready_starts_pr_synthesis_stage() {
        let policy = policy();
        let run = create_claimed_run(&policy.ports().repository).await;
        let run = set_mode(
            &policy.ports().repository,
            run,
            GithubIssueWorkflowMode::Implementation,
            None,
        )
        .await;
        record_stage_completed(
            &policy.ports().repository,
            &run,
            GithubIssueStage::Implementation,
        )
        .await;

        let outcome = policy.tick(run).await.unwrap();

        assert_eq!(outcome.processed_event_count, 1);
        assert_eq!(
            outcome.run.workflow_state.mode,
            GithubIssueWorkflowMode::PrSynthesis
        );
        let requests = policy.ports().stage_turns.requests().await;
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0].stage_turn_identity.stage,
            GithubIssueStage::PrSynthesis
        );
    }

    #[tokio::test]
    async fn pr_synthesis_creates_draft_pr_once() {
        let policy = policy();
        let run = create_claimed_run(&policy.ports().repository).await;
        let run = set_mode(
            &policy.ports().repository,
            run,
            GithubIssueWorkflowMode::PrSynthesis,
            None,
        )
        .await;
        let run = attach_workspace_session(&policy.ports().repository, run).await;
        record_stage_completed(
            &policy.ports().repository,
            &run,
            GithubIssueStage::PrSynthesis,
        )
        .await;

        let first = policy.tick(run).await.unwrap();
        let second = policy.tick(first.run.clone()).await.unwrap();

        assert_eq!(first.processed_event_count, 1);
        assert_eq!(second.processed_event_count, 0);
        assert_eq!(policy.ports().github.created_prs().await.len(), 1);
        assert_eq!(
            first.run.workflow_state.mode,
            GithubIssueWorkflowMode::PrOpen
        );
        assert_eq!(
            first
                .run
                .workflow_state
                .primary_pr
                .as_ref()
                .map(|pull_request| pull_request.number),
            Some(12)
        );
    }

    #[tokio::test]
    async fn draft_pr_ambiguous_write_reconciles_by_branch_and_marker() {
        let repository = Arc::new(InMemoryGithubIssueWorkflowRepository::default());
        let github = Arc::new(FakeGithubPort::new());
        let run = create_claimed_run(&repository).await;
        let marker = stable_pr_marker(&run.workflow_run_id);
        github
            .set_pull_requests(vec![pr(12, Some(&marker)), pr(13, Some(&marker))])
            .await;
        let runner = ironclaw_github_issue_workflow::GithubIssueProviderActionRunner::new(
            repository,
            github.clone(),
        );

        let outcome = runner
            .run_draft_pull_request(RunDraftPullRequestProviderActionRequest {
                run,
                stage_run_id: None,
                title: "Fix bug 42".to_string(),
                body: "This fixes bug 42.".to_string(),
                head_branch: "ironclaw/fix-42".to_string(),
                base_branch: "main".to_string(),
                head_sha: "head-sha-42".to_string(),
                provider_account_ref: provider_account_ref(),
                worker_id: worker(),
                now: fixed_time(40),
                lease_expires_at: fixed_time(100),
            })
            .await
            .unwrap();

        let ProviderActionRunOutcome::NeedsReconciliation { action } = outcome else {
            panic!("ambiguous matching PRs should enter reconciliation");
        };
        assert_eq!(action.status, ProviderActionStatus::NeedsReconciliation);
        assert_eq!(action.kind, ProviderActionKind::DraftPullRequest);
        assert_eq!(
            action.reconciliation_strategy,
            ProviderActionReconciliationStrategy::DraftPullRequestByHeadBranchAndMarker
        );
        assert_eq!(github.created_prs().await.len(), 0);
    }

    #[tokio::test]
    async fn failed_checks_start_ci_repair_stage() {
        let policy = policy();
        let run = create_claimed_run(&policy.ports().repository).await;
        let primary_pr = pr(12, None).pull_request;
        let run = set_mode(
            &policy.ports().repository,
            run,
            GithubIssueWorkflowMode::PrOpen,
            Some(primary_pr.clone()),
        )
        .await;
        record_checks_failed(&policy.ports().repository, &run, primary_pr).await;

        let outcome = policy.tick(run).await.unwrap();

        assert_eq!(
            outcome.run.workflow_state.mode,
            GithubIssueWorkflowMode::CiRepair
        );
        let requests = policy.ports().stage_turns.requests().await;
        assert_eq!(
            requests
                .last()
                .map(|request| &request.stage_turn_identity.stage),
            Some(&GithubIssueStage::CiRepair)
        );
    }

    #[tokio::test]
    async fn second_failed_check_after_completed_repair_starts_new_ci_repair_stage() {
        let policy = policy();
        let run = create_claimed_run(&policy.ports().repository).await;
        let primary_pr = pr(12, None).pull_request;
        let run = set_mode(
            &policy.ports().repository,
            run,
            GithubIssueWorkflowMode::PrOpen,
            Some(primary_pr.clone()),
        )
        .await;
        record_checks_failed(&policy.ports().repository, &run, primary_pr.clone()).await;

        let first_repair = policy.tick(run).await.unwrap();
        let accepted_run = record_stage_completed(
            &policy.ports().repository,
            &first_repair.run,
            GithubIssueStage::CiRepair,
        )
        .await;
        let reopened = policy.tick(accepted_run).await.unwrap();
        record_checks_failed_with_suite(
            &policy.ports().repository,
            &reopened.run,
            primary_pr,
            "test",
            fixed_time(41),
        )
        .await;

        let second_repair = policy.tick(reopened.run).await.unwrap();

        assert_eq!(
            second_repair.run.workflow_state.mode,
            GithubIssueWorkflowMode::CiRepair
        );
        let requests = policy.ports().stage_turns.requests().await;
        assert_eq!(requests.len(), 2);
        assert_eq!(
            requests[0].stage_turn_identity.stage,
            GithubIssueStage::CiRepair
        );
        assert_eq!(
            requests[1].stage_turn_identity.stage,
            GithubIssueStage::CiRepair
        );
        assert_ne!(
            requests[0].stage_turn_identity.stage_run_id,
            requests[1].stage_turn_identity.stage_run_id
        );
    }

    #[tokio::test]
    async fn review_comment_starts_review_response_stage() {
        let policy = policy();
        let run = create_claimed_run(&policy.ports().repository).await;
        let primary_pr = pr(12, None).pull_request;
        let run = set_mode(
            &policy.ports().repository,
            run,
            GithubIssueWorkflowMode::PrOpen,
            Some(primary_pr.clone()),
        )
        .await;
        record_review_comment(&policy.ports().repository, &run, primary_pr).await;

        let outcome = policy.tick(run).await.unwrap();

        assert_eq!(
            outcome.run.workflow_state.mode,
            GithubIssueWorkflowMode::ReviewResponse
        );
        let requests = policy.ports().stage_turns.requests().await;
        assert_eq!(
            requests
                .last()
                .map(|request| &request.stage_turn_identity.stage),
            Some(&GithubIssueStage::ReviewResponse)
        );
    }

    #[tokio::test]
    async fn second_review_comment_after_completed_response_starts_new_review_response_stage() {
        let policy = policy();
        let run = create_claimed_run(&policy.ports().repository).await;
        let primary_pr = pr(12, None).pull_request;
        let run = set_mode(
            &policy.ports().repository,
            run,
            GithubIssueWorkflowMode::PrOpen,
            Some(primary_pr.clone()),
        )
        .await;
        record_review_comment(&policy.ports().repository, &run, primary_pr.clone()).await;

        let first_response = policy.tick(run).await.unwrap();
        let accepted_run = record_stage_completed(
            &policy.ports().repository,
            &first_response.run,
            GithubIssueStage::ReviewResponse,
        )
        .await;
        let reopened = policy.tick(accepted_run).await.unwrap();
        record_review_comment_with_id(
            &policy.ports().repository,
            &reopened.run,
            primary_pr,
            "review-comment-node-2",
            fixed_time(42),
        )
        .await;

        let second_response = policy.tick(reopened.run).await.unwrap();

        assert_eq!(
            second_response.run.workflow_state.mode,
            GithubIssueWorkflowMode::ReviewResponse
        );
        let requests = policy.ports().stage_turns.requests().await;
        assert_eq!(requests.len(), 2);
        assert_eq!(
            requests[0].stage_turn_identity.stage,
            GithubIssueStage::ReviewResponse
        );
        assert_eq!(
            requests[1].stage_turn_identity.stage,
            GithubIssueStage::ReviewResponse
        );
        assert_ne!(
            requests[0].stage_turn_identity.stage_run_id,
            requests[1].stage_turn_identity.stage_run_id
        );
    }

    #[tokio::test]
    async fn merged_pr_completes_workflow() {
        let policy = policy();
        let run = create_claimed_run(&policy.ports().repository).await;
        let primary_pr = pr(12, None).pull_request;
        let run = set_mode(
            &policy.ports().repository,
            run,
            GithubIssueWorkflowMode::PrOpen,
            Some(primary_pr.clone()),
        )
        .await;
        record_pr_updated(&policy.ports().repository, &run, primary_pr, "closed", true).await;

        let outcome = policy.tick(run).await.unwrap();

        assert_eq!(outcome.run.status, GithubIssueWorkflowRunStatus::Succeeded);
        assert_eq!(
            outcome.run.workflow_state.mode,
            GithubIssueWorkflowMode::Done
        );
    }

    #[tokio::test]
    async fn closed_issue_cancels_active_workflow() {
        let policy = policy();
        let run = create_claimed_run(&policy.ports().repository).await;
        let run = set_mode(
            &policy.ports().repository,
            run,
            GithubIssueWorkflowMode::PrOpen,
            Some(pr(12, None).pull_request),
        )
        .await;
        record_issue_closed(&policy.ports().repository, &run).await;

        let outcome = policy.tick(run).await.unwrap();

        assert_eq!(outcome.run.status, GithubIssueWorkflowRunStatus::Cancelled);
        assert_eq!(
            outcome.run.workflow_state.mode,
            GithubIssueWorkflowMode::Done
        );
    }
}
