use clap::Args;
use ironclaw_reborn_composition::{
    RebornRuntimeComponentStatus, reborn_model_slot_names, reborn_runtime_readiness_snapshot,
};

use crate::context::RebornCliContext;
use crate::dto::{ComponentStatus, DriversSnapshot, FilePresence, StatusDto};
use crate::render::{self, OutputMode, Renderable, terminal_safe_text};
use std::io::Write;

#[derive(Debug, Args)]
pub(crate) struct StatusCommand {
    /// Output as JSON.
    #[arg(long)]
    json: bool,
}

impl StatusCommand {
    pub(crate) fn execute(self, context: RebornCliContext) -> anyhow::Result<()> {
        let dto = build_status_dto(&context)?;
        let mode = if self.json {
            OutputMode::Json
        } else {
            OutputMode::Text
        };
        render::output(&dto, mode)
    }
}

fn build_status_dto(context: &RebornCliContext) -> anyhow::Result<StatusDto> {
    let home = context.boot_config().home();
    let profile = context.boot_config().profile();
    let config_path = home.config_file_path();
    let providers_path = home.providers_file_path();

    let snapshot = reborn_runtime_readiness_snapshot();
    let model_slots = reborn_model_slot_names()
        .into_iter()
        .map(|s| s.to_string())
        .collect();

    Ok(StatusDto {
        version: env!("CARGO_PKG_VERSION").to_string(),
        reborn_home: home.path().to_path_buf(),
        home_source: home.source_label(),
        profile: profile.as_str().to_string(),
        config_file: FilePresence {
            present: config_path.exists(),
            path: config_path,
        },
        providers_file: FilePresence {
            present: providers_path.exists(),
            path: providers_path,
        },
        model_slots,
        drivers: DriversSnapshot {
            text_only: convert_component_status(&snapshot.text_only_driver),
            planned: convert_component_status(&snapshot.planned_driver),
            subagent_planned: convert_component_status(&snapshot.subagent_planned_driver),
            planned_default_profile: convert_component_status(&snapshot.planned_default_profile),
        },
    })
}

pub(super) fn convert_component_status(status: &RebornRuntimeComponentStatus) -> ComponentStatus {
    match status {
        RebornRuntimeComponentStatus::Initialized => ComponentStatus::Initialized,
        RebornRuntimeComponentStatus::Failed(reason) => ComponentStatus::Failed {
            reason: reason.clone(),
        },
    }
}

impl Renderable for StatusDto {
    fn render_text_to(&self, w: &mut impl Write) -> std::io::Result<()> {
        writeln!(w, "IronClaw Reborn status")?;
        writeln!(w)?;
        kv(w, "version", &self.version)?;
        kv(w, "reborn_home", &self.reborn_home.display().to_string())?;
        kv(w, "home_source", self.home_source)?;
        kv(w, "profile", &self.profile)?;
        kv(
            w,
            "config_file",
            &format!(
                "{} ({})",
                self.config_file.path.display(),
                if self.config_file.present {
                    "present"
                } else {
                    "absent"
                }
            ),
        )?;
        kv(
            w,
            "providers_file",
            &format!(
                "{} ({})",
                self.providers_file.path.display(),
                if self.providers_file.present {
                    "present"
                } else {
                    "absent"
                }
            ),
        )?;
        kv(w, "model_slots", &self.model_slots.join(", "))?;
        writeln!(w)?;
        writeln!(w, "drivers:")?;
        driver_line(w, "  text_only", &self.drivers.text_only)?;
        driver_line(w, "  planned", &self.drivers.planned)?;
        driver_line(w, "  subagent_planned", &self.drivers.subagent_planned)?;
        driver_line(
            w,
            "  planned_default_profile",
            &self.drivers.planned_default_profile,
        )?;
        Ok(())
    }
}

fn driver_line(w: &mut impl Write, label: &str, status: &ComponentStatus) -> std::io::Result<()> {
    match status {
        ComponentStatus::Initialized => writeln!(w, "{label}: initialized"),
        ComponentStatus::Failed { reason } => {
            writeln!(w, "{label}: unavailable ({})", terminal_safe_text(reason))
        }
    }
}

fn kv(w: &mut impl Write, key: &str, value: &str) -> std::io::Result<()> {
    writeln!(w, "{:<20} {value}", format!("{key}:"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::RebornCliContext;
    use ironclaw_reborn_composition::RebornRuntimeComponentStatus;

    #[test]
    fn status_dto_builds_without_config_file() {
        let (_tmp, context) = RebornCliContext::test_context();
        let dto = build_status_dto(&context).expect("must build");
        assert_eq!(dto.version, env!("CARGO_PKG_VERSION"));
        assert!(!dto.model_slots.is_empty());
    }

    #[test]
    fn convert_component_status_failed_maps_correctly() {
        let status = RebornRuntimeComponentStatus::Failed("db connection refused".to_string());
        let result = convert_component_status(&status);
        match result {
            ComponentStatus::Failed { reason } => {
                assert_eq!(reason, "db connection refused");
            }
            ComponentStatus::Initialized => panic!("expected Failed variant"),
        }
    }
}
