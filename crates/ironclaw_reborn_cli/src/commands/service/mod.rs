//! `ironclaw service` — install/manage the standalone Reborn
//! binary as an OS-native service.
//!
//! - **macOS**: launchd user agent at
//!   `~/Library/LaunchAgents/com.ironclaw.reborn.plist`.
//! - **Linux (systemd)**: systemd user unit at
//!   `~/.config/systemd/user/ironclaw-reborn.service`.
//! - **Linux (container, no systemd)**: no unit to install — `serve` runs
//!   as the container's own supervised process; `restart` signals it
//!   directly and the container runtime's restart policy relaunches it.
//!   See the `container` module doc.
//!
//! The installed service runs `<current_exe> serve`, restarting
//! automatically on failure. Mirrors v1's `src/service.rs` shape with
//! Reborn's label, paths, and env-passthrough contract; no v1 code is
//! shared or reused.
//!
//! Platform dispatch happens once, in [`ServicePlatform::detect`], called
//! a single time from [`ServiceCommand::execute`]. Each verb is a method
//! on [`ServicePlatform`] that `match self`es to delegate into the
//! OS-specific implementation (`launchd`, `systemd`, or `container`),
//! rather than every verb re-checking `cfg!(target_os)`.
//!
//! ## Canonical service identity, shared with the WebUI operator facade
//!
//! [`SERVICE_LABEL`] and [`SYSTEMD_UNIT`] name the **one** OS service
//! identity for the standalone Reborn binary. Two surfaces install and
//! manage it today: this CLI (`ironclaw service install`), and the
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

mod container;
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
            ServiceVerb::Install => platform.install(&context).map(|_warnings| ()),
            ServiceVerb::Start => platform.start(),
            ServiceVerb::Stop => platform.stop(),
            ServiceVerb::Restart => platform.restart(),
            ServiceVerb::Status => platform.status(),
            ServiceVerb::Uninstall => platform.uninstall(),
        }
    }
}

// ── Platform dispatch ───────────────────────────────────────────

/// The supported service-management targets. Detected once per
/// invocation via [`ServicePlatform::detect`]; every verb dispatches
/// off the resolved variant instead of re-checking `cfg!(target_os)`.
///
/// `Container` is Linux without systemd as the running init: hosted
/// deployments where the image entrypoint `exec`s `ironclaw serve` as the
/// container's main process and the container runtime's restart policy is
/// the supervisor (see the `container` module doc). Before this variant,
/// such hosts resolved to `Linux` and every verb died spawning the
/// nonexistent `systemctl`.
///
/// pub(super): `commands::status` queries [`Self::current_state`] to check
/// whether the installed service is actually running, not just present.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ServicePlatform {
    MacOs,
    Linux,
    Container,
}

/// Production procfs root for [`ServicePlatform::Container`] dispatch.
/// `container`'s functions take it as a parameter so tests can point them at
/// a fake proc tree (see `container::tests`).
const PROC_ROOT: &str = "/proc";

impl ServicePlatform {
    pub(super) fn detect() -> Result<Self> {
        if cfg!(target_os = "macos") {
            Ok(Self::MacOs)
        } else if cfg!(target_os = "linux") {
            Ok(Self::linux_platform(
                systemd_booted(),
                container_supervised_declared(),
            ))
        } else {
            bail!(UNSUPPORTED_OS_MESSAGE);
        }
    }

    /// The one runtime decision [`Self::detect`] makes on Linux. `Container`
    /// requires BOTH signals — systemd absent AND `container_supervised`
    /// explicitly declared — not systemd-absence alone: a Linux host can
    /// lack systemd for reasons that have nothing to do with a managed
    /// restart policy (WSL2, an OpenRC distro like Alpine/Gentoo, a
    /// SysV-init host, a plain VM running `ironclaw onboard` directly, or
    /// even a real Docker container started with no restart policy at all —
    /// e.g. the documented `docker run --rm ...` local-run command in
    /// `docs/reborn/deploy-reborn-cli-docker.md`, which is a real container
    /// but has nothing to relaunch `serve` if `restart` kills it).
    ///
    /// `restart` in `Container` mode kills the running `serve` process and
    /// trusts an external restart policy to relaunch it — no signal
    /// observable from inside the container (not `/.dockerenv`, not a
    /// cgroup, not any other ambient marker) can actually prove that policy
    /// exists. Every container host, including Railway, must declare that
    /// policy explicitly. When it is not declared, fall back to `Linux`: that
    /// host gets the old systemd-manager path, which fails loud (a plain
    /// `systemctl` ENOENT) rather than guessing and taking a destructive
    /// action with no recovery guarantee.
    fn linux_platform(systemd_booted: bool, container_supervised_declared: bool) -> Self {
        if systemd_booted {
            Self::Linux
        } else if container_supervised_declared {
            Self::Container
        } else {
            Self::Linux
        }
    }

    /// Production install path: real `launchctl`/`systemctl` commands via
    /// [`OsServiceCommandRunner`].
    fn install(&self, context: &RebornCliContext) -> Result<Vec<String>> {
        self.install_with_runner(context, &mut OsServiceCommandRunner)
    }

    /// Runner-injectable install path shared by production (via
    /// [`Self::install`]) and the `service install` preflight-warning
    /// integration test: computes and prints the same non-fatal
    /// readiness warnings [`Self::install`] always has, then dispatches
    /// to the platform-specific install with the given
    /// [`ServiceCommandRunner`]. Returns the printed warnings (not just
    /// `()`) so a test can assert on their content without reaching for
    /// `preflight_warnings` directly — driving the assertion through the
    /// same call site `service install` actually uses.
    fn install_with_runner(
        &self,
        context: &RebornCliContext,
        runner: &mut dyn ServiceCommandRunner,
    ) -> Result<Vec<String>> {
        // Bail before preflight: warnings about onboarding/config are noise
        // when there is no service manager to install into at all.
        if matches!(self, Self::Container) {
            container::unsupported_in_container("install")?;
        }
        let warnings = preflight_warnings(context)?;
        for warning in &warnings {
            eprintln!("warning: {warning}");
        }
        let invocation = serve_invocation()?;
        match self {
            Self::MacOs => {
                launchd::install_with_runner(context, &invocation, runner).map(|_replaced| ())?
            }
            Self::Linux => {
                systemd::install_with_runner(context, &invocation, runner).map(|_replaced| ())?
            }
            Self::Container => unreachable!("container installs bail before preflight"),
        }
        Ok(warnings)
    }

    fn start(&self) -> Result<()> {
        self.start_with_runner(&mut OsServiceCommandRunner)
    }

    /// Runner-injectable `start`, mirroring [`Self::install_with_runner`]:
    /// lets a test drive the same dispatch `start()`/`ServiceCommand::execute`
    /// use, with a fake [`ServiceCommandRunner`] instead of the host's real
    /// `launchctl`/`systemctl`.
    fn start_with_runner(&self, runner: &mut dyn ServiceCommandRunner) -> Result<()> {
        match self {
            Self::MacOs => launchd::start_with_runner(runner),
            Self::Linux => systemd::start_with_runner(runner),
            Self::Container => container::unsupported_in_container("start"),
        }
    }

    fn stop(&self) -> Result<()> {
        self.stop_with_runner(&mut OsServiceCommandRunner)
    }

    /// Runner-injectable `stop` — see [`Self::start_with_runner`].
    fn stop_with_runner(&self, runner: &mut dyn ServiceCommandRunner) -> Result<()> {
        match self {
            Self::MacOs => launchd::stop_with_runner(runner),
            Self::Linux => systemd::stop_with_runner(runner),
            Self::Container => container::unsupported_in_container("stop"),
        }
    }

    fn restart(&self) -> Result<()> {
        self.restart_with_runner(&mut OsServiceCommandRunner)
    }

    /// Runner-injectable `restart` — see [`Self::start_with_runner`].
    fn restart_with_runner(&self, runner: &mut dyn ServiceCommandRunner) -> Result<()> {
        self.restart_with_runner_at_proc_root(runner, Path::new(PROC_ROOT))
    }

    /// Testable restart dispatch with an injectable procfs root. Production
    /// uses [`PROC_ROOT`]; tests use a fake process table to prove the
    /// `Container` arm reaches the same signal command as a real restart.
    fn restart_with_runner_at_proc_root(
        &self,
        runner: &mut dyn ServiceCommandRunner,
        proc_root: &Path,
    ) -> Result<()> {
        match self {
            Self::MacOs => launchd::restart_with_runner(runner),
            Self::Linux => systemd::restart_with_runner(runner),
            Self::Container => container::restart_with_runner(runner, proc_root),
        }
    }

    fn status(&self) -> Result<()> {
        self.status_with_runner(&mut OsServiceCommandRunner)
    }

    /// Runner-injectable `status` — see [`Self::start_with_runner`].
    fn status_with_runner(&self, runner: &mut dyn ServiceCommandRunner) -> Result<()> {
        match self {
            Self::MacOs => launchd::status_with_runner(runner),
            Self::Linux => systemd::status_with_runner(runner),
            Self::Container => container::status(Path::new(PROC_ROOT)),
        }
    }

    fn uninstall(&self) -> Result<()> {
        self.uninstall_with_runner(&mut OsServiceCommandRunner)
    }

    /// Runner-injectable `uninstall` — see [`Self::start_with_runner`].
    fn uninstall_with_runner(&self, runner: &mut dyn ServiceCommandRunner) -> Result<()> {
        match self {
            Self::MacOs => launchd::uninstall_with_runner(runner),
            Self::Linux => systemd::uninstall_with_runner(runner),
            Self::Container => container::unsupported_in_container("uninstall"),
        }
    }

    /// Production service-state query: real `launchctl`/`systemctl` via
    /// [`OsServiceCommandRunner`]. Used by `commands::status` so `status`
    /// reports the service as actually running, not just installed.
    pub(super) fn current_state(&self) -> Result<ServiceState> {
        self.current_state_with_runner(&mut OsServiceCommandRunner)
    }

    /// Runner-injectable variant of [`Self::current_state`] for tests.
    pub(super) fn current_state_with_runner(
        &self,
        runner: &mut dyn ServiceCommandRunner,
    ) -> Result<ServiceState> {
        match self {
            Self::MacOs => launchd::current_state_with_runner(runner),
            Self::Linux => systemd::current_state_with_runner(runner),
            Self::Container => container::current_state(Path::new(PROC_ROOT)),
        }
    }
}

/// systemd is the running init exactly when `/run/systemd/system` exists —
/// the documented `sd_booted(3)` check. A Linux host where it is absent
/// (hosted containers, minimal images) has no systemd to manage services
/// with, regardless of what unit files are baked into the filesystem.
fn systemd_booted() -> bool {
    systemd_booted_under(Path::new("/"))
}

/// `systemd_booted`'s actual check, parameterized on the filesystem root so
/// tests can point it at a fake tree instead of the host's real `/run`.
fn systemd_booted_under(root: &Path) -> bool {
    root.join("run/systemd/system").is_dir()
}

/// Env var a deployment sets to explicitly declare "I run under a managed
/// restart policy". This deployment contract is provider-neutral: Docker,
/// Railway, Kubernetes, and other container hosts must opt in explicitly.
const CONTAINER_SUPERVISED_ENV_VAR: &str = "IRONCLAW_CONTAINER_SUPERVISE";

/// Whether a deployment declares a managed restart policy. Combined with
/// [`systemd_booted`] absence, this is [`ServicePlatform::linux_platform`]'s
/// second signal for `Container` mode.
fn container_supervised_declared() -> bool {
    is_truthy_env(
        std::env::var_os(CONTAINER_SUPERVISED_ENV_VAR)
            .as_deref()
            .and_then(std::ffi::OsStr::to_str),
    )
}

/// Parses [`CONTAINER_SUPERVISED_ENV_VAR`] without mutating the process
/// environment in tests.
fn is_truthy_env(value: Option<&str>) -> bool {
    matches!(value, Some("1" | "true" | "TRUE" | "yes" | "YES"))
}

/// Install then start the OS service in one call — used by `onboard`'s
/// finale so a fresh install ends with `serve` actually running.
pub(crate) fn install_and_start(context: &RebornCliContext) -> Result<()> {
    let platform = ServicePlatform::detect()?;
    platform.install(context)?;
    platform.start()
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

/// The `serve` process's launched cwd, shared by both platforms' installers.
///
/// - Not the Reborn home itself: composition's default skill/extension
///   roots live under `<reborn_home>/...`, so the home is an *ancestor* of
///   them and trips `paths_overlap` (prefix match, not just equality).
/// - `<reborn_home>/workspace` is a leaf dir, neither ancestor nor
///   descendant of any skill root, so it never overlaps.
fn service_working_directory(reborn_home: &Path) -> PathBuf {
    reborn_home.join("workspace")
}

/// Creates [`service_working_directory`] (0700, `create_dir_all` — a no-op
/// if it already exists) and returns its path. Called by both platforms'
/// `install_with_runner` before writing the unit/plist, so the directory
/// exists by the time `serve` is ever launched with it as cwd.
///
/// 0700, not 0755: this is a single-user local service directory, not a
/// shared or world-readable path, so other local accounts must not be able
/// to list or read it.
fn ensure_service_working_directory(reborn_home: &Path) -> Result<PathBuf> {
    let dir = service_working_directory(reborn_home);
    std::fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))
            .with_context(|| format!("set permissions on {}", dir.display()))?;
    }
    Ok(dir)
}

// ── Preflight ───────────────────────────────────────────────────

/// Non-fatal readiness warnings for `service install`: warn, don't
/// fail, when onboarding hasn't run yet — the service can still be
/// installed, but `serve` starts without config, providers, or a valid
/// WebUI token until `ironclaw onboard` runs. Checks the
/// onboarding marker, `config.toml`, and the WebUI token file's entropy
/// floor.
///
/// Fallible: a real I/O error reading the token file (unreadable,
/// symlinked, oversized — see `webui_token::webui_token_file_is_valid`)
/// is a genuine problem distinct from "token absent/too short" and must
/// surface as an `install` error rather than being folded into the same
/// generic warning text.
fn preflight_warnings(context: &RebornCliContext) -> Result<Vec<String>> {
    let home = context.boot_config().home();
    let mut warnings = Vec::new();

    let marker = crate::commands::onboard::onboarding_marker_path(home);
    if !marker.exists() {
        warnings.push(format!(
            "onboarding marker not found at {} — run `ironclaw onboard` first so \
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

    if !crate::webui_token::webui_token_file_is_valid(home.path())? {
        warnings.push(format!(
            "WebUI token not found or too short at {} — run `ironclaw onboard` first so \
             `serve` has a valid WebUI bearer token",
            crate::webui_token::webui_token_file_path(home.path()).display()
        ));
    }

    Ok(warnings)
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
        bail!("Service not installed. Run `ironclaw service install` first.");
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

/// Normalized service lifecycle state, shared by both platforms and by
/// `commands::status` (see [`ServicePlatform::current_state`]) so the
/// running/stopped/not-installed vocabulary can't drift between the
/// `service status` text output and the `status` command's `service`
/// field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ServiceState {
    NotInstalled,
    Stopped,
    Running,
}

impl ServiceState {
    fn from_installed_running(installed: bool, running: bool) -> Self {
        if !installed {
            Self::NotInstalled
        } else if running {
            Self::Running
        } else {
            Self::Stopped
        }
    }
}

/// Normalized `service status` line, shared by both platforms so the
/// running/stopped/not-installed vocabulary can't drift between them.
fn status_label(installed: bool, running: bool) -> &'static str {
    match ServiceState::from_installed_running(installed, running) {
        ServiceState::NotInstalled => "not installed",
        ServiceState::Stopped => "stopped",
        ServiceState::Running => "running",
    }
}

// ── Install advisory ────────────────────────────────────────────

/// Advisory line printed after `service install` when a pre-existing
/// unit/plist file at the target path was replaced by this install, and
/// a currently-running service process still keeps running off the OLD
/// definition until `service restart` — `None` when there is nothing to
/// advise. Shared by both platforms so the wording (and the "must
/// `service restart`" guidance) can't drift between them. Covers both a
/// prior install by this same CLI and one by the WebUI operator facade
/// (`RebornLocalServiceLifecycle`, see the module doc).
///
/// Callers decide whether the "keeps the OLD definition" claim is true
/// for their platform and pass that pre-resolved bool in: systemd never
/// reloads a running unit as part of `install` (`daemon-reload` alone
/// does not restart the service), so a replaced unit always leaves a
/// running process on the old definition and `systemd::install_with_runner`
/// passes `replaced_existing` through unchanged. launchd's `install`
/// forces an unload/load/start reload when the label was already loaded
/// (see `launchd::install_with_runner`), which makes the new definition
/// live immediately — so `launchd::install_with_runner` passes
/// `replaced_existing && !was_loaded`, suppressing the note exactly when
/// it would be false.
fn replaced_existing_service_file_note(replaced_existing: bool) -> Option<&'static str> {
    replaced_existing.then_some(
        "  Replaced an existing service definition at this path; if the service is currently \
         running, it keeps the OLD definition until `ironclaw service restart`.",
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
///
/// `pub(super)`: `commands::status`'s tests implement this trait with their
/// own mock runner to drive [`ServicePlatform::current_state_with_runner`]
/// hermetically for all three [`ServiceState`] outcomes.
pub(super) trait ServiceCommandRunner {
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
    struct SuccessfulServiceCommandRunner {
        /// Labels of every command run through this stub, in call order.
        /// Used by [`service_verbs_dispatch_through_the_injectable_runner_path`]
        /// to prove a verb actually reached the injected runner rather than
        /// bailing out before calling it (e.g. an "not installed" short
        /// circuit).
        labels: Vec<String>,
    }

    impl ServiceCommandRunner for SuccessfulServiceCommandRunner {
        fn run_checked(&mut self, label: &str, _command: &mut Command) -> Result<()> {
            self.labels.push(label.to_string());
            Ok(())
        }

        fn run_capture_checked(&mut self, label: &str, _command: &mut Command) -> Result<String> {
            self.labels.push(label.to_string());
            match label {
                "launchctl list" => Ok(String::new()),
                "id -u" => Ok("501\n".to_string()),
                // `install_with_runner` (both platforms) now queries unit
                // state before writing the unit/plist file (see
                // `systemd::query_unit_state`). The strict `Key=Value`
                // parser errors on a blank/malformed response rather than
                // silently defaulting to "not loaded" (a malformed
                // response must never read as `enabled=false`, which
                // would make an install-failure rollback skip re-enabling
                // a unit that actually was enabled) — so this mock must
                // return well-formed output, mirroring the dedicated
                // systemd tests' `unit_state_output` pattern.
                "systemctl show unit state" => {
                    Ok("LoadState=loaded\nUnitFileState=enabled\n".to_string())
                }
                _ => Ok(String::new()),
            }
        }
    }

    #[derive(Default)]
    struct RecordingServiceCommandRunner {
        commands: Vec<(String, String, Vec<String>)>,
    }

    impl ServiceCommandRunner for RecordingServiceCommandRunner {
        fn run_checked(&mut self, label: &str, command: &mut Command) -> Result<()> {
            self.commands.push((
                label.to_string(),
                command.get_program().to_string_lossy().into_owned(),
                command
                    .get_args()
                    .map(|arg| arg.to_string_lossy().into_owned())
                    .collect(),
            ));
            Ok(())
        }

        fn run_capture_checked(&mut self, label: &str, command: &mut Command) -> Result<String> {
            self.run_checked(label, command)?;
            Ok(String::new())
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
        // This test only runs in CI/dev on macOS or Linux (possibly inside a
        // systemd-less container), so detect() must resolve to one of the
        // supported variants, never bail.
        let platform = ServicePlatform::detect().expect("detect must resolve on macOS/Linux");
        assert!(matches!(
            platform,
            ServicePlatform::MacOs | ServicePlatform::Linux | ServicePlatform::Container
        ));
    }

    #[test]
    fn linux_without_systemd_and_with_supervision_declared_resolves_to_container() {
        // Regression: hosted containers (image entrypoint execs `ironclaw
        // serve`, PID 1 = docker-init, no systemctl binary) used to resolve
        // to `Linux`, so every service verb died spawning `systemctl`
        // ("failed to spawn command for systemctl show unit state") and
        // `restart` reported "Service not installed" — while the setup docs
        // tell users to finish with `ironclaw service restart`.
        assert_eq!(
            ServicePlatform::linux_platform(false, true),
            ServicePlatform::Container
        );
        assert_eq!(
            ServicePlatform::linux_platform(true, true),
            ServicePlatform::Linux
        );
        assert_eq!(
            ServicePlatform::linux_platform(true, false),
            ServicePlatform::Linux
        );
    }

    #[test]
    fn linux_without_systemd_and_without_supervision_declared_falls_back_to_linux() {
        // Systemd-absence alone must NOT resolve to Container: a host that
        // is neither systemd-managed nor has explicitly declared a managed
        // restart policy (WSL2, an OpenRC distro, a SysV-init host, a plain
        // VM running `ironclaw onboard` directly, or even a real Docker
        // container started with no restart policy at all — e.g. the
        // documented `docker run --rm ...` local-run command) must NOT get
        // Container's destructive restart (kill + trust an external restart
        // policy) — it falls back to the old Linux/systemd path, which
        // fails loud (a plain `systemctl` ENOENT) instead of guessing and
        // taking a destructive action with no recovery guarantee.
        assert_eq!(
            ServicePlatform::linux_platform(false, false),
            ServicePlatform::Linux
        );
    }

    #[test]
    fn container_supervision_requires_an_explicit_truthy_declaration() {
        for truthy in ["1", "true", "TRUE", "yes", "YES"] {
            assert!(is_truthy_env(Some(truthy)), "{truthy} must opt in");
        }
        for falsey in [None, Some(""), Some("false"), Some("railway")] {
            assert!(
                !is_truthy_env(falsey),
                "{falsey:?} must not opt in without an explicit truthy value"
            );
        }
    }

    #[test]
    fn container_restart_dispatches_the_serve_signal_through_the_runner() {
        let proc = tempfile::tempdir().expect("tempdir");
        let serve_dir = proc.path().join("71");
        std::fs::create_dir_all(&serve_dir).expect("create fake proc entry");
        std::fs::write(
            serve_dir.join("cmdline"),
            b"/usr/local/bin/ironclaw\0serve\0--host\0",
        )
        .expect("write fake serve command line");
        let mut runner = RecordingServiceCommandRunner::default();

        ServicePlatform::Container
            .restart_with_runner_at_proc_root(&mut runner, proc.path())
            .expect("container restart must dispatch through the runner");

        assert_eq!(
            runner.commands,
            vec![(
                "terminate serve process".to_string(),
                "sh".to_string(),
                vec!["-c".to_string(), "kill -TERM 71".to_string()],
            )]
        );
    }

    #[test]
    fn systemd_booted_under_reads_run_systemd_system_from_the_given_root() {
        // The detection tests above exercise only the pre-computed bools;
        // this proves the actual filesystem check that feeds one of them
        // (the real bug-fix logic) reads the right path and both directions.
        let tmp = tempfile::tempdir().expect("tempdir");
        assert!(
            !systemd_booted_under(tmp.path()),
            "no run/systemd/system dir must read as not booted"
        );
        std::fs::create_dir_all(tmp.path().join("run/systemd/system"))
            .expect("create run/systemd/system");
        assert!(
            systemd_booted_under(tmp.path()),
            "run/systemd/system dir must read as booted"
        );
    }

    #[test]
    fn is_truthy_env_matches_entrypoint_sh_is_truthy_exactly() {
        // Must stay in sync with docker/reborn/entrypoint.sh's is_truthy():
        // a value the entrypoint treats as true (e.g. when auto-declaring
        // supervision on Railway) must read as true here too.
        for truthy in ["1", "true", "TRUE", "yes", "YES"] {
            assert!(is_truthy_env(Some(truthy)), "{truthy} must be truthy");
        }
        for falsy in [
            None,
            Some(""),
            Some("0"),
            Some("false"),
            Some("no"),
            Some("garbage"),
        ] {
            assert!(!is_truthy_env(falsy), "{falsy:?} must not be truthy");
        }
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

        assert!(
            preflight_warnings(&context)
                .expect("preflight_warnings must succeed")
                .is_empty()
        );
    }

    #[test]
    fn preflight_warnings_flags_missing_marker_config_and_webui_token() {
        let (_tmp, context) = RebornCliContext::test_context();
        let warnings =
            preflight_warnings(&context).expect("preflight_warnings must succeed when absent");
        assert_eq!(warnings.len(), 3);
        assert!(warnings[0].contains("onboarding marker not found"));
        assert!(warnings[1].contains("config.toml not found"));
        assert!(warnings[2].contains("WebUI token not found or too short"));
    }

    #[test]
    fn install_with_runner_surfaces_missing_webui_token_warning_through_the_real_path() {
        // Per the repo's "test through the caller" rule: a unit test on
        // `preflight_warnings` alone doesn't prove `service install`
        // actually calls it with the right inputs and doesn't swallow
        // the result. Drive `ServicePlatform::install_with_runner` (the
        // same call `install()`/`ServiceCommand::execute` use) with a
        // fake runner, marker + config present, and the WebUI token
        // absent, and assert the token warning comes back from the real
        // path.
        let _lock = crate::runtime::test_env::lock_runtime_env();
        let home_tmp = tempfile::tempdir().expect("tempdir");
        let _home = TempHomeGuard::set(home_tmp.path());
        let (_ctx_tmp, context) = RebornCliContext::test_context();
        let reborn_home = context.boot_config().home();
        std::fs::create_dir_all(reborn_home.path()).expect("create reborn home");
        std::fs::write(
            crate::commands::onboard::onboarding_marker_path(reborn_home),
            "{}",
        )
        .expect("write marker");
        std::fs::write(reborn_home.config_file_path(), "").expect("write config");
        // WebUI token deliberately absent.

        let platform = ServicePlatform::detect().expect("detect must resolve on macOS/Linux");
        let mut runner = SuccessfulServiceCommandRunner::default();
        let warnings = platform
            .install_with_runner(&context, &mut runner)
            .expect("install must succeed even with the token warning outstanding");

        assert_eq!(
            warnings.len(),
            1,
            "marker and config are present; only the token warning should fire: {warnings:?}"
        );
        assert!(
            warnings[0].contains("WebUI token not found or too short"),
            "warnings: {warnings:?}"
        );
    }

    #[test]
    fn service_verbs_dispatch_through_the_injectable_runner_path() {
        // Before this fix, only `install` had a runner-injectable split
        // (`install_with_runner`, exercised above) — `start`/`stop`/
        // `restart`/`status`/`uninstall` existed only as `pub(super) fn
        // verb()`, hardcoding `OsServiceCommandRunner` with no way to drive
        // `ServiceCommand`'s dispatch wiring itself without touching the
        // real service manager. This parses each verb via clap (the same
        // way `ServiceCommand` does) and drives it through
        // `ServicePlatform`'s new `*_with_runner` methods with a stub
        // runner, proving the dispatch wiring end to end. It deliberately
        // does NOT re-assert each verb's own behavior (installed/running
        // detection, rollback, etc.) — that's already covered by the
        // dedicated per-platform tests in systemd.rs and launchd.rs.
        let _lock = crate::runtime::test_env::lock_runtime_env();
        let tmp = tempfile::tempdir().expect("tempdir");
        let _home = TempHomeGuard::set(tmp.path());
        let context = RebornCliContext::from_boot_config(
            ironclaw_reborn_config::RebornBootConfig::resolve_from_env()
                .expect("boot config must resolve under temp HOME"),
        );
        let platform = ServicePlatform::detect().expect("detect must resolve on macOS/Linux");
        let mut runner = SuccessfulServiceCommandRunner::default();
        platform.install_with_runner(&context, &mut runner).expect(
            "install must succeed so start/stop/restart/status/uninstall have a unit to act on",
        );

        use clap::Parser;

        #[derive(clap::Parser)]
        struct VerbParser {
            #[command(subcommand)]
            verb: ServiceVerb,
        }

        for (args, verb_name) in [
            (["ironclaw", "start"], "start"),
            (["ironclaw", "stop"], "stop"),
            (["ironclaw", "restart"], "restart"),
            (["ironclaw", "status"], "status"),
            (["ironclaw", "uninstall"], "uninstall"),
        ] {
            let parsed = VerbParser::try_parse_from(args)
                .unwrap_or_else(|e| panic!("{verb_name} must parse via clap: {e}"));
            let before = runner.labels.len();
            let result = match parsed.verb {
                ServiceVerb::Install => unreachable!("install is not under test here"),
                ServiceVerb::Start => platform.start_with_runner(&mut runner),
                ServiceVerb::Stop => platform.stop_with_runner(&mut runner),
                ServiceVerb::Restart => platform.restart_with_runner(&mut runner),
                ServiceVerb::Status => platform.status_with_runner(&mut runner),
                ServiceVerb::Uninstall => platform.uninstall_with_runner(&mut runner),
            };
            result.unwrap_or_else(|e| {
                panic!("{verb_name} through the injected runner must succeed: {e:#}")
            });
            assert!(
                runner.labels.len() > before,
                "{verb_name} must reach the injected runner, not bail out before calling it"
            );
        }
    }

    #[test]
    fn container_platform_dispatches_service_manager_verbs_into_container_module() {
        // Covers the install/start/stop/uninstall match arms only — these
        // reject unconditionally with no I/O, so they're safe to drive
        // directly against `ServicePlatform::Container` here. Proves wiring
        // only: `container::unsupported_verbs_share_one_actionable_message`
        // already pins the message content; this just proves each arm
        // actually reaches `container::` instead of falling through to
        // launchd/systemd, and that the runner is never touched.
        //
        // NOT covered here: the restart/status/current_state match arms,
        // which route through the hardcoded `PROC_ROOT = "/proc"` constant
        // and so aren't independently fakeable at this dispatch layer
        // without threading a proc root through `ServicePlatform` itself.
        // Their underlying logic is fully covered directly against a fake
        // proc tree in `container::tests`; only the one-line match arms
        // that route to it are unverified here.
        let (_tmp, context) = RebornCliContext::test_context();
        let platform = ServicePlatform::Container;
        let mut runner = SuccessfulServiceCommandRunner::default();
        for result in [
            platform.install_with_runner(&context, &mut runner).err(),
            platform.start_with_runner(&mut runner).err(),
            platform.stop_with_runner(&mut runner).err(),
            platform.uninstall_with_runner(&mut runner).err(),
        ] {
            assert!(result.is_some_and(|e| e.to_string().contains("container runtime")));
        }
        assert!(runner.labels.is_empty(), "must never reach the runner");
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
    /// at a tempdir, clearing IRONCLAW_REBORN_HOME so the derived
    /// Reborn home nests under the same tempdir (RebornHome falls back
    /// to `$HOME/.ironclaw/reborn` when unset), and clearing
    /// `$XDG_CONFIG_HOME` so `systemd::unit_path()` resolves under the
    /// tempdir too instead of a real XDG path that may already exist on
    /// the host (CI runners have been observed setting
    /// `XDG_CONFIG_HOME=$HOME/.config`, which `config_home()` correctly
    /// prefers over `$HOME`). Restores all three on drop. Caller must
    /// hold `lock_runtime_env()`.
    struct TempHomeGuard {
        prior_home: Option<std::ffi::OsString>,
        prior_reborn_home: Option<std::ffi::OsString>,
        prior_xdg: Option<std::ffi::OsString>,
    }

    impl TempHomeGuard {
        fn set(tmp: &std::path::Path) -> Self {
            let prior_home = std::env::var_os("HOME");
            let prior_reborn_home = std::env::var_os("IRONCLAW_REBORN_HOME");
            let prior_xdg = std::env::var_os("XDG_CONFIG_HOME");
            // SAFETY: caller holds `lock_runtime_env()` for this guard's lifetime.
            unsafe {
                std::env::set_var("HOME", tmp);
                std::env::remove_var("IRONCLAW_REBORN_HOME");
                std::env::remove_var("XDG_CONFIG_HOME");
            }
            Self {
                prior_home,
                prior_reborn_home,
                prior_xdg,
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
                match self.prior_xdg.take() {
                    Some(v) => std::env::set_var("XDG_CONFIG_HOME", v),
                    None => std::env::remove_var("XDG_CONFIG_HOME"),
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
        let context = RebornCliContext::from_boot_config(
            ironclaw_reborn_config::RebornBootConfig::resolve_from_env()
                .expect("boot config must resolve under temp HOME"),
        );
        let invocation = serve_invocation().expect("serve invocation");
        let mut runner = SuccessfulServiceCommandRunner::default();
        let fresh_replaced = systemd::install_with_runner(&context, &invocation, &mut runner)
            .expect("install must succeed");
        let unit_path = tmp
            .path()
            .join(".config/systemd/user/ironclaw-reborn.service");
        assert!(unit_path.exists(), "unit file must be written");
        let contents = std::fs::read_to_string(&unit_path).expect("read unit file");
        assert!(contents.contains("ExecStart="));
        assert!(contents.contains("IRONCLAW_REBORN_HOME="));
        let reborn_home = context.boot_config().home().path().to_path_buf();
        let expected_working_directory = reborn_home.join("workspace");
        assert!(
            contents.contains(&format!(
                "WorkingDirectory=\"{}\"",
                expected_working_directory.display()
            )),
            "unit file must anchor cwd at <reborn_home>/workspace, not the Reborn home itself \
             (the Reborn home is an ancestor of every default skill root, so cwd=reborn_home \
             still trips composition's overlap check — the crash-loop persisted after the first \
             attempt at this fix): {contents}"
        );
        assert!(
            expected_working_directory.is_dir(),
            "install must create <reborn_home>/workspace so `serve` has somewhere to cd into"
        );
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&expected_working_directory)
                .expect("stat working directory")
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o700, "working directory must be 0700, got {mode:o}");
        }
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
        let reinstall_replaced = systemd::install_with_runner(&context, &invocation, &mut runner)
            .expect("reinstall must succeed");
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
        let invocation = serve_invocation().expect("serve invocation");
        let mut runner = SuccessfulServiceCommandRunner::default();

        launchd::install_with_runner(&context, &invocation, &mut runner)
            .expect("install must succeed");
        let plist_path = tmp
            .path()
            .join("Library/LaunchAgents/com.ironclaw.reborn.plist");
        assert!(plist_path.exists(), "plist file must be written");
        let contents = std::fs::read_to_string(&plist_path).expect("read plist file");
        assert!(contents.contains(SERVICE_LABEL));
        assert!(contents.contains("<key>IRONCLAW_REBORN_HOME</key>"));
        let reborn_home = context.boot_config().home().path().to_path_buf();
        let expected_working_directory = reborn_home.join("workspace");
        assert!(
            contents.contains(&format!(
                "<string>{}</string>",
                expected_working_directory.display()
            )) && contents.contains("<key>WorkingDirectory</key>"),
            "plist must anchor cwd at <reborn_home>/workspace, not the Reborn home itself \
             (the Reborn home is an ancestor of every default skill root, so cwd=reborn_home \
             still trips composition's overlap check — the crash-loop persisted after the first \
             attempt at this fix): {contents}"
        );
        assert!(
            expected_working_directory.is_dir(),
            "install must create <reborn_home>/workspace so `serve` has somewhere to cd into"
        );
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&expected_working_directory)
                .expect("stat working directory")
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o700, "working directory must be 0700, got {mode:o}");
        }

        // Idempotent reinstall.
        launchd::install_with_runner(&context, &invocation, &mut runner)
            .expect("reinstall must succeed");
        assert!(plist_path.exists());

        launchd::uninstall_with_runner(&mut runner).expect("uninstall must succeed");
        assert!(!plist_path.exists(), "plist file must be removed");
    }
}
