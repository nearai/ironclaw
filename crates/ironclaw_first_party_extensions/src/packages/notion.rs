//! Notion MCP package — Notion workspace tools over a hosted MCP server, OAuth
//! credential, host-mediated egress. Assets: per-tool input/output JSON schemas
//! and prompt docs (no bundled WASM; dispatched via MCP). The asset directory is
//! `notion-mcp/` while the in-package schema/prompt paths use `notion/`.

use std::borrow::Cow;

use ironclaw_host_api::EffectKind;

use super::{PackageBundle, PackageOnboarding, bytes_asset};

pub(super) const ID: &str = "notion";

const MANIFEST: &str = include_str!("../../assets/notion-mcp/manifest.toml");

pub(super) fn bundle() -> PackageBundle {
    PackageBundle {
        id: ID,
        display_name: "Notion MCP",
        manifest_toml: Cow::Borrowed(MANIFEST),
        assets: assets(),
        onboarding: Some(PackageOnboarding {
            instructions: "Notion needs OAuth authorization before MCP tools can run.".to_string(),
            credential_instructions: Some(
                "Authorize the Notion workspace that IronClaw should access.".to_string(),
            ),
            setup_url: None,
            credential_next_step: "After authorization completes, IronClaw finishes Notion \
                installation automatically and publishes its MCP tools."
                .to_string(),
        }),
        // MCP OAuth extension: Dispatch + Network + UseSecret + ExternalWrite.
        trust_effects: Some(vec![
            EffectKind::DispatchCapability,
            EffectKind::Network,
            EffectKind::UseSecret,
            EffectKind::ExternalWrite,
        ]),
    }
}

fn assets() -> Vec<super::PackageAsset> {
    macro_rules! notion_schema_asset {
        ($path:literal) => {
            bytes_asset(
                concat!("schemas/notion/", $path),
                include_bytes!(concat!("../../assets/notion-mcp/schemas/notion/", $path)),
            )
        };
    }
    macro_rules! notion_prompt_asset {
        ($path:literal) => {
            bytes_asset(
                concat!("prompts/notion/", $path),
                include_bytes!(concat!("../../assets/notion-mcp/prompts/notion/", $path)),
            )
        };
    }

    vec![
        bytes_asset("manifest.toml", MANIFEST.as_bytes()),
        notion_schema_asset!("notion-search.input.v1.json"),
        notion_schema_asset!("notion-search.output.v1.json"),
        notion_schema_asset!("notion-fetch.input.v1.json"),
        notion_schema_asset!("notion-fetch.output.v1.json"),
        notion_schema_asset!("notion-create-pages.input.v1.json"),
        notion_schema_asset!("notion-create-pages.output.v1.json"),
        notion_schema_asset!("notion-update-page.input.v1.json"),
        notion_schema_asset!("notion-update-page.output.v1.json"),
        notion_schema_asset!("notion-move-pages.input.v1.json"),
        notion_schema_asset!("notion-move-pages.output.v1.json"),
        notion_schema_asset!("notion-duplicate-page.input.v1.json"),
        notion_schema_asset!("notion-duplicate-page.output.v1.json"),
        notion_schema_asset!("notion-create-database.input.v1.json"),
        notion_schema_asset!("notion-create-database.output.v1.json"),
        notion_schema_asset!("notion-update-data-source.input.v1.json"),
        notion_schema_asset!("notion-update-data-source.output.v1.json"),
        notion_schema_asset!("notion-create-view.input.v1.json"),
        notion_schema_asset!("notion-create-view.output.v1.json"),
        notion_schema_asset!("notion-update-view.input.v1.json"),
        notion_schema_asset!("notion-update-view.output.v1.json"),
        notion_schema_asset!("notion-query-data-sources.input.v1.json"),
        notion_schema_asset!("notion-query-data-sources.output.v1.json"),
        notion_schema_asset!("notion-query-database-view.input.v1.json"),
        notion_schema_asset!("notion-query-database-view.output.v1.json"),
        notion_schema_asset!("notion-create-comment.input.v1.json"),
        notion_schema_asset!("notion-create-comment.output.v1.json"),
        notion_schema_asset!("notion-get-comments.input.v1.json"),
        notion_schema_asset!("notion-get-comments.output.v1.json"),
        notion_schema_asset!("notion-get-teams.input.v1.json"),
        notion_schema_asset!("notion-get-teams.output.v1.json"),
        notion_schema_asset!("notion-get-users.input.v1.json"),
        notion_schema_asset!("notion-get-users.output.v1.json"),
        notion_schema_asset!("notion-get-user.input.v1.json"),
        notion_schema_asset!("notion-get-user.output.v1.json"),
        notion_schema_asset!("notion-get-self.input.v1.json"),
        notion_schema_asset!("notion-get-self.output.v1.json"),
        notion_prompt_asset!("notion-search.md"),
        notion_prompt_asset!("notion-fetch.md"),
        notion_prompt_asset!("notion-create-pages.md"),
        notion_prompt_asset!("notion-update-page.md"),
        notion_prompt_asset!("notion-move-pages.md"),
        notion_prompt_asset!("notion-duplicate-page.md"),
        notion_prompt_asset!("notion-create-database.md"),
        notion_prompt_asset!("notion-update-data-source.md"),
        notion_prompt_asset!("notion-create-view.md"),
        notion_prompt_asset!("notion-update-view.md"),
        notion_prompt_asset!("notion-query-data-sources.md"),
        notion_prompt_asset!("notion-query-database-view.md"),
        notion_prompt_asset!("notion-create-comment.md"),
        notion_prompt_asset!("notion-get-comments.md"),
        notion_prompt_asset!("notion-get-teams.md"),
        notion_prompt_asset!("notion-get-users.md"),
        notion_prompt_asset!("notion-get-user.md"),
        notion_prompt_asset!("notion-get-self.md"),
    ]
}
