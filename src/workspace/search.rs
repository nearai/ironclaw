//! Hybrid search combining full-text and semantic search.
//!
//! Uses Reciprocal Rank Fusion (RRF) to combine results from:
//! 1. PostgreSQL full-text search (ts_rank_cd)
//! 2. pgvector cosine similarity search
//!
//! RRF formula: score = sum(1 / (k + rank)) for each retrieval method
//! This is robust to different score scales and produces better results
//! than simple score averaging.

use std::collections::HashMap;

use uuid::Uuid;

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
}

/// A search result with hybrid scoring.
#[derive(Debug, Clone)]
pub struct SearchResult {
    /// Document ID containing this chunk.
    pub document_id: Uuid,
    /// File path of the source document.
    pub document_path: String,
    /// Chunk ID.
    pub chunk_id: Uuid,
    /// Chunk content.
    pub content: String,
    /// Combined RRF score (0.0-1.0 normalized).
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
    /// File path of the source document.
    pub document_path: String,
    pub content: String,
    pub rank: u32, // 1-based rank
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
        document_path: String,
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
                document_path: result.document_path,
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
                document_path: result.document_path,
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
            document_path: info.document_path,
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
            document_path: format!("docs/{}.md", doc_id),
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
    }

    // --- Edge case tests ---

    #[test]
    fn test_rrf_both_empty() {
        let config = SearchConfig::default();
        let results = reciprocal_rank_fusion(Vec::new(), Vec::new(), &config);
        assert!(results.is_empty());
    }

    #[test]
    fn test_rrf_fts_only_no_vector() {
        let config = SearchConfig::default().with_limit(10);

        let chunk1 = Uuid::new_v4();
        let chunk2 = Uuid::new_v4();
        let chunk3 = Uuid::new_v4();
        let doc = Uuid::new_v4();

        let fts_results = vec![
            make_result(chunk1, doc, 1),
            make_result(chunk2, doc, 2),
            make_result(chunk3, doc, 3),
        ];

        let results = reciprocal_rank_fusion(fts_results, Vec::new(), &config);

        assert_eq!(results.len(), 3);
        // All results should come from FTS only
        assert!(results.iter().all(|r| r.from_fts()));
        assert!(results.iter().all(|r| !r.from_vector()));
        assert!(results.iter().all(|r| !r.is_hybrid()));
        // Scores should be in descending order
        for w in results.windows(2) {
            assert!(w[0].score >= w[1].score);
        }
    }

    #[test]
    fn test_rrf_vector_only_no_fts() {
        let config = SearchConfig::default().with_limit(10);

        let chunk1 = Uuid::new_v4();
        let chunk2 = Uuid::new_v4();
        let chunk3 = Uuid::new_v4();
        let doc = Uuid::new_v4();

        let vector_results = vec![
            make_result(chunk1, doc, 1),
            make_result(chunk2, doc, 2),
            make_result(chunk3, doc, 3),
        ];

        let results = reciprocal_rank_fusion(Vec::new(), vector_results, &config);

        assert_eq!(results.len(), 3);
        // All results should come from vector only
        assert!(results.iter().all(|r| r.from_vector()));
        assert!(results.iter().all(|r| !r.from_fts()));
        assert!(results.iter().all(|r| !r.is_hybrid()));
        // Scores should be in descending order
        for w in results.windows(2) {
            assert!(w[0].score >= w[1].score);
        }
    }

    #[test]
    fn test_rrf_duplicate_chunks_merged() {
        let config = SearchConfig::default().with_limit(10);

        let shared_chunk = Uuid::new_v4();
        let fts_only_chunk = Uuid::new_v4();
        let vector_only_chunk = Uuid::new_v4();
        let doc = Uuid::new_v4();

        // shared_chunk appears at rank 2 in FTS and rank 3 in vector
        let fts_results = vec![
            make_result(fts_only_chunk, doc, 1),
            make_result(shared_chunk, doc, 2),
        ];
        let vector_results = vec![
            make_result(vector_only_chunk, doc, 1),
            make_result(shared_chunk, doc, 3),
        ];

        let results = reciprocal_rank_fusion(fts_results, vector_results, &config);

        // Should have 3 unique chunks (not 4)
        assert_eq!(results.len(), 3);

        // Find the shared chunk in results
        let shared = results.iter().find(|r| r.chunk_id == shared_chunk).unwrap();
        assert!(shared.is_hybrid());
        assert_eq!(shared.fts_rank, Some(2));
        assert_eq!(shared.vector_rank, Some(3));

        // The shared chunk's pre-normalization score is 1/(k+2) + 1/(k+3),
        // which is higher than either single-method chunk at rank 1: 1/(k+1).
        // After normalization the shared chunk should be the top result.
        assert_eq!(results[0].chunk_id, shared_chunk);
    }

    #[test]
    fn test_rrf_limit_zero_returns_empty() {
        let config = SearchConfig::default().with_limit(0);

        let doc = Uuid::new_v4();
        let fts_results = vec![
            make_result(Uuid::new_v4(), doc, 1),
            make_result(Uuid::new_v4(), doc, 2),
        ];

        let results = reciprocal_rank_fusion(fts_results, Vec::new(), &config);

        assert!(results.is_empty());
    }

    #[test]
    fn test_rrf_min_score_one_filters_all() {
        // RRF scores are always < 1.0 before normalization (1/(k+rank) where k>=1, rank>=1).
        // After normalization the top result gets score=1.0, so min_score=1.0 should
        // keep only the single top result. To truly filter everything, we need
        // min_score > 1.0 -- but with_min_score clamps to 1.0.
        // With a single result: normalized score = 1.0, so it passes min_score=1.0.
        // With multiple results: only the top (score=1.0) survives.
        // To filter ALL results we need to ensure none reach 1.0 -- but normalization
        // always makes the max = 1.0. So min_score=1.0 keeps exactly 1 result (the top).
        //
        // Verified: the retain check is `score >= min_score` and the top score
        // is normalized to exactly 1.0, so one result survives.
        let config = SearchConfig::default().with_limit(10).with_min_score(1.0);

        let doc = Uuid::new_v4();
        let fts_results = vec![
            make_result(Uuid::new_v4(), doc, 1),
            make_result(Uuid::new_v4(), doc, 2),
            make_result(Uuid::new_v4(), doc, 3),
        ];

        let results = reciprocal_rank_fusion(fts_results, Vec::new(), &config);

        // After normalization the top result has score 1.0, so exactly 1 survives
        assert_eq!(results.len(), 1);
        assert!((results[0].score - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_search_config_fts_only() {
        let config = SearchConfig::default().fts_only();

        assert!(config.use_fts);
        assert!(!config.use_vector);
        // Other defaults should be preserved
        assert_eq!(config.limit, 10);
        assert_eq!(config.rrf_k, 60);
        assert!((config.min_score - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_search_config_vector_only() {
        let config = SearchConfig::default().vector_only();

        assert!(!config.use_fts);
        assert!(config.use_vector);
        // Other defaults should be preserved
        assert_eq!(config.limit, 10);
        assert_eq!(config.rrf_k, 60);
        assert!((config.min_score - 0.0).abs() < f32::EPSILON);
    }
}
