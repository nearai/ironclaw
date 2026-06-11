#[cfg(feature = "slack-v2-host-beta")]
use std::path::Path;

#[cfg(feature = "slack-v2-host-beta")]
use ironclaw_reborn_composition::SlackHostBetaRuntimeConfig;

#[cfg(feature = "slack-v2-host-beta")]
pub(crate) fn resolve_slack_config_for_serve(
    section: Option<&ironclaw_reborn_config::SlackSection>,
    tenant_id: &ironclaw_reborn_composition::host_api::TenantId,
    default_agent_id: &ironclaw_reborn_composition::host_api::AgentId,
    default_project_id: Option<&ironclaw_reborn_composition::host_api::ProjectId>,
    default_user_id: &ironclaw_reborn_composition::host_api::UserId,
    _config_path: &Path,
) -> anyhow::Result<Option<SlackHostBetaRuntimeConfig>> {
    let Some(section) = section else {
        return Ok(None);
    };
    if section.enabled != Some(true) {
        return Ok(None);
    }
    Ok(Some(SlackHostBetaRuntimeConfig::new(
        tenant_id.clone(),
        default_agent_id.clone(),
        default_project_id.cloned(),
        default_user_id.clone(),
    )))
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
    reject_enabled_slack_without_feature(section)?;
    Ok(None)
}

#[cfg(not(feature = "slack-v2-host-beta"))]
pub(crate) fn reject_enabled_slack_without_feature(
    section: Option<&ironclaw_reborn_config::SlackSection>,
) -> anyhow::Result<()> {
    if section.and_then(|section| section.enabled).unwrap_or(false) {
        anyhow::bail!(
            "[slack].enabled = true requires an ironclaw-reborn binary built with \
             the `slack-v2-host-beta` Cargo feature"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "slack-v2-host-beta")]
    #[test]
    fn slack_host_beta_runtime_config_is_disabled_unless_explicitly_enabled() {
        let section = ironclaw_reborn_config::SlackSection { enabled: None };

        let resolved = resolve_slack_config_for_serve(
            Some(&section),
            &tenant_id("tenant"),
            &agent_id("agent"),
            None,
            &user_id("web-user"),
            std::path::Path::new("/tmp/reborn-config.toml"),
        )
        .expect("disabled Slack should not require runtime setup fields");

        assert!(resolved.is_none());
    }

    #[cfg(feature = "slack-v2-host-beta")]
    #[test]
    fn slack_host_beta_runtime_config_uses_webui_scope_when_enabled() {
        let section = ironclaw_reborn_config::SlackSection {
            enabled: Some(true),
        };
        let project_id = project_id("project");

        let resolved = resolve_slack_config_for_serve(
            Some(&section),
            &tenant_id("tenant"),
            &agent_id("agent"),
            Some(&project_id),
            &user_id("web-user"),
            std::path::Path::new("/tmp/reborn-config.toml"),
        )
        .expect("enabled Slack resolves runtime scope")
        .expect("Slack enabled");

        assert_eq!(resolved.tenant_id.as_str(), "tenant");
        assert_eq!(resolved.agent_id.as_str(), "agent");
        assert_eq!(
            resolved.project_id.as_ref().map(|id| id.as_str()),
            Some("project")
        );
        assert_eq!(resolved.operator_user_id.as_str(), "web-user");
    }

    #[cfg(not(feature = "slack-v2-host-beta"))]
    #[test]
    fn slack_config_rejects_enabled_section_without_feature() {
        let section = ironclaw_reborn_config::SlackSection {
            enabled: Some(true),
        };

        let err = reject_enabled_slack_without_feature(Some(&section))
            .expect_err("enabled Slack should require feature");

        assert!(
            err.to_string()
                .contains("requires an ironclaw-reborn binary built with")
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
