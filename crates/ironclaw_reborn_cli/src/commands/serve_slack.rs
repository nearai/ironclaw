#[cfg(feature = "slack-v2-host-beta")]
use anyhow::anyhow;

#[cfg(feature = "slack-v2-host-beta")]
use std::env;
#[cfg(feature = "slack-v2-host-beta")]
use std::path::Path;

#[cfg(feature = "slack-v2-host-beta")]
use ironclaw_reborn_composition::{
    SlackHostBetaChannelRoute, SlackHostBetaLegacySetup, SlackHostBetaRuntimeConfig,
};
#[cfg(feature = "slack-v2-host-beta")]
use secrecy::SecretString;

#[cfg(feature = "slack-v2-host-beta")]
const DEFAULT_SLACK_SIGNING_SECRET_ENV_VAR: &str = "IRONCLAW_REBORN_SLACK_SIGNING_SECRET";
#[cfg(feature = "slack-v2-host-beta")]
const DEFAULT_SLACK_BOT_TOKEN_ENV_VAR: &str = "IRONCLAW_REBORN_SLACK_BOT_TOKEN";

#[cfg(feature = "slack-v2-host-beta")]
pub(crate) fn resolve_slack_config_for_serve(
    section: Option<&ironclaw_reborn_config::SlackSection>,
    tenant_id: &ironclaw_reborn_composition::host_api::TenantId,
    default_agent_id: &ironclaw_reborn_composition::host_api::AgentId,
    default_project_id: Option<&ironclaw_reborn_composition::host_api::ProjectId>,
    default_user_id: &ironclaw_reborn_composition::host_api::UserId,
    config_path: &Path,
) -> anyhow::Result<Option<SlackHostBetaRuntimeConfig>> {
    let Some(section) = section else {
        return Ok(None);
    };
    if section.enabled != Some(true) {
        return Ok(None);
    }
    let runtime_config = SlackHostBetaRuntimeConfig::new(
        tenant_id.clone(),
        default_agent_id.clone(),
        default_project_id.cloned(),
        default_user_id.clone(),
    );
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
    let signing_secret_env =
        optional_slack_config_value("signing_secret_env", &section.signing_secret_env)?
            .unwrap_or_else(|| DEFAULT_SLACK_SIGNING_SECRET_ENV_VAR.to_string());
    let bot_token_env = optional_slack_config_value("bot_token_env", &section.bot_token_env)?
        .unwrap_or_else(|| DEFAULT_SLACK_BOT_TOKEN_ENV_VAR.to_string());
    let signing_secret = required_env_secret(
        "signing secret",
        "signing_secret_env",
        &signing_secret_env,
        config_path,
    )?;
    let bot_token = required_env_secret("bot token", "bot_token_env", &bot_token_env, config_path)?;

    Ok(Some(SlackHostBetaLegacySetup {
        installation_id,
        team_id,
        api_app_id,
        slack_user_id: optional_slack_config_value("slack_user_id", &section.slack_user_id)?,
        user_id,
        shared_subject_user_id,
        channel_routes,
        signing_secret: SecretString::from(signing_secret),
        bot_token: SecretString::from(bot_token),
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
        || section.signing_secret_env.is_some()
        || section.bot_token_env.is_some()
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

#[cfg(feature = "slack-v2-host-beta")]
fn required_env_secret(
    label: &'static str,
    field: &'static str,
    env_var: &str,
    config_path: &Path,
) -> anyhow::Result<String> {
    let value = env::var(env_var).map_err(|_| {
        anyhow!(
            "{env_var} must be set to the Slack {label} for legacy Slack setup. \
             Override the variable name via [slack].{field} in {}.",
            config_path.display()
        )
    })?;
    if value.is_empty() {
        anyhow::bail!("{env_var} must not be empty for legacy Slack setup");
    }
    Ok(value)
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
    use secrecy::ExposeSecret;

    #[cfg(feature = "slack-v2-host-beta")]
    #[test]
    fn slack_host_beta_runtime_config_is_disabled_unless_explicitly_enabled() {
        let section = ironclaw_reborn_config::SlackSection {
            enabled: None,
            ..Default::default()
        };

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
            ..Default::default()
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
        assert!(resolved.legacy_setup.is_none());
    }

    #[cfg(feature = "slack-v2-host-beta")]
    #[test]
    fn slack_host_beta_runtime_config_imports_legacy_setup_from_env_backed_config() {
        const SIGNING_ENV: &str = "IRONCLAW_TEST_SLACK_LEGACY_SIGNING_SECRET";
        const BOT_ENV: &str = "IRONCLAW_TEST_SLACK_LEGACY_BOT_TOKEN";
        let _signing = EnvGuard::set(SIGNING_ENV, "legacy-signing-secret");
        let _bot = EnvGuard::set(BOT_ENV, "xoxb-legacy");
        let section = ironclaw_reborn_config::SlackSection {
            enabled: Some(true),
            installation_id: Some("install-alpha".to_string()),
            team_id: Some("T123".to_string()),
            api_app_id: Some("A123".to_string()),
            slack_user_id: Some("U123".to_string()),
            user_id: Some("user:operator".to_string()),
            shared_subject_user_id: Some("user:shared-slack".to_string()),
            channel_routes: vec![ironclaw_reborn_config::SlackChannelRouteSection {
                channel_id: Some("CENG".to_string()),
                subject_user_id: Some("user:eng-team-agent".to_string()),
            }],
            signing_secret_env: Some(SIGNING_ENV.to_string()),
            bot_token_env: Some(BOT_ENV.to_string()),
        };

        let resolved = resolve_slack_config_for_serve(
            Some(&section),
            &tenant_id("tenant"),
            &agent_id("agent"),
            None,
            &user_id("web-user"),
            std::path::Path::new("/tmp/reborn-config.toml"),
        )
        .expect("legacy Slack config resolves")
        .expect("Slack enabled");
        let legacy = resolved.legacy_setup.expect("legacy setup imported");

        assert_eq!(legacy.installation_id, "install-alpha");
        assert_eq!(legacy.team_id, "T123");
        assert_eq!(legacy.api_app_id, "A123");
        assert_eq!(legacy.slack_user_id.as_deref(), Some("U123"));
        assert_eq!(legacy.user_id.as_str(), "user:operator");
        assert_eq!(
            legacy.shared_subject_user_id.as_ref().map(|id| id.as_str()),
            Some("user:shared-slack")
        );
        assert_eq!(legacy.channel_routes.len(), 1);
        assert_eq!(legacy.channel_routes[0].channel_id, "CENG");
        assert_eq!(
            legacy.channel_routes[0].subject_user_id.as_str(),
            "user:eng-team-agent"
        );
        assert_eq!(
            legacy.signing_secret.expose_secret(),
            "legacy-signing-secret"
        );
        assert_eq!(legacy.bot_token.expose_secret(), "xoxb-legacy");
    }

    #[cfg(not(feature = "slack-v2-host-beta"))]
    #[test]
    fn slack_config_rejects_enabled_section_without_feature() {
        let section = ironclaw_reborn_config::SlackSection {
            enabled: Some(true),
            ..Default::default()
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

    #[cfg(feature = "slack-v2-host-beta")]
    struct EnvGuard {
        key: &'static str,
        prior: Option<String>,
    }

    #[cfg(feature = "slack-v2-host-beta")]
    impl EnvGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let prior = std::env::var(key).ok();
            // SAFETY: these test env names are unique to this module and restored on drop.
            unsafe { std::env::set_var(key, value) };
            Self { key, prior }
        }
    }

    #[cfg(feature = "slack-v2-host-beta")]
    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match self.prior.take() {
                // SAFETY: restores the test env var snapshot captured by EnvGuard::set.
                Some(value) => unsafe { std::env::set_var(self.key, value) },
                // SAFETY: restores the absence of the test env var.
                None => unsafe { std::env::remove_var(self.key) },
            }
        }
    }
}
