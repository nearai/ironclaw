use anyhow::Context;
use clap::{Args, Subcommand, ValueEnum};
use ironclaw_composition::{RebornRuntimeInput, build_reborn_runtime};
use ironclaw_extension_manager::ironhub::{
    IronHubCommand as RebornIronHubCommand, IronHubEntryKind, IronHubInstallOptions,
    IronHubResponse, execute_reborn_ironhub_command, render_reborn_ironhub_response,
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
    /// Install an IronHub tool or skill into Reborn state.
    Install(IronHubInstallCommand),
}

#[derive(Debug, Args)]
struct IronHubSearchCommand {
    /// Optional query by name or description. Omit to list all entries.
    query: Option<String>,
    /// Output the response as JSON.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args)]
struct IronHubListCommand {
    /// Limit results to tools or skills.
    #[arg(long, value_enum)]
    kind: Option<IronHubKindArg>,
    /// Output the response as JSON.
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
    /// Output the response as JSON.
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
    /// Replace an existing registry package.
    #[arg(long)]
    force: bool,
    /// Acknowledge installing unverified community content.
    #[arg(long)]
    acknowledge_unverified: bool,
    /// Require the catalog entry to still have this version.
    #[arg(long)]
    expected_version: Option<String>,
    /// Require the catalog entry to still have this signed artifact digest.
    #[arg(long)]
    expected_artifact_digest: Option<String>,
    /// Install from an org-scoped signed manifest URL read from this file.
    ///
    /// Reading the URL from a file keeps its access token out of argv and shell
    /// history.
    #[arg(long, value_name = "PATH")]
    private_manifest_url_file: Option<std::path::PathBuf>,
    /// Output the response as JSON.
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
                        private_manifest_url: read_private_manifest_url(
                            command.private_manifest_url_file.as_deref(),
                        )?,
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

fn read_private_manifest_url(path: Option<&std::path::Path>) -> anyhow::Result<Option<String>> {
    let Some(path) = path else {
        return Ok(None);
    };
    let url = std::fs::read_to_string(path)
        .with_context(|| format!("reading private manifest URL file {}", path.display()))?
        .trim()
        .to_string();
    if url.is_empty() {
        anyhow::bail!("private manifest URL file {} is empty", path.display());
    }
    Ok(Some(url))
}

fn execute_ironhub_command(
    context: RebornCliContext,
    command: RebornIronHubCommand,
    confirm_host_access: bool,
) -> anyhow::Result<IronHubResponse> {
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
        crate::runtime::initialize_local_runtime_storage_root(
            context.boot_config(),
            runtime_services.profile,
        )
        .await?;
        let services_input =
            crate::runtime::with_binary_host_extension_bindings(runtime_services.services_input)?;
        let mut runtime_input = RebornRuntimeInput::from_build_input(services_input);
        if let Some(manifest_url) = crate::runtime::ironhub_manifest_url_from_env()? {
            runtime_input = runtime_input.with_ironhub_manifest_url(manifest_url);
        }
        let runtime = build_reborn_runtime(runtime_input)
            .await
            .context("failed to assemble Reborn runtime for IronHub command")?;
        let command_result = execute_reborn_ironhub_command(&runtime, command)
            .await
            .map_err(anyhow::Error::from);
        let shutdown_result = runtime
            .shutdown()
            .await
            .context("failed to shut down Reborn runtime after IronHub command");
        reconcile_command_and_shutdown(command_result, shutdown_result)
    })
}

fn reconcile_command_and_shutdown(
    command_result: anyhow::Result<IronHubResponse>,
    shutdown_result: anyhow::Result<()>,
) -> anyhow::Result<IronHubResponse> {
    match (command_result, shutdown_result) {
        (Ok(response), Ok(())) => Ok(response),
        (Err(command_error), Ok(())) => Err(command_error),
        (Ok(_), Err(shutdown_error)) => Err(shutdown_error),
        (Err(command_error), Err(shutdown_error)) => {
            tracing::debug!(
                %shutdown_error,
                "Reborn runtime shutdown also failed after IronHub command failure"
            );
            Err(command_error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_manifest_url_is_trimmed_when_read_from_file() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("private-manifest-url");
        std::fs::write(&path, "  https://catalog.example/private?token=secret\n")
            .expect("write URL file");

        assert_eq!(
            read_private_manifest_url(Some(&path)).expect("read URL"),
            Some("https://catalog.example/private?token=secret".to_string())
        );
    }

    #[test]
    fn absent_private_manifest_url_file_stays_absent() {
        assert_eq!(read_private_manifest_url(None).expect("optional URL"), None);
    }

    #[test]
    fn empty_private_manifest_url_file_is_rejected() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("private-manifest-url");
        std::fs::write(&path, "  \n").expect("write URL file");

        let error = read_private_manifest_url(Some(&path))
            .expect_err("an empty private-manifest URL must fail closed");
        assert!(error.to_string().contains("is empty"));
    }

    #[test]
    fn command_and_shutdown_results_preserve_the_authoritative_error() {
        let response = || {
            serde_json::from_value(serde_json::json!({
                "phase": "discovered",
                "total_entries": 0,
                "returned_entries": 0,
                "truncated": false,
                "catalog_total": 0,
                "entries": []
            }))
            .expect("fixture response")
        };
        assert!(reconcile_command_and_shutdown(Ok(response()), Ok(())).is_ok());

        let command_error =
            reconcile_command_and_shutdown(Err(anyhow::anyhow!("command failed")), Ok(()))
                .expect_err("command failure must propagate");
        assert_eq!(command_error.to_string(), "command failed");

        let shutdown_error =
            reconcile_command_and_shutdown(Ok(response()), Err(anyhow::anyhow!("shutdown failed")))
                .expect_err("shutdown failure must propagate after command success");
        assert_eq!(shutdown_error.to_string(), "shutdown failed");

        let command_error = reconcile_command_and_shutdown(
            Err(anyhow::anyhow!("command failed first")),
            Err(anyhow::anyhow!("shutdown failed second")),
        )
        .expect_err("command failure remains authoritative when shutdown also fails");
        assert_eq!(command_error.to_string(), "command failed first");
    }

    #[test]
    fn install_command_assembles_runtime_and_rejects_invalid_name_without_network() {
        let _env_lock = crate::runtime::test_env::lock_runtime_env();
        let _manifest_url = crate::runtime::test_env::EnvGuard::set(
            "IRONHUB_MANIFEST_URL",
            "https://hub.ironclaw.com/manifest.json",
        );
        let directory = tempfile::tempdir().expect("tempdir");
        let private_url_path = directory.path().join("private-manifest-url");
        std::fs::write(
            &private_url_path,
            "https://hub.ironclaw.com/private/manifest\n",
        )
        .expect("write private URL file");
        let (_context_dir, context) = crate::context::RebornCliContext::test_context();
        let command = IronHubCommand {
            confirm_host_access: false,
            command: IronHubSubcommand::Install(IronHubInstallCommand {
                name: "../invalid".to_string(),
                kind: Some(IronHubKindArg::Skill),
                force: false,
                acknowledge_unverified: false,
                expected_version: None,
                expected_artifact_digest: None,
                private_manifest_url_file: Some(private_url_path),
                json: true,
            }),
        };

        let error = command
            .execute(context)
            .expect_err("invalid package name must fail before network access");
        assert!(
            format!("{error:#}").contains("name must be 1-128 bytes"),
            "unexpected error: {error:#}"
        );
    }
}
