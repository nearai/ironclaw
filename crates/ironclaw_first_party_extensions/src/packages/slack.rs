//! Slack (user-scoped tools) package — search/conversation/message tools over a
//! WASM executor, personal Slack OAuth credential, host-mediated egress. The
//! WASM binary keeps its legacy `slack_user_tool.wasm` filename and the
//! `slack_user_token` credential handle, so the assets are spelled out rather
//! than derived from the id. The connect flow is a personal-OAuth *setup*
//! requirement whose scopes are the manifest-declared union of the tools'
//! per-capability scopes.

use std::borrow::Cow;

use ironclaw_host_api::EffectKind;

use super::{PackageBundle, PackageOnboarding, bytes_asset};

pub(super) const ID: &str = "slack";

pub(super) const MANIFEST: &str = include_str!("../../assets/slack/manifest.toml");
const WASM: &[u8] = include_bytes!("../../assets/slack/wasm/slack_user_tool.wasm");

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
    macro_rules! slack_schema_asset {
        ($path:literal) => {
            bytes_asset(
                concat!("schemas/slack/", $path),
                include_bytes!(concat!("../../assets/slack/schemas/slack/", $path)),
            )
        };
    }
    macro_rules! slack_prompt_asset {
        ($operation:literal) => {
            bytes_asset(
                concat!("prompts/slack/", $operation, ".md"),
                include_bytes!(concat!(
                    "../../assets/slack/prompts/slack/",
                    $operation,
                    ".md"
                )),
            )
        };
    }

    // One schema + prompt pair PER manifest [[tools]] entry — the host
    // runtime's hot capability catalog reads every model-visible tool's
    // `input_schema_ref`/`prompt_doc_ref` from the materialized package root
    // at surface publish, so an omitted pair does not fail install or
    // activation but kills every post-activation turn
    // (`host_stage_unavailable_capability`). Pinned catalog-wide by
    // `bundled_first_party_manifest_asset_refs_are_packaged` in
    // `ironclaw_reborn_composition::extension_host::available_extensions`.
    vec![
        bytes_asset("manifest.toml", MANIFEST.as_bytes()),
        slack_schema_asset!("raw_output.v1.json"),
        slack_schema_asset!("search_messages.input.v1.json"),
        slack_prompt_asset!("search_messages"),
        slack_schema_asset!("list_conversations.input.v1.json"),
        slack_prompt_asset!("list_conversations"),
        slack_schema_asset!("get_conversation_info.input.v1.json"),
        slack_prompt_asset!("get_conversation_info"),
        slack_schema_asset!("get_conversation_history.input.v1.json"),
        slack_prompt_asset!("get_conversation_history"),
        slack_schema_asset!("get_thread_replies.input.v1.json"),
        slack_prompt_asset!("get_thread_replies"),
        slack_schema_asset!("get_user_info.input.v1.json"),
        slack_prompt_asset!("get_user_info"),
        slack_schema_asset!("whoami.input.v1.json"),
        slack_prompt_asset!("whoami"),
        slack_schema_asset!("send_message.input.v1.json"),
        slack_prompt_asset!("send_message"),
        bytes_asset("wasm/slack_user_tool.wasm", WASM),
    ]
}
