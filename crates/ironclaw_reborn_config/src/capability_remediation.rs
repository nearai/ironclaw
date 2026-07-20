//! Shared remediation text for capability BYO setup.
//!
//! `google_remediation_text` is consumed by two independent surfaces that
//! must not drift apart:
//!
//! - `ironclaw_reborn_cli::commands::config::capability_config` — printed as
//!   `config set google.*` follow-up guidance.
//! - `ironclaw_reborn_composition::extension_host::gsuite` — printed in the
//!   Gmail/Google Workspace "not configured" tool-result error a capability
//!   dispatch returns before it ever reaches credential resolution.
//!
//! `slack_remediation_text` mirrors the same split: `ironclaw_reborn_composition`'s
//! `extension_host::provider_instance_readiness` module consumes the
//! base-url-free variant below to build the `slack_personal`
//! readiness-map entry; `ironclaw_reborn_cli`'s `capability_config` module
//! wraps `slack_remediation_text_with_base_url` to keep printing a concrete
//! serve base URL. `slack_setup_sentence` is the single source of truth both
//! call through, so the wording cannot drift between the two surfaces.
//!
//! `ironclaw_reborn_cli` depends on `ironclaw_reborn_composition`, never the
//! reverse, so this text cannot live in the CLI crate (composition could not
//! import it). It lives here instead, since both crates already depend on
//! `ironclaw_reborn_config`.

/// BYO (bring-your-own) console-steps remediation text for Google OAuth
/// setup: the exact `config set` commands and the Google Cloud Console steps
/// that produce their values.
pub fn google_remediation_text() -> String {
    "Google OAuth setup (one-time, per instance):\n  \
     1. https://console.cloud.google.com/apis/credentials -> Create Credentials -> OAuth \
     client ID -> Desktop app\n  \
     2. Enable the Gmail API (and Calendar/Drive as needed) for the project\n  \
     3. ironclaw config set google.client_id <id>.apps.googleusercontent.com\n  \
     4. ironclaw config set google.client_secret   (prompts, hidden input)\n  \
     5. ironclaw config set google.redirect_uri <redirect-uri-from-the-oauth-client>"
        .to_string()
}

/// Single source of truth for the Slack BYO setup sentence, parameterized on
/// WHERE the WebUI extensions page is described (a relative route for the
/// composition-consumed variant, a concrete base URL for the CLI-consumed
/// variant) — see the module doc for why two public wrappers exist. Describes
/// WHAT to configure only; the restart apply-step sentence is appended once
/// by each caller (`apply_step_text()` / `set.rs::print_apply_step`), never
/// embedded here.
fn slack_connect_clause(webui_extensions_location: &str) -> String {
    format!(
        "connect your Slack workspace at {webui_extensions_location} (workspace OAuth \
         happens there; config set cannot supply Slack app identity or credentials)"
    )
}

/// BYO console-steps remediation text for Slack, base-url-free: the
/// composition-time build cannot know the serve base URL (it is a
/// per-invocation `serve` flag, resolved later), so this variant names the
/// route relatively. Consumed by
/// `ironclaw_reborn_composition::extension_host::provider_instance_readiness`.
/// Unlike `google_remediation_text`, this variant embeds its own restart step:
/// Slack's apply step sits in the MIDDLE of the sequence (the route must mount
/// before the WebUI can run workspace OAuth), so a trailing
/// `apply_step_text()` would both misorder the instructions and imply "then
/// ask again" when the user still has a connect step left. Callers of this
/// variant must therefore NOT append `apply_step_text()`.
pub fn slack_remediation_text() -> String {
    format!(
        "Slack setup (one-time, per instance):\n  \
         1. ironclaw config set slack.enabled true\n  \
         2. ironclaw service restart   (mounts the Slack extension route)\n  \
         3. {}",
        slack_connect_clause("/extensions in the WebUI")
    )
}

/// Same sentence, with the concrete serve base URL the CLI resolves at
/// `config set` time. Consumed by
/// `ironclaw_reborn_cli::commands::config::capability_config::slack_remediation_text`.
/// The CLI prints this immediately after the user ran `config set
/// slack.enabled`, so it neither repeats that command nor embeds the restart
/// (`set.rs::print_apply_step` appends the canonical restart sentence right
/// after it) — it only names the remaining connect step.
pub fn slack_remediation_text_with_base_url(base_url: &str) -> String {
    format!(
        "After restarting, {}",
        slack_connect_clause(&format!("{base_url}/extensions"))
    )
}

/// Canonical "apply the change" follow-up sentence: `config set` never
/// restarts the service itself (see the module-level design note in
/// `google_remediation_text` and `ironclaw_reborn_cli::commands::config::set`),
/// so every surface that tells a caller "go configure this" must also tell
/// them the explicit next step rather than implying it happens automatically.
pub fn apply_step_text() -> &'static str {
    "Run `ironclaw service restart` to apply the change, then ask again."
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_step_text_names_the_explicit_restart_command() {
        let text = apply_step_text();
        assert!(text.contains("ironclaw service restart"));
        assert!(!text.contains("automatically"));
    }

    #[test]
    fn remediation_text_points_at_the_right_surfaces() {
        let google = google_remediation_text();
        assert!(google.contains("console.cloud.google.com"));
        assert!(google.contains("config set google.client_id"));
        assert!(google.contains("config set google.client_secret"));
        assert!(google.contains("config set google.redirect_uri"));
    }

    #[test]
    fn slack_remediation_text_names_the_relative_extensions_route() {
        let slack = slack_remediation_text();
        assert!(slack.contains("/extensions"));
        assert!(slack.contains("config set slack.enabled"));
        assert!(!slack.contains("config set slack.bot_token"));
        assert_eq!(
            slack.matches("service restart").count(),
            0,
            "slack_remediation_text must not embed the restart step itself \
             (callers append it exactly once): {slack}"
        );
    }

    #[test]
    fn slack_remediation_text_with_base_url_embeds_the_concrete_url() {
        let slack = slack_remediation_text_with_base_url("http://127.0.0.1:3000");
        assert!(slack.contains("http://127.0.0.1:3000/extensions"));
        assert!(slack.contains("config set slack.enabled"));
        assert_eq!(
            slack.matches("service restart").count(),
            0,
            "slack_remediation_text_with_base_url must not embed the restart step itself: {slack}"
        );
    }
}
