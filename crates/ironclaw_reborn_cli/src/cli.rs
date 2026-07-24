use clap::{CommandFactory, Parser};

use crate::commands::Command;

#[derive(Debug, Parser)]
#[command(name = "ironclaw", about = "IronClaw agent runtime", version)]
pub(crate) struct Cli {
    #[command(flatten)]
    pub(crate) serve: crate::commands::serve::ServeCommand,

    #[command(subcommand)]
    pub(crate) command: Option<Command>,
}

pub(crate) fn command() -> clap::Command {
    Cli::command()
}

pub(crate) fn run() -> anyhow::Result<()> {
    let cli = Cli::parse();
    cli.command.unwrap_or(Command::Serve(cli.serve)).execute()
}
