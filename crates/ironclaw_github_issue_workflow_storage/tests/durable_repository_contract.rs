#![cfg(any(feature = "libsql", feature = "postgres"))]

use chrono::Duration;
use ironclaw_github_issue_workflow::{
    BlockWorkflowRunInput, BlockWorkflowRunOutcome, ClaimRunnableWorkflowRunsInput,
    CreateOrGetProviderActionInput, CreateOrGetWorkflowRunOutcome, CreateStageRunInput,
    CreateStageRunOutcome, FailStageRunInput, FailStageRunOutcome, GetStageRunInput,
    GithubCommentRef, GithubIssueBlockKind, GithubIssueBlockState,
    GithubIssueProviderSnapshotSummary, GithubIssueStage, GithubIssueStageRunId,
    GithubIssueWorkflowMode, GithubIssueWorkflowRunStatus, GithubIssueWorkspaceSession,
    GithubIssueWorkspaceSessionId, GithubProviderAccountRef, GithubPullRequestRef,
    GithubRepositorySelector, ListActiveWorkflowRunsForRepositoryInput, ProviderContentSummary,
    RecordWorkflowEventOutcome, TransitionOutcome, WorkflowIdempotencyKey, WorkflowRunTransition,
    WorkflowVerificationSummary, WorkflowWorkspaceMountRef, WorkflowWorkspaceRef,
};

mod support;

use support::*;

mod durable_repository_contract {
    use super::*;

    #[tokio::test]
    async fn durable_create_or_get_workflow_run_is_idempotent() {
        for case in RepositoryCase::cases("run-idempotent").await {
            let repository = case.open().await;
            let first_tenant = tenant(&case.name, 1);
            let issue_ref = issue(&case.name, 42);

            let first = repository
                .create_or_get_workflow_run(workflow_run_input(
                    &case.name,
                    first_tenant.clone(),
                    issue_ref.clone(),
                    fixed_time(10),
                ))
                .await
                .expect("first create");
            let reopened = case.reopen().await;
            let second = reopened
                .create_or_get_workflow_run(workflow_run_input(
                    &case.name,
                    first_tenant,
                    issue_ref.clone(),
                    fixed_time(20),
                ))
                .await
                .expect("second create");
            let other_tenant = reopened
                .create_or_get_workflow_run(workflow_run_input(
                    &case.name,
                    tenant(&case.name, 2),
                    issue_ref,
                    fixed_time(30),
                ))
                .await
                .expect("other tenant create");

            let CreateOrGetWorkflowRunOutcome::Created { run: first_run } = first else {
                panic!("first call must create the run for {}", case.name);
            };
            let CreateOrGetWorkflowRunOutcome::Existing { run: second_run } = second else {
                panic!("second call must reuse the run for {}", case.name);
            };
            let CreateOrGetWorkflowRunOutcome::Created {
                run: other_tenant_run,
            } = other_tenant
            else {
                panic!(
                    "same key in another tenant must create a run for {}",
                    case.name
                );
            };

            assert_eq!(first_run.workflow_run_id, second_run.workflow_run_id);
            assert_ne!(first_run.workflow_run_id, other_tenant_run.workflow_run_id);
        }
    }

    #[tokio::test]
    async fn durable_lists_active_runs_for_repository_after_reload_with_provider_account() {
        for case in RepositoryCase::cases("active-runs-provider-account").await {
            let repository = case.open().await;
            let tenant_id = tenant(&case.name, 1);
            let issue_ref = issue(&case.name, 42);
            let mut input = workflow_run_input(
                &case.name,
                tenant_id.clone(),
                issue_ref.clone(),
                fixed_time(10),
            );
            input.provider_account_ref = Some(GithubProviderAccountRef {
                provider: "github".to_string(),
                account_id: format!("account-{}", case.name),
            });
            let created = repository
                .create_or_get_workflow_run(input)
                .await
                .expect("create workflow run");
            let CreateOrGetWorkflowRunOutcome::Created { run: created_run } = created else {
                panic!("run should be created for {}", case.name);
            };

            let reopened = case.reopen().await;
            let active_runs = reopened
                .list_active_workflow_runs_for_repository(
                    ListActiveWorkflowRunsForRepositoryInput {
                        tenant_id,
                        repository: GithubRepositorySelector::new(
                            issue_ref.owner.clone(),
                            issue_ref.repo.clone(),
                        )
                        .expect("valid repository selector"),
                        limit: 10,
                    },
                )
                .await
                .expect("list active runs");

            assert_eq!(active_runs.len(), 1, "active runs for {}", case.name);
            assert_eq!(active_runs[0].workflow_run_id, created_run.workflow_run_id);
            let expected_account_id = format!("account-{}", case.name);
            assert_eq!(
                active_runs[0]
                    .provider_account_ref
                    .as_ref()
                    .map(|account| account.account_id.as_str()),
                Some(expected_account_id.as_str())
            );
        }
    }

    #[tokio::test]
    async fn durable_event_recording_is_idempotent() {
        for case in RepositoryCase::cases("event-idempotent").await {
            let repository = case.open().await;
            let run = create_run(
                repository.as_ref(),
                &case.name,
                tenant(&case.name, 1),
                issue(&case.name, 42),
            )
            .await;

            let first = repository
                .record_workflow_event(event_input(
                    &case.name,
                    &run,
                    "issue:42:changed:1",
                    fixed_time(10),
                ))
                .await
                .expect("record first event");
            let reopened = case.reopen().await;
            let duplicate = reopened
                .record_workflow_event(event_input(
                    &case.name,
                    &run,
                    "issue:42:changed:1",
                    fixed_time(20),
                ))
                .await
                .expect("record duplicate event");

            let RecordWorkflowEventOutcome::Recorded { event: first_event } = first else {
                panic!("first event must be recorded for {}", case.name);
            };
            let RecordWorkflowEventOutcome::Duplicate { existing } = duplicate else {
                panic!(
                    "duplicate event key must replay existing event for {}",
                    case.name
                );
            };
            assert_eq!(first_event.workflow_event_id, existing.workflow_event_id);
            assert_eq!(first_event.sequence, existing.sequence);
        }
    }

    #[tokio::test]
    async fn durable_lease_claim_excludes_unexpired_lease() {
        for case in RepositoryCase::cases("lease-claim").await {
            let repository = case.open().await;
            let tenant_id = tenant(&case.name, 1);
            create_run(
                repository.as_ref(),
                &case.name,
                tenant_id.clone(),
                issue(&case.name, 42),
            )
            .await;

            let first_claim = repository
                .claim_runnable_workflow_runs(claim_input(
                    &case.name,
                    tenant_id.clone(),
                    fixed_time(10),
                    1,
                ))
                .await
                .expect("first claim");
            let reopened = case.reopen().await;
            let blocked_claim = reopened
                .claim_runnable_workflow_runs(claim_input(
                    &case.name,
                    tenant_id.clone(),
                    fixed_time(20),
                    2,
                ))
                .await
                .expect("blocked claim");
            let expired_claim = reopened
                .claim_runnable_workflow_runs(ClaimRunnableWorkflowRunsInput {
                    tenant_id,
                    worker_id: worker(&case.name, 2),
                    now: fixed_time(71),
                    lease_expires_at: fixed_time(71) + Duration::seconds(60),
                    limit: 10,
                })
                .await
                .expect("expired claim");

            assert_eq!(first_claim.len(), 1, "first claim for {}", case.name);
            assert!(blocked_claim.is_empty(), "blocked claim for {}", case.name);
            assert_eq!(expired_claim.len(), 1, "expired claim for {}", case.name);
            assert_eq!(expired_claim[0].lease_owner, Some(worker(&case.name, 2)));
        }
    }

    #[tokio::test]
    async fn durable_lease_claim_skips_blocked_runs_after_lease_expiry() {
        for case in RepositoryCase::cases("blocked-claim").await {
            let repository = case.open().await;
            let tenant_id = tenant(&case.name, 1);
            let run = create_run(
                repository.as_ref(),
                &case.name,
                tenant_id.clone(),
                issue(&case.name, 42),
            )
            .await;

            let first_claim = repository
                .claim_runnable_workflow_runs(claim_input(
                    &case.name,
                    tenant_id.clone(),
                    fixed_time(10),
                    1,
                ))
                .await
                .expect("first claim");
            let blocked = repository
                .block_workflow_run(BlockWorkflowRunInput {
                    workflow_run_id: run.workflow_run_id.clone(),
                    worker_id: worker(&case.name, 1),
                    active_block: GithubIssueBlockState {
                        kind: GithubIssueBlockKind::BlockedHuman,
                        reason: "waiting on human input".to_string(),
                        blocked_at: fixed_time(20),
                    },
                    now: fixed_time(20),
                })
                .await
                .expect("block workflow run");
            let reopened = case.reopen().await;
            let blocked_claim = reopened
                .claim_runnable_workflow_runs(ClaimRunnableWorkflowRunsInput {
                    tenant_id,
                    worker_id: worker(&case.name, 2),
                    now: fixed_time(71),
                    lease_expires_at: fixed_time(71) + Duration::seconds(60),
                    limit: 10,
                })
                .await
                .expect("claim blocked run after lease expiry");

            assert_eq!(first_claim.len(), 1, "first claim for {}", case.name);
            let BlockWorkflowRunOutcome::Blocked { .. } = blocked else {
                panic!("run must block for {}", case.name);
            };
            assert!(
                blocked_claim.is_empty(),
                "blocked run must not be claimable after lease expiry for {}",
                case.name
            );
        }
    }

    #[tokio::test]
    async fn durable_transition_rejects_stale_version() {
        for case in RepositoryCase::cases("transition-stale").await {
            let repository = case.open().await;
            let tenant_id = tenant(&case.name, 1);
            let run = create_run(
                repository.as_ref(),
                &case.name,
                tenant_id.clone(),
                issue(&case.name, 42),
            )
            .await;
            let claimed = repository
                .claim_runnable_workflow_runs(claim_input(&case.name, tenant_id, fixed_time(10), 1))
                .await
                .expect("claim run")
                .pop()
                .expect("one claimed run");
            let event = match repository
                .record_workflow_event(event_input(
                    &case.name,
                    &run,
                    "issue:42:changed:1",
                    fixed_time(20),
                ))
                .await
                .expect("record event")
            {
                RecordWorkflowEventOutcome::Recorded { event } => event,
                _ => panic!("event must be recorded for {}", case.name),
            };

            let reopened = case.reopen().await;
            let stale = reopened
                .advance_event_cursor_and_transition(advance_input(
                    &case.name,
                    &run,
                    claimed.workflow_run_version + 1,
                    event.sequence,
                    fixed_time(30),
                ))
                .await
                .expect("stale transition");
            let applied = reopened
                .advance_event_cursor_and_transition(advance_input(
                    &case.name,
                    &run,
                    claimed.workflow_run_version,
                    event.sequence,
                    fixed_time(31),
                ))
                .await
                .expect("matching transition");

            let TransitionOutcome::VersionConflict { current } = stale else {
                panic!("stale expected version must be rejected for {}", case.name);
            };
            let TransitionOutcome::Applied { run: advanced } = applied else {
                panic!("matching version and cursor must apply for {}", case.name);
            };
            assert_eq!(current.workflow_run_version, claimed.workflow_run_version);
            assert_eq!(advanced.event_cursor, event.sequence);
            assert_eq!(
                advanced.workflow_run_version,
                claimed.workflow_run_version + 1
            );
        }
    }

    #[tokio::test]
    async fn durable_transition_persists_workspace_session_after_reload() {
        for case in RepositoryCase::cases("transition-workspace-session").await {
            let repository = case.open().await;
            let tenant_id = tenant(&case.name, 1);
            let issue_ref = issue(&case.name, 42);
            let run = create_run(
                repository.as_ref(),
                &case.name,
                tenant_id.clone(),
                issue_ref.clone(),
            )
            .await;
            let claimed = repository
                .claim_runnable_workflow_runs(claim_input(
                    &case.name,
                    tenant_id.clone(),
                    fixed_time(10),
                    1,
                ))
                .await
                .expect("claim run")
                .pop()
                .expect("one claimed run");
            let event = match repository
                .record_workflow_event(event_input(
                    &case.name,
                    &run,
                    "issue:42:changed:workspace-session",
                    fixed_time(20),
                ))
                .await
                .expect("record event")
            {
                RecordWorkflowEventOutcome::Recorded { event } => event,
                _ => panic!("event must be recorded for {}", case.name),
            };

            let workspace_session_id = GithubIssueWorkspaceSessionId::from_trusted(format!(
                "workspace-session-{}",
                case.name
            ))
            .expect("valid workspace session id");
            let mut input = advance_input(
                &case.name,
                &run,
                claimed.workflow_run_version,
                event.sequence,
                fixed_time(30),
            );
            input.transition.workspace_session = Some(GithubIssueWorkspaceSession {
                workspace_session_id: workspace_session_id.clone(),
                workflow_run_id: run.workflow_run_id.clone(),
                repository: GithubRepositorySelector {
                    owner: issue_ref.owner.clone(),
                    repo: issue_ref.repo.clone(),
                },
                base_branch: "main".to_string(),
                base_sha: Some("base-sha-durable".to_string()),
                working_branch: format!("ironclaw/github-bug/{}", run.workflow_run_id),
                current_head_sha: Some("head-sha-durable".to_string()),
                workspace_ref: WorkflowWorkspaceRef {
                    thread_id: None,
                    workspace_session_id: Some(workspace_session_id),
                    turn_run_id: None,
                },
                mount_ref: WorkflowWorkspaceMountRef {
                    mount_id: format!("workspace-mount-{}", case.name),
                    alias: "/workspace".to_string(),
                },
                created_at: fixed_time(25),
            });

            let applied = repository
                .advance_event_cursor_and_transition(input)
                .await
                .expect("transition applies");
            let TransitionOutcome::Applied { .. } = applied else {
                panic!("workspace transition must apply for {}", case.name);
            };

            let reopened = case.reopen().await;
            let reloaded = reopened
                .create_or_get_workflow_run(workflow_run_input(
                    &case.name,
                    tenant_id,
                    issue_ref,
                    fixed_time(40),
                ))
                .await
                .expect("reload run");
            let CreateOrGetWorkflowRunOutcome::Existing { run: reloaded } = reloaded else {
                panic!("run should exist after reload for {}", case.name);
            };

            let expected_session_id = format!("workspace-session-{}", case.name);
            let expected_mount_id = format!("workspace-mount-{}", case.name);
            assert_eq!(
                reloaded.workspace_session_id.as_ref().map(|id| id.as_str()),
                Some(expected_session_id.as_str())
            );
            assert_eq!(
                reloaded
                    .workflow_state
                    .current_workspace_ref
                    .as_ref()
                    .and_then(|workspace_ref| workspace_ref.workspace_session_id.as_ref())
                    .map(|id| id.as_str()),
                Some(expected_session_id.as_str())
            );
            assert_eq!(
                reloaded
                    .workflow_state
                    .current_workspace_mount_ref
                    .as_ref()
                    .map(|mount| (mount.mount_id.as_str(), mount.alias.as_str())),
                Some((expected_mount_id.as_str(), "/workspace"))
            );
        }
    }

    /// FIX #14 durability contract: a claim comment ref recorded onto a run
    /// through `WorkflowRunTransition::claim_comment` must survive a repository
    /// reload across every durable backend, so a later stage can edit that
    /// comment to link the draft PR.
    #[tokio::test]
    async fn durable_transition_persists_claim_comment_after_reload() {
        for case in RepositoryCase::cases("transition-claim-comment").await {
            let repository = case.open().await;
            let tenant_id = tenant(&case.name, 1);
            let issue_ref = issue(&case.name, 42);
            let run = create_run(
                repository.as_ref(),
                &case.name,
                tenant_id.clone(),
                issue_ref.clone(),
            )
            .await;
            let claimed = repository
                .claim_runnable_workflow_runs(claim_input(
                    &case.name,
                    tenant_id.clone(),
                    fixed_time(10),
                    1,
                ))
                .await
                .expect("claim run")
                .pop()
                .expect("one claimed run");
            let event = match repository
                .record_workflow_event(event_input(
                    &case.name,
                    &run,
                    "issue:42:changed:claim-comment",
                    fixed_time(20),
                ))
                .await
                .expect("record event")
            {
                RecordWorkflowEventOutcome::Recorded { event } => event,
                _ => panic!("event must be recorded for {}", case.name),
            };

            let expected_comment_url = format!(
                "https://github.com/nearai/ironclaw/issues/42#issuecomment-{}",
                case.name
            );
            let expected_comment_node = format!("comment-node-{}", case.name);
            let mut input = advance_input(
                &case.name,
                &run,
                claimed.workflow_run_version,
                event.sequence,
                fixed_time(30),
            );
            input.transition.claim_comment = Some(GithubCommentRef {
                node_id: Some(expected_comment_node.clone()),
                url: expected_comment_url.clone(),
            });

            let applied = repository
                .advance_event_cursor_and_transition(input)
                .await
                .expect("transition applies");
            let TransitionOutcome::Applied { run: applied_run } = applied else {
                panic!("claim comment transition must apply for {}", case.name);
            };
            // The claim comment is visible immediately on the applied run.
            assert_eq!(
                applied_run
                    .workflow_state
                    .claim_comment
                    .as_ref()
                    .map(|comment| comment.url.as_str()),
                Some(expected_comment_url.as_str())
            );

            // And it survives a full repository reload (durable contract).
            let reopened = case.reopen().await;
            let reloaded = reopened
                .create_or_get_workflow_run(workflow_run_input(
                    &case.name,
                    tenant_id,
                    issue_ref,
                    fixed_time(40),
                ))
                .await
                .expect("reload run");
            let CreateOrGetWorkflowRunOutcome::Existing { run: reloaded } = reloaded else {
                panic!("run should exist after reload for {}", case.name);
            };
            assert_eq!(
                reloaded
                    .workflow_state
                    .claim_comment
                    .as_ref()
                    .map(|comment| comment.url.as_str()),
                Some(expected_comment_url.as_str()),
                "claim comment url must survive reload for {}",
                case.name
            );
            assert_eq!(
                reloaded
                    .workflow_state
                    .claim_comment
                    .as_ref()
                    .and_then(|comment| comment.node_id.as_deref()),
                Some(expected_comment_node.as_str()),
                "claim comment node id must survive reload for {}",
                case.name
            );
        }
    }

    /// Durability contract for the provider snapshot summary: a
    /// `WorkflowRunTransition::latest_provider_snapshot` applied during a
    /// transition must land on the run's `workflow_state` AND survive a full
    /// repository reload across every durable backend. This guards the bug where
    /// the durable backends hand-copied each transition field but silently
    /// omitted the `latest_provider_snapshot` arm that the in-memory backend
    /// applies, so the PrSynthesis-visible snapshot was dropped across a
    /// transition on filesystem/libSQL/postgres.
    #[tokio::test]
    async fn durable_transition_persists_latest_provider_snapshot_after_reload() {
        for case in RepositoryCase::cases("transition-provider-snapshot").await {
            let repository = case.open().await;
            let tenant_id = tenant(&case.name, 1);
            let issue_ref = issue(&case.name, 42);
            let run = create_run(
                repository.as_ref(),
                &case.name,
                tenant_id.clone(),
                issue_ref.clone(),
            )
            .await;
            let claimed = repository
                .claim_runnable_workflow_runs(claim_input(
                    &case.name,
                    tenant_id.clone(),
                    fixed_time(10),
                    1,
                ))
                .await
                .expect("claim run")
                .pop()
                .expect("one claimed run");
            let event = match repository
                .record_workflow_event(event_input(
                    &case.name,
                    &run,
                    "issue:42:changed:provider-snapshot",
                    fixed_time(20),
                ))
                .await
                .expect("record event")
            {
                RecordWorkflowEventOutcome::Recorded { event } => event,
                _ => panic!("event must be recorded for {}", case.name),
            };

            let expected_title = format!("Triage snapshot for {}", case.name);
            let expected_summary_source = format!("issue:42:comment:{}", case.name);
            let mut input = advance_input(
                &case.name,
                &run,
                claimed.workflow_run_version,
                event.sequence,
                fixed_time(30),
            );
            input.transition.latest_provider_snapshot = Some(GithubIssueProviderSnapshotSummary {
                title: expected_title.clone(),
                state: "open".to_string(),
                author_login: Some("octocat".to_string()),
                labels: vec!["bug".to_string(), "p1".to_string()],
                updated_at: Some(fixed_time(25)),
                comment_count: 3,
                body_present: true,
                content_summaries: vec![ProviderContentSummary {
                    source_ref: expected_summary_source.clone(),
                    author: Some("octocat".to_string()),
                    summary: "Reproduced on main".to_string(),
                    trust: "first_party".to_string(),
                }],
            });

            let applied = repository
                .advance_event_cursor_and_transition(input)
                .await
                .expect("transition applies");
            let TransitionOutcome::Applied { run: applied_run } = applied else {
                panic!("provider snapshot transition must apply for {}", case.name);
            };
            // The snapshot is visible immediately on the applied run.
            assert_eq!(
                applied_run
                    .workflow_state
                    .latest_provider_snapshot
                    .as_ref()
                    .map(|snapshot| snapshot.title.as_str()),
                Some(expected_title.as_str()),
                "provider snapshot must apply onto the run for {}",
                case.name
            );

            // And it survives a full repository reload (durable contract).
            let reopened = case.reopen().await;
            let reloaded = reopened
                .create_or_get_workflow_run(workflow_run_input(
                    &case.name,
                    tenant_id,
                    issue_ref,
                    fixed_time(40),
                ))
                .await
                .expect("reload run");
            let CreateOrGetWorkflowRunOutcome::Existing { run: reloaded } = reloaded else {
                panic!("run should exist after reload for {}", case.name);
            };
            let Some(snapshot) = reloaded.workflow_state.latest_provider_snapshot.as_ref() else {
                panic!(
                    "latest_provider_snapshot must survive reload for {}",
                    case.name
                );
            };
            assert_eq!(
                snapshot.title.as_str(),
                expected_title.as_str(),
                "provider snapshot title must survive reload for {}",
                case.name
            );
            assert_eq!(
                snapshot
                    .content_summaries
                    .iter()
                    .map(|summary| summary.source_ref.as_str())
                    .collect::<Vec<_>>(),
                vec![expected_summary_source.as_str()],
                "provider snapshot content summaries must survive reload for {}",
                case.name
            );
        }
    }

    /// Broad parity guard: a single transition that sets EVERY field on
    /// `WorkflowRunTransition` must round-trip every field across a repository
    /// reload. `WorkflowRunTransition` is destructured below so a future field is
    /// a compile error here until this test is taught how to assert it — that is
    /// the mechanism that catches the NEXT silently-dropped field.
    #[tokio::test]
    async fn durable_transition_persists_all_fields_after_reload() {
        for case in RepositoryCase::cases("transition-all-fields").await {
            let repository = case.open().await;
            let tenant_id = tenant(&case.name, 1);
            let issue_ref = issue(&case.name, 42);
            let run = create_run(
                repository.as_ref(),
                &case.name,
                tenant_id.clone(),
                issue_ref.clone(),
            )
            .await;
            let claimed = repository
                .claim_runnable_workflow_runs(claim_input(
                    &case.name,
                    tenant_id.clone(),
                    fixed_time(10),
                    1,
                ))
                .await
                .expect("claim run")
                .pop()
                .expect("one claimed run");
            let event = match repository
                .record_workflow_event(event_input(
                    &case.name,
                    &run,
                    "issue:42:changed:all-fields",
                    fixed_time(20),
                ))
                .await
                .expect("record event")
            {
                RecordWorkflowEventOutcome::Recorded { event } => event,
                _ => panic!("event must be recorded for {}", case.name),
            };

            let workspace_session_id =
                GithubIssueWorkspaceSessionId::from_trusted(format!("session-{}", case.name))
                    .expect("valid workspace session id");
            let expected_snapshot_title = format!("All-fields snapshot for {}", case.name);
            let expected_pr_url = format!("https://github.com/nearai/ironclaw/pull/{}", case.name);
            let expected_comment_url = format!(
                "https://github.com/nearai/ironclaw/issues/42#issuecomment-{}",
                case.name
            );
            let expected_command_label = format!("cargo test {}", case.name);
            let expected_block_reason = format!("waiting on approval for {}", case.name);
            let expected_session_id = format!("session-{}", case.name);
            let expected_mount_id = format!("mount-{}", case.name);

            // Build ONE transition that sets every field. Destructuring it forces
            // any future field to be considered here.
            let transition = WorkflowRunTransition {
                // Keep the run non-terminal so every other field has a chance to
                // persist (a terminal status would clear lease/stage state).
                status: Some(GithubIssueWorkflowRunStatus::Blocked),
                mode: Some(GithubIssueWorkflowMode::Triage),
                active_block: Some(GithubIssueBlockState {
                    kind: GithubIssueBlockKind::WaitingApproval,
                    reason: expected_block_reason.clone(),
                    blocked_at: fixed_time(22),
                }),
                // Deliberately false: `clear_active_block` and `active_block` are
                // mutually exclusive in the apply order. The `false` branch is
                // exercised here (the block survives); the `true` branch's clear
                // behavior is covered separately by transitions that omit a block.
                clear_active_block: false,
                latest_provider_snapshot: Some(GithubIssueProviderSnapshotSummary {
                    title: expected_snapshot_title.clone(),
                    state: "open".to_string(),
                    author_login: Some("octocat".to_string()),
                    labels: vec!["bug".to_string()],
                    updated_at: Some(fixed_time(23)),
                    comment_count: 1,
                    body_present: true,
                    content_summaries: vec![ProviderContentSummary {
                        source_ref: format!("issue:42:body:{}", case.name),
                        author: Some("octocat".to_string()),
                        summary: "Body summary".to_string(),
                        trust: "first_party".to_string(),
                    }],
                }),
                workspace_session: Some(GithubIssueWorkspaceSession {
                    workspace_session_id: workspace_session_id.clone(),
                    workflow_run_id: run.workflow_run_id.clone(),
                    repository: GithubRepositorySelector {
                        owner: issue_ref.owner.clone(),
                        repo: issue_ref.repo.clone(),
                    },
                    base_branch: "main".to_string(),
                    base_sha: Some("base-sha-all-fields".to_string()),
                    working_branch: format!("ironclaw/github-bug/{}", run.workflow_run_id),
                    current_head_sha: Some("head-sha-all-fields".to_string()),
                    workspace_ref: WorkflowWorkspaceRef {
                        thread_id: None,
                        workspace_session_id: Some(workspace_session_id.clone()),
                        turn_run_id: None,
                    },
                    mount_ref: WorkflowWorkspaceMountRef {
                        mount_id: expected_mount_id.clone(),
                        alias: "/workspace".to_string(),
                    },
                    created_at: fixed_time(24),
                }),
                primary_pr: Some(GithubPullRequestRef {
                    owner: issue_ref.owner.clone(),
                    repo: issue_ref.repo.clone(),
                    number: 4242,
                    node_id: Some(format!("pr-node-{}", case.name)),
                    url: expected_pr_url.clone(),
                    head_branch: format!("ironclaw/github-bug/{}", run.workflow_run_id),
                    head_sha: Some("pr-head-sha".to_string()),
                }),
                claim_comment: Some(GithubCommentRef {
                    node_id: Some(format!("comment-node-{}", case.name)),
                    url: expected_comment_url.clone(),
                }),
                last_verification: Some(WorkflowVerificationSummary {
                    ran: true,
                    passed: true,
                    command_label: expected_command_label.clone(),
                    exit_code: Some(0),
                }),
            };

            // Destructure so a newly added field forces a compile error here,
            // making the author wire it into the assertions below.
            let WorkflowRunTransition {
                status: _,
                mode: _,
                active_block: _,
                clear_active_block: _,
                latest_provider_snapshot: _,
                workspace_session: _,
                primary_pr: _,
                claim_comment: _,
                last_verification: _,
            } = &transition;

            let mut input = advance_input(
                &case.name,
                &run,
                claimed.workflow_run_version,
                event.sequence,
                fixed_time(30),
            );
            input.transition = transition;

            let applied = repository
                .advance_event_cursor_and_transition(input)
                .await
                .expect("transition applies");
            let TransitionOutcome::Applied { .. } = applied else {
                panic!("all-fields transition must apply for {}", case.name);
            };

            let reopened = case.reopen().await;
            let reloaded = reopened
                .create_or_get_workflow_run(workflow_run_input(
                    &case.name,
                    tenant_id,
                    issue_ref,
                    fixed_time(40),
                ))
                .await
                .expect("reload run");
            let CreateOrGetWorkflowRunOutcome::Existing { run: reloaded } = reloaded else {
                panic!("run should exist after reload for {}", case.name);
            };

            // status
            assert_eq!(
                reloaded.status,
                GithubIssueWorkflowRunStatus::Blocked,
                "status must survive reload for {}",
                case.name
            );
            // mode
            assert_eq!(
                reloaded.workflow_state.mode,
                GithubIssueWorkflowMode::Triage,
                "mode must survive reload for {}",
                case.name
            );
            // active_block (also exercises the clear_active_block == false branch)
            assert_eq!(
                reloaded
                    .workflow_state
                    .active_block
                    .as_ref()
                    .map(|block| (block.kind.clone(), block.reason.as_str())),
                Some((
                    GithubIssueBlockKind::WaitingApproval,
                    expected_block_reason.as_str()
                )),
                "active_block must survive reload for {}",
                case.name
            );
            // latest_provider_snapshot
            assert_eq!(
                reloaded
                    .workflow_state
                    .latest_provider_snapshot
                    .as_ref()
                    .map(|snapshot| snapshot.title.as_str()),
                Some(expected_snapshot_title.as_str()),
                "latest_provider_snapshot must survive reload for {}",
                case.name
            );
            // workspace_session (run + workflow_state projections)
            assert_eq!(
                reloaded.workspace_session_id.as_ref().map(|id| id.as_str()),
                Some(expected_session_id.as_str()),
                "workspace_session id must survive reload for {}",
                case.name
            );
            assert_eq!(
                reloaded
                    .workflow_state
                    .current_workspace_mount_ref
                    .as_ref()
                    .map(|mount| mount.mount_id.as_str()),
                Some(expected_mount_id.as_str()),
                "workspace mount ref must survive reload for {}",
                case.name
            );
            // primary_pr
            assert_eq!(
                reloaded
                    .workflow_state
                    .primary_pr
                    .as_ref()
                    .map(|pr| pr.url.as_str()),
                Some(expected_pr_url.as_str()),
                "primary_pr must survive reload for {}",
                case.name
            );
            // claim_comment
            assert_eq!(
                reloaded
                    .workflow_state
                    .claim_comment
                    .as_ref()
                    .map(|comment| comment.url.as_str()),
                Some(expected_comment_url.as_str()),
                "claim_comment must survive reload for {}",
                case.name
            );
            // last_verification
            assert_eq!(
                reloaded
                    .workflow_state
                    .last_verification
                    .as_ref()
                    .map(|verification| {
                        (
                            verification.ran,
                            verification.passed,
                            verification.command_label.as_str(),
                            verification.exit_code,
                        )
                    }),
                Some((true, true, expected_command_label.as_str(), Some(0))),
                "last_verification must survive reload for {}",
                case.name
            );
        }
    }

    #[tokio::test]
    async fn durable_stage_uniqueness_survives_reload() {
        for case in RepositoryCase::cases("stage-uniqueness").await {
            let repository = case.open().await;
            let run = create_run(
                repository.as_ref(),
                &case.name,
                tenant(&case.name, 1),
                issue(&case.name, 42),
            )
            .await;
            let first = repository
                .create_stage_run(stage_input(&run))
                .await
                .expect("create first stage");

            let reopened = case.reopen().await;
            let second = reopened
                .create_stage_run(CreateStageRunInput {
                    workflow_run_id: run.workflow_run_id,
                    stage: GithubIssueStage::Planning,
                    now: fixed_time(30),
                })
                .await
                .expect("create second stage");

            let CreateStageRunOutcome::Created { stage_run_id, .. } = first else {
                panic!("first active stage must be created for {}", case.name);
            };
            let CreateStageRunOutcome::ActiveStageExists {
                existing_stage_run_id,
                run: blocked_run,
            } = second
            else {
                panic!("second active stage must be rejected for {}", case.name);
            };
            assert_eq!(existing_stage_run_id, stage_run_id);
            assert_eq!(blocked_run.active_stage_run_id, Some(existing_stage_run_id));
        }
    }

    #[tokio::test]
    async fn durable_create_stage_run_points_run_and_stage_consistently_after_reload() {
        for case in RepositoryCase::cases("stage-create-consistency").await {
            let repository = case.open().await;
            let run = create_run(
                repository.as_ref(),
                &case.name,
                tenant(&case.name, 1),
                issue(&case.name, 42),
            )
            .await;
            let created = repository
                .create_stage_run(stage_input(&run))
                .await
                .expect("create stage");
            let CreateStageRunOutcome::Created {
                stage_run_id,
                run: created_run,
            } = created
            else {
                panic!("stage must be created for {}", case.name);
            };
            assert_eq!(created_run.active_stage_run_id, Some(stage_run_id.clone()));

            // Stage-row-FIRST ordering means a reopened repo sees BOTH the stage
            // row (active) AND the run pointer — never a pointer to a missing
            // stage row. Re-creating returns ActiveStageExists pointing at the
            // same id, proving the two writes landed and agree across reload.
            let reopened = case.reopen().await;
            let snapshot = reopened
                .get_stage_run(GetStageRunInput {
                    workflow_run_id: run.workflow_run_id.clone(),
                    stage_run_id: stage_run_id.clone(),
                })
                .await
                .expect("get stage")
                .expect("stage row exists after reload");
            assert!(snapshot.active);
            assert!(!snapshot.failed);

            let again = reopened
                .create_stage_run(stage_input(&run))
                .await
                .expect("re-create stage");
            let CreateStageRunOutcome::ActiveStageExists {
                existing_stage_run_id,
                run: pointer_run,
            } = again
            else {
                panic!("active stage pointer must persist for {}", case.name);
            };
            assert_eq!(existing_stage_run_id, stage_run_id);
            assert_eq!(pointer_run.active_stage_run_id, Some(stage_run_id));
        }
    }

    #[tokio::test]
    async fn durable_stage_run_snapshot_and_fail_survive_reload() {
        for case in RepositoryCase::cases("stage-snapshot-fail").await {
            let repository = case.open().await;
            let run = create_run(
                repository.as_ref(),
                &case.name,
                tenant(&case.name, 1),
                issue(&case.name, 42),
            )
            .await;
            let created = repository
                .create_stage_run(stage_input(&run))
                .await
                .expect("create stage");
            let CreateStageRunOutcome::Created { stage_run_id, .. } = created else {
                panic!("stage must be created for {}", case.name);
            };

            // Snapshot survives reload: active, not failed, heartbeat backfilled
            // to created_at.
            let reopened = case.reopen().await;
            let snapshot = reopened
                .get_stage_run(GetStageRunInput {
                    workflow_run_id: run.workflow_run_id.clone(),
                    stage_run_id: stage_run_id.clone(),
                })
                .await
                .expect("get stage")
                .expect("stage row exists");
            assert!(snapshot.active);
            assert!(!snapshot.failed);
            assert_eq!(snapshot.last_heartbeat_at, snapshot.created_at);

            // Fail it; the flip persists across reload; a second fail is a no-op.
            let failed = reopened
                .fail_stage_run(FailStageRunInput {
                    workflow_run_id: run.workflow_run_id.clone(),
                    stage_run_id: stage_run_id.clone(),
                    now: fixed_time(50),
                })
                .await
                .expect("fail stage");
            assert!(matches!(failed, FailStageRunOutcome::Failed { .. }));

            let reopened = case.reopen().await;
            let after = reopened
                .get_stage_run(GetStageRunInput {
                    workflow_run_id: run.workflow_run_id.clone(),
                    stage_run_id: stage_run_id.clone(),
                })
                .await
                .expect("get stage")
                .expect("stage row exists");
            assert!(!after.active);
            assert!(after.failed);

            let again = reopened
                .fail_stage_run(FailStageRunInput {
                    workflow_run_id: run.workflow_run_id.clone(),
                    stage_run_id: stage_run_id.clone(),
                    now: fixed_time(60),
                })
                .await
                .expect("fail stage again");
            assert!(matches!(again, FailStageRunOutcome::AlreadyInactive));

            // Unknown stage id -> None / NotFound.
            let unknown = GithubIssueStageRunId::new();
            assert!(
                reopened
                    .get_stage_run(GetStageRunInput {
                        workflow_run_id: run.workflow_run_id.clone(),
                        stage_run_id: unknown.clone(),
                    })
                    .await
                    .expect("get unknown stage")
                    .is_none()
            );
            let missing = reopened
                .fail_stage_run(FailStageRunInput {
                    workflow_run_id: run.workflow_run_id.clone(),
                    stage_run_id: unknown,
                    now: fixed_time(70),
                })
                .await
                .expect("fail unknown stage");
            assert!(matches!(missing, FailStageRunOutcome::NotFound));
        }
    }

    #[tokio::test]
    async fn durable_provider_action_dedupes_after_reload() {
        for case in RepositoryCase::cases("provider-action-dedupe").await {
            let repository = case.open().await;
            let run = create_run(
                repository.as_ref(),
                &case.name,
                tenant(&case.name, 1),
                issue(&case.name, 42),
            )
            .await;
            let idempotency_key =
                WorkflowIdempotencyKey::from_trusted("provider-action:claim".to_string())
                    .expect("valid action key");

            let first = repository
                .create_or_get_provider_action(provider_action_input(
                    &run,
                    idempotency_key.clone(),
                    fixed_time(10),
                ))
                .await
                .expect("create provider action");
            let reopened = case.reopen().await;
            let second = reopened
                .create_or_get_provider_action(CreateOrGetProviderActionInput {
                    now: fixed_time(20),
                    ..provider_action_input(&run, idempotency_key, fixed_time(20))
                })
                .await
                .expect("dedupe provider action");

            assert_eq!(first.provider_action_id, second.provider_action_id);
            assert_eq!(second.created_at, fixed_time(10));
        }
    }

    #[tokio::test]
    async fn durable_provider_binding_routes_after_reload() {
        for case in RepositoryCase::cases("provider-binding-route").await {
            let repository = case.open().await;
            let first_run = create_run(
                repository.as_ref(),
                &case.name,
                tenant(&case.name, 1),
                issue(&case.name, 42),
            )
            .await;
            let second_run = create_run(
                repository.as_ref(),
                &case.name,
                tenant(&case.name, 1),
                issue(&case.name, 43),
            )
            .await;
            let provider = provider_ref(&case.name, "issue-node-42");
            let first = repository
                .upsert_provider_binding(binding_input(
                    &first_run,
                    provider.clone(),
                    fixed_time(10),
                ))
                .await
                .expect("create provider binding");

            let reopened = case.reopen().await;
            let routed = reopened
                .upsert_provider_binding(binding_input(&second_run, provider, fixed_time(20)))
                .await
                .expect("route provider binding");

            assert_eq!(first.binding_id, routed.binding_id);
            assert_eq!(routed.workflow_run_id, first_run.workflow_run_id);
            assert_eq!(routed.created_at, fixed_time(10));
        }
    }
}
