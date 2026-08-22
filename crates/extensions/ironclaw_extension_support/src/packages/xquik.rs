//! Xquik package: X data and account tools over a hosted MCP server, OAuth
//! credential, and host-mediated egress. The live server owns tool discovery.

use std::borrow::Cow;

use ironclaw_host_api::capability::EffectKind;

use super::{PackageBundle, PackageOnboarding, bytes_asset};

pub(super) const ID: &str = "xquik";

const MANIFEST: &str = include_str!("../../../packages/xquik-mcp/manifest.toml");

pub(super) fn bundle() -> PackageBundle {
    PackageBundle {
        id: ID,
        display_name: "Xquik MCP",
        manifest_toml: Cow::Borrowed(MANIFEST),
        assets: vec![bytes_asset("manifest.toml", MANIFEST.as_bytes())],
        onboarding: Some(PackageOnboarding {
            instructions: "Connect Xquik to use X data and account tools without browser cookies."
                .to_string(),
            credential_instructions: Some(
                "Sign in to Xquik and approve MCP access for IronClaw.".to_string(),
            ),
            setup_url: None,
            credential_next_step:
                "After authorization, IronClaw discovers and publishes the Xquik MCP tools."
                    .to_string(),
        }),
        trust_effects: Some(vec![
            EffectKind::DispatchCapability,
            EffectKind::Network,
            EffectKind::UseSecret,
            EffectKind::ExternalWrite,
        ]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundle_carries_the_oauth_manifest_as_its_only_asset() {
        let bundle = bundle();

        assert_eq!(bundle.id, ID);
        assert_eq!(bundle.manifest_toml.as_ref(), MANIFEST);
        assert_eq!(bundle.assets.len(), 1);
        assert_eq!(bundle.assets[0].path, "manifest.toml");
        assert!(MANIFEST.contains("server = \"https://xquik.com/mcp\""));
        assert!(MANIFEST.contains("scopes = [\"mcp:tools\"]"));
        assert!(MANIFEST.contains("rotates_refresh_token = true"));
    }
}
