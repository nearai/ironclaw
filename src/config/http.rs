use crate::config::helpers::{
    db_first_bool, db_first_or_default, remove_runtime_env, set_runtime_env,
};
use crate::error::ConfigError;
use crate::settings::Settings;

pub(crate) const EFFECTIVE_HTTP_SECURITY_MODE_ENV: &str = "IRONCLAW_EFFECTIVE_HTTP_SECURITY_MODE";
pub(crate) const EFFECTIVE_HTTP_ALLOW_PRIVATE_HTTP_ENV: &str =
    "IRONCLAW_EFFECTIVE_HTTP_ALLOW_PRIVATE_HTTP";
pub(crate) const EFFECTIVE_HTTP_ALLOW_PRIVATE_IP_LITERALS_ENV: &str =
    "IRONCLAW_EFFECTIVE_HTTP_ALLOW_PRIVATE_IP_LITERALS";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpSecurityMode {
    Strict,
    InfraTrusted,
}

impl HttpSecurityMode {
    fn parse(value: &str, key: &str) -> Result<Self, ConfigError> {
        match value.trim().to_ascii_lowercase().as_str() {
            "strict" => Ok(Self::Strict),
            "infra_trusted" => Ok(Self::InfraTrusted),
            other => Err(ConfigError::InvalidValue {
                key: key.to_string(),
                message: format!("must be 'strict' or 'infra_trusted', got '{other}'"),
            }),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Strict => "strict",
            Self::InfraTrusted => "infra_trusted",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpSecurityConfig {
    pub security_mode: HttpSecurityMode,
    pub allow_private_http: bool,
    pub allow_private_ip_literals: bool,
}

impl Default for HttpSecurityConfig {
    fn default() -> Self {
        Self {
            security_mode: HttpSecurityMode::Strict,
            allow_private_http: false,
            allow_private_ip_literals: false,
        }
    }
}

impl HttpSecurityConfig {
    pub(crate) fn resolve(settings: &Settings) -> Result<Self, ConfigError> {
        let defaults = crate::settings::HttpSecuritySettings::default();
        let mode_raw = db_first_or_default(
            &settings.http.security_mode,
            &defaults.security_mode,
            "HTTP_SECURITY_MODE",
        )?;
        let security_mode = HttpSecurityMode::parse(&mode_raw, "HTTP_SECURITY_MODE")?;

        Ok(Self {
            security_mode,
            allow_private_http: db_first_bool(
                settings.http.allow_private_http,
                defaults.allow_private_http,
                "HTTP_ALLOW_PRIVATE_HTTP",
            )?,
            allow_private_ip_literals: db_first_bool(
                settings.http.allow_private_ip_literals,
                defaults.allow_private_ip_literals,
                "HTTP_ALLOW_PRIVATE_IP_LITERALS",
            )?,
        })
    }

    pub(crate) fn sync_runtime_env(&self) {
        set_runtime_env(
            EFFECTIVE_HTTP_SECURITY_MODE_ENV,
            self.security_mode.as_str(),
        );

        if self.allow_private_http {
            set_runtime_env(EFFECTIVE_HTTP_ALLOW_PRIVATE_HTTP_ENV, "true");
        } else {
            remove_runtime_env(EFFECTIVE_HTTP_ALLOW_PRIVATE_HTTP_ENV);
        }

        if self.allow_private_ip_literals {
            set_runtime_env(EFFECTIVE_HTTP_ALLOW_PRIVATE_IP_LITERALS_ENV, "true");
        } else {
            remove_runtime_env(EFFECTIVE_HTTP_ALLOW_PRIVATE_IP_LITERALS_ENV);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::helpers::lock_env;

    #[test]
    fn resolve_defaults_to_strict_mode() {
        let _guard = lock_env();
        let settings = Settings::default();
        let cfg = HttpSecurityConfig::resolve(&settings).expect("resolve");
        assert_eq!(cfg.security_mode, HttpSecurityMode::Strict);
        assert!(!cfg.allow_private_http);
        assert!(!cfg.allow_private_ip_literals);
    }

    #[test]
    fn settings_override_env() {
        let _guard = lock_env();
        let mut settings = Settings::default();
        settings.http.security_mode = "infra_trusted".to_string();
        settings.http.allow_private_http = true;

        // SAFETY: Under ENV_MUTEX, no concurrent env access.
        unsafe { std::env::set_var("HTTP_SECURITY_MODE", "strict") };
        unsafe { std::env::set_var("HTTP_ALLOW_PRIVATE_HTTP", "false") };

        let cfg = HttpSecurityConfig::resolve(&settings).expect("resolve");

        unsafe { std::env::remove_var("HTTP_SECURITY_MODE") };
        unsafe { std::env::remove_var("HTTP_ALLOW_PRIVATE_HTTP") };

        assert_eq!(cfg.security_mode, HttpSecurityMode::InfraTrusted);
        assert!(cfg.allow_private_http);
    }

    #[test]
    fn sync_runtime_env_publishes_effective_values() {
        let _guard = lock_env();
        let cfg = HttpSecurityConfig {
            security_mode: HttpSecurityMode::InfraTrusted,
            allow_private_http: true,
            allow_private_ip_literals: false,
        };
        cfg.sync_runtime_env();

        assert_eq!(
            crate::config::env_or_override(EFFECTIVE_HTTP_SECURITY_MODE_ENV).as_deref(),
            Some("infra_trusted")
        );
        assert_eq!(
            crate::config::env_or_override(EFFECTIVE_HTTP_ALLOW_PRIVATE_HTTP_ENV).as_deref(),
            Some("true")
        );
        assert_eq!(
            crate::config::env_or_override(EFFECTIVE_HTTP_ALLOW_PRIVATE_IP_LITERALS_ENV),
            None
        );
    }
}
