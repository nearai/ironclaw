use ironclaw_host_api::execution_policy::{
    RequiredSkill, ResultDeliveryPolicy, TurnExecutionPolicy,
};
use ironclaw_triggers::TriggerExecutionSpec;
use serde::{Deserialize, Deserializer};

use crate::first_party_tools::trigger_management::TriggerScheduleInput;

pub(super) const TRIGGER_EXECUTION_CONTRACT_FIELDS: &[&str] = &[
    "goal",
    "success_criteria",
    "output_instructions",
    "no_result_text",
    "policy",
];
pub(super) const TRIGGER_EXECUTION_POLICY_FIELDS: &[&str] = &["required_skills", "result_delivery"];

pub(super) struct TriggerCreateInput {
    pub(super) name: String,
    pub(super) schedule: TriggerScheduleInput,
    pub(super) execution_contract: TriggerExecutionSpec,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TriggerCreateWireInput {
    name: String,
    schedule: TriggerScheduleInput,
    execution_contract: TriggerExecutionContractInput,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TriggerExecutionContractInput {
    goal: String,
    success_criteria: Vec<String>,
    output_instructions: String,
    no_result_text: String,
    policy: TriggerExecutionPolicyInput,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TriggerExecutionPolicyInput {
    #[serde(default)]
    required_skills: Vec<RequiredSkill>,
    result_delivery: ResultDeliveryPolicy,
}

impl From<TriggerExecutionContractInput> for TriggerExecutionSpec {
    fn from(input: TriggerExecutionContractInput) -> Self {
        Self {
            version: Self::VERSION,
            goal: input.goal,
            success_criteria: input.success_criteria,
            output_instructions: input.output_instructions,
            no_result_text: input.no_result_text,
            required_capability_ids: Vec::new(),
            policy: TurnExecutionPolicy {
                allowed_capability_ids: None,
                required_skills: input.policy.required_skills,
                result_delivery: input.policy.result_delivery,
            },
        }
    }
}

impl<'de> Deserialize<'de> for TriggerCreateInput {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let input = TriggerCreateWireInput::deserialize(deserializer)?;
        Ok(Self {
            name: input.name,
            schedule: input.schedule,
            execution_contract: input.execution_contract.into(),
        })
    }
}
