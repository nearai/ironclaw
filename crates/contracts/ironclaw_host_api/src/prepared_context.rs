//! Prepared-context (unbound turn) declaration vocabulary.
//!
//! Per-run declarations ride the prepared-context accept door
//! (`ironclaw_threads::SessionThreadService::accept_prepared_context`), are
//! journaled beside the seeded thread content, and are read back at
//! admission to derive the unbound run profile
//! (docs/internal/design/2026-08-12-unbound-turns.md §4.2/§4.5). This module
//! is vocabulary only — neutral turn-tier types plus the read-side port the
//! coordinator consults; storage lives in `ironclaw_threads`, enforcement in
//! the loop tier.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::turn::{AcceptedMessageRef, TurnActor, TurnScope};
use crate::{ids::CapabilityId, output::OutputContract};

/// Capability id of the synthetic, host-owned result tool a
/// `unbound_structured` run finishes by calling. NOT a capability in the
/// authorization/dispatch sense — it never crosses either (precedent:
/// `capability_info`); the id exists so the loop family's stop condition and
/// the host's synthetic surface agree on one identity.
pub const STRUCTURED_RESULT_CAPABILITY_ID: &str = "builtin.structured_result";

/// Provider-facing tool name for the synthetic result tool.
pub const STRUCTURED_RESULT_PROVIDER_TOOL_NAME: &str = "builtin__structured_result";

/// Per-run limits, narrowing-only against the resolved profile's ceilings.
///
/// The fields map onto the per-run budget seams
/// (`ResourceBudgetPolicy::{max_model_calls, max_capability_invocations,
/// max_wall_clock_seconds}`), all enforced by the loop executor's budget
/// stage.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnLimits {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_model_calls: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_capability_invocations: Option<u32>,
    /// Per-run wall-clock ceiling in seconds. Narrows the profile ceiling;
    /// a declared value can only shorten the run, never extend it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_wall_clock_seconds: Option<u32>,
}

impl TurnLimits {
    pub fn is_unlimited(&self) -> bool {
        self.max_model_calls.is_none()
            && self.max_capability_invocations.is_none()
            && self.max_wall_clock_seconds.is_none()
    }
}

/// The journaled per-run declarations: tool selection (a *selection*, never
/// authority — every call is still authorized at action time), the output
/// contract, and narrowing-only limits.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreparedTurnDeclarations {
    /// Visible-surface selection, validated as a subset of the profile
    /// surface. Empty means "keep the resolved profile's surface" — a
    /// non-empty value intersects it. (It does NOT mean "no tools": the
    /// runner only applies the intersection when this is non-empty.)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<CapabilityId>,
    #[serde(default)]
    pub output: OutputContract,
    #[serde(default, skip_serializing_if = "TurnLimits::is_unlimited")]
    pub limits: TurnLimits,
    /// No human is present for this run: drop capabilities whose
    /// authorization resolves to `RequireApproval`, because an approval gate
    /// with nobody to answer it is a run that parks rather than one that asks.
    ///
    /// Narrowing-only, like [`TurnLimits`]: `false` (the default) changes
    /// nothing, so persisted declarations and other unbound callers are
    /// unaffected.
    ///
    /// This flag does NOT restrict effects. A run that must not cause side
    /// effects is governed by its prompt, not by this field — note that the
    /// approval hard floor is only `Financial`/`ModifyApproval`/`ModifyBudget`
    /// and global auto-approve defaults on, so "needs no approval" admits
    /// write-effect capabilities including shell and outbound delivery.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub require_no_approval: bool,
}

/// Read-side failure for the declarations probe. Unbound admission fails
/// closed on `Unavailable`; explicit ordinary profiles and scheduled triggers
/// may continue without declarations because the probe is optional on those
/// trusted profile paths.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PreparedContextReadError {
    #[error("prepared-context declarations are unavailable: {reason}")]
    Unavailable { reason: String },
}

/// Admission-side probe for prepared context declarations.
///
/// Implemented over the threads-tier prepared-context record and wired in
/// composition; the coordinator consults it when a submission carries no
/// binding refs to derive the unbound run profile from what the accepted
/// ref points at. `Ok(None)` means the ref is not a prepared context. Unbound
/// admission rejects that result fail-closed for ref-less submissions; trusted
/// profile paths may continue with their profile's defaults.
#[async_trait]
pub trait PreparedContextSource: Send + Sync {
    async fn read_declarations(
        &self,
        scope: &TurnScope,
        actor: &TurnActor,
        accepted_message_ref: &AcceptedMessageRef,
    ) -> Result<Option<PreparedTurnDeclarations>, PreparedContextReadError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_contract_serde_is_tagged_snake_case_and_defaults_to_assistant_message() {
        let assistant = serde_json::to_value(OutputContract::AssistantMessage).expect("serialize");
        assert_eq!(
            assistant,
            serde_json::json!({ "kind": "assistant_message" })
        );

        let schema = OutputContract::JsonSchema {
            name: "cards_v1".to_string(),
            schema: serde_json::json!({ "type": "object" }),
        };
        let json = serde_json::to_value(&schema).expect("serialize");
        assert_eq!(json["kind"], "json_schema");
        let back: OutputContract = serde_json::from_value(json).expect("deserialize");
        assert_eq!(back, schema);
        assert!(back.is_json_schema());
    }

    #[test]
    fn declarations_default_to_no_tools_assistant_message_unlimited() {
        let declarations: PreparedTurnDeclarations =
            serde_json::from_value(serde_json::json!({})).expect("empty object deserializes");
        assert!(declarations.tools.is_empty());
        assert_eq!(declarations.output, OutputContract::AssistantMessage);
        assert!(declarations.limits.is_unlimited());
    }

    #[test]
    fn declarations_round_trip_with_schema_and_limits() {
        let declarations = PreparedTurnDeclarations {
            tools: vec![CapabilityId::new("builtin.memory_search").expect("capability id")],
            output: OutputContract::JsonSchema {
                name: "suggestions".to_string(),
                schema: serde_json::json!({
                    "type": "object",
                    "properties": { "cards": { "type": "array" } },
                    "required": ["cards"]
                }),
            },
            limits: TurnLimits {
                max_model_calls: Some(6),
                max_capability_invocations: Some(12),
                max_wall_clock_seconds: Some(120),
            },
            require_no_approval: true,
        };
        let json = serde_json::to_value(&declarations).expect("serialize");
        let back: PreparedTurnDeclarations = serde_json::from_value(json).expect("deserialize");
        assert_eq!(back, declarations);
    }

    /// Declarations persisted before `require_no_approval` existed must still load,
    /// and must load as permissive — the field is narrowing-only, so an absent
    /// value cannot silently restrict an already-running caller's surface.
    #[test]
    fn declarations_without_require_no_approval_load_as_permissive() {
        let legacy = serde_json::json!({
            "tools": ["builtin.memory_search"],
            "output": serde_json::Value::Null,
        });
        let mut legacy = legacy;
        legacy["output"] = serde_json::to_value(OutputContract::default()).expect("output");

        let decoded: PreparedTurnDeclarations =
            serde_json::from_value(legacy).expect("legacy declarations deserialize");

        assert!(!decoded.require_no_approval);
    }

    /// The permissive default is omitted from the wire form, so adding the
    /// field does not rewrite every existing record's serialization.
    #[test]
    fn permissive_require_no_approval_is_omitted_from_the_wire_form() {
        let declarations = PreparedTurnDeclarations::default();

        let json = serde_json::to_value(&declarations).expect("serialize");

        assert!(json.get("require_no_approval").is_none());
    }
}
