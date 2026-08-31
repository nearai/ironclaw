//! The documented Slack app manifest (`docs/channels/slack.mdx`), the
//! extension manifest's egress allowlist (`manifest.toml`), and the Slack
//! calls the code makes move in lockstep.
//!
//! - The app manifest must declare the native Agent surface the reply sink
//!   drives (`features.agent_view`, the Messages tab, `assistant:write` +
//!   `chat:write`, and the Agent event family the ingress handles).
//! - Every Slack Web API path the package calls — reply sink and delivery
//!   half alike — must be declared bot-token egress; the host refuses an
//!   undeclared path before the network, so a missing entry is an outage.
//! - The internal setup guide mirrors the essentials.

use std::path::PathBuf;

use ironclaw_host_api::action::NetworkMethod;
use ironclaw_slack_extension::SlackWebApiMethod;

fn repo_path(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../..")
        .join(relative)
}

fn read(relative: &str) -> String {
    let path = repo_path(relative);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("{} must be readable: {error}", path.display()))
}

/// The JSON block under `## App Manifest` in the public Slack page.
fn documented_app_manifest() -> serde_json::Value {
    let page = read("docs/channels/slack.mdx");
    let section = page
        .split("## App Manifest")
        .nth(1)
        .expect("docs/channels/slack.mdx has an `## App Manifest` section");
    let fenced = section
        .split("```json")
        .nth(1)
        .expect("the App Manifest section carries a ```json block");
    let json = fenced
        .split("```")
        .next()
        .expect("the ```json block closes");
    serde_json::from_str(json).expect("the documented app manifest is valid JSON")
}

fn string_array<'a>(value: &'a serde_json::Value, pointer: &str) -> Vec<&'a str> {
    value
        .pointer(pointer)
        .and_then(serde_json::Value::as_array)
        .unwrap_or_else(|| panic!("app manifest must carry an array at {pointer}"))
        .iter()
        .map(|entry| entry.as_str().expect("string entry"))
        .collect()
}

#[test]
fn the_documented_app_manifest_declares_the_native_agent_surface() {
    let manifest = documented_app_manifest();

    // `features.agent_view` (the Agents feature; switching from
    // `assistant_view` is irreversible per Slack). Description ≤ 300 chars.
    let description = manifest
        .pointer("/features/agent_view/agent_description")
        .and_then(serde_json::Value::as_str)
        .expect("features.agent_view.agent_description");
    assert!(
        !description.trim().is_empty() && description.chars().count() <= 300,
        "agent_description must be 1..=300 chars, got {}",
        description.chars().count()
    );
    assert!(
        manifest.pointer("/features/assistant_view").is_none(),
        "the manifest must declare agent_view, never the legacy assistant_view"
    );
    let prompts = manifest
        .pointer("/features/agent_view/suggested_prompts")
        .and_then(serde_json::Value::as_array)
        .expect("features.agent_view.suggested_prompts");
    assert!(
        (2..=4).contains(&prompts.len()),
        "2–4 suggested prompts, got {}",
        prompts.len()
    );
    for prompt in prompts {
        for field in ["title", "message"] {
            assert!(
                prompt
                    .get(field)
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|value| !value.trim().is_empty()),
                "every suggested prompt needs a non-empty `{field}`: {prompt}"
            );
        }
    }

    // The Messages tab is where an Agent DM lives, and it must be writable.
    assert_eq!(
        manifest.pointer("/features/app_home/messages_tab_enabled"),
        Some(&serde_json::Value::Bool(true))
    );
    assert_eq!(
        manifest.pointer("/features/app_home/messages_tab_read_only_enabled"),
        Some(&serde_json::Value::Bool(false))
    );

    // Bot scopes: `assistant:write` (added with the Agents feature; declared
    // so a manifest import carries it) and `chat:write` (every streaming and
    // session method).
    let bot_scopes = string_array(&manifest, "/oauth_config/scopes/bot");
    for scope in ["assistant:write", "chat:write"] {
        assert!(
            bot_scopes.contains(&scope),
            "bot scopes must include {scope}: {bot_scopes:?}"
        );
    }

    // The Agent event family the ingress handles, plus the message events
    // the channel already relied on.
    let events = string_array(&manifest, "/settings/event_subscriptions/bot_events");
    for event in [
        "app_mention",
        "message.im",
        "app_home_opened",
        "app_context_changed",
        "assistant_thread_started",
        "assistant_thread_context_changed",
        "agent_session_stopped",
        "agent_session_title_changed",
    ] {
        assert!(
            events.contains(&event),
            "bot_events must include {event}: {events:?}"
        );
    }
}

#[test]
fn every_slack_call_the_package_makes_is_declared_bot_token_egress() {
    let manifest: toml::Value =
        toml::from_str(&read("crates/extensions/packages/slack/manifest.toml"))
            .expect("manifest.toml parses");
    let egress = manifest
        .get("channel")
        .and_then(|channel| channel.get("egress"))
        .and_then(toml::Value::as_array)
        .expect("[[channel.egress]] entries");

    for method in SlackWebApiMethod::ALL {
        let http_method = match method.http_method() {
            NetworkMethod::Get => "get",
            NetworkMethod::Post => "post",
            other => panic!("unexpected HTTP method {other:?} for {}", method.name()),
        };
        let declared = egress.iter().any(|entry| {
            let host = entry.get("host").and_then(toml::Value::as_str);
            let credential = entry.get("credential_handle").and_then(toml::Value::as_str);
            let methods: Vec<&str> = entry
                .get("methods")
                .and_then(toml::Value::as_array)
                .map(|methods| methods.iter().filter_map(toml::Value::as_str).collect())
                .unwrap_or_default();
            let paths: Vec<&str> = entry
                .get("paths")
                .and_then(toml::Value::as_array)
                .map(|paths| paths.iter().filter_map(toml::Value::as_str).collect())
                .unwrap_or_default();
            host == Some("slack.com")
                && credential == Some("slack_bot_token")
                && methods.contains(&http_method)
                && paths.contains(&method.path())
        });
        assert!(
            declared,
            "{} {} is called by the package but not declared as exact bot-token egress \
             in manifest.toml",
            http_method.to_ascii_uppercase(),
            method.path()
        );
    }

    // The Agent reply surface is exact-path egress, and the reply transport
    // is the stream cadence that drives it.
    assert_eq!(
        manifest
            .get("channel")
            .and_then(|channel| channel.get("reply"))
            .and_then(|reply| reply.get("transport"))
            .and_then(toml::Value::as_str),
        Some("stream"),
        "[channel.reply] transport must be stream"
    );
    assert_eq!(
        manifest
            .get("channel")
            .and_then(|channel| channel.get("delivery"))
            .and_then(|delivery| delivery.get("transport"))
            .and_then(toml::Value::as_str),
        Some("message"),
        "[channel.delivery] transport stays message"
    );
}

#[test]
fn the_setup_guides_mirror_the_agent_manifest_essentials() {
    for guide in [
        "docs/channels/slack.mdx",
        "docs/internal/reborn/setup-slack-for-reborn-binary.md",
    ] {
        let text = read(guide);
        for needle in [
            "agent_view",
            "assistant:write",
            "agent_session_stopped",
            "feature_disabled",
        ] {
            assert!(
                text.contains(needle),
                "{guide} must mention `{needle}` (the native Agent setup essentials)"
            );
        }
    }
}
