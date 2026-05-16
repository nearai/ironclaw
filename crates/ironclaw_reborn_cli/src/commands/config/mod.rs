use clap::{Args, Subcommand};
use ironclaw_reborn_config::RebornDoctorReport;

use crate::context::RebornCliContext;

mod apply;
mod diff;
mod watch;

#[derive(Debug, Args)]
pub(crate) struct ConfigCommand {
    #[command(subcommand)]
    command: ConfigSubcommand,
}

#[derive(Debug, Subcommand)]
enum ConfigSubcommand {
    /// Show resolved Reborn configuration paths without creating state.
    Path(ConfigPathCommand),
    /// Apply a declarative IronClaw blueprint to the runtime's typed repos.
    ///
    /// Stub today; lands fully via epic
    /// [#3036](https://github.com/nearai/ironclaw/issues/3036) sub-issue
    /// "Blueprint apply service".
    Apply(apply::ConfigApplyCommand),
    /// Diff a declarative blueprint against the runtime's typed repos
    /// without writing.
    ///
    /// Stub today; lands fully via epic #3036 sub-issue "Blueprint diff".
    Diff(diff::ConfigDiffCommand),
    /// Watch a git URL for blueprint revisions and apply on each new
    /// revision (GitOps mode).
    ///
    /// Stub today; lands via epic #3036 sub-issue "GitOps watcher".
    Watch(watch::ConfigWatchCommand),
}

#[derive(Debug, Args)]
struct ConfigPathCommand;

impl ConfigCommand {
    pub(crate) fn execute(self, context: RebornCliContext) -> anyhow::Result<()> {
        match self.command {
            ConfigSubcommand::Path(command) => command.execute(context),
            ConfigSubcommand::Apply(command) => command.execute(context),
            ConfigSubcommand::Diff(command) => command.execute(context),
            ConfigSubcommand::Watch(command) => command.execute(context),
        }
    }
}

impl ConfigPathCommand {
    fn execute(self, context: RebornCliContext) -> anyhow::Result<()> {
        let report = RebornDoctorReport::from_config(context.boot_config().clone());

        println!("IronClaw Reborn config path");
        println!("reborn_home: {}", report.home_path().display());
        println!("home_source: {}", report.home_source_label());
        println!("profile: {}", report.profile());
        println!("v1_state: {}", report.v1_state());
        Ok(())
    }
}
