//! Web Access package — web search and page-content retrieval tools, no
//! credentials. Assets: per-tool input/output JSON schemas and prompt docs (no
//! bundled WASM; dispatched via the web-access first-party executor).

use std::borrow::Cow;

use ironclaw_host_api::EffectKind;

use super::{PackageBundle, PackageOnboarding, bytes_asset};

pub(super) const ID: &str = "web-access";

const MANIFEST: &str = include_str!("../../assets/web-access/manifest.toml");

pub(super) fn bundle() -> PackageBundle {
    PackageBundle {
        id: ID,
        display_name: "Web Access",
        manifest_toml: Cow::Borrowed(MANIFEST),
        assets: vec![
            bytes_asset("manifest.toml", MANIFEST.as_bytes()),
            bytes_asset(
                "schemas/web-access/search.input.v1.json",
                include_bytes!("../../assets/web-access/schemas/web-access/search.input.v1.json"),
            ),
            bytes_asset(
                "schemas/web-access/search.output.v1.json",
                include_bytes!("../../assets/web-access/schemas/web-access/search.output.v1.json"),
            ),
            bytes_asset(
                "schemas/web-access/get_content.input.v1.json",
                include_bytes!(
                    "../../assets/web-access/schemas/web-access/get_content.input.v1.json"
                ),
            ),
            bytes_asset(
                "schemas/web-access/get_content.output.v1.json",
                include_bytes!(
                    "../../assets/web-access/schemas/web-access/get_content.output.v1.json"
                ),
            ),
            bytes_asset(
                "prompts/web-access/search.md",
                include_bytes!("../../assets/web-access/prompts/web-access/search.md"),
            ),
            bytes_asset(
                "prompts/web-access/get_content.md",
                include_bytes!("../../assets/web-access/prompts/web-access/get_content.md"),
            ),
        ],
        onboarding: Some(PackageOnboarding {
            instructions: "Web Access does not need credentials and becomes active as soon as it \
                is installed."
                .to_string(),
            credential_instructions: Some(
                "No credentials are required for Web Access.".to_string(),
            ),
            setup_url: None,
            credential_next_step:
                "IronClaw publishes Web Access tools automatically during installation.".to_string(),
        }),
        // No credentials/egress writes: Dispatch + Network only.
        trust_effects: Some(vec![EffectKind::DispatchCapability, EffectKind::Network]),
    }
}
