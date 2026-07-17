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
    LlmCredentialProvisionOutcome, LocalDevLlmKeyStoreOpener, provision_llm_credentials,
};
use master_key::{MasterKeyProvisionOutcome, provision_master_key};
use prompts::{LlmCredentialPromptError, PromptSource, StdinPromptSource};

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
        // Independent of `--force`: a valid existing token is never
        // regenerated (see `ensure_webui_token_file`'s doc for why), so a
        // repeated `onboard --force` cannot invalidate sessions or an
        // operator-copied env var keyed to the current token value.
        let webui_token_action = crate::webui_token::ensure_webui_token_file(home.path())?;
        let master_key_outcome = provision_master_key(home)?;
        let mut prompts = StdinPromptSource;
        let llm_outcome = match provision_llm_credentials(
            home,
            context.boot_config(),
            &mut prompts,
            &LocalDevLlmKeyStoreOpener,
            self.force,
        ) {
            Ok(outcome) => outcome,
            // A non-interactive session (headless CI, a piped/scripted
            // invocation) is expected and normal — mirrors
            // `MasterKeyProvisionOutcome::Suppressed` above: onboarding must
            // not fail just because there is no terminal to prompt on.
            // `ironclaw-reborn models set-provider` remains the
            // non-interactive path to configure a provider afterward.
            Err(LlmCredentialPromptError::NonInteractive) => {
                LlmCredentialProvisionOutcome::SkippedNonInteractive
            }
            Err(LlmCredentialPromptError::Other(error)) => return Err(error),
        };
        // Computed after `llm_outcome` (not before, as in an earlier
        // revision) so the marker's `steps_pending` reflects what actually
        // happened this run rather than unconditionally listing
        // `llm_credentials` as pending even when it was just configured.
        let llm_configured = matches!(
            llm_outcome,
            LlmCredentialProvisionOutcome::Configured { .. }
                | LlmCredentialProvisionOutcome::AlreadyConfigured { .. }
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
        println!();
        println!("remaining:");
        if llm_configured {
            println!("- none for LLM credentials (configured above)");
        } else {
            println!(
                "- configure LLM credentials: rerun `ironclaw-reborn onboard` from an \
                 interactive terminal, or run \
                 `ironclaw-reborn models set-provider <provider> --model <model>` directly"
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

    /// Onboarding's last two steps, gated behind `webui-v2-beta` since both
    /// depend on `serve`/`service` (only compiled under that feature):
    /// install-and-start the OS service (skippable — see
    /// [`Self::should_install_service`]), then print the CLI-token login
    /// link `serve` will accept once it's running, using the same
    /// `webui-token` value `ensure_webui_token_file` already provisioned
    /// above.
    ///
    /// `interactive` is the same [`PromptSource::is_interactive`] reading the
    /// LLM-credential prompt step already made — passed down rather than
    /// re-derived, so `should_install_service` doesn't need its own
    /// `IsTerminal` check (see that method's doc).
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

        // `.ok().flatten()`, not `?`: this step only needs config.toml to
        // read `[webui].env_token_var` for the login-link-vs-note decision
        // below, a purely informational courtesy. A config.toml that fails
        // to parse (or predates this repo's schema — legacy/custom content
        // is preserved as-is by `write_default_config_files` above) must
        // not abort an otherwise-successful onboarding run; falling back to
        // the default env var name is a fine degradation here, and `serve`
        // itself is still the authority that fails closed on real config
        // errors when it boots. Mirrors `status`'s own resolver, which
        // swallows the same load failure for the same reason.
        let config_file = ironclaw_reborn_config::RebornConfigFile::load(&home.config_file_path())
            .ok()
            .flatten();
        match crate::webui_token::resolve_login_link_announcement(home, config_file.as_ref()) {
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

    /// `true` when onboarding should attempt to install/start the OS
    /// service: the operator didn't pass `--no-service`, AND this is an
    /// interactive session. A non-interactive session (headless CI, a
    /// piped/scripted invocation) must never attempt a launchd/systemd
    /// install regardless of the flag — mirrors the LLM-credential prompt's
    /// own non-interactive short-circuit above.
    ///
    /// `interactive` comes from the same [`PromptSource::is_interactive`]
    /// reading used to gate the LLM-credential prompts
    /// (`prompts::StdinPromptSource` is the sole `IsTerminal` check in this
    /// command), rather than this method independently re-deriving "is this
    /// session interactive" via its own `stdin().is_terminal()` call.
    #[cfg(feature = "webui-v2-beta")]
    fn should_install_service(&self, interactive: bool) -> bool {
        !self.no_service && interactive
    }
}

/// Outcome of onboard's OS-service install/start finale. Every variant is a
/// successful `execute()` (exit 0) except `Failed`, which is reported but
/// does not fail onboarding overall — the operator can always install/start
/// the service manually afterward (see the printed `service_note`), so a
/// service-manager hiccup should not make an otherwise-successful onboarding
/// run exit non-zero.
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
    // Propagates rather than defaulting to "would_write" on an I/O error:
    // an unreadable-but-present token file must be reported as an error,
    // not silently promised a (destructive) overwrite that wouldn't
    // actually happen the same way on a real run.
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
    use std::sync::Arc;

    use super::*;

    /// RED (B4 step 6 truth-up): the marker's `steps_pending` must only list
    /// `llm_credentials` when this run did NOT actually configure it —
    /// before this fix `pending_steps` listed it unconditionally, even right
    /// after a successful interactive `provision_llm_credentials` call.
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
}
