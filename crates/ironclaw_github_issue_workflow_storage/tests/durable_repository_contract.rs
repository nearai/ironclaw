#![cfg(any(feature = "libsql", feature = "postgres"))]

use chrono::Duration;
use ironclaw_github_issue_workflow::{
    ClaimRunnableWorkflowRunsInput, CreateOrGetProviderActionInput, CreateOrGetWorkflowRunOutcome,
    CreateStageRunInput, CreateStageRunOutcome, GithubIssueStage, RecordWorkflowEventOutcome,
    TransitionOutcome, WorkflowIdempotencyKey,
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
