#[cfg(feature = "slack-v2-host-beta")]
use anyhow::anyhow;

#[cfg(feature = "slack-v2-host-beta")]
use std::env;
#[cfg(feature = "slack-v2-host-beta")]
use std::path::Path;

#[cfg(feature = "slack-v2-host-beta")]
use ironclaw_reborn_composition::{
    SlackHostBetaChannelRoute, SlackHostBetaConfig, SlackHostBetaConfigInput, SlackTeamId,
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
) -> anyhow::Result<Option<SlackHostBetaConfig>> {
    resolve_slack_host_beta_config(
        section,
        tenant_id,
        default_agent_id,
        default_project_id,
        default_user_id,
        config_path,
    )
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

#[cfg(feature = "slack-v2-host-beta")]
pub(crate) fn resolve_slack_host_beta_config(
    section: Option<&ironclaw_reborn_config::SlackSection>,
    tenant_id: &ironclaw_reborn_composition::host_api::TenantId,
    default_agent_id: &ironclaw_reborn_composition::host_api::AgentId,
    default_project_id: Option<&ironclaw_reborn_composition::host_api::ProjectId>,
    default_user_id: &ironclaw_reborn_composition::host_api::UserId,
    config_path: &Path,
) -> anyhow::Result<Option<SlackHostBetaConfig>> {
    let Some(section) = section else {
        return Ok(None);
    };
    if section.enabled != Some(true) {
        return Ok(None);
    }

    let installation_id =
        required_slack_config_value("installation_id", &section.installation_id, config_path)?;
    let team_id = required_slack_config_value("team_id", &section.team_id, config_path)?;
    let api_app_id = required_slack_config_value("api_app_id", &section.api_app_id, config_path)?;
    let slack_user_id = optional_slack_config_value("slack_user_id", &section.slack_user_id)?;
    let mapped_user_id = optional_slack_user_id_config_value("user_id", &section.user_id)?
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

    Ok(Some(SlackHostBetaConfig::new(SlackHostBetaConfigInput {
        tenant_id: tenant_id.clone(),
        agent_id: default_agent_id.clone(),
        project_id: default_project_id.cloned(),
        installation_id,
        team_id: SlackTeamId::new(team_id),
        api_app_id: Some(api_app_id),
        slack_user_id,
        user_id: mapped_user_id,
        shared_subject_user_id,
        channel_routes,
        signing_secret: SecretString::from(signing_secret),
        bot_token: SecretString::from(bot_token),
    })?))
}

#[cfg(feature = "slack-v2-host-beta")]
fn required_slack_config_value(
    field: &str,
    value: &Option<String>,
    config_path: &Path,
) -> anyhow::Result<String> {
    optional_slack_config_value(field, value)?.ok_or_else(|| {
        anyhow!(
            "[slack].{field} must be set when [slack].enabled = true in {}",
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
            "{env_var} must be set to the Slack {label} when [slack].enabled = true. \
             Override the variable name via [slack].{field} in {}.",
            config_path.display()
        )
    })?;
    if value.is_empty() {
        anyhow::bail!("{env_var} must not be empty when [slack].enabled = true");
    }
    Ok(value)
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
    fn slack_host_beta_config_is_disabled_unless_explicitly_enabled() {
        let section = ironclaw_reborn_config::SlackSection {
            enabled: None,
            ..Default::default()
        };

        let resolved = resolve_slack_host_beta_config(
            Some(&section),
            &tenant_id("tenant"),
            &agent_id("agent"),
            None,
            &user_id("web-user"),
            Path::new("/tmp/reborn-config.toml"),
        )
        .expect("disabled Slack should not require fields or env vars");

        assert!(resolved.is_none());
    }

    #[cfg(feature = "slack-v2-host-beta")]
    #[test]
    fn slack_host_beta_config_requires_identifiers_when_enabled() {
        let section = ironclaw_reborn_config::SlackSection {
            enabled: Some(true),
            ..Default::default()
        };

        let error = resolve_slack_host_beta_config(
            Some(&section),
            &tenant_id("tenant"),
            &agent_id("agent"),
            None,
            &user_id("web-user"),
            Path::new("/tmp/reborn-config.toml"),
        )
        .expect_err("enabled Slack must require host-selected identifiers");

        assert!(
            error.to_string().contains("[slack].installation_id"),
            "message: {error}"
        );
    }

    #[cfg(feature = "slack-v2-host-beta")]
    #[test]
    fn slack_host_beta_config_rejects_empty_required_identifier() {
        let section = ironclaw_reborn_config::SlackSection {
            enabled: Some(true),
            installation_id: Some(" ".to_string()),
            ..Default::default()
        };

        let error = resolve_slack_host_beta_config(
            Some(&section),
            &tenant_id("tenant"),
            &agent_id("agent"),
            None,
            &user_id("web-user"),
            Path::new("/tmp/reborn-config.toml"),
        )
        .expect_err("empty Slack identifiers must fail at config resolution");

        assert!(
            error
                .to_string()
                .contains("[slack].installation_id must not be empty"),
            "message: {error}"
        );
    }

    #[cfg(feature = "slack-v2-host-beta")]
    #[test]
    fn slack_host_beta_config_rejects_padded_identifier() {
        let section = ironclaw_reborn_config::SlackSection {
            enabled: Some(true),
            installation_id: Some("install-alpha".to_string()),
            team_id: Some(" T123".to_string()),
            ..Default::default()
        };

        let error = resolve_slack_host_beta_config(
            Some(&section),
            &tenant_id("tenant"),
            &agent_id("agent"),
            None,
            &user_id("web-user"),
            Path::new("/tmp/reborn-config.toml"),
        )
        .expect_err("padded Slack identifiers must fail at config resolution");

        assert!(
            error
                .to_string()
                .contains("[slack].team_id must not contain leading or trailing whitespace"),
            "message: {error}"
        );
    }

    #[cfg(feature = "slack-v2-host-beta")]
    #[test]
    fn slack_host_beta_config_reads_env_secrets_and_defaults_user() {
        let _lock = env_lock();
        let _signing = EnvGuard::set(
            "IRONCLAW_TEST_SLACK_SIGNING_SECRET_DEFAULT_USER",
            "signing-secret",
        );
        let _bot = EnvGuard::set("IRONCLAW_TEST_SLACK_BOT_TOKEN_DEFAULT_USER", "xoxb-token");
        let section = ironclaw_reborn_config::SlackSection {
            enabled: Some(true),
            installation_id: Some("install-alpha".to_string()),
            team_id: Some("T123".to_string()),
            api_app_id: Some("A123".to_string()),
            slack_user_id: Some("U123".to_string()),
            signing_secret_env: Some("IRONCLAW_TEST_SLACK_SIGNING_SECRET_DEFAULT_USER".to_string()),
            bot_token_env: Some("IRONCLAW_TEST_SLACK_BOT_TOKEN_DEFAULT_USER".to_string()),
            ..Default::default()
        };

        let resolved = resolve_slack_host_beta_config(
            Some(&section),
            &tenant_id("tenant"),
            &agent_id("agent"),
            Some(&project_id("project")),
            &user_id("web-user"),
            Path::new("/tmp/reborn-config.toml"),
        )
        .expect("Slack config should resolve")
        .expect("Slack should be enabled");

        assert_eq!(resolved.installation_id.as_str(), "install-alpha");
        assert!(format!("{:?}", resolved.installation_selector).contains("AppTeam"));
        assert_eq!(
            resolved.slack_actor.as_ref().expect("legacy actor").id(),
            "U123"
        );
        assert_eq!(resolved.user_id, user_id("web-user"));
        assert_eq!(resolved.signing_secret.expose_secret(), "signing-secret");
        assert_eq!(resolved.bot_token.expose_secret(), "xoxb-token");
    }

    #[cfg(feature = "slack-v2-host-beta")]
    #[test]
    fn slack_host_beta_config_does_not_require_static_slack_user() {
        let _lock = env_lock();
        let _signing = EnvGuard::set(
            "IRONCLAW_TEST_SLACK_SIGNING_SECRET_NO_STATIC_USER",
            "signing-secret",
        );
        let _bot = EnvGuard::set("IRONCLAW_TEST_SLACK_BOT_TOKEN_NO_STATIC_USER", "xoxb-token");
        let section = ironclaw_reborn_config::SlackSection {
            enabled: Some(true),
            installation_id: Some("install-alpha".to_string()),
            team_id: Some("T123".to_string()),
            api_app_id: Some("A123".to_string()),
            signing_secret_env: Some(
                "IRONCLAW_TEST_SLACK_SIGNING_SECRET_NO_STATIC_USER".to_string(),
            ),
            bot_token_env: Some("IRONCLAW_TEST_SLACK_BOT_TOKEN_NO_STATIC_USER".to_string()),
            ..Default::default()
        };

        let resolved = resolve_slack_host_beta_config(
            Some(&section),
            &tenant_id("tenant"),
            &agent_id("agent"),
            None,
            &user_id("web-user"),
            Path::new("/tmp/reborn-config.toml"),
        )
        .expect("Slack config should resolve")
        .expect("Slack should be enabled");

        assert!(resolved.slack_actor.is_none());
        assert_eq!(resolved.user_id, user_id("web-user"));
    }

    #[cfg(feature = "slack-v2-host-beta")]
    #[test]
    fn slack_host_beta_config_requires_api_app_id_for_pairing() {
        let section = ironclaw_reborn_config::SlackSection {
            enabled: Some(true),
            installation_id: Some("install-alpha".to_string()),
            team_id: Some("T123".to_string()),
            ..Default::default()
        };

        let error = resolve_slack_host_beta_config(
            Some(&section),
            &tenant_id("tenant"),
            &agent_id("agent"),
            None,
            &user_id("web-user"),
            Path::new("/tmp/reborn-config.toml"),
        )
        .expect_err("enabled Slack pairing must require api_app_id");

        assert!(
            error.to_string().contains("[slack].api_app_id"),
            "message: {error}"
        );
    }

    #[cfg(feature = "slack-v2-host-beta")]
    #[test]
    fn slack_host_beta_config_rejects_empty_env_secret_value() {
        let _lock = env_lock();
        let _signing = EnvGuard::set("IRONCLAW_TEST_SLACK_EMPTY_SIGNING_SECRET", "");
        let _bot = EnvGuard::set("IRONCLAW_TEST_SLACK_EMPTY_BOT_TOKEN", "xoxb-token");
        let section = ironclaw_reborn_config::SlackSection {
            enabled: Some(true),
            installation_id: Some("install-alpha".to_string()),
            team_id: Some("T123".to_string()),
            api_app_id: Some("A123".to_string()),
            slack_user_id: Some("U123".to_string()),
            signing_secret_env: Some("IRONCLAW_TEST_SLACK_EMPTY_SIGNING_SECRET".to_string()),
            bot_token_env: Some("IRONCLAW_TEST_SLACK_EMPTY_BOT_TOKEN".to_string()),
            ..Default::default()
        };

        let error = resolve_slack_host_beta_config(
            Some(&section),
            &tenant_id("tenant"),
            &agent_id("agent"),
            None,
            &user_id("web-user"),
            Path::new("/tmp/reborn-config.toml"),
        )
        .expect_err("empty secret env values must fail at config resolution");

        assert!(
            error
                .to_string()
                .contains("IRONCLAW_TEST_SLACK_EMPTY_SIGNING_SECRET must not be empty"),
            "message: {error}"
        );
        drop(_signing);
        drop(_bot);
    }

    #[cfg(feature = "slack-v2-host-beta")]
    #[test]
    fn slack_host_beta_config_accepts_matching_explicit_user_mapping() {
        let _lock = env_lock();
        let _signing = EnvGuard::set(
            "IRONCLAW_TEST_SLACK_SIGNING_SECRET_MAPPED_USER",
            "signing-secret",
        );
        let _bot = EnvGuard::set("IRONCLAW_TEST_SLACK_BOT_TOKEN_MAPPED_USER", "xoxb-token");
        let section = ironclaw_reborn_config::SlackSection {
            enabled: Some(true),
            installation_id: Some("install-alpha".to_string()),
            team_id: Some("T123".to_string()),
            api_app_id: Some("A123".to_string()),
            slack_user_id: Some("U123".to_string()),
            user_id: Some("web-user".to_string()),
            shared_subject_user_id: None,
            channel_routes: Vec::new(),
            signing_secret_env: Some("IRONCLAW_TEST_SLACK_SIGNING_SECRET_MAPPED_USER".to_string()),
            bot_token_env: Some("IRONCLAW_TEST_SLACK_BOT_TOKEN_MAPPED_USER".to_string()),
        };

        let resolved = resolve_slack_host_beta_config(
            Some(&section),
            &tenant_id("tenant"),
            &agent_id("agent"),
            None,
            &user_id("web-user"),
            Path::new("/tmp/reborn-config.toml"),
        )
        .expect("Slack config should resolve")
        .expect("Slack should be enabled");

        assert_eq!(resolved.user_id, user_id("web-user"));
        assert_eq!(resolved.shared_subject_user_id, None);
    }

    #[cfg(feature = "slack-v2-host-beta")]
    #[test]
    fn slack_host_beta_config_accepts_distinct_shared_subject_user() {
        let _lock = env_lock();
        let _signing = EnvGuard::set(
            "IRONCLAW_TEST_SLACK_SIGNING_SECRET_DIVERGENT_USER",
            "signing-secret",
        );
        let _bot = EnvGuard::set("IRONCLAW_TEST_SLACK_BOT_TOKEN_DIVERGENT_USER", "xoxb-token");
        let section = ironclaw_reborn_config::SlackSection {
            enabled: Some(true),
            installation_id: Some("install-alpha".to_string()),
            team_id: Some("T123".to_string()),
            api_app_id: Some("A123".to_string()),
            slack_user_id: Some("U123".to_string()),
            user_id: Some("web-user".to_string()),
            shared_subject_user_id: Some("slack-shared-subject".to_string()),
            channel_routes: Vec::new(),
            signing_secret_env: Some(
                "IRONCLAW_TEST_SLACK_SIGNING_SECRET_DIVERGENT_USER".to_string(),
            ),
            bot_token_env: Some("IRONCLAW_TEST_SLACK_BOT_TOKEN_DIVERGENT_USER".to_string()),
        };

        let resolved = resolve_slack_host_beta_config(
            Some(&section),
            &tenant_id("tenant"),
            &agent_id("agent"),
            None,
            &user_id("web-user"),
            Path::new("/tmp/reborn-config.toml"),
        )
        .expect("Slack config should resolve")
        .expect("Slack should be enabled");

        assert_eq!(resolved.user_id, user_id("web-user"));
        assert_eq!(
            resolved.shared_subject_user_id,
            Some(user_id("slack-shared-subject"))
        );
    }

    #[cfg(feature = "slack-v2-host-beta")]
    #[test]
    fn slack_host_beta_config_accepts_channel_routes() {
        let _lock = env_lock();
        let _signing = EnvGuard::set(
            "IRONCLAW_TEST_SLACK_SIGNING_SECRET_CHANNEL_ROUTES",
            "signing-secret",
        );
        let _bot = EnvGuard::set("IRONCLAW_TEST_SLACK_BOT_TOKEN_CHANNEL_ROUTES", "xoxb-token");
        let section = ironclaw_reborn_config::SlackSection {
            enabled: Some(true),
            installation_id: Some("install-alpha".to_string()),
            team_id: Some("T123".to_string()),
            api_app_id: Some("A123".to_string()),
            slack_user_id: Some("U123".to_string()),
            user_id: Some("web-user".to_string()),
            shared_subject_user_id: None,
            channel_routes: vec![ironclaw_reborn_config::SlackChannelRouteSection {
                channel_id: Some("CENG".to_string()),
                subject_user_id: Some("eng-team-agent".to_string()),
            }],
            signing_secret_env: Some(
                "IRONCLAW_TEST_SLACK_SIGNING_SECRET_CHANNEL_ROUTES".to_string(),
            ),
            bot_token_env: Some("IRONCLAW_TEST_SLACK_BOT_TOKEN_CHANNEL_ROUTES".to_string()),
        };

        let resolved = resolve_slack_host_beta_config(
            Some(&section),
            &tenant_id("tenant"),
            &agent_id("agent"),
            None,
            &user_id("web-user"),
            Path::new("/tmp/reborn-config.toml"),
        )
        .expect("Slack config should resolve")
        .expect("Slack should be enabled");

        assert_eq!(resolved.channel_routes.len(), 1);
        assert_eq!(resolved.channel_routes[0].channel_id, "CENG");
        assert_eq!(
            resolved.channel_routes[0].subject_user_id,
            user_id("eng-team-agent")
        );
    }

    #[cfg(feature = "slack-v2-host-beta")]
    #[test]
    fn slack_host_beta_config_channel_route_rejects_missing_channel_id() {
        let section = ironclaw_reborn_config::SlackSection {
            enabled: Some(true),
            installation_id: Some("install-alpha".to_string()),
            team_id: Some("T123".to_string()),
            api_app_id: Some("A123".to_string()),
            channel_routes: vec![ironclaw_reborn_config::SlackChannelRouteSection {
                channel_id: None,
                subject_user_id: Some("eng-team-agent".to_string()),
            }],
            ..Default::default()
        };

        let error = resolve_slack_host_beta_config(
            Some(&section),
            &tenant_id("tenant"),
            &agent_id("agent"),
            None,
            &user_id("web-user"),
            Path::new("/tmp/reborn-config.toml"),
        )
        .expect_err("channel route must require channel_id");

        assert!(
            error
                .to_string()
                .contains("[slack].channel_routes[0].channel_id must be set"),
            "message: {error}"
        );
    }

    #[cfg(feature = "slack-v2-host-beta")]
    #[test]
    fn slack_host_beta_config_channel_route_rejects_missing_subject_user_id() {
        let section = ironclaw_reborn_config::SlackSection {
            enabled: Some(true),
            installation_id: Some("install-alpha".to_string()),
            team_id: Some("T123".to_string()),
            api_app_id: Some("A123".to_string()),
            channel_routes: vec![ironclaw_reborn_config::SlackChannelRouteSection {
                channel_id: Some("CENG".to_string()),
                subject_user_id: None,
            }],
            ..Default::default()
        };

        let error = resolve_slack_host_beta_config(
            Some(&section),
            &tenant_id("tenant"),
            &agent_id("agent"),
            None,
            &user_id("web-user"),
            Path::new("/tmp/reborn-config.toml"),
        )
        .expect_err("channel route must require subject_user_id");

        assert!(
            error
                .to_string()
                .contains("[slack].channel_routes[0].subject_user_id must be set"),
            "message: {error}"
        );
    }

    #[cfg(feature = "slack-v2-host-beta")]
    #[test]
    fn slack_host_beta_config_channel_route_rejects_invalid_subject_user_id() {
        let section = ironclaw_reborn_config::SlackSection {
            enabled: Some(true),
            installation_id: Some("install-alpha".to_string()),
            team_id: Some("T123".to_string()),
            api_app_id: Some("A123".to_string()),
            channel_routes: vec![ironclaw_reborn_config::SlackChannelRouteSection {
                channel_id: Some("CENG".to_string()),
                subject_user_id: Some("invalid\nuser".to_string()),
            }],
            ..Default::default()
        };

        let error = resolve_slack_host_beta_config(
            Some(&section),
            &tenant_id("tenant"),
            &agent_id("agent"),
            None,
            &user_id("web-user"),
            Path::new("/tmp/reborn-config.toml"),
        )
        .expect_err("channel route subject_user_id must be valid");

        assert!(
            error
                .to_string()
                .contains("[slack].channel_routes[0].subject_user_id"),
            "message: {error}"
        );
    }

    #[cfg(feature = "slack-v2-host-beta")]
    #[test]
    fn slack_host_beta_config_rejects_padded_user_id_mapping() {
        let section = ironclaw_reborn_config::SlackSection {
            enabled: Some(true),
            installation_id: Some("install-alpha".to_string()),
            team_id: Some("T123".to_string()),
            api_app_id: Some("A123".to_string()),
            slack_user_id: Some("U123".to_string()),
            user_id: Some(" web-user".to_string()),
            ..Default::default()
        };

        let error = resolve_slack_host_beta_config(
            Some(&section),
            &tenant_id("tenant"),
            &agent_id("agent"),
            None,
            &user_id("web-user"),
            Path::new("/tmp/reborn-config.toml"),
        )
        .expect_err("padded Slack user mapping must fail at config resolution");

        assert!(
            error
                .to_string()
                .contains("[slack].user_id must not contain leading or trailing whitespace"),
            "message: {error}"
        );
    }

    #[cfg(feature = "slack-v2-host-beta")]
    #[test]
    fn slack_host_beta_config_rejects_invalid_user_id_mapping() {
        let _lock = env_lock();
        let _signing = EnvGuard::set(
            "IRONCLAW_TEST_SLACK_SIGNING_SECRET_INVALID_USER",
            "signing-secret",
        );
        let _bot = EnvGuard::set("IRONCLAW_TEST_SLACK_BOT_TOKEN_INVALID_USER", "xoxb-token");
        let section = ironclaw_reborn_config::SlackSection {
            enabled: Some(true),
            installation_id: Some("install-alpha".to_string()),
            team_id: Some("T123".to_string()),
            api_app_id: Some("A123".to_string()),
            slack_user_id: Some("U123".to_string()),
            user_id: Some("invalid\nuser".to_string()),
            shared_subject_user_id: None,
            channel_routes: Vec::new(),
            signing_secret_env: Some("IRONCLAW_TEST_SLACK_SIGNING_SECRET_INVALID_USER".to_string()),
            bot_token_env: Some("IRONCLAW_TEST_SLACK_BOT_TOKEN_INVALID_USER".to_string()),
        };

        let error = resolve_slack_host_beta_config(
            Some(&section),
            &tenant_id("tenant"),
            &agent_id("agent"),
            None,
            &user_id("web-user"),
            Path::new("/tmp/reborn-config.toml"),
        )
        .expect_err("invalid Slack user mapping must fail at config resolution");

        assert!(
            error.to_string().contains("[slack].user_id"),
            "message: {error}"
        );
    }

    #[cfg(feature = "slack-v2-host-beta")]
    #[test]
    fn slack_host_beta_config_rejects_padded_shared_subject_user_id() {
        let section = ironclaw_reborn_config::SlackSection {
            enabled: Some(true),
            installation_id: Some("install-alpha".to_string()),
            team_id: Some("T123".to_string()),
            api_app_id: Some("A123".to_string()),
            slack_user_id: Some("U123".to_string()),
            shared_subject_user_id: Some(" shared-subject".to_string()),
            ..Default::default()
        };

        let error = resolve_slack_host_beta_config(
            Some(&section),
            &tenant_id("tenant"),
            &agent_id("agent"),
            None,
            &user_id("web-user"),
            Path::new("/tmp/reborn-config.toml"),
        )
        .expect_err("padded shared subject user id must fail at config resolution");

        assert!(
            error.to_string().contains(
                "[slack].shared_subject_user_id must not contain leading or trailing whitespace"
            ),
            "message: {error}"
        );
    }

    #[cfg(feature = "slack-v2-host-beta")]
    #[test]
    fn slack_host_beta_config_reports_unset_signing_secret_env() {
        let _lock = env_lock();
        let _signing = EnvGuard::remove("IRONCLAW_TEST_SLACK_UNSET_SIGNING_SECRET");
        let _bot = EnvGuard::set("IRONCLAW_TEST_SLACK_BOT_TOKEN_UNSET_SIGNING", "xoxb-token");
        let section = ironclaw_reborn_config::SlackSection {
            enabled: Some(true),
            installation_id: Some("install-alpha".to_string()),
            team_id: Some("T123".to_string()),
            api_app_id: Some("A123".to_string()),
            slack_user_id: Some("U123".to_string()),
            signing_secret_env: Some("IRONCLAW_TEST_SLACK_UNSET_SIGNING_SECRET".to_string()),
            bot_token_env: Some("IRONCLAW_TEST_SLACK_BOT_TOKEN_UNSET_SIGNING".to_string()),
            ..Default::default()
        };

        let error = resolve_slack_host_beta_config(
            Some(&section),
            &tenant_id("tenant"),
            &agent_id("agent"),
            None,
            &user_id("web-user"),
            Path::new("/tmp/reborn-config.toml"),
        )
        .expect_err("unset signing secret env var must fail at config resolution");

        assert!(
            error
                .to_string()
                .contains("must be set to the Slack signing secret"),
            "message: {error}"
        );
    }

    #[cfg(not(feature = "slack-v2-host-beta"))]
    #[test]
    fn slack_host_beta_config_fails_loud_without_feature() {
        let section = ironclaw_reborn_config::SlackSection {
            enabled: Some(true),
            ..Default::default()
        };

        let error = reject_enabled_slack_without_feature(Some(&section))
            .expect_err("enabled Slack must require the host-beta feature");

        assert!(
            error.to_string().contains("slack-v2-host-beta"),
            "message: {error}"
        );
    }

    #[cfg(not(feature = "slack-v2-host-beta"))]
    #[test]
    fn reject_enabled_slack_without_feature_allows_disabled_section() {
        let section = ironclaw_reborn_config::SlackSection {
            enabled: Some(false),
            ..Default::default()
        };

        reject_enabled_slack_without_feature(Some(&section))
            .expect("disabled Slack config should be a no-op without the host-beta feature");
    }

    #[cfg(not(feature = "slack-v2-host-beta"))]
    #[test]
    fn reject_enabled_slack_without_feature_allows_absent_section() {
        reject_enabled_slack_without_feature(None)
            .expect("absent Slack config should be a no-op without the host-beta feature");
    }

    #[cfg(feature = "slack-v2-host-beta")]
    fn tenant_id(raw: &str) -> ironclaw_reborn_composition::host_api::TenantId {
        ironclaw_reborn_composition::host_api::TenantId::new(raw).expect("valid tenant id")
    }

    #[cfg(feature = "slack-v2-host-beta")]
    fn agent_id(raw: &str) -> ironclaw_reborn_composition::host_api::AgentId {
        ironclaw_reborn_composition::host_api::AgentId::new(raw).expect("valid agent id")
    }

    #[cfg(feature = "slack-v2-host-beta")]
    fn project_id(raw: &str) -> ironclaw_reborn_composition::host_api::ProjectId {
        ironclaw_reborn_composition::host_api::ProjectId::new(raw).expect("valid project id")
    }

    #[cfg(feature = "slack-v2-host-beta")]
    fn user_id(raw: &str) -> ironclaw_reborn_composition::host_api::UserId {
        ironclaw_reborn_composition::host_api::UserId::new(raw).expect("valid user id")
    }

    #[cfg(feature = "slack-v2-host-beta")]
    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        use std::sync::{Mutex, OnceLock};
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    #[cfg(feature = "slack-v2-host-beta")]
    struct EnvGuard {
        key: &'static str,
        prior: Option<String>,
    }

    #[cfg(feature = "slack-v2-host-beta")]
    impl EnvGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let prior = env::var(key).ok();
            // SAFETY: env mutation in these tests is serialized through `env_lock()`.
            unsafe {
                env::set_var(key, value);
            }
            Self { key, prior }
        }

        fn remove(key: &'static str) -> Self {
            let prior = env::var(key).ok();
            // SAFETY: env mutation in these tests is serialized through `env_lock()`.
            unsafe {
                env::remove_var(key);
            }
            Self { key, prior }
        }
    }

    #[cfg(feature = "slack-v2-host-beta")]
    impl Drop for EnvGuard {
        fn drop(&mut self) {
            // SAFETY: env mutation in these tests is serialized through `env_lock()`.
            unsafe {
                match &self.prior {
                    Some(value) => env::set_var(self.key, value),
                    None => env::remove_var(self.key),
                }
            }
        }
    }
}
