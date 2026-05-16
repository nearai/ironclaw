use clap::Args;
use ironclaw_reborn_composition::RebornAdminError;

use crate::commands::stubs::fail_not_yet_wired;
use crate::context::RebornCliContext;

/// Activate an installed harness for a scope.
///
/// Stubbed per epic [#3036](https://github.com/nearai/ironclaw/issues/3036).
#[derive(Debug, Args)]
pub(crate) struct HarnessActivateCommand {
    /// Harness id to activate.
    #[arg(value_name = "HARNESS_ID")]
    pub harness_id: String,

    /// Activate on a thread. Mutually exclusive with `--project` and
    /// `--tenant`.
    #[arg(long = "thread", value_name = "THREAD_ID")]
    pub thread: Option<String>,

    /// Activate on a project. Mutually exclusive with `--thread` and
    /// `--tenant`.
    #[arg(long = "project", value_name = "PROJECT_ID")]
    pub project: Option<String>,

    /// Activate on a tenant. Mutually exclusive with `--thread` and
    /// `--project`. Tenant-wide activation requires admin scope.
    #[arg(long = "tenant", value_name = "TENANT_ID")]
    pub tenant: Option<String>,
}

impl HarnessActivateCommand {
    pub(crate) fn execute(self, _context: RebornCliContext) -> anyhow::Result<()> {
        let _ = (self.harness_id, self.thread, self.project, self.tenant);
        fail_not_yet_wired(RebornAdminError::NotYetWired {
            operation: "harness.activate",
            tracking_issue: "#3036",
            requires:
                "HarnessActivationService + InstructionBundleAssembler overlay path + capability-surface filter",
        })
    }
}
