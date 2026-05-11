use std::{fs, path::Path, sync::Arc};

use ironclaw_event_projections::{
    AuditProjectionRequest, AuditProjectionService, AuditProjectionStage, DurableMemoryAuditSink,
    ProjectionScope, ReplayAuditProjectionService,
};
use ironclaw_events::{AuditSink, DurableAuditSink};
use ironclaw_filesystem::RootFilesystem;
use ironclaw_host_api::{
    AgentId, InvocationId, ProjectId, ResourceScope, TenantId, UserId, VirtualPath,
};
use ironclaw_memory::{
    ChunkingMemoryDocumentIndexer, MemoryBackend, MemoryBackendCapabilities,
    MemoryBackendFilesystemAdapter, MemoryContext, MemoryDocumentScope, MemorySearchRequest,
    RebornLibSqlMemoryDocumentRepository, RepositoryMemoryBackend,
};
use ironclaw_reborn_event_store::{
    RebornEventStoreConfig, RebornProfile, build_reborn_event_stores,
};

#[tokio::test]
async fn memory_write_index_and_search_project_metadata_only_from_jsonl_audit_log() {
    let temp = tempfile::tempdir().unwrap();
    let store_root = temp.path().join("reborn-event-store");
    let stores = build_reborn_event_stores(
        RebornProfile::LocalDev,
        RebornEventStoreConfig::Jsonl {
            root: store_root.clone(),
            accept_single_node_durable: false,
        },
    )
    .await
    .unwrap();
    let audit_log = Arc::clone(&stores.audit);
    let audit_sink: Arc<dyn AuditSink> = Arc::new(DurableAuditSink::new(Arc::clone(&audit_log)));
    let memory_events = Arc::new(DurableMemoryAuditSink::new(audit_sink));

    let memory_db_dir = tempfile::tempdir().unwrap();
    let memory_db = Arc::new(
        libsql::Builder::new_local(memory_db_dir.path().join("reborn-memory.db"))
            .build()
            .await
            .unwrap(),
    );
    let repository = Arc::new(RebornLibSqlMemoryDocumentRepository::new(memory_db));
    repository.run_migrations().await.unwrap();

    let indexer = Arc::new(
        ChunkingMemoryDocumentIndexer::new(Arc::clone(&repository))
            .with_memory_event_sink(Arc::clone(&memory_events)),
    );
    let backend = Arc::new(
        RepositoryMemoryBackend::new(Arc::clone(&repository))
            .without_prompt_write_safety_policy()
            .with_capabilities(MemoryBackendCapabilities {
                file_documents: true,
                metadata: true,
                versioning: true,
                full_text_search: true,
                vector_search: false,
                embeddings: false,
                ..MemoryBackendCapabilities::default()
            })
            .with_indexer(indexer)
            .with_memory_event_sink(Arc::clone(&memory_events)),
    );
    let filesystem = MemoryBackendFilesystemAdapter::new(Arc::clone(&backend))
        .without_prompt_write_safety_policy();
    let virtual_path = VirtualPath::new(
        "/memory/tenants/tenant-a/users/alice/agents/agent-a/projects/project-a/notes/important.md",
    )
    .unwrap();
    let raw_content = "RAW_DOCUMENT_CONTENT_SENTINEL_3022 alpha beta /Users/firatsertgoz/.ssh/id_ed25519 sk-live-memory-significant-secret";

    filesystem
        .write_file(&virtual_path, raw_content.as_bytes())
        .await
        .unwrap();

    let context = MemoryContext::new(
        MemoryDocumentScope::new_with_agent(
            "tenant-a",
            "alice",
            Some("agent-a"),
            Some("project-a"),
        )
        .unwrap(),
    );
    let search_query = "SEARCHQUERYSENTINEL3022";
    let search_request = MemorySearchRequest::new(search_query)
        .unwrap()
        .with_full_text(true)
        .with_vector(false);
    let search_results = backend.search(&context, search_request).await.unwrap();
    assert!(search_results.is_empty());

    let projection = ReplayAuditProjectionService::from_audit_log(Arc::clone(&audit_log));
    let snapshot = projection
        .snapshot(AuditProjectionRequest {
            scope: ProjectionScope::from_resource_scope(&memory_resource_scope(context.scope())),
            after: None,
            limit: 10,
        })
        .await
        .unwrap();

    let action_kinds = snapshot
        .entries
        .iter()
        .map(|entry| entry.action_kind.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        action_kinds,
        vec![
            "memory_document_written",
            "memory_document_indexed",
            "memory_search_performed",
        ]
    );
    for entry in &snapshot.entries {
        assert_eq!(entry.stage, AuditProjectionStage::After);
        assert_eq!(entry.action_target, None);
        assert_eq!(entry.decision_kind, "memory_event_recorded");
    }
    assert_eq!(
        snapshot.entries[0].output_bytes,
        Some(raw_content.len() as u64)
    );
    assert_eq!(snapshot.entries[1].result_status, None);
    assert_eq!(snapshot.entries[2].result_status, None);

    let projection_json = serde_json::to_string(&snapshot).unwrap();
    let jsonl_bytes = read_directory_text(&store_root);
    for forbidden in [
        "RAW_DOCUMENT_CONTENT_SENTINEL_3022",
        "/Users/firatsertgoz/.ssh/id_ed25519",
        "sk-live-memory-significant-secret",
        search_query,
    ] {
        assert!(
            !projection_json.contains(forbidden),
            "memory significant-event projection leaked {forbidden}: {projection_json}"
        );
        assert!(
            !jsonl_bytes.contains(forbidden),
            "durable memory significant-event bytes leaked {forbidden}: {jsonl_bytes}"
        );
    }
}

fn memory_resource_scope(scope: &MemoryDocumentScope) -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new(scope.tenant_id()).unwrap(),
        user_id: UserId::new(scope.user_id()).unwrap(),
        agent_id: scope.agent_id().map(|agent| AgentId::new(agent).unwrap()),
        project_id: scope
            .project_id()
            .map(|project| ProjectId::new(project).unwrap()),
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    }
}

fn read_directory_text(root: &Path) -> String {
    let mut output = String::new();
    read_directory_text_into(root, &mut output);
    output
}

fn read_directory_text_into(path: &Path, output: &mut String) {
    if path.is_dir() {
        for entry in fs::read_dir(path).unwrap() {
            read_directory_text_into(&entry.unwrap().path(), output);
        }
    } else if path.is_file() {
        output.push_str(&fs::read_to_string(path).unwrap_or_default());
    }
}
