use std::path::PathBuf;

use clap::Args;
use ironclaw_reborn_composition::RebornAdminError;

use crate::commands::stubs::fail_not_yet_wired;
use crate::context::RebornCliContext;

/// Show drift between a blueprint and the runtime's typed repos
/// without writing.
///
/// Stubbed per epic [#3036](https://github.com/nearai/ironclaw/issues/3036).
#[derive(Debug, Args)]
pub(crate) struct ConfigDiffCommand {
    /// Path to a blueprint file. Mutually exclusive with `--git`.
    #[arg(value_name = "PATH", required_unless_present = "git")]
    pub path: Option<PathBuf>,

    /// Git URL of a blueprint repository. Mutually exclusive with `PATH`.
    #[arg(long = "git", value_name = "URL")]
    pub git: Option<String>,
}

impl ConfigDiffCommand {
    pub(crate) fn execute(self, _context: RebornCliContext) -> anyhow::Result<()> {
        let _ = (self.path, self.git);
        fail_not_yet_wired(RebornAdminError::NotYetWired {
            operation: "config.diff",
            tracking_issue: "#3036",
            requires: "BlueprintParser + BlueprintApplyService.diff()",
        })
    }
}
