use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

use crate::{GithubIssueStage, stages::stage_slug};

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

    #[error("unknown {stage:?} payload field `{field}` for schema `{schema_version}`")]
    UnknownPayloadField {
        stage: GithubIssueStage,
        schema_version: &'static str,
        field: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StagePayloadFieldKind {
    Array,
    Bool,
    Number,
    String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StagePayloadFieldRequirement {
    pub name: &'static str,
    pub kind: StagePayloadFieldKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StageResultContractField {
    name: &'static str,
    kind: StageResultContractFieldKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StageResultContractFieldKind {
    Outcome,
    NonEmptyString,
    EvidenceArray,
    StringArray,
    PayloadObject,
    OptionalJson,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StageResultSchemaContract {
    pub schema_version: &'static str,
    pub payload_fields: &'static [StagePayloadFieldRequirement],
}

const STAGE_RESULT_OUTCOME_VALUES: &[&str] = &[
    "completed",
    "needs_human",
    "gave_up",
    "exhausted_turns",
    "not_produced",
];

const STAGE_RESULT_ENVELOPE_FIELDS: &[StageResultContractField] = &[
    StageResultContractField {
        name: "outcome",
        kind: StageResultContractFieldKind::Outcome,
    },
    StageResultContractField {
        name: "summary",
        kind: StageResultContractFieldKind::NonEmptyString,
    },
    StageResultContractField {
        name: "evidence",
        kind: StageResultContractFieldKind::EvidenceArray,
    },
    StageResultContractField {
        name: "next_actions",
        kind: StageResultContractFieldKind::StringArray,
    },
    StageResultContractField {
        name: "payload",
        kind: StageResultContractFieldKind::PayloadObject,
    },
];

const STAGE_RESULT_EVIDENCE_FIELDS: &[StageResultContractField] = &[
    StageResultContractField {
        name: "kind",
        kind: StageResultContractFieldKind::NonEmptyString,
    },
    StageResultContractField {
        name: "summary",
        kind: StageResultContractFieldKind::NonEmptyString,
    },
    StageResultContractField {
        name: "data",
        kind: StageResultContractFieldKind::OptionalJson,
    },
];

pub fn stage_result_schema_contract(stage: &GithubIssueStage) -> StageResultSchemaContract {
    StageResultSchemaContract {
        schema_version: stage_result_schema_version(stage),
        payload_fields: required_payload_fields(stage),
    }
}

pub fn render_stage_result_schema_contract(stage: &GithubIssueStage, result_tool: &str) -> String {
    let schema = stage_result_schema_contract(stage);
    let fields = render_payload_field_list(schema.payload_fields);
    let shape = render_payload_shape(schema.payload_fields);
    let envelope_shape = render_stage_result_envelope_shape(&shape);

    format!(
        "Report completion only through `{result_tool}`.\n\
         Use stage `{stage}` and schema version `{schema_version}`.\n\
         The `result` argument must be a strict stage result envelope:\n\n\
         ```json\n\
         {{\n{envelope_shape}\n\
         }}\n\
         ```\n\n\
         Required payload fields:\n\
         {fields}\n\n\
         No unknown payload fields are accepted.",
        stage = stage_slug(stage),
        schema_version = schema.schema_version,
    )
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
    validate_stage_payload(&stage, expected_schema_version, &envelope.payload)?;

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
    schema_version: &'static str,
    payload: &JsonValue,
) -> Result<(), StageResultValidationError> {
    let Some(payload_object) = payload.as_object() else {
        return Err(StageResultValidationError::InvalidPayloadField {
            stage: stage.clone(),
            field: "payload",
            reason: "must be an object",
        });
    };

    let required_fields = required_payload_fields(stage);
    validate_allowed_payload_fields(stage, schema_version, payload_object, required_fields)?;

    for &required_field in required_fields {
        validate_required_payload_field(stage, payload_object, required_field)?;
    }

    Ok(())
}

fn required_payload_fields(stage: &GithubIssueStage) -> &'static [StagePayloadFieldRequirement] {
    match stage {
        GithubIssueStage::Triage => &[
            StagePayloadFieldRequirement {
                name: "is_reproducible",
                kind: StagePayloadFieldKind::Bool,
            },
            StagePayloadFieldRequirement {
                name: "suspected_area",
                kind: StagePayloadFieldKind::String,
            },
            StagePayloadFieldRequirement {
                name: "risk",
                kind: StagePayloadFieldKind::String,
            },
            StagePayloadFieldRequirement {
                name: "recommended_next_stage",
                kind: StagePayloadFieldKind::String,
            },
        ],
        GithubIssueStage::Planning => &[
            StagePayloadFieldRequirement {
                name: "plan_items",
                kind: StagePayloadFieldKind::Array,
            },
            StagePayloadFieldRequirement {
                name: "files_to_inspect_or_change",
                kind: StagePayloadFieldKind::Array,
            },
            StagePayloadFieldRequirement {
                name: "test_strategy",
                kind: StagePayloadFieldKind::String,
            },
            StagePayloadFieldRequirement {
                name: "confidence",
                kind: StagePayloadFieldKind::Number,
            },
        ],
        GithubIssueStage::Implementation => &[
            StagePayloadFieldRequirement {
                name: "changed_files",
                kind: StagePayloadFieldKind::Array,
            },
            StagePayloadFieldRequirement {
                name: "commands_run",
                kind: StagePayloadFieldKind::Array,
            },
            StagePayloadFieldRequirement {
                name: "test_evidence",
                kind: StagePayloadFieldKind::Array,
            },
            StagePayloadFieldRequirement {
                name: "pr_ready",
                kind: StagePayloadFieldKind::Bool,
            },
        ],
        GithubIssueStage::PrSynthesis => &[
            StagePayloadFieldRequirement {
                name: "title",
                kind: StagePayloadFieldKind::String,
            },
            StagePayloadFieldRequirement {
                name: "body",
                kind: StagePayloadFieldKind::String,
            },
            StagePayloadFieldRequirement {
                name: "branch_name",
                kind: StagePayloadFieldKind::String,
            },
            StagePayloadFieldRequirement {
                name: "base_branch",
                kind: StagePayloadFieldKind::String,
            },
            StagePayloadFieldRequirement {
                name: "head_sha",
                kind: StagePayloadFieldKind::String,
            },
        ],
        GithubIssueStage::CiRepair => &[
            StagePayloadFieldRequirement {
                name: "failing_checks",
                kind: StagePayloadFieldKind::Array,
            },
            StagePayloadFieldRequirement {
                name: "diagnosis",
                kind: StagePayloadFieldKind::String,
            },
            StagePayloadFieldRequirement {
                name: "changed_files",
                kind: StagePayloadFieldKind::Array,
            },
            StagePayloadFieldRequirement {
                name: "commands_run",
                kind: StagePayloadFieldKind::Array,
            },
        ],
        GithubIssueStage::ReviewResponse => &[
            StagePayloadFieldRequirement {
                name: "addressed_comments",
                kind: StagePayloadFieldKind::Array,
            },
            StagePayloadFieldRequirement {
                name: "remaining_comments",
                kind: StagePayloadFieldKind::Array,
            },
            StagePayloadFieldRequirement {
                name: "commands_run",
                kind: StagePayloadFieldKind::Array,
            },
        ],
    }
}

fn render_payload_field_list(fields: &[StagePayloadFieldRequirement]) -> String {
    fields
        .iter()
        .map(|field| format!("- `{}`: {}", field.name, field.kind.schema_description()))
        .collect::<Vec<_>>()
        .join("\n")
}

fn render_payload_shape(fields: &[StagePayloadFieldRequirement]) -> String {
    fields
        .iter()
        .map(|field| {
            format!(
                "    \"{}\": \"{}\"",
                field.name,
                field.kind.schema_description()
            )
        })
        .collect::<Vec<_>>()
        .join(",\n")
}

fn render_stage_result_envelope_shape(payload_shape: &str) -> String {
    let outcome_values = STAGE_RESULT_OUTCOME_VALUES.join(" | ");
    let evidence_shape = render_evidence_shape();

    STAGE_RESULT_ENVELOPE_FIELDS
        .iter()
        .map(|field| match field.kind {
            StageResultContractFieldKind::Outcome => {
                format!("  \"{}\": \"{}\"", field.name, outcome_values)
            }
            StageResultContractFieldKind::EvidenceArray => {
                format!("  \"{}\": [{}]", field.name, evidence_shape)
            }
            StageResultContractFieldKind::StringArray => {
                format!("  \"{}\": [\"string\"]", field.name)
            }
            StageResultContractFieldKind::PayloadObject => {
                format!("  \"{}\": {{\n{payload_shape}\n  }}", field.name)
            }
            _ => format!(
                "  \"{}\": \"{}\"",
                field.name,
                field.kind.schema_description()
            ),
        })
        .collect::<Vec<_>>()
        .join(",\n")
}

fn render_evidence_shape() -> String {
    let fields = STAGE_RESULT_EVIDENCE_FIELDS
        .iter()
        .map(|field| {
            format!(
                "\"{}\": \"{}\"",
                field.name,
                field.kind.schema_description()
            )
        })
        .collect::<Vec<_>>()
        .join(", ");

    format!("{{{fields}}}")
}

fn validate_allowed_payload_fields(
    stage: &GithubIssueStage,
    schema_version: &'static str,
    payload_object: &serde_json::Map<String, JsonValue>,
    allowed_fields: &[StagePayloadFieldRequirement],
) -> Result<(), StageResultValidationError> {
    for field in payload_object.keys() {
        if !allowed_fields
            .iter()
            .any(|allowed_field| allowed_field.name == field)
        {
            return Err(StageResultValidationError::UnknownPayloadField {
                stage: stage.clone(),
                schema_version,
                field: field.clone(),
            });
        }
    }

    Ok(())
}

fn validate_required_payload_field(
    stage: &GithubIssueStage,
    payload_object: &serde_json::Map<String, JsonValue>,
    required_field: StagePayloadFieldRequirement,
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

fn field_matches_kind(value: &JsonValue, kind: StagePayloadFieldKind) -> bool {
    match kind {
        StagePayloadFieldKind::Array => value.is_array(),
        StagePayloadFieldKind::Bool => value.is_boolean(),
        StagePayloadFieldKind::Number => value.is_number(),
        StagePayloadFieldKind::String => value
            .as_str()
            .map(|string| !string.trim().is_empty())
            .unwrap_or(false),
    }
}

impl StagePayloadFieldKind {
    pub fn expected_description(self) -> &'static str {
        match self {
            StagePayloadFieldKind::Array => "must be an array",
            StagePayloadFieldKind::Bool => "must be a boolean",
            StagePayloadFieldKind::Number => "must be a number",
            StagePayloadFieldKind::String => "must be a non-empty string",
        }
    }

    pub fn schema_description(self) -> &'static str {
        match self {
            StagePayloadFieldKind::Array => "array",
            StagePayloadFieldKind::Bool => "boolean",
            StagePayloadFieldKind::Number => "number",
            StagePayloadFieldKind::String => "non-empty string",
        }
    }
}

impl StageResultContractFieldKind {
    fn schema_description(self) -> &'static str {
        match self {
            StageResultContractFieldKind::Outcome => "stage outcome",
            StageResultContractFieldKind::NonEmptyString => "non-empty string",
            StageResultContractFieldKind::EvidenceArray => "evidence array",
            StageResultContractFieldKind::StringArray => "string array",
            StageResultContractFieldKind::PayloadObject => "payload object",
            StageResultContractFieldKind::OptionalJson => "optional JSON",
        }
    }
}
