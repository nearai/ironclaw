use clap::Subcommand;

pub(crate) mod completion;
pub(crate) mod doctor;
pub(crate) mod hooks;
pub(crate) mod run;

#[derive(Debug, Subcommand)]
pub(crate) enum Command {
    /// Generate shell completion scripts.
    Completion(completion::CompletionCommand),
    /// Check Reborn binary configuration without creating state.
    Doctor(doctor::DoctorCommand),
    /// Inspect configured Reborn hooks.
    Hooks(hooks::HooksCommand),
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
            Self::Hooks(command) => command.execute(),
            Self::Run(command) => {
                command.execute(crate::context::RebornCliContext::resolve_from_env()?)
            }
        }
    }
}
