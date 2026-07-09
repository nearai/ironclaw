//! Integration test for Task 4b: the `resolve_available_tools` seam
//! (the single call the interactive dispatcher path now uses at both
//! tool-assembly points) against a real `ToolRegistry`.
//!
//! This upgrades Task 4a's data-level unit tests (which exercise
//! `narrow_tools`/`ToolRetriever` directly against hand-built
//! `ToolDefinition` vectors) to a registry-backed, caller-level test: it
//! proves the seam correctly derives its baseline from a real
//! `ToolRegistry::tool_definitions()` call and narrows it via a real
//! `ToolRetriever`, and that disabling retrieval returns the full
//! baseline unchanged (fail toward capability).
//!
//! Run with: cargo test --features integration --test tool_retrieval_integration

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw::context::JobContext;
use ironclaw::tools::retrieval::{ToolRetriever, resolve_available_tools};
use ironclaw::tools::{Tool, ToolError, ToolOutput, ToolRegistry};
use ironclaw_embeddings::{EmbeddingError, EmbeddingProvider};

/// Minimal tool for registry construction in tests: name+description only,
/// `execute` is never called.
struct MinimalTool {
    name: &'static str,
    description: &'static str,
}

#[async_trait]
impl Tool for MinimalTool {
    fn name(&self) -> &str {
        self.name
    }

    fn description(&self) -> &str {
        self.description
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({})
    }

    async fn execute(
        &self,
        _params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        unreachable!("MinimalTool is never executed in this test")
    }
}

/// Same keyword-embedding stub used by `src/tools/retrieval.rs`'s unit
/// tests (dimension=3, keyword->axis): replicated here per the task brief
/// since the original isn't exported from `#[cfg(test)]`.
struct KeywordEmbeddings;

#[async_trait]
impl EmbeddingProvider for KeywordEmbeddings {
    fn dimension(&self) -> usize {
        3
    }

    fn model_name(&self) -> &str {
        "keyword-test"
    }

    fn max_input_length(&self) -> usize {
        10_000
    }

    async fn embed(&self, text: &str) -> Result<Vec<f32>, EmbeddingError> {
        let t = text.to_lowercase();
        // axis 0 = travel, 1 = image/ocr, 2 = everything else (incl. memory)
        let v = if t.contains("trip") || t.contains("travel") {
            [1.0, 0.0, 0.0]
        } else if t.contains("image") || t.contains("ocr") {
            [0.0, 1.0, 0.0]
        } else {
            [0.0, 0.0, 1.0]
        };
        Ok(v.to_vec())
    }
    // embed_batch uses the trait default (calls embed per item).
}

async fn three_tool_registry() -> ToolRegistry {
    let registry = ToolRegistry::new();
    registry
        .register(Arc::new(MinimalTool {
            name: "create_trip",
            description: "plan a trip / travel itinerary",
        }))
        .await;
    registry
        .register(Arc::new(MinimalTool {
            name: "ocr_image",
            description: "read text from an image",
        }))
        .await;
    registry
        .register(Arc::new(MinimalTool {
            name: "memory_search",
            description: "search memory",
        }))
        .await;
    registry
}

#[tokio::test]
async fn resolve_available_tools_narrows_to_core_plus_relevant() {
    let registry = three_tool_registry().await;

    let defs = registry.tool_definitions().await;
    assert_eq!(defs.len(), 3, "baseline = all registered tools");

    let r = ToolRetriever::build(
        &defs,
        vec!["memory_search".into()],
        1,
        -1.0,
        &KeywordEmbeddings,
    )
    .await
    .expect("retriever build should succeed");

    // Drive the seam the dispatcher now calls (policy=None so baseline=all).
    let narrowed = resolve_available_tools(
        &registry,
        None,
        Some(&r),
        Some(&KeywordEmbeddings),
        true,
        Some("plan a trip to Tokyo"),
    )
    .await;

    let names: Vec<&str> = narrowed.iter().map(|d| d.name.as_str()).collect();
    assert_eq!(
        narrowed.len(),
        2,
        "expected core + top-1 relevant, got {names:?}"
    );
    assert!(names.contains(&"create_trip"));
    assert!(names.contains(&"memory_search"));
    assert!(!names.contains(&"ocr_image"));
}

#[tokio::test]
async fn resolve_available_tools_disabled_returns_full_baseline() {
    let registry = three_tool_registry().await;
    let defs = registry.tool_definitions().await;

    let r = ToolRetriever::build(
        &defs,
        vec!["memory_search".into()],
        1,
        -1.0,
        &KeywordEmbeddings,
    )
    .await
    .expect("retriever build should succeed");

    // Disabled path returns the full baseline unchanged, even with a
    // configured retriever/embeddings — fail toward capability.
    let all = resolve_available_tools(
        &registry,
        None,
        Some(&r),
        Some(&KeywordEmbeddings),
        false,
        Some("plan a trip to Tokyo"),
    )
    .await;

    assert_eq!(all.len(), 3);
}
