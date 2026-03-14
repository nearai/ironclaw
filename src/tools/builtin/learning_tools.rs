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
            .list_synthesized_skills(&ctx.user_id, "default", Some("pending"))
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
                "- **{}** (id: {}, quality: {}, safety: {})\n",
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

/// Validate that a skill name is safe for use as a filesystem directory name.
/// Prevents path traversal attacks.
fn is_safe_skill_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && !name.contains('/')
        && !name.contains('\\')
        && !name.contains('\0')
        && name != "."
        && name != ".."
        && !name.starts_with('-')
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
            "accept" => "accepted",
            "reject" => "rejected",
            other => {
                return Err(ToolError::InvalidParameters(format!(
                    "Invalid action '{other}', must be 'accept' or 'reject'"
                )));
            }
        };

        let updated = self
            .store
            .update_synthesized_skill_status(skill_id, &ctx.user_id, status)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to update skill: {e}")))?;

        if !updated {
            return Ok(ToolOutput::text(
                format!("Skill {skill_id} not found or not owned by you."),
                start.elapsed(),
            ));
        }

        // If accepted, write SKILL.md to auto-skills directory
        if status == "accepted" {
            let skill = self
                .store
                .get_synthesized_skill(skill_id, &ctx.user_id)
                .await
                .map_err(|e| ToolError::ExecutionFailed(format!("Failed to fetch skill: {e}")))?;

            if let Some(skill) = skill
                && let Some(content) = &skill.skill_content
            {
                // Validate skill_name to prevent path traversal
                if !is_safe_skill_name(&skill.skill_name) {
                    return Err(ToolError::ExecutionFailed(format!(
                        "Skill name '{}' contains unsafe characters",
                        skill.skill_name
                    )));
                }

                let auto_dir = crate::bootstrap::ironclaw_base_dir().join("installed_skills/auto");
                let skill_dir = auto_dir.join(&skill.skill_name);

                // Verify the resolved path is still under auto_dir (defense in depth)
                if !skill_dir.starts_with(&auto_dir) {
                    return Err(ToolError::ExecutionFailed(
                        "Skill path escapes auto-skills directory".to_string(),
                    ));
                }

                // Use tokio::fs for non-blocking I/O
                tokio::fs::create_dir_all(&skill_dir).await.map_err(|e| {
                    ToolError::ExecutionFailed(format!("Failed to create auto-skill dir: {e}"))
                })?;

                tokio::fs::write(skill_dir.join("SKILL.md"), content.as_bytes())
                    .await
                    .map_err(|e| {
                        ToolError::ExecutionFailed(format!(
                            "Skill accepted in DB but failed to write SKILL.md: {e}"
                        ))
                    })?;

                tracing::info!(
                    "Wrote auto-skill '{}' to {}",
                    skill.skill_name,
                    skill_dir.display()
                );
            }
        }

        Ok(ToolOutput::text(
            format!("Skill '{skill_id_str}' has been {status}."),
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
