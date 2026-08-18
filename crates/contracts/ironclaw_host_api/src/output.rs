//! Product- and loop-family-neutral terminal output contracts.
//!
//! An [`OutputContract`] is an immutable declaration carried by one accepted
//! turn.  It describes the terminal representation the host must produce; it
//! does not select a loop family, alter a profile, or grant any capability.
//! Keeping this vocabulary separate from prepared-context declarations lets
//! bound and unbound turns use the same contract.

use serde::{Deserialize, Serialize};

use crate::error::HostApiError;

/// Stable compatibility name used when rehydrating legacy schema contracts
/// that predate named response formats.
pub const DEFAULT_JSON_SCHEMA_OUTPUT_NAME: &str = "ironclaw_output";
/// Provider response-format names are deliberately bounded before they cross
/// into any adapter or cache key.
pub const MAX_JSON_SCHEMA_OUTPUT_NAME_BYTES: usize = 64;

/// The terminal representation requested for a run.
///
/// The default is deliberately the historical assistant-message behavior so
/// old submissions, process snapshots, and run records remain readable.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OutputContract {
    /// The ordinary assistant-message terminal output.
    #[default]
    AssistantMessage,
    /// The terminal output must be a JSON value shaped by this schema.
    JsonSchema {
        /// Stable provider/cache identity for this response format.
        #[serde(default = "default_json_schema_output_name")]
        name: String,
        schema: serde_json::Value,
    },
    /// The terminal output must be a JSON object, without a caller-supplied
    /// schema. Providers must preserve this native response mode rather than
    /// representing it as a synthetic schema.
    JsonObject,
}

impl OutputContract {
    pub fn is_assistant_message(&self) -> bool {
        matches!(self, Self::AssistantMessage)
    }

    pub fn is_json_schema(&self) -> bool {
        matches!(self, Self::JsonSchema { .. })
    }

    pub fn is_json_object(&self) -> bool {
        matches!(self, Self::JsonObject)
    }

    pub fn is_structured_output(&self) -> bool {
        self.is_json_schema() || self.is_json_object()
    }

    pub fn json_schema(schema: serde_json::Value) -> Self {
        Self::JsonSchema {
            name: DEFAULT_JSON_SCHEMA_OUTPUT_NAME.to_string(),
            schema,
        }
    }

    pub fn try_json_schema(
        name: impl Into<String>,
        schema: serde_json::Value,
    ) -> Result<Self, HostApiError> {
        let contract = Self::JsonSchema {
            name: name.into(),
            schema,
        };
        contract.validate()?;
        Ok(contract)
    }

    pub fn schema_name(&self) -> Option<&str> {
        match self {
            Self::AssistantMessage | Self::JsonObject => None,
            Self::JsonSchema { name, .. } => Some(name.as_str()),
        }
    }

    pub fn schema(&self) -> Option<&serde_json::Value> {
        match self {
            Self::AssistantMessage | Self::JsonObject => None,
            Self::JsonSchema { schema, .. } => Some(schema),
        }
    }

    /// Validate the durable response-format identity at the host boundary.
    /// Schema semantics remain the provider-native authority; this only
    /// constrains the name used by provider request formats and cache keys.
    pub fn validate(&self) -> Result<(), HostApiError> {
        let Self::JsonSchema { name, schema } = self else {
            return Ok(());
        };
        if name.is_empty() || name.len() > MAX_JSON_SCHEMA_OUTPUT_NAME_BYTES {
            return Err(HostApiError::invalid_output_contract(format!(
                "JSON schema output name must be 1..={MAX_JSON_SCHEMA_OUTPUT_NAME_BYTES} bytes"
            )));
        }
        if !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        {
            return Err(HostApiError::invalid_output_contract(
                "JSON schema output name contains an invalid character",
            ));
        }
        if !schema.is_object() {
            return Err(HostApiError::invalid_output_contract(
                "JSON schema output must be a JSON object",
            ));
        }
        Ok(())
    }
}

fn default_json_schema_output_name() -> String {
    DEFAULT_JSON_SCHEMA_OUTPUT_NAME.to_string()
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;

    use super::{
        DEFAULT_JSON_SCHEMA_OUTPUT_NAME, MAX_JSON_SCHEMA_OUTPUT_NAME_BYTES, OutputContract,
    };

    #[test]
    fn legacy_missing_contract_defaults_to_assistant_message() {
        #[derive(Debug, Deserialize)]
        struct LegacyWire {
            #[serde(default)]
            output_contract: OutputContract,
        }
        let wire: LegacyWire = serde_json::from_value(serde_json::json!({}))
            .expect("missing output contract should use the field default");
        assert_eq!(wire.output_contract, OutputContract::AssistantMessage);
    }

    #[test]
    fn schema_contract_round_trips_without_loop_family_information() {
        let contract = OutputContract::JsonSchema {
            name: "answer_v1".to_string(),
            schema: serde_json::json!({
                "type": "object",
                "properties": {"answer": {"type": "string"}},
                "required": ["answer"]
            }),
        };
        let wire = serde_json::to_value(&contract).expect("serialize output contract");
        let restored: OutputContract =
            serde_json::from_value(wire).expect("deserialize output contract");
        assert_eq!(restored, contract);
        assert_eq!(restored.schema_name(), Some("answer_v1"));
    }

    #[test]
    fn legacy_schema_without_name_gets_stable_name() {
        let wire = serde_json::json!({
            "kind": "json_schema",
            "schema": {"type": "object"}
        });
        let restored: OutputContract = serde_json::from_value(wire).expect("legacy schema");
        assert_eq!(
            restored.schema_name(),
            Some(DEFAULT_JSON_SCHEMA_OUTPUT_NAME)
        );
    }

    #[test]
    fn schema_name_validation_is_bounded_and_explicit() {
        assert!(OutputContract::try_json_schema("answer_v1", serde_json::json!({})).is_ok());
        assert!(OutputContract::try_json_schema("", serde_json::json!({})).is_err());
        assert!(OutputContract::try_json_schema("answer.v1", serde_json::json!({})).is_err());
        assert!(OutputContract::try_json_schema("answer/v1", serde_json::json!({})).is_err());
        assert!(
            OutputContract::try_json_schema(
                "a".repeat(MAX_JSON_SCHEMA_OUTPUT_NAME_BYTES + 1),
                serde_json::json!({})
            )
            .is_err()
        );
    }

    #[test]
    fn schema_contract_rejects_non_object_root_schema() {
        for schema in [serde_json::json!("string"), serde_json::json!([])] {
            assert!(
                OutputContract::try_json_schema("answer_v1", schema).is_err(),
                "structured output contracts must have an object root"
            );
        }
    }

    #[test]
    fn json_object_contract_round_trips_without_a_synthetic_schema() {
        let contract = OutputContract::JsonObject;
        let wire = serde_json::to_value(&contract).expect("serialize output contract");
        let restored: OutputContract =
            serde_json::from_value(wire).expect("deserialize output contract");
        assert_eq!(restored, contract);
        assert!(restored.is_structured_output());
        assert_eq!(restored.schema_name(), None);
        assert_eq!(restored.schema(), None);
        assert!(restored.validate().is_ok());
    }
}
