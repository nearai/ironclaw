use crate::error::ConfigError;
use crate::security::outbound_trust::OutboundTrustConfig;

/// Security runtime config resolved from persisted settings.
#[derive(Debug, Clone, Default)]
pub struct SecurityConfig {
    pub outbound_trust: OutboundTrustConfig,
}

pub(crate) fn resolve_security_config(
    settings: &crate::settings::Settings,
) -> Result<SecurityConfig, ConfigError> {
    Ok(SecurityConfig {
        outbound_trust: settings.security.outbound_trust.to_runtime_config(),
    })
}

#[cfg(test)]
mod tests {
    use crate::config::security::resolve_security_config;
    use crate::security::outbound_trust::{OutboundTrustRisk, OutboundTrustSurface};
    use crate::settings::{OutboundTrustPolicySettings, OutboundTrustTargetSettings, Settings};

    #[test]
    fn resolve_uses_settings_outbound_trust() {
        let mut settings = Settings::default();
        settings.security.outbound_trust.enabled = true;
        settings.security.outbound_trust.policies = vec![OutboundTrustPolicySettings {
            id: "corp-internal-api".to_string(),
            display_name: "corp-internal-api".to_string(),
            description: Some("internal API".to_string()),
            enabled: true,
            allowed_surfaces: vec![OutboundTrustSurface::WasmTool],
            allowed_risks: vec![OutboundTrustRisk::AllowInvalidTls],
            targets: vec![OutboundTrustTargetSettings {
                host: "internal-api.example.test".to_string(),
                port: Some(443),
                path_prefix: Some("/api".to_string()),
            }],
        }];

        let cfg = resolve_security_config(&settings).expect("resolve");
        assert!(cfg.outbound_trust.enabled);
        assert_eq!(cfg.outbound_trust.policies.len(), 1);
        assert_eq!(cfg.outbound_trust.policies[0].id, "corp-internal-api");
    }
}
