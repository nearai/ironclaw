//! Image generation tool using cloud API.

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};

use crate::context::JobContext;
use crate::tools::builtin::image_api_endpoint_url;
use crate::tools::{Tool, ToolError, ToolOutput};

/// Tool for generating images using FLUX or compatible image generation APIs.
pub struct ImageGenerateTool {
    /// API base URL (e.g., "https://cloud-api.near.ai").
    api_base_url: String,
    /// Bearer token for API auth.
    api_key: SecretString,
    /// Model to use (e.g., "black-forest-labs/FLUX.1-schnell").
    model: String,
    /// HTTP client.
    client: reqwest::Client,
}

#[derive(Debug, Serialize)]
struct ImageGenRequest {
    model: String,
    prompt: String,
    size: String,
    response_format: String,
    n: u32,
}

#[derive(Debug, Deserialize)]
struct ImageGenResponse {
    data: Vec<ImageGenData>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct ImageGenData {
    b64_json: Option<String>,
    url: Option<String>,
}

impl ImageGenerateTool {
    /// Create a new image generation tool.
    pub fn new(api_base_url: String, api_key: String, model: String) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(180))
            .build()
            .unwrap_or_default();
        Self {
            api_base_url,
            api_key: SecretString::from(api_key),
            model,
            client,
        }
    }

    fn endpoint_url(&self, path: &str) -> String {
        image_api_endpoint_url(&self.api_base_url, path)
    }
}

pub(super) fn infer_generated_image_media_type(image_b64: &str) -> &'static str {
    let prefix_len = image_b64.len().min(64);
    let decodable_len = prefix_len - (prefix_len % 4);
    if decodable_len == 0 {
        return "image/png";
    }
    let prefix = image_b64.get(..decodable_len).unwrap_or("");
    let decoded = STANDARD.decode(prefix);
    let Ok(bytes) = decoded else {
        return "image/png";
    };

    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        "image/jpeg"
    } else if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        "image/png"
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        "image/gif"
    } else if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        "image/webp"
    } else {
        "image/png"
    }
}

#[async_trait]
impl Tool for ImageGenerateTool {
    fn name(&self) -> &str {
        "image_generate"
    }

    fn description(&self) -> &str {
        "Generate an image from a text prompt using an AI image generation model (e.g., FLUX). Returns the generated image data."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "prompt": {
                    "type": "string",
                    "description": "Text description of the image to generate (max 4000 chars)",
                    "maxLength": 4000
                },
                "size": {
                    "type": "string",
                    "description": "Image dimensions",
                    "enum": ["1024x1024", "1792x1024", "1024x1792"],
                    "default": "1024x1024"
                }
            },
            "required": ["prompt"]
        })
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
                ToolError::InvalidParameters("Missing required 'prompt' parameter".to_string())
            })?;

        if prompt.len() > 4000 {
            return Err(ToolError::InvalidParameters(
                "Prompt exceeds 4000 character limit".to_string(),
            ));
        }

        let size = params
            .get("size")
            .and_then(|v| v.as_str())
            .unwrap_or("1024x1024");

        // Validate size
        if !["1024x1024", "1792x1024", "1024x1792"].contains(&size) {
            return Err(ToolError::InvalidParameters(format!(
                "Invalid size '{}'. Must be 1024x1024, 1792x1024, or 1024x1792",
                size
            )));
        }

        let url = self.endpoint_url("/images/generations");

        let request_body = ImageGenRequest {
            model: self.model.clone(),
            prompt: prompt.to_string(),
            size: size.to_string(),
            response_format: "b64_json".to_string(),
            n: 1,
        };

        let response = self
            .client
            .post(&url)
            .bearer_auth(self.api_key.expose_secret())
            .json(&request_body)
            .send()
            .await
            .map_err(|e| {
                ToolError::ExecutionFailed(format!("Image generation request failed: {e}"))
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(ToolError::ExecutionFailed(format!(
                "Image generation API returned {status}: {body}"
            )));
        }

        let gen_response: ImageGenResponse = response.json().await.map_err(|e| {
            ToolError::ExecutionFailed(format!("Failed to parse image generation response: {e}"))
        })?;

        let image_data = gen_response
            .data
            .first()
            .and_then(|d| d.b64_json.as_deref())
            .ok_or_else(|| ToolError::ExecutionFailed("No image data in response".to_string()))?;

        let media_type = infer_generated_image_media_type(image_data);

        // Return sentinel JSON for image display.
        let sentinel = serde_json::json!({
            "type": "image_generated",
            "data": format!("data:{media_type};base64,{}", image_data),
            "media_type": media_type,
            "prompt": prompt,
            "size": size
        });

        Ok(ToolOutput::text(sentinel.to_string(), start.elapsed()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::tool::ApprovalRequirement;

    #[test]
    fn test_tool_metadata() {
        let tool = ImageGenerateTool::new(
            "https://api.example.com".to_string(),
            "test-key".to_string(),
            "flux-1".to_string(),
        );
        assert_eq!(tool.name(), "image_generate");
        assert_eq!(
            tool.requires_approval(&serde_json::json!({})),
            ApprovalRequirement::Never
        );

        let schema = tool.parameters_schema();
        assert!(schema["properties"]["prompt"].is_object());
        assert!(schema["properties"]["size"].is_object());
    }

    #[tokio::test]
    async fn test_missing_prompt() {
        let tool = ImageGenerateTool::new(
            "https://api.example.com".to_string(),
            "test-key".to_string(),
            "flux-1".to_string(),
        );
        let ctx = JobContext::default();
        let result = tool.execute(serde_json::json!({}), &ctx).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_invalid_size() {
        let tool = ImageGenerateTool::new(
            "https://api.example.com".to_string(),
            "test-key".to_string(),
            "flux-1".to_string(),
        );
        let ctx = JobContext::default();
        let result = tool
            .execute(
                serde_json::json!({"prompt": "a cat", "size": "999x999"}),
                &ctx,
            )
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_prompt_too_long() {
        let tool = ImageGenerateTool::new(
            "https://api.example.com".to_string(),
            "test-key".to_string(),
            "flux-1".to_string(),
        );
        let ctx = JobContext::default();
        let long_prompt = "x".repeat(4001);
        let result = tool
            .execute(serde_json::json!({"prompt": long_prompt}), &ctx)
            .await;
        assert!(result.is_err());
    }

    #[test]
    fn infer_generated_image_media_type_detects_jpeg() {
        let jpeg_b64 = STANDARD.encode([0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10]);
        assert_eq!(infer_generated_image_media_type(&jpeg_b64), "image/jpeg");
    }

    #[test]
    fn infer_generated_image_media_type_detects_png() {
        let png_b64 = STANDARD.encode([0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
        assert_eq!(infer_generated_image_media_type(&png_b64), "image/png");
    }

    #[test]
    fn infer_generated_image_media_type_detects_gif() {
        let gif_b64 = STANDARD.encode(b"GIF89a");
        assert_eq!(infer_generated_image_media_type(&gif_b64), "image/gif");
    }

    #[test]
    fn infer_generated_image_media_type_detects_webp() {
        let mut riff = b"RIFF".to_vec();
        riff.extend_from_slice(&[0x00; 4]);
        riff.extend_from_slice(b"WEBP");
        let webp_b64 = STANDARD.encode(&riff);
        assert_eq!(infer_generated_image_media_type(&webp_b64), "image/webp");
    }

    #[test]
    fn infer_generated_image_media_type_detects_short_unaligned_jpeg_prefix() {
        let jpeg_b64 = STANDARD.encode([0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10]);
        let short_unaligned = &jpeg_b64[..jpeg_b64.len() - 1];
        assert_eq!(short_unaligned.len() % 4, 3);
        assert_eq!(
            infer_generated_image_media_type(short_unaligned),
            "image/jpeg"
        );
    }

    #[test]
    fn infer_generated_image_media_type_defaults_for_invalid_base64() {
        assert_eq!(
            infer_generated_image_media_type("!!!not-base64"),
            "image/png"
        );
    }

    #[test]
    fn infer_generated_image_media_type_defaults_for_empty_input() {
        assert_eq!(infer_generated_image_media_type(""), "image/png");
    }

    #[test]
    fn endpoint_url_does_not_duplicate_v1() {
        let tool = ImageGenerateTool::new(
            "https://api.example.com/v1".to_string(),
            "test-key".to_string(),
            "flux-1".to_string(),
        );
        assert_eq!(
            tool.endpoint_url("/images/generations"),
            "https://api.example.com/v1/images/generations"
        );
    }

    #[test]
    fn endpoint_url_adds_v1_when_missing() {
        let tool = ImageGenerateTool::new(
            "https://api.example.com".to_string(),
            "test-key".to_string(),
            "flux-1".to_string(),
        );
        assert_eq!(
            tool.endpoint_url("/images/generations"),
            "https://api.example.com/v1/images/generations"
        );
    }

    #[test]
    fn endpoint_url_preserves_existing_version_segment() {
        let tool = ImageGenerateTool::new(
            "https://api.example.com/v2".to_string(),
            "test-key".to_string(),
            "flux-1".to_string(),
        );
        assert_eq!(
            tool.endpoint_url("/images/generations"),
            "https://api.example.com/v2/images/generations"
        );
    }

    #[test]
    fn endpoint_url_preserves_version_like_suffixes() {
        let tool = ImageGenerateTool::new(
            "https://api.example.com/v1beta".to_string(),
            "test-key".to_string(),
            "flux-1".to_string(),
        );
        assert_eq!(
            tool.endpoint_url("/images/generations"),
            "https://api.example.com/v1beta/images/generations"
        );
    }
}
