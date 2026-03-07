//! Image analysis tool for vision-capable LLMs.
//!
//! Reads images from the workspace and prepares them for vision analysis.
//! The LLM can then analyze the image content based on the user's query.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::context::JobContext;
use crate::tools::tool::{ApprovalRequirement, Tool, ToolError, ToolOutput};
use crate::workspace::Workspace;

/// Tool for analyzing images using a vision-capable LLM.
pub struct ImageAnalyzeTool {
    workspace: Arc<Workspace>,
}

impl ImageAnalyzeTool {
    /// Create a new image analysis tool.
    pub fn new(workspace: Arc<Workspace>) -> Self {
        Self { workspace }
    }

    /// Infer media type from file extension.
    fn infer_media_type(path: &str) -> &'static str {
        let lower_path = path.to_lowercase();
        if lower_path.ends_with(".png") || lower_path.ends_with(".b64") {
            "image/png"
        } else if lower_path.ends_with(".jpg") || lower_path.ends_with(".jpeg") {
            "image/jpeg"
        } else if lower_path.ends_with(".gif") {
            "image/gif"
        } else if lower_path.ends_with(".webp") {
            "image/webp"
        } else {
            "image/png" // Default to PNG
        }
    }
}

#[async_trait]
impl Tool for ImageAnalyzeTool {
    fn name(&self) -> &str {
        "image_analyze"
    }

    fn description(&self) -> &str {
        "Analyze an image using the LLM's vision capabilities. Provide the workspace path to the image and a question or prompt about what you want to know about the image."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Workspace path to the image (e.g., 'images/generated/abc123.b64')"
                },
                "query": {
                    "type": "string",
                    "description": "What do you want to know about the image? (e.g., 'describe the objects in this image', 'is there text in this image?')"
                }
            },
            "required": ["path", "query"]
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

        let query = params
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                ToolError::InvalidParameters("Missing or invalid 'query' parameter".to_string())
            })?
            .to_string();

        if query.is_empty() {
            return Err(ToolError::InvalidParameters(
                "Query cannot be empty".to_string(),
            ));
        }

        // Read image from workspace
        let doc = self.workspace.read(&path).await.map_err(|e| {
            ToolError::ExecutionFailed(format!("Failed to read image from workspace: {}", e))
        })?;

        // Infer media type from path
        let media_type = Self::infer_media_type(&path).to_string();

        // Return the image data and query so the agent can include the image in its vision analysis
        Ok(ToolOutput::success(
            json!({
                "type": "image_analysis_ready",
                "path": path,
                "query": query,
                "data": doc.content,
                "media_type": media_type,
                "instruction": format!("The user wants you to analyze this image with the following query: {}", query)
            }),
            start.elapsed(),
        ))
    }

    fn requires_approval(&self, _params: &Value) -> ApprovalRequirement {
        // Image analysis is read-only, no approval needed
        ApprovalRequirement::Never
    }

    fn sensitive_params(&self) -> &[&str] {
        &[]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_infer_media_type_png() {
        assert_eq!(
            ImageAnalyzeTool::infer_media_type("images/test.png"),
            "image/png"
        );
        assert_eq!(
            ImageAnalyzeTool::infer_media_type("images/test.b64"),
            "image/png"
        );
    }

    #[test]
    fn test_infer_media_type_jpeg() {
        assert_eq!(
            ImageAnalyzeTool::infer_media_type("images/test.jpg"),
            "image/jpeg"
        );
        assert_eq!(
            ImageAnalyzeTool::infer_media_type("images/test.jpeg"),
            "image/jpeg"
        );
    }

    #[test]
    fn test_infer_media_type_gif() {
        assert_eq!(
            ImageAnalyzeTool::infer_media_type("images/test.gif"),
            "image/gif"
        );
    }

    #[test]
    fn test_infer_media_type_webp() {
        assert_eq!(
            ImageAnalyzeTool::infer_media_type("images/test.webp"),
            "image/webp"
        );
    }

    #[test]
    fn test_infer_media_type_default() {
        assert_eq!(
            ImageAnalyzeTool::infer_media_type("images/test.unknown"),
            "image/png"
        );
    }

    #[test]
    fn test_parameters_schema_required_fields() {
        let schema = json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Workspace path to the image (e.g., 'images/generated/abc123.b64')"
                },
                "query": {
                    "type": "string",
                    "description": "What do you want to know about the image? (e.g., 'describe the objects in this image', 'is there text in this image?')"
                }
            },
            "required": ["path", "query"]
        });

        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["path"].is_object());
        assert!(schema["properties"]["query"].is_object());
        assert_eq!(schema["required"], json!(["path", "query"]));
    }

    #[test]
    fn test_infer_media_type_uppercase_extension_defaults() {
        // Uppercase extensions are now case-insensitively matched
        assert_eq!(
            ImageAnalyzeTool::infer_media_type("images/test.PNG"),
            "image/png"
        );
        assert_eq!(
            ImageAnalyzeTool::infer_media_type("images/test.JPG"),
            "image/jpeg"
        );
    }

    #[test]
    fn test_infer_media_type_nested_path() {
        assert_eq!(
            ImageAnalyzeTool::infer_media_type("images/generated/2024-03-06/deep/nested/image.png"),
            "image/png"
        );
    }

    #[test]
    fn test_infer_media_type_multiple_dots() {
        assert_eq!(
            ImageAnalyzeTool::infer_media_type("images/my.test.image.png"),
            "image/png"
        );
        assert_eq!(
            ImageAnalyzeTool::infer_media_type("images/file.backup.jpg"),
            "image/jpeg"
        );
    }
}
