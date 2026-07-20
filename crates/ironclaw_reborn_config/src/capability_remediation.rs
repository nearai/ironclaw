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
//! serve base URL. `slack_connect_clause` is the single source of truth both
//! call through, so the wording cannot drift between the two surfaces.
//!
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

/// Which Slack instance-configuration steps a build found missing. Slack
/// personal OAuth needs BOTH: the extension route must be enabled (so the
/// WebUI can serve the connect card) and the redirect URI must be set (so the
/// personal-OAuth slot is filled). Setting only the first leaves
/// `slack_personal_oauth_credentials` returning a message-less 503 — the exact
/// dead end this remediation exists to prevent — so the two travel together as
/// one named struct rather than as a single "slack configured" bool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlackSetupGaps {
    /// `ironclaw config set slack.enabled true` has not been applied.
    pub enable: bool,
    /// `IRONCLAW_REBORN_SLACK_PERSONAL_OAUTH_REDIRECT_URI` is not set.
    pub redirect_uri: bool,
}

/// The service-environment step that fills the personal-OAuth slot. Named
/// once so the composition and CLI variants cannot drift on the variable name.
const SLACK_REDIRECT_URI_STEP: &str = "set IRONCLAW_REBORN_SLACK_PERSONAL_OAUTH_REDIRECT_URI=<your Slack app's redirect URL> \
     in the service environment";

/// BYO console-steps remediation text for Slack, base-url-free: the
/// composition-time build cannot know the serve base URL (it is a
/// per-invocation `serve` flag, resolved later), so this variant names the
/// route relatively. Consumed by
/// `ironclaw_reborn_composition::extension_host::provider_instance_readiness`.
///
/// Only the steps `gaps` reports missing are listed, so a user who already ran
/// `config set slack.enabled true` and is stuck on the redirect URI is not
/// told to re-run a command they have already applied.
///
/// The no-gap input (`{ enable: false, redirect_uri: false }`) is DEFINED but
/// never produced: it yields just the restart and connect steps. Production
/// guards it at
/// `ironclaw_reborn_composition::extension_host::provider_instance_readiness`,
/// which only calls this fn when at least one gap is open. Pinned by
/// `slack_remediation_text_with_no_gaps_lists_only_restart_and_connect`.
///
/// Unlike `google_remediation_text`, this variant embeds its own restart step:
/// Slack's apply step sits in the MIDDLE of the sequence (the route must mount
/// before the WebUI can run workspace OAuth), so a trailing
/// `apply_step_text()` would both misorder the instructions and imply "then
/// ask again" when the user still has a connect step left. Callers of this
/// variant must therefore NOT append `apply_step_text()`.
pub fn slack_remediation_text(gaps: SlackSetupGaps) -> String {
    let mut steps: Vec<String> = Vec::new();
    if gaps.enable {
        steps.push("ironclaw config set slack.enabled true".to_string());
    }
    if gaps.redirect_uri {
        steps.push(SLACK_REDIRECT_URI_STEP.to_string());
    }
    steps.push("ironclaw service restart   (mounts the Slack extension route)".to_string());
    steps.push(slack_connect_clause("/extensions in the WebUI"));
    let body = steps
        .iter()
        .enumerate()
        .map(|(index, step)| format!("  {}. {step}", index + 1))
        .collect::<Vec<_>>()
        .join("\n");
    format!("Slack setup (one-time, per instance):\n{body}")
}

/// Same sentence, with the concrete serve base URL the CLI resolves at
/// `config set` time. Consumed by
/// `ironclaw_reborn_cli::commands::config::capability_config::slack_remediation_text`.
/// The CLI prints this immediately after the user ran `config set
/// slack.enabled`, so it neither repeats that command nor embeds the restart
/// (`set.rs::print_apply_step` appends the canonical restart sentence right
/// after it) — it names only the steps that remain. The redirect URI is one of
/// them unconditionally: `config set slack.enabled` alone provably leaves the
/// personal-OAuth slot empty, and at `config set` time the CLI has not
/// resolved the service environment the runtime will actually boot with, so it
/// states the requirement rather than guessing it is already met.
pub fn slack_remediation_text_with_base_url(base_url: &str) -> String {
    format!(
        "Also {SLACK_REDIRECT_URI_STEP}. After restarting, {}",
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
/// `reason` string (`ProductWorkflowError::ProviderInstanceNotConfigured` and
/// `InvalidBindingRequest`); those reasons are themselves assembled from the
/// constants below, and the composed result is covered at the integration tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostRemediationText {
    /// `gsuite`: no Google OAuth backend configured on this instance.
    GoogleNotConfigured,
    /// `gsuite`: Google rejected the configured credentials.
    GoogleBackendAuth,
    /// `provider_instance_readiness`: Slack instance setup, both gaps open.
    SlackBothGaps,
    /// `provider_instance_readiness`: Slack instance setup, enable step only.
    SlackEnableOnly,
    /// `provider_instance_readiness`: Slack instance setup, redirect URI only.
    SlackRedirectUriOnly,
    /// The shared "restart to apply" follow-up sentence.
    ApplyStep,
}

impl HostRemediationText {
    /// The text this producer emits. Exhaustive by construction.
    pub fn text(self) -> String {
        match self {
            Self::GoogleNotConfigured => google_not_configured_text(),
            Self::GoogleBackendAuth => google_backend_auth_text(),
            Self::SlackBothGaps => slack_remediation_text(SlackSetupGaps {
                enable: true,
                redirect_uri: true,
            }),
            Self::SlackEnableOnly => slack_remediation_text(SlackSetupGaps {
                enable: true,
                redirect_uri: false,
            }),
            Self::SlackRedirectUriOnly => slack_remediation_text(SlackSetupGaps {
                enable: false,
                redirect_uri: true,
            }),
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
        const ALL: [HostRemediationText; 6] = [
            HostRemediationText::GoogleNotConfigured,
            HostRemediationText::GoogleBackendAuth,
            HostRemediationText::SlackBothGaps,
            HostRemediationText::SlackEnableOnly,
            HostRemediationText::SlackRedirectUriOnly,
            HostRemediationText::ApplyStep,
        ];
        for entry in ALL {
            // Exhaustiveness witness — one arm per variant, no catch-all. A new
            // variant breaks THIS match, forcing `ALL` (and the coverage test
            // that reads it) to be updated.
            match entry {
                Self::GoogleNotConfigured => {}
                Self::GoogleBackendAuth => {}
                Self::SlackBothGaps => {}
                Self::SlackEnableOnly => {}
                Self::SlackRedirectUriOnly => {}
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

    /// Pins the behavior of a `SlackSetupGaps` with NO gaps open. Production
    /// never constructs this input — `provider_instance_readiness` guards the
    /// call with `if slack_gaps.enable || slack_gaps.redirect_uri` — but the
    /// combination is reachable through this public API, so its output is
    /// pinned rather than left undefined.
    #[test]
    fn slack_remediation_text_with_no_gaps_lists_only_restart_and_connect() {
        let text = slack_remediation_text(SlackSetupGaps {
            enable: false,
            redirect_uri: false,
        });
        assert!(!text.contains("config set slack.enabled"));
        assert!(!text.contains("REDIRECT_URI"));
        assert_eq!(text.matches("service restart").count(), 1);
        assert!(text.contains("/extensions"));
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

    fn both_gaps() -> SlackSetupGaps {
        SlackSetupGaps {
            enable: true,
            redirect_uri: true,
        }
    }

    #[test]
    fn slack_remediation_text_names_the_relative_extensions_route() {
        let slack = slack_remediation_text(both_gaps());
        assert!(slack.contains("/extensions"));
        assert!(slack.contains("config set slack.enabled"));
        assert!(slack.contains("IRONCLAW_REBORN_SLACK_PERSONAL_OAUTH_REDIRECT_URI"));
        assert!(!slack.contains("config set slack.bot_token"));
        // The user asked for `config set` to lead: an operator reading top-down
        // must enable the route before being sent to the WebUI to connect.
        let config_set = slack
            .find("config set slack.enabled")
            .expect("config set step present");
        let connect = slack.find("/extensions").expect("connect step present");
        assert!(
            config_set < connect,
            "`config set` must precede the WebUI connect instruction: {slack}"
        );
        // This variant OWNS its restart (Slack's apply step is mid-sequence),
        // so it embeds exactly one and its callers append none.
        assert_eq!(
            slack.matches("service restart").count(),
            1,
            "the composition variant embeds its own mid-sequence restart exactly once: {slack}"
        );
    }

    /// The dead-end case Task A closes: `slack.enabled` is already applied and
    /// only the redirect URI is missing. Re-printing the `config set` the user
    /// has already run is the specific confusion the gap struct prevents.
    #[test]
    fn slack_remediation_text_names_only_the_missing_step() {
        let redirect_only = slack_remediation_text(SlackSetupGaps {
            enable: false,
            redirect_uri: true,
        });
        assert!(redirect_only.contains("IRONCLAW_REBORN_SLACK_PERSONAL_OAUTH_REDIRECT_URI"));
        assert!(
            !redirect_only.contains("config set slack.enabled"),
            "an already-enabled instance must not be told to re-run config set: {redirect_only}"
        );
        assert!(
            redirect_only.contains("1. set IRONCLAW_REBORN_SLACK_PERSONAL_OAUTH_REDIRECT_URI"),
            "the surviving steps must renumber from 1: {redirect_only}"
        );

        let enable_only = slack_remediation_text(SlackSetupGaps {
            enable: true,
            redirect_uri: false,
        });
        assert!(enable_only.contains("config set slack.enabled"));
        assert!(
            !enable_only.contains("IRONCLAW_REBORN_SLACK_PERSONAL_OAUTH_REDIRECT_URI"),
            "a configured redirect URI must not be listed as missing: {enable_only}"
        );
    }

    #[test]
    fn slack_remediation_text_with_base_url_embeds_the_concrete_url() {
        let slack = slack_remediation_text_with_base_url("http://127.0.0.1:3000");
        assert!(slack.contains("http://127.0.0.1:3000/extensions"));
        assert!(slack.contains("IRONCLAW_REBORN_SLACK_PERSONAL_OAUTH_REDIRECT_URI"));
        // The CLI prints this right after `config set slack.enabled` succeeded,
        // so repeating that command would be noise.
        assert!(
            !slack.contains("config set slack.enabled"),
            "the CLI variant must not repeat the command the user just ran: {slack}"
        );
        assert_eq!(
            slack.matches("service restart").count(),
            0,
            "slack_remediation_text_with_base_url must not embed the restart step itself \
             (`set.rs::print_apply_step` appends it exactly once): {slack}"
        );
    }
}
