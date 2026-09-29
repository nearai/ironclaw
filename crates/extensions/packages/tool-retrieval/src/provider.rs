//! The dense provider and the index it fits.

use std::collections::BTreeSet;
use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::ids::{CapabilityId, ProviderToolName};
use ironclaw_llm::embeddings::{EmbeddingError, EmbeddingProvider};
use ironclaw_loop_contracts::{
    LoopSafeSummary, ProviderToolDefinition, RankedTool, ToolCorpusOwner, ToolIndexFitReport,
    ToolRetrievalError, ToolRetrievalIndex, ToolRetrievalProvider, ToolSearchOutcome,
    ToolSearchQueryClass,
};

use crate::document::{ToolDocument, truncate_to_bytes};
use crate::indexing::{Indexing, Jobs, ToolVectorStoreSlot};

/// Ranker identifier, and the name of the score scale.
///
/// Scores are cosine similarity between the query and tool embeddings with
/// negative values clamped to zero, so every score lies in `[0, 1]` and a
/// relative threshold (score over the top score of the same search) keeps
/// its meaning. A tool whose similarity is not positive is not returned.
/// An exact identifier match scores `1.0`, the top of the scale.
pub const DENSE_RANKER_VERSION: &str = "dense-cosine-v1";

/// Most definitions one fit accepts. Larger corpora fail with
/// [`ToolRetrievalError::CorpusTooLarge`] rather than sending an unbounded
/// batch to the embedding endpoint.
pub const MAX_CORPUS_DEFINITIONS: usize = 2_048;

/// Default number of cached document vectors. Twice the corpus limit, so two
/// full catalogs (say, two profiles a deployment alternates between) can stay
/// cached together.
pub const DEFAULT_VECTOR_CACHE_CAPACITY: usize = 2 * MAX_CORPUS_DEFINITIONS;

/// Longest query embedded, in UTF-8 bytes; longer queries are cut at a
/// character boundary. It matches the largest conversation segment the loop
/// host's turn-start selection ranks (4 KiB), which is above its own
/// `tool_search` query bound, so a segment reaches the embedding model whole.
const MAX_QUERY_BYTES: usize = 4 * 1_024;

pub(crate) const LOG_TARGET: &str = "ironclaw::tool_retrieval";

/// Dense (embedding) ranker behind the `tool_search` retrieval port.
///
/// Embeds one document per authorized tool and ranks by cosine similarity to
/// the embedded query on `search`. Vectors are cached in memory by document
/// digest across fits and, once a store is bound
/// ([`Self::with_vector_store`]), persisted per corpus owner, so a catalog
/// change or a restart re-embeds only the tools whose document is new.
///
/// Missing documents are embedded by background jobs the provider owns (see
/// the `indexing` module): a fit waits for them, but a caller that stops
/// waiting (a timeout) never cancels the work, and every finished batch is
/// kept for the next fit. Dropping the provider aborts its jobs.
///
/// **Confidentiality.** Both the tool documents and every search query are
/// sent to the configured embedding endpoint. Nothing else leaves the host,
/// and neither is logged. Prefer an endpoint on the same host or network.
pub struct DenseToolRetrievalProvider {
    indexing: Arc<Indexing>,
    jobs: Jobs,
}

impl std::fmt::Debug for DenseToolRetrievalProvider {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DenseToolRetrievalProvider")
            .field("ranker_version", &DENSE_RANKER_VERSION)
            .field("embedding_model", &self.indexing.embedder.model_name())
            .finish_non_exhaustive()
    }
}

impl DenseToolRetrievalProvider {
    /// A provider ranking with `embedder`, with the default cache capacity
    /// and no durable store.
    pub fn new(embedder: Arc<dyn EmbeddingProvider>) -> Self {
        Self::with_cache_capacity(embedder, DEFAULT_VECTOR_CACHE_CAPACITY)
    }

    /// A provider whose vector cache holds at most `capacity` entries.
    /// Raised to [`MAX_CORPUS_DEFINITIONS`] when smaller, so one full corpus
    /// always fits.
    pub fn with_cache_capacity(embedder: Arc<dyn EmbeddingProvider>, capacity: usize) -> Self {
        Self::build(embedder, capacity, ToolVectorStoreSlot::new())
    }

    /// A provider persisting vectors through `store` (bound now or later),
    /// with the default cache capacity. Only owner-scoped fits
    /// ([`ToolRetrievalProvider::fit_for_owner`]) read or write it.
    pub fn with_vector_store(
        embedder: Arc<dyn EmbeddingProvider>,
        store: ToolVectorStoreSlot,
    ) -> Self {
        Self::build(embedder, DEFAULT_VECTOR_CACHE_CAPACITY, store)
    }

    fn build(
        embedder: Arc<dyn EmbeddingProvider>,
        capacity: usize,
        store: ToolVectorStoreSlot,
    ) -> Self {
        Self {
            indexing: Arc::new(Indexing::new(
                embedder,
                capacity.max(MAX_CORPUS_DEFINITIONS),
                store,
            )),
            jobs: Jobs::default(),
        }
    }

    async fn fit_corpus(
        &self,
        owner: Option<&ToolCorpusOwner>,
        definitions: &[ProviderToolDefinition],
    ) -> Result<Arc<dyn ToolRetrievalIndex>, ToolRetrievalError> {
        if definitions.len() > MAX_CORPUS_DEFINITIONS {
            return Err(ToolRetrievalError::CorpusTooLarge {
                definitions: definitions.len(),
                limit: MAX_CORPUS_DEFINITIONS,
            });
        }
        // Capability-id order is the tie-break, so it is also the corpus
        // order: two fits over the same definitions in any order are equal.
        let mut definitions: Vec<&ProviderToolDefinition> = definitions.iter().collect();
        definitions.sort_by(|left, right| left.capability_id.cmp(&right.capability_id));
        let documents: Vec<ToolDocument> = definitions
            .iter()
            .map(|definition| ToolDocument::new(definition))
            .collect();

        let (_generation, mut gathered) = self.indexing.gather(owner, &documents).await?;
        let missing: Vec<&ToolDocument> = documents
            .iter()
            .zip(&gathered.vectors)
            .filter(|(_, vector)| vector.is_none())
            .map(|(document, _)| document)
            .collect();
        let pending = self
            .jobs
            .start(&self.indexing, owner, &missing, &mut gathered)?;
        let mut report = ToolIndexFitReport {
            stored: gathered.stored,
            loaded: gathered.loaded,
            embedded: 0,
            missing: missing.len(),
            dense_fallback: None,
        };
        let mut vectors = gathered.vectors;
        if !pending.is_empty() {
            // Only the wait lives in this future: if the caller drops it,
            // the jobs finish and keep their vectors for the next fit.
            let abandoned = AbandonedFitLog::new(report);
            let filled = pending.fill(&documents, &mut vectors).await?;
            abandoned.disarm();
            report.embedded = filled;
            report.missing = report.missing.saturating_sub(filled);
        }
        tracing::debug!(
            target: LOG_TARGET,
            documents = documents.len(),
            stored = report.stored,
            loaded = report.loaded,
            embedded = report.embedded,
            owner_scoped = owner.is_some(),
            "fitted dense tool index"
        );

        // Every vector in one index shares a dimension.
        let mut dimension = None;
        let mut entries = Vec::with_capacity(definitions.len());
        for (definition, vector) in definitions.into_iter().zip(vectors) {
            let Some(vector) = vector else {
                return Err(invalid_output(
                    "a tool document was left without an embedding vector",
                ));
            };
            check_vector(&vector, &mut dimension)?;
            entries.push(IndexEntry::new(definition, vector));
        }
        Ok(Arc::new(DenseToolIndex {
            embedder: Arc::clone(&self.indexing.embedder),
            entries,
            dimension,
            report,
        }))
    }
}

/// Logs, at debug, a fit whose caller stopped waiting before its embedding
/// jobs finished. The jobs carry on; this records how far the fit had got.
struct AbandonedFitLog {
    report: ToolIndexFitReport,
    armed: bool,
}

impl AbandonedFitLog {
    fn new(report: ToolIndexFitReport) -> Self {
        Self {
            report,
            armed: true,
        }
    }

    fn disarm(mut self) {
        self.armed = false;
    }
}

impl Drop for AbandonedFitLog {
    fn drop(&mut self) {
        if self.armed {
            tracing::debug!(
                target: LOG_TARGET,
                stored = self.report.stored,
                loaded = self.report.loaded,
                missing = self.report.missing,
                "dense tool index fit abandoned by its caller; indexing continues in the background"
            );
        }
    }
}

#[async_trait]
impl ToolRetrievalProvider for DenseToolRetrievalProvider {
    fn ranker_version(&self) -> &str {
        DENSE_RANKER_VERSION
    }

    async fn fit(
        &self,
        definitions: &[ProviderToolDefinition],
    ) -> Result<Arc<dyn ToolRetrievalIndex>, ToolRetrievalError> {
        self.fit_corpus(None, definitions).await
    }

    async fn fit_for_owner(
        &self,
        owner: &ToolCorpusOwner,
        definitions: &[ProviderToolDefinition],
    ) -> Result<Arc<dyn ToolRetrievalIndex>, ToolRetrievalError> {
        self.fit_corpus(Some(owner), definitions).await
    }

    fn index_in_background(&self, owner: &ToolCorpusOwner, definitions: &[ProviderToolDefinition]) {
        if definitions.len() > MAX_CORPUS_DEFINITIONS {
            tracing::debug!(
                target: LOG_TARGET,
                definitions = definitions.len(),
                limit = MAX_CORPUS_DEFINITIONS,
                "background tool indexing skipped an oversized catalog"
            );
            return;
        }
        let documents: Vec<ToolDocument> = definitions.iter().map(ToolDocument::new).collect();
        if !self
            .jobs
            .start_background(&self.indexing, owner.clone(), documents)
        {
            tracing::debug!(
                target: LOG_TARGET,
                "background tool indexing request dropped: the queue is full"
            );
        }
    }
}

/// One fitted corpus: a vector per authorized tool, in capability-id order.
struct DenseToolIndex {
    embedder: Arc<dyn EmbeddingProvider>,
    entries: Vec<IndexEntry>,
    /// Shared vector length; `None` only for an empty corpus.
    dimension: Option<usize>,
    report: ToolIndexFitReport,
}

impl std::fmt::Debug for DenseToolIndex {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DenseToolIndex")
            .field("tools", &self.entries.len())
            .field("dimension", &self.dimension)
            .finish_non_exhaustive()
    }
}

struct IndexEntry {
    name: String,
    capability_id: CapabilityId,
    /// Lowercased capability id, provider tool name, and encoded capability
    /// name: a query equal to any of them is an identifier lookup.
    exact_identifiers: BTreeSet<String>,
    vector: Arc<[f32]>,
    norm: f64,
}

impl IndexEntry {
    fn new(definition: &ProviderToolDefinition, vector: Arc<[f32]>) -> Self {
        let capability_id = definition.capability_id.as_str();
        let exact_identifiers = BTreeSet::from([
            capability_id.to_lowercase(),
            definition.name.as_str().to_lowercase(),
            ProviderToolName::encode_capability_str(capability_id).to_lowercase(),
        ]);
        let norm = vector_norm(&vector);
        Self {
            name: definition.name.to_string(),
            capability_id: definition.capability_id.clone(),
            exact_identifiers,
            vector,
            norm,
        }
    }
}

#[async_trait]
impl ToolRetrievalIndex for DenseToolIndex {
    async fn search(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<ToolSearchOutcome, ToolRetrievalError> {
        let mut outcomes = self.search_many(&[query], limit).await?;
        match (outcomes.pop(), outcomes.is_empty()) {
            (Some(outcome), true) => Ok(outcome),
            _ => Err(invalid_output(
                "the dense tool index did not rank exactly one query",
            )),
        }
    }

    /// Every query that has text is embedded in one batch (the embeddings
    /// client splits it into endpoint-sized requests itself), then each is
    /// ranked on its own vector.
    async fn search_many(
        &self,
        queries: &[&str],
        limit: usize,
    ) -> Result<Vec<ToolSearchOutcome>, ToolRetrievalError> {
        let queries: Vec<&str> = queries
            .iter()
            .map(|query| truncate_to_bytes(query.trim(), MAX_QUERY_BYTES))
            .collect();
        let searched = |query: &&str| limit > 0 && !query.is_empty() && !self.entries.is_empty();
        let texts: Vec<String> = queries
            .iter()
            .filter(|query| searched(query))
            .map(|query| query.to_string())
            .collect();
        let embedded = if texts.is_empty() {
            Vec::new()
        } else {
            self.embedder
                .embed(&texts)
                .await
                .map_err(map_embedding_error)?
        };
        if embedded.len() != texts.len() {
            return Err(invalid_output(
                "the embedding endpoint did not return exactly one vector per query",
            ));
        }
        let mut vectors = embedded.iter();
        let mut outcomes = Vec::with_capacity(queries.len());
        for query in &queries {
            let vector = if searched(query) {
                vectors.next()
            } else {
                None
            };
            outcomes.push(match vector {
                Some(vector) => self.rank(query, vector, limit)?,
                None => ToolSearchOutcome::no_match(),
            });
        }
        tracing::debug!(
            target: LOG_TARGET,
            queries = queries.len(),
            embedded = texts.len(),
            "dense tool search"
        );
        Ok(outcomes)
    }

    fn fit_report(&self) -> Option<ToolIndexFitReport> {
        Some(self.report)
    }
}

impl DenseToolIndex {
    /// Rank the corpus against one embedded query.
    fn rank(
        &self,
        query: &str,
        query_vector: &[f32],
        limit: usize,
    ) -> Result<ToolSearchOutcome, ToolRetrievalError> {
        let mut dimension = self.dimension;
        check_vector(query_vector, &mut dimension)?;
        let query_norm = vector_norm(query_vector);

        let normalized_query = query.to_lowercase();
        let mut exact = false;
        let mut scored: Vec<(f64, bool, &IndexEntry)> = Vec::new();
        for entry in &self.entries {
            let is_exact = entry.exact_identifiers.contains(&normalized_query);
            exact |= is_exact;
            let score = if is_exact {
                1.0
            } else {
                cosine(query_vector, query_norm, &entry.vector, entry.norm)
            };
            if score > 0.0 {
                scored.push((score, is_exact, entry));
            }
        }
        // Score descending; an exact identifier before an equal cosine; then
        // capability id, which is unique and stable across fits.
        scored.sort_by(|left, right| {
            right
                .0
                .total_cmp(&left.0)
                .then_with(|| right.1.cmp(&left.1))
                .then_with(|| left.2.capability_id.cmp(&right.2.capability_id))
        });
        // Order was decided on the f64 score; the f32 conversion is monotonic,
        // so the reported scores never contradict it.
        let ranked: Vec<RankedTool> = scored
            .into_iter()
            .take(limit)
            .map(|(score, _, entry)| RankedTool::new(entry.name.clone(), score as f32))
            .collect();
        let query_class = if ranked.is_empty() {
            ToolSearchQueryClass::NoMatch
        } else if exact {
            ToolSearchQueryClass::ExactIdentifier
        } else {
            ToolSearchQueryClass::Lexical
        };
        tracing::debug!(
            target: LOG_TARGET,
            results = ranked.len(),
            query_class = query_class.as_str(),
            "dense tool search ranked a query"
        );
        Ok(ToolSearchOutcome {
            ranked,
            query_class,
        })
    }
}

/// Every vector must be non-empty, finite, and as long as the others.
pub(crate) fn check_vector(
    vector: &[f32],
    dimension: &mut Option<usize>,
) -> Result<(), ToolRetrievalError> {
    if vector.is_empty() || vector.iter().any(|value| !value.is_finite()) {
        return Err(invalid_output(
            "the embedding endpoint returned an empty or non-finite vector",
        ));
    }
    match *dimension {
        Some(expected) if expected != vector.len() => Err(invalid_output(
            "the embedding endpoint returned vectors of differing dimensions",
        )),
        Some(_) => Ok(()),
        None => {
            *dimension = Some(vector.len());
            Ok(())
        }
    }
}

fn vector_norm(vector: &[f32]) -> f64 {
    vector
        .iter()
        .map(|value| f64::from(*value) * f64::from(*value))
        .sum::<f64>()
        .sqrt()
}

/// Cosine similarity clamped to `[0, 1]`; a zero-length vector scores zero.
/// Computed in `f64` so the order does not hinge on `f32` rounding.
fn cosine(left: &[f32], left_norm: f64, right: &[f32], right_norm: f64) -> f64 {
    if left_norm <= 0.0 || right_norm <= 0.0 {
        return 0.0;
    }
    let dot: f64 = left
        .iter()
        .zip(right)
        .map(|(left, right)| f64::from(*left) * f64::from(*right))
        .sum();
    let similarity = dot / (left_norm * right_norm);
    if similarity.is_finite() {
        similarity.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// Map an embeddings failure onto the retrieval port's vocabulary.
///
/// Reasons are fixed text plus, at most, an HTTP status or a dimension: the
/// embeddings error can carry a response body, which may echo the query or a
/// tool document, so none of its free text is forwarded.
pub(crate) fn map_embedding_error(error: EmbeddingError) -> ToolRetrievalError {
    let unavailable = |reason: String| ToolRetrievalError::Unavailable {
        reason: summary(&reason),
    };
    match error {
        EmbeddingError::Timeout { timeout } => ToolRetrievalError::Timeout { elapsed: timeout },
        EmbeddingError::InvalidConfig { .. } => {
            unavailable("the embedding provider is misconfigured".to_string())
        }
        EmbeddingError::RequestFailed { .. } => {
            unavailable("the embedding endpoint could not be reached".to_string())
        }
        EmbeddingError::AuthFailed { status } => unavailable(format!(
            "the embedding endpoint rejected its credentials (HTTP {status})"
        )),
        EmbeddingError::RateLimited { .. } => {
            unavailable("the embedding endpoint is rate limiting requests".to_string())
        }
        EmbeddingError::HttpStatus { status, .. } => {
            unavailable(format!("the embedding endpoint returned HTTP {status}"))
        }
        EmbeddingError::InvalidResponse { .. } => {
            invalid_output("the embedding endpoint returned a malformed response")
        }
        EmbeddingError::DimensionMismatch { expected, actual } => {
            ToolRetrievalError::InvalidOutput {
                reason: summary(&format!(
                    "the embedding endpoint returned {actual}-dimensional vectors, expected {expected}"
                )),
            }
        }
        EmbeddingError::InputTooLong { .. } => {
            invalid_output("a tool document or the query exceeded the embedding input limit")
        }
    }
}

pub(crate) fn invalid_output(reason: &str) -> ToolRetrievalError {
    ToolRetrievalError::InvalidOutput {
        reason: summary(reason),
    }
}

/// Every reason here is fixed text, so validation cannot fail in practice;
/// if it ever did, the loop-safe redaction marker stands in.
pub(crate) fn summary(reason: &str) -> LoopSafeSummary {
    LoopSafeSummary::capability_failure_summary(reason)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn cosine_is_clamped_to_the_unit_interval() {
        let norm = |vector: &[f32]| vector_norm(vector);
        let a = [1.0_f32, 0.0];
        let b = [-1.0_f32, 0.0];
        let c = [1.0_f32, 1.0];
        assert_eq!(cosine(&a, norm(&a), &a, norm(&a)), 1.0);
        assert_eq!(cosine(&a, norm(&a), &b, norm(&b)), 0.0);
        let partial = cosine(&a, norm(&a), &c, norm(&c));
        assert!((partial - std::f64::consts::FRAC_1_SQRT_2).abs() < 1e-9);
        let zero = [0.0_f32, 0.0];
        assert_eq!(cosine(&a, norm(&a), &zero, norm(&zero)), 0.0);
    }

    #[test]
    fn embedding_errors_map_without_carrying_free_text() {
        let secret = "SECRET-QUERY-TEXT";
        let cases = [
            (
                EmbeddingError::Timeout {
                    timeout: Duration::from_secs(3),
                },
                "timeout",
            ),
            (
                EmbeddingError::RequestFailed {
                    reason: secret.to_string(),
                },
                "unavailable",
            ),
            (EmbeddingError::AuthFailed { status: 401 }, "unavailable"),
            (
                EmbeddingError::RateLimited {
                    retry_after: Duration::from_secs(1),
                },
                "unavailable",
            ),
            (
                EmbeddingError::HttpStatus {
                    status: 500,
                    body: secret.to_string(),
                },
                "unavailable",
            ),
            (
                EmbeddingError::InvalidConfig {
                    reason: secret.to_string(),
                },
                "unavailable",
            ),
            (
                EmbeddingError::InvalidResponse {
                    reason: secret.to_string(),
                },
                "invalid_output",
            ),
            (
                EmbeddingError::DimensionMismatch {
                    expected: 3,
                    actual: 4,
                },
                "invalid_output",
            ),
            (
                EmbeddingError::InputTooLong {
                    index: 0,
                    length: 10,
                    max: 5,
                },
                "invalid_output",
            ),
        ];
        for (error, kind) in cases {
            let mapped = map_embedding_error(error);
            assert_eq!(mapped.kind_label(), kind);
            assert!(!mapped.to_string().contains(secret), "{mapped}");
        }
    }
}
