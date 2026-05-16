use clap::Subcommand;

pub(crate) mod channels;
pub(crate) mod completion;
pub(crate) mod config;
pub(crate) mod doctor;
pub(crate) mod harness;
pub(crate) mod hooks;
pub(crate) mod logs;
pub(crate) mod models;
pub(crate) mod profile;
pub(crate) mod run;
pub(crate) mod skills;
pub(crate) mod stubs;

#[derive(Debug, Subcommand)]
pub(crate) enum Command {
    /// Inspect configured Reborn channels.
    Channels(channels::ChannelsCommand),
    /// Generate shell completion scripts.
    Completion(completion::CompletionCommand),
    /// Inspect Reborn configuration paths without creating state.
    Config(config::ConfigCommand),
    /// Check Reborn binary configuration without creating state.
    Doctor(doctor::DoctorCommand),
    /// Install / activate / list use-case harnesses (epic #3036).
    /// Subcommands ship as discoverable stubs today; substrate lands
    /// incrementally via epic sub-issues.
    Harness(harness::HarnessCommand),
    /// Inspect configured Reborn hooks.
    Hooks(hooks::HooksCommand),
    /// Inspect Reborn logs.
    Logs(logs::LogsCommand),
    /// Inspect Reborn model slots and route status.
    Models(models::ModelsCommand),
    /// Inspect supported Reborn boot profiles.
    Profile(profile::ProfileCommand),
    /// Initialize the minimal Reborn runtime shell and exit.
    Run(run::RunCommand),
    /// Inspect configured Reborn skills.
    Skills(skills::SkillsCommand),
}

impl Command {
    pub(crate) fn execute(self) -> anyhow::Result<()> {
        match self {
            Self::Channels(command) => command.execute(),
            Self::Completion(command) => command.execute(),
            Self::Config(command) => {
                command.execute(crate::context::RebornCliContext::resolve_from_env()?)
            }
            Self::Doctor(command) => {
                command.execute(crate::context::RebornCliContext::resolve_from_env()?)
            }
            Self::Harness(command) => {
                command.execute(crate::context::RebornCliContext::resolve_from_env()?)
            }
            Self::Hooks(command) => command.execute(),
            Self::Logs(command) => command.execute(),
            Self::Models(command) => command.execute(),
            Self::Profile(command) => command.execute(),
            Self::Run(command) => {
                command.execute(crate::context::RebornCliContext::resolve_from_env()?)
            }
            Self::Skills(command) => command.execute(),
        }
    }
}
