//! Image analysis tool using vision-capable LLM models.

use std::path::PathBuf;

use async_trait::async_trait;
use base64::Engine;
use secrecy::{ExposeSecret, SecretString};

use crate::context::JobContext;
use crate::tools::builtin::path_utils::validate_path;
use crate::tools::tool::{Tool, ToolError, ToolOutput};

/// Tool for analyzing images using a vision-capable model.
pub struct ImageAnalyzeTool {
    /// API base URL.
    api_base_url: String,
    /// Bearer token for API auth.
    api_key: SecretString,
    /// Vision-capable model name.
    model: String,
    /// HTTP client.
    client: reqwest::Client,
    /// Optional base directory for resolving relative image paths.
    base_dir: Option<PathBuf>,
}

impl ImageAnalyzeTool {
    /// Create a new image analysis tool.
    pub fn new(
        api_base_url: String,
        api_key: String,
        model: String,
        base_dir: Option<PathBuf>,
    ) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .unwrap_or_default();
        Self {
            api_base_url,
            api_key: SecretString::from(api_key),
            model,
            client,
            base_dir,
        }
    }

    /// Read binary image bytes from filesystem.
    ///
    /// Validates the path against the base directory sandbox to prevent
    /// path traversal attacks, then reads the file bytes.
    async fn read_image_bytes(&self, image_path: &str) -> Result<Vec<u8>, ToolError> {
        let resolved = validate_path(image_path, self.base_dir.as_deref())?;

        tokio::fs::read(&resolved)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to read image file: {e}")))
    }
}

#[async_trait]
impl Tool for ImageAnalyzeTool {
    fn name(&self) -> &str {
        "image_analyze"
    }

    fn description(&self) -> &str {
        "Analyze an image using a vision-capable AI model. Provide a workspace path to the image and an optional analysis question."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "image_path": {
                    "type": "string",
                    "description": "Path to the image file in the workspace (e.g., 'images/photo.jpg')"
                },
                "question": {
                    "type": "string",
                    "description": "Specific question to answer about the image. Defaults to general analysis.",
                    "default": "Describe this image in detail."
                }
            },
            "required": ["image_path"]
        })
    }

    fn requires_sanitization(&self) -> bool {
        true
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let image_path = params
            .get("image_path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                ToolError::InvalidParameters("Missing required 'image_path' parameter".to_string())
            })?;

        let question = params
            .get("question")
            .and_then(|v| v.as_str())
            .unwrap_or("Describe this image in detail.");

        // Read binary image bytes directly from filesystem
        let image_bytes = self.read_image_bytes(image_path).await?;
        if image_bytes.is_empty() {
            return Err(ToolError::ExecutionFailed(
                "Image file is empty".to_string(),
            ));
        }

        let media_type = super::media_type_from_path(image_path);
        let b64 = base64::engine::general_purpose::STANDARD.encode(&image_bytes);
        let data_url = format!("data:{media_type};base64,{b64}");

        // Call vision model via chat completions API
        let url = format!(
            "{}/v1/chat/completions",
            self.api_base_url.trim_end_matches('/')
        );

        let request_body = serde_json::json!({
            "model": &self.model,
            "messages": [{
                "role": "user",
                "content": [
                    {
                        "type": "text",
                        "text": question
                    },
                    {
                        "type": "image_url",
                        "image_url": {
                            "url": data_url
                        }
                    }
                ]
            }],
            "max_tokens": 2048
        });

        let response = self
            .client
            .post(&url)
            .bearer_auth(self.api_key.expose_secret())
            .json(&request_body)
            .send()
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Vision API request failed: {e}")))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(ToolError::ExecutionFailed(format!(
                "Vision API returned {status}: {body}"
            )));
        }

        let resp: serde_json::Value = response.json().await.map_err(|e| {
            ToolError::ExecutionFailed(format!("Failed to parse vision API response: {e}"))
        })?;

        let analysis = resp
            .pointer("/choices/0/message/content")
            .and_then(|v| v.as_str())
            .unwrap_or("No analysis available.");

        Ok(ToolOutput::text(analysis, start.elapsed()))
    }

    fn is_concurrent_safe(&self, _params: &serde_json::Value) -> bool {
        // No local state mutation — the classification criterion. While this
        // tool makes outbound LLM API calls (external cost), rate limiting
        // for external API usage is handled by the separate `rate_limit_config`
        // mechanism, not the concurrency classifier.
        true
    }
}

#[cfg(test)]
mod tests {
    use super::super::media_type_from_path;
    use super::*;
    use crate::tools::tool::ApprovalRequirement;
    use tempfile::TempDir;

    #[test]
    fn test_media_type_detection() {
        assert_eq!(media_type_from_path("photo.png"), "image/png");
        assert_eq!(media_type_from_path("photo.jpg"), "image/jpeg");
        assert_eq!(media_type_from_path("photo.jpeg"), "image/jpeg");
        assert_eq!(media_type_from_path("photo.gif"), "image/gif");
        assert_eq!(media_type_from_path("photo.webp"), "image/webp");
        assert_eq!(media_type_from_path("photo.bmp"), "image/bmp");
        assert_eq!(media_type_from_path("photo.svg"), "image/svg+xml");
    }

    #[test]
    fn test_requires_approval_returns_never() {
        let tool = ImageAnalyzeTool::new(
            "https://api.example.com".to_string(),
            "test-key".to_string(),
            "gpt-4o".to_string(),
            None,
        );
        assert_eq!(
            tool.requires_approval(&serde_json::json!({})),
            ApprovalRequirement::Never
        );
    }

    #[tokio::test]
    async fn test_read_image_bytes_rejects_path_traversal() {
        let dir = TempDir::new().unwrap();
        let tool = ImageAnalyzeTool::new(
            "https://api.example.com".to_string(),
            "test-key".to_string(),
            "gpt-4o".to_string(),
            Some(dir.path().to_path_buf()),
        );

        let result = tool.read_image_bytes("../../etc/passwd").await;
        assert!(
            result.is_err(),
            "Should reject path traversal, got: {:?}",
            result
        );
    }

    #[tokio::test]
    async fn test_read_image_bytes_rejects_absolute_path_outside_sandbox() {
        let dir = TempDir::new().unwrap();
        let tool = ImageAnalyzeTool::new(
            "https://api.example.com".to_string(),
            "test-key".to_string(),
            "gpt-4o".to_string(),
            Some(dir.path().to_path_buf()),
        );

        let result = tool.read_image_bytes("/etc/passwd").await;
        assert!(
            result.is_err(),
            "Should reject absolute path outside sandbox, got: {:?}",
            result
        );
    }
}
