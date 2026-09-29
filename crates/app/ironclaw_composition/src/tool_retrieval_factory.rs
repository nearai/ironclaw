//! The `tool_search` ranker: the operator's parsed `REBORN_TOOL_RETRIEVAL`
//! mode (`ironclaw_config::ToolRetrievalMode`) in, the provider to bind
//! through `RebornRuntimeInput::with_tool_retrieval_provider` (or none) out.
//!
//! `native` binds nothing, which keeps the host-bundled BM25F ranker. `dense`
//! binds the embedding ranker from the `tool-retrieval` package over the
//! `[embeddings]` provider; `hybrid` binds `ironclaw_loop_host`'s
//! `HybridToolRetrieval` with that same embedding ranker as its dense side.
//! Both refuse startup when no embeddings provider can be built: neither
//! falls back to `native` at startup. (`hybrid` does fall back to BM25F per
//! search at runtime, when its dense side fails or is slow.)

use std::sync::Arc;

use ironclaw_config::{EmbeddingsSection, REBORN_TOOL_RETRIEVAL_ENV, ToolRetrievalMode};
use ironclaw_loop_contracts::ToolRetrievalProvider;
use ironclaw_loop_host::HybridToolRetrieval;
use ironclaw_tool_retrieval::{DenseToolRetrievalProvider, ToolVectorStoreSlot};

use crate::embedding_provider_factory::resolve_embedding_provider;

/// Why the selected `tool_search` ranker could not be bound.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ToolRetrievalConfigError {
    #[error(
        "{REBORN_TOOL_RETRIEVAL_ENV}={} needs an embeddings provider, but none could be built \
         from the [embeddings] config section and the EMBEDDING_* environment variables; \
         configure one, or unset {REBORN_TOOL_RETRIEVAL_ENV} to keep the native ranker",
        mode.as_str()
    )]
    EmbeddingsRequired { mode: ToolRetrievalMode },
}

/// Build the ranker `mode` selects, or `None` for the native one. `embeddings`
/// and `env` are what [`resolve_embedding_provider`] takes. `vector_store`
/// is where the dense ranker persists vectors: pass the runtime input's
/// `tool_vector_store`, which the runtime build binds to the per-user store.
pub async fn resolve_tool_retrieval_provider(
    mode: ToolRetrievalMode,
    embeddings: Option<&EmbeddingsSection>,
    env: &dyn Fn(&str) -> Option<String>,
    vector_store: ToolVectorStoreSlot,
) -> Result<Option<Arc<dyn ToolRetrievalProvider>>, ToolRetrievalConfigError> {
    match mode {
        ToolRetrievalMode::Native => Ok(None),
        ToolRetrievalMode::Dense | ToolRetrievalMode::Hybrid => {
            let embedder = resolve_embedding_provider(embeddings, env)
                .await
                .ok_or(ToolRetrievalConfigError::EmbeddingsRequired { mode })?;
            let dense = Arc::new(DenseToolRetrievalProvider::with_vector_store(
                embedder,
                vector_store,
            ));
            Ok(Some(if mode == ToolRetrievalMode::Hybrid {
                Arc::new(HybridToolRetrieval::new(Some(dense)))
            } else {
                dense
            }))
        }
    }
}
