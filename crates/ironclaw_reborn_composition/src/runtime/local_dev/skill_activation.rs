use std::{collections::HashSet, sync::Arc};

use async_trait::async_trait;
use ironclaw_host_api::InvocationId;
use ironclaw_turns::run_profile::{
    AgentLoopHostError, AgentLoopHostErrorKind, CapabilityOutcome, CapabilityResultMessage,
    ConcurrencyHint,
};

use crate::runtime::{
    LocalDevSelectableSkillContextSource,
    local_dev::synthetic_capability::{
        LocalDevSyntheticCapability, LocalDevSyntheticCapabilityDescriptor,
        LocalDevSyntheticCapabilityHandler, LocalDevSyntheticCapabilityInvocation,
    },
};

pub(crate) const SKILL_ACTIVATE_CAPABILITY_ID: &str = "builtin.skill_activate";
const SKILL_ACTIVATE_PROVIDER_TOOL_NAME: &str = "builtin__skill_activate";
const SKILL_ACTIVATE_DESCRIPTION: &str =
    "Activate one or more listed Reborn skills for the current loop run";
const MAX_SKILL_ACTIVATE_NAMES: usize = 16;

pub(super) fn skill_activation_capability(
    skill_activation_source: Arc<LocalDevSelectableSkillContextSource>,
) -> Result<LocalDevSyntheticCapability, AgentLoopHostError> {
    Ok(LocalDevSyntheticCapability::new(
        LocalDevSyntheticCapabilityDescriptor::new(
            SKILL_ACTIVATE_CAPABILITY_ID,
            SKILL_ACTIVATE_PROVIDER_TOOL_NAME,
            SKILL_ACTIVATE_DESCRIPTION,
            ConcurrencyHint::Exclusive,
            skill_activate_input_schema(),
        )?,
        Arc::new(SkillActivationHandler {
            skill_activation_source,
        }),
    ))
}

struct SkillActivationHandler {
    skill_activation_source: Arc<LocalDevSelectableSkillContextSource>,
}

#[async_trait]
impl LocalDevSyntheticCapabilityHandler for SkillActivationHandler {
    fn validate_provider_arguments(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<(), AgentLoopHostError> {
        parse_skill_activate_names(arguments).map(|_| ())
    }

    async fn invoke(
        &self,
        invocation: LocalDevSyntheticCapabilityInvocation,
    ) -> Result<CapabilityOutcome, AgentLoopHostError> {
        let names = parse_skill_activate_names(&invocation.input)?;
        let requested_names = names
            .iter()
            .map(|name| name.to_ascii_lowercase())
            .collect::<HashSet<_>>();
        let plan = self
            .skill_activation_source
            .activate_skills_for_run(&invocation.run_context, &names)
            .await
            .map_err(skill_activation_host_error)?;
        let activated = plan
            .selection
            .activations
            .iter()
            .filter(|activation| requested_names.contains(&activation.name.to_ascii_lowercase()))
            .map(|activation| activation.name.clone())
            .collect::<Vec<_>>();
        let output = serde_json::json!({
            "activated": activated,
            "count": activated.len(),
        });
        let result_ref = invocation
            .result_writer
            .write_capability_result(
                &invocation.run_context,
                &invocation.request.input_ref,
                InvocationId::new(),
                &invocation.request.capability_id,
                output,
            )
            .await?;
        Ok(CapabilityOutcome::Completed(CapabilityResultMessage {
            result_ref,
            safe_summary: format!("activated {} skill(s)", activated.len()),
            terminate_hint: false,
        }))
    }
}

fn skill_activate_input_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "names": {
                "type": "array",
                "items": { "type": "string" },
                "minItems": 1,
                "maxItems": MAX_SKILL_ACTIVATE_NAMES,
                "description": "Skill names from skill_list to activate for this run"
            }
        },
        "required": ["names"],
        "additionalProperties": false
    })
}

fn parse_skill_activate_names(
    input: &serde_json::Value,
) -> Result<Vec<String>, AgentLoopHostError> {
    let names = input
        .get("names")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::InvalidInvocation,
                "skill_activate requires a names array",
            )
        })?;
    let parsed = names
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(|name| name.trim().to_string())
                .filter(|name| !name.is_empty())
                .ok_or_else(|| {
                    AgentLoopHostError::new(
                        AgentLoopHostErrorKind::InvalidInvocation,
                        "skill_activate names must be non-empty strings",
                    )
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    if parsed.is_empty() {
        return Err(AgentLoopHostError::new(
            AgentLoopHostErrorKind::InvalidInvocation,
            "skill_activate requires at least one skill name",
        ));
    }
    if parsed.len() > MAX_SKILL_ACTIVATE_NAMES {
        return Err(AgentLoopHostError::new(
            AgentLoopHostErrorKind::InvalidInvocation,
            "skill_activate accepts at most 16 skill names",
        ));
    }
    Ok(parsed)
}

fn skill_activation_host_error(
    error: ironclaw_first_party_extension_ports::SkillActivationSelectionError,
) -> AgentLoopHostError {
    let kind = match error {
        ironclaw_first_party_extension_ports::SkillActivationSelectionError::AmbiguousSkill {
            ..
        }
        | ironclaw_first_party_extension_ports::SkillActivationSelectionError::ParseFailed
        | ironclaw_first_party_extension_ports::SkillActivationSelectionError::TrustDataMissing
        | ironclaw_first_party_extension_ports::SkillActivationSelectionError::VisibilityDataMissing => {
            AgentLoopHostErrorKind::InvalidInvocation
        }
        ironclaw_first_party_extension_ports::SkillActivationSelectionError::ContextBudgetExceeded => {
            AgentLoopHostErrorKind::BudgetExceeded
        }
        ironclaw_first_party_extension_ports::SkillActivationSelectionError::SourceUnavailable => {
            AgentLoopHostErrorKind::Unavailable
        }
        ironclaw_first_party_extension_ports::SkillActivationSelectionError::Internal => {
            AgentLoopHostErrorKind::Internal
        }
    };
    ironclaw_loop_support::raw_agent_loop_host_error(
        "local_dev_skill_activate",
        "activate",
        kind,
        "skill activation failed",
        error,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_skill_activate_names_rejects_missing_names_field() {
        let error = parse_skill_activate_names(&serde_json::json!({}))
            .expect_err("missing names field should fail");

        assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
    }

    #[test]
    fn parse_skill_activate_names_rejects_empty_or_whitespace_names() {
        let error = parse_skill_activate_names(&serde_json::json!({"names": ["  "]}))
            .expect_err("empty names should fail");

        assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
    }

    #[test]
    fn parse_skill_activate_names_rejects_empty_array() {
        let error = parse_skill_activate_names(&serde_json::json!({"names": []}))
            .expect_err("empty array should fail");

        assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
    }

    #[test]
    fn parse_skill_activate_names_rejects_too_many_names() {
        let error = parse_skill_activate_names(&serde_json::json!({
            "names": vec!["skill"; MAX_SKILL_ACTIVATE_NAMES + 1]
        }))
        .expect_err("oversized names list should fail");

        assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
    }
}
