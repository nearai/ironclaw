use clap::Args;
use ironclaw_reborn_composition::{
    RebornRuntimeComponentStatus, reborn_model_slot_names, reborn_runtime_readiness_snapshot,
};

use crate::context::RebornCliContext;
use crate::dto::{ComponentStatus, DriversSnapshot, FilePresence, StatusDto};
use crate::render::{self, OutputMode};

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

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_reborn_config::RebornBootConfig;

    fn test_context() -> (tempfile::TempDir, crate::context::RebornCliContext) {
        let tmp = tempfile::tempdir().expect("tempdir");
        let config = RebornBootConfig::resolve_from_env_parts(
            None,
            Some(tmp.path().as_os_str().to_os_string()),
            None,
            None,
        )
        .expect("config must resolve with HOME set");
        (
            tmp,
            crate::context::RebornCliContext::from_boot_config(config),
        )
    }

    #[test]
    fn status_dto_builds_without_config_file() {
        let (_tmp, context) = test_context();
        let dto = build_status_dto(&context).expect("must build");
        assert_eq!(dto.version, env!("CARGO_PKG_VERSION"));
        assert!(!dto.model_slots.is_empty());
    }
}
