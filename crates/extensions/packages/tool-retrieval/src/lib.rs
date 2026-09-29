//! Dense (embedding) tool ranker for IronClaw Reborn.
//!
//! [`DenseToolRetrievalProvider`] implements the loop-tier
//! `ToolRetrievalProvider` / `ToolRetrievalIndex` port
//! (`ironclaw_loop_contracts::tool_retrieval`) with an
//! `ironclaw_llm::embeddings::EmbeddingProvider`:
//!
//! - **Fit.** One document per authorized tool (name, description, parameter
//!   names, and parameter descriptions from verified catalogs). Vectors are
//!   cached in memory by the SHA-256 of the document text and, when a
//!   [`FilesystemToolVectorStore`] is bound, persisted per corpus owner, so a
//!   catalog change or a restart re-embeds only new or changed tools. Missing
//!   documents are embedded by bounded background jobs the provider owns,
//!   batch by batch, so a caller that stops waiting loses nothing.
//! - **Background indexing.** `index_in_background` embeds an owner's catalog
//!   ahead of any turn; the host calls it when the catalog changes.
//! - **Search.** The query is embedded and every tool is scored by brute-force
//!   cosine similarity. Scores are exposed with the order.
//!
//! # Port contracts
//!
//! - *Authorization.* `fit` ranks exactly the definitions it is given, which
//!   the host limits to the effective authorized set. Nothing else is
//!   embedded, cached vectors are only read back for documents in the current
//!   corpus, and a fitted index never names a tool outside its corpus.
//! - *Owner isolation.* The durable store is partitioned by
//!   `ToolCorpusOwner` (tenant and user): a fit reads only its own owner's
//!   stored vectors. The in-memory cache is content-addressed across owners,
//!   which is safe because a vector is a pure function of the document text
//!   and the embedding space: a hit returns exactly what embedding the
//!   caller's own document would.
//! - *Determinism.* The corpus is kept in capability-id order and ties break
//!   on capability id, so equal inputs rank identically across fits.
//! - *Confidentiality.* Queries and schema text are never logged and never
//!   appear in an error. Logs carry counts and the query class only.
//!
//! # Scores
//!
//! See [`DENSE_RANKER_VERSION`]: clamped cosine similarity in `[0, 1]`,
//! higher is better, with an exact identifier match at `1.0`.
//!
//! # What leaves the host
//!
//! Every tool document and every search query is sent to the embedding
//! endpoint. That is why the ranker is opt-in (composition binds it only when
//! the operator selects it) and why a local endpoint is recommended.

mod cache;
mod document;
mod indexing;
mod provider;
mod store;

pub use indexing::ToolVectorStoreSlot;
pub use provider::{
    DEFAULT_VECTOR_CACHE_CAPACITY, DENSE_RANKER_VERSION, DenseToolRetrievalProvider,
    MAX_CORPUS_DEFINITIONS,
};
pub use store::{
    DEFAULT_STORED_VECTORS_PER_OWNER, EmbeddingSpace, FilesystemToolVectorStore,
    TOOL_VECTOR_MOUNT_ALIAS, VectorStoreError,
};
