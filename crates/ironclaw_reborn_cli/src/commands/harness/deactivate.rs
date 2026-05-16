use clap::Args;
use ironclaw_reborn_composition::RebornAdminError;

use crate::commands::stubs::fail_not_yet_wired;
use crate::context::RebornCliContext;

/// Deactivate the active harness for a scope.
///
/// Stubbed per epic [#3036](https://github.com/nearai/ironclaw/issues/3036).
#[derive(Debug, Args)]
pub(crate) struct HarnessDeactivateCommand {
    /// Deactivate on a thread.
    #[arg(long = "thread", value_name = "THREAD_ID")]
    pub thread: Option<String>,

    /// Deactivate on a project.
    #[arg(long = "project", value_name = "PROJECT_ID")]
    pub project: Option<String>,

    /// Deactivate on a tenant.
    #[arg(long = "tenant", value_name = "TENANT_ID")]
    pub tenant: Option<String>,
}

impl HarnessDeactivateCommand {
    pub(crate) fn execute(self, _context: RebornCliContext) -> anyhow::Result<()> {
        let _ = (self.thread, self.project, self.tenant);
        fail_not_yet_wired(RebornAdminError::NotYetWired {
            operation: "harness.deactivate",
            tracking_issue: "#3036",
            requires: "HarnessActivationService",
        })
    }
}
