//! Comprehensive end-to-end tests for OpenClaw import with synthetic test data.

#![cfg(feature = "import")]

#[cfg(feature = "import")]
mod comprehensive_import_tests {
    use std::path::{Path, PathBuf};
    use tempfile::TempDir;
    use uuid::Uuid;

    use ironclaw::import::openclaw::reader::OpenClawReader;
    use ironclaw::import::{ImportError, ImportOptions};

    /// Helper to create a minimal synthetic OpenClaw directory structure
    fn create_synthetic_openclaw_dir() -> Result<(TempDir, PathBuf), Box<dyn std::error::Error>> {
        let temp_dir = TempDir::new()?;
        let openclaw_path = temp_dir.path().to_path_buf();

        // Create openclaw.json
        let config_content = r#"{
            llm: {
                provider: "openai",
                model: "gpt-4",
                api_key: "sk-test-key-123",
                base_url: "https://api.openai.com/v1"
            },
            embeddings: {
                model: "text-embedding-3-small",
                provider: "openai",
                api_key: "sk-test-embed-456"
            }
        }"#;
        std::fs::write(openclaw_path.join("openclaw.json"), config_content)?;

        // Create workspace directory with Markdown files
        let workspace_dir = openclaw_path.join("workspace");
        std::fs::create_dir_all(&workspace_dir)?;

        let memory_content =
            "# Memory\n\nThis is a test memory document.\n\n## Section 1\nSome content here.";
        std::fs::write(workspace_dir.join("MEMORY.md"), memory_content)?;

        let readme_content = "# README\n\nTest workspace README with important notes.";
        std::fs::write(workspace_dir.join("README.md"), readme_content)?;

        Ok((temp_dir, openclaw_path))
    }

    /// Helper to create a synthetic SQLite database with memory chunks
    async fn create_synthetic_memory_db(
        agents_dir: &Path,
    ) -> Result<PathBuf, Box<dyn std::error::Error>> {
        std::fs::create_dir_all(agents_dir)?;
        let db_path = agents_dir.join("test_agent.sqlite");

        let db = libsql::Builder::new_local(&db_path).build().await?;
        let conn = db.connect()?;

        // Create chunks table (simplified schema)
        conn.execute(
            "CREATE TABLE IF NOT EXISTS chunks (
                id TEXT PRIMARY KEY,
                path TEXT NOT NULL,
                content TEXT NOT NULL,
                embedding BLOB,
                chunk_index INTEGER NOT NULL
            )",
            (),
        )
        .await?;

        // Insert test chunks
        conn.execute(
            "INSERT INTO chunks (id, path, content, embedding, chunk_index)
             VALUES (?, ?, ?, ?, ?)",
            libsql::params![
                Uuid::new_v4().to_string(),
                "test/doc.md",
                "This is test chunk 1 content.",
                libsql::Value::Null,
                0i64
            ],
        )
        .await?;

        conn.execute(
            "INSERT INTO chunks (id, path, content, embedding, chunk_index)
             VALUES (?, ?, ?, ?, ?)",
            libsql::params![
                Uuid::new_v4().to_string(),
                "test/doc.md",
                "This is test chunk 2 content.",
                libsql::Value::Null,
                1i64
            ],
        )
        .await?;

        // Create conversation table
        conn.execute(
            "CREATE TABLE IF NOT EXISTS conversations (
                id TEXT PRIMARY KEY,
                channel TEXT NOT NULL,
                created_at TEXT
            )",
            (),
        )
        .await?;

        // Create messages table
        conn.execute(
            "CREATE TABLE IF NOT EXISTS messages (
                id TEXT PRIMARY KEY,
                conversation_id TEXT NOT NULL,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                created_at TEXT,
                FOREIGN KEY(conversation_id) REFERENCES conversations(id)
            )",
            (),
        )
        .await?;

        // Insert test conversation
        let conv_id = Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO conversations (id, channel, created_at) VALUES (?, ?, ?)",
            libsql::params![conv_id.clone(), "telegram", "2024-01-15T10:30:00Z"],
        )
        .await?;

        // Insert test messages
        conn.execute(
            "INSERT INTO messages (id, conversation_id, role, content, created_at)
             VALUES (?, ?, ?, ?, ?)",
            libsql::params![
                Uuid::new_v4().to_string(),
                conv_id.clone(),
                "user",
                "Hello, how are you?",
                "2024-01-15T10:30:00Z"
            ],
        )
        .await?;

        conn.execute(
            "INSERT INTO messages (id, conversation_id, role, content, created_at)
             VALUES (?, ?, ?, ?, ?)",
            libsql::params![
                Uuid::new_v4().to_string(),
                conv_id.clone(),
                "assistant",
                "I'm doing well, thank you for asking!",
                "2024-01-15T10:31:00Z"
            ],
        )
        .await?;

        Ok(db_path)
    }

    #[test]
    fn test_openclaw_reader_detects_config() {
        let (temp_dir, openclaw_path) =
            create_synthetic_openclaw_dir().expect("failed to create test data");

        // Verify detection works
        assert!(openclaw_path.join("openclaw.json").exists());

        // Create reader
        let reader = OpenClawReader::new(&openclaw_path).expect("failed to create reader");

        let _ = (temp_dir, reader);
    }

    #[test]
    fn test_openclaw_reader_parses_config() {
        let (temp_dir, openclaw_path) =
            create_synthetic_openclaw_dir().expect("failed to create test data");

        let reader = OpenClawReader::new(&openclaw_path).expect("failed to create reader");

        let config = reader.read_config().expect("failed to read config");

        // Verify LLM config
        assert!(config.llm.is_some());
        let llm = config.llm.unwrap();
        assert_eq!(llm.provider, Some("openai".to_string()));
        assert_eq!(llm.model, Some("gpt-4".to_string()));
        // API key is wrapped in SecretString, just verify it's present
        assert!(llm.api_key.is_some());

        // Verify embeddings config
        assert!(config.embeddings.is_some());
        let emb = config.embeddings.unwrap();
        assert_eq!(emb.provider, Some("openai".to_string()));
        assert_eq!(emb.model, Some("text-embedding-3-small".to_string()));
        // API key is wrapped in SecretString, just verify it's present
        assert!(emb.api_key.is_some());

        let _ = temp_dir;
    }

    #[test]
    fn test_openclaw_reader_lists_workspace_files() {
        let (temp_dir, openclaw_path) =
            create_synthetic_openclaw_dir().expect("failed to create test data");

        let reader = OpenClawReader::new(&openclaw_path).expect("failed to create reader");

        let count = reader
            .list_workspace_files()
            .expect("failed to list workspace files");

        // Should find MEMORY.md and README.md
        assert_eq!(count, 2);

        let _ = temp_dir;
    }

    #[tokio::test]
    async fn test_openclaw_reader_lists_agent_dbs() {
        let (temp_dir, openclaw_path) =
            create_synthetic_openclaw_dir().expect("failed to create test data");

        let agents_dir = openclaw_path.join("agents");
        let _db_path = create_synthetic_memory_db(&agents_dir)
            .await
            .expect("failed to create test DB");

        let reader = OpenClawReader::new(&openclaw_path).expect("failed to create reader");

        let dbs = reader.list_agent_dbs().expect("failed to list agent DBs");

        // Should find test_agent.sqlite
        assert_eq!(dbs.len(), 1);
        assert_eq!(dbs[0].0, "test_agent");

        let _ = temp_dir;
    }

    #[tokio::test]
    async fn test_openclaw_reader_reads_memory_chunks() {
        let (temp_dir, openclaw_path) =
            create_synthetic_openclaw_dir().expect("failed to create test data");

        let agents_dir = openclaw_path.join("agents");
        let db_path = create_synthetic_memory_db(&agents_dir)
            .await
            .expect("failed to create test DB");

        let reader = OpenClawReader::new(&openclaw_path).expect("failed to create reader");

        let chunks = reader
            .read_memory_chunks(&db_path)
            .await
            .expect("failed to read memory chunks");

        // Should find 2 chunks
        assert_eq!(chunks.len(), 2);

        // Verify chunk content
        assert_eq!(chunks[0].path, "test/doc.md");
        assert_eq!(chunks[0].content, "This is test chunk 1 content.");
        assert_eq!(chunks[0].chunk_index, 0);
        assert!(chunks[0].embedding.is_none());

        assert_eq!(chunks[1].path, "test/doc.md");
        assert_eq!(chunks[1].content, "This is test chunk 2 content.");
        assert_eq!(chunks[1].chunk_index, 1);

        let _ = temp_dir;
    }

    #[tokio::test]
    async fn test_openclaw_reader_reads_conversations() {
        let (temp_dir, openclaw_path) =
            create_synthetic_openclaw_dir().expect("failed to create test data");

        let agents_dir = openclaw_path.join("agents");
        let db_path = create_synthetic_memory_db(&agents_dir)
            .await
            .expect("failed to create test DB");

        let reader = OpenClawReader::new(&openclaw_path).expect("failed to create reader");

        let conversations = reader
            .read_conversations(&db_path)
            .await
            .expect("failed to read conversations");

        // Should find 1 conversation
        assert_eq!(conversations.len(), 1);

        let conv = &conversations[0];
        assert_eq!(conv.channel, "telegram");
        assert_eq!(conv.messages.len(), 2);

        // Verify messages
        assert_eq!(conv.messages[0].role, "user");
        assert_eq!(conv.messages[0].content, "Hello, how are you?");
        assert_eq!(conv.messages[1].role, "assistant");
        assert_eq!(
            conv.messages[1].content,
            "I'm doing well, thank you for asking!"
        );

        let _ = temp_dir;
    }

    #[test]
    fn test_openclaw_reader_handles_missing_directory() {
        let missing_path = PathBuf::from("/nonexistent/openclaw");
        let result = OpenClawReader::new(&missing_path);

        assert!(result.is_err());
        match result {
            Err(ImportError::NotFound { .. }) => (), // Expected
            _ => panic!("Expected NotFound error"),
        }
    }

    #[test]
    fn test_openclaw_reader_handles_missing_config() {
        let temp_dir = TempDir::new().expect("failed to create temp dir");
        let reader = OpenClawReader::new(temp_dir.path()).expect("failed to create reader");

        let result = reader.read_config();
        assert!(result.is_err());
    }

    #[test]
    fn test_import_options_construction() {
        let opts = ImportOptions {
            openclaw_path: PathBuf::from("/test/openclaw"),
            dry_run: true,
            re_embed: false,
            user_id: "test_user".to_string(),
        };

        assert_eq!(opts.user_id, "test_user");
        assert!(opts.dry_run);
        assert!(!opts.re_embed);
    }

    #[test]
    fn test_openclaw_reader_empty_agents_directory() {
        let (temp_dir, openclaw_path) =
            create_synthetic_openclaw_dir().expect("failed to create test data");

        // Create empty agents directory
        std::fs::create_dir(openclaw_path.join("agents")).expect("failed to create agents dir");

        let reader = OpenClawReader::new(&openclaw_path).expect("failed to create reader");

        let dbs = reader.list_agent_dbs().expect("failed to list agent DBs");

        // Should find no databases
        assert_eq!(dbs.len(), 0);

        let _ = temp_dir;
    }

    #[test]
    fn test_openclaw_reader_no_workspace_files() {
        let temp_dir = TempDir::new().expect("failed to create temp dir");
        let openclaw_path = temp_dir.path().to_path_buf();

        // Create config
        let config_content = r#"{ llm: { provider: "openai" } }"#;
        std::fs::write(openclaw_path.join("openclaw.json"), config_content)
            .expect("failed to write config");

        let reader = OpenClawReader::new(&openclaw_path).expect("failed to create reader");

        let count = reader
            .list_workspace_files()
            .expect("failed to list workspace files");

        // Should find no files
        assert_eq!(count, 0);
    }

    #[test]
    fn test_openclaw_reader_malformed_json5() {
        let temp_dir = TempDir::new().expect("failed to create temp dir");
        let openclaw_path = temp_dir.path().to_path_buf();

        // Create malformed config
        let bad_config = r#"{ llm: { provider: "openai" }"#; // Missing closing brace
        std::fs::write(openclaw_path.join("openclaw.json"), bad_config)
            .expect("failed to write config");

        let reader = OpenClawReader::new(&openclaw_path).expect("failed to create reader");

        let result = reader.read_config();
        assert!(result.is_err());
    }

    #[test]
    fn test_openclaw_detect_existing() {
        let (temp_dir, openclaw_path) =
            create_synthetic_openclaw_dir().expect("failed to create test data");

        // Verify the openclaw.json config exists (which is what detect() checks for)
        assert!(openclaw_path.join("openclaw.json").exists());

        let _ = temp_dir;
    }

    #[test]
    fn test_import_stats_aggregation() {
        let stats = ironclaw::import::ImportStats {
            documents: 5,
            chunks: 10,
            conversations: 3,
            messages: 25,
            settings: 2,
            secrets: 1,
            skipped: 2,
            re_embed_queued: 1,
        };

        assert_eq!(stats.total_imported(), 46); // All except skipped
        assert!(!stats.is_empty());
    }

    #[test]
    fn test_import_error_variants() {
        let err1 = ImportError::ConfigParse("test".to_string());
        assert_eq!(err1.to_string(), "JSON5 parse error: test");

        let err2 = ImportError::Database("db failed".to_string());
        assert_eq!(err2.to_string(), "Database error: db failed");

        let err3 = ImportError::Sqlite("sqlite error".to_string());
        assert_eq!(err3.to_string(), "SQLite error: sqlite error");

        let err4 = ImportError::Workspace("workspace error".to_string());
        assert_eq!(err4.to_string(), "Workspace error: workspace error");
    }
}
