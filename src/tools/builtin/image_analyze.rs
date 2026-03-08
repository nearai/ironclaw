//! Image analysis tool using vision-capable LLM models.

use std::sync::Arc;

use async_trait::async_trait;
use base64::Engine;

use crate::context::JobContext;
use crate::tools::tool::{ApprovalRequirement, Tool, ToolError, ToolOutput};
use crate::workspace::Workspace;

/// Tool for analyzing images using a vision-capable model.
pub struct ImageAnalyzeTool {
    /// API base URL.
    api_base_url: String,
    /// Bearer token for API auth.
    api_key: String,
    /// Vision-capable model name.
    model: String,
    /// HTTP client.
    client: reqwest::Client,
    /// Workspace for reading image files.
    workspace: Arc<Workspace>,
}

impl ImageAnalyzeTool {
    /// Create a new image analysis tool.
    pub fn new(
        api_base_url: String,
        api_key: String,
        model: String,
        workspace: Arc<Workspace>,
    ) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .unwrap_or_default();
        Self {
            api_base_url,
            api_key,
            model,
            client,
            workspace,
        }
    }

    /// Detect media type from file extension.
    fn media_type_from_path(path: &str) -> &'static str {
        let lower = path.to_lowercase();
        if lower.ends_with(".png") {
            "image/png"
        } else if lower.ends_with(".gif") {
            "image/gif"
        } else if lower.ends_with(".webp") {
            "image/webp"
        } else {
            "image/jpeg"
        }
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

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::Never
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
                ToolError::InvalidParameters(
                    "Missing required 'image_path' parameter".to_string(),
                )
            })?;

        let question = params
            .get("question")
            .and_then(|v| v.as_str())
            .unwrap_or("Describe this image in detail.");

        // Read image from workspace
        let doc = self
            .workspace
            .read(image_path)
            .await
            .map_err(|e| {
                ToolError::ExecutionFailed(format!(
                    "Failed to read image from workspace: {e}"
                ))
            })?;

        let image_bytes = doc.content.as_bytes();
        if image_bytes.is_empty() {
            return Err(ToolError::ExecutionFailed(
                "Image file is empty".to_string(),
            ));
        }

        let media_type = Self::media_type_from_path(image_path);
        let b64 = base64::engine::general_purpose::STANDARD.encode(image_bytes);
        let data_url = format!("data:{media_type};base64,{b64}");

        // Call vision model via chat completions API
        let url = format!(
            "{}/v1/chat/completions",
            self.api_base_url.trim_end_matches('/')
        );

        let request_body = serde_json::json!({
            "model": self.model,
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
            .bearer_auth(&self.api_key)
            .json(&request_body)
            .send()
            .await
            .map_err(|e| {
                ToolError::ExecutionFailed(format!("Vision API request failed: {e}"))
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(ToolError::ExecutionFailed(format!(
                "Vision API returned {status}: {body}"
            )));
        }

        let resp: serde_json::Value =
            response.json().await.map_err(|e| {
                ToolError::ExecutionFailed(format!(
                    "Failed to parse vision API response: {e}"
                ))
            })?;

        let analysis = resp
            .pointer("/choices/0/message/content")
            .and_then(|v| v.as_str())
            .unwrap_or("No analysis available.");

        Ok(ToolOutput::text(analysis, start.elapsed()))
    }
}

#[cfg(all(test, feature = "postgres"))]
mod tests {
    use super::*;
    use crate::workspace::Workspace;

    fn make_test_workspace() -> Arc<Workspace> {
        Arc::new(Workspace::new(
            "test_user",
            deadpool_postgres::Pool::builder(deadpool_postgres::Manager::new(
                tokio_postgres::Config::new(),
                tokio_postgres::NoTls,
            ))
            .build()
            .unwrap(),
        ))
    }

    #[test]
    fn test_tool_metadata() {
        let workspace = make_test_workspace();
        let tool = ImageAnalyzeTool::new(
            "https://api.example.com".to_string(),
            "test-key".to_string(),
            "gpt-4o".to_string(),
            workspace,
        );
        assert_eq!(tool.name(), "image_analyze");
        assert!(tool.requires_sanitization());
    }

    #[test]
    fn test_media_type_detection() {
        assert_eq!(
            ImageAnalyzeTool::media_type_from_path("photo.png"),
            "image/png"
        );
        assert_eq!(
            ImageAnalyzeTool::media_type_from_path("photo.jpg"),
            "image/jpeg"
        );
        assert_eq!(
            ImageAnalyzeTool::media_type_from_path("photo.jpeg"),
            "image/jpeg"
        );
        assert_eq!(
            ImageAnalyzeTool::media_type_from_path("photo.gif"),
            "image/gif"
        );
        assert_eq!(
            ImageAnalyzeTool::media_type_from_path("photo.webp"),
            "image/webp"
        );
        assert_eq!(
            ImageAnalyzeTool::media_type_from_path("photo.bmp"),
            "image/jpeg"
        );
    }
}
