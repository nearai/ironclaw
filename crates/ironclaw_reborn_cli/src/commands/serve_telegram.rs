use ironclaw_reborn_composition::TelegramHostRuntimeConfig;

const TELEGRAM_ENABLED_ENV: &str = "IRONCLAW_REBORN_TELEGRAM_ENABLED";

pub(crate) fn resolve_telegram_config_for_serve(
    section: Option<&ironclaw_reborn_config::TelegramSection>,
    tenant_id: &ironclaw_reborn_composition::host_api::TenantId,
    default_agent_id: &ironclaw_reborn_composition::host_api::AgentId,
    default_project_id: Option<&ironclaw_reborn_composition::host_api::ProjectId>,
    default_user_id: &ironclaw_reborn_composition::host_api::UserId,
    public_base_url: Option<String>,
) -> anyhow::Result<Option<TelegramHostRuntimeConfig>> {
    let enabled = telegram_enabled(section)?;
    if !enabled {
        return Ok(None);
    }
    Ok(Some(TelegramHostRuntimeConfig::new(
        tenant_id.clone(),
        default_agent_id.clone(),
        default_project_id.cloned(),
        default_user_id.clone(),
        public_base_url,
    )))
}

fn telegram_enabled(
    section: Option<&ironclaw_reborn_config::TelegramSection>,
) -> anyhow::Result<bool> {
    match std::env::var(TELEGRAM_ENABLED_ENV) {
        Ok(value) => {
            crate::commands::parse_channel_enabled_bool(TELEGRAM_ENABLED_ENV, value.as_str())
        }
        Err(std::env::VarError::NotPresent) => Ok(section.and_then(|s| s.enabled).unwrap_or(false)),
        Err(std::env::VarError::NotUnicode(_)) => {
            anyhow::bail!("{TELEGRAM_ENABLED_ENV} must be valid UTF-8 when set")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tenant_id(raw: &str) -> ironclaw_reborn_composition::host_api::TenantId {
        ironclaw_reborn_composition::host_api::TenantId::new(raw).expect("valid tenant")
    }

    fn agent_id(raw: &str) -> ironclaw_reborn_composition::host_api::AgentId {
        ironclaw_reborn_composition::host_api::AgentId::new(raw).expect("valid agent")
    }

    fn project_id(raw: &str) -> ironclaw_reborn_composition::host_api::ProjectId {
        ironclaw_reborn_composition::host_api::ProjectId::new(raw).expect("valid project")
    }

    fn user_id(raw: &str) -> ironclaw_reborn_composition::host_api::UserId {
        ironclaw_reborn_composition::host_api::UserId::new(raw).expect("valid user")
    }

    #[test]
    fn telegram_host_config_uses_webui_scope_and_public_base_url_when_enabled() {
        let project_id = project_id("project");
        let section = ironclaw_reborn_config::TelegramSection::default().set_enabled(true);

        let resolved = resolve_telegram_config_for_serve(
            Some(&section),
            &tenant_id("tenant"),
            &agent_id("agent"),
            Some(&project_id),
            &user_id("web-user"),
            Some("https://ironclaw.example".to_string()),
        )
        .expect("Telegram config resolves")
        .expect("Telegram enabled");

        assert_eq!(resolved.tenant_id.as_str(), "tenant");
        assert_eq!(resolved.agent_id.as_str(), "agent");
        assert_eq!(
            resolved.project_id.as_ref().map(|id| id.as_str()),
            Some("project")
        );
        assert_eq!(resolved.operator_user_id.as_str(), "web-user");
        assert_eq!(
            resolved.public_base_url.as_deref(),
            Some("https://ironclaw.example")
        );
    }

    #[test]
    fn telegram_host_config_is_absent_without_section() {
        let resolved = resolve_telegram_config_for_serve(
            None,
            &tenant_id("tenant"),
            &agent_id("agent"),
            None,
            &user_id("web-user"),
            None,
        )
        .expect("Telegram config resolves without section");

        assert!(resolved.is_none());
    }

    #[test]
    fn parse_telegram_enabled_bool_accepts_known_values_and_rejects_garbage() {
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
}
