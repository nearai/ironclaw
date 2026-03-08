//! Image editing tool using cloud API.

use std::path::PathBuf;

use async_trait::async_trait;
use secrecy::{ExposeSecret, SecretString};

use crate::context::JobContext;
use crate::tools::tool::{ApprovalRequirement, Tool, ToolError, ToolOutput};

/// Tool for editing images using an AI image editing API.
pub struct ImageEditTool {
    /// API base URL.
    api_base_url: String,
    /// Bearer token for API auth.
    api_key: SecretString,
    /// Model to use.
    model: String,
    /// HTTP client.
    client: reqwest::Client,
    /// Optional base directory for resolving relative image paths.
    base_dir: Option<PathBuf>,
}

impl ImageEditTool {
    /// Create a new image edit tool.
    pub fn new(
        api_base_url: String,
        api_key: String,
        model: String,
        base_dir: Option<PathBuf>,
    ) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(180))
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
    async fn read_image_bytes(&self, image_path: &str) -> Result<Vec<u8>, ToolError> {
        let resolved = if let Some(base) = &self.base_dir {
            let candidate = base.join(image_path);
            if candidate.exists() {
                candidate
            } else {
                PathBuf::from(image_path)
            }
        } else {
            PathBuf::from(image_path)
        };

        let canonical = resolved.canonicalize().map_err(|e| {
            ToolError::ExecutionFailed(format!("Image path not found: {e}"))
        })?;

        tokio::fs::read(&canonical).await.map_err(|e| {
            ToolError::ExecutionFailed(format!("Failed to read image file: {e}"))
        })
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
        ApprovalRequirement::UnlessAutoApproved
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

        // Read binary image bytes directly from filesystem
        let image_bytes = self.read_image_bytes(image_path).await?;
        if image_bytes.is_empty() {
            return Err(ToolError::ExecutionFailed(
                "Source image file is empty".to_string(),
            ));
        }

        let media_type = super::media_type_from_path(image_path);

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
                reqwest::multipart::Part::bytes(image_bytes)
                    .mime_str(media_type)
                    .map_err(|e| {
                        ToolError::ExecutionFailed(format!("Invalid media type: {e}"))
                    })?
                    .file_name("image"),
            );

        let response = self
            .client
            .post(&url)
            .bearer_auth(self.api_key.expose_secret())
            .multipart(form)
            .send()
            .await
            .map_err(|e| {
                ToolError::ExecutionFailed(format!("Image edit request failed: {e}"))
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();

            // Fall back to generation if edits endpoint not available
            if status.as_u16() == 404 {
                tracing::warn!(
                    "Image edit endpoint returned 404, falling back to generation API. \
                     Note: the source image will NOT be used — a new image will be generated from the prompt alone."
                );
                return self.fallback_generate(prompt, start).await;
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
    /// Fallback: generate a new image from the prompt when the edit endpoint is unavailable.
    ///
    /// The source image is NOT used — this generates a completely new image.
    /// The response includes a `note` field warning the user.
    async fn fallback_generate(
        &self,
        prompt: &str,
        start: std::time::Instant,
    ) -> Result<ToolOutput, ToolError> {
        let url = format!(
            "{}/v1/images/generations",
            self.api_base_url.trim_end_matches('/')
        );

        let request_body = serde_json::json!({
            "model": &self.model,
            "prompt": prompt,
            "size": "1024x1024",
            "response_format": "b64_json",
            "n": 1
        });

        let response = self
            .client
            .post(&url)
            .bearer_auth(self.api_key.expose_secret())
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
            "note": "Generated new image (edit endpoint unavailable — source image was NOT used)"
        });

        Ok(ToolOutput::text(sentinel.to_string(), start.elapsed()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tool_metadata() {
        let tool = ImageEditTool::new(
            "https://api.example.com".to_string(),
            "test-key".to_string(),
            "flux-1".to_string(),
            None,
        );
        assert_eq!(tool.name(), "image_edit");
        assert!(!tool.requires_sanitization());
        assert_eq!(
            tool.requires_approval(&serde_json::json!({})),
            ApprovalRequirement::UnlessAutoApproved
        );
    }
}
