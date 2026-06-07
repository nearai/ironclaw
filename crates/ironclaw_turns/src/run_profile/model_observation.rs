use serde::{Deserialize, Serialize};

use super::host::CapabilityFailureKind;

#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CapabilityFailureDetail {
    InvalidInput { issues: Vec<CapabilityInputIssue> },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelVisibleToolObservation {
    pub status: ToolObservationStatus,
    pub summary: String,
    pub detail: ToolObservationDetail,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<ModelVisibleArtifact>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery: Option<ToolRecoveryObservation>,
    pub trust: ObservationTrust,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolObservationStatus {
    Success,
    Error,
}

#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ToolObservationDetail {
    InvalidInput { issues: Vec<CapabilityInputIssue> },
    GenericFailure { failure_kind: CapabilityFailureKind },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelVisibleArtifact {
    pub artifact_ref: String,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolRecoveryObservation {
    pub same_call_retry: SameCallRetryConstraint,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub repairs: Vec<CapabilityInputRepair>,
    pub recovery_hint: CapabilityRecoveryHint,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CapabilityInputRepair {
    ProvideRequiredField {
        path: String,
    },
    RemoveUnexpectedField {
        path: String,
    },
    ChangeType {
        path: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expected: Option<String>,
    },
    UseAllowedValue {
        path: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SameCallRetryConstraint {
    Allowed,
    AllowedAfterDelay,
    RequiresChangedInput,
    NotUseful,
    Forbidden,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityRecoveryHint {
    CorrectArgumentsBeforeRetry,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationTrust {
    UntrustedToolOutput,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityInputIssue {
    pub path: String,
    pub code: CapabilityInputIssueCode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub received: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_path: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityInputIssueCode {
    MissingRequired,
    UnexpectedField,
    TypeMismatch,
    InvalidValue,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_visible_tool_observation_serializes_typed_recovery() {
        let observation = ModelVisibleToolObservation {
            status: ToolObservationStatus::Error,
            summary: "Tool input failed schema validation.".to_string(),
            detail: ToolObservationDetail::InvalidInput {
                issues: vec![CapabilityInputIssue {
                    path: "file_path".to_string(),
                    code: CapabilityInputIssueCode::MissingRequired,
                    expected: Some("required field".to_string()),
                    received: None,
                    schema_path: Some("required".to_string()),
                }],
            },
            artifacts: Vec::new(),
            recovery: Some(ToolRecoveryObservation {
                same_call_retry: SameCallRetryConstraint::RequiresChangedInput,
                repairs: vec![CapabilityInputRepair::ProvideRequiredField {
                    path: "file_path".to_string(),
                }],
                recovery_hint: CapabilityRecoveryHint::CorrectArgumentsBeforeRetry,
            }),
            trust: ObservationTrust::UntrustedToolOutput,
        };

        let value = serde_json::to_value(&observation).expect("serialize");

        assert_eq!(value["status"], "error");
        assert_eq!(value["detail"]["kind"], "invalid_input");
        assert_eq!(
            value["detail"]["issues"][0]["code"],
            serde_json::json!("missing_required")
        );
        assert_eq!(
            value["recovery"]["same_call_retry"],
            serde_json::json!("requires_changed_input")
        );
        assert_eq!(
            value["recovery"]["repairs"][0]["kind"],
            serde_json::json!("provide_required_field")
        );
        assert_eq!(value["trust"], "untrusted_tool_output");
    }
}
