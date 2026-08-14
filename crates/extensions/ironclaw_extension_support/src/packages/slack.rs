//! Slack (user-scoped tools) package — search/conversation/message tools over a
//! WASM executor, personal Slack OAuth credential, host-mediated egress. The
//! WASM binary keeps its legacy `slack_user_tool.wasm` filename and the
//! `slack_user_token` credential handle, so the assets are spelled out rather
//! than derived from the id. The connect flow is a personal-OAuth *setup*
//! requirement whose scopes are the manifest-declared union of the tools'
//! per-capability scopes.

use std::borrow::Cow;

use ironclaw_host_api::capability::EffectKind;

use super::{PackageBundle, PackageOnboarding, bytes_asset};

pub(super) const ID: &str = "slack";

pub(super) const MANIFEST: &str = include_str!("../../../packages/slack/manifest.toml");
const WASM: &[u8] = include_bytes!("../../../packages/slack/wasm/slack_user_tool.wasm");

pub(super) fn bundle() -> PackageBundle {
    PackageBundle {
        id: ID,
        display_name: "Slack",
        manifest_toml: Cow::Borrowed(MANIFEST),
        assets: assets(),
        onboarding: Some(PackageOnboarding {
            instructions: "Slack needs OAuth authorization before the Slack channel can recognize \
                your DMs and before the user-scoped Slack tools can run."
                .to_string(),
            credential_instructions: Some(
                "Authorize the Slack account you will use to DM IronClaw.".to_string(),
            ),
            setup_url: None,
            credential_next_step:
                "After authorization completes, DM the Slack bot directly or use \
                the Slack tools from any chat."
                    .to_string(),
        }),
        // User-scoped Slack tools: Dispatch + Network + UseSecret + ExternalWrite.
        trust_effects: Some(vec![
            EffectKind::DispatchCapability,
            EffectKind::Network,
            EffectKind::UseSecret,
            EffectKind::ExternalWrite,
        ]),
    }
}

fn assets() -> Vec<super::PackageAsset> {
    macro_rules! slack_prompt_asset {
        ($operation:literal) => {
            bytes_asset(
                concat!("prompts/slack/", $operation, ".md"),
                include_bytes!(concat!(
                    "../../../packages/slack/prompts/slack/",
                    $operation,
                    ".md"
                )),
            )
        };
    }

    // All 16 [[tools]] entries are standard_op-bound (standardized messaging
    // framework) — Slack binds every core operation: the host resolves their
    // input/output schemas from the compiled-in
    // `ironclaw_host_api::messaging` registry via the synthesized
    // `standard:messaging/<op>.v1` refs, never from a package asset — so no
    // `schemas/slack/*.json` embed exists or is needed. Each entry keeps a
    // package-owned prompt doc (`prompt_doc_ref`), still read from the
    // materialized package root at surface publish. Pinned catalog-wide by
    // `bundled_first_party_manifest_asset_refs_are_packaged` in
    // `ironclaw_extension_host::available_extensions` (that function's
    // `validate_bundled_package_assets` is the production-critical sibling
    // of the same check) — a manifest entry whose addendum is missing from
    // this list fails there, at install time.
    vec![
        bytes_asset("manifest.toml", MANIFEST.as_bytes()),
        // Reads.
        slack_prompt_asset!("search_messages"),
        slack_prompt_asset!("list_conversations"),
        slack_prompt_asset!("get_conversation_info"),
        slack_prompt_asset!("get_conversation_history"),
        slack_prompt_asset!("get_thread_replies"),
        slack_prompt_asset!("get_message"),
        // People.
        slack_prompt_asset!("get_user_info"),
        slack_prompt_asset!("resolve_user"),
        slack_prompt_asset!("list_members"),
        slack_prompt_asset!("whoami"),
        // Writes.
        slack_prompt_asset!("send_message"),
        slack_prompt_asset!("edit_message"),
        slack_prompt_asset!("delete_message"),
        slack_prompt_asset!("add_reaction"),
        slack_prompt_asset!("remove_reaction"),
        slack_prompt_asset!("open_dm"),
        bytes_asset("wasm/slack_user_tool.wasm", WASM),
    ]
}
