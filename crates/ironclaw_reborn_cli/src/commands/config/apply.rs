use std::path::PathBuf;

use clap::Args;
use ironclaw_reborn_composition::RebornAdminError;

use crate::commands::stubs::fail_not_yet_wired;
use crate::context::RebornCliContext;

/// Apply a declarative IronClaw blueprint to the runtime's typed repos.
///
/// Stubbed per epic [#3036](https://github.com/nearai/ironclaw/issues/3036).
/// The argument surface here is the shape epic #3036 commits to:
/// path or git URL source, optional lockfile destination, optional
/// dry-run.
#[derive(Debug, Args)]
pub(crate) struct ConfigApplyCommand {
    /// Path to a blueprint file (or directory of files merged
    /// deterministically). Mutually exclusive with `--git`.
    #[arg(value_name = "PATH", required_unless_present = "git")]
    pub path: Option<PathBuf>,

    /// Git URL of a blueprint repository. Mutually exclusive with `PATH`.
    #[arg(long = "git", value_name = "URL")]
    pub git: Option<String>,

    /// Compute the apply report without performing writes.
    #[arg(long = "dry-run")]
    pub dry_run: bool,

    /// Path to write the lockfile (per-file SHA-256 hashes of resolved
    /// file refs, so re-applies are deterministic across machines).
    #[arg(long = "lockfile", value_name = "PATH")]
    pub lockfile: Option<PathBuf>,
}

impl ConfigApplyCommand {
    pub(crate) fn execute(self, _context: RebornCliContext) -> anyhow::Result<()> {
        let _ = (self.path, self.git, self.dry_run, self.lockfile);
        fail_not_yet_wired(RebornAdminError::NotYetWired {
            operation: "config.apply",
            tracking_issue: "#3036",
            requires:
                "BlueprintParser + BlueprintApplyService + typed repos (Settings, Skill, Mission, Project)",
        })
    }
}
