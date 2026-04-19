//! CLI subcommand definitions for `ironclaw service`.

use clap::Subcommand;

use crate::service::ServiceAction;

#[derive(Subcommand, Debug, Clone)]
pub enum ServiceCommand {
    /// Install the OS service (launchd on macOS, systemd on Linux).
    Install,
    /// Start the installed service.
    Start,
    /// Stop the running service.
    Stop,
    /// Show service status.
    Status,
    /// Uninstall the OS service and remove the unit file.
    Uninstall,
}

impl ServiceCommand {
    /// Convert the CLI variant into the domain action.
    pub fn to_action(&self) -> ServiceAction {
        match self {
            ServiceCommand::Install => ServiceAction::Install,
            ServiceCommand::Start => ServiceAction::Start,
            ServiceCommand::Stop => ServiceAction::Stop,
            ServiceCommand::Status => ServiceAction::Status,
            ServiceCommand::Uninstall => ServiceAction::Uninstall,
        }
    }
}

/// Run the service command.
pub fn run_service_command(cmd: &ServiceCommand) -> anyhow::Result<()> {
    crate::service::handle_command(&cmd.to_action())
}
