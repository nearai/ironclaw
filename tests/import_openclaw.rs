//! Integration tests for OpenClaw import functionality.

#![cfg(feature = "migrate")]

#[cfg(feature = "migrate")]
mod import_tests {
    use ironclaw::import::openclaw::reader::{OpenClawConfig, OpenClawMemoryChunk};
    use ironclaw::import::{ImportError, ImportStats};

    #[test]
    fn test_import_stats_is_empty() {
        let stats = ImportStats::default();
        assert!(stats.is_empty());
        assert_eq!(stats.total_imported(), 0);
    }

    #[test]
    fn test_import_stats_total_imported() {
        let stats = ImportStats {
            documents: 5,
            chunks: 10,
            conversations: 2,
            messages: 50,
            settings: 3,
            secrets: 1,
            ..ImportStats::default()
        };

        assert!(!stats.is_empty());
        assert_eq!(stats.total_imported(), 71);
    }

    #[test]
    fn test_import_error_display() {
        let err = ImportError::ConfigParse("test error".to_string());
        assert_eq!(err.to_string(), "JSON5 parse error: test error");

        let err = ImportError::Database("db error".to_string());
        assert_eq!(err.to_string(), "Database error: db error");
    }

    #[test]
    fn test_openclaw_config_construction() {
        let config = OpenClawConfig {
            llm: None,
            embeddings: None,
            other_settings: std::collections::HashMap::new(),
        };

        assert!(config.llm.is_none());
        assert!(config.embeddings.is_none());
        assert!(config.other_settings.is_empty());
    }

    #[test]
    fn test_memory_chunk_construction() {
        let chunk = OpenClawMemoryChunk {
            path: "test/doc.md".to_string(),
            content: "Test content".to_string(),
            embedding: Some(vec![0.1, 0.2, 0.3]),
            chunk_index: 0,
        };

        assert_eq!(chunk.path, "test/doc.md");
        assert_eq!(chunk.content, "Test content");
        assert!(chunk.embedding.is_some());
        assert_eq!(chunk.chunk_index, 0);
    }
}
