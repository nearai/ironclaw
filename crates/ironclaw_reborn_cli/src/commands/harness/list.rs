use clap::Args;
use ironclaw_reborn_composition::RebornAdminError;

use crate::commands::stubs::fail_not_yet_wired;
use crate::context::RebornCliContext;

/// List installed harnesses.
///
/// Stubbed per epic [#3036](https://github.com/nearai/ironclaw/issues/3036).
#[derive(Debug, Args)]
pub(crate) struct HarnessListCommand {
    /// Emit the list as JSON.
    #[arg(long = "json")]
    pub json: bool,
}

impl HarnessListCommand {
    pub(crate) fn execute(self, _context: RebornCliContext) -> anyhow::Result<()> {
        let _ = self.json;
        fail_not_yet_wired(RebornAdminError::NotYetWired {
            operation: "harness.list",
            tracking_issue: "#3036",
            requires: "HarnessRepo",
        })
    }
}
