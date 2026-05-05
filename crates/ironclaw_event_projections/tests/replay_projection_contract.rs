use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_event_projections::{
    EventProjectionService, MAX_PROJECTION_PAGE_LIMIT, ProjectionCursor, ProjectionError,
    ProjectionRequest, ProjectionScope, ReplayEventProjectionService, RunProjectionStatus,
    TimelineEntryKind,
};
use ironclaw_events::{
    DurableEventLog, EventCursor, EventError, EventLogEntry, EventReplay, EventStreamKey,
    InMemoryDurableEventLog, ReadScope, RuntimeEvent, RuntimeEventId, RuntimeEventKind,
    UNCLASSIFIED_ERROR_KIND,
};
use ironclaw_host_api::{
    AgentId, CapabilityId, ExtensionId, InvocationId, ProcessId, ProjectId, ResourceScope,
    RuntimeKind, TenantId, ThreadId, UserId,
};

#[tokio::test]
async fn replay_projection_service_projects_timeline_and_run_status_by_scope() {
    let log = Arc::new(InMemoryDurableEventLog::new());
    let service = ReplayEventProjectionService::new(Arc::clone(&log));
    let capability = capability_id();
    let provider = provider_id();
    let thread_a = ThreadId::new("thread-a").unwrap();
    let thread_b = ThreadId::new("thread-b").unwrap();
    let scope_a = scope_for_thread(thread_a.clone());
    let scope_b = scope_for_thread(thread_b);

    log.append(RuntimeEvent::dispatch_requested(
        scope_a.clone(),
        capability.clone(),
    ))
    .await
    .unwrap();
    log.append(RuntimeEvent::runtime_selected(
        scope_a.clone(),
        capability.clone(),
        provider.clone(),
        RuntimeKind::Script,
    ))
    .await
    .unwrap();
    log.append(RuntimeEvent::dispatch_requested(
        scope_b,
        capability.clone(),
    ))
    .await
    .unwrap();
    log.append(RuntimeEvent::dispatch_succeeded(
        scope_a.clone(),
        capability.clone(),
        provider.clone(),
        RuntimeKind::Script,
        42,
    ))
    .await
    .unwrap();

    let snapshot = service
        .snapshot(ProjectionRequest {
            scope: ProjectionScope::from_resource_scope(&scope_a),
            after: None,
            limit: 16,
        })
        .await
        .unwrap();

    assert_eq!(snapshot.timeline.entries.len(), 3);
    assert!(snapshot.timeline.entries.iter().all(|entry| {
        entry.thread_id.as_ref() == Some(&thread_a) && entry.capability_id == capability
    }));
    assert_eq!(
        snapshot
            .timeline
            .entries
            .iter()
            .map(|entry| entry.kind)
            .collect::<Vec<_>>(),
        vec![
            TimelineEntryKind::DispatchRequested,
            TimelineEntryKind::RuntimeSelected,
            TimelineEntryKind::DispatchSucceeded,
        ]
    );
    assert_eq!(snapshot.runs.len(), 1);
    assert_eq!(snapshot.runs[0].status, RunProjectionStatus::Completed);
    assert_eq!(snapshot.next_cursor.runtime, EventCursor::new(4));
    assert!(!snapshot.truncated);
}

#[tokio::test]
async fn replay_projection_updates_return_rebase_signal_for_foreign_or_stale_cursor() {
    let log = Arc::new(InMemoryDurableEventLog::new());
    let service = ReplayEventProjectionService::new(Arc::clone(&log));
    let scope = scope_for_thread(ThreadId::new("thread-a").unwrap());
    let capability = capability_id();

    log.append(RuntimeEvent::dispatch_requested(
        scope.clone(),
        capability.clone(),
    ))
    .await
    .unwrap();

    let error = service
        .updates(ProjectionRequest {
            scope: ProjectionScope::from_resource_scope(&scope),
            after: Some(ProjectionCursor::new(EventCursor::new(99))),
            limit: 16,
        })
        .await
        .unwrap_err();

    match error {
        ProjectionError::RebaseRequired { requested, .. } => {
            assert_eq!(requested.runtime, EventCursor::new(99));
        }
        other => panic!("expected rebase-required projection error, got {other:?}"),
    }

    let snapshot = service
        .snapshot(ProjectionRequest {
            scope: ProjectionScope::from_resource_scope(&scope),
            after: None,
            limit: 16,
        })
        .await
        .unwrap();
    assert_eq!(snapshot.timeline.entries.len(), 1);
}

#[tokio::test]
async fn replay_projection_updates_resume_after_projection_cursor() {
    let log = Arc::new(InMemoryDurableEventLog::new());
    let service = ReplayEventProjectionService::new(Arc::clone(&log));
    let scope = scope_for_thread(ThreadId::new("thread-a").unwrap());
    let capability = capability_id();
    let provider = provider_id();

    let first = log
        .append(RuntimeEvent::dispatch_requested(
            scope.clone(),
            capability.clone(),
        ))
        .await
        .unwrap();
    log.append(RuntimeEvent::runtime_selected(
        scope.clone(),
        capability.clone(),
        provider.clone(),
        RuntimeKind::Script,
    ))
    .await
    .unwrap();
    log.append(RuntimeEvent::dispatch_succeeded(
        scope.clone(),
        capability,
        provider,
        RuntimeKind::Script,
        12,
    ))
    .await
    .unwrap();

    let replay = service
        .updates(ProjectionRequest {
            scope: ProjectionScope::from_resource_scope(&scope),
            after: Some(ProjectionCursor::new(first.cursor)),
            limit: 16,
        })
        .await
        .unwrap();

    assert_eq!(replay.updates.len(), 2);
    assert_eq!(
        replay
            .updates
            .iter()
            .map(|entry| entry.kind)
            .collect::<Vec<_>>(),
        vec![
            TimelineEntryKind::RuntimeSelected,
            TimelineEntryKind::DispatchSucceeded,
        ]
    );
    assert_eq!(replay.runs.len(), 1);
    assert_eq!(replay.runs[0].status, RunProjectionStatus::Completed);
    assert_eq!(replay.next_cursor.runtime, EventCursor::new(3));
}

#[tokio::test]
async fn replay_projection_updates_preserve_running_process_state_after_checkpoint() {
    let log = Arc::new(InMemoryDurableEventLog::new());
    let service = ReplayEventProjectionService::new(Arc::clone(&log));
    let scope = scope_for_thread(ThreadId::new("thread-a").unwrap());
    let capability = capability_id();
    let provider = provider_id();
    let process_id = ProcessId::new();

    log.append(RuntimeEvent::dispatch_requested(
        scope.clone(),
        capability.clone(),
    ))
    .await
    .unwrap();
    let started = log
        .append(RuntimeEvent::process_started(
            scope.clone(),
            capability.clone(),
            provider.clone(),
            RuntimeKind::Script,
            process_id,
        ))
        .await
        .unwrap();
    log.append(RuntimeEvent::dispatch_succeeded(
        scope.clone(),
        capability,
        provider,
        RuntimeKind::Script,
        0,
    ))
    .await
    .unwrap();

    let replay = service
        .updates(ProjectionRequest {
            scope: ProjectionScope::from_resource_scope(&scope),
            after: Some(ProjectionCursor::new(started.cursor)),
            limit: 16,
        })
        .await
        .unwrap();

    assert_eq!(replay.updates.len(), 1);
    assert_eq!(replay.updates[0].kind, TimelineEntryKind::DispatchSucceeded);
    assert_eq!(replay.runs.len(), 1);
    assert_eq!(replay.runs[0].process_id, Some(process_id));
    assert_eq!(replay.runs[0].status, RunProjectionStatus::Running);
}

#[tokio::test]
async fn replay_projection_keeps_spawned_process_run_active_until_terminal_process_event() {
    let log = Arc::new(InMemoryDurableEventLog::new());
    let service = ReplayEventProjectionService::new(Arc::clone(&log));
    let scope = scope_for_thread(ThreadId::new("thread-a").unwrap());
    let capability = capability_id();
    let provider = provider_id();
    let process_id = ProcessId::new();

    log.append(RuntimeEvent::dispatch_requested(
        scope.clone(),
        capability.clone(),
    ))
    .await
    .unwrap();
    log.append(RuntimeEvent::process_started(
        scope.clone(),
        capability.clone(),
        provider.clone(),
        RuntimeKind::Script,
        process_id,
    ))
    .await
    .unwrap();
    log.append(RuntimeEvent::dispatch_succeeded(
        scope.clone(),
        capability,
        provider,
        RuntimeKind::Script,
        0,
    ))
    .await
    .unwrap();

    let snapshot = service
        .snapshot(ProjectionRequest {
            scope: ProjectionScope::from_resource_scope(&scope),
            after: None,
            limit: 16,
        })
        .await
        .unwrap();

    assert_eq!(snapshot.runs.len(), 1);
    assert_eq!(snapshot.runs[0].process_id, Some(process_id));
    assert_eq!(snapshot.runs[0].status, RunProjectionStatus::Running);
}

#[tokio::test]
async fn replay_projection_orders_runs_by_recent_activity_descending() {
    let log = Arc::new(InMemoryDurableEventLog::new());
    let service = ReplayEventProjectionService::new(Arc::clone(&log));
    let thread = ThreadId::new("thread-a").unwrap();
    let older_invocation = InvocationId::parse("00000000-0000-4000-8000-000000000001").unwrap();
    let newer_invocation = InvocationId::parse("ffffffff-ffff-4fff-8fff-ffffffffffff").unwrap();
    let older_scope = scope_for_thread_with_invocation(thread.clone(), older_invocation);
    let newer_scope = scope_for_thread_with_invocation(thread, newer_invocation);
    let capability = capability_id();

    log.append(RuntimeEvent::dispatch_requested(
        older_scope.clone(),
        capability.clone(),
    ))
    .await
    .unwrap();
    log.append(RuntimeEvent::dispatch_requested(newer_scope, capability))
        .await
        .unwrap();

    let snapshot = service
        .snapshot(ProjectionRequest {
            scope: ProjectionScope::from_resource_scope(&older_scope),
            after: None,
            limit: 16,
        })
        .await
        .unwrap();

    assert_eq!(snapshot.runs.len(), 2);
    assert_eq!(snapshot.runs[0].invocation_id, newer_invocation);
    assert_eq!(snapshot.runs[1].invocation_id, older_invocation);
}

#[tokio::test]
async fn replay_projection_output_does_not_expose_raw_runtime_details() {
    let log = Arc::new(InMemoryDurableEventLog::new());
    let service = ReplayEventProjectionService::new(Arc::clone(&log));
    let scope = scope_for_thread(ThreadId::new("thread-a").unwrap());
    let capability = capability_id();

    log.append(RuntimeEvent::dispatch_failed(
        scope.clone(),
        capability,
        None,
        None,
        "raw failure /tmp/private-host-path SECRET_PROJECTION_SENTINEL_sk_live",
    ))
    .await
    .unwrap();

    let snapshot = service
        .snapshot(ProjectionRequest {
            scope: ProjectionScope::from_resource_scope(&scope),
            after: None,
            limit: 16,
        })
        .await
        .unwrap();
    let serialized = serde_json::to_string(&snapshot).unwrap();

    for forbidden in [
        "/tmp/private-host-path",
        "SECRET_PROJECTION_SENTINEL",
        "sk_live",
        "raw failure",
    ] {
        assert!(
            !serialized.contains(forbidden),
            "projection output leaked {forbidden}: {serialized}"
        );
    }
    assert!(serialized.contains("Unclassified"));
}

#[tokio::test]
async fn replay_projection_errors_do_not_expose_backend_details() {
    let service = ReplayEventProjectionService::new(Arc::new(FailingDurableEventLog));
    let scope = scope_for_thread(ThreadId::new("thread-a").unwrap());

    let error = service
        .snapshot(ProjectionRequest {
            scope: ProjectionScope::from_resource_scope(&scope),
            after: None,
            limit: 16,
        })
        .await
        .unwrap_err();
    let message = error.to_string();

    for forbidden in [
        "DATABASE_PROJECTION_SENTINEL",
        "/tmp/backend-private-path",
        "sk_live",
    ] {
        assert!(
            !message.contains(forbidden),
            "projection error leaked {forbidden}: {message}"
        );
    }
    assert!(message.contains("projection source failed"));
}

struct FailingDurableEventLog;

#[async_trait]
impl DurableEventLog for FailingDurableEventLog {
    async fn append(
        &self,
        _event: RuntimeEvent,
    ) -> Result<EventLogEntry<RuntimeEvent>, EventError> {
        Err(EventError::DurableLog {
            reason: "DATABASE_PROJECTION_SENTINEL /tmp/backend-private-path sk_live".to_string(),
        })
    }

    async fn read_after_cursor(
        &self,
        _stream: &EventStreamKey,
        _filter: &ReadScope,
        _after: Option<EventCursor>,
        _limit: usize,
    ) -> Result<EventReplay<RuntimeEvent>, EventError> {
        Err(EventError::DurableLog {
            reason: "DATABASE_PROJECTION_SENTINEL /tmp/backend-private-path sk_live".to_string(),
        })
    }
}

fn scope_for_thread(thread_id: ThreadId) -> ResourceScope {
    scope_for_thread_with_invocation(thread_id, InvocationId::new())
}

fn scope_for_thread_with_invocation(
    thread_id: ThreadId,
    invocation_id: InvocationId,
) -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new("tenant-a").unwrap(),
        user_id: UserId::new("user-a").unwrap(),
        agent_id: Some(AgentId::new("agent-a").unwrap()),
        project_id: Some(ProjectId::new("project-a").unwrap()),
        mission_id: None,
        thread_id: Some(thread_id),
        invocation_id,
    }
}

fn capability_id() -> CapabilityId {
    CapabilityId::new("script.echo").unwrap()
}

fn provider_id() -> ExtensionId {
    ExtensionId::new("script").unwrap()
}

// -----------------------------------------------------------------------------
// Regression: PR #3212 review feedback (serrrfirat, 2026-05-03)
// -----------------------------------------------------------------------------

/// Regression for review comment 3178562797: a custom `DurableEventLog`
/// backend can return a `RuntimeEvent` whose `error_kind` was never run
/// through `sanitize_error_kind` (the typed constructors always sanitize, but
/// the struct fields are `pub` so direct construction or a future backend can
/// bypass them). The projection layer must re-sanitize at the projection
/// boundary so leaked paths/secrets cannot reach product DTOs.
#[tokio::test]
async fn replay_projection_re_sanitizes_unsanitized_runtime_events_from_custom_backend() {
    let scope = scope_for_thread(ThreadId::new("thread-a").unwrap());
    let raw = "raw failure /tmp/private-host-path SECRET_PROJECTION_SENTINEL_sk_live";
    let unsanitized = RuntimeEvent {
        event_id: RuntimeEventId::new(),
        timestamp: Utc::now(),
        kind: RuntimeEventKind::ProcessFailed,
        scope: scope.clone(),
        capability_id: capability_id(),
        provider: Some(provider_id()),
        runtime: Some(RuntimeKind::Script),
        process_id: Some(ProcessId::new()),
        output_bytes: None,
        error_kind: Some(raw.to_string()),
    };
    let backend = Arc::new(StaticDurableEventLog {
        entries: vec![EventLogEntry {
            cursor: EventCursor::new(1),
            record: unsanitized,
        }],
    });
    let service = ReplayEventProjectionService::new(Arc::clone(&backend));

    let snapshot = service
        .snapshot(ProjectionRequest {
            scope: ProjectionScope::from_resource_scope(&scope),
            after: None,
            limit: 16,
        })
        .await
        .unwrap();

    assert_eq!(snapshot.timeline.entries.len(), 1);
    assert_eq!(
        snapshot.timeline.entries[0].error_kind.as_deref(),
        Some(UNCLASSIFIED_ERROR_KIND)
    );
    assert_eq!(snapshot.runs.len(), 1);
    assert_eq!(
        snapshot.runs[0].error_kind.as_deref(),
        Some(UNCLASSIFIED_ERROR_KIND)
    );
    let serialized = serde_json::to_string(&snapshot).unwrap();
    for forbidden in [
        "/tmp/private-host-path",
        "SECRET_PROJECTION_SENTINEL",
        "sk_live",
        "raw failure",
    ] {
        assert!(
            !serialized.contains(forbidden),
            "projection leaked {forbidden}: {serialized}"
        );
    }
}

/// Regression for review comment 3178562826: for a process-backed run, a
/// late `DispatchSucceeded` event must NOT clobber a terminal `Failed` /
/// `Killed` status produced by an earlier `process_failed` /
/// `process_killed` event. The previous guard only preserved `Running`; the
/// `process_started -> process_failed -> dispatch_succeeded` ordering would
/// silently mark the run `Completed` and hide the failure.
#[tokio::test]
async fn replay_projection_dispatch_succeeded_does_not_clobber_terminal_process_failure() {
    let log = Arc::new(InMemoryDurableEventLog::new());
    let service = ReplayEventProjectionService::new(Arc::clone(&log));
    let scope = scope_for_thread(ThreadId::new("thread-a").unwrap());
    let capability = capability_id();
    let provider = provider_id();
    let process_id = ProcessId::new();

    log.append(RuntimeEvent::process_started(
        scope.clone(),
        capability.clone(),
        provider.clone(),
        RuntimeKind::Script,
        process_id,
    ))
    .await
    .unwrap();
    log.append(RuntimeEvent::process_failed(
        scope.clone(),
        capability.clone(),
        provider.clone(),
        RuntimeKind::Script,
        process_id,
        "process_crashed",
    ))
    .await
    .unwrap();
    log.append(RuntimeEvent::dispatch_succeeded(
        scope.clone(),
        capability,
        provider,
        RuntimeKind::Script,
        0,
    ))
    .await
    .unwrap();

    let snapshot = service
        .snapshot(ProjectionRequest {
            scope: ProjectionScope::from_resource_scope(&scope),
            after: None,
            limit: 16,
        })
        .await
        .unwrap();

    assert_eq!(snapshot.runs.len(), 1);
    assert_eq!(snapshot.runs[0].status, RunProjectionStatus::Failed);
    assert_eq!(
        snapshot.runs[0].error_kind.as_deref(),
        Some("process_crashed")
    );
}

/// Same regression, but for `process_killed` followed by
/// `dispatch_succeeded`.
#[tokio::test]
async fn replay_projection_dispatch_succeeded_does_not_clobber_terminal_process_killed() {
    let log = Arc::new(InMemoryDurableEventLog::new());
    let service = ReplayEventProjectionService::new(Arc::clone(&log));
    let scope = scope_for_thread(ThreadId::new("thread-a").unwrap());
    let capability = capability_id();
    let provider = provider_id();
    let process_id = ProcessId::new();

    log.append(RuntimeEvent::process_started(
        scope.clone(),
        capability.clone(),
        provider.clone(),
        RuntimeKind::Script,
        process_id,
    ))
    .await
    .unwrap();
    log.append(RuntimeEvent::process_killed(
        scope.clone(),
        capability.clone(),
        provider.clone(),
        RuntimeKind::Script,
        process_id,
    ))
    .await
    .unwrap();
    log.append(RuntimeEvent::dispatch_succeeded(
        scope.clone(),
        capability,
        provider,
        RuntimeKind::Script,
        0,
    ))
    .await
    .unwrap();

    let snapshot = service
        .snapshot(ProjectionRequest {
            scope: ProjectionScope::from_resource_scope(&scope),
            after: None,
            limit: 16,
        })
        .await
        .unwrap();

    assert_eq!(snapshot.runs.len(), 1);
    assert_eq!(snapshot.runs[0].status, RunProjectionStatus::Killed);
}

/// Regression for review comment 3178562852: `updates(limit=1)` on a
/// long-lived thread used to read the full prefix into a `Vec` (via the
/// removed `read_runtime_prefix`) before projecting runs. After the fix it
/// folds the prefix incrementally with `O(touched_runs)` allocation, and a
/// hard cap surfaces `RebaseRequired` rather than allocating without
/// bound. This test seeds many prefix events and asserts the bounded-page
/// contract.
#[tokio::test]
async fn replay_projection_updates_with_small_limit_handles_long_prefix() {
    let log = Arc::new(InMemoryDurableEventLog::new());
    let service = ReplayEventProjectionService::new(Arc::clone(&log));
    let scope = scope_for_thread(ThreadId::new("thread-a").unwrap());
    let capability = capability_id();

    // Seed many prefix entries (smaller than the rebase cap, larger than the
    // internal page limit so we exercise the paging fold path).
    let prefix_len: usize = 600;
    for _ in 0..prefix_len {
        log.append(RuntimeEvent::dispatch_requested(
            scope.clone(),
            capability.clone(),
        ))
        .await
        .unwrap();
    }

    // Resume from "just before the tail" so `updates(limit=1)` returns one
    // new event and must fold the prefix to reconstruct the touched run.
    let resume_after = ProjectionCursor::new(EventCursor::new(prefix_len as u64 - 1));
    let replay = service
        .updates(ProjectionRequest {
            scope: ProjectionScope::from_resource_scope(&scope),
            after: Some(resume_after),
            limit: 1,
        })
        .await
        .unwrap();

    // The page is bounded to `limit=1`, regardless of prefix length.
    assert_eq!(replay.updates.len(), 1);
    // The single touched run is reconstructed from the folded prefix.
    assert_eq!(replay.runs.len(), 1);
    assert_eq!(replay.runs[0].status, RunProjectionStatus::Running);
    assert_eq!(
        replay.next_cursor.runtime,
        EventCursor::new(prefix_len as u64)
    );
}

/// A custom backend that returns a fixed set of (cursor, record) entries on
/// the first `read_after_cursor(after=None, ..)` call and an empty page
/// otherwise. Used for regressions that need to inject hand-built
/// `RuntimeEvent`s that bypass the typed sanitizing constructors.
struct StaticDurableEventLog {
    entries: Vec<EventLogEntry<RuntimeEvent>>,
}

#[async_trait]
impl DurableEventLog for StaticDurableEventLog {
    async fn append(
        &self,
        _event: RuntimeEvent,
    ) -> Result<EventLogEntry<RuntimeEvent>, EventError> {
        Err(EventError::DurableLog {
            reason: "static-log:append-not-supported".to_string(),
        })
    }

    async fn read_after_cursor(
        &self,
        _stream: &EventStreamKey,
        _filter: &ReadScope,
        after: Option<EventCursor>,
        limit: usize,
    ) -> Result<EventReplay<RuntimeEvent>, EventError> {
        let cutoff = after.unwrap_or_else(EventCursor::origin);
        let visible: Vec<EventLogEntry<RuntimeEvent>> = self
            .entries
            .iter()
            .filter(|entry| entry.cursor > cutoff)
            .take(limit)
            .cloned()
            .collect();
        let next_cursor = visible.last().map(|entry| entry.cursor).unwrap_or(cutoff);
        Ok(EventReplay {
            entries: visible,
            next_cursor,
        })
    }
}

// -----------------------------------------------------------------------------
// Regression: PR #3212 review feedback — bounded projection page size
// -----------------------------------------------------------------------------

fn projection_request_with_limit(limit: usize) -> ProjectionRequest {
    ProjectionRequest {
        scope: ProjectionScope::from_resource_scope(&scope_for_thread(
            ThreadId::new("thread-limit").unwrap(),
        )),
        after: None,
        limit,
    }
}

#[tokio::test]
async fn replay_projection_rejects_zero_limit() {
    let service = ReplayEventProjectionService::new(Arc::new(InMemoryDurableEventLog::new()));
    let err = service
        .snapshot(projection_request_with_limit(0))
        .await
        .expect_err("limit=0 must be rejected");
    assert!(matches!(err, ProjectionError::InvalidRequest { .. }));
}

#[tokio::test]
async fn replay_projection_accepts_limit_at_max() {
    let service = ReplayEventProjectionService::new(Arc::new(InMemoryDurableEventLog::new()));
    service
        .snapshot(projection_request_with_limit(MAX_PROJECTION_PAGE_LIMIT))
        .await
        .expect("limit at MAX_PROJECTION_PAGE_LIMIT must be accepted");
}

#[tokio::test]
async fn replay_projection_rejects_limit_above_max() {
    let service = ReplayEventProjectionService::new(Arc::new(InMemoryDurableEventLog::new()));
    let err = service
        .snapshot(projection_request_with_limit(MAX_PROJECTION_PAGE_LIMIT + 1))
        .await
        .expect_err("limit > MAX_PROJECTION_PAGE_LIMIT must be rejected");
    assert!(matches!(err, ProjectionError::InvalidRequest { .. }));
}

#[tokio::test]
async fn replay_projection_rejects_usize_max_limit() {
    let service = ReplayEventProjectionService::new(Arc::new(InMemoryDurableEventLog::new()));
    let err = service
        .snapshot(projection_request_with_limit(usize::MAX))
        .await
        .expect_err("limit=usize::MAX must be rejected");
    assert!(matches!(err, ProjectionError::InvalidRequest { .. }));
}

#[tokio::test]
async fn replay_projection_updates_rejects_limit_above_max() {
    let service = ReplayEventProjectionService::new(Arc::new(InMemoryDurableEventLog::new()));
    let err = service
        .updates(projection_request_with_limit(MAX_PROJECTION_PAGE_LIMIT + 1))
        .await
        .expect_err("updates() must enforce the same cap as snapshot()");
    assert!(matches!(err, ProjectionError::InvalidRequest { .. }));
}
