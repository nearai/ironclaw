#[cfg(feature = "slack-v2-host-beta")]
use std::path::Path;

#[cfg(feature = "slack-v2-host-beta")]
use ironclaw_reborn_composition::SlackHostBetaRuntimeConfig;

const SLACK_ENABLED_ENV: &str = "IRONCLAW_REBORN_SLACK_ENABLED";

#[cfg(feature = "slack-v2-host-beta")]
pub(crate) fn resolve_slack_config_for_serve(
    section: Option<&ironclaw_reborn_config::SlackSection>,
    tenant_id: &ironclaw_reborn_composition::host_api::TenantId,
    default_agent_id: &ironclaw_reborn_composition::host_api::AgentId,
    default_project_id: Option<&ironclaw_reborn_composition::host_api::ProjectId>,
    default_user_id: &ironclaw_reborn_composition::host_api::UserId,
    _config_path: &Path,
) -> anyhow::Result<Option<SlackHostBetaRuntimeConfig>> {
    let enabled = slack_enabled(section)?;
    if !enabled {
        return Ok(None);
    }
    let runtime_config = SlackHostBetaRuntimeConfig::new(
        tenant_id.clone(),
        default_agent_id.clone(),
        default_project_id.cloned(),
        default_user_id.clone(),
    );
    Ok(Some(runtime_config))
}

fn slack_enabled(section: Option<&ironclaw_reborn_config::SlackSection>) -> anyhow::Result<bool> {
    match std::env::var(SLACK_ENABLED_ENV) {
        Ok(value) => parse_slack_enabled_bool(SLACK_ENABLED_ENV, value.as_str()),
        Err(std::env::VarError::NotPresent) => Ok(section.and_then(|s| s.enabled).unwrap_or(false)),
        Err(std::env::VarError::NotUnicode(_)) => {
            anyhow::bail!("{SLACK_ENABLED_ENV} must be valid UTF-8 when set")
        }
    }
}

fn parse_slack_enabled_bool(field: &str, value: &str) -> anyhow::Result<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => anyhow::bail!("{field} must be a boolean value"),
    }
}

#[cfg(not(feature = "slack-v2-host-beta"))]
pub(crate) fn resolve_slack_config_for_serve(
    section: Option<&ironclaw_reborn_config::SlackSection>,
    _tenant_id: &ironclaw_reborn_composition::host_api::TenantId,
    _default_agent_id: &ironclaw_reborn_composition::host_api::AgentId,
    _default_project_id: Option<&ironclaw_reborn_composition::host_api::ProjectId>,
    _default_user_id: &ironclaw_reborn_composition::host_api::UserId,
    _config_path: &std::path::Path,
) -> anyhow::Result<Option<()>> {
    // Fail loud instead of silently starting without Slack: an operator who
    // explicitly enabled Slack must learn the binary lacks the feature.
    if slack_enabled(section)? {
        anyhow::bail!(
            "Slack enablement ([slack].enabled = true or {SLACK_ENABLED_ENV}=true) requires \
             an ironclaw-reborn binary built with the `slack-v2-host-beta` Cargo feature"
        );
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "slack-v2-host-beta")]
    #[test]
    fn slack_host_beta_runtime_config_uses_webui_scope_when_enabled() {
        let project_id = project_id("project");
        let section = ironclaw_reborn_config::SlackSection {
            enabled: Some(true),
            ..Default::default()
        };

        let resolved = resolve_slack_config_for_serve(
            Some(&section),
            &tenant_id("tenant"),
            &agent_id("agent"),
            Some(&project_id),
            &user_id("web-user"),
            std::path::Path::new("/tmp/reborn-config.toml"),
        )
        .expect("Slack config resolves")
        .expect("Slack enabled");

        assert_eq!(resolved.tenant_id.as_str(), "tenant");
        assert_eq!(resolved.agent_id.as_str(), "agent");
        assert_eq!(
            resolved.project_id.as_ref().map(|id| id.as_str()),
            Some("project")
        );
        assert_eq!(resolved.operator_user_id.as_str(), "web-user");
        assert!(resolved.legacy_setup.is_none());
    }

    #[cfg(feature = "slack-v2-host-beta")]
    #[test]
    fn slack_host_beta_runtime_config_is_absent_without_section() {
        let project_id = project_id("project");

        let resolved = resolve_slack_config_for_serve(
            None,
            &tenant_id("tenant"),
            &agent_id("agent"),
            Some(&project_id),
            &user_id("web-user"),
            std::path::Path::new("/tmp/reborn-config.toml"),
        )
        .expect("Slack config resolves without section");

        assert!(resolved.is_none());
    }

    #[test]
    fn parse_slack_enabled_bool_accepts_known_values_and_rejects_garbage() {
        for value in ["1", "true", "YES", " on "] {
            assert!(parse_slack_enabled_bool("test_field", value).expect("truthy value parses"));
        }
        for value in ["0", "false", "No", "off"] {
            assert!(!parse_slack_enabled_bool("test_field", value).expect("falsy value parses"));
        }
        assert!(parse_slack_enabled_bool("test_field", "maybe").is_err());
    }

    #[cfg(not(feature = "slack-v2-host-beta"))]
    #[test]
    fn slack_config_rejects_enabled_section_without_feature() {
        let section = ironclaw_reborn_config::SlackSection {
            enabled: Some(true),
            ..Default::default()
        };

        let err = resolve_slack_config_for_serve(
            Some(&section),
            &ironclaw_reborn_composition::host_api::TenantId::new("tenant").expect("valid tenant"),
            &ironclaw_reborn_composition::host_api::AgentId::new("agent").expect("valid agent"),
            None,
            &ironclaw_reborn_composition::host_api::UserId::new("web-user").expect("valid user"),
            std::path::Path::new("/tmp/reborn-config.toml"),
        )
        .expect_err("explicitly enabled Slack must fail loud without the feature");

        assert!(
            err.to_string()
                .contains("requires an ironclaw-reborn binary built with")
        );
    }

    #[cfg(not(feature = "slack-v2-host-beta"))]
    #[test]
    fn slack_config_is_noop_without_feature_when_disabled_or_unset() {
        let disabled = ironclaw_reborn_config::SlackSection {
            enabled: Some(false),
            ..Default::default()
        };
        for section in [None, Some(&disabled)] {
            let resolved = resolve_slack_config_for_serve(
                section,
                &ironclaw_reborn_composition::host_api::TenantId::new("tenant")
                    .expect("valid tenant"),
                &ironclaw_reborn_composition::host_api::AgentId::new("agent").expect("valid agent"),
                None,
                &ironclaw_reborn_composition::host_api::UserId::new("web-user")
                    .expect("valid user"),
                std::path::Path::new("/tmp/reborn-config.toml"),
            )
            .expect("disabled or unset Slack resolves without the feature");
            assert!(resolved.is_none());
        }
    }

    #[cfg(feature = "slack-v2-host-beta")]
    fn tenant_id(raw: &str) -> ironclaw_reborn_composition::host_api::TenantId {
        ironclaw_reborn_composition::host_api::TenantId::new(raw).expect("valid tenant")
    }

    #[cfg(feature = "slack-v2-host-beta")]
    fn agent_id(raw: &str) -> ironclaw_reborn_composition::host_api::AgentId {
        ironclaw_reborn_composition::host_api::AgentId::new(raw).expect("valid agent")
    }

    #[cfg(feature = "slack-v2-host-beta")]
    fn project_id(raw: &str) -> ironclaw_reborn_composition::host_api::ProjectId {
        ironclaw_reborn_composition::host_api::ProjectId::new(raw).expect("valid project")
    }

    #[cfg(feature = "slack-v2-host-beta")]
    fn user_id(raw: &str) -> ironclaw_reborn_composition::host_api::UserId {
        ironclaw_reborn_composition::host_api::UserId::new(raw).expect("valid user")
    }
}
