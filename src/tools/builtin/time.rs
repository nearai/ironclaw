//! Time utility tool.

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::context::JobContext;
use crate::tools::tool::{Tool, ToolError, ToolOutput, require_str};

/// Tool for getting current time and date operations.
pub struct TimeTool;

#[async_trait]
impl Tool for TimeTool {
    fn name(&self) -> &str {
        "time"
    }

    fn description(&self) -> &str {
        "Get current time, convert timezones, or calculate time differences."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "operation": {
                    "type": "string",
                    "enum": ["now", "parse", "format", "diff"],
                    "description": "The time operation to perform"
                },
                "timestamp": {
                    "type": "string",
                    "description": "ISO 8601 timestamp (for parse/format/diff operations)"
                },
                "format": {
                    "type": "string",
                    "description": "Output format string (for format operation)"
                },
                "timestamp2": {
                    "type": "string",
                    "description": "Second timestamp (for diff operation)"
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

        let result = match operation {
            "now" => {
                let now = Utc::now();
                let mut result = serde_json::json!({
                    "iso": now.to_rfc3339(),
                    "unix": now.timestamp(),
                    "unix_millis": now.timestamp_millis()
                });
                // Include local time in the user's timezone if set
                if let Some(tz) = crate::timezone::parse_timezone(&ctx.user_timezone) {
                    let local = now.with_timezone(&tz);
                    result["local_iso"] = serde_json::Value::String(local.to_rfc3339());
                    result["timezone"] = serde_json::Value::String(tz.name().to_string());
                }
                result
            }
            "parse" => {
                let timestamp = require_str(&params, "timestamp")?;

                let dt: DateTime<Utc> = timestamp.parse().map_err(|e| {
                    ToolError::InvalidParameters(format!("invalid timestamp: {}", e))
                })?;

                serde_json::json!({
                    "iso": dt.to_rfc3339(),
                    "unix": dt.timestamp(),
                    "unix_millis": dt.timestamp_millis()
                })
            }
            "diff" => {
                let ts1 = require_str(&params, "timestamp")?;

                let ts2 = require_str(&params, "timestamp2")?;

                let dt1: DateTime<Utc> = ts1.parse().map_err(|e| {
                    ToolError::InvalidParameters(format!("invalid timestamp: {}", e))
                })?;
                let dt2: DateTime<Utc> = ts2.parse().map_err(|e| {
                    ToolError::InvalidParameters(format!("invalid timestamp2: {}", e))
                })?;

                let diff = dt2.signed_duration_since(dt1);

                serde_json::json!({
                    "seconds": diff.num_seconds(),
                    "minutes": diff.num_minutes(),
                    "hours": diff.num_hours(),
                    "days": diff.num_days()
                })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_now_includes_local_time_when_timezone_set() {
        let tool = TimeTool;
        let mut ctx = JobContext::with_user("test", "chat", "test");
        ctx.user_timezone = "America/New_York".to_string();

        let output = tool
            .execute(serde_json::json!({"operation": "now"}), &ctx)
            .await
            .expect("execute");
        assert!(
            output.result.get("local_iso").is_some(),
            "should have local_iso"
        );
        assert_eq!(
            output.result["timezone"].as_str(),
            Some("America/New_York"),
            "should report timezone"
        );
    }

    #[tokio::test]
    async fn test_now_includes_utc_timezone_by_default() {
        let tool = TimeTool;
        let ctx = JobContext::with_user("test", "chat", "test");
        // Default user_timezone is "UTC" which is a valid IANA timezone
        let output = tool
            .execute(serde_json::json!({"operation": "now"}), &ctx)
            .await
            .expect("execute");
        assert!(output.result.get("iso").is_some(), "should have iso");
        assert_eq!(output.result["timezone"].as_str(), Some("UTC"));
    }
}
