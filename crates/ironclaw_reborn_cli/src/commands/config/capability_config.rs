//! Single chokepoint: `config set` alias -> canonical destination, shape
//! validation, and remediation text for LLM/Google/Slack capability setup.
//!
//! [`super::set::ConfigSetCommand`] is the only consumer of the alias/shape
//! machinery in this module. The Google remediation text itself has moved to
//! `ironclaw_reborn_config::google_remediation_text` (this module just
//! re-exports it as [`google_remediation_text`]) so
//! `ironclaw_reborn_composition::extension_host::gsuite`'s "not configured"
//! tool-result error can share the exact same wording without depending on
//! this crate — `ironclaw_reborn_cli` sits above `composition` in the
//! dependency graph (`cli` depends on `composition`, never the reverse), so
//! composition-layer code cannot import CLI modules directly, but both
//! already depend on `ironclaw_reborn_config`.

/// Where a `config set` value physically lands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ConfigDestination {
    /// A literal value written into `config.toml`.
    ConfigToml,
    /// A secret value written into the encrypted secret store.
    SecretStore,
    /// The WebChat v2 bearer token file (`<reborn_home>/webui-token`) —
    /// rotate-only, no arbitrary value accepted.
    TokenFile,
}

/// Every alias `config set` understands, resolved from the raw key
/// argument. `LlmApiKey`'s `provider_id` carries either the explicit
/// `<provider>.api_key` prefix or the `nearai` default when the bare
/// `api_key` alias is used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ConfigKey {
    // Enum variant fields share the enum's own visibility (unlike struct
    // fields, which need per-field `pub`) — `provider_id` is visible
    // wherever `ConfigKey` itself is, i.e. throughout `commands::config`.
    LlmApiKey { provider_id: String },
    GoogleClientId,
    GoogleClientSecret,
    GoogleRedirectUri,
    SlackEnabled,
    WebuiToken,
}

impl ConfigKey {
    /// Classify a raw `config set <key>` argument. `None` for any key
    /// `config set` does not recognize.
    pub(super) fn classify(key: &str) -> Option<Self> {
        if key == "api_key" {
            return Some(Self::LlmApiKey {
                provider_id: super::init::DEFAULT_LLM_PROVIDER_ID.to_string(),
            });
        }
        if let Some(provider_id) = key.strip_suffix(".api_key") {
            if provider_id.is_empty() {
                return None;
            }
            return Some(Self::LlmApiKey {
                provider_id: provider_id.to_string(),
            });
        }
        match key {
            "google.client_id" => Some(Self::GoogleClientId),
            "google.client_secret" => Some(Self::GoogleClientSecret),
            "google.redirect_uri" => Some(Self::GoogleRedirectUri),
            "slack.enabled" => Some(Self::SlackEnabled),
            "webui.token" => Some(Self::WebuiToken),
            _ => None,
        }
    }

    pub(super) fn destination(&self) -> ConfigDestination {
        match self {
            Self::LlmApiKey { .. } | Self::GoogleClientSecret => ConfigDestination::SecretStore,
            Self::GoogleClientId | Self::GoogleRedirectUri | Self::SlackEnabled => {
                ConfigDestination::ConfigToml
            }
            Self::WebuiToken => ConfigDestination::TokenFile,
        }
    }

    /// `true` when input should be prompted for with terminal echo
    /// suppressed rather than taken as a plain CLI argument default.
    pub(super) fn is_secret_prompted(&self) -> bool {
        matches!(self.destination(), ConfigDestination::SecretStore)
    }
}

/// Reject/warn shape validation applied to a candidate value before it is
/// written, keyed by [`ConfigKey`]. `Reject` refuses the write outright;
/// `Warn` prints a message but still writes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ShapeVerdict {
    Ok,
    Warn(String),
    Reject(String),
}

/// Validate `value`'s shape for `key`, independent of the
/// secret/non-secret destination check (that one lives inline in
/// `set.rs::set_value_key` via `reject_inline_secret` from
/// `ironclaw_reborn_config`).
pub(super) fn validate_shape(key: &ConfigKey, value: &str) -> ShapeVerdict {
    match key {
        ConfigKey::GoogleClientId => {
            if value.ends_with(".apps.googleusercontent.com") {
                ShapeVerdict::Ok
            } else {
                // Never echo `value` back: this key is a plausible target
                // for a mis-pasted secret (e.g. an API key pasted into the
                // wrong prompt), and this shape check runs after the
                // secret-shape law in `set.rs` already let it through —
                // describe the expected shape instead of the rejected
                // input. See the secret-echo review fix in `set.rs`.
                ShapeVerdict::Reject(
                    "google.client_id does not look like a Google OAuth client id (expected it \
                     to end in `.apps.googleusercontent.com`) — copy it from the OAuth client's \
                     page at https://console.cloud.google.com/apis/credentials"
                        .to_string(),
                )
            }
        }
        ConfigKey::GoogleClientSecret => {
            if value.starts_with("GOCSPX-") {
                ShapeVerdict::Ok
            } else {
                ShapeVerdict::Warn(
                    "google.client_secret does not start with `GOCSPX-` — that's the shape \
                     Google issues for OAuth client secrets created since 2022; older projects \
                     may still have a differently-shaped secret, so this is a warning, not a \
                     refusal"
                        .to_string(),
                )
            }
        }
        ConfigKey::GoogleRedirectUri => {
            if value.starts_with("http://") || value.starts_with("https://") {
                ShapeVerdict::Ok
            } else {
                // Never echo `value` back — see the `GoogleClientId` arm
                // above for why.
                ShapeVerdict::Reject(
                    "google.redirect_uri does not look like a URL (expected it to start with \
                     `http://` or `https://`) — it must exactly match a redirect URI registered \
                     on the OAuth client at https://console.cloud.google.com/apis/credentials"
                        .to_string(),
                )
            }
        }
        ConfigKey::SlackEnabled => {
            if value.eq_ignore_ascii_case("true") || value.eq_ignore_ascii_case("false") {
                ShapeVerdict::Ok
            } else {
                ShapeVerdict::Reject(format!("slack.enabled `{value}` must be `true` or `false`"))
            }
        }
        ConfigKey::LlmApiKey { .. } | ConfigKey::WebuiToken => ShapeVerdict::Ok,
    }
}

/// BYO (bring-your-own) console-steps remediation text for Google OAuth
/// setup, printed by `config set` guidance. Delegates to
/// `ironclaw_reborn_config::google_remediation_text` — the single shared
/// source of this text — so this crate's re-export point stays a stable
/// call site for `set.rs` even though the wording itself lives lower in the
/// dependency graph. See the module doc for why the text moved.
pub(super) fn google_remediation_text() -> String {
    ironclaw_reborn_config::google_remediation_text()
}

/// Slack remediation text: per Correction A in the PR-C plan, Slack has
/// no CLI-settable bot token/signing secret — the only supported surface
/// is the WebUI extension setup flow. Describes WHAT to configure only;
/// the restart apply-step sentence is appended once by the caller (see
/// `set.rs::print_apply_step`), not embedded here — see the module doc.
pub(super) fn slack_remediation_text(base_url: &str) -> String {
    format!(
        "Slack setup is WebUI-only: finish connecting Slack at {base_url}/extensions \
         (config set slack.enabled true|false only toggles whether the route mounts; it does \
         not configure Slack app identity or credentials)."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_bare_api_key_defaults_to_nearai() {
        assert_eq!(
            ConfigKey::classify("api_key"),
            Some(ConfigKey::LlmApiKey {
                provider_id: super::super::init::DEFAULT_LLM_PROVIDER_ID.to_string()
            })
        );
    }

    #[test]
    fn classify_provider_scoped_api_key() {
        assert_eq!(
            ConfigKey::classify("openai.api_key"),
            Some(ConfigKey::LlmApiKey {
                provider_id: "openai".to_string()
            })
        );
    }

    #[test]
    fn classify_rejects_empty_provider_prefix() {
        assert_eq!(ConfigKey::classify(".api_key"), None);
    }

    #[test]
    fn classify_known_google_and_slack_and_webui_keys() {
        assert_eq!(
            ConfigKey::classify("google.client_id"),
            Some(ConfigKey::GoogleClientId)
        );
        assert_eq!(
            ConfigKey::classify("google.client_secret"),
            Some(ConfigKey::GoogleClientSecret)
        );
        assert_eq!(
            ConfigKey::classify("google.redirect_uri"),
            Some(ConfigKey::GoogleRedirectUri)
        );
        assert_eq!(
            ConfigKey::classify("slack.enabled"),
            Some(ConfigKey::SlackEnabled)
        );
        assert_eq!(
            ConfigKey::classify("webui.token"),
            Some(ConfigKey::WebuiToken)
        );
    }

    #[test]
    fn classify_unknown_key_is_none() {
        assert_eq!(ConfigKey::classify("slack.bot_token"), None);
        assert_eq!(ConfigKey::classify("slack.signing_secret"), None);
        assert_eq!(ConfigKey::classify("nonsense.key"), None);
    }

    #[test]
    fn destination_matches_alias_table() {
        assert_eq!(
            ConfigKey::LlmApiKey {
                provider_id: "openai".to_string()
            }
            .destination(),
            ConfigDestination::SecretStore
        );
        assert_eq!(
            ConfigKey::GoogleClientSecret.destination(),
            ConfigDestination::SecretStore
        );
        assert_eq!(
            ConfigKey::GoogleClientId.destination(),
            ConfigDestination::ConfigToml
        );
        assert_eq!(
            ConfigKey::GoogleRedirectUri.destination(),
            ConfigDestination::ConfigToml
        );
        assert_eq!(
            ConfigKey::SlackEnabled.destination(),
            ConfigDestination::ConfigToml
        );
        assert_eq!(
            ConfigKey::WebuiToken.destination(),
            ConfigDestination::TokenFile
        );
    }

    #[test]
    fn google_client_id_validator_accepts_and_rejects() {
        assert_eq!(
            validate_shape(
                &ConfigKey::GoogleClientId,
                "123-abc.apps.googleusercontent.com"
            ),
            ShapeVerdict::Ok
        );
        assert!(matches!(
            validate_shape(&ConfigKey::GoogleClientId, "not-a-client-id"),
            ShapeVerdict::Reject(_)
        ));
    }

    /// Thermo MUST: a Reject message must never echo the rejected value —
    /// a value pasted into the wrong key (e.g. a secret pasted into
    /// `google.client_id`) must not be printed back to the terminal/logs.
    #[test]
    fn google_client_id_reject_message_does_not_echo_the_rejected_value() {
        let rejected_value = "sk-proj-mispasted-secret-XXXXXXXXXX";
        let ShapeVerdict::Reject(message) =
            validate_shape(&ConfigKey::GoogleClientId, rejected_value)
        else {
            panic!("expected Reject");
        };
        assert!(
            !message.contains(rejected_value),
            "message must not echo the rejected value: {message}"
        );
    }

    #[test]
    fn google_redirect_uri_reject_message_does_not_echo_the_rejected_value() {
        let rejected_value = "sk-proj-mispasted-secret-XXXXXXXXXX";
        let ShapeVerdict::Reject(message) =
            validate_shape(&ConfigKey::GoogleRedirectUri, rejected_value)
        else {
            panic!("expected Reject");
        };
        assert!(
            !message.contains(rejected_value),
            "message must not echo the rejected value: {message}"
        );
    }

    #[test]
    fn google_client_secret_validator_warns_not_rejects() {
        assert_eq!(
            validate_shape(&ConfigKey::GoogleClientSecret, "GOCSPX-abc123"),
            ShapeVerdict::Ok
        );
        let bogus_shaped_value = "old-style-secret-do-not-echo-me";
        let ShapeVerdict::Warn(message) =
            validate_shape(&ConfigKey::GoogleClientSecret, bogus_shaped_value)
        else {
            panic!("expected Warn");
        };
        // Mirrors the Reject-branch echo checks above (`google_client_id_...`,
        // `google_redirect_uri_...`): the Warn branch must never echo the raw
        // input either, only the Reject branch was previously pinned.
        assert!(
            !message.contains(bogus_shaped_value),
            "warning message must not echo the rejected value: {message}"
        );
    }

    #[test]
    fn google_redirect_uri_validator_requires_url_shape() {
        assert_eq!(
            validate_shape(
                &ConfigKey::GoogleRedirectUri,
                "http://127.0.0.1:3000/oauth/google/callback"
            ),
            ShapeVerdict::Ok
        );
        assert!(matches!(
            validate_shape(&ConfigKey::GoogleRedirectUri, "not-a-url"),
            ShapeVerdict::Reject(_)
        ));
    }

    #[test]
    fn slack_enabled_validator_requires_bool_shape() {
        assert_eq!(
            validate_shape(&ConfigKey::SlackEnabled, "true"),
            ShapeVerdict::Ok
        );
        assert_eq!(
            validate_shape(&ConfigKey::SlackEnabled, "FALSE"),
            ShapeVerdict::Ok
        );
        assert!(matches!(
            validate_shape(&ConfigKey::SlackEnabled, "yes"),
            ShapeVerdict::Reject(_)
        ));
    }

    #[test]
    fn llm_api_key_and_webui_token_have_no_shape_rejection() {
        assert_eq!(
            validate_shape(
                &ConfigKey::LlmApiKey {
                    provider_id: "openai".to_string()
                },
                "anything"
            ),
            ShapeVerdict::Ok
        );
        assert_eq!(
            validate_shape(&ConfigKey::WebuiToken, "anything"),
            ShapeVerdict::Ok
        );
    }

    #[test]
    fn remediation_text_points_at_the_right_surfaces() {
        let google = google_remediation_text();
        assert!(google.contains("console.cloud.google.com"));
        assert!(google.contains("config set google.client_id"));
        assert!(google.contains("config set google.client_secret"));
        assert!(google.contains("config set google.redirect_uri"));
        // The remediation text itself must NOT embed the restart step: that
        // sentence is appended exactly once by the surface (CLI's
        // `print_apply_step` / composition's `apply_step_text`), never
        // baked into the remediation text, or callers that append it too
        // produce a duplicate "service restart" instruction.
        assert_eq!(
            google.matches("service restart").count(),
            0,
            "google_remediation_text must not embed the restart step itself \
             (callers append it exactly once): {google}"
        );

        let slack = slack_remediation_text("http://127.0.0.1:3000");
        assert!(slack.contains("http://127.0.0.1:3000/extensions"));
        assert!(!slack.contains("config set slack.bot_token"));
        assert_eq!(
            slack.matches("service restart").count(),
            0,
            "slack_remediation_text must not embed the restart step itself \
             (callers append it exactly once): {slack}"
        );
    }
}
