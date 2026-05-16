use std::path::PathBuf;

use clap::Args;
use ironclaw_reborn_composition::RebornAdminError;

use crate::commands::stubs::fail_not_yet_wired;
use crate::context::RebornCliContext;

/// Install a harness manifest from a path or git URL.
///
/// Stubbed per epic [#3036](https://github.com/nearai/ironclaw/issues/3036).
#[derive(Debug, Args)]
pub(crate) struct HarnessInstallCommand {
    /// Path to the harness manifest directory (containing `manifest.toml`
    /// plus referenced files).
    #[arg(value_name = "PATH", required_unless_present = "git")]
    pub path: Option<PathBuf>,

    /// Git URL of a harness manifest repository.
    #[arg(long = "git", value_name = "URL")]
    pub git: Option<String>,
}

impl HarnessInstallCommand {
    pub(crate) fn execute(self, _context: RebornCliContext) -> anyhow::Result<()> {
        let _ = (self.path, self.git);
        fail_not_yet_wired(RebornAdminError::NotYetWired {
            operation: "harness.install",
            tracking_issue: "#3036",
            requires: "HarnessManifest parser + HarnessRepo",
        })
    }
}
