// arch-exempt: large_file, filesystem thread contract suite decomposition, plan #5662
//! Contract tests for [`FilesystemSessionThreadService`].
//!
//! Drives the production filesystem-backed store over an
//! [`InMemoryBackend`] composed under a `/threads` mount alias whose
//! `VirtualPath` target encodes a tenant/user prefix. Mirrors the shape of
//! the approval and process-journal filesystem contract suites — see
//! `crates/kernel/ironclaw_approvals/tests/approval_resolution_contract.rs` and
//! `crates/kernel/ironclaw_processes/tests/process_journal_store_contract.rs`.

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
    BackendCapabilities, CasExpectation, DirEntry, DiskFilesystem, Entry, Fault, FaultInjecting,
    FaultKind, FileStat, FilesystemError, FilesystemOperation, Filter, InMemoryBackend, IndexSpec,
    OrderedPage, Page, RecordVersion, RootFilesystem, ScopedFilesystem, SeqNo, StorageTxn,
    TxnCapability, VersionedEntry,
};
use ironclaw_host_api::{
    ids::{
        AgentId, CapabilityId, InvocationId, ProjectId, ProviderToolName, TenantId, ThreadId,
        UserId,
    },
    mount::{MountGrant, MountPermissions, MountView},
    path::{HostPath, MountAlias, ScopedPath, VirtualPath},
};
use ironclaw_threads::{
    AcceptInboundMessageRequest, AppendAssistantDraftRequest,
    AppendCapabilityDisplayPreviewRequest, AppendFinalizedAssistantMessageRequest,
    AppendToolResultReferenceRequest, AttachmentKind, AttachmentRef,
    CapabilityDisplayPreviewEnvelope, CapabilityDisplayPreviewEnvelopeInput,
    CapabilityDisplayPreviewStatus, CreateSummaryArtifactRequest, EnsureThreadRequest,
    FilesystemSessionThreadService, FinalizedAssistantMessageByRunRequest,
    InboundMessageReplayMetadata, ListThreadsForScopeRequest, LoadContextMessagesRequest,
    LoadContextWindowRequest, MessageContent, MessageKind, MessageStatus,
    PREPARED_CONTEXT_METADATA_MARKER_KEY, ProviderToolCallReferenceEnvelope,
    PutToolResultRecordRequest, ReadToolResultRecordRequest, RedactMessageRequest,
    ReplayAcceptedInboundMessageRequest, SessionThreadError, SessionThreadService, SummaryKind,
    SummaryModelContextPolicy, ThreadHistoryRequest, ThreadMessageId, ThreadScope,
    ToolResultIntrinsicOutcome, ToolResultReferenceEnvelope, ToolResultSafeSummary,
    UpdateAssistantDraftRequest, UpdateToolResultReferenceRequest,
};
use tokio::{
    sync::{Barrier, Mutex, OwnedMutexGuard},
    time::{Duration, timeout},
};

fn provider_call_reference(call_id: &str) -> ProviderToolCallReferenceEnvelope {
    ProviderToolCallReferenceEnvelope {
        provider_id: "test-provider".to_string(),
        provider_model_id: "test-model".to_string(),
        provider_turn_id: "turn_1".to_string(),
        provider_call_id: call_id.to_string(),
        provider_tool_name: ProviderToolName::new("builtin__result_read")
            .expect("provider tool name"),
        capability_id: CapabilityId::new("builtin.result_read").unwrap(),
        arguments: serde_json::json!({"offset": 0}),
        response_reasoning: None,
        reasoning: None,
        signature: None,
    }
}

#[tokio::test]
async fn filesystem_history_preserves_typed_intrinsic_outcome_without_provider_metadata() {
    let backend = Arc::new(InMemoryBackend::new());
    let scoped = scoped_threads_fs_at(Arc::clone(&backend), "tenant-provider-evidence", "alice");
    let service = FilesystemSessionThreadService::new(scoped);
    let scope = scope("fs-provider-evidence");
    let thread = service
        .ensure_thread(EnsureThreadRequest {
            scope: scope.clone(),
            thread_id: Some(ThreadId::new("thread-fs-provider-evidence").unwrap()),
            created_by_actor_id: "actor-a".into(),
            title: None,
            metadata_json: None,
        })
        .await
        .unwrap();
    let first = service
        .append_tool_result_reference(AppendToolResultReferenceRequest {
            intrinsic_outcome: Some(ToolResultIntrinsicOutcome::NothingToReport),
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
            turn_run_id: "run-1".into(),
            result_ref: "result:provider-evidence".into(),
            safe_summary: ToolResultSafeSummary::new("structured result recorded").unwrap(),
            provider_call: None,
            model_observation: None,
        })
        .await
        .unwrap();
    let thread_after_first = service
        .read_thread(ThreadHistoryRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
        })
        .await
        .unwrap();
    drop(service);
    let scoped = scoped_threads_fs_at(backend, "tenant-provider-evidence", "alice");
    let service = FilesystemSessionThreadService::new(scoped);
    let duplicate = service
        .append_tool_result_reference(AppendToolResultReferenceRequest {
            intrinsic_outcome: Some(ToolResultIntrinsicOutcome::NothingToReport),
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
            turn_run_id: "run-1".into(),
            result_ref: "result:provider-evidence".into(),
            safe_summary: ToolResultSafeSummary::new("retry ignored").unwrap(),
            provider_call: None,
            model_observation: None,
        })
        .await
        .unwrap();
    let thread_after_duplicate = service
        .read_thread(ThreadHistoryRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
        })
        .await
        .unwrap();
    assert_eq!(first, duplicate, "atomic replay must be side-effect free");
    assert_eq!(
        thread_after_first.updated_at, thread_after_duplicate.updated_at,
        "atomic replay must not move thread activity ordering"
    );

    let history = service
        .list_thread_history(ThreadHistoryRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
        })
        .await
        .unwrap();
    assert!(
        history
            .messages
            .iter()
            .all(|message| message.tool_result_provider_call.is_none()),
        "ordinary history must keep host-only provider metadata redacted"
    );

    let envelope = history
        .messages
        .first()
        .and_then(|message| message.content.as_deref())
        .map(ToolResultReferenceEnvelope::from_json_str)
        .transpose()
        .unwrap()
        .expect("history contains the result envelope");
    assert_eq!(
        envelope.intrinsic_outcome,
        Some(ToolResultIntrinsicOutcome::NothingToReport),
        "host-validated intrinsic evidence must survive the redacted history projection"
    );
}

#[tokio::test]
async fn filesystem_tool_result_update_targets_the_exact_provider_call_row() {
    let backend = Arc::new(InMemoryBackend::new());
    let scoped = scoped_threads_fs_at(backend, "tenant-exact-result-update", "alice");
    let service = FilesystemSessionThreadService::new(scoped);
    let scope = scope("fs-exact-result-update");
    let thread = service
        .ensure_thread(EnsureThreadRequest {
            scope: scope.clone(),
            thread_id: Some(ThreadId::new("thread-fs-exact-result-update").unwrap()),
            created_by_actor_id: "actor-a".into(),
            title: None,
            metadata_json: None,
        })
        .await
        .unwrap();
    let spawn = service
        .append_tool_result_reference(AppendToolResultReferenceRequest {
            intrinsic_outcome: None,
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
            turn_run_id: "run-1".into(),
            result_ref: "result:shared-update".into(),
            safe_summary: ToolResultSafeSummary::new("subagent still running").unwrap(),
            provider_call: Some(provider_call_reference("spawn-call")),
            model_observation: None,
        })
        .await
        .unwrap();
    let page = service
        .append_tool_result_reference(AppendToolResultReferenceRequest {
            intrinsic_outcome: None,
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
            turn_run_id: "run-1".into(),
            result_ref: "result:shared-update".into(),
            safe_summary: ToolResultSafeSummary::new("result page returned").unwrap(),
            provider_call: Some(provider_call_reference("result-read-call")),
            model_observation: None,
        })
        .await
        .unwrap();

    let updated = service
        .update_tool_result_reference(UpdateToolResultReferenceRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
            turn_run_id: "run-1".into(),
            result_ref: "result:shared-update".into(),
            provider_call_id: Some("spawn-call".to_string()),
            safe_summary: ToolResultSafeSummary::new("subagent completed").unwrap(),
        })
        .await
        .unwrap();

    assert_eq!(updated.message_id, spawn.message_id);
    assert_ne!(updated.message_id, page.message_id);
    let updated_envelope =
        ToolResultReferenceEnvelope::from_json_str(updated.content.as_deref().unwrap()).unwrap();
    assert_eq!(updated_envelope.safe_summary.as_str(), "subagent completed");
    let context = service
        .load_context_messages(LoadContextMessagesRequest {
            scope,
            thread_id: thread.thread_id,
            message_ids: vec![page.message_id],
        })
        .await
        .unwrap();
    let page_envelope =
        ToolResultReferenceEnvelope::from_json_str(context.messages[0].content.as_str()).unwrap();
    assert_eq!(page_envelope.safe_summary.as_str(), "result page returned");
}

#[tokio::test]
async fn filesystem_tool_result_dedup_keys_distinct_provider_calls_sharing_a_result_ref() {
    let backend = Arc::new(InMemoryBackend::new());
    let scoped = scoped_threads_fs_at(backend, "tenant-shared-continuation", "alice");
    let service = FilesystemSessionThreadService::new(scoped);
    let scope = scope("fs-shared-continuation");
    let thread = service
        .ensure_thread(EnsureThreadRequest {
            scope: scope.clone(),
            thread_id: Some(ThreadId::new("thread-fs-shared-continuation").unwrap()),
            created_by_actor_id: "actor-a".into(),
            title: None,
            metadata_json: None,
        })
        .await
        .unwrap();
    let first_call = provider_call_reference("call_1");

    let first = service
        .append_tool_result_reference(AppendToolResultReferenceRequest {
            intrinsic_outcome: None,
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
            turn_run_id: "run-1".into(),
            result_ref: "result:shared-continuation".into(),
            safe_summary: ToolResultSafeSummary::new("first page").unwrap(),
            provider_call: Some(first_call.clone()),
            model_observation: None,
        })
        .await
        .unwrap();
    let duplicate = service
        .append_tool_result_reference(AppendToolResultReferenceRequest {
            intrinsic_outcome: None,
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
            turn_run_id: "run-1".into(),
            result_ref: "result:shared-continuation".into(),
            safe_summary: ToolResultSafeSummary::new("first page replay").unwrap(),
            provider_call: Some(first_call),
            model_observation: None,
        })
        .await
        .unwrap();
    let second = service
        .append_tool_result_reference(AppendToolResultReferenceRequest {
            intrinsic_outcome: None,
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
            turn_run_id: "run-1".into(),
            result_ref: "result:shared-continuation".into(),
            safe_summary: ToolResultSafeSummary::new("second page").unwrap(),
            provider_call: Some(provider_call_reference("call_2")),
            model_observation: None,
        })
        .await
        .unwrap();

    assert_eq!(duplicate.message_id, first.message_id);
    assert_ne!(second.message_id, first.message_id);
    assert_eq!(
        first
            .tool_result_provider_call
            .as_ref()
            .expect("first provider call persists")
            .provider_call_id,
        "call_1"
    );
    assert_eq!(
        second
            .tool_result_provider_call
            .as_ref()
            .expect("second provider call persists")
            .provider_call_id,
        "call_2"
    );
    let history = service
        .list_thread_history(ThreadHistoryRequest {
            scope,
            thread_id: thread.thread_id,
        })
        .await
        .unwrap();
    assert_eq!(
        history
            .messages
            .iter()
            .filter(|message| message.kind == MessageKind::ToolResultReference)
            .count(),
        2
    );
}

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
async fn filesystem_tool_result_records_survive_restart_and_enforce_scope() {
    let backend = Arc::new(InMemoryBackend::new());
    let scoped = scoped_threads_fs_at(Arc::clone(&backend), "tenant-tool-results", "alice");
    let service = FilesystemSessionThreadService::new(scoped);
    let owner_scope = scope("fs-tool-results");
    let thread = service
        .ensure_thread(EnsureThreadRequest {
            scope: owner_scope.clone(),
            thread_id: Some(ThreadId::new("thread-fs-tool-results").unwrap()),
            created_by_actor_id: "actor-a".into(),
            title: None,
            metadata_json: None,
        })
        .await
        .unwrap();
    let content = br#"{\"message\":\"persisted tool output\"}"#.to_vec();
    service
        .put_tool_result_record(PutToolResultRecordRequest {
            scope: owner_scope.clone(),
            thread_id: thread.thread_id.clone(),
            result_ref: "result:fs-tool-result".to_string(),
            content: content.clone(),
        })
        .await
        .expect("durable result write succeeds");

    let reopened = FilesystemSessionThreadService::new(scoped_threads_fs_at(
        backend,
        "tenant-tool-results",
        "alice",
    ));
    let chunk = reopened
        .read_tool_result_record(ReadToolResultRecordRequest {
            scope: owner_scope.clone(),
            thread_id: thread.thread_id.clone(),
            result_ref: "result:fs-tool-result".to_string(),
            offset: 0,
            max_bytes: 10,
        })
        .await
        .expect("reopened read succeeds")
        .expect("durable record exists");
    assert_eq!(chunk.content, content[..10]);
    assert_eq!(chunk.next_offset, Some(10));

    let unicode_ref = "result:fs-unicode-tool-result".to_string();
    reopened
        .put_tool_result_record(PutToolResultRecordRequest {
            scope: owner_scope.clone(),
            thread_id: thread.thread_id.clone(),
            result_ref: unicode_ref.clone(),
            content: "abcédef".as_bytes().to_vec(),
        })
        .await
        .expect("unicode durable result write succeeds");
    let unicode_chunk = reopened
        .read_tool_result_record(ReadToolResultRecordRequest {
            scope: owner_scope.clone(),
            thread_id: thread.thread_id.clone(),
            result_ref: unicode_ref,
            offset: 0,
            max_bytes: 4,
        })
        .await
        .expect("unicode durable read succeeds")
        .expect("unicode durable record exists");
    assert_eq!(std::str::from_utf8(&unicode_chunk.content).unwrap(), "abc");
    assert_eq!(unicode_chunk.next_offset, Some(3));

    let wrong_scope_error = reopened
        .read_tool_result_record(ReadToolResultRecordRequest {
            scope: scope("fs-tool-results-wrong"),
            thread_id: thread.thread_id,
            result_ref: "result:fs-tool-result".to_string(),
            offset: 0,
            max_bytes: 8,
        })
        .await
        .expect_err("wrong scope must not expose durable result records");
    assert_unknown_thread(
        wrong_scope_error,
        &ThreadId::new("thread-fs-tool-results").unwrap(),
    );
}

#[tokio::test]
async fn filesystem_concurrent_duplicate_tool_result_writes_converge() {
    let backend = Arc::new(ConcurrentToolResultWriteBackend::new());
    let scoped = scoped_threads_fs_at(backend, "tenant-tool-result-race", "alice");
    let service = Arc::new(FilesystemSessionThreadService::new(scoped));
    let scope = scope("fs-tool-result-race");
    let thread = service
        .ensure_thread(EnsureThreadRequest {
            scope: scope.clone(),
            thread_id: Some(ThreadId::new("thread-fs-tool-result-race").unwrap()),
            created_by_actor_id: "actor-a".into(),
            title: None,
            metadata_json: None,
        })
        .await
        .unwrap();
    let result_ref = "result:fs-concurrent-tool-result".to_string();
    let content = b"concurrent durable tool output".to_vec();

    let left = {
        let service = Arc::clone(&service);
        let scope = scope.clone();
        let thread_id = thread.thread_id.clone();
        let result_ref = result_ref.clone();
        let content = content.clone();
        async move {
            service
                .put_tool_result_record(PutToolResultRecordRequest {
                    scope,
                    thread_id,
                    result_ref,
                    content,
                })
                .await
        }
    };
    let right = {
        let service = Arc::clone(&service);
        let scope = scope.clone();
        let thread_id = thread.thread_id.clone();
        let result_ref = result_ref.clone();
        let content = content.clone();
        async move {
            service
                .put_tool_result_record(PutToolResultRecordRequest {
                    scope,
                    thread_id,
                    result_ref,
                    content,
                })
                .await
        }
    };

    let (left, right) = tokio::join!(left, right);
    left.expect("first concurrent duplicate write converges");
    right.expect("second concurrent duplicate write converges after CAS retry");

    let stored = service
        .read_tool_result_record(ReadToolResultRecordRequest {
            scope,
            thread_id: thread.thread_id,
            result_ref,
            offset: 0,
            max_bytes: 128,
        })
        .await
        .expect("converged record is readable")
        .expect("converged record exists");
    assert_eq!(stored.content, content);
    assert!(stored.next_offset.is_none());
}

#[tokio::test]
async fn filesystem_redaction_retains_durable_tool_result_record() {
    let backend = Arc::new(InMemoryBackend::new());
    let scoped = scoped_threads_fs_at(backend, "tenant-tool-redaction", "alice");
    let service = FilesystemSessionThreadService::new(scoped);
    let scope = scope("fs-tool-redaction");
    let thread = service
        .ensure_thread(EnsureThreadRequest {
            scope: scope.clone(),
            thread_id: Some(ThreadId::new("thread-fs-tool-redaction").unwrap()),
            created_by_actor_id: "actor-a".into(),
            title: None,
            metadata_json: None,
        })
        .await
        .unwrap();
    let result_ref = "result:fs-redacted-tool".to_string();
    let result = service
        .append_tool_result_reference(AppendToolResultReferenceRequest {
            intrinsic_outcome: None,
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
            turn_run_id: "run-fs-tool-redaction".into(),
            result_ref: result_ref.clone(),
            safe_summary: ToolResultSafeSummary::new("safe tool result").unwrap(),
            provider_call: None,
            model_observation: None,
        })
        .await
        .unwrap();
    service
        .put_tool_result_record(PutToolResultRecordRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
            result_ref: result_ref.clone(),
            content: br#"{\"secret\":\"raw tool output\"}"#.to_vec(),
        })
        .await
        .unwrap();

    service
        .redact_message(RedactMessageRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
            message_id: result.message_id,
            redaction_ref: "redaction/audit/fs-tool".into(),
        })
        .await
        .expect("redaction succeeds");

    assert!(
        service
            .read_tool_result_record(ReadToolResultRecordRequest {
                scope,
                thread_id: thread.thread_id,
                result_ref,
                offset: 0,
                max_bytes: 128,
            })
            .await
            .expect("redaction keeps the thread readable")
            .is_some(),
        "redaction retains the durable raw result for audit and recovery"
    );
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
    service
        .migrate_thread_index_for_scope(&request_scope)
        .await
        .expect("explicit repair removes stale projection rows");

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
        "explicit repair must remove an index row whose source thread root is gone"
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
        "stale index row should be removed during explicit repair"
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
    let scoped = scoped_threads_fs_at(Arc::clone(&backend), "tenant-finalized-by-run", "alice");
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
            scope: scope.clone(),
            thread_id: thread.thread_id,
            turn_run_id: "run-finalized-lookup".into(),
        })
        .await
        .unwrap()
        .expect("finalized assistant message is indexed by run");
    assert_eq!(finalized.message_id, draft.message_id);
    assert_eq!(finalized.status, MessageStatus::Finalized);
    assert_eq!(finalized.content.as_deref(), Some("final"));

    let stored_rows = backend
        .query(
            &VirtualPath::new("/tenants/tenant-finalized-by-run/users/alice/threads").unwrap(),
            &Filter::All,
            Page::new(0, Page::MAX_LIMIT),
        )
        .await
        .unwrap();
    assert!(
        stored_rows
            .iter()
            .all(|row| !row.path.as_str().contains("/indexes/")),
        "message lookup must not amplify one message row into sibling index entries"
    );
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
            content: MessageContent::with_attachments(
                "final answer",
                vec![sample_finalized_attachment_ref()],
            ),
        })
        .await
        .unwrap();
    // Idempotency baseline: a retry of the SAME finalized content — text AND
    // attachment refs — reuses the existing record. (A DIFFERENT finalized
    // reply in the same run appends a sibling instead — a steered run replies
    // more than once; asserted below.)
    let duplicate = service
        .append_finalized_assistant_message(AppendFinalizedAssistantMessageRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
            turn_run_id: "run-finalized-append".into(),
            content: MessageContent::with_attachments(
                "final answer",
                vec![sample_finalized_attachment_ref()],
            ),
        })
        .await
        .unwrap();

    assert_eq!(first.message_id, duplicate.message_id);
    assert_eq!(duplicate.kind, MessageKind::Assistant);
    assert_eq!(duplicate.status, MessageStatus::Finalized);
    assert_eq!(duplicate.content.as_deref(), Some("final answer"));
    assert_eq!(
        duplicate.attachments,
        vec![sample_finalized_attachment_ref()]
    );

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
    assert_eq!(
        finalized.attachments,
        vec![sample_finalized_attachment_ref()]
    );

    let history = service
        .list_thread_history(ThreadHistoryRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
        })
        .await
        .unwrap();
    assert_eq!(history.messages.len(), 1);
    assert_eq!(history.messages[0].message_id, first.message_id);
    assert_eq!(history.messages[0].status, MessageStatus::Finalized);
    assert_eq!(
        history.messages[0].attachments,
        vec![sample_finalized_attachment_ref()]
    );

    // A DIFFERENT finalized reply in the same run (steered run replying again)
    // appends a sibling and moves the run index to it.
    let sibling = service
        .append_finalized_assistant_message(AppendFinalizedAssistantMessageRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
            turn_run_id: "run-finalized-append".into(),
            content: MessageContent::text("steered second answer"),
        })
        .await
        .unwrap();
    assert_ne!(sibling.message_id, first.message_id);
    assert_eq!(sibling.status, MessageStatus::Finalized);
    let latest = service
        .finalized_assistant_message_by_run(FinalizedAssistantMessageByRunRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
            turn_run_id: "run-finalized-append".into(),
        })
        .await
        .unwrap()
        .expect("latest finalized assistant message remains indexed by run");
    assert_eq!(latest.message_id, sibling.message_id);
    let history = service
        .list_thread_history(ThreadHistoryRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
        })
        .await
        .unwrap();
    assert_eq!(history.messages.len(), 2);
    assert_eq!(history.messages[1].message_id, sibling.message_id);

    // Attachment identity: the SAME text with a DIFFERENT attachment set is
    // neither the same reply (reuse would silently drop the new attachment
    // refs) nor a steered second reply (a sibling would duplicate the visible
    // bubble). It is a mismatched replay and must fail loud — the loop
    // transcript port pins the caller-visible rejection
    // (`finalized_assistant_attachment_retry_rejects_mismatched_refs`).
    let mismatch = service
        .append_finalized_assistant_message(AppendFinalizedAssistantMessageRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
            turn_run_id: "run-finalized-append".into(),
            content: MessageContent::with_attachments(
                "steered second answer",
                vec![second_finalized_attachment_ref()],
            ),
        })
        .await;
    assert!(
        matches!(
            mismatch,
            Err(SessionThreadError::InvalidMessageTransition { message_id, .. })
                if message_id == sibling.message_id
        ),
        "attachment-mismatched finalized replay must fail loud, got {mismatch:?}"
    );
    let history = service
        .list_thread_history(ThreadHistoryRequest {
            scope,
            thread_id: thread.thread_id,
        })
        .await
        .unwrap();
    assert_eq!(
        history.messages.len(),
        2,
        "the mismatched replay must not append a duplicate visible reply"
    );
    assert!(
        history.messages[1].attachments.is_empty(),
        "the mismatched replay must not overwrite the finalized row's attachments"
    );
}

#[tokio::test]
async fn filesystem_read_thread_message_maps_misses_to_none_and_propagates_backend_errors() {
    let backend = Arc::new(FaultInjecting::new(InMemoryBackend::new()));
    let scoped = scoped_threads_fs_at(Arc::clone(&backend), "tenant-read-message", "alice");
    let service = FilesystemSessionThreadService::new(scoped);
    let read_scope = scope("read-message");
    let thread = service
        .ensure_thread(EnsureThreadRequest {
            scope: read_scope.clone(),
            thread_id: Some(ThreadId::new("thread-read-message").unwrap()),
            created_by_actor_id: "actor-a".into(),
            title: None,
            metadata_json: None,
        })
        .await
        .unwrap();
    let accepted = service
        .accept_inbound_message(AcceptInboundMessageRequest {
            scope: read_scope.clone(),
            thread_id: thread.thread_id.clone(),
            actor_id: "actor-a".into(),
            source_binding_id: None,
            reply_target_binding_id: None,
            external_event_id: None,
            content: MessageContent::text("hello"),
        })
        .await
        .unwrap();

    let present = service
        .read_thread_message(&read_scope, &thread.thread_id, accepted.message_id)
        .await
        .unwrap()
        .expect("accepted message is point-readable");
    assert_eq!(present.message_id, accepted.message_id);
    assert_eq!(present.content.as_deref(), Some("hello"));

    // Genuine lookup misses — unknown message, unknown thread, and a thread
    // hidden by scope isolation — all read as absence.
    assert!(
        service
            .read_thread_message(&read_scope, &thread.thread_id, ThreadMessageId::new())
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        service
            .read_thread_message(
                &read_scope,
                &ThreadId::new("thread-read-message-missing").unwrap(),
                accepted.message_id,
            )
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        service
            .read_thread_message(
                &scope("read-message-other"),
                &thread.thread_id,
                accepted.message_id,
            )
            .await
            .unwrap()
            .is_none()
    );

    // A backend failure on the message record read is NOT a miss: it must
    // propagate as an error instead of masquerading as message absence.
    backend.add_fault(
        Fault::on(FilesystemOperation::ReadFile)
            .path(format!("/messages/{}.json", accepted.message_id))
            .backend("message record reads disabled by contract test"),
    );
    let error = service
        .read_thread_message(&read_scope, &thread.thread_id, accepted.message_id)
        .await
        .expect_err("a backend read failure must propagate, not read as absence");
    assert!(
        matches!(error, SessionThreadError::Backend(_)),
        "expected a backend error, got {error:?}"
    );
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
            content: MessageContent::with_attachments(
                "final answer",
                vec![sample_finalized_attachment_ref()],
            ),
        })
        .await
        .unwrap();

    assert_eq!(finalized.message_id, draft.message_id);
    assert_eq!(finalized.status, MessageStatus::Finalized);
    assert_eq!(finalized.content.as_deref(), Some("final answer"));
    assert_eq!(
        finalized.attachments,
        vec![sample_finalized_attachment_ref()]
    );

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
    assert_eq!(by_run.attachments, vec![sample_finalized_attachment_ref()]);

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
    assert_eq!(
        history.messages[0].attachments,
        vec![sample_finalized_attachment_ref()]
    );
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
async fn filesystem_message_row_write_failure_rejects_source_message() {
    let backend = Arc::new(message_row_write_failure_backend());
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
    service
        .append_assistant_draft(AppendAssistantDraftRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
            turn_run_id: "run-lookup-index-failure".into(),
            content: MessageContent::text("draft"),
        })
        .await
        .expect_err("message-row projection failure must reject the append");

    let history = service
        .list_thread_history(ThreadHistoryRequest {
            scope,
            thread_id: thread.thread_id,
        })
        .await
        .expect("read transcript after rolled-back append");
    assert!(history.messages.is_empty());
}

#[tokio::test]
async fn filesystem_lookup_index_read_failure_fails_without_transcript_scan() {
    let backend = Arc::new(lookup_index_read_failure_backend());
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
    let error = service
        .append_assistant_draft(AppendAssistantDraftRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
            turn_run_id: "run-lookup-index-read-failure".into(),
            content: MessageContent::text("draft"),
        })
        .await
        .expect_err("lookup projection read failures must not scan the transcript");
    assert!(error.to_string().contains("lookup index reads disabled"));
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
async fn filesystem_context_limit_counts_only_model_visible_messages() {
    let backend = Arc::new(InMemoryBackend::new());
    let scoped = scoped_threads_fs_at(backend, "tenant-visible-window", "alice");
    let service = FilesystemSessionThreadService::new(scoped);
    let scope = scope("fs-visible-window");
    let thread = service
        .ensure_thread(EnsureThreadRequest {
            scope: scope.clone(),
            thread_id: Some(ThreadId::new("thread-fs-visible-window").unwrap()),
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
            content: MessageContent::text("original task"),
        })
        .await
        .unwrap();

    // Capability previews are durable transcript rows but are never model
    // context. The limit therefore applies after removing them, just as it
    // does in the in-memory implementation.
    for index in 0..70 {
        service
            .append_capability_display_preview(AppendCapabilityDisplayPreviewRequest {
                scope: scope.clone(),
                thread_id: thread.thread_id.clone(),
                turn_run_id: "run-visible-window".into(),
                preview: preview_envelope(InvocationId::new()),
            })
            .await
            .unwrap();
        service
            .append_tool_result_reference(AppendToolResultReferenceRequest {
                intrinsic_outcome: None,
                scope: scope.clone(),
                thread_id: thread.thread_id.clone(),
                turn_run_id: "run-visible-window".into(),
                result_ref: format!("result:visible-window-{index}"),
                safe_summary: ToolResultSafeSummary::new(format!("result {index}")).unwrap(),
                provider_call: Some(provider_call_reference(&format!("call-{index}"))),
                model_observation: None,
            })
            .await
            .unwrap();
    }

    let context = service
        .load_context_window(LoadContextWindowRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
            max_messages: 128,
        })
        .await
        .unwrap();

    assert_eq!(context.messages.len(), 71);
    assert_eq!(context.messages[0].kind, MessageKind::User);
    assert_eq!(context.messages[0].content, "original task");
    assert!(
        context
            .messages
            .iter()
            .all(|message| message.kind != MessageKind::CapabilityDisplayPreview)
    );
    assert!(context.recent_window_truncation.is_none());

    let truncated = service
        .load_context_window(LoadContextWindowRequest {
            scope,
            thread_id: thread.thread_id,
            max_messages: 2,
        })
        .await
        .unwrap();
    assert_eq!(
        truncated
            .messages
            .iter()
            .map(|message| message.sequence)
            .collect::<Vec<_>>(),
        vec![139, 141]
    );
    let boundary = truncated
        .recent_window_truncation
        .expect("omitted visible messages must produce an exact watermark");
    assert_eq!(boundary.omitted_through_sequence, 137);
    assert_eq!(
        boundary.omitted_through_kind,
        MessageKind::ToolResultReference
    );
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

#[tokio::test]
async fn filesystem_fallback_accept_concurrent_duplicate_replays_existing_message() {
    let backend = Arc::new(FallbackRaceBackend::new());
    let scoped = scoped_threads_fs_at(backend, "tenant-fallback-accept-race", "alice");
    let service = Arc::new(FilesystemSessionThreadService::new(scoped));
    let request_scope = scope("fallback-accept-race");
    let thread = service
        .ensure_thread(EnsureThreadRequest {
            scope: request_scope.clone(),
            thread_id: Some(ThreadId::new("thread-fallback-accept-race").unwrap()),
            created_by_actor_id: "actor-a".into(),
            title: None,
            metadata_json: None,
        })
        .await
        .unwrap();
    let request = AcceptInboundMessageRequest {
        scope: request_scope.clone(),
        thread_id: thread.thread_id.clone(),
        actor_id: "actor-a".into(),
        source_binding_id: Some("binding-fallback-accept-race".into()),
        reply_target_binding_id: None,
        external_event_id: Some("event-fallback-accept-race".into()),
        content: MessageContent::text("same payload"),
    };

    let left = service.accept_inbound_message_with_replay_metadata(
        request.clone(),
        InboundMessageReplayMetadata {
            resolved_model: Some("model-a".into()),
        },
    );
    let right = service.accept_inbound_message_with_replay_metadata(
        request,
        InboundMessageReplayMetadata {
            resolved_model: Some("model-a".into()),
        },
    );
    let (left, right) = tokio::join!(left, right);
    let left = left.expect("first concurrent fallback accept converges");
    let right = right.expect("second concurrent fallback accept converges");

    assert_eq!(left.message_id, right.message_id);
    assert_eq!(left.sequence, right.sequence);
    assert!(left.idempotent_replay || right.idempotent_replay);
    assert_eq!(
        left.replay_metadata.resolved_model.as_deref(),
        Some("model-a")
    );
    assert_eq!(left.replay_metadata, right.replay_metadata);
    let history = service
        .list_thread_history(ThreadHistoryRequest {
            scope: request_scope,
            thread_id: thread.thread_id,
        })
        .await
        .unwrap();
    assert_eq!(history.messages.len(), 1);
    assert_eq!(history.messages[0].message_id, left.message_id);
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
async fn filesystem_accepted_inbound_replay_preserves_resolved_model_metadata() {
    let backend = Arc::new(InMemoryBackend::new());
    let scoped = scoped_threads_fs_at(backend, "tenant-model-replay", "alice");
    let service = FilesystemSessionThreadService::new(scoped);
    let request_scope = scope("model-replay");
    let thread = service
        .ensure_thread(EnsureThreadRequest {
            scope: request_scope.clone(),
            thread_id: Some(ThreadId::new("thread-model-replay").unwrap()),
            created_by_actor_id: "actor-a".into(),
            title: None,
            metadata_json: None,
        })
        .await
        .unwrap();
    let request = AcceptInboundMessageRequest {
        scope: request_scope.clone(),
        thread_id: thread.thread_id,
        actor_id: "actor-a".into(),
        source_binding_id: Some("source-model-replay".into()),
        reply_target_binding_id: Some("reply-model-replay".into()),
        external_event_id: Some("event-model-replay".into()),
        content: MessageContent::text("hello"),
    };

    service
        .accept_inbound_message_with_replay_metadata(
            request.clone(),
            InboundMessageReplayMetadata {
                resolved_model: Some("model-a".into()),
            },
        )
        .await
        .unwrap();
    let duplicate = service
        .accept_inbound_message_with_replay_metadata(
            request,
            InboundMessageReplayMetadata {
                resolved_model: Some("model-b".into()),
            },
        )
        .await
        .unwrap();
    assert!(duplicate.idempotent_replay);
    assert_eq!(
        duplicate.replay_metadata.resolved_model.as_deref(),
        Some("model-a")
    );

    let replay = service
        .replay_accepted_inbound_message(ReplayAcceptedInboundMessageRequest {
            scope: request_scope,
            actor_id: "actor-a".into(),
            source_binding_id: "source-model-replay".into(),
            external_event_id: "event-model-replay".into(),
        })
        .await
        .unwrap()
        .expect("accepted replay");
    assert_eq!(
        replay.replay_metadata.resolved_model.as_deref(),
        Some("model-a")
    );
}

#[tokio::test]
async fn filesystem_fallback_idempotency_failure_precedes_message_persistence() {
    let backend = Arc::new(FaultInjecting::new(InMemoryBackend::new()));
    let scoped = scoped_threads_fs_at(
        Arc::clone(&backend),
        "tenant-fallback-idempotency-failure",
        "alice",
    );
    let service = FilesystemSessionThreadService::new(scoped);
    let request_scope = scope("fallback-idempotency-failure");
    let thread = service
        .ensure_thread(EnsureThreadRequest {
            scope: request_scope.clone(),
            thread_id: Some(ThreadId::new("thread-fallback-idempotency-failure").unwrap()),
            created_by_actor_id: "actor-a".into(),
            title: None,
            metadata_json: None,
        })
        .await
        .unwrap();
    let request = AcceptInboundMessageRequest {
        scope: request_scope.clone(),
        thread_id: thread.thread_id.clone(),
        actor_id: "actor-a".into(),
        source_binding_id: Some("source-fallback-idempotency-failure".into()),
        reply_target_binding_id: Some("reply-fallback-idempotency-failure".into()),
        external_event_id: Some("event-fallback-idempotency-failure".into()),
        content: MessageContent::text("hello"),
    };

    backend.add_fault(
        Fault::on(FilesystemOperation::BeginTxn)
            .nth(1)
            .returning(FaultKind::Unsupported),
    );
    backend.add_fault(
        Fault::on(FilesystemOperation::WriteFile)
            .path("/idempotency/")
            .nth(1)
            .backend("idempotency intent failed"),
    );

    service
        .accept_inbound_message_with_replay_metadata(
            request.clone(),
            InboundMessageReplayMetadata {
                resolved_model: Some("model-a".into()),
            },
        )
        .await
        .expect_err("the failed idempotency intent must fail acceptance");
    let history = service
        .list_thread_history(ThreadHistoryRequest {
            scope: request_scope.clone(),
            thread_id: thread.thread_id.clone(),
        })
        .await
        .unwrap();
    assert!(
        history.messages.is_empty(),
        "fallback must not persist a message before its recovery intent"
    );

    let accepted = service
        .accept_inbound_message_with_replay_metadata(
            request,
            InboundMessageReplayMetadata {
                resolved_model: Some("model-b".into()),
            },
        )
        .await
        .unwrap();
    assert_eq!(
        accepted.replay_metadata.resolved_model.as_deref(),
        Some("model-b")
    );
    let history = service
        .list_thread_history(ThreadHistoryRequest {
            scope: request_scope,
            thread_id: thread.thread_id,
        })
        .await
        .unwrap();
    assert_eq!(history.messages.len(), 1);
}

#[tokio::test]
async fn filesystem_fallback_resumes_intent_with_original_model_after_message_failure() {
    let backend = Arc::new(FaultInjecting::new(InMemoryBackend::new()));
    let scoped = scoped_threads_fs_at(
        Arc::clone(&backend),
        "tenant-fallback-message-failure",
        "alice",
    );
    let service = FilesystemSessionThreadService::new(scoped);
    let request_scope = scope("fallback-message-failure");
    let thread = service
        .ensure_thread(EnsureThreadRequest {
            scope: request_scope.clone(),
            thread_id: Some(ThreadId::new("thread-fallback-message-failure").unwrap()),
            created_by_actor_id: "actor-a".into(),
            title: None,
            metadata_json: None,
        })
        .await
        .unwrap();
    let request = AcceptInboundMessageRequest {
        scope: request_scope.clone(),
        thread_id: thread.thread_id.clone(),
        actor_id: "actor-a".into(),
        source_binding_id: Some("source-fallback-message-failure".into()),
        reply_target_binding_id: Some("reply-fallback-message-failure".into()),
        external_event_id: Some("event-fallback-message-failure".into()),
        content: MessageContent::text("hello"),
    };

    backend.add_fault(
        Fault::on(FilesystemOperation::BeginTxn)
            .nth(1)
            .returning(FaultKind::Unsupported),
    );
    backend.add_fault(
        Fault::on(FilesystemOperation::WriteFile)
            .path("/messages/")
            .nth(1)
            .backend("message write failed"),
    );

    service
        .accept_inbound_message_with_replay_metadata(
            request.clone(),
            InboundMessageReplayMetadata {
                resolved_model: Some("model-a".into()),
            },
        )
        .await
        .expect_err("the injected message failure must fail acceptance");

    let replay = service
        .replay_accepted_inbound_message(ReplayAcceptedInboundMessageRequest {
            scope: request_scope.clone(),
            actor_id: "actor-a".into(),
            source_binding_id: "source-fallback-message-failure".into(),
            external_event_id: "event-fallback-message-failure".into(),
        })
        .await
        .expect("pending recovery intent is not a failed accepted replay");
    assert!(
        replay.is_none(),
        "a recovery intent without a transcript row must fall through to acceptance resume"
    );

    let accepted = service
        .accept_inbound_message_with_replay_metadata(
            request,
            InboundMessageReplayMetadata {
                resolved_model: Some("model-b".into()),
            },
        )
        .await
        .unwrap();
    assert!(accepted.idempotent_replay);
    assert_eq!(
        accepted.replay_metadata.resolved_model.as_deref(),
        Some("model-a")
    );
    let history = service
        .list_thread_history(ThreadHistoryRequest {
            scope: request_scope,
            thread_id: thread.thread_id,
        })
        .await
        .unwrap();
    assert_eq!(history.messages.len(), 1);
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
    assert!(page_1.next_cursor.is_some());

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

/// PR #6696 declared the transcript and listing projections under each thread,
/// so every thread create paid redundant index-declaration work and left a
/// catalog row per thread behind forever — rows the SQL projection then has to
/// consider on every entry write. (Under the per-declaration trigger design
/// that PR shipped, each also added three triggers per spec to the shared
/// entries table.) The specs are now declared once per mount at the `/threads`
/// alias root, so only a mount's first thread create declares anything and
/// listing still resolves the spec from a deeper path.
#[tokio::test]
async fn filesystem_thread_create_declares_indexes_once_per_mount() {
    let backend = Arc::new(FaultInjecting::new(InMemoryBackend::new()));
    let scoped = scoped_threads_fs_at(Arc::clone(&backend), "tenant-index-ddl", "alice");
    let service = FilesystemSessionThreadService::new(scoped);
    let scope = scope("index-ddl");
    let create = async |id: &str| {
        service
            .ensure_thread(EnsureThreadRequest {
                scope: scope.clone(),
                thread_id: Some(ThreadId::new(id.to_string()).unwrap()),
                created_by_actor_id: "actor-a".into(),
                title: None,
                metadata_json: None,
            })
            .await
            .unwrap();
    };

    create("ddl-000").await;
    let after_first = backend.count(FilesystemOperation::EnsureIndex);
    assert_eq!(
        after_first, 9,
        "a mount's first thread declares four transcript specs and five message lookup specs"
    );

    for index in 1..5 {
        create(&format!("ddl-{index:03}")).await;
    }
    assert_eq!(
        backend.count(FilesystemOperation::EnsureIndex),
        after_first,
        "thread create must issue no index DDL after a mount's first thread"
    );

    // Every declaration lands on the mount root, never a per-thread path.
    for path in backend.recorded_paths(FilesystemOperation::EnsureIndex) {
        assert_eq!(
            path.as_str(),
            "/tenants/tenant-index-ddl/users/alice/threads",
            "projections are declared at the mount root, above the per-thread paths"
        );
    }

    // Listing resolves the root-declared spec from the deeper per-scope path,
    // and needs no further declaration to do it.
    let listed = service
        .list_threads_for_scope(ListThreadsForScopeRequest {
            scope,
            limit: None,
            cursor: None,
        })
        .await
        .unwrap();
    assert_eq!(listed.threads.len(), 5);
    assert_eq!(
        backend.count(FilesystemOperation::EnsureIndex),
        after_first,
        "listing resolves the ancestor-declared spec without redeclaring it"
    );
}

/// Sidebar titles for untitled threads used to be derived on EVERY list
/// request with per-thread transcript probes — an N+1 that dominates listing
/// once a user has many threads. The label is now seeded into the index row
/// at message-accept time, so listing reads no transcripts at all; rows that
/// predate the seeding probe once and heal.
#[tokio::test]
async fn filesystem_list_threads_derives_titles_without_transcript_probes() {
    let backend = Arc::new(FaultInjecting::new(InMemoryBackend::new()));
    let scoped = scoped_threads_fs_at(Arc::clone(&backend), "tenant-title-seed", "alice");
    let service = FilesystemSessionThreadService::new(scoped);
    let scope = scope("title-seed");
    for i in 0..3 {
        let thread_id = ThreadId::new(format!("thread-title-{i}")).unwrap();
        service
            .ensure_thread(EnsureThreadRequest {
                scope: scope.clone(),
                thread_id: Some(thread_id.clone()),
                created_by_actor_id: "actor-a".into(),
                title: None,
                metadata_json: None,
            })
            .await
            .unwrap();
        service
            .accept_inbound_message(AcceptInboundMessageRequest {
                scope: scope.clone(),
                thread_id: thread_id.clone(),
                actor_id: "actor-a".into(),
                source_binding_id: Some("binding-title-seed".into()),
                reply_target_binding_id: None,
                external_event_id: Some(format!("event-title-seed-{i}")),
                content: MessageContent::text(format!("hello sidebar label {i}")),
            })
            .await
            .unwrap();
    }

    // The first list for a scope also runs the one-time thread-index
    // migration, which backfills labels for pre-seeding rows and does probe.
    // Steady state is what write-time seeding is for, so measure the list
    // after that migration has run.
    service
        .list_threads_for_scope(ListThreadsForScopeRequest {
            scope: scope.clone(),
            limit: None,
            cursor: None,
        })
        .await
        .unwrap();
    let reads_before = backend
        .recorded_paths(FilesystemOperation::ReadFile)
        .into_iter()
        .filter(|path| path.as_str().contains("/messages/"))
        .count();
    let listed = service
        .list_threads_for_scope(ListThreadsForScopeRequest {
            scope: scope.clone(),
            limit: None,
            cursor: None,
        })
        .await
        .unwrap();
    assert_eq!(listed.threads.len(), 3);
    for thread in &listed.threads {
        let title = thread.title.as_deref().expect("derived title present");
        assert!(
            title.starts_with("hello sidebar label"),
            "derived from the first user message: {title}"
        );
    }
    let reads_after = backend
        .recorded_paths(FilesystemOperation::ReadFile)
        .into_iter()
        .filter(|path| path.as_str().contains("/messages/"))
        .count();
    assert_eq!(
        reads_after, reads_before,
        "steady-state listing must not probe transcripts when titles are seeded \
         at accept time"
    );
}

/// The sidebar label is a copy of user message text, so redaction has to
/// remove the copy too. Serving listings from the index row made this a real
/// exposure: redaction clears the message body, but a cached label kept the
/// redacted words visible in every thread list.
#[tokio::test]
async fn filesystem_redaction_clears_the_cached_sidebar_title() {
    let backend = Arc::new(InMemoryBackend::new());
    let scoped = scoped_threads_fs_at(backend, "tenant-title-redact", "alice");
    let service = FilesystemSessionThreadService::new(scoped);
    let scope = scope("title-redact");
    let thread_id = ThreadId::new("thread-title-redact").unwrap();
    service
        .ensure_thread(EnsureThreadRequest {
            scope: scope.clone(),
            thread_id: Some(thread_id.clone()),
            created_by_actor_id: "actor-a".into(),
            title: None,
            metadata_json: None,
        })
        .await
        .unwrap();
    let accepted = service
        .accept_inbound_message(AcceptInboundMessageRequest {
            scope: scope.clone(),
            thread_id: thread_id.clone(),
            actor_id: "actor-a".into(),
            source_binding_id: Some("binding-title-redact".into()),
            reply_target_binding_id: None,
            external_event_id: Some("event-title-redact".into()),
            content: MessageContent::text("my social security number is 000-00-0000"),
        })
        .await
        .unwrap();
    let listed = service
        .list_threads_for_scope(ListThreadsForScopeRequest {
            scope: scope.clone(),
            limit: None,
            cursor: None,
        })
        .await
        .unwrap();
    assert!(
        listed.threads[0]
            .title
            .as_deref()
            .is_some_and(|title| title.contains("000-00-0000")),
        "precondition: the label was seeded from the message text"
    );

    service
        .redact_message(RedactMessageRequest {
            scope: scope.clone(),
            thread_id: thread_id.clone(),
            message_id: accepted.message_id,
            redaction_ref: "redaction/audit/title".into(),
        })
        .await
        .expect("redaction succeeds");

    let listed = service
        .list_threads_for_scope(ListThreadsForScopeRequest {
            scope: scope.clone(),
            limit: None,
            cursor: None,
        })
        .await
        .unwrap();
    let title = listed.threads[0].title.as_deref().unwrap_or_default();
    assert!(
        !title.contains("000-00-0000"),
        "redacted text must not survive in the sidebar label, got {title:?}"
    );
}

/// Rows written before write-time seeding existed carry no derived label, so
/// listing derives one per request (read-only — repairing the projection from
/// a list request is what the threads guardrail forbids). The durable backfill
/// belongs to the explicit thread-index migration, after which listing is
/// probe-free.
#[tokio::test]
async fn filesystem_migration_backfills_legacy_derived_titles() {
    let backend = Arc::new(FaultInjecting::new(InMemoryBackend::new()));
    let scoped = scoped_threads_fs_at(Arc::clone(&backend), "tenant-title-heal", "alice");
    let service = FilesystemSessionThreadService::new(scoped);
    let scope = scope("title-heal");
    let thread_id = ThreadId::new("thread-title-heal").unwrap();
    service
        .ensure_thread(EnsureThreadRequest {
            scope: scope.clone(),
            thread_id: Some(thread_id.clone()),
            created_by_actor_id: "actor-a".into(),
            title: None,
            metadata_json: None,
        })
        .await
        .unwrap();
    service
        .accept_inbound_message(AcceptInboundMessageRequest {
            scope: scope.clone(),
            thread_id: thread_id.clone(),
            actor_id: "actor-a".into(),
            source_binding_id: Some("binding-title-heal".into()),
            reply_target_binding_id: None,
            external_event_id: Some("event-title-heal".into()),
            content: MessageContent::text("legacy row message"),
        })
        .await
        .unwrap();

    // Simulate a row written before write-time seeding: strip the derived
    // label out of the index row through the raw backend.
    let index_path = backend
        .recorded_paths(FilesystemOperation::WriteFile)
        .into_iter()
        .rev()
        .find(|path| path.as_str().contains("/thread_index/"))
        .expect("index row written during accept");
    let versioned = backend.get(&index_path).await.unwrap().unwrap();
    let mut row: serde_json::Value = serde_json::from_slice(&versioned.entry.body).unwrap();
    assert!(
        row.as_object_mut()
            .unwrap()
            .remove("derived_title")
            .is_some(),
        "accept seeded the derived title"
    );
    let mut entry = versioned.entry.clone();
    entry.body = serde_json::to_vec(&row).unwrap();
    backend
        .put(&index_path, entry, CasExpectation::Any)
        .await
        .unwrap();

    let probes = |backend: &FaultInjecting<InMemoryBackend>| {
        backend
            .recorded_paths(FilesystemOperation::ReadFile)
            .into_iter()
            .filter(|path| path.as_str().contains("/messages/"))
            .count()
    };
    let before_first = probes(&backend);
    let listed = service
        .list_threads_for_scope(ListThreadsForScopeRequest {
            scope: scope.clone(),
            limit: None,
            cursor: None,
        })
        .await
        .unwrap();
    assert_eq!(
        listed.threads[0].title.as_deref(),
        Some("legacy row message"),
        "legacy row derives via the one-time probe"
    );
    assert!(
        probes(&backend) > before_first,
        "the first list after the strip must probe the transcript"
    );

    // The explicit migration is what makes the label durable.
    service
        .migrate_thread_index_for_scope(&scope)
        .await
        .expect("thread index migration backfills derived titles");

    let before_second = probes(&backend);
    let listed = service
        .list_threads_for_scope(ListThreadsForScopeRequest {
            scope: scope.clone(),
            limit: None,
            cursor: None,
        })
        .await
        .unwrap();
    assert_eq!(
        listed.threads[0].title.as_deref(),
        Some("legacy row message"),
        "the backfilled label serves from the index row"
    );
    assert_eq!(
        probes(&backend),
        before_second,
        "after the migration, listing must be probe-free"
    );
}

/// Writer admission is where the observed QA failure happens: acquiring the
/// sole libSQL writer can time out as `BackendBusy`, and the migration runs
/// from the first transcript read. Classifying only the in-transaction calls
/// left that arm escaping to the caller as the retryable timeline 503 the
/// bounded retry exists to absorb.
#[tokio::test]
async fn filesystem_transcript_migration_retries_writer_admission_contention() {
    let backend = Arc::new(FaultInjecting::new(InMemoryBackend::new()));
    let scoped = scoped_threads_fs_at(Arc::clone(&backend), "tenant-migrate-admit", "alice");
    let service = FilesystemSessionThreadService::new(scoped);
    let scope = scope("migrate-admit");
    let thread_id = ThreadId::new("thread-migrate-admit").unwrap();
    service
        .ensure_thread(EnsureThreadRequest {
            scope: scope.clone(),
            thread_id: Some(thread_id.clone()),
            created_by_actor_id: "actor-a".into(),
            title: None,
            metadata_json: None,
        })
        .await
        .unwrap();
    service
        .append_finalized_assistant_message(AppendFinalizedAssistantMessageRequest {
            scope: scope.clone(),
            thread_id: thread_id.clone(),
            turn_run_id: "run-migrate-admit".into(),
            content: MessageContent::text("assistant reply"),
        })
        .await
        .unwrap();
    let marker = backend
        .recorded_paths(FilesystemOperation::WriteFile)
        .into_iter()
        .find(|path| path.as_str().contains("transcript-index-v2.complete"))
        .expect("seeding wrote the transcript migration marker");
    backend.delete(&marker).await.unwrap();

    // The migration's first attempt to take the writer times out.
    backend.add_fault(
        Fault::on(FilesystemOperation::BeginTxn)
            .nth(1)
            .returning(FaultKind::BackendBusy),
    );

    let history = service
        .list_thread_history(ThreadHistoryRequest {
            scope: scope.clone(),
            thread_id: thread_id.clone(),
        })
        .await
        .expect("writer-admission contention must be retried, not surfaced as a 503");
    assert_eq!(history.messages.len(), 1);
}

/// The one-time transcript-index migration runs on a scope's first transcript
/// read and rewrites message rows under CAS expectations. When that first read
/// overlaps a live turn's message writes, the migration can lose the race; the
/// resulting `VersionMismatch` used to escape to WebUI timeline reads as a
/// retryable 503 (`TimelineUnavailable`, observed under the api-user-capacity
/// stress workload). The migration must re-read and retry the page instead.
#[tokio::test]
async fn filesystem_transcript_migration_retries_a_lost_cas_race() {
    let backend = Arc::new(FaultInjecting::new(InMemoryBackend::new()));
    let scoped = scoped_threads_fs_at(Arc::clone(&backend), "tenant-migrate-race", "alice");
    let service = FilesystemSessionThreadService::new(scoped);
    let scope = scope("migrate-race");
    let thread_id = ThreadId::new("thread-migrate-race").unwrap();
    service
        .ensure_thread(EnsureThreadRequest {
            scope: scope.clone(),
            thread_id: Some(thread_id.clone()),
            created_by_actor_id: "actor-a".into(),
            title: None,
            metadata_json: None,
        })
        .await
        .unwrap();
    service
        .append_finalized_assistant_message(AppendFinalizedAssistantMessageRequest {
            scope: scope.clone(),
            thread_id: thread_id.clone(),
            turn_run_id: "run-migrate-race".into(),
            content: MessageContent::text("assistant reply"),
        })
        .await
        .unwrap();

    // Reset the one-time marker so the next transcript read re-runs the
    // migration with a message row present.
    let marker = backend
        .recorded_paths(FilesystemOperation::WriteFile)
        .into_iter()
        .find(|path| path.as_str().contains("transcript-index-v2.complete"))
        .expect("seeding wrote the transcript migration marker");
    backend.delete(&marker).await.unwrap();

    // Lose the CAS race exactly once: the first migration-transaction write to
    // a message row fails the way a concurrent turn write makes it fail.
    backend.add_fault(
        Fault::on(FilesystemOperation::WriteFile)
            .path("/messages/")
            .nth(1)
            .version_mismatch(),
    );

    let history = service
        .list_thread_history(ThreadHistoryRequest {
            scope: scope.clone(),
            thread_id: thread_id.clone(),
        })
        .await
        .expect("a single lost CAS race must be retried, not surfaced");
    assert_eq!(history.messages.len(), 1);
    let marker_writes = backend
        .recorded_paths(FilesystemOperation::WriteFile)
        .into_iter()
        .filter(|path| path.as_str().contains("transcript-index-v2.complete"))
        .count();
    assert_eq!(
        marker_writes, 2,
        "the retried migration completes and re-writes the one-time marker"
    );
}

/// Retry exhaustion must stay bounded and fail loud with the real cause —
/// a writer that conflicts forever must not pin the migration in a loop.
#[tokio::test]
async fn filesystem_transcript_migration_conflict_retries_are_bounded() {
    let backend = Arc::new(FaultInjecting::new(InMemoryBackend::new()));
    let scoped = scoped_threads_fs_at(Arc::clone(&backend), "tenant-migrate-bound", "alice");
    let service = FilesystemSessionThreadService::new(scoped);
    let scope = scope("migrate-bound");
    let thread_id = ThreadId::new("thread-migrate-bound").unwrap();
    service
        .ensure_thread(EnsureThreadRequest {
            scope: scope.clone(),
            thread_id: Some(thread_id.clone()),
            created_by_actor_id: "actor-a".into(),
            title: None,
            metadata_json: None,
        })
        .await
        .unwrap();
    service
        .append_finalized_assistant_message(AppendFinalizedAssistantMessageRequest {
            scope: scope.clone(),
            thread_id: thread_id.clone(),
            turn_run_id: "run-migrate-bound".into(),
            content: MessageContent::text("assistant reply"),
        })
        .await
        .unwrap();
    let marker = backend
        .recorded_paths(FilesystemOperation::WriteFile)
        .into_iter()
        .find(|path| path.as_str().contains("transcript-index-v2.complete"))
        .expect("seeding wrote the transcript migration marker");
    backend.delete(&marker).await.unwrap();

    let writes_before = backend.count(FilesystemOperation::WriteFile);
    backend.add_fault(
        Fault::on(FilesystemOperation::WriteFile)
            .path("/messages/")
            .version_mismatch(),
    );

    let error = service
        .list_thread_history(ThreadHistoryRequest {
            scope: scope.clone(),
            thread_id: thread_id.clone(),
        })
        .await
        .expect_err("a permanently conflicting migration must fail, not loop");
    assert!(
        matches!(error, SessionThreadError::Backend(_)),
        "retry exhaustion surfaces the underlying conflict: {error:?}"
    );
    let message_row_attempts = backend
        .recorded_paths(FilesystemOperation::WriteFile)
        .into_iter()
        .skip(writes_before)
        .filter(|path| path.as_str().contains("/messages/"))
        .count();
    // Initial attempt + TRANSCRIPT_PAGE_CONFLICT_RETRIES (5) retries.
    assert_eq!(
        message_row_attempts, 6,
        "conflict retries are bounded, not unbounded"
    );
}

#[tokio::test]
async fn filesystem_list_threads_page_does_not_scan_scope_or_source_directory() {
    let backend = Arc::new(FaultInjecting::new(InMemoryBackend::new()));
    let scoped = scoped_threads_fs_at(Arc::clone(&backend), "tenant-index-bounded", "alice");
    let service = FilesystemSessionThreadService::new(scoped);
    let scope = scope("index-bounded");
    for index in 0..100 {
        service
            .ensure_thread(EnsureThreadRequest {
                scope: scope.clone(),
                thread_id: Some(ThreadId::new(format!("bounded-{index:03}")).unwrap()),
                created_by_actor_id: "actor-a".into(),
                title: Some(format!("bounded {index}")),
                metadata_json: None,
            })
            .await
            .unwrap();
    }
    service
        .list_threads_for_scope(ListThreadsForScopeRequest {
            scope: scope.clone(),
            limit: Some(10),
            cursor: None,
        })
        .await
        .expect("complete the required index migration before measuring steady-state reads");
    let query_count = backend.count(FilesystemOperation::Query);
    let list_count = backend.count(FilesystemOperation::ListDir);

    let page = service
        .list_threads_for_scope(ListThreadsForScopeRequest {
            scope,
            limit: Some(10),
            cursor: None,
        })
        .await
        .unwrap();

    assert_eq!(page.threads.len(), 10);
    assert_eq!(backend.count(FilesystemOperation::Query) - query_count, 1);
    assert_eq!(backend.count(FilesystemOperation::ListDir), list_count);
}

#[tokio::test]
async fn filesystem_explicit_migration_rebuilds_missing_thread_index_rows() {
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
    assert_eq!(
        service
            .migrate_thread_index_for_scope(&scope)
            .await
            .unwrap(),
        2
    );

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
        "explicit migration should rebuild durable derived index rows"
    );
}

/// Break caught: a message updated after the migration query but before the
/// migration transaction starts must not make conversation history unavailable.
#[tokio::test]
async fn filesystem_history_survives_transcript_migration_racing_a_message_update() {
    let backend = Arc::new(MigrationRaceBackend::new());
    let scoped = scoped_threads_fs_at(
        Arc::clone(&backend),
        "tenant-transcript-migration-race",
        "alice",
    );
    let service = FilesystemSessionThreadService::new(Arc::clone(&scoped));
    let scope = scope("transcript-migration-race");
    let thread = service
        .ensure_thread(EnsureThreadRequest {
            scope: scope.clone(),
            thread_id: Some(ThreadId::new("thread-transcript-migration-race").unwrap()),
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
            external_event_id: Some("event-transcript-migration-race".into()),
            content: MessageContent::text("hello from the migration race"),
        })
        .await
        .unwrap();

    let migration_marker = transcript_index_migration_marker_path_for_test(&scope);
    if scoped
        .get(&scope.to_resource_scope(), &migration_marker)
        .await
        .expect("test setup reads the migration marker")
        .is_some()
    {
        scoped
            .delete(&scope.to_resource_scope(), &migration_marker)
            .await
            .expect("test setup removes the completed migration marker");
    }
    backend.arm();

    let history = service
        .list_thread_history(ThreadHistoryRequest {
            scope,
            thread_id: thread.thread_id,
        })
        .await
        .expect("a concurrent current-code update must not fail history migration");

    assert_eq!(backend.race_count(), 1, "the test must inject one race");
    assert_eq!(history.messages.len(), 1);
    assert_eq!(
        history.messages[0].content.as_deref(),
        Some("hello from the migration race")
    );
}

#[tokio::test]
async fn optional_index_write_cache_does_not_skip_required_migration_marker() {
    use ironclaw_threads::ListThreadsForScopeRequest;

    let backend = Arc::new(InMemoryBackend::new());
    let scoped = scoped_threads_fs_at(backend, "tenant-index-required", "alice");
    let service = FilesystemSessionThreadService::new(Arc::clone(&scoped));
    let scope = scope("index-required");
    service
        .ensure_thread(EnsureThreadRequest {
            scope: scope.clone(),
            thread_id: Some(ThreadId::new("required-marker-thread").unwrap()),
            created_by_actor_id: "actor-a".into(),
            title: None,
            metadata_json: None,
        })
        .await
        .expect("optional index write");

    let marker = thread_index_migration_marker_path_for_test(&scope);
    assert!(
        scoped
            .get(&scope.to_resource_scope(), &marker)
            .await
            .expect("read marker before required query")
            .is_none()
    );
    service
        .list_threads_for_scope(ListThreadsForScopeRequest {
            scope: scope.clone(),
            limit: None,
            cursor: None,
        })
        .await
        .expect("required index query");
    assert!(
        scoped
            .get(&scope.to_resource_scope(), &marker)
            .await
            .expect("read marker after required query")
            .is_some(),
        "required query must durably finish migration despite the optional cache entry"
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
    service
        .migrate_thread_index_for_scope(&scope)
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
    let ids: Vec<&str> = listed
        .threads
        .iter()
        .map(|record| record.thread_id.as_str())
        .collect();
    assert!(ids.contains(&"legacy-indexed"));
    assert!(ids.contains(&"legacy-missing"));
}

/// Break caught: an index row can exist with its projection keys missing —
/// observed in staging, where a thread kept its record, its index row and all
/// of its messages but never appeared in the sidebar. The ordered projection
/// only emits a row when `indexed` carries the listing keys, so such a row is
/// invisible to `list_threads`, and the scope's completion marker makes the
/// repair pass skip the scope on every later start. Without self-healing the
/// thread stays unreachable for the rest of the volume's life.
#[tokio::test]
async fn filesystem_list_threads_recovers_index_row_without_projection_keys() {
    use ironclaw_threads::ListThreadsForScopeRequest;

    let backend = Arc::new(InMemoryBackend::new());
    let scoped = scoped_threads_fs_at(backend, "tenant-index-unprojected", "alice");
    let service = FilesystemSessionThreadService::new(Arc::clone(&scoped));
    let scope = scope("index-unprojected");

    for id in ["projected-thread", "unprojected-thread"] {
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

    // Listing once writes the scope's thread-index completion marker, which is
    // what later suppresses the repair pass.
    let seeded = service
        .list_threads_for_scope(ListThreadsForScopeRequest {
            scope: scope.clone(),
            limit: None,
            cursor: None,
        })
        .await
        .unwrap();
    assert_eq!(
        seeded.threads.len(),
        2,
        "both threads must list before the index row is damaged"
    );

    // Reproduce the staging row shape: no ordered-projection metadata on the
    // entry while the durable record body remains intact.
    let index_path = thread_index_record_path_for_test(&scope, "unprojected-thread");
    let damaged: serde_json::Value = serde_json::from_slice(
        &scoped
            .get(&scope.to_resource_scope(), &index_path)
            .await
            .unwrap()
            .expect("ensure_thread writes a derived index row")
            .entry
            .body,
    )
    .unwrap();
    scoped
        .put(
            &scope.to_resource_scope(),
            &index_path,
            Entry::bytes(serde_json::to_vec_pretty(&damaged).unwrap()),
            CasExpectation::Any,
        )
        .await
        .expect("test setup rewrites the index row without projection keys");

    // A fresh service stands in for a process restart: no in-memory scope
    // cache, so the scope is re-declared from durable state alone.
    let restarted = FilesystemSessionThreadService::new(Arc::clone(&scoped));
    restarted
        .list_threads_for_scope(ListThreadsForScopeRequest {
            scope: scope.clone(),
            limit: None,
            cursor: None,
        })
        .await
        .unwrap();
    wait_for_thread_index_projection_repair(scoped.as_ref(), &scope).await;
    let listed = restarted
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
    assert!(
        ids.contains(&"projected-thread"),
        "the undamaged thread must still list; got {ids:?}"
    );
    assert!(
        ids.contains(&"unprojected-thread"),
        "a thread whose index row lost its projection keys must still be \
         listable after a restart; got {ids:?}"
    );
}

/// Break caught: repairing through the merge helper is not enough. `cas_update`
/// compares only the decoded body, so an index row whose body already matches a
/// rebuilt record — but whose projection keys are gone — takes the equality
/// no-op path and is never physically rewritten. The repair then reports
/// success while the thread stays invisible, and every later start repeats the
/// scan for nothing. Recovery must not depend on the body differing.
#[tokio::test]
async fn filesystem_list_threads_recovers_current_version_index_row_without_projection_keys() {
    use ironclaw_threads::ListThreadsForScopeRequest;

    let backend = Arc::new(InMemoryBackend::new());
    let scoped = scoped_threads_fs_at(backend, "tenant-index-unprojected-current", "alice");
    let service = FilesystemSessionThreadService::new(Arc::clone(&scoped));
    let scope = scope("index-unprojected-current");

    for id in ["projected-thread", "unprojected-thread"] {
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
    service
        .list_threads_for_scope(ListThreadsForScopeRequest {
            scope: scope.clone(),
            limit: None,
            cursor: None,
        })
        .await
        .unwrap();

    // Strip only the projection metadata: the body is left byte-for-byte as
    // written, so a rebuilt record compares equal to it.
    let index_path = thread_index_record_path_for_test(&scope, "unprojected-thread");
    let body = scoped
        .get(&scope.to_resource_scope(), &index_path)
        .await
        .unwrap()
        .expect("ensure_thread writes a derived index row")
        .entry
        .body;
    scoped
        .put(
            &scope.to_resource_scope(),
            &index_path,
            Entry::bytes(body),
            CasExpectation::Any,
        )
        .await
        .expect("test setup strips the projection metadata only");

    let restarted = FilesystemSessionThreadService::new(Arc::clone(&scoped));
    restarted
        .list_threads_for_scope(ListThreadsForScopeRequest {
            scope: scope.clone(),
            limit: None,
            cursor: None,
        })
        .await
        .unwrap();
    wait_for_thread_index_projection_repair(scoped.as_ref(), &scope).await;
    let listed = restarted
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
    assert!(
        ids.contains(&"unprojected-thread"),
        "recovery must not depend on the record body differing from a rebuild; \
         got {ids:?}"
    );
}

/// Break caught: the repair's completeness check only looked for `scope_key`.
/// The ordered projection also sorts and paginates on `activity_sort` and
/// `thread_id`, so a row that kept `scope_key` but lost either of those two
/// keys took the early-return "already projected" path and was never
/// rewritten — invisible to `list_threads` forever, same as a row missing all
/// three keys.
#[tokio::test]
async fn filesystem_list_threads_recovers_index_row_missing_only_activity_sort_key() {
    use ironclaw_threads::ListThreadsForScopeRequest;

    let backend = Arc::new(InMemoryBackend::new());
    let scoped = scoped_threads_fs_at(backend, "tenant-index-partial-keys", "alice");
    let service = FilesystemSessionThreadService::new(Arc::clone(&scoped));
    let scope = scope("index-partial-keys");

    for id in ["projected-thread", "partial-keys-thread"] {
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
    service
        .list_threads_for_scope(ListThreadsForScopeRequest {
            scope: scope.clone(),
            limit: None,
            cursor: None,
        })
        .await
        .unwrap();

    // Rewrite the row keeping `scope_key` (the only key the old check looked
    // at) but dropping `activity_sort` and `thread_id` — the two keys
    // `query_ordered` actually sorts and paginates on.
    let index_path = thread_index_record_path_for_test(&scope, "partial-keys-thread");
    let versioned = scoped
        .get(&scope.to_resource_scope(), &index_path)
        .await
        .unwrap()
        .expect("ensure_thread writes a derived index row");
    let mut damaged = versioned.entry.clone();
    damaged.indexed.retain(|key, _| key.as_str() == "scope_key");
    assert_eq!(
        damaged.indexed.len(),
        1,
        "test setup must strip every key except scope_key"
    );
    scoped
        .put(
            &scope.to_resource_scope(),
            &index_path,
            damaged,
            CasExpectation::Any,
        )
        .await
        .expect("test setup rewrites the index row missing activity_sort/thread_id");

    let restarted = FilesystemSessionThreadService::new(Arc::clone(&scoped));
    restarted
        .list_threads_for_scope(ListThreadsForScopeRequest {
            scope: scope.clone(),
            limit: None,
            cursor: None,
        })
        .await
        .unwrap();
    wait_for_thread_index_projection_repair(scoped.as_ref(), &scope).await;
    let listed = restarted
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
    assert!(
        ids.contains(&"projected-thread"),
        "the undamaged thread must still list; got {ids:?}"
    );
    assert!(
        ids.contains(&"partial-keys-thread"),
        "a row retaining scope_key but missing activity_sort/thread_id must \
         still be repaired and listable; got {ids:?}"
    );
}

/// Break caught: `ensure_thread` (an optional, `required: false` caller of
/// `ensure_thread_index_query`) declared the scope and cached it as "ready"
/// without ever running repair. A later `required: true` listing call then
/// saw the scope already declared *and* the durable migration marker already
/// complete (written by an earlier process) and returned early before
/// reaching the reconcile step — so in the common production ordering, where
/// a write happens before the first list in a process, self-healing never ran
/// at all.
#[tokio::test]
async fn filesystem_list_threads_recovers_index_row_when_write_precedes_first_list() {
    use ironclaw_threads::ListThreadsForScopeRequest;

    let backend = Arc::new(InMemoryBackend::new());
    let scoped = scoped_threads_fs_at(backend, "tenant-index-write-before-list", "alice");
    let scope = scope("index-write-before-list");

    // Seed durable state, including the scope's migration-complete marker,
    // using a first process that lists before ever writing again.
    {
        let seeding_service = FilesystemSessionThreadService::new(Arc::clone(&scoped));
        for id in ["projected-thread", "unprojected-thread"] {
            seeding_service
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
        seeding_service
            .list_threads_for_scope(ListThreadsForScopeRequest {
                scope: scope.clone(),
                limit: None,
                cursor: None,
            })
            .await
            .unwrap();
    }

    // Damage one row's projection keys, same shape as the other recovery
    // tests, so a fresh process has real repair work to do.
    let index_path = thread_index_record_path_for_test(&scope, "unprojected-thread");
    let damaged: serde_json::Value = serde_json::from_slice(
        &scoped
            .get(&scope.to_resource_scope(), &index_path)
            .await
            .unwrap()
            .expect("ensure_thread writes a derived index row")
            .entry
            .body,
    )
    .unwrap();
    scoped
        .put(
            &scope.to_resource_scope(),
            &index_path,
            Entry::bytes(serde_json::to_vec_pretty(&damaged).unwrap()),
            CasExpectation::Any,
        )
        .await
        .expect("test setup rewrites the index row without projection keys");

    // Fresh process, durable migration marker already complete from seeding.
    // Critically, perform a WRITE (`ensure_thread`) before the first list —
    // the ordering that reproduces the bug — so the optional declaration path
    // caches the scope as "ready" before any required call ever runs. Uses a
    // brand-new thread id, distinct from the damaged row: `ensure_thread`
    // would otherwise refresh the damaged row's own index entry as a side
    // effect (because the fresh process's in-memory "known row" cache is
    // empty too), which would repair it incidentally and defeat the test.
    let restarted = FilesystemSessionThreadService::new(Arc::clone(&scoped));
    restarted
        .ensure_thread(EnsureThreadRequest {
            scope: scope.clone(),
            thread_id: Some(ThreadId::new("write-before-list-thread").unwrap()),
            created_by_actor_id: "actor-a".into(),
            title: Some("write-before-list-thread".into()),
            metadata_json: None,
        })
        .await
        .unwrap();

    restarted
        .list_threads_for_scope(ListThreadsForScopeRequest {
            scope: scope.clone(),
            limit: None,
            cursor: None,
        })
        .await
        .unwrap();
    wait_for_thread_index_projection_repair(scoped.as_ref(), &scope).await;
    let listed = restarted
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
    assert!(
        ids.contains(&"projected-thread"),
        "the undamaged thread must still list; got {ids:?}"
    );
    assert!(
        ids.contains(&"unprojected-thread"),
        "a damaged row must still be repaired and listable even when a write \
         happens before the first list in the process; got {ids:?}"
    );
}

/// Projection repair is background work. A blocked repair must not delay the
/// listing that scheduled it or a listing for another scope.
#[tokio::test]
async fn filesystem_list_threads_reconciles_unrelated_scopes_independently() {
    let blocked_scope = scope("index-scope-lock-blocked");
    let unrelated_scope = scope("index-scope-lock-unrelated");
    let backend = Arc::new(BlockingScopeReconcileBackend::new(format!(
        "/agents/{}/",
        blocked_scope.agent_id.as_str()
    )));
    let scoped = scoped_threads_fs_at(Arc::clone(&backend), "tenant-index-scope-lock", "alice");
    let seeding_service = FilesystemSessionThreadService::new(Arc::clone(&scoped));

    for (scope, thread_id) in [
        (&blocked_scope, "blocked-scope-thread"),
        (&unrelated_scope, "unrelated-scope-thread"),
    ] {
        seeding_service
            .ensure_thread(EnsureThreadRequest {
                scope: scope.clone(),
                thread_id: Some(ThreadId::new(thread_id).unwrap()),
                created_by_actor_id: "actor-a".into(),
                title: Some(thread_id.into()),
                metadata_json: None,
            })
            .await
            .unwrap();
        seeding_service
            .list_threads_for_scope(ListThreadsForScopeRequest {
                scope: scope.clone(),
                limit: None,
                cursor: None,
            })
            .await
            .expect("seed listing writes the completed migration marker");
    }

    backend.arm();
    let restarted = FilesystemSessionThreadService::new(Arc::clone(&scoped));
    let blocked = timeout(
        Duration::from_secs(2),
        restarted.list_threads_for_scope(ListThreadsForScopeRequest {
            scope: blocked_scope.clone(),
            limit: None,
            cursor: None,
        }),
    )
    .await
    .expect("listing must not wait for its background projection repair")
    .expect("blocked scope listing succeeds");
    assert_eq!(blocked.threads.len(), 1);
    timeout(Duration::from_secs(5), backend.reconcile_entered.wait())
        .await
        .expect("the blocked scope enters its background repair scan");

    let unrelated = timeout(
        Duration::from_secs(2),
        restarted.list_threads_for_scope(ListThreadsForScopeRequest {
            scope: unrelated_scope.clone(),
            limit: None,
            cursor: None,
        }),
    )
    .await
    .expect("unrelated scope must not wait behind another scope's repair")
    .expect("unrelated scope listing succeeds");
    assert_eq!(unrelated.threads.len(), 1);

    backend.release_reconcile.wait().await;
    wait_for_thread_index_projection_repair(scoped.as_ref(), &blocked_scope).await;
    wait_for_thread_index_projection_repair(scoped.as_ref(), &unrelated_scope).await;
}

/// Reconciliation is a best-effort repair for rows that the listing projection
/// cannot see. A transient failure while discovering those rows must not turn
/// an otherwise-readable sidebar page into a hard listing failure.
#[tokio::test]
async fn filesystem_list_threads_degrades_to_projected_rows_when_reconcile_fails() {
    let backend = Arc::new(FaultInjecting::new(InMemoryBackend::new()));
    let scoped = scoped_threads_fs_at(
        Arc::clone(&backend),
        "tenant-index-reconcile-failure",
        "alice",
    );
    let seeding_service = FilesystemSessionThreadService::new(Arc::clone(&scoped));
    let scope = scope("index-reconcile-failure");

    for id in ["projected-before-failure-a", "projected-before-failure-b"] {
        seeding_service
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
    seeding_service
        .list_threads_for_scope(ListThreadsForScopeRequest {
            scope: scope.clone(),
            limit: None,
            cursor: None,
        })
        .await
        .expect("seed listing writes the completed migration marker");

    let damaged_path = thread_index_record_path_for_test(&scope, "projected-before-failure-b");
    let damaged_body = scoped
        .get(&scope.to_resource_scope(), &damaged_path)
        .await
        .unwrap()
        .expect("seeded index row exists")
        .entry
        .body;
    scoped
        .put(
            &scope.to_resource_scope(),
            &damaged_path,
            Entry::bytes(damaged_body),
            CasExpectation::Any,
        )
        .await
        .expect("test setup removes the damaged row's projection keys");

    // A fresh process reaches the marker-complete repair branch. Fail its
    // first background directory discovery; the listing itself must still
    // return the projected rows.
    backend.add_fault(
        Fault::on(FilesystemOperation::ListDir)
            .path("/thread_index")
            .nth(1)
            .backend("projection reconcile directory listing fails once"),
    );
    let restarted = FilesystemSessionThreadService::new(scoped);
    let degraded = restarted
        .list_threads_for_scope(ListThreadsForScopeRequest {
            scope: scope.clone(),
            limit: None,
            cursor: None,
        })
        .await
        .expect("a reconcile failure must degrade to projected listing rows");
    let degraded_ids: Vec<&str> = degraded
        .threads
        .iter()
        .map(|record| record.thread_id.as_str())
        .collect();
    assert_eq!(degraded_ids.len(), 1);
    assert!(degraded_ids.contains(&"projected-before-failure-a"));
    assert!(!degraded_ids.contains(&"projected-before-failure-b"));

    // A failed background pass must not write its durable completion marker.
    // Once the transient fault is spent, a later list admits a retry.
    let repaired = timeout(Duration::from_secs(5), async {
        loop {
            let listed = restarted
                .list_threads_for_scope(ListThreadsForScopeRequest {
                    scope: scope.clone(),
                    limit: None,
                    cursor: None,
                })
                .await
                .expect("listing remains available while repair retries");
            if listed.threads.len() == 2 {
                break listed;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("a later listing retries the failed background repair");
    let repaired_ids: Vec<&str> = repaired
        .threads
        .iter()
        .map(|record| record.thread_id.as_str())
        .collect();
    assert_eq!(repaired_ids.len(), 2);
    assert!(repaired_ids.contains(&"projected-before-failure-a"));
    assert!(repaired_ids.contains(&"projected-before-failure-b"));
}

/// A corrupt row or permanently unavailable backend path must not trigger a
/// full-scope repair scan on every later sidebar request. The process gives
/// transient failures a bounded retry budget, then stops admitting the scope
/// without ever writing the durable completion marker for partial work.
#[tokio::test]
async fn filesystem_list_threads_bounds_persistent_projection_repair_failures() {
    let backend = Arc::new(FaultInjecting::new(InMemoryBackend::new()));
    let scoped = scoped_threads_fs_at(
        Arc::clone(&backend),
        "tenant-index-reconcile-persistent-failure",
        "alice",
    );
    let scope = scope("index-reconcile-persistent-failure");
    let seeding_service = FilesystemSessionThreadService::new(Arc::clone(&scoped));
    seeding_service
        .ensure_thread(EnsureThreadRequest {
            scope: scope.clone(),
            thread_id: Some(ThreadId::new("persistent-failure-thread").unwrap()),
            created_by_actor_id: "actor-a".into(),
            title: Some("persistent failure thread".into()),
            metadata_json: None,
        })
        .await
        .unwrap();
    seeding_service
        .list_threads_for_scope(ListThreadsForScopeRequest {
            scope: scope.clone(),
            limit: None,
            cursor: None,
        })
        .await
        .expect("seed listing writes the completed v1 migration marker");

    backend.add_fault(
        Fault::on(FilesystemOperation::ListDir)
            .path("/thread_index")
            .backend("projection repair remains unavailable"),
    );
    let baseline = backend.count(FilesystemOperation::ListDir);
    let restarted = FilesystemSessionThreadService::new(Arc::clone(&scoped));

    for expected_attempts in 1..=3 {
        restarted
            .list_threads_for_scope(ListThreadsForScopeRequest {
                scope: scope.clone(),
                limit: None,
                cursor: None,
            })
            .await
            .expect("listing remains available while background repair fails");
        timeout(Duration::from_secs(5), async {
            while backend.count(FilesystemOperation::ListDir) < baseline + expected_attempts {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the admitted background repair attempt must finish");
    }

    for _ in 0..5 {
        restarted
            .list_threads_for_scope(ListThreadsForScopeRequest {
                scope: scope.clone(),
                limit: None,
                cursor: None,
            })
            .await
            .expect("listing remains available after the retry budget is exhausted");
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        backend.count(FilesystemOperation::ListDir),
        baseline + 3,
        "persistent failure must stop scheduling a full-scope scan on every listing"
    );
    assert!(
        scoped
            .get(
                &scope.to_resource_scope(),
                &thread_index_projection_repair_marker_path_for_test(&scope),
            )
            .await
            .unwrap()
            .is_none(),
        "a scope that exhausted its retry budget must remain durably incomplete"
    );
}

/// A current-version record can still be invisible when its entry sidecar
/// loses the ordered keys. Repair must use the shared CAS retry loop rather
/// than a one-shot conditional put, because a benign concurrent rewrite can
/// race this best-effort projection repair.
#[tokio::test]
async fn filesystem_list_threads_retries_reconcile_projection_repair_after_cas_race() {
    let backend = Arc::new(FaultInjecting::new(InMemoryBackend::new()));
    let scoped = scoped_threads_fs_at(
        Arc::clone(&backend),
        "tenant-index-reconcile-cas-race",
        "alice",
    );
    let scope = scope("index-reconcile-cas-race");
    let thread_id = ThreadId::new("current-version-damaged-row").unwrap();
    let seeding_service = FilesystemSessionThreadService::new(Arc::clone(&scoped));
    seeding_service
        .ensure_thread(EnsureThreadRequest {
            scope: scope.clone(),
            thread_id: Some(thread_id.clone()),
            created_by_actor_id: "actor-a".into(),
            title: Some("current version damaged row".into()),
            metadata_json: None,
        })
        .await
        .unwrap();
    seeding_service
        .list_threads_for_scope(ListThreadsForScopeRequest {
            scope: scope.clone(),
            limit: None,
            cursor: None,
        })
        .await
        .expect("seed listing writes the completed migration marker");

    let index_path = thread_index_record_path_for_test(&scope, thread_id.as_str());
    let current_body = scoped
        .get(&scope.to_resource_scope(), &index_path)
        .await
        .unwrap()
        .expect("seeded index row exists")
        .entry
        .body;
    scoped
        .put(
            &scope.to_resource_scope(),
            &index_path,
            Entry::bytes(current_body),
            CasExpectation::Any,
        )
        .await
        .expect("test setup removes current row projection keys");
    backend.add_fault(
        Fault::on(FilesystemOperation::WriteFile)
            .path(format!("/thread_index/{}.json", thread_id.as_str()))
            .nth(1)
            .version_mismatch(),
    );

    let restarted = FilesystemSessionThreadService::new(Arc::clone(&scoped));
    restarted
        .list_threads_for_scope(ListThreadsForScopeRequest {
            scope: scope.clone(),
            limit: None,
            cursor: None,
        })
        .await
        .expect("listing retries the projection repair after a CAS race");
    wait_for_thread_index_projection_repair(scoped.as_ref(), &scope).await;
    let listed = restarted
        .list_threads_for_scope(ListThreadsForScopeRequest {
            scope,
            limit: None,
            cursor: None,
        })
        .await
        .expect("listing observes the repaired projection after a CAS race");
    assert_eq!(listed.threads.len(), 1);
    assert_eq!(listed.threads[0].thread_id, thread_id);
}

/// Regression for the old live-reconcile cap: scopes larger than one backend
/// page must still repair every hidden row. Repair runs in bounded chunks in
/// the background and writes completion only after the entire scope succeeds.
#[tokio::test]
async fn filesystem_list_threads_repairs_oversized_scope_in_background() {
    let backend = Arc::new(QueryCountingBackend::new());
    let scoped = scoped_threads_fs_at(Arc::clone(&backend), "tenant-index-oversized", "alice");
    let service = FilesystemSessionThreadService::new(Arc::clone(&scoped));
    let scope = scope("index-oversized");
    let damaged_id = ThreadId::new("thread-oversized-hidden").unwrap();

    for index in 0..Page::MAX_LIMIT {
        service
            .ensure_thread(EnsureThreadRequest {
                scope: scope.clone(),
                thread_id: Some(ThreadId::new(format!("thread-oversized-{index:04}")).unwrap()),
                created_by_actor_id: "actor-a".into(),
                title: Some(format!("thread {index}")),
                metadata_json: None,
            })
            .await
            .unwrap();
    }
    service
        .ensure_thread(EnsureThreadRequest {
            scope: scope.clone(),
            thread_id: Some(damaged_id.clone()),
            created_by_actor_id: "actor-a".into(),
            title: Some("damaged oversized row".into()),
            metadata_json: None,
        })
        .await
        .unwrap();

    // Make the fresh service take its marker-complete reconcile branch instead
    // of the full migration. Then remove the damaged row's projection keys.
    scoped
        .put(
            &scope.to_resource_scope(),
            &thread_index_migration_marker_path_for_test(&scope),
            Entry::bytes(b"thread-index-v1".to_vec()),
            CasExpectation::Any,
        )
        .await
        .expect("seed completed thread-index migration marker");
    let damaged_path = thread_index_record_path_for_test(&scope, damaged_id.as_str());
    let damaged_body = scoped
        .get(&scope.to_resource_scope(), &damaged_path)
        .await
        .unwrap()
        .expect("damaged row exists")
        .entry
        .body;
    scoped
        .put(
            &scope.to_resource_scope(),
            &damaged_path,
            Entry::bytes(damaged_body),
            CasExpectation::Any,
        )
        .await
        .expect("strip damaged row projection keys");

    let restarted = FilesystemSessionThreadService::new(Arc::clone(&scoped));
    restarted
        .list_threads_for_scope(ListThreadsForScopeRequest {
            scope: scope.clone(),
            limit: Some(200),
            cursor: None,
        })
        .await
        .expect("listing schedules oversized repair without waiting for it");
    wait_for_thread_index_projection_repair(scoped.as_ref(), &scope).await;

    let mut listed_ids = Vec::new();
    let mut cursor = None;
    loop {
        let page = restarted
            .list_threads_for_scope(ListThreadsForScopeRequest {
                scope: scope.clone(),
                limit: Some(200),
                cursor,
            })
            .await
            .expect("oversized scope remains listable after background repair");
        listed_ids.extend(page.threads.into_iter().map(|record| record.thread_id));
        let Some(next_cursor) = page.next_cursor else {
            break;
        };
        cursor = Some(next_cursor);
    }

    assert_eq!(listed_ids.len(), Page::MAX_LIMIT as usize + 1);
    assert!(
        listed_ids.contains(&damaged_id),
        "the background repair must restore a damaged row beyond one backend page"
    );
    let page_calls = backend.directory_page_calls().await;
    assert_eq!(
        page_calls.len(),
        2,
        "1,025 durable rows must be enumerated as two bounded keyset pages"
    );
    assert_eq!(page_calls[0], (None, Page::MAX_LIMIT as usize));
    assert!(
        page_calls[1].0.is_some(),
        "the second page must continue after the prior page's stable final name"
    );
    assert_eq!(page_calls[1].1, Page::MAX_LIMIT as usize);
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
async fn filesystem_explicit_migration_retries_after_source_read_error() {
    use ironclaw_threads::ListThreadsForScopeRequest;

    let backend = Arc::new(FaultInjecting::new(InMemoryBackend::new()));
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
    // Arm the fault mid-test through the shared `Arc`: `.nth(1)` counts only
    // matching reads that occur AFTER `add_fault`, so the first `thread.json`
    // read for `legacy-flaky-read` during the following list fails exactly
    // once — the same one-shot semantics the old `fail_next_read` flag had.
    backend.add_fault(
        Fault::on(FilesystemOperation::ReadFile)
            .path("/threads/legacy-flaky-read/thread.json")
            .nth(1)
            .backend("thread record reads disabled once by contract test"),
    );
    service.clear_thread_index_cache_for_scope(&scope);
    assert!(
        service
            .migrate_thread_index_for_scope(&scope)
            .await
            .is_err()
    );

    service.clear_thread_index_cache_for_scope(&scope);
    service
        .migrate_thread_index_for_scope(&scope)
        .await
        .unwrap();
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
        "an explicit migration can be retried after a transient source read failure"
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
    let service = FilesystemSessionThreadService::new(scoped)
        .with_thread_index_touch_flush_interval(std::time::Duration::from_millis(20));
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

    // This touch follows earlier activity on the same thread inside the
    // coalescing window. Poll until the detached trailing flush makes the
    // expected cross-thread order observable.
    let final_list = tokio::time::timeout(std::time::Duration::from_secs(1), async {
        loop {
            let listed = service
                .list_threads_for_scope(ListThreadsForScopeRequest {
                    scope: scope_a.clone(),
                    limit: None,
                    cursor: None,
                })
                .await
                .unwrap();
            if listed.threads[0].thread_id.as_str() == "t-older" {
                break listed;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("the trailing touch reaches the sidebar after the flush interval");
    let final_ids: Vec<&str> = final_list
        .threads
        .iter()
        .map(|record| record.thread_id.as_str())
        .collect();
    // Recency wins over transcript length.
    assert_eq!(final_ids, ["t-older", "t-newer"]);
}

/// Regression for #7596: the message/draft/finalize path used to rewrite the
/// complete thread-index row once at every activity call site. A single turn
/// should coalesce that burst while still making the finalized thread lead the
/// activity-sorted sidebar.
#[tokio::test]
async fn filesystem_thread_activity_burst_coalesces_index_touches_and_orders_after_finalize() {
    let backend = Arc::new(QueryCountingBackend::new());
    let scoped = scoped_threads_fs_at(
        Arc::clone(&backend),
        "tenant-coalesced-index-touch",
        "alice",
    );
    let service = FilesystemSessionThreadService::new(scoped)
        .with_thread_index_touch_flush_interval(std::time::Duration::from_millis(500));
    let request_scope = scope("coalesced-index-touch");

    let older = service
        .ensure_thread(EnsureThreadRequest {
            scope: request_scope.clone(),
            thread_id: Some(ThreadId::new("thread-coalesced-older").unwrap()),
            created_by_actor_id: "actor-a".into(),
            title: Some("older".into()),
            metadata_json: None,
        })
        .await
        .unwrap();
    service
        .accept_inbound_message(AcceptInboundMessageRequest {
            scope: request_scope.clone(),
            thread_id: older.thread_id.clone(),
            actor_id: "actor-a".into(),
            source_binding_id: Some("binding-coalesced-touch".into()),
            reply_target_binding_id: None,
            external_event_id: Some("event-coalesced-touch".into()),
            content: MessageContent::text("start the turn"),
        })
        .await
        .unwrap();
    let draft = service
        .append_assistant_draft(AppendAssistantDraftRequest {
            scope: request_scope.clone(),
            thread_id: older.thread_id.clone(),
            turn_run_id: "run-coalesced-touch".into(),
            content: MessageContent::text("working"),
        })
        .await
        .unwrap();
    wait_until_after(draft.updated_at.expect("draft activity stamp")).await;
    let newer = service
        .ensure_thread(EnsureThreadRequest {
            scope: request_scope.clone(),
            thread_id: Some(ThreadId::new("thread-coalesced-newer").unwrap()),
            created_by_actor_id: "actor-a".into(),
            title: Some("newer".into()),
            metadata_json: None,
        })
        .await
        .unwrap();
    wait_until_after(newer.updated_at.expect("newer creation stamp")).await;
    backend.reset_thread_index_put_count();

    service
        .update_assistant_draft(UpdateAssistantDraftRequest {
            scope: request_scope.clone(),
            thread_id: older.thread_id.clone(),
            message_id: draft.message_id,
            content: MessageContent::text("still working"),
        })
        .await
        .unwrap();
    service
        .finalize_assistant_message(
            &request_scope,
            &older.thread_id,
            draft.message_id,
            MessageContent::text("done"),
        )
        .await
        .unwrap();

    let listed = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let listed = service
                .list_threads_for_scope(ListThreadsForScopeRequest {
                    scope: request_scope.clone(),
                    limit: None,
                    cursor: None,
                })
                .await
                .unwrap();
            if listed.threads[0].thread_id == older.thread_id {
                break listed;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("finalized activity reaches the sidebar after the flush interval");
    let burst_puts = backend.thread_index_put_count();
    assert!(
        burst_puts <= 1,
        "one turn must write the thread-index touch at most once per flush interval; saw {burst_puts} writes"
    );
    assert_eq!(
        listed.threads[0].thread_id, older.thread_id,
        "the finalized thread must lead the activity-sorted sidebar"
    );
}

#[tokio::test]
async fn filesystem_failed_deferred_thread_index_touch_is_retried() {
    let backend = Arc::new(FaultInjecting::new(InMemoryBackend::new()));
    let scoped = scoped_threads_fs_at(Arc::clone(&backend), "tenant-deferred-index-retry", "alice");
    let service = FilesystemSessionThreadService::new(scoped)
        .with_thread_index_touch_flush_interval(std::time::Duration::from_secs(1));
    let request_scope = scope("deferred-index-retry");
    let older = service
        .ensure_thread(EnsureThreadRequest {
            scope: request_scope.clone(),
            thread_id: Some(ThreadId::new("thread-deferred-retry-older").unwrap()),
            created_by_actor_id: "actor-a".into(),
            title: Some("older".into()),
            metadata_json: None,
        })
        .await
        .unwrap();
    service
        .accept_inbound_message(AcceptInboundMessageRequest {
            scope: request_scope.clone(),
            thread_id: older.thread_id.clone(),
            actor_id: "actor-a".into(),
            source_binding_id: Some("binding-deferred-index-retry".into()),
            reply_target_binding_id: None,
            external_event_id: Some("event-deferred-index-retry".into()),
            content: MessageContent::text("start activity tracking"),
        })
        .await
        .unwrap();
    let draft = service
        .append_assistant_draft(AppendAssistantDraftRequest {
            scope: request_scope.clone(),
            thread_id: older.thread_id.clone(),
            turn_run_id: "run-deferred-index-retry".into(),
            content: MessageContent::text("working"),
        })
        .await
        .unwrap();
    wait_until_after(draft.updated_at.expect("draft activity stamp")).await;
    let newer = service
        .ensure_thread(EnsureThreadRequest {
            scope: request_scope.clone(),
            thread_id: Some(ThreadId::new("thread-deferred-retry-newer").unwrap()),
            created_by_actor_id: "actor-a".into(),
            title: Some("newer".into()),
            metadata_json: None,
        })
        .await
        .unwrap();
    wait_until_after(newer.updated_at.expect("newer creation stamp")).await;
    backend.add_fault(
        Fault::on(FilesystemOperation::WriteFile)
            .path("/thread_index/")
            .nth(1)
            .backend("deferred thread-index write fails once"),
    );

    service
        .update_assistant_draft(UpdateAssistantDraftRequest {
            scope: request_scope.clone(),
            thread_id: older.thread_id.clone(),
            message_id: draft.message_id,
            content: MessageContent::text("newest activity"),
        })
        .await
        .unwrap();

    let listed = tokio::time::timeout(std::time::Duration::from_secs(4), async {
        loop {
            let listed = service
                .list_threads_for_scope(ListThreadsForScopeRequest {
                    scope: request_scope.clone(),
                    limit: None,
                    cursor: None,
                })
                .await
                .unwrap();
            if listed.threads[0].thread_id == older.thread_id {
                break listed;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("the failed deferred touch is retried");
    assert_eq!(listed.threads[0].thread_id, older.thread_id);
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
/// `DiskFilesystem` is the canonical byte-only `RootFilesystem`: its
/// `put` impl rejects entries with `kind.is_some()`, which `cas_update`
/// surfaces as `CasUnsupported`. This mirrors
/// `filesystem_approval_store_fails_closed_on_byte_only_backend` in
/// `crates/kernel/ironclaw_approvals/tests/approval_store_contract.rs`.
#[tokio::test]
async fn filesystem_session_thread_ensure_thread_fails_closed_on_byte_only_backend() {
    let dir = tempfile::tempdir().expect("temp dir");
    let mut local_fs = DiskFilesystem::new();
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
        "expected Backend(CasUnsupported) from byte-only DiskFilesystem but got {err:?}",
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

fn sample_finalized_attachment_ref() -> AttachmentRef {
    AttachmentRef {
        id: "reply-attachment-1".into(),
        kind: AttachmentKind::Document,
        mime_type: "text/csv".into(),
        filename: Some("report.csv".into()),
        size_bytes: Some(19),
        storage_key: Some("/workspace/report.csv".into()),
        extracted_text: None,
    }
}

/// A second, distinct attachment ref for reuse-identity cases: same shape as
/// [`sample_finalized_attachment_ref`], different landed file.
fn second_finalized_attachment_ref() -> AttachmentRef {
    AttachmentRef {
        id: "reply-attachment-2".into(),
        kind: AttachmentKind::Document,
        mime_type: "text/csv".into(),
        filename: Some("addendum.csv".into()),
        size_bytes: Some(23),
        storage_key: Some("/workspace/addendum.csv".into()),
        extracted_text: None,
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

fn thread_index_migration_marker_path_for_test(scope: &ThreadScope) -> ScopedPath {
    ScopedPath::new(format!(
        "/threads/agents/{}/projects/{}/owners/{}/index-migrations/thread-index-v1.complete",
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

fn thread_index_projection_repair_marker_path_for_test(scope: &ThreadScope) -> ScopedPath {
    ScopedPath::new(format!(
        "/threads/agents/{}/projects/{}/owners/{}/index-migrations/thread-index-projection-v2.complete",
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

async fn wait_for_thread_index_projection_repair<F>(
    scoped: &ScopedFilesystem<F>,
    scope: &ThreadScope,
) where
    F: RootFilesystem,
{
    timeout(Duration::from_secs(5), async {
        let marker = thread_index_projection_repair_marker_path_for_test(scope);
        loop {
            if scoped
                .get(&scope.to_resource_scope(), &marker)
                .await
                .expect("projection repair marker read succeeds")
                .is_some()
            {
                return;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("background thread-index projection repair completes");
}

fn transcript_index_migration_marker_path_for_test(scope: &ThreadScope) -> ScopedPath {
    ScopedPath::new(format!(
        "/threads/agents/{}/projects/{}/owners/{}/index-migrations/transcript-index-v2.complete",
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

/// Real thread store backend that fails message-row writes. Lookup projections
/// share that row, so projection failure cannot leave a source message without
/// its exact-lookup keys.
fn message_row_write_failure_backend() -> FaultInjecting<InMemoryBackend> {
    FaultInjecting::new(InMemoryBackend::new()).with_fault(
        Fault::on(FilesystemOperation::WriteFile)
            .path("/messages/")
            .backend("message row writes disabled by contract test"),
    )
}

/// Fails every legacy lookup-index read so a missing message-row projection
/// cannot silently degrade into a transcript scan.
fn lookup_index_read_failure_backend() -> FaultInjecting<InMemoryBackend> {
    FaultInjecting::new(InMemoryBackend::new())
        .with_fault(
            Fault::on(FilesystemOperation::ReadFile)
                .path("/indexes/assistant-runs/")
                .backend("lookup index reads disabled by contract test"),
        )
        .with_fault(
            Fault::on(FilesystemOperation::ReadFile)
                .path("/indexes/tool-results/")
                .backend("lookup index reads disabled by contract test"),
        )
}

struct QueryCountingBackend {
    inner: InMemoryBackend,
    query_count: AtomicUsize,
    get_count: AtomicUsize,
    thread_index_put_count: AtomicUsize,
    directory_page_calls: Mutex<Vec<(Option<String>, usize)>>,
}

/// Holds the first reconcile directory scan for one scope so the caller test
/// can prove another scope's required listing still makes progress.
struct BlockingScopeReconcileBackend {
    inner: InMemoryBackend,
    blocked_scope_path: String,
    armed: AtomicBool,
    blocked_once: AtomicBool,
    reconcile_entered: Arc<Barrier>,
    release_reconcile: Arc<Barrier>,
}

struct MigrationRaceBackend {
    inner: InMemoryBackend,
    armed: AtomicBool,
    pending_message_path: Mutex<Option<VirtualPath>>,
    race_count: AtomicUsize,
}

impl MigrationRaceBackend {
    fn new() -> Self {
        Self {
            inner: InMemoryBackend::new(),
            armed: AtomicBool::new(false),
            pending_message_path: Mutex::new(None),
            race_count: AtomicUsize::new(0),
        }
    }

    fn arm(&self) {
        self.armed.store(true, Ordering::SeqCst);
    }

    fn race_count(&self) -> usize {
        self.race_count.load(Ordering::SeqCst)
    }
}

impl QueryCountingBackend {
    fn new() -> Self {
        Self {
            inner: InMemoryBackend::new(),
            query_count: AtomicUsize::new(0),
            get_count: AtomicUsize::new(0),
            thread_index_put_count: AtomicUsize::new(0),
            directory_page_calls: Mutex::new(Vec::new()),
        }
    }

    fn query_count(&self) -> usize {
        self.query_count.load(Ordering::SeqCst)
    }

    fn get_count(&self) -> usize {
        self.get_count.load(Ordering::SeqCst)
    }

    fn thread_index_put_count(&self) -> usize {
        self.thread_index_put_count.load(Ordering::SeqCst)
    }

    async fn directory_page_calls(&self) -> Vec<(Option<String>, usize)> {
        self.directory_page_calls.lock().await.clone()
    }

    fn reset_thread_index_put_count(&self) {
        self.thread_index_put_count.store(0, Ordering::SeqCst);
    }
}

impl BlockingScopeReconcileBackend {
    fn new(blocked_scope_path: String) -> Self {
        Self {
            inner: InMemoryBackend::new(),
            blocked_scope_path,
            armed: AtomicBool::new(false),
            blocked_once: AtomicBool::new(false),
            reconcile_entered: Arc::new(Barrier::new(2)),
            release_reconcile: Arc::new(Barrier::new(2)),
        }
    }

    fn arm(&self) {
        self.armed.store(true, Ordering::SeqCst);
    }

    fn blocks_reconcile_for(&self, path: &VirtualPath) -> bool {
        self.armed.load(Ordering::SeqCst)
            && path.as_str().contains(&self.blocked_scope_path)
            && path.as_str().ends_with("/thread_index")
            && !self.blocked_once.swap(true, Ordering::SeqCst)
    }
}

struct TransactionalRaceBackend {
    inner: Arc<InMemoryBackend>,
    txn_lock: Arc<Mutex<()>>,
    idempotency_get_barrier: Arc<Barrier>,
    idempotency_get_count: AtomicUsize,
}

struct FallbackRaceBackend {
    inner: InMemoryBackend,
    begin_barrier: Barrier,
    begin_count: AtomicUsize,
}

impl FallbackRaceBackend {
    fn new() -> Self {
        Self {
            inner: InMemoryBackend::new(),
            begin_barrier: Barrier::new(2),
            begin_count: AtomicUsize::new(0),
        }
    }
}

struct ConcurrentToolResultWriteBackend {
    inner: InMemoryBackend,
    tool_result_get_barrier: Barrier,
    tool_result_get_count: AtomicUsize,
}

impl ConcurrentToolResultWriteBackend {
    fn new() -> Self {
        Self {
            inner: InMemoryBackend::new(),
            tool_result_get_barrier: Barrier::new(2),
            tool_result_get_count: AtomicUsize::new(0),
        }
    }

    fn is_tool_result_record_path(path: &VirtualPath) -> bool {
        path.as_str().contains("/tool_results/")
    }
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
        if path.as_str().contains("/thread_index/") {
            self.thread_index_put_count.fetch_add(1, Ordering::SeqCst);
        }
        self.inner.put(path, entry, cas).await
    }

    async fn get(&self, path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
        self.get_count.fetch_add(1, Ordering::SeqCst);
        self.inner.get(path).await
    }

    async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
        self.inner.list_dir(path).await
    }

    async fn list_dir_page(
        &self,
        path: &VirtualPath,
        after: Option<&str>,
        max_entries: usize,
    ) -> Result<Vec<DirEntry>, FilesystemError> {
        self.directory_page_calls
            .lock()
            .await
            .push((after.map(str::to_owned), max_entries));
        self.inner.list_dir_page(path, after, max_entries).await
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

    async fn query_ordered(
        &self,
        path: &VirtualPath,
        filter: &Filter,
        page: &OrderedPage,
    ) -> Result<Vec<VersionedEntry>, FilesystemError> {
        self.query_count.fetch_add(1, Ordering::SeqCst);
        self.inner.query_ordered(path, filter, page).await
    }

    async fn ensure_index(
        &self,
        path: &VirtualPath,
        spec: &IndexSpec,
    ) -> Result<(), FilesystemError> {
        self.inner.ensure_index(path, spec).await
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
impl RootFilesystem for BlockingScopeReconcileBackend {
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
        self.inner.get(path).await
    }

    async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
        if self.blocks_reconcile_for(path) {
            self.reconcile_entered.wait().await;
            self.release_reconcile.wait().await;
        }
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

    async fn query_ordered(
        &self,
        path: &VirtualPath,
        filter: &Filter,
        page: &OrderedPage,
    ) -> Result<Vec<VersionedEntry>, FilesystemError> {
        self.inner.query_ordered(path, filter, page).await
    }

    async fn ensure_index(
        &self,
        path: &VirtualPath,
        spec: &IndexSpec,
    ) -> Result<(), FilesystemError> {
        self.inner.ensure_index(path, spec).await
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
impl RootFilesystem for MigrationRaceBackend {
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
        let rows = self.inner.query(path, filter, page).await?;
        if self.armed.load(Ordering::SeqCst)
            && self.race_count.load(Ordering::SeqCst) == 0
            && let Some(message) = rows
                .iter()
                .find(|row| row.path.as_str().contains("/messages/"))
        {
            *self.pending_message_path.lock().await = Some(message.path.clone());
        }
        Ok(rows)
    }

    async fn query_ordered(
        &self,
        path: &VirtualPath,
        filter: &Filter,
        page: &OrderedPage,
    ) -> Result<Vec<VersionedEntry>, FilesystemError> {
        self.inner.query_ordered(path, filter, page).await
    }

    async fn ensure_index(
        &self,
        path: &VirtualPath,
        spec: &IndexSpec,
    ) -> Result<(), FilesystemError> {
        self.inner.ensure_index(path, spec).await
    }

    async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
        self.inner.stat(path).await
    }

    async fn delete(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        self.inner.delete(path).await
    }

    async fn begin(&self, path: &VirtualPath) -> Result<Box<dyn StorageTxn>, FilesystemError> {
        if let Some(message_path) = self.pending_message_path.lock().await.take() {
            let versioned =
                self.inner
                    .get(&message_path)
                    .await?
                    .ok_or_else(|| FilesystemError::NotFound {
                        path: message_path.clone(),
                        operation: FilesystemOperation::ReadFile,
                    })?;
            self.inner
                .put(&message_path, versioned.entry, CasExpectation::Any)
                .await?;
            self.race_count.fetch_add(1, Ordering::SeqCst);
        }
        self.inner.begin(path).await
    }

    async fn reserve_sequence(&self, path: &VirtualPath) -> Result<SeqNo, FilesystemError> {
        self.inner.reserve_sequence(path).await
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

    async fn query_ordered(
        &self,
        path: &VirtualPath,
        filter: &Filter,
        page: &OrderedPage,
    ) -> Result<Vec<VersionedEntry>, FilesystemError> {
        self.inner.query_ordered(path, filter, page).await
    }

    async fn ensure_index(
        &self,
        path: &VirtualPath,
        spec: &IndexSpec,
    ) -> Result<(), FilesystemError> {
        self.inner.ensure_index(path, spec).await
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
impl RootFilesystem for FallbackRaceBackend {
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

    async fn query_ordered(
        &self,
        path: &VirtualPath,
        filter: &Filter,
        page: &OrderedPage,
    ) -> Result<Vec<VersionedEntry>, FilesystemError> {
        self.inner.query_ordered(path, filter, page).await
    }

    async fn ensure_index(
        &self,
        path: &VirtualPath,
        spec: &IndexSpec,
    ) -> Result<(), FilesystemError> {
        self.inner.ensure_index(path, spec).await
    }

    async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
        self.inner.stat(path).await
    }

    async fn delete(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        self.inner.delete(path).await
    }

    async fn begin(&self, path: &VirtualPath) -> Result<Box<dyn StorageTxn>, FilesystemError> {
        if self.begin_count.fetch_add(1, Ordering::SeqCst) < 2 {
            self.begin_barrier.wait().await;
            return Err(FilesystemError::Unsupported {
                path: path.clone(),
                operation: FilesystemOperation::BeginTxn,
            });
        }
        self.inner.begin(path).await
    }

    async fn reserve_sequence(&self, path: &VirtualPath) -> Result<SeqNo, FilesystemError> {
        self.inner.reserve_sequence(path).await
    }
}

#[async_trait]
impl RootFilesystem for ConcurrentToolResultWriteBackend {
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
        if Self::is_tool_result_record_path(path)
            && self.tool_result_get_count.fetch_add(1, Ordering::SeqCst) < 2
        {
            let result = self.inner.get(path).await?;
            self.tool_result_get_barrier.wait().await;
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

    async fn query_ordered(
        &self,
        path: &VirtualPath,
        filter: &Filter,
        page: &OrderedPage,
    ) -> Result<Vec<VersionedEntry>, FilesystemError> {
        self.inner.query_ordered(path, filter, page).await
    }

    async fn ensure_index(
        &self,
        path: &VirtualPath,
        spec: &IndexSpec,
    ) -> Result<(), FilesystemError> {
        self.inner.ensure_index(path, spec).await
    }

    async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
        self.inner.stat(path).await
    }

    async fn delete(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        self.inner.delete(path).await
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

    // Push the Draft outside the first durable page selected for a 16-message
    // context window. The summary still intersects that recent page, so a
    // backend that validates summaries only against the first raw tail would
    // incorrectly surface it.
    for sequence in 4..=20 {
        service
            .accept_inbound_message(AcceptInboundMessageRequest {
                scope: scope.clone(),
                thread_id: thread.thread_id.clone(),
                actor_id: "actor-a".into(),
                source_binding_id: None,
                reply_target_binding_id: None,
                external_event_id: None,
                content: MessageContent::text(format!("filler {sequence}")),
            })
            .await
            .unwrap();
    }

    // Summary spans [1..4] covering the Draft at seq 2. Its terminal sequence
    // is present in the first durable page, but its start and the Draft are
    // not; pagination must continue far enough to validate the complete range.
    service
        .create_summary_artifact(CreateSummaryArtifactRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
            start_sequence: 1,
            end_sequence: 4,
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
        16,
        "summary must be suppressed for draft-spanning range"
    );
    assert!(
        context
            .messages
            .iter()
            .all(|message| message.kind != MessageKind::Summary)
    );
    assert_eq!(context.messages[0].content, "filler 5");
    assert_eq!(context.messages[15].content, "filler 20");
    let truncation = context
        .recent_window_truncation
        .expect("the omitted fourth user message is the exact truncation boundary");
    assert_eq!(truncation.omitted_through_sequence, 4);
    assert_eq!(truncation.omitted_through_kind, MessageKind::User);
}

// Real thread store backend that fails only the summary-artifact write, so all
// prior reads in `create_summary_artifact` succeed and only the final CAS `put`
// errors — the shape `persist_summary` (ironclaw_loop_host::compaction_task)
// hits in production (#5838). Folds the former hand-rolled
// `SummaryWriteFailureBackend` onto `FaultInjecting`: an always-firing
// path-scoped `WriteFile` fault on `/summaries/`.
fn summary_write_failure_backend() -> FaultInjecting<InMemoryBackend> {
    FaultInjecting::new(InMemoryBackend::new()).with_fault(
        Fault::on(FilesystemOperation::WriteFile)
            .path("/summaries/")
            .backend("synthetic summary artifact write failure (contract test)"),
    )
}

#[tokio::test]
async fn create_summary_artifact_surfaces_backend_error_on_storage_write_failure() {
    // Regression for #5838: the backend cause must survive to the caller
    // instead of being collapsed away before it reaches `persist_summary`.
    let backend = Arc::new(summary_write_failure_backend());
    let scoped = scoped_threads_fs_at(backend, "tenant-summary-write-fail", "alice");
    let service = FilesystemSessionThreadService::new(scoped);
    let scope = scope("fs-summary-write-fail");
    let thread = service
        .ensure_thread(EnsureThreadRequest {
            scope: scope.clone(),
            thread_id: Some(ThreadId::new("thread-summary-write-fail").unwrap()),
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
            content: MessageContent::text("large tool-heavy turn content"),
        })
        .await
        .unwrap();

    let err = service
        .create_summary_artifact(CreateSummaryArtifactRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
            start_sequence: 1,
            end_sequence: 1,
            summary_kind: SummaryKind::Compaction,
            content: MessageContent::text("compacted summary body"),
            model_context_policy: Some(SummaryModelContextPolicy::ReplaceRangeWhenSelected),
        })
        .await
        .expect_err("summary artifact write must surface the backend failure, not succeed");

    match &err {
        SessionThreadError::Backend(reason) => {
            assert!(
                reason.contains("synthetic summary artifact write failure"),
                "expected the real backend cause to survive to the caller, got: {reason}"
            );
        }
        other => panic!("expected SessionThreadError::Backend, got: {other:?}"),
    }
}

#[tokio::test]
async fn filesystem_assistant_draft_append_allows_later_reply_in_same_turn_run() {
    let backend = Arc::new(InMemoryBackend::new());
    let scoped = scoped_threads_fs_at(backend, "tenant-assistant-sibling", "alice");
    let service = FilesystemSessionThreadService::new(scoped);
    let scope = scope("assistant-sibling");
    let thread = service
        .ensure_thread(EnsureThreadRequest {
            scope: scope.clone(),
            thread_id: Some(ThreadId::new("thread-assistant-sibling").unwrap()),
            created_by_actor_id: "actor-a".into(),
            title: None,
            metadata_json: None,
        })
        .await
        .unwrap();

    let first = service
        .append_assistant_draft(AppendAssistantDraftRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
            turn_run_id: "run-assistant-sibling".into(),
            content: MessageContent::text("draft"),
        })
        .await
        .unwrap();
    service
        .finalize_assistant_message(
            &scope,
            &thread.thread_id,
            first.message_id,
            MessageContent::text("first reply"),
        )
        .await
        .unwrap();

    let retry = service
        .append_assistant_draft(AppendAssistantDraftRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
            turn_run_id: "run-assistant-sibling".into(),
            content: MessageContent::text("first reply"),
        })
        .await
        .unwrap();
    assert_eq!(retry.message_id, first.message_id);

    let second = service
        .append_assistant_draft(AppendAssistantDraftRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
            turn_run_id: "run-assistant-sibling".into(),
            content: MessageContent::text("second reply"),
        })
        .await
        .unwrap();
    assert_ne!(second.message_id, first.message_id);
    service
        .finalize_assistant_message(
            &scope,
            &thread.thread_id,
            second.message_id,
            MessageContent::text("second reply"),
        )
        .await
        .unwrap();

    let history = service
        .list_thread_history(ThreadHistoryRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
        })
        .await
        .unwrap();
    let assistant_messages: Vec<_> = history
        .messages
        .iter()
        .filter(|message| message.kind == MessageKind::Assistant)
        .collect();
    assert_eq!(assistant_messages.len(), 2);
    assert_eq!(
        assistant_messages[0].content.as_deref(),
        Some("first reply")
    );
    assert_eq!(
        assistant_messages[1].content.as_deref(),
        Some("second reply")
    );

    let latest = service
        .finalized_assistant_message_by_run(FinalizedAssistantMessageByRunRequest {
            scope,
            thread_id: thread.thread_id,
            turn_run_id: "run-assistant-sibling".into(),
        })
        .await
        .unwrap()
        .expect("latest finalized assistant message remains indexed by run");
    assert_eq!(latest.message_id, second.message_id);
}

#[tokio::test]
async fn filesystem_queued_user_message_is_resequenced_when_submitted() {
    let backend = Arc::new(InMemoryBackend::new());
    let scoped = scoped_threads_fs_at(backend, "tenant-queued-order", "alice");
    let service = FilesystemSessionThreadService::new(scoped);
    let scope = scope("queued-order");
    let thread = service
        .ensure_thread(EnsureThreadRequest {
            scope: scope.clone(),
            thread_id: Some(ThreadId::new("thread-queued-order").unwrap()),
            created_by_actor_id: "actor-a".into(),
            title: None,
            metadata_json: None,
        })
        .await
        .unwrap();

    let queued = service
        .accept_inbound_message(AcceptInboundMessageRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
            actor_id: "actor-a".into(),
            source_binding_id: None,
            reply_target_binding_id: None,
            external_event_id: None,
            content: MessageContent::text("queued follow-up"),
        })
        .await
        .unwrap();
    service
        .mark_message_queued(
            &scope,
            &thread.thread_id,
            queued.message_id,
            "run-queued-order".into(),
        )
        .await
        .unwrap();
    let assistant = service
        .append_assistant_draft(AppendAssistantDraftRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
            turn_run_id: "run-queued-order".into(),
            content: MessageContent::text("assistant boundary"),
        })
        .await
        .unwrap();
    service
        .finalize_assistant_message(
            &scope,
            &thread.thread_id,
            assistant.message_id,
            MessageContent::text("assistant boundary"),
        )
        .await
        .unwrap();

    let submitted = service
        .mark_message_submitted(
            &scope,
            &thread.thread_id,
            queued.message_id,
            "turn-queued-order".into(),
            "run-queued-order".into(),
        )
        .await
        .unwrap();
    assert!(assistant.sequence < submitted.sequence);

    let history = service
        .list_thread_history(ThreadHistoryRequest {
            scope,
            thread_id: thread.thread_id,
        })
        .await
        .unwrap();
    assert_eq!(history.messages[0].kind, MessageKind::Assistant);
    assert_eq!(history.messages[1].kind, MessageKind::User);
    assert_eq!(
        history.messages[1].content.as_deref(),
        Some("queued follow-up")
    );
}

#[tokio::test]
async fn filesystem_mark_message_submitted_is_idempotent_for_same_run() {
    // Dual-backend parity with the in-memory contract: the queued-message
    // consumer acks on an at-least-once path, so the SAME run re-submitting is an
    // idempotent no-op, while a DIFFERENT run is still rejected as an invalid
    // transition.
    let backend = Arc::new(InMemoryBackend::new());
    let scoped = scoped_threads_fs_at(backend, "tenant-idempotent-submit", "alice");
    let service = FilesystemSessionThreadService::new(scoped);
    let scope = scope("idempotent-submit");
    let thread = service
        .ensure_thread(EnsureThreadRequest {
            scope: scope.clone(),
            thread_id: Some(ThreadId::new("thread-idempotent-submit").unwrap()),
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
            content: MessageContent::text("idempotent submit"),
        })
        .await
        .unwrap();

    service
        .mark_message_submitted(
            &scope,
            &thread.thread_id,
            accepted.message_id,
            "turn-1".into(),
            "run-1".into(),
        )
        .await
        .expect("first submit");
    service
        .mark_message_submitted(
            &scope,
            &thread.thread_id,
            accepted.message_id,
            "turn-1".into(),
            "run-1".into(),
        )
        .await
        .expect("idempotent re-submit for the same run must succeed");

    let foreign = service
        .mark_message_submitted(
            &scope,
            &thread.thread_id,
            accepted.message_id,
            "turn-2".into(),
            "run-2".into(),
        )
        .await;
    assert!(
        matches!(
            foreign,
            Err(SessionThreadError::InvalidMessageTransition { .. })
        ),
        "a different run must not re-submit an already-submitted message, got {foreign:?}"
    );

    let history = service
        .list_thread_history(ThreadHistoryRequest {
            scope,
            thread_id: thread.thread_id,
        })
        .await
        .unwrap();
    assert_eq!(history.messages[0].status, MessageStatus::Submitted);
    assert_eq!(history.messages[0].turn_run_id.as_deref(), Some("run-1"));
}

/// Taxonomy baseline for unbound turns (filesystem twin of the in-memory
/// pin): a thread whose scope carries NO `owner_user_id` lands under scope
/// axes without an `/owners/<user>` segment, so an owner-scoped listing —
/// which reads the per-axes thread index — is structurally unable to surface
/// it, and vice versa.
#[tokio::test]
async fn filesystem_list_threads_excludes_ownerless_threads_from_owner_scoped_listings() {
    use ironclaw_threads::ListThreadsForScopeRequest;

    let backend = Arc::new(InMemoryBackend::new());
    let scoped = scoped_threads_fs_at(backend, "tenant-host", "alice");
    let service = FilesystemSessionThreadService::new(scoped);

    let owned_scope = scope("taxonomy");
    let ownerless_scope = ThreadScope {
        owner_user_id: None,
        ..owned_scope.clone()
    };

    service
        .ensure_thread(EnsureThreadRequest {
            scope: owned_scope.clone(),
            thread_id: Some(ThreadId::new("t-owned-001").unwrap()),
            created_by_actor_id: "actor-taxonomy".into(),
            title: Some("owned".into()),
            metadata_json: None,
        })
        .await
        .unwrap();
    service
        .ensure_thread(EnsureThreadRequest {
            scope: ownerless_scope.clone(),
            thread_id: Some(ThreadId::new("t-ownerless-001").unwrap()),
            created_by_actor_id: "actor-taxonomy".into(),
            title: Some("ownerless".into()),
            metadata_json: None,
        })
        .await
        .unwrap();

    let owner_view = service
        .list_threads_for_scope(ListThreadsForScopeRequest {
            scope: owned_scope,
            limit: None,
            cursor: None,
        })
        .await
        .unwrap();
    let owner_ids: Vec<&str> = owner_view
        .threads
        .iter()
        .map(|record| record.thread_id.as_str())
        .collect();
    assert_eq!(
        owner_ids,
        ["t-owned-001"],
        "an owner-scoped listing must never surface ownerless threads"
    );

    let ownerless_view = service
        .list_threads_for_scope(ListThreadsForScopeRequest {
            scope: ownerless_scope,
            limit: None,
            cursor: None,
        })
        .await
        .unwrap();
    let ownerless_ids: Vec<&str> = ownerless_view
        .threads
        .iter()
        .map(|record| record.thread_id.as_str())
        .collect();
    assert_eq!(
        ownerless_ids,
        ["t-ownerless-001"],
        "an ownerless-scope enumeration must never surface owner-scoped threads"
    );
}

fn filesystem_prepared_request(label: &str, key: &str) -> ironclaw_threads::PreparedContextRequest {
    ironclaw_threads::PreparedContextRequest {
        scope: ThreadScope {
            owner_user_id: None,
            ..scope(label)
        },
        actor_id: format!("user-{label}"),
        system_prompt: "You are a background task.".to_string(),
        messages: vec![ironclaw_llm::agent_message::AgentMessage {
            role: ironclaw_llm::agent_message::AgentMessageRole::User,
            content: vec![ironclaw_llm::agent_message::ContentPart::text(
                "do the thing",
            )],
        }],
        declarations: ironclaw_host_api::prepared_context::PreparedTurnDeclarations {
            tools: Vec::new(),
            output: ironclaw_host_api::output::OutputContract::JsonSchema {
                name: "filesystem_v1".to_string(),
                schema: serde_json::json!({ "type": "object" }),
            },
            limits: ironclaw_host_api::prepared_context::TurnLimits {
                max_model_calls: Some(4),
                max_capability_invocations: None,
                max_wall_clock_seconds: None,
            },
            require_no_approval: false,
        },
        idempotency_key: key.to_string(),
        thread_id: ThreadId::new(format!("unbound-{key}")).expect("thread id"),
        title: None,
        metadata_json: None,
    }
}

/// One-time backfill: a legacy subagent thread (metadata
/// `{"kind":"subagent",…}` with no marker) is stamped on the first listing
/// and stays hidden from then on — the single-marker predicate needs no
/// second spelling.
#[tokio::test]
async fn filesystem_listing_backfills_the_marker_onto_legacy_subagent_threads() {
    let backend = Arc::new(InMemoryBackend::new());
    let scoped = scoped_threads_fs_at(backend, "tenant-host", "alice");
    let service = FilesystemSessionThreadService::new(scoped);
    let listing_scope = scope("legacy-backfill");
    service
        .ensure_thread(EnsureThreadRequest {
            scope: listing_scope.clone(),
            thread_id: Some(ThreadId::new("subagent-legacy-001").unwrap()),
            created_by_actor_id: "actor-legacy".into(),
            title: None,
            metadata_json: Some(
                serde_json::json!({"kind": "subagent", "parent_run_id": "run-1"}).to_string(),
            ),
        })
        .await
        .unwrap();
    service
        .ensure_thread(EnsureThreadRequest {
            scope: listing_scope.clone(),
            thread_id: Some(ThreadId::new("t-visible-legacy-001").unwrap()),
            created_by_actor_id: "actor-legacy".into(),
            title: Some("visible".into()),
            metadata_json: None,
        })
        .await
        .unwrap();

    let view = service
        .list_threads_for_scope(ListThreadsForScopeRequest {
            scope: listing_scope.clone(),
            limit: None,
            cursor: None,
        })
        .await
        .unwrap();
    let ids: Vec<&str> = view
        .threads
        .iter()
        .map(|record| record.thread_id.as_str())
        .collect();
    assert_eq!(
        ids,
        ["t-visible-legacy-001"],
        "the legacy subagent thread must be stamped and hidden on first listing"
    );

    // The stamp is durable — a second listing (marker predicate only, no
    // legacy spelling consulted) still hides it.
    let second = service
        .list_threads_for_scope(ListThreadsForScopeRequest {
            scope: listing_scope.clone(),
            limit: None,
            cursor: None,
        })
        .await
        .unwrap();
    assert_eq!(second.threads.len(), 1);

    // The second listing hiding the thread is consistent with either the
    // durable marker stamp OR the legacy `"kind":"subagent"` spelling still
    // matching — prove it is actually the stamp by reading the thread record
    // back and asserting the marker landed.
    let stamped = service
        .read_thread(ThreadHistoryRequest {
            scope: listing_scope,
            thread_id: ThreadId::new("subagent-legacy-001").unwrap(),
        })
        .await
        .expect("legacy subagent thread record is readable after backfill");
    let metadata: serde_json::Value = stamped
        .metadata_json
        .as_deref()
        .and_then(|raw| serde_json::from_str(raw).ok())
        .expect("stamped thread carries JSON object metadata");
    assert_eq!(
        metadata.get(PREPARED_CONTEXT_METADATA_MARKER_KEY),
        Some(&serde_json::Value::Bool(true)),
        "the backfill must durably stamp the marker, not merely rely on the legacy \
         \"kind\":\"subagent\" spelling still matching: {metadata:?}"
    );
}

/// Two racing accepts for the SAME prepared request converge: deterministic
/// ids make the seeds collide row-by-row (CAS Absent; losers skip), and the
/// commit marker written last decides exactly one non-replay winner. Both
/// callers get the same thread and pin, and the rows are not duplicated.
#[tokio::test(flavor = "multi_thread")]
async fn filesystem_concurrent_duplicate_prepared_accepts_converge() {
    let backend = Arc::new(InMemoryBackend::new());
    let scoped = scoped_threads_fs_at(backend, "tenant-host", "alice");
    let service = Arc::new(FilesystemSessionThreadService::new(scoped));
    let request = filesystem_prepared_request("race-fs", "race-fs-key-1");

    let left = {
        let service = Arc::clone(&service);
        let request = request.clone();
        tokio::spawn(async move { service.accept_prepared_context(request).await })
    };
    let right = {
        let service = Arc::clone(&service);
        let request = request.clone();
        tokio::spawn(async move { service.accept_prepared_context(request).await })
    };
    let (left, right) = tokio::join!(left, right);
    let left = left.expect("join").expect("left accept");
    let right = right.expect("join").expect("right accept");

    assert_eq!(left.thread_id, right.thread_id);
    assert_eq!(left.accepted_message_ref, right.accepted_message_ref);
    assert_eq!(
        [left.idempotent_replay, right.idempotent_replay]
            .iter()
            .filter(|replayed| !**replayed)
            .count(),
        1,
        "exactly one racer is the non-replay winner (left={left:?} right={right:?})"
    );
    let history = service
        .list_thread_history(ThreadHistoryRequest {
            scope: request.scope.clone(),
            thread_id: left.thread_id.clone(),
        })
        .await
        .expect("history");
    assert_eq!(
        history.messages.len(),
        2,
        "racing accepts must not duplicate the seeded rows: {:?}",
        history.messages.iter().map(|m| m.kind).collect::<Vec<_>>()
    );
}

/// A crash BEFORE the commit marker (rows seeded, record write fails) is
/// retryable: the retry converges on the same thread, writes the marker, and
/// reports the accept as fresh — never an orphaned half-seeded thread.
#[tokio::test]
async fn filesystem_prepared_accept_retries_after_a_crash_before_the_commit_marker() {
    let backend = Arc::new(FaultInjecting::new(InMemoryBackend::new()));
    let scoped = scoped_threads_fs_at(Arc::clone(&backend), "tenant-host", "alice");
    let service = FilesystemSessionThreadService::new(scoped);
    let request = filesystem_prepared_request("marker-crash-fs", "marker-crash-fs-key-1");

    // The commit marker is the ONE write to `prepared_context.json`.
    backend.add_fault(
        Fault::on(FilesystemOperation::WriteFile)
            .path("prepared_context.json")
            .nth(1)
            .backend("crash before commit marker"),
    );
    let error = service
        .accept_prepared_context(request.clone())
        .await
        .expect_err("the poisoned marker write must fail the accept");
    assert!(
        !matches!(
            error,
            SessionThreadError::InvalidPreparedContext { .. }
                | SessionThreadError::PreparedContextKeyMismatch { .. }
        ),
        "the crash must surface as a backend failure, not a validation shape: {error:?}"
    );

    let retried = service
        .accept_prepared_context(request.clone())
        .await
        .expect("the retry succeeds once the fault clears");
    assert!(
        !retried.idempotent_replay,
        "no commit marker landed, so the retry is the first completed accept"
    );
    let history = service
        .list_thread_history(ThreadHistoryRequest {
            scope: request.scope.clone(),
            thread_id: retried.thread_id.clone(),
        })
        .await
        .expect("history");
    assert_eq!(
        history.messages.len(),
        2,
        "the crashed attempt's rows converge instead of duplicating: {:?}",
        history.messages.iter().map(|m| m.kind).collect::<Vec<_>>()
    );
}

/// Filesystem twin of the delete-then-re-accept pin: the journaled record
/// lives under the thread root, so the delete removes it and the same
/// accept re-run mints fresh.
#[tokio::test]
async fn filesystem_deleting_a_prepared_thread_deletes_its_replay_record() {
    let backend = Arc::new(InMemoryBackend::new());
    let scoped = scoped_threads_fs_at(backend, "tenant-host", "alice");
    let service = FilesystemSessionThreadService::new(scoped);
    let request = filesystem_prepared_request("delete-replay-fs", "delete-replay-fs-key-1");
    let first = service
        .accept_prepared_context(request.clone())
        .await
        .expect("first accept");
    assert!(!first.idempotent_replay);
    service
        .delete_thread(&request.scope, &first.thread_id)
        .await
        .expect("delete");

    let second = service
        .accept_prepared_context(request)
        .await
        .expect("re-accept after delete");
    assert!(
        !second.idempotent_replay,
        "a deleted prepared thread must re-mint, not replay its deleted record"
    );
}

/// Filesystem twin of the in-memory listing-exclusion pin, plus the cursor
/// invariant that only matters here: the page cursor advances over the
/// FETCHED index rows, so a page consisting entirely of hidden
/// prepared-context threads still makes progress instead of stalling.
#[tokio::test]
async fn filesystem_list_threads_excludes_prepared_context_threads_and_cursor_advances() {
    let backend = Arc::new(InMemoryBackend::new());
    let scoped = scoped_threads_fs_at(backend, "tenant-host", "alice");
    let service = FilesystemSessionThreadService::new(scoped);

    let mut listing_scope = None;
    for index in 0..3 {
        let request = filesystem_prepared_request("hidden-fs", &format!("hidden-fs-key-{index}"));
        listing_scope.get_or_insert(request.scope.clone());
        service
            .accept_prepared_context(request)
            .await
            .expect("prepared-context accept succeeds");
    }
    let listing_scope = listing_scope.expect("at least one prepared request");
    service
        .ensure_thread(EnsureThreadRequest {
            scope: listing_scope.clone(),
            thread_id: Some(ThreadId::new("t-visible-fs-001").unwrap()),
            created_by_actor_id: "actor-hidden-fs".into(),
            title: Some("visible".into()),
            metadata_json: None,
        })
        .await
        .unwrap();

    // Walk the listing with a page size smaller than the hidden population:
    // every page must either surface visible threads or advance the cursor.
    let mut cursor = None;
    let mut seen: Vec<String> = Vec::new();
    for _ in 0..8 {
        let view = service
            .list_threads_for_scope(ListThreadsForScopeRequest {
                scope: listing_scope.clone(),
                limit: Some(2),
                cursor: cursor.clone(),
            })
            .await
            .unwrap();
        seen.extend(
            view.threads
                .iter()
                .map(|record| record.thread_id.as_str().to_string()),
        );
        match view.next_cursor {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    assert_eq!(
        seen,
        ["t-visible-fs-001"],
        "only the conversation thread may surface; hidden pages must advance"
    );
}

/// Filesystem twin of the in-memory accept-door pins: mint + seed + journal
/// land durably, the accepted ref pins the last seeded row, and the
/// journaled declarations read back.
#[tokio::test]
async fn filesystem_accept_prepared_context_mints_seeds_and_journals() {
    let backend = Arc::new(InMemoryBackend::new());
    let scoped = scoped_threads_fs_at(backend, "tenant-host", "alice");
    let service = FilesystemSessionThreadService::new(scoped);
    let request = filesystem_prepared_request("unbound-fs", "unbound-fs-key-1");

    let accepted = service
        .accept_prepared_context(request.clone())
        .await
        .expect("prepared-context accept succeeds");
    assert!(!accepted.idempotent_replay);

    let history = service
        .list_thread_history(ThreadHistoryRequest {
            scope: request.scope.clone(),
            thread_id: accepted.thread_id.clone(),
        })
        .await
        .expect("seeded thread history");
    assert_eq!(history.thread.scope.owner_user_id, None);
    assert_eq!(history.messages.len(), 2);
    assert_eq!(history.messages[0].kind, MessageKind::System);
    assert_eq!(history.messages[0].status, MessageStatus::Finalized);
    assert_eq!(history.messages[1].kind, MessageKind::User);
    assert_eq!(history.messages[1].status, MessageStatus::Accepted);
    assert!(history.messages[0].sequence < history.messages[1].sequence);
    assert_eq!(
        accepted.accepted_message_ref.as_str(),
        format!("msg:{}", history.messages[1].message_id)
    );

    let record = service
        .read_prepared_context(&request.scope, &accepted.thread_id)
        .await
        .expect("declarations read-back")
        .expect("record present");
    assert_eq!(record.declarations, request.declarations);

    let window = service
        .load_context_window(LoadContextWindowRequest {
            scope: request.scope.clone(),
            thread_id: accepted.thread_id.clone(),
            max_messages: 16,
        })
        .await
        .expect("context window over seeded thread");
    assert_eq!(window.messages.len(), 2);
}

#[tokio::test]
async fn filesystem_read_prepared_context_rejects_a_corrupt_durable_output_contract() {
    let backend = Arc::new(InMemoryBackend::new());
    let scoped = scoped_threads_fs_at(backend, "tenant-host", "alice");
    let service = FilesystemSessionThreadService::new(Arc::clone(&scoped));
    let request = filesystem_prepared_request("unbound-fs-corrupt", "unbound-fs-key-corrupt");
    let accepted = service
        .accept_prepared_context(request.clone())
        .await
        .expect("prepared-context accept succeeds");
    let project_id = request.scope.project_id.as_ref().expect("project scope");
    let path = ScopedPath::new(format!(
        "/threads/agents/{}/projects/{}/threads/{}/prepared_context.json",
        request.scope.agent_id, project_id, accepted.thread_id
    ))
    .expect("prepared-context path");
    let mut stored: serde_json::Value = serde_json::from_slice(
        &scoped
            .get(&request.scope.to_resource_scope(), &path)
            .await
            .expect("prepared-context read")
            .expect("prepared-context entry")
            .entry
            .body,
    )
    .expect("prepared-context JSON");
    let mut overly_deep_schema = serde_json::json!({"type": "object"});
    for _ in 0..40 {
        overly_deep_schema = serde_json::json!({"properties": {"nested": overly_deep_schema}});
    }
    stored["declarations"]["output"]["schema"] = overly_deep_schema;
    scoped
        .put(
            &request.scope.to_resource_scope(),
            &path,
            Entry::bytes(serde_json::to_vec_pretty(&stored).expect("serialize corruption")),
            CasExpectation::Any,
        )
        .await
        .expect("write corrupt prepared context");

    let error = service
        .read_prepared_context(&request.scope, &accepted.thread_id)
        .await
        .expect_err("durable output contracts are revalidated on read");
    assert!(matches!(
        error,
        SessionThreadError::InvalidPreparedContext { .. }
    ));
}

/// Replay discipline on the durable backend: same request → same thread and
/// pin, no duplicate rows; cross-scope reads stay non-enumerating.
#[tokio::test]
async fn filesystem_accept_prepared_context_replays_without_orphans() {
    let backend = Arc::new(InMemoryBackend::new());
    let scoped = scoped_threads_fs_at(backend, "tenant-host", "alice");
    let service = FilesystemSessionThreadService::new(scoped);
    let request = filesystem_prepared_request("unbound-fs-replay", "unbound-fs-key-replay");

    let first = service
        .accept_prepared_context(request.clone())
        .await
        .expect("first accept");
    let replay = service
        .accept_prepared_context(request.clone())
        .await
        .expect("replay accept");
    assert!(replay.idempotent_replay);
    assert_eq!(replay.thread_id, first.thread_id);
    assert_eq!(replay.accepted_message_ref, first.accepted_message_ref);

    let history = service
        .list_thread_history(ThreadHistoryRequest {
            scope: request.scope.clone(),
            thread_id: first.thread_id.clone(),
        })
        .await
        .expect("history");
    assert_eq!(history.messages.len(), 2, "replay must not duplicate rows");

    let foreign_scope = ThreadScope {
        owner_user_id: None,
        ..scope("unbound-fs-foreign")
    };
    let missing = service
        .read_prepared_context(&foreign_scope, &first.thread_id)
        .await;
    assert!(
        matches!(missing, Err(SessionThreadError::UnknownThread { .. })),
        "cross-scope reads must stay non-enumerating, got {missing:?}"
    );
}
