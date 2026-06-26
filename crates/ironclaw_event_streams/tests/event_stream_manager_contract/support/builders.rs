fn assert_same_error_kind(actual: ProjectionStreamError, expected: ProjectionStreamError) {
    match (actual, expected) {
        (ProjectionStreamError::AccessDenied, ProjectionStreamError::AccessDenied)
        | (ProjectionStreamError::AdmissionDenied, ProjectionStreamError::AdmissionDenied)
        | (ProjectionStreamError::Source, ProjectionStreamError::Source)
        | (ProjectionStreamError::Redaction, ProjectionStreamError::Redaction)
        | (ProjectionStreamError::Outbound, ProjectionStreamError::Outbound) => {}
        (
            ProjectionStreamError::InvalidRequest { reason: actual },
            ProjectionStreamError::InvalidRequest { reason: expected },
        ) => assert_eq!(actual, expected),
        (actual, expected) => panic!("expected {expected:?}, got {actual:?}"),
    }
}

fn expect_thread_update(item: ProjectionStreamItem) -> ProjectionReplay {
    match item {
        ProjectionStreamItem::Update(envelope) => match envelope.as_ref() {
            ProductProjectionEnvelope::ThreadUpdates(replay) => replay.clone(),
            other => panic!("expected thread update, got {other:?}"),
        },
        other => panic!("expected thread update, got {other:?}"),
    }
}

fn subscribe_request(
    scope: ProjectionScope,
    after_cursor: Option<ProjectionCursor>,
) -> ProjectionSubscribeRequest {
    ProjectionSubscribeRequest {
        actor: actor("user-a"),
        target: ProjectionTarget::Thread {
            thread_id: scope.read_scope.thread_id.clone().unwrap(),
        },
        scope,
        view: ProjectionViewClass::ProductThread,
        after_cursor,
        limit: 16,
        capabilities: SubscriberCapabilities { buffer_capacity: 2 },
    }
}

fn subscribe_request_for_stream_user(
    scope: ProjectionScope,
    after_cursor: Option<ProjectionCursor>,
) -> ProjectionSubscribeRequest {
    ProjectionSubscribeRequest {
        actor: TurnActor::new(scope.stream.user_id.clone()),
        target: ProjectionTarget::Thread {
            thread_id: scope.read_scope.thread_id.clone().unwrap(),
        },
        scope,
        view: ProjectionViewClass::ProductThread,
        after_cursor,
        limit: 16,
        capabilities: SubscriberCapabilities { buffer_capacity: 2 },
    }
}

fn fetch_request(scope: ProjectionScope) -> ProjectionFetchRequest {
    ProjectionFetchRequest {
        actor: actor("user-a"),
        target: ProjectionTarget::Thread {
            thread_id: scope.read_scope.thread_id.clone().unwrap(),
        },
        scope,
        view: ProjectionViewClass::ProductThread,
        limit: 16,
    }
}

fn push_request(scope: &TurnScope, kind: OutboundPushKind) -> PushCandidatesForUpdateRequest {
    let projection_scope = ProjectionScope {
        stream: EventStreamKey::new(
            scope.tenant_id.clone(),
            UserId::new("user-a").unwrap(),
            scope.agent_id.clone(),
        ),
        read_scope: ReadScope {
            project_id: scope.project_id.clone(),
            mission_id: None,
            thread_id: Some(scope.thread_id.clone()),
            process_id: None,
        },
    };
    PushCandidatesForUpdateRequest {
        actor: actor("user-a"),
        target: ProjectionTarget::Thread {
            thread_id: scope.thread_id.clone(),
        },
        projection_scope,
        view: ProjectionViewClass::ProductThread,
        scope: scope.clone(),
        turn_run_id: None,
        reply_target: reply_target("reply-default"),
        kind,
        projection_ref: ProjectionUpdateRef::new("projection:update:1").unwrap(),
    }
}

fn snapshot(scope: &ProjectionScope, cursor: u64) -> ProjectionSnapshot {
    ProjectionSnapshot {
        timeline: ThreadTimeline {
            entries: vec![timeline_entry(
                scope,
                cursor,
                TimelineEntryKind::DispatchRequested,
            )],
        },
        runs: vec![run_status(scope, cursor)],
        capability_activities: vec![capability_activity(scope, cursor)],
        next_cursor: ProjectionCursor::for_scope(scope.clone(), EventCursor::new(cursor)),
        truncated: false,
    }
}

fn snapshot_for_thread(scope: &ProjectionScope, cursor: u64, thread: &str) -> ProjectionSnapshot {
    let mut snapshot = snapshot(scope, cursor);
    let thread_id = Some(ThreadId::new(thread).expect("valid test thread id"));
    for entry in &mut snapshot.timeline.entries {
        entry.thread_id = thread_id.clone();
    }
    for run in &mut snapshot.runs {
        run.thread_id = thread_id.clone();
    }
    for activity in &mut snapshot.capability_activities {
        activity.thread_id = thread_id.clone();
    }
    snapshot
}

fn snapshot_with_activity_thread(
    scope: &ProjectionScope,
    cursor: u64,
    thread: &str,
) -> ProjectionSnapshot {
    let mut snapshot = snapshot(scope, cursor);
    let thread_id = Some(ThreadId::new(thread).unwrap());
    for activity in &mut snapshot.capability_activities {
        activity.thread_id = thread_id.clone();
    }
    snapshot
}

fn replay(scope: &ProjectionScope, cursor: u64, next: u64) -> ProjectionReplay {
    ProjectionReplay {
        updates: vec![timeline_entry(
            scope,
            cursor,
            TimelineEntryKind::DispatchSucceeded,
        )],
        capability_activity_transitions: Vec::new(),
        runs: vec![run_status(scope, next)],
        capability_activities: vec![capability_activity(scope, next)],
        next_cursor: ProjectionCursor::for_scope(scope.clone(), EventCursor::new(next)),
        truncated: false,
    }
}

fn replay_with_error_kind(
    scope: &ProjectionScope,
    cursor: u64,
    next: u64,
    error_kind: &str,
) -> ProjectionReplay {
    let mut replay = replay(scope, cursor, next);
    for entry in &mut replay.updates {
        entry.error_kind = Some(error_kind.to_string());
    }
    for run in &mut replay.runs {
        run.error_kind = Some(error_kind.to_string());
    }
    for activity in &mut replay.capability_activities {
        activity.error_kind = Some(error_kind.to_string());
    }
    replay
}

fn replay_for_thread(
    scope: &ProjectionScope,
    cursor: u64,
    next: u64,
    thread: &str,
) -> ProjectionReplay {
    let mut replay = replay(scope, cursor, next);
    let thread_id = Some(ThreadId::new(thread).unwrap());
    for entry in &mut replay.updates {
        entry.thread_id = thread_id.clone();
    }
    for run in &mut replay.runs {
        run.thread_id = thread_id.clone();
    }
    for activity in &mut replay.capability_activities {
        activity.thread_id = thread_id.clone();
    }
    replay
}

fn replay_with_activity_thread(
    scope: &ProjectionScope,
    cursor: u64,
    next: u64,
    thread: &str,
) -> ProjectionReplay {
    let mut replay = replay(scope, cursor, next);
    let thread_id = Some(ThreadId::new(thread).unwrap());
    for activity in &mut replay.capability_activities {
        activity.thread_id = thread_id.clone();
    }
    replay
}

fn replay_with_activity_transition_thread(
    scope: &ProjectionScope,
    cursor: u64,
    next: u64,
    thread: &str,
) -> ProjectionReplay {
    let mut replay = replay(scope, cursor, next);
    let mut transition = capability_activity(scope, next);
    transition.thread_id = Some(ThreadId::new(thread).expect("valid test thread id"));
    transition.status = CapabilityActivityStatus::Started;
    replay.capability_activity_transitions = vec![transition];
    replay
}

fn timeline_entry(scope: &ProjectionScope, cursor: u64, kind: TimelineEntryKind) -> TimelineEntry {
    TimelineEntry {
        cursor: EventCursor::new(cursor),
        event_id: ironclaw_events::RuntimeEventId::new(),
        timestamp: chrono::Utc::now(),
        kind,
        invocation_id: InvocationId::new(),
        thread_id: scope.read_scope.thread_id.clone(),
        capability_id: CapabilityId::new("script.echo").unwrap(),
        provider: Some(ExtensionId::new("script").unwrap()),
        runtime: Some(RuntimeKind::Script),
        process_id: None,
        output_bytes: Some(12),
        error_kind: None,
        hook_id: None,
        hook_point: None,
        hook_trust_class: None,
        hook_decision: None,
        hook_failure_category: None,
        hook_failure_disposition: None,
    }
}

fn run_status(scope: &ProjectionScope, cursor: u64) -> RunStatusProjection {
    RunStatusProjection {
        invocation_id: InvocationId::new(),
        capability_id: CapabilityId::new("script.echo").unwrap(),
        thread_id: scope.read_scope.thread_id.clone(),
        status: RunProjectionStatus::Completed,
        provider: Some(ExtensionId::new("script").unwrap()),
        runtime: Some(RuntimeKind::Script),
        process_id: None,
        error_kind: None,
        last_cursor: EventCursor::new(cursor),
        updated_at: chrono::Utc::now(),
    }
}

fn capability_activity(scope: &ProjectionScope, cursor: u64) -> CapabilityActivityProjection {
    CapabilityActivityProjection {
        invocation_id: InvocationId::new(),
        run_id: None,
        capability_id: CapabilityId::new("script.echo").unwrap(),
        thread_id: scope.read_scope.thread_id.clone(),
        status: CapabilityActivityStatus::Completed,
        provider: Some(ExtensionId::new("script").unwrap()),
        runtime: Some(RuntimeKind::Script),
        process_id: None,
        output_bytes: Some(12),
        error_kind: None,
        first_cursor: EventCursor::new(cursor),
        last_cursor: EventCursor::new(cursor),
        updated_at: chrono::Utc::now(),
    }
}

fn projection_scope(thread: &str) -> ProjectionScope {
    projection_scope_for("tenant-a", "user-a", thread)
}

fn projection_scope_for(tenant: &str, user: &str, thread: &str) -> ProjectionScope {
    let thread_id = ThreadId::new(thread).unwrap();
    ProjectionScope {
        stream: EventStreamKey::new(
            TenantId::new(tenant).unwrap(),
            UserId::new(user).unwrap(),
            None,
        ),
        read_scope: ReadScope {
            project_id: Some(ProjectId::new("project-a").unwrap()),
            mission_id: None,
            thread_id: Some(thread_id),
            process_id: None,
        },
    }
}

fn turn_scope(thread: &str) -> TurnScope {
    TurnScope::new(
        TenantId::new("tenant-a").unwrap(),
        None,
        Some(ProjectId::new("project-a").unwrap()),
        ThreadId::new(thread).unwrap(),
    )
}

fn actor(user: &str) -> TurnActor {
    TurnActor::new(UserId::new(user).unwrap())
}

fn reply_target(value: &str) -> ReplyTargetBindingRef {
    ReplyTargetBindingRef::new(value).unwrap()
}
