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
//! remediation instead of parking an unresolvable `BlockedAuth` gate. That
//! call site — `extension_lifecycle.rs::activation_credential_requirements` —
//! is the single chokepoint for this axis; do not add a second consultation
//! point elsewhere in the activation path.

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
    /// Whether `IRONCLAW_REBORN_SLACK_PERSONAL_OAUTH_REDIRECT_URI` resolved
    /// for this build. Slack personal OAuth needs this IN ADDITION to the
    /// host-beta signal above: with the route mounted but the redirect URI
    /// unset, the Connect button reaches
    /// `product_auth::serve::slack_personal_oauth_credentials` and gets a
    /// message-less 503 — the dead end this readiness map exists to replace
    /// with actionable text.
    pub(crate) slack_personal_oauth_redirect_uri_configured: bool,
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
            // The SAME body the dispatch-time backstop
            // (`gsuite::google_oauth_not_configured_error`) uses. The two
            // enforcement points stay separate (they gate different lifecycle
            // stages) but must not compose the text two different ways.
            ironclaw_reborn_config::google_setup_steps_text(),
        );
    }
    let slack_gaps = ironclaw_reborn_config::SlackSetupGaps {
        enable: !inputs.slack_host_beta_enabled,
        redirect_uri: !inputs.slack_personal_oauth_redirect_uri_configured,
    };
    if slack_gaps.enable || slack_gaps.redirect_uri {
        map.insert(
            RuntimeCredentialAccountProviderId::new(ironclaw_auth::SLACK_PERSONAL_PROVIDER_ID)?,
            // No `apply_step_text()` here, unlike google above: the Slack
            // variant embeds its own restart because Slack's apply step is
            // mid-sequence (the route must mount before the WebUI can run
            // workspace OAuth). Appending would double-print the restart and
            // misorder it after the connect step.
            ironclaw_reborn_config::slack_remediation_text(slack_gaps),
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

    fn slack_provider() -> RuntimeCredentialAccountProviderId {
        RuntimeCredentialAccountProviderId::new(ironclaw_auth::SLACK_PERSONAL_PROVIDER_ID)
            .expect("SLACK_PERSONAL_PROVIDER_ID is a valid provider id")
    }

    /// Fully-configured baseline; each test negates only the signal it covers.
    fn all_configured() -> ProviderInstanceReadinessInputs {
        ProviderInstanceReadinessInputs {
            google_oauth_configured: true,
            slack_host_beta_enabled: true,
            slack_personal_oauth_redirect_uri_configured: true,
        }
    }

    #[test]
    fn google_entry_present_when_not_configured() {
        let map = provider_instance_readiness_map(ProviderInstanceReadinessInputs {
            google_oauth_configured: false,
            ..all_configured()
        })
        .expect("map builds");
        let text = map.get(&google_provider()).expect("google entry present");
        assert!(text.contains("config set google.client_id"));
        // Google's apply step IS a trailing append (unlike slack's embedded,
        // mid-sequence one) — this pins that the append survived the slack change.
        assert!(text.contains("ironclaw service restart"));
    }

    #[test]
    fn google_entry_absent_when_configured() {
        let map = provider_instance_readiness_map(all_configured()).expect("map builds");
        assert!(!map.contains_key(&google_provider()));
    }

    #[test]
    fn slack_entry_present_when_host_beta_not_enabled() {
        let map = provider_instance_readiness_map(ProviderInstanceReadinessInputs {
            slack_host_beta_enabled: false,
            ..all_configured()
        })
        .expect("map builds");
        let text = map.get(&slack_provider()).expect("slack entry present");
        assert!(text.contains("config set slack.enabled"));
    }

    /// The gap Task A closes: host-beta enabled but the redirect URI unset
    /// still leaves the personal-OAuth slot empty, so activation must fail
    /// closed here instead of sending the user to a Connect button that 503s.
    #[test]
    fn slack_entry_present_when_redirect_uri_not_configured() {
        let map = provider_instance_readiness_map(ProviderInstanceReadinessInputs {
            slack_personal_oauth_redirect_uri_configured: false,
            ..all_configured()
        })
        .expect("map builds");
        let text = map.get(&slack_provider()).expect("slack entry present");
        assert!(
            text.contains("IRONCLAW_REBORN_SLACK_PERSONAL_OAUTH_REDIRECT_URI"),
            "the entry must name the missing redirect URI: {text}"
        );
        assert!(
            !text.contains("config set slack.enabled"),
            "slack.enabled is already applied; re-listing it is the confusion \
             the gap struct prevents: {text}"
        );
    }

    /// Regression guard for the double-restart this change removed: the slack
    /// entry embeds exactly one restart and the map appends none.
    #[test]
    fn slack_entry_states_the_restart_exactly_once() {
        let map = provider_instance_readiness_map(ProviderInstanceReadinessInputs {
            slack_host_beta_enabled: false,
            slack_personal_oauth_redirect_uri_configured: false,
            ..all_configured()
        })
        .expect("map builds");
        let text = map.get(&slack_provider()).expect("slack entry present");
        assert_eq!(
            text.matches("service restart").count(),
            1,
            "the readiness map must not append apply_step_text() on top of the \
             slack variant's own embedded restart: {text}"
        );
    }

    #[test]
    fn slack_entry_absent_when_fully_configured() {
        let map = provider_instance_readiness_map(all_configured()).expect("map builds");
        assert!(!map.contains_key(&slack_provider()));
    }
}
