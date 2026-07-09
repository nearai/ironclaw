// arch-exempt: large_file, filesystem thread contract suite decomposition, plan #5662
//! Contract tests for [`FilesystemSessionThreadService`].
//!
//! Drives the production filesystem-backed store over an
//! [`InMemoryBackend`] composed under a `/threads` mount alias whose
//! `VirtualPath` target encodes a tenant/user prefix. Mirrors the shape of
//! the run-state and processes filesystem contract suites — see
//! `crates/ironclaw_run_state/tests/run_state_contract.rs` and
//! `crates/ironclaw_processes/tests/process_store_contract.rs`.

use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_filesystem::{
    BackendCapabilities, CasExpectation, DirEntry, Entry, FileStat, FilesystemError,
    FilesystemOperation, Filter, InMemoryBackend, LocalFilesystem, Page, RecordVersion,
    RootFilesystem, ScopedFilesystem, SeqNo, StorageTxn, TxnCapability, VersionedEntry,
};
use ironclaw_host_api::{
    AgentId, CapabilityId, HostPath, InvocationId, MountAlias, MountGrant, MountPermissions,
    MountView, ProjectId, ScopedPath, TenantId, ThreadId, UserId, VirtualPath,
};
use ironclaw_threads::{
    AcceptInboundMessageRequest, AppendAssistantDraftRequest,
    AppendCapabilityDisplayPreviewRequest, AppendFinalizedAssistantMessageRequest,
    AppendToolResultReferenceRequest, AttachmentKind, AttachmentRef,
    CapabilityDisplayPreviewEnvelope, CapabilityDisplayPreviewEnvelopeInput,
    CapabilityDisplayPreviewStatus, CreateSummaryArtifactRequest, EnsureThreadRequest,
    FilesystemSessionThreadService, FinalizedAssistantMessageByRunRequest,
    LoadContextMessagesRequest, LoadContextWindowRequest, MessageContent, MessageKind,
    MessageStatus, RedactMessageRequest, ReplayAcceptedInboundMessageRequest, SessionThreadError,
    SessionThreadService, SummaryKind, SummaryModelContextPolicy, ThreadHistoryRequest,
    ThreadScope, ToolResultSafeSummary, UpdateAssistantDraftRequest,
};
use tokio::sync::{Barrier, Mutex, OwnedMutexGuard};

#[tokio::test]
async fn filesystem_delete_thread_removes_owned_thread_and_hides_missing_or_wrong_scope() {
    let backend = Arc::new(InMemoryBackend::new());
    let scoped = scoped_threads_fs_at(backend, "tenant-delete", "alice");
    let service = FilesystemSessionThreadService::new(scoped);
    let owned_scope = scope("delete-owned");
    let wrong_scope = scope("delete-wrong");
    let thread = service
        .ensure_thread(EnsureThreadRequest {
            scope: owned_scope.clone(),
            thread_id: Some(ThreadId::new("thread-delete-owned").unwrap()),
            created_by_actor_id: "actor-a".into(),
            title: None,
            metadata_json: None,
        })
        .await
        .unwrap();

    let wrong_scope_error = service
        .delete_thread(&wrong_scope, &thread.thread_id)
        .await
        .expect_err("wrong-scope delete should hide thread existence");
    assert_unknown_thread(wrong_scope_error, &thread.thread_id);

    service
        .read_thread(ThreadHistoryRequest {
            scope: owned_scope.clone(),
            thread_id: thread.thread_id.clone(),
        })
        .await
        .expect("wrong-scope delete must not remove owned thread");

    service
        .delete_thread(&owned_scope, &thread.thread_id)
        .await
        .expect("owned delete succeeds");

    let deleted_error = service
        .read_thread(ThreadHistoryRequest {
            scope: owned_scope.clone(),
            thread_id: thread.thread_id.clone(),
        })
        .await
        .expect_err("deleted thread should no longer be readable");
    assert_unknown_thread(deleted_error, &thread.thread_id);

    let repeat_error = service
        .delete_thread(&owned_scope, &thread.thread_id)
        .await
        .expect_err("repeat delete should be non-enumerating missing shape");
    assert_unknown_thread(repeat_error, &thread.thread_id);

    let missing = ThreadId::new("thread-delete-missing").unwrap();
    let missing_error = service
        .delete_thread(&owned_scope, &missing)
        .await
        .expect_err("missing delete should be non-enumerating");
    assert_unknown_thread(missing_error, &missing);
}

#[tokio::test]
async fn filesystem_delete_thread_invalidates_thread_index_cache() {
    use ironclaw_threads::ListThreadsForScopeRequest;

    let backend = Arc::new(InMemoryBackend::new());
    let scoped = scoped_threads_fs_at(backend, "tenant-delete-cache", "alice");
    let service = FilesystemSessionThreadService::new(scoped);
    let request_scope = scope("delete-cache");
    let keep = service
        .ensure_thread(EnsureThreadRequest {
            scope: request_scope.clone(),
            thread_id: Some(ThreadId::new("thread-delete-cache-keep").unwrap()),
            created_by_actor_id: "actor-a".into(),
            title: None,
            metadata_json: None,
        })
        .await
        .unwrap();
    let delete = service
        .ensure_thread(EnsureThreadRequest {
            scope: request_scope.clone(),
            thread_id: Some(ThreadId::new("thread-delete-cache-remove").unwrap()),
            created_by_actor_id: "actor-a".into(),
            title: None,
            metadata_json: None,
        })
        .await
        .unwrap();

    let warmed = service
        .list_threads_for_scope(ListThreadsForScopeRequest {
            scope: request_scope.clone(),
            limit: None,
            cursor: None,
        })
        .await
        .unwrap();
    assert_eq!(warmed.threads.len(), 2);

    service
        .delete_thread(&request_scope, &delete.thread_id)
        .await
        .expect("delete succeeds");

    let listed = service
        .list_threads_for_scope(ListThreadsForScopeRequest {
            scope: request_scope,
            limit: None,
            cursor: None,
        })
        .await
        .unwrap();
    let ids: Vec<&ThreadId> = listed
        .threads
        .iter()
        .map(|record| &record.thread_id)
        .collect();
    assert!(ids.contains(&&keep.thread_id));
    assert!(!ids.contains(&&delete.thread_id));
}

#[tokio::test]
async fn filesystem_first_context_window_uses_one_shot_accepted_message_cache() {
    let backend = Arc::new(QueryCountingBackend::new());
    let scoped = scoped_threads_fs_at(
        Arc::clone(&backend),
        "tenant-first-context-window-cache",
        "alice",
    );
    let service = FilesystemSessionThreadService::new(scoped);
    let request_scope = scope("first-context-window-cache");
    let thread = service
        .ensure_thread(EnsureThreadRequest {
            scope: request_scope.clone(),
            thread_id: Some(ThreadId::new("thread-first-context-window-cache").unwrap()),
            created_by_actor_id: "actor-a".into(),
            title: None,
            metadata_json: None,
        })
        .await
        .unwrap();

    let accepted = service
        .accept_inbound_message(AcceptInboundMessageRequest {
            scope: request_scope.clone(),
            thread_id: thread.thread_id.clone(),
            actor_id: "actor-a".into(),
            source_binding_id: Some("binding-first-context-window-cache".into()),
            reply_target_binding_id: None,
            external_event_id: Some("event-first-context-window-cache".into()),
            content: MessageContent::text("first prompt should be hot"),
        })
        .await
        .unwrap();
    assert_eq!(accepted.sequence, 1);

    service
        .mark_message_submitted(
            &request_scope,
            &thread.thread_id,
            accepted.message_id,
            "turn-first-context-window-cache".into(),
            "run-first-context-window-cache".into(),
        )
        .await
        .unwrap();
    let query_count_after_submit = backend.query_count();
    let get_count_after_submit = backend.get_count();

    let first_window = service
        .load_context_window(LoadContextWindowRequest {
            scope: request_scope.clone(),
            thread_id: thread.thread_id.clone(),
            max_messages: 16,
        })
        .await
        .unwrap();

    assert_eq!(first_window.messages.len(), 1);
    assert_eq!(
        first_window.messages[0].content,
        "first prompt should be hot"
    );
    assert_eq!(
        backend.query_count(),
        query_count_after_submit,
        "the immediate first-turn context load should consume the submitted message cache"
    );
    assert_eq!(
        backend.get_count(),
        get_count_after_submit,
        "the immediate first-turn context load should not re-read the thread record"
    );

    let second_window = service
        .load_context_window(LoadContextWindowRequest {
            scope: request_scope,
            thread_id: thread.thread_id,
            max_messages: 16,
        })
        .await
        .unwrap();

    assert_eq!(second_window.messages.len(), 1);
    assert!(
        backend.query_count() > query_count_after_submit,
        "the submitted-message context cache must be one-shot, not a transcript source of truth"
    );
    assert!(
        backend.get_count() > get_count_after_submit,
        "the submitted-message context cache must be one-shot, not a thread existence shortcut"
    );
}

#[tokio::test]
async fn filesystem_delete_thread_retry_cleans_stale_thread_index_row() {
    use ironclaw_threads::ListThreadsForScopeRequest;

    let backend = Arc::new(InMemoryBackend::new());
    let scoped = scoped_threads_fs_at(backend, "tenant-delete-stale-index", "alice");
    let service = FilesystemSessionThreadService::new(Arc::clone(&scoped));
    let request_scope = scope("delete-stale-index");
    let thread = service
        .ensure_thread(EnsureThreadRequest {
            scope: request_scope.clone(),
            thread_id: Some(ThreadId::new("thread-delete-stale-index").unwrap()),
            created_by_actor_id: "actor-a".into(),
            title: Some("stale title".into()),
            metadata_json: None,
        })
        .await
        .unwrap();
    let warmed = service
        .list_threads_for_scope(ListThreadsForScopeRequest {
            scope: request_scope.clone(),
            limit: None,
            cursor: None,
        })
        .await
        .unwrap();
    assert_eq!(warmed.threads.len(), 1);

    scoped
        .delete(
            &request_scope.to_resource_scope(),
            &thread_root_path_for_test(&request_scope, thread.thread_id.as_str()),
        )
        .await
        .expect("test setup removes source thread root but leaves derived index row");
    service.clear_thread_index_cache_for_scope(&request_scope);

    let retry_error = service
        .delete_thread(&request_scope, &thread.thread_id)
        .await
        .expect_err("retry after partial delete should still report unknown source");
    assert_unknown_thread(retry_error, &thread.thread_id);

    let listed = service
        .list_threads_for_scope(ListThreadsForScopeRequest {
            scope: request_scope,
            limit: None,
            cursor: None,
        })
        .await
        .unwrap();
    assert!(
        listed
            .threads
            .iter()
            .all(|record| record.thread_id != thread.thread_id),
        "retry cleanup must remove stale derived index metadata from listings"
    );
}

#[tokio::test]
async fn filesystem_list_threads_skips_stale_thread_index_after_partial_delete() {
    use ironclaw_threads::ListThreadsForScopeRequest;

    let backend = Arc::new(InMemoryBackend::new());
    let scoped = scoped_threads_fs_at(backend, "tenant-list-stale-index", "alice");
    let service = FilesystemSessionThreadService::new(Arc::clone(&scoped));
    let request_scope = scope("list-stale-index");
    let thread = service
        .ensure_thread(EnsureThreadRequest {
            scope: request_scope.clone(),
            thread_id: Some(ThreadId::new("thread-list-stale-index").unwrap()),
            created_by_actor_id: "actor-a".into(),
            title: Some("stale title".into()),
            metadata_json: None,
        })
        .await
        .unwrap();
    service
        .list_threads_for_scope(ListThreadsForScopeRequest {
            scope: request_scope.clone(),
            limit: None,
            cursor: None,
        })
        .await
        .expect("warm list creates thread index row");

    scoped
        .delete(
            &request_scope.to_resource_scope(),
            &thread_root_path_for_test(&request_scope, thread.thread_id.as_str()),
        )
        .await
        .expect("test setup removes source thread root but leaves derived index row");
    service.clear_thread_index_cache_for_scope(&request_scope);

    let listed = service
        .list_threads_for_scope(ListThreadsForScopeRequest {
            scope: request_scope.clone(),
            limit: None,
            cursor: None,
        })
        .await
        .unwrap();
    assert!(
        listed
            .threads
            .iter()
            .all(|record| record.thread_id != thread.thread_id),
        "list_threads_for_scope must not expose an index row whose source thread root is gone"
    );
    assert!(
        scoped
            .get(
                &request_scope.to_resource_scope(),
                &thread_index_record_path_for_test(&request_scope, thread.thread_id.as_str()),
            )
            .await
            .unwrap()
            .is_none(),
        "stale index row should be removed during list cleanup"
    );
}

#[tokio::test]
async fn filesystem_read_thread_ignores_stale_thread_index_generation() {
    let backend = Arc::new(InMemoryBackend::new());
    let scoped = scoped_threads_fs_at(backend, "tenant-read-stale-index", "alice");
    let service = FilesystemSessionThreadService::new(Arc::clone(&scoped));
    let request_scope = scope("read-stale-index");
    let thread = service
        .ensure_thread(EnsureThreadRequest {
            scope: request_scope.clone(),
            thread_id: Some(ThreadId::new("thread-read-stale-index").unwrap()),
            created_by_actor_id: "actor-a".into(),
            title: None,
            metadata_json: None,
        })
        .await
        .unwrap();
    let source_updated_at = thread.updated_at;

    let index_path = thread_index_record_path_for_test(&request_scope, thread.thread_id.as_str());
    let mut stale_index: serde_json::Value = serde_json::from_slice(
        &scoped
            .get(&request_scope.to_resource_scope(), &index_path)
            .await
            .unwrap()
            .expect("ensure_thread writes derived index row")
            .entry
            .body,
    )
    .unwrap();
    stale_index["created_at"] = serde_json::json!("2000-01-01T00:00:00Z");
    stale_index["updated_at"] = serde_json::json!("2099-01-01T00:00:00Z");
    stale_index["title"] = serde_json::json!("stale derived title");
    stale_index["flags"]["title_present"] = serde_json::json!(true);
    scoped
        .put(
            &request_scope.to_resource_scope(),
            &index_path,
            Entry::bytes(serde_json::to_vec_pretty(&stale_index).unwrap()),
            CasExpectation::Any,
        )
        .await
        .expect("test setup writes stale derived index row");

    let read = service
        .read_thread(ThreadHistoryRequest {
            scope: request_scope,
            thread_id: thread.thread_id,
        })
        .await
        .unwrap();

    assert_eq!(
        read.title, None,
        "read_thread must not overlay title from a stale index generation"
    );
    assert_eq!(
        read.updated_at, source_updated_at,
        "read_thread must not overlay updated_at from a stale index generation"
    );
}

#[tokio::test]
async fn filesystem_delete_thread_removes_inbound_idempotency_records() {
    let backend = Arc::new(InMemoryBackend::new());
    let scoped = scoped_threads_fs_at(backend, "tenant-delete-idempotency", "alice");
    let service = FilesystemSessionThreadService::new(scoped);
    let request_scope = scope("delete-idempotency");
    let thread = service
        .ensure_thread(EnsureThreadRequest {
            scope: request_scope.clone(),
            thread_id: Some(ThreadId::new("thread-delete-idempotency").unwrap()),
            created_by_actor_id: "actor-a".into(),
            title: None,
            metadata_json: None,
        })
        .await
        .unwrap();
    service
        .accept_inbound_message(AcceptInboundMessageRequest {
            scope: request_scope.clone(),
            thread_id: thread.thread_id.clone(),
            actor_id: "actor-a".into(),
            source_binding_id: Some("binding-delete-idempotency".into()),
            reply_target_binding_id: None,
            external_event_id: Some("event-delete-idempotency".into()),
            content: MessageContent::text("delete me"),
        })
        .await
        .unwrap();

    service
        .delete_thread(&request_scope, &thread.thread_id)
        .await
        .expect("owned delete succeeds");

    let replay = service
        .replay_accepted_inbound_message(ReplayAcceptedInboundMessageRequest {
            scope: request_scope,
            actor_id: "actor-a".into(),
            source_binding_id: "binding-delete-idempotency".into(),
            external_event_id: "event-delete-idempotency".into(),
        })
        .await
        .expect("deleted thread must not leave stale idempotency records");

    assert!(replay.is_none());
}

#[tokio::test]
async fn filesystem_finalized_assistant_lookup_by_run_uses_persisted_message() {
    let backend = Arc::new(InMemoryBackend::new());
    let scoped = scoped_threads_fs_at(backend, "tenant-finalized-by-run", "alice");
    let service = FilesystemSessionThreadService::new(scoped);
    let scope = scope("finalized-by-run");
    let thread = service
        .ensure_thread(EnsureThreadRequest {
            scope: scope.clone(),
            thread_id: Some(ThreadId::new("thread-finalized-by-run").unwrap()),
            created_by_actor_id: "actor-a".into(),
            title: None,
            metadata_json: None,
        })
        .await
        .unwrap();
    let draft = service
        .append_assistant_draft(AppendAssistantDraftRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
            turn_run_id: "run-finalized-lookup".into(),
            content: MessageContent::text("draft"),
        })
        .await
        .unwrap();

    let before_finalize = service
        .finalized_assistant_message_by_run(FinalizedAssistantMessageByRunRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
            turn_run_id: "run-finalized-lookup".into(),
        })
        .await
        .unwrap();
    assert!(before_finalize.is_none());

    service
        .finalize_assistant_message(
            &scope,
            &thread.thread_id,
            draft.message_id,
            MessageContent::text("final"),
        )
        .await
        .unwrap();

    let finalized = service
        .finalized_assistant_message_by_run(FinalizedAssistantMessageByRunRequest {
            scope,
            thread_id: thread.thread_id,
            turn_run_id: "run-finalized-lookup".into(),
        })
        .await
        .unwrap()
        .expect("finalized assistant message is indexed by run");
    assert_eq!(finalized.message_id, draft.message_id);
    assert_eq!(finalized.status, MessageStatus::Finalized);
    assert_eq!(finalized.content.as_deref(), Some("final"));
}

#[tokio::test]
async fn filesystem_list_thread_history_returns_durable_message_timestamps() {
    let backend = Arc::new(InMemoryBackend::new());
    let scoped = scoped_threads_fs_at(backend, "tenant-message-timestamps", "alice");
    let service = FilesystemSessionThreadService::new(scoped);
    let scope = scope("message-timestamps");
    let thread = service
        .ensure_thread(EnsureThreadRequest {
            scope: scope.clone(),
            thread_id: Some(ThreadId::new("thread-message-timestamps").unwrap()),
            created_by_actor_id: "actor-a".into(),
            title: None,
            metadata_json: None,
        })
        .await
        .unwrap();

    let before_user = Utc::now();
    let accepted = service
        .accept_inbound_message(AcceptInboundMessageRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
            actor_id: "actor-a".into(),
            source_binding_id: None,
            reply_target_binding_id: None,
            external_event_id: None,
            content: MessageContent::text("hello"),
        })
        .await
        .unwrap();
    let after_user = Utc::now();

    let draft = service
        .append_assistant_draft(AppendAssistantDraftRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
            turn_run_id: "run-message-timestamps".into(),
            content: MessageContent::text("working"),
        })
        .await
        .unwrap();
    let draft_created_at = draft.created_at.expect("draft has created_at");
    assert_eq!(draft.updated_at, Some(draft_created_at));

    let finalized = service
        .finalize_assistant_message(
            &scope,
            &thread.thread_id,
            draft.message_id,
            MessageContent::text("done"),
        )
        .await
        .unwrap();
    let final_updated_at = finalized.updated_at.expect("finalized has updated_at");

    let history = service
        .list_thread_history(ThreadHistoryRequest {
            scope,
            thread_id: thread.thread_id,
        })
        .await
        .unwrap();
    let user = history
        .messages
        .iter()
        .find(|message| message.message_id == accepted.message_id)
        .expect("user message is in history");
    let user_created_at = user.created_at.expect("user message has created_at");
    assert!(user_created_at >= before_user && user_created_at <= after_user);
    assert_eq!(user.updated_at, Some(user_created_at));

    let assistant = history
        .messages
        .iter()
        .find(|message| message.message_id == draft.message_id)
        .expect("assistant message is in history");
    assert_eq!(assistant.created_at, Some(draft_created_at));
    assert_eq!(assistant.updated_at, Some(final_updated_at));
}

#[tokio::test]
async fn filesystem_append_finalized_assistant_message_is_finalized_and_idempotent_by_turn_run() {
    let backend = Arc::new(InMemoryBackend::new());
    let scoped = scoped_threads_fs_at(backend, "tenant-finalized-append", "alice");
    let service = FilesystemSessionThreadService::new(scoped);
    let scope = scope("finalized-append");
    let thread = service
        .ensure_thread(EnsureThreadRequest {
            scope: scope.clone(),
            thread_id: Some(ThreadId::new("thread-finalized-append").unwrap()),
            created_by_actor_id: "actor-a".into(),
            title: None,
            metadata_json: None,
        })
        .await
        .unwrap();

    let first = service
        .append_finalized_assistant_message(AppendFinalizedAssistantMessageRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
            turn_run_id: "run-finalized-append".into(),
            content: MessageContent::text("final answer"),
        })
        .await
        .unwrap();
    let duplicate = service
        .append_finalized_assistant_message(AppendFinalizedAssistantMessageRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
            turn_run_id: "run-finalized-append".into(),
            content: MessageContent::text("retry answer ignored"),
        })
        .await
        .unwrap();

    assert_eq!(first.message_id, duplicate.message_id);
    assert_eq!(duplicate.kind, MessageKind::Assistant);
    assert_eq!(duplicate.status, MessageStatus::Finalized);
    assert_eq!(duplicate.content.as_deref(), Some("final answer"));

    let finalized = service
        .finalized_assistant_message_by_run(FinalizedAssistantMessageByRunRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
            turn_run_id: "run-finalized-append".into(),
        })
        .await
        .unwrap()
        .expect("finalized assistant message should be indexed by run");
    assert_eq!(finalized.message_id, first.message_id);

    let history = service
        .list_thread_history(ThreadHistoryRequest {
            scope,
            thread_id: thread.thread_id,
        })
        .await
        .unwrap();
    assert_eq!(history.messages.len(), 1);
    assert_eq!(history.messages[0].message_id, first.message_id);
    assert_eq!(history.messages[0].status, MessageStatus::Finalized);
}

#[tokio::test]
async fn filesystem_append_finalized_assistant_message_finalizes_existing_draft_by_turn_run() {
    let backend = Arc::new(InMemoryBackend::new());
    let scoped = scoped_threads_fs_at(backend, "tenant-finalized-existing-draft", "alice");
    let service = FilesystemSessionThreadService::new(scoped);
    let scope = scope("finalized-existing-draft");
    let thread = service
        .ensure_thread(EnsureThreadRequest {
            scope: scope.clone(),
            thread_id: Some(ThreadId::new("thread-finalized-existing-draft").unwrap()),
            created_by_actor_id: "actor-a".into(),
            title: None,
            metadata_json: None,
        })
        .await
        .unwrap();

    let draft = service
        .append_assistant_draft(AppendAssistantDraftRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
            turn_run_id: "run-finalized-existing-draft".into(),
            content: MessageContent::text("draft answer"),
        })
        .await
        .unwrap();
    let finalized = service
        .append_finalized_assistant_message(AppendFinalizedAssistantMessageRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
            turn_run_id: "run-finalized-existing-draft".into(),
            content: MessageContent::text("final answer"),
        })
        .await
        .unwrap();

    assert_eq!(finalized.message_id, draft.message_id);
    assert_eq!(finalized.status, MessageStatus::Finalized);
    assert_eq!(finalized.content.as_deref(), Some("final answer"));

    // The run index resolves to the same single message — finalizing in place
    // must not leave the run pointing at a stale or second record.
    let by_run = service
        .finalized_assistant_message_by_run(FinalizedAssistantMessageByRunRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
            turn_run_id: "run-finalized-existing-draft".into(),
        })
        .await
        .unwrap()
        .expect("finalized assistant message should be indexed by run");
    assert_eq!(by_run.message_id, draft.message_id);
    assert_eq!(by_run.status, MessageStatus::Finalized);

    // Finalize-by-turn-run finalizes the existing draft IN PLACE — it must
    // not materialize a second history row. Assert the caller-visible
    // single-row invariant, not just the returned record.
    let history = service
        .list_thread_history(ThreadHistoryRequest {
            scope,
            thread_id: thread.thread_id,
        })
        .await
        .unwrap();
    assert_eq!(history.messages.len(), 1);
    assert_eq!(history.messages[0].message_id, draft.message_id);
    assert_eq!(history.messages[0].status, MessageStatus::Finalized);
    assert_eq!(history.messages[0].content.as_deref(), Some("final answer"));
}

#[tokio::test]
async fn filesystem_redacts_append_only_finalized_assistant_message() {
    // Regression for the append-log mutation gap: a finalized assistant
    // message written with NO prior draft lives only in the per-thread
    // append log (no individual message file). Redaction must still apply —
    // `apply_message_update` materializes the file on mutation and the
    // file-authoritative merge then shadows the original log entry, so reads
    // surface the redacted record (not the stale appended one) and history
    // stays single-row.
    let backend = Arc::new(InMemoryBackend::new());
    let scoped = scoped_threads_fs_at(backend, "tenant-redact-append-only", "alice");
    let service = FilesystemSessionThreadService::new(scoped);
    let scope = scope("redact-append-only");
    let thread = service
        .ensure_thread(EnsureThreadRequest {
            scope: scope.clone(),
            thread_id: Some(ThreadId::new("thread-redact-append-only").unwrap()),
            created_by_actor_id: "actor-a".into(),
            title: None,
            metadata_json: None,
        })
        .await
        .unwrap();

    // No prior draft -> this finalized message is append-only.
    let finalized = service
        .append_finalized_assistant_message(AppendFinalizedAssistantMessageRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
            turn_run_id: "run-redact-append-only".into(),
            content: MessageContent::text("secret answer"),
        })
        .await
        .unwrap();
    assert_eq!(finalized.status, MessageStatus::Finalized);

    let redacted = service
        .redact_message(RedactMessageRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
            message_id: finalized.message_id,
            redaction_ref: "redaction/audit/append-only".into(),
        })
        .await
        .unwrap();
    assert_eq!(redacted.status, MessageStatus::Redacted);
    assert_eq!(redacted.content, None);

    // Reads must reflect the redaction, not the original append-log entry,
    // and must not duplicate the message.
    let history = service
        .list_thread_history(ThreadHistoryRequest {
            scope,
            thread_id: thread.thread_id,
        })
        .await
        .unwrap();
    assert_eq!(history.messages.len(), 1);
    assert_eq!(history.messages[0].message_id, finalized.message_id);
    assert_eq!(history.messages[0].status, MessageStatus::Redacted);
    assert_eq!(history.messages[0].content, None);
    assert_eq!(
        history.messages[0].redaction_ref.as_deref(),
        Some("redaction/audit/append-only")
    );
}

#[tokio::test]
async fn filesystem_lookup_index_write_failure_does_not_fail_message_contract() {
    let backend = Arc::new(LookupIndexWriteFailureBackend::new());
    let scoped = scoped_threads_fs_at(backend, "tenant-lookup-index-failure", "alice");
    let service = FilesystemSessionThreadService::new(scoped);
    let scope = scope("lookup-index-failure");
    let thread = service
        .ensure_thread(EnsureThreadRequest {
            scope: scope.clone(),
            thread_id: Some(ThreadId::new("thread-lookup-index-failure").unwrap()),
            created_by_actor_id: "actor-a".into(),
            title: None,
            metadata_json: None,
        })
        .await
        .unwrap();
    let draft = service
        .append_assistant_draft(AppendAssistantDraftRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
            turn_run_id: "run-lookup-index-failure".into(),
            content: MessageContent::text("draft"),
        })
        .await
        .expect("message append must not depend on lookup-index write success");

    service
        .finalize_assistant_message(
            &scope,
            &thread.thread_id,
            draft.message_id,
            MessageContent::text("final"),
        )
        .await
        .expect("message update must not depend on lookup-index write success");

    let finalized = service
        .finalized_assistant_message_by_run(FinalizedAssistantMessageByRunRequest {
            scope,
            thread_id: thread.thread_id,
            turn_run_id: "run-lookup-index-failure".into(),
        })
        .await
        .expect("lookup should scan when lookup-index backfill fails")
        .expect("finalized assistant message should be found without lookup index");
    assert_eq!(finalized.message_id, draft.message_id);
    assert_eq!(finalized.status, MessageStatus::Finalized);
    assert_eq!(finalized.content.as_deref(), Some("final"));
}

#[tokio::test]
async fn filesystem_lookup_index_read_failure_falls_back_to_transcript_scan() {
    let backend = Arc::new(LookupIndexReadFailureBackend::new());
    let scoped = scoped_threads_fs_at(backend, "tenant-lookup-index-read-failure", "alice");
    let service = FilesystemSessionThreadService::new(scoped);
    let scope = scope("lookup-index-read-failure");
    let thread = service
        .ensure_thread(EnsureThreadRequest {
            scope: scope.clone(),
            thread_id: Some(ThreadId::new("thread-lookup-index-read-failure").unwrap()),
            created_by_actor_id: "actor-a".into(),
            title: None,
            metadata_json: None,
        })
        .await
        .unwrap();
    let draft = service
        .append_assistant_draft(AppendAssistantDraftRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
            turn_run_id: "run-lookup-index-read-failure".into(),
            content: MessageContent::text("draft"),
        })
        .await
        .unwrap();
    service
        .finalize_assistant_message(
            &scope,
            &thread.thread_id,
            draft.message_id,
            MessageContent::text("final"),
        )
        .await
        .unwrap();

    let finalized = service
        .finalized_assistant_message_by_run(FinalizedAssistantMessageByRunRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
            turn_run_id: "run-lookup-index-read-failure".into(),
        })
        .await
        .expect("assistant lookup should scan after lookup-index read failure")
        .expect("finalized assistant message should be found");
    assert_eq!(finalized.message_id, draft.message_id);

    let first_tool_result = service
        .append_tool_result_reference(AppendToolResultReferenceRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
            turn_run_id: "run-lookup-index-read-failure".into(),
            result_ref: "result:lookup-index-read-failure".into(),
            safe_summary: ToolResultSafeSummary::new("safe tool result").unwrap(),
            provider_call: None,
            model_observation: None,
        })
        .await
        .unwrap();
    let duplicate_tool_result = service
        .append_tool_result_reference(AppendToolResultReferenceRequest {
            scope,
            thread_id: thread.thread_id,
            turn_run_id: "run-lookup-index-read-failure".into(),
            result_ref: "result:lookup-index-read-failure".into(),
            safe_summary: ToolResultSafeSummary::new("retry content ignored").unwrap(),
            provider_call: None,
            model_observation: None,
        })
        .await
        .expect("tool-result lookup should scan after lookup-index read failure");
    assert_eq!(
        duplicate_tool_result.message_id,
        first_tool_result.message_id
    );
}

#[tokio::test]
async fn durable_history_round_trips_through_filesystem_store() {
    let backend = Arc::new(InMemoryBackend::new());
    let scoped = scoped_threads_fs_at(Arc::clone(&backend), "tenant-a", "alice");
    let service = FilesystemSessionThreadService::new(scoped);
    let label = "fs-round-trip";
    let thread_id = durable_history_flow(&service, label).await;

    // Restart-equivalent: drop the service + scoped fs, build a new pair
    // pointed at the same backend with the same MountView. Records must
    // rehydrate without loss.
    let scoped = scoped_threads_fs_at(backend, "tenant-a", "alice");
    let reopened = FilesystemSessionThreadService::new(scoped);
    assert_reopened_history(&reopened, label, thread_id).await;
}

#[tokio::test]
async fn filesystem_store_rejects_wrong_scope_history_reads() {
    let backend = Arc::new(InMemoryBackend::new());
    let scoped = scoped_threads_fs_at(backend, "tenant-a", "alice");
    let service = FilesystemSessionThreadService::new(scoped);
    let request_scope = scope("rejected");
    let thread = service
        .ensure_thread(EnsureThreadRequest {
            scope: request_scope.clone(),
            thread_id: Some(ThreadId::new("thread-rejected").unwrap()),
            created_by_actor_id: "actor-a".into(),
            title: None,
            metadata_json: None,
        })
        .await
        .unwrap();
    let wrong_scope = scope("rejected-other");

    let err = service
        .list_thread_history(ThreadHistoryRequest {
            scope: wrong_scope,
            thread_id: thread.thread_id,
        })
        .await;

    assert!(err.is_err(), "wrong-scope history lookup must fail closed");
}

#[tokio::test]
async fn filesystem_store_persists_preview_history_while_hiding_it_from_context() {
    let backend = Arc::new(InMemoryBackend::new());
    let scoped = scoped_threads_fs_at(backend, "tenant-preview", "alice");
    let service = FilesystemSessionThreadService::new(scoped);
    let scope = scope("fs-preview");
    let thread = service
        .ensure_thread(EnsureThreadRequest {
            scope: scope.clone(),
            thread_id: Some(ThreadId::new("thread-fs-preview").unwrap()),
            created_by_actor_id: "actor-a".into(),
            title: None,
            metadata_json: None,
        })
        .await
        .unwrap();
    service
        .accept_inbound_message(AcceptInboundMessageRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
            actor_id: "actor-a".into(),
            source_binding_id: None,
            reply_target_binding_id: None,
            external_event_id: None,
            content: MessageContent::text("run a tool"),
        })
        .await
        .unwrap();

    let invocation_id = InvocationId::new();
    let first = service
        .append_capability_display_preview(AppendCapabilityDisplayPreviewRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
            turn_run_id: "run-1".into(),
            preview: preview_envelope(invocation_id),
        })
        .await
        .unwrap();
    let duplicate = service
        .append_capability_display_preview(AppendCapabilityDisplayPreviewRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
            turn_run_id: "run-1".into(),
            preview: preview_envelope(invocation_id),
        })
        .await
        .unwrap();
    assert_eq!(first.message_id, duplicate.message_id);

    // A summary whose range contains only a CapabilityDisplayPreview (permanent
    // non-visible, never resurfaces) IS now applied: the preview kind is safe
    // to span.  The summary replaces seq 1 (User) through seq 2 (Preview) in
    // the model context; the preview itself remains absent from context.
    service
        .create_summary_artifact(CreateSummaryArtifactRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
            start_sequence: 1,
            end_sequence: 2,
            summary_kind: SummaryKind::Compaction,
            content: MessageContent::text("run a tool summarized"),
            model_context_policy: Some(SummaryModelContextPolicy::ReplaceRangeWhenSelected),
        })
        .await
        .unwrap();

    let history = service
        .list_thread_history(ThreadHistoryRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
        })
        .await
        .unwrap();
    assert_eq!(
        history
            .messages
            .iter()
            .map(|message| message.kind)
            .collect::<Vec<_>>(),
        vec![MessageKind::User, MessageKind::CapabilityDisplayPreview]
    );

    let context = service
        .load_context_window(LoadContextWindowRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
            max_messages: 10,
        })
        .await
        .unwrap();
    // Summary is now applied (CapabilityDisplayPreview is safe to span — permanent
    // non-visible, never resurfaces).  Context shows the summary, not the raw User
    // or the Preview.
    assert_eq!(context.messages.len(), 1);
    assert_eq!(context.messages[0].kind, MessageKind::Summary);

    let direct_context = service
        .load_context_messages(LoadContextMessagesRequest {
            scope,
            thread_id: thread.thread_id,
            message_ids: vec![first.message_id],
        })
        .await
        .unwrap();
    assert!(direct_context.messages.is_empty());
}

#[tokio::test]
async fn filesystem_store_exact_compaction_replacement_summary_replay_is_idempotent() {
    let backend = Arc::new(InMemoryBackend::new());
    let scoped = scoped_threads_fs_at(backend, "tenant-summary", "alice");
    let service = FilesystemSessionThreadService::new(scoped);
    let thread_scope = scope("fs-summary");
    let thread = service
        .ensure_thread(EnsureThreadRequest {
            scope: thread_scope.clone(),
            thread_id: None,
            created_by_actor_id: "actor-a".into(),
            title: None,
            metadata_json: None,
        })
        .await
        .unwrap();
    for text in ["one", "two"] {
        service
            .accept_inbound_message(AcceptInboundMessageRequest {
                scope: thread_scope.clone(),
                thread_id: thread.thread_id.clone(),
                actor_id: "actor-a".into(),
                source_binding_id: None,
                reply_target_binding_id: None,
                external_event_id: None,
                content: MessageContent::text(text),
            })
            .await
            .unwrap();
    }

    let first = service
        .create_summary_artifact(CreateSummaryArtifactRequest {
            scope: thread_scope.clone(),
            thread_id: thread.thread_id.clone(),
            start_sequence: 1,
            end_sequence: 2,
            summary_kind: SummaryKind::Compaction,
            content: MessageContent::text("one and two summarized"),
            model_context_policy: Some(SummaryModelContextPolicy::ReplaceRangeWhenSelected),
        })
        .await
        .unwrap();
    let replay = service
        .create_summary_artifact(CreateSummaryArtifactRequest {
            scope: thread_scope.clone(),
            thread_id: thread.thread_id.clone(),
            start_sequence: 1,
            end_sequence: 2,
            summary_kind: SummaryKind::Compaction,
            content: MessageContent::text("one and two summarized"),
            model_context_policy: Some(SummaryModelContextPolicy::ReplaceRangeWhenSelected),
        })
        .await
        .unwrap();
    assert_eq!(replay.summary_id, first.summary_id);

    let changed_content = service
        .create_summary_artifact(CreateSummaryArtifactRequest {
            scope: thread_scope.clone(),
            thread_id: thread.thread_id.clone(),
            start_sequence: 1,
            end_sequence: 2,
            summary_kind: SummaryKind::Compaction,
            content: MessageContent::text("different summary"),
            model_context_policy: Some(SummaryModelContextPolicy::ReplaceRangeWhenSelected),
        })
        .await;
    assert!(matches!(
        changed_content,
        Err(SessionThreadError::OverlappingSummaryRange { .. })
    ));

    let history = service
        .list_thread_history(ThreadHistoryRequest {
            scope: thread_scope,
            thread_id: thread.thread_id,
        })
        .await
        .unwrap();
    assert_eq!(history.summary_artifacts.len(), 1);
    assert_eq!(history.summary_artifacts[0].summary_id, first.summary_id);
}

#[tokio::test]
async fn filesystem_store_overlapping_replacement_summaries_are_rejected() {
    let backend = Arc::new(InMemoryBackend::new());
    let scoped = scoped_threads_fs_at(backend, "tenant-overlap", "alice");
    let service = FilesystemSessionThreadService::new(scoped);
    let thread_scope = scope("fs-overlap");
    let thread = service
        .ensure_thread(EnsureThreadRequest {
            scope: thread_scope.clone(),
            thread_id: None,
            created_by_actor_id: "actor-a".into(),
            title: None,
            metadata_json: None,
        })
        .await
        .unwrap();
    for text in ["one", "two", "three"] {
        service
            .accept_inbound_message(AcceptInboundMessageRequest {
                scope: thread_scope.clone(),
                thread_id: thread.thread_id.clone(),
                actor_id: "actor-a".into(),
                source_binding_id: None,
                reply_target_binding_id: None,
                external_event_id: None,
                content: MessageContent::text(text),
            })
            .await
            .unwrap();
    }
    service
        .create_summary_artifact(CreateSummaryArtifactRequest {
            scope: thread_scope.clone(),
            thread_id: thread.thread_id.clone(),
            start_sequence: 1,
            end_sequence: 2,
            summary_kind: SummaryKind::Compaction,
            content: MessageContent::text("one and two summarized"),
            model_context_policy: Some(SummaryModelContextPolicy::ReplaceRangeWhenSelected),
        })
        .await
        .unwrap();

    let overlapping = service
        .create_summary_artifact(CreateSummaryArtifactRequest {
            scope: thread_scope,
            thread_id: thread.thread_id,
            start_sequence: 2,
            end_sequence: 3,
            summary_kind: SummaryKind::Compaction,
            content: MessageContent::text("two and three summarized"),
            model_context_policy: Some(SummaryModelContextPolicy::ReplaceRangeWhenSelected),
        })
        .await;

    assert!(matches!(
        overlapping,
        Err(SessionThreadError::OverlappingSummaryRange { .. })
    ));
}

#[tokio::test]
async fn filesystem_preview_append_retries_converge_on_one_message() {
    let backend = Arc::new(InMemoryBackend::new());
    let scoped = scoped_threads_fs_at(backend, "tenant-preview-race", "alice");
    let service = Arc::new(FilesystemSessionThreadService::new(scoped));
    let scope = scope("fs-preview-race");
    let thread = service
        .ensure_thread(EnsureThreadRequest {
            scope: scope.clone(),
            thread_id: Some(ThreadId::new("thread-fs-preview-race").unwrap()),
            created_by_actor_id: "actor-a".into(),
            title: None,
            metadata_json: None,
        })
        .await
        .unwrap();
    let invocation_id = InvocationId::new();

    let left = {
        let service = Arc::clone(&service);
        let scope = scope.clone();
        let thread_id = thread.thread_id.clone();
        async move {
            service
                .append_capability_display_preview(AppendCapabilityDisplayPreviewRequest {
                    scope,
                    thread_id,
                    turn_run_id: "run-race".into(),
                    preview: preview_envelope(invocation_id),
                })
                .await
        }
    };
    let right = {
        let service = Arc::clone(&service);
        let scope = scope.clone();
        let thread_id = thread.thread_id.clone();
        async move {
            service
                .append_capability_display_preview(AppendCapabilityDisplayPreviewRequest {
                    scope,
                    thread_id,
                    turn_run_id: "run-race".into(),
                    preview: preview_envelope(invocation_id),
                })
                .await
        }
    };

    let (left, right) = tokio::join!(left, right);
    let left = left.unwrap();
    let right = right.unwrap();
    assert_eq!(left.message_id, right.message_id);

    let history = service
        .list_thread_history(ThreadHistoryRequest {
            scope,
            thread_id: thread.thread_id,
        })
        .await
        .unwrap();
    let preview_count = history
        .messages
        .iter()
        .filter(|message| message.kind == MessageKind::CapabilityDisplayPreview)
        .count();
    assert_eq!(preview_count, 1);
}

#[tokio::test]
async fn filesystem_transactional_accept_concurrent_duplicate_replays_existing_message() {
    let backend = Arc::new(TransactionalRaceBackend::new());
    let scoped = scoped_threads_fs_at(backend, "tenant-accept-race", "alice");
    let service = Arc::new(FilesystemSessionThreadService::new(scoped));
    let scope = scope("accept-race");
    let thread = service
        .ensure_thread(EnsureThreadRequest {
            scope: scope.clone(),
            thread_id: Some(ThreadId::new("thread-accept-race").unwrap()),
            created_by_actor_id: "actor-a".into(),
            title: None,
            metadata_json: None,
        })
        .await
        .unwrap();

    let left = {
        let service = Arc::clone(&service);
        let scope = scope.clone();
        let thread_id = thread.thread_id.clone();
        async move {
            service
                .accept_inbound_message(AcceptInboundMessageRequest {
                    scope,
                    thread_id,
                    actor_id: "actor-a".into(),
                    source_binding_id: Some("binding-accept-race".into()),
                    reply_target_binding_id: None,
                    external_event_id: Some("event-accept-race".into()),
                    content: MessageContent::text("first payload"),
                })
                .await
        }
    };
    let right = {
        let service = Arc::clone(&service);
        let scope = scope.clone();
        let thread_id = thread.thread_id.clone();
        async move {
            service
                .accept_inbound_message(AcceptInboundMessageRequest {
                    scope,
                    thread_id,
                    actor_id: "actor-a".into(),
                    source_binding_id: Some("binding-accept-race".into()),
                    reply_target_binding_id: None,
                    external_event_id: Some("event-accept-race".into()),
                    content: MessageContent::text("retry payload ignored"),
                })
                .await
        }
    };

    let (left, right) = tokio::join!(left, right);
    let left = left.unwrap();
    let right = right.unwrap();
    assert_eq!(left.message_id, right.message_id);
    assert_ne!(left.idempotent_replay, right.idempotent_replay);

    let history = service
        .list_thread_history(ThreadHistoryRequest {
            scope,
            thread_id: thread.thread_id,
        })
        .await
        .unwrap();
    assert_eq!(history.messages.len(), 1);
    assert_eq!(history.messages[0].message_id, left.message_id);

    let follow_up = service
        .accept_inbound_message(AcceptInboundMessageRequest {
            scope: history.thread.scope.clone(),
            thread_id: history.thread.thread_id.clone(),
            actor_id: "actor-a".into(),
            source_binding_id: None,
            reply_target_binding_id: None,
            external_event_id: None,
            content: MessageContent::text("real follow-up"),
        })
        .await
        .unwrap();
    assert_eq!(
        follow_up.sequence, 2,
        "losing duplicate accept must not reserve a durable sequence"
    );
}

/// Regression for the ScopedFilesystem migration: two stores share one
/// underlying [`RootFilesystem`] but each is constructed with a
/// [`MountView`] whose `/threads` alias resolves to a different
/// tenant-scoped [`VirtualPath`] subtree. Writing the same
/// `(agent_id, project_id, owner_user_id, thread_id)` tuple on tenant A's
/// store must NOT make the record visible from tenant B's store. Before
/// this migration the legacy SQL stores held a raw `Arc<libsql::Database>`
/// / `deadpool_postgres::Pool` and encoded scope identity inside a single
/// shared table — any composition layer that forgot to scope the
/// `Database`/`Pool` to a tenant prefix would leak across tenants, with
/// the type system saying nothing. The structural fix routes every op
/// through `ScopedFilesystem`, so two MountViews over the same backend
/// cannot see each other's data.
#[tokio::test]
async fn filesystem_session_thread_service_isolates_two_tenants_with_same_user_project_ids() {
    let backend = Arc::new(InMemoryBackend::new());
    let scoped_a = scoped_threads_fs_at(Arc::clone(&backend), "tenant-a", "alice");
    let scoped_b = scoped_threads_fs_at(backend, "tenant-b", "alice");
    let service_a = FilesystemSessionThreadService::new(scoped_a);
    let service_b = FilesystemSessionThreadService::new(scoped_b);

    // Identical within-tenant axes on both scopes — only `tenant_id`
    // differs. The MountView's per-tenant rewriting is the only thing
    // keeping the two stores apart on the shared backend.
    let scope_a = ThreadScope {
        tenant_id: TenantId::new("tenant-a").unwrap(),
        agent_id: AgentId::new("agent-x").unwrap(),
        project_id: Some(ProjectId::new("project-1").unwrap()),
        owner_user_id: Some(UserId::new("alice").unwrap()),
        mission_id: None,
    };
    let scope_b = ThreadScope {
        tenant_id: TenantId::new("tenant-b").unwrap(),
        ..scope_a.clone()
    };
    let thread_id = ThreadId::new("thread-shared-id").unwrap();

    service_a
        .ensure_thread(EnsureThreadRequest {
            scope: scope_a.clone(),
            thread_id: Some(thread_id.clone()),
            created_by_actor_id: "actor-a".into(),
            title: Some("Tenant A".into()),
            metadata_json: None,
        })
        .await
        .unwrap();
    service_a
        .accept_inbound_message(AcceptInboundMessageRequest {
            scope: scope_a.clone(),
            thread_id: thread_id.clone(),
            actor_id: "actor-a".into(),
            source_binding_id: Some("binding".into()),
            reply_target_binding_id: None,
            external_event_id: Some("event-a".into()),
            content: MessageContent::text("tenant a payload"),
        })
        .await
        .unwrap();

    // Tenant A sees its thread.
    let history_a = service_a
        .list_thread_history(ThreadHistoryRequest {
            scope: scope_a,
            thread_id: thread_id.clone(),
        })
        .await
        .unwrap();
    assert_eq!(history_a.thread.title.as_deref(), Some("Tenant A"));
    assert_eq!(history_a.messages.len(), 1);

    // Tenant B does NOT see tenant A's thread despite identical
    // (agent_id, project_id, owner_user_id, thread_id).
    let history_b = service_b
        .list_thread_history(ThreadHistoryRequest {
            scope: scope_b.clone(),
            thread_id: thread_id.clone(),
        })
        .await;
    assert!(
        history_b.is_err(),
        "tenant B must NOT see tenant A's thread history (cross-tenant path leak)"
    );

    // And tenant B's replay lookup for tenant A's external event must
    // come back as None — the idempotency record under tenant A's mount
    // is invisible from tenant B.
    let replay = service_b
        .replay_accepted_inbound_message(ironclaw_threads::ReplayAcceptedInboundMessageRequest {
            scope: scope_b,
            actor_id: "actor-a".into(),
            source_binding_id: "binding".into(),
            external_event_id: "event-a".into(),
        })
        .await
        .unwrap();
    assert!(
        replay.is_none(),
        "tenant B must NOT replay tenant A's inbound idempotency record"
    );
}

#[tokio::test]
async fn filesystem_store_rejects_cross_actor_duplicate_external_event_replay() {
    let backend = Arc::new(InMemoryBackend::new());
    let scoped = scoped_threads_fs_at(backend, "tenant-a", "alice");
    let service = FilesystemSessionThreadService::new(scoped);
    let request_scope = scope("actor-replay");
    let thread = service
        .ensure_thread(EnsureThreadRequest {
            scope: request_scope.clone(),
            thread_id: Some(ThreadId::new("thread-actor-replay").unwrap()),
            created_by_actor_id: "actor-a".into(),
            title: None,
            metadata_json: None,
        })
        .await
        .unwrap();

    service
        .accept_inbound_message(AcceptInboundMessageRequest {
            scope: request_scope.clone(),
            thread_id: thread.thread_id.clone(),
            actor_id: "actor-a".into(),
            source_binding_id: Some("binding".into()),
            reply_target_binding_id: None,
            external_event_id: Some("event-actor-check".into()),
            content: MessageContent::text("actor a event"),
        })
        .await
        .unwrap();

    let replay = service
        .accept_inbound_message(AcceptInboundMessageRequest {
            scope: request_scope,
            thread_id: thread.thread_id,
            actor_id: "actor-b".into(),
            source_binding_id: Some("binding".into()),
            reply_target_binding_id: None,
            external_event_id: Some("event-actor-check".into()),
            content: MessageContent::text("actor b must not replay actor a"),
        })
        .await;

    assert!(matches!(
        replay,
        Err(SessionThreadError::IdempotentReplayActorMismatch { .. })
    ));
}

/// Mirrors the legacy `durable_history_flow` from the old SQL contract
/// suite. Drives every transition the service exposes and returns the
/// thread id so a downstream restart-equivalent test can confirm the
/// records rehydrated identically.
async fn durable_history_flow(service: &impl SessionThreadService, label: &str) -> ThreadId {
    let scope = scope(label);
    let thread = service
        .ensure_thread(EnsureThreadRequest {
            scope: scope.clone(),
            thread_id: Some(ThreadId::new(format!("thread-{label}")).unwrap()),
            created_by_actor_id: "actor-a".into(),
            title: Some("Durable thread".into()),
            metadata_json: Some("{\"source\":\"contract\"}".into()),
        })
        .await
        .unwrap();

    let first = service
        .accept_inbound_message(AcceptInboundMessageRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
            actor_id: "actor-a".into(),
            source_binding_id: Some("telegram-thread-1".into()),
            reply_target_binding_id: Some("telegram-thread-1".into()),
            external_event_id: Some("telegram-event-1".into()),
            content: MessageContent::text("secret token"),
        })
        .await
        .unwrap();
    let duplicate = service
        .accept_inbound_message(AcceptInboundMessageRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
            actor_id: "actor-a".into(),
            source_binding_id: Some("telegram-thread-1".into()),
            reply_target_binding_id: Some("telegram-thread-1".into()),
            external_event_id: Some("telegram-event-1".into()),
            content: MessageContent::text("retry payload ignored"),
        })
        .await
        .unwrap();
    assert_eq!(first.message_id, duplicate.message_id);
    assert!(duplicate.idempotent_replay);

    service
        .accept_inbound_message(AcceptInboundMessageRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
            actor_id: "actor-a".into(),
            source_binding_id: None,
            reply_target_binding_id: None,
            external_event_id: None,
            content: MessageContent::text("safe follow-up"),
        })
        .await
        .unwrap();

    service
        .create_summary_artifact(CreateSummaryArtifactRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
            start_sequence: 1,
            end_sequence: 2,
            summary_kind: SummaryKind::Compaction,
            content: MessageContent::text("summary that mentions secret token"),
            model_context_policy: Some(SummaryModelContextPolicy::ReplaceRangeWhenSelected),
        })
        .await
        .unwrap();

    service
        .redact_message(RedactMessageRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
            message_id: first.message_id,
            redaction_ref: "redaction/audit/1".into(),
        })
        .await
        .unwrap();

    let draft = service
        .append_assistant_draft(AppendAssistantDraftRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
            turn_run_id: "run-1".into(),
            content: MessageContent::text("partial"),
        })
        .await
        .unwrap();
    let duplicate_draft = service
        .append_assistant_draft(AppendAssistantDraftRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
            turn_run_id: "run-1".into(),
            content: MessageContent::text("retry partial ignored"),
        })
        .await
        .unwrap();
    assert_eq!(draft.message_id, duplicate_draft.message_id);
    service
        .update_assistant_draft(UpdateAssistantDraftRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
            message_id: draft.message_id,
            content: MessageContent::text("partial plus more"),
        })
        .await
        .unwrap();
    service
        .finalize_assistant_message(
            &scope,
            &thread.thread_id,
            draft.message_id,
            MessageContent::text("final answer"),
        )
        .await
        .unwrap();

    thread.thread_id
}

async fn assert_reopened_history(
    service: &impl SessionThreadService,
    label: &str,
    thread_id: ThreadId,
) {
    let thread_scope = scope(label);
    let history = service
        .list_thread_history(ThreadHistoryRequest {
            scope: thread_scope.clone(),
            thread_id: thread_id.clone(),
        })
        .await
        .unwrap();
    assert_eq!(history.thread.title.as_deref(), Some("Durable thread"));
    assert_eq!(history.messages.len(), 3);
    assert_eq!(history.messages[0].sequence, 1);
    assert_eq!(history.messages[0].status, MessageStatus::Redacted);
    assert!(history.messages[0].content.is_none());
    assert_eq!(
        history.messages[1].content.as_deref(),
        Some("safe follow-up")
    );
    assert_eq!(history.messages[2].kind, MessageKind::Assistant);
    assert_eq!(history.messages[2].status, MessageStatus::Finalized);
    assert_eq!(history.messages[2].content.as_deref(), Some("final answer"));
    assert_eq!(history.summary_artifacts.len(), 1);
    assert_eq!(history.summary_artifacts[0].content, "[redacted]");

    let context = service
        .load_context_window(LoadContextWindowRequest {
            scope: thread_scope,
            thread_id: thread_id.clone(),
            max_messages: 16,
        })
        .await
        .unwrap();
    assert_eq!(context.messages.len(), 2);
    assert_eq!(context.messages[0].content, "safe follow-up");
    assert_eq!(context.messages[1].content, "final answer");

    let wrong_scope = service
        .list_thread_history(ThreadHistoryRequest {
            scope: scope(&format!("{label}-wrong")),
            thread_id,
        })
        .await;
    assert!(wrong_scope.is_err());
}

/// Wait until the wall clock is strictly past `floor`, so the next thread
/// created/used gets a later activity timestamp — deterministic regardless
/// of clock resolution. Uses async sleep to avoid blocking the test runtime
/// (`std::thread::sleep` would block the tokio executor).
async fn wait_until_after(floor: chrono::DateTime<Utc>) {
    while Utc::now() <= floor {
        tokio::time::sleep(std::time::Duration::from_millis(1)).await;
    }
}

#[tokio::test]
async fn filesystem_list_threads_for_scope_is_scope_filtered_and_paginated() {
    use ironclaw_threads::ListThreadsForScopeRequest;

    let backend = Arc::new(InMemoryBackend::new());
    let scoped = scoped_threads_fs_at(backend, "tenant-host", "alice");
    let service = FilesystemSessionThreadService::new(scoped);

    let scope_a = scope("a");
    let scope_b = scope("b");

    // Empty store → empty list, no cursor (matches the missing-root
    // is_not_found arm in `list_dir`).
    let initial = service
        .list_threads_for_scope(ListThreadsForScopeRequest {
            scope: scope_a.clone(),
            limit: None,
            cursor: None,
        })
        .await
        .unwrap();
    assert!(initial.threads.is_empty(), "fresh store must be empty");
    assert!(initial.next_cursor.is_none());

    // Seed: 3 threads in scope A with deterministic ids so the
    // pagination assertion is stable. 1 thread in scope B that the
    // scope-A enumeration must not see — because the path layout
    // encodes scope axes, this also verifies the directory walk
    // doesn't leak across `(agent, project, owner)` cells.
    for id in ["t-a-001", "t-a-002", "t-a-003"] {
        let record = service
            .ensure_thread(EnsureThreadRequest {
                scope: scope_a.clone(),
                thread_id: Some(ThreadId::new(id).unwrap()),
                created_by_actor_id: "actor-a".into(),
                title: Some(id.into()),
                metadata_json: None,
            })
            .await
            .unwrap();
        // Wait past this thread's activity stamp → strictly increasing
        // `created_at`, so the activity-desc ordering below is deterministic.
        wait_until_after(record.updated_at.expect("new thread has activity stamp")).await;
    }
    service
        .ensure_thread(EnsureThreadRequest {
            scope: scope_b.clone(),
            thread_id: Some(ThreadId::new("t-b-001").unwrap()),
            created_by_actor_id: "actor-b".into(),
            title: None,
            metadata_json: None,
        })
        .await
        .unwrap();

    // Scope filter: A sees only A's threads, newest activity first.
    // The threads were created sequentially (with real backend I/O
    // between each `ensure_thread`), so their `created_at`/`updated_at`
    // stamps strictly increase — the activity-desc ordering therefore
    // surfaces the last-created thread (003) first.
    let scope_a_all = service
        .list_threads_for_scope(ListThreadsForScopeRequest {
            scope: scope_a.clone(),
            limit: None,
            cursor: None,
        })
        .await
        .unwrap();
    let ids: Vec<&str> = scope_a_all
        .threads
        .iter()
        .map(|record| record.thread_id.as_str())
        .collect();
    assert_eq!(ids, ["t-a-003", "t-a-002", "t-a-001"]);
    assert!(
        scope_a_all.next_cursor.is_none(),
        "no more pages when page size > total",
    );

    // Pagination: limit=2 → first page is [003, 002] with cursor=002.
    let page_1 = service
        .list_threads_for_scope(ListThreadsForScopeRequest {
            scope: scope_a.clone(),
            limit: Some(2),
            cursor: None,
        })
        .await
        .unwrap();
    let page_1_ids: Vec<&str> = page_1
        .threads
        .iter()
        .map(|record| record.thread_id.as_str())
        .collect();
    assert_eq!(page_1_ids, ["t-a-003", "t-a-002"]);
    assert_eq!(page_1.next_cursor.as_deref(), Some("t-a-002"));

    // Follow-up: cursor=002 → next page is [001] with no further cursor.
    let page_2 = service
        .list_threads_for_scope(ListThreadsForScopeRequest {
            scope: scope_a.clone(),
            limit: Some(2),
            cursor: page_1.next_cursor.clone(),
        })
        .await
        .unwrap();
    let page_2_ids: Vec<&str> = page_2
        .threads
        .iter()
        .map(|record| record.thread_id.as_str())
        .collect();
    assert_eq!(page_2_ids, ["t-a-001"]);
    assert!(page_2.next_cursor.is_none());

    // Cross-scope safety: scope B sees only its own thread, never A's.
    // For the filesystem backend this is structurally guaranteed by the
    // per-scope directory layout — `scope_axes_string` puts A and B at
    // different paths, so `list_dir` on B's root cannot return A's ids.
    let scope_b_all = service
        .list_threads_for_scope(ListThreadsForScopeRequest {
            scope: scope_b,
            limit: None,
            cursor: None,
        })
        .await
        .unwrap();
    let ids_b: Vec<&str> = scope_b_all
        .threads
        .iter()
        .map(|record| record.thread_id.as_str())
        .collect();
    assert_eq!(ids_b, ["t-b-001"]);
}

#[tokio::test]
async fn filesystem_list_threads_bootstraps_missing_thread_index_rows() {
    use ironclaw_threads::ListThreadsForScopeRequest;

    let backend = Arc::new(InMemoryBackend::new());
    let scoped = scoped_threads_fs_at(backend, "tenant-index-bootstrap", "alice");
    let service = FilesystemSessionThreadService::new(Arc::clone(&scoped));
    let scope = scope("index-bootstrap");

    for id in ["legacy-001", "legacy-002"] {
        service
            .ensure_thread(EnsureThreadRequest {
                scope: scope.clone(),
                thread_id: Some(ThreadId::new(id).unwrap()),
                created_by_actor_id: "actor-a".into(),
                title: Some(id.into()),
                metadata_json: None,
            })
            .await
            .unwrap();
    }

    for id in ["legacy-001", "legacy-002"] {
        scoped
            .delete(
                &scope.to_resource_scope(),
                &thread_index_record_path_for_test(&scope, id),
            )
            .await
            .expect("test setup removes derived index row");
    }
    service.clear_thread_index_cache_for_scope(&scope);

    let listed = service
        .list_threads_for_scope(ListThreadsForScopeRequest {
            scope: scope.clone(),
            limit: None,
            cursor: None,
        })
        .await
        .unwrap();
    let ids: Vec<&str> = listed
        .threads
        .iter()
        .map(|record| record.thread_id.as_str())
        .collect();
    assert_eq!(ids, ["legacy-002", "legacy-001"]);

    service.clear_thread_index_cache_for_scope(&scope);
    let listed_again = service
        .list_threads_for_scope(ListThreadsForScopeRequest {
            scope,
            limit: None,
            cursor: None,
        })
        .await
        .unwrap();
    let ids_again: Vec<&str> = listed_again
        .threads
        .iter()
        .map(|record| record.thread_id.as_str())
        .collect();
    assert_eq!(
        ids_again,
        ["legacy-002", "legacy-001"],
        "first list should rebuild durable derived index rows"
    );
}

#[tokio::test]
async fn filesystem_list_threads_merges_partial_thread_index_with_source_rows() {
    use ironclaw_threads::ListThreadsForScopeRequest;

    let backend = Arc::new(InMemoryBackend::new());
    let scoped = scoped_threads_fs_at(backend, "tenant-index-partial", "alice");
    let service = FilesystemSessionThreadService::new(Arc::clone(&scoped));
    let scope = scope("index-partial");

    for id in ["legacy-indexed", "legacy-missing"] {
        service
            .ensure_thread(EnsureThreadRequest {
                scope: scope.clone(),
                thread_id: Some(ThreadId::new(id).unwrap()),
                created_by_actor_id: "actor-a".into(),
                title: Some(id.into()),
                metadata_json: None,
            })
            .await
            .unwrap();
    }

    scoped
        .delete(
            &scope.to_resource_scope(),
            &thread_index_record_path_for_test(&scope, "legacy-missing"),
        )
        .await
        .expect("test setup removes one derived index row");
    service.clear_thread_index_cache_for_scope(&scope);

    let listed = service
        .list_threads_for_scope(ListThreadsForScopeRequest {
            scope,
            limit: None,
            cursor: None,
        })
        .await
        .unwrap();
    let ids: Vec<&str> = listed
        .threads
        .iter()
        .map(|record| record.thread_id.as_str())
        .collect();
    assert!(ids.contains(&"legacy-indexed"));
    assert!(ids.contains(&"legacy-missing"));
}

#[tokio::test]
async fn filesystem_list_threads_does_not_treat_partial_source_cache_as_complete() {
    use ironclaw_threads::ListThreadsForScopeRequest;

    let backend = Arc::new(InMemoryBackend::new());
    let scoped_a = scoped_threads_fs_at(Arc::clone(&backend), "tenant-index-cache", "alice");
    let scoped_b = scoped_threads_fs_at(backend, "tenant-index-cache", "alice");
    let service_a = FilesystemSessionThreadService::new(scoped_a);
    let service_b = FilesystemSessionThreadService::new(scoped_b);
    let scope = scope("index-cache");

    service_a
        .ensure_thread(EnsureThreadRequest {
            scope: scope.clone(),
            thread_id: Some(ThreadId::new("cached-source-a").unwrap()),
            created_by_actor_id: "actor-a".into(),
            title: Some("cached source a".into()),
            metadata_json: None,
        })
        .await
        .unwrap();
    service_b
        .ensure_thread(EnsureThreadRequest {
            scope: scope.clone(),
            thread_id: Some(ThreadId::new("cached-source-b").unwrap()),
            created_by_actor_id: "actor-b".into(),
            title: Some("cached source b".into()),
            metadata_json: None,
        })
        .await
        .unwrap();

    let listed = service_a
        .list_threads_for_scope(ListThreadsForScopeRequest {
            scope,
            limit: None,
            cursor: None,
        })
        .await
        .unwrap();
    let ids: Vec<&str> = listed
        .threads
        .iter()
        .map(|record| record.thread_id.as_str())
        .collect();

    assert!(ids.contains(&"cached-source-a"));
    assert!(
        ids.contains(&"cached-source-b"),
        "a single-row source cache from this process must not hide durable rows written by another service instance"
    );
}

#[tokio::test]
async fn filesystem_list_threads_retries_bootstrap_after_source_read_error() {
    use ironclaw_threads::ListThreadsForScopeRequest;

    let backend = Arc::new(FailOnceThreadRecordReadBackend::new("legacy-flaky-read"));
    let scoped = scoped_threads_fs_at(
        Arc::clone(&backend),
        "tenant-index-bootstrap-retry",
        "alice",
    );
    let service = FilesystemSessionThreadService::new(Arc::clone(&scoped));
    let scope = scope("index-bootstrap-retry");

    for id in ["legacy-indexed", "legacy-flaky-read"] {
        service
            .ensure_thread(EnsureThreadRequest {
                scope: scope.clone(),
                thread_id: Some(ThreadId::new(id).unwrap()),
                created_by_actor_id: "actor-a".into(),
                title: Some(id.into()),
                metadata_json: None,
            })
            .await
            .unwrap();
    }

    scoped
        .delete(
            &scope.to_resource_scope(),
            &thread_index_record_path_for_test(&scope, "legacy-flaky-read"),
        )
        .await
        .expect("test setup removes one derived index row");
    backend.fail_next_thread_record_read();
    service.clear_thread_index_cache_for_scope(&scope);

    let first = service
        .list_threads_for_scope(ListThreadsForScopeRequest {
            scope: scope.clone(),
            limit: None,
            cursor: None,
        })
        .await
        .unwrap();
    let first_ids: Vec<&str> = first
        .threads
        .iter()
        .map(|record| record.thread_id.as_str())
        .collect();
    assert!(first_ids.contains(&"legacy-indexed"));
    assert!(!first_ids.contains(&"legacy-flaky-read"));

    service.clear_thread_index_cache_for_scope(&scope);
    let second = service
        .list_threads_for_scope(ListThreadsForScopeRequest {
            scope,
            limit: None,
            cursor: None,
        })
        .await
        .unwrap();
    let second_ids: Vec<&str> = second
        .threads
        .iter()
        .map(|record| record.thread_id.as_str())
        .collect();
    assert!(
        second_ids.contains(&"legacy-flaky-read"),
        "a partial bootstrap read failure must not mark the scope complete"
    );
}

#[tokio::test]
async fn filesystem_list_threads_bootstrap_preserves_fresher_existing_index_activity() {
    use ironclaw_threads::ListThreadsForScopeRequest;

    let backend = Arc::new(InMemoryBackend::new());
    let scoped = scoped_threads_fs_at(backend, "tenant-index-bootstrap-fresh", "alice");
    let writer = FilesystemSessionThreadService::new(Arc::clone(&scoped));
    let request_scope = scope("index-bootstrap-fresh");

    let older = writer
        .ensure_thread(EnsureThreadRequest {
            scope: request_scope.clone(),
            thread_id: Some(ThreadId::new("thread-bootstrap-older").unwrap()),
            created_by_actor_id: "actor-a".into(),
            title: Some("older".into()),
            metadata_json: None,
        })
        .await
        .unwrap();
    wait_until_after(older.updated_at.expect("older thread has activity stamp")).await;
    let newer = writer
        .ensure_thread(EnsureThreadRequest {
            scope: request_scope.clone(),
            thread_id: Some(ThreadId::new("thread-bootstrap-newer").unwrap()),
            created_by_actor_id: "actor-a".into(),
            title: Some("newer".into()),
            metadata_json: None,
        })
        .await
        .unwrap();
    wait_until_after(newer.updated_at.expect("newer thread has activity stamp")).await;

    writer
        .append_finalized_assistant_message(AppendFinalizedAssistantMessageRequest {
            scope: request_scope.clone(),
            thread_id: older.thread_id.clone(),
            turn_run_id: "run-bootstrap-fresh".into(),
            content: MessageContent::text("fresh activity"),
        })
        .await
        .unwrap();

    let cold_bootstrap = FilesystemSessionThreadService::new(Arc::clone(&scoped));
    let listed = cold_bootstrap
        .list_threads_for_scope(ListThreadsForScopeRequest {
            scope: request_scope.clone(),
            limit: None,
            cursor: None,
        })
        .await
        .unwrap();
    let ids: Vec<&str> = listed
        .threads
        .iter()
        .map(|record| record.thread_id.as_str())
        .collect();
    assert_eq!(ids, ["thread-bootstrap-older", "thread-bootstrap-newer"]);

    let cold_after_bootstrap = FilesystemSessionThreadService::new(Arc::clone(&scoped));
    let listed_again = cold_after_bootstrap
        .list_threads_for_scope(ListThreadsForScopeRequest {
            scope: request_scope,
            limit: None,
            cursor: None,
        })
        .await
        .unwrap();
    let ids_again: Vec<&str> = listed_again
        .threads
        .iter()
        .map(|record| record.thread_id.as_str())
        .collect();
    assert_eq!(
        ids_again,
        ["thread-bootstrap-older", "thread-bootstrap-newer"],
        "bootstrap must not overwrite a fresher derived index row with stale thread.json activity"
    );
}

/// Regression: the "Recent" list must order by last interaction, not by
/// creation time or thread id. Appending a message to the *older* thread
/// has to bump it ahead of a more recently *created* one. Before this
/// fix, records carried no timestamp and the backend sorted by random
/// UUID, so a freshly-used thread could land anywhere in the list.
#[tokio::test]
async fn filesystem_list_threads_orders_by_last_activity_not_creation() {
    use ironclaw_threads::ListThreadsForScopeRequest;

    let backend = Arc::new(InMemoryBackend::new());
    let scoped = scoped_threads_fs_at(backend, "tenant-activity-fs", "alice");
    let service = FilesystemSessionThreadService::new(scoped);
    let scope_a = scope("activity");

    // Create "older" first, then "newer" — newer has the later
    // `created_at`. Waiting past each stamp keeps them strictly ordered.
    let mut newer_stamp = None;
    for id in ["t-older", "t-newer"] {
        let record = service
            .ensure_thread(EnsureThreadRequest {
                scope: scope_a.clone(),
                thread_id: Some(ThreadId::new(id).unwrap()),
                created_by_actor_id: "actor-a".into(),
                title: Some(id.into()),
                metadata_json: None,
            })
            .await
            .unwrap();
        let stamp = record.updated_at.expect("new thread has activity stamp");
        newer_stamp = Some(stamp);
        wait_until_after(stamp).await;
    }

    // Initially newest-created is first.
    let before = service
        .list_threads_for_scope(ListThreadsForScopeRequest {
            scope: scope_a.clone(),
            limit: None,
            cursor: None,
        })
        .await
        .unwrap();
    let before_ids: Vec<&str> = before
        .threads
        .iter()
        .map(|record| record.thread_id.as_str())
        .collect();
    assert_eq!(before_ids, ["t-newer", "t-older"]);

    // Interact with the older thread — appending a message must bump its
    // last-activity stamp above the newer thread's creation time. Wait
    // past the newer thread's stamp so the append is unambiguously later.
    wait_until_after(newer_stamp.expect("created both threads")).await;
    service
        .accept_inbound_message(AcceptInboundMessageRequest {
            scope: scope_a.clone(),
            thread_id: ThreadId::new("t-older").unwrap(),
            actor_id: "actor-a".into(),
            source_binding_id: Some("binding-activity".into()),
            reply_target_binding_id: None,
            external_event_id: Some("event-activity".into()),
            content: MessageContent::text("ping the old thread"),
        })
        .await
        .unwrap();

    // The freshly-used thread now leads the Recent list.
    let after = service
        .list_threads_for_scope(ListThreadsForScopeRequest {
            scope: scope_a.clone(),
            limit: None,
            cursor: None,
        })
        .await
        .unwrap();
    let after_ids: Vec<&str> = after
        .threads
        .iter()
        .map(|record| record.thread_id.as_str())
        .collect();
    assert_eq!(after_ids, ["t-older", "t-newer"]);

    // Cross-thread recency invariant the activity sort exists for: a
    // *chattier but staler* thread must NOT outrank a *quieter but more
    // recently touched* one. A per-thread-sequence sort (transcript length)
    // would wrongly float `t-newer` above `t-older` after the steps below.
    let older_stamp = service
        .read_thread(ThreadHistoryRequest {
            scope: scope_a.clone(),
            thread_id: ThreadId::new("t-older").unwrap(),
        })
        .await
        .unwrap()
        .updated_at
        .expect("touched thread has activity stamp");
    wait_until_after(older_stamp).await;

    // Pile several messages onto `t-newer`, raising its per-thread sequence
    // well above `t-older`'s — but at this earlier instant.
    for i in 0..3 {
        service
            .accept_inbound_message(AcceptInboundMessageRequest {
                scope: scope_a.clone(),
                thread_id: ThreadId::new("t-newer").unwrap(),
                actor_id: "actor-a".into(),
                source_binding_id: Some(format!("binding-chatter-{i}")),
                reply_target_binding_id: None,
                external_event_id: Some(format!("event-chatter-{i}")),
                content: MessageContent::text("chatter on the new thread"),
            })
            .await
            .unwrap();
    }
    let newer_stamp = service
        .read_thread(ThreadHistoryRequest {
            scope: scope_a.clone(),
            thread_id: ThreadId::new("t-newer").unwrap(),
        })
        .await
        .unwrap()
        .updated_at
        .expect("touched thread has activity stamp");
    wait_until_after(newer_stamp).await;

    // Touch `t-older` once more, strictly later. It now has FEWER total
    // messages than `t-newer` but the most recent activity.
    service
        .accept_inbound_message(AcceptInboundMessageRequest {
            scope: scope_a.clone(),
            thread_id: ThreadId::new("t-older").unwrap(),
            actor_id: "actor-a".into(),
            source_binding_id: Some("binding-activity-2".into()),
            reply_target_binding_id: None,
            external_event_id: Some("event-activity-2".into()),
            content: MessageContent::text("ping the old thread again"),
        })
        .await
        .unwrap();

    let final_list = service
        .list_threads_for_scope(ListThreadsForScopeRequest {
            scope: scope_a,
            limit: None,
            cursor: None,
        })
        .await
        .unwrap();
    let final_ids: Vec<&str> = final_list
        .threads
        .iter()
        .map(|record| record.thread_id.as_str())
        .collect();
    // Recency wins over transcript length.
    assert_eq!(final_ids, ["t-older", "t-newer"]);
}

#[tokio::test]
async fn filesystem_list_threads_for_scope_derives_title_from_first_user_message() {
    use ironclaw_threads::ListThreadsForScopeRequest;

    let backend = Arc::new(InMemoryBackend::new());
    let scoped = scoped_threads_fs_at(backend, "tenant-title-fs", "alice");
    let service = FilesystemSessionThreadService::new(scoped);
    let scope = scope("fs-title");

    // Thread #1: title-less, assistant speaks before the first user message.
    // Derivation must skip assistant records and pick the first non-empty
    // trimmed line from the earliest user message.
    let derived = service
        .ensure_thread(EnsureThreadRequest {
            scope: scope.clone(),
            thread_id: Some(ThreadId::new("t-derived").unwrap()),
            created_by_actor_id: "actor-a".into(),
            title: None,
            metadata_json: None,
        })
        .await
        .unwrap();
    service
        .append_assistant_draft(AppendAssistantDraftRequest {
            scope: scope.clone(),
            thread_id: derived.thread_id.clone(),
            turn_run_id: "run-derived-1".into(),
            content: MessageContent::text("assistant text must not become the title"),
        })
        .await
        .unwrap();
    service
        .accept_inbound_message(AcceptInboundMessageRequest {
            scope: scope.clone(),
            thread_id: derived.thread_id.clone(),
            actor_id: "actor-a".into(),
            source_binding_id: None,
            reply_target_binding_id: None,
            external_event_id: Some("evt-derived-1".into()),
            content: MessageContent::text("  hello there  \nsecond line"),
        })
        .await
        .unwrap();
    service
        .accept_inbound_message(AcceptInboundMessageRequest {
            scope: scope.clone(),
            thread_id: derived.thread_id.clone(),
            actor_id: "actor-a".into(),
            source_binding_id: None,
            reply_target_binding_id: None,
            external_event_id: Some("evt-derived-2".into()),
            content: MessageContent::text("later user message must not replace the title"),
        })
        .await
        .unwrap();

    // Thread #2: title-less and has no messages at all → must stay None
    // (the derive helper has nothing to extract from).
    service
        .ensure_thread(EnsureThreadRequest {
            scope: scope.clone(),
            thread_id: Some(ThreadId::new("t-empty").unwrap()),
            created_by_actor_id: "actor-a".into(),
            title: None,
            metadata_json: None,
        })
        .await
        .unwrap();

    // Thread #3: caller supplied an explicit title → list MUST preserve
    // it untouched. This is the "creator-supplied wins over derivation"
    // invariant.
    service
        .ensure_thread(EnsureThreadRequest {
            scope: scope.clone(),
            thread_id: Some(ThreadId::new("t-explicit").unwrap()),
            created_by_actor_id: "actor-a".into(),
            title: Some("Caller-supplied title".into()),
            metadata_json: None,
        })
        .await
        .unwrap();
    service
        .accept_inbound_message(AcceptInboundMessageRequest {
            scope: scope.clone(),
            thread_id: ThreadId::new("t-explicit").unwrap(),
            actor_id: "actor-a".into(),
            source_binding_id: None,
            reply_target_binding_id: None,
            external_event_id: Some("evt-explicit-1".into()),
            content: MessageContent::text("user message that must NOT replace the title"),
        })
        .await
        .unwrap();

    let listed = service
        .list_threads_for_scope(ListThreadsForScopeRequest {
            scope,
            limit: None,
            cursor: None,
        })
        .await
        .unwrap();

    let by_id: std::collections::HashMap<&str, Option<&str>> = listed
        .threads
        .iter()
        .map(|record| (record.thread_id.as_str(), record.title.as_deref()))
        .collect();

    assert_eq!(
        by_id.get("t-derived").copied().flatten(),
        Some("hello there"),
        "first user message should seed a trimmed first-line title",
    );
    assert!(
        by_id.get("t-empty").copied().flatten().is_none(),
        "thread with no user messages must stay title: None",
    );
    assert_eq!(
        by_id.get("t-explicit").copied().flatten(),
        Some("Caller-supplied title"),
        "explicit EnsureThreadRequest.title must not be overwritten by derivation",
    );
}

// ---------------------------------------------------------------------------
// mark_message_rejected_busy — filesystem backend coverage
// ---------------------------------------------------------------------------

#[tokio::test]
async fn filesystem_rejected_busy_marks_user_message_and_persists_status() {
    let backend = Arc::new(InMemoryBackend::new());
    let scoped = scoped_threads_fs_at(backend, "tenant-rb-ok", "alice");
    let service = FilesystemSessionThreadService::new(scoped);
    let scope = scope("rb-ok");

    let thread = service
        .ensure_thread(EnsureThreadRequest {
            scope: scope.clone(),
            thread_id: None,
            created_by_actor_id: "actor-a".into(),
            title: None,
            metadata_json: None,
        })
        .await
        .unwrap();

    let accepted = service
        .accept_inbound_message(AcceptInboundMessageRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
            actor_id: "actor-a".into(),
            source_binding_id: None,
            reply_target_binding_id: None,
            external_event_id: None,
            content: MessageContent::text("arrived while busy"),
        })
        .await
        .unwrap();
    let rejected = service
        .mark_message_rejected_busy(&scope, &thread.thread_id, accepted.message_id)
        .await
        .unwrap();
    assert_eq!(rejected.status, MessageStatus::RejectedBusy);
    assert!(rejected.turn_run_id.is_none());

    // Re-list to confirm the status was persisted to the filesystem store.
    let history = service
        .list_thread_history(ThreadHistoryRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id,
        })
        .await
        .unwrap();
    assert_eq!(history.messages.len(), 1);
    assert_eq!(history.messages[0].status, MessageStatus::RejectedBusy);
    assert!(history.messages[0].turn_run_id.is_none());
}

#[tokio::test]
async fn filesystem_rejected_busy_rejects_non_user_message() {
    let backend = Arc::new(InMemoryBackend::new());
    let scoped = scoped_threads_fs_at(backend, "tenant-rb-non-user", "alice");
    let service = FilesystemSessionThreadService::new(scoped);
    let scope = scope("rb-non-user");

    let thread = service
        .ensure_thread(EnsureThreadRequest {
            scope: scope.clone(),
            thread_id: None,
            created_by_actor_id: "actor-a".into(),
            title: None,
            metadata_json: None,
        })
        .await
        .unwrap();

    // An assistant draft is not a user message — the transition must be rejected.
    let draft = service
        .append_assistant_draft(AppendAssistantDraftRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
            turn_run_id: "run-1".into(),
            content: MessageContent::text("partial"),
        })
        .await
        .unwrap();

    let result = service
        .mark_message_rejected_busy(&scope, &thread.thread_id, draft.message_id)
        .await;

    assert!(
        matches!(
            result,
            Err(SessionThreadError::InvalidMessageTransition { .. })
        ),
        "mark_message_rejected_busy must return InvalidMessageTransition for a non-user (assistant draft) message, got {result:?}"
    );
}

#[tokio::test]
async fn filesystem_rejected_busy_rejects_already_submitted_user_message() {
    let backend = Arc::new(InMemoryBackend::new());
    let scoped = scoped_threads_fs_at(backend, "tenant-rb-submitted", "alice");
    let service = FilesystemSessionThreadService::new(scoped);
    let scope = scope("rb-submitted");

    let thread = service
        .ensure_thread(EnsureThreadRequest {
            scope: scope.clone(),
            thread_id: None,
            created_by_actor_id: "actor-a".into(),
            title: None,
            metadata_json: None,
        })
        .await
        .unwrap();

    let accepted = service
        .accept_inbound_message(AcceptInboundMessageRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
            actor_id: "actor-a".into(),
            source_binding_id: None,
            reply_target_binding_id: None,
            external_event_id: None,
            content: MessageContent::text("already submitted"),
        })
        .await
        .unwrap();

    // Advance past Accepted → Submitted so the message is finalized.
    service
        .mark_message_submitted(
            &scope,
            &thread.thread_id,
            accepted.message_id,
            "turn-id-x".into(),
            "run-id-x".into(),
        )
        .await
        .unwrap();

    let result = service
        .mark_message_rejected_busy(&scope, &thread.thread_id, accepted.message_id)
        .await;

    assert!(
        matches!(
            result,
            Err(SessionThreadError::InvalidMessageTransition { .. })
        ),
        "mark_message_rejected_busy must return InvalidMessageTransition on an already-submitted user message, got {result:?}"
    );
}

#[tokio::test]
async fn filesystem_rejected_busy_cannot_be_marked_submitted_is_terminal() {
    // RejectedBusy is a durable terminal state — the stored row must never
    // transition to Submitted.  ensure_user_accepted no longer admits
    // RejectedBusy, so mark_message_submitted must return
    // InvalidMessageTransition and the persisted status must remain RejectedBusy.
    let backend = Arc::new(InMemoryBackend::new());
    let scoped = scoped_threads_fs_at(backend, "tenant-rb-terminal", "alice");
    let service = FilesystemSessionThreadService::new(scoped);
    let scope = scope("rb-terminal");

    let thread = service
        .ensure_thread(EnsureThreadRequest {
            scope: scope.clone(),
            thread_id: None,
            created_by_actor_id: "actor-a".into(),
            title: None,
            metadata_json: None,
        })
        .await
        .unwrap();

    let accepted = service
        .accept_inbound_message(AcceptInboundMessageRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
            actor_id: "actor-a".into(),
            source_binding_id: None,
            reply_target_binding_id: None,
            external_event_id: None,
            content: MessageContent::text("resend after busy"),
        })
        .await
        .unwrap();

    // Drive the message into RejectedBusy.
    service
        .mark_message_rejected_busy(&scope, &thread.thread_id, accepted.message_id)
        .await
        .unwrap();

    // Attempting to submit the rejected row must fail — RejectedBusy is terminal.
    let result = service
        .mark_message_submitted(
            &scope,
            &thread.thread_id,
            accepted.message_id,
            "turn-id-resend".into(),
            "run-id-resend".into(),
        )
        .await;

    assert!(
        matches!(
            result,
            Err(SessionThreadError::InvalidMessageTransition { .. })
        ),
        "mark_message_submitted must fail with InvalidMessageTransition on a RejectedBusy message (terminal state), got {result:?}"
    );

    // Re-list to confirm the status was NOT mutated in the filesystem store.
    let history = service
        .list_thread_history(ThreadHistoryRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id,
        })
        .await
        .unwrap();
    assert_eq!(
        history.messages[0].status,
        MessageStatus::RejectedBusy,
        "persisted status must remain RejectedBusy after the failed Submitted transition"
    );
}

#[tokio::test]
async fn legacy_deferred_busy_message_round_trips_through_filesystem_store() {
    // Regression guard for the on-disk legacy `deferred_busy` status.
    // `DeferredBusy` is no longer written by new code but may exist in older
    // transcripts. This test proves that a row injected with that status
    // survives the JSON serialize → filesystem store → deserialize round-trip
    // with the status preserved and still appears in history.
    let backend = Arc::new(InMemoryBackend::new());
    let scoped = scoped_threads_fs_at(backend, "tenant-legacy-db", "alice");
    let service = FilesystemSessionThreadService::new(scoped);
    let scope = scope("legacy-deferred-busy");

    let thread = service
        .ensure_thread(EnsureThreadRequest {
            scope: scope.clone(),
            thread_id: None,
            created_by_actor_id: "actor-a".into(),
            title: None,
            metadata_json: None,
        })
        .await
        .unwrap();

    let accepted = service
        .accept_inbound_message(AcceptInboundMessageRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
            actor_id: "actor-a".into(),
            source_binding_id: None,
            reply_target_binding_id: None,
            external_event_id: None,
            content: MessageContent::text("arrived while busy"),
        })
        .await
        .unwrap();

    // Inject a legacy DeferredBusy row directly — the mark_message_deferred_busy
    // writer has been retired; this back-door preserves read/replay coverage.
    service
        .inject_legacy_deferred_busy_for_test(&scope, &thread.thread_id, accepted.message_id)
        .await
        .unwrap();

    let history = service
        .list_thread_history(ThreadHistoryRequest {
            scope,
            thread_id: thread.thread_id,
        })
        .await
        .unwrap();
    assert_eq!(
        history.messages.len(),
        1,
        "legacy DeferredBusy message must appear in history"
    );
    assert_eq!(
        history.messages[0].status,
        MessageStatus::DeferredBusy,
        "on-disk legacy deferred_busy status must round-trip without mutation"
    );
    assert!(
        history.messages[0].turn_run_id.is_none(),
        "legacy DeferredBusy message must have no turn_run_id"
    );
}

/// Regression: `ensure_thread` was migrated to the shared `cas_update`
/// helper, which maps `CasUpdateError::CasUnsupported` to
/// `SessionThreadError::Backend(...)` (`map_cas_error`,
/// `filesystem_service.rs`). Existing `ensure_thread` coverage only runs
/// over `InMemoryBackend`, which supports versioned CAS — so the
/// byte-only/`Unsupported` fail-closed branch for thread creation was
/// unpinned.
///
/// `LocalFilesystem` is the canonical byte-only `RootFilesystem`: its
/// `put` impl rejects entries with `kind.is_some()`, which `cas_update`
/// surfaces as `CasUnsupported`. This mirrors
/// `filesystem_approval_store_fails_closed_on_byte_only_backend` in
/// `crates/ironclaw_run_state/tests/run_state_contract.rs`.
#[tokio::test]
async fn filesystem_session_thread_ensure_thread_fails_closed_on_byte_only_backend() {
    let dir = tempfile::tempdir().expect("temp dir");
    let mut local_fs = LocalFilesystem::new();
    local_fs
        .mount_local(
            VirtualPath::new("/tenants").expect("virtual root"),
            HostPath::from_path_buf(dir.path().to_path_buf()),
        )
        .expect("mount /tenants at temp dir");
    let scoped = scoped_threads_fs_at(Arc::new(local_fs), "tenant-byte-only", "alice");
    let service = FilesystemSessionThreadService::new(scoped);

    let err = service
        .ensure_thread(EnsureThreadRequest {
            scope: scope("byte-only"),
            thread_id: Some(ThreadId::new("thread-byte-only").unwrap()),
            created_by_actor_id: "actor-a".into(),
            title: None,
            metadata_json: None,
        })
        .await
        .expect_err("ensure_thread must fail closed on a byte-only backend");
    assert!(
        matches!(&err, SessionThreadError::Backend(msg) if msg.contains("compare-and-swap")),
        "expected Backend(CasUnsupported) from byte-only LocalFilesystem but got {err:?}",
    );
}

fn scope(label: &str) -> ThreadScope {
    ThreadScope {
        tenant_id: TenantId::new(format!("tenant-{label}")).unwrap(),
        agent_id: AgentId::new(format!("agent-{label}")).unwrap(),
        project_id: Some(ProjectId::new(format!("project-{label}")).unwrap()),
        owner_user_id: Some(UserId::new(format!("user-{label}")).unwrap()),
        mission_id: None,
    }
}

fn thread_index_record_path_for_test(scope: &ThreadScope, thread_id: &str) -> ScopedPath {
    ScopedPath::new(format!(
        "/threads/agents/{}/projects/{}/owners/{}/thread_index/{thread_id}.json",
        scope.agent_id.as_str(),
        scope
            .project_id
            .as_ref()
            .expect("test scope has project")
            .as_str(),
        scope
            .owner_user_id
            .as_ref()
            .expect("test scope has owner")
            .as_str()
    ))
    .unwrap()
}

fn thread_root_path_for_test(scope: &ThreadScope, thread_id: &str) -> ScopedPath {
    ScopedPath::new(format!(
        "/threads/agents/{}/projects/{}/owners/{}/threads/{thread_id}",
        scope.agent_id.as_str(),
        scope
            .project_id
            .as_ref()
            .expect("test scope has project")
            .as_str(),
        scope
            .owner_user_id
            .as_ref()
            .expect("test scope has owner")
            .as_str()
    ))
    .unwrap()
}

fn assert_unknown_thread(error: SessionThreadError, thread_id: &ThreadId) {
    match error {
        SessionThreadError::UnknownThread { thread_id: actual } => assert_eq!(actual, *thread_id),
        other => panic!("expected UnknownThread for {thread_id}, got {other:?}"),
    }
}

fn preview_envelope(invocation_id: InvocationId) -> CapabilityDisplayPreviewEnvelope {
    CapabilityDisplayPreviewEnvelope::new(CapabilityDisplayPreviewEnvelopeInput {
        invocation_id,
        capability_id: CapabilityId::new("demo.echo").unwrap(),
        status: CapabilityDisplayPreviewStatus::Completed,
        title: "echo".to_string(),
        subtitle: None,
        input_summary: Some("{\"message\":\"hello\"}".to_string()),
        output_summary: Some("text output".to_string()),
        output_preview: Some("hello".to_string()),
        output_kind: Some("text".to_string()),
        output_bytes: Some(5),
        result_ref: Some("result:demo-preview".to_string()),
        truncated: false,
        updated_at: Utc::now(),
        activity_order: None,
    })
    .unwrap()
}

/// Wrap a [`RootFilesystem`] in a [`ScopedFilesystem`] that exposes the
/// `/threads` alias rooted under a single tenant/user subtree of the
/// underlying backend. The `tenant`/`user` arguments map to the
/// production composition's `invocation_mount_view`-style rewriting:
/// `/threads → /tenants/<tenant>/users/<user>/threads`. Two
/// `ScopedFilesystem`s built with different `tenant` arguments over the
/// same `RootFilesystem` cannot see each other's data.
fn scoped_threads_fs_at<F>(backend: Arc<F>, tenant: &str, user: &str) -> Arc<ScopedFilesystem<F>>
where
    F: RootFilesystem,
{
    let target = format!("/tenants/{tenant}/users/{user}/threads");
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/threads").expect("alias"),
        VirtualPath::new(target).expect("target"),
        MountPermissions::read_write_list_delete(),
    )])
    .expect("mount view");
    Arc::new(ScopedFilesystem::with_fixed_view(backend, mounts))
}

struct LookupIndexWriteFailureBackend {
    inner: InMemoryBackend,
}

impl LookupIndexWriteFailureBackend {
    fn new() -> Self {
        Self {
            inner: InMemoryBackend::new(),
        }
    }

    fn is_lookup_index_path(path: &VirtualPath) -> bool {
        path.as_str().contains("/indexes/assistant-runs/")
            || path.as_str().contains("/indexes/tool-results/")
    }
}

struct LookupIndexReadFailureBackend {
    inner: InMemoryBackend,
}

impl LookupIndexReadFailureBackend {
    fn new() -> Self {
        Self {
            inner: InMemoryBackend::new(),
        }
    }
}

struct QueryCountingBackend {
    inner: InMemoryBackend,
    query_count: AtomicUsize,
    get_count: AtomicUsize,
}

impl QueryCountingBackend {
    fn new() -> Self {
        Self {
            inner: InMemoryBackend::new(),
            query_count: AtomicUsize::new(0),
            get_count: AtomicUsize::new(0),
        }
    }

    fn query_count(&self) -> usize {
        self.query_count.load(Ordering::SeqCst)
    }

    fn get_count(&self) -> usize {
        self.get_count.load(Ordering::SeqCst)
    }
}

struct FailOnceThreadRecordReadBackend {
    inner: InMemoryBackend,
    thread_id: String,
    fail_next_read: AtomicBool,
}

impl FailOnceThreadRecordReadBackend {
    fn new(thread_id: &str) -> Self {
        Self {
            inner: InMemoryBackend::new(),
            thread_id: thread_id.to_string(),
            fail_next_read: AtomicBool::new(false),
        }
    }

    fn fail_next_thread_record_read(&self) {
        self.fail_next_read.store(true, Ordering::SeqCst);
    }

    fn is_target_thread_record_path(&self, path: &VirtualPath) -> bool {
        path.as_str()
            .contains(&format!("/threads/{}/thread.json", self.thread_id))
    }
}

struct TransactionalRaceBackend {
    inner: Arc<InMemoryBackend>,
    txn_lock: Arc<Mutex<()>>,
    idempotency_get_barrier: Arc<Barrier>,
    idempotency_get_count: AtomicUsize,
}

impl TransactionalRaceBackend {
    fn new() -> Self {
        Self {
            inner: Arc::new(InMemoryBackend::new()),
            txn_lock: Arc::new(Mutex::new(())),
            idempotency_get_barrier: Arc::new(Barrier::new(2)),
            idempotency_get_count: AtomicUsize::new(0),
        }
    }

    fn is_idempotency_path(path: &VirtualPath) -> bool {
        path.as_str().contains("/threads/idempotency/")
    }
}

struct TransactionalRaceTxn {
    inner: Arc<InMemoryBackend>,
    prefix: VirtualPath,
    _guard: OwnedMutexGuard<()>,
    staged_puts: HashMap<VirtualPath, (Entry, RecordVersion)>,
}

impl TransactionalRaceTxn {
    fn check_path(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        if std::path::Path::new(path.as_str())
            .starts_with(std::path::Path::new(self.prefix.as_str()))
        {
            Ok(())
        } else {
            Err(FilesystemError::PathOutsideMount { path: path.clone() })
        }
    }

    async fn current_version(
        &self,
        path: &VirtualPath,
    ) -> Result<Option<RecordVersion>, FilesystemError> {
        if let Some((_, version)) = self.staged_puts.get(path) {
            return Ok(Some(*version));
        }
        Ok(self
            .inner
            .get(path)
            .await?
            .map(|versioned| versioned.version))
    }

    fn check_cas(
        path: &VirtualPath,
        cas: CasExpectation,
        current: Option<RecordVersion>,
    ) -> Result<RecordVersion, FilesystemError> {
        match (cas, current) {
            (CasExpectation::Any, current) => Ok(current
                .map(|version| version.next())
                .unwrap_or_else(|| RecordVersion::from_backend(1))),
            (CasExpectation::Absent, None) => Ok(RecordVersion::from_backend(1)),
            (CasExpectation::Absent, found @ Some(_)) => Err(FilesystemError::VersionMismatch {
                path: path.clone(),
                expected: None,
                found,
            }),
            (CasExpectation::Version(expected), Some(found)) if expected == found => {
                Ok(expected.next())
            }
            (CasExpectation::Version(expected), found) => Err(FilesystemError::VersionMismatch {
                path: path.clone(),
                expected: Some(expected),
                found,
            }),
        }
    }
}

#[async_trait]
impl RootFilesystem for LookupIndexWriteFailureBackend {
    fn capabilities(&self) -> BackendCapabilities {
        self.inner.capabilities()
    }

    async fn put(
        &self,
        path: &VirtualPath,
        entry: Entry,
        cas: CasExpectation,
    ) -> Result<RecordVersion, FilesystemError> {
        if Self::is_lookup_index_path(path) {
            return Err(FilesystemError::Backend {
                path: path.clone(),
                operation: FilesystemOperation::WriteFile,
                reason: "lookup index writes disabled by contract test".to_string(),
            });
        }
        self.inner.put(path, entry, cas).await
    }

    async fn get(&self, path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
        self.inner.get(path).await
    }

    async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
        self.inner.list_dir(path).await
    }

    async fn query(
        &self,
        path: &VirtualPath,
        filter: &Filter,
        page: Page,
    ) -> Result<Vec<VersionedEntry>, FilesystemError> {
        self.inner.query(path, filter, page).await
    }

    async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
        self.inner.stat(path).await
    }

    async fn delete(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        self.inner.delete(path).await
    }
}

#[async_trait]
impl RootFilesystem for LookupIndexReadFailureBackend {
    fn capabilities(&self) -> BackendCapabilities {
        self.inner.capabilities()
    }

    async fn put(
        &self,
        path: &VirtualPath,
        entry: Entry,
        cas: CasExpectation,
    ) -> Result<RecordVersion, FilesystemError> {
        self.inner.put(path, entry, cas).await
    }

    async fn get(&self, path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
        if LookupIndexWriteFailureBackend::is_lookup_index_path(path) {
            return Err(FilesystemError::Backend {
                path: path.clone(),
                operation: FilesystemOperation::ReadFile,
                reason: "lookup index reads disabled by contract test".to_string(),
            });
        }
        self.inner.get(path).await
    }

    async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
        self.inner.list_dir(path).await
    }

    async fn query(
        &self,
        path: &VirtualPath,
        filter: &Filter,
        page: Page,
    ) -> Result<Vec<VersionedEntry>, FilesystemError> {
        self.inner.query(path, filter, page).await
    }

    async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
        self.inner.stat(path).await
    }

    async fn delete(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        self.inner.delete(path).await
    }
}

#[async_trait]
impl RootFilesystem for QueryCountingBackend {
    fn capabilities(&self) -> BackendCapabilities {
        self.inner.capabilities()
    }

    async fn put(
        &self,
        path: &VirtualPath,
        entry: Entry,
        cas: CasExpectation,
    ) -> Result<RecordVersion, FilesystemError> {
        self.inner.put(path, entry, cas).await
    }

    async fn get(&self, path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
        self.get_count.fetch_add(1, Ordering::SeqCst);
        self.inner.get(path).await
    }

    async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
        self.inner.list_dir(path).await
    }

    async fn query(
        &self,
        path: &VirtualPath,
        filter: &Filter,
        page: Page,
    ) -> Result<Vec<VersionedEntry>, FilesystemError> {
        self.query_count.fetch_add(1, Ordering::SeqCst);
        self.inner.query(path, filter, page).await
    }

    async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
        self.inner.stat(path).await
    }

    async fn delete(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        self.inner.delete(path).await
    }

    async fn begin(&self, path: &VirtualPath) -> Result<Box<dyn StorageTxn>, FilesystemError> {
        self.inner.begin(path).await
    }

    async fn reserve_sequence(&self, path: &VirtualPath) -> Result<SeqNo, FilesystemError> {
        self.inner.reserve_sequence(path).await
    }
}

#[async_trait]
impl RootFilesystem for FailOnceThreadRecordReadBackend {
    fn capabilities(&self) -> BackendCapabilities {
        self.inner.capabilities()
    }

    async fn put(
        &self,
        path: &VirtualPath,
        entry: Entry,
        cas: CasExpectation,
    ) -> Result<RecordVersion, FilesystemError> {
        self.inner.put(path, entry, cas).await
    }

    async fn get(&self, path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
        if self.is_target_thread_record_path(path)
            && self.fail_next_read.swap(false, Ordering::SeqCst)
        {
            return Err(FilesystemError::Backend {
                path: path.clone(),
                operation: FilesystemOperation::ReadFile,
                reason: "thread record reads disabled once by contract test".to_string(),
            });
        }
        self.inner.get(path).await
    }

    async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
        self.inner.list_dir(path).await
    }

    async fn query(
        &self,
        path: &VirtualPath,
        filter: &Filter,
        page: Page,
    ) -> Result<Vec<VersionedEntry>, FilesystemError> {
        self.inner.query(path, filter, page).await
    }

    async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
        self.inner.stat(path).await
    }

    async fn delete(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        self.inner.delete(path).await
    }
}

#[async_trait]
impl RootFilesystem for TransactionalRaceBackend {
    fn capabilities(&self) -> BackendCapabilities {
        self.inner.capabilities().with_txn(TxnCapability::MultiKey)
    }

    async fn put(
        &self,
        path: &VirtualPath,
        entry: Entry,
        cas: CasExpectation,
    ) -> Result<RecordVersion, FilesystemError> {
        self.inner.put(path, entry, cas).await
    }

    async fn get(&self, path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
        if Self::is_idempotency_path(path)
            && self.idempotency_get_count.fetch_add(1, Ordering::SeqCst) < 2
        {
            let result = self.inner.get(path).await?;
            self.idempotency_get_barrier.wait().await;
            return Ok(result);
        }
        self.inner.get(path).await
    }

    async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
        self.inner.list_dir(path).await
    }

    async fn query(
        &self,
        path: &VirtualPath,
        filter: &Filter,
        page: Page,
    ) -> Result<Vec<VersionedEntry>, FilesystemError> {
        self.inner.query(path, filter, page).await
    }

    async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
        self.inner.stat(path).await
    }

    async fn delete(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        self.inner.delete(path).await
    }

    async fn begin(&self, path: &VirtualPath) -> Result<Box<dyn StorageTxn>, FilesystemError> {
        let guard = Arc::clone(&self.txn_lock).lock_owned().await;
        Ok(Box::new(TransactionalRaceTxn {
            inner: Arc::clone(&self.inner),
            prefix: path.clone(),
            _guard: guard,
            staged_puts: HashMap::new(),
        }))
    }
}

#[async_trait]
impl StorageTxn for TransactionalRaceTxn {
    async fn put(
        &mut self,
        path: &VirtualPath,
        entry: Entry,
        cas: CasExpectation,
    ) -> Result<RecordVersion, FilesystemError> {
        self.check_path(path)?;
        let version = Self::check_cas(path, cas, self.current_version(path).await?)?;
        self.staged_puts.insert(path.clone(), (entry, version));
        Ok(version)
    }

    async fn get(&mut self, path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
        self.check_path(path)?;
        if let Some((entry, version)) = self.staged_puts.get(path) {
            return Ok(Some(VersionedEntry {
                path: path.clone(),
                entry: entry.clone(),
                version: *version,
            }));
        }
        self.inner.get(path).await
    }

    async fn delete(&mut self, path: &VirtualPath) -> Result<(), FilesystemError> {
        self.check_path(path)?;
        Err(FilesystemError::Unsupported {
            path: path.clone(),
            operation: FilesystemOperation::Delete,
        })
    }

    async fn commit(self: Box<Self>) -> Result<(), FilesystemError> {
        let txn = *self;
        for (path, (entry, _)) in txn.staged_puts {
            txn.inner.put(&path, entry, CasExpectation::Any).await?;
        }
        Ok(())
    }

    async fn rollback(self: Box<Self>) {}
}

#[tokio::test]
async fn filesystem_persists_attachment_refs_and_clears_them_on_redaction() {
    let backend = Arc::new(InMemoryBackend::new());
    let scoped = scoped_threads_fs_at(backend, "tenant-attachments", "alice");
    let service = FilesystemSessionThreadService::new(Arc::clone(&scoped));
    let scope = scope("attachments");
    let thread = service
        .ensure_thread(EnsureThreadRequest {
            scope: scope.clone(),
            thread_id: Some(ThreadId::new("thread-attachments").unwrap()),
            created_by_actor_id: "actor-a".into(),
            title: None,
            metadata_json: None,
        })
        .await
        .unwrap();

    let attachment = AttachmentRef {
        id: "att-1".into(),
        kind: AttachmentKind::Image,
        mime_type: "image/png".into(),
        filename: Some("diagram.png".into()),
        size_bytes: Some(4096),
        storage_key: Some("attachments/2026-06-09/m1-diagram.png".into()),
        extracted_text: None,
    };
    let accepted = service
        .accept_inbound_message(AcceptInboundMessageRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
            actor_id: "actor-a".into(),
            source_binding_id: None,
            reply_target_binding_id: None,
            external_event_id: Some("event-att".into()),
            content: MessageContent::with_attachments("look at this", vec![attachment.clone()]),
        })
        .await
        .unwrap();

    // Re-open the store over the same backend to prove the refs survive a
    // serialize → store → deserialize round trip, not just an in-process cache.
    let reopened = FilesystemSessionThreadService::new(scoped);
    let history = reopened
        .list_thread_history(ThreadHistoryRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
        })
        .await
        .unwrap();
    assert_eq!(history.messages.len(), 1);
    assert_eq!(history.messages[0].attachments, vec![attachment]);

    reopened
        .redact_message(RedactMessageRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
            message_id: accepted.message_id,
            redaction_ref: "redaction:test".into(),
        })
        .await
        .unwrap();

    let after = reopened
        .list_thread_history(ThreadHistoryRequest {
            scope,
            thread_id: thread.thread_id,
        })
        .await
        .unwrap();
    assert_eq!(after.messages[0].status, MessageStatus::Redacted);
    assert!(after.messages[0].content.is_none());
    assert!(after.messages[0].attachments.is_empty());
}

#[tokio::test]
async fn filesystem_persists_multiple_attachment_refs_in_order() {
    // The single-ref test can't catch an ordering or per-element bug in the
    // JSON array round trip. Drive a multi-ref message — distinct kinds, one
    // with `extracted_text: Some(..)` (which the single-ref test never sets) —
    // through the real serialize → store → deserialize path and assert the full
    // vec survives in order.
    let backend = Arc::new(InMemoryBackend::new());
    let scoped = scoped_threads_fs_at(backend, "tenant-multi-att", "alice");
    let service = FilesystemSessionThreadService::new(Arc::clone(&scoped));
    let scope = scope("multi-attachments");
    let thread = service
        .ensure_thread(EnsureThreadRequest {
            scope: scope.clone(),
            thread_id: Some(ThreadId::new("thread-multi-att").unwrap()),
            created_by_actor_id: "actor-a".into(),
            title: None,
            metadata_json: None,
        })
        .await
        .unwrap();

    let attachments = vec![
        AttachmentRef {
            id: "att-1".into(),
            kind: AttachmentKind::Image,
            mime_type: "image/png".into(),
            filename: Some("diagram.png".into()),
            size_bytes: Some(4096),
            storage_key: Some("attachments/2026-06-09/m1-0-diagram.png".into()),
            extracted_text: None,
        },
        AttachmentRef {
            id: "att-2".into(),
            kind: AttachmentKind::Document,
            mime_type: "application/pdf".into(),
            filename: Some("report.pdf".into()),
            size_bytes: Some(20_480),
            storage_key: Some("attachments/2026-06-09/m1-1-report.pdf".into()),
            extracted_text: Some("Quarterly revenue up 12%".into()),
        },
        AttachmentRef {
            id: "att-3".into(),
            kind: AttachmentKind::Audio,
            mime_type: "audio/mpeg".into(),
            filename: Some("note.mp3".into()),
            size_bytes: Some(8192),
            storage_key: Some("attachments/2026-06-09/m1-2-note.mp3".into()),
            extracted_text: None,
        },
    ];
    service
        .accept_inbound_message(AcceptInboundMessageRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
            actor_id: "actor-a".into(),
            source_binding_id: None,
            reply_target_binding_id: None,
            external_event_id: Some("event-multi-att".into()),
            content: MessageContent::with_attachments("three files", attachments.clone()),
        })
        .await
        .unwrap();

    // Re-open over the same backend so the assertion crosses the real JSON
    // serialize → store → deserialize boundary.
    let reopened = FilesystemSessionThreadService::new(scoped);
    let history = reopened
        .list_thread_history(ThreadHistoryRequest {
            scope,
            thread_id: thread.thread_id,
        })
        .await
        .unwrap();
    assert_eq!(history.messages.len(), 1);
    assert_eq!(history.messages[0].attachments, attachments);
}

#[tokio::test]
async fn filesystem_accept_rejects_duplicate_attachment_ids() {
    use ironclaw_threads::ListThreadsForScopeRequest;
    // The accept path validates attachment refs before persisting. Drive the
    // real caller (not just the helper) so a regression that drops the check
    // would fail here, and assert nothing was written on rejection.
    let backend = Arc::new(InMemoryBackend::new());
    let scoped = scoped_threads_fs_at(backend, "tenant-dup-att", "alice");
    let service = FilesystemSessionThreadService::new(Arc::clone(&scoped));
    let scope = scope("dup-attachments");
    let thread = service
        .ensure_thread(EnsureThreadRequest {
            scope: scope.clone(),
            thread_id: Some(ThreadId::new("thread-dup-att").unwrap()),
            created_by_actor_id: "actor-a".into(),
            title: None,
            metadata_json: None,
        })
        .await
        .unwrap();

    // Capture the creation activity stamp and let the clock advance, so a
    // spurious activity bump on the rejected accept would be strictly later
    // and therefore observable below.
    let created_activity = thread.updated_at.expect("new thread has activity stamp");
    wait_until_after(created_activity).await;

    let dup = AttachmentRef {
        id: "att-dup".into(),
        kind: AttachmentKind::Image,
        mime_type: "image/png".into(),
        filename: Some("diagram.png".into()),
        size_bytes: Some(4096),
        storage_key: Some("attachments/2026-06-09/m1-0-diagram.png".into()),
        extracted_text: None,
    };
    let err = service
        .accept_inbound_message(AcceptInboundMessageRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
            actor_id: "actor-a".into(),
            source_binding_id: None,
            reply_target_binding_id: None,
            external_event_id: Some("event-dup-att".into()),
            content: MessageContent::with_attachments("two refs, one id", vec![dup.clone(), dup]),
        })
        .await
        .expect_err("duplicate attachment ids must be rejected at accept");
    assert!(matches!(err, SessionThreadError::InvalidAttachment(_)));

    // Rejection must not leave a half-written message behind.
    let history = service
        .list_thread_history(ThreadHistoryRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
        })
        .await
        .unwrap();
    assert!(history.messages.is_empty());

    // Rejection must also not bump the thread's last-activity stamp —
    // otherwise an invalid message would float the thread to the top of
    // the sidebar without ever being appended.
    let listed = service
        .list_threads_for_scope(ListThreadsForScopeRequest {
            scope,
            limit: None,
            cursor: None,
        })
        .await
        .unwrap();
    let record = listed
        .threads
        .iter()
        .find(|record| record.thread_id == thread.thread_id)
        .expect("thread is still listed");
    assert_eq!(
        record.updated_at,
        Some(created_activity),
        "rejected attachment must not bump last-activity",
    );
}

/// Mirrors `summary_spanning_interior_rejected_busy_is_applied` from the
/// in-memory contract suite.  A compaction summary whose span contains an
/// interior RejectedBusy message (permanently-terminal, never resurfaces)
/// MUST be applied by the filesystem backend.
#[tokio::test]
async fn filesystem_summary_spanning_interior_rejected_busy_is_applied() {
    let backend = Arc::new(InMemoryBackend::new());
    let scoped = scoped_threads_fs_at(backend, "tenant-rej-busy-sum", "alice");
    let service = FilesystemSessionThreadService::new(scoped);
    let scope = scope("rej-busy-sum");

    let thread = service
        .ensure_thread(EnsureThreadRequest {
            scope: scope.clone(),
            thread_id: Some(ThreadId::new("thread-rej-busy-sum").unwrap()),
            created_by_actor_id: "actor-a".into(),
            title: None,
            metadata_json: None,
        })
        .await
        .unwrap();

    // seq 1: visible user message
    service
        .accept_inbound_message(AcceptInboundMessageRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
            actor_id: "actor-a".into(),
            source_binding_id: None,
            reply_target_binding_id: None,
            external_event_id: None,
            content: MessageContent::text("first"),
        })
        .await
        .unwrap();

    // seq 2: accepted then rejected-busy (permanently terminal, never resurfaces)
    let second = service
        .accept_inbound_message(AcceptInboundMessageRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
            actor_id: "actor-a".into(),
            source_binding_id: None,
            reply_target_binding_id: None,
            external_event_id: None,
            content: MessageContent::text("rejected busy interior"),
        })
        .await
        .unwrap();
    service
        .mark_message_rejected_busy(&scope, &thread.thread_id, second.message_id)
        .await
        .unwrap();

    // seq 3: visible user message
    service
        .accept_inbound_message(AcceptInboundMessageRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
            actor_id: "actor-a".into(),
            source_binding_id: None,
            reply_target_binding_id: None,
            external_event_id: None,
            content: MessageContent::text("third"),
        })
        .await
        .unwrap();

    // Summary spans [1..3] covering the interior RejectedBusy.  Must be applied.
    service
        .create_summary_artifact(CreateSummaryArtifactRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
            start_sequence: 1,
            end_sequence: 3,
            summary_kind: SummaryKind::Compaction,
            content: MessageContent::text("first and third summarized"),
            model_context_policy: Some(SummaryModelContextPolicy::ReplaceRangeWhenSelected),
        })
        .await
        .unwrap();

    let context = service
        .load_context_window(LoadContextWindowRequest {
            scope,
            thread_id: thread.thread_id,
            max_messages: 16,
        })
        .await
        .unwrap();

    assert_eq!(context.messages.len(), 1, "summary must be applied");
    assert_eq!(context.messages[0].kind, MessageKind::Summary);
    assert_eq!(context.messages[0].content, "first and third summarized");
}

/// Mirrors `summary_spanning_interior_draft_is_not_applied` from the
/// in-memory contract suite.  A compaction summary spanning a Draft
/// (resurfaceable) message must still be suppressed by the filesystem backend.
#[tokio::test]
async fn filesystem_summary_spanning_interior_draft_is_not_applied() {
    let backend = Arc::new(InMemoryBackend::new());
    let scoped = scoped_threads_fs_at(backend, "tenant-draft-sum", "alice");
    let service = FilesystemSessionThreadService::new(scoped);
    let scope = scope("draft-sum");

    let thread = service
        .ensure_thread(EnsureThreadRequest {
            scope: scope.clone(),
            thread_id: Some(ThreadId::new("thread-draft-sum").unwrap()),
            created_by_actor_id: "actor-a".into(),
            title: None,
            metadata_json: None,
        })
        .await
        .unwrap();

    // seq 1: visible user message
    service
        .accept_inbound_message(AcceptInboundMessageRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
            actor_id: "actor-a".into(),
            source_binding_id: None,
            reply_target_binding_id: None,
            external_event_id: None,
            content: MessageContent::text("first"),
        })
        .await
        .unwrap();

    // seq 2: assistant Draft — resurfaceable, must block the summary.
    service
        .append_assistant_draft(AppendAssistantDraftRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
            turn_run_id: "run-draft-sum".into(),
            content: MessageContent::text("draft interior"),
        })
        .await
        .unwrap();

    // seq 3: visible user message
    service
        .accept_inbound_message(AcceptInboundMessageRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
            actor_id: "actor-a".into(),
            source_binding_id: None,
            reply_target_binding_id: None,
            external_event_id: None,
            content: MessageContent::text("third"),
        })
        .await
        .unwrap();

    // Summary spans [1..3] covering the Draft at seq 2.  Must be suppressed.
    service
        .create_summary_artifact(CreateSummaryArtifactRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
            start_sequence: 1,
            end_sequence: 3,
            summary_kind: SummaryKind::Compaction,
            content: MessageContent::text("should not appear"),
            model_context_policy: Some(SummaryModelContextPolicy::ReplaceRangeWhenSelected),
        })
        .await
        .unwrap();

    let context = service
        .load_context_window(LoadContextWindowRequest {
            scope,
            thread_id: thread.thread_id,
            max_messages: 16,
        })
        .await
        .unwrap();

    assert_eq!(
        context.messages.len(),
        2,
        "summary must be suppressed for draft-spanning range"
    );
    assert_eq!(context.messages[0].content, "first");
    assert_eq!(context.messages[1].content, "third");
}
