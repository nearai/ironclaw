use std::{env, path::PathBuf};

use clap::{Parser, Subcommand};

const REBORN_HOME_ENV: &str = "IRONCLAW_REBORN_HOME";

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
    let home = reborn_home();
    let _registry = ironclaw_reborn::driver_registry::DriverRegistry::new();

    println!("IronClaw Reborn doctor");
    println!("reborn_home: {}", home.display());
    println!("home_source: {}", home_source());
    println!("v1_state: not-used");
    println!("driver_registry: initialized");
    Ok(())
}

fn reborn_home() -> PathBuf {
    env::var_os(REBORN_HOME_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(default_reborn_home)
}

fn home_source() -> &'static str {
    if env::var_os(REBORN_HOME_ENV).is_some() {
        REBORN_HOME_ENV
    } else {
        "default"
    }
}

fn default_reborn_home() -> PathBuf {
    home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".ironclaw")
        .join("reborn")
}

fn home_dir() -> Option<PathBuf> {
    env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}
