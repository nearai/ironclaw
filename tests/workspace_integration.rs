//! Integration tests for the workspace module.
//!
//! Requires a running PostgreSQL with pgvector extension.
//! Set DATABASE_URL=postgres://localhost/ironclaw_test

use std::sync::Arc;

use ironclaw::workspace::{MockEmbeddings, SearchConfig, Workspace, paths};

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

async fn cleanup_user(pool: &deadpool_postgres::Pool, user_id: &str) {
    let conn = pool.get().await.expect("Failed to get connection");
    conn.execute(
        "DELETE FROM memory_documents WHERE user_id = $1",
        &[&user_id],
    )
    .await
    .ok();
}

#[tokio::test]
async fn test_workspace_write_and_read() {
    let pool = get_pool();
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
    let user_id = "test_hybrid_search";
    cleanup_user(&pool, user_id).await;

    // Create workspace with mock embeddings (1536 dimensions to match OpenAI)
    let embeddings = Arc::new(MockEmbeddings::new(1536));
    let workspace = Workspace::new(user_id, pool.clone()).with_embeddings(embeddings);

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
async fn test_workspace_list_all() {
    let pool = get_pool();
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
