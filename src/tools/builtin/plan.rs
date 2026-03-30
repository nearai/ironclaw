//! Plan progress tool.
//!
//! A structured way for the LLM to report plan progress that clients can
//! render as a live checklist. Inspired by OpenAI Codex's `update_plan` —
//! "this function doesn't do anything useful; it gives the model a structured
//! way to record its plan that clients can read and render."

use std::sync::Arc;

use async_trait::async_trait;

use crate::channels::web::sse::SseManager;
use crate::context::JobContext;
use crate::tools::tool::{Tool, ToolError, ToolOutput, require_str};
use ironclaw_common::{AppEvent, PlanStepDto};

/// Tool for emitting structured plan progress updates via SSE.
#[derive(Default)]
pub struct PlanUpdateTool {
    sse_tx: Option<Arc<SseManager>>,
}

impl PlanUpdateTool {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_sse(mut self, sse: Arc<SseManager>) -> Self {
        self.sse_tx = Some(sse);
        self
    }
}

#[async_trait]
impl Tool for PlanUpdateTool {
    fn name(&self) -> &str {
        "plan_update"
    }

    fn description(&self) -> &str {
        "Update the plan progress checklist displayed to the user. Call this when creating a \
         plan, starting execution, completing a step, or when the plan fails. The UI renders \
         this as a live checklist. Always send the FULL list of steps (not incremental diffs)."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "plan_id": {
                    "type": "string",
                    "description": "Plan identifier (slug or ID)"
                },
                "title": {
                    "type": "string",
                    "description": "Plan title"
                },
                "status": {
                    "type": "string",
                    "enum": ["draft", "approved", "executing", "completed", "failed"],
                    "description": "Overall plan status"
                },
                "steps": {
                    "type": "array",
                    "description": "Full list of plan steps with their current status",
                    "items": {
                        "type": "object",
                        "properties": {
                            "title": {
                                "type": "string",
                                "description": "Step description"
                            },
                            "status": {
                                "type": "string",
                                "enum": ["pending", "in_progress", "completed", "failed"],
                                "description": "Step status"
                            },
                            "result": {
                                "type": "string",
                                "description": "Step result or error message (optional)"
                            }
                        },
                        "required": ["title", "status"]
                    }
                },
                "mission_id": {
                    "type": "string",
                    "description": "Associated mission ID (set after plan is approved and executing)"
                }
            },
            "required": ["plan_id", "title", "status", "steps"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let plan_id = require_str(&params, "plan_id")?;
        let title = require_str(&params, "title")?;
        let status = require_str(&params, "status")?;

        let steps: Vec<PlanStepDto> = params
            .get("steps")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .enumerate()
                    .filter_map(|(i, s)| {
                        Some(PlanStepDto {
                            index: i,
                            title: s.get("title")?.as_str()?.to_string(),
                            status: s.get("status")?.as_str()?.to_string(),
                            result: s
                                .get("result")
                                .and_then(|r| r.as_str())
                                .map(|s| s.to_string()),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        let mission_id = params
            .get("mission_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let completed = steps.iter().filter(|s| s.status == "completed").count();
        let total = steps.len();

        // Broadcast SSE event if manager is available
        if let Some(ref sse) = self.sse_tx {
            sse.broadcast(AppEvent::PlanUpdate {
                plan_id: plan_id.to_string(),
                title: title.to_string(),
                status: status.to_string(),
                steps: steps.clone(),
                mission_id: mission_id.clone(),
                thread_id: None,
            });
        }

        let summary = format!(
            "Plan '{}' updated: {} ({}/{} steps completed)",
            title, status, completed, total
        );

        Ok(ToolOutput::text(summary, start.elapsed()))
    }

    fn requires_sanitization(&self) -> bool {
        false // Internal tool, no external data
    }
}
