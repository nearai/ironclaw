//! Onboarding's OS-keychain local-dev secrets master-key provisioning step.

use ironclaw_reborn_config::RebornHome;

/// Outcome of onboarding's OS-keychain master-key provisioning attempt.
///
/// - Status enum, not an error type: every variant is a successful `execute()`.
/// - `Suppressed` is expected/normal (headless CI via `IRONCLAW_DISABLE_OS_KEYCHAIN`,
///   or the OS denies the prompt) — the resolver
///   (`ironclaw_reborn_composition::factory::resolve_local_dev_secret_master_key_with_env`)
///   still falls back to dotfile auto-generation, so this must never fail onboarding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MasterKeyProvisionOutcome {
    /// A cached `.reborn-local-dev-secrets-master-key` dotfile already
    /// exists under this Reborn home; nothing to provision.
    DotfileAlreadyPresent,
    /// The OS keychain already has a master key from a prior onboarding run.
    KeychainAlreadyPresent,
    /// A fresh key was generated and stored in the OS keychain.
    Provisioned,
    /// Keychain unavailable (test/CI suppression or OS denial). Resolver
    /// falls through to `SECRETS_MASTER_KEY` env var, then dotfile
    /// auto-generation on first boot.
    Suppressed,
}

impl MasterKeyProvisionOutcome {
    pub(crate) fn display_line(self) -> &'static str {
        match self {
            Self::DotfileAlreadyPresent => "cached dotfile already present",
            Self::KeychainAlreadyPresent => "already provisioned in OS keychain",
            Self::Provisioned => "provisioned in OS keychain",
            Self::Suppressed => "OS keychain unavailable; falling back to env/dotfile",
        }
    }
}

/// Provisions a local-dev master key in the OS keychain if absent (no cached
/// dotfile, no keychain key); no-op if either already exists. Never fails
/// `execute()` — an unavailable/denied keychain reports
/// [`MasterKeyProvisionOutcome::Suppressed`], matching the resolver's own
/// env/dotfile fallback (`crates/ironclaw_reborn_composition/src/factory.rs`).
///
/// Accepted risk (TOCTOU): the `dotfile_path.exists()` check below and the
/// keychain's own internal `has_master_key()` check
/// (`provision_local_dev_keychain_master_key`) are two separate
/// check-then-act steps with no lock between them, so two concurrent
/// `onboard` runs against the same home could both observe "absent" and
/// both provision. This is accepted for LocalDev: onboarding is a
/// single-operator, run-once-by-hand flow (never invoked concurrently by
/// `serve`, which only reads keys, never writes the keychain), so the
/// realistic worst case is a wrongly-regenerated key from running `onboard`
/// twice at once by hand — recoverable by re-entering one API key.
#[cfg(any(feature = "libsql", feature = "postgres"))]
pub(crate) fn provision_master_key(home: &RebornHome) -> anyhow::Result<MasterKeyProvisionOutcome> {
    let dotfile_path = home
        .path()
        .join(ironclaw_reborn_composition::LOCAL_DEV_SECRETS_MASTER_KEY_PATH);
    if dotfile_path.exists() {
        return Ok(MasterKeyProvisionOutcome::DotfileAlreadyPresent);
    }

    crate::runtime::block_on_cli(async move {
        let outcome = ironclaw_reborn_composition::provision_local_dev_keychain_master_key().await;
        Ok::<_, anyhow::Error>(match outcome {
            ironclaw_reborn_composition::KeychainMasterKeyOutcome::AlreadyPresent => {
                MasterKeyProvisionOutcome::KeychainAlreadyPresent
            }
            ironclaw_reborn_composition::KeychainMasterKeyOutcome::Provisioned => {
                MasterKeyProvisionOutcome::Provisioned
            }
            ironclaw_reborn_composition::KeychainMasterKeyOutcome::Suppressed => {
                MasterKeyProvisionOutcome::Suppressed
            }
        })
    })
}

/// No storage backend, no secret store: the resolver lives behind the same
/// `libsql`/`postgres` feature gate in `ironclaw_reborn_composition`.
#[cfg(not(any(feature = "libsql", feature = "postgres")))]
pub(crate) fn provision_master_key(
    _home: &RebornHome,
) -> anyhow::Result<MasterKeyProvisionOutcome> {
    Ok(MasterKeyProvisionOutcome::Suppressed)
}
