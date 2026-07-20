//! Static per-provider "did the operator configure this instance's OAuth
//! backend at all" readiness map.
//!
//! Deliberately a THIRD readiness axis, alongside static per-package
//! requirements (`extension_credential_requirements.rs`) and per-user account
//! setups (`ironclaw_product_workflow::ExtensionAccountSetupRegistry`) — the
//! same distinction `gsuite.rs:69-73` draws for the dispatch-time backstop
//! this module shares its build-time signal with. Do not fold this into
//! either of the other two axes: it answers "did the OPERATOR configure this
//! provider on this instance at all", not "does this package/user need a
//! credential" or "has this user connected an account".
//!
//! `RebornLocalExtensionManagementPort::activation_credential_requirements`
//! consults the built map BEFORE the per-account credential gate, so a
//! never-configured instance fails activation with actionable `config set`
//! remediation instead of parking an unresolvable `BlockedAuth` gate (design
//! §B chokepoint).

use std::collections::BTreeMap;

use ironclaw_host_api::{HostApiError, RuntimeCredentialAccountProviderId};

/// Build-time signals this composition build resolved for provider-instance
/// readiness. A named struct (not positional bools) so a new signal is an
/// obvious addition at the call site instead of a silent param-order hazard.
pub(crate) struct ProviderInstanceReadinessInputs {
    /// Same build-time signal `GsuiteFirstPartyHandler`'s dispatch backstop
    /// uses (`factory::google_oauth_configured`) — one source, two
    /// consumers, so the gate and this map can never drift apart.
    pub(crate) google_oauth_configured: bool,
    /// Slack host-beta build-time wiring signal:
    /// `RebornBuildInput::slack_host_beta_enabled`, resolved by the CLI
    /// serve path before the composition build.
    pub(crate) slack_host_beta_enabled: bool,
}

/// Build the readiness map: an entry is present only for a provider whose
/// instance-level configuration is missing. A provider absent from the map
/// (github, telegram — manual-token/pairing gates; notion-mcp — DCR;
/// web-access — no provider account) is unaffected by the chokepoint;
/// nearai-mcp is deliberately deferred: it is DCR-based like notion-mcp, so
/// it needs the same treatment before it can join this map.
pub(crate) fn provider_instance_readiness_map(
    inputs: ProviderInstanceReadinessInputs,
) -> Result<BTreeMap<RuntimeCredentialAccountProviderId, String>, HostApiError> {
    let mut map = BTreeMap::new();
    if !inputs.google_oauth_configured {
        map.insert(
            RuntimeCredentialAccountProviderId::new(ironclaw_auth::GOOGLE_PROVIDER_ID)?,
            format!(
                "{}\n\n{}",
                ironclaw_reborn_config::google_remediation_text(),
                ironclaw_reborn_config::apply_step_text()
            ),
        );
    }
    if !inputs.slack_host_beta_enabled {
        map.insert(
            RuntimeCredentialAccountProviderId::new(ironclaw_auth::SLACK_PERSONAL_PROVIDER_ID)?,
            format!(
                "{}\n\n{}",
                ironclaw_reborn_config::slack_remediation_text(),
                ironclaw_reborn_config::apply_step_text()
            ),
        );
    }
    Ok(map)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn google_provider() -> RuntimeCredentialAccountProviderId {
        RuntimeCredentialAccountProviderId::new(ironclaw_auth::GOOGLE_PROVIDER_ID)
            .expect("GOOGLE_PROVIDER_ID is a valid provider id")
    }

    #[test]
    fn google_entry_present_when_not_configured() {
        let map = provider_instance_readiness_map(ProviderInstanceReadinessInputs {
            google_oauth_configured: false,
            slack_host_beta_enabled: true,
        })
        .expect("map builds");
        let text = map.get(&google_provider()).expect("google entry present");
        assert!(text.contains("config set google.client_id"));
        assert!(text.contains("ironclaw service restart"));
    }

    #[test]
    fn google_entry_absent_when_configured() {
        let map = provider_instance_readiness_map(ProviderInstanceReadinessInputs {
            google_oauth_configured: true,
            slack_host_beta_enabled: true,
        })
        .expect("map builds");
        assert!(!map.contains_key(&google_provider()));
    }

    #[test]
    fn slack_entry_present_when_host_beta_not_enabled() {
        let map = provider_instance_readiness_map(ProviderInstanceReadinessInputs {
            google_oauth_configured: true,
            slack_host_beta_enabled: false,
        })
        .expect("map builds");
        let provider =
            RuntimeCredentialAccountProviderId::new(ironclaw_auth::SLACK_PERSONAL_PROVIDER_ID)
                .expect("SLACK_PERSONAL_PROVIDER_ID is a valid provider id");
        let text = map.get(&provider).expect("slack entry present");
        assert!(text.contains("config set slack.enabled"));
    }

    #[test]
    fn slack_entry_absent_when_host_beta_enabled() {
        let map = provider_instance_readiness_map(ProviderInstanceReadinessInputs {
            google_oauth_configured: true,
            slack_host_beta_enabled: true,
        })
        .expect("map builds");
        let provider =
            RuntimeCredentialAccountProviderId::new(ironclaw_auth::SLACK_PERSONAL_PROVIDER_ID)
                .expect("SLACK_PERSONAL_PROVIDER_ID is a valid provider id");
        assert!(!map.contains_key(&provider));
    }
}
