//! Image generation tool using NEAR AI cloud-api (FLUX model).

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use secrecy::ExposeSecret;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::config::NearAiConfig;
use crate::context::JobContext;
use crate::tools::tool::{ApprovalRequirement, Tool, ToolError, ToolOutput, ToolRateLimitConfig};
use crate::workspace::Workspace;

/// Tool for generating images from text prompts using NEAR AI cloud-api (FLUX).
pub struct ImageGenerateTool {
    config: NearAiConfig,
    client: reqwest::Client,
    workspace: Arc<Workspace>,
}

impl ImageGenerateTool {
    /// Create a new image generation tool.
    pub fn new(config: NearAiConfig, workspace: Arc<Workspace>) -> Self {
        Self {
            config,
            client: reqwest::Client::new(),
            workspace,
        }
    }
}

#[async_trait]
impl Tool for ImageGenerateTool {
    fn name(&self) -> &str {
        "image_generate"
    }

    fn description(&self) -> &str {
        "Generate an image from a text prompt using NEAR AI cloud-api (FLUX.2-klein-4B). \
         Returns the generated image saved to the workspace."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "prompt": {
                    "type": "string",
                    "description": "Detailed text description of the image to generate (max 4000 characters)"
                },
                "size": {
                    "type": "string",
                    "enum": ["1024x1024", "1792x1024", "1024x1792"],
                    "description": "Image dimensions. Default: 1024x1024"
                }
            },
            "required": ["prompt"]
        })
    }

    async fn execute(&self, params: Value, _ctx: &JobContext) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        // Parse parameters
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

        // Call NEAR AI cloud-api for image generation (FLUX model)
        let request_body = json!({
            "model": "black-forest-labs/FLUX.2-klein-4B",
            "prompt": prompt,
            "n": 1,
            "size": size,
            "response_format": "b64_json"
        });

        let endpoint = format!(
            "{}/v1/images/generations",
            self.config.base_url.trim_end_matches('/')
        );

        let auth_header = if let Some(api_key) = &self.config.api_key {
            format!("Bearer {}", api_key.expose_secret())
        } else {
            // Fallback: use default NEAR AI cloud-api without explicit key
            // (expects auth via environment or other mechanism)
            "Bearer ".to_string()
        };

        let response = self
            .client
            .post(&endpoint)
            .header("Authorization", auth_header)
            .json(&request_body)
            .timeout(Duration::from_secs(120))
            .send()
            .await
            .map_err(|e| {
                ToolError::ExternalService(format!("NEAR AI image generation failed: {}", e))
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            return Err(ToolError::ExternalService(format!(
                "NEAR AI image generation error ({}): {}",
                status, error_text
            )));
        }

        let response_json: Value = response.json().await.map_err(|e| {
            ToolError::ExternalService(format!("Failed to parse NEAR AI response: {}", e))
        })?;

        // Extract base64 image data
        let base64_data = response_json
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

        // Generate unique filename
        let image_id = Uuid::new_v4().to_string();
        let filename = format!("images/generated/{}.png", image_id);

        // Store the image file (with extension) containing the base64 data
        let image_path = format!("images/generated/{}.b64", image_id);
        self.workspace
            .write(&image_path, &base64_data)
            .await
            .map_err(|e| {
                ToolError::ExecutionFailed(format!("Failed to save image to workspace: {}", e))
            })?;

        // Return sentinel JSON for agent_loop to detect and emit SSE event
        Ok(ToolOutput::success(
            json!({
                "type": "image_generated",
                "path": image_path,
                "data": base64_data,
                "media_type": "image/png",
                "prompt": prompt,
                "size": size,
                "filename": filename
            }),
            start.elapsed(),
        ))
    }

    fn requires_approval(&self, _params: &Value) -> ApprovalRequirement {
        // Image generation from a prompt is read-only on external state
        // so no approval needed
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
        // Image generation can take 2+ minutes on the NEAR AI cloud-api
        std::time::Duration::from_secs(180)
    }
}
