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
                    "type": "object",
                    "description": "The JSON data to operate on (string for parse, object otherwise)"
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
                let json_str = serde_json::to_string_pretty(data).map_err(|e| {
                    ToolError::ExecutionFailed(format!("failed to stringify: {}", e))
                })?;

                serde_json::Value::String(json_str)
            }
            "query" => {
                let path = params.get("path").and_then(|v| v.as_str()).ok_or_else(|| {
                    ToolError::InvalidParameters("missing 'path' parameter for query".to_string())
                })?;

                query_json(data, path)?
            }
            "validate" => {
                let is_valid = if let Some(s) = data.as_str() {
                    serde_json::from_str::<serde_json::Value>(s).is_ok()
                } else {
                    true // Already a valid JSON value
                };

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
}
