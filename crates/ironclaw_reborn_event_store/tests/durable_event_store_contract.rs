use std::path::Path;

use ironclaw_events::{
    EventCursor, EventError, EventStreamKey, ReadScope, RuntimeEvent, RuntimeEventKind,
};
use ironclaw_host_api::{
    ActionResultSummary, ActionSummary, AgentId, AuditEnvelope, AuditEventId, AuditStage,
    CapabilityId, CorrelationId, DecisionSummary, ExtensionId, InvocationId, ProjectId,
    ResourceScope, RuntimeKind, TenantId, UserId,
};
use ironclaw_reborn_event_store::{
    RebornEventStoreConfig, RebornProfile, build_reborn_event_stores,
};
#[cfg(feature = "postgres")]
use secrecy::SecretString;

fn capability_id() -> CapabilityId {
    CapabilityId::new("demo.echo").expect("capability id")
}

fn extension_id() -> ExtensionId {
    ExtensionId::new("demo").expect("extension id")
}

fn scope_for(user: &str, project: &str) -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new("default").expect("tenant id"),
        user_id: UserId::new(user).expect("user id"),
        agent_id: Some(AgentId::new("default").expect("agent id")),
        project_id: Some(ProjectId::new(project).expect("project id")),
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    }
}

fn audit_record(scope: &ResourceScope, status: &str) -> AuditEnvelope {
    AuditEnvelope {
        event_id: AuditEventId::new(),
        correlation_id: CorrelationId::new(),
        stage: AuditStage::After,
        timestamp: chrono::Utc::now(),
        tenant_id: scope.tenant_id.clone(),
        user_id: scope.user_id.clone(),
        agent_id: scope.agent_id.clone(),
        project_id: scope.project_id.clone(),
        mission_id: scope.mission_id.clone(),
        thread_id: scope.thread_id.clone(),
        invocation_id: scope.invocation_id,
        process_id: None,
        approval_request_id: None,
        extension_id: Some(extension_id()),
        action: ActionSummary {
            kind: "dispatch".to_string(),
            target: Some(capability_id().as_str().to_string()),
            effects: Vec::new(),
        },
        decision: DecisionSummary {
            kind: "allow".to_string(),
            reason: None,
            actor: None,
        },
        result: Some(ActionResultSummary {
            success: true,
            status: Some(status.to_string()),
            output_bytes: Some(12),
        }),
    }
}

#[tokio::test]
async fn jsonl_runtime_log_survives_rebuild_and_preserves_filtered_cursor_semantics() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().join("event-store");
    let scope_a = scope_for("alice", "project-a");
    let scope_b = scope_for("alice", "project-b");
    let stream = EventStreamKey::from_scope(&scope_a);

    let stores = build_reborn_event_stores(
        RebornProfile::LocalDev,
        RebornEventStoreConfig::Jsonl {
            root: root.clone(),
            accept_single_node_durable: false,
        },
    )
    .await
    .expect("jsonl stores");

    stores
        .events
        .append(RuntimeEvent::dispatch_requested(
            scope_a.clone(),
            capability_id(),
        ))
        .await
        .expect("append project a 1");
    stores
        .events
        .append(RuntimeEvent::dispatch_requested(
            scope_b.clone(),
            capability_id(),
        ))
        .await
        .expect("append project b");
    stores
        .events
        .append(RuntimeEvent::dispatch_succeeded(
            scope_a.clone(),
            capability_id(),
            extension_id(),
            RuntimeKind::Script,
            7,
        ))
        .await
        .expect("append project a 2");
    drop(stores);

    let stores = build_reborn_event_stores(
        RebornProfile::LocalDev,
        RebornEventStoreConfig::Jsonl {
            root,
            accept_single_node_durable: false,
        },
    )
    .await
    .expect("jsonl stores after restart");

    let project_a = ReadScope {
        project_id: scope_a.project_id.clone(),
        ..ReadScope::default()
    };
    let first = stores
        .events
        .read_after_cursor(&stream, &project_a, None, 1)
        .await
        .expect("first limited replay");
    assert_eq!(first.entries.len(), 1);
    assert_eq!(first.entries[0].cursor, EventCursor::new(1));
    assert_eq!(first.next_cursor, EventCursor::new(1));

    let second = stores
        .events
        .read_after_cursor(&stream, &project_a, Some(first.next_cursor), 10)
        .await
        .expect("second replay skips filtered record");
    assert_eq!(second.entries.len(), 1);
    assert_eq!(second.entries[0].cursor, EventCursor::new(3));
    assert_eq!(
        second.entries[0].record.kind,
        RuntimeEventKind::DispatchSucceeded
    );
    assert_eq!(second.next_cursor, EventCursor::new(3));

    let project_b = ReadScope {
        project_id: scope_b.project_id.clone(),
        ..ReadScope::default()
    };
    let replay_b = stores
        .events
        .read_after_cursor(&stream, &project_b, None, 10)
        .await
        .expect("project b replay");
    assert_eq!(replay_b.entries.len(), 1);
    assert_eq!(replay_b.entries[0].cursor, EventCursor::new(2));
}

#[tokio::test]
async fn jsonl_runtime_log_rejects_zero_limit_and_foreign_future_cursor() {
    let temp = tempfile::tempdir().expect("tempdir");
    let stores = build_reborn_event_stores(
        RebornProfile::LocalDev,
        RebornEventStoreConfig::Jsonl {
            root: temp.path().join("event-store"),
            accept_single_node_durable: false,
        },
    )
    .await
    .expect("jsonl stores");
    let scope = scope_for("alice", "project-a");
    let stream = EventStreamKey::from_scope(&scope);

    let zero = stores
        .events
        .read_after_cursor(&stream, &ReadScope::any(), None, 0)
        .await;
    assert!(matches!(zero, Err(EventError::InvalidReplayRequest { .. })));

    let future = stores
        .events
        .read_after_cursor(&stream, &ReadScope::any(), Some(EventCursor::new(99)), 10)
        .await;
    match future {
        Err(EventError::ReplayGap {
            requested,
            earliest,
        }) => {
            assert_eq!(requested, EventCursor::new(99));
            assert_eq!(earliest, EventCursor::origin());
        }
        other => panic!("expected replay gap for future cursor, got {other:?}"),
    }
}

#[tokio::test]
async fn jsonl_audit_log_survives_rebuild_and_filters_scope() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().join("event-store");
    let scope_a = scope_for("alice", "project-a");
    let scope_b = scope_for("alice", "project-b");
    let stream = EventStreamKey::from_scope(&scope_a);

    let stores = build_reborn_event_stores(
        RebornProfile::LocalDev,
        RebornEventStoreConfig::Jsonl {
            root: root.clone(),
            accept_single_node_durable: false,
        },
    )
    .await
    .expect("jsonl stores");

    stores
        .audit
        .append(audit_record(&scope_a, "project-a"))
        .await
        .expect("append project a audit");
    stores
        .audit
        .append(audit_record(&scope_b, "project-b"))
        .await
        .expect("append project b audit");
    drop(stores);

    let stores = build_reborn_event_stores(
        RebornProfile::LocalDev,
        RebornEventStoreConfig::Jsonl {
            root,
            accept_single_node_durable: false,
        },
    )
    .await
    .expect("jsonl stores after restart");
    let project_a = ReadScope {
        project_id: scope_a.project_id.clone(),
        ..ReadScope::default()
    };
    let replay = stores
        .audit
        .read_after_cursor(&stream, &project_a, None, 10)
        .await
        .expect("audit replay");

    assert_eq!(replay.entries.len(), 1);
    assert_eq!(replay.entries[0].cursor, EventCursor::new(1));
    assert_eq!(
        replay.entries[0].record.result.as_ref().unwrap().status,
        Some("project-a".to_string())
    );
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_runtime_and_audit_logs_survive_rebuild_with_filtered_cursor_semantics() {
    let temp = tempfile::tempdir().expect("tempdir");
    let db_path = temp.path().join("event-store.db");
    let scope_a = scope_for("libsql-alice", "project-a");
    let scope_b = scope_for("libsql-alice", "project-b");
    let stream = EventStreamKey::from_scope(&scope_a);

    let stores = build_reborn_event_stores(
        RebornProfile::Production,
        RebornEventStoreConfig::Libsql {
            path_or_url: db_path.display().to_string(),
            auth_token: None,
        },
    )
    .await
    .expect("libsql stores");

    stores
        .events
        .append(RuntimeEvent::dispatch_requested(
            scope_a.clone(),
            capability_id(),
        ))
        .await
        .expect("append project a 1");
    stores
        .events
        .append(RuntimeEvent::dispatch_requested(
            scope_b.clone(),
            capability_id(),
        ))
        .await
        .expect("append project b");
    stores
        .events
        .append(RuntimeEvent::dispatch_succeeded(
            scope_a.clone(),
            capability_id(),
            extension_id(),
            RuntimeKind::Script,
            7,
        ))
        .await
        .expect("append project a 2");
    stores
        .audit
        .append(audit_record(&scope_a, "project-a"))
        .await
        .expect("append project a audit");
    stores
        .audit
        .append(audit_record(&scope_b, "project-b"))
        .await
        .expect("append project b audit");
    drop(stores);

    let stores = build_reborn_event_stores(
        RebornProfile::Production,
        RebornEventStoreConfig::Libsql {
            path_or_url: db_path.display().to_string(),
            auth_token: None,
        },
    )
    .await
    .expect("libsql stores after restart");

    let project_a = ReadScope {
        project_id: scope_a.project_id.clone(),
        ..ReadScope::default()
    };
    let first = stores
        .events
        .read_after_cursor(&stream, &project_a, None, 1)
        .await
        .expect("first limited replay");
    assert_eq!(first.entries.len(), 1);
    assert_eq!(first.entries[0].cursor, EventCursor::new(1));

    let second = stores
        .events
        .read_after_cursor(&stream, &project_a, Some(first.next_cursor), 10)
        .await
        .expect("second replay skips filtered record");
    assert_eq!(second.entries.len(), 1);
    assert_eq!(second.entries[0].cursor, EventCursor::new(3));
    assert_eq!(second.next_cursor, EventCursor::new(3));

    let audit_replay = stores
        .audit
        .read_after_cursor(&stream, &project_a, None, 10)
        .await
        .expect("audit replay");
    assert_eq!(audit_replay.entries.len(), 1);
    assert_eq!(
        audit_replay.entries[0]
            .record
            .result
            .as_ref()
            .unwrap()
            .status,
        Some("project-a".to_string())
    );
}

#[cfg(feature = "postgres")]
#[tokio::test]
async fn postgres_runtime_and_audit_logs_survive_rebuild_with_filtered_cursor_semantics() {
    let Ok(url) = std::env::var("IRONCLAW_REBORN_EVENT_STORE_POSTGRES_URL") else {
        eprintln!(
            "skipping postgres event-store contract: IRONCLAW_REBORN_EVENT_STORE_POSTGRES_URL not set"
        );
        return;
    };
    let suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    let scope_a = scope_for(&format!("postgres-alice-{suffix}"), "project-a");
    let scope_b = scope_for(&format!("postgres-alice-{suffix}"), "project-b");
    let stream = EventStreamKey::from_scope(&scope_a);

    let stores = build_reborn_event_stores(
        RebornProfile::Production,
        RebornEventStoreConfig::Postgres {
            url: SecretString::new(url.clone().into_boxed_str()),
        },
    )
    .await
    .expect("postgres stores");

    stores
        .events
        .append(RuntimeEvent::dispatch_requested(
            scope_a.clone(),
            capability_id(),
        ))
        .await
        .expect("append project a 1");
    stores
        .events
        .append(RuntimeEvent::dispatch_requested(
            scope_b.clone(),
            capability_id(),
        ))
        .await
        .expect("append project b");
    stores
        .events
        .append(RuntimeEvent::dispatch_succeeded(
            scope_a.clone(),
            capability_id(),
            extension_id(),
            RuntimeKind::Script,
            7,
        ))
        .await
        .expect("append project a 2");
    stores
        .audit
        .append(audit_record(&scope_a, "project-a"))
        .await
        .expect("append project a audit");
    stores
        .audit
        .append(audit_record(&scope_b, "project-b"))
        .await
        .expect("append project b audit");
    drop(stores);

    let stores = build_reborn_event_stores(
        RebornProfile::Production,
        RebornEventStoreConfig::Postgres {
            url: SecretString::new(url.into_boxed_str()),
        },
    )
    .await
    .expect("postgres stores after reconnect");

    let project_a = ReadScope {
        project_id: scope_a.project_id.clone(),
        ..ReadScope::default()
    };
    let replay = stores
        .events
        .read_after_cursor(&stream, &project_a, None, 10)
        .await
        .expect("runtime replay");
    assert_eq!(replay.entries.len(), 2);
    assert_eq!(replay.entries[0].cursor, EventCursor::new(1));
    assert_eq!(replay.entries[1].cursor, EventCursor::new(3));

    let audit_replay = stores
        .audit
        .read_after_cursor(&stream, &project_a, None, 10)
        .await
        .expect("audit replay");
    assert_eq!(audit_replay.entries.len(), 1);
    assert_eq!(
        audit_replay.entries[0]
            .record
            .result
            .as_ref()
            .unwrap()
            .status,
        Some("project-a".to_string())
    );
}

#[tokio::test]
async fn jsonl_runtime_records_do_not_serialize_raw_error_sentinels() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().join("event-store");
    let stores = build_reborn_event_stores(
        RebornProfile::LocalDev,
        RebornEventStoreConfig::Jsonl {
            root: root.clone(),
            accept_single_node_durable: false,
        },
    )
    .await
    .expect("jsonl stores");
    let scope = scope_for("alice", "project-a");
    let raw_error = "RAW_SECRET_SENTINEL_3162_sk-live /tmp/HOST_PATH_SENTINEL_3162/output.log";

    stores
        .events
        .append(RuntimeEvent::dispatch_failed(
            scope,
            capability_id(),
            Some(extension_id()),
            Some(RuntimeKind::Script),
            raw_error,
        ))
        .await
        .expect("append failed event");

    let serialized = String::from_utf8(collect_file_bytes(&root)).expect("utf8 jsonl");
    for forbidden in [
        "RAW_SECRET_SENTINEL_3162",
        "HOST_PATH_SENTINEL_3162",
        "sk-live",
        "/tmp/",
        "output.log",
    ] {
        assert!(
            !serialized.contains(forbidden),
            "serialized JSONL leaked {forbidden}: {serialized}"
        );
    }
    assert!(serialized.contains("Unclassified"));
}

fn collect_file_bytes(path: &Path) -> Vec<u8> {
    let mut bytes = Vec::new();
    collect_file_bytes_inner(path, &mut bytes);
    bytes
}

fn collect_file_bytes_inner(path: &Path, bytes: &mut Vec<u8>) {
    if path.is_file() {
        bytes.extend(std::fs::read(path).expect("read file"));
        return;
    }
    for entry in std::fs::read_dir(path).expect("read dir") {
        let entry = entry.expect("dir entry");
        collect_file_bytes_inner(&entry.path(), bytes);
    }
}

// Regression for review comment 3178548646: when a record matches the filter
// but the *next* record is filtered out, `next_cursor` must advance past the
// filtered tail so the caller does not rescan it indefinitely.
#[tokio::test]
async fn jsonl_replay_advances_next_cursor_past_trailing_filtered_records() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().join("event-store");
    let scope_a = scope_for("alice", "project-a");
    let scope_b = scope_for("alice", "project-b");
    let stream = EventStreamKey::from_scope(&scope_a);

    let stores = build_reborn_event_stores(
        RebornProfile::LocalDev,
        RebornEventStoreConfig::Jsonl {
            root,
            accept_single_node_durable: false,
        },
    )
    .await
    .expect("jsonl stores");

    // Cursor 1 matches filter (project-a), cursor 2 does not (project-b).
    stores
        .events
        .append(RuntimeEvent::dispatch_requested(
            scope_a.clone(),
            capability_id(),
        ))
        .await
        .expect("append a1");
    stores
        .events
        .append(RuntimeEvent::dispatch_requested(
            scope_b.clone(),
            capability_id(),
        ))
        .await
        .expect("append b1");

    let project_a = ReadScope {
        project_id: scope_a.project_id.clone(),
        ..ReadScope::default()
    };
    let replay = stores
        .events
        .read_after_cursor(&stream, &project_a, None, 10)
        .await
        .expect("replay");
    assert_eq!(replay.entries.len(), 1);
    assert_eq!(replay.entries[0].cursor, EventCursor::new(1));
    // `next_cursor` must be the highest scanned cursor (2), not the last
    // matched cursor (1), otherwise the next replay would rescan cursor 2.
    assert_eq!(
        replay.next_cursor,
        EventCursor::new(2),
        "next_cursor must advance past trailing filtered record"
    );

    // Confirm the next call does not return the filtered-out record again.
    let follow_up = stores
        .events
        .read_after_cursor(&stream, &project_a, Some(replay.next_cursor), 10)
        .await
        .expect("follow up replay");
    assert!(follow_up.entries.is_empty());
}

// Regression for review comment 3178548701: two `JsonlStore` instances
// pointing at the same root must serialise cursor assignment via the OS-level
// file lock, even though they hold independent in-process Tokio mutexes.
#[tokio::test]
async fn jsonl_concurrent_appenders_emit_monotonic_cursors_through_file_lock() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().join("event-store");
    let scope = scope_for("alice", "project-a");
    let stream = EventStreamKey::from_scope(&scope);

    // Two independent store instances simulate two processes sharing the
    // same JSONL root. They do NOT share an in-process Tokio mutex.
    let stores_one = build_reborn_event_stores(
        RebornProfile::LocalDev,
        RebornEventStoreConfig::Jsonl {
            root: root.clone(),
            accept_single_node_durable: false,
        },
    )
    .await
    .expect("stores one");
    let stores_two = build_reborn_event_stores(
        RebornProfile::LocalDev,
        RebornEventStoreConfig::Jsonl {
            root: root.clone(),
            accept_single_node_durable: false,
        },
    )
    .await
    .expect("stores two");

    let mut handles = Vec::new();
    for stores in [stores_one, stores_two] {
        let scope = scope.clone();
        let events = stores.events.clone();
        handles.push(tokio::spawn(async move {
            for _ in 0..16 {
                events
                    .append(RuntimeEvent::dispatch_requested(
                        scope.clone(),
                        capability_id(),
                    ))
                    .await
                    .expect("append");
            }
        }));
    }
    for handle in handles {
        handle.await.expect("join");
    }

    // Open a fresh reader. If the two appenders raced and both observed the
    // same prior tail, the JSONL file would contain duplicate cursors and
    // `parse_jsonl_entries` would error out via `read_after_cursor` with an
    // "invalid cursor sequence" durable-log error.
    let reader = build_reborn_event_stores(
        RebornProfile::LocalDev,
        RebornEventStoreConfig::Jsonl {
            root,
            accept_single_node_durable: false,
        },
    )
    .await
    .expect("reader");
    let replay = reader
        .events
        .read_after_cursor(&stream, &ReadScope::any(), None, 100)
        .await
        .expect("replay must observe a monotonically-sequenced stream");
    assert_eq!(replay.entries.len(), 32, "all 32 appends must be visible");
    for (index, entry) in replay.entries.iter().enumerate() {
        assert_eq!(entry.cursor, EventCursor::new((index + 1) as u64));
    }
}

// Regression for review comment 3178548670: a small `limit` against a large
// JSONL stream must not load or parse the entire file. We assert this by
// truncating the JSONL file after a known prefix and confirming that the
// reader still returns the requested limited prefix without erroring on the
// truncated-tail garbage that follows.
#[tokio::test]
async fn jsonl_bounded_replay_does_not_parse_the_whole_file() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().join("event-store");
    let scope = scope_for("alice", "project-a");
    let stream = EventStreamKey::from_scope(&scope);

    let stores = build_reborn_event_stores(
        RebornProfile::LocalDev,
        RebornEventStoreConfig::Jsonl {
            root: root.clone(),
            accept_single_node_durable: false,
        },
    )
    .await
    .expect("stores");
    for _ in 0..3 {
        stores
            .events
            .append(RuntimeEvent::dispatch_requested(
                scope.clone(),
                capability_id(),
            ))
            .await
            .expect("append");
    }
    drop(stores);

    // Append garbage that would fail JSON parsing if the reader scanned past
    // its first matched record. With streaming + early-exit on `limit`,
    // a `limit = 1` request returns successfully without ever touching the
    // garbage line.
    let stream_path = std::fs::read_dir(root.join("events"))
        .expect("events dir")
        .next()
        .expect("tenant dir")
        .expect("dir entry")
        .path();
    let stream_path = std::fs::read_dir(&stream_path)
        .expect("user dir")
        .next()
        .expect("user")
        .expect("user entry")
        .path();
    let file = std::fs::read_dir(&stream_path)
        .expect("agent dir")
        .next()
        .expect("agent")
        .expect("agent entry")
        .path();
    use std::io::Write;
    let mut handle = std::fs::OpenOptions::new()
        .append(true)
        .open(&file)
        .expect("open jsonl");
    handle
        .write_all(b"{\"cursor\":4,\"record\": THIS IS NOT JSON\n")
        .expect("write garbage");
    handle.flush().expect("flush");
    drop(handle);

    let stores = build_reborn_event_stores(
        RebornProfile::LocalDev,
        RebornEventStoreConfig::Jsonl {
            root,
            accept_single_node_durable: false,
        },
    )
    .await
    .expect("reopen");
    let bounded = stores
        .events
        .read_after_cursor(&stream, &ReadScope::any(), None, 1)
        .await
        .expect("bounded replay must not parse trailing garbage");
    assert_eq!(bounded.entries.len(), 1);
    assert_eq!(bounded.entries[0].cursor, EventCursor::new(1));
}
