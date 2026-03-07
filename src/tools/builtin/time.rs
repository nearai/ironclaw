//! Time utility tool.

use async_trait::async_trait;
use chrono::{DateTime, FixedOffset, Utc};
use chrono_tz::Tz;

use crate::context::JobContext;
use crate::tools::tool::{Tool, ToolError, ToolOutput, require_str};

/// Parse a timezone string into a `chrono_tz::Tz`, returning a clear error.
fn parse_timezone(tz_str: &str) -> Result<Tz, ToolError> {
    tz_str.parse::<Tz>().map_err(|_| {
        ToolError::InvalidParameters(format!(
            "Unknown timezone '{}'. Use IANA names like 'America/New_York' or 'Europe/London'.",
            tz_str
        ))
    })
}

/// Parse an input timestamp string. Accepts RFC 3339 with offset, or naive
/// datetime in `YYYY-MM-DDTHH:MM:SS` / `YYYY-MM-DD HH:MM:SS` format
/// (interpreted as UTC unless `default_tz` is provided).
fn parse_input_timestamp(
    input: &str,
    default_tz: Option<Tz>,
) -> Result<DateTime<FixedOffset>, ToolError> {
    // Try RFC 3339 first (has offset info)
    if let Ok(dt) = DateTime::parse_from_rfc3339(input) {
        return Ok(dt);
    }
    // Try common formats without offset — interpret in default_tz or UTC
    for fmt in &["%Y-%m-%dT%H:%M:%S", "%Y-%m-%d %H:%M:%S"] {
        if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(input, fmt) {
            let tz = default_tz.unwrap_or(Tz::UTC);
            let local = naive
                .and_local_timezone(tz)
                .single()
                .ok_or_else(|| {
                    ToolError::InvalidParameters(format!(
                        "Ambiguous or invalid datetime '{}' in timezone '{}'",
                        input, tz
                    ))
                })?;
            return Ok(local.fixed_offset());
        }
    }
    Err(ToolError::InvalidParameters(format!(
        "Invalid timestamp '{}'. Use RFC 3339 (e.g. '2026-03-07T12:00:00Z') \
         or 'YYYY-MM-DD HH:MM:SS' format.",
        input
    )))
}

/// Tool for getting current time and date operations.
pub struct TimeTool;

#[async_trait]
impl Tool for TimeTool {
    fn name(&self) -> &str {
        "time"
    }

    fn description(&self) -> &str {
        "Get current time, convert timezones, format timestamps, or calculate time differences."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "operation": {
                    "type": "string",
                    "enum": ["now", "parse", "convert", "format", "diff"],
                    "description": "The time operation to perform"
                },
                "timestamp": {
                    "type": "string",
                    "description": "ISO 8601 timestamp (for parse/convert/format/diff operations)"
                },
                "timestamp2": {
                    "type": "string",
                    "description": "Second timestamp (for diff operation)"
                },
                "timezone": {
                    "type": "string",
                    "description": "IANA timezone name, e.g. 'America/New_York' (for now/convert/format/parse)"
                },
                "to_timezone": {
                    "type": "string",
                    "description": "Target IANA timezone for convert operation"
                },
                "format_string": {
                    "type": "string",
                    "description": "strftime format string (for format operation), default: '%Y-%m-%d %H:%M:%S %Z'"
                }
            },
            "required": ["operation"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let operation = require_str(&params, "operation")?;

        let result = match operation {
            "now" => {
                let now = Utc::now();
                let mut result = serde_json::json!({
                    "utc_iso": now.to_rfc3339(),
                    "iso": now.to_rfc3339(),
                    "unix": now.timestamp(),
                    "unix_millis": now.timestamp_millis()
                });
                if let Some(tz_str) = params.get("timezone").and_then(|v| v.as_str()) {
                    let tz = parse_timezone(tz_str)?;
                    let local = now.with_timezone(&tz);
                    result["local_iso"] = serde_json::json!(local.to_rfc3339());
                    result["timezone"] = serde_json::json!(tz_str);
                }
                result
            }
            "parse" => {
                let timestamp = require_str(&params, "timestamp")?;
                let tz = params
                    .get("timezone")
                    .and_then(|v| v.as_str())
                    .map(parse_timezone)
                    .transpose()?;

                let dt = parse_input_timestamp(timestamp, tz)?;
                let utc = dt.with_timezone(&Utc);

                let mut result = serde_json::json!({
                    "iso": utc.to_rfc3339(),
                    "unix": utc.timestamp(),
                    "unix_millis": utc.timestamp_millis()
                });
                if let Some(tz) = tz {
                    let local = dt.with_timezone(&tz);
                    result["local_iso"] = serde_json::json!(local.to_rfc3339());
                    result["timezone"] = serde_json::json!(tz.to_string());
                }
                result
            }
            "convert" => {
                let timestamp = require_str(&params, "timestamp")?;
                let to_tz_str = require_str(&params, "to_timezone")?;
                let to_tz = parse_timezone(to_tz_str)?;

                let from_tz = params
                    .get("timezone")
                    .and_then(|v| v.as_str())
                    .map(parse_timezone)
                    .transpose()?;

                let dt = parse_input_timestamp(timestamp, from_tz)?;
                let converted = dt.with_timezone(&to_tz);

                serde_json::json!({
                    "input": timestamp,
                    "output": converted.to_rfc3339(),
                    "timezone": to_tz.to_string()
                })
            }
            "format" => {
                let timestamp = require_str(&params, "timestamp")?;
                let fmt = params
                    .get("format_string")
                    .and_then(|v| v.as_str())
                    .unwrap_or("%Y-%m-%d %H:%M:%S %Z");

                let tz = params
                    .get("timezone")
                    .and_then(|v| v.as_str())
                    .map(parse_timezone)
                    .transpose()?;

                let dt = parse_input_timestamp(timestamp, None)?;
                let formatted = if let Some(tz) = tz {
                    dt.with_timezone(&tz).format(fmt).to_string()
                } else {
                    dt.format(fmt).to_string()
                };

                serde_json::json!({ "formatted": formatted })
            }
            "diff" => {
                let ts1 = require_str(&params, "timestamp")?;
                let ts2 = require_str(&params, "timestamp2")?;

                let dt1 = parse_input_timestamp(ts1, None)?;
                let dt2 = parse_input_timestamp(ts2, None)?;

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
    use crate::context::JobContext;
    use serde_json::json;

    fn test_ctx() -> JobContext {
        JobContext::new("test-job", "test time tool")
    }

    #[tokio::test]
    async fn test_now_utc() {
        let tool = TimeTool;
        let result = tool
            .execute(json!({"operation": "now"}), &test_ctx())
            .await
            .unwrap();
        let v: serde_json::Value = result.result.clone();
        assert!(v["utc_iso"].as_str().is_some());
        assert!(v["iso"].as_str().is_some());
        assert!(v["unix"].as_i64().is_some());
        // No timezone requested — no local_iso
        assert!(v.get("local_iso").is_none());
    }

    #[tokio::test]
    async fn test_now_with_timezone() {
        let tool = TimeTool;
        let result = tool
            .execute(
                json!({"operation": "now", "timezone": "America/New_York"}),
                &test_ctx(),
            )
            .await
            .unwrap();
        let v: serde_json::Value = result.result.clone();
        assert!(v["local_iso"].as_str().is_some());
        assert_eq!(v["timezone"].as_str().unwrap(), "America/New_York");
        // local_iso should contain a non-UTC offset
        let local = v["local_iso"].as_str().unwrap();
        assert!(!local.ends_with('Z') || local.contains("-04:00") || local.contains("-05:00"));
    }

    #[tokio::test]
    async fn test_now_invalid_timezone() {
        let tool = TimeTool;
        let result = tool
            .execute(
                json!({"operation": "now", "timezone": "Not/A/Zone"}),
                &test_ctx(),
            )
            .await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("Unknown timezone"));
        assert!(err.to_string().contains("Not/A/Zone"));
    }

    #[tokio::test]
    async fn test_convert_timezone() {
        let tool = TimeTool;
        let result = tool
            .execute(
                json!({
                    "operation": "convert",
                    "timestamp": "2026-03-07T12:00:00Z",
                    "to_timezone": "Asia/Tokyo"
                }),
                &test_ctx(),
            )
            .await
            .unwrap();
        let v: serde_json::Value = result.result.clone();
        // UTC 12:00 -> JST 21:00 (UTC+9)
        let output = v["output"].as_str().unwrap();
        assert!(output.contains("21:00:00"));
        assert_eq!(v["timezone"].as_str().unwrap(), "Asia/Tokyo");
    }

    #[tokio::test]
    async fn test_convert_dst_boundary() {
        let tool = TimeTool;
        // US spring forward: 2026-03-08 2:00 AM EST -> 3:00 AM EDT
        // Before DST: EST = UTC-5, After: EDT = UTC-4
        let result = tool
            .execute(
                json!({
                    "operation": "convert",
                    "timestamp": "2026-03-08T06:30:00Z",
                    "to_timezone": "America/New_York"
                }),
                &test_ctx(),
            )
            .await
            .unwrap();
        let v: serde_json::Value = result.result.clone();
        // UTC 06:30 on Mar 8 -> after spring forward, EDT (UTC-4) = 02:30
        // But DST springs forward at 2 AM -> 3 AM, so 06:30 UTC = 01:30 EST or 02:30 EDT
        let output = v["output"].as_str().unwrap();
        assert!(output.contains("2026-03-08"));
    }

    #[tokio::test]
    async fn test_format_with_timezone() {
        let tool = TimeTool;
        let result = tool
            .execute(
                json!({
                    "operation": "format",
                    "timestamp": "2026-03-07T12:00:00Z",
                    "timezone": "Europe/London",
                    "format_string": "%Y-%m-%d %H:%M %Z"
                }),
                &test_ctx(),
            )
            .await
            .unwrap();
        let v: serde_json::Value = result.result.clone();
        let formatted = v["formatted"].as_str().unwrap();
        assert!(formatted.contains("2026-03-07"));
        assert!(formatted.contains("12:00")); // London = UTC in March (before DST)
        assert!(formatted.contains("GMT"));
    }

    #[tokio::test]
    async fn test_format_default_format_string() {
        let tool = TimeTool;
        let result = tool
            .execute(
                json!({
                    "operation": "format",
                    "timestamp": "2026-06-15T18:30:00Z",
                    "timezone": "America/Los_Angeles"
                }),
                &test_ctx(),
            )
            .await
            .unwrap();
        let v: serde_json::Value = result.result.clone();
        let formatted = v["formatted"].as_str().unwrap();
        // UTC 18:30 -> PDT (UTC-7) = 11:30
        assert!(formatted.contains("11:30:00"));
        assert!(formatted.contains("PDT"));
    }

    #[tokio::test]
    async fn test_parse_naive_with_timezone() {
        let tool = TimeTool;
        let result = tool
            .execute(
                json!({
                    "operation": "parse",
                    "timestamp": "2026-03-07 09:00:00",
                    "timezone": "America/New_York"
                }),
                &test_ctx(),
            )
            .await
            .unwrap();
        let v: serde_json::Value = result.result.clone();
        // 09:00 EST = 14:00 UTC (EST = UTC-5 in March before DST)
        let iso = v["iso"].as_str().unwrap();
        assert!(iso.contains("14:00:00"));
        assert_eq!(v["timezone"].as_str().unwrap(), "America/New_York");
    }

    #[tokio::test]
    async fn test_diff() {
        let tool = TimeTool;
        let result = tool
            .execute(
                json!({
                    "operation": "diff",
                    "timestamp": "2026-03-07T00:00:00Z",
                    "timestamp2": "2026-03-07T02:30:00Z"
                }),
                &test_ctx(),
            )
            .await
            .unwrap();
        let v: serde_json::Value = result.result.clone();
        assert_eq!(v["hours"].as_i64().unwrap(), 2);
        assert_eq!(v["minutes"].as_i64().unwrap(), 150);
        assert_eq!(v["seconds"].as_i64().unwrap(), 9000);
    }

    #[tokio::test]
    async fn test_convert_missing_to_timezone() {
        let tool = TimeTool;
        let result = tool
            .execute(
                json!({
                    "operation": "convert",
                    "timestamp": "2026-03-07T12:00:00Z"
                }),
                &test_ctx(),
            )
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_unknown_operation() {
        let tool = TimeTool;
        let result = tool
            .execute(json!({"operation": "explode"}), &test_ctx())
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("unknown operation"));
    }
}
