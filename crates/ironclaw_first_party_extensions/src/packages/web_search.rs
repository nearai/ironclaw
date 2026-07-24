//! Web Search package — Brave-backed web search over a WASM executor.
//! Assets: input/output JSON schemas, the tool prompt doc, and the tool WASM
//! module. Requires a `brave_api_key` secret (see the manifest's
//! `runtime_credentials`); mutually exclusive with `web-access` at bootstrap
//! time (composition activates whichever one has a usable credential).

use std::borrow::Cow;

use super::{PackageBundle, bytes_asset};

pub(super) const ID: &str = "web_search";

const MANIFEST: &str = include_str!("../../assets/web-search/manifest.toml");
const WASM: &[u8] = include_bytes!("../../assets/web-search/wasm/web_search_tool.wasm");

pub(super) fn bundle() -> PackageBundle {
    PackageBundle {
        id: ID,
        display_name: "Web Search",
        manifest_toml: Cow::Borrowed(MANIFEST),
        assets: vec![
            bytes_asset("manifest.toml", MANIFEST.as_bytes()),
            bytes_asset(
                "schemas/web_search/search.input.v1.json",
                include_bytes!("../../assets/web-search/schemas/web_search/search.input.v1.json"),
            ),
            bytes_asset(
                "schemas/web_search/search.output.v1.json",
                include_bytes!("../../assets/web-search/schemas/web_search/search.output.v1.json"),
            ),
            bytes_asset(
                "prompts/web_search/search.md",
                include_bytes!("../../assets/web-search/prompts/web_search/search.md"),
            ),
            bytes_asset("wasm/web_search_tool.wasm", WASM),
        ],
        // No bespoke onboarding copy: the manifest's `runtime_credentials`
        // entry (a bare `SecretHandle`, no `source`) already carries enough
        // for the generic credential-setup path, matching `github`'s pattern
        // for tools with a single simple secret.
        onboarding: None,
        // WASM tool package: trust comes from the extension registry, not an
        // admin local-manifest effect grant.
        trust_effects: None,
    }
}
