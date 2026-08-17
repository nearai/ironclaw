use std::collections::HashSet;

use ironclaw_host_api::execution_policy::{ResultDeliveryPolicy, TurnExecutionPolicy};
use serde::{Deserialize, Serialize};

use crate::{MAX_TRIGGER_PROMPT_BYTES, TriggerError, TriggerRecordValidationKind};

const EXECUTION_SPEC_VERSION: u16 = 1;
const MAX_GOAL_BYTES: usize = 4 * 1024;
const MAX_SUCCESS_CRITERIA: usize = 32;
const MAX_SUCCESS_CRITERIA_BYTES: usize = 8 * 1024;
const MAX_OUTPUT_INSTRUCTIONS_BYTES: usize = 4 * 1024;
const MAX_NO_RESULT_TEXT_BYTES: usize = 2 * 1024;
const MAX_ALLOWED_CAPABILITIES: usize = 64;
const MAX_REQUIRED_SKILLS: usize = 8;
const PROMPT_TEMPLATE: &str = include_str!("prompts/trigger_execution.md");
const SUPPRESSED_NO_RESULT_TEMPLATE: &str =
    include_str!("prompts/suppress_when_nothing_to_report.md");

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TriggerExecutionSpec {
    pub version: u16,
    pub goal: String,
    pub success_criteria: Vec<String>,
    pub output_instructions: String,
    pub no_result_text: String,
    #[serde(default)]
    pub policy: TurnExecutionPolicy,
}

impl TriggerExecutionSpec {
    pub fn validate(&self) -> Result<(), TriggerError> {
        if self.version != EXECUTION_SPEC_VERSION {
            return invalid(format!(
                "unsupported execution spec version {}; expected {EXECUTION_SPEC_VERSION}",
                self.version
            ));
        }
        validate_text("goal", &self.goal, MAX_GOAL_BYTES)?;
        if self.success_criteria.is_empty() {
            return invalid("success_criteria must contain at least one item");
        }
        if self.success_criteria.len() > MAX_SUCCESS_CRITERIA {
            return invalid(format!(
                "success_criteria must contain at most {MAX_SUCCESS_CRITERIA} items"
            ));
        }
        let mut criteria_bytes = 0usize;
        for criterion in &self.success_criteria {
            validate_text("success criterion", criterion, MAX_SUCCESS_CRITERIA_BYTES)?;
            criteria_bytes = criteria_bytes.saturating_add(criterion.len());
        }
        if criteria_bytes > MAX_SUCCESS_CRITERIA_BYTES {
            return invalid(format!(
                "success_criteria must total at most {MAX_SUCCESS_CRITERIA_BYTES} bytes"
            ));
        }
        validate_text(
            "output_instructions",
            &self.output_instructions,
            MAX_OUTPUT_INSTRUCTIONS_BYTES,
        )?;
        validate_text(
            "no_result_text",
            &self.no_result_text,
            MAX_NO_RESULT_TEXT_BYTES,
        )?;
        validate_policy(&self.policy)?;
        if self.render_prompt().len() > MAX_TRIGGER_PROMPT_BYTES {
            return invalid(format!(
                "rendered prompt must be at most {MAX_TRIGGER_PROMPT_BYTES} bytes"
            ));
        }
        Ok(())
    }

    pub fn render_prompt(&self) -> String {
        let criteria = self
            .success_criteria
            .iter()
            .map(|criterion| format!("- {criterion}"))
            .collect::<Vec<_>>()
            .join("\n");
        let no_result_instruction = match self.policy.result_delivery {
            ResultDeliveryPolicy::Deliver => self.no_result_text.clone(),
            ResultDeliveryPolicy::SuppressWhenNothingToReport => render_template(
                SUPPRESSED_NO_RESULT_TEMPLATE,
                &[("{{no_result_text}}", self.no_result_text.as_str())],
            )
            .trim_end()
            .to_string(),
        };
        render_template(
            PROMPT_TEMPLATE,
            &[
                ("{{goal}}", self.goal.as_str()),
                ("{{success_criteria}}", criteria.as_str()),
                ("{{output_instructions}}", self.output_instructions.as_str()),
                ("{{no_result_text}}", no_result_instruction.as_str()),
            ],
        )
    }
}

/// Substitutes every placeholder in one pass over the template, so text
/// inserted for one field is never rescanned for other placeholders — a goal
/// containing the literal `{{success_criteria}}` must stay verbatim.
fn render_template(template: &str, substitutions: &[(&str, &str)]) -> String {
    let mut rendered = String::with_capacity(template.len());
    let mut rest = template;
    while !rest.is_empty() {
        let mut earliest: Option<(usize, &str, &str)> = None;
        for (token, value) in substitutions {
            if let Some(index) = rest.find(token)
                && earliest.is_none_or(|(best, _, _)| index < best)
            {
                earliest = Some((index, token, value));
            }
        }
        match earliest {
            Some((index, token, value)) => {
                // safety: `str::find` returns a match-start byte index, which is always a char boundary.
                rendered.push_str(&rest[..index]); // safety: index from `str::find` is a char boundary
                rendered.push_str(value);
                rest = &rest[index + token.len()..];
            }
            None => {
                rendered.push_str(rest);
                break;
            }
        }
    }
    rendered
}

fn validate_text(label: &str, value: &str, max_bytes: usize) -> Result<(), TriggerError> {
    if value.trim().is_empty() {
        return invalid(format!("{label} must not be empty"));
    }
    if value.len() > max_bytes {
        return invalid(format!("{label} must be at most {max_bytes} bytes"));
    }
    Ok(())
}

fn validate_policy(policy: &TurnExecutionPolicy) -> Result<(), TriggerError> {
    if let Some(capabilities) = &policy.allowed_capability_ids {
        if capabilities.len() > MAX_ALLOWED_CAPABILITIES {
            return invalid(format!(
                "allowed_capability_ids must contain at most {MAX_ALLOWED_CAPABILITIES} items"
            ));
        }
        let unique = capabilities.iter().collect::<HashSet<_>>();
        if unique.len() != capabilities.len() {
            return invalid("allowed_capability_ids must not contain duplicates");
        }
    }
    if policy.required_skills.len() > MAX_REQUIRED_SKILLS {
        return invalid(format!(
            "required_skills must contain at most {MAX_REQUIRED_SKILLS} items"
        ));
    }
    let unique = policy.required_skills.iter().collect::<HashSet<_>>();
    if unique.len() != policy.required_skills.len() {
        return invalid("required_skills must not contain duplicates");
    }
    Ok(())
}

fn invalid<T>(reason: impl Into<String>) -> Result<T, TriggerError> {
    Err(TriggerError::InvalidRecord {
        kind: TriggerRecordValidationKind::ExecutionSpecInvalid,
        reason: reason.into(),
    })
}
