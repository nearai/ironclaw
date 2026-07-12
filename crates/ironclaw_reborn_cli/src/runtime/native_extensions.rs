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
use ironclaw_telegram_v2_adapter::TelegramChannelAdapter;

/// Every native factory the binary assembles. (Slack's fold from the
/// composition-bundled binding into this registry is extension-runtime P6.)
pub(crate) fn bundled_native_extension_factories() -> Vec<Arc<dyn NativeExtensionFactory>> {
    vec![Arc::new(TelegramExtensionFactory)]
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
}
