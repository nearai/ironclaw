//! Job completion tool.
//!
//! This tool is only visible in job/ container contexts (registered as a
//! job-only tool). When the LLM calls it, the delegate intercepts the call
//! and terminates the agentic loop. No string-matching heuristics required.

use std::time::Duration;
use std::{fmt, str::FromStr};

use async_trait::async_trait;

use crate::context::JobContext;
use crate::tools::tool::{EngineCompatibility, Tool, ToolError, ToolOutput, require_str};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FinishJobStatus {
    Completed,
    Failed,
}

impl FinishJobStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }
}

impl fmt::Display for FinishJobStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for FinishJobStatus {
    type Err = ToolError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "completed" => Ok(Self::Completed),
            "failed" => Ok(Self::Failed),
            other => Err(ToolError::InvalidParameters(format!(
                "Field 'status' must be one of [\"completed\", \"failed\"], got '{other}'"
            ))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinishJobSignal {
    pub status: FinishJobStatus,
    pub summary: String,
}

const DEFAULT_NO_SUMMARY_COMPLETED: &str = "Job completed successfully with no summary.";
const DEFAULT_NO_SUMMARY_FAILED: &str = "Job failed with no summary.";

fn normalize_finish_job_summary(status: FinishJobStatus, summary: &str) -> String {
    if !summary.trim().is_empty() {
        return summary.to_string();
    }

    match status {
        FinishJobStatus::Completed => DEFAULT_NO_SUMMARY_COMPLETED.to_string(),
        FinishJobStatus::Failed => DEFAULT_NO_SUMMARY_FAILED.to_string(),
    }
}

pub fn parse_finish_job_signal(params: &serde_json::Value) -> Result<FinishJobSignal, ToolError> {
    let status = require_str(params, "status")?.parse()?;
    let summary = normalize_finish_job_summary(status, require_str(params, "summary")?);
    Ok(FinishJobSignal { status, summary })
}

/// Parse a `finish_job` signal from the tool's executed JSON result payload.
///
/// Delegates should prefer this over reparsing the model's original arguments so
/// hook-modified parameters and tool-side normalization remain authoritative.
pub fn parse_finish_job_signal_from_output(output: &str) -> Result<FinishJobSignal, ToolError> {
    let parsed: serde_json::Value = serde_json::from_str(output).map_err(|e| {
        ToolError::ExecutionFailed(format!("finish_job returned non-JSON result payload: {e}"))
    })?;
    parse_finish_job_signal(&parsed)
}

/// Signal job completion or failure.
///
/// The delegate (JobDelegate / ContainerDelegate) intercepts calls to this
/// tool and converts them into `LoopOutcome::Response` or `LoopOutcome::Failure`,
/// breaking the agentic loop. The tool itself is a no-op; the side effects live
/// in the delegate so they can access loop state.
pub struct FinishJobTool;

#[async_trait]
impl Tool for FinishJobTool {
    fn name(&self) -> &str {
        "finish_job"
    }

    fn description(&self) -> &str {
        "Signal that the autonomous job is fully complete or has failed. \
         IMPORTANT: This is the only way to end a job. \
         Do NOT rely on a plain text reply to stop execution. \
         Prefer to call this tool after all other work is done. \
         If you include it in the same batch as other tools, those tools may run first \
         and then the job will be finalized. \
         Emit at most one finish_job call per batch. \
         Use status \"completed\" when all required work is finished, \
         or status \"failed\" when you encounter an unresolvable blocker."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "status": {
                    "type": "string",
                    "enum": ["completed", "failed"],
                    "description": "Whether the job succeeded or failed"
                },
                "summary": {
                    "type": "string",
                    "description": "A concise summary of what was accomplished (completed) or why execution was blocked (failed). This becomes the job's final output."
                }
            },
            "required": ["status", "summary"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let signal = parse_finish_job_signal(&params)?;
        Ok(ToolOutput::success(
            serde_json::json!({
                "status": signal.status.as_str(),
                "summary": signal.summary,
                "message": format!("Job {}: {}", signal.status, signal.summary)
            }),
            Duration::from_millis(1),
        ))
    }

    fn requires_sanitization(&self) -> bool {
        false
    }

    fn engine_compatibility(&self) -> EngineCompatibility {
        EngineCompatibility::Both
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- Payload validation --------------------------------------------------

    #[test]
    fn parse_finish_job_signal_accepts_valid_payload() {
        let signal = parse_finish_job_signal(&serde_json::json!({
            "status": "completed",
            "summary": "All requested work is done",
        }))
        .expect("valid finish_job payload");

        assert_eq!(signal.status, FinishJobStatus::Completed);
        assert_eq!(signal.summary, "All requested work is done");
    }

    #[test]
    fn parse_finish_job_signal_rejects_missing_summary() {
        let err = parse_finish_job_signal(&serde_json::json!({
            "status": "completed",
        }))
        .expect_err("missing summary must fail");

        assert!(
            err.to_string().contains("summary"),
            "error should mention missing summary: {err}"
        );
    }

    #[test]
    fn parse_finish_job_signal_rejects_invalid_status() {
        let err = parse_finish_job_signal(&serde_json::json!({
            "status": "done",
            "summary": "done",
        }))
        .expect_err("invalid status must fail");

        assert!(
            err.to_string().contains("status"),
            "error should mention invalid status: {err}"
        );
    }

    // --- Blank summary normalization ----------------------------------------

    #[test]
    fn parse_finish_job_signal_normalizes_blank_completed_summary() {
        let signal = parse_finish_job_signal(&serde_json::json!({
            "status": "completed",
            "summary": "   ",
        }))
        .expect("blank completed summary should be normalized");

        assert_eq!(signal.status, FinishJobStatus::Completed);
        assert_eq!(signal.summary, DEFAULT_NO_SUMMARY_COMPLETED);
    }

    #[test]
    fn parse_finish_job_signal_normalizes_blank_failed_summary() {
        let signal = parse_finish_job_signal(&serde_json::json!({
            "status": "failed",
            "summary": "\n\t",
        }))
        .expect("blank failed summary should be normalized");

        assert_eq!(signal.status, FinishJobStatus::Failed);
        assert_eq!(signal.summary, DEFAULT_NO_SUMMARY_FAILED);
    }
}
