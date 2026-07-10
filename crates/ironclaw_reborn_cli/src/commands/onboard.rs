use std::path::{Path, PathBuf};

use clap::Args;
use ironclaw_reborn_config::RebornHome;

use crate::commands::config::init::{ExistingConfigPolicy, write_default_config_files};
use crate::context::{RebornCliContext, V1MigrationSourceCandidate};
use crate::file_write::{FileWriteAction, write_atomic};

const ONBOARDING_MARKER_FILE: &str = ".onboard-completed.json";

/// Initialize the standalone Reborn home and first-run setup marker.
#[derive(Debug, Args)]
pub(crate) struct OnboardCommand {
    /// Overwrite generated config.toml, providers.json, and the completion marker.
    #[arg(long = "force")]
    force: bool,

    /// Show what would be initialized without writing files.
    #[arg(long = "dry-run")]
    dry_run: bool,

    /// Inventory a detected v1 installation and write a migration plan.
    #[arg(long = "migrate-v1", conflicts_with = "skip_v1_migration")]
    migrate_v1: bool,

    /// Deprecated alias for --migrate-v1.
    #[arg(
        long = "import-history",
        hide = true,
        conflicts_with_all = ["migrate_v1", "skip_v1_migration"]
    )]
    import_history: bool,

    /// Record that v1 migration was explicitly skipped during onboarding.
    #[arg(long = "skip-v1-migration", conflicts_with = "migrate_v1")]
    skip_v1_migration: bool,
}

impl OnboardCommand {
    pub(crate) fn execute(self, context: RebornCliContext) -> anyhow::Result<()> {
        let home = context.boot_config().home();
        let marker_path = onboarding_marker_path(home);
        let source = context.v1_migration_source_candidate();
        let source_detected = source.is_some();
        let migration_requested = self.migrate_v1 || self.import_history;
        let manifest_path = default_migration_manifest_path(home);

        if self.import_history {
            eprintln!("warning: --import-history is deprecated; use --migrate-v1");
        }

        if self.dry_run {
            print_dry_run(
                home,
                &marker_path,
                self.force,
                migration_requested,
                self.skip_v1_migration,
                source.as_ref(),
                &manifest_path,
            );
            return Ok(());
        }

        let outcome = write_default_config_files(home, self.force, ExistingConfigPolicy::Preserve)?;
        let migration_state = if self.skip_v1_migration {
            "explicitly_skipped"
        } else if migration_requested {
            let source = source.ok_or_else(|| {
                anyhow::anyhow!(
                    "--migrate-v1 was requested, but no v1 source was detected; set MIGRATION_SOURCE_POSTGRES or place a stopped-source snapshot at $IRONCLAW_BASE_DIR/ironclaw.db, then run `ironclaw-reborn migrate v1 plan --help`"
                )
            })?;
            crate::commands::migrate::plan_detected_v1(source, &manifest_path)?;
            "planned"
        } else if source_detected {
            "available"
        } else {
            "not_detected"
        };
        let marker_action = write_onboarding_marker(
            home,
            &marker_path,
            self.force || migration_requested || self.skip_v1_migration,
            migration_state,
            &manifest_path,
        )?;

        println!("IronClaw Reborn onboarding");
        println!("reborn_home: {}", home.path().display());
        println!("home_source: {}", home.source_label());
        println!("{}", outcome.config.display_line());
        println!("{}", outcome.providers.display_line());
        println!(
            "onboarding_marker: {} ({})",
            marker_path.display(),
            marker_action
        );
        println!("v1_state: not-used");
        println!("v1_migration_state: {migration_state}");
        println!();
        println!("completed:");
        println!("- reborn home initialized");
        println!("- config.toml and providers.json available");
        println!("- onboarding completion marker available");
        println!();
        println!("remaining:");
        println!("- configure LLM credentials through env vars referenced by config.toml");
        println!(
            "- run `ironclaw-reborn models set-provider <provider> --model <model>` as needed"
        );
        match migration_state {
            "available" => println!(
                "- v1 data detected; review `ironclaw-reborn migrate v1 plan --help` before cutover"
            ),
            "planned" => println!(
                "- review the v1 migration plan at {} before stopping v1 and applying",
                manifest_path.display()
            ),
            "explicitly_skipped" => println!("- v1 migration explicitly skipped"),
            _ => println!("- no v1 installation detected"),
        }
        Ok(())
    }
}

pub(crate) fn onboarding_marker_path(home: &RebornHome) -> PathBuf {
    home.path().join(ONBOARDING_MARKER_FILE)
}

pub(crate) fn default_migration_manifest_path(home: &RebornHome) -> PathBuf {
    home.path().join("v1-migration-manifest.json")
}

fn print_dry_run(
    home: &RebornHome,
    marker_path: &Path,
    force: bool,
    migration_requested: bool,
    migration_skipped: bool,
    source: Option<&V1MigrationSourceCandidate>,
    manifest_path: &Path,
) {
    println!("IronClaw Reborn onboarding dry run");
    println!("reborn_home: {}", home.path().display());
    println!("home_source: {}", home.source_label());
    println!("would_ensure: {}", home.path().display());
    println!(
        "would_write_or_preserve: {}",
        home.config_file_path().display()
    );
    println!(
        "would_write_or_preserve: {}",
        home.providers_file_path().display()
    );
    let marker_action = if marker_path.exists() && !force {
        "would_preserve"
    } else {
        "would_write"
    };
    println!("{marker_action}: {}", marker_path.display());
    println!("migrate_v1_requested: {migration_requested}");
    println!("skip_v1_migration_requested: {migration_skipped}");
    println!(
        "v1_migration_state: {}",
        if migration_skipped {
            "explicitly_skipped"
        } else if migration_requested && source.is_some() {
            "would_plan"
        } else if source.is_some() {
            "available"
        } else {
            "not_detected"
        }
    );
    if migration_requested && source.is_some() {
        println!(
            "would_write_migration_manifest: {}",
            manifest_path.display()
        );
    } else if migration_requested {
        println!("migration_plan_blocker: no v1 source detected");
    }
    println!("v1_state: not-used");
}

fn write_onboarding_marker(
    home: &RebornHome,
    marker_path: &Path,
    force: bool,
    migration_state: &'static str,
    manifest_path: &Path,
) -> anyhow::Result<FileWriteAction> {
    if marker_path.exists() && !force {
        return Ok(FileWriteAction::Preserved);
    }
    let body = serde_json::to_string_pretty(&serde_json::json!({
        "schema_version": "ironclaw.reborn.onboarding/v2",
        "completed_at": chrono::Utc::now().to_rfc3339(),
        "reborn_home": home.path(),
        "home_source": home.source_label(),
        "config_file": home.config_file_path(),
        "providers_file": home.providers_file_path(),
        "steps_completed": [
            "reborn_home",
            "config_files",
            "completion_marker"
        ],
        "steps_pending": pending_steps(migration_state),
        "v1_state": "not-used",
        "v1_migration": {
            "state": migration_state,
            "manifest": manifest_path,
        }
    }))?;
    write_atomic(
        marker_path,
        &format!("{body}\n"),
        force,
        ONBOARDING_MARKER_FILE,
    )
}

fn pending_steps(migration_state: &str) -> Vec<&'static str> {
    let mut steps = vec!["llm_credentials", "model_selection", "channel_setup"];
    if matches!(migration_state, "available" | "planned") {
        steps.push("v1_migration");
    }
    steps
}
