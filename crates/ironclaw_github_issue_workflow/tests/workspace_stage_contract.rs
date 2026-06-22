mod workspace_stage_contract {
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex as StdMutex};

    use async_trait::async_trait;
    use chrono::{Duration, TimeZone, Utc};
    use ironclaw_github_issue_workflow::{
        AcceptStageResultInput, CompleteWorkflowStepInput, CreateIssueCommentInput,
        CreateOrGetWorkflowRunInput, CreateOrGetWorkflowRunOutcome, CreateOrGetWorkflowStepInput,
        CreateOrGetWorkflowStepOutcome, GetAuthenticatedWorkflowActorInput, GithubActorSnapshot,
        GithubCommentRef, GithubIssueCommentSnapshot, GithubIssueDiscoveredPayload, GithubIssueRef,
        GithubIssueStage, GithubIssueWorkflowError, GithubIssueWorkflowEventType,
        GithubIssueWorkflowMode, GithubIssueWorkflowPolicy, GithubIssueWorkflowPolicyPorts,
        GithubIssueWorkflowPort, GithubIssueWorkflowRepository, GithubIssueWorkflowRun,
        GithubIssueWorkspaceSession, GithubIssueWorkspaceSessionId, GithubProviderRef,
        GithubRepositorySelector, InMemoryGithubIssueWorkflowRepository, ListIssueCommentsInput,
        PrepareWorkflowWorkspaceOutcome, PrepareWorkflowWorkspaceRequest, RecordWorkflowEventInput,
        RecordWorkflowEventOutcome, StageCompletedPayload, StageTurnSubmitter,
        SubmitStageTurnOutcome, SubmitStageTurnRequest, WorkflowClock, WorkflowEventEnvelope,
        WorkflowEventSourceKind, WorkflowIdempotencyKey, WorkflowProjectAccess,
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
        TenantId::new("tenant-workspace-contract").unwrap()
    }

    fn user() -> UserId {
        UserId::new("user-workspace-contract").unwrap()
    }

    fn agent() -> AgentId {
        AgentId::new("agent-workspace-contract").unwrap()
    }

    fn project() -> ProjectId {
        ProjectId::new("project-workspace-contract").unwrap()
    }

    fn worker() -> WorkflowWorkerId {
        WorkflowWorkerId::from_trusted("workspace-contract-worker".to_string()).unwrap()
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
                    "suspected_area": "workspace",
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
                    "plan_items": ["prepare workspace", "implement fix"],
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

    async fn planning_completed_run(
        policy: &GithubIssueWorkflowPolicy<FakePolicyPorts>,
    ) -> GithubIssueWorkflowRun {
        let run = create_claimed_run(&policy.ports().repository).await;
        record_issue_discovered(&policy.ports().repository, &run).await;
        let triage = policy.tick(run).await.unwrap().run;
        let after_triage =
            complete_active_stage(&policy.ports().repository, triage, GithubIssueStage::Triage)
                .await;
        let planning = policy.tick(after_triage).await.unwrap().run;
        complete_active_stage(
            &policy.ports().repository,
            planning,
            GithubIssueStage::Planning,
        )
        .await
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

    #[derive(Debug, Default)]
    struct FakeGithubPort {
        comments: Mutex<Vec<GithubIssueCommentSnapshot>>,
        created_bodies: Mutex<Vec<String>>,
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

    #[derive(Debug)]
    struct FakeWorkspaceManager {
        requests: Mutex<Vec<PrepareWorkflowWorkspaceRequest>>,
        failures: Mutex<VecDeque<GithubIssueWorkflowError>>,
    }

    impl FakeWorkspaceManager {
        fn new() -> Self {
            Self {
                requests: Mutex::new(Vec::new()),
                failures: Mutex::new(VecDeque::new()),
            }
        }

        fn fail_once() -> Self {
            Self {
                requests: Mutex::new(Vec::new()),
                failures: Mutex::new(VecDeque::from([GithubIssueWorkflowError::Repository {
                    reason: "workspace backend unavailable".to_string(),
                }])),
            }
        }

        fn denied() -> Self {
            Self {
                requests: Mutex::new(Vec::new()),
                failures: Mutex::new(VecDeque::from([GithubIssueWorkflowError::PolicyDenied {
                    reason: "workspace access denied".to_string(),
                }])),
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
            let mut failures = self.failures.lock().await;
            if let Some(error) = failures.pop_front() {
                return Err(error);
            }
            drop(failures);

            let workspace_session_id =
                GithubIssueWorkspaceSessionId::from_trusted("workspace-session-42".to_string())
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
                    base_sha: Some("base-sha-42".to_string()),
                    working_branch: "ironclaw/workspace-session-42".to_string(),
                    current_head_sha: Some("head-sha-42".to_string()),
                    workspace_ref: WorkflowWorkspaceRef {
                        thread_id: Some(ThreadId::new("workspace-thread-42").unwrap()),
                        workspace_session_id: Some(workspace_session_id),
                        turn_run_id: Some(TurnRunId::new()),
                    },
                    mount_ref: WorkflowWorkspaceMountRef {
                        mount_id: "workspace-mount-42".to_string(),
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
                thread_id: ThreadId::new(format!("thread-workspace-{request_count}")).unwrap(),
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

    fn policy_with_workspace(
        workspace: Arc<FakeWorkspaceManager>,
    ) -> GithubIssueWorkflowPolicy<FakePolicyPorts> {
        GithubIssueWorkflowPolicy::new(
            FakePolicyPorts {
                repository: Arc::new(InMemoryGithubIssueWorkflowRepository::default()),
                github: Arc::new(FakeGithubPort::default()),
                stage_turns: Arc::new(FakeStageTurnSubmitter::accepting()),
                project_access: Arc::new(FakeProjectAccess),
                workspace,
                clock: Arc::new(FakeClock::new(fixed_time(30))),
                worker_id: worker(),
            },
            "workspace-contract-v1",
        )
    }

    #[tokio::test]
    async fn planning_completion_prepares_workspace_once() {
        let workspace = Arc::new(FakeWorkspaceManager::new());
        let policy = policy_with_workspace(workspace.clone());
        let after_planning = planning_completed_run(&policy).await;

        let first = policy.tick(after_planning.clone()).await.unwrap();
        let second = policy.tick(after_planning).await.unwrap();

        assert_eq!(first.processed_event_count, 1);
        assert_eq!(second.processed_event_count, 1);
        assert_eq!(workspace.request_count().await, 1);
        assert_eq!(
            first.run.workflow_state.mode,
            GithubIssueWorkflowMode::Implementation
        );
    }

    #[tokio::test]
    async fn workspace_ref_not_raw_host_path() {
        let workspace = Arc::new(FakeWorkspaceManager::new());
        let policy = policy_with_workspace(workspace);
        let after_planning = planning_completed_run(&policy).await;

        let outcome = policy.tick(after_planning).await.unwrap();

        let workspace_ref = outcome
            .run
            .workflow_state
            .current_workspace_ref
            .as_ref()
            .expect("workspace ref must be stored on run state");
        assert!(workspace_ref.thread_id.is_some());
        assert!(workspace_ref.workspace_session_id.is_some());
        assert!(workspace_ref.turn_run_id.is_some());
        let mount = outcome
            .run
            .workflow_state
            .current_workspace_mount_ref
            .as_ref()
            .expect("workspace mount ref must be stored on run state");
        assert_eq!(mount.alias, "/workspace");
        assert!(
            !mount.mount_id.starts_with('/'),
            "mount id must be an opaque ref, not a host path"
        );
    }

    #[tokio::test]
    async fn implementation_stage_receives_mount_ref() {
        let workspace = Arc::new(FakeWorkspaceManager::new());
        let policy = policy_with_workspace(workspace);
        let after_planning = planning_completed_run(&policy).await;

        policy.tick(after_planning).await.unwrap();

        let requests = policy.ports().stage_turns.requests().await;
        let implementation = requests.last().expect("implementation request");
        assert_eq!(
            implementation.stage_turn_identity.stage,
            GithubIssueStage::Implementation
        );
        assert_eq!(
            implementation
                .workspace_mount_ref
                .as_ref()
                .map(|mount| mount.alias.as_str()),
            Some("/workspace")
        );
        assert!(implementation.prompt.content.contains("/workspace"));
        assert!(implementation.prompt.content.contains("main"));
        assert!(
            !implementation.prompt.content.contains("/tmp/"),
            "implementation prompt must not contain raw host temp paths"
        );
    }

    #[tokio::test]
    async fn workspace_prepare_failure_blocks_run_retryably() {
        let workspace = Arc::new(FakeWorkspaceManager::fail_once());
        let policy = policy_with_workspace(workspace.clone());
        let after_planning = planning_completed_run(&policy).await;

        let blocked = policy.tick(after_planning.clone()).await.unwrap();

        assert_eq!(blocked.processed_event_count, 0);
        assert_eq!(
            blocked.run.workflow_state.mode,
            GithubIssueWorkflowMode::Planning
        );
        assert_eq!(policy.ports().stage_turns.requests().await.len(), 2);
        let workspace_step = blocked
            .steps
            .iter()
            .find(|step| step.step_name == "prepare_workspace")
            .expect("workspace step should be recorded");
        assert_eq!(workspace_step.status, WorkflowStepStatus::Retryable);
        assert_eq!(workspace_step.next_attempt_at, Some(fixed_time(60)));

        policy.ports().clock.set(fixed_time(61));
        let retry = policy.tick(after_planning).await.unwrap();

        assert_eq!(retry.processed_event_count, 1);
        assert_eq!(workspace.request_count().await, 2);
        assert_eq!(
            retry.run.workflow_state.mode,
            GithubIssueWorkflowMode::Implementation
        );
        assert_eq!(policy.ports().stage_turns.requests().await.len(), 3);
    }

    #[tokio::test]
    async fn workspace_prepare_policy_denial_is_not_retryable() {
        let workspace = Arc::new(FakeWorkspaceManager::denied());
        let policy = policy_with_workspace(workspace);
        let after_planning = planning_completed_run(&policy).await;

        let error = policy
            .tick(after_planning)
            .await
            .expect_err("permanent workspace denial should block through poller error path");

        assert!(matches!(
            error,
            GithubIssueWorkflowError::PolicyDenied { .. }
        ));
        assert_eq!(policy.ports().stage_turns.requests().await.len(), 2);
    }

    #[tokio::test]
    async fn completed_prepare_step_replays_legacy_result_shape() {
        let workspace = Arc::new(FakeWorkspaceManager::new());
        let policy = policy_with_workspace(workspace.clone());
        let after_planning = planning_completed_run(&policy).await;

        let workspace_session_id =
            GithubIssueWorkspaceSessionId::from_trusted("legacy-workspace-session".to_string())
                .unwrap();
        let step_input = serde_json::json!({
            "workflow_run_id": after_planning.workflow_run_id.clone(),
            "issue": after_planning.issue_ref.clone(),
            "policy_version": "workspace-contract-v1",
        });
        let idempotency_key = WorkflowIdempotencyKey::from_trusted(format!(
            "policy-step:workspace-contract-v1:{}:prepare_workspace",
            after_planning.workflow_run_id
        ))
        .unwrap();
        let step = match policy
            .ports()
            .repository
            .create_or_get_workflow_step(CreateOrGetWorkflowStepInput {
                workflow_run_id: after_planning.workflow_run_id.clone(),
                step_name: "prepare_workspace".to_string(),
                idempotency_key,
                input_hash: workflow_input_hash("prepare_workspace", &step_input),
                now: fixed_time(30),
            })
            .await
            .unwrap()
        {
            CreateOrGetWorkflowStepOutcome::Created { step }
            | CreateOrGetWorkflowStepOutcome::Existing { step } => step,
        };
        policy
            .ports()
            .repository
            .complete_workflow_step(CompleteWorkflowStepInput {
                step_run_id: step.step_run_id,
                status: WorkflowStepStatus::Succeeded,
                result: Some(serde_json::json!({
                    "workspace_session_id": workspace_session_id,
                    "workspace_ref": {
                        "thread_id": null,
                        "workspace_session_id": "legacy-workspace-session",
                        "turn_run_id": null
                    },
                    "mount_ref": {
                        "mount_id": "legacy-workspace-mount",
                        "alias": "/workspace"
                    }
                })),
                error: None,
                next_attempt_at: None,
                now: fixed_time(31),
            })
            .await
            .unwrap();

        let outcome = policy.tick(after_planning).await.unwrap();

        assert_eq!(workspace.request_count().await, 0);
        assert_eq!(
            outcome.run.workflow_state.mode,
            GithubIssueWorkflowMode::Implementation
        );
        assert_eq!(
            outcome
                .run
                .workflow_state
                .current_workspace_mount_ref
                .as_ref()
                .map(|mount| mount.mount_id.as_str()),
            Some("legacy-workspace-mount")
        );
    }

    fn workflow_input_hash(label: &str, value: &JsonValue) -> String {
        use sha2::{Digest, Sha256};

        let mut hasher = Sha256::new();
        hasher.update(label.as_bytes());
        hasher.update(serde_json::to_vec(value).unwrap());
        format!("sha256:{:x}", hasher.finalize())
    }
}
