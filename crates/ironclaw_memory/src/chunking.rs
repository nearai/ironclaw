//! Document chunking and content hashing.

use sha2::{Digest, Sha256};

/// Configuration for document chunking.
///
/// Ported from the current workspace chunker so Reborn memory indexing preserves
/// existing search recall behavior.
#[derive(Debug, Clone)]
pub struct ChunkConfig {
    pub chunk_size: usize,
    pub overlap_percent: f32,
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
    pub fn with_chunk_size(mut self, size: usize) -> Self {
        self.chunk_size = size.max(1);
        self
    }

    pub fn with_overlap(mut self, percent: f32) -> Self {
        self.overlap_percent = percent.clamp(0.0, 0.5);
        self
    }

    fn effective_chunk_size(&self) -> usize {
        self.chunk_size.max(1)
    }

    fn overlap_size(&self) -> usize {
        (self.effective_chunk_size() as f32 * self.overlap_percent) as usize
    }

    fn step_size(&self) -> usize {
        self.effective_chunk_size()
            .saturating_sub(self.overlap_size())
            .max(1)
    }
}

/// A new chunk to insert for a document.
#[derive(Debug, Clone, PartialEq)]
pub struct MemoryChunkWrite {
    pub content: String,
    pub embedding: Option<Vec<f32>>,
}

/// Split a document into overlapping chunks using current workspace semantics.
pub fn chunk_document(content: &str, config: ChunkConfig) -> Vec<String> {
    if content.is_empty() {
        return Vec::new();
    }

    let words: Vec<&str> = content.split_whitespace().collect();
    if words.is_empty() {
        return Vec::new();
    }

    let chunk_size = config.effective_chunk_size();
    if words.len() <= chunk_size {
        return vec![content.to_string()];
    }

    let step = config.step_size();
    let mut chunks = Vec::new();
    let mut start = 0;

    while start < words.len() {
        let end = (start + chunk_size).min(words.len());
        let chunk_words = &words[start..end];

        if chunk_words.len() < config.min_chunk_size
            && let Some(last) = chunks.pop()
        {
            let combined = format!("{} {}", last, chunk_words.join(" "));
            chunks.push(combined);
            break;
        }

        chunks.push(chunk_words.join(" "));
        start += step;

        if start + config.min_chunk_size >= words.len() && end == words.len() {
            break;
        }
    }

    chunks
}

/// Compute a SHA-256 content hash using the current workspace format.
pub fn content_sha256(content: &str) -> String {
    content_bytes_sha256(content.as_bytes())
}

pub(crate) fn content_bytes_sha256(content: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content);
    format!("sha256:{:x}", hasher.finalize())
}
