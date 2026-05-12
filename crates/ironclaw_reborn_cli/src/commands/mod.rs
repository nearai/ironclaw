use clap::Subcommand;

pub(crate) mod completion;
pub(crate) mod config;
pub(crate) mod doctor;
pub(crate) mod run;

#[derive(Debug, Subcommand)]
pub(crate) enum Command {
    /// Generate shell completion scripts.
    Completion(completion::CompletionCommand),
    /// Inspect Reborn configuration paths without creating state.
    Config(config::ConfigCommand),
    /// Check Reborn binary configuration without creating state.
    Doctor(doctor::DoctorCommand),
    /// Initialize the minimal Reborn runtime shell and exit.
    Run(run::RunCommand),
}

impl Command {
    pub(crate) fn execute(self) -> anyhow::Result<()> {
        match self {
            Self::Completion(command) => command.execute(),
            Self::Config(command) => {
                command.execute(crate::context::RebornCliContext::resolve_from_env()?)
            }
            Self::Doctor(command) => {
                command.execute(crate::context::RebornCliContext::resolve_from_env()?)
            }
            Self::Run(command) => {
                command.execute(crate::context::RebornCliContext::resolve_from_env()?)
            }
        }
    }
}
