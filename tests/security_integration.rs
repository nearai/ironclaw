//! Security integration tests.
//!
//! Tests path traversal prevention, input sanitization, and other security features.

use ironclaw::workspace::Workspace;

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
async fn test_path_traversal_read_blocked() {
    let pool = get_pool();
    let user_id = "test_traversal_read";
    cleanup_user(&pool, user_id).await;

    let workspace = Workspace::new(user_id, pool.clone());

    // Attempt to read with path traversal
    let result = workspace.read("../etc/passwd").await;
    assert!(result.is_err(), "Path traversal should be blocked");

    let result = workspace.read("foo/../../../etc/passwd").await;
    assert!(result.is_err(), "Deep path traversal should be blocked");

    let result = workspace.read("..").await;
    assert!(result.is_err(), "Double-dot alone should be blocked");

    cleanup_user(&pool, user_id).await;
}

#[tokio::test]
async fn test_path_traversal_write_blocked() {
    let pool = get_pool();
    let user_id = "test_traversal_write";
    cleanup_user(&pool, user_id).await;

    let workspace = Workspace::new(user_id, pool.clone());

    // Attempt to write with path traversal
    let result = workspace.write("../malicious.md", "evil content").await;
    assert!(result.is_err(), "Path traversal write should be blocked");

    let result = workspace.write("docs/../../etc/cron.d/evil", "* * * * * root /bin/bash -c 'malicious'").await;
    assert!(result.is_err(), "Deep path traversal write should be blocked");

    cleanup_user(&pool, user_id).await;
}

#[tokio::test]
async fn test_path_traversal_delete_blocked() {
    let pool = get_pool();
    let user_id = "test_traversal_delete";
    cleanup_user(&pool, user_id).await;

    let workspace = Workspace::new(user_id, pool.clone());

    // First create a legitimate file
    workspace.write("safe.md", "legitimate").await.expect("write failed");

    // Attempt to delete with path traversal
    let result = workspace.delete("../safe.md").await;
    assert!(result.is_err(), "Path traversal delete should be blocked");

    cleanup_user(&pool, user_id).await;
}

#[tokio::test]
async fn test_path_traversal_list_blocked() {
    let pool = get_pool();
    let user_id = "test_traversal_list";
    cleanup_user(&pool, user_id).await;

    let workspace = Workspace::new(user_id, pool.clone());

    // Attempt to list with path traversal
    let result = workspace.list("../").await;
    assert!(result.is_err(), "Path traversal list should be blocked");

    let result = workspace.list("..").await;
    assert!(result.is_err(), "Double-dot list should be blocked");

    cleanup_user(&pool, user_id).await;
}

#[tokio::test]
async fn test_null_byte_injection_blocked() {
    let pool = get_pool();
    let user_id = "test_null_byte";
    cleanup_user(&pool, user_id).await;

    let workspace = Workspace::new(user_id, pool.clone());

    // Null byte injection attempt
    let result = workspace.read("file.md\0.txt").await;
    assert!(result.is_err(), "Null byte injection should be blocked");

    let result = workspace.write("evil\0.md", "content").await;
    assert!(result.is_err(), "Null byte in write should be blocked");

    cleanup_user(&pool, user_id).await;
}

#[tokio::test]
async fn test_valid_paths_still_work() {
    let pool = get_pool();
    let user_id = "test_valid_paths";
    cleanup_user(&pool, user_id).await;

    let workspace = Workspace::new(user_id, pool.clone());

    // These should all work fine
    workspace.write("simple.md", "content").await.expect("simple write");
    workspace.write("nested/path/file.md", "content").await.expect("nested write");
    workspace.write("dots.in.name.md", "content").await.expect("dots in name");
    workspace.write("path-with-dashes/file.md", "content").await.expect("dashes");
    workspace.write("path_with_underscores/file.md", "content").await.expect("underscores");

    // Read them back
    workspace.read("simple.md").await.expect("simple read");
    workspace.read("nested/path/file.md").await.expect("nested read");

    // List directories
    workspace.list("").await.expect("list root");
    workspace.list("nested").await.expect("list nested");
    workspace.list("nested/path").await.expect("list deep nested");

    cleanup_user(&pool, user_id).await;
}

#[tokio::test]
async fn test_path_normalization() {
    let pool = get_pool();
    let user_id = "test_normalization";
    cleanup_user(&pool, user_id).await;

    let workspace = Workspace::new(user_id, pool.clone());

    // Write with various path formats
    workspace.write("/leading/slash.md", "content").await.expect("leading slash");
    
    // Should be accessible without leading slash
    let doc = workspace.read("leading/slash.md").await.expect("read normalized");
    assert_eq!(doc.content, "content");

    // Double slashes should be collapsed
    workspace.write("double//slash.md", "content2").await.expect("double slash");
    let doc = workspace.read("double/slash.md").await.expect("read collapsed");
    assert_eq!(doc.content, "content2");

    cleanup_user(&pool, user_id).await;
}
