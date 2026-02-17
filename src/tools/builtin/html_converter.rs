//! HTML to Markdown conversion for HTTP responses.
//!
//! Two-stage pipeline: readability (extract article) -> html-to-markdown-rs (convert to md).
//! When the `html-to-markdown` feature is disabled, passthrough only.

use crate::tools::tool::ToolError;

#[cfg(feature = "html-to-markdown")]
use html_to_markdown_rs::convert;
#[cfg(feature = "html-to-markdown")]
use readabilityrs::Readability;

#[cfg(not(feature = "html-to-markdown"))]
pub fn convert_html_to_markdown(html: &str, _url: &str) -> Result<String, ToolError> {
    Ok(html.to_string())
}

#[cfg(feature = "html-to-markdown")]
pub fn convert_html_to_markdown(html: &str, url: &str) -> Result<String, ToolError> {
    let readability = Readability::new(html, Some(url), None)
        .map_err(|e| ToolError::ExecutionFailed(format!("readability parser: {:?}", e)))?;

    let article = readability.parse().ok_or_else(|| {
        ToolError::ExecutionFailed("failed to extract article content".to_string())
    })?;

    let clean_html = article.content.ok_or_else(|| {
        ToolError::ExecutionFailed("no content extracted from article".to_string())
    })?;

    let markdown = convert(&clean_html, None)
        .map_err(|e| ToolError::ExecutionFailed(format!("HTML to markdown: {}", e)))?;

    Ok(markdown)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passthrough_returns_input_unchanged_when_feature_disabled() {
        #[cfg(not(feature = "html-to-markdown"))]
        {
            let html = "<html><body>raw</body></html>";
            let out = convert_html_to_markdown(html, "https://example.com/").unwrap();
            assert_eq!(out, html);
        }
    }

    #[test]
    fn passthrough_ignores_url_when_feature_disabled() {
        #[cfg(not(feature = "html-to-markdown"))]
        {
            let html = "anything";
            let _ = convert_html_to_markdown(html, "").unwrap();
            let _ = convert_html_to_markdown(html, "https://example.com/page").unwrap();
        }
    }

    #[cfg(feature = "html-to-markdown")]
    #[test]
    fn simple_article_extracted_and_converted_to_markdown() {
        // Readability needs enough content (default char_threshold ~500) and clear main content.
        let html = r#"<!DOCTYPE html>
<html><head><title>Test</title></head><body>
<nav><a href="/">Home</a></nav>
<main>
  <article>
    <h1>Test Title</h1>
    <p>First paragraph with enough text so that readability's scoring finds this as the main content block. We need to exceed the default character threshold.</p>
    <p>Second paragraph. More body text here to make the article clearly the dominant content area versus the short nav and footer.</p>
    <p>Third paragraph for good measure. The extraction algorithm scores candidates by paragraph count and text length; this block should win.</p>
  </article>
</main>
<footer><p>Footer</p></footer>
</body></html>"#;
        let out = convert_html_to_markdown(html, "https://example.com/article").unwrap();
        assert!(
            out.contains("Test Title"),
            "expected title in output: {}",
            out
        );
        assert!(
            out.contains("First paragraph"),
            "expected content in output: {}",
            out
        );
        assert!(
            out.contains("Second paragraph"),
            "expected content in output: {}",
            out
        );
        assert!(
            !out.contains("<article>"),
            "expected markdown, not raw HTML"
        );
    }

    #[cfg(feature = "html-to-markdown")]
    #[test]
    fn returns_execution_error_on_empty_html() {
        let result = convert_html_to_markdown("", "https://example.com/");
        let err = result.unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("Execution failed") || msg.contains("extract") || msg.contains("content"),
            "{}",
            msg
        );
    }

    #[cfg(feature = "html-to-markdown")]
    #[test]
    fn returns_execution_error_on_plain_text_not_html() {
        let result = convert_html_to_markdown("not html at all", "https://example.com/");
        let err = result.unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("Execution failed")
                || msg.contains("extract")
                || msg.contains("content")
                || msg.contains("parser"),
            "{}",
            msg
        );
    }
}
