mod repository_contract {
    use chrono::{Duration, TimeZone, Utc};
    use ironclaw_github_issue_workflow::{
        AdvanceWorkflowRunInput, ClaimRunnableWorkflowRunsInput, CreateOrGetProviderActionInput,
        CreateOrGetWorkflowRunInput, CreateOrGetWorkflowRunOutcome, CreateStageRunInput,
        CreateStageRunOutcome, GithubIssueRef, GithubIssueStage, GithubIssueStageRunId,
        GithubIssueWorkflowEventType, GithubIssueWorkflowMode, GithubIssueWorkflowRepository,
        GithubIssueWorkflowRun, GithubProviderRef, GithubPullRequestRef,
        InMemoryGithubIssueWorkflowRepository, RecordWorkflowEventInput,
        RecordWorkflowEventOutcome, TransitionOutcome, UpsertProviderBindingInput,
        WorkflowEventEnvelope, WorkflowEventSourceKind, WorkflowIdempotencyKey,
        WorkflowRunTransition, WorkflowWorkerId, checks_changed_key, issue_changed_key,
        issue_discovered_key, pr_opened_key, review_comment_created_key, stage_result_reported_key,
    };
    use ironclaw_host_api::{AgentId, ProjectId, TenantId, UserId};
    use serde_json::json;

    fn fixed_time(seconds: i64) -> chrono::DateTime<Utc> {
        Utc.timestamp_opt(seconds, 0).unwrap()
    }

    fn tenant(suffix: u64) -> TenantId {
        TenantId::new(format!("tenant-{suffix}")).unwrap()
    }

    fn user(suffix: u64) -> UserId {
        UserId::new(format!("user-{suffix}")).unwrap()
    }

    fn agent(suffix: u64) -> AgentId {
        AgentId::new(format!("agent-{suffix}")).unwrap()
    }

    fn project(suffix: u64) -> ProjectId {
        ProjectId::new(format!("project-{suffix}")).unwrap()
    }

    fn issue(number: u64) -> GithubIssueRef {
        GithubIssueRef {
            owner: "nearai".to_string(),
            repo: "ironclaw".to_string(),
            number,
            node_id: Some(format!("issue-node-{number}")),
            url: format!("https://github.com/nearai/ironclaw/issues/{number}"),
            default_branch: "main".to_string(),
        }
    }

    fn provider_ref(provider_id: impl Into<String>) -> GithubProviderRef {
        GithubProviderRef {
            system: "github".to_string(),
            resource_type: "issue".to_string(),
            owner: "nearai".to_string(),
            repo: "ironclaw".to_string(),
            provider_id: provider_id.into(),
            provider_url: Some("https://github.com/nearai/ironclaw/issues/42".to_string()),
        }
    }

    fn workflow_run_input(
        tenant_id: TenantId,
        issue_ref: GithubIssueRef,
        now: chrono::DateTime<Utc>,
    ) -> CreateOrGetWorkflowRunInput {
        CreateOrGetWorkflowRunInput {
            tenant_id,
            creator_user_id: user(1),
            agent_id: Some(agent(1)),
            project_id: Some(project(1)),
            issue_ref,
            workflow_policy_key: "github-bug-workflow".to_string(),
            workflow_policy_version: "2026-06-22".to_string(),
            now,
        }
    }

    async fn create_run(
        repository: &InMemoryGithubIssueWorkflowRepository,
        tenant_id: TenantId,
        issue_ref: GithubIssueRef,
    ) -> GithubIssueWorkflowRun {
        match repository
            .create_or_get_workflow_run(workflow_run_input(tenant_id, issue_ref, fixed_time(10)))
            .await
            .unwrap()
        {
            CreateOrGetWorkflowRunOutcome::Created { run }
            | CreateOrGetWorkflowRunOutcome::Existing { run } => run,
        }
    }

    fn event_input(
        run: &GithubIssueWorkflowRun,
        key: &str,
        observed_at: chrono::DateTime<Utc>,
    ) -> RecordWorkflowEventInput {
        RecordWorkflowEventInput {
            workflow_run_id: run.workflow_run_id.clone(),
            workflow_event_type: GithubIssueWorkflowEventType::GithubIssueChanged,
            envelope: WorkflowEventEnvelope {
                source_kind: WorkflowEventSourceKind::Poller,
                source_delivery_id: None,
                provider: provider_ref("issue-node-42"),
                observed_at,
                provider_updated_at: Some(observed_at),
                idempotency_key: WorkflowIdempotencyKey::from_trusted(key.to_string()).unwrap(),
                payload_schema: "github.issue.changed.v1".to_string(),
                payload: json!({ "issue_number": run.issue_ref.number, "key": key }),
            },
        }
    }

    fn worker(suffix: u64) -> WorkflowWorkerId {
        WorkflowWorkerId::from_trusted(format!("worker-{suffix}")).unwrap()
    }

    #[test]
    fn deterministic_idempotency_key_builders_use_provider_identity() {
        let issue_ref = issue(42);
        let pr_ref = GithubPullRequestRef {
            owner: "nearai".to_string(),
            repo: "ironclaw".to_string(),
            number: 7,
            node_id: Some("pr-node-7".to_string()),
            url: "https://github.com/nearai/ironclaw/pull/7".to_string(),
            head_branch: "bugfix".to_string(),
            head_sha: Some("abc123".to_string()),
        };
        let stage_run_id = GithubIssueStageRunId::from_trusted("stage-run-1".to_string()).unwrap();

        assert_eq!(
            issue_discovered_key(&issue_ref).as_str(),
            "issue:issue-node-42:discovered"
        );
        assert_eq!(
            issue_changed_key(&issue_ref, Some(fixed_time(10))).as_str(),
            "issue:issue-node-42:updated:1970-01-01T00:00:10.000000000Z"
        );
        assert_eq!(pr_opened_key(&pr_ref).as_str(), "pr:pr-node-7:opened");
        assert_eq!(
            checks_changed_key("abc123", "suite-1", "failure").as_str(),
            "checks:abc123:suite-1:failure"
        );
        assert_eq!(
            review_comment_created_key("review-comment-node-1").as_str(),
            "review-comment:review-comment-node-1"
        );
        assert_eq!(
            stage_result_reported_key(&stage_run_id, "triage.v1").as_str(),
            "stage-result:stage-run-1:triage.v1"
        );
    }

    #[tokio::test]
    async fn create_or_get_workflow_run_is_idempotent_per_tenant() {
        let repository = InMemoryGithubIssueWorkflowRepository::default();
        let first_tenant = tenant(1);
        let issue_ref = issue(42);

        let first = repository
            .create_or_get_workflow_run(workflow_run_input(
                first_tenant.clone(),
                issue_ref.clone(),
                fixed_time(10),
            ))
            .await
            .unwrap();
        let second = repository
            .create_or_get_workflow_run(workflow_run_input(
                first_tenant,
                issue_ref.clone(),
                fixed_time(20),
            ))
            .await
            .unwrap();
        let other_tenant = repository
            .create_or_get_workflow_run(workflow_run_input(tenant(2), issue_ref, fixed_time(30)))
            .await
            .unwrap();

        let CreateOrGetWorkflowRunOutcome::Created { run: first_run } = first else {
            panic!("first call must create the run");
        };
        let CreateOrGetWorkflowRunOutcome::Existing { run: second_run } = second else {
            panic!("second call must reuse the run");
        };
        let CreateOrGetWorkflowRunOutcome::Created {
            run: other_tenant_run,
        } = other_tenant
        else {
            panic!("same key in a different tenant must create a separate run");
        };

        assert_eq!(first_run.workflow_run_id, second_run.workflow_run_id);
        assert_ne!(first_run.workflow_run_id, other_tenant_run.workflow_run_id);
    }

    #[tokio::test]
    async fn record_workflow_event_dedupes_by_run_and_key() {
        let repository = InMemoryGithubIssueWorkflowRepository::default();
        let run = create_run(&repository, tenant(1), issue(42)).await;

        let first = repository
            .record_workflow_event(event_input(&run, "issue:42:changed:1", fixed_time(10)))
            .await
            .unwrap();
        let second = repository
            .record_workflow_event(event_input(&run, "issue:42:changed:1", fixed_time(20)))
            .await
            .unwrap();

        let RecordWorkflowEventOutcome::Recorded { event: first_event } = first else {
            panic!("first event must be recorded");
        };
        let RecordWorkflowEventOutcome::Duplicate { existing } = second else {
            panic!("duplicate event key must replay the existing event");
        };

        assert_eq!(first_event.workflow_event_id, existing.workflow_event_id);
        assert_eq!(first_event.sequence, existing.sequence);
    }

    #[tokio::test]
    async fn record_workflow_event_sequences_are_monotonic() {
        let repository = InMemoryGithubIssueWorkflowRepository::default();
        let run = create_run(&repository, tenant(1), issue(42)).await;

        let first = repository
            .record_workflow_event(event_input(&run, "issue:42:changed:1", fixed_time(10)))
            .await
            .unwrap();
        let second = repository
            .record_workflow_event(event_input(&run, "issue:42:changed:2", fixed_time(20)))
            .await
            .unwrap();

        let RecordWorkflowEventOutcome::Recorded { event: first_event } = first else {
            panic!("first event must be recorded");
        };
        let RecordWorkflowEventOutcome::Recorded {
            event: second_event,
        } = second
        else {
            panic!("second event must be recorded");
        };

        assert_eq!(first_event.sequence, 1);
        assert_eq!(second_event.sequence, 2);
    }

    #[tokio::test]
    async fn claim_runnable_workflow_runs_honors_lease_expiry() {
        let repository = InMemoryGithubIssueWorkflowRepository::default();
        let first_run = create_run(&repository, tenant(1), issue(42)).await;
        let second_run = create_run(&repository, tenant(1), issue(43)).await;
        let lease_until = fixed_time(10) + Duration::seconds(60);

        let first_claim = repository
            .claim_runnable_workflow_runs(ClaimRunnableWorkflowRunsInput {
                tenant_id: tenant(1),
                worker_id: worker(1),
                now: fixed_time(10),
                lease_expires_at: lease_until,
                limit: 10,
            })
            .await
            .unwrap();
        let blocked_claim = repository
            .claim_runnable_workflow_runs(ClaimRunnableWorkflowRunsInput {
                tenant_id: tenant(1),
                worker_id: worker(2),
                now: fixed_time(20),
                lease_expires_at: fixed_time(20) + Duration::seconds(60),
                limit: 10,
            })
            .await
            .unwrap();
        let expired_claim = repository
            .claim_runnable_workflow_runs(ClaimRunnableWorkflowRunsInput {
                tenant_id: tenant(1),
                worker_id: worker(2),
                now: fixed_time(71),
                lease_expires_at: fixed_time(71) + Duration::seconds(60),
                limit: 10,
            })
            .await
            .unwrap();

        let first_claimed_ids: Vec<_> = first_claim
            .iter()
            .map(|run| run.workflow_run_id.clone())
            .collect();
        let expired_claimed_ids: Vec<_> = expired_claim
            .iter()
            .map(|run| run.workflow_run_id.clone())
            .collect();

        assert_eq!(
            first_claimed_ids,
            vec![first_run.workflow_run_id, second_run.workflow_run_id]
        );
        assert!(blocked_claim.is_empty());
        assert_eq!(expired_claimed_ids.len(), 2);
        assert!(
            expired_claim
                .iter()
                .all(|run| run.lease_owner == Some(worker(2)))
        );
    }

    #[tokio::test]
    async fn advance_event_cursor_requires_expected_version() {
        let repository = InMemoryGithubIssueWorkflowRepository::default();
        let run = create_run(&repository, tenant(1), issue(42)).await;
        let claimed = repository
            .claim_runnable_workflow_runs(ClaimRunnableWorkflowRunsInput {
                tenant_id: tenant(1),
                worker_id: worker(1),
                now: fixed_time(10),
                lease_expires_at: fixed_time(70),
                limit: 1,
            })
            .await
            .unwrap()
            .pop()
            .unwrap();
        let event = match repository
            .record_workflow_event(event_input(&run, "issue:42:changed:1", fixed_time(20)))
            .await
            .unwrap()
        {
            RecordWorkflowEventOutcome::Recorded { event } => event,
            _ => panic!("event must be recorded"),
        };

        let stale = repository
            .advance_event_cursor_and_transition(AdvanceWorkflowRunInput {
                workflow_run_id: run.workflow_run_id.clone(),
                worker_id: worker(1),
                expected_workflow_run_version: claimed.workflow_run_version + 1,
                expected_event_cursor: 0,
                next_event_cursor: event.sequence,
                transition: WorkflowRunTransition::default(),
                now: fixed_time(30),
            })
            .await
            .unwrap();
        let applied = repository
            .advance_event_cursor_and_transition(AdvanceWorkflowRunInput {
                workflow_run_id: run.workflow_run_id,
                worker_id: worker(1),
                expected_workflow_run_version: claimed.workflow_run_version,
                expected_event_cursor: 0,
                next_event_cursor: event.sequence,
                transition: WorkflowRunTransition {
                    mode: Some(GithubIssueWorkflowMode::Planning),
                    ..WorkflowRunTransition::default()
                },
                now: fixed_time(31),
            })
            .await
            .unwrap();

        let TransitionOutcome::VersionConflict { current } = stale else {
            panic!("stale expected version must be rejected");
        };
        let TransitionOutcome::Applied { run: advanced } = applied else {
            panic!("matching version and cursor must apply");
        };

        assert_eq!(current.workflow_run_version, claimed.workflow_run_version);
        assert_eq!(advanced.event_cursor, event.sequence);
        assert_eq!(
            advanced.workflow_run_version,
            claimed.workflow_run_version + 1
        );
        assert_eq!(
            advanced.workflow_state.mode,
            GithubIssueWorkflowMode::Planning
        );
    }

    #[tokio::test]
    async fn create_stage_run_rejects_second_active_stage() {
        let repository = InMemoryGithubIssueWorkflowRepository::default();
        let run = create_run(&repository, tenant(1), issue(42)).await;

        let first = repository
            .create_stage_run(CreateStageRunInput {
                workflow_run_id: run.workflow_run_id.clone(),
                stage: GithubIssueStage::Triage,
                now: fixed_time(10),
            })
            .await
            .unwrap();
        let second = repository
            .create_stage_run(CreateStageRunInput {
                workflow_run_id: run.workflow_run_id,
                stage: GithubIssueStage::Planning,
                now: fixed_time(20),
            })
            .await
            .unwrap();

        let CreateStageRunOutcome::Created { stage_run_id, run } = first else {
            panic!("first active stage must be created");
        };
        let CreateStageRunOutcome::ActiveStageExists {
            existing_stage_run_id,
            run: blocked_run,
        } = second
        else {
            panic!("second active stage must be rejected");
        };

        assert_eq!(run.active_stage_run_id, Some(stage_run_id.clone()));
        assert_eq!(existing_stage_run_id, stage_run_id);
        assert_eq!(blocked_run.active_stage_run_id, Some(existing_stage_run_id));
    }

    #[tokio::test]
    async fn create_or_get_provider_action_dedupes_by_input_hash() {
        let repository = InMemoryGithubIssueWorkflowRepository::default();
        let run = create_run(&repository, tenant(1), issue(42)).await;
        let input_hash = "sha256:claim-comment-input";
        let idempotency_key =
            WorkflowIdempotencyKey::from_trusted(format!("provider-action:{input_hash}")).unwrap();

        let first = repository
            .create_or_get_provider_action(CreateOrGetProviderActionInput {
                workflow_run_id: run.workflow_run_id.clone(),
                stage_run_id: None,
                step_run_id: None,
                name: "claim-comment".to_string(),
                idempotency_key: idempotency_key.clone(),
                input_hash: input_hash.to_string(),
                now: fixed_time(10),
            })
            .await
            .unwrap();
        let second = repository
            .create_or_get_provider_action(CreateOrGetProviderActionInput {
                workflow_run_id: run.workflow_run_id,
                stage_run_id: None,
                step_run_id: None,
                name: "claim-comment".to_string(),
                idempotency_key,
                input_hash: input_hash.to_string(),
                now: fixed_time(20),
            })
            .await
            .unwrap();

        assert_eq!(first.provider_action_id, second.provider_action_id);
        assert_eq!(first.input_hash, input_hash);
        assert_eq!(second.created_at, fixed_time(10));
    }

    #[tokio::test]
    async fn upsert_provider_binding_routes_by_provider_ref() {
        let repository = InMemoryGithubIssueWorkflowRepository::default();
        let first_run = create_run(&repository, tenant(1), issue(42)).await;
        let second_run = create_run(&repository, tenant(1), issue(43)).await;
        let provider = provider_ref("issue-node-42");

        let first = repository
            .upsert_provider_binding(UpsertProviderBindingInput {
                workflow_run_id: first_run.workflow_run_id.clone(),
                provider_ref: provider.clone(),
                role: "primary".to_string(),
                created_by_provider_action_id: None,
                created_at: fixed_time(10),
            })
            .await
            .unwrap();
        let routed = repository
            .upsert_provider_binding(UpsertProviderBindingInput {
                workflow_run_id: second_run.workflow_run_id,
                provider_ref: provider,
                role: "primary".to_string(),
                created_by_provider_action_id: None,
                created_at: fixed_time(20),
            })
            .await
            .unwrap();

        assert_eq!(first.binding_id, routed.binding_id);
        assert_eq!(routed.workflow_run_id, first_run.workflow_run_id);
        assert_eq!(routed.created_at, fixed_time(10));
    }
}
