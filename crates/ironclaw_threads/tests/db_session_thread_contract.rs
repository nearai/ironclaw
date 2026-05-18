#![cfg(any(feature = "libsql", feature = "postgres"))]

use futures::future::join_all;
use ironclaw_host_api::{AgentId, CapabilityId, ProjectId, TenantId, ThreadId, UserId};
use ironclaw_threads::{
    AcceptInboundMessageRequest, AppendAssistantDraftRequest, AppendToolResultReferenceRequest,
    CreateSummaryArtifactRequest, EnsureThreadRequest, LoadContextMessagesRequest,
    LoadContextWindowRequest, MessageContent, MessageKind, MessageStatus,
    ProviderToolCallReferenceEnvelope, RedactMessageRequest, SessionThreadService,
    ThreadHistoryRequest, ThreadScope, ToolResultReferenceEnvelope, ToolResultSafeSummary,
    UpdateAssistantDraftRequest,
};

#[cfg(feature = "libsql")]
use ironclaw_threads::LibSqlSessionThreadService;
#[cfg(feature = "postgres")]
use ironclaw_threads::PostgresSessionThreadService;
#[cfg(feature = "libsql")]
use std::sync::Arc;

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_persists_thread_history_and_context_across_reopen() {
    let (db_path, _dir) = libsql_db_path();
    let db = Arc::new(
        libsql::Builder::new_local(db_path.clone())
            .build()
            .await
            .unwrap(),
    );
    let service = LibSqlSessionThreadService::new(Arc::clone(&db));
    service.run_migrations().await.unwrap();
    let thread_id = durable_history_flow(&service, "libsql-persist").await;

    drop(service);
    drop(db);

    let reopened_db = Arc::new(libsql::Builder::new_local(db_path).build().await.unwrap());
    let reopened = LibSqlSessionThreadService::new(reopened_db);
    assert_reopened_history(&reopened, "libsql-persist", thread_id).await;
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_concurrent_writes_keep_sequences_unique_and_dedupe_external_events() {
    let (db_path, _dir) = libsql_db_path();
    let db = Arc::new(libsql::Builder::new_local(db_path).build().await.unwrap());
    let service = LibSqlSessionThreadService::new(Arc::clone(&db));
    service.run_migrations().await.unwrap();
    libsql_durable_concurrency_flow(db, "libsql-concurrent").await;
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_accepts_long_external_event_ids_without_index_key_bloat() {
    let (db_path, _dir) = libsql_db_path();
    let db = Arc::new(libsql::Builder::new_local(db_path).build().await.unwrap());
    let service = LibSqlSessionThreadService::new(db);
    service.run_migrations().await.unwrap();
    durable_long_external_event_flow(&service, "libsql-long-id").await;
}

#[cfg(feature = "postgres")]
#[tokio::test]
async fn postgres_migrations_are_serialized_when_called_concurrently() {
    let Some(pool) = postgres_pool().await else {
        return;
    };
    let stores = (0..8)
        .map(|_| PostgresSessionThreadService::new(pool.clone()))
        .collect::<Vec<_>>();

    let results = join_all(stores.iter().map(|store| store.run_migrations())).await;

    for result in results {
        result.unwrap();
    }
}

#[cfg(feature = "postgres")]
#[tokio::test]
async fn postgres_persists_thread_history_and_context_across_instances_when_configured() {
    let Some(pool) = postgres_pool().await else {
        return;
    };
    let service = PostgresSessionThreadService::new(pool.clone());
    service.run_migrations().await.unwrap();
    let label = format!("pg-persist-{}", unique_suffix());
    let thread_id = durable_history_flow(&service, &label).await;

    let reopened = PostgresSessionThreadService::new(pool);
    assert_reopened_history(&reopened, &label, thread_id).await;
}

#[cfg(feature = "postgres")]
#[tokio::test]
async fn postgres_concurrent_writes_keep_sequences_unique_and_dedupe_external_events_when_configured()
 {
    let Some(pool) = postgres_pool().await else {
        return;
    };
    let service = PostgresSessionThreadService::new(pool.clone());
    service.run_migrations().await.unwrap();
    let label = format!("pg-concurrent-{}", unique_suffix());
    postgres_durable_concurrency_flow(pool, &label).await;
}

#[cfg(feature = "postgres")]
#[tokio::test]
async fn postgres_accepts_long_external_event_ids_without_index_key_bloat_when_configured() {
    let Some(pool) = postgres_pool().await else {
        return;
    };
    let service = PostgresSessionThreadService::new(pool);
    service.run_migrations().await.unwrap();
    let label = format!("pg-long-id-{}", unique_suffix());
    durable_long_external_event_flow(&service, &label).await;
}

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
            summary_kind: "model_context".into(),
            content: MessageContent::text("summary that mentions secret token"),
            model_context_policy: Some("replace_range_when_selected".into()),
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

    let provider_call = ProviderToolCallReferenceEnvelope {
        provider_id: "test-provider".to_string(),
        provider_model_id: "test-model".to_string(),
        provider_turn_id: "turn_1".to_string(),
        provider_call_id: "call_1".to_string(),
        provider_tool_name: "demo__echo".to_string(),
        capability_id: CapabilityId::new("demo.echo").unwrap(),
        arguments: serde_json::json!({"message":"hello"}),
        response_reasoning: Some("provider response reasoning".to_string()),
        reasoning: Some("provider call reasoning".to_string()),
        signature: Some("sig-1".to_string()),
    };
    let tool_result = service
        .append_tool_result_reference(AppendToolResultReferenceRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
            turn_run_id: "run-1".into(),
            result_ref: "result:durable-demo".into(),
            safe_summary: ToolResultSafeSummary::new("safe durable tool result").unwrap(),
            provider_call: Some(provider_call.clone()),
        })
        .await
        .unwrap();
    let mut invalid_provider_call = provider_call;
    invalid_provider_call.provider_id.clear();
    let duplicate_tool_result = service
        .append_tool_result_reference(AppendToolResultReferenceRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
            turn_run_id: "run-1".into(),
            result_ref: "result:durable-demo".into(),
            safe_summary: ToolResultSafeSummary::new("retry safe durable tool result ignored")
                .unwrap(),
            provider_call: Some(invalid_provider_call),
        })
        .await
        .unwrap();
    assert_eq!(tool_result.message_id, duplicate_tool_result.message_id);

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
    assert_eq!(history.messages.len(), 4);
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
    assert_eq!(history.messages[3].kind, MessageKind::ToolResultReference);
    assert_eq!(history.messages[3].status, MessageStatus::Finalized);
    assert_eq!(
        history.messages[3].tool_result_ref.as_deref(),
        Some("result:durable-demo")
    );
    assert!(history.messages[3].tool_result_provider_call.is_none());
    assert_eq!(history.summary_artifacts.len(), 1);
    assert_eq!(history.summary_artifacts[0].content, "[redacted]");

    let context = service
        .load_context_window(LoadContextWindowRequest {
            scope: thread_scope.clone(),
            thread_id: thread_id.clone(),
            max_messages: 16,
        })
        .await
        .unwrap();
    assert_eq!(context.messages.len(), 3);
    assert_eq!(context.messages[0].content, "safe follow-up");
    assert_eq!(context.messages[1].content, "final answer");
    let envelope: ToolResultReferenceEnvelope =
        serde_json::from_str(&context.messages[2].content).unwrap();
    assert_eq!(envelope.version, 1);
    assert_eq!(envelope.result_ref, "result:durable-demo");
    assert_eq!(envelope.safe_summary.as_str(), "safe durable tool result");
    let provider_call = context.messages[2]
        .tool_result_provider_call
        .as_ref()
        .expect("provider metadata survives durable reload in model context");
    assert_eq!(provider_call.provider_turn_id, "turn_1");
    assert_eq!(provider_call.provider_call_id, "call_1");
    assert_eq!(provider_call.provider_tool_name, "demo__echo");
    assert_eq!(provider_call.capability_id.as_str(), "demo.echo");
    assert_eq!(
        provider_call.arguments,
        serde_json::json!({"message":"hello"})
    );
    assert_eq!(
        provider_call.response_reasoning.as_deref(),
        Some("provider response reasoning")
    );
    assert_eq!(
        provider_call.reasoning.as_deref(),
        Some("provider call reasoning")
    );
    assert_eq!(provider_call.signature.as_deref(), Some("sig-1"));

    let exact_context = service
        .load_context_messages(LoadContextMessagesRequest {
            scope: thread_scope.clone(),
            thread_id: thread_id.clone(),
            message_ids: vec![history.messages[3].message_id],
        })
        .await
        .unwrap();
    let exact_provider_call = exact_context.messages[0]
        .tool_result_provider_call
        .as_ref()
        .expect("exact context lookup preserves provider metadata");
    assert_eq!(exact_provider_call.provider_turn_id, "turn_1");
    assert_eq!(exact_provider_call.provider_call_id, "call_1");
    assert_eq!(exact_provider_call.provider_tool_name, "demo__echo");

    let wrong_scope = service
        .list_thread_history(ThreadHistoryRequest {
            scope: scope(&format!("{label}-wrong")),
            thread_id,
        })
        .await;
    assert!(wrong_scope.is_err());
}

#[cfg(feature = "libsql")]
async fn libsql_durable_concurrency_flow(db: Arc<libsql::Database>, label: &str) {
    let service = LibSqlSessionThreadService::new(Arc::clone(&db));
    durable_concurrency_setup(&service, label).await;
    let thread_id = ThreadId::new(format!("thread-{label}")).unwrap();
    let writes = (0..16).map(|index| {
        let db = Arc::clone(&db);
        let label = label.to_string();
        let thread_id = thread_id.clone();
        async move {
            let service = LibSqlSessionThreadService::new(db);
            service
                .accept_inbound_message(AcceptInboundMessageRequest {
                    scope: scope(&label),
                    thread_id,
                    actor_id: "actor-a".into(),
                    source_binding_id: Some("web-client".into()),
                    reply_target_binding_id: None,
                    external_event_id: Some(format!("event-{index}")),
                    content: MessageContent::text(format!("message-{index}")),
                })
                .await
                .unwrap()
        }
    });
    join_all(writes).await;
    assert_concurrent_history(&service, label).await;
}

#[cfg(feature = "postgres")]
async fn postgres_durable_concurrency_flow(pool: deadpool_postgres::Pool, label: &str) {
    let service = PostgresSessionThreadService::new(pool.clone());
    durable_concurrency_setup(&service, label).await;
    let thread_id = ThreadId::new(format!("thread-{label}")).unwrap();
    let writes = (0..16).map(|index| {
        let pool = pool.clone();
        let label = label.to_string();
        let thread_id = thread_id.clone();
        async move {
            let service = PostgresSessionThreadService::new(pool);
            service
                .accept_inbound_message(AcceptInboundMessageRequest {
                    scope: scope(&label),
                    thread_id,
                    actor_id: "actor-a".into(),
                    source_binding_id: Some("web-client".into()),
                    reply_target_binding_id: None,
                    external_event_id: Some(format!("event-{index}")),
                    content: MessageContent::text(format!("message-{index}")),
                })
                .await
                .unwrap()
        }
    });
    join_all(writes).await;
    assert_concurrent_history(&service, label).await;
}

async fn durable_concurrency_setup(service: &impl SessionThreadService, label: &str) {
    service
        .ensure_thread(EnsureThreadRequest {
            scope: scope(label),
            thread_id: Some(ThreadId::new(format!("thread-{label}")).unwrap()),
            created_by_actor_id: "actor-a".into(),
            title: None,
            metadata_json: None,
        })
        .await
        .unwrap();
}

async fn durable_long_external_event_flow(service: &impl SessionThreadService, label: &str) {
    let thread = service
        .ensure_thread(EnsureThreadRequest {
            scope: scope(label),
            thread_id: Some(ThreadId::new(format!("thread-{label}")).unwrap()),
            created_by_actor_id: "actor-a".into(),
            title: None,
            metadata_json: None,
        })
        .await
        .unwrap();
    let long_event_id = format!("event-{}", pseudo_random_ascii(12_000));
    let first = service
        .accept_inbound_message(AcceptInboundMessageRequest {
            scope: scope(label),
            thread_id: thread.thread_id.clone(),
            actor_id: "actor-a".into(),
            source_binding_id: Some("web-client".into()),
            reply_target_binding_id: None,
            external_event_id: Some(long_event_id.clone()),
            content: MessageContent::text("large id event"),
        })
        .await
        .unwrap();
    let duplicate = service
        .accept_inbound_message(AcceptInboundMessageRequest {
            scope: scope(label),
            thread_id: thread.thread_id.clone(),
            actor_id: "actor-a".into(),
            source_binding_id: Some("web-client".into()),
            reply_target_binding_id: None,
            external_event_id: Some(long_event_id),
            content: MessageContent::text("retry ignored"),
        })
        .await
        .unwrap();

    assert!(duplicate.idempotent_replay);
    assert_eq!(first.message_id, duplicate.message_id);
    let history = service
        .list_thread_history(ThreadHistoryRequest {
            scope: scope(label),
            thread_id: thread.thread_id,
        })
        .await
        .unwrap();
    assert_eq!(history.messages.len(), 1);
    assert_eq!(
        history.messages[0].content.as_deref(),
        Some("large id event")
    );
}

async fn assert_concurrent_history(service: &impl SessionThreadService, label: &str) {
    let history = service
        .list_thread_history(ThreadHistoryRequest {
            scope: scope(label),
            thread_id: ThreadId::new(format!("thread-{label}")).unwrap(),
        })
        .await
        .unwrap();
    let sequences = history
        .messages
        .iter()
        .map(|message| message.sequence)
        .collect::<Vec<_>>();
    assert_eq!(sequences, (1..=16).collect::<Vec<_>>());

    let replay = service
        .accept_inbound_message(AcceptInboundMessageRequest {
            scope: scope(label),
            thread_id: ThreadId::new(format!("thread-{label}")).unwrap(),
            actor_id: "actor-a".into(),
            source_binding_id: Some("web-client".into()),
            reply_target_binding_id: None,
            external_event_id: Some("event-3".into()),
            content: MessageContent::text("retry ignored"),
        })
        .await
        .unwrap();
    assert!(replay.idempotent_replay);

    let after_replay = service
        .list_thread_history(ThreadHistoryRequest {
            scope: scope(label),
            thread_id: ThreadId::new(format!("thread-{label}")).unwrap(),
        })
        .await
        .unwrap();
    assert_eq!(after_replay.messages.len(), 16);
}

fn pseudo_random_ascii(len: usize) -> String {
    let mut state = 0x5eed_u64;
    (0..len)
        .map(|_| {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
            (b'!' + ((state >> 32) % 94) as u8) as char
        })
        .collect()
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

#[cfg(feature = "libsql")]
fn libsql_db_path() -> (String, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("threads.db");
    (db_path.to_string_lossy().into_owned(), dir)
}

#[cfg(feature = "postgres")]
async fn postgres_pool() -> Option<deadpool_postgres::Pool> {
    let url = std::env::var("IRONCLAW_THREADS_POSTGRES_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .unwrap_or_else(|_| "postgres://localhost/ironclaw_test".to_string());
    let config: tokio_postgres::Config =
        url.parse().expect("thread postgres test URL must be valid");
    let manager = deadpool_postgres::Manager::new(config, tokio_postgres::NoTls);
    let pool = deadpool_postgres::Pool::builder(manager)
        .max_size(8)
        .build()
        .unwrap();
    match pool.get().await {
        Ok(_) => Some(pool),
        Err(error) if skip_postgres_requested() => {
            eprintln!(
                "skipping postgres thread contract (IRONCLAW_SKIP_POSTGRES_TESTS=1): {error}"
            );
            None
        }
        Err(error) => panic!(
            "postgres thread contract could not reach Postgres ({error}); set \
             IRONCLAW_THREADS_POSTGRES_URL or DATABASE_URL, or set \
             IRONCLAW_SKIP_POSTGRES_TESTS=1 to explicitly skip."
        ),
    }
}

#[cfg(feature = "postgres")]
fn skip_postgres_requested() -> bool {
    std::env::var("IRONCLAW_SKIP_POSTGRES_TESTS").is_ok_and(|value| value == "1" || value == "true")
}

#[cfg(feature = "postgres")]
fn unique_suffix() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time")
        .as_nanos()
}
