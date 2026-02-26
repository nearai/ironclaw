//! CLI import subcommand.

use std::path::PathBuf;
use std::sync::Arc;

use clap::Subcommand;

use crate::db::Database;
use crate::import::{
    ImportError, OpenClawConfig, OpenClawImporter, OpenClawInstallation, TerminalProgress,
};
use crate::secrets::SecretsStore;
use crate::workspace::Workspace;

#[derive(Subcommand, Debug, Clone)]
pub enum ImportCommand {
    /// Import data from an OpenClaw installation
    Openclaw {
        /// Path to the OpenClaw state directory (default: ~/.openclaw)
        #[arg(long)]
        path: Option<PathBuf>,

        /// Show what would be imported without writing anything
        #[arg(long)]
        dry_run: bool,
    },
}

/// Run an import command with pre-existing database and workspace handles.
pub async fn run_import_command_with_db(
    cmd: ImportCommand,
    db: Arc<dyn Database>,
    workspace: Option<Workspace>,
    secrets_store: Option<Arc<dyn SecretsStore + Send + Sync>>,
) -> anyhow::Result<()> {
    match cmd {
        ImportCommand::Openclaw { path, dry_run } => {
            run_openclaw_import(path, dry_run, db, workspace, secrets_store).await
        }
    }
}

async fn run_openclaw_import(
    path: Option<PathBuf>,
    dry_run: bool,
    db: Arc<dyn Database>,
    workspace: Option<Workspace>,
    secrets_store: Option<Arc<dyn SecretsStore + Send + Sync>>,
) -> anyhow::Result<()> {
    // Discover installation
    let installation = OpenClawInstallation::discover(path.as_deref()).map_err(|e| match e {
        ImportError::NotFound => {
            anyhow::anyhow!("No OpenClaw installation found. Use --path to specify the location.")
        }
        other => anyhow::anyhow!("{}", other),
    })?;

    println!(
        "Found OpenClaw installation at {}",
        installation.state_dir.display()
    );
    println!(
        "  Config: {}",
        if installation.config_file.is_file() {
            installation.config_file.display().to_string()
        } else {
            "(not found)".to_string()
        }
    );
    println!("  Identity files: {}", installation.identity_files.len());
    println!("  Memory files: {}", installation.memory_files.len());
    println!(
        "  Session dirs: {} (with {} .jsonl files)",
        installation.session_dirs.len(),
        installation
            .session_dirs
            .iter()
            .map(|s| s.jsonl_files.len())
            .sum::<usize>()
    );
    println!(
        "  OAuth file: {}",
        if installation.oauth_file.is_some() {
            "found"
        } else {
            "not found"
        }
    );
    println!();

    if dry_run {
        println!("Dry run mode: no data will be written.");
        println!();
    }

    // Parse config
    let config = OpenClawConfig::parse(
        &installation.config_file,
        installation.oauth_file.as_deref(),
    )
    .map_err(|e| anyhow::anyhow!("{}", e))?;

    // Run import
    let importer = OpenClawImporter::new(installation, config, dry_run);
    let mut progress = TerminalProgress::new();

    let report = importer
        .run(
            &db,
            workspace.as_ref(),
            secrets_store.as_ref(),
            &mut progress,
        )
        .await
        .map_err(|e| anyhow::anyhow!("{}", e))?;

    report.print_summary();

    Ok(())
}
