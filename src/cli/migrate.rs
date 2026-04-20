//! CLI command for migrating from external assistants into IronClaw.

use std::path::PathBuf;

use clap::Subcommand;

use crate::migrate::{MigrationServices, MigrationStats, hermes, openclaw};

/// Migrate data from external assistants into IronClaw.
#[derive(Subcommand, Debug, Clone)]
pub enum MigrateCommand {
    /// Migrate an OpenClaw home directory into IronClaw.
    Openclaw {
        /// Path to the OpenClaw home directory (defaults to ~/.openclaw).
        #[arg(long)]
        path: Option<PathBuf>,

        /// Preview what would be imported without writing any state.
        #[arg(long)]
        dry_run: bool,

        /// Target IronClaw user scope (defaults to config owner_id).
        #[arg(long)]
        user_id: Option<String>,
    },

    /// Migrate a Hermes Agent home/profile into IronClaw.
    Hermes {
        /// Path to the Hermes home directory (defaults to ~/.hermes).
        #[arg(long)]
        path: Option<PathBuf>,

        /// One or more named Hermes profiles to import (repeatable).
        #[arg(long = "profile")]
        profiles: Vec<String>,

        /// Import the default Hermes root plus every named profile under profiles/.
        #[arg(long)]
        all_profiles: bool,

        /// Preview what would be imported without writing any state.
        #[arg(long)]
        dry_run: bool,

        /// Target IronClaw user scope (defaults to config owner_id).
        #[arg(long)]
        user_id: Option<String>,
    },
}

pub async fn run_migrate_command(
    cmd: &MigrateCommand,
    config: &crate::config::Config,
) -> anyhow::Result<()> {
    let user_id = migrate_user_id(cmd, config);
    let services = MigrationServices::from_config(config, user_id).await?;
    let stats = run_migrate_command_with_services(cmd, &services).await?;
    print_summary(cmd, &stats);
    Ok(())
}

#[doc(hidden)]
pub async fn run_migrate_command_with_services(
    cmd: &MigrateCommand,
    services: &MigrationServices,
) -> anyhow::Result<MigrationStats> {
    let stats = match cmd {
        MigrateCommand::Openclaw { path, dry_run, .. } => {
            let path = path.clone().or_else(openclaw::detect).unwrap_or_else(|| {
                std::env::var("HOME")
                    .map(PathBuf::from)
                    .unwrap_or_else(|_| PathBuf::from("."))
                    .join(".openclaw")
            });
            openclaw::migrate(
                services,
                &openclaw::OpenClawMigrationOptions {
                    path,
                    dry_run: *dry_run,
                },
            )
            .await?
        }
        MigrateCommand::Hermes {
            path,
            profiles,
            all_profiles,
            dry_run,
            ..
        } => {
            let path = path.clone().or_else(hermes::detect).unwrap_or_else(|| {
                std::env::var("HOME")
                    .map(PathBuf::from)
                    .unwrap_or_else(|_| PathBuf::from("."))
                    .join(".hermes")
            });
            hermes::migrate(
                services,
                &hermes::HermesMigrationOptions {
                    path,
                    dry_run: *dry_run,
                    profiles: profiles.clone(),
                    all_profiles: *all_profiles,
                },
            )
            .await?
        }
    };

    Ok(stats)
}

fn migrate_user_id(cmd: &MigrateCommand, config: &crate::config::Config) -> String {
    match cmd {
        MigrateCommand::Openclaw { user_id, .. } | MigrateCommand::Hermes { user_id, .. } => {
            user_id.clone().unwrap_or_else(|| config.owner_id.clone())
        }
    }
}

fn print_summary(cmd: &MigrateCommand, stats: &MigrationStats) {
    let source_name = match cmd {
        MigrateCommand::Openclaw { .. } => "OpenClaw",
        MigrateCommand::Hermes { .. } => "Hermes",
    };

    println!("Migration complete: {source_name}");
    println!();
    println!("Summary:");
    println!("  Workspace docs:       {}", stats.workspace_documents);
    println!("  Memory docs:          {}", stats.memory_docs);
    println!("  Engine threads:       {}", stats.engine_threads);
    println!("  Engine conversations: {}", stats.engine_conversations);
    println!("  Legacy conversations: {}", stats.legacy_conversations);
    println!("  Messages:             {}", stats.messages);
    println!("  Settings:             {}", stats.settings);
    println!("  Secrets:              {}", stats.secrets);
    if stats.projects > 0 {
        println!("  Projects created:     {}", stats.projects);
    }
    if stats.skipped > 0 {
        println!("  Skipped:              {}", stats.skipped);
    }
    println!();
    println!("Total migrated: {}", stats.total_imported());

    match cmd {
        MigrateCommand::Openclaw { dry_run, .. } | MigrateCommand::Hermes { dry_run, .. } => {
            if *dry_run {
                println!();
                println!("[DRY RUN] No data was written.");
            }
        }
    }

    if !stats.notes.is_empty() {
        println!();
        println!("Notes:");
        for note in &stats.notes {
            println!("  - {}", note);
        }
    }
}
