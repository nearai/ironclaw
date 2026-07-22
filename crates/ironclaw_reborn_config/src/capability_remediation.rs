//! Shared remediation text for capability BYO setup.
//!
//! `google_remediation_text` preserves the standalone CLI's legacy
//! `config set google.*` guidance. Runtime extension setup is declared by
//! Manifest V3 and edited through the WebUI administrator configuration.

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

/// "The account resolved but Google rejected the credentials" text — the
/// backend-auth arm, distinct from an ordinary auth gate (no/expired account).
///
/// Host-authored remediation is exempt from the downstream
/// credential-vocabulary scan by PROVENANCE (`ObservationTrust::HostAuthored`),
/// so it can name the fields the operator must replace. The same bytes remain
/// rejected if an untrusted capability tries to emit them.
pub fn google_backend_auth_text() -> String {
    "Google OAuth is configured but the provider rejected the request while exchanging \
     or refreshing the token (e.g. invalid_client). Replace the Google OAuth client ID \
     and client secret in WebUI Admin > Extension Configuration, confirm them at \
     https://console.cloud.google.com/apis/credentials, then retry."
        .to_string()
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
/// Only fixed texts live here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostRemediationText {
    /// `gsuite`: Google rejected the configured credentials.
    GoogleBackendAuth,
    /// The shared "restart to apply" follow-up sentence.
    ApplyStep,
}

impl HostRemediationText {
    /// The text this producer emits. Exhaustive by construction.
    pub fn text(self) -> String {
        match self {
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
        const ALL: [HostRemediationText; 2] = [
            HostRemediationText::GoogleBackendAuth,
            HostRemediationText::ApplyStep,
        ];
        for entry in ALL {
            // Exhaustiveness witness — one arm per variant, no catch-all. A new
            // variant breaks THIS match, forcing `ALL` (and the coverage test
            // that reads it) to be updated.
            match entry {
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
}
