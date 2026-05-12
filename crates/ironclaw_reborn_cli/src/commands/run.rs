use clap::Args;
use ironclaw_reborn_config::RebornBootConfig;

use crate::context::RebornCliContext;

#[derive(Debug, Args)]
pub(crate) struct RunCommand;

impl RunCommand {
    pub(crate) fn execute(self, context: RebornCliContext) -> anyhow::Result<()> {
        RuntimeShellReport::initialize(context).print();
        Ok(())
    }
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
