use clap::{CommandFactory, Parser};

use crate::commands::Command;

#[derive(Debug, Parser)]
#[command(
    name = "ironclaw-reborn",
    about = "Standalone IronClaw Reborn runtime",
    version
)]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub(crate) command: Command,
}

pub(crate) fn command() -> clap::Command {
    Cli::command()
}

pub(crate) fn run() -> anyhow::Result<()> {
    Cli::parse().command.execute()
}
