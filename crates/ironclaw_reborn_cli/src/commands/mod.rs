use clap::Subcommand;

pub(crate) mod completion;
pub(crate) mod doctor;
pub(crate) mod logs;
pub(crate) mod run;

#[derive(Debug, Subcommand)]
pub(crate) enum Command {
    /// Generate shell completion scripts.
    Completion(completion::CompletionCommand),
    /// Check Reborn binary configuration without creating state.
    Doctor(doctor::DoctorCommand),
    /// Inspect Reborn logs.
    Logs(logs::LogsCommand),
    /// Initialize the minimal Reborn runtime shell and exit.
    Run(run::RunCommand),
}

impl Command {
    pub(crate) fn execute(self) -> anyhow::Result<()> {
        match self {
            Self::Completion(command) => command.execute(),
            Self::Doctor(command) => {
                command.execute(crate::context::RebornCliContext::resolve_from_env()?)
            }
            Self::Logs(command) => command.execute(),
            Self::Run(command) => {
                command.execute(crate::context::RebornCliContext::resolve_from_env()?)
            }
        }
    }
}
