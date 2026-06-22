use serde::{Deserialize, Serialize};

use crate::{
    EngineeredWorkflowSnapshot, GithubIssueStage, GithubIssueWorkflowError, WorkflowPromptContent,
    WorkflowPromptContentRef, render_stage_result_schema_contract, snapshot_hash,
    snapshot_serde_error, snapshots::sha256_hex_bytes, stage_result_schema_contract,
    stages::stage_slug,
};

const PROMPT_VERSION: &str = "v1";
const PROMPT_PACK: &str = "github_issue_bugfix";
const RESULT_TOOL: &str = "builtin.workflow_report_stage_result";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StagePromptBundle {
    pub prompt_ref: String,
    pub prompt_version: String,
    pub content: String,
    pub content_hash: String,
    pub snapshot_hash: String,
}

impl From<StagePromptBundle> for WorkflowPromptContent {
    fn from(prompt: StagePromptBundle) -> Self {
        Self {
            content_ref: WorkflowPromptContentRef {
                prompt_ref: prompt.prompt_ref,
                prompt_version: prompt.prompt_version,
                input_snapshot_hash: prompt.snapshot_hash,
            },
            content: prompt.content,
            content_hash: prompt.content_hash,
        }
    }
}

pub fn render_stage_prompt(
    stage: GithubIssueStage,
    snapshot: &EngineeredWorkflowSnapshot,
) -> Result<StagePromptBundle, GithubIssueWorkflowError> {
    ensure_snapshot_matches_stage(&stage, snapshot)?;
    let asset = prompt_asset(&stage);
    let schema_block = render_stage_result_schema_contract(&stage, RESULT_TOOL);
    let snapshot_hash = snapshot_hash(snapshot)?;
    let snapshot_json = serde_json::to_string_pretty(snapshot).map_err(snapshot_serde_error)?;
    let content = format!(
        "{template}\n\n---\n\n## Authoritative Result Schema\n{schema_block}\n\n## Engineered Workflow Snapshot\nSnapshot hash: `{snapshot_hash}`\n\n```json\n{snapshot_json}\n```\n",
        template = asset.template.trim(),
    );
    let content_hash = sha256_hex_bytes(content.as_bytes());

    Ok(StagePromptBundle {
        prompt_ref: asset.prompt_ref,
        prompt_version: PROMPT_VERSION.to_string(),
        content,
        content_hash,
        snapshot_hash,
    })
}

fn ensure_snapshot_matches_stage(
    stage: &GithubIssueStage,
    snapshot: &EngineeredWorkflowSnapshot,
) -> Result<(), GithubIssueWorkflowError> {
    if &snapshot.constraints.stage != stage {
        return Err(GithubIssueWorkflowError::Policy {
            reason: format!(
                "stage prompt `{}` cannot render snapshot constraints for `{}`",
                stage_slug(stage),
                stage_slug(&snapshot.constraints.stage)
            ),
        });
    }

    let schema = stage_result_schema_contract(stage);
    if snapshot.constraints.result_schema_version != schema.schema_version {
        return Err(GithubIssueWorkflowError::Policy {
            reason: format!(
                "stage prompt `{}` expected schema `{}` but snapshot declared `{}`",
                stage_slug(stage),
                schema.schema_version,
                snapshot.constraints.result_schema_version
            ),
        });
    }

    if snapshot.constraints.completion_tool != RESULT_TOOL {
        return Err(GithubIssueWorkflowError::Policy {
            reason: format!(
                "stage prompt `{}` requires completion tool `{}`",
                stage_slug(stage),
                RESULT_TOOL
            ),
        });
    }

    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PromptAsset {
    prompt_ref: String,
    template: &'static str,
}

fn prompt_asset(stage: &GithubIssueStage) -> PromptAsset {
    let file_stem = prompt_file_stem(stage);
    let template = match stage {
        GithubIssueStage::Triage => {
            include_str!("../prompts/github_issue_bugfix/v1/triage.md")
        }
        GithubIssueStage::Planning => {
            include_str!("../prompts/github_issue_bugfix/v1/plan.md")
        }
        GithubIssueStage::Implementation => {
            include_str!("../prompts/github_issue_bugfix/v1/implement.md")
        }
        GithubIssueStage::PrSynthesis => {
            include_str!("../prompts/github_issue_bugfix/v1/synthesize_pr.md")
        }
        GithubIssueStage::CiRepair => {
            include_str!("../prompts/github_issue_bugfix/v1/repair_ci.md")
        }
        GithubIssueStage::ReviewResponse => {
            include_str!("../prompts/github_issue_bugfix/v1/address_review.md")
        }
    };

    PromptAsset {
        prompt_ref: format!("{PROMPT_PACK}/{PROMPT_VERSION}/{file_stem}"),
        template,
    }
}

fn prompt_file_stem(stage: &GithubIssueStage) -> &'static str {
    match stage {
        GithubIssueStage::Triage => "triage",
        GithubIssueStage::Planning => "plan",
        GithubIssueStage::Implementation => "implement",
        GithubIssueStage::PrSynthesis => "synthesize_pr",
        GithubIssueStage::CiRepair => "repair_ci",
        GithubIssueStage::ReviewResponse => "address_review",
    }
}
