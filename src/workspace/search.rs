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
    /// Chunk ID.
    pub chunk_id: Uuid,
    /// Document path (e.g., "daily/2024-01-15.md").
    pub path: String,
    /// Chunk content.
    pub content: String,
    /// Combined RRF score (0.0-1.0 normalized).
    pub score: f32,
    /// Rank in FTS results (1-based, None if not in FTS results).
    pub fts_rank: Option<u32>,
    /// Rank in vector results (1-based, None if not in vector results).
    pub vector_rank: Option<u32>,
    /// Starting line number in the original document (1-based).
    pub line_start: Option<u32>,
    /// Ending line number in the original document (1-based).
    pub line_end: Option<u32>,
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

    /// Generate a citation string for this result.
    ///
    /// Format: `path#lines X-Y` or `path#line X` or just `path` if no line info.
    pub fn citation(&self) -> String {
        match (self.line_start, self.line_end) {
            (Some(start), Some(end)) if start == end => {
                format!("{}#line {}", self.path, start)
            }
            (Some(start), Some(end)) => {
                format!("{}#lines {}-{}", self.path, start, end)
            }
            _ => self.path.clone(),
        }
    }
}

/// Raw result from a single search method.
#[derive(Debug, Clone)]
pub struct RankedResult {
    pub chunk_id: Uuid,
    pub document_id: Uuid,
    pub path: String,
    pub content: String,
    pub rank: u32, // 1-based rank
    pub line_start: Option<u32>,
    pub line_end: Option<u32>,
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
        path: String,
        content: String,
        score: f32,
        fts_rank: Option<u32>,
        vector_rank: Option<u32>,
        line_start: Option<u32>,
        line_end: Option<u32>,
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
                path: result.path,
                content: result.content,
                score: rrf_score,
                fts_rank: Some(result.rank),
                vector_rank: None,
                line_start: result.line_start,
                line_end: result.line_end,
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
                path: result.path,
                content: result.content,
                score: rrf_score,
                fts_rank: None,
                vector_rank: Some(result.rank),
                line_start: result.line_start,
                line_end: result.line_end,
            });
    }

    // Convert to SearchResult and sort by score
    let mut results: Vec<SearchResult> = chunk_scores
        .into_iter()
        .map(|(chunk_id, info)| SearchResult {
            document_id: info.document_id,
            chunk_id,
            path: info.path,
            content: info.content,
            score: info.score,
            fts_rank: info.fts_rank,
            vector_rank: info.vector_rank,
            line_start: info.line_start,
            line_end: info.line_end,
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
            path: "test/document.md".to_string(),
            content: format!("content for chunk {}", chunk_id),
            rank,
            line_start: Some(1),
            line_end: Some(10),
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
}
