use clap::Subcommand;

pub(crate) mod channels;
pub(crate) mod completion;
pub(crate) mod config;
pub(crate) mod doctor;
pub(crate) mod hooks;
pub(crate) mod logs;
pub(crate) mod models;
pub(crate) mod profile;
pub(crate) mod repl;
pub(crate) mod run;
#[cfg(feature = "webui-v2-beta")]
pub(crate) mod serve;
pub(crate) mod skills;
pub(crate) mod traces;

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
    /// Inspect configured Reborn hooks.
    Hooks(hooks::HooksCommand),
    /// Inspect Reborn logs.
    Logs(logs::LogsCommand),
    /// Inspect Reborn model slots and route status.
    Models(models::ModelsCommand),
    /// Inspect supported Reborn boot profiles.
    Profile(profile::ProfileCommand),
    /// Start the composed Reborn CLI REPL.
    Repl(repl::ReplCommand),
    /// Initialize the minimal Reborn runtime shell and exit.
    Run(run::RunCommand),
    /// Start the Reborn WebUI service. Available only when the binary
    /// is built with the `webui-v2-beta` Cargo feature; off by default
    /// because the beta HTTP/auth gateway requires explicit opt-in
    /// before being linked into a production binary.
    #[cfg(feature = "webui-v2-beta")]
    Serve(serve::ServeCommand),
    /// Inspect configured Reborn skills.
    Skills(skills::SkillsCommand),
    /// Manage trace contributions to TraceCommons.
    Traces(traces::TracesCommand),
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
            Self::Hooks(command) => command.execute(),
            Self::Logs(command) => command.execute(),
            Self::Models(command) => command.execute(),
            Self::Profile(command) => command.execute(),
            Self::Repl(command) => {
                command.execute(crate::context::RebornCliContext::resolve_from_env()?)
            }
            Self::Run(command) => {
                command.execute(crate::context::RebornCliContext::resolve_from_env()?)
            }
            #[cfg(feature = "webui-v2-beta")]
            Self::Serve(command) => {
                command.execute(crate::context::RebornCliContext::resolve_from_env()?)
            }
            Self::Skills(command) => command.execute(),
            Self::Traces(command) => command.execute(),
        }
    }
}
