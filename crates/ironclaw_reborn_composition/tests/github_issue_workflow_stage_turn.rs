#![cfg(feature = "github-issue-workflow-beta")]

mod github_issue_workflow_stage_turn {
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use ironclaw_approvals::{
        InMemoryPersistentApprovalPolicyStore, PersistentApprovalAction,
        PersistentApprovalPolicyKey, PersistentApprovalPolicyStore, PersistentApprovalScope,
    };
    use ironclaw_github_issue_workflow::{
        EngineeredWorkflowSnapshot, GithubIssueSnapshot, GithubIssueStage, GithubIssueStageRunId,
        GithubIssueWorkflowRunId, ProviderContentSummary, RepositorySnapshot,
        StageConstraintSnapshot, StageResultSummary, StageTurnIdentity, StageTurnSubmitter,
        SubmitStageTurnOutcome, SubmitStageTurnRequest, WorkflowActorScope, WorkflowPromptContent,
        WorkflowStateSnapshot, WorkflowWorkspaceMountRef, WorkflowWorkspaceSnapshot,
        render_stage_prompt, stage_result_schema_version,
    };
    use ironclaw_host_api::{
        AgentId, CapabilityId, Principal, ProjectId, TenantId, ThreadId, UserId,
    };
    use ironclaw_reborn_composition::test_support::{
        github_issue_stage_turn_submitter_for_test,
        seed_github_issue_workflow_stage_approval_policies_for_test,
    };
    use ironclaw_threads::{
        InMemorySessionThreadService, MessageStatus, SessionThreadService, ThreadHistoryRequest,
        ThreadScope,
    };
    use ironclaw_turns::{
        CancelRunRequest, CancelRunResponse, EventCursor, GetRunStateRequest, ResumeTurnRequest,
        ResumeTurnResponse, RunProfileId, RunProfileVersion, SubmitTurnRequest, SubmitTurnResponse,
        ThreadBusy, TurnCoordinator, TurnError, TurnId, TurnOriginKind, TurnRunId, TurnRunState,
        TurnScope, TurnStatus, TurnSurfaceType,
    };

    #[tokio::test]
    async fn stage_submitter_persists_thread_message_before_turn_submit() {
        let harness = StageTurnHarness::new(SubmitMode::Accepted);
        let request = submit_request();

        let outcome = harness
            .submitter
            .submit_stage_turn(request.clone())
            .await
            .expect("submit stage turn");

        let SubmitStageTurnOutcome::Submitted { thread_id, .. } = outcome else {
            panic!("expected submitted outcome, got {outcome:?}");
        };
        assert_eq!(thread_id, expected_thread_id(&request.stage_turn_identity));
        assert_eq!(
            harness.coordinator.message_statuses_at_submit(),
            vec![Some(MessageStatus::Accepted)],
            "the thread message must already be accepted before turn submission"
        );

        let history = harness.history(&request).await;
        assert_eq!(history.messages.len(), 1);
        let content = history.messages[0]
            .content
            .as_deref()
            .expect("accepted stage message content");
        let prompt = triage_prompt_content();
        assert_eq!(content, prompt);
        assert!(
            content.contains("builtin.workflow_report_stage_result"),
            "accepted message must carry the rendered result-reporting instructions"
        );
        let metadata: serde_json::Value = serde_json::from_str(
            history
                .thread
                .metadata_json
                .as_deref()
                .expect("stage thread metadata"),
        )
        .expect("metadata json");
        assert_eq!(
            metadata["workspace_mount_ref"]["mount_id"],
            workspace_session_id()
        );
        assert_eq!(metadata["workspace_mount_ref"]["alias"], "/workspace");
    }

    #[tokio::test]
    async fn stage_submitter_replays_existing_submitted_message() {
        let harness = StageTurnHarness::new(SubmitMode::Accepted);
        let request = submit_request();

        let first = harness
            .submitter
            .submit_stage_turn(request.clone())
            .await
            .expect("submit stage turn");
        let second = harness
            .submitter
            .submit_stage_turn(request.clone())
            .await
            .expect("replay submitted stage turn");

        assert_eq!(harness.coordinator.submission_count(), 1);
        assert_eq!(
            second,
            match first {
                SubmitStageTurnOutcome::Submitted {
                    thread_id,
                    turn_run_id,
                } => SubmitStageTurnOutcome::Replayed {
                    thread_id,
                    turn_run_id,
                },
                other => panic!("expected initial submitted outcome, got {other:?}"),
            }
        );
    }

    #[tokio::test]
    async fn stage_submitter_marks_busy_without_second_turn() {
        let harness = StageTurnHarness::new(SubmitMode::Busy);
        let request = submit_request();

        let first = harness
            .submitter
            .submit_stage_turn(request.clone())
            .await
            .expect("busy stage turn");
        let second = harness
            .submitter
            .submit_stage_turn(request.clone())
            .await
            .expect("busy stage turn replay");

        assert!(matches!(first, SubmitStageTurnOutcome::Busy { .. }));
        assert!(matches!(second, SubmitStageTurnOutcome::Busy { .. }));
        assert_eq!(
            harness.coordinator.submission_count(),
            1,
            "busy replay must not submit another turn for the same stage identity"
        );

        let history = harness.history(&request).await;
        assert_eq!(history.messages.len(), 1);
        assert_eq!(history.messages[0].status, MessageStatus::RejectedBusy);
        assert_eq!(history.messages[0].turn_run_id, None);
    }

    #[tokio::test]
    async fn stage_submitter_uses_deterministic_idempotency_key() {
        let harness = StageTurnHarness::new(SubmitMode::Accepted);
        let request = submit_request();

        harness
            .submitter
            .submit_stage_turn(request.clone())
            .await
            .expect("submit stage turn");

        let submissions = harness.coordinator.submissions();
        let submitted = submissions.first().expect("recorded submission");
        assert_eq!(
            submitted.idempotency_key.as_str(),
            request.idempotency_key.as_str()
        );
        assert_eq!(
            submitted
                .requested_run_profile
                .as_ref()
                .expect("requested profile")
                .as_str(),
            "github-bug-triage-v1"
        );
        assert_eq!(
            submitted.source_binding_ref.as_str(),
            request.stage_turn_identity.source_binding_ref()
        );
        assert_eq!(
            submitted.reply_target_binding_ref.as_str(),
            request.stage_turn_identity.reply_target_binding_ref()
        );
    }

    #[tokio::test]
    async fn stage_submitter_does_not_use_trusted_trigger_ingress() {
        let harness = StageTurnHarness::new(SubmitMode::Accepted);
        let request = submit_request();

        harness
            .submitter
            .submit_stage_turn(request)
            .await
            .expect("submit stage turn");

        let submissions = harness.coordinator.submissions();
        let product_context = submissions[0]
            .product_context
            .as_ref()
            .expect("workflow product context");
        assert_eq!(product_context.origin, TurnOriginKind::Inbound);
        assert_eq!(product_context.surface_type, Some(TurnSurfaceType::Direct));
        assert_eq!(
            product_context
                .adapter
                .as_ref()
                .expect("workflow adapter")
                .as_str(),
            "github_issue_workflow"
        );

        let source = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/github_issue_workflow.rs"
        ))
        .expect("read composition source");
        for forbidden in [
            "TrustedInboundTurnRequest",
            "TrustedTriggerSubmitRequest",
            "submit_trusted_trigger_fire",
            "trusted_submit",
        ] {
            assert!(
                !source.contains(forbidden),
                "stage submitter must not use trusted trigger ingress: found {forbidden}"
            );
        }
    }

    #[tokio::test]
    async fn stage_approval_seed_persists_project_scoped_policies_for_side_effect_capabilities() {
        let store = Arc::new(InMemoryPersistentApprovalPolicyStore::new());
        let seeded = seed_github_issue_workflow_stage_approval_policies_for_test(
            store.clone(),
            tenant_id(),
            owner_user_id(),
            agent_id(),
            Some(project_id()),
        )
        .await
        .expect("seed workflow stage approval policies");

        assert_eq!(
            seeded.capability_ids,
            ["builtin.apply_patch", "builtin.shell", "builtin.write_file"]
                .into_iter()
                .map(String::from)
                .collect()
        );

        for capability_id in &seeded.capability_ids {
            let key = PersistentApprovalPolicyKey {
                scope: PersistentApprovalScope {
                    tenant_id: tenant_id(),
                    user_id: owner_user_id(),
                    agent_id: Some(agent_id()),
                    project_id: Some(project_id()),
                },
                action: PersistentApprovalAction::Dispatch,
                capability_id: CapabilityId::new(capability_id).expect("capability id"),
                grantee: Principal::Extension(seeded.loop_driver_grantee.clone()),
            };
            let policy = store
                .lookup(&key)
                .await
                .expect("lookup seeded policy")
                .unwrap_or_else(|| panic!("missing policy for {capability_id}"));
            assert_eq!(policy.constraints.max_invocations, None);
            assert!(
                policy
                    .constraints
                    .allowed_effects
                    .contains(&ironclaw_host_api::EffectKind::DispatchCapability),
                "policy for {capability_id} must authorize dispatch"
            );
        }
    }

    struct StageTurnHarness {
        threads: Arc<InMemorySessionThreadService>,
        coordinator: Arc<RecordingTurnCoordinator>,
        submitter: Arc<dyn StageTurnSubmitter>,
    }

    impl StageTurnHarness {
        fn new(mode: SubmitMode) -> Self {
            let threads = Arc::new(InMemorySessionThreadService::default());
            let coordinator = Arc::new(RecordingTurnCoordinator::new(threads.clone(), mode));
            let submitter = github_issue_stage_turn_submitter_for_test(
                threads.clone(),
                coordinator.clone(),
                actor_user_id(),
                default_agent_id(),
            );
            Self {
                threads,
                coordinator,
                submitter,
            }
        }

        async fn history(
            &self,
            request: &SubmitStageTurnRequest,
        ) -> ironclaw_threads::ThreadHistory {
            self.threads
                .list_thread_history(ThreadHistoryRequest {
                    scope: expected_thread_scope(request),
                    thread_id: expected_thread_id(&request.stage_turn_identity),
                })
                .await
                .expect("thread history")
        }
    }

    #[derive(Debug, Clone, Copy)]
    enum SubmitMode {
        Accepted,
        Busy,
    }

    #[derive(Debug)]
    struct RecordingTurnCoordinator {
        threads: Arc<InMemorySessionThreadService>,
        mode: SubmitMode,
        submissions: Mutex<Vec<SubmitTurnRequest>>,
        message_statuses_at_submit: Mutex<Vec<Option<MessageStatus>>>,
        accepted_run_id: TurnRunId,
        busy_run_id: TurnRunId,
    }

    impl RecordingTurnCoordinator {
        fn new(threads: Arc<InMemorySessionThreadService>, mode: SubmitMode) -> Self {
            Self {
                threads,
                mode,
                submissions: Mutex::new(Vec::new()),
                message_statuses_at_submit: Mutex::new(Vec::new()),
                accepted_run_id: TurnRunId::new(),
                busy_run_id: TurnRunId::new(),
            }
        }

        fn submissions(&self) -> Vec<SubmitTurnRequest> {
            self.submissions
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone()
        }

        fn submission_count(&self) -> usize {
            self.submissions
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .len()
        }

        fn message_statuses_at_submit(&self) -> Vec<Option<MessageStatus>> {
            self.message_statuses_at_submit
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone()
        }
    }

    #[async_trait]
    impl TurnCoordinator for RecordingTurnCoordinator {
        async fn prepare_turn(&self, _scope: TurnScope) -> Result<TurnRunId, TurnError> {
            Ok(TurnRunId::new())
        }

        async fn submit_turn(
            &self,
            request: SubmitTurnRequest,
        ) -> Result<SubmitTurnResponse, TurnError> {
            let observed_status = self
                .threads
                .list_thread_history(ThreadHistoryRequest {
                    scope: thread_scope_from_turn_scope(&request.scope),
                    thread_id: request.scope.thread_id.clone(),
                })
                .await
                .ok()
                .and_then(|history| {
                    history
                        .messages
                        .into_iter()
                        .find(|message| {
                            message.message_id.to_string() == request.accepted_message_ref.as_str()
                        })
                        .map(|message| message.status)
                });
            self.message_statuses_at_submit
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(observed_status);
            self.submissions
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(request.clone());

            match self.mode {
                SubmitMode::Accepted => {
                    let profile = request
                        .requested_run_profile
                        .as_ref()
                        .map(RunProfileId::from_request)
                        .unwrap_or_else(RunProfileId::default_profile);
                    Ok(SubmitTurnResponse::Accepted {
                        turn_id: TurnId::new(),
                        run_id: self.accepted_run_id,
                        status: TurnStatus::Queued,
                        resolved_run_profile_id: profile,
                        resolved_run_profile_version: RunProfileVersion::new(1),
                        event_cursor: EventCursor(1),
                        accepted_message_ref: request.accepted_message_ref,
                        reply_target_binding_ref: request.reply_target_binding_ref,
                    })
                }
                SubmitMode::Busy => Err(TurnError::ThreadBusy(ThreadBusy {
                    active_run_id: self.busy_run_id,
                    status: TurnStatus::Running,
                    event_cursor: EventCursor(7),
                })),
            }
        }

        async fn resume_turn(
            &self,
            _request: ResumeTurnRequest,
        ) -> Result<ResumeTurnResponse, TurnError> {
            Err(TurnError::Unavailable {
                reason: "resume not used in stage turn tests".to_string(),
            })
        }

        async fn cancel_run(
            &self,
            _request: CancelRunRequest,
        ) -> Result<CancelRunResponse, TurnError> {
            Err(TurnError::Unavailable {
                reason: "cancel not used in stage turn tests".to_string(),
            })
        }

        async fn get_run_state(
            &self,
            _request: GetRunStateRequest,
        ) -> Result<TurnRunState, TurnError> {
            Err(TurnError::Unavailable {
                reason: "get state not used in stage turn tests".to_string(),
            })
        }
    }

    fn submit_request() -> SubmitStageTurnRequest {
        let stage_turn_identity = StageTurnIdentity::new(
            GithubIssueWorkflowRunId::from_trusted("workflow-run-stage-turn".to_string())
                .expect("workflow run id"),
            GithubIssueStageRunId::from_trusted("stage-run-triage".to_string())
                .expect("stage run id"),
            GithubIssueStage::Triage,
            1,
            "policy-v1".to_string(),
        );
        let idempotency_key = stage_turn_identity.turn_idempotency_key();
        SubmitStageTurnRequest {
            stage_turn_identity,
            scope: WorkflowActorScope {
                tenant_id: tenant_id(),
                creator_user_id: owner_user_id(),
                agent_id: Some(agent_id()),
                project_id: Some(project_id()),
                workflow_run_id: GithubIssueWorkflowRunId::from_trusted(
                    "workflow-run-stage-turn".to_string(),
                )
                .expect("workflow run id"),
            },
            prompt: WorkflowPromptContent::from(triage_prompt()),
            capability_profile_id: "github_issue_workflow.stage.default".to_string(),
            workspace_mount_ref: Some(WorkflowWorkspaceMountRef {
                mount_id: workspace_session_id().to_string(),
                alias: "/workspace".to_string(),
            }),
            idempotency_key,
        }
    }

    fn triage_prompt_content() -> String {
        triage_prompt().content
    }

    fn triage_prompt() -> ironclaw_github_issue_workflow::StagePromptBundle {
        render_stage_prompt(GithubIssueStage::Triage, &triage_snapshot())
            .expect("render triage prompt")
    }

    fn triage_snapshot() -> EngineeredWorkflowSnapshot {
        EngineeredWorkflowSnapshot {
            issue: GithubIssueSnapshot {
                owner: "nearai".to_string(),
                repo: "ironclaw".to_string(),
                number: 42,
                title: "Stage turn should contain rendered prompt".to_string(),
                url: "https://github.com/nearai/ironclaw/issues/42".to_string(),
                default_branch: "main".to_string(),
                state: "open".to_string(),
                labels: vec!["bug".to_string()],
                summary: "Accepted inbound message lacks prompt body".to_string(),
                provider_content_summaries: vec![ProviderContentSummary {
                    source_ref: "issue-body".to_string(),
                    author: Some("octocat".to_string()),
                    summary: "The stage turn submitter persisted metadata only.".to_string(),
                    trust: "provider".to_string(),
                }],
            },
            workflow: WorkflowStateSnapshot {
                workflow_run_id: "workflow-run-stage-turn".to_string(),
                workflow_policy_key: "github-bug-workflow".to_string(),
                workflow_policy_version: "policy-v1".to_string(),
                status: "running".to_string(),
                mode: "claimed".to_string(),
                active_stage_run_id: Some("stage-run-triage".to_string()),
                event_cursor: 12,
                workflow_run_version: 3,
                active_block_summary: None,
                plan: Vec::new(),
            },
            repository: RepositorySnapshot {
                owner: "nearai".to_string(),
                name: "ironclaw".to_string(),
                default_branch: "main".to_string(),
                base_ref: Some("main".to_string()),
                base_sha: Some("base-sha".to_string()),
                working_branch: None,
                head_sha: None,
                primary_pr_url: None,
            },
            previous_stage_results: vec![StageResultSummary {
                stage: GithubIssueStage::Planning,
                outcome: "not_started".to_string(),
                summary: "No prior completed stages".to_string(),
                evidence: Vec::new(),
            }],
            workspace: Some(WorkflowWorkspaceSnapshot {
                workspace_session_id: Some(workspace_session_id().to_string()),
                thread_id: None,
                turn_run_id: None,
                mount_alias: Some("/workspace".to_string()),
                virtual_root: "/workspace".to_string(),
                changed_files: Vec::new(),
            }),
            constraints: StageConstraintSnapshot {
                stage: GithubIssueStage::Triage,
                stage_goal: "Classify the GitHub issue and choose the next stage.".to_string(),
                allowed_capabilities: vec!["builtin.workflow_report_stage_result".to_string()],
                disallowed_capabilities: Vec::new(),
                result_schema_version: stage_result_schema_version(&GithubIssueStage::Triage)
                    .to_string(),
                completion_tool: "builtin.workflow_report_stage_result".to_string(),
                provider_write_policy: "no_provider_writes".to_string(),
            },
        }
    }

    fn expected_thread_scope(request: &SubmitStageTurnRequest) -> ThreadScope {
        ThreadScope {
            tenant_id: request.scope.tenant_id.clone(),
            agent_id: request
                .scope
                .agent_id
                .clone()
                .unwrap_or_else(default_agent_id),
            project_id: request.scope.project_id.clone(),
            owner_user_id: Some(request.scope.creator_user_id.clone()),
            mission_id: None,
        }
    }

    fn thread_scope_from_turn_scope(scope: &TurnScope) -> ThreadScope {
        ThreadScope {
            tenant_id: scope.tenant_id.clone(),
            agent_id: scope.agent_id.clone().unwrap_or_else(default_agent_id),
            project_id: scope.project_id.clone(),
            owner_user_id: scope.explicit_owner_user_id().cloned(),
            mission_id: None,
        }
    }

    fn expected_thread_id(identity: &StageTurnIdentity) -> ThreadId {
        ThreadId::new(identity.thread_id_seed()).expect("deterministic thread id")
    }

    fn tenant_id() -> TenantId {
        TenantId::new("tenant-github").expect("tenant id")
    }

    fn owner_user_id() -> UserId {
        UserId::new("workflow-owner").expect("owner user id")
    }

    fn actor_user_id() -> UserId {
        UserId::new("workflow-actor").expect("actor user id")
    }

    fn agent_id() -> AgentId {
        AgentId::new("agent-github").expect("agent id")
    }

    fn default_agent_id() -> AgentId {
        AgentId::new("agent-default").expect("default agent id")
    }

    fn project_id() -> ProjectId {
        ProjectId::new("project-github").expect("project id")
    }

    fn workspace_session_id() -> &'static str {
        "11111111-1111-4111-8111-111111111111"
    }
}
