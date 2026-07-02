use clap::{Args, Subcommand};

mod run;

pub(crate) use run::MultiAgentRunCommand;

#[derive(Debug, Args)]
pub(crate) struct MultiAgentCommand {
    #[command(subcommand)]
    command: MultiAgentSubcommand,
}

#[derive(Debug, Subcommand)]
enum MultiAgentSubcommand {
    /// Run a task through the recursive multi-agent orchestrator.
    Run(MultiAgentRunCommand),
}

impl MultiAgentCommand {
    pub(crate) fn execute(self) -> anyhow::Result<()> {
        match self.command {
            MultiAgentSubcommand::Run(command) => command.execute(),
        }
    }
}
