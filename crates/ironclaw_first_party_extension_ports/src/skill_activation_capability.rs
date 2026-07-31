//! Loop-facing skill activation capability.

use std::{collections::HashSet, sync::Arc};

use async_trait::async_trait;
use ironclaw_host_api::{ids::InvocationId, resolution::Resolution, result_meta::FailureKind};
use ironclaw_loop_host::{
    CapabilityResultWrite, DurablePersistence, SkillBundleSource, SyntheticCapability,
    SyntheticCapabilityDescriptor, SyntheticCapabilityHandler, SyntheticCapabilityInvocation,
};
use ironclaw_turns::run_profile::{
    AgentLoopHostError, AgentLoopHostErrorKind, CapabilityFailureDetail, ConcurrencyHint,
    resolution,
};

use crate::{
    DEFAULT_MAX_ACTIVE_SKILLS, SelectableSkillContextSource, SkillActivationSelectionError,
};

pub const SKILL_ACTIVATE_CAPABILITY_ID: &str = "builtin.skill_activate";
const SKILL_ACTIVATE_PROVIDER_TOOL_NAME: &str = "builtin__skill_activate";
const SKILL_ACTIVATE_DESCRIPTION: &str = "A skill is a packaged set of instructions someone has already written for a particular kind of task. Available skills are listed for you, each with a one-line description. When the task at hand is one a listed skill covers, call this FIRST: the skill's instructions load into the run and you follow them instead of your own default approach, which is the point -- a listed skill encodes decisions you would otherwise have to rediscover. Use exact listed names, activate every skill the task needs rather than only the first match -- a task often needs several, and naming them together in one call is correct (at most eight active skills total per run; large skills may reduce that), and do not activate skills unrelated to the task. An ambiguous name fails without loading anything.";

pub fn skill_activation_capability<S>(
    skill_activation_source: Arc<SelectableSkillContextSource<S>>,
) -> Result<SyntheticCapability, AgentLoopHostError>
where
    S: SkillBundleSource + ?Sized + 'static,
{
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

struct SkillActivationHandler<S>
where
    S: SkillBundleSource + ?Sized,
{
    skill_activation_source: Arc<SelectableSkillContextSource<S>>,
}

#[async_trait]
impl<S> SyntheticCapabilityHandler for SkillActivationHandler<S>
where
    S: SkillBundleSource + ?Sized + 'static,
{
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
        let output = build_activation_output(&activated, &plan.selection.feedback);
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
                "description": "Exact skill names copied from the available-skills list; at most eight total per run"
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

fn skill_activation_host_error(error: SkillActivationSelectionError) -> AgentLoopHostError {
    let kind = match error {
        SkillActivationSelectionError::AmbiguousSkill { .. }
        | SkillActivationSelectionError::ParseFailed
        | SkillActivationSelectionError::TrustDataMissing
        | SkillActivationSelectionError::VisibilityDataMissing => {
            AgentLoopHostErrorKind::InvalidInvocation
        }
        SkillActivationSelectionError::ContextBudgetExceeded => {
            AgentLoopHostErrorKind::BudgetExceeded
        }
        SkillActivationSelectionError::SourceUnavailable => AgentLoopHostErrorKind::Unavailable,
        SkillActivationSelectionError::Internal => AgentLoopHostErrorKind::Internal,
    };
    ironclaw_loop_host::raw_agent_loop_host_error(
        "skill_activate",
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
    error: SkillActivationSelectionError,
) -> Result<Resolution, AgentLoopHostError> {
    use crate::SkillActivationSelectionError as SelectionError;
    match error {
        // A resource limit, not an encoding fault — `Resource` keeps the
        // precise kind for downstream fate/wire/UI projections (both kinds
        // are ModelVisible, so recoverability is unchanged).
        SelectionError::ContextBudgetExceeded => Ok(diagnostic_failure(
            FailureKind::Resource,
            "skill activation exceeds the per-run skill context budget; activate fewer or smaller skills".to_string(),
        )),
        SelectionError::AmbiguousSkill { .. } => Ok(diagnostic_failure(
            FailureKind::InputEncode,
            "ambiguous skill name; specify a single unique skill to activate".to_string(),
        )),
        other => Err(skill_activation_host_error(other)),
    }
}

fn diagnostic_failure(error_kind: FailureKind, safe_summary: String) -> Resolution {
    resolution::failed(
        error_kind,
        safe_summary.clone(),
        CapabilityFailureDetail::Diagnostic { text: safe_summary },
    )
}

/// Build the model-visible result for `skill_activate`.
///
/// Extracted so the contract is unit-testable: the result previously carried only
/// `{activated, count}` and **discarded `plan.selection.feedback` entirely**, so every refusal
/// reason the selector builds was constructed and then thrown away. The model saw
/// `{"activated":[],"count":0}` and had to guess whether it had used a bad name, hit a trust
/// wall, or tripped an unmet requirement -- three situations needing three different responses.
///
/// Measured on the missing/unusable fixtures, every refusal was silent, so improving the reason
/// *text* moved nothing until the delivery was fixed too.
///
/// Routine "activated after model selection" confirmations are filtered out: next to
/// `activated` they are noise, and they would dilute the refusals that matter.
fn build_activation_output(activated: &[String], feedback: &[String]) -> serde_json::Value {
    let notes = feedback
        .iter()
        .filter(|note| !note.contains("activated after model selection"))
        .cloned()
        .collect::<Vec<_>>();
    if notes.is_empty() {
        serde_json::json!({
            "activated": activated,
            "count": activated.len(),
        })
    } else {
        serde_json::json!({
            "activated": activated,
            "count": activated.len(),
            "not_activated": notes,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A refusal must reach the MODEL, not just the live projection. Without this the reason is
    /// built and dropped, and the model sees an empty result it cannot act on.
    #[test]
    fn a_refusal_reason_is_surfaced_to_the_model() {
        let output = build_activation_output(
            &[],
            &[
                "trust-probe: found, but its trust is Installed and activation requires Trusted"
                    .to_string(),
            ],
        );
        assert_eq!(output["count"], 0);
        let notes = output["not_activated"]
            .as_array()
            .expect("a refusal must carry its reason to the model");
        assert_eq!(notes.len(), 1);
        assert!(notes[0].as_str().unwrap().contains("trust is Installed"));
    }

    /// A clean activation stays exactly as it was: no empty field, no extra noise.
    #[test]
    fn a_successful_activation_carries_no_not_activated_field() {
        let output = build_activation_output(
            &["citation-management".to_string()],
            &["citation-management: activated after model selection".to_string()],
        );
        assert_eq!(output["count"], 1);
        assert!(
            output.get("not_activated").is_none(),
            "a routine confirmation is noise next to `activated`: {output}"
        );
    }

    /// Mixed outcome: what loaded and what did not, in one result.
    #[test]
    fn a_mixed_result_reports_both_what_loaded_and_what_did_not() {
        let output = build_activation_output(
            &["pdf".to_string()],
            &[
                "pdf: activated after model selection".to_string(),
                "xlsx: not activated because its requirements are unmet: required binary not \
                 found: soffice"
                    .to_string(),
            ],
        );
        assert_eq!(output["count"], 1);
        let notes = output["not_activated"].as_array().unwrap();
        assert_eq!(notes.len(), 1, "only the refusal is forwarded: {output}");
        assert!(notes[0].as_str().unwrap().contains("soffice"));
    }

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
            SkillActivationSelectionError::ContextBudgetExceeded,
        )
        .expect("budget-exceeded must be a model-visible failure, not a terminal host error");

        // A budget limit is a resource failure, not an input-encoding fault.
        assert_recoverable_failure(
            &outcome,
            ironclaw_host_api::result_meta::FailureKind::Resource,
        );
    }

    #[test]
    fn ambiguous_skill_selection_is_a_recoverable_tool_failure_not_terminal() {
        let outcome =
            skill_activation_selection_outcome(SkillActivationSelectionError::AmbiguousSkill {
                name: "deploy".to_string(),
                sources: Vec::new(),
            })
            .expect("ambiguous skill must be a model-visible failure, not a terminal host error");

        assert_recoverable_failure(
            &outcome,
            ironclaw_host_api::result_meta::FailureKind::InputEncode,
        );
    }

    /// A recoverable model-visible failure is `Resolution::Done` carrying a
    /// `RecoverableFailure` verdict with the per-case precise kind (the §5.3
    /// collapse of the old `CapabilityOutcome::Failed { .. }`).
    fn assert_recoverable_failure(
        resolution: &ironclaw_host_api::resolution::Resolution,
        expected_kind: ironclaw_host_api::result_meta::FailureKind,
    ) {
        match resolution {
            ironclaw_host_api::resolution::Resolution::Done(outcome) => {
                assert_eq!(outcome.verdict.error_kind(), Some(&expected_kind));
                assert!(
                    outcome.verdict.diagnostic().is_some(),
                    "recoverable failures must carry a model-visible cause"
                );
            }
            other => panic!("expected Resolution::Done recoverable failure, got {other:?}"),
        }
    }

    #[test]
    fn source_unavailable_selection_stays_a_terminal_host_error() {
        let error =
            skill_activation_selection_outcome(SkillActivationSelectionError::SourceUnavailable)
                .expect_err("genuine host/infra failures must stay terminal");

        assert_eq!(error.kind, AgentLoopHostErrorKind::Unavailable);
    }

    #[test]
    fn internal_selection_stays_a_terminal_host_error() {
        let error = skill_activation_selection_outcome(SkillActivationSelectionError::Internal)
            .expect_err("internal bugs must stay terminal");

        assert_eq!(error.kind, AgentLoopHostErrorKind::Internal);
    }
}
