//! Resolves how to invoke `ironclaw serve` as a child process.
//!
//! Consumed by the `service` subcommand's plist/unit generators
//! (`commands/service/`) so the installed service unit's `serve`
//! invocation agrees with the installing process on Reborn home/profile.

use std::path::PathBuf;

use anyhow::{Context, Result};
use ironclaw_reborn_config::{REBORN_HOME_ENV, REBORN_PROFILE_ENV, RebornBootConfig};

/// A fully resolved way to launch `ironclaw serve` as a child
/// process: the binary path, its arguments, and the environment pairs a
/// caller must set so `serve` resolves the same Reborn home/profile as
/// the invoking process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServeInvocation {
    pub exe: PathBuf,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

/// Resolve the current executable and the `serve` invocation environment.
///
/// `IRONCLAW_REBORN_HOME` is always included — the resolved home path —
/// so a spawned `serve` always agrees with the spawning process on where
/// state lives. `IRONCLAW_REBORN_PROFILE` is included only when the
/// operator set it explicitly: this reads the raw env var rather than
/// `RebornBootConfig::profile()` (which always resolves to a default),
/// because baking a silently-defaulted profile into a long-lived service
/// unit would pin behavior the operator never asked for.
pub fn serve_invocation() -> Result<ServeInvocation> {
    let exe = std::env::current_exe().context("failed to resolve current executable")?;
    let boot_config = RebornBootConfig::resolve_from_env()
        .context("failed to resolve Reborn home for the serve invocation")?;

    let mut env = vec![(
        REBORN_HOME_ENV.to_string(),
        boot_config.home().path().display().to_string(),
    )];
    if let Ok(profile) = std::env::var(REBORN_PROFILE_ENV) {
        env.push((REBORN_PROFILE_ENV.to_string(), profile));
    }

    Ok(ServeInvocation {
        exe,
        args: vec!["serve".to_string()],
        env,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RAII guard restoring IRONCLAW_REBORN_HOME/IRONCLAW_REBORN_PROFILE
    /// on drop. Caller must hold `lock_runtime_env()` for the guard's
    /// lifetime — mirrors serve_sso.rs's manual save/set/restore pattern.
    struct EnvPairGuard {
        prior_home: Option<std::ffi::OsString>,
        prior_profile: Option<std::ffi::OsString>,
    }

    impl EnvPairGuard {
        fn set_home_only(tmp: &std::path::Path) -> Self {
            let prior_home = std::env::var_os("IRONCLAW_REBORN_HOME");
            let prior_profile = std::env::var_os("IRONCLAW_REBORN_PROFILE");
            // SAFETY: caller holds `lock_runtime_env()` for this guard's lifetime.
            unsafe {
                std::env::set_var("IRONCLAW_REBORN_HOME", tmp);
                std::env::remove_var("IRONCLAW_REBORN_PROFILE");
            }
            Self {
                prior_home,
                prior_profile,
            }
        }

        fn set_home_and_profile(tmp: &std::path::Path, profile: &str) -> Self {
            let prior_home = std::env::var_os("IRONCLAW_REBORN_HOME");
            let prior_profile = std::env::var_os("IRONCLAW_REBORN_PROFILE");
            // SAFETY: see `set_home_only`.
            unsafe {
                std::env::set_var("IRONCLAW_REBORN_HOME", tmp);
                std::env::set_var("IRONCLAW_REBORN_PROFILE", profile);
            }
            Self {
                prior_home,
                prior_profile,
            }
        }
    }

    impl Drop for EnvPairGuard {
        fn drop(&mut self) {
            // SAFETY: see `set_home_only`.
            unsafe {
                match self.prior_home.take() {
                    Some(v) => std::env::set_var("IRONCLAW_REBORN_HOME", v),
                    None => std::env::remove_var("IRONCLAW_REBORN_HOME"),
                }
                match self.prior_profile.take() {
                    Some(v) => std::env::set_var("IRONCLAW_REBORN_PROFILE", v),
                    None => std::env::remove_var("IRONCLAW_REBORN_PROFILE"),
                }
            }
        }
    }

    #[test]
    fn includes_resolved_home_and_omits_profile_when_unset() {
        let _lock = crate::runtime::test_env::lock_runtime_env();
        let tmp = tempfile::tempdir().expect("tempdir");
        let _guard = EnvPairGuard::set_home_only(tmp.path());

        let invocation =
            serve_invocation().expect("serve_invocation must resolve under a valid HOME");

        assert_eq!(invocation.args, vec!["serve".to_string()]);
        assert_eq!(
            invocation.env,
            vec![(
                "IRONCLAW_REBORN_HOME".to_string(),
                tmp.path().display().to_string()
            )]
        );
    }

    #[test]
    fn includes_profile_when_explicitly_set() {
        let _lock = crate::runtime::test_env::lock_runtime_env();
        let tmp = tempfile::tempdir().expect("tempdir");
        let _guard = EnvPairGuard::set_home_and_profile(tmp.path(), "production");

        let invocation =
            serve_invocation().expect("serve_invocation must resolve with an explicit profile");

        assert_eq!(
            invocation.env,
            vec![
                (
                    "IRONCLAW_REBORN_HOME".to_string(),
                    tmp.path().display().to_string()
                ),
                (
                    "IRONCLAW_REBORN_PROFILE".to_string(),
                    "production".to_string()
                ),
            ]
        );
    }

    #[test]
    fn resolves_current_exe_as_an_existing_absolute_path() {
        let _lock = crate::runtime::test_env::lock_runtime_env();
        let invocation = serve_invocation().expect("serve_invocation must resolve");
        assert!(invocation.exe.is_absolute());
        assert!(invocation.exe.exists());
    }
}
