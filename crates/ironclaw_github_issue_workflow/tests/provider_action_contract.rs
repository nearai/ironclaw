mod provider_action_contract {
    use std::collections::VecDeque;
    use std::sync::Arc;

    use async_trait::async_trait;
    use chrono::{TimeZone, Utc};
    use ironclaw_github_issue_workflow::{
        CreateIssueCommentInput, CreateOrGetWorkflowRunInput, CreateOrGetWorkflowRunOutcome,
        GetAuthenticatedWorkflowActorInput, GithubActorSnapshot, GithubCommentRef,
        GithubIssueCommentSnapshot, GithubIssueProviderActionRunner, GithubIssueRef,
        GithubIssueWorkflowError, GithubIssueWorkflowPort, GithubIssueWorkflowRepository,
        GithubIssueWorkflowRun, InMemoryGithubIssueWorkflowRepository, ListIssueCommentsInput,
        ProviderActionKind, ProviderActionReconciliationStrategy, ProviderActionRunOutcome,
        ProviderActionStatus, RunClaimCommentProviderActionRequest, WorkflowWorkerId,
        stable_claim_marker,
    };
    use ironclaw_host_api::{AgentId, ProjectId, TenantId, UserId};
    use tokio::sync::{Mutex, Notify};

    fn fixed_time(seconds: i64) -> chrono::DateTime<Utc> {
        Utc.timestamp_opt(seconds, 0).unwrap()
    }

    fn tenant() -> TenantId {
        TenantId::new("tenant-provider-action-contract").unwrap()
    }

    fn user() -> UserId {
        UserId::new("user-provider-action-contract").unwrap()
    }

    fn agent() -> AgentId {
        AgentId::new("agent-provider-action-contract").unwrap()
    }

    fn project() -> ProjectId {
        ProjectId::new("project-provider-action-contract").unwrap()
    }

    fn worker(suffix: u64) -> WorkflowWorkerId {
        WorkflowWorkerId::from_trusted(format!("provider-action-worker-{suffix}")).unwrap()
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

    async fn create_run(
        repository: &InMemoryGithubIssueWorkflowRepository,
        issue_ref: GithubIssueRef,
    ) -> GithubIssueWorkflowRun {
        match repository
            .create_or_get_workflow_run(CreateOrGetWorkflowRunInput {
                tenant_id: tenant(),
                creator_user_id: user(),
                agent_id: Some(agent()),
                project_id: Some(project()),
                issue_ref,
                workflow_policy_key: "github-bug-workflow".to_string(),
                workflow_policy_version: "2026-06-22".to_string(),
                now: fixed_time(10),
            })
            .await
            .unwrap()
        {
            CreateOrGetWorkflowRunOutcome::Created { run }
            | CreateOrGetWorkflowRunOutcome::Existing { run } => run,
        }
    }

    fn comment_snapshot(
        marker: &str,
        author_login: &str,
        node_id: &str,
    ) -> GithubIssueCommentSnapshot {
        GithubIssueCommentSnapshot {
            comment: GithubCommentRef {
                node_id: Some(node_id.to_string()),
                url: format!("https://github.com/nearai/ironclaw/issues/42#issuecomment-{node_id}"),
            },
            body: format!("{marker}\nIronClaw is attempting this bug fix."),
            author_login: author_login.to_string(),
            created_at: fixed_time(12),
            updated_at: fixed_time(12),
        }
    }

    #[derive(Debug)]
    struct FakeGithubIssueWorkflowPort {
        actor: GithubActorSnapshot,
        comments: Mutex<Vec<GithubIssueCommentSnapshot>>,
        create_results: Mutex<VecDeque<Result<GithubCommentRef, GithubIssueWorkflowError>>>,
        create_bodies: Mutex<Vec<String>>,
        list_calls: Mutex<usize>,
        create_call_observed: Notify,
        pause_first_create_until: Mutex<Option<Arc<Notify>>>,
    }

    impl FakeGithubIssueWorkflowPort {
        fn new() -> Self {
            Self {
                actor: GithubActorSnapshot {
                    login: "ironclaw-bot".to_string(),
                    node_id: Some("actor-node-1".to_string()),
                },
                comments: Mutex::new(Vec::new()),
                create_results: Mutex::new(VecDeque::from([Ok(GithubCommentRef {
                    node_id: Some("created-comment-node-1".to_string()),
                    url: "https://github.com/nearai/ironclaw/issues/42#issuecomment-created"
                        .to_string(),
                })])),
                create_bodies: Mutex::new(Vec::new()),
                list_calls: Mutex::new(0),
                create_call_observed: Notify::new(),
                pause_first_create_until: Mutex::new(None),
            }
        }

        async fn set_comments(&self, comments: Vec<GithubIssueCommentSnapshot>) {
            *self.comments.lock().await = comments;
        }

        async fn set_create_results(
            &self,
            results: VecDeque<Result<GithubCommentRef, GithubIssueWorkflowError>>,
        ) {
            *self.create_results.lock().await = results;
        }

        async fn create_bodies(&self) -> Vec<String> {
            self.create_bodies.lock().await.clone()
        }

        async fn list_call_count(&self) -> usize {
            *self.list_calls.lock().await
        }

        async fn pause_first_create_until(&self, release: Arc<Notify>) {
            *self.pause_first_create_until.lock().await = Some(release);
        }

        async fn wait_for_create_call_count(&self, expected_count: usize) {
            loop {
                if self.create_bodies.lock().await.len() >= expected_count {
                    return;
                }
                self.create_call_observed.notified().await;
            }
        }
    }

    #[async_trait]
    impl GithubIssueWorkflowPort for FakeGithubIssueWorkflowPort {
        async fn get_authenticated_workflow_actor(
            &self,
            _input: GetAuthenticatedWorkflowActorInput,
        ) -> Result<GithubActorSnapshot, GithubIssueWorkflowError> {
            Ok(self.actor.clone())
        }

        async fn list_issue_comments(
            &self,
            _input: ListIssueCommentsInput,
        ) -> Result<Vec<GithubIssueCommentSnapshot>, GithubIssueWorkflowError> {
            *self.list_calls.lock().await += 1;
            Ok(self.comments.lock().await.clone())
        }

        async fn create_issue_comment(
            &self,
            input: CreateIssueCommentInput,
        ) -> Result<GithubCommentRef, GithubIssueWorkflowError> {
            let create_call_count = {
                let mut create_bodies = self.create_bodies.lock().await;
                create_bodies.push(input.body);
                create_bodies.len()
            };
            self.create_call_observed.notify_waiters();
            let release = if create_call_count == 1 {
                self.pause_first_create_until.lock().await.clone()
            } else {
                None
            };
            if let Some(release) = release {
                release.notified().await;
            }
            self.create_results
                .lock()
                .await
                .pop_front()
                .unwrap_or_else(|| {
                    Ok(GithubCommentRef {
                        node_id: Some("created-comment-node-fallback".to_string()),
                        url: "https://github.com/nearai/ironclaw/issues/42#issuecomment-fallback"
                            .to_string(),
                    })
                })
        }
    }

    fn request(
        run: GithubIssueWorkflowRun,
        worker_id: WorkflowWorkerId,
    ) -> RunClaimCommentProviderActionRequest {
        RunClaimCommentProviderActionRequest {
            run,
            worker_id,
            now: fixed_time(20),
            lease_expires_at: fixed_time(80),
        }
    }

    #[tokio::test]
    async fn claim_comment_uses_stable_marker_and_records_binding() {
        let repository = Arc::new(InMemoryGithubIssueWorkflowRepository::default());
        let port = Arc::new(FakeGithubIssueWorkflowPort::new());
        let run = create_run(&repository, issue()).await;
        let marker = stable_claim_marker(&run.workflow_run_id);
        let runner = GithubIssueProviderActionRunner::new(repository, port.clone());

        let outcome = runner
            .run_claim_comment(request(run.clone(), worker(1)))
            .await
            .unwrap();

        let ProviderActionRunOutcome::Succeeded { action, binding } = outcome else {
            panic!("claim comment action must succeed");
        };
        let bodies = port.create_bodies().await;
        assert_eq!(bodies.len(), 1);
        assert!(bodies[0].contains(&marker));
        assert_eq!(action.status, ProviderActionStatus::Succeeded);
        assert_eq!(action.kind, ProviderActionKind::ClaimComment);
        assert_eq!(
            action.reconciliation_strategy,
            ProviderActionReconciliationStrategy::ClaimCommentByMarker
        );
        assert_eq!(action.stable_marker.as_deref(), Some(marker.as_str()));
        assert_eq!(action.attempt_count, 1);
        assert_eq!(binding.workflow_run_id, run.workflow_run_id);
        assert_eq!(binding.role, "claim");
        assert_eq!(binding.resource_type, "issue_comment");
        assert_eq!(binding.provider_id, marker);
        assert_eq!(
            binding.created_by_provider_action_id,
            Some(action.provider_action_id)
        );
    }

    #[tokio::test]
    async fn duplicate_claim_comment_replays_existing_action() {
        let repository = Arc::new(InMemoryGithubIssueWorkflowRepository::default());
        let port = Arc::new(FakeGithubIssueWorkflowPort::new());
        let run = create_run(&repository, issue()).await;
        let runner = GithubIssueProviderActionRunner::new(repository, port.clone());

        let first = runner
            .run_claim_comment(request(run.clone(), worker(1)))
            .await
            .unwrap();
        let second = runner
            .run_claim_comment(request(run, worker(2)))
            .await
            .unwrap();

        let ProviderActionRunOutcome::Succeeded {
            action: first_action,
            ..
        } = first
        else {
            panic!("first claim comment action must succeed");
        };
        let ProviderActionRunOutcome::Replayed {
            action: second_action,
        } = second
        else {
            panic!("second claim comment action must replay existing action");
        };
        assert_eq!(
            first_action.provider_action_id,
            second_action.provider_action_id
        );
        assert_eq!(second_action.status, ProviderActionStatus::Succeeded);
        assert_eq!(port.create_bodies().await.len(), 1);
        assert_eq!(port.list_call_count().await, 1);
    }

    #[tokio::test]
    async fn same_worker_running_claim_comment_is_busy_until_lease_expiry() {
        let repository = Arc::new(InMemoryGithubIssueWorkflowRepository::default());
        let port = Arc::new(FakeGithubIssueWorkflowPort::new());
        let release_first_create = Arc::new(Notify::new());
        port.pause_first_create_until(release_first_create.clone())
            .await;
        let run = create_run(&repository, issue()).await;
        let runner = Arc::new(GithubIssueProviderActionRunner::new(
            repository,
            port.clone(),
        ));
        let first_runner = runner.clone();
        let first_run = run.clone();
        let first_worker = worker(1);
        let first_handle = tokio::spawn(async move {
            first_runner
                .run_claim_comment(request(first_run, first_worker))
                .await
        });
        port.wait_for_create_call_count(1).await;

        let second = runner
            .run_claim_comment(request(run, worker(1)))
            .await
            .unwrap();

        let ProviderActionRunOutcome::Busy {
            action: second_action,
        } = second
        else {
            panic!("same-worker invocation must not reclaim an unexpired running provider action");
        };
        assert_eq!(second_action.status, ProviderActionStatus::Running);
        assert_eq!(second_action.attempt_count, 1);
        assert_eq!(port.create_bodies().await.len(), 1);
        assert_eq!(port.list_call_count().await, 1);

        release_first_create.notify_waiters();
        let first = first_handle.await.unwrap().unwrap();
        let ProviderActionRunOutcome::Succeeded { action, .. } = first else {
            panic!("first claim comment action must still complete after release");
        };
        assert_eq!(action.status, ProviderActionStatus::Succeeded);
        assert_eq!(port.create_bodies().await.len(), 1);
    }

    #[tokio::test]
    async fn ambiguous_claim_comment_enters_needs_reconciliation() {
        let repository = Arc::new(InMemoryGithubIssueWorkflowRepository::default());
        let port = Arc::new(FakeGithubIssueWorkflowPort::new());
        let run = create_run(&repository, issue()).await;
        let marker = stable_claim_marker(&run.workflow_run_id);
        port.set_comments(vec![
            comment_snapshot(&marker, "ironclaw-bot", "claim-1"),
            comment_snapshot(&marker, "ironclaw-bot", "claim-2"),
        ])
        .await;
        let runner = GithubIssueProviderActionRunner::new(repository, port.clone());

        let outcome = runner
            .run_claim_comment(request(run, worker(1)))
            .await
            .unwrap();

        let ProviderActionRunOutcome::NeedsReconciliation { action } = outcome else {
            panic!("ambiguous existing claim comments must need reconciliation");
        };
        assert_eq!(action.status, ProviderActionStatus::NeedsReconciliation);
        assert_eq!(
            action.redacted_failure_kind.as_deref(),
            Some("ambiguous_claim_comment")
        );
        assert!(port.create_bodies().await.is_empty());
    }

    #[tokio::test]
    async fn self_authored_comment_with_known_marker_is_echo_suppressed() {
        let repository = Arc::new(InMemoryGithubIssueWorkflowRepository::default());
        let port = Arc::new(FakeGithubIssueWorkflowPort::new());
        let run = create_run(&repository, issue()).await;
        let marker = stable_claim_marker(&run.workflow_run_id);
        port.set_comments(vec![comment_snapshot(&marker, "ironclaw-bot", "claim-1")])
            .await;
        let runner = GithubIssueProviderActionRunner::new(repository, port.clone());

        let outcome = runner
            .run_claim_comment(request(run, worker(1)))
            .await
            .unwrap();

        let ProviderActionRunOutcome::Succeeded { action, binding } = outcome else {
            panic!("self-authored known marker must be treated as our existing write");
        };
        assert_eq!(action.status, ProviderActionStatus::Succeeded);
        assert_eq!(binding.provider_id, marker);
        assert!(
            binding
                .provider_url
                .unwrap()
                .contains("issuecomment-claim-1")
        );
        assert!(port.create_bodies().await.is_empty());
    }

    #[tokio::test]
    async fn provider_write_failure_is_sanitized() {
        let repository = Arc::new(InMemoryGithubIssueWorkflowRepository::default());
        let port = Arc::new(FakeGithubIssueWorkflowPort::new());
        port.set_create_results(VecDeque::from([Err(
            GithubIssueWorkflowError::Repository {
                reason: "GitHub token ghp_secret123 cannot create comment".to_string(),
            },
        )]))
        .await;
        let run = create_run(&repository, issue()).await;
        let runner = GithubIssueProviderActionRunner::new(repository, port);

        let outcome = runner
            .run_claim_comment(request(run, worker(1)))
            .await
            .unwrap();

        let ProviderActionRunOutcome::Failed { action } = outcome else {
            panic!("provider write failure must be recorded as a sanitized failure");
        };
        assert_eq!(action.status, ProviderActionStatus::Failed);
        assert_eq!(
            action.redacted_failure_kind.as_deref(),
            Some("provider_write_failed")
        );
        let serialized = serde_json::to_string(&action).unwrap();
        assert!(!serialized.contains("ghp_secret123"));
        assert!(!serialized.contains("cannot create comment"));
    }
}
