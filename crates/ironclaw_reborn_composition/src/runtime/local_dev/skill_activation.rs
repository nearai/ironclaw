use std::{collections::HashSet, sync::Arc};

use async_trait::async_trait;
use ironclaw_first_party_extension_ports::DEFAULT_MAX_ACTIVE_SKILLS;
use ironclaw_host_api::{InvocationId, Resolution};
use ironclaw_loop_host::{CapabilityResultWrite, DurablePersistence};
use ironclaw_turns::run_profile::{
    AgentLoopHostError, AgentLoopHostErrorKind, CapabilityFailureKind, ConcurrencyHint, resolution,
};

use crate::runtime::{
    ComposedSelectableSkillContextSource,
    local_dev::synthetic_capability::{
        SyntheticCapability, SyntheticCapabilityDescriptor, SyntheticCapabilityHandler,
        SyntheticCapabilityInvocation,
    },
};

pub(crate) const SKILL_ACTIVATE_CAPABILITY_ID: &str = "builtin.skill_activate";
const SKILL_ACTIVATE_PROVIDER_TOOL_NAME: &str = "builtin__skill_activate";
const SKILL_ACTIVATE_DESCRIPTION: &str = "Load full instructions for one or more skills from the available-skills list. Call this before answering when a listed skill could help with any part of the task; use only exact listed names. Choose the smallest relevant set, with at most four active skills total per run; large skills may reduce that number. Ambiguous names fail without loading a skill.";

pub(super) fn skill_activation_capability(
    skill_activation_source: Arc<ComposedSelectableSkillContextSource>,
) -> Result<SyntheticCapability, AgentLoopHostError> {
    Ok(SyntheticCapability::new(
        SyntheticCapabilityDescriptor::new(
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
    skill_activation_source: Arc<ComposedSelectableSkillContextSource>,
}

#[async_trait]
impl SyntheticCapabilityHandler for SkillActivationHandler {
    fn validate_provider_arguments(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<(), AgentLoopHostError> {
        parse_skill_activate_names(arguments).map(|_| ())
    }

    async fn invoke(
        &self,
        invocation: SyntheticCapabilityInvocation,
    ) -> Result<Resolution, AgentLoopHostError> {
        // Normalise to lowercase at the parse boundary so that `names` (passed
        // to `activate_skills_for_run`) and the response-filter set both use the
        // same canonical form. `activate_skills_for_run` matches with
        // `eq_ignore_ascii_case`, so lowercase input is always accepted. Without
        // this normalisation, the original-case `names` would be passed to the
        // registry while the filter set was lowercased, causing a mismatch when
        // `activation.name` differs in case from the caller's input.
        let names = parse_skill_activate_names(&invocation.input)?
            .into_iter()
            .map(|name| name.to_ascii_lowercase())
            .collect::<Vec<_>>();
        let requested_names = names.iter().cloned().collect::<HashSet<_>>();
        let plan = match self
            .skill_activation_source
            .activate_skills_for_run(&invocation.run_context, &names)
            .await
        {
            Ok(plan) => plan,
            // A model-recoverable selection failure (the model selected too many
            // or too-large skills, or named an ambiguous skill) must surface as a
            // model-visible tool error so the run continues and the model can
            // retry with a smaller/disambiguated selection — NOT a terminal
            // `Err(AgentLoopHostError)`, which `ironclaw_agent_loop`'s executor
            // maps to a run-ending `HostUnavailable { stage: Capability }`. Only
            // genuine host/infra failures stay terminal. See
            // `skill_activation_selection_outcome`.
            Err(error) => return skill_activation_selection_outcome(error),
        };
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
        let write_result = invocation
            .result_writer
            .write_capability_result(CapabilityResultWrite {
                run_context: &invocation.run_context,
                input_ref: &invocation.request.input_ref,
                invocation_id: InvocationId::new(),
                capability_id: &invocation.request.capability_id,
                output,
                display_preview: None,
                durable_persistence: DurablePersistence::Persist,
            })
            .await?;
        Ok(resolution::completed(
            write_result.result_ref,
            format!("activated {} skill(s)", activated.len()),
            ironclaw_turns::run_profile::CapabilityProgress::MadeProgress,
            false,
            write_result.byte_len,
            write_result.output_digest,
            write_result.model_observation,
        ))
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
                "maxItems": DEFAULT_MAX_ACTIVE_SKILLS,
                "description": "Exact skill names copied from the available-skills list; at most four total per run"
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
    if parsed.len() > DEFAULT_MAX_ACTIVE_SKILLS {
        return Err(AgentLoopHostError::new(
            AgentLoopHostErrorKind::InvalidInvocation,
            format!(
                "skill_activate accepts at most {DEFAULT_MAX_ACTIVE_SKILLS} skill names per call"
            ),
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
    ironclaw_loop_host::raw_agent_loop_host_error(
        "local_dev_skill_activate",
        "activate",
        kind,
        "skill activation failed",
        error,
    )
}

/// Disposition a skill-activation selection failure into either a model-visible,
/// recoverable capability failure or a terminal host error.
///
/// The two arms map onto the executor's two failure paths
/// (`ironclaw_agent_loop::executor::mapping`):
///
/// - `CapabilityOutcome::Failed` is handed back to the model and the run
///   continues, so the model can retry. Selection failures the model directly
///   controls — picking too many/too-large skills (`ContextBudgetExceeded`) or
///   naming an ambiguous skill (`AmbiguousSkill`) — take this path.
/// - `Err(AgentLoopHostError)` is mapped to a run-ending
///   `HostUnavailable { stage: Capability }`. Only genuine host/infra failures
///   (unavailable source, unparsable bundle, missing trust/visibility metadata,
///   internal bug) stay terminal, because the model cannot recover from them by
///   adjusting its request.
fn skill_activation_selection_outcome(
    error: ironclaw_first_party_extension_ports::SkillActivationSelectionError,
) -> Result<Resolution, AgentLoopHostError> {
    use ironclaw_first_party_extension_ports::SkillActivationSelectionError as SelectionError;
    match error {
        SelectionError::ContextBudgetExceeded => Ok(resolution::failed(
            CapabilityFailureKind::InvalidInput,
            "skill activation exceeds the per-run skill context budget; activate fewer or smaller skills".to_string(),
            None,
        )),
        SelectionError::AmbiguousSkill { .. } => Ok(resolution::failed(
            CapabilityFailureKind::InvalidInput,
            "ambiguous skill name; specify a single unique skill to activate".to_string(),
            None,
        )),
        other => Err(skill_activation_host_error(other)),
    }
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
            "names": vec!["skill"; DEFAULT_MAX_ACTIVE_SKILLS + 1]
        }))
        .expect_err("oversized names list should fail");

        assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
    }

    #[test]
    fn budget_exceeded_selection_is_a_recoverable_tool_failure_not_terminal() {
        let outcome = skill_activation_selection_outcome(
            ironclaw_first_party_extension_ports::SkillActivationSelectionError::ContextBudgetExceeded,
        )
        .expect("budget-exceeded must be a model-visible failure, not a terminal host error");

        assert_recoverable_invalid_input(&outcome);
    }

    #[test]
    fn ambiguous_skill_selection_is_a_recoverable_tool_failure_not_terminal() {
        let outcome = skill_activation_selection_outcome(
            ironclaw_first_party_extension_ports::SkillActivationSelectionError::AmbiguousSkill {
                name: "deploy".to_string(),
                sources: Vec::new(),
            },
        )
        .expect("ambiguous skill must be a model-visible failure, not a terminal host error");

        assert_recoverable_invalid_input(&outcome);
    }

    /// A recoverable model-visible failure is `Resolution::Done` carrying a
    /// `RecoverableFailure(InvalidInput)` verdict (the §5.3 collapse of the old
    /// `CapabilityOutcome::Failed { InvalidInput }`).
    fn assert_recoverable_invalid_input(resolution: &ironclaw_host_api::Resolution) {
        match resolution {
            ironclaw_host_api::Resolution::Done(outcome) => assert_eq!(
                outcome.verdict,
                ironclaw_host_api::ToolVerdict::recoverable_failure(
                    ironclaw_host_api::FailureKind::InvalidInput
                )
            ),
            other => panic!("expected Resolution::Done recoverable failure, got {other:?}"),
        }
    }

    #[test]
    fn source_unavailable_selection_stays_a_terminal_host_error() {
        let error = skill_activation_selection_outcome(
            ironclaw_first_party_extension_ports::SkillActivationSelectionError::SourceUnavailable,
        )
        .expect_err("genuine host/infra failures must stay terminal");

        assert_eq!(error.kind, AgentLoopHostErrorKind::Unavailable);
    }

    #[test]
    fn internal_selection_stays_a_terminal_host_error() {
        let error = skill_activation_selection_outcome(
            ironclaw_first_party_extension_ports::SkillActivationSelectionError::Internal,
        )
        .expect_err("internal bugs must stay terminal");

        assert_eq!(error.kind, AgentLoopHostErrorKind::Internal);
    }
}
