use ironclaw_extensions::ExtensionPackage;
use ironclaw_host_api::RuntimeCredentialAccountSetup;
use ironclaw_product::adapter_registry::PRODUCT_ADAPTER_HOST_API_ID;
use ironclaw_product::{
    ChannelConnectionRequirement, ExtensionAccountSetupDescriptor, RebornChannelConnectStrategy,
};

use crate::package_runtime_credential_auth_requirements;

/// The connect strategy for a channel surface, derived from the manifest's
/// declared auth setup.
pub fn channel_connect_strategy(package: &ExtensionPackage) -> RebornChannelConnectStrategy {
    let uses_oauth = package_runtime_credential_auth_requirements(package)
        .iter()
        .any(|requirement| {
            matches!(
                requirement.setup,
                RuntimeCredentialAccountSetup::OAuth { .. }
            )
        });
    if uses_oauth {
        RebornChannelConnectStrategy::OAuth
    } else {
        RebornChannelConnectStrategy::InboundProofCode
    }
}

/// The structured connect affordance for a channel surface.
pub fn channel_connection_requirement(
    channel_id: &str,
    display_name: &str,
    strategy: RebornChannelConnectStrategy,
    account_setup: Option<&ExtensionAccountSetupDescriptor>,
) -> ChannelConnectionRequirement {
    if let Some(setup) = account_setup {
        return setup.connection_requirement.clone();
    }
    let (instructions, input_placeholder, submit_label, error_message) = match strategy {
        RebornChannelConnectStrategy::OAuth => (
            format!(
                "Connect {display_name} with OAuth from the extension configuration, then \
                 message {display_name} directly."
            ),
            String::new(),
            format!("Connect {display_name}"),
            format!(
                "{display_name} OAuth connection failed. Try configuring {display_name} again."
            ),
        ),
        RebornChannelConnectStrategy::InboundProofCode
        | RebornChannelConnectStrategy::WebGeneratedCode
        | RebornChannelConnectStrategy::QrCode
        | RebornChannelConnectStrategy::AdminManagedChannels => (
            format!("Open {display_name}'s app or bot, get the pairing code, and paste it here."),
            "Enter pairing code".to_string(),
            "Connect".to_string(),
            "Pairing failed. Check the code and try again.".to_string(),
        ),
    };
    ChannelConnectionRequirement {
        channel: channel_id.to_string(),
        display_name: display_name.to_string(),
        strategy,
        instructions,
        input_placeholder,
        submit_label,
        error_message,
    }
}

pub fn package_declares_inbound_product_adapter(package: &ExtensionPackage) -> bool {
    package.manifest.host_apis.iter().any(|host_api| {
        host_api.id.as_str() == PRODUCT_ADAPTER_HOST_API_ID
            && host_api.section.as_str() == "product_adapter.inbound"
    })
}
