//! Neutral authenticated-session channel fixture for composition tests.
//!
//! The shipping binary links the concrete Web App package. Generic
//! composition tests instead install this manifest-backed fixture so they
//! exercise session-channel discovery without teaching composition about a
//! concrete extension crate or relying on a built-in fallback.

/// Extension identity declared by [`with_test_authenticated_session_channel`].
pub const TEST_SESSION_EXTENSION_ID: &str = "test-session-channel";
const TEST_SESSION_EXTENSION_MANIFEST: &str = r#"
schema_version = "reborn.extension_manifest.v3"
id = "test-session-channel"
name = "Test session channel"
version = "0.1.0"
description = "composition fixture: authenticated-session channel"
trust = "first_party_requested"

[runtime]
kind = "first_party"
service = "test-session-channel.extension/v1"

[channel]
id = "chat"
display_name = "Test session channel"
conversation_model = "isolated"

[channel.reply]
transport = "stream"

[channel.delivery]
transport = "push"

[channel.ingress]
method = "post"

[channel.ingress.verification]
kind = "authenticated_session"
"#;

/// Add one manifest-declared authenticated-session channel through the same
/// neutral bundle and binding seams the binary uses.
pub fn with_test_authenticated_session_channel(
    bindings: crate::input::RebornHostBindings,
) -> crate::input::RebornHostBindings {
    let mut bundles = ironclaw_extension_host::test_support::first_party_bundles_from_inventory();
    bundles.push(ironclaw_extension_host::FirstPartyPackageBundle {
        id: TEST_SESSION_EXTENSION_ID.to_string(),
        display_name: "Test session channel".to_string(),
        manifest_toml: TEST_SESSION_EXTENSION_MANIFEST.to_string(),
        assets: vec![ironclaw_extension_host::FirstPartyPackageAsset {
            path: "manifest.toml".to_string(),
            bytes: TEST_SESSION_EXTENSION_MANIFEST.as_bytes().to_vec(),
        }],
        onboarding: None,
        oauth_setup: None,
        trust_effects: None,
        search_aliases: Vec::new(),
    });
    bindings
        .with_first_party_bundles(bundles)
        .with_first_party_registrars(
            ironclaw_extension_host::test_support::first_party_registrars::bundled_first_party_registrars(),
        )
        .with_credential_account_visibility_policy(
            ironclaw_extension_host::test_support::first_party_registrars::bundled_credential_account_visibility_policy(),
        )
        .with_channel_extension_bindings(vec![crate::input::ChannelExtensionBinding {
            extension_id: ironclaw_host_api::ids::ExtensionId::from_trusted(
                TEST_SESSION_EXTENSION_ID.to_string(),
            ),
            surfaces: ironclaw_extension_host::test_support::FakeChannelAdapter::delivery_only(),
            preference_target_codec: None,
            outbound_target_provider: None,
            first_party_initializer: None,
            registration_document_path: None,
        }])
}
