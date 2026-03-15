//! JSON manipulation tool.

use async_trait::async_trait;

use crate::context::JobContext;
use crate::tools::tool::{Tool, ToolError, ToolOutput, require_param, require_str};

/// Tool for JSON manipulation (parse, query, transform).
pub struct JsonTool;

#[async_trait]
impl Tool for JsonTool {
    fn name(&self) -> &str {
        "json"
    }

    fn description(&self) -> &str {
        "Parse, query, and transform JSON data. Supports JSONPath-like queries. \
         Use `source_tool_call_id` to reference the full output of a previous tool call \
         (avoids truncation issues with large responses)."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "operation": {
                    "type": "string",
                    "enum": ["parse", "query", "stringify", "validate"],
                    "description": "The JSON operation to perform"
                },
                "data": {
                    "description": "JSON input data. Pass a string for parse, or any JSON value otherwise. Not required when source_tool_call_id is provided."
                },
                "source_tool_call_id": {
                    "type": "string",
                    "description": "Reference a previous tool call's full output by its ID (e.g., 'call_abc123'). Use this instead of data when the previous tool output was large and may have been truncated."
                },
                "path": {
                    "type": "string",
                    "description": "JSONPath-like path for query operation (e.g., 'foo.bar[0].baz')"
                }
            },
            "required": ["operation"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let operation = require_str(&params, "operation")?;

        // Resolve data: from stash (via source_tool_call_id) or from params
        let data_value =
            if let Some(ref_id) = params.get("source_tool_call_id").and_then(|v| v.as_str()) {
                let stash = ctx.tool_output_stash.read().await;
                let full_output = stash.get(ref_id).ok_or_else(|| {
                    ToolError::InvalidParameters(format!(
                        "no tool output found for call ID '{}'. Available IDs: {:?}",
                        ref_id,
                        stash.keys().collect::<Vec<_>>()
                    ))
                })?;
                // Parse the stashed output as JSON, or wrap as string
                serde_json::from_str::<serde_json::Value>(full_output)
                    .unwrap_or_else(|_| serde_json::Value::String(full_output.clone()))
            } else {
                require_param(&params, "data")?.clone()
            };
        let data = &data_value;

        let result = match operation {
            "parse" => {
                let json_str = data.as_str().ok_or_else(|| {
                    ToolError::InvalidParameters(
                        "'data' must be a string for parse operation".to_string(),
                    )
                })?;

                let parsed: serde_json::Value = serde_json::from_str(json_str)
                    .map_err(|e| ToolError::InvalidParameters(format!("invalid JSON: {}", e)))?;

                parsed
            }
            "stringify" => {
                let value = if data.is_string() {
                    parse_json_input(data)?
                } else {
                    data.clone()
                };
                let json_str = serde_json::to_string_pretty(&value).map_err(|e| {
                    ToolError::ExecutionFailed(format!("failed to stringify: {}", e))
                })?;

                serde_json::Value::String(json_str)
            }
            "query" => {
                let path = params.get("path").and_then(|v| v.as_str()).ok_or_else(|| {
                    ToolError::InvalidParameters("missing 'path' parameter for query".to_string())
                })?;

                let value = if data.is_string() {
                    parse_json_input(data)?
                } else {
                    data.clone()
                };
                query_json(&value, path)?
            }
            "validate" => {
                let is_valid = data
                    .as_str()
                    .map(|s| serde_json::from_str::<serde_json::Value>(s).is_ok())
                    .unwrap_or(false);

                serde_json::json!({ "valid": is_valid })
            }
            _ => {
                return Err(ToolError::InvalidParameters(format!(
                    "unknown operation: {}",
                    operation
                )));
            }
        };

        Ok(ToolOutput::success(result, start.elapsed()))
    }

    fn requires_sanitization(&self) -> bool {
        false // Internal tool, no external data
    }
}

fn parse_json_input(data: &serde_json::Value) -> Result<serde_json::Value, ToolError> {
    let json_str = data
        .as_str()
        .ok_or_else(|| ToolError::InvalidParameters("'data' must be a JSON string".to_string()))?;
    serde_json::from_str(json_str)
        .map_err(|e| ToolError::InvalidParameters(format!("invalid JSON input: {}", e)))
}

/// Simple JSONPath-like query implementation.
fn query_json(data: &serde_json::Value, path: &str) -> Result<serde_json::Value, ToolError> {
    let mut current = data;

    for segment in path.split('.') {
        if segment.is_empty() {
            continue;
        }

        // Check for array indexing: field[0]
        if let Some((field, index_str)) = segment.split_once('[') {
            // First navigate to the field
            if !field.is_empty() {
                current = current.get(field).ok_or_else(|| {
                    ToolError::ExecutionFailed(format!("field not found: {}", field))
                })?;
            }

            // Then get the array index
            let index_str = index_str.trim_end_matches(']');
            let index: usize = index_str.parse().map_err(|_| {
                ToolError::InvalidParameters(format!("invalid array index: {}", index_str))
            })?;

            current = current.get(index).ok_or_else(|| {
                ToolError::ExecutionFailed(format!("array index out of bounds: {}", index))
            })?;
        } else {
            // Simple field access
            current = current.get(segment).ok_or_else(|| {
                ToolError::ExecutionFailed(format!("field not found: {}", segment))
            })?;
        }
    }

    Ok(current.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_query_json() {
        let data = serde_json::json!({
            "foo": {
                "bar": [1, 2, 3],
                "baz": "hello"
            }
        });

        assert_eq!(
            query_json(&data, "foo.baz").unwrap(),
            serde_json::json!("hello")
        );
        assert_eq!(
            query_json(&data, "foo.bar[0]").unwrap(),
            serde_json::json!(1)
        );
        assert_eq!(
            query_json(&data, "foo.bar[2]").unwrap(),
            serde_json::json!(3)
        );
    }

    #[test]
    fn test_parse_json_input_accepts_valid_json_string() {
        let input = serde_json::json!("{\"ok\":true}");
        let parsed = parse_json_input(&input).unwrap();
        assert_eq!(parsed, serde_json::json!({"ok": true}));
    }

    #[test]
    fn test_parse_json_input_rejects_invalid_json_string() {
        let input = serde_json::json!("{not valid json}");
        let err = parse_json_input(&input).unwrap_err();
        assert!(err.to_string().contains("invalid JSON input"));
    }

    #[tokio::test]
    async fn test_query_with_object_data_from_stash() {
        use crate::context::JobContext;

        let ctx = JobContext::with_user("test", "chat", "test-session");

        // Simulate stashed output: the http tool stores serialized JSON
        // containing {"status": 200, "body": {"leagues": [{"name": "MLB"}]}}
        let stashed = r#"{"status": 200, "body": {"leagues": [{"name": "MLB"}]}}"#;
        ctx.tool_output_stash
            .write()
            .await
            .insert("call_http_01".to_string(), stashed.to_string());

        let tool = JsonTool;
        let params = serde_json::json!({
            "operation": "query",
            "source_tool_call_id": "call_http_01",
            "path": "body.leagues[0].name"
        });

        let result = tool.execute(params, &ctx).await.unwrap();
        assert_eq!(result.result, serde_json::json!("MLB"));
    }

    #[tokio::test]
    async fn test_stringify_with_object_data_from_stash() {
        use crate::context::JobContext;

        let ctx = JobContext::with_user("test", "chat", "test-session");

        let stashed = r#"{"key": "value"}"#;
        ctx.tool_output_stash
            .write()
            .await
            .insert("call_01".to_string(), stashed.to_string());

        let tool = JsonTool;
        let params = serde_json::json!({
            "operation": "stringify",
            "source_tool_call_id": "call_01"
        });

        let result = tool.execute(params, &ctx).await.unwrap();
        let stringified = result.result.as_str().unwrap();
        assert!(stringified.contains("\"key\": \"value\""));
    }

    #[test]
    fn test_json_tool_schema_data_is_freeform() {
        let schema = JsonTool.parameters_schema();
        let data = schema
            .get("properties")
            .and_then(|p| p.get("data"))
            .expect("data schema missing");

        // Data is intentionally freeform (no "type" constraint) for OpenAI
        // compatibility. OpenAI rejects union types containing "array" unless
        // "items" is also specified.
        assert!(
            data.get("type").is_none(),
            "data schema should not have a 'type' to be freeform for OpenAI compatibility"
        );
    }
}
