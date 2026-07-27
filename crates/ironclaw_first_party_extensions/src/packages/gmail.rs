//! Gmail extension package — message/draft tools over the gsuite executor,
//! Google OAuth credential, host-mediated egress. Assets: per-tool input/output
//! JSON schemas and prompt docs (no bundled WASM; dispatched by the gsuite
//! first-party executor).

use std::borrow::Cow;

use ironclaw_host_api::EffectKind;

use super::{PackageBundle, PackageOnboarding, bytes_asset};

pub(super) const ID: &str = "gmail";

const MANIFEST: &str = include_str!("../../assets/gmail/manifest.toml");

pub(super) fn bundle() -> PackageBundle {
    PackageBundle {
        id: ID,
        display_name: "Gmail",
        manifest_toml: Cow::Borrowed(MANIFEST),
        assets: assets(),
        onboarding: Some(PackageOnboarding {
            instructions: "Gmail needs Google OAuth authorization before mail tools can run."
                .to_string(),
            credential_instructions: Some(
                "Authorize the Google account that IronClaw should use for Gmail.".to_string(),
            ),
            setup_url: None,
            credential_next_step: "After authorization completes, IronClaw finishes Gmail \
                installation automatically and publishes its tools."
                .to_string(),
        }),
        // gsuite-family effect grant (Dispatch + Network + UseSecret +
        // ExternalWrite): OAuth-backed Google API access with mail writes.
        trust_effects: Some(vec![
            EffectKind::DispatchCapability,
            EffectKind::Network,
            EffectKind::UseSecret,
            EffectKind::ExternalWrite,
        ]),
    }
}

fn assets() -> Vec<super::PackageAsset> {
    macro_rules! gmail_schema_asset {
        ($path:literal) => {
            bytes_asset(
                concat!("schemas/gmail/", $path),
                include_bytes!(concat!("../../assets/gmail/schemas/gmail/", $path)),
            )
        };
    }
    macro_rules! gmail_prompt_asset {
        ($path:literal) => {
            bytes_asset(
                concat!("prompts/gmail/", $path),
                include_bytes!(concat!("../../assets/gmail/prompts/gmail/", $path)),
            )
        };
    }

    vec![
        bytes_asset("manifest.toml", MANIFEST.as_bytes()),
        gmail_schema_asset!("list_messages.input.v1.json"),
        gmail_schema_asset!("list_messages.output.v1.json"),
        gmail_schema_asset!("get_message.input.v1.json"),
        gmail_schema_asset!("get_message.output.v1.json"),
        gmail_schema_asset!("send_message.input.v1.json"),
        gmail_schema_asset!("send_message.output.v1.json"),
        gmail_schema_asset!("create_draft.input.v1.json"),
        gmail_schema_asset!("create_draft.output.v1.json"),
        gmail_schema_asset!("reply_to_message.input.v1.json"),
        gmail_schema_asset!("reply_to_message.output.v1.json"),
        gmail_schema_asset!("trash_message.input.v1.json"),
        gmail_schema_asset!("trash_message.output.v1.json"),
        gmail_prompt_asset!("list_messages.md"),
        gmail_prompt_asset!("get_message.md"),
        gmail_prompt_asset!("send_message.md"),
        gmail_prompt_asset!("create_draft.md"),
        gmail_prompt_asset!("reply_to_message.md"),
        gmail_prompt_asset!("trash_message.md"),
    ]
}
