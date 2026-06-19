use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use async_trait::async_trait;
use chrono::{TimeZone, Utc};
use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId, Timestamp, UserId};
use ironclaw_turns::{TurnRunId, TurnScope};

use super::*;
use crate::{
    ActiveTriggerScanCursor, ClaimDueFireOutcome, ClaimDueFireRequest, ClaimedTriggerFire,
    ClearActiveFireRequest, FireAcceptedRequest, FirePermanentFailedRequest, FireReplayedRequest,
    FireRetryableFailedRequest, FireTerminalFailedRequest, InMemoryTriggerRepository,
    TRIGGER_TRUSTED_ADAPTER_INSTALLATION_ID, TRIGGER_TRUSTED_ADAPTER_KIND,
    TRIGGER_TRUSTED_EXTERNAL_ACTOR_NAMESPACE, TriggerError, TriggerFire, TriggerId,
    TriggerInboundContentRef, TriggerMaterializedPrompt, TriggerPromptMaterializer, TriggerRecord,
    TriggerRepository, TriggerRunHistoryStatus, TriggerRunRecord, TriggerRunStatus,
    TriggerSchedule, TriggerSourceKind, TriggerSourceProvider, TriggerState,
};

fn ts(seconds: i64) -> Timestamp {
    Utc.timestamp_opt(seconds, 0).single().expect("valid ts")
}

fn ymd_hms(year: i32, month: u32, day: u32, hour: u32, minute: u32, second: u32) -> Timestamp {
    Utc.with_ymd_and_hms(year, month, day, hour, minute, second)
        .single()
        .expect("valid ts")
}

fn tenant(value: &str) -> TenantId {
    TenantId::new(value).expect("valid tenant")
}

fn test_turn_scope() -> TurnScope {
    TurnScope::new(
        TenantId::new("test-tenant").expect("tenant id"),
        None,
        None,
        ThreadId::new("test-thread").expect("thread id"),
    )
}

fn user(value: &str) -> UserId {
    UserId::new(value).expect("valid user")
}

fn sample_record(
    trigger_id: TriggerId,
    tenant_id: TenantId,
    next_run_at: Timestamp,
) -> TriggerRecord {
    TriggerRecord {
        trigger_id,
        tenant_id,
        creator_user_id: user("user-a"),
        agent_id: Some(AgentId::new("agent-a").expect("valid agent")),
        project_id: Some(ProjectId::new("project-a").expect("valid project")),
        name: "daily summary".to_string(),
        source: TriggerSourceKind::Schedule,
        schedule: TriggerSchedule::cron("0 8 * * *").expect("valid cron"),
        prompt: "summarize unread mail".to_string(),
        state: TriggerState::Scheduled,
        next_run_at,
        last_run_at: None,
        last_fired_slot: None,
        last_status: None,
        active_fire_slot: None,
        active_run_ref: None,
        created_at: ts(1_704_067_000),
    }
}

#[test]
fn worker_config_rejects_noop_or_unsupported_settings() {
    let config = TriggerPollerWorkerConfig {
        poll_interval: Duration::ZERO,
        ..TriggerPollerWorkerConfig::default()
    };
    assert!(matches!(
        config.validate(),
        Err(TriggerError::InvalidPollerConfig { .. })
    ));

    let config = TriggerPollerWorkerConfig {
        fires_per_tick: 0,
        ..TriggerPollerWorkerConfig::default()
    };
    assert!(matches!(
        config.validate(),
        Err(TriggerError::InvalidPollerConfig { .. })
    ));

    let config = TriggerPollerWorkerConfig {
        max_concurrent_fires_per_trigger: 2,
        ..TriggerPollerWorkerConfig::default()
    };
    assert!(matches!(
        config.validate(),
        Err(TriggerError::InvalidPollerConfig { .. })
    ));
}

#[test]
fn worker_new_rejects_invalid_config() {
    let config = TriggerPollerWorkerConfig {
        fires_per_tick: 0,
        ..TriggerPollerWorkerConfig::default()
    };
    let result = TriggerPollerWorker::new(
        config,
        TriggerPollerWorkerDeps {
            repository: Arc::new(InMemoryTriggerRepository::default()),
            source_provider: Arc::new(crate::ScheduleTriggerSourceProvider),
            materializer: Arc::new(RecordingMaterializer::success("content:trigger-fire")),
            trusted_submitter: Arc::new(RecordingSubmitter::with_outcomes(Vec::new())),
            active_run_lookup: Arc::new(RecordingActiveRunLookup::default()),
        },
    );

    assert!(matches!(
        result,
        Err(TriggerError::InvalidPollerConfig { .. })
    ));
}

fn worker(
    repo: Arc<dyn TriggerRepository>,
    materializer: Arc<RecordingMaterializer>,
    submitter: Arc<RecordingSubmitter>,
    active_lookup: Arc<RecordingActiveRunLookup>,
) -> TriggerPollerWorker {
    worker_with_source_provider(
        repo,
        Arc::new(crate::ScheduleTriggerSourceProvider),
        materializer,
        submitter,
        active_lookup,
    )
}

fn worker_with_source_provider(
    repo: Arc<dyn TriggerRepository>,
    source_provider: Arc<dyn TriggerSourceProvider>,
    materializer: Arc<RecordingMaterializer>,
    submitter: Arc<RecordingSubmitter>,
    active_lookup: Arc<RecordingActiveRunLookup>,
) -> TriggerPollerWorker {
    worker_with_config(
        repo,
        source_provider,
        materializer,
        submitter,
        active_lookup,
        TriggerPollerWorkerConfig::default(),
    )
}

fn worker_with_config(
    repo: Arc<dyn TriggerRepository>,
    source_provider: Arc<dyn TriggerSourceProvider>,
    materializer: Arc<RecordingMaterializer>,
    submitter: Arc<RecordingSubmitter>,
    active_lookup: Arc<RecordingActiveRunLookup>,
    config: TriggerPollerWorkerConfig,
) -> TriggerPollerWorker {
    TriggerPollerWorker::new(
        config,
        TriggerPollerWorkerDeps {
            repository: repo,
            source_provider,
            materializer,
            trusted_submitter: submitter,
            active_run_lookup: active_lookup,
        },
    )
    .expect("valid worker")
}

#[tokio::test]
async fn tick_once_serializes_overlapping_calls_for_one_worker() {
    let repo = Arc::new(TickConcurrencyRepository::default());
    let worker = Arc::new(worker(
        repo.clone(),
        Arc::new(RecordingMaterializer::success("content:trigger-fire")),
        Arc::new(RecordingSubmitter::with_outcomes(Vec::new())),
        Arc::new(RecordingActiveRunLookup::default()),
    ));
    let first = worker.clone();
    let second = worker;

    let (first_result, second_result) = tokio::join!(
        async move { first.tick_once(ts(1_704_067_200)).await },
        async move { second.tick_once(ts(1_704_067_260)).await },
    );

    first_result.expect("first tick");
    second_result.expect("second tick");
    assert_eq!(repo.max_concurrent_due_scans(), 1);
}

#[tokio::test]
async fn tick_processes_one_due_trigger_happy_path() {
    let repo = Arc::new(InMemoryTriggerRepository::default());
    let trigger_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
    let fire_slot = ts(1_704_067_200);
    let record = sample_record(trigger_id, tenant("tenant-a"), fire_slot);
    let expected_next_run_at = record
        .schedule
        .next_slot_after(fire_slot)
        .expect("next run")
        .expect("future run");
    repo.upsert_trigger(record.clone()).await.expect("insert");
    let run_id = TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5a").expect("run id");
    let submitter = Arc::new(RecordingSubmitter::with_outcomes(vec![Ok(
        TrustedTriggerFireSubmitOutcome::Accepted {
            run_id,
            submitted_at: ts(1_704_067_205),
            turn_scope: test_turn_scope(),
        },
    )]));
    let materializer = Arc::new(RecordingMaterializer::success("content:trigger-fire"));
    let worker = worker(
        repo.clone(),
        materializer.clone(),
        submitter.clone(),
        Arc::new(RecordingActiveRunLookup::default()),
    );

    let report = worker.tick_once(fire_slot).await.expect("tick succeeds");

    assert_eq!(report.due_records, 1);
    assert_eq!(
        report.results.last().map(|result| &result.outcome),
        Some(&TriggerPollerFireOutcome::Submitted { run_id })
    );
    assert_eq!(materializer.fires().len(), 1);
    assert_eq!(submitter.requests().len(), 1);
    let request = submitter.requests().pop().expect("submit request");
    assert_eq!(request.fire().identity.trigger_id, trigger_id);
    assert_eq!(request.fire().identity.fire_slot, fire_slot);
    assert_eq!(request.fire().creator_user_id, record.creator_user_id);
    assert_eq!(request.fire().agent_id, record.agent_id);
    assert_eq!(request.fire().project_id, record.project_id);
    assert_eq!(request.content_ref().as_str(), "content:trigger-fire");
    assert_eq!(
        request
            .materialized_prompt()
            .trusted_inbound_binding()
            .external_conversation_id(),
        format!("trigger-{trigger_id}")
    );
    assert_eq!(request.received_at(), fire_slot);
    let (fire, materialized_prompt, received_at) = request.into_parts();
    let (content_ref, trusted_inbound_binding) = materialized_prompt.into_parts();
    assert_eq!(fire.identity.trigger_id, trigger_id);
    assert_eq!(fire.identity.fire_slot, fire_slot);
    assert_eq!(fire.creator_user_id, record.creator_user_id);
    assert_eq!(fire.agent_id, record.agent_id);
    assert_eq!(fire.project_id, record.project_id);
    assert_eq!(content_ref.as_str(), "content:trigger-fire");
    assert_eq!(
        trusted_inbound_binding.adapter_kind(),
        TRIGGER_TRUSTED_ADAPTER_KIND
    );
    assert_eq!(
        trusted_inbound_binding.adapter_installation_id(),
        TRIGGER_TRUSTED_ADAPTER_INSTALLATION_ID
    );
    assert_eq!(
        trusted_inbound_binding.external_actor_namespace(),
        TRIGGER_TRUSTED_EXTERNAL_ACTOR_NAMESPACE
    );
    assert_eq!(
        trusted_inbound_binding.external_actor_id(),
        record.creator_user_id.as_str()
    );
    assert_eq!(
        trusted_inbound_binding.route_thread_id(),
        fire.identity.route_thread_id().as_str()
    );
    assert_eq!(
        trusted_inbound_binding.external_event_id(),
        fire.identity.external_event_id().as_str()
    );
    assert_eq!(received_at, fire_slot);

    let persisted = repo
        .get_trigger(tenant("tenant-a"), trigger_id)
        .await
        .expect("load")
        .expect("record present");
    assert_eq!(persisted.last_status, Some(TriggerRunStatus::Ok));
    assert_eq!(persisted.last_fired_slot, Some(fire_slot));
    assert_eq!(persisted.active_fire_slot, Some(fire_slot));
    assert_eq!(persisted.active_run_ref, Some(run_id));
    assert_eq!(persisted.next_run_at, expected_next_run_at);
}

#[tokio::test]
async fn tick_persists_replayed_submit_with_original_run_ref() {
    let repo = Arc::new(InMemoryTriggerRepository::default());
    let trigger_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
    let fire_slot = ts(1_704_067_200);
    let record = sample_record(trigger_id, tenant("tenant-a"), fire_slot);
    let expected_next_run_at = record
        .schedule
        .next_slot_after(fire_slot)
        .expect("next run")
        .expect("future run");
    repo.upsert_trigger(record).await.expect("insert");
    let original_run_id = TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5a").expect("run id");
    let worker = worker(
        repo.clone(),
        Arc::new(RecordingMaterializer::success("content:trigger-fire")),
        Arc::new(RecordingSubmitter::with_outcomes(vec![Ok(
            TrustedTriggerFireSubmitOutcome::Replayed {
                original_run_id,
                replayed_at: ts(1_704_067_205),
                thread_id: None,
            },
        )])),
        Arc::new(RecordingActiveRunLookup::default()),
    );

    let report = worker.tick_once(fire_slot).await.expect("tick succeeds");

    assert_eq!(
        report.results.last().map(|result| &result.outcome),
        Some(&TriggerPollerFireOutcome::Replayed { original_run_id })
    );
    let persisted = repo
        .get_trigger(tenant("tenant-a"), trigger_id)
        .await
        .expect("load")
        .expect("record present");
    assert_eq!(persisted.last_status, Some(TriggerRunStatus::Ok));
    assert_eq!(persisted.last_fired_slot, Some(fire_slot));
    assert_eq!(persisted.active_fire_slot, Some(fire_slot));
    assert_eq!(persisted.active_run_ref, Some(original_run_id));
    assert_eq!(persisted.next_run_at, expected_next_run_at);
}

/// Regression guard: when the submitter returns `Replayed { thread_id: Some(id) }`,
/// the worker must forward the canonical thread_id to `mark_fire_replayed`.
/// Without this guard, a silent drop of the `thread_id` field in the worker's
/// `Replayed` arm would leave the run row with `thread_id = None`, breaking the
/// automation panel's ability to link to the replayed thread.
#[tokio::test]
async fn tick_persists_replayed_submit_with_canonical_thread_id() {
    let repo = Arc::new(InMemoryTriggerRepository::default());
    let trigger_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
    let fire_slot = ts(1_704_067_200);
    let record = sample_record(trigger_id, tenant("tenant-a"), fire_slot);
    repo.upsert_trigger(record).await.expect("insert");
    let original_run_id = TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5a").expect("run id");
    let canonical_thread_id = ThreadId::new("thread-canonical-replay").expect("valid thread id");
    let worker = worker(
        repo.clone(),
        Arc::new(RecordingMaterializer::success("content:trigger-fire")),
        Arc::new(RecordingSubmitter::with_outcomes(vec![Ok(
            TrustedTriggerFireSubmitOutcome::Replayed {
                original_run_id,
                replayed_at: ts(1_704_067_205),
                thread_id: Some(canonical_thread_id.clone()),
            },
        )])),
        Arc::new(RecordingActiveRunLookup::default()),
    );

    worker.tick_once(fire_slot).await.expect("tick succeeds");

    // The run history row for this fire slot must carry the canonical thread_id.
    let runs = repo
        .list_trigger_run_history(tenant("tenant-a"), trigger_id, 10)
        .await
        .expect("list run history");
    let run = runs
        .iter()
        .find(|r| r.fire_slot == fire_slot)
        .expect("run row for fire_slot");
    assert_eq!(
        run.thread_id,
        Some(canonical_thread_id),
        "replayed thread_id must be forwarded to mark_fire_replayed and stored on the run row"
    );
}

#[tokio::test]
async fn tick_skips_claim_race_already_active_without_materializing() {
    let trigger_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
    let fire_slot = ts(1_704_067_200);
    let active_run_ref = TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5a").expect("run id");
    let repository = Arc::new(ClaimRaceRepository::new(
        sample_record(trigger_id, tenant("tenant-a"), fire_slot),
        ClaimDueFireOutcome::AlreadyActive {
            active_fire_slot: Some(fire_slot),
            active_run_ref: Some(active_run_ref),
        },
    ));
    let materializer = Arc::new(RecordingMaterializer::success("content:trigger-fire"));
    let submitter = Arc::new(RecordingSubmitter::with_outcomes(Vec::new()));
    let worker = worker(
        repository,
        materializer.clone(),
        submitter.clone(),
        Arc::new(RecordingActiveRunLookup::default()),
    );

    let report = worker.tick_once(fire_slot).await.expect("tick succeeds");

    assert_eq!(report.due_records, 1);
    assert_eq!(
        report.results.last().map(|result| &result.outcome),
        Some(&TriggerPollerFireOutcome::SkippedAlreadyActive {
            active_fire_slot: fire_slot,
            active_run_ref: Some(active_run_ref)
        })
    );
    assert_eq!(materializer.fires().len(), 0);
    assert_eq!(submitter.requests().len(), 0);
}

#[tokio::test]
async fn tick_rejects_already_active_claim_without_active_fire_slot() {
    let trigger_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
    let fire_slot = ts(1_704_067_200);
    let active_run_ref = TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5a").expect("run id");
    let repository = Arc::new(ClaimRaceRepository::new(
        sample_record(trigger_id, tenant("tenant-a"), fire_slot),
        ClaimDueFireOutcome::AlreadyActive {
            active_fire_slot: None,
            active_run_ref: Some(active_run_ref),
        },
    ));
    let materializer = Arc::new(RecordingMaterializer::success("content:trigger-fire"));
    let submitter = Arc::new(RecordingSubmitter::with_outcomes(Vec::new()));
    let worker = worker(
        repository,
        materializer.clone(),
        submitter.clone(),
        Arc::new(RecordingActiveRunLookup::default()),
    );

    let report = worker.tick_once(fire_slot).await.expect("tick succeeds");

    assert_eq!(report.due_records, 1);
    assert_eq!(
        report.results.last().map(|result| &result.outcome),
        Some(&TriggerPollerFireOutcome::DueFireFailed {
            reason: TriggerPollerFailureReason::Backend
        })
    );
    assert_eq!(materializer.fires().len(), 0);
    assert_eq!(submitter.requests().len(), 0);
}

#[tokio::test]
async fn tick_skips_claim_race_not_due_without_materializing() {
    let trigger_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
    let fire_slot = ts(1_704_067_200);
    let record = sample_record(trigger_id, tenant("tenant-a"), fire_slot);
    let repository = Arc::new(ClaimRaceRepository::new(
        record.clone(),
        ClaimDueFireOutcome::NotDue { record },
    ));
    let materializer = Arc::new(RecordingMaterializer::success("content:trigger-fire"));
    let submitter = Arc::new(RecordingSubmitter::with_outcomes(Vec::new()));
    let worker = worker(
        repository,
        materializer.clone(),
        submitter.clone(),
        Arc::new(RecordingActiveRunLookup::default()),
    );

    let report = worker.tick_once(fire_slot).await.expect("tick succeeds");

    assert_eq!(report.due_records, 1);
    assert_eq!(
        report.results.last().map(|result| &result.outcome),
        Some(&TriggerPollerFireOutcome::SkippedNotDue)
    );
    assert_eq!(materializer.fires().len(), 0);
    assert_eq!(submitter.requests().len(), 0);
}

#[tokio::test]
async fn tick_skips_claim_race_not_found_without_materializing() {
    let trigger_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
    let fire_slot = ts(1_704_067_200);
    let repository = Arc::new(ClaimRaceRepository::new(
        sample_record(trigger_id, tenant("tenant-a"), fire_slot),
        ClaimDueFireOutcome::NotFound,
    ));
    let materializer = Arc::new(RecordingMaterializer::success("content:trigger-fire"));
    let submitter = Arc::new(RecordingSubmitter::with_outcomes(Vec::new()));
    let worker = worker(
        repository,
        materializer.clone(),
        submitter.clone(),
        Arc::new(RecordingActiveRunLookup::default()),
    );

    let report = worker.tick_once(fire_slot).await.expect("tick succeeds");

    assert_eq!(report.due_records, 1);
    assert_eq!(
        report.results.last().map(|result| &result.outcome),
        Some(&TriggerPollerFireOutcome::SkippedNotFound)
    );
    assert_eq!(materializer.fires().len(), 0);
    assert_eq!(submitter.requests().len(), 0);
}

#[tokio::test]
async fn tick_skips_active_trigger_but_processes_other_due_trigger() {
    let repo = Arc::new(InMemoryTriggerRepository::default());
    let fire_slot = ts(1_704_067_200);
    let active_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
    let due_id = TriggerId::parse("01J00000000000000000000000").expect("ulid");
    let active_run_ref = TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5a").expect("run id");
    let mut active = sample_record(active_id, tenant("tenant-a"), fire_slot);
    active.active_fire_slot = Some(fire_slot);
    active.active_run_ref = Some(active_run_ref);
    let due = sample_record(due_id, tenant("tenant-a"), fire_slot);
    repo.upsert_trigger(active).await.expect("insert active");
    repo.upsert_trigger(due).await.expect("insert due");
    let due_run_ref = TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5b").expect("run id");
    let submitter = Arc::new(RecordingSubmitter::with_outcomes(vec![Ok(
        TrustedTriggerFireSubmitOutcome::Accepted {
            run_id: due_run_ref,
            submitted_at: fire_slot,
            turn_scope: test_turn_scope(),
        },
    )]));
    let active_lookup = Arc::new(RecordingActiveRunLookup::with_state(
        TriggerActiveRunState::Nonterminal,
    ));
    let worker = worker(
        repo.clone(),
        Arc::new(RecordingMaterializer::success("content:trigger-fire")),
        submitter,
        active_lookup,
    );

    let report = worker.tick_once(fire_slot).await.expect("tick succeeds");

    assert_eq!(report.active_records, 1);
    assert_eq!(report.due_records, 1);
    assert!(
        report
            .results
            .iter()
            .any(|result| result.trigger_id == active_id
                && matches!(
                    result.outcome,
                    TriggerPollerFireOutcome::SkippedAlreadyActive { .. }
                ))
    );
    assert!(
        report
            .results
            .iter()
            .any(|result| result.trigger_id == due_id
                && result.outcome
                    == TriggerPollerFireOutcome::Submitted {
                        run_id: due_run_ref
                    })
    );
}

#[tokio::test]
async fn tick_clears_terminal_active_run() {
    let repo = Arc::new(InMemoryTriggerRepository::default());
    let trigger_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
    let fire_slot = ts(1_704_067_200);
    let run_id = TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5a").expect("run id");
    let mut record = sample_record(trigger_id, tenant("tenant-a"), ts(1_704_067_260));
    record.active_fire_slot = Some(fire_slot);
    record.active_run_ref = Some(run_id);
    repo.upsert_trigger(record).await.expect("insert active");
    let active_lookup = Arc::new(RecordingActiveRunLookup::with_state(
        TriggerActiveRunState::Terminal {
            status: TriggerRunHistoryStatus::Ok,
        },
    ));
    let worker = worker(
        repo.clone(),
        Arc::new(RecordingMaterializer::success("content:trigger-fire")),
        Arc::new(RecordingSubmitter::with_outcomes(Vec::new())),
        active_lookup.clone(),
    );

    let report = worker.tick_once(fire_slot).await.expect("tick succeeds");

    assert_eq!(report.active_records, 1);
    assert_eq!(
        report.results.last().map(|result| &result.outcome),
        Some(&TriggerPollerFireOutcome::ClearedTerminalActive { run_id })
    );
    assert_eq!(
        active_lookup.requests(),
        vec![TriggerActiveRunStateRequest {
            tenant_id: tenant("tenant-a"),
            trigger_id,
            fire_slot,
            run_id,
        }]
    );
    let persisted = repo
        .get_trigger(tenant("tenant-a"), trigger_id)
        .await
        .expect("load")
        .expect("record present");
    assert_eq!(persisted.active_fire_slot, None);
    assert_eq!(persisted.active_run_ref, None);
    let runs = repo
        .list_trigger_run_history(tenant("tenant-a"), trigger_id, 10)
        .await
        .expect("list run history");
    assert_eq!(runs[0].status, TriggerRunHistoryStatus::Ok);
}

#[tokio::test]
async fn tick_records_failed_terminal_active_run_as_error() {
    let repo = Arc::new(InMemoryTriggerRepository::default());
    let trigger_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZY").expect("ulid");
    let fire_slot = ts(1_704_067_200);
    let run_id = TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5d").expect("run id");
    let mut record = sample_record(trigger_id, tenant("tenant-a"), ts(1_704_067_260));
    record.active_fire_slot = Some(fire_slot);
    record.active_run_ref = Some(run_id);
    repo.upsert_trigger(record).await.expect("insert active");
    let worker = worker(
        repo.clone(),
        Arc::new(RecordingMaterializer::success("content:trigger-fire")),
        Arc::new(RecordingSubmitter::with_outcomes(Vec::new())),
        Arc::new(RecordingActiveRunLookup::with_state(
            TriggerActiveRunState::Terminal {
                status: TriggerRunHistoryStatus::Error,
            },
        )),
    );

    let report = worker.tick_once(fire_slot).await.expect("tick succeeds");

    assert_eq!(
        report.results.last().map(|result| &result.outcome),
        Some(&TriggerPollerFireOutcome::ClearedTerminalActive { run_id })
    );
    let runs = repo
        .list_trigger_run_history(tenant("tenant-a"), trigger_id, 10)
        .await
        .expect("list run history");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].run_id, Some(run_id));
    assert_eq!(runs[0].status, TriggerRunHistoryStatus::Error);
    assert!(runs[0].completed_at.is_some());
}

#[tokio::test]
async fn tick_clears_blocked_active_run_and_unblocks_schedule() {
    // Regression for #4986: a recurring fire parked on an approval/auth gate
    // must not hold the active lock forever. The poller clears it, records the
    // fire as failed, and the trigger is left schedulable again.
    let repo = Arc::new(InMemoryTriggerRepository::default());
    let trigger_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZX").expect("ulid");
    let fire_slot = ts(1_704_067_200);
    let run_id = TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5e").expect("run id");
    let mut record = sample_record(trigger_id, tenant("tenant-a"), ts(1_704_067_260));
    record.active_fire_slot = Some(fire_slot);
    record.active_run_ref = Some(run_id);
    repo.upsert_trigger(record).await.expect("insert active");
    let worker = worker(
        repo.clone(),
        Arc::new(RecordingMaterializer::success("content:trigger-fire")),
        Arc::new(RecordingSubmitter::with_outcomes(Vec::new())),
        Arc::new(RecordingActiveRunLookup::with_state(
            TriggerActiveRunState::Blocked,
        )),
    );

    let report = worker.tick_once(fire_slot).await.expect("tick succeeds");

    assert_eq!(
        report.results.last().map(|result| &result.outcome),
        Some(&TriggerPollerFireOutcome::ClearedBlockedActive { run_id })
    );
    let persisted = repo
        .get_trigger(tenant("tenant-a"), trigger_id)
        .await
        .expect("load")
        .expect("record present");
    // Active lock released, so the next due slot can fire.
    assert_eq!(persisted.active_fire_slot, None);
    assert_eq!(persisted.active_run_ref, None);
    assert_eq!(persisted.state, TriggerState::Scheduled);
    let runs = repo
        .list_trigger_run_history(tenant("tenant-a"), trigger_id, 10)
        .await
        .expect("list run history");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].run_id, Some(run_id));
    assert_eq!(runs[0].status, TriggerRunHistoryStatus::Error);
    assert!(runs[0].completed_at.is_some());
}

#[tokio::test]
async fn tick_active_cleanup_cursor_reaches_terminal_rows_after_blocked_page() {
    let repo = Arc::new(InMemoryTriggerRepository::default());
    let first_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
    let second_id = TriggerId::parse("01J00000000000000000000000").expect("ulid");
    let third_id = TriggerId::parse("01J00000000000000000000001").expect("ulid");
    let fourth_id = TriggerId::parse("01J00000000000000000000002").expect("ulid");
    let terminal_id = TriggerId::parse("01J00000000000000000000003").expect("ulid");
    let first_slot = ts(1_704_067_200);
    let second_slot = ts(1_704_067_260);
    let third_slot = ts(1_704_067_320);
    let fourth_slot = ts(1_704_067_380);
    let terminal_slot = ts(1_704_067_440);
    let first_run = TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5a").expect("run id");
    let second_run = TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5b").expect("run id");
    let third_run = TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5c").expect("run id");
    let fourth_run = TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5d").expect("run id");
    let terminal_run = TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5e").expect("run id");

    let mut first = sample_record(first_id, tenant("tenant-a"), ts(1_704_067_800));
    first.active_fire_slot = Some(first_slot);
    first.active_run_ref = Some(first_run);
    let mut second = sample_record(second_id, tenant("tenant-a"), ts(1_704_067_800));
    second.active_fire_slot = Some(second_slot);
    second.active_run_ref = Some(second_run);
    let mut third = sample_record(third_id, tenant("tenant-a"), ts(1_704_067_800));
    third.active_fire_slot = Some(third_slot);
    third.active_run_ref = Some(third_run);
    let mut fourth = sample_record(fourth_id, tenant("tenant-a"), ts(1_704_067_800));
    fourth.active_fire_slot = Some(fourth_slot);
    fourth.active_run_ref = Some(fourth_run);
    let mut terminal = sample_record(terminal_id, tenant("tenant-a"), ts(1_704_067_800));
    terminal.active_fire_slot = Some(terminal_slot);
    terminal.active_run_ref = Some(terminal_run);
    repo.upsert_trigger(first).await.expect("insert first");
    repo.upsert_trigger(second).await.expect("insert second");
    repo.upsert_trigger(third).await.expect("insert third");
    repo.upsert_trigger(fourth).await.expect("insert fourth");
    repo.upsert_trigger(terminal)
        .await
        .expect("insert terminal");

    let active_lookup = Arc::new(RecordingActiveRunLookup::with_results(vec![
        Ok(TriggerActiveRunState::Nonterminal),
        Ok(TriggerActiveRunState::Nonterminal),
        Ok(TriggerActiveRunState::Nonterminal),
        Ok(TriggerActiveRunState::Nonterminal),
        Ok(TriggerActiveRunState::Terminal {
            status: TriggerRunHistoryStatus::Ok,
        }),
    ]));
    let worker = worker_with_config(
        repo.clone(),
        Arc::new(crate::ScheduleTriggerSourceProvider),
        Arc::new(RecordingMaterializer::success("content:trigger-fire")),
        Arc::new(RecordingSubmitter::with_outcomes(Vec::new())),
        active_lookup.clone(),
        TriggerPollerWorkerConfig {
            fires_per_tick: 2,
            ..TriggerPollerWorkerConfig::default()
        },
    );

    let first_report = worker.tick_once(first_slot).await.expect("first tick");
    assert_eq!(first_report.active_records, 2);
    assert!(
        first_report
            .results
            .iter()
            .all(|result| result.trigger_id != terminal_id)
    );

    let second_report = worker.tick_once(second_slot).await.expect("second tick");
    assert_eq!(second_report.active_records, 2);
    assert!(
        second_report
            .results
            .iter()
            .all(|result| result.trigger_id != terminal_id)
    );

    let third_report = worker.tick_once(third_slot).await.expect("third tick");
    assert_eq!(third_report.active_records, 1);
    assert!(
        third_report
            .results
            .iter()
            .any(|result| result.trigger_id == terminal_id
                && result.outcome
                    == TriggerPollerFireOutcome::ClearedTerminalActive {
                        run_id: terminal_run
                    })
    );
    assert_eq!(active_lookup.requests().len(), 5);
    let persisted = repo
        .get_trigger(tenant("tenant-a"), terminal_id)
        .await
        .expect("load terminal")
        .expect("terminal record");
    assert_eq!(persisted.active_fire_slot, None);
    assert_eq!(persisted.active_run_ref, None);
}

#[tokio::test]
async fn tick_active_cleanup_cursor_wraps_to_start_when_page_is_empty() {
    let repo = Arc::new(InMemoryTriggerRepository::default());
    let first_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
    let second_id = TriggerId::parse("01J00000000000000000000000").expect("ulid");
    let third_id = TriggerId::parse("01J00000000000000000000001").expect("ulid");
    let first_slot = ts(1_704_067_200);
    let second_slot = ts(1_704_067_260);
    let third_slot = ts(1_704_067_320);
    let first_run = TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5a").expect("run id");
    let second_run = TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5b").expect("run id");
    let third_run = TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5c").expect("run id");

    let mut first = sample_record(first_id, tenant("tenant-a"), ts(1_704_067_800));
    first.active_fire_slot = Some(first_slot);
    first.active_run_ref = Some(first_run);
    let mut second = sample_record(second_id, tenant("tenant-a"), ts(1_704_067_800));
    second.active_fire_slot = Some(second_slot);
    second.active_run_ref = Some(second_run);
    let mut third = sample_record(third_id, tenant("tenant-a"), ts(1_704_067_800));
    third.active_fire_slot = Some(third_slot);
    third.active_run_ref = Some(third_run);
    repo.upsert_trigger(first).await.expect("insert first");
    repo.upsert_trigger(second).await.expect("insert second");
    repo.upsert_trigger(third).await.expect("insert third");

    let active_lookup = Arc::new(RecordingActiveRunLookup::with_results(vec![
        Ok(TriggerActiveRunState::Nonterminal),
        Ok(TriggerActiveRunState::Nonterminal),
        Ok(TriggerActiveRunState::Nonterminal),
        Ok(TriggerActiveRunState::Nonterminal),
        Ok(TriggerActiveRunState::Nonterminal),
    ]));
    let worker = worker_with_config(
        repo,
        Arc::new(crate::ScheduleTriggerSourceProvider),
        Arc::new(RecordingMaterializer::success("content:trigger-fire")),
        Arc::new(RecordingSubmitter::with_outcomes(Vec::new())),
        active_lookup.clone(),
        TriggerPollerWorkerConfig {
            fires_per_tick: 2,
            ..TriggerPollerWorkerConfig::default()
        },
    );

    let first_report = worker.tick_once(first_slot).await.expect("first tick");
    assert_eq!(first_report.active_records, 2);
    let second_report = worker.tick_once(second_slot).await.expect("second tick");
    assert_eq!(second_report.active_records, 1);

    let third_report = worker.tick_once(third_slot).await.expect("third tick");
    assert_eq!(third_report.active_records, 2);
    assert_eq!(
        third_report
            .results
            .iter()
            .map(|result| result.trigger_id)
            .collect::<Vec<_>>(),
        vec![first_id, second_id]
    );
    assert_eq!(
        active_lookup
            .requests()
            .into_iter()
            .map(|request| request.trigger_id)
            .collect::<Vec<_>>(),
        vec![first_id, second_id, third_id, first_id, second_id]
    );
}

#[tokio::test]
async fn tick_active_cleanup_cursor_wraps_to_empty_page_succeeds_with_zero_active_records() {
    let repo = Arc::new(InMemoryTriggerRepository::default());
    let trigger_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
    let fire_slot = ts(1_704_067_200);
    let run_id = TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5a").expect("run id");
    let mut record = sample_record(trigger_id, tenant("tenant-a"), ts(1_704_067_800));
    record.active_fire_slot = Some(fire_slot);
    record.active_run_ref = Some(run_id);
    repo.upsert_trigger(record).await.expect("insert active");

    let worker = worker_with_config(
        repo.clone(),
        Arc::new(crate::ScheduleTriggerSourceProvider),
        Arc::new(RecordingMaterializer::success("content:trigger-fire")),
        Arc::new(RecordingSubmitter::with_outcomes(Vec::new())),
        Arc::new(RecordingActiveRunLookup::with_state(
            TriggerActiveRunState::Nonterminal,
        )),
        TriggerPollerWorkerConfig {
            fires_per_tick: 1,
            ..TriggerPollerWorkerConfig::default()
        },
    );

    let first_report = worker.tick_once(fire_slot).await.expect("first tick");
    assert_eq!(first_report.active_records, 1);

    repo.remove_trigger(tenant("tenant-a"), trigger_id)
        .await
        .expect("remove active");

    let second_report = worker.tick_once(fire_slot).await.expect("second tick");
    assert_eq!(second_report.active_records, 0);
    let third_report = worker.tick_once(fire_slot).await.expect("third tick");
    assert_eq!(third_report.active_records, 0);
}

#[tokio::test]
async fn tick_fails_when_wrap_refetch_returns_backend_error() {
    let trigger_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
    let fire_slot = ts(1_704_067_200);
    let run_id = TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5a").expect("run id");
    let mut record = sample_record(trigger_id, tenant("tenant-a"), ts(1_704_067_800));
    record.active_fire_slot = Some(fire_slot);
    record.active_run_ref = Some(run_id);
    let repo = Arc::new(ActiveWrapRefetchErrorRepository::new(record));
    let worker = worker_with_config(
        repo.clone(),
        Arc::new(crate::ScheduleTriggerSourceProvider),
        Arc::new(RecordingMaterializer::success("content:trigger-fire")),
        Arc::new(RecordingSubmitter::with_outcomes(Vec::new())),
        Arc::new(RecordingActiveRunLookup::with_state(
            TriggerActiveRunState::Nonterminal,
        )),
        TriggerPollerWorkerConfig {
            fires_per_tick: 1,
            ..TriggerPollerWorkerConfig::default()
        },
    );

    let first_report = worker.tick_once(fire_slot).await.expect("first tick");
    assert_eq!(first_report.active_records, 1);

    let error = worker
        .tick_once(fire_slot)
        .await
        .expect_err("wrap refetch fails");
    assert!(matches!(error, TriggerError::Backend { .. }));
    assert_eq!(repo.active_scan_call_shapes(), vec![false, true, false]);
}

#[tokio::test]
async fn tick_retries_active_page_when_clear_fails_before_advancing_cursor() {
    let first_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
    let second_id = TriggerId::parse("01J00000000000000000000000").expect("ulid");
    let third_id = TriggerId::parse("01J00000000000000000000001").expect("ulid");
    let first_slot = ts(1_704_067_200);
    let second_slot = ts(1_704_067_260);
    let third_slot = ts(1_704_067_320);
    let first_run = TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5a").expect("run id");
    let second_run = TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5b").expect("run id");
    let third_run = TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5c").expect("run id");

    let mut first = sample_record(first_id, tenant("tenant-a"), ts(1_704_067_800));
    first.active_fire_slot = Some(first_slot);
    first.active_run_ref = Some(first_run);
    let mut second = sample_record(second_id, tenant("tenant-a"), ts(1_704_067_800));
    second.active_fire_slot = Some(second_slot);
    second.active_run_ref = Some(second_run);
    let mut third = sample_record(third_id, tenant("tenant-a"), ts(1_704_067_800));
    third.active_fire_slot = Some(third_slot);
    third.active_run_ref = Some(third_run);

    let repo = Arc::new(ActiveClearFailsOnceRepository::new(
        vec![first, second, third],
        second_id,
    ));
    let active_lookup = Arc::new(RecordingActiveRunLookup::with_results(vec![
        Ok(TriggerActiveRunState::Terminal {
            status: TriggerRunHistoryStatus::Ok,
        }),
        Ok(TriggerActiveRunState::Terminal {
            status: TriggerRunHistoryStatus::Ok,
        }),
        Ok(TriggerActiveRunState::Terminal {
            status: TriggerRunHistoryStatus::Ok,
        }),
        Ok(TriggerActiveRunState::Terminal {
            status: TriggerRunHistoryStatus::Ok,
        }),
    ]));
    let worker = worker_with_config(
        repo.clone(),
        Arc::new(crate::ScheduleTriggerSourceProvider),
        Arc::new(RecordingMaterializer::success("content:trigger-fire")),
        Arc::new(RecordingSubmitter::with_outcomes(Vec::new())),
        active_lookup,
        TriggerPollerWorkerConfig {
            fires_per_tick: 2,
            ..TriggerPollerWorkerConfig::default()
        },
    );

    let first_error = worker.tick_once(first_slot).await.expect_err("clear fails");
    assert!(matches!(first_error, TriggerError::Backend { .. }));

    let second_report = worker.tick_once(second_slot).await.expect("retry tick");

    assert_eq!(second_report.active_records, 2);
    assert_eq!(
        repo.clear_requests(),
        vec![first_id, second_id, second_id, third_id]
    );
    assert!(
        second_report
            .results
            .iter()
            .any(|result| result.trigger_id == second_id
                && result.outcome
                    == TriggerPollerFireOutcome::ClearedTerminalActive { run_id: second_run })
    );
    assert!(
        second_report
            .results
            .iter()
            .any(|result| result.trigger_id == third_id
                && result.outcome
                    == TriggerPollerFireOutcome::ClearedTerminalActive { run_id: third_run })
    );
}

#[tokio::test]
async fn tick_reports_terminal_active_clear_race() {
    let trigger_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
    let fire_slot = ts(1_704_067_200);
    let run_id = TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5a").expect("run id");
    let mut record = sample_record(trigger_id, tenant("tenant-a"), ts(1_704_067_260));
    record.active_fire_slot = Some(fire_slot);
    record.active_run_ref = Some(run_id);
    let worker = worker(
        Arc::new(ActiveClearRaceRepository {
            active_record: record,
        }),
        Arc::new(RecordingMaterializer::success("content:trigger-fire")),
        Arc::new(RecordingSubmitter::with_outcomes(Vec::new())),
        Arc::new(RecordingActiveRunLookup::with_state(
            TriggerActiveRunState::Terminal {
                status: TriggerRunHistoryStatus::Ok,
            },
        )),
    );

    let report = worker.tick_once(fire_slot).await.expect("tick succeeds");

    assert_eq!(report.active_records, 1);
    assert_eq!(
        report.results.last().map(|result| &result.outcome),
        Some(&TriggerPollerFireOutcome::SkippedAlreadyCleared { run_id })
    );
}

#[tokio::test]
async fn tick_clears_terminal_active_and_processes_due_trigger() {
    let repo = Arc::new(InMemoryTriggerRepository::default());
    let active_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
    let due_id = TriggerId::parse("01J00000000000000000000000").expect("ulid");
    let fire_slot = ts(1_704_067_200);
    let run_id = TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5a").expect("run id");
    let mut active = sample_record(active_id, tenant("tenant-a"), ts(1_704_067_260));
    active.active_fire_slot = Some(fire_slot);
    active.active_run_ref = Some(run_id);
    repo.upsert_trigger(active).await.expect("insert active");
    repo.upsert_trigger(sample_record(due_id, tenant("tenant-a"), fire_slot))
        .await
        .expect("insert due");
    let due_run_id = TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5b").expect("run id");
    let worker = worker(
        repo.clone(),
        Arc::new(RecordingMaterializer::success("content:trigger-fire")),
        Arc::new(RecordingSubmitter::with_outcomes(vec![Ok(
            TrustedTriggerFireSubmitOutcome::Accepted {
                run_id: due_run_id,
                submitted_at: fire_slot,
                turn_scope: test_turn_scope(),
            },
        )])),
        Arc::new(RecordingActiveRunLookup::with_state(
            TriggerActiveRunState::Terminal {
                status: TriggerRunHistoryStatus::Ok,
            },
        )),
    );

    let report = worker.tick_once(fire_slot).await.expect("tick succeeds");

    assert!(
        report
            .results
            .iter()
            .any(|result| result.trigger_id == active_id
                && result.outcome == TriggerPollerFireOutcome::ClearedTerminalActive { run_id })
    );
    assert!(
        report
            .results
            .iter()
            .any(|result| result.trigger_id == due_id
                && result.outcome == TriggerPollerFireOutcome::Submitted { run_id: due_run_id })
    );
}

#[tokio::test]
async fn tick_reports_active_lookup_error_and_continues_to_due_triggers() {
    let repo = Arc::new(InMemoryTriggerRepository::default());
    let active_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
    let due_id = TriggerId::parse("01J00000000000000000000000").expect("ulid");
    let fire_slot = ts(1_704_067_200);
    let run_id = TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5a").expect("run id");
    let mut active = sample_record(active_id, tenant("tenant-a"), ts(1_704_067_260));
    active.active_fire_slot = Some(fire_slot);
    active.active_run_ref = Some(run_id);
    repo.upsert_trigger(active).await.expect("insert active");
    repo.upsert_trigger(sample_record(due_id, tenant("tenant-a"), fire_slot))
        .await
        .expect("insert due");
    let due_run_id = TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5b").expect("run id");
    let worker = worker(
        repo,
        Arc::new(RecordingMaterializer::success("content:trigger-fire")),
        Arc::new(RecordingSubmitter::with_outcomes(vec![Ok(
            TrustedTriggerFireSubmitOutcome::Accepted {
                run_id: due_run_id,
                submitted_at: fire_slot,
                turn_scope: test_turn_scope(),
            },
        )])),
        Arc::new(RecordingActiveRunLookup::with_results(vec![Err(
            TriggerError::Backend {
                reason: "turn state unavailable".to_string(),
            },
        )])),
    );

    let report = worker.tick_once(fire_slot).await.expect("tick succeeds");

    assert!(
        report
            .results
            .iter()
            .any(|result| result.trigger_id == active_id
                && matches!(
                    result.outcome,
                    TriggerPollerFireOutcome::ActiveRunLookupFailed {
                        run_id: actual_run_id,
                        reason: TriggerPollerFailureReason::ActiveRunLookup,
                    } if actual_run_id == run_id
                ))
    );
    assert!(
        report
            .results
            .iter()
            .any(|result| result.trigger_id == due_id
                && result.outcome == TriggerPollerFireOutcome::Submitted { run_id: due_run_id })
    );
}

#[tokio::test]
async fn tick_retries_active_lookup_error_before_advancing_cursor() {
    let repo = Arc::new(InMemoryTriggerRepository::default());
    let failed_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
    let terminal_id = TriggerId::parse("01J00000000000000000000000").expect("ulid");
    let failed_slot = ts(1_704_067_200);
    let terminal_slot = ts(1_704_067_260);
    let failed_run = TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5a").expect("run id");
    let terminal_run = TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5b").expect("run id");
    let mut failed = sample_record(failed_id, tenant("tenant-a"), ts(1_704_067_800));
    failed.active_fire_slot = Some(failed_slot);
    failed.active_run_ref = Some(failed_run);
    let mut terminal = sample_record(terminal_id, tenant("tenant-a"), ts(1_704_067_800));
    terminal.active_fire_slot = Some(terminal_slot);
    terminal.active_run_ref = Some(terminal_run);
    repo.upsert_trigger(failed).await.expect("insert failed");
    repo.upsert_trigger(terminal)
        .await
        .expect("insert terminal");

    let active_lookup = Arc::new(RecordingActiveRunLookup::with_results(vec![
        Err(TriggerError::Backend {
            reason: "turn state unavailable".to_string(),
        }),
        Ok(TriggerActiveRunState::Terminal {
            status: TriggerRunHistoryStatus::Ok,
        }),
        Ok(TriggerActiveRunState::Terminal {
            status: TriggerRunHistoryStatus::Ok,
        }),
    ]));
    let worker = worker_with_config(
        repo.clone(),
        Arc::new(crate::ScheduleTriggerSourceProvider),
        Arc::new(RecordingMaterializer::success("content:trigger-fire")),
        Arc::new(RecordingSubmitter::with_outcomes(Vec::new())),
        active_lookup.clone(),
        TriggerPollerWorkerConfig {
            fires_per_tick: 2,
            ..TriggerPollerWorkerConfig::default()
        },
    );

    let first_report = worker.tick_once(failed_slot).await.expect("first tick");
    assert!(
        first_report
            .results
            .iter()
            .any(|result| result.trigger_id == failed_id
                && matches!(
                    result.outcome,
                    TriggerPollerFireOutcome::ActiveRunLookupFailed { .. }
                ))
    );
    assert!(
        first_report
            .results
            .iter()
            .any(|result| result.trigger_id == terminal_id
                && result.outcome
                    == TriggerPollerFireOutcome::ClearedTerminalActive {
                        run_id: terminal_run
                    })
    );

    let second_report = worker.tick_once(terminal_slot).await.expect("second tick");
    assert_eq!(second_report.active_records, 1);
    assert!(
        second_report
            .results
            .iter()
            .any(|result| result.trigger_id == failed_id
                && result.outcome
                    == TriggerPollerFireOutcome::ClearedTerminalActive { run_id: failed_run })
    );
    assert_eq!(
        active_lookup
            .requests()
            .into_iter()
            .map(|request| request.trigger_id)
            .collect::<Vec<_>>(),
        vec![failed_id, terminal_id, failed_id]
    );
}

#[tokio::test]
async fn tick_replayed_submit_can_be_cleared_on_a_later_tick_without_stopping_due_work() {
    let repo = Arc::new(InMemoryTriggerRepository::default());
    let replayed_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
    let due_id = TriggerId::parse("01J00000000000000000000000").expect("ulid");
    let fire_slot = ts(1_704_067_200);
    let replayed_run_id = TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5a").expect("run id");
    repo.upsert_trigger(sample_record(replayed_id, tenant("tenant-a"), fire_slot))
        .await
        .expect("insert replayed candidate");
    let first_worker = worker(
        repo.clone(),
        Arc::new(RecordingMaterializer::success("content:trigger-fire")),
        Arc::new(RecordingSubmitter::with_outcomes(vec![Ok(
            TrustedTriggerFireSubmitOutcome::Replayed {
                original_run_id: replayed_run_id,
                replayed_at: ts(1_704_067_205),
                thread_id: None,
            },
        )])),
        Arc::new(RecordingActiveRunLookup::default()),
    );

    let first_report = first_worker.tick_once(fire_slot).await.expect("first tick");
    assert_eq!(
        first_report.results.last().map(|result| &result.outcome),
        Some(&TriggerPollerFireOutcome::Replayed {
            original_run_id: replayed_run_id
        })
    );
    let persisted_after_replay = repo
        .get_trigger(tenant("tenant-a"), replayed_id)
        .await
        .expect("reload replayed")
        .expect("replayed record");
    assert_eq!(persisted_after_replay.active_fire_slot, Some(fire_slot));
    assert_eq!(persisted_after_replay.active_run_ref, Some(replayed_run_id));

    repo.upsert_trigger(sample_record(due_id, tenant("tenant-a"), fire_slot))
        .await
        .expect("insert later due");

    let second_due_run_id =
        TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5c").expect("run id");
    let second_worker = worker(
        repo.clone(),
        Arc::new(RecordingMaterializer::success("content:trigger-fire")),
        Arc::new(RecordingSubmitter::with_outcomes(vec![Ok(
            TrustedTriggerFireSubmitOutcome::Accepted {
                run_id: second_due_run_id,
                submitted_at: fire_slot,
                turn_scope: test_turn_scope(),
            },
        )])),
        Arc::new(RecordingActiveRunLookup::with_results(vec![Ok(
            TriggerActiveRunState::Terminal {
                status: TriggerRunHistoryStatus::Ok,
            },
        )])),
    );

    let second_report = second_worker
        .tick_once(fire_slot)
        .await
        .expect("second tick");

    assert_eq!(second_report.active_records, 1);
    assert_eq!(second_report.due_records, 1);
    assert!(
        second_report
            .results
            .iter()
            .any(|result| result.trigger_id == replayed_id
                && result.outcome
                    == TriggerPollerFireOutcome::ClearedTerminalActive {
                        run_id: replayed_run_id,
                    })
    );
    assert!(
        second_report
            .results
            .iter()
            .any(|result| result.trigger_id == due_id
                && result.outcome
                    == TriggerPollerFireOutcome::Submitted {
                        run_id: second_due_run_id
                    })
    );
    assert_eq!(
        repo.get_trigger(tenant("tenant-a"), replayed_id)
            .await
            .expect("reload replayed after cleanup")
            .expect("replayed record after cleanup")
            .active_fire_slot,
        None
    );
    assert_eq!(
        repo.get_trigger(tenant("tenant-a"), replayed_id)
            .await
            .expect("reload replayed after cleanup")
            .expect("replayed record after cleanup")
            .active_run_ref,
        None
    );
}

#[tokio::test]
async fn tick_keeps_missing_active_run_blocked() {
    let repo = Arc::new(InMemoryTriggerRepository::default());
    let trigger_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
    let fire_slot = ts(1_704_067_200);
    let run_id = TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5a").expect("run id");
    let mut record = sample_record(trigger_id, tenant("tenant-a"), fire_slot);
    record.active_fire_slot = Some(fire_slot);
    record.active_run_ref = Some(run_id);
    repo.upsert_trigger(record).await.expect("insert active");
    let active_lookup = Arc::new(RecordingActiveRunLookup::with_state(
        TriggerActiveRunState::Missing,
    ));
    let worker = worker(
        repo.clone(),
        Arc::new(RecordingMaterializer::success("content:trigger-fire")),
        Arc::new(RecordingSubmitter::with_outcomes(Vec::new())),
        active_lookup.clone(),
    );

    let report = worker.tick_once(fire_slot).await.expect("tick succeeds");

    assert!(matches!(
        report.results.last().map(|result| &result.outcome),
        Some(TriggerPollerFireOutcome::SkippedAlreadyActive { .. })
    ));
    assert_eq!(
        active_lookup.requests(),
        vec![TriggerActiveRunStateRequest {
            tenant_id: tenant("tenant-a"),
            trigger_id,
            fire_slot,
            run_id,
        }]
    );
    let persisted = repo
        .get_trigger(tenant("tenant-a"), trigger_id)
        .await
        .expect("load")
        .expect("record present");
    assert_eq!(persisted.active_fire_slot, Some(fire_slot));
    assert_eq!(persisted.active_run_ref, Some(run_id));
}

#[tokio::test]
async fn tick_keeps_claim_only_active_fire_blocked() {
    let repo = Arc::new(InMemoryTriggerRepository::default());
    let trigger_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
    let fire_slot = ts(1_704_067_200);
    let mut record = sample_record(trigger_id, tenant("tenant-a"), fire_slot);
    record.active_fire_slot = Some(fire_slot);
    record.active_run_ref = None;
    repo.upsert_trigger(record).await.expect("insert active");
    let materializer = Arc::new(RecordingMaterializer::success("content:trigger-fire"));
    let submitter = Arc::new(RecordingSubmitter::with_outcomes(Vec::new()));
    let active_lookup = Arc::new(RecordingActiveRunLookup::with_state(
        TriggerActiveRunState::Terminal {
            status: TriggerRunHistoryStatus::Ok,
        },
    ));
    let worker = worker(
        repo.clone(),
        materializer.clone(),
        submitter.clone(),
        active_lookup.clone(),
    );

    let report = worker.tick_once(fire_slot).await.expect("tick succeeds");

    assert!(matches!(
        report.results.first().map(|result| &result.outcome),
        Some(TriggerPollerFireOutcome::SkippedAlreadyActive {
            active_fire_slot: _,
            active_run_ref: None
        })
    ));
    assert_eq!(materializer.fires().len(), 0);
    assert_eq!(submitter.requests().len(), 0);
    assert_eq!(active_lookup.requests().len(), 0);
    let persisted = repo
        .get_trigger(tenant("tenant-a"), trigger_id)
        .await
        .expect("load")
        .expect("record present");
    assert_eq!(persisted.active_fire_slot, Some(fire_slot));
    assert_eq!(persisted.active_run_ref, None);
}

#[tokio::test]
async fn tick_active_cleanup_cursor_advances_past_claim_only_record() {
    let repo = Arc::new(InMemoryTriggerRepository::default());
    let claim_only_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZY").expect("ulid");
    let claim_only_slot = ts(1_704_067_200);
    let mut claim_only = sample_record(claim_only_id, tenant("tenant-a"), claim_only_slot);
    claim_only.next_run_at = ts(1_704_067_800);
    claim_only.active_fire_slot = Some(claim_only_slot);
    claim_only.active_run_ref = None;
    repo.upsert_trigger(claim_only)
        .await
        .expect("insert claim-only active");

    let terminal_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
    let terminal_slot = ts(1_704_067_260);
    let terminal_run = TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5a").expect("run id");
    let mut terminal = sample_record(terminal_id, tenant("tenant-a"), terminal_slot);
    terminal.next_run_at = ts(1_704_067_800);
    terminal.active_fire_slot = Some(terminal_slot);
    terminal.active_run_ref = Some(terminal_run);
    repo.upsert_trigger(terminal)
        .await
        .expect("insert terminal active");

    let materializer = Arc::new(RecordingMaterializer::success("content:trigger-fire"));
    let submitter = Arc::new(RecordingSubmitter::with_outcomes(Vec::new()));
    let active_lookup = Arc::new(RecordingActiveRunLookup::with_state(
        TriggerActiveRunState::Terminal {
            status: TriggerRunHistoryStatus::Ok,
        },
    ));
    let worker = worker_with_config(
        repo.clone(),
        Arc::new(crate::ScheduleTriggerSourceProvider),
        materializer,
        submitter,
        active_lookup.clone(),
        TriggerPollerWorkerConfig {
            fires_per_tick: 1,
            ..TriggerPollerWorkerConfig::default()
        },
    );

    let first_report = worker.tick_once(claim_only_slot).await.expect("first tick");
    let second_report = worker.tick_once(terminal_slot).await.expect("second tick");

    assert!(matches!(
        first_report.results.first().map(|result| &result.outcome),
        Some(TriggerPollerFireOutcome::SkippedAlreadyActive {
            active_run_ref: None,
            ..
        })
    ));
    assert!(matches!(
        second_report.results.first().map(|result| &result.outcome),
        Some(TriggerPollerFireOutcome::ClearedTerminalActive { run_id })
            if *run_id == terminal_run
    ));
    assert_eq!(
        active_lookup.requests(),
        vec![TriggerActiveRunStateRequest {
            tenant_id: tenant("tenant-a"),
            trigger_id: terminal_id,
            fire_slot: terminal_slot,
            run_id: terminal_run,
        }]
    );
    let claim_only = repo
        .get_trigger(tenant("tenant-a"), claim_only_id)
        .await
        .expect("load claim-only")
        .expect("claim-only active record present");
    assert_eq!(claim_only.active_fire_slot, Some(claim_only_slot));
    assert_eq!(claim_only.active_run_ref, None);
    let terminal = repo
        .get_trigger(tenant("tenant-a"), terminal_id)
        .await
        .expect("load terminal")
        .expect("terminal record present");
    assert_eq!(terminal.active_fire_slot, None);
    assert_eq!(terminal.active_run_ref, None);
}

#[tokio::test]
async fn tick_retryable_submit_failure_clears_active_and_keeps_slot_retryable() {
    let repo = Arc::new(InMemoryTriggerRepository::default());
    let trigger_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
    let fire_slot = ts(1_704_067_200);
    repo.upsert_trigger(sample_record(trigger_id, tenant("tenant-a"), fire_slot))
        .await
        .expect("insert");
    let worker = worker(
        repo.clone(),
        Arc::new(RecordingMaterializer::success("content:trigger-fire")),
        Arc::new(RecordingSubmitter::with_outcomes(vec![Err(
            TriggerError::Backend {
                reason: "trusted submit retryable".to_string(),
            },
        )])),
        Arc::new(RecordingActiveRunLookup::default()),
    );

    let report = worker.tick_once(fire_slot).await.expect("tick succeeds");

    assert!(matches!(
        report.results.last().map(|result| &result.outcome),
        Some(TriggerPollerFireOutcome::RetryableFailed {
            reason: TriggerPollerFailureReason::Backend,
        })
    ));
    let persisted = repo
        .get_trigger(tenant("tenant-a"), trigger_id)
        .await
        .expect("load")
        .expect("record present");
    assert_eq!(persisted.last_status, Some(TriggerRunStatus::Error));
    assert_eq!(persisted.next_run_at, fire_slot);
    assert_eq!(persisted.active_fire_slot, None);
    assert_eq!(persisted.active_run_ref, None);
}

#[tokio::test]
async fn tick_accepted_mark_fire_missing_reports_due_failure() {
    let trigger_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
    let fire_slot = ts(1_704_067_200);
    let mut claimed_record = sample_record(trigger_id, tenant("tenant-a"), fire_slot);
    claimed_record.active_fire_slot = Some(fire_slot);
    let run_id = TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5a").expect("run id");
    let worker = worker(
        Arc::new(AcceptedMissingRepository {
            claimed_record,
            fire_slot,
        }),
        Arc::new(RecordingMaterializer::success("content:trigger-fire")),
        Arc::new(RecordingSubmitter::with_outcomes(vec![Ok(
            TrustedTriggerFireSubmitOutcome::Accepted {
                run_id,
                submitted_at: fire_slot,
                turn_scope: test_turn_scope(),
            },
        )])),
        Arc::new(RecordingActiveRunLookup::default()),
    );

    let report = worker.tick_once(fire_slot).await.expect("tick succeeds");

    assert!(report.results.iter().any(|result| {
        result.trigger_id == trigger_id
            && matches!(
                &result.outcome,
                TriggerPollerFireOutcome::DueFireFailed { reason }
                    if *reason == TriggerPollerFailureReason::Backend
            )
    }));
}

#[tokio::test]
async fn tick_replayed_mark_fire_missing_reports_due_failure() {
    let trigger_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
    let fire_slot = ts(1_704_067_200);
    let mut claimed_record = sample_record(trigger_id, tenant("tenant-a"), fire_slot);
    claimed_record.active_fire_slot = Some(fire_slot);
    let original_run_id = TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5a").expect("run id");
    let worker = worker(
        Arc::new(ReplayedMissingRepository {
            claimed_record,
            fire_slot,
        }),
        Arc::new(RecordingMaterializer::success("content:trigger-fire")),
        Arc::new(RecordingSubmitter::with_outcomes(vec![Ok(
            TrustedTriggerFireSubmitOutcome::Replayed {
                original_run_id,
                replayed_at: fire_slot,
                thread_id: None,
            },
        )])),
        Arc::new(RecordingActiveRunLookup::default()),
    );

    let report = worker.tick_once(fire_slot).await.expect("tick succeeds");

    assert!(report.results.iter().any(|result| {
        result.trigger_id == trigger_id
            && matches!(
                &result.outcome,
                TriggerPollerFireOutcome::DueFireFailed { reason }
                    if *reason == TriggerPollerFailureReason::Backend
            )
    }));
}

#[tokio::test]
async fn tick_fails_when_active_trigger_list_returns_backend_error() {
    let worker = worker(
        Arc::new(ActiveListErrorRepository),
        Arc::new(RecordingMaterializer::success("content:trigger-fire")),
        Arc::new(RecordingSubmitter::with_outcomes(Vec::new())),
        Arc::new(RecordingActiveRunLookup::default()),
    );

    let error = worker
        .tick_once(ts(1_704_067_200))
        .await
        .expect_err("active list failure should abort tick");

    assert!(matches!(error, TriggerError::Backend { .. }));
}

#[tokio::test]
async fn tick_reports_due_record_error_and_continues_to_later_due_trigger() {
    let failed_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
    let success_id = TriggerId::parse("01J00000000000000000000000").expect("ulid");
    let fire_slot = ts(1_704_067_200);
    let mut failed = sample_record(failed_id, tenant("tenant-a"), fire_slot);
    failed.active_fire_slot = Some(fire_slot);
    let mut success = sample_record(success_id, tenant("tenant-b"), fire_slot);
    success.active_fire_slot = Some(fire_slot);
    let success_run_id = TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5a").expect("run id");
    let worker = worker(
        Arc::new(DueErrorThenSuccessRepository {
            failed_record: failed,
            success_record: success,
            fire_slot,
        }),
        Arc::new(RecordingMaterializer::success("content:trigger-fire")),
        Arc::new(RecordingSubmitter::with_outcomes(vec![Ok(
            TrustedTriggerFireSubmitOutcome::Accepted {
                run_id: success_run_id,
                submitted_at: fire_slot,
                turn_scope: test_turn_scope(),
            },
        )])),
        Arc::new(RecordingActiveRunLookup::default()),
    );

    let report = worker.tick_once(fire_slot).await.expect("tick succeeds");

    assert_eq!(report.due_records, 2);
    assert!(
        report
            .results
            .iter()
            .any(|result| result.trigger_id == failed_id
                && matches!(
                    result.outcome,
                    TriggerPollerFireOutcome::DueFireFailed {
                        reason: TriggerPollerFailureReason::Backend,
                    }
                ))
    );
    assert!(
        report
            .results
            .iter()
            .any(|result| result.trigger_id == success_id
                && result.outcome
                    == TriggerPollerFireOutcome::Submitted {
                        run_id: success_run_id
                    })
    );
}

#[tokio::test]
async fn tick_submitter_backend_error_clears_active_and_keeps_slot_retryable() {
    let repo = Arc::new(InMemoryTriggerRepository::default());
    let trigger_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
    let fire_slot = ts(1_704_067_200);
    repo.upsert_trigger(sample_record(trigger_id, tenant("tenant-a"), fire_slot))
        .await
        .expect("insert");
    let worker = worker(
        repo.clone(),
        Arc::new(RecordingMaterializer::success("content:trigger-fire")),
        Arc::new(RecordingSubmitter::with_outcomes(vec![Err(
            TriggerError::Backend {
                reason: "turn submit unavailable".to_string(),
            },
        )])),
        Arc::new(RecordingActiveRunLookup::default()),
    );

    let report = worker.tick_once(fire_slot).await.expect("tick succeeds");

    assert!(matches!(
        report.results.last().map(|result| &result.outcome),
        Some(TriggerPollerFireOutcome::RetryableFailed {
            reason: TriggerPollerFailureReason::Backend,
        })
    ));
    let persisted = repo
        .get_trigger(tenant("tenant-a"), trigger_id)
        .await
        .expect("load")
        .expect("record present");
    assert_eq!(persisted.last_status, Some(TriggerRunStatus::Error));
    assert_eq!(persisted.next_run_at, fire_slot);
    assert_eq!(persisted.active_fire_slot, None);
    assert_eq!(persisted.active_run_ref, None);
}

#[tokio::test]
async fn tick_permanent_submit_failure_advances_next_slot() {
    let repo = Arc::new(InMemoryTriggerRepository::default());
    let trigger_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
    let fire_slot = ts(1_704_067_200);
    let record = sample_record(trigger_id, tenant("tenant-a"), fire_slot);
    let expected_next_run_at = record
        .schedule
        .next_slot_after(fire_slot)
        .expect("next run")
        .expect("future run");
    repo.upsert_trigger(record).await.expect("insert");
    let worker = worker(
        repo.clone(),
        Arc::new(RecordingMaterializer::success("content:trigger-fire")),
        Arc::new(RecordingSubmitter::with_outcomes(vec![Err(
            TriggerError::InvalidMaterialization {
                reason: "trusted submit permanent".to_string(),
            },
        )])),
        Arc::new(RecordingActiveRunLookup::default()),
    );

    let report = worker.tick_once(fire_slot).await.expect("tick succeeds");

    assert!(matches!(
        report.results.last().map(|result| &result.outcome),
        Some(TriggerPollerFireOutcome::PermanentFailed {
            reason: TriggerPollerFailureReason::InvalidMaterialization,
        })
    ));
    let persisted = repo
        .get_trigger(tenant("tenant-a"), trigger_id)
        .await
        .expect("load")
        .expect("record present");
    assert_eq!(persisted.last_status, Some(TriggerRunStatus::Error));
    assert_eq!(persisted.next_run_at, expected_next_run_at);
    assert_eq!(persisted.active_fire_slot, None);
    assert_eq!(persisted.active_run_ref, None);
}

#[tokio::test]
async fn tick_permanent_materialization_failure_advances_next_slot() {
    let repo = Arc::new(InMemoryTriggerRepository::default());
    let trigger_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
    let fire_slot = ts(1_704_067_200);
    let record = sample_record(trigger_id, tenant("tenant-a"), fire_slot);
    let expected_next_run_at = record
        .schedule
        .next_slot_after(fire_slot)
        .expect("next run")
        .expect("future run");
    repo.upsert_trigger(record).await.expect("insert");
    let worker = worker(
        repo.clone(),
        Arc::new(RecordingMaterializer::failure(
            TriggerError::InvalidMaterialization {
                reason: "bad prompt content ref".to_string(),
            },
        )),
        Arc::new(RecordingSubmitter::with_outcomes(Vec::new())),
        Arc::new(RecordingActiveRunLookup::default()),
    );

    let report = worker.tick_once(fire_slot).await.expect("tick succeeds");

    assert!(matches!(
        report.results.last().map(|result| &result.outcome),
        Some(TriggerPollerFireOutcome::PermanentFailed {
            reason: TriggerPollerFailureReason::InvalidMaterialization,
        })
    ));
    let persisted = repo
        .get_trigger(tenant("tenant-a"), trigger_id)
        .await
        .expect("load")
        .expect("record present");
    assert_eq!(persisted.last_status, Some(TriggerRunStatus::Error));
    assert_eq!(persisted.next_run_at, expected_next_run_at);
    assert_eq!(persisted.active_fire_slot, None);
    assert_eq!(persisted.active_run_ref, None);
}

#[tokio::test]
async fn tick_source_provider_none_persists_permanent_failure_with_next_slot() {
    let repo = Arc::new(InMemoryTriggerRepository::default());
    let trigger_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
    let fire_slot = ts(1_704_067_200);
    let record = sample_record(trigger_id, tenant("tenant-a"), fire_slot);
    let expected_next_run_at = record
        .schedule
        .next_slot_after(fire_slot)
        .expect("next run")
        .expect("future run");
    repo.upsert_trigger(record).await.expect("insert");
    let worker = worker_with_source_provider(
        repo.clone(),
        Arc::new(NullSourceProvider),
        Arc::new(RecordingMaterializer::success("content:trigger-fire")),
        Arc::new(RecordingSubmitter::with_outcomes(Vec::new())),
        Arc::new(RecordingActiveRunLookup::default()),
    );

    let report = worker.tick_once(fire_slot).await.expect("tick succeeds");

    assert!(matches!(
        report.results.last().map(|result| &result.outcome),
        Some(TriggerPollerFireOutcome::PermanentFailed {
            reason: TriggerPollerFailureReason::SourceNoFire,
        })
    ));
    let persisted = repo
        .get_trigger(tenant("tenant-a"), trigger_id)
        .await
        .expect("load")
        .expect("record present");
    assert_eq!(persisted.last_status, Some(TriggerRunStatus::Error));
    assert_eq!(persisted.next_run_at, expected_next_run_at);
    assert_eq!(persisted.active_fire_slot, None);
    assert_eq!(persisted.active_run_ref, None);
}

#[tokio::test]
async fn tick_source_provider_not_found_persists_permanent_failure_with_next_slot() {
    let repo = Arc::new(InMemoryTriggerRepository::default());
    let trigger_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
    let fire_slot = ts(1_704_067_200);
    let record = sample_record(trigger_id, tenant("tenant-a"), fire_slot);
    let expected_next_run_at = record
        .schedule
        .next_slot_after(fire_slot)
        .expect("next run")
        .expect("future run");
    repo.upsert_trigger(record).await.expect("insert");
    let worker = worker_with_source_provider(
        repo.clone(),
        Arc::new(NotFoundSourceProvider),
        Arc::new(RecordingMaterializer::success("content:trigger-fire")),
        Arc::new(RecordingSubmitter::with_outcomes(Vec::new())),
        Arc::new(RecordingActiveRunLookup::default()),
    );

    let report = worker.tick_once(fire_slot).await.expect("tick succeeds");

    assert!(matches!(
        report.results.last().map(|result| &result.outcome),
        Some(TriggerPollerFireOutcome::PermanentFailed {
            reason: TriggerPollerFailureReason::NotFound,
        })
    ));
    let persisted = repo
        .get_trigger(tenant("tenant-a"), trigger_id)
        .await
        .expect("load")
        .expect("record present");
    assert_eq!(persisted.last_status, Some(TriggerRunStatus::Error));
    assert_eq!(persisted.next_run_at, expected_next_run_at);
    assert_eq!(persisted.active_fire_slot, None);
    assert_eq!(persisted.active_run_ref, None);
}

#[tokio::test]
async fn tick_source_provider_errors_report_bounded_permanent_reasons() {
    let cases = vec![
        (
            TriggerError::InvalidTriggerId {
                reason: "bad trigger".to_string(),
            },
            TriggerPollerFailureReason::InvalidTriggerId,
        ),
        (
            TriggerError::InvalidFireIdentityComponent {
                label: "fire_slot".to_string(),
                reason: "bad component".to_string(),
            },
            TriggerPollerFailureReason::InvalidFireIdentityComponent,
        ),
        (
            TriggerError::InvalidRecord {
                reason: "bad record".to_string(),
            },
            TriggerPollerFailureReason::InvalidRecord,
        ),
        (
            TriggerError::InvalidPollerConfig {
                reason: "bad config".to_string(),
            },
            TriggerPollerFailureReason::InvalidPollerConfig,
        ),
    ];

    for (error, expected_reason) in cases {
        let repo = Arc::new(InMemoryTriggerRepository::default());
        let trigger_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
        let fire_slot = ts(1_704_067_200);
        repo.upsert_trigger(sample_record(trigger_id, tenant("tenant-a"), fire_slot))
            .await
            .expect("insert");
        let worker = worker_with_source_provider(
            repo,
            Arc::new(ErrorSourceProvider::new(error)),
            Arc::new(RecordingMaterializer::success("content:trigger-fire")),
            Arc::new(RecordingSubmitter::with_outcomes(Vec::new())),
            Arc::new(RecordingActiveRunLookup::default()),
        );

        let report = worker.tick_once(fire_slot).await.expect("tick succeeds");

        assert!(matches!(
            report.results.last().map(|result| &result.outcome),
            Some(TriggerPollerFireOutcome::PermanentFailed { reason })
                if *reason == expected_reason
        ));
    }
}

#[tokio::test]
async fn tick_permanent_failure_without_next_slot_completes_trigger() {
    // When a Cron trigger's next_slot_after returns None (schedule exhausted), a
    // permanent pre-submission failure must be treated as Retryable (fail-closed)
    // rather than completing the trigger. This keeps the trigger Scheduled so it
    // can be investigated or removed manually.
    let repo = Arc::new(InMemoryTriggerRepository::default());
    let trigger_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
    // Far-future fire slot whose daily Cron has no valid next slot after it.
    let fire_slot = ymd_hms(9999, 12, 31, 8, 0, 0);
    repo.upsert_trigger(sample_record(trigger_id, tenant("tenant-a"), fire_slot))
        .await
        .expect("insert");
    // Use a source provider that returns None (SourceNoFire): a permanent failure
    // that, because next_slot_after is None, must resolve to Retryable (fail-closed).
    let worker = worker_with_source_provider(
        repo.clone(),
        Arc::new(NullSourceProvider),
        Arc::new(RecordingMaterializer::success("content:trigger-fire")),
        Arc::new(RecordingSubmitter::with_outcomes(Vec::new())),
        Arc::new(RecordingActiveRunLookup::default()),
    );

    let report = worker.tick_once(fire_slot).await.expect("tick succeeds");

    // Fail-closed: permanent failure on an exhausted schedule stays Retryable.
    assert!(
        matches!(
            report.results.last().map(|result| &result.outcome),
            Some(TriggerPollerFireOutcome::RetryableFailed {
                reason: TriggerPollerFailureReason::SourceNoFire,
            })
        ),
        "exhausted-cron permanent failure must be retryable (fail-closed)"
    );
    let persisted = repo
        .get_trigger(tenant("tenant-a"), trigger_id)
        .await
        .expect("load")
        .expect("record present");
    // Trigger remains Scheduled (not Completed) so it can be manually removed.
    assert_eq!(persisted.state, TriggerState::Scheduled);
    assert_eq!(persisted.last_status, Some(TriggerRunStatus::Error));
    assert_eq!(persisted.active_fire_slot, None);
    assert_eq!(persisted.active_run_ref, None);
}

#[tokio::test]
async fn tick_fire_once_trigger_submits_without_terminal_error() {
    // A fire-once trigger whose schedule has no future slot (year-pinned one-shot) must
    // still succeed at submission — process_claimed_fire must NOT call next_run_at_after_fire
    // for CompleteAfterFirstFire triggers.
    let repo = Arc::new(InMemoryTriggerRepository::default());
    let trigger_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
    // Use a specific year-pinned timestamp far in the past: the schedule has no next slot.
    let fire_slot = ymd_hms(2025, 1, 1, 8, 0, 0);
    let mut record = sample_record(trigger_id, tenant("tenant-a"), fire_slot);
    record.schedule = TriggerSchedule::once(fire_slot, "UTC").expect("valid once");
    repo.upsert_trigger(record).await.expect("insert");
    let run_id = TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5a").expect("run id");
    let submitter = Arc::new(RecordingSubmitter::with_outcomes(vec![Ok(
        TrustedTriggerFireSubmitOutcome::Accepted {
            run_id,
            submitted_at: fire_slot,
            turn_scope: test_turn_scope(),
        },
    )]));

    let worker = worker(
        repo.clone(),
        Arc::new(RecordingMaterializer::success("content:fire-once")),
        submitter.clone(),
        Arc::new(RecordingActiveRunLookup::default()),
    );

    let report = worker.tick_once(fire_slot).await.expect("tick succeeds");

    assert_eq!(
        report.results.last().map(|r| &r.outcome),
        Some(&TriggerPollerFireOutcome::Submitted { run_id }),
        "fire-once trigger must submit successfully"
    );
    // The trigger must have an active fire (not yet cleared by the run engine).
    let persisted = repo
        .get_trigger(tenant("tenant-a"), trigger_id)
        .await
        .expect("load")
        .expect("record present");
    assert_eq!(persisted.active_fire_slot, Some(fire_slot));
    assert_eq!(persisted.active_run_ref, Some(run_id));
    assert_eq!(
        persisted.last_status,
        Some(TriggerRunStatus::Ok),
        "status must be Ok after accepted submit"
    );
    // state remains Scheduled until clear_active_fire is called.
    assert_eq!(persisted.state, TriggerState::Scheduled);
}

#[tokio::test]
async fn tick_fire_once_trigger_becomes_completed_after_clear() {
    // After the run engine signals completion, clear_active_fire must move a
    // fire-once trigger from Scheduled to Completed.
    let repo = Arc::new(InMemoryTriggerRepository::default());
    let trigger_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
    let fire_slot = ymd_hms(2025, 1, 1, 8, 0, 0);
    let run_id = TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5a").expect("run id");
    // Start with an already-active fire-once trigger (simulating state after submit).
    let mut record = sample_record(trigger_id, tenant("tenant-a"), fire_slot);
    record.schedule = TriggerSchedule::once(fire_slot, "UTC").expect("valid once");
    record.active_fire_slot = Some(fire_slot);
    record.active_run_ref = Some(run_id);
    repo.upsert_trigger(record).await.expect("insert");
    let active_lookup = Arc::new(RecordingActiveRunLookup::with_state(
        TriggerActiveRunState::Terminal {
            status: TriggerRunHistoryStatus::Ok,
        },
    ));
    let worker = worker(
        repo.clone(),
        Arc::new(RecordingMaterializer::success("content:fire-once")),
        Arc::new(RecordingSubmitter::with_outcomes(Vec::new())),
        active_lookup,
    );

    let report = worker.tick_once(fire_slot).await.expect("tick succeeds");

    assert_eq!(
        report.results.last().map(|r| &r.outcome),
        Some(&TriggerPollerFireOutcome::ClearedTerminalActive { run_id }),
    );
    let persisted = repo
        .get_trigger(tenant("tenant-a"), trigger_id)
        .await
        .expect("load")
        .expect("record present");
    assert_eq!(
        persisted.state,
        TriggerState::Completed,
        "fire-once trigger must be Completed after clear_active_fire"
    );
    assert_eq!(persisted.active_fire_slot, None);
    assert_eq!(persisted.active_run_ref, None);
}

#[tokio::test]
async fn tick_fire_once_blocked_active_run_is_left_pending() {
    // Regression guard: a fire-once trigger parked on a human-interaction gate
    // (Blocked) must NOT be cleared by the poller. The gate is still answerable;
    // clearing it would mark it Completed prematurely (before it ever fired
    // successfully). The active_fire_slot and active_run_ref must be retained so
    // the gate can still be resolved. The outcome must be SkippedAlreadyActive
    // (the pending/skip path), not ClearedBlockedActive.
    let repo = Arc::new(InMemoryTriggerRepository::default());
    let trigger_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
    let fire_slot = ymd_hms(2025, 1, 1, 8, 0, 0);
    let run_id = TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5a").expect("run id");
    let mut record = sample_record(trigger_id, tenant("tenant-a"), fire_slot);
    record.schedule = TriggerSchedule::once(fire_slot, "UTC").expect("valid once");
    record.active_fire_slot = Some(fire_slot);
    record.active_run_ref = Some(run_id);
    repo.upsert_trigger(record).await.expect("insert");
    let worker = worker(
        repo.clone(),
        Arc::new(RecordingMaterializer::success("content:fire-once")),
        Arc::new(RecordingSubmitter::with_outcomes(Vec::new())),
        Arc::new(RecordingActiveRunLookup::with_state(
            TriggerActiveRunState::Blocked,
        )),
    );

    let report = worker.tick_once(fire_slot).await.expect("tick succeeds");

    // The outcome must be the pending/skip path, not ClearedBlockedActive.
    assert!(
        matches!(
            report.results.last().map(|r| &r.outcome),
            Some(TriggerPollerFireOutcome::SkippedAlreadyActive { .. })
        ),
        "fire-once blocked run must produce SkippedAlreadyActive, not ClearedBlockedActive"
    );
    let persisted = repo
        .get_trigger(tenant("tenant-a"), trigger_id)
        .await
        .expect("load")
        .expect("record present");
    // Active lock must be retained so the gate remains answerable.
    assert_eq!(
        persisted.active_fire_slot,
        Some(fire_slot),
        "fire-once blocked run must retain active_fire_slot"
    );
    assert_eq!(
        persisted.active_run_ref,
        Some(run_id),
        "fire-once blocked run must retain active_run_ref"
    );
    // Must NOT be Completed — it never fired successfully.
    assert_ne!(
        persisted.state,
        TriggerState::Completed,
        "fire-once blocked run must not be marked Completed"
    );
}

#[tokio::test]
async fn tick_fire_once_terminal_ok_active_run_clears_to_completed() {
    // Regression guard: a fire-once trigger whose run reaches Terminal with Ok
    // must be cleared and moved to Completed — the normal one-shot completion path.
    let repo = Arc::new(InMemoryTriggerRepository::default());
    let trigger_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZY").expect("ulid");
    let fire_slot = ymd_hms(2025, 1, 1, 8, 0, 0);
    let run_id = TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5b").expect("run id");
    let mut record = sample_record(trigger_id, tenant("tenant-a"), fire_slot);
    record.schedule = TriggerSchedule::once(fire_slot, "UTC").expect("valid once");
    record.active_fire_slot = Some(fire_slot);
    record.active_run_ref = Some(run_id);
    repo.upsert_trigger(record).await.expect("insert");
    let worker = worker(
        repo.clone(),
        Arc::new(RecordingMaterializer::success("content:fire-once")),
        Arc::new(RecordingSubmitter::with_outcomes(Vec::new())),
        Arc::new(RecordingActiveRunLookup::with_state(
            TriggerActiveRunState::Terminal {
                status: TriggerRunHistoryStatus::Ok,
            },
        )),
    );

    let report = worker.tick_once(fire_slot).await.expect("tick succeeds");

    assert_eq!(
        report.results.last().map(|r| &r.outcome),
        Some(&TriggerPollerFireOutcome::ClearedTerminalActive { run_id }),
        "fire-once terminal-ok run must produce ClearedTerminalActive"
    );
    let persisted = repo
        .get_trigger(tenant("tenant-a"), trigger_id)
        .await
        .expect("load")
        .expect("record present");
    assert_eq!(
        persisted.state,
        TriggerState::Completed,
        "fire-once terminal-ok run must be Completed after clear"
    );
    assert_eq!(persisted.active_fire_slot, None);
    assert_eq!(persisted.active_run_ref, None);
    let runs = repo
        .list_trigger_run_history(tenant("tenant-a"), trigger_id, 10)
        .await
        .expect("list run history");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].status, TriggerRunHistoryStatus::Ok);
}

#[tokio::test]
async fn tick_fire_once_terminal_error_active_run_clears_to_completed() {
    // Regression guard: a fire-once trigger whose run reaches Terminal with Error
    // (ran and errored) must also be cleared and moved to Completed — the one-shot
    // is exhausted regardless of the run's own error status.
    let repo = Arc::new(InMemoryTriggerRepository::default());
    let trigger_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZY").expect("ulid");
    let fire_slot = ymd_hms(2025, 1, 1, 8, 0, 0);
    let run_id = TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5c").expect("run id");
    let mut record = sample_record(trigger_id, tenant("tenant-a"), fire_slot);
    record.schedule = TriggerSchedule::once(fire_slot, "UTC").expect("valid once");
    record.active_fire_slot = Some(fire_slot);
    record.active_run_ref = Some(run_id);
    repo.upsert_trigger(record).await.expect("insert");
    let worker = worker(
        repo.clone(),
        Arc::new(RecordingMaterializer::success("content:fire-once")),
        Arc::new(RecordingSubmitter::with_outcomes(Vec::new())),
        Arc::new(RecordingActiveRunLookup::with_state(
            TriggerActiveRunState::Terminal {
                status: TriggerRunHistoryStatus::Error,
            },
        )),
    );

    let report = worker.tick_once(fire_slot).await.expect("tick succeeds");

    assert_eq!(
        report.results.last().map(|r| &r.outcome),
        Some(&TriggerPollerFireOutcome::ClearedTerminalActive { run_id }),
        "fire-once terminal-error run must produce ClearedTerminalActive"
    );
    let persisted = repo
        .get_trigger(tenant("tenant-a"), trigger_id)
        .await
        .expect("load")
        .expect("record present");
    assert_eq!(
        persisted.state,
        TriggerState::Completed,
        "fire-once terminal-error run must be Completed after clear (one-shot exhausted)"
    );
    assert_eq!(persisted.active_fire_slot, None);
    assert_eq!(persisted.active_run_ref, None);
    let runs = repo
        .list_trigger_run_history(tenant("tenant-a"), trigger_id, 10)
        .await
        .expect("list run history");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].status, TriggerRunHistoryStatus::Error);
}

#[tokio::test]
async fn tick_recurring_blocked_active_run_clears_as_failed() {
    // Regression guard: the fire-once exemption in the Blocked arm must NOT affect
    // recurring triggers. A recurring trigger parked on a gate must still be cleared
    // with Error and left Scheduled so its next slot can fire.
    // This is the same scenario as tick_clears_blocked_active_run_and_unblocks_schedule
    // but re-stated here as an explicit policy regression guard for the Recurring branch.
    let repo = Arc::new(InMemoryTriggerRepository::default());
    let trigger_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZX").expect("ulid");
    let fire_slot = ts(1_704_067_200);
    let run_id = TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5d").expect("run id");
    let mut record = sample_record(trigger_id, tenant("tenant-a"), ts(1_704_067_260));
    assert!(matches!(record.schedule, TriggerSchedule::Cron { .. }));
    record.active_fire_slot = Some(fire_slot);
    record.active_run_ref = Some(run_id);
    repo.upsert_trigger(record).await.expect("insert");
    let worker = worker(
        repo.clone(),
        Arc::new(RecordingMaterializer::success("content:trigger-fire")),
        Arc::new(RecordingSubmitter::with_outcomes(Vec::new())),
        Arc::new(RecordingActiveRunLookup::with_state(
            TriggerActiveRunState::Blocked,
        )),
    );

    let report = worker.tick_once(fire_slot).await.expect("tick succeeds");

    assert_eq!(
        report.results.last().map(|r| &r.outcome),
        Some(&TriggerPollerFireOutcome::ClearedBlockedActive { run_id }),
        "recurring blocked run must produce ClearedBlockedActive"
    );
    let persisted = repo
        .get_trigger(tenant("tenant-a"), trigger_id)
        .await
        .expect("load")
        .expect("record present");
    assert_eq!(
        persisted.state,
        TriggerState::Scheduled,
        "recurring blocked run must stay Scheduled after clear so the next slot can fire"
    );
    assert_eq!(persisted.active_fire_slot, None);
    assert_eq!(persisted.active_run_ref, None);
    let runs = repo
        .list_trigger_run_history(tenant("tenant-a"), trigger_id, 10)
        .await
        .expect("list run history");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].run_id, Some(run_id));
    assert_eq!(runs[0].status, TriggerRunHistoryStatus::Error);
}

#[tokio::test]
async fn tick_fire_once_permanent_submit_failure_stays_scheduled_for_retry() {
    // A CompleteAfterFirstFire trigger whose submitter returns a Permanent
    // failure (e.g. unpaired actor, bad materialization) has NOT fired — the
    // one-shot must remain Scheduled so it can retry once the cause is fixed.
    // It must NOT be marked Completed; that only happens via clear_active_fire
    // after a real run terminates.
    let repo = Arc::new(InMemoryTriggerRepository::default());
    let trigger_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
    let fire_slot = ts(1_704_067_200);
    let mut record = sample_record(trigger_id, tenant("tenant-a"), fire_slot);
    record.schedule = TriggerSchedule::once(fire_slot, "UTC").expect("valid once");
    let original_next_run_at = record.next_run_at;
    repo.upsert_trigger(record).await.expect("insert");
    let worker = worker(
        repo.clone(),
        Arc::new(RecordingMaterializer::success("content:fire-once")),
        Arc::new(RecordingSubmitter::with_outcomes(vec![Err(
            TriggerError::InvalidMaterialization {
                reason: "fire-once permanent failure".to_string(),
            },
        )])),
        Arc::new(RecordingActiveRunLookup::default()),
    );

    let report = worker.tick_once(fire_slot).await.expect("tick succeeds");

    // Permanent pre-submission failure for a fire-once triggers Retryable (fail-closed).
    assert!(
        matches!(
            report.results.last().map(|r| &r.outcome),
            Some(TriggerPollerFireOutcome::RetryableFailed { .. })
        ),
        "fire-once permanent submit failure must produce RetryableFailed (fail-closed, not PermanentFailed)"
    );
    let persisted = repo
        .get_trigger(tenant("tenant-a"), trigger_id)
        .await
        .expect("load")
        .expect("record present");
    assert_ne!(
        persisted.state,
        TriggerState::Completed,
        "fire-once that never fired must NOT be Completed — it must remain Scheduled"
    );
    assert_eq!(
        persisted.state,
        TriggerState::Scheduled,
        "fire-once permanent submit failure must leave trigger Scheduled"
    );
    assert_eq!(
        persisted.next_run_at, original_next_run_at,
        "fire-once trigger must NOT advance next_run_at on permanent failure"
    );
    assert_eq!(persisted.active_fire_slot, None);
    assert_eq!(persisted.active_run_ref, None);
}

#[tokio::test]
async fn tick_recurring_trigger_reschedules_unchanged() {
    // Regression guard: the fire-once path must not affect recurring trigger behavior.
    // A recurring trigger must still advance next_run_at after a successful fire.
    let repo = Arc::new(InMemoryTriggerRepository::default());
    let trigger_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
    let fire_slot = ts(1_704_067_200);
    let record = sample_record(trigger_id, tenant("tenant-a"), fire_slot);
    assert!(matches!(record.schedule, TriggerSchedule::Cron { .. }));
    let expected_next_run_at = record
        .schedule
        .next_slot_after(fire_slot)
        .expect("next run")
        .expect("future run");
    repo.upsert_trigger(record).await.expect("insert");
    let run_id = TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5a").expect("run id");
    let worker = worker(
        repo.clone(),
        Arc::new(RecordingMaterializer::success("content:trigger-fire")),
        Arc::new(RecordingSubmitter::with_outcomes(vec![Ok(
            TrustedTriggerFireSubmitOutcome::Accepted {
                run_id,
                submitted_at: ts(1_704_067_205),
                turn_scope: test_turn_scope(),
            },
        )])),
        Arc::new(RecordingActiveRunLookup::default()),
    );

    let report = worker.tick_once(fire_slot).await.expect("tick succeeds");

    assert_eq!(
        report.results.last().map(|r| &r.outcome),
        Some(&TriggerPollerFireOutcome::Submitted { run_id })
    );
    let persisted = repo
        .get_trigger(tenant("tenant-a"), trigger_id)
        .await
        .expect("load")
        .expect("record present");
    assert_eq!(
        persisted.next_run_at, expected_next_run_at,
        "recurring trigger must advance to the next scheduled slot"
    );
    assert_eq!(persisted.state, TriggerState::Scheduled);
}

struct RecordingMaterializer {
    result: Mutex<Option<Result<TriggerInboundContentRef, TriggerError>>>,
    fires: Mutex<Vec<TriggerFire>>,
}

struct NullSourceProvider;

struct NotFoundSourceProvider;

struct ErrorSourceProvider {
    error: Mutex<Option<TriggerError>>,
}

impl ErrorSourceProvider {
    fn new(error: TriggerError) -> Self {
        Self {
            error: Mutex::new(Some(error)),
        }
    }
}

#[async_trait]
impl TriggerSourceProvider for NullSourceProvider {
    async fn evaluate(
        &self,
        _record: &TriggerRecord,
        _now: Timestamp,
    ) -> Result<Option<TriggerFire>, TriggerError> {
        Ok(None)
    }
}

#[async_trait]
impl TriggerSourceProvider for NotFoundSourceProvider {
    async fn evaluate(
        &self,
        _record: &TriggerRecord,
        _now: Timestamp,
    ) -> Result<Option<TriggerFire>, TriggerError> {
        Err(TriggerError::NotFound)
    }
}

#[async_trait]
impl TriggerSourceProvider for ErrorSourceProvider {
    async fn evaluate(
        &self,
        _record: &TriggerRecord,
        _now: Timestamp,
    ) -> Result<Option<TriggerFire>, TriggerError> {
        Err(self
            .error
            .lock()
            .expect("error lock")
            .take()
            .expect("source provider error configured"))
    }
}

impl RecordingMaterializer {
    fn success(content_ref: &str) -> Self {
        Self {
            result: Mutex::new(Some(
                Ok(TriggerInboundContentRef::new(content_ref).unwrap()),
            )),
            fires: Mutex::new(Vec::new()),
        }
    }

    fn failure(error: TriggerError) -> Self {
        Self {
            result: Mutex::new(Some(Err(error))),
            fires: Mutex::new(Vec::new()),
        }
    }

    fn fires(&self) -> Vec<TriggerFire> {
        self.fires.lock().expect("fires lock").clone()
    }
}

#[async_trait]
impl TriggerPromptMaterializer for RecordingMaterializer {
    async fn materialize_prompt(
        &self,
        fire: TriggerFire,
    ) -> Result<TriggerMaterializedPrompt, TriggerError> {
        self.fires.lock().expect("fires lock").push(fire.clone());
        let content_ref = self
            .result
            .lock()
            .expect("result lock")
            .take()
            .expect("materializer result configured")?;
        Ok(TriggerMaterializedPrompt::for_fire(&fire, content_ref))
    }
}

struct RecordingSubmitter {
    outcomes: Mutex<Vec<Result<TrustedTriggerFireSubmitOutcome, TriggerError>>>,
    requests: Mutex<Vec<TrustedTriggerSubmitRequest>>,
}

impl RecordingSubmitter {
    fn with_outcomes(outcomes: Vec<Result<TrustedTriggerFireSubmitOutcome, TriggerError>>) -> Self {
        Self {
            outcomes: Mutex::new(outcomes.into_iter().rev().collect()),
            requests: Mutex::new(Vec::new()),
        }
    }

    fn requests(&self) -> Vec<TrustedTriggerSubmitRequest> {
        self.requests.lock().expect("requests lock").clone()
    }
}

#[async_trait]
impl TrustedTriggerFireSubmitter for RecordingSubmitter {
    async fn submit_trusted_trigger_fire(
        &self,
        request: TrustedTriggerSubmitRequest,
    ) -> Result<TrustedTriggerFireSubmitOutcome, TriggerError> {
        self.requests.lock().expect("requests lock").push(request);
        self.outcomes
            .lock()
            .expect("outcomes lock")
            .pop()
            .expect("submit outcome configured")
    }
}

#[derive(Default)]
struct RecordingActiveRunLookup {
    results: Mutex<Vec<Result<TriggerActiveRunState, TriggerError>>>,
    requests: Mutex<Vec<TriggerActiveRunStateRequest>>,
}

impl RecordingActiveRunLookup {
    fn with_state(state: TriggerActiveRunState) -> Self {
        Self::with_results(vec![Ok(state)])
    }

    fn with_results(results: Vec<Result<TriggerActiveRunState, TriggerError>>) -> Self {
        Self {
            results: Mutex::new(results.into_iter().rev().collect()),
            requests: Mutex::new(Vec::new()),
        }
    }

    fn requests(&self) -> Vec<TriggerActiveRunStateRequest> {
        self.requests.lock().expect("requests lock").clone()
    }
}

#[async_trait]
impl TriggerActiveRunLookup for RecordingActiveRunLookup {
    async fn active_run_state(
        &self,
        request: TriggerActiveRunStateRequest,
    ) -> Result<TriggerActiveRunState, TriggerError> {
        self.requests.lock().expect("requests lock").push(request);
        self.results.lock().expect("results lock").pop().expect(
            "RecordingActiveRunLookup: more active_run_state calls than configured outcomes",
        )
    }
}

#[derive(Default)]
struct TickConcurrencyRepository {
    current_due_scans: Mutex<usize>,
    max_concurrent_due_scans: Mutex<usize>,
}

impl TickConcurrencyRepository {
    fn max_concurrent_due_scans(&self) -> usize {
        *self
            .max_concurrent_due_scans
            .lock()
            .expect("max concurrent due scans lock")
    }
}

#[async_trait]
impl TriggerRepository for TickConcurrencyRepository {
    async fn find_trigger_run_by_thread_id(
        &self,
        _tenant_id: TenantId,
        _thread_id: &ThreadId,
    ) -> Result<Option<(TriggerRecord, TriggerRunRecord)>, TriggerError> {
        // Trigger-thread lookup is not exercised by this fake.
        Ok(None)
    }

    async fn upsert_trigger(&self, _record: TriggerRecord) -> Result<(), TriggerError> {
        unreachable!("tick-concurrency repository is read-only")
    }

    async fn get_trigger(
        &self,
        _tenant_id: TenantId,
        _trigger_id: TriggerId,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        unreachable!("tick-concurrency repository does not load records")
    }

    async fn list_triggers(
        &self,
        _tenant_id: TenantId,
    ) -> Result<Vec<TriggerRecord>, TriggerError> {
        unreachable!("tick-concurrency repository does not list tenant records")
    }

    async fn list_scoped_triggers(
        &self,
        _tenant_id: TenantId,
        _creator_user_id: UserId,
        _agent_id: Option<AgentId>,
        _project_id: Option<ProjectId>,
        _limit: usize,
        _excluded_states: &[TriggerState],
    ) -> Result<Vec<TriggerRecord>, TriggerError> {
        unreachable!("tick-concurrency repository does not list scoped records")
    }

    async fn remove_trigger(
        &self,
        _tenant_id: TenantId,
        _trigger_id: TriggerId,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        unreachable!("tick-concurrency repository does not remove records")
    }

    async fn remove_scoped_trigger(
        &self,
        _tenant_id: TenantId,
        _creator_user_id: UserId,
        _agent_id: Option<AgentId>,
        _project_id: Option<ProjectId>,
        _trigger_id: TriggerId,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        unreachable!("test repository does not remove scoped records")
    }

    async fn list_due_triggers(
        &self,
        _now: Timestamp,
        _limit: usize,
    ) -> Result<Vec<TriggerRecord>, TriggerError> {
        {
            let mut current = self
                .current_due_scans
                .lock()
                .expect("current due scans lock");
            *current += 1;
            let mut max = self
                .max_concurrent_due_scans
                .lock()
                .expect("max concurrent due scans lock");
            *max = (*max).max(*current);
        }
        tokio::task::yield_now().await;
        *self
            .current_due_scans
            .lock()
            .expect("current due scans lock") -= 1;
        Ok(Vec::new())
    }

    async fn list_active_triggers(&self, limit: usize) -> Result<Vec<TriggerRecord>, TriggerError> {
        self.list_active_triggers_after(None, limit).await
    }

    async fn list_active_triggers_after(
        &self,
        _after: Option<ActiveTriggerScanCursor>,
        _limit: usize,
    ) -> Result<Vec<TriggerRecord>, TriggerError> {
        Ok(Vec::new())
    }

    async fn claim_due_fire(
        &self,
        _request: ClaimDueFireRequest,
    ) -> Result<ClaimDueFireOutcome, TriggerError> {
        unreachable!("tick-concurrency repository should not claim fires")
    }

    async fn mark_fire_accepted(
        &self,
        _request: FireAcceptedRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        unreachable!("tick-concurrency repository should not persist accepted fires")
    }

    async fn mark_fire_replayed(
        &self,
        _request: FireReplayedRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        unreachable!("tick-concurrency repository should not persist replayed fires")
    }

    async fn mark_fire_retryable_failed(
        &self,
        _request: FireRetryableFailedRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        unreachable!("tick-concurrency repository should not persist retryable failures")
    }

    async fn mark_fire_permanently_failed(
        &self,
        _request: FirePermanentFailedRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        unreachable!("tick-concurrency repository should not persist permanent failures")
    }

    async fn mark_fire_terminally_failed(
        &self,
        _request: FireTerminalFailedRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        unreachable!("tick-concurrency repository should not persist terminal failures")
    }

    async fn clear_active_fire(
        &self,
        _request: ClearActiveFireRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        unreachable!("tick-concurrency repository should not clear active fires")
    }
}

struct ActiveListErrorRepository;

#[async_trait]
impl TriggerRepository for ActiveListErrorRepository {
    async fn find_trigger_run_by_thread_id(
        &self,
        _tenant_id: TenantId,
        _thread_id: &ThreadId,
    ) -> Result<Option<(TriggerRecord, TriggerRunRecord)>, TriggerError> {
        // Trigger-thread lookup is not exercised by this fake.
        Ok(None)
    }

    async fn upsert_trigger(&self, _record: TriggerRecord) -> Result<(), TriggerError> {
        unreachable!("active-list-error repository is read-only")
    }

    async fn get_trigger(
        &self,
        _tenant_id: TenantId,
        _trigger_id: TriggerId,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        unreachable!("active-list-error repository does not load records")
    }

    async fn list_triggers(
        &self,
        _tenant_id: TenantId,
    ) -> Result<Vec<TriggerRecord>, TriggerError> {
        unreachable!("active-list-error repository does not list tenant records")
    }

    async fn list_scoped_triggers(
        &self,
        _tenant_id: TenantId,
        _creator_user_id: UserId,
        _agent_id: Option<AgentId>,
        _project_id: Option<ProjectId>,
        _limit: usize,
        _excluded_states: &[TriggerState],
    ) -> Result<Vec<TriggerRecord>, TriggerError> {
        unreachable!("active-list-error repository does not list scoped records")
    }

    async fn remove_trigger(
        &self,
        _tenant_id: TenantId,
        _trigger_id: TriggerId,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        unreachable!("active-list-error repository does not remove records")
    }

    async fn remove_scoped_trigger(
        &self,
        _tenant_id: TenantId,
        _creator_user_id: UserId,
        _agent_id: Option<AgentId>,
        _project_id: Option<ProjectId>,
        _trigger_id: TriggerId,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        unreachable!("test repository does not remove scoped records")
    }

    async fn list_due_triggers(
        &self,
        _now: Timestamp,
        _limit: usize,
    ) -> Result<Vec<TriggerRecord>, TriggerError> {
        unreachable!("active-list-error should abort before due scan")
    }

    async fn list_active_triggers(&self, limit: usize) -> Result<Vec<TriggerRecord>, TriggerError> {
        self.list_active_triggers_after(None, limit).await
    }

    async fn list_active_triggers_after(
        &self,
        _after: Option<ActiveTriggerScanCursor>,
        _limit: usize,
    ) -> Result<Vec<TriggerRecord>, TriggerError> {
        Err(TriggerError::Backend {
            reason: "active list unavailable".to_string(),
        })
    }

    async fn claim_due_fire(
        &self,
        _request: ClaimDueFireRequest,
    ) -> Result<ClaimDueFireOutcome, TriggerError> {
        unreachable!("active-list-error repository should not claim fires")
    }

    async fn mark_fire_accepted(
        &self,
        _request: FireAcceptedRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        unreachable!("active-list-error repository should not persist accepted fires")
    }

    async fn mark_fire_replayed(
        &self,
        _request: FireReplayedRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        unreachable!("active-list-error repository should not persist replayed fires")
    }

    async fn mark_fire_retryable_failed(
        &self,
        _request: FireRetryableFailedRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        unreachable!("active-list-error repository should not persist retryable failures")
    }

    async fn mark_fire_permanently_failed(
        &self,
        _request: FirePermanentFailedRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        unreachable!("active-list-error repository should not persist permanent failures")
    }

    async fn mark_fire_terminally_failed(
        &self,
        _request: FireTerminalFailedRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        unreachable!("active-list-error repository should not persist terminal failures")
    }

    async fn clear_active_fire(
        &self,
        _request: ClearActiveFireRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        unreachable!("active-list-error repository should not clear active fires")
    }
}

struct ActiveWrapRefetchErrorRepository {
    record: TriggerRecord,
    active_scan_calls: Mutex<Vec<bool>>,
}

impl ActiveWrapRefetchErrorRepository {
    fn new(record: TriggerRecord) -> Self {
        Self {
            record,
            active_scan_calls: Mutex::new(Vec::new()),
        }
    }

    fn active_scan_call_shapes(&self) -> Vec<bool> {
        self.active_scan_calls
            .lock()
            .expect("active scan calls lock")
            .clone()
    }
}

#[async_trait]
impl TriggerRepository for ActiveWrapRefetchErrorRepository {
    async fn find_trigger_run_by_thread_id(
        &self,
        _tenant_id: TenantId,
        _thread_id: &ThreadId,
    ) -> Result<Option<(TriggerRecord, TriggerRunRecord)>, TriggerError> {
        // Trigger-thread lookup is not exercised by this fake.
        Ok(None)
    }

    async fn upsert_trigger(&self, _record: TriggerRecord) -> Result<(), TriggerError> {
        unreachable!("active-wrap-refetch-error repository is read-only")
    }

    async fn get_trigger(
        &self,
        _tenant_id: TenantId,
        _trigger_id: TriggerId,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        unreachable!("active-wrap-refetch-error repository does not load records")
    }

    async fn list_triggers(
        &self,
        _tenant_id: TenantId,
    ) -> Result<Vec<TriggerRecord>, TriggerError> {
        unreachable!("active-wrap-refetch-error repository does not list tenant records")
    }

    async fn list_scoped_triggers(
        &self,
        _tenant_id: TenantId,
        _creator_user_id: UserId,
        _agent_id: Option<AgentId>,
        _project_id: Option<ProjectId>,
        _limit: usize,
        _excluded_states: &[TriggerState],
    ) -> Result<Vec<TriggerRecord>, TriggerError> {
        unreachable!("active-wrap-refetch-error repository does not list scoped records")
    }

    async fn remove_trigger(
        &self,
        _tenant_id: TenantId,
        _trigger_id: TriggerId,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        unreachable!("active-wrap-refetch-error repository does not remove records")
    }

    async fn remove_scoped_trigger(
        &self,
        _tenant_id: TenantId,
        _creator_user_id: UserId,
        _agent_id: Option<AgentId>,
        _project_id: Option<ProjectId>,
        _trigger_id: TriggerId,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        unreachable!("test repository does not remove scoped records")
    }

    async fn list_due_triggers(
        &self,
        _now: Timestamp,
        _limit: usize,
    ) -> Result<Vec<TriggerRecord>, TriggerError> {
        Ok(Vec::new())
    }

    async fn list_active_triggers(&self, limit: usize) -> Result<Vec<TriggerRecord>, TriggerError> {
        self.list_active_triggers_after(None, limit).await
    }

    async fn list_active_triggers_after(
        &self,
        after: Option<ActiveTriggerScanCursor>,
        _limit: usize,
    ) -> Result<Vec<TriggerRecord>, TriggerError> {
        let mut calls = self
            .active_scan_calls
            .lock()
            .expect("active scan calls lock");
        calls.push(after.is_some());
        match calls.len() {
            1 => Ok(vec![self.record.clone()]),
            2 => Ok(Vec::new()),
            3 => Err(TriggerError::Backend {
                reason: "wrap refetch unavailable".to_string(),
            }),
            _ => unreachable!("unexpected active scan call"),
        }
    }

    async fn claim_due_fire(
        &self,
        _request: ClaimDueFireRequest,
    ) -> Result<ClaimDueFireOutcome, TriggerError> {
        unreachable!("active-wrap-refetch-error repository should not claim fires")
    }

    async fn mark_fire_accepted(
        &self,
        _request: FireAcceptedRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        unreachable!("active-wrap-refetch-error repository should not persist accepted fires")
    }

    async fn mark_fire_replayed(
        &self,
        _request: FireReplayedRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        unreachable!("active-wrap-refetch-error repository should not persist replayed fires")
    }

    async fn mark_fire_retryable_failed(
        &self,
        _request: FireRetryableFailedRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        unreachable!("active-wrap-refetch-error repository should not persist retryable failures")
    }

    async fn mark_fire_permanently_failed(
        &self,
        _request: FirePermanentFailedRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        unreachable!("active-wrap-refetch-error repository should not persist permanent failures")
    }

    async fn mark_fire_terminally_failed(
        &self,
        _request: FireTerminalFailedRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        unreachable!("active-wrap-refetch-error repository should not persist terminal failures")
    }

    async fn clear_active_fire(
        &self,
        _request: ClearActiveFireRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        unreachable!("active-wrap-refetch-error repository should not clear active fires")
    }
}

struct ActiveClearRaceRepository {
    active_record: TriggerRecord,
}

#[async_trait]
impl TriggerRepository for ActiveClearRaceRepository {
    async fn find_trigger_run_by_thread_id(
        &self,
        _tenant_id: TenantId,
        _thread_id: &ThreadId,
    ) -> Result<Option<(TriggerRecord, TriggerRunRecord)>, TriggerError> {
        // Trigger-thread lookup is not exercised by this fake.
        Ok(None)
    }

    async fn upsert_trigger(&self, _record: TriggerRecord) -> Result<(), TriggerError> {
        unreachable!("active-clear-race repository is read-only")
    }

    async fn get_trigger(
        &self,
        _tenant_id: TenantId,
        _trigger_id: TriggerId,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        unreachable!("active-clear-race repository does not load records")
    }

    async fn list_triggers(
        &self,
        _tenant_id: TenantId,
    ) -> Result<Vec<TriggerRecord>, TriggerError> {
        unreachable!("active-clear-race repository does not list tenant records")
    }

    async fn list_scoped_triggers(
        &self,
        _tenant_id: TenantId,
        _creator_user_id: UserId,
        _agent_id: Option<AgentId>,
        _project_id: Option<ProjectId>,
        _limit: usize,
        _excluded_states: &[TriggerState],
    ) -> Result<Vec<TriggerRecord>, TriggerError> {
        unreachable!("active-clear-race repository does not list scoped records")
    }

    async fn remove_trigger(
        &self,
        _tenant_id: TenantId,
        _trigger_id: TriggerId,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        unreachable!("active-clear-race repository does not remove records")
    }

    async fn remove_scoped_trigger(
        &self,
        _tenant_id: TenantId,
        _creator_user_id: UserId,
        _agent_id: Option<AgentId>,
        _project_id: Option<ProjectId>,
        _trigger_id: TriggerId,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        unreachable!("test repository does not remove scoped records")
    }

    async fn list_due_triggers(
        &self,
        _now: Timestamp,
        _limit: usize,
    ) -> Result<Vec<TriggerRecord>, TriggerError> {
        Ok(Vec::new())
    }

    async fn list_active_triggers(&self, limit: usize) -> Result<Vec<TriggerRecord>, TriggerError> {
        self.list_active_triggers_after(None, limit).await
    }

    async fn list_active_triggers_after(
        &self,
        after: Option<ActiveTriggerScanCursor>,
        _limit: usize,
    ) -> Result<Vec<TriggerRecord>, TriggerError> {
        if after.is_some() {
            return Ok(Vec::new());
        }
        Ok(vec![self.active_record.clone()])
    }

    async fn claim_due_fire(
        &self,
        _request: ClaimDueFireRequest,
    ) -> Result<ClaimDueFireOutcome, TriggerError> {
        unreachable!("active-clear-race repository should not claim fires")
    }

    async fn mark_fire_accepted(
        &self,
        _request: FireAcceptedRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        unreachable!("active-clear-race repository should not persist accepted fires")
    }

    async fn mark_fire_replayed(
        &self,
        _request: FireReplayedRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        unreachable!("active-clear-race repository should not persist replayed fires")
    }

    async fn mark_fire_retryable_failed(
        &self,
        _request: FireRetryableFailedRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        unreachable!("active-clear-race repository should not persist retryable failures")
    }

    async fn mark_fire_permanently_failed(
        &self,
        _request: FirePermanentFailedRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        unreachable!("active-clear-race repository should not persist permanent failures")
    }

    async fn mark_fire_terminally_failed(
        &self,
        _request: FireTerminalFailedRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        unreachable!("active-clear-race repository should not persist terminal failures")
    }

    async fn clear_active_fire(
        &self,
        _request: ClearActiveFireRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        Ok(None)
    }
}

struct ActiveClearFailsOnceRepository {
    records: Mutex<Vec<TriggerRecord>>,
    clear_requests: Mutex<Vec<TriggerId>>,
    fail_once_trigger_id: TriggerId,
    failed_once: Mutex<bool>,
}

impl ActiveClearFailsOnceRepository {
    fn new(records: Vec<TriggerRecord>, fail_once_trigger_id: TriggerId) -> Self {
        Self {
            records: Mutex::new(records),
            clear_requests: Mutex::new(Vec::new()),
            fail_once_trigger_id,
            failed_once: Mutex::new(false),
        }
    }

    fn clear_requests(&self) -> Vec<TriggerId> {
        self.clear_requests
            .lock()
            .expect("clear requests lock")
            .clone()
    }
}

#[async_trait]
impl TriggerRepository for ActiveClearFailsOnceRepository {
    async fn find_trigger_run_by_thread_id(
        &self,
        _tenant_id: TenantId,
        _thread_id: &ThreadId,
    ) -> Result<Option<(TriggerRecord, TriggerRunRecord)>, TriggerError> {
        // Trigger-thread lookup is not exercised by this fake.
        Ok(None)
    }

    async fn upsert_trigger(&self, _record: TriggerRecord) -> Result<(), TriggerError> {
        unreachable!("active-clear-fails-once repository is read-only")
    }

    async fn get_trigger(
        &self,
        _tenant_id: TenantId,
        _trigger_id: TriggerId,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        unreachable!("active-clear-fails-once repository does not load records")
    }

    async fn list_triggers(
        &self,
        _tenant_id: TenantId,
    ) -> Result<Vec<TriggerRecord>, TriggerError> {
        unreachable!("active-clear-fails-once repository does not list tenant records")
    }

    async fn list_scoped_triggers(
        &self,
        _tenant_id: TenantId,
        _creator_user_id: UserId,
        _agent_id: Option<AgentId>,
        _project_id: Option<ProjectId>,
        _limit: usize,
        _excluded_states: &[TriggerState],
    ) -> Result<Vec<TriggerRecord>, TriggerError> {
        unreachable!("active-clear-fails-once repository does not list scoped records")
    }

    async fn remove_trigger(
        &self,
        _tenant_id: TenantId,
        _trigger_id: TriggerId,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        unreachable!("active-clear-fails-once repository does not remove records")
    }

    async fn remove_scoped_trigger(
        &self,
        _tenant_id: TenantId,
        _creator_user_id: UserId,
        _agent_id: Option<AgentId>,
        _project_id: Option<ProjectId>,
        _trigger_id: TriggerId,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        unreachable!("test repository does not remove scoped records")
    }

    async fn list_due_triggers(
        &self,
        _now: Timestamp,
        _limit: usize,
    ) -> Result<Vec<TriggerRecord>, TriggerError> {
        Ok(Vec::new())
    }

    async fn list_active_triggers(&self, limit: usize) -> Result<Vec<TriggerRecord>, TriggerError> {
        self.list_active_triggers_after(None, limit).await
    }

    async fn list_active_triggers_after(
        &self,
        after: Option<ActiveTriggerScanCursor>,
        limit: usize,
    ) -> Result<Vec<TriggerRecord>, TriggerError> {
        let mut records = self
            .records
            .lock()
            .expect("active records lock")
            .iter()
            .filter_map(|record| {
                let active_fire_slot = record.active_fire_slot?;
                Some((
                    active_fire_slot,
                    record.tenant_id.clone(),
                    record.trigger_id,
                    record.clone(),
                ))
            })
            .filter(
                |(active_fire_slot, tenant_id, trigger_id, _)| match after.as_ref() {
                    Some(cursor) => {
                        (*active_fire_slot, tenant_id, *trigger_id)
                            > (
                                cursor.active_fire_slot(),
                                cursor.tenant_id(),
                                cursor.trigger_id(),
                            )
                    }
                    None => true,
                },
            )
            .collect::<Vec<_>>();
        records.sort_by_key(|(active_fire_slot, tenant_id, trigger_id, _)| {
            (*active_fire_slot, tenant_id.clone(), *trigger_id)
        });
        records.truncate(limit);
        Ok(records
            .into_iter()
            .map(|(_, _, _, record)| record)
            .collect())
    }

    async fn claim_due_fire(
        &self,
        _request: ClaimDueFireRequest,
    ) -> Result<ClaimDueFireOutcome, TriggerError> {
        unreachable!("active-clear-fails-once repository should not claim fires")
    }

    async fn mark_fire_accepted(
        &self,
        _request: FireAcceptedRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        unreachable!("active-clear-fails-once repository should not persist accepted fires")
    }

    async fn mark_fire_replayed(
        &self,
        _request: FireReplayedRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        unreachable!("active-clear-fails-once repository should not persist replayed fires")
    }

    async fn mark_fire_retryable_failed(
        &self,
        _request: FireRetryableFailedRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        unreachable!("active-clear-fails-once repository should not persist retryable failures")
    }

    async fn mark_fire_permanently_failed(
        &self,
        _request: FirePermanentFailedRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        unreachable!("active-clear-fails-once repository should not persist permanent failures")
    }

    async fn mark_fire_terminally_failed(
        &self,
        _request: FireTerminalFailedRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        unreachable!("active-clear-fails-once repository should not persist terminal failures")
    }

    async fn clear_active_fire(
        &self,
        request: ClearActiveFireRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        self.clear_requests
            .lock()
            .expect("clear requests lock")
            .push(request.trigger_id);
        if request.trigger_id == self.fail_once_trigger_id {
            let mut failed_once = self.failed_once.lock().expect("failed-once lock");
            if !*failed_once {
                *failed_once = true;
                return Err(TriggerError::Backend {
                    reason: "clear failed once".to_string(),
                });
            }
        }

        let mut records = self.records.lock().expect("active records lock");
        let Some(record) = records.iter_mut().find(|record| {
            record.tenant_id == request.tenant_id && record.trigger_id == request.trigger_id
        }) else {
            return Ok(None);
        };
        let updated = record.clone();
        record.active_fire_slot = None;
        record.active_run_ref = None;
        Ok(Some(updated))
    }
}

struct AcceptedMissingRepository {
    claimed_record: TriggerRecord,
    fire_slot: Timestamp,
}

#[async_trait]
impl TriggerRepository for AcceptedMissingRepository {
    async fn find_trigger_run_by_thread_id(
        &self,
        _tenant_id: TenantId,
        _thread_id: &ThreadId,
    ) -> Result<Option<(TriggerRecord, TriggerRunRecord)>, TriggerError> {
        // Trigger-thread lookup is not exercised by this fake.
        Ok(None)
    }

    async fn upsert_trigger(&self, _record: TriggerRecord) -> Result<(), TriggerError> {
        unreachable!("accepted-missing repository is read-only")
    }

    async fn get_trigger(
        &self,
        _tenant_id: TenantId,
        _trigger_id: TriggerId,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        unreachable!("accepted-missing repository does not load records")
    }

    async fn list_triggers(
        &self,
        _tenant_id: TenantId,
    ) -> Result<Vec<TriggerRecord>, TriggerError> {
        unreachable!("accepted-missing repository does not list tenant records")
    }

    async fn list_scoped_triggers(
        &self,
        _tenant_id: TenantId,
        _creator_user_id: UserId,
        _agent_id: Option<AgentId>,
        _project_id: Option<ProjectId>,
        _limit: usize,
        _excluded_states: &[TriggerState],
    ) -> Result<Vec<TriggerRecord>, TriggerError> {
        unreachable!("accepted-missing repository does not list scoped records")
    }

    async fn remove_trigger(
        &self,
        _tenant_id: TenantId,
        _trigger_id: TriggerId,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        unreachable!("accepted-missing repository does not remove records")
    }

    async fn remove_scoped_trigger(
        &self,
        _tenant_id: TenantId,
        _creator_user_id: UserId,
        _agent_id: Option<AgentId>,
        _project_id: Option<ProjectId>,
        _trigger_id: TriggerId,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        unreachable!("test repository does not remove scoped records")
    }

    async fn list_due_triggers(
        &self,
        _now: Timestamp,
        _limit: usize,
    ) -> Result<Vec<TriggerRecord>, TriggerError> {
        Ok(vec![self.claimed_record.clone()])
    }

    async fn list_active_triggers(&self, limit: usize) -> Result<Vec<TriggerRecord>, TriggerError> {
        self.list_active_triggers_after(None, limit).await
    }

    async fn list_active_triggers_after(
        &self,
        _after: Option<ActiveTriggerScanCursor>,
        _limit: usize,
    ) -> Result<Vec<TriggerRecord>, TriggerError> {
        Ok(Vec::new())
    }

    async fn claim_due_fire(
        &self,
        _request: ClaimDueFireRequest,
    ) -> Result<ClaimDueFireOutcome, TriggerError> {
        Ok(ClaimDueFireOutcome::Claimed(ClaimedTriggerFire {
            record: self.claimed_record.clone(),
            fire_slot: self.fire_slot,
        }))
    }

    async fn mark_fire_accepted(
        &self,
        _request: FireAcceptedRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        Ok(None)
    }

    async fn mark_fire_replayed(
        &self,
        _request: FireReplayedRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        unreachable!("accepted-missing repository should not persist replayed fires")
    }

    async fn mark_fire_retryable_failed(
        &self,
        _request: FireRetryableFailedRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        unreachable!("accepted-missing repository should not persist retryable failures")
    }

    async fn mark_fire_permanently_failed(
        &self,
        _request: FirePermanentFailedRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        unreachable!("accepted-missing repository should not persist permanent failures")
    }

    async fn mark_fire_terminally_failed(
        &self,
        _request: FireTerminalFailedRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        unreachable!("accepted-missing repository should not persist terminal failures")
    }

    async fn clear_active_fire(
        &self,
        _request: ClearActiveFireRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        unreachable!("accepted-missing repository should not clear active fires")
    }
}

struct ReplayedMissingRepository {
    claimed_record: TriggerRecord,
    fire_slot: Timestamp,
}

#[async_trait]
impl TriggerRepository for ReplayedMissingRepository {
    async fn find_trigger_run_by_thread_id(
        &self,
        _tenant_id: TenantId,
        _thread_id: &ThreadId,
    ) -> Result<Option<(TriggerRecord, TriggerRunRecord)>, TriggerError> {
        // Trigger-thread lookup is not exercised by this fake.
        Ok(None)
    }

    async fn upsert_trigger(&self, _record: TriggerRecord) -> Result<(), TriggerError> {
        unreachable!("replayed-missing repository is read-only")
    }

    async fn get_trigger(
        &self,
        _tenant_id: TenantId,
        _trigger_id: TriggerId,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        unreachable!("replayed-missing repository does not load records")
    }

    async fn list_triggers(
        &self,
        _tenant_id: TenantId,
    ) -> Result<Vec<TriggerRecord>, TriggerError> {
        unreachable!("replayed-missing repository does not list tenant records")
    }

    async fn list_scoped_triggers(
        &self,
        _tenant_id: TenantId,
        _creator_user_id: UserId,
        _agent_id: Option<AgentId>,
        _project_id: Option<ProjectId>,
        _limit: usize,
        _excluded_states: &[TriggerState],
    ) -> Result<Vec<TriggerRecord>, TriggerError> {
        unreachable!("replayed-missing repository does not list scoped records")
    }

    async fn remove_trigger(
        &self,
        _tenant_id: TenantId,
        _trigger_id: TriggerId,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        unreachable!("replayed-missing repository does not remove records")
    }

    async fn remove_scoped_trigger(
        &self,
        _tenant_id: TenantId,
        _creator_user_id: UserId,
        _agent_id: Option<AgentId>,
        _project_id: Option<ProjectId>,
        _trigger_id: TriggerId,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        unreachable!("test repository does not remove scoped records")
    }

    async fn list_due_triggers(
        &self,
        _now: Timestamp,
        _limit: usize,
    ) -> Result<Vec<TriggerRecord>, TriggerError> {
        Ok(vec![self.claimed_record.clone()])
    }

    async fn list_active_triggers(&self, limit: usize) -> Result<Vec<TriggerRecord>, TriggerError> {
        self.list_active_triggers_after(None, limit).await
    }

    async fn list_active_triggers_after(
        &self,
        _after: Option<ActiveTriggerScanCursor>,
        _limit: usize,
    ) -> Result<Vec<TriggerRecord>, TriggerError> {
        Ok(Vec::new())
    }

    async fn claim_due_fire(
        &self,
        _request: ClaimDueFireRequest,
    ) -> Result<ClaimDueFireOutcome, TriggerError> {
        Ok(ClaimDueFireOutcome::Claimed(ClaimedTriggerFire {
            record: self.claimed_record.clone(),
            fire_slot: self.fire_slot,
        }))
    }

    async fn mark_fire_accepted(
        &self,
        _request: FireAcceptedRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        unreachable!("replayed-missing repository should not persist accepted fires")
    }

    async fn mark_fire_replayed(
        &self,
        _request: FireReplayedRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        Ok(None)
    }

    async fn mark_fire_retryable_failed(
        &self,
        _request: FireRetryableFailedRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        unreachable!("replayed-missing repository should not persist retryable failures")
    }

    async fn mark_fire_permanently_failed(
        &self,
        _request: FirePermanentFailedRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        unreachable!("replayed-missing repository should not persist permanent failures")
    }

    async fn mark_fire_terminally_failed(
        &self,
        _request: FireTerminalFailedRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        unreachable!("replayed-missing repository should not persist terminal failures")
    }

    async fn clear_active_fire(
        &self,
        _request: ClearActiveFireRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        unreachable!("replayed-missing repository should not clear active fires")
    }
}

struct DueErrorThenSuccessRepository {
    failed_record: TriggerRecord,
    success_record: TriggerRecord,
    fire_slot: Timestamp,
}

#[async_trait]
impl TriggerRepository for DueErrorThenSuccessRepository {
    async fn find_trigger_run_by_thread_id(
        &self,
        _tenant_id: TenantId,
        _thread_id: &ThreadId,
    ) -> Result<Option<(TriggerRecord, TriggerRunRecord)>, TriggerError> {
        // Trigger-thread lookup is not exercised by this fake.
        Ok(None)
    }

    async fn upsert_trigger(&self, _record: TriggerRecord) -> Result<(), TriggerError> {
        unreachable!("due-error repository is read-only")
    }

    async fn get_trigger(
        &self,
        _tenant_id: TenantId,
        _trigger_id: TriggerId,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        unreachable!("due-error repository does not load records")
    }

    async fn list_triggers(
        &self,
        _tenant_id: TenantId,
    ) -> Result<Vec<TriggerRecord>, TriggerError> {
        unreachable!("due-error repository does not list tenant records")
    }

    async fn list_scoped_triggers(
        &self,
        _tenant_id: TenantId,
        _creator_user_id: UserId,
        _agent_id: Option<AgentId>,
        _project_id: Option<ProjectId>,
        _limit: usize,
        _excluded_states: &[TriggerState],
    ) -> Result<Vec<TriggerRecord>, TriggerError> {
        unreachable!("due-error repository does not list scoped records")
    }

    async fn remove_trigger(
        &self,
        _tenant_id: TenantId,
        _trigger_id: TriggerId,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        unreachable!("due-error repository does not remove records")
    }

    async fn remove_scoped_trigger(
        &self,
        _tenant_id: TenantId,
        _creator_user_id: UserId,
        _agent_id: Option<AgentId>,
        _project_id: Option<ProjectId>,
        _trigger_id: TriggerId,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        unreachable!("test repository does not remove scoped records")
    }

    async fn list_due_triggers(
        &self,
        _now: Timestamp,
        _limit: usize,
    ) -> Result<Vec<TriggerRecord>, TriggerError> {
        Ok(vec![
            self.failed_record.clone(),
            self.success_record.clone(),
        ])
    }

    async fn list_active_triggers(&self, limit: usize) -> Result<Vec<TriggerRecord>, TriggerError> {
        self.list_active_triggers_after(None, limit).await
    }

    async fn list_active_triggers_after(
        &self,
        _after: Option<ActiveTriggerScanCursor>,
        _limit: usize,
    ) -> Result<Vec<TriggerRecord>, TriggerError> {
        Ok(Vec::new())
    }

    async fn claim_due_fire(
        &self,
        request: ClaimDueFireRequest,
    ) -> Result<ClaimDueFireOutcome, TriggerError> {
        if request.trigger_id == self.failed_record.trigger_id {
            return Err(TriggerError::Backend {
                reason: "claim failed".to_string(),
            });
        }
        Ok(ClaimDueFireOutcome::Claimed(ClaimedTriggerFire {
            record: self.success_record.clone(),
            fire_slot: self.fire_slot,
        }))
    }

    async fn mark_fire_accepted(
        &self,
        _request: FireAcceptedRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        Ok(Some(self.success_record.clone()))
    }

    async fn mark_fire_replayed(
        &self,
        _request: FireReplayedRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        unreachable!("due-error repository should not persist replayed fires")
    }

    async fn mark_fire_retryable_failed(
        &self,
        _request: FireRetryableFailedRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        unreachable!("due-error repository should not persist retryable failures")
    }

    async fn mark_fire_permanently_failed(
        &self,
        _request: FirePermanentFailedRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        unreachable!("due-error repository should not persist permanent failures")
    }

    async fn mark_fire_terminally_failed(
        &self,
        _request: FireTerminalFailedRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        unreachable!("due-error repository should not persist terminal failures")
    }

    async fn clear_active_fire(
        &self,
        _request: ClearActiveFireRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        unreachable!("due-error repository should not clear active fires")
    }
}

struct ClaimRaceRepository {
    due_record: TriggerRecord,
    claim_outcome: Mutex<Option<ClaimDueFireOutcome>>,
}

impl ClaimRaceRepository {
    fn new(due_record: TriggerRecord, claim_outcome: ClaimDueFireOutcome) -> Self {
        Self {
            due_record,
            claim_outcome: Mutex::new(Some(claim_outcome)),
        }
    }
}

#[async_trait]
impl TriggerRepository for ClaimRaceRepository {
    async fn find_trigger_run_by_thread_id(
        &self,
        _tenant_id: TenantId,
        _thread_id: &ThreadId,
    ) -> Result<Option<(TriggerRecord, TriggerRunRecord)>, TriggerError> {
        // Trigger-thread lookup is not exercised by this fake.
        Ok(None)
    }

    async fn upsert_trigger(&self, _record: TriggerRecord) -> Result<(), TriggerError> {
        unreachable!("claim-race repository is read-only")
    }

    async fn get_trigger(
        &self,
        _tenant_id: TenantId,
        _trigger_id: TriggerId,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        unreachable!("claim-race repository does not load records")
    }

    async fn list_triggers(
        &self,
        _tenant_id: TenantId,
    ) -> Result<Vec<TriggerRecord>, TriggerError> {
        unreachable!("claim-race repository does not list tenant records")
    }

    async fn list_scoped_triggers(
        &self,
        _tenant_id: TenantId,
        _creator_user_id: UserId,
        _agent_id: Option<AgentId>,
        _project_id: Option<ProjectId>,
        _limit: usize,
        _excluded_states: &[TriggerState],
    ) -> Result<Vec<TriggerRecord>, TriggerError> {
        unreachable!("claim-race repository does not list scoped records")
    }

    async fn remove_trigger(
        &self,
        _tenant_id: TenantId,
        _trigger_id: TriggerId,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        unreachable!("claim-race repository does not remove records")
    }

    async fn remove_scoped_trigger(
        &self,
        _tenant_id: TenantId,
        _creator_user_id: UserId,
        _agent_id: Option<AgentId>,
        _project_id: Option<ProjectId>,
        _trigger_id: TriggerId,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        unreachable!("test repository does not remove scoped records")
    }

    async fn list_due_triggers(
        &self,
        _now: Timestamp,
        _limit: usize,
    ) -> Result<Vec<TriggerRecord>, TriggerError> {
        Ok(vec![self.due_record.clone()])
    }

    async fn list_active_triggers(&self, limit: usize) -> Result<Vec<TriggerRecord>, TriggerError> {
        self.list_active_triggers_after(None, limit).await
    }

    async fn list_active_triggers_after(
        &self,
        _after: Option<ActiveTriggerScanCursor>,
        _limit: usize,
    ) -> Result<Vec<TriggerRecord>, TriggerError> {
        Ok(Vec::new())
    }

    async fn claim_due_fire(
        &self,
        _request: ClaimDueFireRequest,
    ) -> Result<ClaimDueFireOutcome, TriggerError> {
        Ok(self
            .claim_outcome
            .lock()
            .expect("claim outcome lock")
            .take()
            .expect("claim outcome configured"))
    }

    async fn mark_fire_accepted(
        &self,
        _request: FireAcceptedRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        unreachable!("claim-race repository should not persist accepted fires")
    }

    async fn mark_fire_replayed(
        &self,
        _request: FireReplayedRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        unreachable!("claim-race repository should not persist replayed fires")
    }

    async fn mark_fire_retryable_failed(
        &self,
        _request: FireRetryableFailedRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        unreachable!("claim-race repository should not persist retryable failures")
    }

    async fn mark_fire_permanently_failed(
        &self,
        _request: FirePermanentFailedRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        unreachable!("claim-race repository should not persist permanent failures")
    }

    async fn mark_fire_terminally_failed(
        &self,
        _request: FireTerminalFailedRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        unreachable!("claim-race repository should not persist terminal failures")
    }

    async fn clear_active_fire(
        &self,
        _request: ClearActiveFireRequest,
    ) -> Result<Option<TriggerRecord>, TriggerError> {
        unreachable!("claim-race repository should not clear active fires")
    }
}

#[tokio::test]
async fn timezone_aware_firing_harness() {
    // Fixed instants — all UTC.
    // "0 9 * * *" America/New_York in January (UTC-5) = 14:00 UTC.
    let before = ymd_hms(2026, 1, 2, 13, 0, 0);
    let expected_seed_next_run_at = ymd_hms(2026, 1, 2, 14, 0, 0);
    let tick1 = ymd_hms(2026, 1, 2, 13, 59, 0);
    let tick2 = ymd_hms(2026, 1, 2, 14, 0, 0);
    let expected_post_fire_next_run_at = ymd_hms(2026, 1, 3, 14, 0, 0);

    // Build the schedule and compute the seed next_run_at from the reference instant.
    let schedule = TriggerSchedule::cron_with_timezone("0 9 * * *", "America/New_York")
        .expect("valid schedule");
    let computed_seed = schedule
        .next_slot_after(before)
        .expect("next slot computation succeeds")
        .expect("there is a future slot after 2026-01-02 13:00:00 UTC");

    // Conversion assertion: 9 AM ET (UTC-5 in January) = 14:00 UTC.
    assert_eq!(
        computed_seed, expected_seed_next_run_at,
        "next_slot_after(13:00 UTC) for '0 9 * * *' America/New_York must be 14:00 UTC"
    );

    // Seed one trigger record with next_run_at = 14:00 UTC.
    let repo = Arc::new(InMemoryTriggerRepository::default());
    let trigger_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
    let record = TriggerRecord {
        trigger_id,
        tenant_id: tenant("tenant-tz"),
        creator_user_id: user("user-a"),
        agent_id: Some(AgentId::new("agent-a").expect("valid agent")),
        project_id: Some(ProjectId::new("project-a").expect("valid project")),
        name: "daily 9am eastern".to_string(),
        source: TriggerSourceKind::Schedule,
        schedule: schedule.clone(),
        prompt: "run daily summary".to_string(),
        state: TriggerState::Scheduled,
        next_run_at: computed_seed,
        last_run_at: None,
        last_fired_slot: None,
        last_status: None,
        active_fire_slot: None,
        active_run_ref: None,
        created_at: ts(1_704_067_000),
    };
    repo.upsert_trigger(record.clone()).await.expect("insert");

    // ── tick1: 13:59 UTC — not yet due, must not fire ────────────────
    {
        let submitter = Arc::new(RecordingSubmitter::with_outcomes(Vec::new()));
        let materializer = Arc::new(RecordingMaterializer::success("content:trigger-fire"));
        let w = worker(
            repo.clone(),
            materializer.clone(),
            submitter.clone(),
            Arc::new(RecordingActiveRunLookup::default()),
        );

        w.tick_once(tick1).await.expect("tick1 succeeds");

        assert_eq!(
            submitter.requests().len(),
            0,
            "tick1 (13:59 UTC): submitter must not be called before 14:00 UTC fire slot"
        );
        let state_after_tick1 = repo
            .get_trigger(tenant("tenant-tz"), trigger_id)
            .await
            .expect("load after tick1")
            .expect("record present");
        assert!(
            state_after_tick1.last_fired_slot.is_none(),
            "tick1 (13:59 UTC): last_fired_slot must remain None"
        );
        assert_eq!(
            state_after_tick1.next_run_at, expected_seed_next_run_at,
            "tick1 (13:59 UTC): next_run_at must be unchanged"
        );
    }

    // ── tick2: 14:00 UTC — due, must fire ───────────────────────────
    {
        let run_id = TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5a").expect("run id");
        let submitter = Arc::new(RecordingSubmitter::with_outcomes(vec![Ok(
            TrustedTriggerFireSubmitOutcome::Accepted {
                run_id,
                submitted_at: tick2,
                turn_scope: test_turn_scope(),
            },
        )]));
        let materializer = Arc::new(RecordingMaterializer::success("content:trigger-fire"));
        let w = worker(
            repo.clone(),
            materializer.clone(),
            submitter.clone(),
            Arc::new(RecordingActiveRunLookup::default()),
        );

        w.tick_once(tick2).await.expect("tick2 succeeds");

        assert_eq!(
            submitter.requests().len(),
            1,
            "tick2 (14:00 UTC): submitter must be called exactly once"
        );
        let state_after_tick2 = repo
            .get_trigger(tenant("tenant-tz"), trigger_id)
            .await
            .expect("load after tick2")
            .expect("record present");
        assert_eq!(
            state_after_tick2.last_fired_slot,
            Some(tick2),
            "tick2 (14:00 UTC): last_fired_slot must equal the fire slot"
        );
        assert_eq!(
            state_after_tick2.next_run_at, expected_post_fire_next_run_at,
            "tick2 (14:00 UTC): next_run_at must advance to 2026-01-03 14:00:00 UTC (next day 9 AM ET)"
        );
    }
}
