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
use crate::db::Database;
use crate::tools::builtin::memory::WorkspaceResolver;
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
        ctx: &JobContext,
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
                thread_id: ctx.conversation_id.map(|id| id.to_string()),
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

/// Save a thread-scoped markdown plan artifact for later review/approval.
pub struct PlanArtifactSaveTool {
    store: Option<Arc<dyn Database>>,
    resolver: Option<Arc<dyn WorkspaceResolver>>,
}

impl PlanArtifactSaveTool {
    pub fn new(
        store: Option<Arc<dyn Database>>,
        resolver: Option<Arc<dyn WorkspaceResolver>>,
    ) -> Self {
        Self { store, resolver }
    }
}

#[async_trait]
impl Tool for PlanArtifactSaveTool {
    fn name(&self) -> &str {
        "plan_artifact_save"
    }

    fn description(&self) -> &str {
        "Save the current thread's plan as markdown for review before leaving plan mode. \
         Use this once you have a concrete plan. Include a concise title, the full markdown \
         plan body, and 2-4 suggested next actions to surface after approval."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "title": {
                    "type": "string",
                    "description": "Short title for the plan"
                },
                "markdown": {
                    "type": "string",
                    "description": "Full markdown plan body to show in exit review"
                },
                "suggested_actions": {
                    "type": "array",
                    "description": "2-4 short suggested next actions to show after approval",
                    "items": { "type": "string" },
                    "minItems": 0,
                    "maxItems": 4
                }
            },
            "required": ["title", "markdown"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        let title = require_str(&params, "title")?.trim().to_string();
        let markdown = require_str(&params, "markdown")?.trim().to_string();
        let thread_id = ctx.conversation_id.ok_or_else(|| {
            ToolError::ExecutionFailed(
                "plan_artifact_save requires a conversation-bound chat thread".to_string(),
            )
        })?;

        if title.is_empty() {
            return Err(ToolError::InvalidParameters(
                "title cannot be empty".to_string(),
            ));
        }
        if markdown.is_empty() {
            return Err(ToolError::InvalidParameters(
                "markdown cannot be empty".to_string(),
            ));
        }

        let suggested_actions: Vec<String> = params
            .get("suggested_actions")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(str::trim).filter(|s| !s.is_empty()))
                    .take(4)
                    .map(ToOwned::to_owned)
                    .collect()
            })
            .unwrap_or_default();

        let artifact = crate::agent::session::PlanArtifact::for_thread(
            thread_id,
            title.clone(),
            markdown.clone(),
            suggested_actions.clone(),
        );

        if let Some(resolver) = self.resolver.as_ref() {
            let workspace = resolver.resolve(&ctx.user_id).await;
            let file_contents = format!(
                "---\nplan_id: {}\nthread_id: {}\ntitle: {}\nupdated_at: {}\nsuggested_actions: {}\n---\n\n{}",
                artifact.plan_id,
                thread_id,
                title,
                artifact.updated_at.to_rfc3339(),
                serde_json::to_string(&artifact.suggested_actions)
                    .unwrap_or_else(|_| "[]".to_string()),
                markdown
            );
            workspace
                .write(&artifact.path, &file_contents)
                .await
                .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;
        }

        if let Some(store) = self.store.as_ref() {
            let updates = [
                ("plan_artifact_title", serde_json::json!(artifact.title)),
                (
                    "plan_artifact_markdown",
                    serde_json::json!(artifact.markdown),
                ),
                ("plan_artifact_path", serde_json::json!(artifact.path)),
                (
                    "plan_artifact_updated_at",
                    serde_json::json!(artifact.updated_at.to_rfc3339()),
                ),
                (
                    "plan_artifact_suggested_actions",
                    serde_json::json!(artifact.suggested_actions),
                ),
            ];
            for (key, value) in updates {
                store
                    .update_conversation_metadata_field(thread_id, key, &value)
                    .await
                    .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;
            }
        }

        Ok(ToolOutput::success(
            serde_json::json!({
                "plan_id": artifact.plan_id,
                "path": artifact.path,
                "title": artifact.title,
                "suggested_actions": artifact.suggested_actions,
            }),
            start.elapsed(),
        ))
    }

    fn requires_sanitization(&self) -> bool {
        false
    }
}
