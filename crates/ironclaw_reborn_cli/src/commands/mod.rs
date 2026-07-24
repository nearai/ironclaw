use clap::Subcommand;

pub(crate) mod channels;
pub(crate) mod completion;
pub(crate) mod config;
pub(crate) mod doctor;
pub(crate) mod extension;
pub(crate) mod hooks;
pub(crate) mod logs;
pub(crate) mod models;
pub(crate) mod onboard;
pub(crate) mod profile;
pub(crate) mod repl;
pub(crate) mod run;
pub(crate) mod serve;
pub(crate) mod serve_sso;
pub(crate) mod service;
pub(crate) mod skills;
pub(crate) mod status;
pub(crate) mod traces;
pub(crate) mod user_directory;
pub(crate) mod webui_auth;

#[derive(Debug, Subcommand)]
pub(crate) enum Command {
    /// Inspect configured IronClaw channels.
    Channels(channels::ChannelsCommand),
    /// Generate shell completion scripts.
    Completion(completion::CompletionCommand),
    /// Inspect IronClaw configuration paths without creating state.
    Config(config::ConfigCommand),
    /// Check IronClaw configuration without creating state.
    Doctor(doctor::DoctorCommand),
    /// Manage the local IronClaw extension lifecycle.
    Extension(extension::ExtensionCommand),
    /// Inspect configured IronClaw hooks.
    Hooks(hooks::HooksCommand),
    /// Inspect IronClaw logs.
    Logs(logs::LogsCommand),
    /// Inspect IronClaw model slots and route status.
    Models(models::ModelsCommand),
    /// Initialize the IronClaw home and first-run setup marker.
    Onboard(onboard::OnboardCommand),
    /// Inspect supported IronClaw boot profiles.
    Profile(profile::ProfileCommand),
    /// Start the composed IronClaw CLI REPL.
    Repl(repl::ReplCommand),
    /// Initialize the minimal IronClaw runtime shell and exit.
    Run(run::RunCommand),
    /// Start the IronClaw WebUI service.
    Serve(serve::ServeCommand),
    /// Install/start/stop/status/uninstall the IronClaw service.
    /// as an OS-native service (launchd on macOS, systemd on Linux). The
    /// installed unit runs `serve`.
    Service(service::ServiceCommand),
    /// Inspect configured IronClaw skills.
    Skills(skills::SkillsCommand),
    /// Show the IronClaw runtime status snapshot.
    Status(status::StatusCommand),
    /// Manage trace contributions to TraceCommons.
    Traces(Box<traces::TracesCommand>),
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
            Self::Extension(command) => {
                command.execute(crate::context::RebornCliContext::resolve_from_env()?)
            }
            Self::Hooks(command) => command.execute(),
            Self::Logs(command) => command.execute(),
            Self::Models(command) => command.execute(),
            Self::Onboard(command) => {
                command.execute(crate::context::RebornCliContext::resolve_from_env()?)
            }
            Self::Profile(command) => command.execute(),
            Self::Repl(command) => {
                command.execute(crate::context::RebornCliContext::resolve_from_env()?)
            }
            Self::Run(command) => {
                command.execute(crate::context::RebornCliContext::resolve_from_env()?)
            }
            Self::Serve(command) => {
                command.execute(crate::context::RebornCliContext::resolve_from_env()?)
            }
            Self::Service(command) => {
                command.execute(crate::context::RebornCliContext::resolve_from_env()?)
            }
            Self::Skills(command) => {
                command.execute(crate::context::RebornCliContext::resolve_from_env()?)
            }
            Self::Status(command) => {
                command.execute(crate::context::RebornCliContext::resolve_from_env()?)
            }
            Self::Traces(command) => command.execute(),
        }
    }
}

/// Shared error for CLI surfaces that are intentionally kept visible in
/// `--help`/shell completions but do not yet have a working implementation
/// (`channels`, `hooks`, `logs`).
pub(crate) fn not_yet_implemented(command: &str) -> anyhow::Error {
    anyhow::anyhow!("`{command}` is not implemented yet")
}
