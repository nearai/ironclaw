mod poller_contract {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex as StdMutex};

    use async_trait::async_trait;
    use chrono::{TimeZone, Utc};
    use ironclaw_github_issue_workflow::{
        AdvanceWorkflowRunInput, ClaimRunnableWorkflowRunsInput, CreateIssueCommentInput,
        CreateOrGetWorkflowRunInput, CreateOrGetWorkflowRunOutcome,
        GetAuthenticatedWorkflowActorInput, GetGithubIssueInput, GetPullRequestInput,
        GetStageRunInput, GithubActorSnapshot, GithubCheckConclusion, GithubCommentRef,
        GithubIssueBlockKind, GithubIssueCandidateSelector, GithubIssueCommentSnapshot,
        GithubIssueProviderSnapshot, GithubIssueSearchHit, GithubIssueStageRunId,
        GithubIssueWorkflowConfig, GithubIssueWorkflowConfigSource, GithubIssueWorkflowError,
        GithubIssueWorkflowEventType, GithubIssueWorkflowMode, GithubIssueWorkflowPolicyPorts,
        GithubIssueWorkflowPoller, GithubIssueWorkflowPollerConfig, GithubIssueWorkflowPollerPorts,
        GithubIssueWorkflowPort, GithubIssueWorkflowRepository, GithubIssueWorkflowRun,
        GithubIssueWorkflowRunStatus, GithubIssueWorkspaceSession, GithubIssueWorkspaceSessionId,
        GithubProviderAccountRef, GithubPullRequestCheckSnapshot, GithubPullRequestRef,
        GithubPullRequestSnapshot, GithubRepositorySelector, GithubReviewCommentSnapshot,
        InMemoryGithubIssueWorkflowRepository, ListIssueCommentsInput, ListPullRequestChecksInput,
        ListPullRequestReviewCommentsInput, PrepareWorkflowWorkspaceOutcome,
        PrepareWorkflowWorkspaceRequest, SearchGithubIssuesInput, StageTurnSubmitter,
        SubmitStageTurnOutcome, SubmitStageTurnRequest, TransitionOutcome, WorkflowClock,
        WorkflowConfigAccessRequest, WorkflowProjectAccess, WorkflowProjectAccessRequest,
        WorkflowRunTransition, WorkflowWorkerId, WorkflowWorkspaceManager,
        WorkflowWorkspaceMountRef, WorkflowWorkspaceRef,
    };
    use ironclaw_host_api::{ProjectId, TenantId, ThreadId, UserId};
    use ironclaw_turns::TurnRunId;
    use tokio::sync::Mutex;

    fn fixed_time(seconds: i64) -> chrono::DateTime<Utc> {
        Utc.timestamp_opt(seconds, 0).unwrap()
    }

    fn tenant(suffix: &str) -> TenantId {
        TenantId::new(format!("tenant-poller-{suffix}")).unwrap()
    }

    fn user(suffix: &str) -> UserId {
        UserId::new(format!("user-poller-{suffix}")).unwrap()
    }

    fn project(suffix: &str) -> ProjectId {
        ProjectId::new(format!("project-poller-{suffix}")).unwrap()
    }

    fn worker() -> WorkflowWorkerId {
        WorkflowWorkerId::from_trusted("poller-contract-worker".to_string()).unwrap()
    }

    fn provider_account() -> GithubProviderAccountRef {
        GithubProviderAccountRef {
            provider: "github".to_string(),
            account_id: "account-poller".to_string(),
        }
    }

    fn workflow_config(suffix: &str, owner: &str, repo: &str) -> GithubIssueWorkflowConfig {
        GithubIssueWorkflowConfig {
            tenant_id: tenant(suffix),
            project_id: project(suffix),
            owner_user_id: user(suffix),
            repositories: vec![GithubRepositorySelector {
                owner: owner.to_string(),
                repo: repo.to_string(),
            }],
            candidate_selector: Default::default(),
            max_active_runs_per_repo: 1,
            default_run_profile: "default".to_string(),
            provider_account_ref: provider_account(),
        }
    }

    fn issue_snapshot(
        owner: &str,
        repo: &str,
        number: u64,
        updated_at: i64,
    ) -> GithubIssueProviderSnapshot {
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
            updated_at: Some(fixed_time(updated_at)),
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

    fn comment(owner: &str, repo: &str, number: u64) -> GithubIssueCommentSnapshot {
        GithubIssueCommentSnapshot {
            comment: GithubCommentRef {
                node_id: Some(format!("comment-node-{repo}-{number}")),
                url: format!("https://github.com/{owner}/{repo}/issues/{number}#issuecomment-1"),
            },
            body: "I can reproduce this".to_string(),
            author_login: "octocat".to_string(),
            created_at: fixed_time(5),
            updated_at: fixed_time(6),
        }
    }

    fn pull_request_snapshot(updated_at: i64, merged: bool) -> GithubPullRequestSnapshot {
        GithubPullRequestSnapshot {
            pull_request: GithubPullRequestRef {
                owner: "nearai".to_string(),
                repo: "ironclaw".to_string(),
                number: 12,
                node_id: Some("pr-node-poller-12".to_string()),
                url: "https://github.com/nearai/ironclaw/pull/12".to_string(),
                head_branch: "ironclaw/fix-42".to_string(),
                head_sha: Some("head-sha-poller".to_string()),
            },
            title: "Fix bug 42".to_string(),
            body: "Draft PR".to_string(),
            state: if merged { "closed" } else { "open" }.to_string(),
            draft: false,
            merged,
            updated_at: Some(fixed_time(updated_at)),
        }
    }

    fn failed_check() -> GithubPullRequestCheckSnapshot {
        GithubPullRequestCheckSnapshot {
            suite_or_run_id: "clippy".to_string(),
            name: "clippy".to_string(),
            head_sha: "head-sha-poller".to_string(),
            conclusion: GithubCheckConclusion::Failure,
            completed_at: Some(fixed_time(310)),
            details_url: Some("https://github.com/nearai/ironclaw/actions/runs/1".to_string()),
        }
    }

    fn successful_check() -> GithubPullRequestCheckSnapshot {
        GithubPullRequestCheckSnapshot {
            suite_or_run_id: "test".to_string(),
            name: "test".to_string(),
            head_sha: "head-sha-poller".to_string(),
            conclusion: GithubCheckConclusion::Success,
            completed_at: Some(fixed_time(311)),
            details_url: Some("https://github.com/nearai/ironclaw/actions/runs/2".to_string()),
        }
    }

    fn review_comment() -> GithubReviewCommentSnapshot {
        GithubReviewCommentSnapshot {
            comment: GithubCommentRef {
                node_id: Some("review-comment-node-poller".to_string()),
                url: "https://github.com/nearai/ironclaw/pull/12#discussion_r1".to_string(),
            },
            body: "Please add a regression test.".to_string(),
            author_login: "reviewer".to_string(),
            created_at: fixed_time(320),
            updated_at: fixed_time(320),
        }
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
    struct FakeConfigSource {
        configs: Mutex<Vec<GithubIssueWorkflowConfig>>,
    }

    impl FakeConfigSource {
        fn new(configs: Vec<GithubIssueWorkflowConfig>) -> Self {
            Self {
                configs: Mutex::new(configs),
            }
        }
    }

    #[async_trait]
    impl GithubIssueWorkflowConfigSource for FakeConfigSource {
        async fn list_enabled_workflow_configs(
            &self,
        ) -> Result<Vec<GithubIssueWorkflowConfig>, GithubIssueWorkflowError> {
            Ok(self.configs.lock().await.clone())
        }
    }

    #[derive(Debug, Default)]
    struct FakeGithubPort {
        search_results: Mutex<HashMap<(String, String), Vec<GithubIssueSearchHit>>>,
        issue_snapshots: Mutex<HashMap<(String, String, u64), GithubIssueProviderSnapshot>>,
        comments: Mutex<HashMap<(String, String, u64), Vec<GithubIssueCommentSnapshot>>>,
        pull_requests: Mutex<HashMap<(String, String, u64), GithubPullRequestSnapshot>>,
        checks: Mutex<HashMap<(String, String, u64), Vec<GithubPullRequestCheckSnapshot>>>,
        review_comments: Mutex<HashMap<(String, String, u64), Vec<GithubReviewCommentSnapshot>>>,
        rate_limited_repos: Mutex<Vec<(String, String)>>,
        search_calls: Mutex<Vec<SearchGithubIssuesInput>>,
        get_issue_calls: Mutex<Vec<GetGithubIssueInput>>,
        list_comment_calls: Mutex<Vec<ListIssueCommentsInput>>,
        get_pull_request_calls: Mutex<Vec<GetPullRequestInput>>,
        list_check_calls: Mutex<Vec<ListPullRequestChecksInput>>,
        list_review_comment_calls: Mutex<Vec<ListPullRequestReviewCommentsInput>>,
        create_comment_bodies: Mutex<Vec<String>>,
    }

    impl FakeGithubPort {
        async fn add_issue(&self, snapshot: GithubIssueProviderSnapshot) {
            let key = (snapshot.owner.clone(), snapshot.repo.clone());
            self.search_results
                .lock()
                .await
                .entry(key)
                .or_default()
                .push(issue_hit(&snapshot));
            self.comments.lock().await.insert(
                (
                    snapshot.owner.clone(),
                    snapshot.repo.clone(),
                    snapshot.number,
                ),
                vec![comment(&snapshot.owner, &snapshot.repo, snapshot.number)],
            );
            self.issue_snapshots.lock().await.insert(
                (
                    snapshot.owner.clone(),
                    snapshot.repo.clone(),
                    snapshot.number,
                ),
                snapshot,
            );
        }

        async fn add_search_hit_for_repo(
            &self,
            owner: &str,
            repo: &str,
            hit: GithubIssueSearchHit,
        ) {
            self.search_results
                .lock()
                .await
                .entry((owner.to_string(), repo.to_string()))
                .or_default()
                .push(hit);
        }

        async fn add_issue_snapshot_for_request(
            &self,
            request_owner: &str,
            request_repo: &str,
            number: u64,
            snapshot: GithubIssueProviderSnapshot,
        ) {
            self.issue_snapshots.lock().await.insert(
                (request_owner.to_string(), request_repo.to_string(), number),
                snapshot,
            );
        }

        async fn clear_search_results(&self, owner: &str, repo: &str) {
            self.search_results
                .lock()
                .await
                .insert((owner.to_string(), repo.to_string()), Vec::new());
        }

        async fn set_issue_snapshot(&self, snapshot: GithubIssueProviderSnapshot) {
            self.issue_snapshots.lock().await.insert(
                (
                    snapshot.owner.clone(),
                    snapshot.repo.clone(),
                    snapshot.number,
                ),
                snapshot,
            );
        }

        async fn set_pull_request(&self, snapshot: GithubPullRequestSnapshot) {
            self.pull_requests.lock().await.insert(
                (
                    snapshot.pull_request.owner.clone(),
                    snapshot.pull_request.repo.clone(),
                    snapshot.pull_request.number,
                ),
                snapshot,
            );
        }

        async fn set_checks(
            &self,
            pull_request: &GithubPullRequestRef,
            checks: Vec<GithubPullRequestCheckSnapshot>,
        ) {
            self.checks.lock().await.insert(
                (
                    pull_request.owner.clone(),
                    pull_request.repo.clone(),
                    pull_request.number,
                ),
                checks,
            );
        }

        async fn set_review_comments(
            &self,
            pull_request: &GithubPullRequestRef,
            comments: Vec<GithubReviewCommentSnapshot>,
        ) {
            self.review_comments.lock().await.insert(
                (
                    pull_request.owner.clone(),
                    pull_request.repo.clone(),
                    pull_request.number,
                ),
                comments,
            );
        }

        async fn rate_limit_repo(&self, owner: &str, repo: &str) {
            self.rate_limited_repos
                .lock()
                .await
                .push((owner.to_string(), repo.to_string()));
        }

        async fn search_calls(&self) -> Vec<SearchGithubIssuesInput> {
            self.search_calls.lock().await.clone()
        }

        async fn get_issue_calls(&self) -> Vec<GetGithubIssueInput> {
            self.get_issue_calls.lock().await.clone()
        }

        async fn list_comment_calls(&self) -> Vec<ListIssueCommentsInput> {
            self.list_comment_calls.lock().await.clone()
        }

        async fn get_pull_request_calls(&self) -> Vec<GetPullRequestInput> {
            self.get_pull_request_calls.lock().await.clone()
        }

        async fn list_check_calls(&self) -> Vec<ListPullRequestChecksInput> {
            self.list_check_calls.lock().await.clone()
        }

        async fn list_review_comment_calls(&self) -> Vec<ListPullRequestReviewCommentsInput> {
            self.list_review_comment_calls.lock().await.clone()
        }

        async fn created_comment_count(&self) -> usize {
            self.create_comment_bodies.lock().await.len()
        }
    }

    #[async_trait]
    impl GithubIssueWorkflowPort for FakeGithubPort {
        async fn search_open_bug_issues(
            &self,
            input: SearchGithubIssuesInput,
        ) -> Result<Vec<GithubIssueSearchHit>, GithubIssueWorkflowError> {
            self.search_calls.lock().await.push(input.clone());
            if self
                .rate_limited_repos
                .lock()
                .await
                .contains(&(input.owner.clone(), input.repo.clone()))
            {
                return Err(GithubIssueWorkflowError::ProviderRateLimited {
                    reason: format!("{}/{} search was rate limited", input.owner, input.repo),
                });
            }
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
            self.get_issue_calls.lock().await.push(input.clone());
            self.issue_snapshots
                .lock()
                .await
                .get(&(input.owner, input.repo, input.number))
                .cloned()
                .ok_or_else(|| GithubIssueWorkflowError::ProviderRead {
                    reason: "missing fake issue snapshot".to_string(),
                })
        }

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
            input: ListIssueCommentsInput,
        ) -> Result<Vec<GithubIssueCommentSnapshot>, GithubIssueWorkflowError> {
            self.list_comment_calls.lock().await.push(input.clone());
            Ok(self
                .comments
                .lock()
                .await
                .get(&(input.issue.owner, input.issue.repo, input.issue.number))
                .cloned()
                .unwrap_or_default())
        }

        async fn get_pull_request(
            &self,
            input: GetPullRequestInput,
        ) -> Result<GithubPullRequestSnapshot, GithubIssueWorkflowError> {
            self.get_pull_request_calls.lock().await.push(input.clone());
            self.pull_requests
                .lock()
                .await
                .get(&(input.owner, input.repo, input.number))
                .cloned()
                .ok_or_else(|| GithubIssueWorkflowError::ProviderRead {
                    reason: "missing fake pull request snapshot".to_string(),
                })
        }

        async fn list_pull_request_checks(
            &self,
            input: ListPullRequestChecksInput,
        ) -> Result<Vec<GithubPullRequestCheckSnapshot>, GithubIssueWorkflowError> {
            self.list_check_calls.lock().await.push(input.clone());
            Ok(self
                .checks
                .lock()
                .await
                .get(&(input.owner, input.repo, input.pull_request_number))
                .cloned()
                .unwrap_or_default())
        }

        async fn list_pull_request_review_comments(
            &self,
            input: ListPullRequestReviewCommentsInput,
        ) -> Result<Vec<GithubReviewCommentSnapshot>, GithubIssueWorkflowError> {
            self.list_review_comment_calls
                .lock()
                .await
                .push(input.clone());
            Ok(self
                .review_comments
                .lock()
                .await
                .get(&(input.owner, input.repo, input.pull_request_number))
                .cloned()
                .unwrap_or_default())
        }

        async fn create_issue_comment(
            &self,
            input: CreateIssueCommentInput,
        ) -> Result<GithubCommentRef, GithubIssueWorkflowError> {
            self.create_comment_bodies.lock().await.push(input.body);
            Ok(GithubCommentRef {
                node_id: Some("created-comment-node".to_string()),
                url: "https://github.com/nearai/ironclaw/issues/42#issuecomment-created"
                    .to_string(),
            })
        }
    }

    #[derive(Debug)]
    struct FakeProjectAccess {
        config_allowed: bool,
        config_requests: Mutex<Vec<WorkflowConfigAccessRequest>>,
        run_requests: Mutex<Vec<WorkflowProjectAccessRequest>>,
    }

    impl FakeProjectAccess {
        fn allow() -> Self {
            Self {
                config_allowed: true,
                config_requests: Mutex::new(Vec::new()),
                run_requests: Mutex::new(Vec::new()),
            }
        }

        fn deny_config() -> Self {
            Self {
                config_allowed: false,
                config_requests: Mutex::new(Vec::new()),
                run_requests: Mutex::new(Vec::new()),
            }
        }

        async fn config_request_count(&self) -> usize {
            self.config_requests.lock().await.len()
        }
    }

    #[async_trait]
    impl WorkflowProjectAccess for FakeProjectAccess {
        async fn assert_workflow_config_access(
            &self,
            request: WorkflowConfigAccessRequest,
        ) -> Result<(), GithubIssueWorkflowError> {
            self.config_requests.lock().await.push(request);
            if self.config_allowed {
                return Ok(());
            }
            Err(GithubIssueWorkflowError::PolicyDenied {
                reason: "config access denied".to_string(),
            })
        }

        async fn assert_workflow_project_access(
            &self,
            request: WorkflowProjectAccessRequest,
        ) -> Result<(), GithubIssueWorkflowError> {
            self.run_requests.lock().await.push(request);
            Ok(())
        }
    }

    #[derive(Debug, Default)]
    struct MissingConfigAccessProjectAccess {
        run_requests: Mutex<Vec<WorkflowProjectAccessRequest>>,
    }

    #[async_trait]
    impl WorkflowProjectAccess for MissingConfigAccessProjectAccess {
        async fn assert_workflow_project_access(
            &self,
            request: WorkflowProjectAccessRequest,
        ) -> Result<(), GithubIssueWorkflowError> {
            self.run_requests.lock().await.push(request);
            Ok(())
        }
    }

    #[derive(Debug, Default)]
    struct FakeStageTurnSubmitter {
        requests: Mutex<Vec<SubmitStageTurnRequest>>,
    }

    impl FakeStageTurnSubmitter {
        async fn request_count(&self) -> usize {
            self.requests.lock().await.len()
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
                thread_id: ThreadId::new(format!("thread-poller-{request_count}")).unwrap(),
                turn_run_id: TurnRunId::new(),
            })
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
                GithubIssueWorkspaceSessionId::from_trusted("poller-workspace-session".to_string())
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
                    working_branch: "ironclaw/poller-workspace-session".to_string(),
                    current_head_sha: None,
                    workspace_ref: WorkflowWorkspaceRef {
                        thread_id: None,
                        workspace_session_id: Some(workspace_session_id),
                        turn_run_id: None,
                    },
                    mount_ref: WorkflowWorkspaceMountRef {
                        mount_id: "mount-poller".to_string(),
                        alias: "/workspace".to_string(),
                    },
                    created_at: request.requested_at,
                },
            })
        }
    }

    #[derive(Debug)]
    struct FakePollerPorts<A = FakeProjectAccess> {
        repository: Arc<InMemoryGithubIssueWorkflowRepository>,
        configs: Arc<FakeConfigSource>,
        github: Arc<FakeGithubPort>,
        project_access: Arc<A>,
        stage_turns: Arc<FakeStageTurnSubmitter>,
        workspace: Arc<FakeWorkspaceManager>,
        clock: Arc<FakeClock>,
        worker_id: WorkflowWorkerId,
    }

    impl FakePollerPorts<FakeProjectAccess> {
        fn new(configs: Vec<GithubIssueWorkflowConfig>) -> Self {
            Self {
                repository: Arc::new(InMemoryGithubIssueWorkflowRepository::default()),
                configs: Arc::new(FakeConfigSource::new(configs)),
                github: Arc::new(FakeGithubPort::default()),
                project_access: Arc::new(FakeProjectAccess::allow()),
                stage_turns: Arc::new(FakeStageTurnSubmitter::default()),
                workspace: Arc::new(FakeWorkspaceManager),
                clock: Arc::new(FakeClock::new(fixed_time(100))),
                worker_id: worker(),
            }
        }
    }

    impl<A> FakePollerPorts<A> {
        fn with_project_access<B>(self, project_access: Arc<B>) -> FakePollerPorts<B> {
            FakePollerPorts {
                repository: self.repository,
                configs: self.configs,
                github: self.github,
                project_access,
                stage_turns: self.stage_turns,
                workspace: self.workspace,
                clock: self.clock,
                worker_id: self.worker_id,
            }
        }
    }

    impl<A> GithubIssueWorkflowPollerPorts for FakePollerPorts<A>
    where
        A: WorkflowProjectAccess,
    {
        type Clock = FakeClock;
        type ConfigSource = FakeConfigSource;
        type GithubPort = FakeGithubPort;
        type ProjectAccess = A;
        type Repository = InMemoryGithubIssueWorkflowRepository;
        type StageTurnSubmitter = FakeStageTurnSubmitter;
        type WorkspaceManager = FakeWorkspaceManager;

        fn clock(&self) -> Arc<Self::Clock> {
            self.clock.clone()
        }

        fn config_source(&self) -> Arc<Self::ConfigSource> {
            self.configs.clone()
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

    impl<A> GithubIssueWorkflowPolicyPorts for FakePollerPorts<A>
    where
        A: WorkflowProjectAccess,
    {
        type Clock = FakeClock;
        type GithubPort = FakeGithubPort;
        type ProjectAccess = A;
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

    fn poller_config() -> GithubIssueWorkflowPollerConfig {
        GithubIssueWorkflowPollerConfig {
            enabled: true,
            ..GithubIssueWorkflowPollerConfig::default()
        }
    }

    fn poller(ports: FakePollerPorts) -> GithubIssueWorkflowPoller<FakePollerPorts> {
        GithubIssueWorkflowPoller::new(ports, poller_config(), "poller-contract-v1")
    }

    fn generic_poller<A>(ports: FakePollerPorts<A>) -> GithubIssueWorkflowPoller<FakePollerPorts<A>>
    where
        A: WorkflowProjectAccess,
    {
        GithubIssueWorkflowPoller::new(ports, poller_config(), "poller-contract-v1")
    }

    async fn existing_run(
        repository: &InMemoryGithubIssueWorkflowRepository,
        config: &GithubIssueWorkflowConfig,
        snapshot: &GithubIssueProviderSnapshot,
    ) -> GithubIssueWorkflowRun {
        let issue = snapshot.issue_ref();
        match repository
            .create_or_get_workflow_run(CreateOrGetWorkflowRunInput {
                tenant_id: config.tenant_id.clone(),
                creator_user_id: config.owner_user_id.clone(),
                agent_id: None,
                project_id: Some(config.project_id.clone()),
                provider_account_ref: Some(config.provider_account_ref.clone()),
                issue_ref: issue,
                workflow_policy_key: "github-bug-workflow".to_string(),
                workflow_policy_version: "poller-contract-v1".to_string(),
                now: fixed_time(200),
            })
            .await
            .unwrap()
        {
            CreateOrGetWorkflowRunOutcome::Existing { run } => run,
            CreateOrGetWorkflowRunOutcome::Created { .. } => {
                panic!("poller should have created the workflow run")
            }
        }
    }

    async fn set_run_primary_pr(
        repository: &InMemoryGithubIssueWorkflowRepository,
        run: GithubIssueWorkflowRun,
        pull_request: GithubPullRequestRef,
    ) -> GithubIssueWorkflowRun {
        let claimed = repository
            .claim_runnable_workflow_runs(ClaimRunnableWorkflowRunsInput {
                tenant_id: run.tenant_id.clone(),
                worker_id: worker(),
                now: fixed_time(250),
                lease_expires_at: fixed_time(310),
                limit: 1,
            })
            .await
            .unwrap()
            .pop()
            .expect("run should be claimable for test transition");
        let outcome = repository
            .advance_event_cursor_and_transition(AdvanceWorkflowRunInput {
                workflow_run_id: claimed.workflow_run_id.clone(),
                worker_id: worker(),
                expected_workflow_run_version: claimed.workflow_run_version,
                expected_event_cursor: claimed.event_cursor,
                next_event_cursor: claimed.event_cursor,
                transition: WorkflowRunTransition {
                    mode: Some(GithubIssueWorkflowMode::PrOpen),
                    primary_pr: Some(pull_request),
                    clear_active_block: true,
                    ..WorkflowRunTransition::default()
                },
                now: fixed_time(251),
            })
            .await
            .unwrap();
        match outcome {
            TransitionOutcome::Applied { run } => run,
            other => panic!("primary PR transition should apply: {other:?}"),
        }
    }

    #[tokio::test]
    async fn poller_discovers_bug_issue_and_records_event() {
        assert!(!GithubIssueWorkflowPollerConfig::default().enabled);
        let config = workflow_config("discover", "nearai", "ironclaw");
        let ports = FakePollerPorts::new(vec![config.clone()]);
        let mut snapshot = issue_snapshot("nearai", "ironclaw", 42, 100);
        snapshot.body =
            "Please add a canary file containing GitHub issue workflow poller evidence."
                .to_string();
        ports.github.add_issue(snapshot.clone()).await;
        let poller = poller(ports);

        let outcome = poller.tick_once().await.unwrap();

        assert_eq!(outcome.configs_loaded, 1);
        assert_eq!(outcome.repositories_scanned, 1);
        assert_eq!(outcome.issues_seen, 1);
        assert_eq!(outcome.events_recorded, 1);
        assert_eq!(outcome.policy_ticks, 1);
        let run = existing_run(&poller.ports().repository, &config, &snapshot).await;
        let events = poller
            .ports()
            .repository
            .list_workflow_events_after(
                ironclaw_github_issue_workflow::ListWorkflowEventsAfterInput {
                    workflow_run_id: run.workflow_run_id.clone(),
                    after_sequence: 0,
                    limit: 10,
                },
            )
            .await
            .unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0].workflow_event_type,
            GithubIssueWorkflowEventType::GithubIssueDiscovered
        );
        assert_eq!(events[0].payload["issue"]["number"], 42);
        assert_eq!(events[0].payload["provider_snapshot"]["comment_count"], 1);
        assert!(
            events[0].payload["provider_snapshot"]["content_summaries"][0]["summary"]
                .as_str()
                .unwrap()
                .contains("GitHub issue workflow poller evidence")
        );
        assert_eq!(
            events[0].payload["provider_snapshot"]["content_summaries"][0]["trust"],
            "untrusted_provider_content"
        );
        let search_calls = poller.ports().github.search_calls().await;
        assert_eq!(
            search_calls[0].query,
            "repo:nearai/ironclaw is:issue state:open label:bug"
        );
    }

    #[tokio::test]
    async fn poller_skips_issue_from_author_outside_allowlist_before_comments_or_run_creation() {
        let mut config = workflow_config("author-skip", "nearai", "ironclaw");
        config.candidate_selector = GithubIssueCandidateSelector {
            labels: vec!["bug".to_string()],
            allowed_author_logins: vec!["core-dev".to_string()],
        };
        let ports = FakePollerPorts::new(vec![config.clone()]);
        let mut snapshot = issue_snapshot("nearai", "ironclaw", 42, 100);
        snapshot.author_login = Some("drive-by-reporter".to_string());
        ports.github.add_issue(snapshot.clone()).await;
        let poller = poller(ports);

        let outcome = poller.tick_once().await.unwrap();

        assert_eq!(outcome.issues_seen, 1);
        assert_eq!(outcome.events_recorded, 0);
        assert_eq!(outcome.policy_ticks, 0);
        assert_eq!(poller.ports().github.get_issue_calls().await.len(), 1);
        assert!(poller.ports().github.list_comment_calls().await.is_empty());
        let existing = poller
            .ports()
            .repository
            .create_or_get_workflow_run(CreateOrGetWorkflowRunInput {
                tenant_id: config.tenant_id.clone(),
                creator_user_id: config.owner_user_id.clone(),
                agent_id: None,
                project_id: Some(config.project_id.clone()),
                provider_account_ref: Some(config.provider_account_ref.clone()),
                issue_ref: snapshot.issue_ref(),
                workflow_policy_key: "github-bug-workflow".to_string(),
                workflow_policy_version: "poller-contract-v1".to_string(),
                now: fixed_time(200),
            })
            .await
            .unwrap();
        assert!(
            matches!(existing, CreateOrGetWorkflowRunOutcome::Created { .. }),
            "poller must not create a workflow run for an issue author outside the allowlist"
        );
    }

    #[tokio::test]
    async fn poller_allows_issue_from_author_allowlist_case_insensitively() {
        let mut config = workflow_config("author-allow", "nearai", "ironclaw");
        config.candidate_selector = GithubIssueCandidateSelector {
            labels: vec!["bug".to_string()],
            allowed_author_logins: vec!["CORE-DEV".to_string()],
        };
        let ports = FakePollerPorts::new(vec![config.clone()]);
        let snapshot = issue_snapshot("nearai", "ironclaw", 42, 100);
        ports.github.add_issue(snapshot.clone()).await;
        let poller = poller(ports);

        let outcome = poller.tick_once().await.unwrap();

        assert_eq!(outcome.events_recorded, 1);
        let run = existing_run(&poller.ports().repository, &config, &snapshot).await;
        assert_eq!(run.issue_ref.number, 42);
    }

    #[tokio::test]
    async fn poller_dedupes_same_issue_on_second_tick() {
        let config = workflow_config("dedupe", "nearai", "ironclaw");
        let ports = FakePollerPorts::new(vec![config.clone()]);
        let snapshot = issue_snapshot("nearai", "ironclaw", 42, 100);
        ports.github.add_issue(snapshot.clone()).await;
        let poller = poller(ports);

        poller.tick_once().await.unwrap();
        let second = poller.tick_once().await.unwrap();

        let run = existing_run(&poller.ports().repository, &config, &snapshot).await;
        let events = poller
            .ports()
            .repository
            .list_workflow_events_after(
                ironclaw_github_issue_workflow::ListWorkflowEventsAfterInput {
                    workflow_run_id: run.workflow_run_id,
                    after_sequence: 0,
                    limit: 10,
                },
            )
            .await
            .unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(second.events_recorded, 0);
        assert_eq!(second.events_deduped, 1);
        assert_eq!(poller.ports().stage_turns.request_count().await, 1);
        assert_eq!(poller.ports().github.created_comment_count().await, 1);
    }

    #[tokio::test]
    async fn poller_checks_project_access_before_github_read() {
        let config = workflow_config("denied", "nearai", "ironclaw");
        let project_access = Arc::new(FakeProjectAccess::deny_config());
        let ports = FakePollerPorts::new(vec![config]).with_project_access(project_access.clone());
        let poller = poller(ports);

        let outcome = poller.tick_once().await.unwrap();

        assert_eq!(outcome.blocked_configs.len(), 1);
        assert_eq!(project_access.config_request_count().await, 1);
        assert!(poller.ports().github.search_calls().await.is_empty());
        assert!(poller.ports().github.get_issue_calls().await.is_empty());
    }

    #[tokio::test]
    async fn poller_default_denies_config_access_when_not_implemented() {
        let config = workflow_config("default-denied", "nearai", "ironclaw");
        let ports = FakePollerPorts::new(vec![config])
            .with_project_access(Arc::new(MissingConfigAccessProjectAccess::default()));
        ports
            .github
            .add_issue(issue_snapshot("nearai", "ironclaw", 42, 100))
            .await;
        let poller = generic_poller(ports);

        let outcome = poller.tick_once().await.unwrap();

        assert_eq!(outcome.blocked_configs.len(), 1);
        assert!(poller.ports().github.search_calls().await.is_empty());
        assert!(poller.ports().github.get_issue_calls().await.is_empty());
    }

    #[tokio::test]
    async fn poller_uses_configured_repo_for_reads_when_search_hit_points_elsewhere() {
        let config = workflow_config("cross-hit", "nearai", "ironclaw");
        let ports = FakePollerPorts::new(vec![config.clone()]);
        let allowed_snapshot = issue_snapshot("nearai", "ironclaw", 42, 100);
        let mut cross_repo_hit = issue_hit(&issue_snapshot("attacker", "shadow", 42, 100));
        cross_repo_hit.number = allowed_snapshot.number;
        ports
            .github
            .add_search_hit_for_repo("nearai", "ironclaw", cross_repo_hit)
            .await;
        ports
            .github
            .add_issue_snapshot_for_request(
                "nearai",
                "ironclaw",
                allowed_snapshot.number,
                allowed_snapshot.clone(),
            )
            .await;
        let poller = poller(ports);

        let outcome = poller.tick_once().await.unwrap();

        assert_eq!(outcome.events_recorded, 1);
        let get_issue_calls = poller.ports().github.get_issue_calls().await;
        assert_eq!(get_issue_calls.len(), 1);
        assert_eq!(get_issue_calls[0].owner, "nearai");
        assert_eq!(get_issue_calls[0].repo, "ironclaw");
        let comment_calls = poller.ports().github.list_comment_calls().await;
        assert!(!comment_calls.is_empty());
        assert!(
            comment_calls
                .iter()
                .all(|call| call.issue.owner == "nearai" && call.issue.repo == "ironclaw")
        );
        let run = existing_run(&poller.ports().repository, &config, &allowed_snapshot).await;
        assert_eq!(run.issue_ref.owner, "nearai");
        assert_eq!(run.issue_ref.repo, "ironclaw");
    }

    #[tokio::test]
    async fn poller_blocks_provider_snapshot_for_unchecked_repo_before_comments() {
        let config = workflow_config("cross-snapshot", "nearai", "ironclaw");
        let ports = FakePollerPorts::new(vec![config]);
        let checked_hit = issue_hit(&issue_snapshot("nearai", "ironclaw", 42, 100));
        let unchecked_snapshot = issue_snapshot("attacker", "shadow", 42, 100);
        ports
            .github
            .add_search_hit_for_repo("nearai", "ironclaw", checked_hit)
            .await;
        ports
            .github
            .add_issue_snapshot_for_request(
                "nearai",
                "ironclaw",
                unchecked_snapshot.number,
                unchecked_snapshot,
            )
            .await;
        let poller = poller(ports);

        let outcome = poller.tick_once().await.unwrap();

        assert_eq!(outcome.blocked_configs.len(), 1);
        let get_issue_calls = poller.ports().github.get_issue_calls().await;
        assert_eq!(get_issue_calls.len(), 1);
        assert_eq!(get_issue_calls[0].owner, "nearai");
        assert_eq!(get_issue_calls[0].repo, "ironclaw");
        assert!(poller.ports().github.list_comment_calls().await.is_empty());
    }

    #[tokio::test]
    async fn poller_applies_per_repo_issue_limit() {
        let config = workflow_config("limit", "nearai", "ironclaw");
        let ports = FakePollerPorts::new(vec![config]);
        ports
            .github
            .add_issue(issue_snapshot("nearai", "ironclaw", 41, 100))
            .await;
        ports
            .github
            .add_issue(issue_snapshot("nearai", "ironclaw", 42, 101))
            .await;
        ports
            .github
            .add_issue(issue_snapshot("nearai", "ironclaw", 43, 102))
            .await;
        let poller = GithubIssueWorkflowPoller::new(
            ports,
            GithubIssueWorkflowPollerConfig {
                enabled: true,
                max_issues_per_repo_per_tick: 2,
                ..GithubIssueWorkflowPollerConfig::default()
            },
            "poller-contract-v1",
        );

        let outcome = poller.tick_once().await.unwrap();

        assert_eq!(outcome.issues_seen, 2);
        assert_eq!(outcome.events_recorded, 2);
        let search_calls = poller.ports().github.search_calls().await;
        assert_eq!(search_calls[0].limit, 2);
        let get_issue_calls = poller.ports().github.get_issue_calls().await;
        assert_eq!(get_issue_calls.len(), 2);
        assert_eq!(get_issue_calls[0].number, 41);
        assert_eq!(get_issue_calls[1].number, 42);
    }

    #[tokio::test]
    async fn poller_ticks_runnable_runs_after_discovery() {
        let config = workflow_config("tick", "nearai", "ironclaw");
        let ports = FakePollerPorts::new(vec![config.clone()]);
        let snapshot = issue_snapshot("nearai", "ironclaw", 42, 100);
        ports.github.add_issue(snapshot.clone()).await;
        let poller = poller(ports);

        let outcome = poller.tick_once().await.unwrap();

        assert_eq!(outcome.runnable_runs_claimed, 1);
        assert_eq!(outcome.policy_ticks, 1);
        assert_eq!(poller.ports().stage_turns.request_count().await, 1);
        let run = existing_run(&poller.ports().repository, &config, &snapshot).await;
        assert_eq!(run.event_cursor, 1);
        assert!(run.active_stage_run_id.is_some());
        assert!(run.lease_owner.is_none());
    }

    #[tokio::test]
    async fn poller_reconciles_stale_active_stage_to_recovery_required() {
        let config = workflow_config("reconcile-stale", "nearai", "ironclaw");
        let ports = FakePollerPorts::new(vec![config.clone()]);
        let snapshot = issue_snapshot("nearai", "ironclaw", 42, 100);
        ports.github.add_issue(snapshot.clone()).await;
        let poller = GithubIssueWorkflowPoller::new(
            ports,
            GithubIssueWorkflowPollerConfig {
                enabled: true,
                stage_stale_after: std::time::Duration::from_secs(60),
                ..GithubIssueWorkflowPollerConfig::default()
            },
            "poller-contract-v1",
        );

        // Tick 1: discover -> create run -> create active stage -> submit turn.
        let first = poller.tick_once().await.unwrap();
        assert_eq!(first.stale_stages_failed, 0);
        let run = existing_run(&poller.ports().repository, &config, &snapshot).await;
        let stage_run_id = run
            .active_stage_run_id
            .clone()
            .expect("active stage after first tick");

        // The stage turn never reports a result; advance well past the
        // staleness threshold so the next no-progress tick reconciles it.
        let now = poller.ports().clock.now();
        poller
            .ports()
            .clock
            .set(now + chrono::Duration::seconds(120));

        // Tick 2: no new event -> reconciler escalates the stale stage.
        let second = poller.tick_once().await.unwrap();
        assert_eq!(second.stale_stages_failed, 1);
        assert_eq!(second.blocked_runs.len(), 1);
        assert_eq!(
            second.blocked_runs[0].kind,
            GithubIssueBlockKind::RecoveryRequired
        );

        let blocked = existing_run(&poller.ports().repository, &config, &snapshot).await;
        assert_eq!(blocked.status, GithubIssueWorkflowRunStatus::Blocked);
        assert_eq!(
            blocked
                .workflow_state
                .active_block
                .as_ref()
                .map(|block| block.kind.clone()),
            Some(GithubIssueBlockKind::RecoveryRequired)
        );
        assert!(blocked.active_stage_run_id.is_none());

        let stage = poller
            .ports()
            .repository
            .get_stage_run(GetStageRunInput {
                workflow_run_id: blocked.workflow_run_id.clone(),
                stage_run_id,
            })
            .await
            .unwrap()
            .expect("stage row persists");
        assert!(!stage.active);
        assert!(stage.failed);
    }

    #[tokio::test]
    async fn poller_does_not_reconcile_fresh_active_stage() {
        let config = workflow_config("reconcile-fresh", "nearai", "ironclaw");
        let ports = FakePollerPorts::new(vec![config.clone()]);
        let snapshot = issue_snapshot("nearai", "ironclaw", 42, 100);
        ports.github.add_issue(snapshot.clone()).await;
        let poller = GithubIssueWorkflowPoller::new(
            ports,
            GithubIssueWorkflowPollerConfig {
                enabled: true,
                stage_stale_after: std::time::Duration::from_secs(60),
                ..GithubIssueWorkflowPollerConfig::default()
            },
            "poller-contract-v1",
        );

        poller.tick_once().await.unwrap();
        // Advance only halfway to the staleness threshold: a healthy in-flight
        // stage must NOT be reconciled.
        let now = poller.ports().clock.now();
        poller
            .ports()
            .clock
            .set(now + chrono::Duration::seconds(30));

        let second = poller.tick_once().await.unwrap();
        assert_eq!(second.stale_stages_failed, 0);

        let run = existing_run(&poller.ports().repository, &config, &snapshot).await;
        assert_ne!(run.status, GithubIssueWorkflowRunStatus::Blocked);
        assert!(run.active_stage_run_id.is_some());
        assert!(run.workflow_state.active_block.is_none());
    }

    #[tokio::test]
    async fn poller_reconciles_orphan_active_stage_pointer_to_recovery_required() {
        // Orphan crash state: `active_stage_run_id` is set but NO stage row
        // backs it (a crash between the run-pointer write and the stage-row
        // write). `get_stage_run` returns None, so the stuck-stage reconciler
        // cannot read a stage-level heartbeat — it must instead escalate the run
        // to RecoveryRequired once the run itself has carried the orphan pointer
        // for longer than `stage_stale_after`, rather than re-claiming it as a
        // permanent no-op every tick.
        let config = workflow_config("reconcile-orphan", "nearai", "ironclaw");
        let ports = FakePollerPorts::new(vec![config.clone()]);
        // No search hits: discovery records nothing, so the only run is the one
        // we seed directly with the orphan pointer.
        let snapshot = issue_snapshot("nearai", "ironclaw", 42, 100);
        let run = create_run(&ports.repository, &config, &snapshot).await;
        // `create_run` stamps `created_at = fixed_time(200)`. Point the run at a
        // fabricated stage id that has no backing stage row.
        let orphan_stage_run_id =
            GithubIssueStageRunId::from_trusted("orphan-stage-run-id".to_string()).unwrap();
        ports
            .repository
            .seed_orphan_active_stage_pointer(
                &run.workflow_run_id,
                orphan_stage_run_id.clone(),
                fixed_time(205),
            )
            .await
            .unwrap();
        let poller = GithubIssueWorkflowPoller::new(
            ports,
            GithubIssueWorkflowPollerConfig {
                enabled: true,
                stage_stale_after: std::time::Duration::from_secs(60),
                ..GithubIssueWorkflowPollerConfig::default()
            },
            "poller-contract-v1",
        );

        // Confirm the orphan really has no backing stage row before the tick.
        assert!(
            poller
                .ports()
                .repository
                .get_stage_run(GetStageRunInput {
                    workflow_run_id: run.workflow_run_id.clone(),
                    stage_run_id: orphan_stage_run_id.clone(),
                })
                .await
                .unwrap()
                .is_none(),
            "seeded orphan pointer must have no backing stage row"
        );

        // Advance well past `stage_stale_after` measured from the run's
        // `created_at` (the claim refreshes `updated_at`/`last_heartbeat_at`
        // every tick, so only `created_at` is a stable staleness anchor).
        poller.ports().clock.set(fixed_time(400));

        let outcome = poller.tick_once().await.unwrap();

        assert_eq!(outcome.runnable_runs_claimed, 1);
        assert_eq!(outcome.stale_stages_failed, 1);
        assert_eq!(outcome.blocked_runs.len(), 1);
        assert_eq!(
            outcome.blocked_runs[0].kind,
            GithubIssueBlockKind::RecoveryRequired
        );

        let blocked = existing_run(&poller.ports().repository, &config, &snapshot).await;
        assert_eq!(blocked.status, GithubIssueWorkflowRunStatus::Blocked);
        assert_eq!(
            blocked
                .workflow_state
                .active_block
                .as_ref()
                .map(|block| block.kind.clone()),
            Some(GithubIssueBlockKind::RecoveryRequired)
        );
        // `block_workflow_run` clears the orphan pointer, so the run stops
        // re-claiming as a no-op.
        assert!(blocked.active_stage_run_id.is_none());
    }

    #[tokio::test]
    async fn poller_does_not_reconcile_fresh_orphan_active_stage_pointer() {
        // A run whose orphan pointer is younger than `stage_stale_after` (the
        // brief window between the run-pointer write and the stage-row write
        // during normal stage creation) must NOT be escalated.
        let config = workflow_config("reconcile-orphan-fresh", "nearai", "ironclaw");
        let ports = FakePollerPorts::new(vec![config.clone()]);
        let snapshot = issue_snapshot("nearai", "ironclaw", 42, 100);
        let run = create_run(&ports.repository, &config, &snapshot).await;
        let orphan_stage_run_id =
            GithubIssueStageRunId::from_trusted("fresh-orphan-stage-run-id".to_string()).unwrap();
        ports
            .repository
            .seed_orphan_active_stage_pointer(
                &run.workflow_run_id,
                orphan_stage_run_id,
                fixed_time(205),
            )
            .await
            .unwrap();
        let poller = GithubIssueWorkflowPoller::new(
            ports,
            GithubIssueWorkflowPollerConfig {
                enabled: true,
                stage_stale_after: std::time::Duration::from_secs(60),
                ..GithubIssueWorkflowPollerConfig::default()
            },
            "poller-contract-v1",
        );

        // Only 30s past `created_at` (fixed_time(200)) — under the 60s
        // threshold, so the fresh orphan must survive.
        poller.ports().clock.set(fixed_time(230));

        let outcome = poller.tick_once().await.unwrap();
        assert_eq!(outcome.stale_stages_failed, 0);
        assert!(outcome.blocked_runs.is_empty());

        let run = existing_run(&poller.ports().repository, &config, &snapshot).await;
        assert_ne!(run.status, GithubIssueWorkflowRunStatus::Blocked);
        assert!(run.active_stage_run_id.is_some());
        assert!(run.workflow_state.active_block.is_none());
    }

    #[tokio::test]
    async fn poller_records_pr_check_and_review_lifecycle_events_for_active_run() {
        let config = workflow_config("pr-refresh", "nearai", "ironclaw");
        let ports = FakePollerPorts::new(vec![config.clone()]);
        let snapshot = issue_snapshot("nearai", "ironclaw", 42, 100);
        ports.github.add_issue(snapshot.clone()).await;
        let poller = poller(ports);
        poller.tick_once().await.unwrap();

        let run = existing_run(&poller.ports().repository, &config, &snapshot).await;
        let pr_snapshot = pull_request_snapshot(300, false);
        let pull_request = pr_snapshot.pull_request.clone();
        set_run_primary_pr(&poller.ports().repository, run, pull_request.clone()).await;
        poller.ports().github.set_pull_request(pr_snapshot).await;
        poller
            .ports()
            .github
            .set_checks(&pull_request, vec![failed_check()])
            .await;
        poller
            .ports()
            .github
            .set_review_comments(&pull_request, vec![review_comment()])
            .await;

        let outcome = poller.tick_once().await.unwrap();

        assert_eq!(outcome.events_recorded, 3);
        assert_eq!(
            poller.ports().github.get_pull_request_calls().await.len(),
            1
        );
        assert_eq!(poller.ports().github.list_check_calls().await.len(), 1);
        assert_eq!(
            poller
                .ports()
                .github
                .list_review_comment_calls()
                .await
                .len(),
            1
        );
        let run = existing_run(&poller.ports().repository, &config, &snapshot).await;
        let events = poller
            .ports()
            .repository
            .list_workflow_events_after(
                ironclaw_github_issue_workflow::ListWorkflowEventsAfterInput {
                    workflow_run_id: run.workflow_run_id,
                    after_sequence: 0,
                    limit: 10,
                },
            )
            .await
            .unwrap();
        let event_types: Vec<_> = events
            .iter()
            .map(|event| event.workflow_event_type.clone())
            .collect();
        assert!(event_types.contains(&GithubIssueWorkflowEventType::GithubPullRequestUpdated));
        assert!(event_types.contains(&GithubIssueWorkflowEventType::GithubChecksFailed));
        assert!(event_types.contains(&GithubIssueWorkflowEventType::GithubReviewCommentCreated));
    }

    #[tokio::test]
    async fn poller_does_not_record_check_success_when_any_check_failed() {
        let config = workflow_config("mixed-checks", "nearai", "ironclaw");
        let ports = FakePollerPorts::new(vec![config.clone()]);
        let snapshot = issue_snapshot("nearai", "ironclaw", 42, 100);
        ports.github.add_issue(snapshot.clone()).await;
        let poller = poller(ports);
        poller.tick_once().await.unwrap();

        let run = existing_run(&poller.ports().repository, &config, &snapshot).await;
        let pr_snapshot = pull_request_snapshot(300, false);
        let pull_request = pr_snapshot.pull_request.clone();
        set_run_primary_pr(&poller.ports().repository, run, pull_request.clone()).await;
        poller.ports().github.set_pull_request(pr_snapshot).await;
        poller
            .ports()
            .github
            .set_checks(&pull_request, vec![failed_check(), successful_check()])
            .await;

        let outcome = poller.tick_once().await.unwrap();

        assert_eq!(outcome.events_recorded, 2);
        let run = existing_run(&poller.ports().repository, &config, &snapshot).await;
        let events = poller
            .ports()
            .repository
            .list_workflow_events_after(
                ironclaw_github_issue_workflow::ListWorkflowEventsAfterInput {
                    workflow_run_id: run.workflow_run_id,
                    after_sequence: 0,
                    limit: 10,
                },
            )
            .await
            .unwrap();
        let event_types: Vec<_> = events
            .iter()
            .map(|event| event.workflow_event_type.clone())
            .collect();
        assert!(event_types.contains(&GithubIssueWorkflowEventType::GithubChecksFailed));
        assert!(!event_types.contains(&GithubIssueWorkflowEventType::GithubChecksSucceeded));
    }

    #[tokio::test]
    async fn poller_records_closed_issue_for_active_run_without_open_search_hit() {
        let config = workflow_config("closed-refresh", "nearai", "ironclaw");
        let ports = FakePollerPorts::new(vec![config.clone()]);
        let open_snapshot = issue_snapshot("nearai", "ironclaw", 42, 100);
        ports.github.add_issue(open_snapshot.clone()).await;
        let poller = poller(ports);
        poller.tick_once().await.unwrap();

        let run = existing_run(&poller.ports().repository, &config, &open_snapshot).await;
        let pr_snapshot = pull_request_snapshot(300, false);
        let pull_request = pr_snapshot.pull_request.clone();
        set_run_primary_pr(&poller.ports().repository, run, pull_request.clone()).await;
        poller.ports().github.set_pull_request(pr_snapshot).await;
        poller
            .ports()
            .github
            .clear_search_results("nearai", "ironclaw")
            .await;
        let mut closed_snapshot = open_snapshot.clone();
        closed_snapshot.state = "closed".to_string();
        closed_snapshot.updated_at = Some(fixed_time(360));
        poller
            .ports()
            .github
            .set_issue_snapshot(closed_snapshot)
            .await;

        let outcome = poller.tick_once().await.unwrap();

        assert_eq!(outcome.issues_seen, 0);
        assert!(outcome.events_recorded >= 1);
        let run = existing_run(&poller.ports().repository, &config, &open_snapshot).await;
        let events = poller
            .ports()
            .repository
            .list_workflow_events_after(
                ironclaw_github_issue_workflow::ListWorkflowEventsAfterInput {
                    workflow_run_id: run.workflow_run_id,
                    after_sequence: 0,
                    limit: 10,
                },
            )
            .await
            .unwrap();
        assert!(events.iter().any(|event| {
            event.workflow_event_type == GithubIssueWorkflowEventType::GithubIssueClosed
        }));
    }

    #[tokio::test]
    async fn poller_records_pr_merge_before_closed_issue_when_both_refresh() {
        let config = workflow_config("merge-before-closed", "nearai", "ironclaw");
        let ports = FakePollerPorts::new(vec![config.clone()]);
        let open_snapshot = issue_snapshot("nearai", "ironclaw", 42, 100);
        ports.github.add_issue(open_snapshot.clone()).await;
        let poller = poller(ports);
        poller.tick_once().await.unwrap();

        let run = existing_run(&poller.ports().repository, &config, &open_snapshot).await;
        let pr_snapshot = pull_request_snapshot(370, true);
        let pull_request = pr_snapshot.pull_request.clone();
        set_run_primary_pr(&poller.ports().repository, run, pull_request).await;
        poller.ports().github.set_pull_request(pr_snapshot).await;
        poller
            .ports()
            .github
            .clear_search_results("nearai", "ironclaw")
            .await;
        let mut closed_snapshot = open_snapshot.clone();
        closed_snapshot.state = "closed".to_string();
        closed_snapshot.updated_at = Some(fixed_time(360));
        poller
            .ports()
            .github
            .set_issue_snapshot(closed_snapshot)
            .await;

        poller.tick_once().await.unwrap();

        let run = existing_run(&poller.ports().repository, &config, &open_snapshot).await;
        let events = poller
            .ports()
            .repository
            .list_workflow_events_after(
                ironclaw_github_issue_workflow::ListWorkflowEventsAfterInput {
                    workflow_run_id: run.workflow_run_id,
                    after_sequence: 1,
                    limit: 10,
                },
            )
            .await
            .unwrap();
        assert_eq!(
            events[0].workflow_event_type,
            GithubIssueWorkflowEventType::GithubPullRequestUpdated
        );
        assert_eq!(
            events[1].workflow_event_type,
            GithubIssueWorkflowEventType::GithubIssueClosed
        );
    }

    #[tokio::test]
    async fn poller_provider_rate_limit_blocks_config_not_process() {
        let limited = workflow_config("limited", "nearai", "limited");
        let healthy = workflow_config("healthy", "nearai", "ironclaw");
        let ports = FakePollerPorts::new(vec![limited, healthy.clone()]);
        ports.github.rate_limit_repo("nearai", "limited").await;
        let snapshot = issue_snapshot("nearai", "ironclaw", 42, 100);
        ports.github.add_issue(snapshot.clone()).await;
        let poller = poller(ports);

        let outcome = poller.tick_once().await.unwrap();

        assert_eq!(outcome.configs_loaded, 2);
        assert_eq!(outcome.blocked_configs.len(), 1);
        assert_eq!(outcome.events_recorded, 1);
        let search_calls = poller.ports().github.search_calls().await;
        assert_eq!(search_calls.len(), 2);
        let get_issue_calls = poller.ports().github.get_issue_calls().await;
        assert_eq!(get_issue_calls.len(), 1);
        assert_eq!(get_issue_calls[0].repo, "ironclaw");
        let run = existing_run(&poller.ports().repository, &healthy, &snapshot).await;
        assert_eq!(run.issue_ref.repo, "ironclaw");
    }

    /// Create (or fetch) the workflow run for an issue up front, so the test can
    /// seed events onto it before the poller claims it. Accepts either outcome.
    async fn create_run(
        repository: &InMemoryGithubIssueWorkflowRepository,
        config: &GithubIssueWorkflowConfig,
        snapshot: &GithubIssueProviderSnapshot,
    ) -> GithubIssueWorkflowRun {
        match repository
            .create_or_get_workflow_run(CreateOrGetWorkflowRunInput {
                tenant_id: config.tenant_id.clone(),
                creator_user_id: config.owner_user_id.clone(),
                agent_id: None,
                project_id: Some(config.project_id.clone()),
                provider_account_ref: Some(config.provider_account_ref.clone()),
                issue_ref: snapshot.issue_ref(),
                workflow_policy_key: "github-bug-workflow".to_string(),
                workflow_policy_version: "poller-contract-v1".to_string(),
                now: fixed_time(200),
            })
            .await
            .unwrap()
        {
            CreateOrGetWorkflowRunOutcome::Existing { run }
            | CreateOrGetWorkflowRunOutcome::Created { run } => run,
        }
    }

    /// Record a `GithubIssueChanged` event (cursor-advancing, no stage submit)
    /// directly into the repository so a single claimed tick can drain it.
    async fn record_issue_changed_event(
        repository: &InMemoryGithubIssueWorkflowRepository,
        run: &GithubIssueWorkflowRun,
        updated_at: i64,
    ) {
        let provider =
            ironclaw_github_issue_workflow::issue_binding_ref(&run.issue_ref).provider_ref;
        repository
            .record_workflow_event(ironclaw_github_issue_workflow::RecordWorkflowEventInput {
                workflow_run_id: run.workflow_run_id.clone(),
                workflow_event_type: GithubIssueWorkflowEventType::GithubIssueChanged,
                envelope: ironclaw_github_issue_workflow::WorkflowEventEnvelope {
                    source_kind: ironclaw_github_issue_workflow::WorkflowEventSourceKind::Poller,
                    source_delivery_id: None,
                    provider,
                    observed_at: fixed_time(updated_at),
                    provider_updated_at: Some(fixed_time(updated_at)),
                    idempotency_key: ironclaw_github_issue_workflow::issue_changed_key(
                        &run.issue_ref,
                        Some(fixed_time(updated_at)),
                    ),
                    payload_schema: "github.issue.changed.v1".to_string(),
                    payload: serde_json::json!({ "issue": run.issue_ref }),
                },
            })
            .await
            .expect("record issue changed event");
    }

    #[tokio::test]
    async fn claimed_tick_drains_multiple_pending_events_in_one_tick() {
        // A1: a single `tick_once` must drain every already-recorded queued
        // event for a claimed run, not just one. We pre-seed three
        // cursor-advancing `GithubIssueChanged` events; after one tick the run
        // cursor must have advanced past all three (three policy transitions in
        // one claimed tick), rather than one-per-tick.
        let config = workflow_config("drain", "nearai", "ironclaw");
        let ports = FakePollerPorts::new(vec![config.clone()]);
        // No search hits added: discovery records nothing, so the only events
        // are the three we seed below.
        let snapshot = issue_snapshot("nearai", "ironclaw", 42, 100);
        let run = create_run(&ports.repository, &config, &snapshot).await;
        record_issue_changed_event(&ports.repository, &run, 101).await;
        record_issue_changed_event(&ports.repository, &run, 102).await;
        record_issue_changed_event(&ports.repository, &run, 103).await;
        let poller = poller(ports);

        let outcome = poller.tick_once().await.unwrap();

        // All three transitions happened in this ONE claimed tick (the drain
        // re-ticks until a tick makes no progress, so policy_ticks is the three
        // event transitions plus the final no-op tick that detects the drain is
        // done). Without draining this would be a single transition.
        assert_eq!(outcome.runnable_runs_claimed, 1);
        assert!(
            outcome.policy_ticks >= 3,
            "all queued events should drain in one claimed tick, got {} policy ticks",
            outcome.policy_ticks
        );
        assert_eq!(outcome.leases_released, 1);

        // The run cursor advanced past every seeded event.
        let drained_run = existing_run(&poller.ports().repository, &config, &snapshot).await;
        let events = poller
            .ports()
            .repository
            .list_workflow_events_after(
                ironclaw_github_issue_workflow::ListWorkflowEventsAfterInput {
                    workflow_run_id: drained_run.workflow_run_id.clone(),
                    after_sequence: 0,
                    limit: 10,
                },
            )
            .await
            .unwrap();
        assert_eq!(events.len(), 3);
        assert_eq!(drained_run.event_cursor, events.last().unwrap().sequence);
    }
}
