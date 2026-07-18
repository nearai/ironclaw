//! Telegram-owned declaration for the generic extension account-setup gate.

use ironclaw_host_api::{
    ExtensionId, RuntimeCredentialAccountProviderId, RuntimeCredentialAccountSetup,
    RuntimeCredentialAuthRequirement,
};
use ironclaw_product_workflow::{
    ChannelConnectionRequirement, ExtensionAccountSetupDescriptor, RebornChannelConnectStrategy,
};
use thiserror::Error;

use crate::telegram_actor_identity::TELEGRAM_IDENTITY_PROVIDER;

pub const TELEGRAM_EXTENSION_ID: &str = "telegram";

#[derive(Debug, Error)]
pub enum TelegramHostBuildError {
    #[error("Telegram host requires local runtime HTTP egress")]
    RuntimeHttpEgressUnavailable,
    #[error("Telegram host requires durable host state")]
    DurableHostStateUnavailable,
    #[error("Telegram host requires composed product-auth services")]
    ProductAuthUnavailable,
    #[error("Telegram host conversation store unavailable: {reason}")]
    ConversationStoreUnavailable { reason: String },
    #[error("Telegram host outbound delivery target registration failed: {reason}")]
    OutboundDeliveryTargetRegistration { reason: String },
    #[error("invalid Telegram host config field {field}: {reason}")]
    InvalidConfig { field: &'static str, reason: String },
}

/// Declares Telegram's user-scoped pairing gate and activation projection.
/// Generic lifecycle code consumes this descriptor without knowing Telegram's
/// identifier, setup kind, or product copy.
pub fn telegram_account_setup_descriptor()
-> Result<ExtensionAccountSetupDescriptor, TelegramHostBuildError> {
    let extension_id = ExtensionId::new(TELEGRAM_EXTENSION_ID)
        .map_err(|error| invalid_account_setup("extension_id", error.to_string()))?;
    let provider = RuntimeCredentialAccountProviderId::new(TELEGRAM_IDENTITY_PROVIDER)
        .map_err(|error| invalid_account_setup("provider", error.to_string()))?;

    Ok(ExtensionAccountSetupDescriptor {
        extension_id: extension_id.clone(),
        auth_requirement: RuntimeCredentialAuthRequirement {
            provider,
            setup: RuntimeCredentialAccountSetup::Pairing,
            requester_extension: extension_id,
            provider_scopes: Vec::new(),
        },
        connection_requirement: ChannelConnectionRequirement {
            channel: TELEGRAM_EXTENSION_ID.to_string(),
            strategy: RebornChannelConnectStrategy::WebGeneratedCode,
            instructions: "Pair your Telegram account: tap the link or scan the QR in the pairing panel, or send the shown code to the bot in Telegram.".to_string(),
            input_placeholder: String::new(),
            submit_label: "Open pairing".to_string(),
            error_message: "Telegram pairing failed. Get a fresh code and try again.".to_string(),
        },
        activation_success_message: "Telegram is installed as an inbound entrypoint. If WebChat shows a Telegram pairing panel, tell the user to pair via the link, the QR code, or by sending the shown code to the bot in Telegram — nothing is pasted into this chat. Once paired the user can DM the bot directly. Telegram exposes no tools and cannot read messages or send on the user's behalf.".to_string(),
    })
}

fn invalid_account_setup(field: &'static str, reason: String) -> TelegramHostBuildError {
    TelegramHostBuildError::InvalidConfig { field, reason }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_setup_descriptor_matches_pairing_and_fanout_contracts() {
        let descriptor = telegram_account_setup_descriptor().expect("descriptor builds");

        assert_eq!(descriptor.extension_id.as_str(), TELEGRAM_EXTENSION_ID);
        assert_eq!(
            descriptor.auth_requirement.provider.as_str(),
            TELEGRAM_IDENTITY_PROVIDER
        );
        assert_eq!(
            descriptor.auth_requirement.setup,
            RuntimeCredentialAccountSetup::Pairing
        );
        assert_eq!(
            descriptor.auth_requirement.requester_extension,
            descriptor.extension_id
        );
        assert_eq!(
            descriptor.connection_requirement.strategy,
            RebornChannelConnectStrategy::WebGeneratedCode
        );
        assert!(
            descriptor
                .connection_requirement
                .input_placeholder
                .is_empty(),
            "WebGeneratedCode displays a code; it never collects one"
        );
    }
}
