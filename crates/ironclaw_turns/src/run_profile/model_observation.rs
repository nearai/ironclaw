use ironclaw_host_api::DispatchInputIssueCode;
use serde::{Deserialize, Serialize};

use super::host::CapabilityFailureKind;

const MODEL_OBSERVATION_SUMMARY_MAX_BYTES: usize = 512;
const MODEL_OBSERVATION_ARTIFACTS_MAX: usize = 16;
const MODEL_OBSERVATION_REPAIRS_MAX: usize = 16;
const MODEL_OBSERVATION_INPUT_ISSUES_MAX: usize = 16;
const MODEL_OBSERVATION_TEXT_MAX_BYTES: usize = 512;
pub const MODEL_VISIBLE_TOOL_OBSERVATION_SCHEMA_VERSION: u32 = 1;

/// Maximum size of a model-visible free-text diagnostic. Larger than the
/// summary cap because the diagnostic carries raw (secret-scrubbed) error text.
pub const MODEL_OBSERVATION_DETAIL_MAX_BYTES: usize = 4096;

#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CapabilityFailureDetail {
    InvalidInput {
        issues: Vec<CapabilityInputIssue>,
    },
    /// Free-text, secret-scrubbed raw cause carried to the model. Allows path
    /// and payload delimiters (`/ { } [ ] < >`) that the strict summary
    /// validator rejects — the producer redacts secret VALUES instead.
    Diagnostic {
        text: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelVisibleToolObservation {
    #[serde(default = "current_model_visible_tool_observation_schema_version")]
    pub schema_version: u32,
    pub status: ToolObservationStatus,
    pub summary: String,
    pub detail: ToolObservationDetail,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<ModelVisibleArtifact>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery: Option<ToolRecoveryObservation>,
    pub trust: ObservationTrust,
}

impl ModelVisibleToolObservation {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != MODEL_VISIBLE_TOOL_OBSERVATION_SCHEMA_VERSION {
            return Err(format!(
                "model observation schema version {} is unsupported",
                self.schema_version
            ));
        }
        validate_non_empty_text(&self.summary, "model observation summary")?;
        validate_text_len(
            &self.summary,
            "model observation summary",
            MODEL_OBSERVATION_SUMMARY_MAX_BYTES,
        )?;
        self.detail.validate()?;
        validate_len(
            self.artifacts.len(),
            MODEL_OBSERVATION_ARTIFACTS_MAX,
            "model observation artifacts",
        )?;
        for artifact in &self.artifacts {
            artifact.validate()?;
        }
        if let Some(recovery) = &self.recovery {
            recovery.validate()?;
        }
        Ok(())
    }
}

fn current_model_visible_tool_observation_schema_version() -> u32 {
    MODEL_VISIBLE_TOOL_OBSERVATION_SCHEMA_VERSION
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
    InvalidInput {
        issues: Vec<CapabilityInputIssue>,
    },
    GenericFailure {
        failure_kind: CapabilityFailureKind,
        /// Bounded, secret-scrubbed raw cause shown to the model alongside the
        /// fixed-template summary. Validated leniently — path and payload
        /// delimiters are allowed; only NUL/control chars and length are
        /// rejected. The producer is responsible for redacting secret VALUES.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
    },
    ResultReference {
        result_ref: String,
        byte_len: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        preview: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        total_bytes: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        next_offset: Option<u64>,
    },
}

impl ToolObservationDetail {
    fn validate(&self) -> Result<(), String> {
        match self {
            Self::InvalidInput { issues } => {
                validate_len(
                    issues.len(),
                    MODEL_OBSERVATION_INPUT_ISSUES_MAX,
                    "model observation input issues",
                )?;
                for issue in issues {
                    issue.validate()?;
                }
                Ok(())
            }
            Self::GenericFailure { detail, .. } => {
                if let Some(detail) = detail {
                    validate_model_observation_detail(detail)?;
                }
                Ok(())
            }
            Self::ResultReference { result_ref, .. } => {
                // `preview` is intentionally NOT content-checked here: this
                // neutral gate has no graceful-degrade path, so an unsafe
                // preview would drop the whole observation (losing
                // `result_ref` too) instead of falling back to ref-only.
                // `ironclaw_threads::ToolResultReferenceEnvelope::new` owns
                // that canonical secret/control-char scan and degrades
                // correctly; this arm only bounds shape (issue #5838).
                validate_non_empty_text(result_ref, "model observation result ref")?;
                validate_text_len(
                    result_ref,
                    "model observation result ref",
                    MODEL_OBSERVATION_TEXT_MAX_BYTES,
                )
            }
        }
    }
}

/// Lenient validation for the model-visible free-text diagnostic channel.
///
/// Unlike the strict safe-summary validator, this ALLOWS path and payload
/// delimiters (`/ { } [ ] < >`) so the model can see the real cause (paths,
/// schema refs, codes). It only rejects NUL/disallowed control characters and
/// caps length. Secret VALUE redaction is the producer's responsibility.
pub fn validate_model_observation_detail(value: &str) -> Result<(), String> {
    if value.is_empty() {
        return Err("model observation detail must not be empty".to_string());
    }
    if value.len() > MODEL_OBSERVATION_DETAIL_MAX_BYTES {
        return Err(format!(
            "model observation detail exceeds {MODEL_OBSERVATION_DETAIL_MAX_BYTES} bytes"
        ));
    }
    if value.chars().any(is_disallowed_control_character) {
        return Err("model observation detail must not contain NUL/control characters".to_string());
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelVisibleArtifact {
    pub artifact_ref: String,
    pub summary: String,
}

impl ModelVisibleArtifact {
    fn validate(&self) -> Result<(), String> {
        validate_non_empty_text(&self.artifact_ref, "model observation artifact ref")?;
        validate_text_len(
            &self.artifact_ref,
            "model observation artifact ref",
            MODEL_OBSERVATION_TEXT_MAX_BYTES,
        )?;
        validate_non_empty_text(&self.summary, "model observation artifact summary")?;
        validate_text_len(
            &self.summary,
            "model observation artifact summary",
            MODEL_OBSERVATION_TEXT_MAX_BYTES,
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolRecoveryObservation {
    pub same_call_retry: SameCallRetryConstraint,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub repairs: Vec<CapabilityInputRepair>,
    pub recovery_hint: CapabilityRecoveryHint,
}

impl ToolRecoveryObservation {
    fn validate(&self) -> Result<(), String> {
        validate_len(
            self.repairs.len(),
            MODEL_OBSERVATION_REPAIRS_MAX,
            "model observation repairs",
        )?;
        for repair in &self.repairs {
            repair.validate()?;
        }
        Ok(())
    }
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

impl CapabilityInputRepair {
    fn validate(&self) -> Result<(), String> {
        match self {
            Self::ProvideRequiredField { path }
            | Self::RemoveUnexpectedField { path }
            | Self::UseAllowedValue { path } => {
                validate_non_empty_text(path, "model observation repair path")?;
                validate_text_len(
                    path,
                    "model observation repair path",
                    MODEL_OBSERVATION_TEXT_MAX_BYTES,
                )
            }
            Self::ChangeType { path, expected } => {
                validate_non_empty_text(path, "model observation repair path")?;
                validate_text_len(
                    path,
                    "model observation repair path",
                    MODEL_OBSERVATION_TEXT_MAX_BYTES,
                )?;
                validate_optional_text(expected.as_deref(), "model observation repair expected")
            }
        }
    }
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
    RespectFailureConstraint,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationTrust {
    UntrustedToolOutput,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityInputIssue {
    pub path: String,
    pub code: DispatchInputIssueCode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub received: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_path: Option<String>,
}

impl CapabilityInputIssue {
    fn validate(&self) -> Result<(), String> {
        validate_non_empty_text(&self.path, "model observation issue path")?;
        validate_text_len(
            &self.path,
            "model observation issue path",
            MODEL_OBSERVATION_TEXT_MAX_BYTES,
        )?;
        validate_optional_text(self.expected.as_deref(), "model observation issue expected")?;
        validate_optional_text(self.received.as_deref(), "model observation issue received")?;
        validate_optional_text(
            self.schema_path.as_deref(),
            "model observation issue schema path",
        )
    }
}

fn validate_len(len: usize, max: usize, label: &'static str) -> Result<(), String> {
    if len > max {
        return Err(format!("{label} exceeds maximum item count {max}"));
    }
    Ok(())
}

fn validate_optional_text(value: Option<&str>, label: &'static str) -> Result<(), String> {
    if let Some(value) = value {
        validate_text_len(value, label, MODEL_OBSERVATION_TEXT_MAX_BYTES)?;
    }
    Ok(())
}

fn validate_non_empty_text(value: &str, label: &'static str) -> Result<(), String> {
    if value.is_empty() {
        return Err(format!("{label} must not be empty"));
    }
    Ok(())
}

fn validate_text_len(value: &str, label: &'static str, max: usize) -> Result<(), String> {
    if value.len() > max {
        return Err(format!("{label} exceeds {max} bytes"));
    }
    if value.chars().any(is_disallowed_control_character) {
        return Err(format!("{label} must not contain NUL/control characters"));
    }
    Ok(())
}

fn is_disallowed_control_character(character: char) -> bool {
    character == '\0' || character.is_control() && !matches!(character, '\n' | '\r' | '\t')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_visible_tool_observation_serializes_typed_recovery() {
        let observation = ModelVisibleToolObservation {
            schema_version: MODEL_VISIBLE_TOOL_OBSERVATION_SCHEMA_VERSION,
            status: ToolObservationStatus::Error,
            summary: "Tool input failed schema validation.".to_string(),
            detail: ToolObservationDetail::InvalidInput {
                issues: vec![CapabilityInputIssue {
                    path: "file_path".to_string(),
                    code: DispatchInputIssueCode::MissingRequired,
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
        assert_eq!(value["schema_version"], serde_json::json!(1));
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

    #[test]
    fn generic_failure_detail_allows_paths_and_payload_delimiters() {
        let path = "missing input_schema_ref at /system/extensions/google-calendar/schemas/google-calendar/list_calendars.input.v1.json";
        let observation = ModelVisibleToolObservation {
            schema_version: MODEL_VISIBLE_TOOL_OBSERVATION_SCHEMA_VERSION,
            status: ToolObservationStatus::Error,
            summary: "Capability failed with missing_runtime.".to_string(),
            detail: ToolObservationDetail::GenericFailure {
                failure_kind: CapabilityFailureKind::MissingRuntime,
                detail: Some(path.to_string()),
            },
            artifacts: Vec::new(),
            recovery: None,
            trust: ObservationTrust::UntrustedToolOutput,
        };

        observation
            .validate()
            .expect("path-bearing diagnostic detail must validate");

        let value = serde_json::to_value(&observation).expect("serialize");
        assert_eq!(value["detail"]["detail"], serde_json::json!(path));
    }

    #[test]
    fn validate_model_observation_detail_rejects_control_chars() {
        validate_model_observation_detail("clean /path/ok").expect("ordinary text ok");
        validate_model_observation_detail("bad\u{0}null").expect_err("NUL must be rejected");
        validate_model_observation_detail("").expect_err("empty must be rejected");
    }

    #[test]
    fn generic_failure_deserializes_legacy_json_without_detail() {
        // Legacy wire payloads predate the `detail` field; they must still
        // deserialize (defaulting `detail` to None).
        let legacy = serde_json::json!({
            "kind": "generic_failure",
            "failure_kind": "backend"
        });
        let detail: ToolObservationDetail =
            serde_json::from_value(legacy).expect("legacy generic_failure deserializes");
        assert!(matches!(
            detail,
            ToolObservationDetail::GenericFailure {
                failure_kind: CapabilityFailureKind::Backend,
                detail: None
            }
        ));
    }

    #[test]
    fn capability_failure_detail_diagnostic_round_trips() {
        let detail = CapabilityFailureDetail::Diagnostic {
            text: "missing input_schema_ref at /system/extensions/x.json".to_string(),
        };
        let value = serde_json::to_value(&detail).expect("serialize");
        assert_eq!(value["kind"], "diagnostic");
        assert_eq!(
            value["text"],
            serde_json::json!("missing input_schema_ref at /system/extensions/x.json")
        );
        let back: CapabilityFailureDetail = serde_json::from_value(value).expect("deserialize");
        assert_eq!(back, detail);
    }
}
