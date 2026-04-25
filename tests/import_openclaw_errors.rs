//! Error handling and edge case tests for OpenClaw import.
//!
//! These tests verify proper error handling for:
//! - Missing/corrupt files
//! - Invalid configurations
//! - Database corruption
//! - Permission issues
//! - Edge cases in data

#![cfg(feature = "migrate")]

#[cfg(feature = "migrate")]
mod error_handling_tests {
    use std::path::PathBuf;
    use tempfile::TempDir;

    use ironclaw::import::ImportError;
    use ironclaw::import::openclaw::reader::OpenClawReader;

    // ────────────────────────────────────────────────────────────────────
    // Missing Directory Tests
    // ────────────────────────────────────────────────────────────────────

    #[test]
    fn test_error_nonexistent_openclaw_directory() {
        let nonexistent = PathBuf::from("/nonexistent/path/openclaw");
        let result = OpenClawReader::new(&nonexistent);

        assert!(result.is_err());
        if let Err(e) = result {
            match e {
                ImportError::NotFound { .. } => (), // Expected
                _ => panic!("Expected NotFound, got: {}", e),
            }
        }
    }

    #[test]
    fn test_error_empty_openclaw_directory() {
        let temp_dir = TempDir::new().expect("temp dir creation failed");
        let result = OpenClawReader::new(temp_dir.path());

        // Should succeed (directory exists)
        assert!(result.is_ok());

        let reader = result.unwrap();
        let config_result = reader.read_config();

        // But reading config should fail
        assert!(config_result.is_err());
    }

    // ────────────────────────────────────────────────────────────────────
    // Config File Errors
    // ────────────────────────────────────────────────────────────────────

    #[test]
    fn test_error_missing_openclaw_json() {
        let temp_dir = TempDir::new().expect("temp dir creation failed");
        let openclaw_path = temp_dir.path().to_path_buf();

        let reader = OpenClawReader::new(&openclaw_path).expect("reader creation failed");

        let result = reader.read_config();
        assert!(result.is_err());
    }

    #[test]
    fn test_error_invalid_json5_syntax() {
        let temp_dir = TempDir::new().expect("temp dir creation failed");
        let openclaw_path = temp_dir.path().to_path_buf();

        // Invalid JSON5: missing closing brace
        let bad_config = r#"{ llm: { provider: "openai" }"#;
        std::fs::write(openclaw_path.join("openclaw.json"), bad_config).expect("write failed");

        let reader = OpenClawReader::new(&openclaw_path).expect("reader creation failed");

        let result = reader.read_config();
        assert!(result.is_err());
    }

    #[test]
    fn test_error_truncated_json5() {
        let temp_dir = TempDir::new().expect("temp dir creation failed");
        let openclaw_path = temp_dir.path().to_path_buf();

        // Truncated JSON5
        std::fs::write(openclaw_path.join("openclaw.json"), "{").expect("write failed");

        let reader = OpenClawReader::new(&openclaw_path).expect("reader creation failed");

        let result = reader.read_config();
        assert!(result.is_err());
    }

    #[test]
    fn test_error_empty_openclaw_json() {
        let temp_dir = TempDir::new().expect("temp dir creation failed");
        let openclaw_path = temp_dir.path().to_path_buf();

        // Empty file
        std::fs::write(openclaw_path.join("openclaw.json"), "").expect("write failed");

        let reader = OpenClawReader::new(&openclaw_path).expect("reader creation failed");

        let result = reader.read_config();
        assert!(result.is_err());
    }

    // ────────────────────────────────────────────────────────────────────
    // SQLite Database Errors
    // ────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_error_corrupt_sqlite_file() {
        let temp_dir = TempDir::new().expect("temp dir creation failed");
        let openclaw_path = temp_dir.path().to_path_buf();

        let agents_dir = openclaw_path.join("agents");
        std::fs::create_dir_all(&agents_dir).expect("mkdir failed");

        // Write invalid SQLite data
        std::fs::write(
            agents_dir.join("bad.sqlite"),
            "this is definitely not a sqlite database",
        )
        .expect("write failed");

        let reader = OpenClawReader::new(&openclaw_path).expect("reader creation failed");

        let dbs = reader.list_agent_dbs().expect("list agent dbs failed");
        assert_eq!(dbs.len(), 1);

        // But reading should fail
        let result = reader.read_memory_chunks(&dbs[0].1).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_error_missing_chunks_table() {
        let temp_dir = TempDir::new().expect("temp dir creation failed");
        let openclaw_path = temp_dir.path().to_path_buf();

        let agents_dir = openclaw_path.join("agents");
        std::fs::create_dir_all(&agents_dir).expect("mkdir failed");

        let db_path = agents_dir.join("no_chunks.sqlite");

        // Create valid SQLite but without chunks table
        let db = libsql::Builder::new_local(&db_path)
            .build()
            .await
            .expect("db creation failed");
        let conn = db.connect().expect("connect failed");
        conn.execute(
            "CREATE TABLE metadata (key TEXT PRIMARY KEY, value TEXT)",
            (),
        )
        .await
        .expect("create table failed");

        let reader = OpenClawReader::new(&openclaw_path).expect("reader creation failed");

        let dbs = reader.list_agent_dbs().expect("list agent dbs failed");
        assert_eq!(dbs.len(), 1);

        // Should fail: chunks table doesn't exist
        let result = reader.read_memory_chunks(&dbs[0].1).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_error_missing_conversations_table() {
        let temp_dir = TempDir::new().expect("temp dir creation failed");
        let openclaw_path = temp_dir.path().to_path_buf();

        let agents_dir = openclaw_path.join("agents");
        std::fs::create_dir_all(&agents_dir).expect("mkdir failed");

        let db_path = agents_dir.join("no_conversations.sqlite");

        let db = libsql::Builder::new_local(&db_path)
            .build()
            .await
            .expect("db creation failed");
        let conn = db.connect().expect("connect failed");
        // Only create chunks table, not conversations
        conn.execute(
            "CREATE TABLE chunks (id TEXT, path TEXT, content TEXT, embedding BLOB, chunk_index INTEGER)",
            (),
        )
        .await
        .expect("create table failed");

        let reader = OpenClawReader::new(&openclaw_path).expect("reader creation failed");

        let dbs = reader.list_agent_dbs().expect("list agent dbs failed");
        assert_eq!(dbs.len(), 1);

        // Should fail: conversations table doesn't exist
        let result = reader.read_conversations(&dbs[0].1).await;
        assert!(result.is_err());
    }

    // ────────────────────────────────────────────────────────────────────
    // Edge Cases
    // ────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_edge_case_empty_chunks_table() {
        let temp_dir = TempDir::new().expect("temp dir creation failed");
        let openclaw_path = temp_dir.path().to_path_buf();

        let agents_dir = openclaw_path.join("agents");
        std::fs::create_dir_all(&agents_dir).expect("mkdir failed");

        let db_path = agents_dir.join("empty.sqlite");

        let db = libsql::Builder::new_local(&db_path)
            .build()
            .await
            .expect("db creation failed");
        let conn = db.connect().expect("connect failed");
        conn.execute(
            "CREATE TABLE chunks (id TEXT, path TEXT, content TEXT, embedding BLOB, chunk_index INTEGER)",
            (),
        )
        .await
        .expect("create table failed");

        let reader = OpenClawReader::new(&openclaw_path).expect("reader creation failed");

        let dbs = reader.list_agent_dbs().expect("list agent dbs failed");

        // Should succeed but return empty list
        let chunks = reader
            .read_memory_chunks(&dbs[0].1)
            .await
            .expect("read chunks failed");
        assert_eq!(chunks.len(), 0);
    }

    #[tokio::test]
    async fn test_edge_case_empty_conversations_table() {
        let temp_dir = TempDir::new().expect("temp dir creation failed");
        let openclaw_path = temp_dir.path().to_path_buf();

        let agents_dir = openclaw_path.join("agents");
        std::fs::create_dir_all(&agents_dir).expect("mkdir failed");

        let db_path = agents_dir.join("empty_conv.sqlite");

        let db = libsql::Builder::new_local(&db_path)
            .build()
            .await
            .expect("db creation failed");
        let conn = db.connect().expect("connect failed");
        conn.execute(
            "CREATE TABLE conversations (id TEXT, channel TEXT, created_at TEXT)",
            (),
        )
        .await
        .expect("create table failed");
        conn.execute(
            "CREATE TABLE messages (id TEXT, conversation_id TEXT, role TEXT, content TEXT, created_at TEXT)",
            (),
        )
        .await
        .expect("create table failed");

        let reader = OpenClawReader::new(&openclaw_path).expect("reader creation failed");

        let dbs = reader.list_agent_dbs().expect("list agent dbs failed");

        // Should succeed but return empty list
        let conversations = reader
            .read_conversations(&dbs[0].1)
            .await
            .expect("read conversations failed");
        assert_eq!(conversations.len(), 0);
    }

    #[tokio::test]
    async fn test_edge_case_very_large_content() {
        let temp_dir = TempDir::new().expect("temp dir creation failed");
        let openclaw_path = temp_dir.path().to_path_buf();

        let agents_dir = openclaw_path.join("agents");
        std::fs::create_dir_all(&agents_dir).expect("mkdir failed");

        let db_path = agents_dir.join("large.sqlite");

        let db = libsql::Builder::new_local(&db_path)
            .build()
            .await
            .expect("db creation failed");
        let conn = db.connect().expect("connect failed");
        conn.execute(
            "CREATE TABLE chunks (id TEXT, path TEXT, content TEXT, embedding BLOB, chunk_index INTEGER)",
            (),
        )
        .await
        .expect("create table failed");

        // Insert very large content (1MB)
        let large_content = "x".repeat(1024 * 1024);
        conn.execute(
            "INSERT INTO chunks VALUES (?, ?, ?, ?, ?)",
            libsql::params!["id1", "path", large_content, libsql::Value::Null, 0i64],
        )
        .await
        .expect("insert failed");

        let reader = OpenClawReader::new(&openclaw_path).expect("reader creation failed");

        let dbs = reader.list_agent_dbs().expect("list agent dbs failed");

        // Should still succeed
        let chunks = reader
            .read_memory_chunks(&dbs[0].1)
            .await
            .expect("read chunks failed");
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].content.len(), 1024 * 1024);
    }

    #[tokio::test]
    async fn test_edge_case_special_characters_in_content() {
        let temp_dir = TempDir::new().expect("temp dir creation failed");
        let openclaw_path = temp_dir.path().to_path_buf();

        let agents_dir = openclaw_path.join("agents");
        std::fs::create_dir_all(&agents_dir).expect("mkdir failed");

        let db_path = agents_dir.join("special.sqlite");

        let db = libsql::Builder::new_local(&db_path)
            .build()
            .await
            .expect("db creation failed");
        let conn = db.connect().expect("connect failed");
        conn.execute(
            "CREATE TABLE chunks (id TEXT, path TEXT, content TEXT, embedding BLOB, chunk_index INTEGER)",
            (),
        )
        .await
        .expect("create table failed");

        // Insert content with special characters
        let special_content = "Content with emoji \u{1f680} and UTF-8: \u{4e2d}\u{6587}, \u{0627}\u{0644}\u{0639}\u{0631}\u{0628}\u{064a}\u{0629}, \u{03b5}\u{03bb}\u{03bb}\u{03b7}\u{03bd}\u{03b9}\u{03ba}\u{03ac}";
        conn.execute(
            "INSERT INTO chunks VALUES (?, ?, ?, ?, ?)",
            libsql::params!["id1", "path", special_content, libsql::Value::Null, 0i64],
        )
        .await
        .expect("insert failed");

        let reader = OpenClawReader::new(&openclaw_path).expect("reader creation failed");

        let dbs = reader.list_agent_dbs().expect("list agent dbs failed");

        // Should handle special characters
        let chunks = reader
            .read_memory_chunks(&dbs[0].1)
            .await
            .expect("read chunks failed");
        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].content.contains("\u{1f680}"));
        assert!(chunks[0].content.contains("\u{4e2d}\u{6587}"));
    }

    #[tokio::test]
    async fn test_edge_case_null_values_in_fields() {
        let temp_dir = TempDir::new().expect("temp dir creation failed");
        let openclaw_path = temp_dir.path().to_path_buf();

        let agents_dir = openclaw_path.join("agents");
        std::fs::create_dir_all(&agents_dir).expect("mkdir failed");

        let db_path = agents_dir.join("nulls.sqlite");

        let db = libsql::Builder::new_local(&db_path)
            .build()
            .await
            .expect("db creation failed");
        let conn = db.connect().expect("connect failed");
        conn.execute(
            "CREATE TABLE conversations (id TEXT, channel TEXT, created_at TEXT)",
            (),
        )
        .await
        .expect("create table failed");
        conn.execute(
            "CREATE TABLE messages (id TEXT, conversation_id TEXT, role TEXT, content TEXT, created_at TEXT)",
            (),
        )
        .await
        .expect("create table failed");

        // Insert conversation with NULL created_at
        conn.execute(
            "INSERT INTO conversations VALUES (?, ?, ?)",
            libsql::params!["conv1", "telegram", libsql::Value::Null],
        )
        .await
        .expect("insert failed");

        // Insert message with NULL created_at
        conn.execute(
            "INSERT INTO messages VALUES (?, ?, ?, ?, ?)",
            libsql::params!["msg1", "conv1", "user", "hello", libsql::Value::Null],
        )
        .await
        .expect("insert failed");

        let reader = OpenClawReader::new(&openclaw_path).expect("reader creation failed");

        let dbs = reader.list_agent_dbs().expect("list agent dbs failed");

        // Should handle NULL timestamps gracefully
        let conversations = reader
            .read_conversations(&dbs[0].1)
            .await
            .expect("read conversations failed");
        assert_eq!(conversations.len(), 1);
        assert!(conversations[0].created_at.is_none());
        assert!(conversations[0].messages[0].created_at.is_none());
    }

    // ────────────────────────────────────────────────────────────────────
    // Workspace File Errors
    // ────────────────────────────────────────────────────────────────────

    #[test]
    fn test_error_workspace_not_directory() {
        let temp_dir = TempDir::new().expect("temp dir creation failed");
        let openclaw_path = temp_dir.path().to_path_buf();

        // Create "workspace" as a file, not a directory
        std::fs::write(openclaw_path.join("workspace"), "not a directory").expect("write failed");

        let reader = OpenClawReader::new(&openclaw_path).expect("reader creation failed");

        // Should handle gracefully (no files found)
        let count = reader
            .list_workspace_files()
            .expect("list workspace files failed");
        assert_eq!(count, 0);
    }

    #[test]
    fn test_edge_case_many_markdown_files() {
        let temp_dir = TempDir::new().expect("temp dir creation failed");
        let openclaw_path = temp_dir.path().to_path_buf();

        let workspace_dir = openclaw_path.join("workspace");
        std::fs::create_dir_all(&workspace_dir).expect("mkdir failed");

        // Create 100 markdown files
        for i in 0..100 {
            std::fs::write(workspace_dir.join(format!("doc_{}.md", i)), "content")
                .expect("write failed");
        }

        let reader = OpenClawReader::new(&openclaw_path).expect("reader creation failed");

        let count = reader
            .list_workspace_files()
            .expect("list workspace files failed");
        assert_eq!(count, 100);
    }
}
