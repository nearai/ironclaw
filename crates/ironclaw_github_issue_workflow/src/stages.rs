use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

use crate::{
    GithubIssueStage, GithubIssueStageRunId, GithubIssueWorkflowRunId, StageResultValidationError,
    ValidatedStageResult, validate_stage_result,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StageResultBinding {
    pub workflow_run_id: GithubIssueWorkflowRunId,
    pub stage_run_id: GithubIssueStageRunId,
    pub turn_run_id: ironclaw_turns::TurnRunId,
    pub stage: GithubIssueStage,
    pub schema_version: String,
    pub completion_nonce: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StageResultAttempt {
    pub workflow_run_id: GithubIssueWorkflowRunId,
    pub stage_run_id: GithubIssueStageRunId,
    pub turn_run_id: ironclaw_turns::TurnRunId,
    pub stage: GithubIssueStage,
    pub schema_version: String,
    pub completion_nonce: String,
    pub result: JsonValue,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcceptedStageResult {
    pub validated_result: ValidatedStageResult,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StageResultReportDecision {
    Accepted {
        accepted_result: AcceptedStageResult,
    },
    Duplicate {
        accepted_result: AcceptedStageResult,
    },
    ValidationFailed {
        error: StageResultValidationError,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StageResultReportError {
    #[error("stage result attempt is stale")]
    StaleAttempt,

    #[error("stage result binding field `{field}` does not match the active stage run")]
    MismatchedBinding { field: &'static str },

    #[error("stage result conflicts with the already accepted result")]
    ConflictingAcceptedResult,
}

pub fn evaluate_stage_result_attempt(
    binding: &StageResultBinding,
    accepted_result: Option<&AcceptedStageResult>,
    attempt: StageResultAttempt,
) -> Result<StageResultReportDecision, StageResultReportError> {
    validate_attempt_binding(binding, &attempt)?;

    let validated_result =
        match validate_stage_result(attempt.stage, &attempt.schema_version, attempt.result) {
            Ok(result) => result,
            Err(error) => return Ok(StageResultReportDecision::ValidationFailed { error }),
        };
    let candidate = AcceptedStageResult { validated_result };

    if let Some(existing_result) = accepted_result {
        if existing_result == &candidate {
            return Ok(StageResultReportDecision::Duplicate {
                accepted_result: existing_result.clone(),
            });
        }
        return Err(StageResultReportError::ConflictingAcceptedResult);
    }

    Ok(StageResultReportDecision::Accepted {
        accepted_result: candidate,
    })
}

fn validate_attempt_binding(
    binding: &StageResultBinding,
    attempt: &StageResultAttempt,
) -> Result<(), StageResultReportError> {
    if attempt.workflow_run_id != binding.workflow_run_id {
        return Err(StageResultReportError::MismatchedBinding {
            field: "workflow_run_id",
        });
    }
    if attempt.stage_run_id != binding.stage_run_id {
        return Err(StageResultReportError::StaleAttempt);
    }
    if attempt.turn_run_id != binding.turn_run_id {
        return Err(StageResultReportError::MismatchedBinding {
            field: "turn_run_id",
        });
    }
    if attempt.stage != binding.stage {
        return Err(StageResultReportError::MismatchedBinding { field: "stage" });
    }
    if attempt.schema_version != binding.schema_version {
        return Err(StageResultReportError::MismatchedBinding {
            field: "schema_version",
        });
    }
    if attempt.completion_nonce != binding.completion_nonce {
        return Err(StageResultReportError::MismatchedBinding {
            field: "completion_nonce",
        });
    }

    Ok(())
}
