//! Hybrid search combining full-text and semantic search.
//!
//! Supports two fusion strategies:
//! 1. **RRF** (Reciprocal Rank Fusion) — the default, rank-based method.
//!    `score = sum(1 / (k + rank))` for each retrieval method.
//! 2. **WeightedScore** — converts ranks to scores via `1/rank`, combines with
//!    configurable weights (`fts_weight * fts_score + vector_weight * vector_score`),
//!    then normalizes to \[0,1\] by dividing by the maximum combined score.
//!
//! Both strategies combine results from:
//! - PostgreSQL / libSQL full-text search
//! - pgvector / libsql_vector cosine similarity search

use std::collections::HashMap;

use uuid::Uuid;

/// Strategy used to fuse FTS and vector search results.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum FusionStrategy {
    /// Reciprocal Rank Fusion (default). Ignores `fts_weight`/`vector_weight`.
    #[default]
    Rrf,
    /// Weighted score fusion using normalized rank-derived scores.
    WeightedScore,
}

/// Configuration for hybrid search.
#[derive(Debug, Clone)]
pub struct SearchConfig {
    /// Maximum number of results to return.
    pub limit: usize,
    /// RRF constant (typically 60). Higher values favor top results more.
    pub rrf_k: u32,
    /// Whether to include FTS results.
    pub use_fts: bool,
    /// Whether to include vector results.
    pub use_vector: bool,
    /// Minimum score threshold (0.0-1.0).
    pub min_score: f32,
    /// Maximum results to fetch from each method before fusion.
    pub pre_fusion_limit: usize,
    /// Fusion strategy to use when combining results.
    pub fusion_strategy: FusionStrategy,
    /// Weight for FTS results in `WeightedScore` fusion.
    /// Default depends on strategy: 0.5 for RRF, 0.3 for WeightedScore.
    pub fts_weight: f32,
    /// Weight for vector results in `WeightedScore` fusion.
    /// Default depends on strategy: 0.5 for RRF, 0.7 for WeightedScore.
    pub vector_weight: f32,
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            limit: 10,
            rrf_k: 60,
            use_fts: true,
            use_vector: true,
            min_score: 0.0,
            pre_fusion_limit: 50,
            fusion_strategy: FusionStrategy::default(),
            fts_weight: 0.5,
            vector_weight: 0.5,
        }
    }
}

impl SearchConfig {
    /// Set the result limit.
    pub fn with_limit(mut self, limit: usize) -> Self {
        self.limit = limit;
        self
    }

    /// Set the RRF constant.
    pub fn with_rrf_k(mut self, k: u32) -> Self {
        self.rrf_k = k;
        self
    }

    /// Disable FTS (only use vector search).
    pub fn vector_only(mut self) -> Self {
        self.use_fts = false;
        self.use_vector = true;
        self
    }

    /// Disable vector search (only use FTS).
    pub fn fts_only(mut self) -> Self {
        self.use_fts = true;
        self.use_vector = false;
        self
    }

    /// Set minimum score threshold.
    pub fn with_min_score(mut self, score: f32) -> Self {
        self.min_score = score.clamp(0.0, 1.0);
        self
    }

    /// Set the fusion strategy.
    pub fn with_fusion_strategy(mut self, strategy: FusionStrategy) -> Self {
        self.fusion_strategy = strategy;
        self
    }

    /// Set the FTS weight for `WeightedScore` fusion.
    ///
    /// Non-finite (NaN, ±inf) or negative values are ignored.
    pub fn with_fts_weight(mut self, weight: f32) -> Self {
        if weight.is_finite() && weight >= 0.0 {
            self.fts_weight = weight;
        }
        self
    }

    /// Set the vector weight for `WeightedScore` fusion.
    ///
    /// Non-finite (NaN, ±inf) or negative values are ignored.
    pub fn with_vector_weight(mut self, weight: f32) -> Self {
        if weight.is_finite() && weight >= 0.0 {
            self.vector_weight = weight;
        }
        self
    }
}

/// A search result with hybrid scoring.
#[derive(Debug, Clone)]
pub struct SearchResult {
    /// Document ID containing this chunk.
    pub document_id: Uuid,
    /// Chunk ID.
    pub chunk_id: Uuid,
    /// Chunk content.
    pub content: String,
    /// Combined fusion score (0.0-1.0 normalized). Strategy-dependent (RRF or WeightedScore).
    pub score: f32,
    /// Rank in FTS results (1-based, None if not in FTS results).
    pub fts_rank: Option<u32>,
    /// Rank in vector results (1-based, None if not in vector results).
    pub vector_rank: Option<u32>,
}

impl SearchResult {
    /// Check if this result came from FTS.
    pub fn from_fts(&self) -> bool {
        self.fts_rank.is_some()
    }

    /// Check if this result came from vector search.
    pub fn from_vector(&self) -> bool {
        self.vector_rank.is_some()
    }

    /// Check if this result came from both methods (hybrid match).
    pub fn is_hybrid(&self) -> bool {
        self.fts_rank.is_some() && self.vector_rank.is_some()
    }
}

/// Raw result from a single search method.
#[derive(Debug, Clone)]
pub struct RankedResult {
    pub chunk_id: Uuid,
    pub document_id: Uuid,
    pub content: String,
    pub rank: u32, // 1-based rank
}

/// Fuse FTS and vector search results using the strategy specified in `config`.
///
/// This is the primary entry point for result fusion. Delegates to
/// [`reciprocal_rank_fusion`] or [`weighted_score_fusion`] based on
/// `config.fusion_strategy`.
pub fn fuse_results(
    fts_results: Vec<RankedResult>,
    vector_results: Vec<RankedResult>,
    config: &SearchConfig,
) -> Vec<SearchResult> {
    match config.fusion_strategy {
        FusionStrategy::Rrf => reciprocal_rank_fusion(fts_results, vector_results, config),
        FusionStrategy::WeightedScore => weighted_score_fusion(fts_results, vector_results, config),
    }
}

/// Reciprocal Rank Fusion algorithm.
///
/// Combines ranked results from multiple retrieval methods using the formula:
/// score(d) = sum(1 / (k + rank(d))) for each method where d appears
///
/// # Arguments
///
/// * `fts_results` - Results from full-text search, ordered by relevance
/// * `vector_results` - Results from vector search, ordered by similarity
/// * `config` - Search configuration
///
/// # Returns
///
/// Combined results sorted by RRF score (descending).
pub fn reciprocal_rank_fusion(
    fts_results: Vec<RankedResult>,
    vector_results: Vec<RankedResult>,
    config: &SearchConfig,
) -> Vec<SearchResult> {
    let k = config.rrf_k as f32;

    // Track scores and metadata for each chunk
    struct ChunkInfo {
        document_id: Uuid,
        content: String,
        score: f32,
        fts_rank: Option<u32>,
        vector_rank: Option<u32>,
    }

    let mut chunk_scores: HashMap<Uuid, ChunkInfo> = HashMap::new();

    // Process FTS results
    for result in fts_results {
        let rrf_score = 1.0 / (k + result.rank as f32);
        chunk_scores
            .entry(result.chunk_id)
            .and_modify(|info| {
                info.score += rrf_score;
                info.fts_rank = Some(result.rank);
            })
            .or_insert(ChunkInfo {
                document_id: result.document_id,
                content: result.content,
                score: rrf_score,
                fts_rank: Some(result.rank),
                vector_rank: None,
            });
    }

    // Process vector results
    for result in vector_results {
        let rrf_score = 1.0 / (k + result.rank as f32);
        chunk_scores
            .entry(result.chunk_id)
            .and_modify(|info| {
                info.score += rrf_score;
                info.vector_rank = Some(result.rank);
            })
            .or_insert(ChunkInfo {
                document_id: result.document_id,
                content: result.content,
                score: rrf_score,
                fts_rank: None,
                vector_rank: Some(result.rank),
            });
    }

    // Convert to SearchResult and sort by score
    let mut results: Vec<SearchResult> = chunk_scores
        .into_iter()
        .map(|(chunk_id, info)| SearchResult {
            document_id: info.document_id,
            chunk_id,
            content: info.content,
            score: info.score,
            fts_rank: info.fts_rank,
            vector_rank: info.vector_rank,
        })
        .collect();

    // Normalize scores to 0-1 range
    if let Some(max_score) = results.iter().map(|r| r.score).reduce(f32::max)
        && max_score > 0.0
    {
        for result in &mut results {
            result.score /= max_score;
        }
    }

    // Filter by minimum score
    if config.min_score > 0.0 {
        results.retain(|r| r.score >= config.min_score);
    }

    // Sort by score descending
    results.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // Limit results
    results.truncate(config.limit);

    results
}

/// Weighted score fusion.
///
/// Converts ranks from each method into scores using `1/rank`
/// (so rank 1 → 1.0, rank N → 1/N), then combines them with
/// configurable weights: `fts_weight * fts_score + vector_weight * vector_score`.
///
/// The combined scores are then normalized to [0,1] by dividing by the
/// maximum score; post-processing (normalization, min_score filter, sort,
/// truncate) matches RRF.
pub fn weighted_score_fusion(
    fts_results: Vec<RankedResult>,
    vector_results: Vec<RankedResult>,
    config: &SearchConfig,
) -> Vec<SearchResult> {
    struct ChunkInfo {
        document_id: Uuid,
        content: String,
        score: f32,
        fts_rank: Option<u32>,
        vector_rank: Option<u32>,
    }

    let mut chunk_scores: HashMap<Uuid, ChunkInfo> = HashMap::new();

    // Process FTS results: score = fts_weight * (1 / rank)
    for result in fts_results {
        let score = config.fts_weight * (1.0 / result.rank as f32);
        chunk_scores
            .entry(result.chunk_id)
            .and_modify(|info| {
                info.score += score;
                info.fts_rank = Some(result.rank);
            })
            .or_insert(ChunkInfo {
                document_id: result.document_id,
                content: result.content,
                score,
                fts_rank: Some(result.rank),
                vector_rank: None,
            });
    }

    // Process vector results: score = vector_weight * (1 / rank)
    for result in vector_results {
        let score = config.vector_weight * (1.0 / result.rank as f32);
        chunk_scores
            .entry(result.chunk_id)
            .and_modify(|info| {
                info.score += score;
                info.vector_rank = Some(result.rank);
            })
            .or_insert(ChunkInfo {
                document_id: result.document_id,
                content: result.content,
                score,
                fts_rank: None,
                vector_rank: Some(result.rank),
            });
    }

    let mut results: Vec<SearchResult> = chunk_scores
        .into_iter()
        .map(|(chunk_id, info)| SearchResult {
            document_id: info.document_id,
            chunk_id,
            content: info.content,
            score: info.score,
            fts_rank: info.fts_rank,
            vector_rank: info.vector_rank,
        })
        .collect();

    // Normalize scores to 0-1 range
    if let Some(max_score) = results.iter().map(|r| r.score).reduce(f32::max)
        && max_score > 0.0
    {
        for result in &mut results {
            result.score /= max_score;
        }
    }

    // Filter by minimum score
    if config.min_score > 0.0 {
        results.retain(|r| r.score >= config.min_score);
    }

    // Sort by score descending
    results.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // Limit results
    results.truncate(config.limit);

    results
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_result(chunk_id: Uuid, doc_id: Uuid, rank: u32) -> RankedResult {
        RankedResult {
            chunk_id,
            document_id: doc_id,
            content: format!("content for chunk {}", chunk_id),
            rank,
        }
    }

    #[test]
    fn test_rrf_single_method() {
        let config = SearchConfig::default().with_limit(10);

        let chunk1 = Uuid::new_v4();
        let chunk2 = Uuid::new_v4();
        let doc = Uuid::new_v4();

        let fts_results = vec![make_result(chunk1, doc, 1), make_result(chunk2, doc, 2)];

        let results = reciprocal_rank_fusion(fts_results, Vec::new(), &config);

        assert_eq!(results.len(), 2);
        // First result should have higher score
        assert!(results[0].score > results[1].score);
        // All should have FTS rank
        assert!(results.iter().all(|r| r.fts_rank.is_some()));
        assert!(results.iter().all(|r| r.vector_rank.is_none()));
    }

    #[test]
    fn test_rrf_hybrid_match_boosted() {
        let config = SearchConfig::default().with_limit(10);

        let chunk1 = Uuid::new_v4(); // In both
        let chunk2 = Uuid::new_v4(); // FTS only
        let chunk3 = Uuid::new_v4(); // Vector only
        let doc = Uuid::new_v4();

        let fts_results = vec![make_result(chunk1, doc, 1), make_result(chunk2, doc, 2)];

        let vector_results = vec![make_result(chunk1, doc, 1), make_result(chunk3, doc, 2)];

        let results = reciprocal_rank_fusion(fts_results, vector_results, &config);

        assert_eq!(results.len(), 3);

        // chunk1 should be first (hybrid match)
        assert_eq!(results[0].chunk_id, chunk1);
        assert!(results[0].is_hybrid());
        assert!(results[0].score > results[1].score);

        // Other chunks should not be hybrid
        assert!(!results[1].is_hybrid());
        assert!(!results[2].is_hybrid());
    }

    #[test]
    fn test_rrf_score_normalization() {
        let config = SearchConfig::default();

        let chunk1 = Uuid::new_v4();
        let doc = Uuid::new_v4();

        let fts_results = vec![make_result(chunk1, doc, 1)];

        let results = reciprocal_rank_fusion(fts_results, Vec::new(), &config);

        // Single result should have normalized score of 1.0
        assert_eq!(results.len(), 1);
        assert!((results[0].score - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_rrf_min_score_filter() {
        let config = SearchConfig::default().with_limit(10).with_min_score(0.5);

        let chunk1 = Uuid::new_v4();
        let chunk2 = Uuid::new_v4();
        let chunk3 = Uuid::new_v4();
        let doc = Uuid::new_v4();

        // chunk1 has rank 1, chunk3 has rank 100 (low score)
        let fts_results = vec![
            make_result(chunk1, doc, 1),
            make_result(chunk2, doc, 50),
            make_result(chunk3, doc, 100),
        ];

        let results = reciprocal_rank_fusion(fts_results, Vec::new(), &config);

        // Low-scoring results should be filtered out
        // All results should have score >= 0.5
        for result in &results {
            assert!(result.score >= 0.5);
        }
    }

    #[test]
    fn test_rrf_limit() {
        let config = SearchConfig::default().with_limit(2);

        let doc = Uuid::new_v4();
        let fts_results: Vec<_> = (1..=5)
            .map(|i| make_result(Uuid::new_v4(), doc, i))
            .collect();

        let results = reciprocal_rank_fusion(fts_results, Vec::new(), &config);

        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_rrf_k_parameter() {
        // Higher k values make ranking differences less pronounced
        let chunk1 = Uuid::new_v4();
        let chunk2 = Uuid::new_v4();
        let doc = Uuid::new_v4();

        let fts_results = vec![make_result(chunk1, doc, 1), make_result(chunk2, doc, 2)];

        // Low k: rank 1 score = 1/(10+1) = 0.091, rank 2 = 1/(10+2) = 0.083
        let config_low_k = SearchConfig::default().with_rrf_k(10);
        let results_low = reciprocal_rank_fusion(fts_results.clone(), Vec::new(), &config_low_k);

        // High k: rank 1 score = 1/(100+1) = 0.0099, rank 2 = 1/(100+2) = 0.0098
        let config_high_k = SearchConfig::default().with_rrf_k(100);
        let results_high = reciprocal_rank_fusion(fts_results, Vec::new(), &config_high_k);

        // With low k, the score difference is larger (relatively)
        let diff_low = results_low[0].score - results_low[1].score;
        let diff_high = results_high[0].score - results_high[1].score;

        // Low k should have larger relative difference
        assert!(diff_low > diff_high);
    }

    #[test]
    fn test_search_config_builders() {
        let config = SearchConfig::default()
            .with_limit(20)
            .with_rrf_k(30)
            .with_min_score(0.1);

        assert_eq!(config.limit, 20);
        assert_eq!(config.rrf_k, 30);
        assert!((config.min_score - 0.1).abs() < 0.001);
        assert!(config.use_fts);
        assert!(config.use_vector);

        let fts_only = SearchConfig::default().fts_only();
        assert!(fts_only.use_fts);
        assert!(!fts_only.use_vector);

        let vector_only = SearchConfig::default().vector_only();
        assert!(!vector_only.use_fts);
        assert!(vector_only.use_vector);

        let weighted = SearchConfig::default()
            .with_fusion_strategy(FusionStrategy::WeightedScore)
            .with_fts_weight(0.8)
            .with_vector_weight(0.2);
        assert_eq!(weighted.fusion_strategy, FusionStrategy::WeightedScore);
        assert!((weighted.fts_weight - 0.8).abs() < 0.001);
        assert!((weighted.vector_weight - 0.2).abs() < 0.001);
    }

    #[test]
    fn test_weighted_fusion_basic() {
        // With equal weights, a hybrid match should still rank highest.
        let config = SearchConfig::default()
            .with_fusion_strategy(FusionStrategy::WeightedScore)
            .with_fts_weight(1.0)
            .with_vector_weight(1.0)
            .with_limit(10);

        let chunk1 = Uuid::new_v4(); // In both
        let chunk2 = Uuid::new_v4(); // FTS only
        let chunk3 = Uuid::new_v4(); // Vector only
        let doc = Uuid::new_v4();

        let fts = vec![make_result(chunk1, doc, 1), make_result(chunk2, doc, 2)];
        let vec_results = vec![make_result(chunk1, doc, 1), make_result(chunk3, doc, 2)];

        let results = weighted_score_fusion(fts, vec_results, &config);

        assert_eq!(results.len(), 3);
        // Hybrid match (chunk1) should be first — it gets score from both
        assert_eq!(results[0].chunk_id, chunk1);
        assert!(results[0].is_hybrid());
        assert!(results[0].score > results[1].score);
    }

    #[test]
    fn test_weighted_fusion_fts_boost() {
        // High FTS weight should elevate FTS-only results above vector-only.
        let config = SearchConfig::default()
            .with_fusion_strategy(FusionStrategy::WeightedScore)
            .with_fts_weight(2.0)
            .with_vector_weight(0.5)
            .with_limit(10);

        let chunk_fts = Uuid::new_v4(); // FTS only, rank 2
        let chunk_vec = Uuid::new_v4(); // Vector only, rank 2
        let doc = Uuid::new_v4();

        let fts = vec![make_result(chunk_fts, doc, 2)];
        let vec_results = vec![make_result(chunk_vec, doc, 2)];

        let results = weighted_score_fusion(fts, vec_results, &config);

        assert_eq!(results.len(), 2);
        // FTS result should rank higher because of the 2.0 weight vs 0.5
        assert_eq!(results[0].chunk_id, chunk_fts);
        assert!(results[0].from_fts());
        assert!(!results[0].from_vector());
    }

    #[test]
    fn test_weighted_fusion_single_source() {
        // Only FTS results — should still work correctly.
        let config = SearchConfig::default()
            .with_fusion_strategy(FusionStrategy::WeightedScore)
            .with_limit(10);

        let chunk1 = Uuid::new_v4();
        let chunk2 = Uuid::new_v4();
        let doc = Uuid::new_v4();

        let fts = vec![make_result(chunk1, doc, 1), make_result(chunk2, doc, 3)];

        let results = weighted_score_fusion(fts, Vec::new(), &config);

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].chunk_id, chunk1);
        assert!(results[0].score > results[1].score);
        // Top result should be normalized to 1.0
        assert!((results[0].score - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_weight_setters_reject_invalid() {
        let config = SearchConfig::default();
        let original_fts = config.fts_weight;
        let original_vec = config.vector_weight;

        // NaN is ignored
        let c = config.clone().with_fts_weight(f32::NAN);
        assert!((c.fts_weight - original_fts).abs() < 0.001);

        // Infinity is ignored
        let c = config.clone().with_vector_weight(f32::INFINITY);
        assert!((c.vector_weight - original_vec).abs() < 0.001);

        // Negative is ignored
        let c = config.clone().with_fts_weight(-1.0);
        assert!((c.fts_weight - original_fts).abs() < 0.001);

        // Negative infinity is ignored
        let c = config.clone().with_vector_weight(f32::NEG_INFINITY);
        assert!((c.vector_weight - original_vec).abs() < 0.001);

        // Valid values > 1.0 are accepted (weights don't need to sum to 1.0)
        let c = config.clone().with_fts_weight(2.0);
        assert!((c.fts_weight - 2.0).abs() < 0.001);

        // Zero is valid
        let c = config.clone().with_vector_weight(0.0);
        assert!(c.vector_weight.abs() < 0.001);
    }

    #[test]
    fn test_fuse_results_dispatches_correctly() {
        let chunk1 = Uuid::new_v4();
        let doc = Uuid::new_v4();

        let fts = vec![make_result(chunk1, doc, 1)];

        // RRF strategy
        let rrf_config = SearchConfig::default().with_limit(10);
        let rrf_results = fuse_results(fts.clone(), Vec::new(), &rrf_config);
        assert_eq!(rrf_results.len(), 1);

        // Weighted strategy
        let weighted_config = SearchConfig::default()
            .with_fusion_strategy(FusionStrategy::WeightedScore)
            .with_limit(10);
        let weighted_results = fuse_results(fts, Vec::new(), &weighted_config);
        assert_eq!(weighted_results.len(), 1);

        // Both should normalize single result to 1.0
        assert!((rrf_results[0].score - 1.0).abs() < 0.001);
        assert!((weighted_results[0].score - 1.0).abs() < 0.001);
    }
}
