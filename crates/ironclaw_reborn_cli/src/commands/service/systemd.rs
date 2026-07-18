//! Linux systemd user-unit generators, path resolution, and verb
//! bodies for `ironclaw service`.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::context::RebornCliContext;
use crate::serve_invocation::ServeInvocation;

use super::{SYSTEMD_UNIT, ServiceCommandRunner, home_dir};

// ── Quoting ─────────────────────────────────────────────────────

fn unit_quote(value: &str, escape_dollar: bool) -> Result<String> {
    let mut escaped = String::with_capacity(value.len() + 2);
    escaped.push('"');
    for character in value.chars() {
        match character {
            '\0' => bail!("systemd unit value contains NUL"),
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            '%' => escaped.push_str("%%"),
            '$' if escape_dollar => escaped.push_str("$$"),
            character if character.is_control() => {
                bail!("systemd unit value contains unsupported control character")
            }
            character => escaped.push(character),
        }
    }
    escaped.push('"');
    Ok(escaped)
}

// ── Unit generation ─────────────────────────────────────────────

fn unit_content(invocation: &ServeInvocation, working_directory: &Path) -> Result<String> {
    let environment_lines = invocation
        .env
        .iter()
        .map(|(key, value)| {
            unit_quote(&format!("{key}={value}"), false)
                .map(|value| format!("Environment={value}\n"))
        })
        .collect::<Result<String>>()?;

    let exec_start_args = std::iter::once(invocation.exe.display().to_string())
        .chain(invocation.args.iter().cloned())
        .map(|value| unit_quote(&value, true))
        .collect::<Result<Vec<_>>>()?
        .join(" ");

    // WorkingDirectory anchors cwd at `<reborn_home>/workspace`, not
    // systemd's default and not the Reborn home itself — the home is an
    // ancestor of every default skill root, so it still trips
    // composition's `paths_overlap` check (see `service_working_directory`).
    let working_directory = unit_quote(&working_directory.display().to_string(), false)?;

    Ok(format!(
        "[Unit]\n\
         Description=IronClaw Reborn daemon\n\
         After=network.target\n\
         \n\
         [Service]\n\
         Type=simple\n\
         WorkingDirectory={working_directory}\n\
         {environment_lines}\
         ExecStart={exec_start_args}\n\
         Restart=always\n\
         RestartSec=3\n\
         \n\
         [Install]\n\
         WantedBy=default.target\n",
    ))
}

fn restore_previous_unit(path: &Path, previous: Option<&[u8]>) -> Result<()> {
    match previous {
        Some(contents) => super::write_atomic(path, contents),
        None if path.exists() => {
            std::fs::remove_file(path).with_context(|| format!("remove {}", path.display()))
        }
        None => Ok(()),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SystemdUnitState {
    loaded: bool,
    enabled: bool,
}

/// Query `LoadState`/`UnitFileState` and parse them as `Key=Value` lines
/// (no `--value`, so the output is self-describing and order-independent
/// — `--value` alone would print bare values in property-declaration
/// order, which is undocumented and not guaranteed to be stable across
/// systemd versions). A required key missing from the output is an
/// error, not a silently-assumed default: a malformed/truncated response
/// must not read as `enabled=false`, which would make an install-failure
/// rollback skip re-enabling a unit that actually was enabled.
fn query_unit_state(runner: &mut dyn ServiceCommandRunner) -> Result<SystemdUnitState> {
    let output = runner.run_capture_checked(
        "systemctl show unit state",
        Command::new("systemctl").args([
            "--user",
            "show",
            "--property=LoadState",
            "--property=UnitFileState",
            SYSTEMD_UNIT,
        ]),
    )?;
    let mut load_state = None;
    let mut unit_file_state = None;
    for line in output.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key {
            "LoadState" => load_state = Some(value.trim()),
            "UnitFileState" => unit_file_state = Some(value.trim()),
            _ => {}
        }
    }
    let load_state = load_state.context("systemctl show output missing LoadState=... line")?;
    let unit_file_state =
        unit_file_state.context("systemctl show output missing UnitFileState=... line")?;
    Ok(SystemdUnitState {
        loaded: !matches!(load_state, "" | "not-found"),
        enabled: matches!(
            unit_file_state,
            "enabled" | "enabled-runtime" | "linked" | "linked-runtime" | "alias"
        ),
    })
}

/// Combines a primary operation failure with any errors hit while rolling
/// back its partial effects. The primary is kept as the error's `source()`
/// (via `.context()`) rather than flattened into a single formatted string,
/// so an operator inspecting the chain (`{:?}`/`{:#}`/`.source()`) can still
/// see the underlying cause (e.g. permission-denied vs. disk-full) beneath
/// the rollback outcome, instead of losing it inside one opaque message.
fn combined_failure(primary: anyhow::Error, rollback_errors: Vec<String>) -> anyhow::Error {
    if rollback_errors.is_empty() {
        primary
    } else {
        primary.context(format!("rollback failures: {}", rollback_errors.join("; ")))
    }
}

// ── Path helpers ────────────────────────────────────────────────

fn unit_path() -> Result<PathBuf> {
    Ok(config_home()?
        .join("systemd")
        .join("user")
        .join(SYSTEMD_UNIT))
}

/// The base config directory for the user systemd unit search path, per
/// systemd's own rule: prefer `$XDG_CONFIG_HOME` when it is set and
/// non-empty, otherwise fall back to `$HOME/.config` (systemd's
/// documented default when `XDG_CONFIG_HOME` is unset). An empty
/// `XDG_CONFIG_HOME` is treated as unset, matching the XDG base directory
/// spec.
fn config_home() -> Result<PathBuf> {
    match std::env::var_os("XDG_CONFIG_HOME") {
        Some(value) if !value.is_empty() => {
            let path = PathBuf::from(value);
            if !path.is_absolute() {
                bail!("XDG_CONFIG_HOME must be an absolute path to manage an OS service");
            }
            Ok(path)
        }
        _ => Ok(home_dir()?.join(".config")),
    }
}

// ── Verb bodies ─────────────────────────────────────────────────

/// Returns whether a pre-existing unit file at the target path was
/// replaced by this install (captured before the write). Exposed for
/// tests, and for `super::ServicePlatform::install_with_runner` (the
/// runner-injectable install path driven by the `service install`
/// preflight-warning integration test); production reaches this through
/// [`super::ServicePlatform::install`], which discards the bool once the
/// advisory line has been printed.
pub(super) fn install_with_runner(
    context: &RebornCliContext,
    invocation: &ServeInvocation,
    runner: &mut dyn ServiceCommandRunner,
) -> Result<bool> {
    let file = unit_path()?;
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let previous = match std::fs::read(&file) {
        Ok(contents) => Some(contents),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error).with_context(|| format!("read {}", file.display())),
    };
    // Captured before the write: a pre-existing file at this path may
    // have been installed by this CLI's own prior run, or by the WebUI
    // operator facade (`RebornLocalServiceLifecycle`) — both surfaces
    // target the same unit name/path by design (see the module doc). The
    // write below atomically replaces it.
    let replaced_existing = previous.is_some();
    let reborn_home = context.boot_config().home().path();
    let working_directory = super::ensure_service_working_directory(reborn_home)?;
    let unit = unit_content(invocation, &working_directory)?;
    let previous_state = query_unit_state(runner)?;
    super::write_atomic(&file, unit.as_bytes())?;
    if let Err(error) = runner.run_checked(
        "systemctl daemon-reload",
        Command::new("systemctl").args(["--user", "daemon-reload"]),
    ) {
        let mut rollback_errors = Vec::new();
        if let Err(rollback) = restore_previous_unit(&file, previous.as_deref()) {
            rollback_errors.push(format!("restore unit: {rollback:#}"));
        }
        if let Err(rollback) = runner.run_checked(
            "systemctl rollback daemon-reload",
            Command::new("systemctl").args(["--user", "daemon-reload"]),
        ) {
            rollback_errors.push(format!("reload restored unit: {rollback:#}"));
        }
        return Err(combined_failure(error, rollback_errors));
    }
    if let Err(error) = runner.run_checked(
        "systemctl enable",
        Command::new("systemctl").args(["--user", "enable", SYSTEMD_UNIT]),
    ) {
        let mut rollback_errors = Vec::new();
        if let Err(rollback) = runner.run_checked(
            "systemctl rollback disable",
            Command::new("systemctl").args(["--user", "disable", SYSTEMD_UNIT]),
        ) {
            rollback_errors.push(format!("disable partially enabled unit: {rollback:#}"));
        }
        if let Err(rollback) = restore_previous_unit(&file, previous.as_deref()) {
            rollback_errors.push(format!("restore unit: {rollback:#}"));
        }
        if let Err(rollback) = runner.run_checked(
            "systemctl rollback daemon-reload",
            Command::new("systemctl").args(["--user", "daemon-reload"]),
        ) {
            rollback_errors.push(format!("reload restored unit: {rollback:#}"));
        }
        if previous_state.enabled
            && let Err(rollback) = runner.run_checked(
                "systemctl rollback enable previous unit",
                Command::new("systemctl").args(["--user", "enable", SYSTEMD_UNIT]),
            )
        {
            rollback_errors.push(format!("re-enable previous unit: {rollback:#}"));
        }
        return Err(combined_failure(error, rollback_errors));
    }
    println!("Installed systemd user service: {}", file.display());
    if let Some(note) = super::replaced_existing_service_file_note(replaced_existing) {
        println!("{note}");
    }
    println!("  Start with: ironclaw service start");
    Ok(replaced_existing)
}

pub(super) fn start_with_runner(runner: &mut dyn ServiceCommandRunner) -> Result<()> {
    start_with_runner_impl(runner, true)
}

/// Same start sequence, but without the "Service started" line — used by
/// [`restart_with_runner`] via [`super::restart_generic`], which prints its
/// own single restart summary instead of letting the inner start/stop calls
/// print their own lines too.
fn start_with_runner_quiet(runner: &mut dyn ServiceCommandRunner) -> Result<()> {
    start_with_runner_impl(runner, false)
}

fn start_with_runner_impl(runner: &mut dyn ServiceCommandRunner, verbose: bool) -> Result<()> {
    if !unit_path()?.exists() {
        bail!("Service not installed. Run `ironclaw service install` first.");
    }
    runner.run_checked(
        "systemctl daemon-reload",
        Command::new("systemctl").args(["--user", "daemon-reload"]),
    )?;
    runner.run_checked(
        "systemctl start",
        Command::new("systemctl").args(["--user", "start", SYSTEMD_UNIT]),
    )?;
    if verbose {
        println!("Service started");
    }
    Ok(())
}

pub(super) fn stop_with_runner(runner: &mut dyn ServiceCommandRunner) -> Result<()> {
    stop_with_runner_impl(runner, true)
}

/// Same stop sequence, but without the "Service stopped" line — see
/// [`start_with_runner_quiet`].
fn stop_with_runner_quiet(runner: &mut dyn ServiceCommandRunner) -> Result<()> {
    stop_with_runner_impl(runner, false)
}

fn stop_with_runner_impl(runner: &mut dyn ServiceCommandRunner, verbose: bool) -> Result<()> {
    if !unit_path()?.exists() {
        // The unit file is gone, but it may still be an orphan: removed
        // out-of-band while systemd still shows it loaded/enabled (mirrors
        // the manager-state check `status`/`uninstall` already do via
        // `resolve_installed`/`query_unit_state`). Only skip the stop when
        // the manager also shows nothing.
        let unit_state = query_unit_state(runner)?;
        if !unit_state.loaded && !unit_state.enabled {
            if verbose {
                println!("Service stopped");
            }
            return Ok(());
        }
    }
    runner.run_checked(
        "systemctl stop",
        Command::new("systemctl").args(["--user", "stop", SYSTEMD_UNIT]),
    )?;
    if verbose {
        println!("Service stopped");
    }
    Ok(())
}

/// Detects install/running state, then delegates the stop/start decision
/// tree to [`super::restart_generic`], which both platforms share.
pub(super) fn restart_with_runner(runner: &mut dyn ServiceCommandRunner) -> Result<()> {
    let installed = unit_path()?.exists();
    let was_running = if installed {
        let active_state = runner.run_capture_checked(
            "systemctl show ActiveState",
            Command::new("systemctl").args([
                "--user",
                "show",
                "--property=ActiveState",
                "--value",
                SYSTEMD_UNIT,
            ]),
        )?;
        active_state.trim() == "active"
    } else {
        false
    };
    super::restart_generic(
        runner,
        installed,
        was_running,
        stop_with_runner_quiet,
        start_with_runner_quiet,
    )
}

/// Secondary detail line for a non-`active` raw `ActiveState`, e.g. a
/// crashed unit (`failed`) versus one that was simply never started
/// (`inactive`). `None` when the raw state doesn't warrant a detail line
/// (unit is `active`; line 1 already says `running`).
///
/// Chosen rule: always show the raw state when not running — simplest and
/// most minimal; no allowlist of "interesting" states to keep in sync with
/// systemd's ActiveState vocabulary (active/reloading/inactive/failed/
/// activating/deactivating).
fn systemd_status_detail(raw_state: &str) -> Option<String> {
    let trimmed = raw_state.trim();
    if trimmed == "active" {
        None
    } else {
        Some(format!("  systemd ActiveState: {trimmed}"))
    }
}

/// Whether `service status` should report the service as installed:
/// either the unit file exists, or systemd still shows it loaded or
/// enabled (an orphan left behind after the unit file was removed
/// out-of-band).
fn resolve_installed(file_exists: bool, unit_state: SystemdUnitState) -> bool {
    file_exists || unit_state.loaded || unit_state.enabled
}

/// Installed/running state (plus the raw `ActiveState` detail) shared by
/// [`status_with_runner`] and [`current_state_with_runner`] so the two
/// don't drift on how "installed" and "running" are derived from
/// `systemctl show`.
struct SystemdStatusInfo {
    file_exists: bool,
    installed: bool,
    running: bool,
    active_state: String,
}

fn resolve_status_info(runner: &mut dyn ServiceCommandRunner) -> Result<SystemdStatusInfo> {
    let file_exists = unit_path()?.exists();
    // Query the manager unconditionally — a unit file removed out-of-band
    // while systemd still has it loaded/enabled is an orphan we must
    // still report as installed, not silently claim "not installed".
    let unit_state = query_unit_state(runner)?;
    // `is-active` uses non-zero exits for ordinary inactive states.
    // `show` returns those states in stdout and reserves failure for a
    // broken query.
    let active_state = runner.run_capture_checked(
        "systemctl show ActiveState",
        Command::new("systemctl").args([
            "--user",
            "show",
            "--property=ActiveState",
            "--value",
            SYSTEMD_UNIT,
        ]),
    )?;
    let running = active_state.trim() == "active";
    let installed = resolve_installed(file_exists, unit_state);
    Ok(SystemdStatusInfo {
        file_exists,
        installed,
        running,
        active_state,
    })
}

pub(super) fn status_with_runner(runner: &mut dyn ServiceCommandRunner) -> Result<()> {
    let file = unit_path()?;
    let info = resolve_status_info(runner)?;
    // Detail line stays keyed off file presence: for a genuine orphan
    // (no unit file) the `Service: running/stopped` line already covers
    // it, and there's no installed-config context to attach the raw
    // ActiveState to.
    let detail = if info.file_exists {
        systemd_status_detail(&info.active_state)
    } else {
        None
    };
    println!(
        "Service: {}",
        super::status_label(info.installed, info.running)
    );
    if let Some(detail) = detail {
        println!("{detail}");
    }
    println!("Unit: {}", file.display());
    Ok(())
}

/// Runner-injectable service-state query behind
/// [`super::ServicePlatform::current_state_with_runner`] — see that
/// method's doc.
pub(super) fn current_state_with_runner(
    runner: &mut dyn ServiceCommandRunner,
) -> Result<super::ServiceState> {
    let info = resolve_status_info(runner)?;
    Ok(super::ServiceState::from_installed_running(
        info.installed,
        info.running,
    ))
}

/// Shared uninstall rollback: restore the previous unit file (or remove
/// it if there was none), reload the manager, and — if the unit was
/// previously enabled — re-enable it. Used by both the `remove_file`
/// failure path and the `daemon-reload` failure path, which must leave
/// the host in the same recovered state regardless of which step failed.
fn rollback_uninstall(
    file: &Path,
    previous: Option<&[u8]>,
    manager_state: SystemdUnitState,
    runner: &mut dyn ServiceCommandRunner,
) -> Vec<String> {
    let mut rollback_errors = Vec::new();
    if let Err(rollback) = restore_previous_unit(file, previous) {
        rollback_errors.push(format!("restore unit: {rollback:#}"));
    }
    if let Err(rollback) = runner.run_checked(
        "systemctl rollback daemon-reload",
        Command::new("systemctl").args(["--user", "daemon-reload"]),
    ) {
        rollback_errors.push(format!("reload restored unit: {rollback:#}"));
    }
    if manager_state.enabled
        && let Err(rollback) = runner.run_checked(
            "systemctl rollback enable previous unit",
            Command::new("systemctl").args(["--user", "enable", SYSTEMD_UNIT]),
        )
    {
        rollback_errors.push(format!("re-enable previous unit: {rollback:#}"));
    }
    rollback_errors
}

pub(super) fn uninstall_with_runner(runner: &mut dyn ServiceCommandRunner) -> Result<()> {
    uninstall_with_runner_and_remover(runner, remove_unit_file)
}

/// Non-generic wrapper around `std::fs::remove_file` so it can be named as
/// a concrete `fn(&Path) -> io::Result<()>` pointer (the generic
/// `std::fs::remove_file::<P: AsRef<Path>>` item cannot itself coerce to a
/// fixed fn-pointer type without an instantiation hint).
fn remove_unit_file(path: &Path) -> std::io::Result<()> {
    std::fs::remove_file(path)
}

/// Same as [`uninstall_with_runner`] but with the unit-file removal step
/// itself injectable, mirroring how [`ServiceCommandRunner`] is already
/// injected for the `systemctl` calls. Lets a test force a `remove_file`
/// failure deterministically — unlike locking down the parent directory's
/// permission bits (0o555), which a root-running process (some CI
/// containers) bypasses entirely, making that regression test a silent
/// no-op instead of a real red-before-green check.
fn uninstall_with_runner_and_remover(
    runner: &mut dyn ServiceCommandRunner,
    remove_file: fn(&Path) -> std::io::Result<()>,
) -> Result<()> {
    let file = unit_path()?;
    let previous = match std::fs::read(&file) {
        Ok(contents) => Some(contents),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error).with_context(|| format!("read {}", file.display())),
    };
    let manager_state = query_unit_state(runner)?;
    if previous.is_none() && !manager_state.loaded && !manager_state.enabled {
        println!("Service uninstalled ({})", file.display());
        return Ok(());
    }
    if (manager_state.loaded || manager_state.enabled)
        && let Err(error) = runner.run_checked(
            "systemctl disable",
            Command::new("systemctl").args(["--user", "disable", "--now", SYSTEMD_UNIT]),
        )
    {
        let rollback_errors = rollback_uninstall(&file, previous.as_deref(), manager_state, runner);
        return Err(combined_failure(error, rollback_errors));
    }
    if previous.is_some()
        && let Err(error) = remove_file(&file).with_context(|| format!("remove {}", file.display()))
    {
        let rollback_errors = rollback_uninstall(&file, previous.as_deref(), manager_state, runner);
        return Err(combined_failure(error, rollback_errors));
    }
    if let Err(error) = runner.run_checked(
        "systemctl daemon-reload",
        Command::new("systemctl").args(["--user", "daemon-reload"]),
    ) {
        let rollback_errors = rollback_uninstall(&file, previous.as_deref(), manager_state, runner);
        return Err(combined_failure(error, rollback_errors));
    }
    println!("Service uninstalled ({})", file.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RAII guard pointing `$HOME` at a tempdir and clearing
    /// `$XDG_CONFIG_HOME` for the guard's lifetime, so `unit_path()`
    /// resolves under the tempdir instead of a real XDG path that may
    /// already exist on the host (CI runners have been observed setting
    /// `XDG_CONFIG_HOME=$HOME/.config`, which `config_home()` correctly
    /// prefers over `$HOME` — see `unit_path_honors_xdg_config_home_when_set_and_nonempty`
    /// below). Restores both on drop. Caller must hold `lock_runtime_env()`.
    struct TempHomeGuard {
        prior_home: Option<std::ffi::OsString>,
        prior_xdg: Option<std::ffi::OsString>,
    }

    impl TempHomeGuard {
        fn set(tmp: &Path) -> Self {
            let prior_home = std::env::var_os("HOME");
            let prior_xdg = std::env::var_os("XDG_CONFIG_HOME");
            // SAFETY: caller holds `lock_runtime_env()` for this guard's lifetime.
            unsafe {
                std::env::set_var("HOME", tmp);
                std::env::remove_var("XDG_CONFIG_HOME");
            }
            Self {
                prior_home,
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
                match self.prior_xdg.take() {
                    Some(v) => std::env::set_var("XDG_CONFIG_HOME", v),
                    None => std::env::remove_var("XDG_CONFIG_HOME"),
                }
            }
        }
    }

    #[derive(Default)]
    struct RecordingRunner {
        labels: Vec<String>,
        args: Vec<Vec<String>>,
        fail_args: Option<Vec<&'static str>>,
        fail_nth_args: Option<(Vec<&'static str>, usize)>,
        fail_capture_args: Option<Vec<&'static str>>,
        unit_state_output: Option<String>,
        active_state_output: Option<String>,
    }

    impl ServiceCommandRunner for RecordingRunner {
        fn run_checked(&mut self, label: &str, command: &mut Command) -> Result<()> {
            self.labels.push(label.to_string());
            self.args.push(
                command
                    .get_args()
                    .map(|arg| arg.to_string_lossy().into_owned())
                    .collect(),
            );
            if self.fail_args.as_ref().is_some_and(|expected| {
                command
                    .get_args()
                    .map(|arg| arg.to_string_lossy())
                    .eq(expected.iter().copied())
            }) {
                anyhow::bail!("injected argv failure: {label}");
            }
            if let Some((expected, occurrence)) = self.fail_nth_args.as_ref() {
                let current_args = self.args.last().cloned().unwrap_or_default();
                let seen = self
                    .args
                    .iter()
                    .filter(|args| *args == &current_args)
                    .count();
                if current_args
                    .iter()
                    .map(String::as_str)
                    .eq(expected.iter().copied())
                    && seen == *occurrence
                {
                    anyhow::bail!("injected nth argv failure: {label}");
                }
            }
            Ok(())
        }

        fn run_capture_checked(&mut self, label: &str, command: &mut Command) -> Result<String> {
            self.labels.push(label.to_string());
            self.args.push(
                command
                    .get_args()
                    .map(|arg| arg.to_string_lossy().into_owned())
                    .collect(),
            );
            if self.fail_capture_args.as_ref().is_some_and(|expected| {
                command
                    .get_args()
                    .map(|arg| arg.to_string_lossy())
                    .eq(expected.iter().copied())
            }) {
                anyhow::bail!("injected argv capture failure: {label}");
            }
            let args = self.args.last().cloned().unwrap_or_default();
            match args.as_slice() {
                [user, show, load, unit_file, unit]
                    if user == "--user"
                        && show == "show"
                        && load == "--property=LoadState"
                        && unit_file == "--property=UnitFileState"
                        && unit == SYSTEMD_UNIT =>
                {
                    Ok(self
                        .unit_state_output
                        .clone()
                        .unwrap_or_else(|| "LoadState=not-found\nUnitFileState=\n".to_string()))
                }
                [user, show, property, value, unit]
                    if user == "--user"
                        && show == "show"
                        && property == "--property=ActiveState"
                        && value == "--value"
                        && unit == SYSTEMD_UNIT =>
                {
                    Ok(self
                        .active_state_output
                        .clone()
                        .unwrap_or_else(|| "inactive\n".to_string()))
                }
                _ => anyhow::bail!("unexpected capture argv: {args:?}"),
            }
        }
    }

    fn sample_invocation() -> ServeInvocation {
        ServeInvocation {
            exe: PathBuf::from("/usr/local/bin/ironclaw"),
            args: vec!["serve".to_string()],
            env: vec![(
                "IRONCLAW_REBORN_HOME".to_string(),
                "/home/op/.ironclaw/reborn".to_string(),
            )],
        }
    }

    fn sample_reborn_home() -> PathBuf {
        PathBuf::from("/home/op/.ironclaw/reborn")
    }

    /// Resolves a `RebornCliContext` from the currently-set `$HOME` (set by
    /// [`TempHomeGuard::set`]) — used by `install_with_runner` call sites
    /// below, which now need `context` to derive the unit's
    /// WorkingDirectory.
    fn sample_context() -> RebornCliContext {
        RebornCliContext::from_boot_config(
            ironclaw_reborn_config::RebornBootConfig::resolve_from_env()
                .expect("boot config must resolve under temp HOME"),
        )
    }

    #[test]
    fn unit_quote_escapes_backslash_and_double_quote() {
        assert_eq!(
            unit_quote(r#"a"b\c"#, false).expect("valid value"),
            r#""a\"b\\c""#
        );
    }

    #[test]
    fn unit_quote_escapes_directive_and_exec_expansion_syntax() {
        assert_eq!(
            unit_quote("line\n%h$HOME", true).expect("valid exec value"),
            "\"line\\n%%h$$HOME\""
        );
        assert_eq!(
            unit_quote("value%h$HOME", false).expect("valid environment value"),
            "\"value%%h$HOME\""
        );
        assert!(unit_quote("bad\0value", false).is_err());
    }

    #[test]
    fn unit_content_includes_service_type() {
        let unit = unit_content(&sample_invocation(), &sample_reborn_home()).expect("valid unit");
        assert!(unit.contains("Type=simple"));
    }

    #[test]
    fn unit_content_includes_exec_start_tokens() {
        let unit = unit_content(&sample_invocation(), &sample_reborn_home()).expect("valid unit");
        assert!(unit.contains(r#""/usr/local/bin/ironclaw""#));
        assert!(unit.contains(r#""serve""#));
    }

    #[test]
    fn reinstall_rewrites_legacy_executable_path() {
        let _lock = crate::runtime::test_env::lock_runtime_env();
        let tmp = tempfile::tempdir().expect("tempdir");
        let _home = TempHomeGuard::set(tmp.path());
        let file = unit_path().expect("unit path");
        std::fs::create_dir_all(file.parent().expect("unit parent")).expect("create parent");
        std::fs::write(
            &file,
            "[Service]\nExecStart=\"/usr/local/bin/ironclaw-reborn\" serve\n",
        )
        .expect("write legacy unit");

        let mut runner = RecordingRunner::default();
        let replaced = install_with_runner(&sample_context(), &sample_invocation(), &mut runner)
            .expect("install over legacy unit");
        let contents = std::fs::read_to_string(&file).expect("read upgraded unit");

        assert!(replaced);
        assert!(contents.contains(r#""/usr/local/bin/ironclaw""#));
        assert!(!contents.contains("/usr/local/bin/ironclaw-reborn"));
    }

    #[test]
    fn unit_content_includes_environment_line() {
        let unit = unit_content(&sample_invocation(), &sample_reborn_home()).expect("valid unit");
        assert!(unit.contains(r#"Environment="IRONCLAW_REBORN_HOME=/home/op/.ironclaw/reborn""#));
    }

    #[test]
    fn unit_content_includes_restart_policy_and_install_target() {
        let unit = unit_content(&sample_invocation(), &sample_reborn_home()).expect("valid unit");
        assert!(unit.contains("Restart=always"));
        assert!(unit.contains("RestartSec=3"));
        assert!(unit.contains("WantedBy=default.target"));
    }

    /// Pins the crash-loop fix: without WorkingDirectory, systemd's default
    /// cwd overlaps a default skill root and composition refuses to boot.
    /// `unit_content` just writes the caller-supplied path faithfully — see
    /// `install_with_runner` / `ensure_service_working_directory` for the
    /// actual path choice.
    #[test]
    fn unit_content_includes_working_directory_line() {
        let unit = unit_content(&sample_invocation(), &sample_reborn_home()).expect("valid unit");
        assert!(unit.contains(r#"WorkingDirectory="/home/op/.ironclaw/reborn""#));
        let working_dir_index = unit.find("WorkingDirectory=").unwrap();
        let exec_start_index = unit.find("ExecStart=").unwrap();
        assert!(working_dir_index < exec_start_index);
    }

    #[test]
    fn unit_content_escapes_quotes_in_env_value() {
        let invocation = ServeInvocation {
            exe: PathBuf::from("/usr/local/bin/ironclaw"),
            args: vec!["serve".to_string()],
            env: vec![(
                "IRONCLAW_REBORN_PROFILE".to_string(),
                r#"has"quote"#.to_string(),
            )],
        };
        let unit = unit_content(&invocation, &sample_reborn_home()).expect("valid unit");
        assert!(unit.contains(r#"IRONCLAW_REBORN_PROFILE=has\"quote"#));
    }

    #[test]
    fn unit_content_cannot_inject_directives_through_newlines() {
        let invocation = ServeInvocation {
            exe: PathBuf::from("/opt/%n/$bin\nInjected=true"),
            args: vec!["serve\nEnvironment=EVIL=yes".to_string()],
            env: vec![(
                "IRONCLAW_REBORN_PROFILE".to_string(),
                "safe\nExecStart=/bin/evil%h".to_string(),
            )],
        };
        let unit = unit_content(&invocation, &sample_reborn_home()).expect("escaped unit");

        assert!(
            unit.contains(r#"Environment="IRONCLAW_REBORN_PROFILE=safe\nExecStart=/bin/evil%%h""#)
        );
        assert!(unit.contains(r#""/opt/%%n/$$bin\nInjected=true""#));
        assert!(unit.contains(r#""serve\nEnvironment=EVIL=yes""#));
        assert!(!unit.lines().any(|line| line == "Injected=true"));
        assert!(!unit.lines().any(|line| line == "Environment=EVIL=yes"));
    }

    #[test]
    fn unit_path_ends_with_expected_suffix() {
        let _lock = crate::runtime::test_env::lock_runtime_env();
        let prior_home = std::env::var_os("HOME");
        let prior_xdg = std::env::var_os("XDG_CONFIG_HOME");
        // SAFETY: serialized by `lock_runtime_env`; restored before returning.
        unsafe {
            std::env::set_var("HOME", "/home/op");
            std::env::remove_var("XDG_CONFIG_HOME");
        }
        let path = unit_path();
        // SAFETY: see above.
        unsafe {
            match prior_home {
                Some(v) => std::env::set_var("HOME", v),
                None => std::env::remove_var("HOME"),
            }
            match prior_xdg {
                Some(v) => std::env::set_var("XDG_CONFIG_HOME", v),
                None => std::env::remove_var("XDG_CONFIG_HOME"),
            }
        }
        assert_eq!(
            path.expect("must resolve"),
            PathBuf::from("/home/op/.config/systemd/user/ironclaw-reborn.service")
        );
    }

    #[test]
    fn unit_path_honors_xdg_config_home_when_set_and_nonempty() {
        // RED-then-green: systemd's user-unit search path prefers
        // `$XDG_CONFIG_HOME/systemd/user` over `$HOME/.config/systemd/user`
        // when `XDG_CONFIG_HOME` is set and non-empty. Before this fix,
        // `unit_path()` ignored `XDG_CONFIG_HOME` entirely and always wrote
        // under `$HOME/.config`, which a systemd install with a customized
        // `XDG_CONFIG_HOME` would never look at.
        let _lock = crate::runtime::test_env::lock_runtime_env();
        let prior_home = std::env::var_os("HOME");
        let prior_xdg = std::env::var_os("XDG_CONFIG_HOME");
        // SAFETY: serialized by `lock_runtime_env`; restored before returning.
        unsafe {
            std::env::set_var("HOME", "/home/op");
            std::env::set_var("XDG_CONFIG_HOME", "/custom/xdg-config");
        }
        let path = unit_path();
        // SAFETY: see above.
        unsafe {
            match prior_home {
                Some(v) => std::env::set_var("HOME", v),
                None => std::env::remove_var("HOME"),
            }
            match prior_xdg {
                Some(v) => std::env::set_var("XDG_CONFIG_HOME", v),
                None => std::env::remove_var("XDG_CONFIG_HOME"),
            }
        }
        assert_eq!(
            path.expect("must resolve"),
            PathBuf::from("/custom/xdg-config/systemd/user/ironclaw-reborn.service")
        );
    }

    #[test]
    fn unit_path_falls_back_to_home_config_when_xdg_config_home_is_empty() {
        // An empty `XDG_CONFIG_HOME` must be treated as unset (XDG base
        // directory spec), not as a literal empty-string base path.
        let _lock = crate::runtime::test_env::lock_runtime_env();
        let prior_home = std::env::var_os("HOME");
        let prior_xdg = std::env::var_os("XDG_CONFIG_HOME");
        // SAFETY: serialized by `lock_runtime_env`; restored before returning.
        unsafe {
            std::env::set_var("HOME", "/home/op");
            std::env::set_var("XDG_CONFIG_HOME", "");
        }
        let path = unit_path();
        // SAFETY: see above.
        unsafe {
            match prior_home {
                Some(v) => std::env::set_var("HOME", v),
                None => std::env::remove_var("HOME"),
            }
            match prior_xdg {
                Some(v) => std::env::set_var("XDG_CONFIG_HOME", v),
                None => std::env::remove_var("XDG_CONFIG_HOME"),
            }
        }
        assert_eq!(
            path.expect("must resolve"),
            PathBuf::from("/home/op/.config/systemd/user/ironclaw-reborn.service")
        );
    }

    #[test]
    fn install_propagates_daemon_reload_failure_before_enable() {
        let _lock = crate::runtime::test_env::lock_runtime_env();
        let tmp = tempfile::tempdir().expect("tempdir");
        let _home = TempHomeGuard::set(tmp.path());
        let mut runner = RecordingRunner {
            fail_args: Some(vec!["--user", "daemon-reload"]),
            ..RecordingRunner::default()
        };
        let result = install_with_runner(&sample_context(), &sample_invocation(), &mut runner);

        assert!(result.is_err());
        assert_eq!(
            runner.labels,
            [
                "systemctl show unit state",
                "systemctl daemon-reload",
                "systemctl rollback daemon-reload"
            ]
        );
    }

    #[test]
    fn install_propagates_enable_failure() {
        let _lock = crate::runtime::test_env::lock_runtime_env();
        let tmp = tempfile::tempdir().expect("tempdir");
        let _home = TempHomeGuard::set(tmp.path());
        let mut runner = RecordingRunner {
            fail_nth_args: Some((vec!["--user", "enable", SYSTEMD_UNIT], 1)),
            ..RecordingRunner::default()
        };
        let result = install_with_runner(&sample_context(), &sample_invocation(), &mut runner);

        assert!(result.is_err());
        assert_eq!(
            runner.labels,
            [
                "systemctl show unit state",
                "systemctl daemon-reload",
                "systemctl enable",
                "systemctl rollback disable",
                "systemctl rollback daemon-reload"
            ]
        );
        assert_eq!(
            runner.args[3],
            ["--user", "disable", SYSTEMD_UNIT],
            "a possibly partial enable must be compensated before file rollback"
        );
    }

    #[test]
    fn uninstall_disables_before_removing_and_reloading() {
        let _lock = crate::runtime::test_env::lock_runtime_env();
        let tmp = tempfile::tempdir().expect("tempdir");
        let _home = TempHomeGuard::set(tmp.path());
        let file = unit_path().expect("unit path");
        std::fs::create_dir_all(file.parent().expect("unit parent")).expect("create parent");
        std::fs::write(&file, "unit").expect("write unit");
        let mut runner = RecordingRunner {
            unit_state_output: Some("LoadState=loaded\nUnitFileState=enabled\n".to_string()),
            ..RecordingRunner::default()
        };
        let result = uninstall_with_runner(&mut runner);

        result.expect("uninstall succeeds");
        assert!(!file.exists());
        assert_eq!(
            runner.labels,
            [
                "systemctl show unit state",
                "systemctl disable",
                "systemctl daemon-reload"
            ]
        );
        assert_eq!(runner.args[1], ["--user", "disable", "--now", SYSTEMD_UNIT]);
    }

    #[test]
    fn uninstall_disable_failure_rolls_back_like_reload_failure() {
        // Before this fix, a `disable --now` error propagated with a bare
        // `?` and no rollback, unlike every sibling failure branch in this
        // function (remove_file, daemon-reload), which routes through
        // `rollback_uninstall` + `combined_failure`. A disable failure can
        // leave the unit partially disabled with no compensating
        // daemon-reload/re-enable, so this pins that it now goes through
        // the same rollback path as the other two.
        let _lock = crate::runtime::test_env::lock_runtime_env();
        let tmp = tempfile::tempdir().expect("tempdir");
        let _home = TempHomeGuard::set(tmp.path());
        let file = unit_path().expect("unit path");
        std::fs::create_dir_all(file.parent().expect("unit parent")).expect("create parent");
        std::fs::write(&file, "unit").expect("write unit");
        let mut runner = RecordingRunner {
            fail_args: Some(vec!["--user", "disable", "--now", SYSTEMD_UNIT]),
            unit_state_output: Some("LoadState=loaded\nUnitFileState=enabled\n".to_string()),
            ..RecordingRunner::default()
        };
        let result = uninstall_with_runner(&mut runner);

        let error = result.expect_err("a failed disable must propagate");
        assert!(file.exists(), "failed disable must not remove the unit");
        let rendered = format!("{error:#}");
        assert!(rendered.contains("systemctl disable"), "{rendered}");
        assert_eq!(
            runner.labels,
            [
                "systemctl show unit state",
                "systemctl disable",
                "systemctl rollback daemon-reload",
                "systemctl rollback enable previous unit",
            ],
            "a disable failure must run the same rollback sequence as a reload failure"
        );
    }

    #[test]
    fn failed_reinstall_restores_previous_unit() {
        let _lock = crate::runtime::test_env::lock_runtime_env();
        let tmp = tempfile::tempdir().expect("tempdir");
        let _home = TempHomeGuard::set(tmp.path());
        let file = unit_path().expect("unit path");
        std::fs::create_dir_all(file.parent().expect("unit parent")).expect("create parent");
        std::fs::write(&file, "previous unit").expect("write previous unit");
        let mut runner = RecordingRunner {
            unit_state_output: Some("LoadState=loaded\nUnitFileState=enabled\n".to_string()),
            fail_nth_args: Some((vec!["--user", "enable", SYSTEMD_UNIT], 1)),
            ..RecordingRunner::default()
        };
        let result = install_with_runner(&sample_context(), &sample_invocation(), &mut runner);

        assert!(result.is_err());
        assert_eq!(
            std::fs::read_to_string(file).expect("restored unit"),
            "previous unit"
        );
        assert_eq!(
            runner.args,
            [
                vec![
                    "--user",
                    "show",
                    "--property=LoadState",
                    "--property=UnitFileState",
                    SYSTEMD_UNIT,
                ],
                vec!["--user", "daemon-reload"],
                vec!["--user", "enable", SYSTEMD_UNIT],
                vec!["--user", "disable", SYSTEMD_UNIT],
                vec!["--user", "daemon-reload"],
                vec!["--user", "enable", SYSTEMD_UNIT],
            ]
        );
    }

    #[test]
    fn failed_reinstall_does_not_enable_previously_disabled_unit() {
        let _lock = crate::runtime::test_env::lock_runtime_env();
        let tmp = tempfile::tempdir().expect("tempdir");
        let _home = TempHomeGuard::set(tmp.path());
        let file = unit_path().expect("unit path");
        std::fs::create_dir_all(file.parent().expect("unit parent")).expect("create parent");
        std::fs::write(&file, "previous unit").expect("write previous unit");
        let mut runner = RecordingRunner {
            unit_state_output: Some("LoadState=loaded\nUnitFileState=disabled\n".to_string()),
            fail_nth_args: Some((vec!["--user", "enable", SYSTEMD_UNIT], 1)),
            ..RecordingRunner::default()
        };
        let result = install_with_runner(&sample_context(), &sample_invocation(), &mut runner);

        assert!(result.is_err());
        assert_eq!(
            std::fs::read_to_string(file).expect("restored unit"),
            "previous unit"
        );
        assert_eq!(
            runner.args,
            [
                vec![
                    "--user",
                    "show",
                    "--property=LoadState",
                    "--property=UnitFileState",
                    SYSTEMD_UNIT,
                ],
                vec!["--user", "daemon-reload"],
                vec!["--user", "enable", SYSTEMD_UNIT],
                vec!["--user", "disable", SYSTEMD_UNIT],
                vec!["--user", "daemon-reload"],
            ]
        );
    }

    #[test]
    fn absent_uninstall_queries_manager_then_no_ops_when_not_found() {
        let _lock = crate::runtime::test_env::lock_runtime_env();
        let tmp = tempfile::tempdir().expect("tempdir");
        let _home = TempHomeGuard::set(tmp.path());
        let mut runner = RecordingRunner::default();
        let result = uninstall_with_runner(&mut runner);

        result.expect("absent uninstall succeeds");
        assert_eq!(runner.labels, ["systemctl show unit state"]);
    }

    #[test]
    fn absent_uninstall_disables_loaded_or_enabled_manager_state() {
        let _lock = crate::runtime::test_env::lock_runtime_env();
        let tmp = tempfile::tempdir().expect("tempdir");
        let _home = TempHomeGuard::set(tmp.path());
        let mut runner = RecordingRunner {
            unit_state_output: Some("LoadState=loaded\nUnitFileState=enabled\n".to_string()),
            ..RecordingRunner::default()
        };
        let result = uninstall_with_runner(&mut runner);

        result.expect("orphan manager state uninstall succeeds");
        assert_eq!(
            runner.args,
            [
                vec![
                    "--user",
                    "show",
                    "--property=LoadState",
                    "--property=UnitFileState",
                    SYSTEMD_UNIT,
                ],
                vec!["--user", "disable", "--now", SYSTEMD_UNIT],
                vec!["--user", "daemon-reload"],
            ]
        );
    }

    #[test]
    fn enable_failure_reports_compensating_reload_failure_with_primary_error() {
        let _lock = crate::runtime::test_env::lock_runtime_env();
        let tmp = tempfile::tempdir().expect("tempdir");
        let _home = TempHomeGuard::set(tmp.path());
        let mut runner = RecordingRunner {
            fail_args: Some(vec!["--user", "enable", SYSTEMD_UNIT]),
            fail_nth_args: Some((vec!["--user", "daemon-reload"], 2)),
            ..RecordingRunner::default()
        };
        let error = install_with_runner(&sample_context(), &sample_invocation(), &mut runner)
            .expect_err("enable and compensating reload failure must surface");

        // Top-level `Display` (`to_string()`) now only shows the rollback
        // context, matching anyhow's chain convention; the primary failure
        // lives in `source()`/`{:#}` — see `enable_failure_preserves_primary_error_as_source`.
        let rendered = error.to_string();
        assert!(rendered.contains("rollback failures"), "{rendered}");
        assert!(rendered.contains("reload restored unit"), "{rendered}");

        let chained = format!("{error:#}");
        assert!(chained.contains("systemctl enable"), "{chained}");
        assert!(chained.contains("reload restored unit"), "{chained}");
        assert!(chained.contains("rollback failures"), "{chained}");
    }

    #[test]
    fn enable_failure_preserves_primary_error_as_source() {
        // RED-then-green regression for the reviewer-flagged flattening bug:
        // `combined_failure` used to fold the primary error into a single
        // formatted string via `anyhow!("{primary:#}; ...")`, which drops
        // the source chain entirely (`.source()` was `None`). An operator
        // could no longer tell a permission-denied primary failure apart
        // from a disk-full one once a rollback step also failed. This
        // asserts the primary is preserved as the combined error's source.
        let _lock = crate::runtime::test_env::lock_runtime_env();
        let tmp = tempfile::tempdir().expect("tempdir");
        let _home = TempHomeGuard::set(tmp.path());
        let mut runner = RecordingRunner {
            fail_args: Some(vec!["--user", "enable", SYSTEMD_UNIT]),
            fail_nth_args: Some((vec!["--user", "daemon-reload"], 2)),
            ..RecordingRunner::default()
        };
        let error = install_with_runner(&sample_context(), &sample_invocation(), &mut runner)
            .expect_err("enable and compensating reload failure must surface");

        let source = error
            .source()
            .expect("combined failure must retain a source chain");
        let source_text = format!("{source:#}");
        assert!(
            source_text.contains("systemctl enable"),
            "primary error text must survive under source(): {source_text}"
        );
    }

    #[test]
    fn uninstall_reload_failure_restores_unit_for_recovery() {
        let _lock = crate::runtime::test_env::lock_runtime_env();
        let tmp = tempfile::tempdir().expect("tempdir");
        let _home = TempHomeGuard::set(tmp.path());
        let file = unit_path().expect("unit path");
        std::fs::create_dir_all(file.parent().expect("unit parent")).expect("create parent");
        std::fs::write(&file, "previous unit").expect("write unit");
        let mut runner = RecordingRunner {
            unit_state_output: Some("LoadState=loaded\nUnitFileState=enabled\n".to_string()),
            fail_nth_args: Some((vec!["--user", "daemon-reload"], 1)),
            ..RecordingRunner::default()
        };
        let result = uninstall_with_runner(&mut runner);

        assert!(result.is_err());
        assert_eq!(
            std::fs::read_to_string(file).expect("restored unit"),
            "previous unit"
        );
        assert!(
            runner
                .labels
                .contains(&"systemctl rollback daemon-reload".to_string())
        );
        assert!(
            runner
                .labels
                .contains(&"systemctl rollback enable previous unit".to_string())
        );
    }

    #[test]
    fn uninstall_remove_file_failure_rolls_back_like_reload_failure() {
        // Before this fix, a `remove_file` error after a successful
        // `disable` propagated with a bare `?` and no rollback: the unit
        // stayed disabled with no unit file, and (if it had been
        // enabled) never got re-enabled. This pins that the failure now
        // goes through the same rollback path as a daemon-reload
        // failure.
        //
        // The failure is injected through `uninstall_with_runner_and_remover`
        // rather than by chmod-locking the parent directory: a
        // root-running test process (some CI containers) bypasses
        // directory permission checks entirely, which made the old
        // version of this test a silent no-op instead of a real
        // red-before-green regression check.
        let _lock = crate::runtime::test_env::lock_runtime_env();
        let tmp = tempfile::tempdir().expect("tempdir");
        let _home = TempHomeGuard::set(tmp.path());
        let file = unit_path().expect("unit path");
        std::fs::create_dir_all(file.parent().expect("unit parent")).expect("create parent");
        std::fs::write(&file, "previous unit").expect("write unit");
        let mut runner = RecordingRunner {
            unit_state_output: Some("LoadState=loaded\nUnitFileState=enabled\n".to_string()),
            ..RecordingRunner::default()
        };
        fn failing_remove_file(_path: &Path) -> std::io::Result<()> {
            Err(std::io::Error::other("injected remove_file failure"))
        }
        let result = uninstall_with_runner_and_remover(&mut runner, failing_remove_file);

        let error = result.expect_err("an injected remove_file failure must propagate");
        // `Display` on `anyhow::Error` only prints the outermost context
        // (`remove <path>`); the alternate `{:#}` form walks the full
        // source chain down to the injected io::Error's own message.
        let rendered = format!("{error:#}");
        assert!(
            rendered.contains("injected remove_file failure"),
            "error: {rendered}"
        );
        assert!(
            file.exists(),
            "the unit file rollback must restore it after a failed removal"
        );
        assert_eq!(
            std::fs::read_to_string(&file).expect("read restored unit"),
            "previous unit"
        );
        assert!(
            runner
                .labels
                .contains(&"systemctl rollback daemon-reload".to_string()),
            "remove_file failure must roll back like a reload failure: {:?}",
            runner.labels
        );
        assert!(
            runner
                .labels
                .contains(&"systemctl rollback enable previous unit".to_string()),
            "a previously-enabled unit must be re-enabled during rollback: {:?}",
            runner.labels
        );
    }

    #[test]
    fn stop_propagates_service_manager_failure_when_unit_exists() {
        let _lock = crate::runtime::test_env::lock_runtime_env();
        let tmp = tempfile::tempdir().expect("tempdir");
        let _home = TempHomeGuard::set(tmp.path());
        let file = unit_path().expect("unit path");
        std::fs::create_dir_all(file.parent().expect("unit parent")).expect("create parent");
        std::fs::write(file, "unit").expect("write unit");
        let mut runner = RecordingRunner {
            fail_args: Some(vec!["--user", "stop", SYSTEMD_UNIT]),
            ..RecordingRunner::default()
        };
        let result = stop_with_runner(&mut runner);

        assert!(result.is_err());
        assert_eq!(runner.labels, ["systemctl stop"]);
    }

    #[test]
    fn absent_stop_stops_loaded_or_enabled_manager_state() {
        // Mirrors `absent_uninstall_disables_loaded_or_enabled_manager_state`:
        // a unit file removed out-of-band while systemd still shows it
        // loaded/enabled is an orphan that still needs tearing down. The old
        // `stop_with_runner_impl` gated solely on `unit_path()?.exists()`,
        // so this state printed "Service stopped" without ever calling
        // `systemctl stop`.
        let _lock = crate::runtime::test_env::lock_runtime_env();
        let tmp = tempfile::tempdir().expect("tempdir");
        let _home = TempHomeGuard::set(tmp.path());
        let mut runner = RecordingRunner {
            unit_state_output: Some("LoadState=loaded\nUnitFileState=enabled\n".to_string()),
            ..RecordingRunner::default()
        };
        let result = stop_with_runner(&mut runner);

        result.expect("stop of an orphaned loaded unit succeeds");
        assert_eq!(
            runner.labels,
            ["systemctl show unit state", "systemctl stop"],
            "a unit file absent from disk but still loaded/enabled in the manager must still be stopped"
        );
    }

    #[test]
    fn status_queries_systemctl_unconditionally_and_reports_not_installed_when_absent_everywhere() {
        // Was: "skips systemctl query when unit absent". That skip was
        // itself the orphan-hiding bug (finding 5) — a unit file removed
        // out-of-band while systemd still had it loaded/enabled would
        // silently read as "not installed". The query must always run so
        // an orphan can be detected; only report "not installed" when the
        // file AND the manager both show nothing.
        let _lock = crate::runtime::test_env::lock_runtime_env();
        let tmp = tempfile::tempdir().expect("tempdir");
        let _home = TempHomeGuard::set(tmp.path());
        let mut runner = RecordingRunner::default();
        let result = status_with_runner(&mut runner);

        result.expect("status succeeds when not installed");
        assert_eq!(
            runner.labels,
            ["systemctl show unit state", "systemctl show ActiveState"],
            "must query systemd even when the unit file is absent, to detect orphans"
        );
    }

    #[test]
    fn resolve_installed_true_when_file_absent_but_manager_still_shows_it_loaded_or_enabled() {
        let none = SystemdUnitState {
            loaded: false,
            enabled: false,
        };
        let loaded_only = SystemdUnitState {
            loaded: true,
            enabled: false,
        };
        let enabled_only = SystemdUnitState {
            loaded: false,
            enabled: true,
        };
        assert!(!resolve_installed(false, none));
        assert!(
            resolve_installed(false, loaded_only),
            "orphaned unit still loaded"
        );
        assert!(
            resolve_installed(false, enabled_only),
            "orphaned unit still enabled"
        );
        assert!(resolve_installed(true, none));
    }

    #[test]
    fn query_unit_state_parses_key_value_lines_regardless_of_order() {
        let mut runner = RecordingRunner {
            unit_state_output: Some("UnitFileState=enabled\nLoadState=loaded\n".to_string()),
            ..RecordingRunner::default()
        };
        let state = query_unit_state(&mut runner).expect("out-of-order lines must parse");
        assert!(state.loaded);
        assert!(state.enabled);
    }

    #[test]
    fn query_unit_state_errors_when_a_required_key_is_missing() {
        let mut runner = RecordingRunner {
            unit_state_output: Some("LoadState=loaded\n".to_string()),
            ..RecordingRunner::default()
        };
        let error = query_unit_state(&mut runner)
            .expect_err("a missing UnitFileState line must error, not silently default");
        assert!(error.to_string().contains("UnitFileState"));
    }

    #[test]
    fn status_propagates_capture_failure() {
        let _lock = crate::runtime::test_env::lock_runtime_env();
        let tmp = tempfile::tempdir().expect("tempdir");
        let _home = TempHomeGuard::set(tmp.path());
        let file = unit_path().expect("unit path");
        std::fs::create_dir_all(file.parent().expect("unit parent")).expect("create parent");
        std::fs::write(file, "unit").expect("write unit");
        let mut runner = RecordingRunner {
            fail_capture_args: Some(vec![
                "--user",
                "show",
                "--property=ActiveState",
                "--value",
                SYSTEMD_UNIT,
            ]),
            ..RecordingRunner::default()
        };
        let result = status_with_runner(&mut runner);

        assert!(result.is_err());
    }

    // ── restart ─────────────────────────────────────────────────

    #[test]
    fn restart_running_service_stops_then_starts() {
        let _lock = crate::runtime::test_env::lock_runtime_env();
        let tmp = tempfile::tempdir().expect("tempdir");
        let _home = TempHomeGuard::set(tmp.path());
        let file = unit_path().expect("unit path");
        std::fs::create_dir_all(file.parent().expect("unit parent")).expect("create parent");
        std::fs::write(&file, "unit").expect("write unit");
        let mut runner = RecordingRunner {
            active_state_output: Some("active\n".to_string()),
            ..RecordingRunner::default()
        };
        let result = restart_with_runner(&mut runner);

        result.expect("restart of a running service must succeed");
        assert_eq!(
            runner.labels,
            [
                "systemctl show ActiveState",
                "systemctl stop",
                "systemctl daemon-reload",
                "systemctl start",
            ]
        );
    }

    #[test]
    fn restart_stopped_service_just_starts() {
        let _lock = crate::runtime::test_env::lock_runtime_env();
        let tmp = tempfile::tempdir().expect("tempdir");
        let _home = TempHomeGuard::set(tmp.path());
        let file = unit_path().expect("unit path");
        std::fs::create_dir_all(file.parent().expect("unit parent")).expect("create parent");
        std::fs::write(&file, "unit").expect("write unit");
        let mut runner = RecordingRunner {
            active_state_output: Some("inactive\n".to_string()),
            ..RecordingRunner::default()
        };
        let result = restart_with_runner(&mut runner);

        result.expect("restart of a stopped service must succeed without error");
        assert_eq!(
            runner.labels,
            [
                "systemctl show ActiveState",
                "systemctl daemon-reload",
                "systemctl start",
            ]
        );
    }

    #[test]
    fn restart_not_installed_errors_with_install_guidance() {
        let _lock = crate::runtime::test_env::lock_runtime_env();
        let tmp = tempfile::tempdir().expect("tempdir");
        let _home = TempHomeGuard::set(tmp.path());
        let mut runner = RecordingRunner::default();
        let result = restart_with_runner(&mut runner);

        let error = result.expect_err("restart without an installed service must error");
        assert!(error.to_string().contains("service install"));
        assert!(runner.labels.is_empty(), "no commands should run");
    }

    #[test]
    fn restart_reports_stopped_when_start_fails_after_successful_stop() {
        let _lock = crate::runtime::test_env::lock_runtime_env();
        let tmp = tempfile::tempdir().expect("tempdir");
        let _home = TempHomeGuard::set(tmp.path());
        let file = unit_path().expect("unit path");
        std::fs::create_dir_all(file.parent().expect("unit parent")).expect("create parent");
        std::fs::write(&file, "unit").expect("write unit");
        let mut runner = RecordingRunner {
            active_state_output: Some("active\n".to_string()),
            fail_args: Some(vec!["--user", "start", SYSTEMD_UNIT]),
            ..RecordingRunner::default()
        };
        let result = restart_with_runner(&mut runner);

        let error = result.expect_err("failed start after successful stop must error");
        assert!(
            error.to_string().contains("STOPPED"),
            "error must report the service as stopped, got: {error}"
        );
        assert_eq!(
            runner.labels,
            [
                "systemctl show ActiveState",
                "systemctl stop",
                "systemctl daemon-reload",
                "systemctl start",
            ]
        );
    }

    #[test]
    fn status_detail_line_reports_raw_failed_state_but_omits_for_active() {
        assert_eq!(
            systemd_status_detail("failed\n"),
            Some("  systemd ActiveState: failed".to_string()),
            "a crashed unit's raw ActiveState must survive as a detail line"
        );
        assert_eq!(
            systemd_status_detail("active\n"),
            None,
            "line 1 already says running; no redundant detail line for active"
        );
    }

    #[test]
    fn status_accepts_active_and_inactive_states_from_exact_show_command() {
        let _lock = crate::runtime::test_env::lock_runtime_env();
        let tmp = tempfile::tempdir().expect("tempdir");
        let _home = TempHomeGuard::set(tmp.path());
        let file = unit_path().expect("unit path");
        std::fs::create_dir_all(file.parent().expect("unit parent")).expect("create parent");
        std::fs::write(file, "unit").expect("write unit");
        for state in ["active\n", "inactive\n"] {
            let mut runner = RecordingRunner {
                active_state_output: Some(state.to_string()),
                ..RecordingRunner::default()
            };
            status_with_runner(&mut runner).expect("status succeeds");
            assert_eq!(
                runner.args,
                [
                    vec![
                        "--user",
                        "show",
                        "--property=LoadState",
                        "--property=UnitFileState",
                        SYSTEMD_UNIT,
                    ],
                    vec![
                        "--user",
                        "show",
                        "--property=ActiveState",
                        "--value",
                        SYSTEMD_UNIT,
                    ]
                ]
            );
        }
    }

    // ── current_state ───────────────────────────────────────────

    #[test]
    fn current_state_reports_not_installed_when_absent_everywhere() {
        let _lock = crate::runtime::test_env::lock_runtime_env();
        let tmp = tempfile::tempdir().expect("tempdir");
        let _home = TempHomeGuard::set(tmp.path());
        let mut runner = RecordingRunner::default();
        let state = current_state_with_runner(&mut runner).expect("current_state must succeed");
        assert_eq!(state, super::super::ServiceState::NotInstalled);
    }

    #[test]
    fn current_state_reports_stopped_when_installed_but_inactive() {
        let _lock = crate::runtime::test_env::lock_runtime_env();
        let tmp = tempfile::tempdir().expect("tempdir");
        let _home = TempHomeGuard::set(tmp.path());
        let file = unit_path().expect("unit path");
        std::fs::create_dir_all(file.parent().expect("unit parent")).expect("create parent");
        std::fs::write(file, "unit").expect("write unit");
        let mut runner = RecordingRunner {
            unit_state_output: Some("LoadState=loaded\nUnitFileState=enabled\n".to_string()),
            active_state_output: Some("inactive\n".to_string()),
            ..RecordingRunner::default()
        };
        let state = current_state_with_runner(&mut runner).expect("current_state must succeed");
        assert_eq!(state, super::super::ServiceState::Stopped);
    }

    #[test]
    fn current_state_reports_running_when_active() {
        let _lock = crate::runtime::test_env::lock_runtime_env();
        let tmp = tempfile::tempdir().expect("tempdir");
        let _home = TempHomeGuard::set(tmp.path());
        let file = unit_path().expect("unit path");
        std::fs::create_dir_all(file.parent().expect("unit parent")).expect("create parent");
        std::fs::write(file, "unit").expect("write unit");
        let mut runner = RecordingRunner {
            unit_state_output: Some("LoadState=loaded\nUnitFileState=enabled\n".to_string()),
            active_state_output: Some("active\n".to_string()),
            ..RecordingRunner::default()
        };
        let state = current_state_with_runner(&mut runner).expect("current_state must succeed");
        assert_eq!(state, super::super::ServiceState::Running);
    }
}
