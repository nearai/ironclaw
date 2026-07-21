//! Test-support constructor for [`crate::RebornAutomationProductFacade`]
//! (W5-WEBUI-API-1 Enabler B.2). Constructor is `pub(crate)` in production;
//! this same-crate wrapper builds the real facade over the harness's shared
//! repository instead of a hand-rolled double duplicating its filter/join logic.

use std::sync::Arc;

use ironclaw_filesystem::RootFilesystem;
use ironclaw_product_workflow::AutomationProductFacade;
use ironclaw_triggers::{TriggerActiveRunLookup, TriggerRepository};
use ironclaw_turns::FilesystemTurnStateRowStore;

use crate::automation::trigger_poller::SnapshotActiveRunLookup;
use crate::turn_run_snapshot::TurnRunSnapshotSource;

/// Build the production `RebornAutomationProductFacade` over
/// `trigger_repository` plus the harness's own turn-state store, for
/// `RebornServices::with_automation_product_facade`
/// (`ironclaw_product_workflow::RebornServices`) test wiring. The turn-state
/// store backs the active-hold projection from the same run state the harness
/// coordinator writes, mirroring production's automation-backing pair (#5886).
#[cfg(feature = "test-support")]
pub fn local_dev_automation_product_facade_for_test<F>(
    trigger_repository: Arc<dyn TriggerRepository>,
    turn_state: Arc<FilesystemTurnStateRowStore<F>>,
) -> Arc<dyn AutomationProductFacade>
where
    F: RootFilesystem + Send + Sync + 'static,
{
    let active_run_lookup = Arc::new(SnapshotActiveRunLookup::new(
        turn_state as Arc<dyn TurnRunSnapshotSource>,
    ));
    Arc::new(
        crate::automation::facade::RebornAutomationProductFacade::new(
            trigger_repository,
            active_run_lookup,
        ),
    )
}

/// Build the raw [`TriggerActiveRunLookup`] the production automation panel
/// wiring uses (`build_local_runtime`'s `trigger_active_run_lookup`), without
/// the `RebornAutomationProductFacade` wrapper. For test harnesses that need
/// to wire the SAME lookup semantics directly into a `builtin.trigger_list`
/// capability registry (`ironclaw_host_runtime::builtin_first_party_handlers_with_trigger_create_hook`)
/// instead of through the WebUI automations facade — see
/// `HostRuntimeCapabilityHarness::install_trigger_active_run_lookup_for_test` (#5886).
#[cfg(feature = "test-support")]
pub fn local_dev_trigger_active_run_lookup_for_test<F>(
    turn_state: Arc<FilesystemTurnStateRowStore<F>>,
) -> Arc<dyn TriggerActiveRunLookup>
where
    F: RootFilesystem + Send + Sync + 'static,
{
    Arc::new(SnapshotActiveRunLookup::new(
        turn_state as Arc<dyn TurnRunSnapshotSource>,
    ))
}
