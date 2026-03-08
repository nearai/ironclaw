//! Time utility tool.

use async_trait::async_trait;
use chrono::{DateTime, LocalResult, NaiveDate, NaiveDateTime, TimeZone, Utc};
use chrono_tz::Tz;

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
        "Get current time, parse or format timestamps, convert timezones, or calculate time differences."
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
                "input": {
                    "type": "string",
                    "description": "Input timestamp. Accepts RFC 3339, or a naive timestamp when timezone/from_timezone is provided."
                },
                "timestamp": {
                    "type": "string",
                    "description": "Alias for input (kept for backward compatibility)."
                },
                "timezone": {
                    "type": "string",
                    "description": "IANA timezone name (e.g. 'America/New_York'). Used by now/format, and can also interpret naive timestamps."
                },
                "from_timezone": {
                    "type": "string",
                    "description": "Source IANA timezone for naive input timestamps during convert/format/diff."
                },
                "to_timezone": {
                    "type": "string",
                    "description": "Target IANA timezone for convert."
                },
                "format": {
                    "type": "string",
                    "description": "strftime format string for format (kept for backward compatibility)."
                },
                "format_string": {
                    "type": "string",
                    "description": "strftime format string for format."
                },
                "timestamp2": {
                    "type": "string",
                    "description": "Second timestamp for diff."
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
            "now" => execute_now(&params, ctx)?,
            "parse" => execute_parse(&params, ctx)?,
            "convert" => execute_convert(&params, ctx)?,
            "format" => execute_format(&params, ctx)?,
            "diff" => execute_diff(&params, ctx)?,
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

fn execute_now(
    params: &serde_json::Value,
    ctx: &JobContext,
) -> Result<serde_json::Value, ToolError> {
    let now = Utc::now();
    let mut result = serde_json::json!({
        "iso": now.to_rfc3339(),
        "utc_iso": now.to_rfc3339(),
        "unix": now.timestamp(),
        "unix_millis": now.timestamp_millis()
    });

    if let Some((tz, tz_name)) = resolve_timezone_for_output(params, ctx)? {
        let local = now.with_timezone(&tz);
        result["local_iso"] = serde_json::Value::String(local.to_rfc3339());
        result["timezone"] = serde_json::Value::String(tz_name);
    }

    Ok(result)
}

fn execute_parse(
    params: &serde_json::Value,
    ctx: &JobContext,
) -> Result<serde_json::Value, ToolError> {
    let input = require_input(params)?;
    let parse_tz = resolve_parse_timezone(params, ctx)?;
    let dt = parse_timestamp(input, parse_tz.as_ref())?;

    Ok(serde_json::json!({
        "iso": dt.to_rfc3339(),
        "unix": dt.timestamp(),
        "unix_millis": dt.timestamp_millis()
    }))
}

fn execute_convert(
    params: &serde_json::Value,
    ctx: &JobContext,
) -> Result<serde_json::Value, ToolError> {
    let input = require_input(params)?;
    let source_tz = optional_timezone(params, &["from_timezone", "timezone"])?;
    let dt = parse_timestamp(input, source_tz.as_ref())?;

    let target_name = params
        .get("to_timezone")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            ToolError::InvalidParameters("convert operation requires 'to_timezone'".to_string())
        })?;
    let target_tz = parse_timezone(target_name)?;
    let converted = dt.with_timezone(&target_tz);

    let mut result = serde_json::json!({
        "input": input,
        "utc_iso": dt.to_rfc3339(),
        "output": converted.to_rfc3339(),
        "timezone": target_tz.to_string()
    });

    if let Some((ctx_tz, ctx_tz_name)) = context_timezone(ctx)? {
        result["context_timezone"] = serde_json::Value::String(ctx_tz_name);
        result["context_iso"] = serde_json::Value::String(dt.with_timezone(&ctx_tz).to_rfc3339());
    }

    Ok(result)
}

fn execute_format(
    params: &serde_json::Value,
    ctx: &JobContext,
) -> Result<serde_json::Value, ToolError> {
    let input = require_input(params)?;
    let output_tz = resolve_timezone_for_output(params, ctx)?;
    let source_tz = optional_timezone(params, &["from_timezone"])?
        .or_else(|| output_tz.as_ref().map(|(tz, _)| *tz));
    let dt = parse_timestamp(input, source_tz.as_ref())?;
    let format_string = params
        .get("format_string")
        .and_then(|v| v.as_str())
        .or_else(|| params.get("format").and_then(|v| v.as_str()))
        .unwrap_or("%Y-%m-%d %H:%M:%S %Z");

    let mut result = if let Some((tz, tz_name)) = output_tz {
        serde_json::json!({
            "formatted": dt.with_timezone(&tz).format(format_string).to_string(),
            "timezone": tz_name
        })
    } else {
        serde_json::json!({
            "formatted": dt.format(format_string).to_string()
        })
    };

    result["utc_iso"] = serde_json::Value::String(dt.to_rfc3339());
    Ok(result)
}

fn execute_diff(
    params: &serde_json::Value,
    ctx: &JobContext,
) -> Result<serde_json::Value, ToolError> {
    let parse_tz = resolve_parse_timezone(params, ctx)?;
    let ts1 = require_input(params)?;
    let ts2 = params
        .get("timestamp2")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            ToolError::InvalidParameters("diff operation requires 'timestamp2'".to_string())
        })?;

    let dt1 = parse_timestamp(ts1, parse_tz.as_ref())?;
    let dt2 = parse_timestamp(ts2, parse_tz.as_ref())?;
    let diff = dt2.signed_duration_since(dt1);

    Ok(serde_json::json!({
        "seconds": diff.num_seconds(),
        "minutes": diff.num_minutes(),
        "hours": diff.num_hours(),
        "days": diff.num_days()
    }))
}

fn require_input(params: &serde_json::Value) -> Result<&str, ToolError> {
    params
        .get("input")
        .and_then(|v| v.as_str())
        .or_else(|| params.get("timestamp").and_then(|v| v.as_str()))
        .ok_or_else(|| {
            ToolError::InvalidParameters(
                "missing 'input' (or legacy 'timestamp') parameter".to_string(),
            )
        })
}

fn resolve_parse_timezone(
    params: &serde_json::Value,
    ctx: &JobContext,
) -> Result<Option<Tz>, ToolError> {
    if let Some(tz) = optional_timezone(params, &["from_timezone", "timezone"])? {
        return Ok(Some(tz));
    }

    Ok(context_timezone(ctx)?.map(|(tz, _)| tz))
}

fn resolve_timezone_for_output(
    params: &serde_json::Value,
    ctx: &JobContext,
) -> Result<Option<(Tz, String)>, ToolError> {
    if let Some(name) = params.get("timezone").and_then(|v| v.as_str()) {
        let tz = parse_timezone(name)?;
        return Ok(Some((tz, tz.to_string())));
    }

    context_timezone(ctx)
}

/// Resolve the user's timezone from the JobContext.
///
/// Uses `ctx.user_timezone` (set from main's timezone resolution) as the
/// primary source. Falls back to metadata fields for backward compatibility.
fn context_timezone(ctx: &JobContext) -> Result<Option<(Tz, String)>, ToolError> {
    // Primary: use the dedicated user_timezone field from JobContext
    if ctx.user_timezone != "UTC"
        && !ctx.user_timezone.is_empty()
        && let Some(tz) = crate::timezone::parse_timezone(&ctx.user_timezone)
    {
        return Ok(Some((tz, tz.to_string())));
    }

    // Fallback: check metadata for backward compatibility
    let tz_name = ctx
        .metadata
        .get("user_timezone")
        .and_then(|v| v.as_str())
        .or_else(|| ctx.metadata.get("timezone").and_then(|v| v.as_str()));

    match tz_name {
        Some(name) => {
            let tz = parse_timezone(name)?;
            Ok(Some((tz, tz.to_string())))
        }
        None => Ok(None),
    }
}

fn optional_timezone(params: &serde_json::Value, keys: &[&str]) -> Result<Option<Tz>, ToolError> {
    for key in keys {
        if let Some(value) = params.get(*key).and_then(|v| v.as_str()) {
            return parse_timezone(value).map(Some);
        }
    }
    Ok(None)
}

fn parse_timezone(value: &str) -> Result<Tz, ToolError> {
    value.parse::<Tz>().map_err(|_| {
        ToolError::InvalidParameters(format!(
            "Unknown timezone '{}'. Use IANA names like 'America/New_York' or 'Europe/London'.",
            value
        ))
    })
}

fn parse_timestamp(input: &str, fallback_tz: Option<&Tz>) -> Result<DateTime<Utc>, ToolError> {
    if let Ok(dt) = DateTime::parse_from_rfc3339(input) {
        return Ok(dt.with_timezone(&Utc));
    }

    if let Some(naive) = parse_naive_datetime(input) {
        return localize_naive_datetime(naive, fallback_tz, input);
    }

    Err(ToolError::InvalidParameters(format!(
        "invalid timestamp '{}': expected RFC 3339 or a naive timestamp with timezone/from_timezone",
        input
    )))
}

fn parse_naive_datetime(input: &str) -> Option<NaiveDateTime> {
    const DATETIME_FORMATS: &[&str] = &[
        "%Y-%m-%d %H:%M:%S%.f",
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%d %H:%M",
        "%Y-%m-%dT%H:%M",
    ];
    const DATE_FORMATS: &[&str] = &["%Y-%m-%d"];

    for format in DATETIME_FORMATS {
        if let Ok(value) = NaiveDateTime::parse_from_str(input, format) {
            return Some(value);
        }
    }

    for format in DATE_FORMATS {
        if let Ok(date) = NaiveDate::parse_from_str(input, format) {
            return date.and_hms_opt(0, 0, 0);
        }
    }

    None
}

fn localize_naive_datetime(
    naive: NaiveDateTime,
    fallback_tz: Option<&Tz>,
    original_input: &str,
) -> Result<DateTime<Utc>, ToolError> {
    let tz = fallback_tz.ok_or_else(|| {
        ToolError::InvalidParameters(format!(
            "timestamp '{}' has no UTC offset; provide 'timezone' or 'from_timezone'",
            original_input
        ))
    })?;

    match tz.from_local_datetime(&naive) {
        LocalResult::Single(dt) => Ok(dt.with_timezone(&Utc)),
        LocalResult::Ambiguous(_, _) => Err(ToolError::InvalidParameters(format!(
            "timestamp '{}' is ambiguous in timezone '{}'; include an explicit UTC offset instead",
            original_input, tz
        ))),
        LocalResult::None => Err(ToolError::InvalidParameters(format!(
            "timestamp '{}' does not exist in timezone '{}'",
            original_input, tz
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_now_accepts_explicit_timezone() {
        let tool = TimeTool;
        let ctx = JobContext::with_user("test", "chat", "test");

        let output = tool
            .execute(
                serde_json::json!({
                    "operation": "now",
                    "timezone": "America/New_York"
                }),
                &ctx,
            )
            .await
            .expect("execute");

        assert_eq!(output.result["timezone"].as_str(), Some("America/New_York"));
        assert!(
            output.result.get("utc_iso").is_some(),
            "should have utc_iso"
        );
        assert!(
            output.result.get("local_iso").is_some(),
            "should have local_iso"
        );
    }

    #[tokio::test]
    async fn test_now_includes_local_time_when_user_timezone_set() {
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
    async fn test_now_uses_context_metadata_timezone_fallback() {
        let tool = TimeTool;
        let mut ctx = JobContext::with_user("test", "chat", "test");
        ctx.metadata = serde_json::json!({
            "user_timezone": "America/Los_Angeles"
        });

        let output = tool
            .execute(serde_json::json!({"operation": "now"}), &ctx)
            .await
            .expect("execute");

        assert_eq!(
            output.result["timezone"].as_str(),
            Some("America/Los_Angeles")
        );
        assert!(
            output.result.get("local_iso").is_some(),
            "should have local_iso"
        );
    }

    #[tokio::test]
    async fn test_now_returns_utc_by_default() {
        let tool = TimeTool;
        let ctx = JobContext::with_user("test", "chat", "test");
        // Default user_timezone is "UTC" -- context_timezone skips UTC so no
        // local_iso is added, but iso and utc_iso are always present.
        let output = tool
            .execute(serde_json::json!({"operation": "now"}), &ctx)
            .await
            .expect("execute");
        assert!(output.result.get("iso").is_some(), "should have iso");
    }

    #[tokio::test]
    async fn test_convert_across_dst_boundary() {
        let tool = TimeTool;
        let ctx = JobContext::with_user("test", "chat", "test");

        let output = tool
            .execute(
                serde_json::json!({
                    "operation": "convert",
                    "input": "2026-03-08T07:30:00Z",
                    "to_timezone": "America/New_York"
                }),
                &ctx,
            )
            .await
            .expect("execute");

        assert_eq!(output.result["timezone"].as_str(), Some("America/New_York"));
        assert_eq!(
            output.result["output"].as_str(),
            Some("2026-03-08T03:30:00-04:00")
        );
    }

    #[tokio::test]
    async fn test_format_with_timezone() {
        let tool = TimeTool;
        let ctx = JobContext::with_user("test", "chat", "test");

        let output = tool
            .execute(
                serde_json::json!({
                    "operation": "format",
                    "input": "2026-03-08T07:30:00Z",
                    "timezone": "America/New_York",
                    "format_string": "%Y-%m-%d %H:%M:%S %Z"
                }),
                &ctx,
            )
            .await
            .expect("execute");

        assert_eq!(output.result["timezone"].as_str(), Some("America/New_York"));
        assert_eq!(
            output.result["formatted"].as_str(),
            Some("2026-03-08 03:30:00 EDT")
        );
    }

    #[tokio::test]
    async fn test_invalid_timezone_returns_clear_error() {
        let tool = TimeTool;
        let ctx = JobContext::with_user("test", "chat", "test");

        let err = tool
            .execute(
                serde_json::json!({
                    "operation": "now",
                    "timezone": "Mars/Olympus"
                }),
                &ctx,
            )
            .await
            .expect_err("expected invalid timezone error");

        match err {
            ToolError::InvalidParameters(message) => {
                assert!(message.contains("Unknown timezone 'Mars/Olympus'"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn test_parse_naive_timestamp_with_timezone() {
        let dt = parse_timestamp("2026-03-08 03:30:00", Some(&chrono_tz::America::New_York))
            .expect("parse timestamp");

        assert_eq!(dt.to_rfc3339(), "2026-03-08T07:30:00+00:00");
    }
}
