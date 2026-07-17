use std::path::{Path, PathBuf};

use clap::Args;
use ironclaw_reborn_config::RebornHome;

use crate::commands::config::init::{ExistingConfigPolicy, write_default_config_files};
use crate::context::RebornCliContext;
use crate::file_write::{FileWriteAction, write_atomic};

mod master_key;
mod prompts;

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

/// Default LLM provider offered by the onboarding prompt. Reuses
/// `config::init`'s [`DEFAULT_LLM_PROVIDER_ID`](crate::commands::config::init::DEFAULT_LLM_PROVIDER_ID)
/// — the same constant the `config.toml` stub seeds — so the interactive
/// prompt default and the non-interactive stub can never drift apart on
/// which provider a fresh install boots against.
use crate::commands::config::init::DEFAULT_LLM_PROVIDER_ID as DEFAULT_LLM_PROVIDER;

/// Outcome of onboard's LLM provider/API-key prompt step. Every variant is a
/// successful `execute()` (exit 0) — mirrors [`MasterKeyProvisionOutcome`]'s
/// shape: `SkippedNonInteractive` is expected and normal, not a failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LlmCredentialProvisionOutcome {
    Configured {
        provider_id: String,
        model: String,
    },
    /// `[llm.default]` was already pointed at a provider AND the encrypted
    /// secret store already has a key for it (see
    /// [`already_configured_outcome`]) — this run skipped prompting
    /// entirely rather than re-asking for credentials that are already
    /// durably stored.
    AlreadyConfigured {
        provider_id: String,
        model: String,
    },
    SkippedNonInteractive,
}

impl LlmCredentialProvisionOutcome {
    fn display_line(&self) -> String {
        match self {
            Self::Configured { provider_id, model } => {
                format!("configured provider `{provider_id}` (model `{model}`)")
            }
            Self::AlreadyConfigured { provider_id, model } => {
                format!(
                    "already configured (provider `{provider_id}`, model `{model}`); use \
                     --force to reconfigure"
                )
            }
            Self::SkippedNonInteractive => "skipped (non-interactive session)".to_string(),
        }
    }
}

/// Where [`provision_llm_credentials`] gets its (already-open) encrypted
/// secret store from. Injected — mirrors [`PromptSource`] — so a test can
/// supply a store whose `put` fails, proving the write-ordering guarantee
/// (secret stored before config is written; see
/// [`provision_llm_credentials`]'s doc) without touching the real
/// local-dev libsql-backed store.
///
/// Gated with the same `libsql`+`root-llm-provider` cfg as
/// `ironclaw_reborn_composition::LlmKeyStore` itself: that type (and
/// `open_local_dev_secret_store`) only exists behind those features, so this
/// trait's return type can't compile without them. See the `#[cfg(not(...))]`
/// stub below for the feature-off case.
#[cfg(all(feature = "libsql", feature = "root-llm-provider"))]
trait LlmKeyStoreOpener {
    fn open(&self, home_path: &Path) -> anyhow::Result<ironclaw_reborn_composition::LlmKeyStore>;
}

/// Production [`LlmKeyStoreOpener`]: opens the real local-dev encrypted
/// secret store `serve` later reads from (see
/// `ironclaw_reborn_composition::open_local_dev_secret_store`'s doc for why
/// this is the same physical storage `serve` opens).
#[cfg(all(feature = "libsql", feature = "root-llm-provider"))]
struct LocalDevLlmKeyStoreOpener;

#[cfg(all(feature = "libsql", feature = "root-llm-provider"))]
impl LlmKeyStoreOpener for LocalDevLlmKeyStoreOpener {
    fn open(&self, home_path: &Path) -> anyhow::Result<ironclaw_reborn_composition::LlmKeyStore> {
        let home_path = home_path.to_path_buf();
        crate::runtime::block_on_cli(async move {
            let store = ironclaw_reborn_composition::open_local_dev_secret_store(&home_path)
                .await
                .map_err(anyhow::Error::from)?;
            Ok::<_, anyhow::Error>(ironclaw_reborn_composition::LlmKeyStore::new(store))
        })
    }
}

/// Feature-off stub for [`LlmKeyStoreOpener`]/[`LocalDevLlmKeyStoreOpener`]:
/// without both `libsql` and `root-llm-provider` there is no `LlmKeyStore`
/// type to open at all. This stub exists solely so `execute()`'s
/// unconditional `&LocalDevLlmKeyStoreOpener` call site compiles across every
/// feature combination — the feature-off `provision_llm_credentials` below
/// ignores its `store_opener` parameter entirely, so `open` is never called.
#[cfg(not(all(feature = "libsql", feature = "root-llm-provider")))]
trait LlmKeyStoreOpener {
    fn open(&self, home_path: &Path) -> anyhow::Result<()>;
}

#[cfg(not(all(feature = "libsql", feature = "root-llm-provider")))]
struct LocalDevLlmKeyStoreOpener;

#[cfg(not(all(feature = "libsql", feature = "root-llm-provider")))]
impl LlmKeyStoreOpener for LocalDevLlmKeyStoreOpener {
    fn open(&self, _home_path: &Path) -> anyhow::Result<()> {
        Ok(())
    }
}

/// Prompt for an LLM provider + API key and persist both: the key value goes
/// into the encrypted secret store via the canonical `LlmKeyStore` handle
/// (`llm_provider_<id>_api_key`) FIRST — the same handle the webui2 settings
/// surface writes and `apply_startup_stored_llm_key` reads at boot — and
/// only once that succeeds does the provider selection land in
/// `[llm.default]` in `config.toml` SECOND (existing
/// `RebornProviderAdmin::set_provider` config machinery, the same one
/// `ironclaw-reborn models set-provider` uses). This ordering means
/// `config.toml` can never point at a provider whose key failed to persist
/// durably: a `LlmKeyStore::put` failure returns an error before
/// `set_provider` is ever called, leaving `config.toml` exactly as it was.
///
/// Gathers both prompt answers before writing anything: a non-interactive
/// `provider()` or `api_key()` failure must leave config.toml and the secret
/// store untouched, not partially written.
///
/// Skips prompting entirely (an idempotent no-op) on a rerun where
/// `[llm.default]` is already user-configured AND the store already has a
/// key for that provider, unless `force` is set — see
/// [`already_configured_outcome`].
#[cfg(all(feature = "libsql", feature = "root-llm-provider"))]
fn provision_llm_credentials(
    home: &RebornHome,
    boot: &ironclaw_reborn_config::RebornBootConfig,
    prompts: &mut dyn PromptSource,
    store_opener: &dyn LlmKeyStoreOpener,
    force: bool,
) -> Result<LlmCredentialProvisionOutcome, LlmCredentialPromptError> {
    if !prompts.is_interactive() {
        return Err(LlmCredentialPromptError::NonInteractive);
    }

    let admin = ironclaw_reborn_composition::RebornProviderAdmin::new(boot.clone());

    if !force && let Some(outcome) = already_configured_outcome(&admin, home, store_opener)? {
        return Ok(outcome);
    }

    let provider = prompts.provider(DEFAULT_LLM_PROVIDER)?;
    let key = prompts.api_key(&provider)?;
    // Defense in depth: `StdinPromptSource::api_key` already re-prompts on a
    // blank answer, but this guards every `PromptSource` implementation —
    // present or future — so a blank key can never reach the secret store
    // regardless of where it slipped through.
    if key.trim().is_empty() {
        return Err(LlmCredentialPromptError::Other(anyhow::anyhow!(
            "LLM API key must not be blank"
        )));
    }

    let canonical_provider_id = admin
        .resolve_provider_id(&provider)
        .map_err(|error| LlmCredentialPromptError::Other(error.into()))?;

    let store = store_opener
        .open(home.path())
        .map_err(LlmCredentialPromptError::Other)?;
    let provider_id_for_store = canonical_provider_id.clone();
    crate::runtime::block_on_cli(async move {
        store
            .put_plaintext(&provider_id_for_store, key)
            .await
            .map_err(anyhow::Error::from)
    })
    .map_err(LlmCredentialPromptError::Other)?;

    let write_outcome = admin
        .set_provider(&provider, None)
        .map_err(|error| LlmCredentialPromptError::Other(error.into()))?;

    Ok(LlmCredentialProvisionOutcome::Configured {
        provider_id: write_outcome.provider_id,
        model: write_outcome.model,
    })
}

/// `Some` when `[llm.default]` already names a provider AND the encrypted
/// secret store already has a key stored for it — the idempotent-rerun
/// case [`provision_llm_credentials`] must skip prompting for (a bare
/// stub-seeded `[llm.default]` with no stored key, e.g. right after a fresh
/// `onboard` on a headless box, does NOT count: that provider has never
/// actually been credentialed, so a later interactive rerun must still
/// prompt). A store-open failure is treated as "can't tell" and falls
/// through to prompting rather than erroring the whole run.
#[cfg(all(feature = "libsql", feature = "root-llm-provider"))]
fn already_configured_outcome(
    admin: &ironclaw_reborn_composition::RebornProviderAdmin,
    home: &RebornHome,
    store_opener: &dyn LlmKeyStoreOpener,
) -> Result<Option<LlmCredentialProvisionOutcome>, LlmCredentialPromptError> {
    let status = admin
        .status()
        .map_err(|error| LlmCredentialPromptError::Other(error.into()))?;
    let Some(selection) = status.default else {
        return Ok(None);
    };
    let Some(provider_id) = selection.provider_id else {
        return Ok(None);
    };
    let store = match store_opener.open(home.path()) {
        Ok(store) => store,
        Err(error) => {
            tracing::debug!(
                %error,
                "secret store open failed while checking already-configured LLM; falling \
                 through to prompt"
            );
            return Ok(None);
        }
    };
    let provider_id_for_check = provider_id.clone();
    let has_key = crate::runtime::block_on_cli(async move {
        store
            .exists(&provider_id_for_check)
            .await
            .map_err(anyhow::Error::from)
    })
    .map_err(LlmCredentialPromptError::Other)?;
    if !has_key {
        return Ok(None);
    }
    Ok(Some(LlmCredentialProvisionOutcome::AlreadyConfigured {
        provider_id,
        model: selection.model.unwrap_or_default(),
    }))
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
    _store_opener: &dyn LlmKeyStoreOpener,
    _force: bool,
) -> Result<LlmCredentialProvisionOutcome, LlmCredentialPromptError> {
    Ok(LlmCredentialProvisionOutcome::SkippedNonInteractive)
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

    struct FakePromptSource {
        provider: &'static str,
        key: &'static str,
    }

    impl PromptSource for FakePromptSource {
        fn is_interactive(&self) -> bool {
            true
        }

        fn provider(&mut self, _default: &str) -> Result<String, LlmCredentialPromptError> {
            Ok(self.provider.to_string())
        }

        fn api_key(&mut self, _provider: &str) -> Result<String, LlmCredentialPromptError> {
            Ok(self.key.to_string())
        }
    }

    struct NonInteractivePromptSource;

    impl PromptSource for NonInteractivePromptSource {
        fn is_interactive(&self) -> bool {
            false
        }

        fn provider(&mut self, _default: &str) -> Result<String, LlmCredentialPromptError> {
            unreachable!("provider() must not be called once is_interactive() is false")
        }

        fn api_key(&mut self, _provider: &str) -> Result<String, LlmCredentialPromptError> {
            unreachable!("api_key must not be prompted once provider() has already failed")
        }
    }

    /// A [`PromptSource`] whose prompt methods panic if called — used to
    /// prove an idempotent rerun (item 3) skips prompting entirely rather
    /// than merely tolerating a repeated answer.
    struct PanickingPromptSource;

    impl PromptSource for PanickingPromptSource {
        fn is_interactive(&self) -> bool {
            true
        }

        fn provider(&mut self, _default: &str) -> Result<String, LlmCredentialPromptError> {
            panic!("provider() must not be called on an idempotent, already-configured rerun")
        }

        fn api_key(&mut self, _provider: &str) -> Result<String, LlmCredentialPromptError> {
            panic!("api_key() must not be called on an idempotent, already-configured rerun")
        }
    }

    /// A [`LlmKeyStoreOpener`] whose store's `put` always fails — used to
    /// prove `provision_llm_credentials` writes the secret store BEFORE
    /// `config.toml`: a `put` failure must leave `config.toml` untouched.
    struct FailingLlmKeyStoreOpener;

    impl LlmKeyStoreOpener for FailingLlmKeyStoreOpener {
        fn open(
            &self,
            _home_path: &Path,
        ) -> anyhow::Result<ironclaw_reborn_composition::LlmKeyStore> {
            Ok(ironclaw_reborn_composition::LlmKeyStore::new(Arc::new(
                FailingSecretStore,
            )))
        }
    }

    struct FailingSecretStore;

    #[async_trait::async_trait]
    impl ironclaw_secrets::SecretStore for FailingSecretStore {
        async fn put(
            &self,
            _scope: ironclaw_host_api::ResourceScope,
            _handle: ironclaw_host_api::SecretHandle,
            _material: ironclaw_secrets::SecretMaterial,
            _expires_at: Option<ironclaw_host_api::Timestamp>,
        ) -> Result<ironclaw_secrets::SecretMetadata, ironclaw_secrets::SecretStoreError> {
            Err(ironclaw_secrets::SecretStoreError::StoreUnavailable {
                reason: "simulated failure for write-ordering RED test".to_string(),
            })
        }

        async fn metadata(
            &self,
            _scope: &ironclaw_host_api::ResourceScope,
            _handle: &ironclaw_host_api::SecretHandle,
        ) -> Result<Option<ironclaw_secrets::SecretMetadata>, ironclaw_secrets::SecretStoreError>
        {
            unreachable!("not exercised by provision_llm_credentials")
        }

        async fn metadata_for_scope(
            &self,
            _scope: &ironclaw_host_api::ResourceScope,
        ) -> Result<Vec<ironclaw_secrets::SecretMetadata>, ironclaw_secrets::SecretStoreError>
        {
            unreachable!("not exercised by provision_llm_credentials")
        }

        async fn delete(
            &self,
            _scope: &ironclaw_host_api::ResourceScope,
            _handle: &ironclaw_host_api::SecretHandle,
        ) -> Result<bool, ironclaw_secrets::SecretStoreError> {
            unreachable!("not exercised by provision_llm_credentials")
        }

        async fn lease_once(
            &self,
            _scope: &ironclaw_host_api::ResourceScope,
            _handle: &ironclaw_host_api::SecretHandle,
        ) -> Result<ironclaw_secrets::SecretLease, ironclaw_secrets::SecretStoreError> {
            unreachable!("not exercised by provision_llm_credentials")
        }

        async fn consume(
            &self,
            _scope: &ironclaw_host_api::ResourceScope,
            _lease_id: ironclaw_secrets::SecretLeaseId,
        ) -> Result<ironclaw_secrets::SecretMaterial, ironclaw_secrets::SecretStoreError> {
            unreachable!("not exercised by provision_llm_credentials")
        }

        async fn revoke(
            &self,
            _scope: &ironclaw_host_api::ResourceScope,
            _lease_id: ironclaw_secrets::SecretLeaseId,
        ) -> Result<ironclaw_secrets::SecretLease, ironclaw_secrets::SecretStoreError> {
            unreachable!("not exercised by provision_llm_credentials")
        }

        async fn leases_for_scope(
            &self,
            _scope: &ironclaw_host_api::ResourceScope,
        ) -> Result<Vec<ironclaw_secrets::SecretLease>, ironclaw_secrets::SecretStoreError>
        {
            unreachable!("not exercised by provision_llm_credentials")
        }
    }

    /// Seed a cached master-key dotfile so the real local-dev store opener's
    /// resolver never reaches the OS keychain step in a test — see
    /// `ironclaw_reborn_composition::factory`'s
    /// `open_local_dev_secret_store_opens_a_working_store_over_the_bare_root`
    /// for the same seeding pattern.
    fn seed_cached_master_key(home: &RebornHome) {
        std::fs::write(
            home.path()
                .join(ironclaw_reborn_composition::LOCAL_DEV_SECRETS_MASTER_KEY_PATH),
            ironclaw_secrets::keychain::generate_master_key_hex(),
        )
        .expect("seed cached master key");
    }

    /// RED (B2 step 1): a fake interactive `PromptSource` answering
    /// `("nearai", "sk-test-value")` must land the provider selection in
    /// `config.toml` and the key value in the encrypted secret store,
    /// readable back through a *fresh* open of the same root — proving the
    /// opener and `LlmKeyStore::put`/`read` agree on physical storage.
    ///
    /// Also proves item 3's idempotent-rerun guard: a second call with a
    /// `PanickingPromptSource` (whose prompt methods panic if invoked) must
    /// return `AlreadyConfigured` without ever calling `provider()`/
    /// `api_key()` — proving the rerun is skipped, not merely tolerated.
    #[test]
    fn provision_llm_credentials_writes_config_and_secret_store_through_fake_prompts() {
        let (_tmp, context) = RebornCliContext::test_context();
        let home = context.boot_config().home();
        std::fs::create_dir_all(home.path()).expect("create reborn home");
        seed_cached_master_key(home);

        let mut prompts = FakePromptSource {
            provider: "nearai",
            key: "sk-test-value",
        };
        let outcome = provision_llm_credentials(
            home,
            context.boot_config(),
            &mut prompts,
            &LocalDevLlmKeyStoreOpener,
            false,
        )
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

        // Item 3: a rerun with an already-configured provider + stored key
        // must skip prompting entirely.
        let mut second_prompts = PanickingPromptSource;
        let second_outcome = provision_llm_credentials(
            home,
            context.boot_config(),
            &mut second_prompts,
            &LocalDevLlmKeyStoreOpener,
            false,
        )
        .expect("an idempotent rerun must succeed without prompting");
        assert_eq!(
            second_outcome,
            LlmCredentialProvisionOutcome::AlreadyConfigured {
                provider_id: "nearai".to_string(),
                model: "deepseek-ai/DeepSeek-V4-Flash".to_string(),
            }
        );
    }

    /// RED (B2 step 2): a non-interactive fake source must surface as a
    /// typed [`LlmCredentialPromptError::NonInteractive`] — never a panic or
    /// process exit — and must not write anything: `provider()`/`api_key()`
    /// are `unreachable!()` (proving the interactivity check short-circuits
    /// before either prompt runs) and `config.toml` must not exist
    /// afterward (proving no store/config touch happens before both prompts
    /// have succeeded).
    #[test]
    fn provision_llm_credentials_propagates_non_interactive_error_without_touching_anything() {
        let (_tmp, context) = RebornCliContext::test_context();
        let home = context.boot_config().home();
        std::fs::create_dir_all(home.path()).expect("create reborn home");

        let mut prompts = NonInteractivePromptSource;
        let error = provision_llm_credentials(
            home,
            context.boot_config(),
            &mut prompts,
            &LocalDevLlmKeyStoreOpener,
            false,
        )
        .expect_err("a non-interactive source must return a typed error");
        assert!(matches!(error, LlmCredentialPromptError::NonInteractive));
        assert!(
            !home.config_file_path().exists(),
            "a non-interactive prompt failure must not write config.toml"
        );
    }

    /// RED (item 2, write ordering): a store whose `put` always fails must
    /// leave `config.toml` completely untouched — proving the secret is
    /// written BEFORE the provider selection, not after. Under the old
    /// ordering (config first, store second) `config.toml` would already
    /// carry `provider_id = "nearai"` by the time the store write failed.
    #[test]
    fn provision_llm_credentials_leaves_config_untouched_when_the_store_put_fails() {
        let (_tmp, context) = RebornCliContext::test_context();
        let home = context.boot_config().home();
        std::fs::create_dir_all(home.path()).expect("create reborn home");

        let mut prompts = FakePromptSource {
            provider: "nearai",
            key: "sk-test-value",
        };
        let error = provision_llm_credentials(
            home,
            context.boot_config(),
            &mut prompts,
            &FailingLlmKeyStoreOpener,
            false,
        )
        .expect_err("a failing store put must surface as an error");
        assert!(matches!(error, LlmCredentialPromptError::Other(_)));
        assert!(
            !home.config_file_path().exists(),
            "a failed key-store write must leave config.toml untouched — store first, config \
             second"
        );
    }

    /// RED (round-2 review item 3): a `PromptSource` whose `api_key()`
    /// returns a whitespace-only answer (e.g. a fake standing in for an
    /// implementation that didn't get the blank-rejection retry loop
    /// `StdinPromptSource::api_key` has) must never reach the secret store —
    /// `provision_llm_credentials`'s own blank guard is the backstop for
    /// every `PromptSource`, not just the terminal-backed one.
    #[test]
    fn provision_llm_credentials_rejects_a_blank_api_key_without_touching_anything() {
        let (_tmp, context) = RebornCliContext::test_context();
        let home = context.boot_config().home();
        std::fs::create_dir_all(home.path()).expect("create reborn home");
        seed_cached_master_key(home);

        let mut prompts = FakePromptSource {
            provider: "nearai",
            key: "   ",
        };
        let error = provision_llm_credentials(
            home,
            context.boot_config(),
            &mut prompts,
            &LocalDevLlmKeyStoreOpener,
            false,
        )
        .expect_err("a blank API key must be rejected");
        assert!(matches!(error, LlmCredentialPromptError::Other(_)));
        assert!(
            !home.config_file_path().exists(),
            "a rejected blank API key must leave config.toml untouched"
        );
    }
}
