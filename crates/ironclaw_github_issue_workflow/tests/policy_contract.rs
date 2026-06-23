mod policy_contract {
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex as StdMutex};

    use async_trait::async_trait;
    use chrono::{Duration, TimeZone, Utc};
    use ironclaw_github_issue_workflow::{
        AcceptStageResultInput, CreateIssueCommentInput, CreateOrGetWorkflowRunInput,
        CreateOrGetWorkflowRunOutcome, GetAuthenticatedWorkflowActorInput, GithubActorSnapshot,
        GithubCommentRef, GithubIssueCommentSnapshot, GithubIssueDiscoveredPayload, GithubIssueRef,
        GithubIssueStage, GithubIssueWorkflowError, GithubIssueWorkflowEventType,
        GithubIssueWorkflowMode, GithubIssueWorkflowPolicy, GithubIssueWorkflowPolicyPorts,
        GithubIssueWorkflowPort, GithubIssueWorkflowRepository, GithubIssueWorkflowRun,
        GithubIssueWorkflowRunStatus, GithubIssueWorkspaceSession, GithubIssueWorkspaceSessionId,
        GithubProviderRef, GithubRepositorySelector, InMemoryGithubIssueWorkflowRepository,
        ListIssueCommentsInput, PrepareWorkflowWorkspaceOutcome, PrepareWorkflowWorkspaceRequest,
        RecordWorkflowEventInput, RecordWorkflowEventOutcome, StageCompletedPayload,
        StageTurnSubmitter, SubmitStageTurnOutcome, SubmitStageTurnRequest, WorkflowClock,
        WorkflowEventEnvelope, WorkflowEventSourceKind, WorkflowProjectAccess,
        WorkflowProjectAccessRequest, WorkflowStepStatus, WorkflowWorkerId,
        WorkflowWorkspaceManager, WorkflowWorkspaceMountRef, WorkflowWorkspaceRef,
        issue_binding_ref, issue_discovered_key, stage_result_reported_key,
    };
    use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId, UserId};
    use ironclaw_turns::TurnRunId;
    use serde_json::{Value as JsonValue, json};
    use tokio::sync::Mutex;

    fn fixed_time(seconds: i64) -> chrono::DateTime<Utc> {
        Utc.timestamp_opt(seconds, 0).unwrap()
    }

    fn tenant() -> TenantId {
        TenantId::new("tenant-policy-contract").unwrap()
    }

    fn user() -> UserId {
        UserId::new("user-policy-contract").unwrap()
    }

    fn agent() -> AgentId {
        AgentId::new("agent-policy-contract").unwrap()
    }

    fn project() -> ProjectId {
        ProjectId::new("project-policy-contract").unwrap()
    }

    fn worker() -> WorkflowWorkerId {
        WorkflowWorkerId::from_trusted("policy-contract-worker".to_string()).unwrap()
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

    fn stage_result(stage: GithubIssueStage) -> JsonValue {
        match stage {
            GithubIssueStage::Triage => json!({
                "outcome": "completed",
                "summary": "triage completed",
                "evidence": [],
                "next_actions": [],
                "payload": {
                    "is_reproducible": true,
                    "suspected_area": "github_issue_workflow",
                    "risk": "medium",
                    "recommended_next_stage": "planning"
                }
            }),
            GithubIssueStage::Planning => json!({
                "outcome": "completed",
                "summary": "planning completed",
                "evidence": [],
                "next_actions": [],
                "payload": {
                    "plan_items": ["implement the fix"],
                    "files_to_inspect_or_change": ["src/lib.rs"],
                    "test_strategy": "cargo test",
                    "confidence": 0.9
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
                provider_account_ref: None,
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
                    lease_expires_at: fixed_time(11) + Duration::seconds(60),
                    limit: 1,
                },
            )
            .await
            .unwrap()
            .pop()
            .unwrap_or(run)
    }

    async fn record_issue_discovered(
        repository: &InMemoryGithubIssueWorkflowRepository,
        run: &GithubIssueWorkflowRun,
    ) {
        let outcome = repository
            .record_workflow_event(RecordWorkflowEventInput {
                workflow_run_id: run.workflow_run_id.clone(),
                workflow_event_type: GithubIssueWorkflowEventType::GithubIssueDiscovered,
                envelope: WorkflowEventEnvelope {
                    source_kind: WorkflowEventSourceKind::Poller,
                    source_delivery_id: None,
                    provider: provider_ref(&run.issue_ref),
                    observed_at: fixed_time(12),
                    provider_updated_at: Some(fixed_time(12)),
                    idempotency_key: issue_discovered_key(&run.issue_ref),
                    payload_schema: "github.issue.discovered.v1".to_string(),
                    payload: serde_json::to_value(GithubIssueDiscoveredPayload {
                        issue: run.issue_ref.clone(),
                    })
                    .unwrap(),
                },
            })
            .await
            .unwrap();
        assert!(matches!(
            outcome,
            RecordWorkflowEventOutcome::Recorded { .. }
        ));
    }

    async fn complete_active_stage(
        repository: &InMemoryGithubIssueWorkflowRepository,
        run: GithubIssueWorkflowRun,
        stage: GithubIssueStage,
    ) -> GithubIssueWorkflowRun {
        let stage_run_id = run
            .active_stage_run_id
            .clone()
            .expect("test setup must have an active stage run");
        let result = stage_result(stage.clone());
        let accepted = repository
            .accept_stage_result(AcceptStageResultInput {
                workflow_run_id: run.workflow_run_id.clone(),
                stage_run_id: stage_run_id.clone(),
                result: result.clone(),
                now: fixed_time(20),
            })
            .await
            .unwrap();
        let accepted_run = match accepted {
            ironclaw_github_issue_workflow::AcceptStageResultOutcome::Accepted { run } => run,
            other => panic!("stage result must be accepted, got {other:?}"),
        };

        let outcome = repository
            .record_workflow_event(RecordWorkflowEventInput {
                workflow_run_id: run.workflow_run_id,
                workflow_event_type: GithubIssueWorkflowEventType::StageCompleted,
                envelope: WorkflowEventEnvelope {
                    source_kind: WorkflowEventSourceKind::WorkflowInternal,
                    source_delivery_id: None,
                    provider: provider_ref(&accepted_run.issue_ref),
                    observed_at: fixed_time(21),
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
            .await
            .unwrap();
        assert!(matches!(
            outcome,
            RecordWorkflowEventOutcome::Recorded { .. }
        ));

        accepted_run
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

        fn set(&self, now: chrono::DateTime<Utc>) {
            *self.now.lock().unwrap() = now;
        }
    }

    impl WorkflowClock for FakeClock {
        fn now(&self) -> chrono::DateTime<Utc> {
            self.now.lock().unwrap().to_owned()
        }
    }

    #[derive(Debug)]
    struct FakeGithubPort {
        comments: Mutex<Vec<GithubIssueCommentSnapshot>>,
        created_bodies: Mutex<Vec<String>>,
    }

    impl FakeGithubPort {
        fn new() -> Self {
            Self {
                comments: Mutex::new(Vec::new()),
                created_bodies: Mutex::new(Vec::new()),
            }
        }

        async fn created_body_count(&self) -> usize {
            self.created_bodies.lock().await.len()
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
            Ok(self.comments.lock().await.clone())
        }

        async fn create_issue_comment(
            &self,
            input: CreateIssueCommentInput,
        ) -> Result<GithubCommentRef, GithubIssueWorkflowError> {
            self.created_bodies.lock().await.push(input.body);
            Ok(GithubCommentRef {
                node_id: Some("created-comment-node-1".to_string()),
                url: "https://github.com/nearai/ironclaw/issues/42#issuecomment-created"
                    .to_string(),
            })
        }
    }

    #[derive(Debug)]
    struct FakeProjectAccess {
        allowed: bool,
        requests: Mutex<Vec<WorkflowProjectAccessRequest>>,
    }

    impl FakeProjectAccess {
        fn allow() -> Self {
            Self {
                allowed: true,
                requests: Mutex::new(Vec::new()),
            }
        }

        fn deny() -> Self {
            Self {
                allowed: false,
                requests: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl WorkflowProjectAccess for FakeProjectAccess {
        async fn assert_workflow_project_access(
            &self,
            request: WorkflowProjectAccessRequest,
        ) -> Result<(), GithubIssueWorkflowError> {
            self.requests.lock().await.push(request);
            if self.allowed {
                return Ok(());
            }
            Err(GithubIssueWorkflowError::PolicyDenied {
                reason: "project access denied".to_string(),
            })
        }
    }

    #[derive(Debug)]
    struct FakeWorkspaceManager {
        requests: Mutex<Vec<PrepareWorkflowWorkspaceRequest>>,
    }

    impl FakeWorkspaceManager {
        fn new() -> Self {
            Self {
                requests: Mutex::new(Vec::new()),
            }
        }

        async fn request_count(&self) -> usize {
            self.requests.lock().await.len()
        }
    }

    #[async_trait]
    impl WorkflowWorkspaceManager for FakeWorkspaceManager {
        async fn prepare_workspace(
            &self,
            request: PrepareWorkflowWorkspaceRequest,
        ) -> Result<PrepareWorkflowWorkspaceOutcome, GithubIssueWorkflowError> {
            self.requests.lock().await.push(request.clone());
            let workspace_session_id =
                GithubIssueWorkspaceSessionId::from_trusted("workspace-session-1".to_string())
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
                    base_sha: Some("base-sha-1".to_string()),
                    working_branch: "ironclaw/workspace-session-1".to_string(),
                    current_head_sha: None,
                    workspace_ref: WorkflowWorkspaceRef {
                        thread_id: None,
                        workspace_session_id: Some(workspace_session_id),
                        turn_run_id: None,
                    },
                    mount_ref: WorkflowWorkspaceMountRef {
                        mount_id: "mount-1".to_string(),
                        alias: "/workspace".to_string(),
                    },
                    created_at: request.requested_at,
                },
            })
        }
    }

    #[derive(Debug)]
    struct FakeStageTurnSubmitter {
        requests: Mutex<Vec<SubmitStageTurnRequest>>,
        outcomes: Mutex<VecDeque<SubmitStageTurnOutcome>>,
    }

    impl FakeStageTurnSubmitter {
        fn accepting() -> Self {
            Self {
                requests: Mutex::new(Vec::new()),
                outcomes: Mutex::new(VecDeque::new()),
            }
        }

        fn busy_once() -> Self {
            Self {
                requests: Mutex::new(Vec::new()),
                outcomes: Mutex::new(VecDeque::from([SubmitStageTurnOutcome::Busy {
                    reason: "thread busy".to_string(),
                }])),
            }
        }

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
            if let Some(outcome) = self.outcomes.lock().await.pop_front() {
                return Ok(outcome);
            }
            Ok(SubmitStageTurnOutcome::Submitted {
                thread_id: ThreadId::new(format!("thread-policy-{request_count}")).unwrap(),
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

    impl FakePolicyPorts {
        fn new(
            stage_turns: Arc<FakeStageTurnSubmitter>,
            project_access: Arc<FakeProjectAccess>,
        ) -> Self {
            Self {
                repository: Arc::new(InMemoryGithubIssueWorkflowRepository::default()),
                github: Arc::new(FakeGithubPort::new()),
                stage_turns,
                project_access,
                workspace: Arc::new(FakeWorkspaceManager::new()),
                clock: Arc::new(FakeClock::new(fixed_time(30))),
                worker_id: worker(),
            }
        }
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

    fn policy(
        stage_turns: Arc<FakeStageTurnSubmitter>,
        project_access: Arc<FakeProjectAccess>,
    ) -> GithubIssueWorkflowPolicy<FakePolicyPorts> {
        GithubIssueWorkflowPolicy::new(
            FakePolicyPorts::new(stage_turns, project_access),
            "policy-contract-v1",
        )
    }

    #[tokio::test]
    async fn issue_discovered_claims_then_starts_triage_once() {
        let stage_turns = Arc::new(FakeStageTurnSubmitter::accepting());
        let project_access = Arc::new(FakeProjectAccess::allow());
        let policy = policy(stage_turns.clone(), project_access);
        let run = create_claimed_run(&policy.ports().repository).await;
        record_issue_discovered(&policy.ports().repository, &run).await;

        let outcome = policy.tick(run).await.unwrap();

        assert_eq!(policy.ports().github.created_body_count().await, 1);
        assert_eq!(
            outcome.run.workflow_state.mode,
            GithubIssueWorkflowMode::Claimed
        );
        assert!(outcome.run.active_stage_run_id.is_some());
        let requests = stage_turns.requests().await;
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0].stage_turn_identity.stage,
            GithubIssueStage::Triage
        );
        assert_eq!(
            requests[0].stage_turn_identity.thread_id_seed(),
            format!(
                "github-issue-workflow:{}:stage:{}",
                outcome.run.workflow_run_id,
                outcome.run.active_stage_run_id.as_ref().unwrap()
            )
        );
        assert!(
            requests[0]
                .stage_turn_identity
                .completion_nonce()
                .starts_with("stage-completion:")
        );
    }

    #[tokio::test]
    async fn policy_tick_replays_completed_claim_step_without_second_comment() {
        let stage_turns = Arc::new(FakeStageTurnSubmitter::accepting());
        let project_access = Arc::new(FakeProjectAccess::allow());
        let policy = policy(stage_turns.clone(), project_access);
        let run = create_claimed_run(&policy.ports().repository).await;
        record_issue_discovered(&policy.ports().repository, &run).await;

        policy.tick(run.clone()).await.unwrap();
        policy.tick(run).await.unwrap();

        assert_eq!(policy.ports().github.created_body_count().await, 1);
        assert_eq!(stage_turns.requests().await.len(), 1);
    }

    #[tokio::test]
    async fn triage_completion_starts_planning_stage() {
        let stage_turns = Arc::new(FakeStageTurnSubmitter::accepting());
        let project_access = Arc::new(FakeProjectAccess::allow());
        let policy = policy(stage_turns.clone(), project_access);
        let run = create_claimed_run(&policy.ports().repository).await;
        record_issue_discovered(&policy.ports().repository, &run).await;
        let triage = policy.tick(run).await.unwrap().run;
        let after_triage =
            complete_active_stage(&policy.ports().repository, triage, GithubIssueStage::Triage)
                .await;

        let outcome = policy.tick(after_triage).await.unwrap();

        assert_eq!(
            outcome.run.workflow_state.mode,
            GithubIssueWorkflowMode::Planning
        );
        assert!(outcome.run.active_stage_run_id.is_some());
        let requests = stage_turns.requests().await;
        assert_eq!(requests.len(), 2);
        assert_eq!(
            requests.last().unwrap().stage_turn_identity.stage,
            GithubIssueStage::Planning
        );
    }

    #[tokio::test]
    async fn planning_completion_prepares_workspace_then_starts_implementation() {
        let stage_turns = Arc::new(FakeStageTurnSubmitter::accepting());
        let project_access = Arc::new(FakeProjectAccess::allow());
        let policy = policy(stage_turns.clone(), project_access);
        let run = create_claimed_run(&policy.ports().repository).await;
        record_issue_discovered(&policy.ports().repository, &run).await;
        let triage = policy.tick(run).await.unwrap().run;
        let after_triage =
            complete_active_stage(&policy.ports().repository, triage, GithubIssueStage::Triage)
                .await;
        let planning = policy.tick(after_triage).await.unwrap().run;
        let after_planning = complete_active_stage(
            &policy.ports().repository,
            planning,
            GithubIssueStage::Planning,
        )
        .await;

        let outcome = policy.tick(after_planning).await.unwrap();

        assert_eq!(
            outcome.run.workflow_state.mode,
            GithubIssueWorkflowMode::Implementation
        );
        assert_eq!(policy.ports().workspace.request_count().await, 1);
        assert!(outcome.run.workspace_session_id.is_some());
        let requests = stage_turns.requests().await;
        assert_eq!(requests.len(), 3);
        let implementation_request = requests.last().unwrap();
        assert_eq!(
            implementation_request.stage_turn_identity.stage,
            GithubIssueStage::Implementation
        );
        assert_eq!(
            implementation_request
                .workspace_mount_ref
                .as_ref()
                .map(|mount| mount.alias.as_str()),
            Some("/workspace")
        );
    }

    #[tokio::test]
    async fn project_access_denial_blocks_run_without_stage_submission() {
        let stage_turns = Arc::new(FakeStageTurnSubmitter::accepting());
        let project_access = Arc::new(FakeProjectAccess::deny());
        let policy = policy(stage_turns.clone(), project_access);
        let run = create_claimed_run(&policy.ports().repository).await;
        record_issue_discovered(&policy.ports().repository, &run).await;

        let outcome = policy.tick(run).await.unwrap();

        assert_eq!(outcome.run.status, GithubIssueWorkflowRunStatus::Blocked);
        assert!(outcome.run.workflow_state.active_block.is_some());
        assert!(outcome.run.active_stage_run_id.is_none());
        assert!(stage_turns.requests().await.is_empty());
    }

    #[tokio::test]
    async fn turn_submission_busy_keeps_stage_active_without_duplicate_submit() {
        let stage_turns = Arc::new(FakeStageTurnSubmitter::busy_once());
        let project_access = Arc::new(FakeProjectAccess::allow());
        let policy = policy(stage_turns.clone(), project_access);
        let run = create_claimed_run(&policy.ports().repository).await;
        record_issue_discovered(&policy.ports().repository, &run).await;

        let first = policy.tick(run).await.unwrap();
        let immediate_replay = policy.tick(first.run.clone()).await.unwrap();

        assert!(first.run.active_stage_run_id.is_some());
        assert_eq!(first.run.event_cursor, 0);
        let busy_step = first
            .steps
            .iter()
            .find(|step| step.step_name == "start_stage:triage")
            .expect("busy stage submission should record the start-stage step");
        assert_eq!(busy_step.status, WorkflowStepStatus::Retryable);
        assert_eq!(busy_step.next_attempt_at, Some(fixed_time(60)));
        assert_eq!(immediate_replay.processed_event_count, 0);
        assert_eq!(stage_turns.requests().await.len(), 1);

        policy.ports().clock.set(fixed_time(61));
        let retry = policy.tick(first.run).await.unwrap();

        assert_eq!(retry.processed_event_count, 1);
        assert_eq!(retry.run.event_cursor, 1);
        assert_eq!(
            retry.run.workflow_state.mode,
            GithubIssueWorkflowMode::Claimed
        );
        let retried_step = retry
            .steps
            .iter()
            .find(|step| step.step_name == "start_stage:triage")
            .expect("retry should return the start-stage step");
        assert_eq!(retried_step.status, WorkflowStepStatus::Succeeded);
        assert_eq!(stage_turns.requests().await.len(), 2);
    }
}
