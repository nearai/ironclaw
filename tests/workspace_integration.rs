#![cfg(feature = "postgres")]
//! Integration tests for the workspace module.
//!
//! Requires a running PostgreSQL with pgvector extension.
//! Set DATABASE_URL=postgres://localhost/ironclaw_test

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw::llm::{
    CompletionRequest, CompletionResponse, FinishReason, LlmError, LlmProvider,
    ToolCompletionRequest, ToolCompletionResponse,
};
use ironclaw::workspace::{MockEmbeddings, SearchConfig, Workspace, paths};
use rust_decimal::Decimal;

fn get_pool() -> deadpool_postgres::Pool {
    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://localhost/ironclaw_test".to_string());

    let config: tokio_postgres::Config = database_url.parse().expect("Invalid DATABASE_URL");

    let mgr = deadpool_postgres::Manager::new(config, tokio_postgres::NoTls);
    deadpool_postgres::Pool::builder(mgr)
        .max_size(4)
        .build()
        .expect("Failed to create pool")
}

/// Try to get a connection, returning None if Postgres is unreachable.
/// Tests call this to skip gracefully in CI where no database is available.
async fn try_connect(pool: &deadpool_postgres::Pool) -> Option<()> {
    match pool.get().await {
        Ok(_) => Some(()),
        Err(e) => {
            eprintln!("skipping: database unavailable ({e})");
            None
        }
    }
}

async fn cleanup_user(pool: &deadpool_postgres::Pool, user_id: &str) {
    let conn = pool.get().await.expect("Failed to get connection");
    conn.execute(
        "DELETE FROM memory_documents WHERE user_id = $1",
        &[&user_id],
    )
    .await
    .ok();
}

struct StubLlm {
    response: String,
}

impl StubLlm {
    fn new(response: impl Into<String>) -> Self {
        Self {
            response: response.into(),
        }
    }
}

#[async_trait]
impl LlmProvider for StubLlm {
    fn model_name(&self) -> &str {
        "stub-model"
    }

    fn cost_per_token(&self) -> (Decimal, Decimal) {
        (Decimal::ZERO, Decimal::ZERO)
    }

    async fn complete(&self, _request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        Ok(CompletionResponse {
            content: self.response.clone(),
            input_tokens: 0,
            output_tokens: 0,
            finish_reason: FinishReason::Stop,
            cache_read_input_tokens: 0,
            cache_creation_input_tokens: 0,
        })
    }

    async fn complete_with_tools(
        &self,
        _request: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
        Err(LlmError::RequestFailed {
            provider: "stub-model".to_string(),
            reason: "tool calls not supported in integration stub".to_string(),
        })
    }
}

#[tokio::test]
async fn test_workspace_write_and_read() {
    let pool = get_pool();
    if try_connect(&pool).await.is_none() {
        return;
    }
    let user_id = "test_write_read";
    cleanup_user(&pool, user_id).await;

    let workspace = Workspace::new(user_id, pool.clone());

    // Write a file
    let doc = workspace
        .write("README.md", "# Hello World\n\nThis is a test.")
        .await
        .expect("Failed to write");

    assert_eq!(doc.path, "README.md");
    assert!(doc.content.contains("Hello World"));

    // Read it back
    let doc2 = workspace.read("README.md").await.expect("Failed to read");
    assert_eq!(doc2.content, "# Hello World\n\nThis is a test.");

    // Cleanup
    cleanup_user(&pool, user_id).await;
}

#[tokio::test]
async fn test_workspace_append() {
    let pool = get_pool();
    if try_connect(&pool).await.is_none() {
        return;
    }
    let user_id = "test_append";
    cleanup_user(&pool, user_id).await;

    let workspace = Workspace::new(user_id, pool.clone());

    // Write initial content
    workspace
        .write("notes.md", "Line 1")
        .await
        .expect("Failed to write");

    // Append more
    workspace
        .append("notes.md", "Line 2")
        .await
        .expect("Failed to append");

    // Read and verify
    let doc = workspace.read("notes.md").await.expect("Failed to read");
    assert_eq!(doc.content, "Line 1\nLine 2");

    cleanup_user(&pool, user_id).await;
}

#[tokio::test]
async fn test_workspace_nested_paths() {
    let pool = get_pool();
    if try_connect(&pool).await.is_none() {
        return;
    }
    let user_id = "test_nested";
    cleanup_user(&pool, user_id).await;

    let workspace = Workspace::new(user_id, pool.clone());

    // Write nested files
    workspace
        .write("projects/alpha/README.md", "# Alpha")
        .await
        .expect("Failed to write alpha");
    workspace
        .write("projects/alpha/notes.md", "Notes here")
        .await
        .expect("Failed to write notes");
    workspace
        .write("projects/beta/README.md", "# Beta")
        .await
        .expect("Failed to write beta");

    // List root
    let root = workspace.list("").await.expect("Failed to list root");
    assert_eq!(root.len(), 1); // just "projects/"
    assert!(root[0].is_directory);
    assert_eq!(root[0].name(), "projects");

    // List projects
    let projects = workspace
        .list("projects")
        .await
        .expect("Failed to list projects");
    assert_eq!(projects.len(), 2); // alpha/, beta/

    // List alpha
    let alpha = workspace
        .list("projects/alpha")
        .await
        .expect("Failed to list alpha");
    assert_eq!(alpha.len(), 2); // README.md, notes.md

    cleanup_user(&pool, user_id).await;
}

#[tokio::test]
async fn test_workspace_delete() {
    let pool = get_pool();
    if try_connect(&pool).await.is_none() {
        return;
    }
    let user_id = "test_delete";
    cleanup_user(&pool, user_id).await;

    let workspace = Workspace::new(user_id, pool.clone());

    // Write and verify exists
    workspace
        .write("temp.md", "temporary")
        .await
        .expect("Failed to write");
    assert!(workspace.exists("temp.md").await.expect("exists failed"));

    // Delete
    workspace.delete("temp.md").await.expect("Failed to delete");

    // Verify gone
    assert!(!workspace.exists("temp.md").await.expect("exists failed"));

    cleanup_user(&pool, user_id).await;
}

#[tokio::test]
async fn test_workspace_memory_operations() {
    let pool = get_pool();
    if try_connect(&pool).await.is_none() {
        return;
    }
    let user_id = "test_memory_ops";
    cleanup_user(&pool, user_id).await;

    let workspace = Workspace::new(user_id, pool.clone());

    // Append to memory
    workspace
        .append_memory("User prefers dark mode")
        .await
        .expect("Failed to append memory");
    workspace
        .append_memory("User's timezone is PST")
        .await
        .expect("Failed to append memory");

    // Read memory
    let memory = workspace.memory().await.expect("Failed to get memory");
    assert!(memory.content.contains("dark mode"));
    assert!(memory.content.contains("PST"));
    // Entries should be separated by double newline
    assert!(memory.content.contains("\n\n"));

    cleanup_user(&pool, user_id).await;
}

#[tokio::test]
async fn test_workspace_daily_log() {
    let pool = get_pool();
    if try_connect(&pool).await.is_none() {
        return;
    }
    let user_id = "test_daily_log";
    cleanup_user(&pool, user_id).await;

    let workspace = Workspace::new(user_id, pool.clone());

    // Append to daily log (timestamped)
    workspace
        .append_daily_log("Started working on feature X")
        .await
        .expect("Failed to append daily log");

    // Read today's log
    let log = workspace
        .today_log()
        .await
        .expect("Failed to get today log");
    assert!(log.content.contains("feature X"));
    // Should have timestamp prefix like [HH:MM:SS]
    assert!(log.content.contains("["));

    cleanup_user(&pool, user_id).await;
}

#[tokio::test]
async fn test_workspace_fts_search() {
    let pool = get_pool();
    if try_connect(&pool).await.is_none() {
        return;
    }
    let user_id = "test_fts_search";
    cleanup_user(&pool, user_id).await;

    let workspace = Workspace::new(user_id, pool.clone());

    // Write some documents
    workspace
        .write(
            "docs/authentication.md",
            "# Authentication\n\nThe system uses JWT tokens for authentication.",
        )
        .await
        .expect("write failed");
    workspace
        .write(
            "docs/database.md",
            "# Database\n\nWe use PostgreSQL with pgvector for vector search.",
        )
        .await
        .expect("write failed");
    workspace
        .write(
            "docs/api.md",
            "# API\n\nThe REST API uses JSON for request and response bodies.",
        )
        .await
        .expect("write failed");

    // Search for JWT (FTS only since no embeddings)
    let results = workspace
        .search_with_config("JWT authentication", SearchConfig::default().fts_only())
        .await
        .expect("search failed");

    assert!(!results.is_empty(), "Should find results for JWT");
    assert!(
        results[0].content.contains("JWT"),
        "Top result should contain JWT"
    );

    // Search for PostgreSQL
    let results = workspace
        .search_with_config("PostgreSQL database", SearchConfig::default().fts_only())
        .await
        .expect("search failed");

    assert!(!results.is_empty(), "Should find results for PostgreSQL");
    assert!(
        results[0].content.contains("PostgreSQL"),
        "Top result should contain PostgreSQL"
    );

    cleanup_user(&pool, user_id).await;
}

#[tokio::test]
async fn test_workspace_hybrid_search_with_mock_embeddings() {
    let pool = get_pool();
    if try_connect(&pool).await.is_none() {
        return;
    }
    let user_id = "test_hybrid_search";
    cleanup_user(&pool, user_id).await;

    // Create workspace with mock embeddings (1536 dimensions to match OpenAI)
    let embeddings = Arc::new(MockEmbeddings::new(1536));
    let workspace = Workspace::new(user_id, pool.clone()).with_embeddings_uncached(embeddings);

    // Write documents
    workspace
        .write(
            "memory.md",
            "The user prefers dark mode and vim keybindings.",
        )
        .await
        .expect("write failed");
    workspace
        .write(
            "prefs.md",
            "Settings: theme=dark, editor=vim, font=monospace",
        )
        .await
        .expect("write failed");

    // Hybrid search
    let results = workspace
        .search("dark theme preference", 5)
        .await
        .expect("search failed");

    assert!(!results.is_empty(), "Should find results");
    // At least one result should be a hybrid match (found by both FTS and vector)
    // or we should have results from either method

    cleanup_user(&pool, user_id).await;
}

#[tokio::test]
async fn test_workspace_tiered_summaries_default_search_returns_l1() {
    let pool = get_pool();
    if try_connect(&pool).await.is_none() {
        return;
    }
    let user_id = "test_tiered_summaries";
    cleanup_user(&pool, user_id).await;

    let llm = Arc::new(StubLlm::new(
        r#"{"l0":"Project Alpha overview.","l1":"Topic: Project Alpha\n\nKey facts:\n- The roadmap is stable.\n- The owner prefers concise updates.\n\nEntities: Project Alpha, Alice\nDates: 2026-03-22\nAction items: Review next milestone."}"#,
    ));
    let workspace = Workspace::new(user_id, pool.clone()).with_llm(llm);

    let mut body = String::new();
    for _ in 0..80 {
        body.push_str("Project Alpha status remains stable and the roadmap stays on track. ");
    }

    workspace
        .write("projects/alpha/status.md", &body)
        .await
        .expect("write failed");

    let default_result = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let results = workspace
                .search("Project Alpha", 5)
                .await
                .expect("search failed");
            if let Some(first) = results.first()
                && first.content.contains("Topic: Project Alpha")
            {
                break first.clone();
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("timed out waiting for summary generation");

    assert!(
        default_result.content.contains("Topic: Project Alpha"),
        "default search should return L1 summary"
    );

    let l0_result = workspace
        .search_with_config(
            "Project Alpha",
            SearchConfig::default()
                .fts_only()
                .with_detail(ironclaw::workspace::SearchDetailLevel::L0),
        )
        .await
        .expect("search l0 failed");
    assert!(
        l0_result
            .first()
            .expect("expected l0 result")
            .content
            .contains("Project Alpha overview"),
        "L0 search should return the abstract"
    );

    let l2_result = workspace
        .search_with_config(
            "Project Alpha",
            SearchConfig::default()
                .fts_only()
                .with_detail(ironclaw::workspace::SearchDetailLevel::L2),
        )
        .await
        .expect("search l2 failed");
    assert!(
        l2_result
            .first()
            .expect("expected l2 result")
            .content
            .contains("roadmap stays on track"),
        "L2 search should return the raw chunk content"
    );

    cleanup_user(&pool, user_id).await;
}

#[tokio::test]
async fn test_workspace_list_all() {
    let pool = get_pool();
    if try_connect(&pool).await.is_none() {
        return;
    }
    let user_id = "test_list_all";
    cleanup_user(&pool, user_id).await;

    let workspace = Workspace::new(user_id, pool.clone());

    // Write files at various depths
    workspace.write("README.md", "root").await.unwrap();
    workspace.write("docs/intro.md", "intro").await.unwrap();
    workspace.write("docs/api/rest.md", "rest").await.unwrap();
    workspace.write("src/main.md", "main").await.unwrap();

    // List all
    let all = workspace.list_all().await.expect("list_all failed");
    assert_eq!(all.len(), 4);
    assert!(all.contains(&"README.md".to_string()));
    assert!(all.contains(&"docs/intro.md".to_string()));
    assert!(all.contains(&"docs/api/rest.md".to_string()));
    assert!(all.contains(&"src/main.md".to_string()));

    cleanup_user(&pool, user_id).await;
}

#[tokio::test]
async fn test_workspace_system_prompt() {
    let pool = get_pool();
    if try_connect(&pool).await.is_none() {
        return;
    }
    let user_id = "test_system_prompt";
    cleanup_user(&pool, user_id).await;

    let workspace = Workspace::new(user_id, pool.clone());

    // Write identity files
    workspace
        .write(paths::AGENTS, "You are a helpful assistant.")
        .await
        .unwrap();
    workspace
        .write(paths::SOUL, "Be kind and thorough.")
        .await
        .unwrap();
    workspace.write(paths::USER, "Name: Alice").await.unwrap();

    // Get system prompt
    let prompt = workspace
        .system_prompt()
        .await
        .expect("system_prompt failed");

    assert!(
        prompt.contains("helpful assistant"),
        "Should include AGENTS.md"
    );
    assert!(
        prompt.contains("kind and thorough"),
        "Should include SOUL.md"
    );
    assert!(prompt.contains("Alice"), "Should include USER.md");

    cleanup_user(&pool, user_id).await;
}

// ── Multi-scope workspace read tests ──────────────────────────────────
//
// These exercise the PostgreSQL-optimized `_multi` query paths
// (repository.rs) that the libSQL backend covers via default trait impls.

#[tokio::test]
async fn test_multi_scope_read_across_scopes() {
    let pool = get_pool();
    if try_connect(&pool).await.is_none() {
        return;
    }
    let shared_id = "ms_shared_read";
    let alice_id = "ms_alice_read";
    cleanup_user(&pool, shared_id).await;
    cleanup_user(&pool, alice_id).await;

    // Write a doc as "shared"
    let ws_shared = Workspace::new(shared_id, pool.clone());
    ws_shared
        .write("docs/team-standup.md", "Team standup notes from Monday")
        .await
        .expect("shared write failed");

    // Alice with "shared" as an additional read scope
    let ws_alice = Workspace::new(alice_id, pool.clone())
        .with_additional_read_scopes(vec![shared_id.to_string()]);

    let doc = ws_alice
        .read("docs/team-standup.md")
        .await
        .expect("cross-scope read failed");
    assert_eq!(doc.content, "Team standup notes from Monday");

    cleanup_user(&pool, shared_id).await;
    cleanup_user(&pool, alice_id).await;
}

#[tokio::test]
async fn test_multi_scope_write_stays_in_primary() {
    let pool = get_pool();
    if try_connect(&pool).await.is_none() {
        return;
    }
    let shared_id = "ms_shared_write";
    let alice_id = "ms_alice_write";
    cleanup_user(&pool, shared_id).await;
    cleanup_user(&pool, alice_id).await;

    let ws_alice = Workspace::new(alice_id, pool.clone())
        .with_additional_read_scopes(vec![shared_id.to_string()]);

    ws_alice
        .write("notes/personal.md", "Alice's private note")
        .await
        .expect("alice write failed");

    // Shared workspace should NOT see Alice's note
    let ws_shared = Workspace::new(shared_id, pool.clone());
    let result = ws_shared.read("notes/personal.md").await;
    assert!(result.is_err(), "Shared scope should not see Alice's note");

    cleanup_user(&pool, shared_id).await;
    cleanup_user(&pool, alice_id).await;
}

#[tokio::test]
async fn test_multi_scope_list_all_merges() {
    let pool = get_pool();
    if try_connect(&pool).await.is_none() {
        return;
    }
    let shared_id = "ms_shared_list";
    let alice_id = "ms_alice_list";
    cleanup_user(&pool, shared_id).await;
    cleanup_user(&pool, alice_id).await;

    // Write as alice (plain, no multi-scope)
    let ws_alice_plain = Workspace::new(alice_id, pool.clone());
    ws_alice_plain
        .write("notes/personal.md", "My notes")
        .await
        .expect("alice write failed");

    // Write as shared
    let ws_shared = Workspace::new(shared_id, pool.clone());
    ws_shared
        .write("docs/shared-doc.md", "Shared document")
        .await
        .expect("shared write failed");

    // Alice with multi-scope should see both
    let ws_alice = Workspace::new(alice_id, pool.clone())
        .with_additional_read_scopes(vec![shared_id.to_string()]);

    let all_paths = ws_alice.list_all().await.expect("list_all failed");
    assert!(
        all_paths.contains(&"notes/personal.md".to_string()),
        "Should contain alice's note: {:?}",
        all_paths
    );
    assert!(
        all_paths.contains(&"docs/shared-doc.md".to_string()),
        "Should contain shared doc: {:?}",
        all_paths
    );

    cleanup_user(&pool, shared_id).await;
    cleanup_user(&pool, alice_id).await;
}

#[tokio::test]
async fn test_multi_scope_list_directory_merges() {
    let pool = get_pool();
    if try_connect(&pool).await.is_none() {
        return;
    }
    let shared_id = "ms_shared_dir";
    let alice_id = "ms_alice_dir";
    cleanup_user(&pool, shared_id).await;
    cleanup_user(&pool, alice_id).await;

    let ws_alice_plain = Workspace::new(alice_id, pool.clone());
    ws_alice_plain
        .write("docs/alice-doc.md", "Alice's doc")
        .await
        .expect("alice write failed");

    let ws_shared = Workspace::new(shared_id, pool.clone());
    ws_shared
        .write("docs/shared-doc.md", "Shared doc")
        .await
        .expect("shared write failed");

    let ws_alice = Workspace::new(alice_id, pool.clone())
        .with_additional_read_scopes(vec![shared_id.to_string()]);

    let entries = ws_alice.list("docs").await.expect("list failed");
    let paths: Vec<&str> = entries.iter().map(|e| e.path.as_str()).collect();
    assert!(
        paths.contains(&"docs/alice-doc.md"),
        "Should contain alice's doc: {:?}",
        paths
    );
    assert!(
        paths.contains(&"docs/shared-doc.md"),
        "Should contain shared doc: {:?}",
        paths
    );

    cleanup_user(&pool, shared_id).await;
    cleanup_user(&pool, alice_id).await;
}

#[tokio::test]
async fn test_multi_scope_read_priority_primary_first() {
    let pool = get_pool();
    if try_connect(&pool).await.is_none() {
        return;
    }
    let shared_id = "ms_shared_prio";
    let alice_id = "ms_alice_prio";
    cleanup_user(&pool, shared_id).await;
    cleanup_user(&pool, alice_id).await;

    // Write same path in both scopes
    let ws_shared = Workspace::new(shared_id, pool.clone());
    ws_shared
        .write("config/settings.md", "Shared settings v1")
        .await
        .expect("shared write failed");

    let ws_alice_plain = Workspace::new(alice_id, pool.clone());
    ws_alice_plain
        .write("config/settings.md", "Alice's settings override")
        .await
        .expect("alice write failed");

    // Alice with multi-scope should get her own version (primary scope wins)
    let ws_alice = Workspace::new(alice_id, pool.clone())
        .with_additional_read_scopes(vec![shared_id.to_string()]);

    let doc = ws_alice
        .read("config/settings.md")
        .await
        .expect("read failed");
    assert_eq!(
        doc.content, "Alice's settings override",
        "Primary scope should take priority"
    );

    cleanup_user(&pool, shared_id).await;
    cleanup_user(&pool, alice_id).await;
}

#[tokio::test]
async fn test_multi_scope_exists_spans_scopes() {
    let pool = get_pool();
    if try_connect(&pool).await.is_none() {
        return;
    }
    let shared_id = "ms_shared_exists";
    let alice_id = "ms_alice_exists";
    cleanup_user(&pool, shared_id).await;
    cleanup_user(&pool, alice_id).await;

    let ws_shared = Workspace::new(shared_id, pool.clone());
    ws_shared
        .write("docs/shared-only.md", "Shared content")
        .await
        .expect("shared write failed");

    // Alice without multi-scope should NOT see it
    let ws_alice_plain = Workspace::new(alice_id, pool.clone());
    assert!(
        !ws_alice_plain
            .exists("docs/shared-only.md")
            .await
            .expect("exists failed"),
        "Alice without multi-scope should not see shared doc"
    );

    // Alice with multi-scope should see it
    let ws_alice = Workspace::new(alice_id, pool.clone())
        .with_additional_read_scopes(vec![shared_id.to_string()]);
    assert!(
        ws_alice
            .exists("docs/shared-only.md")
            .await
            .expect("exists failed"),
        "Alice with multi-scope should see shared doc"
    );

    cleanup_user(&pool, shared_id).await;
    cleanup_user(&pool, alice_id).await;
}

#[tokio::test]
async fn test_multi_scope_search_spans_scopes() {
    let pool = get_pool();
    if try_connect(&pool).await.is_none() {
        return;
    }
    let shared_id = "ms_shared_search";
    let alice_id = "ms_alice_search";
    cleanup_user(&pool, shared_id).await;
    cleanup_user(&pool, alice_id).await;

    let ws_shared = Workspace::new(shared_id, pool.clone());
    ws_shared
        .write(
            "docs/architecture.md",
            "The microservice architecture uses gRPC for inter-service communication",
        )
        .await
        .expect("shared write failed");

    let ws_alice_plain = Workspace::new(alice_id, pool.clone());
    ws_alice_plain
        .write("notes/ideas.md", "Consider switching to GraphQL federation")
        .await
        .expect("alice write failed");

    let ws_alice = Workspace::new(alice_id, pool.clone())
        .with_additional_read_scopes(vec![shared_id.to_string()]);

    // Search for content in the shared scope
    let results = ws_alice
        .search_with_config(
            "microservice gRPC architecture",
            SearchConfig::default().fts_only(),
        )
        .await
        .expect("search failed");
    assert!(!results.is_empty(), "Should find results from shared scope");

    cleanup_user(&pool, shared_id).await;
    cleanup_user(&pool, alice_id).await;
}

#[tokio::test]
async fn test_multi_scope_append_stays_in_primary() {
    let pool = get_pool();
    if try_connect(&pool).await.is_none() {
        return;
    }
    let shared_id = "ms_shared_append";
    let alice_id = "ms_alice_append";
    cleanup_user(&pool, shared_id).await;
    cleanup_user(&pool, alice_id).await;

    // Write a document as "shared"
    let ws_shared = Workspace::new(shared_id, pool.clone());
    ws_shared
        .write("notes/log.md", "shared original content")
        .await
        .expect("shared write failed");

    // Alice has "shared" as a read scope and appends to the same path
    let ws_alice = Workspace::new(alice_id, pool.clone())
        .with_additional_read_scopes(vec![shared_id.to_string()]);
    ws_alice
        .append("notes/log.md", "alice appended line")
        .await
        .expect("alice append failed");

    // Shared document must be unchanged (write isolation)
    let shared_doc = ws_shared
        .read("notes/log.md")
        .await
        .expect("shared read failed");
    assert_eq!(
        shared_doc.content, "shared original content",
        "Append must not modify the secondary scope's document"
    );

    // Alice should have her own copy with the appended content
    let ws_alice_plain = Workspace::new(alice_id, pool.clone());
    let alice_doc = ws_alice_plain
        .read("notes/log.md")
        .await
        .expect("alice read failed");
    assert_eq!(
        alice_doc.content, "alice appended line",
        "Append should create a new document in alice's scope"
    );

    cleanup_user(&pool, shared_id).await;
    cleanup_user(&pool, alice_id).await;
}
