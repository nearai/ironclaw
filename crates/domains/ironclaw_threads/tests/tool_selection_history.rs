//! Contract for the append-only tool-selection history, run against both
//! backends: the in-memory service and the filesystem service over the
//! scoped filesystem (the same seam PostgreSQL/libSQL/local mounts serve).

use std::sync::Arc;

use chrono::Utc;
use ironclaw_host_api::{
    ids::{AgentId, ProjectId, TenantId, ThreadId, UserId},
    path::ScopedPath,
    turn::TurnId,
};
use ironclaw_threads::{
    AppendToolSelectionEntryRequest, EnsureThreadRequest, FilesystemSessionThreadService,
    InMemorySessionThreadService, ModelCallMark, RecordToolSelectionActivityRequest,
    SessionThreadError, SessionThreadService, ThreadScope, ToolSelectionActivityUpdate,
    ToolSelectionEffectiveFrom, ToolSelectionEntry, ToolSelectionHistory, ToolSelectionReason,
    ToolSelectionScore,
};

fn scope(agent: &str) -> ThreadScope {
    ThreadScope {
        tenant_id: TenantId::new("tenant").expect("tenant"),
        agent_id: AgentId::new(agent).expect("agent"),
        project_id: Some(ProjectId::new("project").expect("project")),
        owner_user_id: Some(UserId::new("user").expect("user")),
        mission_id: None,
    }
}

fn scoped_filesystem()
-> Arc<ironclaw_filesystem::ScopedFilesystem<ironclaw_filesystem::InMemoryBackend>> {
    use ironclaw_filesystem::{InMemoryBackend, ScopedFilesystem};
    use ironclaw_host_api::{
        mount::{MountGrant, MountPermissions, MountView},
        path::{MountAlias, VirtualPath},
    };

    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/threads").expect("alias"),
        VirtualPath::new("/tenants/tenant/users/user/threads").expect("virtual path"),
        MountPermissions::read_write_list_delete(),
    )])
    .expect("mounts");
    Arc::new(ScopedFilesystem::with_fixed_view(
        Arc::new(InMemoryBackend::new()),
        mounts,
    ))
}

async fn ensure(service: &dyn SessionThreadService, scope: &ThreadScope, thread_id: &ThreadId) {
    service
        .ensure_thread(EnsureThreadRequest {
            scope: scope.clone(),
            thread_id: Some(thread_id.clone()),
            created_by_actor_id: "test".to_string(),
            title: None,
            metadata_json: None,
        })
        .await
        .expect("thread");
}

fn entry(reason: ToolSelectionReason, advertised: &[&str]) -> ToolSelectionEntry {
    ToolSelectionEntry {
        effective_from: ToolSelectionEffectiveFrom {
            turn_id: TurnId::new(),
            message_sequence: Some(1),
        },
        reason,
        advertised: advertised.iter().map(|name| name.to_string()).collect(),
        scores: advertised
            .iter()
            .filter(|name| name.starts_with("github"))
            .map(|name| ToolSelectionScore {
                name: name.to_string(),
                score: 0.5,
            })
            .collect(),
        ranker_version: Some("bounded-bm25f-v1".to_string()),
        tool_search_description: Some("Search the tools not listed.".to_string()),
        fallback_reason: None,
        recorded_at: Utc::now(),
    }
}

fn append(
    scope: &ThreadScope,
    thread_id: &ThreadId,
    expected_entries: usize,
    entry: ToolSelectionEntry,
) -> AppendToolSelectionEntryRequest {
    AppendToolSelectionEntryRequest {
        scope: scope.clone(),
        thread_id: thread_id.clone(),
        expected_entries,
        entry,
    }
}

/// The append-only contract every backend keeps: nothing before the first
/// entry, `initial` first, stale writers refused, `revoked` removes a tool,
/// a re-selection changes the list, and the whole history reads back in
/// order.
async fn assert_append_only_contract(service: &dyn SessionThreadService) {
    let scope = scope("agent");
    let thread_id = ThreadId::new("thread-history").expect("thread");
    ensure(service, &scope, &thread_id).await;

    assert!(
        service
            .read_tool_selection_history(&scope, &thread_id)
            .await
            .expect("read")
            .is_none(),
        "a conversation has no history before its first selection"
    );

    let initial = entry(
        ToolSelectionReason::Initial,
        &["github__create_issue", "result_read", "tool_search"],
    );
    let history = service
        .append_tool_selection_entry(append(&scope, &thread_id, 0, initial.clone()))
        .await
        .expect("initial append");
    assert_eq!(history.entries, vec![initial.clone()]);

    let stale = service
        .append_tool_selection_entry(append(
            &scope,
            &thread_id,
            0,
            entry(ToolSelectionReason::Initial, &["tool_search"]),
        ))
        .await
        .expect_err("a second writer that missed the first entry");
    assert!(
        matches!(
            stale,
            SessionThreadError::ToolSelectionHistoryConflict {
                expected_entries: 0,
                actual_entries: 1,
                ..
            }
        ),
        "{stale:?}"
    );

    let widening = service
        .append_tool_selection_entry(append(
            &scope,
            &thread_id,
            1,
            entry(
                ToolSelectionReason::Revoked,
                &[
                    "github__create_issue",
                    "github__get_repo",
                    "result_read",
                    "tool_search",
                ],
            ),
        ))
        .await
        .expect_err("a revocation that removes nothing");
    assert_eq!(widening.kind_name(), "invalid_tool_selection");
    let unchanged = service
        .append_tool_selection_entry(append(
            &scope,
            &thread_id,
            1,
            entry(
                ToolSelectionReason::CacheCold,
                &["github__create_issue", "result_read", "tool_search"],
            ),
        ))
        .await
        .expect_err("a re-selection that changes nothing");
    assert_eq!(unchanged.kind_name(), "invalid_tool_selection");

    let revoked = entry(
        ToolSelectionReason::Revoked,
        &["result_read", "tool_search"],
    );
    service
        .append_tool_selection_entry(append(&scope, &thread_id, 1, revoked.clone()))
        .await
        .expect("revocation append");

    let read: ToolSelectionHistory = service
        .read_tool_selection_history(&scope, &thread_id)
        .await
        .expect("read")
        .expect("history");
    assert_eq!(read.entries, vec![initial, revoked]);
    assert_eq!(read.thread_id, thread_id);

    let foreign = service
        .read_tool_selection_history(&self::scope("other-agent"), &thread_id)
        .await
        .expect_err("cross-scope read");
    assert!(matches!(foreign, SessionThreadError::UnknownThread { .. }));
    let missing = service
        .read_tool_selection_history(&scope, &ThreadId::new("missing").expect("thread"))
        .await
        .expect_err("missing thread");
    assert!(matches!(missing, SessionThreadError::UnknownThread { .. }));
}

/// A recreated thread id is a new conversation: it never reuses the deleted
/// conversation's selection.
async fn assert_recreated_thread_selects_afresh(service: &dyn SessionThreadService) {
    let scope = scope("agent");
    let thread_id = ThreadId::new("thread-recreated").expect("thread");
    ensure(service, &scope, &thread_id).await;
    service
        .append_tool_selection_entry(append(
            &scope,
            &thread_id,
            0,
            entry(ToolSelectionReason::Initial, &["tool_search"]),
        ))
        .await
        .expect("initial append");
    service
        .delete_thread(&scope, &thread_id)
        .await
        .expect("delete");
    ensure(service, &scope, &thread_id).await;
    assert!(
        service
            .read_tool_selection_history(&scope, &thread_id)
            .await
            .expect("read")
            .is_none(),
        "a recreated thread id must not replay the deleted conversation's tools"
    );
    service
        .append_tool_selection_entry(append(
            &scope,
            &thread_id,
            0,
            entry(ToolSelectionReason::Initial, &["tool_call"]),
        ))
        .await
        .expect("the new conversation records its own initial entry");
}

#[tokio::test]
async fn in_memory_history_is_append_only() {
    assert_append_only_contract(&InMemorySessionThreadService::default()).await;
}

#[tokio::test]
async fn filesystem_history_is_append_only() {
    assert_append_only_contract(&FilesystemSessionThreadService::new(scoped_filesystem())).await;
}

#[tokio::test]
async fn in_memory_recreated_thread_selects_afresh() {
    assert_recreated_thread_selects_afresh(&InMemorySessionThreadService::default()).await;
}

#[tokio::test]
async fn filesystem_recreated_thread_selects_afresh() {
    assert_recreated_thread_selects_afresh(&FilesystemSessionThreadService::new(
        scoped_filesystem(),
    ))
    .await;
}

/// Two writers racing the first selection: exactly one `initial` entry lands
/// and the loser is told to re-read, on both backends.
#[tokio::test]
async fn concurrent_initial_selections_record_exactly_one_entry() {
    let services: [Arc<dyn SessionThreadService>; 2] = [
        Arc::new(InMemorySessionThreadService::default()),
        Arc::new(FilesystemSessionThreadService::new(scoped_filesystem())),
    ];
    for service in services {
        let scope = scope("agent");
        let thread_id = ThreadId::new("thread-race").expect("thread");
        ensure(service.as_ref(), &scope, &thread_id).await;
        let attempts = (0..4).map(|index| {
            let service = Arc::clone(&service);
            let request = append(
                &scope,
                &thread_id,
                0,
                entry(
                    ToolSelectionReason::Initial,
                    &[["tool_search", "tool_call", "tool_describe", "result_read"][index]],
                ),
            );
            async move { service.append_tool_selection_entry(request).await }
        });
        let outcomes = futures::future::join_all(attempts).await;
        let winners = outcomes.iter().filter(|outcome| outcome.is_ok()).count();
        assert_eq!(winners, 1, "exactly one initial selection wins");
        for outcome in outcomes.iter().filter_map(|outcome| outcome.as_ref().err()) {
            assert!(
                matches!(
                    outcome,
                    SessionThreadError::ToolSelectionHistoryConflict { .. }
                ),
                "{outcome:?}"
            );
        }
        let history = service
            .read_tool_selection_history(&scope, &thread_id)
            .await
            .expect("read")
            .expect("history");
        assert_eq!(history.entries.len(), 1);
    }
}

/// The history is stored outside the thread root, so deleting the thread
/// keeps it (LLM-facing data is never deleted), and it survives a service
/// restart without being recomputed.
#[tokio::test]
async fn filesystem_history_survives_restart_and_thread_deletion() {
    let filesystem = scoped_filesystem();
    let scope = scope("agent");
    let thread_id = ThreadId::new("retained-thread").expect("thread");
    let initial = entry(
        ToolSelectionReason::Initial,
        &["github__get_repo", "tool_search"],
    );
    {
        let service = FilesystemSessionThreadService::new(Arc::clone(&filesystem));
        ensure(&service, &scope, &thread_id).await;
        service
            .append_tool_selection_entry(append(&scope, &thread_id, 0, initial.clone()))
            .await
            .expect("initial append");
    }

    let restarted = FilesystemSessionThreadService::new(Arc::clone(&filesystem));
    let history = restarted
        .read_tool_selection_history(&scope, &thread_id)
        .await
        .expect("restart read")
        .expect("durable history");
    assert_eq!(history.entries, vec![initial.clone()]);

    let thread_path = ScopedPath::new(
        "/threads/agents/agent/projects/project/owners/user/threads/retained-thread/thread.json",
    )
    .expect("thread path");
    let thread_entry = filesystem
        .get(&scope.to_resource_scope(), &thread_path)
        .await
        .expect("thread read")
        .expect("thread entry");
    let incarnation = serde_json::from_slice::<serde_json::Value>(&thread_entry.entry.body)
        .expect("thread JSON")["incarnation_id"]
        .as_str()
        .expect("incarnation id")
        .to_string();
    let history_path = ScopedPath::new(format!(
        "/threads/agents/agent/projects/project/owners/user/tool-selections/retained-thread/{incarnation}.json"
    ))
    .expect("history path");

    restarted
        .delete_thread(&scope, &thread_id)
        .await
        .expect("delete thread");
    let retained = filesystem
        .get(&scope.to_resource_scope(), &history_path)
        .await
        .expect("history read")
        .expect("the history must survive thread deletion");
    let retained: ToolSelectionHistory =
        serde_json::from_slice(&retained.entry.body).expect("history JSON");
    assert_eq!(retained.entries, vec![initial]);
}

/// Tool-selection activity on both backends: nothing before the first
/// record, the model-call clock only moves forward, a tool is recorded once,
/// and it is scoped like the history.
async fn assert_activity_contract(service: &dyn SessionThreadService) {
    let scope = scope("agent");
    let thread_id = ThreadId::new("thread-activity").expect("thread");
    ensure(service, &scope, &thread_id).await;
    assert!(
        service
            .read_tool_selection_activity(&scope, &thread_id)
            .await
            .expect("read")
            .is_none()
    );
    let record = |update| RecordToolSelectionActivityRequest {
        scope: scope.clone(),
        thread_id: thread_id.clone(),
        update,
    };
    let later = ModelCallMark {
        turn_id: TurnId::new(),
        called_at: Utc::now(),
        model: Some("anthropic/claude-sonnet".to_string()),
    };
    let earlier = ModelCallMark {
        turn_id: TurnId::new(),
        called_at: later.called_at - chrono::Duration::minutes(10),
        model: None,
    };
    service
        .record_tool_selection_activity(record(ToolSelectionActivityUpdate::ModelCall(
            later.clone(),
        )))
        .await
        .expect("mark");
    service
        .record_tool_selection_activity(record(ToolSelectionActivityUpdate::ModelCall(earlier)))
        .await
        .expect("stale mark");
    for name in ["github__create_issue", "github__create_issue"] {
        service
            .record_tool_selection_activity(record(ToolSelectionActivityUpdate::CalledTool(
                name.to_string(),
            )))
            .await
            .expect("called tool");
    }
    let read = service
        .read_tool_selection_activity(&scope, &thread_id)
        .await
        .expect("read")
        .expect("activity");
    assert_eq!(read.last_model_call, Some(later));
    assert_eq!(read.called_tools, ["github__create_issue"]);

    let foreign = service
        .read_tool_selection_activity(&self::scope("other-agent"), &thread_id)
        .await
        .expect_err("cross-scope read");
    assert!(matches!(foreign, SessionThreadError::UnknownThread { .. }));
}

#[tokio::test]
async fn in_memory_activity_contract() {
    assert_activity_contract(&InMemorySessionThreadService::default()).await;
}

#[tokio::test]
async fn filesystem_activity_contract() {
    assert_activity_contract(&FilesystemSessionThreadService::new(scoped_filesystem())).await;
}

/// The model-call clock is durable: a restarted service reads the same mark.
#[tokio::test]
async fn filesystem_activity_survives_restart() {
    let filesystem = scoped_filesystem();
    let scope = scope("agent");
    let thread_id = ThreadId::new("activity-restart").expect("thread");
    let mark = ModelCallMark {
        turn_id: TurnId::new(),
        called_at: Utc::now(),
        model: None,
    };
    {
        let service = FilesystemSessionThreadService::new(Arc::clone(&filesystem));
        ensure(&service, &scope, &thread_id).await;
        service
            .record_tool_selection_activity(RecordToolSelectionActivityRequest {
                scope: scope.clone(),
                thread_id: thread_id.clone(),
                update: ToolSelectionActivityUpdate::ModelCall(mark.clone()),
            })
            .await
            .expect("mark");
    }
    let restarted = FilesystemSessionThreadService::new(filesystem);
    let read = restarted
        .read_tool_selection_activity(&scope, &thread_id)
        .await
        .expect("read")
        .expect("activity");
    assert_eq!(read.last_model_call, Some(mark));
}
