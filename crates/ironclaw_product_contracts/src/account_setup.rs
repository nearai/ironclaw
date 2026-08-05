//! Extension account-setup vocabulary and the account-status port
//! (PROPOSAL §6.1.3).
//!
//! An extension whose activation depends on a user-scoped external account
//! declares immutable setup metadata once, and the crate that hosts that
//! account answers "is this user connected?" through
//! [`AccountConnectionStatusSource`]. The declaration registry itself is
//! product-owned mutable state and stays in `ironclaw_assistant`; what lives
//! here is the descriptor it stores, the sanitized error classes it reports,
//! the probe port `ironclaw_extension_host` implements over its pairing
//! service, and — since WS2.5 — [`ExtensionAccountSetupReader`], the registry's
//! two-method *read* surface the extension host consumes.
//!
//! Never here: the registry, activation preflight policy, or any
//! implementation of either port.

use async_trait::async_trait;
use ironclaw_host_api::{
    decision::RuntimeCredentialAuthRequirement,
    ids::{ExtensionId, UserId},
};
use thiserror::Error;

use crate::package_lifecycle::ChannelConnectionRequirement;

/// A connection-status read failed inside the extension-owned host service.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("account connection status read failed: {reason}")]
pub struct AccountConnectionStatusError {
    reason: String,
}

impl AccountConnectionStatusError {
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }
}

/// Narrow per-user account-connection probe used during activation preflight.
#[async_trait]
pub trait AccountConnectionStatusSource: Send + Sync + std::fmt::Debug {
    async fn connected(&self, user_id: &UserId) -> Result<bool, AccountConnectionStatusError>;
}

/// Product-owned copy for a channel account's pairing lifecycle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelConnectionNoticePolicy {
    pub connect_required: String,
    pub paired: String,
    pub already_paired_same_user: String,
    pub already_bound_to_other_user: String,
    pub expired_or_unknown: String,
}

impl ChannelConnectionNoticePolicy {
    pub fn generic(display_name: &str) -> Self {
        Self {
            connect_required: format!(
                "👋 To use {display_name}, connect it in the Ironclaw web app, then message me here again."
            ),
            paired: format!("✅ {display_name} is paired. You can talk to Ironclaw here."),
            already_paired_same_user: format!(
                "✅ This {display_name} account is already paired to you."
            ),
            already_bound_to_other_user: format!(
                "This {display_name} account is already paired to another Ironclaw user."
            ),
            expired_or_unknown: format!(
                "That {display_name} pairing code is invalid or expired. Get a fresh code from Ironclaw and try again."
            ),
        }
    }
}

/// Immutable product metadata for an extension whose activation depends on a
/// user-scoped external-account connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionAccountSetupDescriptor {
    pub extension_id: ExtensionId,
    pub auth_requirement: RuntimeCredentialAuthRequirement,
    pub connection_requirement: ChannelConnectionRequirement,
    pub connection_notices: ChannelConnectionNoticePolicy,
    pub activation_success_message: String,
    /// `WebGeneratedCode` presentation: an optional deep-link template with
    /// `{code}` plus non-secret `[channel.config]` field-handle placeholders
    /// (e.g. `https://vendor.example/{bot_username}?start={code}`). `None`
    /// presents the minted code alone.
    pub pairing_deep_link_template: Option<String>,
    /// Exact message prefixes the channel's pairing parser may strip before
    /// validating a host-issued proof code.
    pub inbound_code_prefixes: Vec<String>,
}

/// Sanitized lifecycle classification for an unavailable setup host or status
/// backend. The concrete backend error never crosses this boundary.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ExtensionAccountSetupError {
    #[error("account setup host is unavailable for extension {extension_id}")]
    HostUnavailable { extension_id: ExtensionId },
    #[error("account connection status is unavailable for extension {extension_id}")]
    StatusUnavailable {
        extension_id: ExtensionId,
        #[source]
        source: AccountConnectionStatusError,
    },
}

/// The read half of the product-owned account-setup registry, as the extension
/// host consumes it.
///
/// The registry itself — single-assignment declarations plus connected status
/// sources, under a lock — is product-owned mutable state and stays in
/// `ironclaw_assistant`, exactly as this module's header says. What the extension
/// host needs is these two reads, and both speak only `host_api` +
/// this module's vocabulary, so the port is declarable here and the state is
/// not. Dependency inversion: declared below, implemented above
/// (`.claude/rules/type-placement.md`, traits §2).
///
/// A caller with **no** reader wired behaves exactly as an empty registry
/// does: no descriptor, no missing requirement. That equivalence is why the
/// extension host holds an `Option<Arc<dyn ExtensionAccountSetupReader>>`
/// rather than needing a null implementation in this crate.
#[async_trait]
pub trait ExtensionAccountSetupReader: Send + Sync {
    /// The declared setup descriptor for an extension, if one was declared.
    fn descriptor(&self, extension_id: &ExtensionId) -> Option<ExtensionAccountSetupDescriptor>;

    /// The outstanding credential requirement for a user, and only when the
    /// declared account is disconnected. Undeclared extensions have no account
    /// gate; a declared extension whose host or status backend is unavailable
    /// fails closed with an error.
    async fn missing_requirement(
        &self,
        extension_id: &ExtensionId,
        user_id: &UserId,
    ) -> Result<Option<RuntimeCredentialAuthRequirement>, ExtensionAccountSetupError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_error_carries_its_sanitized_reason() {
        let error = AccountConnectionStatusError::new("backend timed out");
        assert_eq!(
            error.to_string(),
            "account connection status read failed: backend timed out"
        );
    }

    #[test]
    fn generic_notice_policy_interpolates_the_display_name_into_every_notice() {
        let policy = ChannelConnectionNoticePolicy::generic("Slack");
        for notice in [
            &policy.connect_required,
            &policy.paired,
            &policy.already_paired_same_user,
            &policy.already_bound_to_other_user,
            &policy.expired_or_unknown,
        ] {
            assert!(
                notice.contains("Slack"),
                "notice must name the channel: {notice}"
            );
        }
        // Each notice is distinct copy, not one string reused for five states.
        let all = [
            policy.connect_required.clone(),
            policy.paired.clone(),
            policy.already_paired_same_user.clone(),
            policy.already_bound_to_other_user.clone(),
            policy.expired_or_unknown.clone(),
        ];
        let mut unique = all.to_vec();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), all.len(), "notices must not collapse");
    }

    #[test]
    fn setup_error_sources_the_status_error_it_wraps() {
        let extension_id = ExtensionId::new("slack").expect("valid extension id");
        let unavailable = ExtensionAccountSetupError::HostUnavailable {
            extension_id: extension_id.clone(),
        };
        assert!(unavailable.to_string().contains("slack"));

        let status = ExtensionAccountSetupError::StatusUnavailable {
            extension_id,
            source: AccountConnectionStatusError::new("no backend"),
        };
        assert!(status.to_string().contains("slack"));
        assert_ne!(unavailable, status);
    }
}
