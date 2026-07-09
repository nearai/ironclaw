//! Per-turn semantic tool retrieval: rank tools by similarity to the message.

/// Cosine similarity. Returns 0.0 if the vectors differ in length or are empty/zero.
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let mut dot = 0.0f32;
    let mut na = 0.0f32;
    let mut nb = 0.0f32;
    for (ai, bi) in a.iter().zip(b.iter()) {
        dot += ai * bi;
        na += ai * ai;
        nb += bi * bi;
    }
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    dot / (na.sqrt() * nb.sqrt())
}

/// Rank `items` (key, vector) by cosine to `query`; keep score >= `min_score`,
/// highest first, capped at `k`. Returns the keys.
pub fn rank_top_k(
    query: &[f32],
    items: &[(String, Vec<f32>)],
    k: usize,
    min_score: f32,
) -> Vec<String> {
    let mut scored: Vec<(&String, f32)> = items
        .iter()
        .map(|(key, vec)| (key, cosine(query, vec)))
        .filter(|(_, s)| *s >= min_score)
        .collect();
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scored
        .into_iter()
        .take(k)
        .map(|(key, _)| key.clone())
        .collect()
}

use ironclaw_embeddings::{EmbeddingError, EmbeddingProvider};
use ironclaw_llm::ToolDefinition;

/// Per-turn tool selector: embeds each tool once, then ranks tools against the
/// incoming message and returns `core ∪ top-K` tool names.
pub struct ToolRetriever {
    index: Vec<(String, Vec<f32>)>, // (tool name, embedding of "name: description")
    core: Vec<String>,
    top_k: usize,
    min_score: f32,
}

impl ToolRetriever {
    /// Build the index by embedding `"{name}: {description}"` for each tool.
    pub async fn build(
        defs: &[ToolDefinition],
        core: Vec<String>,
        top_k: usize,
        min_score: f32,
        embed: &dyn EmbeddingProvider,
    ) -> Result<Self, EmbeddingError> {
        let texts: Vec<String> = defs
            .iter()
            .map(|d| format!("{}: {}", d.name, d.description))
            .collect();
        let vectors = embed.embed_batch(&texts).await?;
        let index = defs.iter().map(|d| d.name.clone()).zip(vectors).collect();
        Ok(Self {
            index,
            core,
            top_k,
            min_score,
        })
    }

    /// Return `core ∪ top-K` tool names for `message` (deduplicated, core first).
    pub async fn select(
        &self,
        message: &str,
        embed: &dyn EmbeddingProvider,
    ) -> Result<Vec<String>, EmbeddingError> {
        let query = embed.embed(message).await?;
        let mut names = self.core.clone();
        for name in rank_top_k(&query, &self.index, self.top_k, self.min_score) {
            if !names.contains(&name) {
                names.push(name);
            }
        }
        Ok(names)
    }
}

use std::collections::HashSet;

/// Narrow an already-policy-filtered tool `baseline` down to `core ∪ top-K` for this
/// message. Fails toward capability: when retrieval is disabled, unconfigured, or errors,
/// the full `baseline` is returned unchanged (never fewer tools than today).
pub async fn narrow_tools(
    baseline: Vec<ToolDefinition>,
    retriever: Option<&ToolRetriever>,
    embeddings: Option<&dyn EmbeddingProvider>,
    enabled: bool,
    message: Option<&str>,
) -> Vec<ToolDefinition> {
    // If any precondition is missing, return baseline unchanged.
    if !enabled {
        return baseline;
    }
    let (retriever, embeddings, message) = match (retriever, embeddings, message) {
        (Some(r), Some(e), Some(m)) => (r, e, m),
        // silent-ok: retrieval disabled/unconfigured, fail toward full capability
        _ => return baseline,
    };
    match retriever.select(message, embeddings).await {
        Ok(names) => {
            let keep: HashSet<&str> = names.iter().map(String::as_str).collect();
            baseline
                .into_iter()
                .filter(|d| keep.contains(d.name.as_str()))
                .collect()
        }
        Err(e) => {
            tracing::warn!("tool retrieval failed ({e}); using all tools");
            baseline
        }
    }
}

use crate::config::RetrievalConfig;
use crate::tools::registry::ToolRegistry;
use std::sync::Arc;

/// Build a `ToolRetriever` from the registry's current tool set, or `None` when retrieval is
/// disabled, no embeddings provider is available, or the build fails (fail toward all-tools).
pub async fn build_if_enabled(
    registry: &ToolRegistry,
    embeddings: Option<&Arc<dyn EmbeddingProvider>>,
    config: &RetrievalConfig,
) -> Option<Arc<ToolRetriever>> {
    if !config.enabled {
        return None;
    }
    let embed = embeddings?;
    let defs = registry.tool_definitions().await;
    match ToolRetriever::build(
        &defs,
        config.core_set.clone(),
        config.top_k,
        config.min_score,
        embed.as_ref(),
    )
    .await
    {
        Ok(r) => Some(Arc::new(r)),
        Err(e) => {
            tracing::warn!("tool retriever build failed ({e}); retrieval disabled this run");
            None
        }
    }
}

use ironclaw_host_api::runtime_policy::EffectiveRuntimePolicy;

/// Compute the model-facing tool list: policy-visible baseline, narrowed by retrieval.
/// This is the single seam every injection site will call in Task 4b.
pub async fn resolve_available_tools(
    registry: &ToolRegistry,
    policy: Option<&EffectiveRuntimePolicy>,
    retriever: Option<&ToolRetriever>,
    embeddings: Option<&dyn EmbeddingProvider>,
    enabled: bool,
    message: Option<&str>,
) -> Vec<ToolDefinition> {
    let baseline = match policy {
        Some(p) => registry.tool_definitions_visible_under(p).await,
        None => registry.tool_definitions().await,
    };
    narrow_tools(baseline, retriever, embeddings, enabled, message).await
}

#[cfg(test)]
mod tests {
    use super::*;

    use async_trait::async_trait;
    use ironclaw_embeddings::{EmbeddingError, EmbeddingProvider};
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

    #[tokio::test]
    async fn retriever_selects_relevant_plus_core() {
        use ironclaw_llm::ToolDefinition;
        let embed = KeywordEmbeddings;
        let defs = vec![
            ToolDefinition {
                name: "create_trip".to_string(),
                description: "plan a trip / travel itinerary".to_string(),
                parameters: serde_json::json!({}),
            },
            ToolDefinition {
                name: "ocr_image".to_string(),
                description: "read text from an image".to_string(),
                parameters: serde_json::json!({}),
            },
            ToolDefinition {
                name: "memory_search".to_string(),
                description: "search memory".to_string(),
                parameters: serde_json::json!({}),
            },
        ];
        // top_k=1, floor=-1.0 => exactly the single best-ranked tool, plus the core set.
        let r = ToolRetriever::build(&defs, vec!["memory_search".to_string()], 1, -1.0, &embed)
            .await
            .expect("build should succeed");
        let picked = r
            .select("plan a trip to Tokyo", &embed)
            .await
            .expect("select should succeed");
        assert!(picked.contains(&"memory_search".to_string())); // core always present
        assert!(picked.contains(&"create_trip".to_string())); // relevant retrieved (axis 0)
        assert!(!picked.contains(&"ocr_image".to_string())); // irrelevant excluded (k=1)
    }

    #[test]
    fn cosine_and_ranking() {
        // orthogonal -> 0, identical -> 1
        assert!((cosine(&[1.0, 0.0], &[0.0, 1.0])).abs() < 1e-6);
        assert!((cosine(&[1.0, 1.0], &[1.0, 1.0]) - 1.0).abs() < 1e-6);
        let items = vec![
            ("trip".to_string(), vec![1.0, 0.0]),
            ("ocr".to_string(), vec![0.0, 1.0]),
            ("place".to_string(), vec![0.9, 0.1]),
        ];
        // query aligned with the "trip"/"place" axis; k=2, floor 0.5
        let got = rank_top_k(&[1.0, 0.0], &items, 2, 0.5);
        assert_eq!(got, vec!["trip".to_string(), "place".to_string()]);
    }
    #[test]
    fn min_score_floor_excludes_weak_and_k_caps() {
        let items = vec![
            ("a".to_string(), vec![1.0, 0.0]),
            ("b".to_string(), vec![0.2, 0.98]),
        ];
        assert_eq!(
            rank_top_k(&[1.0, 0.0], &items, 5, 0.5),
            vec!["a".to_string()]
        );
    }

    fn three_tool_baseline() -> Vec<ToolDefinition> {
        vec![
            ToolDefinition {
                name: "create_trip".to_string(),
                description: "plan a trip / travel itinerary".to_string(),
                parameters: serde_json::json!({}),
            },
            ToolDefinition {
                name: "ocr_image".to_string(),
                description: "read text from an image".to_string(),
                parameters: serde_json::json!({}),
            },
            ToolDefinition {
                name: "memory_search".to_string(),
                description: "search memory".to_string(),
                parameters: serde_json::json!({}),
            },
        ]
    }

    #[tokio::test]
    async fn narrow_tools_narrows_to_core_plus_relevant() {
        let embed = KeywordEmbeddings;
        let baseline = three_tool_baseline();
        let r = ToolRetriever::build(
            &baseline,
            vec!["memory_search".to_string()],
            1,
            -1.0,
            &embed,
        )
        .await
        .expect("build should succeed");

        let result = narrow_tools(
            baseline,
            Some(&r),
            Some(&embed),
            true,
            Some("plan a trip to Tokyo"),
        )
        .await;

        let names: Vec<&str> = result.iter().map(|d| d.name.as_str()).collect();
        assert!(names.contains(&"create_trip"));
        assert!(names.contains(&"memory_search"));
        assert!(!names.contains(&"ocr_image"));
    }

    #[tokio::test]
    async fn narrow_tools_disabled_returns_baseline() {
        let embed = KeywordEmbeddings;
        let baseline = three_tool_baseline();
        let r = ToolRetriever::build(
            &baseline,
            vec!["memory_search".to_string()],
            1,
            -1.0,
            &embed,
        )
        .await
        .expect("build should succeed");

        let result = narrow_tools(
            three_tool_baseline(),
            Some(&r),
            Some(&embed),
            false,
            Some("plan a trip to Tokyo"),
        )
        .await;

        let names: Vec<&str> = result.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names.len(), 3);
        assert!(names.contains(&"create_trip"));
        assert!(names.contains(&"ocr_image"));
        assert!(names.contains(&"memory_search"));
    }

    #[tokio::test]
    async fn build_if_enabled_disabled_is_none() {
        let registry = crate::tools::registry::ToolRegistry::new();
        let config = crate::config::RetrievalConfig {
            enabled: false,
            ..Default::default()
        };

        let result = build_if_enabled(&registry, None, &config).await;

        assert!(result.is_none());
    }
}
