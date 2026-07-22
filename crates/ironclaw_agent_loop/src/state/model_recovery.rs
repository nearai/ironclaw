use ironclaw_turns::ModelInvalidOutputDetailReason;

use super::ModelErrorObservationClass;

const MODEL_ERROR_OBSERVATION_SCHEMA_VERSION: u32 = 1;

/// Typed, host-authored recovery context carried to the next model request.
///
/// This state is loop-local and checkpointed so a worker restart cannot lose
/// a consumed recovery budget or rebuild the retry without its safe control
/// message. Provider text and diagnostics are deliberately excluded.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ModelErrorRecoveryObservation {
    #[serde(default = "current_schema_version")]
    pub(crate) schema_version: u32,
    detail: ModelErrorRecoveryDetail,
}

impl ModelErrorRecoveryObservation {
    pub fn context_overflow() -> Self {
        Self {
            schema_version: MODEL_ERROR_OBSERVATION_SCHEMA_VERSION,
            detail: ModelErrorRecoveryDetail::ContextOverflow,
        }
    }

    pub fn content_filtered() -> Self {
        Self {
            schema_version: MODEL_ERROR_OBSERVATION_SCHEMA_VERSION,
            detail: ModelErrorRecoveryDetail::ContentFiltered,
        }
    }

    pub fn invalid_output(reason: Option<ModelInvalidOutputDetailReason>) -> Self {
        Self {
            schema_version: MODEL_ERROR_OBSERVATION_SCHEMA_VERSION,
            detail: ModelErrorRecoveryDetail::InvalidOutput { reason },
        }
    }

    pub fn class(&self) -> ModelErrorObservationClass {
        match self.detail {
            ModelErrorRecoveryDetail::ContextOverflow => {
                ModelErrorObservationClass::ContextOverflow
            }
            ModelErrorRecoveryDetail::ContentFiltered => {
                ModelErrorObservationClass::ContentFiltered
            }
            ModelErrorRecoveryDetail::InvalidOutput { .. } => {
                ModelErrorObservationClass::InvalidOutput
            }
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != MODEL_ERROR_OBSERVATION_SCHEMA_VERSION {
            return Err(format!(
                "model error observation schema version {} is unsupported",
                self.schema_version
            ));
        }
        Ok(())
    }

    /// Deterministic control text for the next model request. The text reports
    /// only the typed failure class; it never includes provider-supplied text.
    pub fn model_instruction(&self) -> String {
        match self.detail {
            ModelErrorRecoveryDetail::ContextOverflow => {
                "model error observation: context overflowed; use the available context and continue"
                    .to_string()
            }
            ModelErrorRecoveryDetail::ContentFiltered => {
                "model error observation: completion refused by content filter; provide a policy compliant alternative without reproducing blocked content"
                    .to_string()
            }
            ModelErrorRecoveryDetail::InvalidOutput { reason } => {
                let reason = reason.map_or("unspecified", ModelInvalidOutputDetailReason::as_str);
                format!(
                    "model error observation: invalid_output reason={reason}; repair the response and continue"
                )
            }
        }
    }
}

fn current_schema_version() -> u32 {
    MODEL_ERROR_OBSERVATION_SCHEMA_VERSION
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum ModelErrorRecoveryDetail {
    ContextOverflow,
    ContentFiltered,
    InvalidOutput {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<ModelInvalidOutputDetailReason>,
    },
}

/// Prompt-shape control that must survive the retry-transition checkpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PendingModelRetryDirective {
    RepairInvalidOutput,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observation_serializes_typed_invalid_output_reason() {
        let observation = ModelErrorRecoveryObservation::invalid_output(Some(
            ModelInvalidOutputDetailReason::EmptyAssistantResponse,
        ));

        observation.validate().expect("observation validates");
        let value = serde_json::to_value(&observation).expect("serialize");

        assert_eq!(value["schema_version"], serde_json::json!(1));
        assert_eq!(value["detail"]["kind"], "invalid_output");
        assert_eq!(
            value["detail"]["reason"],
            serde_json::json!("empty_assistant_response")
        );
        assert_eq!(
            observation.class(),
            ModelErrorObservationClass::InvalidOutput
        );
        assert!(
            observation
                .model_instruction()
                .contains("reason=empty_assistant_response")
        );
    }

    #[test]
    fn observation_rejects_unknown_schema_version() {
        let mut observation = ModelErrorRecoveryObservation::content_filtered();
        observation.schema_version += 1;

        assert!(observation.validate().is_err());
    }
}
