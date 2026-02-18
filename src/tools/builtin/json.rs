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
        "Parse, query, and transform JSON data. Supports JSONPath-like queries."
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
                    "type": "string",
                    "description": "JSON input string. For query/stringify/validate, pass serialized JSON."
                },
                "path": {
                    "type": "string",
                    "description": "JSONPath-like path for query operation (e.g., 'foo.bar[0].baz')"
                }
            },
            "required": ["operation", "data"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let operation = require_str(&params, "operation")?;

        let data = require_param(&params, "data")?;

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
                let value = parse_json_input(data)?;
                let json_str = serde_json::to_string_pretty(&value).map_err(|e| {
                    ToolError::ExecutionFailed(format!("failed to stringify: {}", e))
                })?;

                serde_json::Value::String(json_str)
            }
            "query" => {
                let path = params.get("path").and_then(|v| v.as_str()).ok_or_else(|| {
                    ToolError::InvalidParameters("missing 'path' parameter for query".to_string())
                })?;

                let value = parse_json_input(data)?;
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
    fn test_json_tool_schema_data_has_type() {
        let tool = JsonTool;
        let schema = tool.parameters_schema();
        assert_eq!(schema["properties"]["data"]["type"], "string");
    }

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
}
