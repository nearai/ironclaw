//! Document chunking for search indexing.
//!
//! Documents are split into overlapping chunks for better search recall.
//! The overlap ensures context is preserved across chunk boundaries.

/// Configuration for document chunking.
#[derive(Debug, Clone)]
pub struct ChunkConfig {
    /// Target chunk size in words (approximate tokens).
    /// Default: 800 (roughly 800 tokens for English text).
    pub chunk_size: usize,
    /// Overlap percentage between chunks.
    /// Default: 0.15 (15% overlap).
    pub overlap_percent: f32,
    /// Minimum chunk size (don't create tiny trailing chunks).
    /// Default: 50 words.
    pub min_chunk_size: usize,
}

impl Default for ChunkConfig {
    fn default() -> Self {
        Self {
            chunk_size: 800,
            overlap_percent: 0.15,
            min_chunk_size: 50,
        }
    }
}

impl ChunkConfig {
    /// Create a config with a specific chunk size.
    pub fn with_chunk_size(mut self, size: usize) -> Self {
        self.chunk_size = size;
        self
    }

    /// Create a config with a specific overlap percentage.
    pub fn with_overlap(mut self, percent: f32) -> Self {
        self.overlap_percent = percent.clamp(0.0, 0.5);
        self
    }

    /// Calculate the overlap size in words.
    fn overlap_size(&self) -> usize {
        (self.chunk_size as f32 * self.overlap_percent) as usize
    }

    /// Calculate the step size (chunk_size - overlap).
    fn step_size(&self) -> usize {
        self.chunk_size.saturating_sub(self.overlap_size())
    }
}

/// Split a document into overlapping chunks.
///
/// Each chunk contains approximately `chunk_size` words, with `overlap_percent`
/// overlap between adjacent chunks. This ensures that:
/// 1. Context is preserved across chunk boundaries
/// 2. Search can find content that spans chunk boundaries
///
/// # Arguments
///
/// * `content` - The document text to chunk
/// * `config` - Chunking configuration
///
/// # Returns
///
/// A vector of chunk strings. Empty documents return an empty vector.
pub fn chunk_document(content: &str, config: ChunkConfig) -> Vec<String> {
    if content.is_empty() {
        return Vec::new();
    }

    // Split into words while preserving structure
    let words: Vec<&str> = content.split_whitespace().collect();

    if words.is_empty() {
        return Vec::new();
    }

    // If content is smaller than chunk size, return as single chunk
    if words.len() <= config.chunk_size {
        return vec![content.to_string()];
    }

    let step = config.step_size();
    let mut chunks = Vec::new();
    let mut start = 0;

    while start < words.len() {
        let end = (start + config.chunk_size).min(words.len());
        let chunk_words = &words[start..end];

        // Don't create tiny trailing chunks, merge with previous
        if chunk_words.len() < config.min_chunk_size && !chunks.is_empty() {
            let last = chunks.pop().expect("checked non-empty above");
            let combined = format!("{} {}", last, chunk_words.join(" "));
            chunks.push(combined);
            break;
        }

        chunks.push(chunk_words.join(" "));

        // Move to next chunk position
        start += step;

        // Avoid creating duplicate chunks at the end
        if start + config.min_chunk_size >= words.len() && end == words.len() {
            break;
        }
    }

    chunks
}

/// Split content by paragraphs first, then chunk.
///
/// This is better for preserving semantic boundaries.
#[allow(dead_code)] // Alternative chunking strategy for paragraph-aware indexing
pub fn chunk_by_paragraphs(content: &str, config: ChunkConfig) -> Vec<String> {
    if content.is_empty() {
        return Vec::new();
    }

    // Split by double newlines (paragraphs)
    let paragraphs: Vec<&str> = content
        .split("\n\n")
        .map(|p| p.trim())
        .filter(|p| !p.is_empty())
        .collect();

    if paragraphs.is_empty() {
        return chunk_document(content, config);
    }

    let mut chunks = Vec::new();
    let mut current_chunk = String::new();
    let mut current_word_count = 0;

    for paragraph in paragraphs {
        let para_words = paragraph.split_whitespace().count();

        // If this paragraph alone exceeds chunk size, chunk it separately
        if para_words > config.chunk_size {
            // Flush current chunk first
            if !current_chunk.is_empty() {
                chunks.push(current_chunk.trim().to_string());
                current_chunk = String::new();
                current_word_count = 0;
            }
            // Chunk the large paragraph
            let para_chunks = chunk_document(paragraph, config.clone());
            chunks.extend(para_chunks);
            continue;
        }

        // Check if adding this paragraph would exceed chunk size
        if current_word_count + para_words > config.chunk_size {
            // Flush current chunk
            if !current_chunk.is_empty() {
                chunks.push(current_chunk.trim().to_string());
            }
            current_chunk = paragraph.to_string();
            current_word_count = para_words;
        } else {
            // Add paragraph to current chunk
            if !current_chunk.is_empty() {
                current_chunk.push_str("\n\n");
            }
            current_chunk.push_str(paragraph);
            current_word_count += para_words;
        }
    }

    // Flush remaining content
    if !current_chunk.is_empty() {
        // If too small, merge with previous chunk if possible
        if current_word_count < config.min_chunk_size && !chunks.is_empty() {
            let last = chunks.pop().expect("checked non-empty above");
            chunks.push(format!("{}\n\n{}", last, current_chunk.trim()));
        } else {
            chunks.push(current_chunk.trim().to_string());
        }
    }

    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_content() {
        let config = ChunkConfig::default();
        assert!(chunk_document("", config.clone()).is_empty());
        assert!(chunk_document("   ", config).is_empty());
    }

    #[test]
    fn test_small_content() {
        let config = ChunkConfig::default();
        let content = "Hello world, this is a test.";
        let chunks = chunk_document(content, config);

        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], content);
    }

    #[test]
    fn test_exact_chunk_size() {
        let config = ChunkConfig::default().with_chunk_size(5);
        let content = "one two three four five";
        let chunks = chunk_document(content, config);

        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], content);
    }

    #[test]
    fn test_chunking_with_overlap() {
        let config = ChunkConfig {
            chunk_size: 10,
            overlap_percent: 0.2, // 2 word overlap
            min_chunk_size: 3,    // Low threshold for test
        };

        // 20 words
        let content = "one two three four five six seven eight nine ten eleven twelve thirteen fourteen fifteen sixteen seventeen eighteen nineteen twenty";
        let chunks = chunk_document(content, config);

        // Should create overlapping chunks
        assert!(
            chunks.len() >= 2,
            "Expected at least 2 chunks, got {}",
            chunks.len()
        );

        // Each chunk should have roughly 10 words (allowing for overlap/merging)
        for chunk in &chunks {
            let word_count = chunk.split_whitespace().count();
            assert!(word_count >= 3, "Chunk too small: {} words", word_count);
        }
    }

    #[test]
    fn test_overlap_calculation() {
        let config = ChunkConfig::default()
            .with_chunk_size(100)
            .with_overlap(0.15);

        assert_eq!(config.overlap_size(), 15);
        assert_eq!(config.step_size(), 85);
    }

    #[test]
    fn test_paragraph_chunking() {
        let config = ChunkConfig::default().with_chunk_size(20);

        let content = "First paragraph with some words.\n\nSecond paragraph with different content.\n\nThird paragraph here.";
        let chunks = chunk_by_paragraphs(content, config);

        // Should preserve paragraph boundaries
        assert!(!chunks.is_empty());
        for chunk in &chunks {
            // No chunk should start or end with \n\n
            assert!(!chunk.starts_with("\n"));
            assert!(!chunk.ends_with("\n"));
        }
    }

    #[test]
    fn test_large_paragraph_handling() {
        let config = ChunkConfig {
            chunk_size: 10,
            overlap_percent: 0.15,
            min_chunk_size: 3, // Low threshold for test
        };

        // Create a paragraph with 30 words
        let large_para = (1..=30)
            .map(|i| format!("word{}", i))
            .collect::<Vec<_>>()
            .join(" ");
        let content = format!("Short intro.\n\n{}\n\nShort outro.", large_para);

        let chunks = chunk_by_paragraphs(&content, config);

        // Should have multiple chunks due to large paragraph
        // 30 words + 2 intro + 2 outro = 34 words, chunk_size=10
        // Expect at least 3 chunks
        assert!(
            chunks.len() >= 3,
            "Expected at least 3 chunks for 34 words with chunk_size=10, got {}",
            chunks.len()
        );
    }

    #[test]
    fn test_min_chunk_size_merging() {
        let config = ChunkConfig {
            chunk_size: 10,
            overlap_percent: 0.0,
            min_chunk_size: 5,
        };

        // 12 words: should create one chunk of 10, and merge the remaining 2 with it
        let content = "one two three four five six seven eight nine ten eleven twelve";
        let chunks = chunk_document(content, config);

        // Should merge the tiny trailing chunk
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].split_whitespace().count(), 12);
    }
}
