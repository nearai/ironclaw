#[cfg(feature = "slack-v2-host-beta")]
use std::path::Path;

#[cfg(feature = "slack-v2-host-beta")]
use anyhow::anyhow;

#[cfg(feature = "slack-v2-host-beta")]
use ironclaw_reborn_composition::{
    SlackHostBetaChannelRoute, SlackHostBetaLegacySetup, SlackHostBetaRuntimeConfig,
};

#[cfg(feature = "slack-v2-host-beta")]
pub(crate) fn resolve_slack_config_for_serve(
    section: Option<&ironclaw_reborn_config::SlackSection>,
    tenant_id: &ironclaw_reborn_composition::host_api::TenantId,
    default_agent_id: &ironclaw_reborn_composition::host_api::AgentId,
    default_project_id: Option<&ironclaw_reborn_composition::host_api::ProjectId>,
    default_user_id: &ironclaw_reborn_composition::host_api::UserId,
    config_path: &Path,
) -> anyhow::Result<Option<SlackHostBetaRuntimeConfig>> {
    let runtime_config = SlackHostBetaRuntimeConfig::new(
        tenant_id.clone(),
        default_agent_id.clone(),
        default_project_id.cloned(),
        default_user_id.clone(),
    );
    let Some(section) = section else {
        return Ok(Some(runtime_config));
    };
    let Some(legacy_setup) = resolve_legacy_slack_setup(section, default_user_id, config_path)?
    else {
        return Ok(Some(runtime_config));
    };
    Ok(Some(runtime_config.with_legacy_setup(legacy_setup)))
}

#[cfg(feature = "slack-v2-host-beta")]
fn resolve_legacy_slack_setup(
    section: &ironclaw_reborn_config::SlackSection,
    default_user_id: &ironclaw_reborn_composition::host_api::UserId,
    config_path: &Path,
) -> anyhow::Result<Option<SlackHostBetaLegacySetup>> {
    if !has_legacy_slack_setup(section) {
        return Ok(None);
    }

    let installation_id =
        required_slack_config_value("installation_id", &section.installation_id, config_path)?;
    let team_id = required_slack_config_value("team_id", &section.team_id, config_path)?;
    let api_app_id = required_slack_config_value("api_app_id", &section.api_app_id, config_path)?;
    let user_id = optional_slack_user_id_config_value("user_id", &section.user_id)?
        .unwrap_or_else(|| default_user_id.clone());
    let shared_subject_user_id = optional_slack_user_id_config_value(
        "shared_subject_user_id",
        &section.shared_subject_user_id,
    )?;
    let channel_routes = section
        .channel_routes
        .iter()
        .enumerate()
        .map(parse_slack_channel_route_config)
        .collect::<anyhow::Result<Vec<_>>>()?;

    Ok(Some(SlackHostBetaLegacySetup {
        installation_id,
        team_id,
        api_app_id,
        slack_user_id: optional_slack_config_value("slack_user_id", &section.slack_user_id)?,
        user_id,
        shared_subject_user_id,
        channel_routes,
    }))
}

#[cfg(feature = "slack-v2-host-beta")]
fn has_legacy_slack_setup(section: &ironclaw_reborn_config::SlackSection) -> bool {
    section.installation_id.is_some()
        || section.team_id.is_some()
        || section.api_app_id.is_some()
        || section.slack_user_id.is_some()
        || section.user_id.is_some()
        || section.shared_subject_user_id.is_some()
        || !section.channel_routes.is_empty()
}

#[cfg(feature = "slack-v2-host-beta")]
fn required_slack_config_value(
    field: &str,
    value: &Option<String>,
    config_path: &Path,
) -> anyhow::Result<String> {
    optional_slack_config_value(field, value)?.ok_or_else(|| {
        anyhow!(
            "[slack].{field} must be set when legacy Slack setup fields are present in {}",
            config_path.display()
        )
    })
}

#[cfg(feature = "slack-v2-host-beta")]
fn optional_slack_config_value(
    field: &str,
    value: &Option<String>,
) -> anyhow::Result<Option<String>> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.trim().is_empty() {
        anyhow::bail!("[slack].{field} must not be empty when set");
    }
    if value.trim() != value {
        anyhow::bail!("[slack].{field} must not contain leading or trailing whitespace when set");
    }
    Ok(Some(value.clone()))
}

#[cfg(feature = "slack-v2-host-beta")]
fn optional_slack_user_id_config_value(
    field: &str,
    value: &Option<String>,
) -> anyhow::Result<Option<ironclaw_reborn_composition::host_api::UserId>> {
    optional_slack_config_value(field, value)?
        .map(|raw| {
            ironclaw_reborn_composition::host_api::UserId::new(&raw)
                .map_err(|err| anyhow!("[slack].{field} `{raw}` is invalid: {err}"))
        })
        .transpose()
}

#[cfg(feature = "slack-v2-host-beta")]
fn parse_slack_channel_route_config(
    (index, route): (usize, &ironclaw_reborn_config::SlackChannelRouteSection),
) -> anyhow::Result<SlackHostBetaChannelRoute> {
    let channel_field = format!("channel_routes[{index}].channel_id");
    let subject_field = format!("channel_routes[{index}].subject_user_id");
    let channel_id = optional_slack_config_value(&channel_field, &route.channel_id)?
        .ok_or_else(|| anyhow!("[slack].{channel_field} must be set"))?;
    let subject_user_id =
        optional_slack_user_id_config_value(&subject_field, &route.subject_user_id)?
            .ok_or_else(|| anyhow!("[slack].{subject_field} must be set"))?;
    Ok(SlackHostBetaChannelRoute::new(channel_id, subject_user_id))
}

#[cfg(not(feature = "slack-v2-host-beta"))]
pub(crate) fn resolve_slack_config_for_serve(
    _section: Option<&ironclaw_reborn_config::SlackSection>,
    _tenant_id: &ironclaw_reborn_composition::host_api::TenantId,
    _default_agent_id: &ironclaw_reborn_composition::host_api::AgentId,
    _default_project_id: Option<&ironclaw_reborn_composition::host_api::ProjectId>,
    _default_user_id: &ironclaw_reborn_composition::host_api::UserId,
    _config_path: &std::path::Path,
) -> anyhow::Result<Option<()>> {
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
        .expect("Slack config always resolves")
        .expect("Slack always enabled");

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
    fn slack_host_beta_runtime_config_uses_webui_scope_without_section() {
        let project_id = project_id("project");

        let resolved = resolve_slack_config_for_serve(
            None,
            &tenant_id("tenant"),
            &agent_id("agent"),
            Some(&project_id),
            &user_id("web-user"),
            std::path::Path::new("/tmp/reborn-config.toml"),
        )
        .expect("Slack config always resolves without section")
        .expect("Slack always enabled");

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
