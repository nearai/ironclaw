use clap::{Parser, Subcommand};
use ironclaw_reborn_config::{RebornBootConfig, RebornDoctorReport};

#[derive(Debug, Parser)]
#[command(
    name = "ironclaw-reborn",
    about = "Standalone IronClaw Reborn runtime",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Check Reborn binary configuration without creating state.
    Doctor,
}

pub(crate) fn run() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Doctor => run_doctor(),
    }
}

fn run_doctor() -> anyhow::Result<()> {
    let report = RebornDoctorReport::from_config(RebornBootConfig::resolve_from_env()?);
    let _registry = ironclaw_reborn::driver_registry::DriverRegistry::new();

    println!("IronClaw Reborn doctor");
    println!("reborn_home: {}", report.home_path().display());
    println!("home_source: {}", report.home_source_label());
    println!("profile: {}", report.profile());
    println!("v1_state: {}", report.v1_state());
    println!("driver_registry: initialized");
    Ok(())
}
