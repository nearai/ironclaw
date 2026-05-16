//! `ironclaw-reborn harness ...` subcommand tree.
//!
//! These commands compose / install / activate "use-case harnesses" —
//! named bundles of extensions, skills, prompt overlay, runtime
//! constraints, capability surface filter, and required exit artifacts.
//! See epic [#3036](https://github.com/nearai/ironclaw/issues/3036)
//! "Configuration-as-Code for IronClaw Reborn: tenant blueprints and
//! use-case harnesses" for the contract.
//!
//! Every subcommand is a stub today: the substrate (HarnessRepo,
//! HarnessActivationService, capability-surface filter, instruction-
//! bundle overlay path) lands incrementally via the epic's sub-issues.
//! The command tree shape is locked here so operator tooling and shell
//! completion can discover the final surface immediately.

use clap::{Args, Subcommand};

use crate::context::RebornCliContext;

mod activate;
mod deactivate;
mod install;
mod list;

#[derive(Debug, Args)]
pub(crate) struct HarnessCommand {
    #[command(subcommand)]
    command: HarnessSubcommand,
}

#[derive(Debug, Subcommand)]
enum HarnessSubcommand {
    /// Install a harness manifest into the typed harness repo.
    Install(install::HarnessInstallCommand),
    /// List installed harnesses.
    List(list::HarnessListCommand),
    /// Activate an installed harness for a scope (thread / project / tenant).
    Activate(activate::HarnessActivateCommand),
    /// Deactivate the active harness for a scope.
    Deactivate(deactivate::HarnessDeactivateCommand),
}

impl HarnessCommand {
    pub(crate) fn execute(self, context: RebornCliContext) -> anyhow::Result<()> {
        match self.command {
            HarnessSubcommand::Install(command) => command.execute(context),
            HarnessSubcommand::List(command) => command.execute(context),
            HarnessSubcommand::Activate(command) => command.execute(context),
            HarnessSubcommand::Deactivate(command) => command.execute(context),
        }
    }
}
