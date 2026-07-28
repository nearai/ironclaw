//! Binary-assembled native extension factories (extension-runtime DEL-7's
//! target architecture, first exercised by Telegram for DEL-10): the CLI is
//! the one generic-side crate allowed to link concrete extension crates, and
//! it hands the factory registry to composition as input — composition never
//! names a concrete extension.

use std::sync::Arc;

use ironclaw_extension_host::{
    BindContext, BindError, ExtensionBindings, ExtensionEntrypoint, LoadContext,
    NativeExtensionFactory,
};
use ironclaw_reborn_composition::ChannelExtensionBinding;
use ironclaw_telegram_extension::{TelegramChannelAdapter, TelegramPreferenceTargetCodec};

/// Every native factory the binary assembles (`first_party`-runtime
/// extensions bind their adapters through these).
pub(crate) fn bundled_native_extension_factories() -> Vec<Arc<dyn NativeExtensionFactory>> {
    vec![Arc::new(TelegramExtensionFactory)]
}

/// Deployment channel-adapter bindings. These are independent of native tool
/// loading: the host mounts manifest-declared ingress before any user
/// installation exists, so every deployment channel adapter is linked here.
/// Composition never names a concrete extension crate.
pub(crate) fn bundled_channel_extension_bindings() -> Vec<ChannelExtensionBinding> {
    vec![
        ChannelExtensionBinding {
            extension_id: "slack".to_string(),
            adapter: Arc::new(ironclaw_slack_extension::SlackChannelAdapter),
            preference_target_codec: Some(Arc::new(
                ironclaw_slack_extension::SlackPreferenceTargetCodec,
            )),
        },
        ChannelExtensionBinding {
            extension_id: "telegram".to_string(),
            adapter: Arc::new(TelegramChannelAdapter::default()),
            preference_target_codec: Some(Arc::new(TelegramPreferenceTargetCodec)),
        },
    ]
}

/// `runtime.service = "telegram.extension/v1"` — the Telegram channel
/// extension: channel adapter only, no tools.
struct TelegramExtensionFactory;

impl NativeExtensionFactory for TelegramExtensionFactory {
    fn service(&self) -> &str {
        "telegram.extension/v1"
    }

    fn load(&self, _ctx: &LoadContext) -> Result<Box<dyn ExtensionEntrypoint>, BindError> {
        Ok(Box::new(TelegramExtensionEntrypoint))
    }
}

struct TelegramExtensionEntrypoint;

impl ExtensionEntrypoint for TelegramExtensionEntrypoint {
    fn bind(&self, _ctx: BindContext) -> Result<ExtensionBindings, BindError> {
        Ok(ExtensionBindings {
            tools: None,
            channel: Some(Arc::new(TelegramChannelAdapter::default())),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn telegram_factory_binds_a_channel_and_no_tools() {
        let factories = bundled_native_extension_factories();
        assert!(
            factories
                .iter()
                .any(|factory| factory.service() == "telegram.extension/v1"),
            "the binary assembles the telegram factory"
        );
    }

    #[test]
    fn bundled_channel_bindings_carry_their_production_extras() {
        let bindings = bundled_channel_extension_bindings();
        let slack = bindings
            .iter()
            .find(|binding| binding.extension_id == "slack")
            .expect("the binary supplies the slack channel binding");
        assert!(slack.preference_target_codec.is_some());
        let telegram = bindings
            .iter()
            .find(|binding| binding.extension_id == "telegram")
            .expect("the binary supplies the telegram deployment channel binding");
        assert!(
            telegram.preference_target_codec.is_some(),
            "the shipping Telegram binding must expose outbound preference targets"
        );
    }
}
