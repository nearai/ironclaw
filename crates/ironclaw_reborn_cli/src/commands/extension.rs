use anyhow::Context;
use clap::{Args, Subcommand};
use ironclaw_reborn_composition::{
    LifecycleProductResponse, RebornExtensionLifecycleCommand, build_reborn_services,
    execute_reborn_extension_lifecycle_command, render_reborn_extension_lifecycle_response,
};

use crate::context::RebornCliContext;
use crate::runtime::{RuntimeInputCaller, RuntimeInputOptions};

#[derive(Debug, Args)]
pub(crate) struct ExtensionCommand {
    /// Confirm trusted-laptop host filesystem access for local-dev-yolo.
    #[arg(long = "confirm-host-access", global = true)]
    confirm_host_access: bool,

    #[command(subcommand)]
    command: ExtensionSubcommand,
}

#[derive(Debug, Subcommand)]
enum ExtensionSubcommand {
    /// Search local Reborn extension packages.
    Search(ExtensionSearchCommand),
    /// Install a local Reborn extension package.
    Install(ExtensionPackageCommand),
    /// Activate an installed local Reborn extension package.
    Activate(ExtensionPackageCommand),
    /// Remove an installed local Reborn extension package.
    Remove(ExtensionPackageCommand),
}

#[derive(Debug, Args)]
struct ExtensionSearchCommand {
    /// Query extension id, name, or description. Omit to list all local packages.
    query: Option<String>,

    /// Output the lifecycle response as JSON.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args)]
struct ExtensionPackageCommand {
    /// Extension id from `ironclaw extension search`.
    id: String,

    /// Output the lifecycle response as JSON.
    #[arg(long)]
    json: bool,
}

impl ExtensionCommand {
    pub(crate) fn execute(self, context: RebornCliContext) -> anyhow::Result<()> {
        crate::runtime::init_tracing();
        let (command, json, label) = match self.command {
            ExtensionSubcommand::Search(command) => (
                RebornExtensionLifecycleCommand::Search {
                    query: command.query.unwrap_or_default(),
                },
                command.json,
                "search",
            ),
            ExtensionSubcommand::Install(command) => (
                RebornExtensionLifecycleCommand::Install { id: command.id },
                command.json,
                "install",
            ),
            ExtensionSubcommand::Activate(command) => (
                RebornExtensionLifecycleCommand::Activate { id: command.id },
                command.json,
                "activate",
            ),
            ExtensionSubcommand::Remove(command) => (
                RebornExtensionLifecycleCommand::Remove { id: command.id },
                command.json,
                "remove",
            ),
        };
        let response = execute_lifecycle_command(context, command, self.confirm_host_access)?;
        if json {
            println!("{}", serde_json::to_string(&response)?);
        } else {
            print!(
                "{}",
                render_reborn_extension_lifecycle_response(label, &response)
            );
        }
        Ok(())
    }
}

fn execute_lifecycle_command(
    context: RebornCliContext,
    command: RebornExtensionLifecycleCommand,
    confirm_host_access: bool,
) -> anyhow::Result<LifecycleProductResponse> {
    let runtime_services = crate::runtime::build_services_input_with_options(
        context.boot_config(),
        RuntimeInputCaller::Run,
        RuntimeInputOptions {
            confirm_host_access,
        },
    )?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("failed to build tokio runtime for extension lifecycle command")?;
    runtime.block_on(async move {
        let services = build_reborn_services(runtime_services.services_input)
            .await
            .context("failed to assemble Reborn services for extension lifecycle command")?;
        execute_reborn_extension_lifecycle_command(&services, command)
            .await
            .map_err(anyhow::Error::from)
    })
}
