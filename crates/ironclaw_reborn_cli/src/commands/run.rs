use clap::Args;
use ironclaw_reborn_config::RebornBootConfig;

use crate::context::RebornCliContext;

#[derive(Debug, Args)]
pub(crate) struct RunCommand;

impl RunCommand {
    pub(crate) fn execute(self, context: RebornCliContext) -> anyhow::Result<()> {
        if try_serve_telegram_v2()? {
            // serve_from_env returns Ok(()) on graceful axum shutdown; the
            // runtime-shell snapshot is only meaningful when no channel
            // booted, so skip it in that case.
            return Ok(());
        }
        RuntimeShellReport::initialize(context).print();
        Ok(())
    }
}

/// If a Reborn channel is configured in env, boot it and block on its serve
/// loop until shutdown. Returns `true` when a channel ran (and exited), or
/// `false` when none was configured and the caller should fall through to
/// the runtime-shell snapshot.
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
    ironclaw_reborn_telegram_v2_host::init_tracing();
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

/// Side-effect-free runtime-shell snapshot for the standalone Reborn binary.
#[derive(Debug, Clone)]
struct RuntimeShellReport {
    config: RebornBootConfig,
    driver_registry_initialized: bool,
}

impl RuntimeShellReport {
    fn initialize(context: RebornCliContext) -> Self {
        let _registry = ironclaw_reborn::driver_registry::DriverRegistry::new();
        Self {
            config: context.boot_config().clone(),
            driver_registry_initialized: true,
        }
    }

    fn print(&self) {
        println!("IronClaw Reborn runtime shell");
        println!("binary: ironclaw-reborn");
        println!("version: {}", env!("CARGO_PKG_VERSION"));
        println!("reborn_home: {}", self.config.home().path().display());
        println!("home_source: {}", self.config.home().source_label());
        println!("profile: {}", self.config.profile());
        println!("v1_state: not-used");
        println!("driver_registry: initialized");
        println!(
            "runtime_shell: {}",
            if self.driver_registry_initialized {
                "initialized"
            } else {
                "unavailable"
            }
        );
    }
}
