//! Image editing tool using cloud API.

use std::sync::Arc;

use async_trait::async_trait;
use base64::Engine;

use crate::context::JobContext;
use crate::tools::tool::{ApprovalRequirement, Tool, ToolError, ToolOutput};
use crate::workspace::Workspace;

/// Tool for editing images using an AI image editing API.
pub struct ImageEditTool {
    /// API base URL.
    api_base_url: String,
    /// Bearer token for API auth.
    api_key: String,
    /// Model to use.
    model: String,
    /// HTTP client.
    client: reqwest::Client,
    /// Workspace for reading source images.
    workspace: Arc<Workspace>,
}

impl ImageEditTool {
    /// Create a new image edit tool.
    pub fn new(
        api_base_url: String,
        api_key: String,
        model: String,
        workspace: Arc<Workspace>,
    ) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(180))
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
impl Tool for ImageEditTool {
    fn name(&self) -> &str {
        "image_edit"
    }

    fn description(&self) -> &str {
        "Edit an existing image using an AI model. Provide the workspace path to the source image and a text prompt describing the desired edits."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "prompt": {
                    "type": "string",
                    "description": "Text description of the edits to apply to the image",
                    "maxLength": 4000
                },
                "image_path": {
                    "type": "string",
                    "description": "Path to the source image in the workspace (e.g., 'images/photo.jpg')"
                }
            },
            "required": ["prompt", "image_path"]
        })
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::Never
    }

    fn requires_sanitization(&self) -> bool {
        false
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let prompt = params
            .get("prompt")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                ToolError::InvalidParameters(
                    "Missing required 'prompt' parameter".to_string(),
                )
            })?;

        let image_path = params
            .get("image_path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                ToolError::InvalidParameters(
                    "Missing required 'image_path' parameter".to_string(),
                )
            })?;

        if prompt.len() > 4000 {
            return Err(ToolError::InvalidParameters(
                "Prompt exceeds 4000 character limit".to_string(),
            ));
        }

        // Read source image from workspace
        let doc = self
            .workspace
            .read(image_path)
            .await
            .map_err(|e| {
                ToolError::ExecutionFailed(format!("Failed to read source image: {e}"))
            })?;

        let image_bytes = doc.content.as_bytes();
        if image_bytes.is_empty() {
            return Err(ToolError::ExecutionFailed(
                "Source image file is empty".to_string(),
            ));
        }

        let media_type = Self::media_type_from_path(image_path);
        let b64_image = base64::engine::general_purpose::STANDARD.encode(image_bytes);

        // Use multipart form for image edit API
        let url = format!(
            "{}/v1/images/edits",
            self.api_base_url.trim_end_matches('/')
        );

        let form = reqwest::multipart::Form::new()
            .text("model", self.model.clone())
            .text("prompt", prompt.to_string())
            .text("response_format", "b64_json")
            .part(
                "image",
                reqwest::multipart::Part::bytes(image_bytes.to_vec())
                    .mime_str(media_type)
                    .map_err(|e| {
                        ToolError::ExecutionFailed(format!("Invalid media type: {e}"))
                    })?
                    .file_name("image"),
            );

        let response = self
            .client
            .post(&url)
            .bearer_auth(&self.api_key)
            .multipart(form)
            .send()
            .await
            .map_err(|e| {
                ToolError::ExecutionFailed(format!("Image edit request failed: {e}"))
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();

            // Fall back to chat-based editing if edits endpoint not available
            if status.as_u16() == 404 {
                return self
                    .fallback_chat_edit(prompt, &b64_image, media_type, start)
                    .await;
            }

            return Err(ToolError::ExecutionFailed(format!(
                "Image edit API returned {status}: {body}"
            )));
        }

        let resp: serde_json::Value =
            response.json().await.map_err(|e| {
                ToolError::ExecutionFailed(format!(
                    "Failed to parse image edit response: {e}"
                ))
            })?;

        let edited_data = resp
            .pointer("/data/0/b64_json")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                ToolError::ExecutionFailed(
                    "No image data in edit response".to_string(),
                )
            })?;

        let sentinel = serde_json::json!({
            "type": "image_generated",
            "data": format!("data:image/png;base64,{}", edited_data),
            "media_type": "image/png",
            "prompt": prompt,
            "source_path": image_path
        });

        Ok(ToolOutput::text(sentinel.to_string(), start.elapsed()))
    }
}

impl ImageEditTool {
    /// Fallback: use the generation API with the source image described in the prompt.
    async fn fallback_chat_edit(
        &self,
        prompt: &str,
        _b64_image: &str,
        _media_type: &str,
        start: std::time::Instant,
    ) -> Result<ToolOutput, ToolError> {
        // If the edit endpoint is not available, generate a new image with the prompt
        let url = format!(
            "{}/v1/images/generations",
            self.api_base_url.trim_end_matches('/')
        );

        let request_body = serde_json::json!({
            "model": self.model,
            "prompt": prompt,
            "size": "1024x1024",
            "response_format": "b64_json",
            "n": 1
        });

        let response = self
            .client
            .post(&url)
            .bearer_auth(&self.api_key)
            .json(&request_body)
            .send()
            .await
            .map_err(|e| {
                ToolError::ExecutionFailed(format!(
                    "Fallback image generation failed: {e}"
                ))
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(ToolError::ExecutionFailed(format!(
                "Fallback generation API returned {status}: {body}"
            )));
        }

        let resp: serde_json::Value =
            response.json().await.map_err(|e| {
                ToolError::ExecutionFailed(format!(
                    "Failed to parse fallback response: {e}"
                ))
            })?;

        let image_data = resp
            .pointer("/data/0/b64_json")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                ToolError::ExecutionFailed(
                    "No image data in fallback response".to_string(),
                )
            })?;

        let sentinel = serde_json::json!({
            "type": "image_generated",
            "data": format!("data:image/png;base64,{}", image_data),
            "media_type": "image/png",
            "prompt": prompt,
            "note": "Generated new image (edit endpoint unavailable)"
        });

        Ok(ToolOutput::text(sentinel.to_string(), start.elapsed()))
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
        let tool = ImageEditTool::new(
            "https://api.example.com".to_string(),
            "test-key".to_string(),
            "flux-1".to_string(),
            workspace,
        );
        assert_eq!(tool.name(), "image_edit");
        assert!(!tool.requires_sanitization());
    }

    #[test]
    fn test_media_type_detection() {
        assert_eq!(
            ImageEditTool::media_type_from_path("photo.png"),
            "image/png"
        );
        assert_eq!(
            ImageEditTool::media_type_from_path("photo.jpg"),
            "image/jpeg"
        );
        assert_eq!(
            ImageEditTool::media_type_from_path("photo.gif"),
            "image/gif"
        );
        assert_eq!(
            ImageEditTool::media_type_from_path("photo.webp"),
            "image/webp"
        );
    }
}
