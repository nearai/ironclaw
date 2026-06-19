//! Telegram host-beta config resolution for `ironclaw-reborn serve`.
//!
//! Mirrors `serve_slack` but for the Telegram webhook host-ingress path. Secrets
//! are env-only: `[telegram].bot_token_env` / `[telegram].secret_token_env` name
//! the environment variables that hold the bot token and webhook shared secret;
//! the values never live in the config file.

#[cfg(feature = "telegram-v2-host-beta")]
use anyhow::anyhow;

#[cfg(feature = "telegram-v2-host-beta")]
use std::env;
#[cfg(feature = "telegram-v2-host-beta")]
use std::path::Path;

#[cfg(feature = "telegram-v2-host-beta")]
use ironclaw_reborn_composition::TelegramHostBetaConfig;
#[cfg(feature = "telegram-v2-host-beta")]
use secrecy::SecretString;

#[cfg(feature = "telegram-v2-host-beta")]
const DEFAULT_TELEGRAM_BOT_TOKEN_ENV_VAR: &str = "IRONCLAW_REBORN_TELEGRAM_BOT_TOKEN";
#[cfg(feature = "telegram-v2-host-beta")]
const DEFAULT_TELEGRAM_SECRET_TOKEN_ENV_VAR: &str = "IRONCLAW_REBORN_TELEGRAM_SECRET_TOKEN";

#[cfg(feature = "telegram-v2-host-beta")]
pub(crate) fn resolve_telegram_config_for_serve(
    section: Option<&ironclaw_reborn_config::TelegramSection>,
    tenant_id: &ironclaw_reborn_composition::host_api::TenantId,
    default_agent_id: &ironclaw_reborn_composition::host_api::AgentId,
    default_project_id: Option<&ironclaw_reborn_composition::host_api::ProjectId>,
    default_user_id: &ironclaw_reborn_composition::host_api::UserId,
    config_path: &Path,
) -> anyhow::Result<Option<TelegramHostBetaConfig>> {
    resolve_telegram_host_beta_config(
        section,
        tenant_id,
        default_agent_id,
        default_project_id,
        default_user_id,
        config_path,
    )
}

#[cfg(not(feature = "telegram-v2-host-beta"))]
pub(crate) fn resolve_telegram_config_for_serve(
    section: Option<&ironclaw_reborn_config::TelegramSection>,
    _tenant_id: &ironclaw_reborn_composition::host_api::TenantId,
    _default_agent_id: &ironclaw_reborn_composition::host_api::AgentId,
    _default_project_id: Option<&ironclaw_reborn_composition::host_api::ProjectId>,
    _default_user_id: &ironclaw_reborn_composition::host_api::UserId,
    _config_path: &std::path::Path,
) -> anyhow::Result<Option<()>> {
    reject_enabled_telegram_without_feature(section)?;
    Ok(None)
}

#[cfg(feature = "telegram-v2-host-beta")]
pub(crate) fn resolve_telegram_host_beta_config(
    section: Option<&ironclaw_reborn_config::TelegramSection>,
    tenant_id: &ironclaw_reborn_composition::host_api::TenantId,
    default_agent_id: &ironclaw_reborn_composition::host_api::AgentId,
    default_project_id: Option<&ironclaw_reborn_composition::host_api::ProjectId>,
    default_user_id: &ironclaw_reborn_composition::host_api::UserId,
    config_path: &Path,
) -> anyhow::Result<Option<TelegramHostBetaConfig>> {
    let Some(section) = section else {
        return Ok(None);
    };
    if section.enabled != Some(true) {
        return Ok(None);
    }

    let installation_id_raw =
        required_telegram_config_value("installation_id", &section.installation_id, config_path)?;
    let installation_id =
        ironclaw_reborn_composition::AdapterInstallationId::new(&installation_id_raw).map_err(
            |err| anyhow!("[telegram].installation_id `{installation_id_raw}` is invalid: {err}"),
        )?;
    let bot_username =
        required_telegram_config_value("bot_username", &section.bot_username, config_path)?;
    let bot_user_id = section.bot_user_id.ok_or_else(|| {
        anyhow!(
            "[telegram].bot_user_id must be set when [telegram].enabled = true in {}",
            config_path.display()
        )
    })?;
    let mapped_user_id = optional_telegram_user_id_config_value("user_id", &section.user_id)?
        .unwrap_or_else(|| default_user_id.clone());
    let shared_subject_user_id = optional_telegram_user_id_config_value(
        "shared_subject_user_id",
        &section.shared_subject_user_id,
    )?;
    let recognized_commands = section
        .recognized_commands
        .iter()
        .enumerate()
        .map(|(index, command)| {
            optional_telegram_config_value(
                &format!("recognized_commands[{index}]"),
                &Some(command.clone()),
            )?
            .ok_or_else(|| anyhow!("[telegram].recognized_commands[{index}] must not be empty"))
        })
        .collect::<anyhow::Result<Vec<_>>>()?;

    let bot_token_env = optional_telegram_config_value("bot_token_env", &section.bot_token_env)?
        .unwrap_or_else(|| DEFAULT_TELEGRAM_BOT_TOKEN_ENV_VAR.to_string());
    let secret_token_env =
        optional_telegram_config_value("secret_token_env", &section.secret_token_env)?
            .unwrap_or_else(|| DEFAULT_TELEGRAM_SECRET_TOKEN_ENV_VAR.to_string());
    let bot_token = required_env_secret("bot token", "bot_token_env", &bot_token_env, config_path)?;
    let webhook_secret = required_env_secret(
        "webhook secret",
        "secret_token_env",
        &secret_token_env,
        config_path,
    )?;

    Ok(Some(TelegramHostBetaConfig {
        tenant_id: tenant_id.clone(),
        installation_id,
        user_id: mapped_user_id,
        agent_id: default_agent_id.clone(),
        project_id: default_project_id.cloned(),
        shared_subject_user_id,
        bot_username,
        bot_user_id,
        recognized_commands,
        bot_token: SecretString::from(bot_token),
        webhook_secret: SecretString::from(webhook_secret),
        progress_push_enabled: section.progress_push_enabled.unwrap_or(false),
    }))
}

#[cfg(feature = "telegram-v2-host-beta")]
fn required_telegram_config_value(
    field: &str,
    value: &Option<String>,
    config_path: &Path,
) -> anyhow::Result<String> {
    optional_telegram_config_value(field, value)?.ok_or_else(|| {
        anyhow!(
            "[telegram].{field} must be set when [telegram].enabled = true in {}",
            config_path.display()
        )
    })
}

#[cfg(feature = "telegram-v2-host-beta")]
fn optional_telegram_config_value(
    field: &str,
    value: &Option<String>,
) -> anyhow::Result<Option<String>> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.trim().is_empty() {
        anyhow::bail!("[telegram].{field} must not be empty when set");
    }
    if value.trim() != value {
        anyhow::bail!(
            "[telegram].{field} must not contain leading or trailing whitespace when set"
        );
    }
    Ok(Some(value.clone()))
}

#[cfg(feature = "telegram-v2-host-beta")]
fn optional_telegram_user_id_config_value(
    field: &str,
    value: &Option<String>,
) -> anyhow::Result<Option<ironclaw_reborn_composition::host_api::UserId>> {
    optional_telegram_config_value(field, value)?
        .map(|raw| {
            ironclaw_reborn_composition::host_api::UserId::new(&raw)
                .map_err(|err| anyhow!("[telegram].{field} `{raw}` is invalid: {err}"))
        })
        .transpose()
}

#[cfg(feature = "telegram-v2-host-beta")]
fn required_env_secret(
    label: &'static str,
    field: &'static str,
    env_var: &str,
    config_path: &Path,
) -> anyhow::Result<String> {
    let value = env::var(env_var).map_err(|_| {
        anyhow!(
            "{env_var} must be set to the Telegram {label} when [telegram].enabled = true. \
             Override the variable name via [telegram].{field} in {}.",
            config_path.display()
        )
    })?;
    if value.is_empty() {
        anyhow::bail!("{env_var} must not be empty when [telegram].enabled = true");
    }
    Ok(value)
}

#[cfg(not(feature = "telegram-v2-host-beta"))]
pub(crate) fn reject_enabled_telegram_without_feature(
    section: Option<&ironclaw_reborn_config::TelegramSection>,
) -> anyhow::Result<()> {
    if section.and_then(|section| section.enabled).unwrap_or(false) {
        anyhow::bail!(
            "[telegram].enabled = true requires an ironclaw-reborn binary built with \
             the `telegram-v2-host-beta` Cargo feature"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "telegram-v2-host-beta")]
    use secrecy::ExposeSecret;

    #[cfg(feature = "telegram-v2-host-beta")]
    #[test]
    fn telegram_host_beta_config_is_disabled_unless_explicitly_enabled() {
        let section = ironclaw_reborn_config::TelegramSection {
            enabled: None,
            ..Default::default()
        };

        let resolved = resolve_telegram_host_beta_config(
            Some(&section),
            &tenant_id("tenant"),
            &agent_id("agent"),
            None,
            &user_id("web-user"),
            Path::new("/tmp/reborn-config.toml"),
        )
        .expect("disabled Telegram should not require fields or env vars");

        assert!(resolved.is_none());
    }

    #[cfg(feature = "telegram-v2-host-beta")]
    #[test]
    fn telegram_host_beta_config_requires_installation_id_when_enabled() {
        let section = ironclaw_reborn_config::TelegramSection {
            enabled: Some(true),
            ..Default::default()
        };

        let error = resolve_telegram_host_beta_config(
            Some(&section),
            &tenant_id("tenant"),
            &agent_id("agent"),
            None,
            &user_id("web-user"),
            Path::new("/tmp/reborn-config.toml"),
        )
        .expect_err("enabled Telegram must require an installation id");

        assert!(
            error.to_string().contains("[telegram].installation_id"),
            "message: {error}"
        );
    }

    #[cfg(feature = "telegram-v2-host-beta")]
    #[test]
    fn telegram_host_beta_config_requires_bot_user_id() {
        let section = ironclaw_reborn_config::TelegramSection {
            enabled: Some(true),
            installation_id: Some("telegram-default".to_string()),
            bot_username: Some("my_bot".to_string()),
            ..Default::default()
        };

        let error = resolve_telegram_host_beta_config(
            Some(&section),
            &tenant_id("tenant"),
            &agent_id("agent"),
            None,
            &user_id("web-user"),
            Path::new("/tmp/reborn-config.toml"),
        )
        .expect_err("enabled Telegram must require bot_user_id");

        assert!(
            error.to_string().contains("[telegram].bot_user_id"),
            "message: {error}"
        );
    }

    #[cfg(feature = "telegram-v2-host-beta")]
    #[test]
    fn telegram_host_beta_config_reads_env_secrets_and_defaults_user() {
        let _lock = env_lock();
        let _bot = EnvGuard::set("IRONCLAW_TEST_TELEGRAM_BOT_TOKEN_DEFAULT", "123:bot-token");
        let _secret = EnvGuard::set("IRONCLAW_TEST_TELEGRAM_SECRET_DEFAULT", "webhook-secret");
        let section = ironclaw_reborn_config::TelegramSection {
            enabled: Some(true),
            installation_id: Some("telegram-default".to_string()),
            bot_username: Some("my_bot".to_string()),
            bot_user_id: Some(123_456_789),
            bot_token_env: Some("IRONCLAW_TEST_TELEGRAM_BOT_TOKEN_DEFAULT".to_string()),
            secret_token_env: Some("IRONCLAW_TEST_TELEGRAM_SECRET_DEFAULT".to_string()),
            ..Default::default()
        };

        let resolved = resolve_telegram_host_beta_config(
            Some(&section),
            &tenant_id("tenant"),
            &agent_id("agent"),
            Some(&project_id("project")),
            &user_id("web-user"),
            Path::new("/tmp/reborn-config.toml"),
        )
        .expect("Telegram config should resolve")
        .expect("Telegram should be enabled");

        assert_eq!(resolved.installation_id.as_str(), "telegram-default");
        assert_eq!(resolved.bot_username, "my_bot");
        assert_eq!(resolved.bot_user_id, 123_456_789);
        assert_eq!(resolved.user_id, user_id("web-user"));
        assert_eq!(resolved.bot_token.expose_secret(), "123:bot-token");
        assert_eq!(resolved.webhook_secret.expose_secret(), "webhook-secret");
        assert!(!resolved.progress_push_enabled);
    }

    #[cfg(feature = "telegram-v2-host-beta")]
    #[test]
    fn telegram_host_beta_config_reports_unset_bot_token_env() {
        let _lock = env_lock();
        let _bot = EnvGuard::remove("IRONCLAW_TEST_TELEGRAM_BOT_TOKEN_UNSET");
        let _secret = EnvGuard::set("IRONCLAW_TEST_TELEGRAM_SECRET_UNSET", "webhook-secret");
        let section = ironclaw_reborn_config::TelegramSection {
            enabled: Some(true),
            installation_id: Some("telegram-default".to_string()),
            bot_username: Some("my_bot".to_string()),
            bot_user_id: Some(123_456_789),
            bot_token_env: Some("IRONCLAW_TEST_TELEGRAM_BOT_TOKEN_UNSET".to_string()),
            secret_token_env: Some("IRONCLAW_TEST_TELEGRAM_SECRET_UNSET".to_string()),
            ..Default::default()
        };

        let error = resolve_telegram_host_beta_config(
            Some(&section),
            &tenant_id("tenant"),
            &agent_id("agent"),
            None,
            &user_id("web-user"),
            Path::new("/tmp/reborn-config.toml"),
        )
        .expect_err("unset bot token env var must fail at config resolution");

        assert!(
            error
                .to_string()
                .contains("must be set to the Telegram bot token"),
            "message: {error}"
        );
    }

    #[cfg(not(feature = "telegram-v2-host-beta"))]
    #[test]
    fn telegram_host_beta_config_fails_loud_without_feature() {
        let section = ironclaw_reborn_config::TelegramSection {
            enabled: Some(true),
            ..Default::default()
        };

        let error = reject_enabled_telegram_without_feature(Some(&section))
            .expect_err("enabled Telegram must require the host-beta feature");

        assert!(
            error.to_string().contains("telegram-v2-host-beta"),
            "message: {error}"
        );
    }

    #[cfg(not(feature = "telegram-v2-host-beta"))]
    #[test]
    fn reject_enabled_telegram_without_feature_allows_disabled_and_absent() {
        let disabled = ironclaw_reborn_config::TelegramSection {
            enabled: Some(false),
            ..Default::default()
        };
        reject_enabled_telegram_without_feature(Some(&disabled))
            .expect("disabled Telegram config should be a no-op without the feature");
        reject_enabled_telegram_without_feature(None)
            .expect("absent Telegram config should be a no-op without the feature");
    }

    #[cfg(feature = "telegram-v2-host-beta")]
    fn tenant_id(raw: &str) -> ironclaw_reborn_composition::host_api::TenantId {
        ironclaw_reborn_composition::host_api::TenantId::new(raw).expect("valid tenant id")
    }

    #[cfg(feature = "telegram-v2-host-beta")]
    fn agent_id(raw: &str) -> ironclaw_reborn_composition::host_api::AgentId {
        ironclaw_reborn_composition::host_api::AgentId::new(raw).expect("valid agent id")
    }

    #[cfg(feature = "telegram-v2-host-beta")]
    fn project_id(raw: &str) -> ironclaw_reborn_composition::host_api::ProjectId {
        ironclaw_reborn_composition::host_api::ProjectId::new(raw).expect("valid project id")
    }

    #[cfg(feature = "telegram-v2-host-beta")]
    fn user_id(raw: &str) -> ironclaw_reborn_composition::host_api::UserId {
        ironclaw_reborn_composition::host_api::UserId::new(raw).expect("valid user id")
    }

    #[cfg(feature = "telegram-v2-host-beta")]
    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        use std::sync::{Mutex, OnceLock};
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    #[cfg(feature = "telegram-v2-host-beta")]
    struct EnvGuard {
        key: &'static str,
        prior: Option<String>,
    }

    #[cfg(feature = "telegram-v2-host-beta")]
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

    #[cfg(feature = "telegram-v2-host-beta")]
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
