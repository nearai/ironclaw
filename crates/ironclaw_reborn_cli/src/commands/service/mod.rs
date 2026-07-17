//! `ironclaw-reborn service` — install/manage the standalone Reborn
//! binary as an OS-native service.
//!
//! - **macOS**: launchd user agent at
//!   `~/Library/LaunchAgents/com.ironclaw.reborn.plist`.
//! - **Linux**: systemd user unit at
//!   `~/.config/systemd/user/ironclaw-reborn.service`.
//!
//! The installed service runs `<current_exe> serve`, restarting
//! automatically on failure. Mirrors v1's `src/service.rs` shape with
//! Reborn's label, paths, and env-passthrough contract; no v1 code is
//! shared or reused.
//!
//! Platform dispatch happens once, in [`ServicePlatform::detect`], called
//! a single time from [`ServiceCommand::execute`]. Each verb is a method
//! on [`ServicePlatform`] that `match self`es to delegate into the
//! OS-specific implementation (`launchd` or `systemd`), rather than every
//! verb re-checking `cfg!(target_os)`.
//!
//! ## Canonical service identity, shared with the WebUI operator facade
//!
//! [`SERVICE_LABEL`] and [`SYSTEMD_UNIT`] name the **one** OS service
//! identity for the standalone Reborn binary. Two surfaces install and
//! manage it today: this CLI (`ironclaw-reborn service install`), and the
//! WebUI operator facade (`RebornLocalServiceLifecycle` in
//! `ironclaw_reborn_composition::observability::operator_service_lifecycle`,
//! behind `POST /api/webchat/v2/operator/service`). Both target the same
//! plist/unit path, so an install from either surface atomically replaces
//! whatever the other last wrote there — see [`write_atomic`].
//!
//! The two implementations are not yet unified: `composition` cannot
//! depend on this CLI crate (dependency direction runs the other way), so
//! there is no shared crate to host one implementation today. Until that
//! consolidation lands, this CLI's unit is the target state — it is
//! secret-free (only `HOME`/profile env, no bearer token; `serve` reads
//! the WebUI token from the 0600 token file at start), where the facade's
//! generated unit bakes the WebUI bearer token directly into the unit
//! file's `Environment=` lines. A CLI install replacing a facade-installed
//! unit is therefore a security improvement, not just a collision to
//! avoid.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};

use crate::context::RebornCliContext;
use crate::serve_invocation::serve_invocation;

mod launchd;
mod systemd;

/// launchd label / systemd unit name for the canonical Reborn service
/// identity. Shared, deliberately, with `RebornLocalServiceLifecycle` in
/// `ironclaw_reborn_composition::observability::operator_service_lifecycle`
/// (the WebUI operator-service facade) — see the module doc above. An
/// install from either surface atomically replaces the other's file at
/// this same path; do not fork these constants to "avoid collisions"
/// without updating both sides together.
const SERVICE_LABEL: &str = "com.ironclaw.reborn";
/// See [`SERVICE_LABEL`] — same shared-identity contract, Linux side.
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
    ///
    /// On macOS, stdout/stderr are captured to
    /// `<reborn_home>/logs/serve.std{out,err}.log`. These files are
    /// never rotated by this tool — set up external rotation (e.g.
    /// `newsyslog`) if a long-running install needs it. On Linux, output
    /// goes to the systemd user journal, which the OS already rotates.
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
/// installed, but `serve` starts without config, providers, or a valid
/// WebUI token until `ironclaw-reborn onboard` runs. Checks the
/// onboarding marker, `config.toml`, and the WebUI token file's entropy
/// floor.
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

    if !crate::webui_token::webui_token_file_is_valid(home.path()) {
        warnings.push(format!(
            "WebUI token not found or too short at {} — run `ironclaw-reborn onboard` first so \
             `serve` has a valid WebUI bearer token",
            crate::webui_token::webui_token_file_path(home.path()).display()
        ));
    }

    warnings
}

// ── Restart decision tree ───────────────────────────────────────

/// Shared restart decision tree for both platforms: a running service is
/// stopped then started, an already-stopped-but-installed service is just
/// started, and a start failure after a successful stop is reported as
/// leaving the service stopped rather than a silent half-restart. Each
/// platform does its own installed/running detection, then delegates here.
///
/// `stop`/`start` are function pointers, not closures — closures would
/// each need to capture (and thus mutably borrow) `runner`, but `runner`
/// is already borrowed by this function's own `&mut dyn` parameter, so two
/// closures over it would double-mutably-borrow. Plain `fn` items sidestep
/// that: they capture nothing and take `runner` as an explicit argument.
fn restart_generic(
    runner: &mut dyn ServiceCommandRunner,
    installed: bool,
    was_running: bool,
    stop: fn(&mut dyn ServiceCommandRunner) -> Result<()>,
    start: fn(&mut dyn ServiceCommandRunner) -> Result<()>,
) -> Result<()> {
    if !installed {
        bail!("Service not installed. Run `ironclaw-reborn service install` first.");
    }
    if was_running {
        stop(runner)?;
    }
    if let Err(start_error) = start(runner) {
        if was_running {
            bail!(
                "service restart: stop succeeded but start failed; the service is now \
                 STOPPED: {start_error:#}"
            );
        }
        return Err(start_error);
    }
    println!(
        "{}",
        if was_running {
            "Service restarted"
        } else {
            "Service was not running; started"
        }
    );
    Ok(())
}

// ── Status vocabulary ───────────────────────────────────────────

/// Normalized `service status` line, shared by both platforms so the
/// running/stopped/not-installed vocabulary can't drift between them.
fn status_label(installed: bool, running: bool) -> &'static str {
    if !installed {
        "not installed"
    } else if running {
        "running"
    } else {
        "stopped"
    }
}

// ── Install advisory ────────────────────────────────────────────

/// Advisory line printed after `service install` when a pre-existing
/// unit/plist file at the target path was replaced by this install —
/// `None` when the install wrote a fresh file, nothing to advise. Shared
/// by both platforms so the wording (and the "must `service restart`"
/// guidance) can't drift between them. Covers both a prior install by
/// this same CLI and one by the WebUI operator facade
/// (`RebornLocalServiceLifecycle`, see the module doc): either way the
/// write already happened atomically by the time this is read; a
/// currently-running service process keeps running off the old
/// definition until restarted.
fn replaced_existing_service_file_note(replaced_existing: bool) -> Option<&'static str> {
    replaced_existing.then_some(
        "  Replaced an existing service definition at this path; if the service is currently \
         running, it keeps the OLD definition until `ironclaw-reborn service restart`.",
    )
}

// ── Atomic file writes ──────────────────────────────────────────

static TEMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Write `contents` to `path` via create-temp-then-rename in the same
/// directory, so a reader never observes a partially written file and a
/// crash mid-write never corrupts the previous contents. Used by both
/// platforms' install paths (the systemd unit file and the launchd plist).
fn write_atomic(path: &Path, contents: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .context("service file path has no parent directory")?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("service-file");
    for _ in 0..16 {
        let suffix = TEMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let temp = parent.join(format!(".{file_name}.tmp-{}-{suffix}", std::process::id()));
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            // Service definition files can carry operator env values
            // (`ServeInvocation::env`); keep them unreadable to other
            // local users from the moment they're created, same as
            // `operator_service_lifecycle::write_service_file`.
            options.mode(0o600);
        }
        let mut file = match options.open(&temp) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error).with_context(|| format!("create {}", temp.display())),
        };
        let result = (|| -> Result<()> {
            file.write_all(contents)
                .with_context(|| format!("write {}", temp.display()))?;
            file.sync_all()
                .with_context(|| format!("sync {}", temp.display()))?;
            std::fs::rename(&temp, path).with_context(|| format!("replace {}", path.display()))?;
            Ok(())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&temp);
        }
        return result;
    }
    bail!("could not allocate temporary service file")
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
    fn status_label_covers_running_stopped_and_not_installed() {
        assert_eq!(status_label(false, false), "not installed");
        assert_eq!(status_label(false, true), "not installed");
        assert_eq!(status_label(true, false), "stopped");
        assert_eq!(status_label(true, true), "running");
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
    fn preflight_warnings_empty_when_marker_config_and_token_present() {
        let (_tmp, context) = RebornCliContext::test_context();
        let home = context.boot_config().home();
        std::fs::create_dir_all(home.path()).expect("create home");
        std::fs::write(crate::commands::onboard::onboarding_marker_path(home), "{}")
            .expect("write marker");
        std::fs::write(home.config_file_path(), "").expect("write config");
        std::fs::write(
            crate::webui_token::webui_token_file_path(home.path()),
            "0".repeat(crate::webui_token::WEBUI_TOKEN_MIN_BYTES),
        )
        .expect("write webui token");

        assert!(preflight_warnings(&context).is_empty());
    }

    #[test]
    fn preflight_warnings_flags_missing_marker_config_and_webui_token() {
        let (_tmp, context) = RebornCliContext::test_context();
        let warnings = preflight_warnings(&context);
        assert_eq!(warnings.len(), 3);
        assert!(warnings[0].contains("onboarding marker not found"));
        assert!(warnings[1].contains("config.toml not found"));
        assert!(warnings[2].contains("WebUI token not found or too short"));
    }

    #[cfg(unix)]
    #[test]
    fn write_atomic_creates_file_with_0600_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("service-file");
        write_atomic(&path, b"contents").expect("write succeeds");
        let mode = std::fs::metadata(&path)
            .expect("stat service file")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "service file must be 0600, got {mode:o}");
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
        let fresh_replaced =
            systemd::install_with_runner(&invocation, &mut runner).expect("install must succeed");
        let unit_path = tmp
            .path()
            .join(".config/systemd/user/ironclaw-reborn.service");
        assert!(unit_path.exists(), "unit file must be written");
        let contents = std::fs::read_to_string(&unit_path).expect("read unit file");
        assert!(contents.contains("ExecStart="));
        assert!(contents.contains("IRONCLAW_REBORN_HOME="));
        assert!(
            !fresh_replaced,
            "a fresh install must not report a replaced unit"
        );
        assert!(replaced_existing_service_file_note(fresh_replaced).is_none());

        // Idempotent reinstall: overwrites cleanly, no fail or duplicate —
        // and, per the shared-identity contract with the WebUI operator
        // facade (`RebornLocalServiceLifecycle`), must now report that it
        // replaced an existing unit (this reinstall stands in for the
        // facade's unit being atomically replaced by the CLI's).
        let reinstall_replaced =
            systemd::install_with_runner(&invocation, &mut runner).expect("reinstall must succeed");
        assert!(unit_path.exists());
        assert!(
            reinstall_replaced,
            "reinstalling over an existing unit must report the replacement"
        );
        let note = replaced_existing_service_file_note(reinstall_replaced)
            .expect("a replaced install must carry an advisory line");
        assert!(note.contains("service restart"));

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
            .join("Library/LaunchAgents/com.ironclaw.reborn.plist");
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
