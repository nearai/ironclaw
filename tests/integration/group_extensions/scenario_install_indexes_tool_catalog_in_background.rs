//! Installing an extension indexes the user's new tool catalog in the
//! background, so the next conversation's turn-0 selection embeds only its
//! opening request, never a tool document.
//!
//! The group ranks with a hybrid ranker whose dense side persists vectors,
//! and runs the planned runtime's tool-catalog indexer on the composed
//! runtime's active-registry change signal, as `build_reborn_runtime` does.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use ironclaw_loop_contracts::{
    ProviderToolDefinition, ToolCorpusOwner, ToolRetrievalError, ToolRetrievalIndex,
    ToolRetrievalProvider,
};
use serde_json::json;

use super::reborn_support::group::{HarnessResult, RebornIntegrationGroup};
use super::reborn_support::reply::RebornScriptedReply;

/// Words the stub embedder knows; each is one vector dimension.
const VOCABULARY: &[&str] = &["create", "issue", "comment", "pull", "list", "repo"];

/// Bag-of-words embedder recording every text it embeds.
#[derive(Default)]
pub struct RecordingEmbedder {
    embedded: Mutex<Vec<String>>,
}

impl RecordingEmbedder {
    pub fn embedded(&self) -> Vec<String> {
        self.embedded.lock().expect("embedder lock").clone()
    }

    fn documents(&self) -> usize {
        self.embedded()
            .iter()
            .filter(|text| text.starts_with("tool: "))
            .count()
    }
}

#[async_trait]
impl ironclaw_llm::embeddings::EmbeddingProvider for RecordingEmbedder {
    fn provider_id(&self) -> &str {
        "integration_stub"
    }

    fn model_name(&self) -> &str {
        "bag-of-words"
    }

    fn dimension(&self) -> Option<usize> {
        Some(VOCABULARY.len() + 1)
    }

    async fn embed(
        &self,
        texts: &[String],
    ) -> Result<Vec<Vec<f32>>, ironclaw_llm::embeddings::EmbeddingError> {
        self.embedded
            .lock()
            .expect("embedder lock")
            .extend(texts.iter().cloned());
        Ok(texts
            .iter()
            .map(|text| {
                let lower = text.to_lowercase();
                let words: Vec<&str> = lower
                    .split(|character: char| !character.is_ascii_alphanumeric())
                    .collect();
                VOCABULARY
                    .iter()
                    .map(|term| words.iter().filter(|word| *word == term).count() as f32)
                    .chain(std::iter::once(0.01))
                    .collect()
            })
            .collect())
    }
}

/// Delegates to the real ranker and records every background indexing
/// request (owner and capability ids), so the scenario can see the indexer
/// react to the install.
#[derive(Debug)]
pub struct IndexRequestRecorder {
    inner: Arc<dyn ToolRetrievalProvider>,
    requests: Mutex<Vec<(ToolCorpusOwner, Vec<String>)>>,
}

impl IndexRequestRecorder {
    fn indexed_capability(&self, prefix: &str) -> bool {
        self.requests
            .lock()
            .expect("requests lock")
            .iter()
            .any(|(_, ids)| ids.iter().any(|id| id.starts_with(prefix)))
    }
}

#[async_trait]
impl ToolRetrievalProvider for IndexRequestRecorder {
    fn ranker_version(&self) -> &str {
        self.inner.ranker_version()
    }

    async fn fit(
        &self,
        definitions: &[ProviderToolDefinition],
    ) -> Result<Arc<dyn ToolRetrievalIndex>, ToolRetrievalError> {
        self.inner.fit(definitions).await
    }

    async fn fit_for_owner(
        &self,
        owner: &ToolCorpusOwner,
        definitions: &[ProviderToolDefinition],
    ) -> Result<Arc<dyn ToolRetrievalIndex>, ToolRetrievalError> {
        self.inner.fit_for_owner(owner, definitions).await
    }

    fn index_in_background(&self, owner: &ToolCorpusOwner, definitions: &[ProviderToolDefinition]) {
        self.requests.lock().expect("requests lock").push((
            owner.clone(),
            definitions
                .iter()
                .map(|definition| definition.capability_id.as_str().to_string())
                .collect(),
        ));
        self.inner.index_in_background(owner, definitions);
    }
}

/// The ranker the group binds: hybrid over a persisting dense side.
pub fn ranker(embedder: &Arc<RecordingEmbedder>) -> Arc<IndexRequestRecorder> {
    let store = ironclaw_tool_retrieval::ToolVectorStoreSlot::new();
    store.bind(ironclaw_tool_retrieval::FilesystemToolVectorStore::new(
        ironclaw_composition::wrap_scoped(Arc::new(ironclaw_filesystem::InMemoryBackend::new())),
        ironclaw_tool_retrieval::DEFAULT_STORED_VECTORS_PER_OWNER,
    ));
    let dense = Arc::new(
        ironclaw_tool_retrieval::DenseToolRetrievalProvider::with_vector_store(
            Arc::clone(embedder) as Arc<dyn ironclaw_llm::embeddings::EmbeddingProvider>,
            store,
        ),
    );
    Arc::new(IndexRequestRecorder {
        inner: Arc::new(ironclaw_loop_host::HybridToolRetrieval::new(Some(dense))),
        requests: Mutex::new(Vec::new()),
    })
}

/// Semantic turn-start selection over the bound ranker.
pub fn semantic_selection() -> ironclaw_loop_host::ToolPrefetchConfig {
    ironclaw_loop_host::ToolPrefetchConfig::new(
        ironclaw_loop_host::ToolPrefetchRanking::Semantic,
        100,
        16_000,
        0.35,
        0.0,
        Vec::new(),
    )
    .expect("valid prefetch config")
}

pub async fn run(
    g: &RebornIntegrationGroup,
    embedder: &RecordingEmbedder,
    ranker: &IndexRequestRecorder,
) -> HarnessResult<()> {
    // ── Thread A: installs github ───────────────────────────────────────────
    let installer = g
        .thread("index-installer")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.extension_install",
                json!({"extension_id": "github"}),
            ),
            RebornScriptedReply::text("installed"),
        ])
        .build()
        .await?;
    installer
        .seed_capability_credential_account("github", "itest github ready path", &[])
        .await?;
    installer.submit_turn("install github").await?;
    installer
        .assert_tool_result_contains("\"phase\":\"active\"")
        .await?;

    // The install changed the active registry; the indexer derives the
    // user's catalog again and hands it (github tools included) to the
    // ranker in the background.
    wait_until("the indexer to request the new github tools", || {
        ranker.indexed_capability("github.")
    })
    .await?;
    // Let the background embedding settle.
    let mut settled = embedder.documents();
    loop {
        tokio::time::sleep(Duration::from_millis(300)).await;
        let now = embedder.documents();
        if now == settled {
            break;
        }
        settled = now;
    }
    assert!(
        embedder
            .embedded()
            .iter()
            .any(|text| text.starts_with("tool: github ")),
        "the github tool documents were embedded"
    );

    // ── Thread B: a new conversation's turn-0 selection ─────────────────────
    const OPENING: &str = "Please create an issue about the flaky test";
    let before = embedder.embedded().len();
    let viewer = g
        .thread("index-viewer")
        .script([RebornScriptedReply::text("noted")])
        .build()
        .await?;
    viewer.submit_turn(OPENING).await?;
    viewer.assert_reply_contains("noted").await?;
    let during: Vec<String> = embedder.embedded().split_off(before);
    let documents: Vec<&String> = during
        .iter()
        .filter(|text| text.starts_with("tool: "))
        .collect();
    if !documents.is_empty() {
        return Err(format!(
            "turn-0 selection after the install embedded {} tool documents",
            documents.len()
        )
        .into());
    }
    if !during.iter().any(|text| text == OPENING) {
        return Err("turn-0 selection did not rank the opening request densely".into());
    }
    Ok(())
}

async fn wait_until(what: &str, condition: impl Fn() -> bool) -> HarnessResult<()> {
    for _ in 0..1_000 {
        if condition() {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    Err(format!("timed out waiting for {what}").into())
}
