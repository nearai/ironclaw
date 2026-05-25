use clap::Args;
use ironclaw_reborn_composition::reborn_runtime_readiness_snapshot;

use crate::context::RebornCliContext;

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
}

impl RunCommand {
    pub(crate) fn execute(self, context: RebornCliContext) -> anyhow::Result<()> {
        crate::runtime::init_tracing();
        if self.dry_run {
            return run_dry(context);
        }

        if try_serve_telegram_v2()? {
            // serve_from_env returns Ok(()) on graceful axum shutdown; when
            // a channel booted we've finished the run lifecycle, so do not
            // also drop into the REPL/single-shot path.
            return Ok(());
        }

        crate::runtime::execute(context, self.message)
    }
}

/// If a Reborn channel is configured in env, boot it and block on its serve
/// loop until shutdown. Returns `true` when a channel ran (and exited), or
/// `false` when none was configured and the caller should fall through to
/// the REPL/single-shot runtime.
///
/// Future channels (Slack/Discord/WeChat per #3577) plug in here by adding
/// another `cfg(feature = "...")` arm that calls their host crate's
/// `serve_from_env`. Multiple channels in one process require composing
/// their `Router`s onto a single axum app — out of scope until the second
/// channel lands.
#[cfg(feature = "telegram-v2")]
fn try_serve_telegram_v2() -> anyhow::Result<bool> {
    if !ironclaw_reborn_telegram_v2_host::telegram_v2_configured_in_env() {
        return Ok(false);
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| anyhow::anyhow!("build tokio runtime: {e}"))?;
    runtime
        .block_on(ironclaw_reborn_telegram_v2_host::serve_from_env())
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(true)
}

#[cfg(not(feature = "telegram-v2"))]
fn try_serve_telegram_v2() -> anyhow::Result<bool> {
    Ok(false)
}

fn run_dry(context: RebornCliContext) -> anyhow::Result<()> {
    let config = context.boot_config();
    let readiness = reborn_runtime_readiness_snapshot();
    let driver_registry_initialized =
        readiness.text_only_driver.is_initialized() && readiness.planned_driver.is_initialized();
    println!("IronClaw Reborn runtime readiness snapshot");
    println!("binary: ironclaw-reborn");
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
