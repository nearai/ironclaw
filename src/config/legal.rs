use std::path::PathBuf;

use crate::config::helpers::{optional_env, parse_bool_env, parse_string_env};
use crate::error::ConfigError;
use crate::settings::Settings;

/// Hardening profile for legal-mode enforcement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegalHardeningProfile {
    Standard,
    MaxLockdown,
}

impl LegalHardeningProfile {
    fn from_str(value: &str) -> Result<Self, ConfigError> {
        match value.to_ascii_lowercase().as_str() {
            "standard" => Ok(Self::Standard),
            "max_lockdown" | "max-lockdown" => Ok(Self::MaxLockdown),
            other => Err(ConfigError::InvalidValue {
                key: "LEGAL_HARDENING".to_string(),
                message: format!("unsupported profile '{other}'"),
            }),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::MaxLockdown => "max_lockdown",
        }
    }
}

/// Legal network controls.
#[derive(Debug, Clone)]
pub struct LegalNetworkConfig {
    pub deny_by_default: bool,
    pub allowed_domains: Vec<String>,
}

/// Legal audit controls.
#[derive(Debug, Clone)]
pub struct LegalAuditConfig {
    pub enabled: bool,
    pub path: PathBuf,
    pub hash_chain: bool,
}

/// Legal redaction controls.
#[derive(Debug, Clone)]
pub struct LegalRedactionConfig {
    pub pii: bool,
    pub phi: bool,
    pub financial: bool,
    pub government_id: bool,
}

/// Legal workflow profile and policy controls.
#[derive(Debug, Clone)]
pub struct LegalConfig {
    pub enabled: bool,
    pub jurisdiction: String,
    pub hardening: LegalHardeningProfile,
    pub require_matter_context: bool,
    pub citation_required: bool,
    pub matter_root: String,
    pub active_matter: Option<String>,
    pub privilege_guard: bool,
    pub conflict_check_enabled: bool,
    pub network: LegalNetworkConfig,
    pub audit: LegalAuditConfig,
    pub redaction: LegalRedactionConfig,
}

fn parse_domains_csv(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_ascii_lowercase())
        .collect()
}

impl LegalConfig {
    pub(crate) fn resolve(settings: &Settings) -> Result<Self, ConfigError> {
        let hardening_raw = parse_string_env("LEGAL_HARDENING", settings.legal.hardening.clone())?;
        let hardening = LegalHardeningProfile::from_str(&hardening_raw)?;

        let allowed_domains = match optional_env("LEGAL_NETWORK_ALLOWED_DOMAINS")? {
            Some(raw) => parse_domains_csv(&raw),
            None => settings
                .legal
                .network
                .allowed_domains
                .iter()
                .map(|d| d.to_ascii_lowercase())
                .collect(),
        };

        Ok(Self {
            enabled: parse_bool_env("LEGAL_ENABLED", settings.legal.enabled)?,
            jurisdiction: parse_string_env(
                "LEGAL_JURISDICTION",
                settings.legal.jurisdiction.clone(),
            )?,
            hardening,
            require_matter_context: parse_bool_env(
                "LEGAL_REQUIRE_MATTER_CONTEXT",
                settings.legal.require_matter_context,
            )?,
            citation_required: parse_bool_env(
                "LEGAL_CITATION_REQUIRED",
                settings.legal.citation_required,
            )?,
            matter_root: parse_string_env("LEGAL_MATTER_ROOT", settings.legal.matter_root.clone())?,
            active_matter: optional_env("LEGAL_MATTER")?
                .or_else(|| optional_env("MATTER_ID").ok().flatten())
                .or_else(|| settings.legal.active_matter.clone()),
            privilege_guard: parse_bool_env(
                "LEGAL_PRIVILEGE_GUARD",
                settings.legal.privilege_guard,
            )?,
            conflict_check_enabled: parse_bool_env(
                "LEGAL_CONFLICT_CHECK_ENABLED",
                settings.legal.conflict_check_enabled,
            )?,
            network: LegalNetworkConfig {
                deny_by_default: parse_bool_env(
                    "LEGAL_NETWORK_DENY_BY_DEFAULT",
                    settings.legal.network.deny_by_default,
                )?,
                allowed_domains,
            },
            audit: LegalAuditConfig {
                enabled: parse_bool_env("LEGAL_AUDIT_ENABLED", settings.legal.audit.enabled)?,
                path: PathBuf::from(parse_string_env(
                    "LEGAL_AUDIT_PATH",
                    settings.legal.audit.path.clone(),
                )?),
                hash_chain: parse_bool_env(
                    "LEGAL_AUDIT_HASH_CHAIN",
                    settings.legal.audit.hash_chain,
                )?,
            },
            redaction: LegalRedactionConfig {
                pii: parse_bool_env("LEGAL_REDACTION_PII", settings.legal.redaction.pii)?,
                phi: parse_bool_env("LEGAL_REDACTION_PHI", settings.legal.redaction.phi)?,
                financial: parse_bool_env(
                    "LEGAL_REDACTION_FINANCIAL",
                    settings.legal.redaction.financial,
                )?,
                government_id: parse_bool_env(
                    "LEGAL_REDACTION_GOVERNMENT_ID",
                    settings.legal.redaction.government_id,
                )?,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::settings::Settings;

    #[test]
    fn legal_resolve_uses_secure_defaults() {
        let settings = Settings::default();
        let config = super::LegalConfig::resolve(&settings).expect("legal config");

        assert!(config.enabled);
        assert_eq!(config.jurisdiction, "us-general");
        assert_eq!(config.hardening.as_str(), "max_lockdown");
        assert!(config.require_matter_context);
        assert!(config.citation_required);
        assert_eq!(config.matter_root, "matters");
        assert!(config.network.deny_by_default);
        assert!(config.audit.enabled);
        assert!(config.audit.hash_chain);
    }
}
