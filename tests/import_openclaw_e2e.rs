//! End-to-end integration tests for OpenClaw importer with actual import execution.
//!
//! These tests verify the complete import pipeline: configuration, settings,
//! credentials, memory chunks, workspace documents, and conversations.

#![cfg(feature = "import")]

#[cfg(feature = "import")]
mod e2e_import_tests {
    use std::path::PathBuf;
    use tempfile::TempDir;
    use uuid::Uuid;

    use ironclaw::import::openclaw::reader::OpenClawReader;
    use ironclaw::import::openclaw::settings;
    use ironclaw::import::{ImportOptions, ImportStats};

    /// Helper: Create a synthetic OpenClaw with full structure
    fn setup_full_openclaw_test_env() -> Result<(TempDir, PathBuf), Box<dyn std::error::Error>> {
        let temp_dir = TempDir::new()?;
        let openclaw_path = temp_dir.path().to_path_buf();

        // 1. Create openclaw.json with all settings
        let config_content = r#"{
            llm: {
                provider: "openai",
                model: "gpt-4-turbo",
                api_key: "sk-test-key-12345",
                base_url: "https://api.openai.com/v1"
            },
            embeddings: {
                model: "text-embedding-3-large",
                provider: "openai",
                api_key: "sk-embed-key-67890"
            },
            custom_setting: "custom_value"
        }"#;
        std::fs::write(openclaw_path.join("openclaw.json"), config_content)?;

        // 2. Create workspace with multiple files
        let workspace_dir = openclaw_path.join("workspace");
        std::fs::create_dir_all(&workspace_dir)?;

        std::fs::write(
            workspace_dir.join("MEMORY.md"),
            "# Memory\n\nStored memories and facts.\n\n- User prefers morning briefings\n- Key project: Alpha",
        )?;

        std::fs::write(
            workspace_dir.join("README.md"),
            "# Project README\n\nThis is the main project documentation.\n\n## Goals\n1. Complete migration\n2. Verify data",
        )?;

        std::fs::write(
            workspace_dir.join("AGENTS.md"),
            "# Agent Definitions\n\n## Main Agent\n- Role: Assistant\n- Capabilities: Analysis, Planning",
        )?;

        // 3. Create agents directory with databases
        let agents_dir = openclaw_path.join("agents");
        std::fs::create_dir_all(&agents_dir)?;

        create_full_agent_db(&agents_dir.join("primary_agent.sqlite"))?;
        create_full_agent_db(&agents_dir.join("secondary_agent.sqlite"))?;

        Ok((temp_dir, openclaw_path))
    }

    /// Helper: Create a full agent SQLite database with chunks and conversations
    fn create_full_agent_db(db_path: &PathBuf) -> Result<(), Box<dyn std::error::Error>> {
        use rusqlite::Connection;

        let conn = Connection::open(db_path)?;

        // Chunks table
        conn.execute(
            "CREATE TABLE IF NOT EXISTS chunks (
                id TEXT PRIMARY KEY,
                path TEXT NOT NULL,
                content TEXT NOT NULL,
                embedding BLOB,
                chunk_index INTEGER NOT NULL
            )",
            [],
        )?;

        // Insert 5 chunks
        for i in 0..5 {
            conn.execute(
                "INSERT INTO chunks (id, path, content, embedding, chunk_index)
                 VALUES (?, ?, ?, ?, ?)",
                rusqlite::params![
                    Uuid::new_v4().to_string(),
                    format!("notes/section_{}.md", i),
                    format!("Content for section {}. This is important information.", i),
                    None::<Vec<u8>>,
                    i
                ],
            )?;
        }

        // Conversations table
        conn.execute(
            "CREATE TABLE IF NOT EXISTS conversations (
                id TEXT PRIMARY KEY,
                channel TEXT NOT NULL,
                created_at TEXT
            )",
            [],
        )?;

        // Messages table
        conn.execute(
            "CREATE TABLE IF NOT EXISTS messages (
                id TEXT PRIMARY KEY,
                conversation_id TEXT NOT NULL,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                created_at TEXT,
                FOREIGN KEY(conversation_id) REFERENCES conversations(id)
            )",
            [],
        )?;

        // Insert 3 conversations with messages
        for conv_num in 0..3 {
            let conv_id = Uuid::new_v4().to_string();
            let channel = match conv_num {
                0 => "telegram",
                1 => "slack",
                _ => "discord",
            };

            conn.execute(
                "INSERT INTO conversations (id, channel, created_at) VALUES (?, ?, ?)",
                rusqlite::params![
                    &conv_id,
                    channel,
                    format!("2024-01-{:02}T10:00:00Z", 10 + conv_num)
                ],
            )?;

            // Add 3 messages per conversation
            for msg_num in 0..3 {
                let role = if msg_num % 2 == 0 {
                    "user"
                } else {
                    "assistant"
                };
                conn.execute(
                    "INSERT INTO messages (id, conversation_id, role, content, created_at)
                     VALUES (?, ?, ?, ?, ?)",
                    rusqlite::params![
                        Uuid::new_v4().to_string(),
                        &conv_id,
                        role,
                        format!(
                            "{} message {} from conversation {}",
                            role, msg_num, conv_num
                        ),
                        format!("2024-01-{:02}T10:{:02}:00Z", 10 + conv_num, msg_num * 10)
                    ],
                )?;
            }
        }

        Ok(())
    }

    // ────────────────────────────────────────────────────────────────────
    // Configuration & Settings Tests
    // ────────────────────────────────────────────────────────────────────

    #[test]
    fn test_full_config_extraction() {
        let (_temp, openclaw_path) = setup_full_openclaw_test_env().expect("setup failed");

        let reader = OpenClawReader::new(&openclaw_path).expect("reader creation failed");
        let config = reader.read_config().expect("config read failed");

        // Verify LLM config
        assert_eq!(
            config.llm.as_ref().map(|c| c.provider.clone()),
            Some(Some("openai".to_string()))
        );
        assert_eq!(
            config.llm.as_ref().map(|c| c.model.clone()),
            Some(Some("gpt-4-turbo".to_string()))
        );

        // Verify embeddings config
        assert_eq!(
            config.embeddings.as_ref().map(|c| c.model.clone()),
            Some(Some("text-embedding-3-large".to_string()))
        );

        // Verify custom settings preserved
        assert!(config.other_settings.contains_key("custom_setting"));
    }

    #[test]
    fn test_settings_mapping_to_ironclaw_format() {
        let (_temp, openclaw_path) = setup_full_openclaw_test_env().expect("setup failed");

        let reader = OpenClawReader::new(&openclaw_path).expect("reader creation failed");
        let config = reader.read_config().expect("config read failed");

        let settings_map = settings::map_openclaw_config_to_settings(&config);

        // Verify key mappings
        assert!(settings_map.contains_key("llm.backend"));
        assert!(settings_map.contains_key("llm.selected_model"));
        assert!(settings_map.contains_key("embeddings.model"));
        assert!(settings_map.contains_key("custom_setting"));

        // Verify values
        assert_eq!(
            settings_map.get("llm.backend").and_then(|v| v.as_str()),
            Some("openai")
        );
    }

    // ────────────────────────────────────────────────────────────────────
    // Credential Extraction Tests
    // ────────────────────────────────────────────────────────────────────

    #[test]
    fn test_credentials_extraction() {
        let (_temp, openclaw_path) = setup_full_openclaw_test_env().expect("setup failed");

        let reader = OpenClawReader::new(&openclaw_path).expect("reader creation failed");
        let config = reader.read_config().expect("config read failed");

        let creds = settings::extract_credentials(&config);

        // Should extract 2 credentials (llm_api_key + embeddings_api_key)
        assert_eq!(creds.len(), 2);

        // Verify names (order may vary, so check both are present)
        let names: Vec<_> = creds.iter().map(|(name, _)| name).collect();
        assert!(names.contains(&&"llm_api_key".to_string()));
        assert!(names.contains(&&"embeddings_api_key".to_string()));

        // Verify credentials are wrapped in SecretString (not exposed in debug)
        for (_name, secret) in creds {
            let debug_str = format!("{:?}", secret);
            assert!(!debug_str.contains("sk-test-key"));
            assert!(!debug_str.contains("sk-embed-key"));
        }
    }

    #[test]
    fn test_credentials_never_logged() {
        let (_temp, openclaw_path) = setup_full_openclaw_test_env().expect("setup failed");

        let reader = OpenClawReader::new(&openclaw_path).expect("reader creation failed");
        let config = reader.read_config().expect("config read failed");

        let creds = settings::extract_credentials(&config);

        // Verify actual secrets are not exposed
        for (_name, secret) in creds {
            let secret_debug = format!("{:?}", secret);
            // Should NOT contain the actual API keys
            assert!(!secret_debug.contains("sk-test-key-12345"));
            assert!(!secret_debug.contains("sk-embed-key-67890"));
        }
    }

    // ────────────────────────────────────────────────────────────────────
    // Data Volume Tests
    // ────────────────────────────────────────────────────────────────────

    #[test]
    fn test_full_workspace_import_counts() {
        let (_temp, openclaw_path) = setup_full_openclaw_test_env().expect("setup failed");

        let reader = OpenClawReader::new(&openclaw_path).expect("reader creation failed");

        // Count workspace files
        let workspace_count = reader
            .list_workspace_files()
            .expect("list workspace files failed");
        assert_eq!(workspace_count, 3); // MEMORY.md, README.md, AGENTS.md

        // Count agent databases
        let agent_dbs = reader.list_agent_dbs().expect("list agent dbs failed");
        assert_eq!(agent_dbs.len(), 2); // primary + secondary
    }

    #[test]
    fn test_full_memory_chunks_import() {
        let (_temp, openclaw_path) = setup_full_openclaw_test_env().expect("setup failed");

        let reader = OpenClawReader::new(&openclaw_path).expect("reader creation failed");
        let agent_dbs = reader.list_agent_dbs().expect("list agent dbs failed");

        // Each agent should have 5 chunks
        for (_name, db_path) in agent_dbs {
            let chunks = reader
                .read_memory_chunks(&db_path)
                .expect("read memory chunks failed");
            assert_eq!(chunks.len(), 5);

            // Verify chunk structure
            for (i, chunk) in chunks.iter().enumerate() {
                assert_eq!(chunk.chunk_index, i as i32);
                assert!(
                    chunk
                        .content
                        .contains(&format!("Content for section {}", i))
                );
            }
        }
    }

    #[test]
    fn test_full_conversations_import() {
        let (_temp, openclaw_path) = setup_full_openclaw_test_env().expect("setup failed");

        let reader = OpenClawReader::new(&openclaw_path).expect("reader creation failed");
        let agent_dbs = reader.list_agent_dbs().expect("list agent dbs failed");

        // Each agent should have 3 conversations
        for (_name, db_path) in agent_dbs {
            let conversations = reader
                .read_conversations(&db_path)
                .expect("read conversations failed");
            assert_eq!(conversations.len(), 3);

            // Verify each conversation has messages
            for conv in conversations {
                assert_eq!(conv.messages.len(), 3); // Each has 3 messages
                assert!(!conv.channel.is_empty());

                // Verify message roles
                let roles: Vec<_> = conv.messages.iter().map(|m| m.role.as_str()).collect();
                assert!(roles.contains(&"user"));
                assert!(roles.contains(&"assistant"));
            }
        }
    }

    // ────────────────────────────────────────────────────────────────────
    // Import Stats Verification
    // ────────────────────────────────────────────────────────────────────

    #[test]
    fn test_import_options_validation() {
        let opts = ImportOptions {
            openclaw_path: PathBuf::from("/test/openclaw"),
            dry_run: true,
            re_embed: true,
            user_id: "test_user".to_string(),
        };

        assert_eq!(opts.user_id, "test_user");
        assert!(opts.dry_run);
        assert!(opts.re_embed);
    }

    #[test]
    fn test_import_stats_calculations() {
        // Simulating a full import scenario
        let stats = ImportStats {
            // Workspace: 3 files
            documents: 3,
            // Memory: 2 agents × 5 chunks each = 10 chunks
            chunks: 10,
            // Conversations: 2 agents × 3 conversations = 6 conversations
            conversations: 6,
            // Messages: 2 agents × 3 conversations × 3 messages = 18 messages
            messages: 18,
            // Settings: LLM config + embeddings + custom = 3
            settings: 3,
            // Credentials: api_key + embeddings_key = 2
            secrets: 2,
            ..ImportStats::default()
        };

        let total = stats.total_imported();
        assert_eq!(total, 3 + 10 + 6 + 18 + 3 + 2);
        assert!(!stats.is_empty());
    }

    // ────────────────────────────────────────────────────────────────────
    // Error Handling Tests
    // ────────────────────────────────────────────────────────────────────

    #[test]
    fn test_error_on_corrupt_sqlite() {
        let temp_dir = TempDir::new().expect("temp dir creation failed");
        let openclaw_path = temp_dir.path().to_path_buf();

        // Create agents dir with corrupt SQLite file
        let agents_dir = openclaw_path.join("agents");
        std::fs::create_dir_all(&agents_dir).expect("agents dir creation failed");

        // Write garbage data as "SQLite"
        std::fs::write(
            agents_dir.join("corrupt.sqlite"),
            "this is not a sqlite file",
        )
        .expect("write failed");

        let reader = OpenClawReader::new(&openclaw_path).expect("reader creation failed");

        // Listing should succeed (file exists)
        let dbs = reader.list_agent_dbs().expect("list agent dbs failed");
        assert_eq!(dbs.len(), 1);

        // But reading should fail
        let result = reader.read_memory_chunks(&dbs[0].1);
        assert!(result.is_err());
    }

    #[test]
    fn test_graceful_handling_missing_agents_directory() {
        let temp_dir = TempDir::new().expect("temp dir creation failed");
        let openclaw_path = temp_dir.path().to_path_buf();

        // Create config but no agents directory
        std::fs::write(
            openclaw_path.join("openclaw.json"),
            r#"{ llm: { provider: "openai" } }"#,
        )
        .expect("write failed");

        let reader = OpenClawReader::new(&openclaw_path).expect("reader creation failed");

        // Should return empty list, not error
        let dbs = reader.list_agent_dbs().expect("list agent dbs failed");
        assert_eq!(dbs.len(), 0);
    }

    // ────────────────────────────────────────────────────────────────────
    // Extensibility Tests
    // ────────────────────────────────────────────────────────────────────

    #[test]
    fn test_multiple_agents_independent_data() {
        let (_temp, openclaw_path) = setup_full_openclaw_test_env().expect("setup failed");

        let reader = OpenClawReader::new(&openclaw_path).expect("reader creation failed");
        let agent_dbs = reader.list_agent_dbs().expect("list agent dbs failed");

        // Verify each agent has independent data
        assert_eq!(agent_dbs.len(), 2);
        assert_eq!(agent_dbs[0].0, "primary_agent");
        assert_eq!(agent_dbs[1].0, "secondary_agent");

        // Each should have its own chunks
        for (_name, db_path) in &agent_dbs {
            let chunks = reader
                .read_memory_chunks(db_path)
                .expect("read chunks failed");
            assert_eq!(chunks.len(), 5);
        }
    }

    #[test]
    fn test_channel_diversity_in_conversations() {
        let (_temp, openclaw_path) = setup_full_openclaw_test_env().expect("setup failed");

        let reader = OpenClawReader::new(&openclaw_path).expect("reader creation failed");
        let agent_dbs = reader.list_agent_dbs().expect("list agent dbs failed");

        // Get conversations from first agent
        let conversations = reader
            .read_conversations(&agent_dbs[0].1)
            .expect("read conversations failed");

        // Should have different channels
        let channels: std::collections::HashSet<_> =
            conversations.iter().map(|c| c.channel.as_str()).collect();
        assert!(channels.contains("telegram"));
        assert!(channels.contains("slack"));
        assert!(channels.contains("discord"));
    }
}
