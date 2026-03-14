//! Tools for the adaptive learning system (skill synthesis + approval).

use std::sync::Arc;

use async_trait::async_trait;

use crate::context::JobContext;
use crate::db::LearningStore;
use crate::tools::tool::{ApprovalRequirement, Tool, ToolError, ToolOutput, require_str};

/// Tool for listing pending synthesized skills awaiting approval.
pub struct SkillListPendingTool {
    store: Arc<dyn LearningStore>,
}

impl SkillListPendingTool {
    pub fn new(store: Arc<dyn LearningStore>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Tool for SkillListPendingTool {
    fn name(&self) -> &str {
        "skill_list_pending"
    }

    fn description(&self) -> &str {
        "List synthesized skills awaiting user approval. Shows skill name, \
         quality score, and whether safety checks passed."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
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
        let start = std::time::Instant::now();

        // TODO: use ctx.agent_id when multi-agent is supported
        let skills = self
            .store
            .list_synthesized_skills(
                &ctx.user_id,
                "default",
                Some(crate::db::SkillStatus::Pending),
            )
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to list skills: {e}")))?;

        if skills.is_empty() {
            return Ok(ToolOutput::text(
                "No pending synthesized skills.",
                start.elapsed(),
            ));
        }

        let mut output = format!("{} pending skill(s):\n\n", skills.len());
        for s in &skills {
            output.push_str(&format!(
                "- **{}** (id: {}, quality: {}/100, safety: {})\n",
                s.skill_name,
                s.id,
                s.quality_score,
                if s.safety_scan_passed {
                    "passed"
                } else {
                    "FAILED"
                },
            ));
        }

        Ok(ToolOutput::text(output, start.elapsed()))
    }

    fn requires_sanitization(&self) -> bool {
        false
    }
}

/// Tool for approving or rejecting a synthesized skill.
///
/// Approval requires explicit user consent (`ApprovalRequirement::Always`)
/// to prevent the LLM from auto-approving skills. Rejection is safe and
/// does not require approval.
pub struct SkillApproveTool {
    store: Arc<dyn LearningStore>,
}

impl SkillApproveTool {
    pub fn new(store: Arc<dyn LearningStore>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Tool for SkillApproveTool {
    fn name(&self) -> &str {
        "skill_approve"
    }

    fn description(&self) -> &str {
        "Approve or reject a pending synthesized skill. Approved skills \
         are saved to the auto-skills directory and available in future sessions. \
         Rejected skills are discarded."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "skill_id": {
                    "type": "string",
                    "description": "UUID of the synthesized skill to approve/reject"
                },
                "action": {
                    "type": "string",
                    "enum": ["accept", "reject"],
                    "description": "Whether to approve or reject the skill"
                }
            },
            "required": ["skill_id", "action"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let skill_id_str = require_str(&params, "skill_id")?;
        let action = require_str(&params, "action")?;

        let skill_id = uuid::Uuid::parse_str(skill_id_str)
            .map_err(|e| ToolError::InvalidParameters(format!("Invalid skill_id UUID: {e}")))?;

        let status = match action {
            "accept" => crate::db::SkillStatus::Accepted,
            "reject" => crate::db::SkillStatus::Rejected,
            other => {
                return Err(ToolError::InvalidParameters(format!(
                    "Invalid action '{other}', must be 'accept' or 'reject'"
                )));
            }
        };

        // Load skill for display name and (for accept) content validation.
        let skill_row = self
            .store
            .get_synthesized_skill(skill_id, &ctx.user_id)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to fetch skill: {e}")))?;

        let skill_display_name = skill_row.as_ref().map(|s| s.skill_name.clone());
        let skill_current_status = skill_row.as_ref().map(|s| s.status);

        // For "accept": validate and write to disk BEFORE updating status.
        // This prevents TOCTOU: if disk write fails, status stays "pending".
        // Reuse skill_row from the initial fetch (no redundant DB query).
        if status == crate::db::SkillStatus::Accepted {
            let Some(skill) = skill_row else {
                return Ok(ToolOutput::text(
                    format!("Skill {skill_id} not found or not owned by you."),
                    start.elapsed(),
                ));
            };

            let Some(content) = &skill.skill_content else {
                return Err(ToolError::ExecutionFailed(
                    "Skill has no content to install".to_string(),
                ));
            };

            // Block skills that failed safety scan
            if !skill.safety_scan_passed {
                return Err(ToolError::ExecutionFailed(
                    "Cannot accept skill that failed safety scan".to_string(),
                ));
            }

            // Re-validate content before writing to disk (defense in depth)
            let validator = crate::learning::validator::SkillValidator::new();
            if let Err(e) = validator.validate(content) {
                return Err(ToolError::ExecutionFailed(format!(
                    "Skill re-validation failed: {e}"
                )));
            }

            let auto_dir = crate::bootstrap::ironclaw_base_dir().join("installed_skills/auto");
            // Use UUID for directory name — guarantees uniqueness (no hash-prefix collisions)
            // and is inherently safe for filesystem paths (no traversal possible).
            // The skill's activation name is read from SKILL.md frontmatter, not the directory.
            let skill_dir = auto_dir.join(skill_id.to_string());

            // Write to disk first, update status only on success
            tokio::fs::create_dir_all(&skill_dir).await.map_err(|e| {
                ToolError::ExecutionFailed(format!("Failed to create auto-skill dir: {e}"))
            })?;

            tokio::fs::write(skill_dir.join("SKILL.md"), content.as_bytes())
                .await
                .map_err(|e| {
                    ToolError::ExecutionFailed(format!("Failed to write SKILL.md: {e}"))
                })?;

            tracing::info!(
                "Wrote auto-skill '{}' to {}",
                skill.skill_name,
                skill_dir.display()
            );
        }

        // Update status in DB — after successful disk write for accept,
        // or immediately for reject. This prevents TOCTOU: if disk write
        // fails above, status stays "pending" and user can retry.
        let updated = self
            .store
            .update_synthesized_skill_status(skill_id, &ctx.user_id, status)
            .await
            .map_err(|e| {
                ToolError::ExecutionFailed(format!("Failed to update skill status: {e}"))
            })?;

        if !updated {
            // Distinguish "not found" from "already processed" using
            // data saved from the initial fetch (no redundant DB query).
            let msg = match (&skill_display_name, skill_current_status) {
                (Some(name), Some(s)) => {
                    format!("Skill '{name}' is already {s} and cannot be changed.")
                }
                _ => format!("Skill {skill_id} not found or not owned by you."),
            };
            return Ok(ToolOutput::text(msg, start.elapsed()));
        }

        let display_name = skill_display_name.unwrap_or_else(|| skill_id_str.to_string());

        Ok(ToolOutput::text(
            format!("Skill '{display_name}' has been {status}."),
            start.elapsed(),
        ))
    }

    fn requires_sanitization(&self) -> bool {
        false
    }

    fn requires_approval(&self, params: &serde_json::Value) -> ApprovalRequirement {
        // Only require approval for "accept" — rejecting is always safe
        match params.get("action").and_then(|v| v.as_str()) {
            Some("reject") => ApprovalRequirement::Never,
            _ => ApprovalRequirement::Always,
        }
    }
}
