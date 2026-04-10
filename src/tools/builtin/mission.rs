//! LLM-facing tools for managing missions (engine v2).
//!
//! Six tools let the agent manage missions conversationally:
//! - `mission_create` - Create a new long-running mission
//! - `mission_list` - List all missions with status
//! - `mission_update` - Modify a mission's settings
//! - `mission_delete` - Remove a mission
//! - `mission_fire` - Manually trigger a mission
//! - `mission_pause` / `mission_resume` - Pause or resume a mission

use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use serde_json::json;

use ironclaw_engine::types::mission::MissionCadence;
use ironclaw_engine::{MissionManager, MissionUpdate, ProjectId};

use crate::context::JobContext;
use crate::tools::tool::{
    EngineCompatibility, RiskLevel, Tool, ToolDomain, ToolError, ToolOutput, require_str,
};

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Parse a cadence string into a MissionCadence.
fn parse_cadence(s: &str) -> MissionCadence {
    let trimmed = s.trim().to_lowercase();
    if trimmed == "manual" {
        MissionCadence::Manual
    } else if trimmed.contains(' ') && trimmed.split_whitespace().count() >= 5 {
        MissionCadence::Cron {
            expression: s.trim().to_string(),
            timezone: None,
        }
    } else if let Some(pattern) = trimmed.strip_prefix("event:") {
        MissionCadence::OnEvent {
            event_pattern: pattern.trim().to_string(),
            channel: None,
        }
    } else if let Some(path) = trimmed.strip_prefix("webhook:") {
        MissionCadence::Webhook {
            path: path.trim().to_string(),
            secret: None,
        }
    } else {
        MissionCadence::Manual
    }
}

fn parse_mission_id(params: &serde_json::Value) -> Result<ironclaw_engine::MissionId, ToolError> {
    let id_str = require_str(params, "id")?;
    uuid::Uuid::parse_str(id_str)
        .map(ironclaw_engine::MissionId)
        .map_err(|e| ToolError::InvalidParameters(format!("invalid mission id: {e}")))
}

/// Extract notify_channels from params, falling back to the job context channel.
fn extract_notify_channels(params: &serde_json::Value, ctx: &JobContext) -> Vec<String> {
    if let Some(arr) = params.get("notify_channels").and_then(|v| v.as_array()) {
        arr.iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect()
    } else if let Some(ch) = ctx
        .metadata
        .get("notify_channel")
        .and_then(|v| v.as_str())
    {
        vec![ch.to_string()]
    } else {
        vec![]
    }
}

// ===========================================================================
// mission_create
// ===========================================================================

pub struct MissionCreateTool {
    manager: Arc<MissionManager>,
    project_id: ProjectId,
}

impl MissionCreateTool {
    pub fn new(manager: Arc<MissionManager>, project_id: ProjectId) -> Self {
        Self {
            manager,
            project_id,
        }
    }
}

#[async_trait]
impl Tool for MissionCreateTool {
    fn name(&self) -> &str {
        "mission_create"
    }

    fn description(&self) -> &str {
        "Create a new long-running mission that spawns threads over time. \
         Supports cron schedules (e.g. '0 */6 * * *'), event patterns ('event:<pattern>'), \
         webhooks ('webhook:<path>'), or manual triggers."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Human-readable mission name"
                },
                "goal": {
                    "type": "string",
                    "description": "What the mission should accomplish"
                },
                "cadence": {
                    "type": "string",
                    "description": "Trigger cadence: 'manual', cron expression (e.g. '0 9 * * *'), 'event:<pattern>', or 'webhook:<path>'. Default: 'manual'."
                },
                "notify_channels": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Channels to notify on completion (e.g. ['gateway', 'telegram']). Defaults to current channel."
                },
                "success_criteria": {
                    "type": "string",
                    "description": "Optional criteria for declaring the mission complete"
                }
            },
            "required": ["name", "goal"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = Instant::now();
        let name = require_str(&params, "name")?;
        let goal = require_str(&params, "goal")?;
        let cadence_str = params
            .get("cadence")
            .and_then(|v| v.as_str())
            .unwrap_or("manual");
        let notify_channels = extract_notify_channels(&params, ctx);

        let id = self
            .manager
            .create_mission(
                self.project_id,
                &ctx.user_id,
                name,
                goal,
                parse_cadence(cadence_str),
                notify_channels,
            )
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("failed to create mission: {e}")))?;

        // Set success_criteria if provided
        if let Some(criteria) = params.get("success_criteria").and_then(|v| v.as_str()) {
            let updates = MissionUpdate {
                success_criteria: Some(criteria.to_string()),
                ..Default::default()
            };
            let _ = self
                .manager
                .update_mission(id, &ctx.user_id, updates)
                .await;
        }

        let result = json!({"mission_id": id.to_string(), "status": "created"});
        Ok(ToolOutput::success(result, start.elapsed()))
    }

    fn engine_compatibility(&self) -> EngineCompatibility {
        EngineCompatibility::V2Only
    }

    fn domain(&self) -> ToolDomain {
        ToolDomain::Orchestrator
    }

    fn risk_level_for(&self, _params: &serde_json::Value) -> RiskLevel {
        RiskLevel::Low
    }
}

// ===========================================================================
// mission_list
// ===========================================================================

pub struct MissionListTool {
    manager: Arc<MissionManager>,
    project_id: ProjectId,
}

impl MissionListTool {
    pub fn new(manager: Arc<MissionManager>, project_id: ProjectId) -> Self {
        Self {
            manager,
            project_id,
        }
    }
}

#[async_trait]
impl Tool for MissionListTool {
    fn name(&self) -> &str {
        "mission_list"
    }

    fn description(&self) -> &str {
        "List all missions with their status, goal, and current focus."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {},
            "required": []
        })
    }

    async fn execute(
        &self,
        _params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = Instant::now();
        let missions = self
            .manager
            .list_missions(self.project_id, &ctx.user_id)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("failed to list missions: {e}")))?;

        let list: Vec<serde_json::Value> = missions
            .iter()
            .map(|m| {
                json!({
                    "id": m.id.to_string(),
                    "name": m.name,
                    "goal": m.goal,
                    "status": format!("{:?}", m.status),
                    "threads": m.thread_history.len(),
                    "current_focus": m.current_focus,
                    "notify_channels": m.notify_channels,
                })
            })
            .collect();

        Ok(ToolOutput::success(json!(list), start.elapsed()))
    }

    fn engine_compatibility(&self) -> EngineCompatibility {
        EngineCompatibility::V2Only
    }

    fn domain(&self) -> ToolDomain {
        ToolDomain::Orchestrator
    }

    fn risk_level_for(&self, _params: &serde_json::Value) -> RiskLevel {
        RiskLevel::Low
    }
}

// ===========================================================================
// mission_fire
// ===========================================================================

pub struct MissionFireTool {
    manager: Arc<MissionManager>,
}

impl MissionFireTool {
    pub fn new(manager: Arc<MissionManager>) -> Self {
        Self { manager }
    }
}

#[async_trait]
impl Tool for MissionFireTool {
    fn name(&self) -> &str {
        "mission_fire"
    }

    fn description(&self) -> &str {
        "Manually trigger a mission to spawn a thread now."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "id": {
                    "type": "string",
                    "description": "Mission ID to fire"
                }
            },
            "required": ["id"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = Instant::now();
        let id = parse_mission_id(&params)?;

        match self
            .manager
            .fire_mission(id, &ctx.user_id, None)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("failed to fire mission: {e}")))?
        {
            Some(tid) => Ok(ToolOutput::success(
                json!({"thread_id": tid.to_string(), "status": "fired"}),
                start.elapsed(),
            )),
            None => Ok(ToolOutput::success(
                json!({"status": "not_fired", "reason": "mission is terminal or budget exhausted"}),
                start.elapsed(),
            )),
        }
    }

    fn engine_compatibility(&self) -> EngineCompatibility {
        EngineCompatibility::V2Only
    }

    fn domain(&self) -> ToolDomain {
        ToolDomain::Orchestrator
    }

    fn risk_level_for(&self, _params: &serde_json::Value) -> RiskLevel {
        RiskLevel::Medium
    }
}

// ===========================================================================
// mission_pause / mission_resume
// ===========================================================================

pub struct MissionPauseTool {
    manager: Arc<MissionManager>,
}

impl MissionPauseTool {
    pub fn new(manager: Arc<MissionManager>) -> Self {
        Self { manager }
    }
}

#[async_trait]
impl Tool for MissionPauseTool {
    fn name(&self) -> &str {
        "mission_pause"
    }

    fn description(&self) -> &str {
        "Pause a mission so it stops spawning new threads."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "id": {
                    "type": "string",
                    "description": "Mission ID to pause"
                }
            },
            "required": ["id"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = Instant::now();
        let id = parse_mission_id(&params)?;

        self.manager
            .pause_mission(id, &ctx.user_id)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("failed to pause mission: {e}")))?;

        Ok(ToolOutput::success(json!({"status": "paused"}), start.elapsed()))
    }

    fn engine_compatibility(&self) -> EngineCompatibility {
        EngineCompatibility::V2Only
    }

    fn domain(&self) -> ToolDomain {
        ToolDomain::Orchestrator
    }

    fn risk_level_for(&self, _params: &serde_json::Value) -> RiskLevel {
        RiskLevel::Medium
    }
}

pub struct MissionResumeTool {
    manager: Arc<MissionManager>,
}

impl MissionResumeTool {
    pub fn new(manager: Arc<MissionManager>) -> Self {
        Self { manager }
    }
}

#[async_trait]
impl Tool for MissionResumeTool {
    fn name(&self) -> &str {
        "mission_resume"
    }

    fn description(&self) -> &str {
        "Resume a paused mission so it starts spawning threads again."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "id": {
                    "type": "string",
                    "description": "Mission ID to resume"
                }
            },
            "required": ["id"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = Instant::now();
        let id = parse_mission_id(&params)?;

        self.manager
            .resume_mission(id, &ctx.user_id)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("failed to resume mission: {e}")))?;

        Ok(ToolOutput::success(json!({"status": "resumed"}), start.elapsed()))
    }

    fn engine_compatibility(&self) -> EngineCompatibility {
        EngineCompatibility::V2Only
    }

    fn domain(&self) -> ToolDomain {
        ToolDomain::Orchestrator
    }

    fn risk_level_for(&self, _params: &serde_json::Value) -> RiskLevel {
        RiskLevel::Medium
    }
}

// ===========================================================================
// mission_delete
// ===========================================================================

pub struct MissionDeleteTool {
    manager: Arc<MissionManager>,
}

impl MissionDeleteTool {
    pub fn new(manager: Arc<MissionManager>) -> Self {
        Self { manager }
    }
}

#[async_trait]
impl Tool for MissionDeleteTool {
    fn name(&self) -> &str {
        "mission_delete"
    }

    fn description(&self) -> &str {
        "Delete a mission. This marks the mission as completed and stops all future thread spawning."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "id": {
                    "type": "string",
                    "description": "Mission ID to delete"
                }
            },
            "required": ["id"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = Instant::now();
        let id = parse_mission_id(&params)?;

        self.manager
            .complete_mission(id)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("failed to delete mission: {e}")))?;

        Ok(ToolOutput::success(json!({"status": "deleted"}), start.elapsed()))
    }

    fn engine_compatibility(&self) -> EngineCompatibility {
        EngineCompatibility::V2Only
    }

    fn domain(&self) -> ToolDomain {
        ToolDomain::Orchestrator
    }

    fn risk_level_for(&self, _params: &serde_json::Value) -> RiskLevel {
        RiskLevel::High
    }
}

// ===========================================================================
// mission_update
// ===========================================================================

pub struct MissionUpdateTool {
    manager: Arc<MissionManager>,
}

impl MissionUpdateTool {
    pub fn new(manager: Arc<MissionManager>) -> Self {
        Self { manager }
    }
}

#[async_trait]
impl Tool for MissionUpdateTool {
    fn name(&self) -> &str {
        "mission_update"
    }

    fn description(&self) -> &str {
        "Update a mission's name, goal, cadence, notification channels, budget, or success criteria."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "id": {
                    "type": "string",
                    "description": "Mission ID to update"
                },
                "name": {
                    "type": "string",
                    "description": "New mission name"
                },
                "goal": {
                    "type": "string",
                    "description": "New mission goal"
                },
                "cadence": {
                    "type": "string",
                    "description": "New cadence: 'manual', cron expression, 'event:<pattern>', or 'webhook:<path>'"
                },
                "notify_channels": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "New notification channels"
                },
                "max_threads_per_day": {
                    "type": "integer",
                    "description": "Maximum threads per day (0 = unlimited)"
                },
                "success_criteria": {
                    "type": "string",
                    "description": "Criteria for declaring the mission complete"
                }
            },
            "required": ["id"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = Instant::now();
        let id = parse_mission_id(&params)?;

        let mut updates = MissionUpdate::default();
        if let Some(name) = params.get("name").and_then(|v| v.as_str()) {
            updates.name = Some(name.to_string());
        }
        if let Some(goal) = params.get("goal").and_then(|v| v.as_str()) {
            updates.goal = Some(goal.to_string());
        }
        if let Some(cadence) = params.get("cadence").and_then(|v| v.as_str()) {
            updates.cadence = Some(parse_cadence(cadence));
        }
        if let Some(arr) = params.get("notify_channels").and_then(|v| v.as_array()) {
            updates.notify_channels = Some(
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect(),
            );
        }
        if let Some(max) = params.get("max_threads_per_day").and_then(|v| v.as_u64()) {
            updates.max_threads_per_day = Some(max as u32);
        }
        if let Some(criteria) = params.get("success_criteria").and_then(|v| v.as_str()) {
            updates.success_criteria = Some(criteria.to_string());
        }

        self.manager
            .update_mission(id, &ctx.user_id, updates)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("failed to update mission: {e}")))?;

        Ok(ToolOutput::success(json!({"status": "updated"}), start.elapsed()))
    }

    fn engine_compatibility(&self) -> EngineCompatibility {
        EngineCompatibility::V2Only
    }

    fn domain(&self) -> ToolDomain {
        ToolDomain::Orchestrator
    }

    fn risk_level_for(&self, _params: &serde_json::Value) -> RiskLevel {
        RiskLevel::Medium
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_cadence_manual() {
        assert!(matches!(parse_cadence("manual"), MissionCadence::Manual));
        assert!(matches!(parse_cadence("MANUAL"), MissionCadence::Manual));
    }

    #[test]
    fn test_parse_cadence_cron() {
        match parse_cadence("0 */6 * * *") {
            MissionCadence::Cron { expression, .. } => {
                assert_eq!(expression, "0 */6 * * *");
            }
            other => panic!("expected Cron, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_cadence_event() {
        match parse_cadence("event:price_alert") {
            MissionCadence::OnEvent { event_pattern, .. } => {
                assert_eq!(event_pattern, "price_alert");
            }
            other => panic!("expected OnEvent, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_cadence_webhook() {
        match parse_cadence("webhook:/hooks/deploy") {
            MissionCadence::Webhook { path, secret } => {
                assert_eq!(path, "/hooks/deploy");
                assert!(secret.is_none());
            }
            other => panic!("expected Webhook, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_cadence_unknown_defaults_manual() {
        assert!(matches!(
            parse_cadence("something_weird"),
            MissionCadence::Manual
        ));
    }
}
