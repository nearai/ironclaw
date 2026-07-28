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

/// The console steps plus the apply step — the complete "here is how to
/// configure Google on this instance" body.
///
/// TWO independent enforcement points produce a "Google is not configured"
/// message, and they stay separate on purpose (defense in depth at different
/// lifecycle stages): the activation-time readiness map
/// (`extension_host::provider_instance_readiness`) and the dispatch-time
/// backstop (`extension_host::gsuite`). What they must NOT do is compose the
/// same body two different ways — that is how the two strings drift. Both call
/// this one function; only the leading sentence differs.
pub fn google_setup_steps_text() -> String {
    format!("{}\n\n{}", google_remediation_text(), apply_step_text())
}

/// Pre-dispatch "no Google OAuth backend configured on this instance at all"
/// text — distinct from a per-account credential problem. Consumed by
/// `ironclaw_reborn_composition::extension_host::gsuite`.
pub fn google_not_configured_text() -> String {
    format!(
        "Google Workspace access is not configured on this ironclaw instance.\n\n{}",
        google_setup_steps_text()
    )
}

/// "The account resolved but Google rejected the credentials" text — the
/// backend-auth arm, distinct from both `google_not_configured_text` (no
/// backend at all) and an ordinary auth gate (no/expired account).
///
/// Phrased to name the config key once and then refer back to it ("to update
/// it"), rather than repeating "the client secret" in prose — a readability
/// choice, not a constraint. Host-authored remediation is exempt from the
/// downstream credential-vocabulary scan by PROVENANCE
/// (`ObservationTrust::HostAuthored`), so this text may say whatever it needs
/// to; there is no parser in another crate to appease.
pub fn google_backend_auth_text() -> String {
    format!(
        "Google OAuth is configured but the provider rejected the request while exchanging \
         or refreshing the token (e.g. invalid_client). Re-run `ironclaw config set \
         google.client_secret` to update it, then confirm the OAuth client credentials at \
         https://console.cloud.google.com/apis/credentials. {}",
        apply_step_text()
    )
}

/// Every FIXED host-authored remediation text in the Reborn stack, enumerated
/// so remediation coverage cannot silently lapse.
///
/// This exists because of the #6299 regression: one full-path test covered the
/// google readiness scenario, so when the host_api hop started collapsing
/// host-authored text to the safe-summary placeholder, every OTHER producer
/// degraded silently and nothing went red. A hand-maintained list inside a test
/// rots the same way. Adding a variant here breaks the exhaustive `match` in
/// [`Self::text`], so a new producer cannot ship without at least being given
/// its text here — and [`Self::all`]'s witness match points the author at the
/// array they must extend (see that method's note on the residual gap).
///
/// Only fixed texts live here. Two producers build their text from a runtime
/// `reason` string (`ProductSurfaceFailure::ProviderInstanceNotConfigured` and
/// `InvalidBindingRequest`); those reasons are themselves assembled from the
/// constants below, and the composed result is covered at the integration tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostRemediationText {
    /// `gsuite`: no Google OAuth backend configured on this instance.
    GoogleNotConfigured,
    /// `gsuite`: Google rejected the configured credentials.
    GoogleBackendAuth,
    /// The shared "restart to apply" follow-up sentence.
    ApplyStep,
}

impl HostRemediationText {
    /// The text this producer emits. Exhaustive by construction.
    pub fn text(self) -> String {
        match self {
            Self::GoogleNotConfigured => google_not_configured_text(),
            Self::GoogleBackendAuth => google_backend_auth_text(),
            Self::ApplyStep => apply_step_text().to_string(),
        }
    }

    /// Every variant.
    ///
    /// The witness match below is a REMINDER, not a proof: a variant added to
    /// the enum and to [`Self::text`] but omitted from `ALL` still compiles
    /// (the witness only iterates what `ALL` already contains) and would escape
    /// coverage. Rust cannot enumerate a variant set without naming it, so the
    /// residual gap is documented rather than claimed away. The length
    /// annotation on `ALL` is the second guard: adding an entry without
    /// bumping it fails to compile.
    pub fn all() -> Vec<Self> {
        const ALL: [HostRemediationText; 3] = [
            HostRemediationText::GoogleNotConfigured,
            HostRemediationText::GoogleBackendAuth,
            HostRemediationText::ApplyStep,
        ];
        for entry in ALL {
            // Exhaustiveness witness — one arm per variant, no catch-all. A new
            // variant breaks THIS match, forcing `ALL` (and the coverage test
            // that reads it) to be updated.
            match entry {
                Self::GoogleNotConfigured => {}
                Self::GoogleBackendAuth => {}
                Self::ApplyStep => {}
            }
        }
        ALL.to_vec()
    }
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

    /// The two independent "Google is not configured" enforcement points
    /// (activation-time readiness map, dispatch-time gsuite backstop) stay
    /// separate BY DESIGN — they gate different lifecycle stages. What they
    /// must share is the body, so the console steps and the apply step cannot
    /// drift between them.
    #[test]
    fn google_not_configured_text_embeds_the_shared_setup_steps_verbatim() {
        let steps = google_setup_steps_text();
        assert!(steps.contains("config set google.client_id"));
        assert!(steps.contains("ironclaw service restart"));
        assert!(
            google_not_configured_text().contains(&steps),
            "the dispatch-time text must embed the shared body verbatim, not recompose it"
        );
    }
}
