use clap::{Args, Subcommand};

use crate::context::RebornCliContext;

pub(crate) mod init;

#[derive(Debug, Args)]
pub(crate) struct ConfigCommand {
    #[command(subcommand)]
    command: ConfigSubcommand,
}

#[derive(Debug, Subcommand)]
enum ConfigSubcommand {
    /// Show resolved Reborn configuration paths without creating state.
    Path(ConfigPathCommand),
    /// Write a commented stub `config.toml` and `providers.json` into
    /// the Reborn home directory. Refuses to clobber unless --force.
    Init(init::ConfigInitCommand),
    /// List all configuration keys and their values.
    List(ConfigListCommand),
    /// Get a single configuration value by dot-separated key.
    Get(ConfigGetCommand),
    /// Set a single configuration value by dot-separated key, routed
    /// through the alias table to its destination (config.toml, the
    /// encrypted secret store, or the WebChat v2 token file).
    Set(set::ConfigSetCommand),
}

#[derive(Debug, Args)]
struct ConfigPathCommand;

mod capability_config;
mod get;
mod list;
mod read;
mod set;

use self::get::ConfigGetCommand;
use self::list::ConfigListCommand;

impl ConfigCommand {
    pub(crate) fn execute(self, context: RebornCliContext) -> anyhow::Result<()> {
        match self.command {
            ConfigSubcommand::Path(command) => command.execute(context),
            ConfigSubcommand::Init(command) => command.execute(context),
            ConfigSubcommand::List(command) => command.execute(context),
            ConfigSubcommand::Get(command) => command.execute(context),
            ConfigSubcommand::Set(command) => command.execute(context),
        }
    }
}

impl ConfigPathCommand {
    fn execute(self, context: RebornCliContext) -> anyhow::Result<()> {
        let report =
            ironclaw_reborn_config::RebornDoctorReport::from_config(context.boot_config().clone());
        let home = context.boot_config().home();

        let config_path = home.config_file_path();
        let providers_path = home.providers_file_path();
        let exists = |path: &std::path::Path| {
            if path.exists() {
                "present"
            } else {
                "absent (optional; falls back to defaults)"
            }
        };

        println!("IronClaw Reborn config path");
        println!("reborn_home: {}", report.home_path().display());
        println!("home_source: {}", report.home_source_label());
        println!("profile: {}", report.profile());
        println!(
            "config_file: {} ({})",
            config_path.display(),
            exists(&config_path)
        );
        println!(
            "providers: {} ({})",
            providers_path.display(),
            exists(&providers_path)
        );
        println!("v1_state: {}", report.v1_state());
        Ok(())
    }
}
