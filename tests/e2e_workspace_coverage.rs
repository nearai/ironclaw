//! E2E trace tests: workspace persistence (#574).
//!
//! Covers chunking, multi-document search, hybrid search, directory tree,
//! document lifecycle (write/read/overwrite), and identity in system prompt.

#[cfg(feature = "libsql")]
mod support;

#[cfg(feature = "libsql")]
mod tests {
    use std::time::Duration;

    use crate::support::test_rig::TestRigBuilder;
    use crate::support::trace_llm::LlmTrace;

    // -----------------------------------------------------------------------
    // Test 1: write_chunk_search
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn write_chunk_search() {
        let trace = LlmTrace::from_file(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/llm_traces/workspace/write_chunk_search.json"
        ))
        .expect("failed to load write_chunk_search.json");

        let rig = TestRigBuilder::new()
            .with_trace(trace.clone())
            .build()
            .await;

        rig.send_message("Write a long architecture document and search it")
            .await;
        let responses = rig.wait_for_responses(1, Duration::from_secs(15)).await;

        rig.verify_trace_expects(&trace, &responses);

        // Verify the document was persisted via workspace.
        let ws = rig.workspace().expect("workspace must be available");
        let doc = ws
            .read("context/architecture.md")
            .await
            .expect("architecture.md should exist");
        assert!(
            doc.content.contains("Payment Service"),
            "Document should contain 'Payment Service'"
        );
        assert!(
            doc.content.len() > 1000,
            "Document should be long (>1000 chars), got {}",
            doc.content.len()
        );

        rig.shutdown();
    }

    // -----------------------------------------------------------------------
    // Test 2: multi_document_search
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn multi_document_search() {
        let trace = LlmTrace::from_file(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/llm_traces/workspace/multi_doc_search.json"
        ))
        .expect("failed to load multi_doc_search.json");

        let rig = TestRigBuilder::new()
            .with_trace(trace.clone())
            .build()
            .await;

        rig.send_message("Write three docs and search across them")
            .await;
        let responses = rig.wait_for_responses(1, Duration::from_secs(15)).await;

        rig.verify_trace_expects(&trace, &responses);

        // Verify all three documents were written.
        let ws = rig.workspace().expect("workspace must be available");
        let frontend = ws.read("context/frontend.md").await;
        let backend = ws.read("context/backend.md").await;
        let devops = ws.read("context/devops.md").await;
        assert!(frontend.is_ok(), "frontend.md should exist");
        assert!(backend.is_ok(), "backend.md should exist");
        assert!(devops.is_ok(), "devops.md should exist");

        rig.shutdown();
    }

    // -----------------------------------------------------------------------
    // Test 3: hybrid_search_with_embeddings
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn hybrid_search_with_embeddings() {
        let trace = LlmTrace::from_file(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/llm_traces/workspace/hybrid_search.json"
        ))
        .expect("failed to load hybrid_search.json");

        let rig = TestRigBuilder::new()
            .with_trace(trace.clone())
            .build()
            .await;

        rig.send_message("Write and semantically search for ML content")
            .await;
        let responses = rig.wait_for_responses(1, Duration::from_secs(15)).await;

        rig.verify_trace_expects(&trace, &responses);

        // Verify memory_search was used (even without real embeddings,
        // the FTS path should find keyword matches).
        let started = rig.tool_calls_started();
        assert!(
            started.contains(&"memory_search".to_string()),
            "memory_search should be called"
        );

        rig.shutdown();
    }

    // -----------------------------------------------------------------------
    // Test 4: directory_tree
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn directory_tree() {
        let trace = LlmTrace::from_file(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/llm_traces/workspace/directory_tree.json"
        ))
        .expect("failed to load directory_tree.json");

        let rig = TestRigBuilder::new()
            .with_trace(trace.clone())
            .build()
            .await;

        rig.send_message("Write files in a hierarchy and show the tree")
            .await;
        let responses = rig.wait_for_responses(1, Duration::from_secs(15)).await;

        rig.verify_trace_expects(&trace, &responses);

        // Verify tree tool was called.
        let started = rig.tool_calls_started();
        assert!(
            started.contains(&"memory_tree".to_string()),
            "memory_tree should be called: {started:?}"
        );

        // Verify the tree result contains expected entries.
        let results = rig.tool_results();
        let tree_results: Vec<_> = results
            .iter()
            .filter(|(name, _)| name == "memory_tree")
            .collect();
        assert!(!tree_results.is_empty(), "Expected memory_tree results");

        rig.shutdown();
    }

    // -----------------------------------------------------------------------
    // Test 5: document_lifecycle
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn document_lifecycle() {
        let trace = LlmTrace::from_file(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/llm_traces/workspace/doc_lifecycle.json"
        ))
        .expect("failed to load doc_lifecycle.json");

        let rig = TestRigBuilder::new()
            .with_trace(trace.clone())
            .build()
            .await;

        rig.send_message("Write, read, overwrite, and read a document")
            .await;
        let responses = rig.wait_for_responses(1, Duration::from_secs(15)).await;

        rig.verify_trace_expects(&trace, &responses);

        // Verify the document has the updated content.
        let ws = rig.workspace().expect("workspace must be available");
        let doc = ws
            .read("context/lifecycle.md")
            .await
            .expect("lifecycle.md should exist");
        assert!(
            doc.content.contains("Version 2"),
            "Document should contain 'Version 2', got: {:?}",
            doc.content
        );

        // memory_write and memory_read should each be called twice.
        let started = rig.tool_calls_started();
        let write_count = started
            .iter()
            .filter(|n| n.as_str() == "memory_write")
            .count();
        let read_count = started
            .iter()
            .filter(|n| n.as_str() == "memory_read")
            .count();
        assert_eq!(write_count, 2, "Expected 2 memory_write calls");
        assert_eq!(read_count, 2, "Expected 2 memory_read calls");

        rig.shutdown();
    }

    // -----------------------------------------------------------------------
    // Test 6: identity_in_system_prompt
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn identity_in_system_prompt() {
        let trace = LlmTrace::from_file(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/llm_traces/workspace/identity_prompt.json"
        ))
        .expect("failed to load identity_prompt.json");

        let rig = TestRigBuilder::new()
            .with_trace(trace.clone())
            .build()
            .await;

        rig.send_message("Who are you?").await;
        let responses = rig.wait_for_responses(1, Duration::from_secs(15)).await;

        rig.verify_trace_expects(&trace, &responses);

        // Verify the TraceLlm captured requests include a system message.
        let trace_llm = rig.trace_llm().expect("trace_llm must be available");
        let captured = trace_llm.captured_requests();
        assert!(
            !captured.is_empty(),
            "Expected at least one captured request"
        );
        // The first request should have a system message with identity content.
        let first_request = &captured[0];
        let has_system = first_request
            .iter()
            .any(|msg| matches!(msg.role, ironclaw::llm::Role::System));
        assert!(has_system, "Expected a system message in the first request");

        rig.shutdown();
    }
}
