use clap::Args;
use ironclaw_reborn_config::RebornBootConfig;

use crate::context::RebornCliContext;

#[derive(Debug, Args)]
pub(crate) struct RunCommand;

impl RunCommand {
    pub(crate) fn execute(self, context: RebornCliContext) -> anyhow::Result<()> {
        RuntimeReadinessSnapshot::initialize(context).print();
        Ok(())
    }
}

/// Side-effect-free runtime readiness snapshot for the standalone Reborn binary.
#[derive(Debug, Clone)]
struct RuntimeReadinessSnapshot {
    config: RebornBootConfig,
    text_only_driver: ComponentStatus,
    planned_driver: ComponentStatus,
    planned_default_profile: ComponentStatus,
}

#[derive(Debug, Clone)]
enum ComponentStatus {
    Initialized,
    Failed(String),
}

impl ComponentStatus {
    fn from_result<T, E: std::fmt::Display>(result: Result<T, E>) -> Self {
        match result {
            Ok(_) => Self::Initialized,
            Err(error) => Self::Failed(error.to_string()),
        }
    }

    fn is_initialized(&self) -> bool {
        matches!(self, Self::Initialized)
    }

    fn render(&self, ok_label: &str) -> String {
        match self {
            Self::Initialized => ok_label.to_string(),
            Self::Failed(reason) => format!("unavailable: {reason}"),
        }
    }
}

impl RuntimeReadinessSnapshot {
    fn initialize(context: RebornCliContext) -> Self {
        let mut registry = ironclaw_reborn::driver_registry::DriverRegistry::new();
        let text_only_driver =
            ComponentStatus::from_result(ironclaw_reborn::register_default_text_only_driver(
                &mut registry,
                ironclaw_reborn::TextOnlyModelReplyDriverConfig::default(),
            ));
        let planned_driver = match ironclaw_reborn::build_loop_family_registry() {
            Ok(family_registry) => ComponentStatus::from_result(
                ironclaw_reborn::register_default_planned_driver(&mut registry, family_registry),
            ),
            Err(error) => ComponentStatus::Failed(error.to_string()),
        };
        let planned_default_profile =
            ComponentStatus::from_result(ironclaw_reborn::default_planned_run_profile_resolver());
        Self {
            config: context.boot_config().clone(),
            text_only_driver,
            planned_driver,
            planned_default_profile,
        }
    }

    fn print(&self) {
        println!("IronClaw Reborn runtime readiness snapshot");
        println!("binary: ironclaw-reborn");
        println!("version: {}", env!("CARGO_PKG_VERSION"));
        println!("reborn_home: {}", self.config.home().path().display());
        println!("home_source: {}", self.config.home().source_label());
        println!("profile: {}", self.config.profile());
        println!("v1_state: not-used");
        println!(
            "text_only_driver: {}",
            self.text_only_driver.render("initialized")
        );
        println!(
            "planned_driver: {}",
            self.planned_driver.render("initialized")
        );
        let driver_registry_initialized =
            self.text_only_driver.is_initialized() && self.planned_driver.is_initialized();
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
            if driver_registry_initialized && self.planned_default_profile.is_initialized() {
                "ready"
            } else {
                "unavailable"
            }
        );
        println!(
            "planned_default_profile: {}",
            self.planned_default_profile.render("available")
        );
    }
}
