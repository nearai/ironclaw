//! TaskRabbit tool for real-world task delegation.

use async_trait::async_trait;
use rust_decimal::Decimal;

use crate::context::JobContext;
use crate::tools::tool::{Tool, ToolError, ToolOutput};

/// Tool for delegating real-world tasks via TaskRabbit.
pub struct TaskRabbitTool {
    // TODO: Add TaskRabbit API client
}

impl TaskRabbitTool {
    /// Create a new TaskRabbit tool.
    pub fn new() -> Self {
        Self {}
    }
}

impl Default for TaskRabbitTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for TaskRabbitTool {
    fn name(&self) -> &str {
        "taskrabbit"
    }

    fn description(&self) -> &str {
        "Delegate real-world tasks to TaskRabbit taskers (delivery, assembly, cleaning, etc.)."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["search_taskers", "get_quote", "book_task", "get_status", "cancel_task"],
                    "description": "The TaskRabbit action to perform"
                },
                "task_type": {
                    "type": "string",
                    "enum": ["delivery", "assembly", "moving", "cleaning", "handyman", "other"],
                    "description": "Type of task"
                },
                "description": {
                    "type": "string",
                    "description": "Detailed description of the task"
                },
                "location": {
                    "type": "object",
                    "properties": {
                        "address": { "type": "string" },
                        "city": { "type": "string" },
                        "state": { "type": "string" },
                        "zip": { "type": "string" }
                    },
                    "description": "Location for the task"
                },
                "scheduled_time": {
                    "type": "string",
                    "description": "ISO 8601 datetime for when the task should be performed"
                },
                "budget": {
                    "type": "number",
                    "description": "Maximum budget for the task in USD"
                },
                "task_id": {
                    "type": "string",
                    "description": "Task ID (for get_status, cancel_task)"
                }
            },
            "required": ["action"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let action = params
            .get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                ToolError::InvalidParameters("missing 'action' parameter".to_string())
            })?;

        // TODO: Implement actual TaskRabbit API integration
        let result = match action {
            "search_taskers" => {
                serde_json::json!({
                    "taskers": [],
                    "message": "TaskRabbit integration not yet implemented"
                })
            }
            "get_quote" => {
                serde_json::json!({
                    "quotes": [],
                    "message": "TaskRabbit integration not yet implemented"
                })
            }
            "book_task" => {
                serde_json::json!({
                    "booked": false,
                    "message": "TaskRabbit integration not yet implemented"
                })
            }
            "get_status" => {
                let task_id = params.get("task_id").and_then(|v| v.as_str());

                serde_json::json!({
                    "task_id": task_id,
                    "status": "unknown",
                    "message": "TaskRabbit integration not yet implemented"
                })
            }
            "cancel_task" => {
                serde_json::json!({
                    "cancelled": false,
                    "message": "TaskRabbit integration not yet implemented"
                })
            }
            _ => {
                return Err(ToolError::InvalidParameters(format!(
                    "unknown action: {}",
                    action
                )));
            }
        };

        Ok(ToolOutput::success(result, start.elapsed()))
    }

    fn estimated_cost(&self, params: &serde_json::Value) -> Option<Decimal> {
        // Booking a task has associated costs
        if params.get("action").and_then(|v| v.as_str()) == Some("book_task") {
            params
                .get("budget")
                .and_then(|v| v.as_f64())
                .map(|b| Decimal::try_from(b).unwrap_or_default())
        } else {
            None
        }
    }

    fn requires_sanitization(&self) -> bool {
        true // External TaskRabbit data
    }
}
