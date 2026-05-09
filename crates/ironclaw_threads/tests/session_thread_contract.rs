use futures::future::join_all;
use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId, UserId};
use ironclaw_threads::{
    AcceptInboundMessageRequest, AppendAssistantDraftRequest, CreateSummaryArtifactRequest,
    EnsureThreadRequest, InMemorySessionThreadService, LoadContextWindowRequest, MessageContent,
    MessageKind, MessageStatus, RedactMessageRequest, SessionThreadService, ThreadHistoryRequest,
    ThreadMessageId, ThreadScope, UpdateAssistantDraftRequest,
};

fn scope(label: &str) -> ThreadScope {
    ThreadScope {
        tenant_id: TenantId::new(format!("tenant-{label}")).unwrap(),
        agent_id: AgentId::new(format!("agent-{label}")).unwrap(),
        project_id: Some(ProjectId::new(format!("project-{label}")).unwrap()),
        owner_user_id: Some(UserId::new(format!("user-{label}")).unwrap()),
        mission_id: None,
    }
}

fn user_message(text: &str) -> MessageContent {
    MessageContent::text(text)
}

fn same_tenant_scope(agent_label: &str) -> ThreadScope {
    ThreadScope {
        tenant_id: TenantId::new("tenant-shared").unwrap(),
        agent_id: AgentId::new(format!("agent-{agent_label}")).unwrap(),
        project_id: Some(ProjectId::new(format!("project-{agent_label}")).unwrap()),
        owner_user_id: Some(UserId::new(format!("user-{agent_label}")).unwrap()),
        mission_id: None,
    }
}

#[tokio::test]
async fn creates_thread_without_channel_binding_and_assigns_monotonic_sequences_concurrently() {
    let service = InMemorySessionThreadService::default();
    let thread = service
        .ensure_thread(EnsureThreadRequest {
            scope: scope("a"),
            thread_id: Some(ThreadId::new("thread-a").unwrap()),
            created_by_actor_id: "actor-a".into(),
            title: Some("Canonical thread".into()),
            metadata_json: None,
        })
        .await
        .unwrap();

    let writes = (0..16).map(|index| {
        let service = service.clone();
        let thread_id = thread.thread_id.clone();
        async move {
            service
                .accept_inbound_message(AcceptInboundMessageRequest {
                    scope: scope("a"),
                    thread_id,
                    actor_id: "actor-a".into(),
                    source_binding_id: None,
                    reply_target_binding_id: None,
                    external_event_id: None,
                    content: user_message(&format!("message-{index}")),
                })
                .await
                .unwrap()
        }
    });

    join_all(writes).await;

    let history = service
        .list_thread_history(ThreadHistoryRequest {
            scope: scope("a"),
            thread_id: thread.thread_id,
        })
        .await
        .unwrap();

    let sequences = history
        .messages
        .iter()
        .map(|message| message.sequence)
        .collect::<Vec<_>>();
    assert_eq!(sequences, (1..=16).collect::<Vec<_>>());
    assert!(
        history
            .messages
            .iter()
            .all(|message| message.kind == MessageKind::User)
    );
}

#[tokio::test]
async fn duplicate_external_event_returns_same_message_without_duplicate_history_rows() {
    let service = InMemorySessionThreadService::default();
    let thread = service
        .ensure_thread(EnsureThreadRequest {
            scope: scope("a"),
            thread_id: None,
            created_by_actor_id: "actor-a".into(),
            title: None,
            metadata_json: None,
        })
        .await
        .unwrap();

    let first = service
        .accept_inbound_message(AcceptInboundMessageRequest {
            scope: scope("a"),
            thread_id: thread.thread_id.clone(),
            actor_id: "actor-a".into(),
            source_binding_id: Some("telegram-thread-1".into()),
            reply_target_binding_id: Some("telegram-thread-1".into()),
            external_event_id: Some("telegram-event-9".into()),
            content: user_message("hello once"),
        })
        .await
        .unwrap();
    let duplicate = service
        .accept_inbound_message(AcceptInboundMessageRequest {
            scope: scope("a"),
            thread_id: thread.thread_id.clone(),
            actor_id: "actor-a".into(),
            source_binding_id: Some("telegram-thread-1".into()),
            reply_target_binding_id: Some("telegram-thread-1".into()),
            external_event_id: Some("telegram-event-9".into()),
            content: user_message("retry payload is ignored"),
        })
        .await
        .unwrap();

    assert_eq!(first.message_id, duplicate.message_id);
    assert!(duplicate.idempotent_replay);

    let history = service
        .list_thread_history(ThreadHistoryRequest {
            scope: scope("a"),
            thread_id: thread.thread_id,
        })
        .await
        .unwrap();
    assert_eq!(history.messages.len(), 1);
    assert_eq!(history.messages[0].content.as_deref(), Some("hello once"));
}

#[tokio::test]
async fn duplicate_external_event_with_wrong_thread_does_not_replay_cross_thread_message() {
    let service = InMemorySessionThreadService::default();
    let first_thread = service
        .ensure_thread(EnsureThreadRequest {
            scope: scope("a"),
            thread_id: None,
            created_by_actor_id: "actor-a".into(),
            title: None,
            metadata_json: None,
        })
        .await
        .unwrap();
    let second_thread = service
        .ensure_thread(EnsureThreadRequest {
            scope: scope("a"),
            thread_id: None,
            created_by_actor_id: "actor-a".into(),
            title: None,
            metadata_json: None,
        })
        .await
        .unwrap();

    service
        .accept_inbound_message(AcceptInboundMessageRequest {
            scope: scope("a"),
            thread_id: first_thread.thread_id,
            actor_id: "actor-a".into(),
            source_binding_id: Some("telegram-thread-1".into()),
            reply_target_binding_id: Some("telegram-thread-1".into()),
            external_event_id: Some("telegram-event-9".into()),
            content: user_message("first thread only"),
        })
        .await
        .unwrap();

    let replay = service
        .accept_inbound_message(AcceptInboundMessageRequest {
            scope: scope("a"),
            thread_id: second_thread.thread_id,
            actor_id: "actor-a".into(),
            source_binding_id: Some("telegram-thread-1".into()),
            reply_target_binding_id: Some("telegram-thread-1".into()),
            external_event_id: Some("telegram-event-9".into()),
            content: user_message("must not leak first thread"),
        })
        .await;

    assert!(replay.is_err());
}

#[tokio::test]
async fn duplicate_external_event_is_scoped_to_full_thread_scope() {
    let service = InMemorySessionThreadService::default();
    let first_scope = same_tenant_scope("a");
    let second_scope = same_tenant_scope("b");
    let first_thread = service
        .ensure_thread(EnsureThreadRequest {
            scope: first_scope.clone(),
            thread_id: None,
            created_by_actor_id: "actor-a".into(),
            title: None,
            metadata_json: None,
        })
        .await
        .unwrap();
    let second_thread = service
        .ensure_thread(EnsureThreadRequest {
            scope: second_scope.clone(),
            thread_id: None,
            created_by_actor_id: "actor-b".into(),
            title: None,
            metadata_json: None,
        })
        .await
        .unwrap();

    let first = service
        .accept_inbound_message(AcceptInboundMessageRequest {
            scope: first_scope,
            thread_id: first_thread.thread_id.clone(),
            actor_id: "actor-a".into(),
            source_binding_id: Some("telegram-thread-1".into()),
            reply_target_binding_id: Some("telegram-thread-1".into()),
            external_event_id: Some("telegram-event-9".into()),
            content: user_message("first scope only"),
        })
        .await
        .unwrap();
    let second = service
        .accept_inbound_message(AcceptInboundMessageRequest {
            scope: second_scope.clone(),
            thread_id: second_thread.thread_id.clone(),
            actor_id: "actor-b".into(),
            source_binding_id: Some("telegram-thread-1".into()),
            reply_target_binding_id: Some("telegram-thread-1".into()),
            external_event_id: Some("telegram-event-9".into()),
            content: user_message("second scope is independent"),
        })
        .await
        .unwrap();

    assert_ne!(first.message_id, second.message_id);
    assert!(!second.idempotent_replay);
    let second_history = service
        .list_thread_history(ThreadHistoryRequest {
            scope: second_scope,
            thread_id: second_thread.thread_id,
        })
        .await
        .unwrap();
    assert_eq!(second_history.messages.len(), 1);
    assert_eq!(
        second_history.messages[0].content.as_deref(),
        Some("second scope is independent")
    );
}

#[tokio::test]
async fn busy_message_is_visible_deferred_and_not_tied_to_a_run() {
    let service = InMemorySessionThreadService::default();
    let thread = service
        .ensure_thread(EnsureThreadRequest {
            scope: scope("a"),
            thread_id: None,
            created_by_actor_id: "actor-a".into(),
            title: None,
            metadata_json: None,
        })
        .await
        .unwrap();
    let accepted = service
        .accept_inbound_message(AcceptInboundMessageRequest {
            scope: scope("a"),
            thread_id: thread.thread_id.clone(),
            actor_id: "actor-a".into(),
            source_binding_id: None,
            reply_target_binding_id: None,
            external_event_id: None,
            content: user_message("arrived while busy"),
        })
        .await
        .unwrap();

    service
        .mark_message_deferred_busy(&scope("a"), &thread.thread_id, accepted.message_id)
        .await
        .unwrap();

    let history = service
        .list_thread_history(ThreadHistoryRequest {
            scope: scope("a"),
            thread_id: thread.thread_id,
        })
        .await
        .unwrap();
    assert_eq!(history.messages[0].status, MessageStatus::DeferredBusy);
    assert!(history.messages[0].turn_run_id.is_none());
}

#[tokio::test]
async fn deferred_busy_rejects_non_user_and_non_accepted_messages() {
    let service = InMemorySessionThreadService::default();
    let thread = service
        .ensure_thread(EnsureThreadRequest {
            scope: scope("a"),
            thread_id: None,
            created_by_actor_id: "actor-a".into(),
            title: None,
            metadata_json: None,
        })
        .await
        .unwrap();
    let draft = service
        .append_assistant_draft(AppendAssistantDraftRequest {
            scope: scope("a"),
            thread_id: thread.thread_id.clone(),
            turn_run_id: "run-1".into(),
            content: MessageContent::text("partial"),
        })
        .await
        .unwrap();

    let result = service
        .mark_message_deferred_busy(&scope("a"), &thread.thread_id, draft.message_id)
        .await;

    assert!(result.is_err());
}

#[tokio::test]
async fn assistant_streaming_updates_one_draft_and_finalizes_one_canonical_message() {
    let service = InMemorySessionThreadService::default();
    let thread = service
        .ensure_thread(EnsureThreadRequest {
            scope: scope("a"),
            thread_id: None,
            created_by_actor_id: "actor-a".into(),
            title: None,
            metadata_json: None,
        })
        .await
        .unwrap();
    service
        .accept_inbound_message(AcceptInboundMessageRequest {
            scope: scope("a"),
            thread_id: thread.thread_id.clone(),
            actor_id: "actor-a".into(),
            source_binding_id: None,
            reply_target_binding_id: None,
            external_event_id: None,
            content: user_message("question"),
        })
        .await
        .unwrap();

    let draft = service
        .append_assistant_draft(AppendAssistantDraftRequest {
            scope: scope("a"),
            thread_id: thread.thread_id.clone(),
            turn_run_id: "run-1".into(),
            content: MessageContent::text("partial"),
        })
        .await
        .unwrap();
    service
        .update_assistant_draft(UpdateAssistantDraftRequest {
            scope: scope("a"),
            thread_id: thread.thread_id.clone(),
            message_id: draft.message_id,
            content: MessageContent::text("partial plus more"),
        })
        .await
        .unwrap();
    service
        .finalize_assistant_message(
            &scope("a"),
            &thread.thread_id,
            draft.message_id,
            MessageContent::text("final answer"),
        )
        .await
        .unwrap();

    let history = service
        .list_thread_history(ThreadHistoryRequest {
            scope: scope("a"),
            thread_id: thread.thread_id,
        })
        .await
        .unwrap();
    assert_eq!(history.messages.len(), 2);
    assert_eq!(history.messages[1].kind, MessageKind::Assistant);
    assert_eq!(history.messages[1].status, MessageStatus::Finalized);
    assert_eq!(history.messages[1].content.as_deref(), Some("final answer"));
}

#[tokio::test]
async fn redaction_preserves_sequence_but_model_context_hides_message_content() {
    let service = InMemorySessionThreadService::default();
    let thread = service
        .ensure_thread(EnsureThreadRequest {
            scope: scope("a"),
            thread_id: None,
            created_by_actor_id: "actor-a".into(),
            title: None,
            metadata_json: None,
        })
        .await
        .unwrap();
    let sensitive = service
        .accept_inbound_message(AcceptInboundMessageRequest {
            scope: scope("a"),
            thread_id: thread.thread_id.clone(),
            actor_id: "actor-a".into(),
            source_binding_id: None,
            reply_target_binding_id: None,
            external_event_id: None,
            content: user_message("secret token"),
        })
        .await
        .unwrap();
    service
        .accept_inbound_message(AcceptInboundMessageRequest {
            scope: scope("a"),
            thread_id: thread.thread_id.clone(),
            actor_id: "actor-a".into(),
            source_binding_id: None,
            reply_target_binding_id: None,
            external_event_id: None,
            content: user_message("safe follow-up"),
        })
        .await
        .unwrap();

    service
        .redact_message(RedactMessageRequest {
            scope: scope("a"),
            thread_id: thread.thread_id.clone(),
            message_id: sensitive.message_id,
            redaction_ref: "redaction/audit/1".into(),
        })
        .await
        .unwrap();

    let history = service
        .list_thread_history(ThreadHistoryRequest {
            scope: scope("a"),
            thread_id: thread.thread_id.clone(),
        })
        .await
        .unwrap();
    assert_eq!(history.messages[0].message_id, sensitive.message_id);
    assert_eq!(history.messages[0].sequence, 1);
    assert_eq!(history.messages[0].status, MessageStatus::Redacted);
    assert!(history.messages[0].content.is_none());
    assert_eq!(
        history.messages[0].redaction_ref.as_deref(),
        Some("redaction/audit/1")
    );

    let context = service
        .load_context_window(LoadContextWindowRequest {
            scope: scope("a"),
            thread_id: thread.thread_id,
            max_messages: 16,
        })
        .await
        .unwrap();
    assert_eq!(context.messages.len(), 1);
    assert_eq!(context.messages[0].content, "safe follow-up");
}

#[tokio::test]
async fn summaries_are_range_artifacts_and_policy_filtered_context_replacements() {
    let service = InMemorySessionThreadService::default();
    let thread = service
        .ensure_thread(EnsureThreadRequest {
            scope: scope("a"),
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
                scope: scope("a"),
                thread_id: thread.thread_id.clone(),
                actor_id: "actor-a".into(),
                source_binding_id: None,
                reply_target_binding_id: None,
                external_event_id: None,
                content: user_message(text),
            })
            .await
            .unwrap();
    }

    let summary = service
        .create_summary_artifact(CreateSummaryArtifactRequest {
            scope: scope("a"),
            thread_id: thread.thread_id.clone(),
            start_sequence: 1,
            end_sequence: 2,
            summary_kind: "model_context".into(),
            content: MessageContent::text("one and two summarized"),
            model_context_policy: Some("replace_range_when_selected".into()),
        })
        .await
        .unwrap();

    assert_eq!(summary.start_sequence, 1);
    assert_eq!(summary.end_sequence, 2);

    let history = service
        .list_thread_history(ThreadHistoryRequest {
            scope: scope("a"),
            thread_id: thread.thread_id.clone(),
        })
        .await
        .unwrap();
    assert_eq!(history.messages.len(), 3);
    assert_eq!(history.summary_artifacts.len(), 1);

    let context = service
        .load_context_window(LoadContextWindowRequest {
            scope: scope("a"),
            thread_id: thread.thread_id,
            max_messages: 16,
        })
        .await
        .unwrap();
    assert_eq!(context.messages.len(), 2);
    assert_eq!(context.messages[0].kind, MessageKind::Summary);
    assert_eq!(context.messages[0].content, "one and two summarized");
    assert_eq!(context.messages[1].content, "three");
}

#[tokio::test]
async fn summary_covering_redacted_message_is_not_loaded_into_model_context() {
    let service = InMemorySessionThreadService::default();
    let thread = service
        .ensure_thread(EnsureThreadRequest {
            scope: scope("a"),
            thread_id: None,
            created_by_actor_id: "actor-a".into(),
            title: None,
            metadata_json: None,
        })
        .await
        .unwrap();
    let sensitive = service
        .accept_inbound_message(AcceptInboundMessageRequest {
            scope: scope("a"),
            thread_id: thread.thread_id.clone(),
            actor_id: "actor-a".into(),
            source_binding_id: None,
            reply_target_binding_id: None,
            external_event_id: None,
            content: user_message("secret token"),
        })
        .await
        .unwrap();
    service
        .accept_inbound_message(AcceptInboundMessageRequest {
            scope: scope("a"),
            thread_id: thread.thread_id.clone(),
            actor_id: "actor-a".into(),
            source_binding_id: None,
            reply_target_binding_id: None,
            external_event_id: None,
            content: user_message("safe follow-up"),
        })
        .await
        .unwrap();
    service
        .create_summary_artifact(CreateSummaryArtifactRequest {
            scope: scope("a"),
            thread_id: thread.thread_id.clone(),
            start_sequence: 1,
            end_sequence: 2,
            summary_kind: "model_context".into(),
            content: MessageContent::text("summary mentions secret token"),
            model_context_policy: Some("replace_range_when_selected".into()),
        })
        .await
        .unwrap();
    service
        .redact_message(RedactMessageRequest {
            scope: scope("a"),
            thread_id: thread.thread_id.clone(),
            message_id: sensitive.message_id,
            redaction_ref: "redaction/audit/3".into(),
        })
        .await
        .unwrap();

    let history = service
        .list_thread_history(ThreadHistoryRequest {
            scope: scope("a"),
            thread_id: thread.thread_id.clone(),
        })
        .await
        .unwrap();
    assert_eq!(history.summary_artifacts.len(), 1);
    assert_eq!(history.summary_artifacts[0].content, "[redacted]");
    assert_ne!(
        history.summary_artifacts[0].content,
        "summary mentions secret token"
    );

    let context = service
        .load_context_window(LoadContextWindowRequest {
            scope: scope("a"),
            thread_id: thread.thread_id,
            max_messages: 16,
        })
        .await
        .unwrap();

    assert_eq!(context.messages.len(), 1);
    assert_eq!(context.messages[0].kind, MessageKind::User);
    assert_eq!(context.messages[0].content, "safe follow-up");
}

#[tokio::test]
async fn summary_covering_draft_message_is_not_loaded_into_model_context() {
    let service = InMemorySessionThreadService::default();
    let thread = service
        .ensure_thread(EnsureThreadRequest {
            scope: scope("a"),
            thread_id: None,
            created_by_actor_id: "actor-a".into(),
            title: None,
            metadata_json: None,
        })
        .await
        .unwrap();
    service
        .append_assistant_draft(AppendAssistantDraftRequest {
            scope: scope("a"),
            thread_id: thread.thread_id.clone(),
            turn_run_id: "run-1".into(),
            content: MessageContent::text("draft secret"),
        })
        .await
        .unwrap();
    service
        .accept_inbound_message(AcceptInboundMessageRequest {
            scope: scope("a"),
            thread_id: thread.thread_id.clone(),
            actor_id: "actor-a".into(),
            source_binding_id: None,
            reply_target_binding_id: None,
            external_event_id: None,
            content: user_message("visible user message"),
        })
        .await
        .unwrap();
    service
        .create_summary_artifact(CreateSummaryArtifactRequest {
            scope: scope("a"),
            thread_id: thread.thread_id.clone(),
            start_sequence: 1,
            end_sequence: 2,
            summary_kind: "model_context".into(),
            content: MessageContent::text("summary leaks draft secret"),
            model_context_policy: Some("replace_range_when_selected".into()),
        })
        .await
        .unwrap();

    let context = service
        .load_context_window(LoadContextWindowRequest {
            scope: scope("a"),
            thread_id: thread.thread_id,
            max_messages: 16,
        })
        .await
        .unwrap();

    assert_eq!(context.messages.len(), 1);
    assert_eq!(context.messages[0].kind, MessageKind::User);
    assert_eq!(context.messages[0].content, "visible user message");
}

#[tokio::test]
async fn duplicate_assistant_draft_for_same_turn_run_is_idempotent() {
    let service = InMemorySessionThreadService::default();
    let thread = service
        .ensure_thread(EnsureThreadRequest {
            scope: scope("a"),
            thread_id: None,
            created_by_actor_id: "actor-a".into(),
            title: None,
            metadata_json: None,
        })
        .await
        .unwrap();

    let first = service
        .append_assistant_draft(AppendAssistantDraftRequest {
            scope: scope("a"),
            thread_id: thread.thread_id.clone(),
            turn_run_id: "run-1".into(),
            content: MessageContent::text("partial"),
        })
        .await
        .unwrap();
    let duplicate = service
        .append_assistant_draft(AppendAssistantDraftRequest {
            scope: scope("a"),
            thread_id: thread.thread_id.clone(),
            turn_run_id: "run-1".into(),
            content: MessageContent::text("retry partial ignored"),
        })
        .await
        .unwrap();

    assert_eq!(first.message_id, duplicate.message_id);
    assert_eq!(duplicate.content.as_deref(), Some("partial"));

    service
        .finalize_assistant_message(
            &scope("a"),
            &thread.thread_id,
            first.message_id,
            MessageContent::text("final answer"),
        )
        .await
        .unwrap();
    let after_final = service
        .append_assistant_draft(AppendAssistantDraftRequest {
            scope: scope("a"),
            thread_id: thread.thread_id.clone(),
            turn_run_id: "run-1".into(),
            content: MessageContent::text("retry after final ignored"),
        })
        .await
        .unwrap();

    assert_eq!(first.message_id, after_final.message_id);

    service
        .redact_message(RedactMessageRequest {
            scope: scope("a"),
            thread_id: thread.thread_id.clone(),
            message_id: first.message_id,
            redaction_ref: "redaction/audit/assistant".into(),
        })
        .await
        .unwrap();
    let after_redaction = service
        .append_assistant_draft(AppendAssistantDraftRequest {
            scope: scope("a"),
            thread_id: thread.thread_id.clone(),
            turn_run_id: "run-1".into(),
            content: MessageContent::text("retry after redaction must not create a new row"),
        })
        .await
        .unwrap();
    assert_eq!(first.message_id, after_redaction.message_id);
    assert_eq!(after_redaction.status, MessageStatus::Redacted);
    assert!(after_redaction.content.is_none());

    let history = service
        .list_thread_history(ThreadHistoryRequest {
            scope: scope("a"),
            thread_id: thread.thread_id,
        })
        .await
        .unwrap();
    assert_eq!(history.messages.len(), 1);
    assert_eq!(history.messages[0].status, MessageStatus::Redacted);
}

#[tokio::test]
async fn overlapping_replacement_summaries_are_rejected() {
    let service = InMemorySessionThreadService::default();
    let thread = service
        .ensure_thread(EnsureThreadRequest {
            scope: scope("a"),
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
                scope: scope("a"),
                thread_id: thread.thread_id.clone(),
                actor_id: "actor-a".into(),
                source_binding_id: None,
                reply_target_binding_id: None,
                external_event_id: None,
                content: user_message(text),
            })
            .await
            .unwrap();
    }
    service
        .create_summary_artifact(CreateSummaryArtifactRequest {
            scope: scope("a"),
            thread_id: thread.thread_id.clone(),
            start_sequence: 1,
            end_sequence: 2,
            summary_kind: "model_context".into(),
            content: MessageContent::text("one and two summarized"),
            model_context_policy: Some("replace_range_when_selected".into()),
        })
        .await
        .unwrap();

    let overlapping = service
        .create_summary_artifact(CreateSummaryArtifactRequest {
            scope: scope("a"),
            thread_id: thread.thread_id,
            start_sequence: 2,
            end_sequence: 3,
            summary_kind: "model_context".into(),
            content: MessageContent::text("two and three summarized"),
            model_context_policy: Some("replace_range_when_selected".into()),
        })
        .await;

    assert!(overlapping.is_err());
}

#[tokio::test]
async fn summary_replacement_still_applies_when_range_starts_with_redacted_message() {
    let service = InMemorySessionThreadService::default();
    let thread = service
        .ensure_thread(EnsureThreadRequest {
            scope: scope("a"),
            thread_id: None,
            created_by_actor_id: "actor-a".into(),
            title: None,
            metadata_json: None,
        })
        .await
        .unwrap();
    let sensitive = service
        .accept_inbound_message(AcceptInboundMessageRequest {
            scope: scope("a"),
            thread_id: thread.thread_id.clone(),
            actor_id: "actor-a".into(),
            source_binding_id: None,
            reply_target_binding_id: None,
            external_event_id: None,
            content: user_message("secret token"),
        })
        .await
        .unwrap();
    service
        .accept_inbound_message(AcceptInboundMessageRequest {
            scope: scope("a"),
            thread_id: thread.thread_id.clone(),
            actor_id: "actor-a".into(),
            source_binding_id: None,
            reply_target_binding_id: None,
            external_event_id: None,
            content: user_message("context that should be summarized"),
        })
        .await
        .unwrap();
    service
        .redact_message(RedactMessageRequest {
            scope: scope("a"),
            thread_id: thread.thread_id.clone(),
            message_id: sensitive.message_id,
            redaction_ref: "redaction/audit/2".into(),
        })
        .await
        .unwrap();
    service
        .create_summary_artifact(CreateSummaryArtifactRequest {
            scope: scope("a"),
            thread_id: thread.thread_id.clone(),
            start_sequence: 1,
            end_sequence: 2,
            summary_kind: "model_context".into(),
            content: MessageContent::text("redacted range summary"),
            model_context_policy: Some("replace_range_when_selected".into()),
        })
        .await
        .unwrap();

    let context = service
        .load_context_window(LoadContextWindowRequest {
            scope: scope("a"),
            thread_id: thread.thread_id,
            max_messages: 16,
        })
        .await
        .unwrap();

    assert_eq!(context.messages.len(), 1);
    assert_eq!(context.messages[0].kind, MessageKind::User);
    assert_eq!(
        context.messages[0].content,
        "context that should be summarized"
    );
}

#[tokio::test]
async fn wrong_scope_lookup_returns_not_found_instead_of_cross_tenant_history() {
    let service = InMemorySessionThreadService::default();
    let thread = service
        .ensure_thread(EnsureThreadRequest {
            scope: scope("a"),
            thread_id: None,
            created_by_actor_id: "actor-a".into(),
            title: None,
            metadata_json: None,
        })
        .await
        .unwrap();

    let result = service
        .list_thread_history(ThreadHistoryRequest {
            scope: scope("b"),
            thread_id: thread.thread_id,
        })
        .await;

    assert!(result.is_err());
}

#[test]
fn message_ids_are_stable_values() {
    let id = ThreadMessageId::new();
    assert_eq!(ThreadMessageId::parse(&id.to_string()).unwrap(), id);
}
