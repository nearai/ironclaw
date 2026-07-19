use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};

use clap::Args;
use ironclaw_reborn_config::RebornHome;

use crate::commands::config::init::{ExistingConfigPolicy, write_default_config_files};
use crate::context::RebornCliContext;
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

    /// Reserve the history-import step in the onboarding summary.
    ///
    /// History import is not wired in this slice; the flag makes the missing
    /// step explicit without touching v1 setup/import state.
    #[arg(long = "import-history")]
    import_history: bool,

    /// Scaffold only — never prompt to launch REPL/Web UI, even on a TTY.
    /// Use in scripts and CI.
    #[arg(long = "no-launch")]
    no_launch: bool,
}

/// Surface an operator can launch straight after onboarding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LaunchSurface {
    Repl,
    WebUi,
}

/// Map a prompt answer to a launch surface. `None` = skip / launch nothing.
fn parse_launch_choice(input: &str) -> Option<LaunchSurface> {
    match input.trim().to_ascii_lowercase().as_str() {
        "1" | "repl" | "r" => Some(LaunchSurface::Repl),
        "2" | "webui" | "web" | "w" => Some(LaunchSurface::WebUi),
        _ => None,
    }
}

impl OnboardCommand {
    pub(crate) fn execute(self, context: RebornCliContext) -> anyhow::Result<()> {
        let home = context.boot_config().home();
        let marker_path = onboarding_marker_path(home);

        if self.dry_run {
            print_dry_run(home, &marker_path, self.force, self.import_history);
            return Ok(());
        }

        let outcome = write_default_config_files(home, self.force, ExistingConfigPolicy::Preserve)?;
        let marker_action =
            write_onboarding_marker(home, &marker_path, self.force, self.import_history)?;

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
        println!();
        println!("completed:");
        println!("- reborn home initialized");
        println!("- config.toml and providers.json available");
        println!("- onboarding completion marker available");
        println!();
        println!("remaining:");
        println!("- choose a model (in the REPL prompt or the Web UI setup screen below)");
        if self.import_history {
            println!("- history import requested but not wired yet");
        } else {
            println!("- history import not requested");
        }

        // Offer to launch straight into a usable surface, where model setup
        // happens (Web UI setup screen / REPL prompt). Skipped for --no-launch
        // and non-interactive stdin so scripts/CI keep the scaffold-only path.
        if !self.no_launch && io::stdin().is_terminal() {
            if let Some(surface) = prompt_launch_surface()? {
                return launch_surface(surface);
            }
        } else {
            println!();
            println!("next: run `ironclaw-reborn repl` or `ironclaw-reborn serve` to finish setup");
        }
        Ok(())
    }
}

/// Whether this binary was compiled with the Web UI (`serve`) subcommand.
const WEBUI_AVAILABLE: bool = cfg!(feature = "webui-v2-beta");

fn prompt_launch_surface() -> anyhow::Result<Option<LaunchSurface>> {
    if WEBUI_AVAILABLE {
        print!("\nStart now?  [1] REPL   [2] Web UI   [Enter] skip: ");
    } else {
        print!(
            "\nStart now?  [1] REPL   [Enter] skip\n\
             (Web UI needs a build with `--features webui-v2-beta`): "
        );
    }
    io::stdout().flush()?;
    let mut line = String::new();
    if io::stdin().read_line(&mut line)? == 0 {
        return Ok(None); // EOF (piped input)
    }
    match parse_launch_choice(&line) {
        Some(LaunchSurface::WebUi) if !WEBUI_AVAILABLE => {
            println!(
                "Web UI is not available in this build. Rebuild with \
                 `cargo build --features webui-v2-beta`, or choose REPL."
            );
            Ok(None)
        }
        other => Ok(other),
    }
}

/// Re-exec this same binary into the chosen surface so onboarding hands off to a
/// live session (REPL prompt or Web UI, where the model is configured).
fn launch_surface(surface: LaunchSurface) -> anyhow::Result<()> {
    let exe = std::env::current_exe()?;
    let subcommand = match surface {
        LaunchSurface::Repl => "repl",
        LaunchSurface::WebUi => "serve",
    };
    println!("launching `{subcommand}`…");
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // exec replaces this process; only returns on failure.
        Err(std::process::Command::new(&exe)
            .arg(subcommand)
            .exec()
            .into())
    }
    #[cfg(not(unix))]
    {
        let status = std::process::Command::new(&exe).arg(subcommand).status()?;
        std::process::exit(status.code().unwrap_or(1));
    }
}

pub(crate) fn onboarding_marker_path(home: &RebornHome) -> PathBuf {
    home.path().join(ONBOARDING_MARKER_FILE)
}

fn print_dry_run(home: &RebornHome, marker_path: &Path, force: bool, import_history: bool) {
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
    println!("import_history_requested: {import_history}");
    println!("v1_state: not-used");
}

fn write_onboarding_marker(
    home: &RebornHome,
    marker_path: &Path,
    force: bool,
    import_history: bool,
) -> anyhow::Result<FileWriteAction> {
    if marker_path.exists() && !force {
        return Ok(FileWriteAction::Preserved);
    }
    let body = serde_json::to_string_pretty(&serde_json::json!({
        "schema_version": "ironclaw.reborn.onboarding/v1",
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
        "steps_pending": pending_steps(import_history),
        "v1_state": "not-used"
    }))?;
    write_atomic(
        marker_path,
        &format!("{body}\n"),
        force,
        ONBOARDING_MARKER_FILE,
    )
}

fn pending_steps(import_history: bool) -> Vec<&'static str> {
    let mut steps = vec!["llm_credentials", "model_selection", "channel_setup"];
    if import_history {
        steps.push("history_import");
    }
    steps
}

#[cfg(test)]
mod launch_choice_tests {
    use super::*;

    #[test]
    fn parses_repl_webui_and_skip_answers() {
        for a in ["1", "repl", "REPL", " r ", "\tRepl\n"] {
            assert_eq!(parse_launch_choice(a), Some(LaunchSurface::Repl), "{a:?}");
        }
        for a in ["2", "webui", "web", "W"] {
            assert_eq!(parse_launch_choice(a), Some(LaunchSurface::WebUi), "{a:?}");
        }
        for a in ["", "\n", "3", "quit", "xyz"] {
            assert_eq!(parse_launch_choice(a), None, "{a:?}");
        }
    }
}
