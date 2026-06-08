use anyhow::Context;
use clap::{Args, Subcommand, ValueEnum};
use ironclaw_reborn_composition::{
    IronHubCommand as RebornIronHubCommand, IronHubEntryKind, IronHubInstallOptions,
    build_reborn_services, execute_reborn_ironhub_command, render_reborn_ironhub_response,
};

use crate::context::RebornCliContext;
use crate::runtime::{RuntimeInputCaller, RuntimeInputOptions};

#[derive(Debug, Args)]
pub(crate) struct IronHubCommand {
    /// Confirm trusted-laptop host filesystem access for local-dev-yolo.
    #[arg(long = "confirm-host-access", global = true)]
    confirm_host_access: bool,

    #[command(subcommand)]
    command: IronHubSubcommand,
}

#[derive(Debug, Subcommand)]
enum IronHubSubcommand {
    /// Search the signed IronHub catalog.
    Search(IronHubSearchCommand),
    /// List available IronHub tools or skills.
    List(IronHubListCommand),
    /// Show one IronHub catalog entry.
    Info(IronHubInfoCommand),
    /// Install an IronHub tool or skill into Reborn local-dev state.
    Install(IronHubInstallCommand),
}

#[derive(Debug, Args)]
struct IronHubSearchCommand {
    /// Optional query by name or description. Omit to list all entries.
    query: Option<String>,

    /// Output the lifecycle response as JSON.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args)]
struct IronHubListCommand {
    /// Limit results to tools or skills.
    #[arg(long, value_enum)]
    kind: Option<IronHubKindArg>,

    /// Output the lifecycle response as JSON.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args)]
struct IronHubInfoCommand {
    /// Tool or skill name.
    name: String,

    /// Disambiguate when a name exists as both a tool and a skill.
    #[arg(long, value_enum)]
    kind: Option<IronHubKindArg>,

    /// Output the lifecycle response as JSON.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args)]
struct IronHubInstallCommand {
    /// Tool or skill name.
    name: String,

    /// Disambiguate when a name exists as both a tool and a skill.
    #[arg(long, value_enum)]
    kind: Option<IronHubKindArg>,

    /// Replace an already installed package.
    #[arg(long)]
    force: bool,

    /// Acknowledge installing unverified community content.
    #[arg(long)]
    acknowledge_unverified: bool,

    /// Require the catalog entry to still have this version.
    #[arg(long)]
    expected_version: Option<String>,

    /// Require the catalog entry to still have this artifact digest.
    #[arg(long)]
    expected_artifact_digest: Option<String>,

    /// Output the lifecycle response as JSON.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum IronHubKindArg {
    Tool,
    Skill,
}

impl IronHubCommand {
    pub(crate) fn execute(self, context: RebornCliContext) -> anyhow::Result<()> {
        crate::runtime::init_tracing();
        let (command, json, label) = match self.command {
            IronHubSubcommand::Search(command) => (
                RebornIronHubCommand::Search {
                    query: command.query.unwrap_or_default(),
                },
                command.json,
                "search",
            ),
            IronHubSubcommand::List(command) => (
                RebornIronHubCommand::List {
                    kind: command.kind.map(Into::into),
                },
                command.json,
                "list",
            ),
            IronHubSubcommand::Info(command) => (
                RebornIronHubCommand::Info {
                    name: command.name,
                    kind: command.kind.map(Into::into),
                },
                command.json,
                "info",
            ),
            IronHubSubcommand::Install(command) => (
                RebornIronHubCommand::Install {
                    name: command.name,
                    options: IronHubInstallOptions {
                        kind: command.kind.map(Into::into),
                        force: command.force,
                        acknowledge_unverified: command.acknowledge_unverified,
                        expected_version: command.expected_version,
                        expected_artifact_digest: command.expected_artifact_digest,
                    },
                },
                command.json,
                "install",
            ),
        };
        let response = execute_ironhub_command(context, command, self.confirm_host_access)?;
        if json {
            println!("{}", serde_json::to_string(&response)?);
        } else {
            print!("{}", render_reborn_ironhub_response(label, &response));
        }
        Ok(())
    }
}

impl From<IronHubKindArg> for IronHubEntryKind {
    fn from(value: IronHubKindArg) -> Self {
        match value {
            IronHubKindArg::Tool => Self::Tool,
            IronHubKindArg::Skill => Self::Skill,
        }
    }
}

fn execute_ironhub_command(
    context: RebornCliContext,
    command: RebornIronHubCommand,
    confirm_host_access: bool,
) -> anyhow::Result<ironclaw_reborn_composition::LifecycleProductResponse> {
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
        .context("failed to build tokio runtime for IronHub command")?;
    runtime.block_on(async move {
        let services = build_reborn_services(runtime_services.services_input)
            .await
            .context("failed to assemble Reborn services for IronHub command")?;
        execute_reborn_ironhub_command(&services, command)
            .await
            .map_err(anyhow::Error::from)
    })
}
