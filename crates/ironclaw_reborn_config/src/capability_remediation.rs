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
}
