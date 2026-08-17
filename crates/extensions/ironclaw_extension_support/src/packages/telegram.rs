//! Telegram package — two surfaces on one extension: the deployment's Bot API
//! channel, and the user's own account linked as a device (`[[tools]]` +
//! `[auth.telegram]`). The tool bindings are all `standard_op`, so their
//! input/output schemas come from the compiled-in
//! `ironclaw_host_api::messaging` registry and no `schemas/telegram/*.json`
//! asset exists; each binding does keep a package-owned prompt doc, read from
//! the materialized package root at surface publish and therefore embedded
//! here (`bundled_first_party_manifest_asset_refs_are_packaged`).

use std::borrow::Cow;

use super::{PackageBundle, bytes_asset};

pub(super) const ID: &str = "telegram";

const MANIFEST: &str = include_str!("../../../packages/telegram/manifest.toml");

pub(super) fn bundle() -> PackageBundle {
    PackageBundle {
        id: ID,
        display_name: "Telegram",
        manifest_toml: Cow::Borrowed(MANIFEST),
        assets: assets(),
        // Telegram is bot-token setup handled by the channel host, not an
        // extension-card onboarding flow: no bespoke copy.
        onboarding: None,
        // Trust comes from the extension registry, not an admin local-manifest
        // effect grant.
        trust_effects: None,
    }
}

fn assets() -> Vec<super::PackageAsset> {
    macro_rules! telegram_prompt_asset {
        ($operation:literal) => {
            bytes_asset(
                concat!("prompts/telegram/", $operation, ".md"),
                include_bytes!(concat!(
                    "../../../packages/telegram/prompts/telegram/",
                    $operation,
                    ".md"
                )),
            )
        };
    }

    vec![
        bytes_asset("manifest.toml", MANIFEST.as_bytes()),
        telegram_prompt_asset!("send_message"),
        telegram_prompt_asset!("edit_message"),
        telegram_prompt_asset!("delete_message"),
        telegram_prompt_asset!("add_reaction"),
        telegram_prompt_asset!("remove_reaction"),
        telegram_prompt_asset!("open_dm"),
        telegram_prompt_asset!("list_conversations"),
        telegram_prompt_asset!("get_conversation_info"),
        telegram_prompt_asset!("get_conversation_history"),
        telegram_prompt_asset!("get_message"),
        telegram_prompt_asset!("search_messages"),
        telegram_prompt_asset!("whoami"),
        telegram_prompt_asset!("get_user_info"),
        telegram_prompt_asset!("resolve_user"),
        telegram_prompt_asset!("list_members"),
    ]
}
