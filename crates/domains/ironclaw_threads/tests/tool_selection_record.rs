//! Contract tests for a conversation's turn-start tool-selection record, run
//! against both `SessionThreadService` backends.

use std::sync::Arc;

use chrono::Utc;
use ironclaw_filesystem::{InMemoryBackend, ScopedFilesystem};
use ironclaw_host_api::{
    ids::{AgentId, CapabilityId, TenantId, ThreadId, UserId},
    mount::{MountGrant, MountPermissions, MountView},
    path::{MountAlias, VirtualPath},
    turn::TurnId,
};
use ironclaw_threads::{
    EnsureThreadRequest, FilesystemSessionThreadService, InMemorySessionThreadService,
    RecordToolSelectionRequest, SelectedTool, SessionThreadError, SessionThreadService,
    TOOL_SELECTION_SCHEMA_VERSION, ThreadScope, ToolSelectionFallbackReason, ToolSelectionRecord,
};

fn backends() -> Vec<(&'static str, Arc<dyn SessionThreadService>)> {
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/threads").expect("alias"),
        VirtualPath::new("/tenants/tenant-a/users/alice/threads").expect("target"),
        MountPermissions::read_write_list_delete(),
    )])
    .expect("mount view");
    let scoped = Arc::new(ScopedFilesystem::with_fixed_view(
        Arc::new(InMemoryBackend::new()),
        mounts,
    ));
    vec![
        (
            "in_memory",
            Arc::new(InMemorySessionThreadService::default()),
        ),
        (
            "filesystem",
            Arc::new(FilesystemSessionThreadService::new(scoped)),
        ),
    ]
}

fn scope(label: &str) -> ThreadScope {
    ThreadScope {
        tenant_id: TenantId::new(format!("tenant-{label}")).unwrap(),
        agent_id: AgentId::new(format!("agent-{label}")).unwrap(),
        project_id: None,
        owner_user_id: Some(UserId::new(format!("user-{label}")).unwrap()),
        mission_id: None,
    }
}

fn record(names: &[&str]) -> ToolSelectionRecord {
    ToolSelectionRecord {
        schema_version: TOOL_SELECTION_SCHEMA_VERSION,
        turn_id: TurnId::new(),
        selected: names
            .iter()
            .map(|name| SelectedTool {
                capability_id: CapabilityId::new(*name).unwrap(),
                score: 0.75,
            })
            .collect(),
        scorer: Some("jev:jev-latest".to_string()),
        fallback_reason: None,
        recorded_at: Utc::now(),
    }
}

async fn ensure_thread(service: &dyn SessionThreadService, scope: &ThreadScope) -> ThreadId {
    service
        .ensure_thread(EnsureThreadRequest {
            scope: scope.clone(),
            thread_id: Some(ThreadId::new("thread-selection").unwrap()),
            created_by_actor_id: "actor-a".into(),
            title: None,
            metadata_json: None,
        })
        .await
        .unwrap()
        .thread_id
}

#[tokio::test]
async fn a_selection_is_recorded_once_and_read_back() {
    for (backend, service) in backends() {
        let scope = scope("owner");
        let thread_id = ensure_thread(service.as_ref(), &scope).await;
        assert_eq!(
            service
                .read_tool_selection(&scope, &thread_id)
                .await
                .unwrap(),
            None,
            "{backend}: nothing is recorded before the first selection"
        );

        let first = record(&["gmail.send", "slack.post"]);
        let stored = service
            .record_tool_selection(RecordToolSelectionRequest {
                scope: scope.clone(),
                thread_id: thread_id.clone(),
                record: first.clone(),
            })
            .await
            .unwrap();
        assert_eq!(stored, first, "{backend}");

        // A second writer is handed the stored record, not its own.
        let second = service
            .record_tool_selection(RecordToolSelectionRequest {
                scope: scope.clone(),
                thread_id: thread_id.clone(),
                record: record(&["github.create_issue"]),
            })
            .await
            .unwrap();
        assert_eq!(second, first, "{backend}: the first record wins");
        assert_eq!(
            service
                .read_tool_selection(&scope, &thread_id)
                .await
                .unwrap(),
            Some(first),
            "{backend}"
        );
    }
}

#[tokio::test]
async fn concurrent_writers_are_all_handed_the_one_stored_record() {
    for (backend, service) in backends() {
        let scope = scope("owner");
        let thread_id = ensure_thread(service.as_ref(), &scope).await;
        let write = |names: &'static [&'static str]| {
            service.record_tool_selection(RecordToolSelectionRequest {
                scope: scope.clone(),
                thread_id: thread_id.clone(),
                record: record(names),
            })
        };
        let (left, right) = tokio::join!(write(&["gmail.send"]), write(&["slack.post"]));
        let (left, right) = (left.unwrap(), right.unwrap());
        assert_eq!(left, right, "{backend}: both writers see one record");
        assert_eq!(
            service
                .read_tool_selection(&scope, &thread_id)
                .await
                .unwrap(),
            Some(left),
            "{backend}"
        );

        // A thread that was never created has no record to read or write.
        let missing = ThreadId::new("thread-never-created").unwrap();
        assert!(
            service.read_tool_selection(&scope, &missing).await.is_err(),
            "{backend}"
        );
        let refused = service
            .record_tool_selection(RecordToolSelectionRequest {
                scope: scope.clone(),
                thread_id: missing,
                record: record(&["gmail.send"]),
            })
            .await;
        assert!(refused.is_err(), "{backend}");
    }
}

#[tokio::test]
async fn a_selection_is_scoped_to_its_thread_and_survives_only_its_incarnation() {
    for (backend, service) in backends() {
        let scope = scope("owner");
        let thread_id = ensure_thread(service.as_ref(), &scope).await;
        let mut fallback = record(&[]);
        fallback.scorer = None;
        fallback.fallback_reason = Some(ToolSelectionFallbackReason::Timeout);
        service
            .record_tool_selection(RecordToolSelectionRequest {
                scope: scope.clone(),
                thread_id: thread_id.clone(),
                record: fallback,
            })
            .await
            .unwrap();

        let other = self::scope("intruder");
        assert!(
            matches!(
                service.read_tool_selection(&other, &thread_id).await,
                Err(SessionThreadError::UnknownThread { .. })
            ),
            "{backend}: another scope cannot read the record"
        );
        assert!(
            matches!(
                service
                    .record_tool_selection(RecordToolSelectionRequest {
                        scope: other,
                        thread_id: thread_id.clone(),
                        record: record(&["gmail.send"]),
                    })
                    .await,
                Err(SessionThreadError::UnknownThread { .. })
            ),
            "{backend}: another scope cannot write the record"
        );

        // A recreated thread id is a new conversation and selects afresh.
        service.delete_thread(&scope, &thread_id).await.unwrap();
        let recreated = ensure_thread(service.as_ref(), &scope).await;
        assert_eq!(
            service
                .read_tool_selection(&scope, &recreated)
                .await
                .unwrap(),
            None,
            "{backend}: a recreated thread does not read its predecessor's selection"
        );
    }
}

#[tokio::test]
async fn an_invalid_record_is_refused_before_it_is_stored() {
    for (backend, service) in backends() {
        let scope = scope("owner");
        let thread_id = ensure_thread(service.as_ref(), &scope).await;
        let result = service
            .record_tool_selection(RecordToolSelectionRequest {
                scope: scope.clone(),
                thread_id: thread_id.clone(),
                record: record(&["gmail.send", "gmail.send"]),
            })
            .await;
        assert!(
            matches!(result, Err(SessionThreadError::InvalidToolSelection { .. })),
            "{backend}: {result:?}"
        );
        assert_eq!(
            service
                .read_tool_selection(&scope, &thread_id)
                .await
                .unwrap(),
            None,
            "{backend}"
        );
    }
}
