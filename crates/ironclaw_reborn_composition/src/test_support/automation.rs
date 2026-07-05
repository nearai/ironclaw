//! Test-support constructor for [`crate::RebornAutomationProductFacade`]
//! (W5-WEBUI-API-1 Enabler B.2, automations-cold-LIST scenario).
//!
//! The facade type itself is re-exported crate-wide (`pub use
//! automation::RebornAutomationProductFacade` in `lib.rs`), but its
//! constructor is `pub(crate)` — production composition is the only intended
//! caller — so a hand-rolled test double would have to duplicate the real
//! visibility-filter / run-history-join logic in `automation.rs` instead of
//! exercising it. This thin same-crate wrapper is the seam: it builds the
//! production facade over a caller-supplied `TriggerRepository` (the harness's
//! own shared repository, via
//! [`crate::RebornServices::local_dev_shared_trigger_repository_for_test`]),
//! matching the shape of `local_dev_project_service_for_test` et al.

use std::sync::Arc;

use ironclaw_product_workflow::AutomationProductFacade;
use ironclaw_triggers::TriggerRepository;

/// Build the production `RebornAutomationProductFacade` over
/// `trigger_repository`, for test wiring of
/// `RebornServices::with_automation_product_facade` (the WebUI-facing
/// `ironclaw_product_workflow::RebornServices` facade, not this crate's
/// composition-level type of the same name).
#[cfg(feature = "test-support")]
pub fn local_dev_automation_product_facade_for_test(
    trigger_repository: Arc<dyn TriggerRepository>,
) -> Arc<dyn AutomationProductFacade> {
    Arc::new(crate::automation::RebornAutomationProductFacade::new(
        trigger_repository,
    ))
}
