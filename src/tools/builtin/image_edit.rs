//! Image editing tool using NEAR AI cloud-api (FLUX model).

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use base64::Engine;
use secrecy::ExposeSecret;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::config::NearAiConfig;
use crate::context::JobContext;
use crate::tools::tool::{ApprovalRequirement, Tool, ToolError, ToolOutput, ToolRateLimitConfig};
use crate::workspace::Workspace;

/// Tool for editing existing images using NEAR AI cloud-api (FLUX).
pub struct ImageEditTool {
    config: NearAiConfig,
    client: reqwest::Client,
    workspace: Arc<Workspace>,
}

impl ImageEditTool {
    /// Create a new image editing tool.
    pub fn new(config: NearAiConfig, workspace: Arc<Workspace>) -> Self {
        Self {
            config,
            client: reqwest::Client::new(),
            workspace,
        }
    }
}

#[async_trait]
impl Tool for ImageEditTool {
    fn name(&self) -> &str {
        "image_edit"
    }

    fn description(&self) -> &str {
        "Edit an existing image using NEAR AI cloud-api (FLUX) by providing the workspace path and a description of changes. \
         Returns the edited image saved to the workspace."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Workspace path to the source image (e.g., 'images/generated/abc123.b64')"
                },
                "prompt": {
                    "type": "string",
                    "description": "Description of the edits to apply (max 4000 characters)"
                },
                "size": {
                    "type": "string",
                    "enum": ["1024x1024", "1792x1024", "1024x1792"],
                    "description": "Image dimensions. Default: 1024x1024"
                }
            },
            "required": ["path", "prompt"]
        })
    }

    async fn execute(&self, params: Value, _ctx: &JobContext) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        // Parse parameters
        let path = params
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                ToolError::InvalidParameters("Missing or invalid 'path' parameter".to_string())
            })?
            .to_string();

        let prompt = params
            .get("prompt")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                ToolError::InvalidParameters("Missing or invalid 'prompt' parameter".to_string())
            })?
            .to_string();

        if prompt.is_empty() {
            return Err(ToolError::InvalidParameters(
                "Prompt cannot be empty".to_string(),
            ));
        }

        if prompt.len() > 4000 {
            return Err(ToolError::InvalidParameters(format!(
                "Prompt exceeds 4000 character limit (got {})",
                prompt.len()
            )));
        }

        let size = params
            .get("size")
            .and_then(|v| v.as_str())
            .unwrap_or("1024x1024");

        // Read base64 image data from workspace
        let doc = self.workspace.read(&path).await.map_err(|e| {
            ToolError::ExecutionFailed(format!("Failed to read image from workspace: {}", e))
        })?;

        // Decode base64 to bytes
        let image_bytes = base64::engine::general_purpose::STANDARD
            .decode(&doc.content)
            .map_err(|e| {
                ToolError::ExecutionFailed(format!("Failed to decode base64 image data: {}", e))
            })?;

        // Build multipart form
        let form = reqwest::multipart::Form::new()
            .text("model", "black-forest-labs/FLUX.2-klein-4B")
            .part(
                "image",
                reqwest::multipart::Part::bytes(image_bytes).file_name("image.png"),
            )
            .text("prompt", prompt.clone())
            .text("n", "1")
            .text("size", size.to_string())
            .text("response_format", "b64_json");

        // Call NEAR AI cloud-api edit endpoint
        let endpoint = format!(
            "{}/v1/images/edits",
            self.config.base_url.trim_end_matches('/')
        );

        let auth_header = if let Some(api_key) = &self.config.api_key {
            format!("Bearer {}", api_key.expose_secret())
        } else {
            "Bearer ".to_string()
        };

        let response = self
            .client
            .post(&endpoint)
            .header("Authorization", auth_header)
            .multipart(form)
            .timeout(Duration::from_secs(120))
            .send()
            .await
            .map_err(|e| ToolError::ExternalService(format!("NEAR AI image edit failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            return Err(ToolError::ExternalService(format!(
                "NEAR AI image edit error ({}): {}",
                status, error_text
            )));
        }

        let response_json: Value = response.json().await.map_err(|e| {
            ToolError::ExternalService(format!("Failed to parse NEAR AI response: {}", e))
        })?;

        // Extract base64 edited image data
        let edited_base64 = response_json
            .get("data")
            .and_then(|d| d.get(0))
            .and_then(|item| item.get("b64_json"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                ToolError::ExternalService(
                    "Invalid NEAR AI response structure: missing base64 data".to_string(),
                )
            })?
            .to_string();

        // Generate unique filename for edited image
        let edit_id = Uuid::new_v4().to_string();
        let filename = format!("images/generated/{}_edit.png", edit_id);

        // Store edited image to workspace
        let edit_path = format!("images/generated/{}_edit.b64", edit_id);
        self.workspace
            .write(&edit_path, &edited_base64)
            .await
            .map_err(|e| {
                ToolError::ExecutionFailed(format!(
                    "Failed to save edited image to workspace: {}",
                    e
                ))
            })?;

        // Return sentinel JSON for agent_loop to detect and emit SSE event
        Ok(ToolOutput::success(
            json!({
                "type": "image_generated",
                "path": edit_path,
                "data": edited_base64,
                "media_type": "image/png",
                "prompt": prompt,
                "size": size,
                "filename": filename,
                "source_path": path
            }),
            start.elapsed(),
        ))
    }

    fn requires_approval(&self, _params: &Value) -> ApprovalRequirement {
        // Image editing is read-only on external state
        ApprovalRequirement::Never
    }

    fn rate_limit_config(&self) -> Option<ToolRateLimitConfig> {
        // DALL-E is expensive; rate limit aggressively
        Some(ToolRateLimitConfig::new(6, 30))
    }

    fn sensitive_params(&self) -> &[&str] {
        &[]
    }

    fn execution_timeout(&self) -> std::time::Duration {
        // Image editing can take 2+ minutes on the NEAR AI cloud-api
        std::time::Duration::from_secs(180)
    }
}
