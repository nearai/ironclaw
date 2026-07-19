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
    config_path: &Path,
) -> anyhow::Result<Option<SlackHostBetaRuntimeConfig>> {
    let enabled = slack_enabled(section)?;
    if !enabled {
        return Ok(None);
    }
    if let Some(section) = section {
        reject_legacy_slack_setup_fields(section, config_path)?;
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
        Ok(value) => crate::commands::parse_channel_enabled_bool(SLACK_ENABLED_ENV, value.as_str()),
        Err(std::env::VarError::NotPresent) => Ok(section.and_then(|s| s.enabled).unwrap_or(false)),
        Err(std::env::VarError::NotUnicode(_)) => {
            anyhow::bail!("{SLACK_ENABLED_ENV} must be valid UTF-8 when set")
        }
    }
}

#[cfg(feature = "slack-v2-host-beta")]
fn reject_legacy_slack_setup_fields(
    section: &ironclaw_reborn_config::SlackSection,
    config_path: &Path,
) -> anyhow::Result<()> {
    let deprecated_fields = [
        ("installation_id", section.installation_id.as_ref()),
        ("team_id", section.team_id.as_ref()),
        ("api_app_id", section.api_app_id.as_ref()),
        ("slack_user_id", section.slack_user_id.as_ref()),
        ("user_id", section.user_id.as_ref()),
        (
            "shared_subject_user_id",
            section.shared_subject_user_id.as_ref(),
        ),
        ("signing_secret_env", section.signing_secret_env.as_ref()),
        ("bot_token_env", section.bot_token_env.as_ref()),
    ];
    let mut violated: Vec<&str> = deprecated_fields
        .into_iter()
        .filter(|(_, value)| value.is_some())
        .map(|(field, _)| field)
        .collect();
    if !section.channel_routes.is_empty() {
        violated.push("channel_routes");
    }
    if !violated.is_empty() {
        anyhow::bail!(
            "[slack].{} {} no longer supported in {}; configure Slack from the WebUI and connect the user through Slack OAuth",
            violated.join(", "),
            if violated.len() == 1 { "is" } else { "are" },
            config_path.display()
        );
    }
    Ok(())
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
             an ironclaw binary built with the `slack-v2-host-beta` Cargo feature"
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
        let section = ironclaw_reborn_config::SlackSection::default().set_enabled(true);

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
            assert!(
                crate::commands::parse_channel_enabled_bool("test_field", value)
                    .expect("truthy value parses")
            );
        }
        for value in ["0", "false", "No", "off"] {
            assert!(
                !crate::commands::parse_channel_enabled_bool("test_field", value)
                    .expect("falsy value parses")
            );
        }
        assert!(crate::commands::parse_channel_enabled_bool("test_field", "maybe").is_err());
    }

    #[cfg(not(feature = "slack-v2-host-beta"))]
    #[test]
    fn slack_config_rejects_enabled_section_without_feature() {
        let section = ironclaw_reborn_config::SlackSection::default().set_enabled(true);

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
                .contains("requires an ironclaw binary built with")
        );
    }

    #[cfg(not(feature = "slack-v2-host-beta"))]
    #[test]
    fn slack_config_is_noop_without_feature_when_disabled_or_unset() {
        let disabled = ironclaw_reborn_config::SlackSection::default().set_enabled(false);
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
    #[test]
    fn slack_host_beta_runtime_config_rejects_legacy_static_user_binding() {
        let section = ironclaw_reborn_config::SlackSection::default()
            .set_enabled(true)
            .set_slack_user_id("U123");

        let error = resolve_slack_config_for_serve(
            Some(&section),
            &tenant_id("tenant"),
            &agent_id("agent"),
            None,
            &user_id("web-user"),
            std::path::Path::new("/tmp/reborn-config.toml"),
        )
        .expect_err("legacy static Slack user binding must be rejected");

        assert!(
            error.to_string().contains("slack_user_id"),
            "error should name the rejected field: {error}"
        );
    }

    #[cfg(feature = "slack-v2-host-beta")]
    #[test]
    fn slack_host_beta_runtime_config_rejects_legacy_channel_routes() {
        let section = ironclaw_reborn_config::SlackSection::default()
            .set_enabled(true)
            .set_channel_routes(vec![ironclaw_reborn_config::SlackChannelRouteSection {
                channel_id: Some("CENG".to_string()),
                subject_user_id: Some("U123".to_string()),
            }]);

        let error = resolve_slack_config_for_serve(
            Some(&section),
            &tenant_id("tenant"),
            &agent_id("agent"),
            None,
            &user_id("web-user"),
            std::path::Path::new("/tmp/reborn-config.toml"),
        )
        .expect_err("legacy channel_routes must be rejected");

        assert!(
            error.to_string().contains("channel_routes"),
            "error should name the rejected field: {error}"
        );
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
