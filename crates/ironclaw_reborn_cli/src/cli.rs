use std::{env, path::PathBuf};

use anyhow::{bail, ensure};
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
    let home = reborn_home()?;
    let _registry = ironclaw_reborn::driver_registry::DriverRegistry::new();

    println!("IronClaw Reborn doctor");
    println!("reborn_home: {}", home.path.display());
    println!("home_source: {}", home.source);
    println!("v1_state: not-used");
    println!("driver_registry: initialized");
    Ok(())
}

struct RebornHome {
    path: PathBuf,
    source: &'static str,
}

fn reborn_home() -> anyhow::Result<RebornHome> {
    if let Some(raw_home) = env::var_os(REBORN_HOME_ENV) {
        ensure!(
            !raw_home.as_os_str().is_empty(),
            "{REBORN_HOME_ENV} must not be empty"
        );
        let path = PathBuf::from(raw_home);
        ensure_absolute(&path, REBORN_HOME_ENV)?;
        return Ok(RebornHome {
            path,
            source: REBORN_HOME_ENV,
        });
    }

    let Some(home_dir) = home_dir() else {
        bail!("HOME or USERPROFILE must be set when {REBORN_HOME_ENV} is unset");
    };
    ensure_absolute(&home_dir, "home directory")?;

    Ok(RebornHome {
        path: home_dir.join(".ironclaw").join("reborn"),
        source: "default",
    })
}

fn ensure_absolute(path: &std::path::Path, label: &str) -> anyhow::Result<()> {
    ensure!(path.is_absolute(), "{label} must be an absolute path");
    Ok(())
}

fn home_dir() -> Option<PathBuf> {
    env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .filter(|home| !home.as_os_str().is_empty())
        .map(PathBuf::from)
}
