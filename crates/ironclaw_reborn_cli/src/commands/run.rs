use clap::Args;
use ironclaw_reborn_composition::reborn_runtime_readiness_snapshot;

use crate::context::RebornCliContext;
use crate::runtime::RuntimeInputOptions;

/// Start the standalone Reborn runtime. Sends `--message` if provided
/// (single-shot mode), otherwise drops into a stdin REPL.
#[derive(Debug, Args)]
pub(crate) struct RunCommand {
    /// Send a single message, print the assistant reply, and exit.
    /// Without this flag, the CLI reads lines from stdin in a loop.
    #[arg(short = 'm', long = "message")]
    message: Option<String>,

    /// Print the substrate readiness snapshot and exit without starting
    /// the agent. Preserves the legacy `run` diagnostic shape so existing
    /// smoke tests keep passing.
    #[arg(long = "dry-run")]
    dry_run: bool,

    /// Confirm trusted-laptop host filesystem access for local-dev-yolo.
    #[arg(long = "confirm-host-access")]
    confirm_host_access: bool,
}

impl RunCommand {
    pub(crate) fn execute(self, context: RebornCliContext) -> anyhow::Result<()> {
        crate::runtime::init_tracing();
        if self.dry_run {
            return run_dry(context);
        }

        crate::runtime::execute(
            context,
            self.message,
            RuntimeInputOptions {
                confirm_host_access: self.confirm_host_access,
            },
        )
    }
}

fn run_dry(context: RebornCliContext) -> anyhow::Result<()> {
    let config = context.boot_config();
    let readiness = reborn_runtime_readiness_snapshot();
    let driver_registry_initialized =
        readiness.text_only_driver.is_initialized() && readiness.planned_driver.is_initialized();
    println!("IronClaw Reborn runtime readiness snapshot");
    println!("binary: ironclaw");
    println!("version: {}", env!("CARGO_PKG_VERSION"));
    println!("reborn_home: {}", config.home().path().display());
    println!("home_source: {}", config.home().source_label());
    println!("profile: {}", config.profile());
    println!("v1_state: not-used");
    println!("runtime_driver: planned-agent-loop");
    println!(
        "text_only_driver: {}",
        readiness.text_only_driver.render("initialized")
    );
    println!(
        "planned_driver: {}",
        readiness.planned_driver.render("initialized")
    );
    println!(
        "driver_registry: {}",
        if driver_registry_initialized {
            "initialized"
        } else {
            "unavailable"
        }
    );
    println!(
        "local_runtime_shell_readiness: {}",
        if driver_registry_initialized && readiness.planned_default_profile.is_initialized() {
            "ready"
        } else {
            "unavailable"
        }
    );
    println!(
        "planned_default_profile: {}",
        readiness.planned_default_profile.render("available")
    );
    Ok(())
}
