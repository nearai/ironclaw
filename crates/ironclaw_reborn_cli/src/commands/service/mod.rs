//! `ironclaw-reborn service` — install/manage the standalone Reborn
//! binary as an OS-native service.
//!
//! - **macOS**: launchd user agent at
//!   `~/Library/LaunchAgents/com.ironclaw.reborn.daemon.plist`.
//! - **Linux**: systemd user unit at
//!   `~/.config/systemd/user/ironclaw-reborn.service`.
//!
//! The installed service runs `<current_exe> serve`, restarting
//! automatically on failure. Mirrors v1's `src/service.rs` shape with
//! Reborn's label, paths, and env-passthrough contract; no v1 code is
//! shared or reused.
//!
//! Platform dispatch happens once, in [`ServicePlatform::detect`], called
//! a single time from [`ServiceCommand::execute`]. The five verbs are
//! methods on [`ServicePlatform`] that each `match self` to delegate
//! into the OS-specific implementation (`launchd` or `systemd`), rather
//! than every verb re-checking `cfg!(target_os)`.

use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};

use crate::context::RebornCliContext;
use crate::serve_invocation::serve_invocation;

mod launchd;
mod systemd;

const SERVICE_LABEL: &str = "com.ironclaw.reborn.daemon";
const SYSTEMD_UNIT: &str = "ironclaw-reborn.service";
const UNSUPPORTED_OS_MESSAGE: &str = "Service management is only supported on macOS and Linux";

// ── Clap surface ────────────────────────────────────────────────

#[derive(Debug, Args)]
pub(crate) struct ServiceCommand {
    #[command(subcommand)]
    command: ServiceVerb,
}

#[derive(Debug, Subcommand)]
enum ServiceVerb {
    /// Install the OS service (launchd on macOS, systemd on Linux).
    Install,
    /// Start the installed service.
    Start,
    /// Stop the running service.
    Stop,
    /// Restart the service: stop then start if running, or just start if
    /// stopped. Errors if the service is not installed.
    Restart,
    /// Show service status.
    Status,
    /// Uninstall the OS service and remove the unit file.
    Uninstall,
}

impl ServiceCommand {
    pub(crate) fn execute(self, context: RebornCliContext) -> Result<()> {
        let platform = ServicePlatform::detect()?;
        match self.command {
            ServiceVerb::Install => platform.install(&context),
            ServiceVerb::Start => platform.start(),
            ServiceVerb::Stop => platform.stop(),
            ServiceVerb::Restart => platform.restart(),
            ServiceVerb::Status => platform.status(),
            ServiceVerb::Uninstall => platform.uninstall(),
        }
    }
}

// ── Platform dispatch ───────────────────────────────────────────

/// The two supported service-management targets. Detected once per
/// invocation via [`ServicePlatform::detect`]; every verb dispatches
/// off the resolved variant instead of re-checking `cfg!(target_os)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ServicePlatform {
    MacOs,
    Linux,
}

impl ServicePlatform {
    fn detect() -> Result<Self> {
        if cfg!(target_os = "macos") {
            Ok(Self::MacOs)
        } else if cfg!(target_os = "linux") {
            Ok(Self::Linux)
        } else {
            bail!(UNSUPPORTED_OS_MESSAGE);
        }
    }

    fn install(&self, context: &RebornCliContext) -> Result<()> {
        for warning in preflight_warnings(context) {
            eprintln!("warning: {warning}");
        }
        let invocation = serve_invocation()?;
        match self {
            Self::MacOs => launchd::install(context, &invocation),
            Self::Linux => systemd::install(&invocation),
        }
    }

    fn start(&self) -> Result<()> {
        match self {
            Self::MacOs => launchd::start(),
            Self::Linux => systemd::start(),
        }
    }

    fn stop(&self) -> Result<()> {
        match self {
            Self::MacOs => launchd::stop(),
            Self::Linux => systemd::stop(),
        }
    }

    fn restart(&self) -> Result<()> {
        match self {
            Self::MacOs => launchd::restart(),
            Self::Linux => systemd::restart(),
        }
    }

    fn status(&self) -> Result<()> {
        match self {
            Self::MacOs => launchd::status(),
            Self::Linux => systemd::status(),
        }
    }

    fn uninstall(&self) -> Result<()> {
        match self {
            Self::MacOs => launchd::uninstall(),
            Self::Linux => systemd::uninstall(),
        }
    }
}

// ── Path helpers ────────────────────────────────────────────────

/// The OS user's real home directory (`$HOME`), used only for the
/// service-definition file location. Distinct from the Reborn home
/// (`IRONCLAW_REBORN_HOME`), which holds operator config/logs and may
/// point anywhere. Windows never reaches this — `ServicePlatform::detect`
/// bails first — so only `$HOME` (POSIX) is read.
fn home_dir() -> Result<PathBuf> {
    let raw = std::env::var_os("HOME").context("HOME must be set to manage an OS service")?;
    let path = PathBuf::from(raw);
    if !path.is_absolute() {
        bail!("HOME must be an absolute path to manage an OS service");
    }
    Ok(path)
}

// ── Preflight ───────────────────────────────────────────────────

/// Non-fatal readiness warnings for `service install`: warn, don't
/// fail, when onboarding hasn't run yet — the service can still be
/// installed, but `serve` starts without config/providers/token until
/// `ironclaw-reborn onboard` runs.
fn preflight_warnings(context: &RebornCliContext) -> Vec<String> {
    let home = context.boot_config().home();
    let mut warnings = Vec::new();

    let marker = crate::commands::onboard::onboarding_marker_path(home);
    if !marker.exists() {
        warnings.push(format!(
            "onboarding marker not found at {} — run `ironclaw-reborn onboard` first so \
             `serve` has config, providers, and the WebUI token available",
            marker.display()
        ));
    }

    let config = home.config_file_path();
    if !config.exists() {
        warnings.push(format!(
            "config.toml not found at {} — `serve` will run with compiled-in defaults only",
            config.display()
        ));
    }

    warnings
}

// ── Shell helpers ───────────────────────────────────────────────

/// Run `command`, treating a non-zero exit as an error. `label` names
/// the operation for the error message (e.g. "launchctl load").
fn run_checked(label: &str, command: &mut Command) -> Result<()> {
    let output = command
        .output()
        .with_context(|| format!("failed to spawn command for {label}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("{label} failed: {}", stderr.trim());
    }
    Ok(())
}

fn run_capture_checked(label: &str, command: &mut Command) -> Result<String> {
    let output = command
        .output()
        .with_context(|| format!("failed to spawn command for {label}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("{label} failed: {}", stderr.trim());
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Injectable command boundary for service-manager lifecycle operations.
///
/// Production uses [`OsServiceCommandRunner`]. Tests provide a recorder so
/// failure propagation and command ordering are verified without reaching the
/// host's real launchd/systemd instance.
trait ServiceCommandRunner {
    fn run_checked(&mut self, label: &str, command: &mut Command) -> Result<()>;
    fn run_capture_checked(&mut self, label: &str, command: &mut Command) -> Result<String>;
}

struct OsServiceCommandRunner;

impl ServiceCommandRunner for OsServiceCommandRunner {
    fn run_checked(&mut self, label: &str, command: &mut Command) -> Result<()> {
        run_checked(label, command)
    }

    fn run_capture_checked(&mut self, label: &str, command: &mut Command) -> Result<String> {
        run_capture_checked(label, command)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct SuccessfulServiceCommandRunner;

    impl ServiceCommandRunner for SuccessfulServiceCommandRunner {
        fn run_checked(&mut self, _label: &str, _command: &mut Command) -> Result<()> {
            Ok(())
        }

        fn run_capture_checked(&mut self, label: &str, _command: &mut Command) -> Result<String> {
            match label {
                "launchctl list" => Ok(String::new()),
                "id -u" => Ok("501\n".to_string()),
                _ => Ok(String::new()),
            }
        }
    }

    #[test]
    fn detect_returns_a_supported_platform_on_this_test_host() {
        // This test only runs in CI/dev on macOS or Linux, so detect()
        // must resolve to one of the two supported variants, never bail.
        let platform = ServicePlatform::detect().expect("detect must resolve on macOS/Linux");
        assert!(matches!(
            platform,
            ServicePlatform::MacOs | ServicePlatform::Linux
        ));
    }

    #[test]
    fn preflight_warnings_empty_when_marker_and_config_present() {
        let (_tmp, context) = RebornCliContext::test_context();
        let home = context.boot_config().home();
        std::fs::create_dir_all(home.path()).expect("create home");
        std::fs::write(crate::commands::onboard::onboarding_marker_path(home), "{}")
            .expect("write marker");
        std::fs::write(home.config_file_path(), "").expect("write config");

        assert!(preflight_warnings(&context).is_empty());
    }

    #[test]
    fn preflight_warnings_flags_missing_marker_and_config() {
        let (_tmp, context) = RebornCliContext::test_context();
        let warnings = preflight_warnings(&context);
        assert_eq!(warnings.len(), 2);
        assert!(warnings[0].contains("onboarding marker not found"));
        assert!(warnings[1].contains("config.toml not found"));
    }

    #[test]
    fn run_capture_checked_reads_stdout() {
        let out = run_capture_checked("test echo", Command::new("sh").args(["-c", "echo hello"]))
            .expect("stdout capture should succeed");
        assert_eq!(out.trim(), "hello");
    }

    #[test]
    fn run_capture_checked_rejects_non_zero_exit() {
        let error = run_capture_checked(
            "test capture",
            Command::new("sh").args(["-c", "echo warn 1>&2; exit 17"]),
        )
        .expect_err("non-zero capture must fail");
        assert!(error.to_string().contains("test capture failed: warn"));
    }

    #[test]
    fn run_checked_errors_on_non_zero_exit_and_names_the_label() {
        let err = run_checked("test exit 17", Command::new("sh").args(["-c", "exit 17"]))
            .expect_err("non-zero exit should error");
        assert!(err.to_string().contains("test exit 17 failed"));
    }

    #[test]
    fn run_checked_succeeds_on_zero_exit() {
        assert!(run_checked("test exit 0", Command::new("sh").args(["-c", "exit 0"])).is_ok());
    }

    // ── Command-level file lifecycle (temp-$HOME) ──────────────────

    /// RAII guard pointing the OS home (`$HOME`, read by `home_dir()`)
    /// at a tempdir, and clearing IRONCLAW_REBORN_HOME so the derived
    /// Reborn home nests under the same tempdir (RebornHome falls back
    /// to `$HOME/.ironclaw/reborn` when unset). Restores both on drop.
    /// Caller must hold `lock_runtime_env()`.
    struct TempHomeGuard {
        prior_home: Option<std::ffi::OsString>,
        prior_reborn_home: Option<std::ffi::OsString>,
    }

    impl TempHomeGuard {
        fn set(tmp: &std::path::Path) -> Self {
            let prior_home = std::env::var_os("HOME");
            let prior_reborn_home = std::env::var_os("IRONCLAW_REBORN_HOME");
            // SAFETY: caller holds `lock_runtime_env()` for this guard's lifetime.
            unsafe {
                std::env::set_var("HOME", tmp);
                std::env::remove_var("IRONCLAW_REBORN_HOME");
            }
            Self {
                prior_home,
                prior_reborn_home,
            }
        }
    }

    impl Drop for TempHomeGuard {
        fn drop(&mut self) {
            // SAFETY: see `set`.
            unsafe {
                match self.prior_home.take() {
                    Some(v) => std::env::set_var("HOME", v),
                    None => std::env::remove_var("HOME"),
                }
                match self.prior_reborn_home.take() {
                    Some(v) => std::env::set_var("IRONCLAW_REBORN_HOME", v),
                    None => std::env::remove_var("IRONCLAW_REBORN_HOME"),
                }
            }
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn install_then_uninstall_linux_writes_and_removes_unit_file() {
        let _lock = crate::runtime::test_env::lock_runtime_env();
        let tmp = tempfile::tempdir().expect("tempdir");
        let _home = TempHomeGuard::set(tmp.path());
        let invocation = serve_invocation().expect("serve invocation");
        let mut runner = SuccessfulServiceCommandRunner;
        systemd::install_with_runner(&invocation, &mut runner).expect("install must succeed");
        let unit_path = tmp
            .path()
            .join(".config/systemd/user/ironclaw-reborn.service");
        assert!(unit_path.exists(), "unit file must be written");
        let contents = std::fs::read_to_string(&unit_path).expect("read unit file");
        assert!(contents.contains("ExecStart="));
        assert!(contents.contains("IRONCLAW_REBORN_HOME="));

        // Idempotent reinstall: overwrites cleanly, no fail or duplicate.
        systemd::install_with_runner(&invocation, &mut runner).expect("reinstall must succeed");
        assert!(unit_path.exists());

        systemd::uninstall_with_runner(&mut runner).expect("uninstall must succeed");
        assert!(!unit_path.exists(), "unit file must be removed");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn install_then_uninstall_macos_writes_and_removes_plist() {
        let _lock = crate::runtime::test_env::lock_runtime_env();
        let tmp = tempfile::tempdir().expect("tempdir");
        let _home = TempHomeGuard::set(tmp.path());
        let context = RebornCliContext::from_boot_config(
            ironclaw_reborn_config::RebornBootConfig::resolve_from_env()
                .expect("boot config must resolve under temp HOME"),
        );

        ServicePlatform::MacOs
            .install(&context)
            .expect("install must succeed");
        let plist_path = tmp
            .path()
            .join("Library/LaunchAgents/com.ironclaw.reborn.daemon.plist");
        assert!(plist_path.exists(), "plist file must be written");
        let contents = std::fs::read_to_string(&plist_path).expect("read plist file");
        assert!(contents.contains(SERVICE_LABEL));
        assert!(contents.contains("<key>IRONCLAW_REBORN_HOME</key>"));

        // Idempotent reinstall.
        ServicePlatform::MacOs
            .install(&context)
            .expect("reinstall must succeed");
        assert!(plist_path.exists());

        let mut runner = SuccessfulServiceCommandRunner;
        launchd::uninstall_with_runner(&mut runner).expect("uninstall must succeed");
        assert!(!plist_path.exists(), "plist file must be removed");
    }
}
