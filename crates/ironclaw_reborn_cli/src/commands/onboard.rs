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
        let marker_action =
            write_onboarding_marker(home, &marker_path, self.force, self.import_history)?;
        let master_key_outcome = provision_master_key(home)?;

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
        println!("v1_state: not-used");
        println!();
        println!("completed:");
        println!("- reborn home initialized");
        println!("- config.toml and providers.json available");
        println!("- webui bearer token provisioned (used by `serve` when the env var is unset)");
        println!("- onboarding completion marker available");
        println!();
        println!("remaining:");
        println!("- configure LLM credentials through env vars referenced by config.toml");
        println!(
            "- run `ironclaw-reborn models set-provider <provider> --model <model>` as needed"
        );
        if self.import_history {
            println!("- history import requested but not wired yet");
        } else {
            println!("- history import not requested");
        }
        Ok(())
    }
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
        "steps_pending": pending_steps(import_history),
        "v1_state": "not-used"
    }))?;
    write_atomic(
        marker_path,
        &format!("{body}\n"),
        force,
        ONBOARDING_MARKER_FILE,
    )
}

fn pending_steps(import_history: bool) -> Vec<&'static str> {
    let mut steps = vec!["llm_credentials", "model_selection", "channel_setup"];
    if import_history {
        steps.push("history_import");
    }
    steps
}
