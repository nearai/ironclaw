use ironclaw_triggers::TriggerExecutionSpec;
use serde::Deserialize;

use super::trigger_management::TriggerScheduleInput;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct TriggerCreateInput {
    pub(super) name: String,
    pub(super) schedule: TriggerScheduleInput,
    pub(super) execution_contract: TriggerExecutionSpec,
}
