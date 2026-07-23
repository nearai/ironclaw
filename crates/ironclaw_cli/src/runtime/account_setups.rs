//! Binary-assembled account-setup declarations (extension-runtime §5.5).
//!
//! The CLI is the one generic-side crate allowed to name concrete
//! extensions: it declares each channel extension's activation gate and
//! connect-strategy presentation here and hands the list to composition as
//! build input. Composition consumes the descriptors without knowing any
//! vendor.

use ironclaw_composition::{
    ChannelConnectionNoticePolicy, ChannelConnectionRequirement, ExtensionAccountSetupDescriptor,
    ExtensionId, IronClawChannelConnectStrategy, RuntimeCredentialAccountSetup,
    RuntimeCredentialAuthRequirement, VendorId,
};

/// Every account-setup declaration the binary assembles.
pub(crate) fn bundled_account_setup_descriptors() -> Vec<ExtensionAccountSetupDescriptor> {
    vec![telegram_account_setup_descriptor()]
}

/// Telegram's user-scoped pairing gate and activation projection: the
/// `WebGeneratedCode` flow (mint a code in WebChat, consume it in Telegram
/// via the bot deep link). Provider identity is the extension id — the same
/// provider the pairing service binds under and the blocked-run fan-out
/// resumes on.
fn telegram_account_setup_descriptor() -> ExtensionAccountSetupDescriptor {
    let extension_id = ExtensionId::new("telegram").expect("static extension id"); // safety: static literal uses the validated extension id grammar.
    ExtensionAccountSetupDescriptor {
        extension_id: extension_id.clone(),
        auth_requirement: RuntimeCredentialAuthRequirement {
            provider: VendorId::new("telegram").expect("static provider id"), // safety: static literal uses the validated vendor id grammar.
            setup: RuntimeCredentialAccountSetup::Pairing,
            requester_extension: extension_id,
            provider_scopes: Vec::new(),
        },
        connection_requirement: ChannelConnectionRequirement {
            channel: "telegram".to_string(),
            display_name: "Telegram".to_string(),
            strategy: IronClawChannelConnectStrategy::WebGeneratedCode,
            instructions: "Pair your Telegram account: open the link (or scan the QR) from the \
                           pairing panel, or send the shown code to the bot in Telegram."
                .to_string(),
            input_placeholder: String::new(),
            submit_label: "Open pairing".to_string(),
            error_message: "Telegram pairing failed. Get a fresh code and try again.".to_string(),
        },
        connection_notices: ChannelConnectionNoticePolicy {
            connect_required: "👋 Pair your Telegram account in the Ironclaw web app, then message me here again.".to_string(),
            paired: "✅ Telegram is paired. You can talk to Ironclaw right here.".to_string(),
            already_paired_same_user: "✅ This Telegram account is already paired to you. You can talk to Ironclaw right here.".to_string(),
            already_bound_to_other_user: "This Telegram account is already paired to another Ironclaw user.".to_string(),
            expired_or_unknown: "That Telegram pairing code is invalid or expired. Get a fresh link or code from Ironclaw and try again.".to_string(),
        },
        activation_success_message: "Telegram is installed as an inbound entrypoint. If WebChat \
                                     shows a Telegram pairing panel, tell the user to pair via \
                                     the link, the QR code, or by sending the shown code to the \
                                     bot in Telegram — nothing is pasted into this chat. Once \
                                     paired the user can DM the bot directly. Telegram exposes \
                                     no tools and cannot read messages or send on the user's \
                                     behalf."
            .to_string(),
        pairing_deep_link_template: Some("https://t.me/{bot_username}?start={code}".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn telegram_descriptor_matches_pairing_and_fanout_contracts() {
        let descriptor = telegram_account_setup_descriptor();
        assert_eq!(descriptor.extension_id.as_str(), "telegram");
        assert_eq!(descriptor.auth_requirement.provider.as_str(), "telegram");
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
            IronClawChannelConnectStrategy::WebGeneratedCode
        );
        assert!(
            descriptor
                .connection_requirement
                .input_placeholder
                .is_empty(),
            "WebGeneratedCode displays a code; it never collects one"
        );
        let template = descriptor
            .pairing_deep_link_template
            .as_deref()
            .expect("telegram declares a deep-link template");
        assert!(template.contains("{code}"));
        assert!(template.contains("{bot_username}"));
        for text in [
            &descriptor.connection_notices.connect_required,
            &descriptor.connection_notices.paired,
            &descriptor.connection_notices.already_paired_same_user,
            &descriptor.connection_notices.already_bound_to_other_user,
            &descriptor.connection_notices.expired_or_unknown,
        ] {
            assert!(!text.trim().is_empty());
        }
        assert!(
            descriptor
                .connection_notices
                .connect_required
                .contains("Telegram")
        );
    }
}
