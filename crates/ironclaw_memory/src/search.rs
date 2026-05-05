//! Memory search request/result types and rank-fusion helpers.

use ironclaw_host_api::HostApiError;

use crate::path::MemoryDocumentPath;

/// Upper bound on the requested final result count. Keeps a faulty caller
/// from translating `usize::MAX` into an unbounded SQL `LIMIT`.
const MAX_LIMIT: usize = 1_000;

/// Upper bound on the per-branch candidate budget before fusion. 5x the
/// final-limit ceiling leaves enough headroom for hybrid fusion to do
/// useful re-ranking without letting an attacker request millions of rows.
const MAX_PRE_FUSION_LIMIT: usize = 5_000;

/// Strategy used to fuse full-text and vector search result ranks.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum FusionStrategy {
    /// Reciprocal Rank Fusion, matching the current workspace default.
    #[default]
    Rrf,
    /// Weighted rank-derived score fusion.
    WeightedScore,
}

/// Search request passed to memory backends that expose search APIs.
#[derive(Debug, Clone, PartialEq)]
pub struct MemorySearchRequest {
    query: String,
    limit: usize,
    pre_fusion_limit: usize,
    full_text: bool,
    vector: bool,
    query_embedding: Option<Vec<f32>>,
    fusion_strategy: FusionStrategy,
    rrf_k: u32,
    min_score: f32,
    full_text_weight: f32,
    vector_weight: f32,
}

impl MemorySearchRequest {
    pub fn new(query: impl Into<String>) -> Result<Self, HostApiError> {
        let query = query.into();
        if query.trim().is_empty() {
            return Err(HostApiError::InvalidId {
                kind: "memory search query",
                value: query,
                reason: "query must not be empty".to_string(),
            });
        }
        Ok(Self {
            query,
            limit: 20,
            pre_fusion_limit: 50,
            full_text: true,
            vector: true,
            query_embedding: None,
            fusion_strategy: FusionStrategy::default(),
            rrf_k: 60,
            min_score: 0.0,
            full_text_weight: 0.5,
            vector_weight: 0.5,
        })
    }

    pub fn with_limit(mut self, limit: usize) -> Self {
        self.limit = limit.clamp(1, MAX_LIMIT);
        // Re-clamp pre_fusion_limit so the invariant `pre_fusion_limit >= limit`
        // holds regardless of builder call order: if the caller sets
        // `with_pre_fusion_limit(2).with_limit(5)`, the pre-fusion candidate
        // budget must also widen to at least 5, otherwise the per-branch SQL
        // `LIMIT` would silently be smaller than the requested final limit.
        // Also keep the upper DB-safety bound on pre_fusion_limit.
        self.pre_fusion_limit = self
            .pre_fusion_limit
            .max(self.limit)
            .min(MAX_PRE_FUSION_LIMIT);
        self
    }

    pub fn with_pre_fusion_limit(mut self, limit: usize) -> Self {
        // `pre_fusion_limit >= limit` (lower bound) and an upper DB-safety
        // ceiling so a `usize::MAX` from a faulty caller can't translate
        // into an unbounded SQL `LIMIT`.
        self.pre_fusion_limit = limit.max(self.limit).clamp(1, MAX_PRE_FUSION_LIMIT);
        self
    }

    pub fn with_full_text(mut self, enabled: bool) -> Self {
        self.full_text = enabled;
        self
    }

    pub fn with_vector(mut self, enabled: bool) -> Self {
        self.vector = enabled;
        self
    }

    pub fn with_query_embedding(mut self, embedding: Vec<f32>) -> Self {
        self.query_embedding = Some(embedding);
        self
    }

    pub fn with_fusion_strategy(mut self, strategy: FusionStrategy) -> Self {
        self.fusion_strategy = strategy;
        self
    }

    pub fn with_rrf_k(mut self, k: u32) -> Self {
        self.rrf_k = k;
        self
    }

    pub fn with_min_score(mut self, score: f32) -> Self {
        if score.is_finite() {
            self.min_score = score.clamp(0.0, 1.0);
        }
        self
    }

    pub fn with_full_text_weight(mut self, weight: f32) -> Self {
        if weight.is_finite() && weight >= 0.0 {
            self.full_text_weight = weight;
        }
        self
    }

    pub fn with_vector_weight(mut self, weight: f32) -> Self {
        if weight.is_finite() && weight >= 0.0 {
            self.vector_weight = weight;
        }
        self
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn limit(&self) -> usize {
        self.limit
    }

    pub fn pre_fusion_limit(&self) -> usize {
        self.pre_fusion_limit
    }

    pub fn full_text(&self) -> bool {
        self.full_text
    }

    pub fn vector(&self) -> bool {
        self.vector
    }

    pub fn query_embedding(&self) -> Option<&[f32]> {
        self.query_embedding.as_deref()
    }

    pub fn fusion_strategy(&self) -> FusionStrategy {
        self.fusion_strategy
    }

    pub fn rrf_k(&self) -> u32 {
        self.rrf_k
    }

    pub fn min_score(&self) -> f32 {
        self.min_score
    }

    pub fn full_text_weight(&self) -> f32 {
        self.full_text_weight
    }

    pub fn vector_weight(&self) -> f32 {
        self.vector_weight
    }
}

/// Search result returned by memory backends that expose search APIs.
#[derive(Debug, Clone, PartialEq)]
pub struct MemorySearchResult {
    pub path: MemoryDocumentPath,
    pub score: f32,
    pub snippet: String,
    pub full_text_rank: Option<u32>,
    pub vector_rank: Option<u32>,
}

impl MemorySearchResult {
    pub fn from_full_text(&self) -> bool {
        self.full_text_rank.is_some()
    }

    pub fn from_vector(&self) -> bool {
        self.vector_rank.is_some()
    }

    pub fn is_hybrid(&self) -> bool {
        self.full_text_rank.is_some() && self.vector_rank.is_some()
    }
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
#[derive(Debug, Clone)]
pub(crate) struct RankedMemorySearchResult {
    pub(crate) chunk_key: String,
    pub(crate) path: MemoryDocumentPath,
    pub(crate) snippet: String,
    pub(crate) rank: u32,
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
pub(crate) fn fuse_memory_search_results(
    full_text_results: Vec<RankedMemorySearchResult>,
    vector_results: Vec<RankedMemorySearchResult>,
    request: &MemorySearchRequest,
) -> Vec<MemorySearchResult> {
    use std::collections::HashMap;

    #[derive(Debug)]
    struct ResultAccumulator {
        path: MemoryDocumentPath,
        snippet: String,
        score: f32,
        full_text_rank: Option<u32>,
        vector_rank: Option<u32>,
    }

    let mut results = HashMap::<String, ResultAccumulator>::new();
    for result in full_text_results {
        let score = match request.fusion_strategy() {
            FusionStrategy::Rrf => 1.0 / (request.rrf_k() as f32 + result.rank as f32),
            FusionStrategy::WeightedScore => request.full_text_weight() / result.rank as f32,
        };
        results
            .entry(result.chunk_key)
            .and_modify(|existing| {
                existing.score += score;
                existing.full_text_rank = Some(result.rank);
            })
            .or_insert(ResultAccumulator {
                path: result.path,
                snippet: result.snippet,
                score,
                full_text_rank: Some(result.rank),
                vector_rank: None,
            });
    }
    for result in vector_results {
        let score = match request.fusion_strategy() {
            FusionStrategy::Rrf => 1.0 / (request.rrf_k() as f32 + result.rank as f32),
            FusionStrategy::WeightedScore => request.vector_weight() / result.rank as f32,
        };
        results
            .entry(result.chunk_key)
            .and_modify(|existing| {
                existing.score += score;
                existing.vector_rank = Some(result.rank);
            })
            .or_insert(ResultAccumulator {
                path: result.path,
                snippet: result.snippet,
                score,
                full_text_rank: None,
                vector_rank: Some(result.rank),
            });
    }

    let mut fused = results
        .into_values()
        .map(|result| MemorySearchResult {
            path: result.path,
            score: result.score,
            snippet: result.snippet,
            full_text_rank: result.full_text_rank,
            vector_rank: result.vector_rank,
        })
        .collect::<Vec<_>>();
    // Normalize before applying `min_score`: raw RRF/weighted scores are tiny
    // (e.g. `1/(60+1) ≈ 0.016` for the top RRF hit), so a caller-supplied
    // `min_score` like `0.5` would drop every result if applied to the
    // unnormalized values. Workspace's existing fusion normalizes first, then
    // filters; we mirror that contract here.
    if let Some(max_score) = fused.iter().map(|result| result.score).reduce(f32::max)
        && max_score > 0.0
    {
        for result in &mut fused {
            result.score /= max_score;
        }
    }
    if request.min_score() > 0.0 {
        fused.retain(|result| result.score >= request.min_score());
    }
    fused.sort_by(|left, right| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.path.relative_path().cmp(right.path.relative_path()))
    });
    fused.truncate(request.limit());
    fused
}

#[cfg(feature = "libsql")]
pub(crate) fn escape_fts5_query(query: &str) -> Option<String> {
    let phrases = query
        .split_whitespace()
        .map(|token| format!("\"{}\"", token.replace('"', "\"\"")))
        .collect::<Vec<_>>();
    if phrases.is_empty() {
        None
    } else {
        Some(phrases.join(" "))
    }
}

#[cfg(all(test, any(feature = "libsql", feature = "postgres")))]
mod tests {
    use super::*;
    use crate::path::MemoryDocumentPath;

    fn ranked(chunk_key: &str, relative_path: &str, rank: u32) -> RankedMemorySearchResult {
        RankedMemorySearchResult {
            chunk_key: chunk_key.to_string(),
            path: MemoryDocumentPath::new("tenant-a", "alice", None, relative_path).expect("path"),
            snippet: format!("snippet for {relative_path}"),
            rank,
        }
    }

    fn rrf_request(min_score: f32) -> MemorySearchRequest {
        MemorySearchRequest::new("query")
            .unwrap()
            .with_fusion_strategy(FusionStrategy::Rrf)
            .with_rrf_k(60)
            .with_min_score(min_score)
    }

    fn weighted_request(min_score: f32) -> MemorySearchRequest {
        MemorySearchRequest::new("query")
            .unwrap()
            .with_fusion_strategy(FusionStrategy::WeightedScore)
            .with_full_text_weight(0.5)
            .with_vector_weight(0.5)
            .with_min_score(min_score)
    }

    #[test]
    fn fusion_ties_break_deterministically_by_path_ascending() {
        // Two distinct chunks with identical FT rank (and no vector
        // contribution) produce identical fusion scores. The tiebreak
        // must order them by relative path ascending — proving
        // hybrid-search ordering is deterministic across runs even when
        // scores are equal.
        let request = MemorySearchRequest::new("q").unwrap().with_limit(10);
        let ft = vec![ranked("chunk-z", "z.md", 1), ranked("chunk-a", "a.md", 1)];
        let fused = fuse_memory_search_results(ft, Vec::new(), &request);
        let paths: Vec<_> = fused
            .iter()
            .map(|r| r.path.relative_path().to_string())
            .collect();
        assert_eq!(
            paths,
            vec!["a.md".to_string(), "z.md".to_string()],
            "tied scores must sort by path ascending"
        );
    }

    #[test]
    fn fusion_reverses_when_path_order_flips_under_ties() {
        // Reverse insertion order to confirm path-asc tiebreak does not
        // depend on insertion order — the sort is genuinely stable on
        // the path key, not coincidentally on iteration order.
        let request = MemorySearchRequest::new("q").unwrap().with_limit(10);
        let ft = vec![ranked("chunk-a", "a.md", 1), ranked("chunk-z", "z.md", 1)];
        let fused = fuse_memory_search_results(ft, Vec::new(), &request);
        let paths: Vec<_> = fused
            .iter()
            .map(|r| r.path.relative_path().to_string())
            .collect();
        assert_eq!(paths, vec!["a.md".to_string(), "z.md".to_string()]);
    }

    // Regression for PR #3180 review: `min_score` was filtering raw RRF/weighted
    // scores before the later normalization pass. With the default RRF k=60 the
    // top single-source raw score is ~0.016, so `min_score = 0.5` would drop
    // every result before normalization could lift the best hit to 1.0. The
    // contract is "filter against normalized scores", matching the existing
    // workspace fusion.
    #[test]
    fn rrf_min_score_filters_normalized_scores_not_raw_rrf_values() {
        let full_text = vec![
            ranked("chunk-alpha", "alpha.md", 1),
            ranked("chunk-beta", "beta.md", 2),
            ranked("chunk-gamma", "gamma.md", 3),
        ];
        let request = rrf_request(0.5);
        let fused = fuse_memory_search_results(full_text, Vec::new(), &request);
        // Normalized to [0, 1]; min_score=0.5 should keep at least the top hit
        // even though its raw RRF score (1/61) is far below 0.5.
        assert!(
            !fused.is_empty(),
            "expected at least the top hit to survive"
        );
        assert_eq!(fused[0].score, 1.0);
        for result in &fused {
            assert!(
                result.score >= 0.5,
                "filtered score {} fell below min_score=0.5",
                result.score
            );
        }
    }

    #[test]
    fn weighted_min_score_filters_normalized_scores_not_raw_weighted_values() {
        let full_text = vec![
            ranked("chunk-alpha", "alpha.md", 1),
            ranked("chunk-beta", "beta.md", 2),
        ];
        let vector = vec![
            ranked("chunk-alpha", "alpha.md", 1),
            ranked("chunk-gamma", "gamma.md", 3),
        ];
        let request = weighted_request(0.5);
        let fused = fuse_memory_search_results(full_text, vector, &request);
        assert!(
            !fused.is_empty(),
            "expected at least the top hit to survive"
        );
        assert_eq!(fused[0].score, 1.0);
        for result in &fused {
            assert!(
                result.score >= 0.5,
                "filtered score {} fell below min_score=0.5",
                result.score
            );
        }
    }

    #[test]
    fn min_score_zero_keeps_every_fused_result() {
        let full_text = vec![
            ranked("chunk-alpha", "alpha.md", 1),
            ranked("chunk-beta", "beta.md", 2),
            ranked("chunk-gamma", "gamma.md", 3),
        ];
        let fused = fuse_memory_search_results(full_text, Vec::new(), &rrf_request(0.0));
        assert_eq!(fused.len(), 3);
    }
}
