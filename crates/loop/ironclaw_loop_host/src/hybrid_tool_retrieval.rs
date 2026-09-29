//! Hybrid tool ranking: BM25F fused with an optional dense ranker.
//!
//! [`HybridToolRetrieval`] keeps the host-bundled BM25F ranker as the
//! relevance authority and adds a second, dense opinion when one is bound. The
//! two rankings are combined by reciprocal-rank fusion (RRF), which uses only
//! each tool's rank in each list, so the two score scales never have to be
//! reconciled.
//!
//! The dense side is best-effort. When it is not bound, when its fit fails or
//! runs past [`DEFAULT_DENSE_FIT_TIMEOUT`], or when a search fails or runs past
//! [`DEFAULT_DENSE_SEARCH_TIMEOUT`], that search returns exactly what the
//! native BM25F ranker would have returned. A dense failure therefore never
//! reaches the model; it only costs the semantic signal for that search (or,
//! for a failed fit, until the host refits, at the next turn at the latest).
//! A fit timeout stops only the wait: a dense ranker that embeds in the
//! background keeps the work for its next fit. The fitted index's
//! `fit_report` records the dense side's counts, or why it is absent.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
    time::{Duration, Instant},
};

use async_trait::async_trait;
use ironclaw_loop_contracts::{
    ProviderToolDefinition, RankedTool, ToolCorpusOwner, ToolIndexFitReport, ToolRetrievalError,
    ToolRetrievalIndex, ToolRetrievalProvider, ToolSearchOutcome, ToolSearchQueryClass,
};
use tracing::debug;

use crate::tool_search::{
    AuthorizedToolSearchIndex, NativeBm25fToolRetrieval, sanitize_provider_ranking,
};

/// Version of the fusion itself. The full ranker version also names both
/// inputs; see [`HybridToolRetrieval::ranker_version`].
const HYBRID_FUSION_VERSION: &str = "hybrid-rrf-v1";

/// The RRF constant `k`: a tool at 1-based rank `r` in one list contributes
/// `1 / (k + r)`. 60 is the value from the original RRF paper and the usual
/// default; it damps the gap between the first few ranks so that neither list
/// can dominate on its top hit alone.
const RRF_K: f64 = 60.0;

/// How deep each input list is read before fusion, at minimum. A tool ranked
/// just outside the requested `limit` by both rankers can still fuse into it,
/// so fusing only the top `limit` of each would lose exactly the agreement RRF
/// is meant to reward.
const FUSION_CANDIDATE_DEPTH: usize = 50;

/// Time the dense ranker gets to answer one `tool_search` query. Past it, the
/// query is answered by BM25F alone. This is the per-turn latency the dense
/// side may add to a search.
pub const DEFAULT_DENSE_SEARCH_TIMEOUT: Duration = Duration::from_millis(150);

/// Time the dense ranker gets to fit (embed) the authorized catalog. Larger
/// than the search bound because a first fit over hundreds of tools embeds
/// every one of them; later fits are mostly cache hits. Past it, the fitted
/// index is lexical-only until the next refit.
pub const DEFAULT_DENSE_FIT_TIMEOUT: Duration = Duration::from_secs(5);

const LOG_TARGET: &str = "ironclaw::reborn::tool_search";

/// A [`ToolRetrievalProvider`] that fuses BM25F with an optional dense ranker.
///
/// See the module docs for the fusion and fallback rules. Fusion is
/// deterministic: for one fitted index and one `(query, limit)` it returns the
/// same ranking, ties broken by BM25F rank and then by tool name.
#[derive(Debug)]
pub struct HybridToolRetrieval {
    dense: Option<Arc<dyn ToolRetrievalProvider>>,
    ranker_version: String,
    dense_fit_timeout: Duration,
    dense_search_timeout: Duration,
}

impl HybridToolRetrieval {
    /// Hybrid ranking over BM25F and `dense`, with the default time bounds.
    /// With `dense` absent every search is plain BM25F.
    pub fn new(dense: Option<Arc<dyn ToolRetrievalProvider>>) -> Self {
        let dense_version = dense
            .as_ref()
            .map_or("none", |dense| dense.ranker_version());
        let ranker_version = format!(
            "{HYBRID_FUSION_VERSION}({},{dense_version})",
            NativeBm25fToolRetrieval.ranker_version()
        );
        Self {
            dense,
            ranker_version,
            dense_fit_timeout: DEFAULT_DENSE_FIT_TIMEOUT,
            dense_search_timeout: DEFAULT_DENSE_SEARCH_TIMEOUT,
        }
    }

    /// Replace [`DEFAULT_DENSE_FIT_TIMEOUT`].
    pub fn with_dense_fit_timeout(mut self, timeout: Duration) -> Self {
        self.dense_fit_timeout = timeout;
        self
    }

    /// Replace [`DEFAULT_DENSE_SEARCH_TIMEOUT`].
    pub fn with_dense_search_timeout(mut self, timeout: Duration) -> Self {
        self.dense_search_timeout = timeout;
        self
    }
}

#[async_trait]
impl ToolRetrievalProvider for HybridToolRetrieval {
    /// `hybrid-rrf-v1(<native>,<dense>)`, with `none` for an absent dense
    /// side. Naming both inputs means the host's fitted-index fingerprint
    /// separates hybrid from either ranker alone, and changes when the dense
    /// ranker does.
    fn ranker_version(&self) -> &str {
        &self.ranker_version
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

    /// BM25F has nothing to prepare; the dense side does.
    fn index_in_background(&self, owner: &ToolCorpusOwner, definitions: &[ProviderToolDefinition]) {
        if let Some(dense) = &self.dense {
            dense.index_in_background(owner, definitions);
        }
    }
}

impl HybridToolRetrieval {
    async fn fit_corpus(
        &self,
        owner: Option<&ToolCorpusOwner>,
        definitions: &[ProviderToolDefinition],
    ) -> Result<Arc<dyn ToolRetrievalIndex>, ToolRetrievalError> {
        let lexical = AuthorizedToolSearchIndex::new(definitions.iter());
        let (dense, report) = match &self.dense {
            None => (
                None,
                ToolIndexFitReport {
                    dense_fallback: Some(DenseFallback::Absent.label()),
                    ..ToolIndexFitReport::default()
                },
            ),
            Some(dense) => {
                let started = Instant::now();
                let fit = async {
                    match owner {
                        Some(owner) => dense.fit_for_owner(owner, definitions).await,
                        None => dense.fit(definitions).await,
                    }
                };
                // A timeout drops only this wait: a dense ranker that embeds
                // in the background (the tool-retrieval package does) keeps
                // the work and serves it to the next fit.
                match tokio::time::timeout(self.dense_fit_timeout, fit).await {
                    Ok(Ok(index)) => {
                        debug!(
                            target: LOG_TARGET,
                            definitions = definitions.len(),
                            latency_ms = started.elapsed().as_millis() as u64,
                            "hybrid tool retrieval fitted the dense index"
                        );
                        let report = index.fit_report().unwrap_or_default();
                        (Some(index), report)
                    }
                    Ok(Err(error)) => {
                        debug!(
                            target: LOG_TARGET,
                            definitions = definitions.len(),
                            error_kind = error.kind_label(),
                            latency_ms = started.elapsed().as_millis() as u64,
                            "hybrid tool retrieval dense fit failed; this index is lexical only"
                        );
                        (
                            None,
                            ToolIndexFitReport {
                                missing: definitions.len(),
                                dense_fallback: Some(error.kind_label()),
                                ..ToolIndexFitReport::default()
                            },
                        )
                    }
                    Err(_elapsed) => {
                        debug!(
                            target: LOG_TARGET,
                            definitions = definitions.len(),
                            timeout_ms = self.dense_fit_timeout.as_millis() as u64,
                            "hybrid tool retrieval dense fit timed out; this index is lexical only"
                        );
                        (
                            None,
                            ToolIndexFitReport {
                                missing: definitions.len(),
                                dense_fallback: Some(DenseFallback::Timeout.label()),
                                ..ToolIndexFitReport::default()
                            },
                        )
                    }
                }
            }
        };
        let corpus = lexical.document_names();
        Ok(Arc::new(HybridToolIndex {
            lexical,
            corpus,
            dense,
            dense_search_timeout: self.dense_search_timeout,
            report,
        }))
    }
}

/// An index fitted by [`HybridToolRetrieval`].
#[derive(Debug)]
struct HybridToolIndex {
    lexical: AuthorizedToolSearchIndex,
    /// Names in the fitted corpus: dense output outside it is dropped before
    /// fusion so it can never take a slot.
    corpus: BTreeSet<String>,
    dense: Option<Arc<dyn ToolRetrievalIndex>>,
    dense_search_timeout: Duration,
    /// The dense side's fit report, or why it is absent from this index.
    report: ToolIndexFitReport,
}

/// Why a search fell back to BM25F alone, for telemetry.
#[derive(Debug, Clone, Copy)]
enum DenseFallback {
    Absent,
    Timeout,
    Error(&'static str),
}

impl DenseFallback {
    fn label(self) -> &'static str {
        match self {
            Self::Absent => "absent",
            Self::Timeout => "timeout",
            Self::Error(kind) => kind,
        }
    }
}

#[async_trait]
impl ToolRetrievalIndex for HybridToolIndex {
    async fn search(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<ToolSearchOutcome, ToolRetrievalError> {
        if limit == 0 {
            return Ok(ToolSearchOutcome::no_match());
        }
        // An exact identifier is a lookup, not a relevance question: BM25F
        // already puts that tool first, so the dense side is not consulted.
        if self.lexical.is_exact_identifier_query(query) {
            debug!(
                target: LOG_TARGET,
                dense = "skipped_exact_identifier",
                "hybrid tool retrieval answered an exact identifier lexically"
            );
            return Ok(self.lexical.search(query, limit));
        }
        let Some(dense) = &self.dense else {
            return Ok(self.lexical_fallback(query, limit, DenseFallback::Absent, None));
        };

        let depth = limit.max(FUSION_CANDIDATE_DEPTH);
        let started = Instant::now();
        let dense_outcome =
            match tokio::time::timeout(self.dense_search_timeout, dense.search(query, depth)).await
            {
                Ok(Ok(outcome)) => outcome,
                Ok(Err(error)) => {
                    return Ok(self.lexical_fallback(
                        query,
                        limit,
                        DenseFallback::Error(error.kind_label()),
                        Some(started.elapsed()),
                    ));
                }
                Err(_elapsed) => {
                    return Ok(self.lexical_fallback(
                        query,
                        limit,
                        DenseFallback::Timeout,
                        Some(started.elapsed()),
                    ));
                }
            };
        Ok(self.fused(query, dense_outcome, limit, started.elapsed()))
    }

    /// The dense side ranks every query that is not an exact identifier in
    /// one `search_many` call (one embedding request for a dense index),
    /// under the same time bound as one search; each query is then fused on
    /// its own. When that call fails or runs out of time, every query falls
    /// back to BM25F alone, exactly as [`Self::search`] would.
    async fn search_many(
        &self,
        queries: &[&str],
        limit: usize,
    ) -> Result<Vec<ToolSearchOutcome>, ToolRetrievalError> {
        let Some(dense) = self.dense.as_ref().filter(|_| limit > 0) else {
            let mut outcomes = Vec::with_capacity(queries.len());
            for query in queries {
                outcomes.push(self.search(query, limit).await?);
            }
            return Ok(outcomes);
        };
        let fused_queries: Vec<&str> = queries
            .iter()
            .copied()
            .filter(|query| !self.lexical.is_exact_identifier_query(query))
            .collect();
        let depth = limit.max(FUSION_CANDIDATE_DEPTH);
        let started = Instant::now();
        let dense_outcomes = if fused_queries.is_empty() {
            Ok(Vec::new())
        } else {
            match tokio::time::timeout(
                self.dense_search_timeout,
                dense.search_many(&fused_queries, depth),
            )
            .await
            {
                Ok(Ok(outcomes)) if outcomes.len() == fused_queries.len() => Ok(outcomes),
                Ok(Ok(_)) => Err(DenseFallback::Error("invalid_output")),
                Ok(Err(error)) => Err(DenseFallback::Error(error.kind_label())),
                Err(_elapsed) => Err(DenseFallback::Timeout),
            }
        };
        let dense_latency = started.elapsed();
        let (mut dense_outcomes, fallback) = match dense_outcomes {
            Ok(outcomes) => (outcomes.into_iter(), None),
            Err(reason) => (Vec::new().into_iter(), Some(reason)),
        };
        let mut outcomes = Vec::with_capacity(queries.len());
        for query in queries {
            let outcome = if self.lexical.is_exact_identifier_query(query) {
                self.lexical.search(query, limit)
            } else {
                match (fallback, dense_outcomes.next()) {
                    (None, Some(dense_outcome)) => {
                        self.fused(query, dense_outcome, limit, dense_latency)
                    }
                    (reason, _) => self.lexical_fallback(
                        query,
                        limit,
                        reason.unwrap_or(DenseFallback::Error("invalid_output")),
                        Some(dense_latency),
                    ),
                }
            };
            outcomes.push(outcome);
        }
        Ok(outcomes)
    }

    fn fit_report(&self) -> Option<ToolIndexFitReport> {
        Some(self.report)
    }
}

impl HybridToolIndex {
    /// Fuse BM25F with the dense side's outcome for `query`.
    fn fused(
        &self,
        query: &str,
        dense_outcome: ToolSearchOutcome,
        limit: usize,
        dense_latency: Duration,
    ) -> ToolSearchOutcome {
        let depth = limit.max(FUSION_CANDIDATE_DEPTH);
        let (dense_ranked, repairs) =
            sanitize_provider_ranking(dense_outcome.ranked, &self.corpus, depth);
        let lexical_ranked = self.lexical.score_terms(query, depth);
        let ranked = fuse(&lexical_ranked, &dense_ranked, limit);
        debug!(
            target: LOG_TARGET,
            dense = "fused",
            lexical_candidates = lexical_ranked.len(),
            dense_candidates = dense_ranked.len(),
            dense_repaired = repairs.any(),
            results = ranked.len(),
            dense_latency_ms = dense_latency.as_millis() as u64,
            "hybrid tool retrieval fused lexical and dense rankings"
        );
        ToolSearchOutcome {
            query_class: if ranked.is_empty() {
                ToolSearchQueryClass::NoMatch
            } else {
                ToolSearchQueryClass::Lexical
            },
            ranked,
        }
    }

    /// The native BM25F outcome, unchanged: order, scores and query class.
    fn lexical_fallback(
        &self,
        query: &str,
        limit: usize,
        reason: DenseFallback,
        dense_latency: Option<Duration>,
    ) -> ToolSearchOutcome {
        let outcome = self.lexical.search(query, limit);
        debug!(
            target: LOG_TARGET,
            dense = "fallback",
            reason = reason.label(),
            results = outcome.ranked.len(),
            dense_latency_ms = dense_latency.map(|latency| latency.as_millis() as u64),
            "hybrid tool retrieval fell back to lexical ranking"
        );
        outcome
    }
}

/// Reciprocal-rank fusion of the two lists, best first, at most `limit`.
///
/// Each tool scores `sum(1 / (RRF_K + rank))` over the lists it appears in,
/// with 1-based ranks; a list it is absent from adds nothing. Equal scores
/// are ordered by BM25F rank (tools BM25F did not list come after those it
/// did), then by name, so the result is fully deterministic.
fn fuse(lexical: &[RankedTool], dense: &[RankedTool], limit: usize) -> Vec<RankedTool> {
    // name -> (fused score, BM25F rank or usize::MAX)
    let mut fused: BTreeMap<&str, (f64, usize)> = BTreeMap::new();
    for (index, tool) in lexical.iter().enumerate() {
        let entry = fused.entry(tool.name.as_str()).or_insert((0.0, usize::MAX));
        entry.0 += reciprocal_rank(index);
        entry.1 = entry.1.min(index);
    }
    for (index, tool) in dense.iter().enumerate() {
        let entry = fused.entry(tool.name.as_str()).or_insert((0.0, usize::MAX));
        entry.0 += reciprocal_rank(index);
    }
    let mut fused: Vec<_> = fused.into_iter().collect();
    fused.sort_by(|(left_name, left), (right_name, right)| {
        right
            .0
            .total_cmp(&left.0)
            .then_with(|| left.1.cmp(&right.1))
            .then_with(|| left_name.cmp(right_name))
    });
    // The f32 conversion is monotonic, so reported scores never contradict
    // the order decided on f64 above.
    fused
        .into_iter()
        .take(limit)
        .map(|(name, (score, _lexical_rank))| RankedTool::new(name, score as f32))
        .collect()
}

fn reciprocal_rank(zero_based_rank: usize) -> f64 {
    1.0 / (RRF_K + zero_based_rank as f64 + 1.0)
}

#[cfg(test)]
mod tests;
