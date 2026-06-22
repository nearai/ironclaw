use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

use crate::GithubIssueStage;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StageResultEnvelope {
    pub outcome: StageResultOutcome,
    pub summary: String,
    pub evidence: Vec<StageEvidence>,
    pub next_actions: Vec<String>,
    pub payload: JsonValue,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StageResultOutcome {
    Completed,
    NeedsHuman,
    GaveUp,
    ExhaustedTurns,
    NotProduced,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StageEvidence {
    pub kind: String,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<JsonValue>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidatedStageResult {
    pub stage: GithubIssueStage,
    pub schema_version: String,
    pub envelope: StageResultEnvelope,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StageResultValidationError {
    #[error(
        "unsupported schema version `{schema_version}` for {stage:?}; expected `{expected_schema_version}`"
    )]
    UnsupportedSchemaVersion {
        stage: GithubIssueStage,
        schema_version: String,
        expected_schema_version: &'static str,
    },

    #[error("invalid stage result envelope: {reason}")]
    InvalidEnvelope { reason: String },

    #[error("missing required {stage:?} payload field `{field}`")]
    MissingPayloadField {
        stage: GithubIssueStage,
        field: &'static str,
    },

    #[error("invalid {stage:?} payload field `{field}`: {reason}")]
    InvalidPayloadField {
        stage: GithubIssueStage,
        field: &'static str,
        reason: &'static str,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RequiredFieldKind {
    Array,
    Bool,
    Number,
    String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RequiredPayloadField {
    name: &'static str,
    kind: RequiredFieldKind,
}

pub fn validate_stage_result(
    stage: GithubIssueStage,
    schema_version: &str,
    value: JsonValue,
) -> Result<ValidatedStageResult, StageResultValidationError> {
    let expected_schema_version = stage_result_schema_version(&stage);
    if schema_version != expected_schema_version {
        return Err(StageResultValidationError::UnsupportedSchemaVersion {
            stage,
            schema_version: schema_version.to_string(),
            expected_schema_version,
        });
    }

    let envelope: StageResultEnvelope = serde_json::from_value(value).map_err(|error| {
        StageResultValidationError::InvalidEnvelope {
            reason: error.to_string(),
        }
    })?;
    validate_envelope(&stage, &envelope)?;
    validate_stage_payload(&stage, &envelope.payload)?;

    Ok(ValidatedStageResult {
        stage,
        schema_version: schema_version.to_string(),
        envelope,
    })
}

pub fn stage_result_schema_version(stage: &GithubIssueStage) -> &'static str {
    match stage {
        GithubIssueStage::Triage => "triage.v1",
        GithubIssueStage::Planning => "planning.v1",
        GithubIssueStage::Implementation => "implementation.v1",
        GithubIssueStage::PrSynthesis => "pr_synthesis.v1",
        GithubIssueStage::CiRepair => "ci_repair.v1",
        GithubIssueStage::ReviewResponse => "review_response.v1",
    }
}

fn validate_envelope(
    stage: &GithubIssueStage,
    envelope: &StageResultEnvelope,
) -> Result<(), StageResultValidationError> {
    if envelope.summary.trim().is_empty() {
        return Err(StageResultValidationError::InvalidPayloadField {
            stage: stage.clone(),
            field: "summary",
            reason: "must not be empty",
        });
    }
    for evidence in &envelope.evidence {
        if evidence.kind.trim().is_empty() {
            return Err(StageResultValidationError::InvalidPayloadField {
                stage: stage.clone(),
                field: "evidence.kind",
                reason: "must not be empty",
            });
        }
        if evidence.summary.trim().is_empty() {
            return Err(StageResultValidationError::InvalidPayloadField {
                stage: stage.clone(),
                field: "evidence.summary",
                reason: "must not be empty",
            });
        }
    }
    Ok(())
}

fn validate_stage_payload(
    stage: &GithubIssueStage,
    payload: &JsonValue,
) -> Result<(), StageResultValidationError> {
    let Some(payload_object) = payload.as_object() else {
        return Err(StageResultValidationError::InvalidPayloadField {
            stage: stage.clone(),
            field: "payload",
            reason: "must be an object",
        });
    };

    for &required_field in required_payload_fields(stage) {
        validate_required_payload_field(stage, payload_object, required_field)?;
    }

    Ok(())
}

fn required_payload_fields(stage: &GithubIssueStage) -> &'static [RequiredPayloadField] {
    match stage {
        GithubIssueStage::Triage => &[
            RequiredPayloadField {
                name: "is_reproducible",
                kind: RequiredFieldKind::Bool,
            },
            RequiredPayloadField {
                name: "suspected_area",
                kind: RequiredFieldKind::String,
            },
            RequiredPayloadField {
                name: "risk",
                kind: RequiredFieldKind::String,
            },
            RequiredPayloadField {
                name: "recommended_next_stage",
                kind: RequiredFieldKind::String,
            },
        ],
        GithubIssueStage::Planning => &[
            RequiredPayloadField {
                name: "plan_items",
                kind: RequiredFieldKind::Array,
            },
            RequiredPayloadField {
                name: "files_to_inspect_or_change",
                kind: RequiredFieldKind::Array,
            },
            RequiredPayloadField {
                name: "test_strategy",
                kind: RequiredFieldKind::String,
            },
            RequiredPayloadField {
                name: "confidence",
                kind: RequiredFieldKind::Number,
            },
        ],
        GithubIssueStage::Implementation => &[
            RequiredPayloadField {
                name: "changed_files",
                kind: RequiredFieldKind::Array,
            },
            RequiredPayloadField {
                name: "commands_run",
                kind: RequiredFieldKind::Array,
            },
            RequiredPayloadField {
                name: "test_evidence",
                kind: RequiredFieldKind::Array,
            },
            RequiredPayloadField {
                name: "pr_ready",
                kind: RequiredFieldKind::Bool,
            },
        ],
        GithubIssueStage::PrSynthesis => &[
            RequiredPayloadField {
                name: "title",
                kind: RequiredFieldKind::String,
            },
            RequiredPayloadField {
                name: "body",
                kind: RequiredFieldKind::String,
            },
            RequiredPayloadField {
                name: "branch_name",
                kind: RequiredFieldKind::String,
            },
            RequiredPayloadField {
                name: "base_branch",
                kind: RequiredFieldKind::String,
            },
            RequiredPayloadField {
                name: "head_sha",
                kind: RequiredFieldKind::String,
            },
        ],
        GithubIssueStage::CiRepair => &[
            RequiredPayloadField {
                name: "failing_checks",
                kind: RequiredFieldKind::Array,
            },
            RequiredPayloadField {
                name: "diagnosis",
                kind: RequiredFieldKind::String,
            },
            RequiredPayloadField {
                name: "changed_files",
                kind: RequiredFieldKind::Array,
            },
            RequiredPayloadField {
                name: "commands_run",
                kind: RequiredFieldKind::Array,
            },
        ],
        GithubIssueStage::ReviewResponse => &[
            RequiredPayloadField {
                name: "addressed_comments",
                kind: RequiredFieldKind::Array,
            },
            RequiredPayloadField {
                name: "remaining_comments",
                kind: RequiredFieldKind::Array,
            },
            RequiredPayloadField {
                name: "commands_run",
                kind: RequiredFieldKind::Array,
            },
        ],
    }
}

fn validate_required_payload_field(
    stage: &GithubIssueStage,
    payload_object: &serde_json::Map<String, JsonValue>,
    required_field: RequiredPayloadField,
) -> Result<(), StageResultValidationError> {
    let value = payload_object.get(required_field.name).ok_or_else(|| {
        StageResultValidationError::MissingPayloadField {
            stage: stage.clone(),
            field: required_field.name,
        }
    })?;
    if value.is_null() {
        return Err(StageResultValidationError::InvalidPayloadField {
            stage: stage.clone(),
            field: required_field.name,
            reason: "must not be null",
        });
    }
    if field_matches_kind(value, required_field.kind) {
        return Ok(());
    }

    Err(StageResultValidationError::InvalidPayloadField {
        stage: stage.clone(),
        field: required_field.name,
        reason: required_field.kind.expected_description(),
    })
}

fn field_matches_kind(value: &JsonValue, kind: RequiredFieldKind) -> bool {
    match kind {
        RequiredFieldKind::Array => value.is_array(),
        RequiredFieldKind::Bool => value.is_boolean(),
        RequiredFieldKind::Number => value.is_number(),
        RequiredFieldKind::String => value
            .as_str()
            .map(|string| !string.trim().is_empty())
            .unwrap_or(false),
    }
}

impl RequiredFieldKind {
    fn expected_description(self) -> &'static str {
        match self {
            RequiredFieldKind::Array => "must be an array",
            RequiredFieldKind::Bool => "must be a boolean",
            RequiredFieldKind::Number => "must be a number",
            RequiredFieldKind::String => "must be a non-empty string",
        }
    }
}
