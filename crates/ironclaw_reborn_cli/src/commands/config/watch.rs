use std::time::Duration;

use clap::Args;
use ironclaw_reborn_composition::RebornAdminError;

use crate::commands::stubs::fail_not_yet_wired;
use crate::context::RebornCliContext;

/// Poll a git URL for blueprint revisions and apply on each new
/// revision (GitOps mode).
///
/// Stubbed per epic [#3036](https://github.com/nearai/ironclaw/issues/3036).
#[derive(Debug, Args)]
pub(crate) struct ConfigWatchCommand {
    /// Git URL to watch.
    #[arg(value_name = "URL")]
    pub url: String,

    /// Poll interval in seconds.
    #[arg(long = "interval-secs", default_value_t = 60)]
    pub interval_secs: u64,

    /// Require signature verification on each revision. When set, the
    /// watcher rejects a revision whose signature can't be verified
    /// against a configured key.
    #[arg(long = "require-signature")]
    pub require_signature: bool,
}

impl ConfigWatchCommand {
    pub(crate) fn execute(self, _context: RebornCliContext) -> anyhow::Result<()> {
        let _ = (self.url, Duration::from_secs(self.interval_secs), self.require_signature);
        fail_not_yet_wired(RebornAdminError::NotYetWired {
            operation: "config.watch",
            tracking_issue: "#3036",
            requires: "BlueprintApplyService + GitOps watcher routine + signature verification",
        })
    }
}
