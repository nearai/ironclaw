//! Contract tests for the typed subagent-result acceptance door.
//!
//! Every case runs against both production backends, and both are driven
//! behind `Arc<dyn SessionThreadService>` so the blanket `Arc<S>` forward sits
//! on the call path: a forgotten forwarding arm silently inherits the trait's
//! fail-closed default, which would leave the door shut in production without
//! a compile error.

use std::sync::Arc;

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
    AcceptSubagentResultRequest, EnsureThreadRequest, FilesystemSessionThreadService,
    InMemorySessionThreadService, LoadContextMessagesRequest, MessageContent, MessageKind,
    MessageStatus, SessionThreadError, SessionThreadService, ThreadHistoryRequest, ThreadScope,
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
        .accept_subagent_result(request)
        .await
        .expect_err("the orphan claim must not resurface as an accepted replay");
    assert!(
        matches!(&retry, SessionThreadError::UnknownThread { thread_id } if thread_id == &unknown),
        "expected UnknownThread on retry, got {retry:?}"
    );
}
