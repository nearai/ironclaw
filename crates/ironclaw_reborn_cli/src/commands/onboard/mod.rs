use std::path::{Path, PathBuf};

use clap::Args;
use ironclaw_reborn_config::RebornHome;

use crate::commands::config::init::{ExistingConfigPolicy, write_default_config_files};
use crate::context::RebornCliContext;
use crate::file_write::{FileWriteAction, write_atomic};

mod llm_credentials;
mod master_key;
mod prompts;

use llm_credentials::{
    EncryptedLlmKeyStoreOpener, LiveLlmProbe, LlmCredentialProvisionOutcome,
    provision_llm_credentials,
};
use master_key::{MasterKeyProvisionOutcome, provision_master_key};
#[cfg(feature = "webui-v2-beta")]
use prompts::PromptSource;
use prompts::{LlmCredentialPromptError, StdinPromptSource};

const ONBOARDING_MARKER_FILE: &str = ".onboard-completed.json";

/// Initialize the standalone Reborn home and first-run setup marker.
#[derive(Debug, Args)]
pub(crate) struct OnboardCommand {
    /// Overwrite generated config.toml, providers.json, and the completion marker.
    #[arg(long = "force")]
    force: bool,

    /// Show what would be initialized without writing files.
    #[arg(long = "dry-run")]
    dry_run: bool,

    /// Reserve the history-import step in the onboarding summary.
    ///
    /// History import is not wired in this slice; the flag makes the missing
    /// step explicit without touching v1 setup/import state.
    #[arg(long = "import-history")]
    import_history: bool,

    /// Skip installing/starting the OS service (launchd/systemd) at the end
    /// of onboarding. Always effectively on in a non-interactive session
    /// (headless CI, a piped/scripted invocation) regardless of this flag —
    /// see `execute()`'s service step.
    #[cfg(feature = "webui-v2-beta")]
    #[arg(long = "no-service")]
    no_service: bool,
}

impl OnboardCommand {
    pub(crate) fn execute(self, context: RebornCliContext) -> anyhow::Result<()> {
        let home = context.boot_config().home();
        let marker_path = onboarding_marker_path(home);

        if self.dry_run {
            print_dry_run(home, &marker_path, self.force, self.import_history)?;
            return Ok(());
        }

        let outcome = write_default_config_files(home, self.force, ExistingConfigPolicy::Preserve)?;
        // Independent of `--force`: a valid existing token is never regenerated
        // (see `ensure_webui_token_file`), so repeated `onboard --force` can't
        // invalidate sessions or an operator-copied env var.
        let webui_token_action = crate::webui_token::ensure_webui_token_file(home.path())?;
        let master_key_outcome = provision_master_key(context.boot_config())?;
        let mut prompts = StdinPromptSource;
        let llm_outcome = match provision_llm_credentials(
            home,
            context.boot_config(),
            &mut prompts,
            &EncryptedLlmKeyStoreOpener,
            &LiveLlmProbe,
            self.force,
        ) {
            Ok(outcome) => outcome,
            // Non-interactive session (headless CI, piped/scripted) is expected —
            // mirrors `MasterKeyProvisionOutcome::Suppressed`; `models set-provider`
            // remains the non-interactive path to configure a provider.
            Err(LlmCredentialPromptError::NonInteractive) => {
                LlmCredentialProvisionOutcome::SkippedNonInteractive
            }
            Err(LlmCredentialPromptError::Other(error)) => return Err(error),
        };
        // Computed after `llm_outcome` so `steps_pending` reflects what actually
        // happened this run, not an unconditional `llm_credentials` pending.
        let llm_configured = matches!(
            llm_outcome,
            LlmCredentialProvisionOutcome::Configured { .. }
                | LlmCredentialProvisionOutcome::AlreadyConfigured { .. }
                | LlmCredentialProvisionOutcome::ConfiguredFromEnv { .. }
        );
        let marker_action = write_onboarding_marker(
            home,
            &marker_path,
            self.force,
            self.import_history,
            llm_configured,
        )?;

        println!("IronClaw Reborn onboarding");
        println!("reborn_home: {}", home.path().display());
        println!("home_source: {}", home.source_label());
        println!("{}", outcome.config.display_line());
        println!("{}", outcome.providers.display_line());
        println!(
            "webui_token: {} ({})",
            crate::webui_token::webui_token_file_path(home.path()).display(),
            webui_token_action
        );
        println!(
            "onboarding_marker: {} ({})",
            marker_path.display(),
            marker_action
        );
        println!("master_key: {}", master_key_outcome.display_line());
        if let MasterKeyProvisionOutcome::Suppressed = master_key_outcome {
            println!(
                "master_key_note: OS keychain unavailable; set SECRETS_MASTER_KEY yourself or \
                 let the first `serve`/`onboard` run auto-generate and cache \
                 .reborn-local-dev-secrets-master-key in the Reborn home"
            );
        }
        println!("llm_credentials: {}", llm_outcome.display_line());
        println!("v1_state: not-used");
        println!();
        println!("completed:");
        println!("- reborn home initialized");
        println!("- config.toml and providers.json available");
        println!("- webui bearer token provisioned (used by `serve` when the env var is unset)");
        println!("- onboarding completion marker available");
        if let LlmCredentialProvisionOutcome::Configured { provider_id, .. }
        | LlmCredentialProvisionOutcome::AlreadyConfigured { provider_id, .. } = &llm_outcome
        {
            println!("- LLM provider `{provider_id}` credentials stored");
        }
        if let LlmCredentialProvisionOutcome::ConfiguredFromEnv { provider_id, .. } = &llm_outcome {
            println!(
                "- LLM provider `{provider_id}` configured from environment (key also saved to \
                 the encrypted secret store so the background service can use it)"
            );
        }
        println!();
        println!("remaining:");
        if llm_configured {
            println!("- none for LLM credentials (configured above)");
        } else {
            println!(
                "- configure LLM credentials: rerun `ironclaw-reborn onboard` from an \
                 interactive terminal, run \
                 `ironclaw-reborn models set-provider <provider> --model <model>` directly, or \
                 export a provider's LLM environment variables (e.g. `LLM_BACKEND` or \
                 `OPENAI_API_KEY`; see `.env.example`) before the next `onboard`/`serve`"
            );
        }
        if self.import_history {
            println!("- history import requested but not wired yet");
        } else {
            println!("- history import not requested");
        }

        #[cfg(feature = "webui-v2-beta")]
        self.finish_with_service_and_login_link(&context, home, prompts.is_interactive())?;

        Ok(())
    }

    /// Onboarding's last two steps, gated behind `webui-v2-beta` (both depend
    /// on `serve`/`service`):
    /// - install-and-start the OS service (skippable, see [`Self::should_install_service`])
    /// - print the CLI-token login link, reusing the `webui-token` value
    ///   `ensure_webui_token_file` already provisioned above
    ///
    /// `interactive` is passed down from the same [`PromptSource::is_interactive`]
    /// reading the LLM-credential prompt step made, so `should_install_service`
    /// doesn't need its own `IsTerminal` check.
    #[cfg(feature = "webui-v2-beta")]
    fn finish_with_service_and_login_link(
        &self,
        context: &RebornCliContext,
        home: &RebornHome,
        interactive: bool,
    ) -> anyhow::Result<()> {
        let service_outcome = if self.should_install_service(interactive) {
            match crate::commands::service::install_and_start(context) {
                Ok(()) => ServiceStartOutcome::InstalledAndStarted,
                Err(error) => ServiceStartOutcome::Failed(error.to_string()),
            }
        } else if self.no_service {
            ServiceStartOutcome::SkippedFlag
        } else {
            ServiceStartOutcome::SkippedNonInteractive
        };
        println!("service: {}", service_outcome.display_line());
        if let ServiceStartOutcome::Failed(reason) = &service_outcome {
            println!(
                "service_note: install/start failed ({reason}); run `ironclaw-reborn service \
                 install` and `ironclaw-reborn service start` manually"
            );
        }

        // `.ok().flatten()`, not `?`: only needed to read `[webui].env_token_var`
        // for the login-link-vs-note decision below. `serve` remains the
        // authority that fails closed on real config errors at boot; mirrors
        // `status`'s own resolver swallowing the same load failure.
        // silent-ok: a config.toml that fails to parse (or predates this
        // repo's schema) must not abort an otherwise-successful onboarding
        // run; falling back to the default env var name is a fine
        // degradation for this purely informational courtesy.
        let config_file = ironclaw_reborn_config::RebornConfigFile::load(&home.config_file_path())
            .ok()
            .flatten();
        match crate::webui_token::resolve_login_link_announcement(home, config_file.as_ref())? {
            crate::webui_token::LoginLinkAnnouncement::Link(login_link) => {
                println!("login_link: {login_link}");
            }
            crate::webui_token::LoginLinkAnnouncement::EnvTokenActive { env_var_name } => {
                println!(
                    "login_note: {env_var_name} is set; `serve` authenticates with that env \
                     token directly (no login link — the CLI-token login route only mounts for \
                     a file-sourced token)"
                );
            }
            crate::webui_token::LoginLinkAnnouncement::Unavailable => {}
        }
        println!("hint: add Gmail or Slack any time: ironclaw-reborn config set --help");
        Ok(())
    }

    /// `true` when onboarding should install/start the OS service:
    /// `--no-service` unset AND session is interactive. Non-interactive
    /// (headless CI, piped/scripted) must never attempt a launchd/systemd
    /// install regardless of the flag — mirrors the LLM-credential prompt's
    /// non-interactive short-circuit.
    ///
    /// `interactive` comes from the same [`PromptSource::is_interactive`]
    /// reading used to gate the LLM-credential prompts (`prompts::StdinPromptSource`
    /// is the sole `IsTerminal` check in this command) rather than re-deriving it here.
    #[cfg(feature = "webui-v2-beta")]
    fn should_install_service(&self, interactive: bool) -> bool {
        !self.no_service && interactive
    }
}

/// Outcome of onboard's OS-service install/start finale. Even `Failed` is a
/// successful `execute()` (exit 0) — reported via `service_note`, but a
/// service-manager hiccup must not fail an otherwise-successful onboarding run.
#[cfg(feature = "webui-v2-beta")]
#[derive(Debug, Clone, PartialEq, Eq)]
enum ServiceStartOutcome {
    InstalledAndStarted,
    SkippedFlag,
    SkippedNonInteractive,
    Failed(String),
}

#[cfg(feature = "webui-v2-beta")]
impl ServiceStartOutcome {
    fn display_line(&self) -> String {
        match self {
            Self::InstalledAndStarted => "installed and started".to_string(),
            Self::SkippedFlag => "skipped (--no-service)".to_string(),
            Self::SkippedNonInteractive => "skipped (non-interactive session)".to_string(),
            Self::Failed(reason) => format!("failed: {reason}"),
        }
    }
}

pub(crate) fn onboarding_marker_path(home: &RebornHome) -> PathBuf {
    home.path().join(ONBOARDING_MARKER_FILE)
}

fn print_dry_run(
    home: &RebornHome,
    marker_path: &Path,
    force: bool,
    import_history: bool,
) -> anyhow::Result<()> {
    println!("IronClaw Reborn onboarding dry run");
    println!("reborn_home: {}", home.path().display());
    println!("home_source: {}", home.source_label());
    println!("would_ensure: {}", home.path().display());
    println!(
        "would_write_or_preserve: {}",
        home.config_file_path().display()
    );
    println!(
        "would_write_or_preserve: {}",
        home.providers_file_path().display()
    );
    // Propagates rather than defaulting to "would_write" on I/O error: an
    // unreadable-but-present token file must error, not be silently promised
    // an overwrite that wouldn't happen the same way on a real run.
    let webui_token_action = if crate::webui_token::webui_token_file_is_valid(home.path())? {
        "would_preserve"
    } else {
        "would_write"
    };
    println!(
        "{webui_token_action}: {}",
        crate::webui_token::webui_token_file_path(home.path()).display()
    );
    let marker_action = if marker_path.exists() && !force {
        "would_preserve"
    } else {
        "would_write"
    };
    println!("{marker_action}: {}", marker_path.display());
    println!("import_history_requested: {import_history}");
    println!("v1_state: not-used");
    Ok(())
}

fn write_onboarding_marker(
    home: &RebornHome,
    marker_path: &Path,
    force: bool,
    import_history: bool,
    llm_configured: bool,
) -> anyhow::Result<FileWriteAction> {
    if marker_path.exists() && !force {
        return Ok(FileWriteAction::Preserved);
    }
    let body = serde_json::to_string_pretty(&serde_json::json!({
        "schema_version": "ironclaw.reborn.onboarding/v1",
        "completed_at": chrono::Utc::now().to_rfc3339(),
        "reborn_home": home.path(),
        "home_source": home.source_label(),
        "config_file": home.config_file_path(),
        "providers_file": home.providers_file_path(),
        "webui_token_file": crate::webui_token::webui_token_file_path(home.path()),
        "steps_completed": [
            "reborn_home",
            "config_files",
            "webui_token",
            "completion_marker"
        ],
        "steps_pending": pending_steps(import_history, llm_configured),
        "v1_state": "not-used"
    }))?;
    write_atomic(
        marker_path,
        &format!("{body}\n"),
        force,
        ONBOARDING_MARKER_FILE,
    )
}

/// `llm_credentials` is only reported pending when this run did NOT
/// configure it — an interactive `provision_llm_credentials` success means
/// there is nothing left to do for that step, so the marker must not
/// unconditionally claim it is still outstanding.
fn pending_steps(import_history: bool, llm_configured: bool) -> Vec<&'static str> {
    let mut steps = Vec::new();
    if !llm_configured {
        steps.push("llm_credentials");
    }
    steps.push("model_selection");
    steps.push("channel_setup");
    if import_history {
        steps.push("history_import");
    }
    steps
}

#[cfg(all(test, feature = "libsql", feature = "root-llm-provider"))]
mod tests {
    use super::*;

    /// The marker's `steps_pending` must only list `llm_credentials` when
    /// this run did NOT actually configure it.
    #[test]
    fn pending_steps_omits_llm_credentials_once_configured() {
        assert_eq!(
            pending_steps(false, true),
            vec!["model_selection", "channel_setup"],
            "llm_credentials must not be reported pending once this run configured it"
        );
        assert_eq!(
            pending_steps(false, false),
            vec!["llm_credentials", "model_selection", "channel_setup"],
            "llm_credentials must still be reported pending when this run did not configure it"
        );
        assert_eq!(
            pending_steps(true, true),
            vec!["model_selection", "channel_setup", "history_import"],
            "import_history must still append history_import regardless of llm_configured"
        );
    }

    /// Same guarantee as `pending_steps_omits_llm_credentials_once_configured`,
    /// but driven through `write_onboarding_marker` (the production caller) —
    /// parses the actual `.onboard-completed.json` `steps_pending` field
    /// rather than calling `pending_steps` directly.
    #[test]
    fn write_onboarding_marker_steps_pending_reflects_llm_configured() {
        let (_tmp, context) = crate::context::RebornCliContext::test_context();
        let home = context.boot_config().home();
        std::fs::create_dir_all(home.path()).expect("create reborn home");

        let marker_path = home.path().join("configured.json");
        write_onboarding_marker(home, &marker_path, false, false, true)
            .expect("write marker (llm configured)");
        let marker: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&marker_path).expect("read marker"))
                .expect("marker must be valid JSON");
        assert_eq!(
            marker["steps_pending"],
            serde_json::json!(["model_selection", "channel_setup"]),
            "steps_pending must omit llm_credentials once configured: {marker}"
        );

        let marker_path = home.path().join("unconfigured.json");
        write_onboarding_marker(home, &marker_path, false, false, false)
            .expect("write marker (llm not configured)");
        let marker: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&marker_path).expect("read marker"))
                .expect("marker must be valid JSON");
        assert_eq!(
            marker["steps_pending"],
            serde_json::json!(["llm_credentials", "model_selection", "channel_setup"]),
            "steps_pending must include llm_credentials when not configured: {marker}"
        );
    }
}
