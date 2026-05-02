//! Slack app manifest generation.
//!
//! The manifest declares the bot scopes, slash commands, and event
//! subscriptions Slack needs to install IronClaw into a workspace.
//! The installer uploads the JSON to <https://api.slack.com/apps?new_app=1>
//! (the "From an app manifest" flow); Slack creates the app and returns a
//! Client ID / Client Secret pair the operator pastes into the IronClaw
//! config to complete the OAuth dance.
//!
//! ## Scope policy
//!
//! Minimal-by-default: only the scopes IronClaw needs for the v0.1 surface
//! ship in the manifest. Optional scopes (channel reads, user lookups) are
//! gated behind explicit operator opt-in in a follow-up commit.

use serde::Serialize;
use serde_json::json;

/// Bot scopes IronClaw requires for the v0.1 Slack surface (DMs, slash
/// command, mention handling). Order is not significant.
pub const MINIMAL_BOT_SCOPES: &[&str] = &[
    "chat:write",        // post replies
    "app_mentions:read", // see @ironclaw in channels
    "im:history",        // read DM history (per-user paired)
    "im:write",          // open DMs to users
    "commands",          // slash command surface
];

/// Bot events IronClaw subscribes to. Drives the WASM channel's inbound
/// stream.
const BOT_EVENTS: &[&str] = &["app_mention", "message.im"];

/// Top-level manifest shape. Slack accepts a superset; unknown keys are
/// rejected, so we serialise only what we set.
#[derive(Debug, Serialize)]
pub struct SlackManifest {
    pub display_information: DisplayInformation,
    pub features: Features,
    pub oauth_config: OAuthConfig,
    pub settings: Settings,
}

#[derive(Debug, Serialize)]
pub struct DisplayInformation {
    pub name: String,
    pub description: String,
    pub background_color: String,
}

#[derive(Debug, Serialize)]
pub struct Features {
    pub bot_user: BotUser,
    pub slash_commands: Vec<SlashCommand>,
    pub app_home: AppHome,
}

#[derive(Debug, Serialize)]
pub struct BotUser {
    pub display_name: String,
    pub always_online: bool,
}

#[derive(Debug, Serialize)]
pub struct SlashCommand {
    pub command: String,
    pub url: String,
    pub description: String,
    pub usage_hint: String,
    pub should_escape: bool,
}

#[derive(Debug, Serialize)]
pub struct AppHome {
    pub home_tab_enabled: bool,
    pub messages_tab_enabled: bool,
    pub messages_tab_read_only_enabled: bool,
}

#[derive(Debug, Serialize)]
pub struct OAuthConfig {
    pub redirect_urls: Vec<String>,
    pub scopes: Scopes,
}

#[derive(Debug, Serialize)]
pub struct Scopes {
    pub bot: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct Settings {
    pub event_subscriptions: EventSubscriptions,
    pub interactivity: Interactivity,
    pub org_deploy_enabled: bool,
    pub socket_mode_enabled: bool,
    pub token_rotation_enabled: bool,
}

#[derive(Debug, Serialize)]
pub struct EventSubscriptions {
    pub request_url: String,
    pub bot_events: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct Interactivity {
    pub is_enabled: bool,
    pub request_url: String,
}

/// Generate a Slack app manifest for an IronClaw deployment.
///
/// `install_base_url` is the public HTTPS origin Slack will hit for events,
/// slash commands, and OAuth callbacks (e.g. `https://ironclaw.example.com`).
/// Trailing slashes are tolerated.
pub fn generate_manifest(install_base_url: &str) -> SlackManifest {
    let base = install_base_url.trim_end_matches('/').to_string();
    SlackManifest {
        display_information: DisplayInformation {
            name: "IronClaw".to_string(),
            description: "IronClaw agent — knowledge, automations, and \
                          approvals for your workspace."
                .to_string(),
            background_color: "#0b1b2b".to_string(),
        },
        features: Features {
            bot_user: BotUser {
                display_name: "IronClaw".to_string(),
                always_online: true,
            },
            slash_commands: vec![SlashCommand {
                command: "/ironclaw".to_string(),
                url: format!("{base}/api/channels/slack/slash"),
                description: "Ask IronClaw a question or run a tool.".to_string(),
                usage_hint: "<your prompt>".to_string(),
                should_escape: false,
            }],
            app_home: AppHome {
                home_tab_enabled: false,
                messages_tab_enabled: true,
                messages_tab_read_only_enabled: false,
            },
        },
        oauth_config: OAuthConfig {
            redirect_urls: vec![format!("{base}/auth/slack/install/callback")],
            scopes: Scopes {
                bot: MINIMAL_BOT_SCOPES.iter().map(|s| s.to_string()).collect(),
            },
        },
        settings: Settings {
            event_subscriptions: EventSubscriptions {
                request_url: format!("{base}/api/channels/slack/events"),
                bot_events: BOT_EVENTS.iter().map(|s| s.to_string()).collect(),
            },
            interactivity: Interactivity {
                is_enabled: true,
                request_url: format!("{base}/api/channels/slack/interactivity"),
            },
            org_deploy_enabled: false,
            socket_mode_enabled: false,
            token_rotation_enabled: false,
        },
    }
}

/// Slash-command target URL the installer pastes into the Slack app config
/// after the manifest is uploaded. Returned as a convenience for operator
/// output paths that do not need the full manifest.
pub fn slash_command_url(install_base_url: &str) -> String {
    let base = install_base_url.trim_end_matches('/');
    format!("{base}/api/channels/slack/slash")
}

/// Serialise the manifest as a `serde_json::Value` for callers that want
/// to munge it before printing (e.g. add an `org_deploy_enabled` override).
pub fn manifest_json(install_base_url: &str) -> serde_json::Value {
    serde_json::to_value(generate_manifest(install_base_url)).unwrap_or_else(|_| json!({}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_includes_minimal_bot_scopes() {
        let m = manifest_json("https://ironclaw.example.com");
        let scopes = m["oauth_config"]["scopes"]["bot"]
            .as_array()
            .expect("bot scopes array")
            .iter()
            .filter_map(|v| v.as_str())
            .collect::<Vec<_>>();

        for required in MINIMAL_BOT_SCOPES {
            assert!(
                scopes.contains(required),
                "manifest is missing required scope {required}; got {scopes:?}"
            );
        }
        // Minimal-by-default — no extra scopes leak in.
        assert_eq!(
            scopes.len(),
            MINIMAL_BOT_SCOPES.len(),
            "manifest carries unexpected extra scopes: {scopes:?}"
        );
    }

    #[test]
    fn manifest_slash_command_targets_install_base_url() {
        let m = manifest_json("https://ironclaw.example.com/");
        let url = m["features"]["slash_commands"][0]["url"].as_str().unwrap();
        assert_eq!(url, "https://ironclaw.example.com/api/channels/slack/slash");
        // Trailing-slash normalisation.
        assert_eq!(
            slash_command_url("https://ironclaw.example.com//"),
            "https://ironclaw.example.com/api/channels/slack/slash"
        );
    }

    #[test]
    fn manifest_oauth_redirect_includes_install_callback() {
        let m = manifest_json("https://ironclaw.example.com");
        let urls = m["oauth_config"]["redirect_urls"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|v| v.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            urls,
            vec!["https://ironclaw.example.com/auth/slack/install/callback"]
        );
    }

    #[test]
    fn manifest_does_not_enable_socket_mode_or_org_deploy() {
        let m = manifest_json("https://ironclaw.example.com");
        assert_eq!(m["settings"]["socket_mode_enabled"], false);
        assert_eq!(m["settings"]["org_deploy_enabled"], false);
    }
}
