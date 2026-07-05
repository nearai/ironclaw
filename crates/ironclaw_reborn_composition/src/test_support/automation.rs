//! Test-support constructor for [`crate::RebornAutomationProductFacade`]
//! (W5-WEBUI-API-1 Enabler B.2). Constructor is `pub(crate)` in production;
//! this same-crate wrapper builds the real facade over the harness's shared
//! repository instead of a hand-rolled double duplicating its filter/join logic.

use std::sync::Arc;

use ironclaw_product_workflow::AutomationProductFacade;
use ironclaw_triggers::TriggerRepository;

/// Build the production `RebornAutomationProductFacade` over
/// `trigger_repository`, for `RebornServices::with_automation_product_facade`
/// (`ironclaw_product_workflow::RebornServices`) test wiring.
#[cfg(feature = "test-support")]
pub fn local_dev_automation_product_facade_for_test(
    trigger_repository: Arc<dyn TriggerRepository>,
) -> Arc<dyn AutomationProductFacade> {
    Arc::new(crate::automation::RebornAutomationProductFacade::new(
        trigger_repository,
    ))
}
