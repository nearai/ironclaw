//! NEAR AI MCP package — hosted MCP tools over the NEAR AI cloud endpoint,
//! API-key credential, host-mediated egress. Assets: one tool's input/output
//! JSON schemas and its prompt doc (no bundled WASM; dispatched via MCP). The
//! asset directory is `nearai-mcp/` while the in-package schema/prompt paths use
//! `nearai/` — the same asymmetry `notion.rs` carries.
//!
//! ## Why this package is not in [`super::PACKAGES`]
//!
//! Every other inventory entry is a `fn() -> PackageBundle`: the bundle is a
//! pure function of the embedded bytes, so the binary can hand the whole
//! inventory to composition without knowing what is in it. NEAR AI is the one
//! package whose shipped manifest is **incomplete on its own** — its
//! `[mcp].server` is a placeholder that the host rewrites from the operator's
//! LLM-admin bootstrap configuration before the manifest is ever parsed. A
//! config-free builder cannot produce that value, and putting the unpatched
//! manifest in the inventory would mean the binary hands composition a manifest
//! pointing at the placeholder endpoint.
//!
//! So the *embeds* live here, with every other package's embeds and under the
//! specificity gate's sanctioned-crate exemption, and the *patch* stays with the
//! endpoint authority in `ironclaw_extension_host`. [`nearai_bundle`] is the
//! seam: it returns the same [`PackageBundle`] shape as the inventory entries,
//! carrying the manifest as shipped, and the caller replaces `manifest_toml`
//! (which is a [`Cow`] precisely so an owned, patched manifest is representable)
//! and the `manifest.toml` asset with the patched text.

use std::borrow::Cow;

use super::{PackageAsset, PackageBundle, bytes_asset};

/// Extension id declared by the package manifest. The asset directory is
/// `nearai-mcp/`; the id is `nearai`.
pub const NEARAI_ID: &str = "nearai";

/// In-package path of the manifest asset, which the caller re-writes with the
/// endpoint-patched text.
pub const NEARAI_MANIFEST_ASSET_PATH: &str = "manifest.toml";

const MANIFEST: &str = include_str!("../../../packages/nearai-mcp/manifest.toml");

/// The NEAR AI package as shipped, with the manifest's placeholder `[mcp].server`
/// still in place.
///
/// Callers that dispatch NEAR AI tools **must** replace `manifest_toml` and the
/// [`NEARAI_MANIFEST_ASSET_PATH`] asset with an endpoint-patched manifest; see
/// the module docs for why that patch cannot happen here.
pub fn nearai_bundle() -> PackageBundle {
    PackageBundle {
        id: NEARAI_ID,
        display_name: "NEAR AI",
        manifest_toml: Cow::Borrowed(MANIFEST),
        assets: assets(),
        onboarding: None,
        trust_effects: None,
    }
}

fn assets() -> Vec<PackageAsset> {
    macro_rules! nearai_schema_asset {
        ($path:literal) => {
            bytes_asset(
                concat!("schemas/nearai/", $path),
                include_bytes!(concat!(
                    "../../../packages/nearai-mcp/schemas/nearai/",
                    $path
                )),
            )
        };
    }
    macro_rules! nearai_prompt_asset {
        ($path:literal) => {
            bytes_asset(
                concat!("prompts/nearai/", $path),
                include_bytes!(concat!(
                    "../../../packages/nearai-mcp/prompts/nearai/",
                    $path
                )),
            )
        };
    }

    vec![
        bytes_asset(NEARAI_MANIFEST_ASSET_PATH, MANIFEST.as_bytes()),
        nearai_schema_asset!("web_search.input.v1.json"),
        nearai_schema_asset!("web_search.output.v1.json"),
        nearai_prompt_asset!("web_search.md"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shipped_bundle_carries_the_manifest_and_every_asset() {
        let bundle = nearai_bundle();
        assert_eq!(bundle.id, NEARAI_ID);
        assert_eq!(bundle.manifest_toml.as_ref(), MANIFEST);

        let paths = bundle
            .assets
            .iter()
            .map(|asset| asset.path.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            paths,
            vec![
                "manifest.toml",
                "schemas/nearai/web_search.input.v1.json",
                "schemas/nearai/web_search.output.v1.json",
                "prompts/nearai/web_search.md",
            ]
        );
    }

    #[test]
    fn shipped_manifest_declares_the_placeholder_server_the_host_patches() {
        // The reason this package is not a `PACKAGES` entry: the shipped
        // `[mcp].server` is not the endpoint any deployment talks to — the host
        // rewrites it. If this assertion fails because the key was renamed or
        // dropped, the module docs above and the patch in
        // `ironclaw_extension_host::available_extensions` are both wrong.
        assert!(
            MANIFEST.contains("[mcp]"),
            "shipped manifest must declare an [mcp] table for the host to patch"
        );
        assert!(
            MANIFEST.contains("server = "),
            "shipped manifest must declare [mcp].server for the host to overwrite"
        );
    }
}
