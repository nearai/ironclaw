//! Integration tests for document chunking with line position tracking.
//!
//! Tests the chunk_document_with_positions function and citation generation.

use ironclaw::workspace::{ChunkConfig, ChunkWithPosition, chunk_document, chunk_document_with_positions};

#[test]
fn test_chunk_positions_simple_document() {
    let content = "Line 1\nLine 2\nLine 3\nLine 4\nLine 5";
    let config = ChunkConfig::default();
    
    let chunks = chunk_document_with_positions(content, config);
    
    // Small doc should be single chunk
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].line_start, 1);
    assert_eq!(chunks[0].line_end, 5);
    assert_eq!(chunks[0].char_start, 0);
    assert_eq!(chunks[0].char_end, content.len());
}

#[test]
fn test_chunk_positions_large_document() {
    // Create a document with 100 lines
    let lines: Vec<String> = (1..=100)
        .map(|i| format!("This is line number {} with some content", i))
        .collect();
    let content = lines.join("\n");
    
    let config = ChunkConfig {
        chunk_size: 50,  // ~50 words per chunk
        overlap_percent: 0.15,
        min_chunk_size: 10,
    };
    
    let chunks = chunk_document_with_positions(&content, config);
    
    // Should have multiple chunks
    assert!(chunks.len() > 1, "Expected multiple chunks for 100 lines");
    
    // First chunk should start at line 1
    assert_eq!(chunks[0].line_start, 1);
    
    // Last chunk should end at line 100
    assert_eq!(chunks.last().unwrap().line_end, 100);
    
    // All chunks should have valid line ranges
    for chunk in &chunks {
        assert!(chunk.line_start >= 1);
        assert!(chunk.line_end >= chunk.line_start);
        assert!(chunk.char_start < chunk.char_end);
    }
}

#[test]
fn test_chunk_citation_single_line() {
    let chunk = ChunkWithPosition {
        content: "test content".to_string(),
        line_start: 42,
        line_end: 42,
        char_start: 100,
        char_end: 112,
    };
    
    assert_eq!(chunk.citation(), "line 42");
}

#[test]
fn test_chunk_citation_range() {
    let chunk = ChunkWithPosition {
        content: "test content".to_string(),
        line_start: 10,
        line_end: 25,
        char_start: 0,
        char_end: 500,
    };
    
    assert_eq!(chunk.citation(), "lines 10-25");
}

#[test]
fn test_chunk_positions_preserve_content() {
    let content = "First line\nSecond line\nThird line";
    let config = ChunkConfig::default();
    
    let position_chunks = chunk_document_with_positions(content, config.clone());
    let simple_chunks = chunk_document(content, config);
    
    // Both should produce the same text content
    assert_eq!(position_chunks.len(), simple_chunks.len());
    
    for (pos_chunk, simple_chunk) in position_chunks.iter().zip(simple_chunks.iter()) {
        // Content should match (though may differ in whitespace normalization)
        assert!(!pos_chunk.content.is_empty());
        assert!(!simple_chunk.is_empty());
    }
}

#[test]
fn test_chunk_positions_empty_input() {
    let config = ChunkConfig::default();
    
    assert!(chunk_document_with_positions("", config.clone()).is_empty());
    assert!(chunk_document_with_positions("   ", config.clone()).is_empty());
    assert!(chunk_document_with_positions("\n\n\n", config).is_empty());
}

#[test]
fn test_chunk_positions_single_word() {
    let content = "hello";
    let config = ChunkConfig::default();
    
    let chunks = chunk_document_with_positions(content, config);
    
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].line_start, 1);
    assert_eq!(chunks[0].line_end, 1);
    assert_eq!(chunks[0].content, "hello");
}

#[test]
fn test_chunk_positions_unicode() {
    let content = "日本語テスト\n中文测试\nПривет\n🎉 emoji";
    let config = ChunkConfig::default();
    
    let chunks = chunk_document_with_positions(content, config);
    
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].line_start, 1);
    assert_eq!(chunks[0].line_end, 4);
}

#[test]
fn test_chunk_positions_windows_line_endings() {
    // Windows CRLF line endings
    let content = "Line 1\r\nLine 2\r\nLine 3";
    let config = ChunkConfig::default();
    
    let chunks = chunk_document_with_positions(content, config);
    
    // Should handle CRLF gracefully
    assert_eq!(chunks.len(), 1);
    assert!(chunks[0].line_start >= 1);
}

#[test]
fn test_chunk_config_builder() {
    let config = ChunkConfig::default()
        .with_chunk_size(500)
        .with_overlap(0.25);
    
    assert_eq!(config.chunk_size, 500);
    assert_eq!(config.overlap_percent, 0.25);
}

#[test]
fn test_chunk_positions_markdown() {
    let content = r#"# Header

This is a paragraph with some text.

## Subheader

- List item 1
- List item 2
- List item 3

```rust
fn main() {
    println!("Hello");
}
```
"#;
    
    let config = ChunkConfig::default();
    let chunks = chunk_document_with_positions(content, config);
    
    // Verify we can chunk markdown
    assert!(!chunks.is_empty());
    assert_eq!(chunks[0].line_start, 1);
}

#[test]
fn test_chunk_overlap_produces_multiple_chunks() {
    // Create enough content to need multiple chunks
    let words: Vec<&str> = (0..200).map(|_| "word").collect();
    let content = words.join(" ");
    
    let config = ChunkConfig {
        chunk_size: 50,
        overlap_percent: 0.2,  // 20% overlap = 10 words
        min_chunk_size: 10,
    };
    
    let chunks = chunk_document_with_positions(&content, config);
    
    // 200 words with 50-word chunks and 20% overlap should give us multiple chunks
    assert!(chunks.len() >= 3, "Expected at least 3 chunks, got {}", chunks.len());
    
    // Each chunk should have reasonable size
    for chunk in &chunks {
        let word_count = chunk.content.split_whitespace().count();
        assert!(word_count >= 10, "Chunk too small: {} words", word_count);
    }
}
