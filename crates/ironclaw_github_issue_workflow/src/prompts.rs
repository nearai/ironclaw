use serde::{Deserialize, Serialize};

use crate::{
    EngineeredWorkflowSnapshot, GithubIssueStage, GithubIssueWorkflowError, snapshot_hash,
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

pub fn render_stage_prompt(
    stage: GithubIssueStage,
    snapshot: &EngineeredWorkflowSnapshot,
) -> Result<StagePromptBundle, GithubIssueWorkflowError> {
    ensure_snapshot_matches_stage(&stage, snapshot)?;
    let asset = prompt_asset(&stage);
    let schema_block = render_schema_block(&stage);
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

fn render_schema_block(stage: &GithubIssueStage) -> String {
    let schema = stage_result_schema_contract(stage);
    let fields = schema
        .payload_fields
        .iter()
        .map(|field| format!("- `{}`: {}", field.name, field.kind.schema_description()))
        .collect::<Vec<_>>()
        .join("\n");
    let shape = schema
        .payload_fields
        .iter()
        .map(|field| {
            format!(
                "    \"{}\": \"{}\"",
                field.name,
                field.kind.schema_description()
            )
        })
        .collect::<Vec<_>>()
        .join(",\n");

    format!(
        "Report completion only through `{RESULT_TOOL}`.\n\
         Use stage `{stage}` and schema version `{schema_version}`.\n\
         The `result` argument must be a strict stage result envelope:\n\n\
         ```json\n\
         {{\n\
           \"outcome\": \"completed | needs_human | gave_up | exhausted_turns | not_produced\",\n\
           \"summary\": \"non-empty string\",\n\
           \"evidence\": [{{\"kind\": \"non-empty string\", \"summary\": \"non-empty string\", \"data\": \"optional JSON\"}}],\n\
           \"next_actions\": [\"string\"],\n\
           \"payload\": {{\n{shape}\n\
           }}\n\
         }}\n\
         ```\n\n\
         Required payload fields:\n\
         {fields}\n\n\
         No unknown payload fields are accepted.",
        stage = stage_slug(stage),
        schema_version = schema.schema_version,
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PromptAsset {
    prompt_ref: String,
    template: &'static str,
}

fn prompt_asset(stage: &GithubIssueStage) -> PromptAsset {
    let slug = stage_slug(stage);
    let template = match stage {
        GithubIssueStage::Triage => {
            include_str!("../prompts/github_issue_bugfix/v1/triage.md")
        }
        GithubIssueStage::Planning => {
            include_str!("../prompts/github_issue_bugfix/v1/planning.md")
        }
        GithubIssueStage::Implementation => {
            include_str!("../prompts/github_issue_bugfix/v1/implementation.md")
        }
        GithubIssueStage::PrSynthesis => {
            include_str!("../prompts/github_issue_bugfix/v1/pr_synthesis.md")
        }
        GithubIssueStage::CiRepair => {
            include_str!("../prompts/github_issue_bugfix/v1/ci_repair.md")
        }
        GithubIssueStage::ReviewResponse => {
            include_str!("../prompts/github_issue_bugfix/v1/review_response.md")
        }
    };

    PromptAsset {
        prompt_ref: format!("{PROMPT_PACK}/{PROMPT_VERSION}/{slug}"),
        template,
    }
}
