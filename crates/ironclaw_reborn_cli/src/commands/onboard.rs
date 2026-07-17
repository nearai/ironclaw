use std::io::{IsTerminal, Write as _};
use std::path::{Path, PathBuf};

use clap::Args;
use ironclaw_reborn_config::RebornHome;

use crate::commands::config::init::{ExistingConfigPolicy, write_default_config_files};
use crate::context::RebornCliContext;
use crate::file_write::{FileWriteAction, write_atomic};

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
        let llm_outcome =
            match provision_llm_credentials(home, context.boot_config(), &mut StdinPromptSource) {
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
        if let LlmCredentialProvisionOutcome::Configured { provider_id, .. } = &llm_outcome {
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
        self.finish_with_service_and_login_link(&context, home)?;

        Ok(())
    }

    /// Onboarding's last two steps, gated behind `webui-v2-beta` since both
    /// depend on `serve`/`service` (only compiled under that feature):
    /// install-and-start the OS service (skippable — see
    /// [`Self::should_install_service`]), then print the CLI-token login
    /// link `serve` will accept once it's running, using the same
    /// `webui-token` value `ensure_webui_token_file` already provisioned
    /// above.
    #[cfg(feature = "webui-v2-beta")]
    fn finish_with_service_and_login_link(
        &self,
        context: &RebornCliContext,
        home: &RebornHome,
    ) -> anyhow::Result<()> {
        let service_outcome = if self.should_install_service() {
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

        if let Some(login_link) = login_link(home) {
            println!("login_link: {login_link}");
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
    #[cfg(feature = "webui-v2-beta")]
    fn should_install_service(&self) -> bool {
        !self.no_service && std::io::stdin().is_terminal()
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

/// The CLI-printed bootstrap link into the browser session (see
/// `ironclaw_reborn_webui_ingress::cli_token_login`'s module doc for the
/// flow it plugs into) — `Some` only when a valid `webui-token` file is
/// present (it always is right after `ensure_webui_token_file` runs above,
/// but this is also called from contexts, like `status`, where onboarding
/// may not have run). Uses `serve`'s own default host:port constants rather
/// than duplicating the literals — see their doc comment in `commands/serve.rs`.
#[cfg(feature = "webui-v2-beta")]
pub(crate) fn login_link(home: &RebornHome) -> Option<String> {
    if !crate::webui_token::webui_token_file_is_valid(home.path()) {
        return None;
    }
    let token =
        std::fs::read_to_string(crate::webui_token::webui_token_file_path(home.path())).ok()?;
    Some(format!(
        "http://{}:{}/login?token={}",
        crate::commands::serve::DEFAULT_SERVE_HOST,
        crate::commands::serve::DEFAULT_SERVE_PORT,
        token.trim()
    ))
}

pub(crate) fn onboarding_marker_path(home: &RebornHome) -> PathBuf {
    home.path().join(ONBOARDING_MARKER_FILE)
}

/// Outcome of onboarding's OS-keychain master-key provisioning attempt.
///
/// Every variant is a successful `execute()` (exit 0) — this is a status
/// enum, not an error type. `Suppressed` is expected and normal on headless
/// Linux/CI (`IRONCLAW_DISABLE_OS_KEYCHAIN`) or when the OS denies the
/// keychain prompt: the resolver chain
/// (`ironclaw_reborn_composition::factory::resolve_local_dev_secret_master_key_with_env`)
/// still has the dotfile auto-generation fallback, so onboarding must not
/// fail just because the keychain step didn't provision anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MasterKeyProvisionOutcome {
    /// A cached `.reborn-local-dev-secrets-master-key` dotfile already
    /// exists under this Reborn home; nothing to provision.
    DotfileAlreadyPresent,
    /// The OS keychain already has a master key from a prior onboarding run.
    KeychainAlreadyPresent,
    /// A fresh key was generated and stored in the OS keychain.
    Provisioned,
    /// The OS keychain is unavailable (suppressed under test/CI, or the OS
    /// denied the write). `serve`/`onboard` still work: the resolver falls
    /// through to the `SECRETS_MASTER_KEY` env var, then to auto-generating
    /// and caching the dotfile on first boot.
    Suppressed,
}

impl MasterKeyProvisionOutcome {
    fn display_line(self) -> &'static str {
        match self {
            Self::DotfileAlreadyPresent => "cached dotfile already present",
            Self::KeychainAlreadyPresent => "already provisioned in OS keychain",
            Self::Provisioned => "provisioned in OS keychain",
            Self::Suppressed => "OS keychain unavailable; falling back to env/dotfile",
        }
    }
}

/// Provision a local-dev secrets master key in the OS keychain on a fresh
/// desktop: if there is no cached dotfile AND the keychain has no key,
/// generate one and store it. A second run (dotfile or keychain already
/// populated) is a no-op. Never fails `execute()` — an unavailable/denied
/// keychain is reported via [`MasterKeyProvisionOutcome::Suppressed`] and
/// onboarding continues, matching the resolver's own env/dotfile fallback
/// (`crates/ironclaw_reborn_composition/src/factory.rs`).
#[cfg(any(feature = "libsql", feature = "postgres"))]
fn provision_master_key(home: &RebornHome) -> anyhow::Result<MasterKeyProvisionOutcome> {
    let dotfile_path = home
        .path()
        .join(ironclaw_reborn_composition::LOCAL_DEV_SECRETS_MASTER_KEY_PATH);
    if dotfile_path.exists() {
        return Ok(MasterKeyProvisionOutcome::DotfileAlreadyPresent);
    }

    crate::runtime::block_on_cli(async move {
        if ironclaw_secrets::keychain::has_master_key().await {
            return Ok::<_, anyhow::Error>(MasterKeyProvisionOutcome::KeychainAlreadyPresent);
        }
        let key = ironclaw_secrets::keychain::generate_master_key();
        match ironclaw_secrets::keychain::store_master_key(&key).await {
            Ok(()) => Ok(MasterKeyProvisionOutcome::Provisioned),
            Err(_) => Ok(MasterKeyProvisionOutcome::Suppressed),
        }
    })
}

/// Without a storage backend feature there is no secret store to provision a
/// master key for at all — the master-key resolver lives behind the same
/// `libsql`/`postgres` feature gate in `ironclaw_reborn_composition`.
#[cfg(not(any(feature = "libsql", feature = "postgres")))]
fn provision_master_key(_home: &RebornHome) -> anyhow::Result<MasterKeyProvisionOutcome> {
    Ok(MasterKeyProvisionOutcome::Suppressed)
}

/// Default LLM provider offered by the onboarding prompt. `nearai` requires
/// no upfront third-party account (session-token setup via a NEAR account),
/// which is why it's the zero-friction default for a fresh desktop install.
const DEFAULT_LLM_PROVIDER: &str = "nearai";

/// Where onboard's two LLM-credential prompts (provider id, API key) come
/// from. Injected so [`provision_llm_credentials`] is testable with a fixed
/// answer sequence, and so [`StdinPromptSource`] is the *only* place that
/// decides "is this session interactive" — matching the injected-lookup
/// convention `resolve_google_oauth_config` already established
/// (`crate::runtime::resolve_google_oauth_config`, which takes a `lookup`
/// closure rather than reading `std::env` inline) and the "only `main.rs`
/// may exit; `execute()` returns typed errors" rule: this trait's methods
/// return [`LlmCredentialPromptError::NonInteractive`] rather than calling
/// `process::exit`.
pub(crate) trait PromptSource {
    /// Prompt for the LLM provider id. `default` is used verbatim when the
    /// operator submits an empty answer.
    fn provider(&mut self, default: &str) -> Result<String, LlmCredentialPromptError>;
    /// Prompt for `provider`'s API key with input masked (not echoed).
    fn api_key(&mut self, provider: &str) -> Result<String, LlmCredentialPromptError>;
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum LlmCredentialPromptError {
    /// stdin is not a terminal (headless CI, a piped/scripted invocation).
    /// Callers should treat this as "skip, don't fail" — see
    /// `OnboardCommand::execute`'s handling next to
    /// `MasterKeyProvisionOutcome::Suppressed`, the same non-fatal shape for
    /// an unavailable interactive input.
    #[error(
        "onboarding LLM credential prompts require an interactive terminal; run \
         `ironclaw-reborn models set-provider <provider>` and set the provider's API key env \
         var instead, or rerun `onboard` from an interactive shell"
    )]
    NonInteractive,
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

/// Outcome of onboard's LLM provider/API-key prompt step. Every variant is a
/// successful `execute()` (exit 0) — mirrors [`MasterKeyProvisionOutcome`]'s
/// shape: `SkippedNonInteractive` is expected and normal, not a failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LlmCredentialProvisionOutcome {
    Configured { provider_id: String, model: String },
    SkippedNonInteractive,
}

impl LlmCredentialProvisionOutcome {
    fn display_line(&self) -> String {
        match self {
            Self::Configured { provider_id, model } => {
                format!("configured provider `{provider_id}` (model `{model}`)")
            }
            Self::SkippedNonInteractive => "skipped (non-interactive session)".to_string(),
        }
    }
}

/// Prompt for an LLM provider + API key and persist both: the provider
/// selection goes to `[llm.default]` in `config.toml` (existing
/// `RebornProviderAdmin::set_provider` config machinery, the same one
/// `ironclaw-reborn models set-provider` uses); the key value goes into the
/// encrypted secret store via the canonical `LlmKeyStore` handle
/// (`llm_provider_<id>_api_key`) — the same handle the webui2 settings
/// surface writes and `apply_startup_stored_llm_key` reads at boot, so no
/// new read-side mapping is needed for the stored key to take effect.
///
/// Gathers both prompt answers before writing anything: a non-interactive
/// `provider()` or `api_key()` failure must leave config.toml and the secret
/// store untouched, not partially written.
#[cfg(all(feature = "libsql", feature = "root-llm-provider"))]
fn provision_llm_credentials(
    home: &RebornHome,
    boot: &ironclaw_reborn_config::RebornBootConfig,
    prompts: &mut dyn PromptSource,
) -> Result<LlmCredentialProvisionOutcome, LlmCredentialPromptError> {
    let provider = prompts.provider(DEFAULT_LLM_PROVIDER)?;
    let key = prompts.api_key(&provider)?;

    let admin = ironclaw_reborn_composition::RebornProviderAdmin::new(boot.clone());
    let write_outcome = admin
        .set_provider(&provider, None)
        .map_err(|error| LlmCredentialPromptError::Other(error.into()))?;

    let home_path = home.path().to_path_buf();
    let provider_id = write_outcome.provider_id.clone();
    crate::runtime::block_on_cli(async move {
        let store = ironclaw_reborn_composition::open_local_dev_secret_store(&home_path)
            .await
            .map_err(anyhow::Error::from)?;
        ironclaw_reborn_composition::LlmKeyStore::new(store)
            .put(&provider_id, ironclaw_secrets::SecretMaterial::from(key))
            .await
            .map_err(anyhow::Error::from)
    })
    .map_err(LlmCredentialPromptError::Other)?;

    Ok(LlmCredentialProvisionOutcome::Configured {
        provider_id: write_outcome.provider_id,
        model: write_outcome.model,
    })
}

/// Without both `libsql` (the store opener) and `root-llm-provider`
/// (`RebornProviderAdmin`/`LlmKeyStore`) the LLM credential step has nothing
/// to write to — same reasoning as `provision_master_key`'s
/// not-any-storage-feature fallback above.
#[cfg(not(all(feature = "libsql", feature = "root-llm-provider")))]
fn provision_llm_credentials(
    _home: &RebornHome,
    _boot: &ironclaw_reborn_config::RebornBootConfig,
    _prompts: &mut dyn PromptSource,
) -> Result<LlmCredentialProvisionOutcome, LlmCredentialPromptError> {
    Ok(LlmCredentialProvisionOutcome::SkippedNonInteractive)
}

/// Production [`PromptSource`]: reads the provider id as a plain line, the
/// API key with terminal echo suppressed. The *only* place in this module
/// that checks [`IsTerminal`] or touches the real terminal — everything
/// else goes through the trait, matching the "only `main.rs` may exit"
/// convention (this impl never calls `process::exit`; it returns
/// [`LlmCredentialPromptError::NonInteractive`] and lets the caller decide).
struct StdinPromptSource;

impl PromptSource for StdinPromptSource {
    fn provider(&mut self, default: &str) -> Result<String, LlmCredentialPromptError> {
        if !std::io::stdin().is_terminal() {
            return Err(LlmCredentialPromptError::NonInteractive);
        }
        print!("LLM provider [{default}]: ");
        std::io::stdout()
            .flush()
            .map_err(|error| LlmCredentialPromptError::Other(error.into()))?;
        let mut input = String::new();
        std::io::stdin()
            .read_line(&mut input)
            .map_err(|error| LlmCredentialPromptError::Other(error.into()))?;
        let trimmed = input.trim();
        Ok(if trimmed.is_empty() {
            default.to_string()
        } else {
            trimmed.to_string()
        })
    }

    fn api_key(&mut self, provider: &str) -> Result<String, LlmCredentialPromptError> {
        if !std::io::stdin().is_terminal() {
            return Err(LlmCredentialPromptError::NonInteractive);
        }
        print!("{provider} API key (input hidden): ");
        std::io::stdout()
            .flush()
            .map_err(|error| LlmCredentialPromptError::Other(error.into()))?;
        let key =
            read_masked_line().map_err(|error| LlmCredentialPromptError::Other(error.into()))?;
        println!();
        Ok(key)
    }
}

/// Read one line with terminal echo suppressed, showing `*` per character.
///
/// Ported from v1's `src/setup/prompts.rs` (`secret_input`/
/// `read_secret_line`) — same crossterm raw-mode key-event loop — per this
/// repo's "porting = copy, never depend" convention (v1 is read for shape,
/// not imported; `ironclaw_secrets::keychain::os_keychain_suppressed` was
/// ported into the Reborn stack the same way for the master-key work this
/// command already does above).
fn read_masked_line() -> std::io::Result<String> {
    use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
    use crossterm::{execute, style::Print, terminal};

    let mut input = String::new();
    terminal::enable_raw_mode()?;
    let result = (|| -> std::io::Result<()> {
        loop {
            if let Event::Key(KeyEvent {
                code,
                modifiers,
                kind: KeyEventKind::Press,
                ..
            }) = event::read()?
            {
                match code {
                    KeyCode::Enter => break,
                    KeyCode::Backspace if !input.is_empty() => {
                        input.pop();
                        execute!(std::io::stdout(), Print("\x08 \x08"))?;
                        std::io::stdout().flush()?;
                    }
                    KeyCode::Backspace => {}
                    KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::Interrupted,
                            "Ctrl-C",
                        ));
                    }
                    KeyCode::Char(c) => {
                        input.push(c);
                        execute!(std::io::stdout(), Print('*'))?;
                        std::io::stdout().flush()?;
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    })();
    terminal::disable_raw_mode()?;
    result?;
    Ok(input)
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

    struct FakePromptSource {
        provider: &'static str,
        key: &'static str,
    }

    impl PromptSource for FakePromptSource {
        fn provider(&mut self, _default: &str) -> Result<String, LlmCredentialPromptError> {
            Ok(self.provider.to_string())
        }

        fn api_key(&mut self, _provider: &str) -> Result<String, LlmCredentialPromptError> {
            Ok(self.key.to_string())
        }
    }

    struct NonInteractivePromptSource;

    impl PromptSource for NonInteractivePromptSource {
        fn provider(&mut self, _default: &str) -> Result<String, LlmCredentialPromptError> {
            Err(LlmCredentialPromptError::NonInteractive)
        }

        fn api_key(&mut self, _provider: &str) -> Result<String, LlmCredentialPromptError> {
            unreachable!("api_key must not be prompted once provider() has already failed")
        }
    }

    /// RED (B2 step 1): a fake interactive `PromptSource` answering
    /// `("nearai", "sk-test-value")` must land the provider selection in
    /// `config.toml` and the key value in the encrypted secret store,
    /// readable back through a *fresh* open of the same root — proving the
    /// opener and `LlmKeyStore::put`/`read` agree on physical storage.
    #[test]
    fn provision_llm_credentials_writes_config_and_secret_store_through_fake_prompts() {
        let (_tmp, context) = RebornCliContext::test_context();
        let home = context.boot_config().home();
        std::fs::create_dir_all(home.path()).expect("create reborn home");
        // Seed a cached master-key dotfile so the store opener's resolver
        // never reaches the OS keychain step in this test — this test must
        // pass whether or not the invoking `cargo test` run happens to have
        // `IRONCLAW_DISABLE_OS_KEYCHAIN` exported (see
        // `ironclaw_reborn_composition::factory`'s
        // `open_local_dev_secret_store_opens_a_working_store_over_the_bare_root`
        // for the same seeding pattern).
        std::fs::write(
            home.path()
                .join(ironclaw_reborn_composition::LOCAL_DEV_SECRETS_MASTER_KEY_PATH),
            ironclaw_secrets::keychain::generate_master_key_hex(),
        )
        .expect("seed cached master key");

        let mut prompts = FakePromptSource {
            provider: "nearai",
            key: "sk-test-value",
        };
        let outcome = provision_llm_credentials(home, context.boot_config(), &mut prompts)
            .expect("provision must succeed with a fake interactive source");
        assert_eq!(
            outcome,
            LlmCredentialProvisionOutcome::Configured {
                provider_id: "nearai".to_string(),
                model: "deepseek-ai/DeepSeek-V4-Flash".to_string(),
            }
        );

        let home_path = home.path().to_path_buf();
        let stored = crate::runtime::block_on_cli(async move {
            let store = ironclaw_reborn_composition::open_local_dev_secret_store(&home_path)
                .await
                .map_err(anyhow::Error::from)?;
            ironclaw_reborn_composition::LlmKeyStore::new(store)
                .read("nearai")
                .await
                .map_err(anyhow::Error::from)
        })
        .expect("read back through a fresh open of the same root");
        let material = stored.expect("a value must have been written");
        assert_eq!(
            secrecy::ExposeSecret::expose_secret(&material),
            "sk-test-value"
        );

        let config_text =
            std::fs::read_to_string(home.config_file_path()).expect("read config.toml");
        assert!(
            config_text.contains("provider_id = \"nearai\""),
            "config.toml: {config_text}"
        );
    }

    /// RED (B2 step 2): a non-interactive fake source must surface as a
    /// typed [`LlmCredentialPromptError::NonInteractive`] — never a panic or
    /// process exit — and must not write anything: `api_key()` is
    /// `unreachable!()` (proving prompting stops at the first failure) and
    /// `config.toml` must not exist afterward (proving no store/config touch
    /// happens before both prompts have succeeded).
    #[test]
    fn provision_llm_credentials_propagates_non_interactive_error_without_touching_anything() {
        let (_tmp, context) = RebornCliContext::test_context();
        let home = context.boot_config().home();
        std::fs::create_dir_all(home.path()).expect("create reborn home");

        let mut prompts = NonInteractivePromptSource;
        let error = provision_llm_credentials(home, context.boot_config(), &mut prompts)
            .expect_err("a non-interactive source must return a typed error");
        assert!(matches!(error, LlmCredentialPromptError::NonInteractive));
        assert!(
            !home.config_file_path().exists(),
            "a non-interactive prompt failure must not write config.toml"
        );
    }
}
