use ironclaw_reborn_composition::RebornRuntimeInput;

const DEFAULT_INTERVAL_MINUTES: u32 = 30;
const DEFAULT_FAILURE_LIMIT: u32 = 3;
const DEFAULT_TIMEZONE: &str = "UTC";

/// Promote the dependency-neutral TOML section through composition into the
/// trigger domain's validated heartbeat configuration.
pub(super) fn apply_heartbeat_config(
    input: RebornRuntimeInput,
    config_file: Option<&ironclaw_reborn_config::RebornConfigFile>,
) -> anyhow::Result<RebornRuntimeInput> {
    let Some(section) = config_file.and_then(|file| file.heartbeat.as_ref()) else {
        return Ok(input);
    };

    let (quiet_start, quiet_end) = section.quiet_hours.as_ref().map_or((None, None), |quiet| {
        (quiet.start.clone(), quiet.end.clone())
    });
    input
        .with_heartbeat_boot_config(
            section.enabled.unwrap_or(false),
            section.interval_minutes.unwrap_or(DEFAULT_INTERVAL_MINUTES),
            section
                .timezone
                .clone()
                .unwrap_or_else(|| DEFAULT_TIMEZONE.to_string()),
            quiet_start,
            quiet_end,
            section.delivery_target.clone(),
            section.failure_limit.unwrap_or(DEFAULT_FAILURE_LIMIT),
        )
        .map_err(anyhow::Error::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_reborn_config::{
        HeartbeatConfigSection, HeartbeatQuietHoursSection, RebornConfigFile,
    };

    #[test]
    fn absent_section_keeps_heartbeat_disabled_without_a_managed_record() {
        let input = apply_heartbeat_config(
            RebornRuntimeInput::default(),
            Some(&RebornConfigFile::default()),
        )
        .expect("valid config");
        assert!(input.heartbeat.is_none());
    }

    #[test]
    fn sparse_section_uses_safe_disabled_defaults() {
        let file = RebornConfigFile {
            heartbeat: Some(HeartbeatConfigSection::default()),
            ..Default::default()
        };
        let input = apply_heartbeat_config(RebornRuntimeInput::default(), Some(&file))
            .expect("valid config");
        assert!(input.heartbeat.is_some());
    }

    #[test]
    fn invalid_or_incomplete_quiet_hours_fail_boot() {
        let file = RebornConfigFile {
            heartbeat: Some(HeartbeatConfigSection {
                quiet_hours: Some(HeartbeatQuietHoursSection {
                    start: Some("22:00".to_string()),
                    end: None,
                }),
                ..Default::default()
            }),
            ..Default::default()
        };
        let error = match apply_heartbeat_config(RebornRuntimeInput::default(), Some(&file)) {
            Ok(_) => panic!("missing end must fail"),
            Err(error) => error,
        };
        assert!(
            error
                .to_string()
                .contains("quiet hours require both start and end")
        );
    }

    #[test]
    fn enabled_section_promotes_and_validates_all_fields() {
        let file = RebornConfigFile {
            heartbeat: Some(HeartbeatConfigSection {
                enabled: Some(true),
                interval_minutes: Some(60),
                timezone: Some("America/Los_Angeles".to_string()),
                quiet_hours: Some(HeartbeatQuietHoursSection {
                    start: Some("22:00".to_string()),
                    end: Some("07:00".to_string()),
                }),
                delivery_target: Some("target-1".to_string()),
                failure_limit: Some(5),
            }),
            ..Default::default()
        };
        let input = apply_heartbeat_config(RebornRuntimeInput::default(), Some(&file))
            .expect("valid config");
        assert!(input.heartbeat.is_some());
    }
}
