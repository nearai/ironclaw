//! Contract tests for the typed subagent-result acceptance door.
//!
//! Every case runs against both production backends, and both are driven
//! behind `Arc<dyn SessionThreadService>` so the blanket `Arc<S>` forward sits
//! on the call path: a forgotten forwarding arm silently inherits the trait's
//! fail-closed default, which would leave the door shut in production without
//! a compile error.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_filesystem::{
    Fault, FaultInjecting, FaultKind, FilesystemOperation, InMemoryBackend, RootFilesystem,
    ScopedFilesystem,
};
use ironclaw_host_api::{
    ids::{AgentId, ProjectId, TenantId, ThreadId, UserId},
    mount::{MountGrant, MountPermissions, MountView},
    path::{MountAlias, VirtualPath},
};
use ironclaw_threads::{
    AcceptInboundMessageRequest, AcceptSubagentResultRequest, AcceptedInboundMessage,
    AcceptedInboundMessageReplay, AppendAssistantDraftRequest,
    AppendCapabilityDisplayPreviewRequest, AppendToolResultReferenceRequest, ContextMessages,
    ContextWindow, CreateSummaryArtifactRequest, EnsureThreadRequest,
    FilesystemSessionThreadService, InMemorySessionThreadService, LoadContextMessagesRequest,
    LoadContextWindowRequest, MessageContent, MessageKind, MessageStatus, RedactMessageRequest,
    ReplayAcceptedInboundMessageRequest, SessionThreadError, SessionThreadRecord,
    SessionThreadService, SummaryArtifact, ThreadHistory, ThreadHistoryRequest, ThreadMessageId,
    ThreadMessageRecord, ThreadScope, UpdateAssistantDraftRequest,
    UpdateToolResultReferenceRequest,
};

fn scope() -> ThreadScope {
    ThreadScope {
        tenant_id: TenantId::new("tenant").expect("tenant"),
        agent_id: AgentId::new("agent").expect("agent"),
        project_id: Some(ProjectId::new("project").expect("project")),
        owner_user_id: Some(UserId::new("user").expect("user")),
        mission_id: None,
    }
}

fn scoped_threads_fs<F>(backend: Arc<F>) -> Arc<ScopedFilesystem<F>>
where
    F: RootFilesystem,
{
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/threads").expect("alias"),
        VirtualPath::new("/tenants/tenant/users/user/threads").expect("virtual path"),
        MountPermissions::read_write_list_delete(),
    )])
    .expect("mounts");
    Arc::new(ScopedFilesystem::with_fixed_view(backend, mounts))
}

fn in_memory_service() -> Arc<dyn SessionThreadService> {
    Arc::new(InMemorySessionThreadService::default())
}

fn filesystem_service() -> Arc<dyn SessionThreadService> {
    Arc::new(FilesystemSessionThreadService::new(scoped_threads_fs(
        Arc::new(InMemoryBackend::new()),
    )))
}

async fn ensure_thread(service: &Arc<dyn SessionThreadService>) -> ThreadId {
    service
        .ensure_thread(EnsureThreadRequest {
            scope: scope(),
            thread_id: Some(ThreadId::new("thread-parent").expect("thread id")),
            created_by_actor_id: "actor-parent".to_string(),
            title: None,
            metadata_json: None,
        })
        .await
        .expect("thread")
        .thread_id
}

fn result_request(
    thread: &ThreadId,
    child_run_id: &str,
    text: &str,
) -> AcceptSubagentResultRequest {
    AcceptSubagentResultRequest {
        scope: scope(),
        thread_id: thread.clone(),
        source_binding_id: "subagent-result:parent-1".to_string(),
        external_event_id: child_run_id.to_string(),
        content: MessageContent::text(text),
    }
}

async fn subagent_result_is_accepted_as_a_system_row(service: Arc<dyn SessionThreadService>) {
    let thread = ensure_thread(&service).await;

    let accepted = service
        .accept_subagent_result(result_request(&thread, "child-1", "framed child output"))
        .await
        .expect("acceptance succeeds");
    assert!(!accepted.idempotent_replay);

    let row = service
        .read_thread_message(&scope(), &thread, accepted.message_id)
        .await
        .expect("read succeeds")
        .expect("row exists");
    assert_eq!(row.kind, MessageKind::System, "never MessageKind::User");
    assert_eq!(row.status, MessageStatus::Finalized);
    assert_eq!(row.sequence, accepted.sequence);
    assert_eq!(row.content.as_deref(), Some("framed child output"));
    assert_eq!(
        row.actor_id, None,
        "a child's output has no human actor on the thread"
    );
    assert_eq!(
        row.source_binding_id.as_deref(),
        Some("subagent-result:parent-1"),
        "the accepted row carries its acceptance binding"
    );

    // Durable is not enough: a system row whose status or kind the context
    // projection drops would leave the parent unable to see its child's work
    // while every write-side assertion above still passed.
    let context = service
        .load_context_messages(LoadContextMessagesRequest {
            scope: scope(),
            thread_id: thread,
            message_ids: vec![accepted.message_id],
        })
        .await
        .expect("context load succeeds");
    assert_eq!(
        context.messages.len(),
        1,
        "the accepted row must survive into model context: {context:?}"
    );
    assert_eq!(context.messages[0].kind, MessageKind::System);
    assert_eq!(context.messages[0].content, "framed child output");
}

async fn replaying_a_subagent_result_returns_the_same_row(service: Arc<dyn SessionThreadService>) {
    let thread = ensure_thread(&service).await;
    let request = result_request(&thread, "child-1", "framed child output");

    let first = service
        .accept_subagent_result(request.clone())
        .await
        .expect("first acceptance");
    let replay = service
        .accept_subagent_result(request)
        .await
        .expect("replay acceptance");

    assert!(replay.idempotent_replay, "replay must be flagged");
    assert_eq!(replay.message_id, first.message_id, "same durable row");
    assert_eq!(replay.sequence, first.sequence);

    // The identity of the returned ref is only half the promise: the thread
    // must hold exactly one durable row for this child, not two rows of which
    // the first is echoed back.
    let history = service
        .list_thread_history(ThreadHistoryRequest {
            scope: scope(),
            thread_id: thread,
        })
        .await
        .expect("history");
    let rows: Vec<_> = history
        .messages
        .iter()
        .filter(|message| message.kind == MessageKind::System)
        .collect();
    assert_eq!(
        rows.len(),
        1,
        "replay must not append a second row: {rows:?}"
    );
    assert_eq!(rows[0].message_id, first.message_id);
}

async fn distinct_children_get_distinct_rows(service: Arc<dyn SessionThreadService>) {
    let thread = ensure_thread(&service).await;
    let mut ids = Vec::new();
    for child in ["child-1", "child-2"] {
        let accepted = service
            .accept_subagent_result(result_request(&thread, child, child))
            .await
            .expect("acceptance succeeds");
        ids.push(accepted.message_id);
    }
    assert_ne!(ids[0], ids[1], "one row per child (D6)");
}

#[tokio::test]
async fn in_memory_subagent_result_is_accepted_as_a_system_row() {
    subagent_result_is_accepted_as_a_system_row(in_memory_service()).await;
}

#[tokio::test]
async fn filesystem_subagent_result_is_accepted_as_a_system_row() {
    subagent_result_is_accepted_as_a_system_row(filesystem_service()).await;
}

#[tokio::test]
async fn in_memory_replaying_a_subagent_result_returns_the_same_row() {
    replaying_a_subagent_result_returns_the_same_row(in_memory_service()).await;
}

#[tokio::test]
async fn filesystem_replaying_a_subagent_result_returns_the_same_row() {
    replaying_a_subagent_result_returns_the_same_row(filesystem_service()).await;
}

#[tokio::test]
async fn in_memory_distinct_children_get_distinct_rows() {
    distinct_children_get_distinct_rows(in_memory_service()).await;
}

#[tokio::test]
async fn filesystem_distinct_children_get_distinct_rows() {
    distinct_children_get_distinct_rows(filesystem_service()).await;
}

/// The hard crash window that only exists on a backend without transactions:
/// the durable idempotency claim landed, the transcript row did not. A retry
/// must resume the claim — same message id, one row — not mint an orphan or
/// fail closed forever. Mirrors
/// `filesystem_fallback_resumes_intent_with_original_model_after_message_failure`
/// in the inbound suite: force the fallback protocol by refusing the first
/// `BeginTxn`, then fail the row write it performs.
#[tokio::test]
async fn filesystem_subagent_result_resumes_a_claim_whose_row_never_landed() {
    let backend = Arc::new(FaultInjecting::new(InMemoryBackend::new()));
    let service: Arc<dyn SessionThreadService> = Arc::new(FilesystemSessionThreadService::new(
        scoped_threads_fs(Arc::clone(&backend)),
    ));
    let thread = ensure_thread(&service).await;
    let request = result_request(&thread, "child-1", "framed child output");

    backend.add_fault(
        Fault::on(FilesystemOperation::BeginTxn)
            .nth(1)
            .returning(FaultKind::Unsupported),
    );
    backend.add_fault(
        Fault::on(FilesystemOperation::WriteFile)
            .path("/messages/")
            .nth(1)
            .backend("crash between the idempotency claim and the transcript row"),
    );

    service
        .accept_subagent_result(request.clone())
        .await
        .expect_err("the injected row write must fail the first acceptance");

    let resumed = service
        .accept_subagent_result(request.clone())
        .await
        .expect("the retry resumes the claim");
    assert!(
        resumed.idempotent_replay,
        "the retry must resume the durable claim, not mint a fresh identity"
    );
    let replay = service
        .accept_subagent_result(request)
        .await
        .expect("a further replay is a plain replay");
    assert_eq!(
        replay.message_id, resumed.message_id,
        "the resumed claim is the durable row"
    );
    assert!(replay.idempotent_replay);

    let history = service
        .list_thread_history(ThreadHistoryRequest {
            scope: scope(),
            thread_id: thread,
        })
        .await
        .expect("history");
    let rows: Vec<_> = history
        .messages
        .iter()
        .filter(|message| message.kind == MessageKind::System)
        .collect();
    assert_eq!(
        rows.len(),
        1,
        "the crash must not duplicate the row: {rows:?}"
    );
    assert_eq!(rows[0].message_id, resumed.message_id);
}

/// A child's result may only land in a thread that already exists under the
/// caller's scope. The door must not conjure a thread, and — because it claims
/// the acceptance identity before the row on non-transactional backends — a
/// rejected acceptance must stay rejected rather than burning the identity and
/// reporting a replay on the retry.
async fn subagent_result_into_an_unknown_thread_fails_closed(
    service: Arc<dyn SessionThreadService>,
) {
    let unknown = ThreadId::new("thread-never-created").expect("thread id");
    let request = result_request(&unknown, "child-1", "framed child output");

    let error = service
        .accept_subagent_result(request.clone())
        .await
        .expect_err("an unknown thread must not accept a subagent result");
    assert!(
        matches!(&error, SessionThreadError::UnknownThread { thread_id } if thread_id == &unknown),
        "expected UnknownThread, got {error:?}"
    );

    let retry = service
        .accept_subagent_result(request)
        .await
        .expect_err("the retry must still be rejected, not reported as a replay");
    assert!(
        matches!(&retry, SessionThreadError::UnknownThread { thread_id } if thread_id == &unknown),
        "expected UnknownThread on retry, got {retry:?}"
    );
}

#[tokio::test]
async fn in_memory_subagent_result_into_an_unknown_thread_fails_closed() {
    subagent_result_into_an_unknown_thread_fails_closed(in_memory_service()).await;
}

#[tokio::test]
async fn filesystem_subagent_result_into_an_unknown_thread_fails_closed() {
    subagent_result_into_an_unknown_thread_fails_closed(filesystem_service()).await;
}

/// The same fail-closed promise on the backend whose window actually exists.
/// The two cases above both commit (or reject) the identity claim and the row
/// together, so neither reaches the orphan-claim state the doc comment above
/// describes. Refusing `BeginTxn` forces the two-phase fallback: the claim
/// lands, `reserve_sequence` then rejects the unknown thread, and a durable
/// claim is left behind pointing at a row that will never exist. The retry
/// must still be rejected — an orphan claim must never be mistaken for a
/// committed row and replayed back as an accepted result.
#[tokio::test]
async fn filesystem_fallback_unknown_thread_claim_is_not_replayed_as_accepted() {
    let backend = Arc::new(FaultInjecting::new(InMemoryBackend::new()));
    let service: Arc<dyn SessionThreadService> = Arc::new(FilesystemSessionThreadService::new(
        scoped_threads_fs(Arc::clone(&backend)),
    ));
    let unknown = ThreadId::new("thread-never-created").expect("thread id");
    let request = result_request(&unknown, "child-1", "framed child output");

    backend.add_fault(
        Fault::on(FilesystemOperation::BeginTxn)
            .nth(1)
            .returning(FaultKind::Unsupported),
    );

    let error = service
        .accept_subagent_result(request.clone())
        .await
        .expect_err("an unknown thread must not accept a subagent result");
    assert!(
        matches!(&error, SessionThreadError::UnknownThread { thread_id } if thread_id == &unknown),
        "expected UnknownThread, got {error:?}"
    );

    let retry = service
        .accept_subagent_result(request.clone())
        .await
        .expect_err("the orphan claim must not resurface as an accepted replay");
    assert!(
        matches!(&retry, SessionThreadError::UnknownThread { thread_id } if thread_id == &unknown),
        "expected UnknownThread on retry, got {retry:?}"
    );

    // The sharp end of the same promise, and the only assertion here that a
    // "an orphan claim counts as a committed row" bug cannot slip past: once
    // the parent thread does exist, the burned identity must HEAL into the row
    // it always intended — not report `idempotent_replay` over an empty
    // thread, which is what claiming-without-writing looks like if the claim
    // is ever read as an answer.
    service
        .ensure_thread(EnsureThreadRequest {
            scope: scope(),
            thread_id: Some(unknown.clone()),
            created_by_actor_id: "actor-parent".to_string(),
            title: None,
            metadata_json: None,
        })
        .await
        .expect("thread");
    let healed = service
        .accept_subagent_result(request)
        .await
        .expect("the claim resumes once its thread exists");
    let history = service
        .list_thread_history(ThreadHistoryRequest {
            scope: scope(),
            thread_id: unknown,
        })
        .await
        .expect("history");
    let rows: Vec<_> = history
        .messages
        .iter()
        .filter(|message| message.kind == MessageKind::System)
        .collect();
    assert_eq!(
        rows.len(),
        1,
        "the healed claim must be one real row, not a reported replay of nothing: {rows:?}"
    );
    assert_eq!(rows[0].message_id, healed.message_id);
}

// ── Identity halves must carry a value ────────────────────────────────────

async fn an_empty_identity_half_is_refused(service: Arc<dyn SessionThreadService>) {
    let thread = ensure_thread(&service).await;

    for (source_binding_id, external_event_id, field) in [
        ("", "child-1", "source_binding_id"),
        ("subagent-result:parent-1", "", "external_event_id"),
        ("   ", "child-1", "source_binding_id"),
        ("subagent-result:parent-1", "\t", "external_event_id"),
    ] {
        let error = service
            .accept_subagent_result(AcceptSubagentResultRequest {
                scope: scope(),
                thread_id: thread.clone(),
                source_binding_id: source_binding_id.to_string(),
                external_event_id: external_event_id.to_string(),
                content: MessageContent::text("framed child output"),
            })
            .await
            .expect_err("an empty identity half must not be hashed into the dedupe index");
        assert!(
            matches!(
                &error,
                SessionThreadError::InvalidSubagentResult { reason } if reason.contains(field)
            ),
            "expected InvalidSubagentResult naming {field}, got {error:?}"
        );
    }

    // The rejection is total: nothing was appended, so no later child can be
    // mistaken for a replay of a row that a blank identity smuggled in.
    let history = service
        .list_thread_history(ThreadHistoryRequest {
            scope: scope(),
            thread_id: thread,
        })
        .await
        .expect("history");
    assert!(
        history.messages.is_empty(),
        "a refused acceptance must append nothing: {:?}",
        history.messages
    );
}

#[tokio::test]
async fn in_memory_an_empty_identity_half_is_refused() {
    an_empty_identity_half_is_refused(in_memory_service()).await;
}

#[tokio::test]
async fn filesystem_an_empty_identity_half_is_refused() {
    an_empty_identity_half_is_refused(filesystem_service()).await;
}

// ── The trait's fail-closed default ───────────────────────────────────────

/// A backend that implements every method `SessionThreadService` requires and
/// overrides nothing else — the exact shape of the 11 test doubles this slice
/// deliberately left at zero diff. It exists to pin one thing: a backend that
/// has not implemented the new door must REFUSE, not quietly succeed or return
/// an empty answer that a caller would read as "the child's result landed".
struct BackendWithoutTheDoor;

#[async_trait]
impl SessionThreadService for BackendWithoutTheDoor {
    async fn ensure_thread(
        &self,
        _request: EnsureThreadRequest,
    ) -> Result<SessionThreadRecord, SessionThreadError> {
        unimplemented!("ensure_thread is not part of this test")
    }

    async fn accept_inbound_message(
        &self,
        _request: AcceptInboundMessageRequest,
    ) -> Result<AcceptedInboundMessage, SessionThreadError> {
        unimplemented!("accept_inbound_message is not part of this test")
    }

    async fn replay_accepted_inbound_message(
        &self,
        _request: ReplayAcceptedInboundMessageRequest,
    ) -> Result<Option<AcceptedInboundMessageReplay>, SessionThreadError> {
        unimplemented!("replay_accepted_inbound_message is not part of this test")
    }

    async fn mark_message_submitted(
        &self,
        _scope: &ThreadScope,
        _thread_id: &ThreadId,
        _message_id: ThreadMessageId,
        _turn_id: String,
        _turn_run_id: String,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        unimplemented!("mark_message_submitted is not part of this test")
    }

    async fn mark_message_rejected_busy(
        &self,
        _scope: &ThreadScope,
        _thread_id: &ThreadId,
        _message_id: ThreadMessageId,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        unimplemented!("mark_message_rejected_busy is not part of this test")
    }

    async fn append_assistant_draft(
        &self,
        _request: AppendAssistantDraftRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        unimplemented!("append_assistant_draft is not part of this test")
    }

    async fn append_tool_result_reference(
        &self,
        _request: AppendToolResultReferenceRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        unimplemented!("append_tool_result_reference is not part of this test")
    }

    async fn append_capability_display_preview(
        &self,
        _request: AppendCapabilityDisplayPreviewRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        unimplemented!("append_capability_display_preview is not part of this test")
    }

    async fn update_tool_result_reference(
        &self,
        _request: UpdateToolResultReferenceRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        unimplemented!("update_tool_result_reference is not part of this test")
    }

    async fn update_assistant_draft(
        &self,
        _request: UpdateAssistantDraftRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        unimplemented!("update_assistant_draft is not part of this test")
    }

    async fn finalize_assistant_message(
        &self,
        _scope: &ThreadScope,
        _thread_id: &ThreadId,
        _message_id: ThreadMessageId,
        _content: MessageContent,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        unimplemented!("finalize_assistant_message is not part of this test")
    }

    async fn redact_message(
        &self,
        _request: RedactMessageRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        unimplemented!("redact_message is not part of this test")
    }

    async fn load_context_window(
        &self,
        _request: LoadContextWindowRequest,
    ) -> Result<ContextWindow, SessionThreadError> {
        unimplemented!("load_context_window is not part of this test")
    }

    async fn load_context_messages(
        &self,
        _request: LoadContextMessagesRequest,
    ) -> Result<ContextMessages, SessionThreadError> {
        unimplemented!("load_context_messages is not part of this test")
    }

    async fn list_thread_history(
        &self,
        _request: ThreadHistoryRequest,
    ) -> Result<ThreadHistory, SessionThreadError> {
        unimplemented!("list_thread_history is not part of this test")
    }

    async fn create_summary_artifact(
        &self,
        _request: CreateSummaryArtifactRequest,
    ) -> Result<SummaryArtifact, SessionThreadError> {
        unimplemented!("create_summary_artifact is not part of this test")
    }
}

#[tokio::test]
async fn a_backend_without_the_door_fails_closed() {
    let service: Arc<dyn SessionThreadService> = Arc::new(BackendWithoutTheDoor);
    let thread = ThreadId::new("thread-parent").expect("thread id");

    let error = service
        .accept_subagent_result(result_request(&thread, "child-1", "framed child output"))
        .await
        .expect_err("a backend that never implemented the door must refuse");
    assert!(
        matches!(
            &error,
            SessionThreadError::Backend(reason)
                if reason == "accept_subagent_result is not implemented by this \
                              SessionThreadService backend"
        ),
        "expected the fail-closed default's Backend error, got {error:?}"
    );
}
