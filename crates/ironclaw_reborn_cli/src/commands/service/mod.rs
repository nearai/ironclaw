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

    fn status(&self) -> Result<()> {
        match self {
            Self::MacOs => launchd::status(),
            Self::Linux => systemd::status(),
        }
    }

    fn uninstall(&self) -> Result<()> {
        // Shared across platforms: best-effort stop before removing the
        // unit file, so an uninstall never leaves a running process
        // behind. Errors ignored — the service may not be loaded/active.
        self.stop().ok();
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

/// Run `command` and capture its stdout, falling back to stderr when
/// stdout is empty (some tools, e.g. `systemctl is-active`, print
/// their result on stdout only on success). `label` names the
/// operation for the spawn-failure error message.
fn run_capture(label: &str, command: &mut Command) -> Result<String> {
    let output = command
        .output()
        .with_context(|| format!("failed to spawn command for {label}"))?;
    let mut text = String::from_utf8_lossy(&output.stdout).to_string();
    if text.trim().is_empty() {
        text = String::from_utf8_lossy(&output.stderr).to_string();
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

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
        std::fs::write(
            crate::commands::onboard::onboarding_marker_path(home),
            "{}",
        )
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
    fn run_capture_reads_stdout() {
        let out = run_capture("test echo", Command::new("sh").args(["-c", "echo hello"]))
            .expect("stdout capture should succeed");
        assert_eq!(out.trim(), "hello");
    }

    #[test]
    fn run_capture_falls_back_to_stderr() {
        let out = run_capture(
            "test warn",
            Command::new("sh").args(["-c", "echo warn 1>&2"]),
        )
        .expect("stderr capture should succeed");
        assert_eq!(out.trim(), "warn");
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
}
