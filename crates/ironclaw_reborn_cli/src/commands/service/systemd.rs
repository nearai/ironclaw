//! Linux systemd user-unit generators, path resolution, and verb
//! bodies for `ironclaw-reborn service`.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::serve_invocation::ServeInvocation;

use super::{OsServiceCommandRunner, SYSTEMD_UNIT, ServiceCommandRunner, home_dir};

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

fn unit_content(invocation: &ServeInvocation) -> Result<String> {
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

    Ok(format!(
        "[Unit]\n\
         Description=IronClaw Reborn daemon\n\
         After=network.target\n\
         \n\
         [Service]\n\
         Type=simple\n\
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

fn query_unit_state(runner: &mut dyn ServiceCommandRunner) -> Result<SystemdUnitState> {
    let output = runner.run_capture_checked(
        "systemctl show unit state",
        Command::new("systemctl").args([
            "--user",
            "show",
            "--property=LoadState",
            "--property=UnitFileState",
            "--value",
            SYSTEMD_UNIT,
        ]),
    )?;
    let mut lines = output.split('\n');
    let load_state = lines.next().unwrap_or_default().trim();
    let unit_file_state = lines.next().unwrap_or_default().trim();
    Ok(SystemdUnitState {
        loaded: !matches!(load_state, "" | "not-found"),
        enabled: matches!(
            unit_file_state,
            "enabled" | "enabled-runtime" | "linked" | "linked-runtime" | "alias"
        ),
    })
}

fn combined_failure(primary: anyhow::Error, rollback_errors: Vec<String>) -> anyhow::Error {
    if rollback_errors.is_empty() {
        primary
    } else {
        anyhow::anyhow!(
            "{primary:#}; rollback failures: {}",
            rollback_errors.join("; ")
        )
    }
}

// ── Path helpers ────────────────────────────────────────────────

fn unit_path() -> Result<PathBuf> {
    Ok(home_dir()?
        .join(".config")
        .join("systemd")
        .join("user")
        .join(SYSTEMD_UNIT))
}

// ── Verb bodies ─────────────────────────────────────────────────

pub(super) fn install(invocation: &ServeInvocation) -> Result<()> {
    install_with_runner(invocation, &mut OsServiceCommandRunner)
}

pub(super) fn install_with_runner(
    invocation: &ServeInvocation,
    runner: &mut dyn ServiceCommandRunner,
) -> Result<()> {
    let file = unit_path()?;
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let previous = match std::fs::read(&file) {
        Ok(contents) => Some(contents),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error).with_context(|| format!("read {}", file.display())),
    };
    let unit = unit_content(invocation)?;
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
    println!("  Start with: ironclaw-reborn service start");
    Ok(())
}

pub(super) fn start() -> Result<()> {
    start_with_runner(&mut OsServiceCommandRunner)
}

fn start_with_runner(runner: &mut dyn ServiceCommandRunner) -> Result<()> {
    if !unit_path()?.exists() {
        bail!("Service not installed. Run `ironclaw-reborn service install` first.");
    }
    runner.run_checked(
        "systemctl daemon-reload",
        Command::new("systemctl").args(["--user", "daemon-reload"]),
    )?;
    runner.run_checked(
        "systemctl start",
        Command::new("systemctl").args(["--user", "start", SYSTEMD_UNIT]),
    )?;
    println!("Service started");
    Ok(())
}

pub(super) fn stop() -> Result<()> {
    stop_with_runner(&mut OsServiceCommandRunner)
}

fn stop_with_runner(runner: &mut dyn ServiceCommandRunner) -> Result<()> {
    if !unit_path()?.exists() {
        println!("Service stopped");
        return Ok(());
    }
    runner.run_checked(
        "systemctl stop",
        Command::new("systemctl").args(["--user", "stop", SYSTEMD_UNIT]),
    )?;
    println!("Service stopped");
    Ok(())
}

pub(super) fn restart() -> Result<()> {
    restart_with_runner(&mut OsServiceCommandRunner)
}

/// Detects install/running state, then delegates the stop/start decision
/// tree to [`super::restart_generic`], which both platforms share.
fn restart_with_runner(runner: &mut dyn ServiceCommandRunner) -> Result<()> {
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
        stop_with_runner,
        start_with_runner,
    )
}

pub(super) fn status() -> Result<()> {
    status_with_runner(&mut OsServiceCommandRunner)
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

fn status_with_runner(runner: &mut dyn ServiceCommandRunner) -> Result<()> {
    let file = unit_path()?;
    let installed = file.exists();
    let mut detail = None;
    let running = if installed {
        // `is-active` uses non-zero exits for ordinary inactive states.
        // `show` returns those states in stdout and reserves failure for a
        // broken query.
        let state = runner.run_capture_checked(
            "systemctl show ActiveState",
            Command::new("systemctl").args([
                "--user",
                "show",
                "--property=ActiveState",
                "--value",
                SYSTEMD_UNIT,
            ]),
        )?;
        detail = systemd_status_detail(&state);
        state.trim() == "active"
    } else {
        false
    };
    println!("Service: {}", super::status_label(installed, running));
    if let Some(detail) = detail {
        println!("{detail}");
    }
    println!("Unit: {}", file.display());
    Ok(())
}

pub(super) fn uninstall() -> Result<()> {
    uninstall_with_runner(&mut OsServiceCommandRunner)
}

pub(super) fn uninstall_with_runner(runner: &mut dyn ServiceCommandRunner) -> Result<()> {
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
    if manager_state.loaded || manager_state.enabled {
        runner.run_checked(
            "systemctl disable",
            Command::new("systemctl").args(["--user", "disable", "--now", SYSTEMD_UNIT]),
        )?;
    }
    if previous.is_some() {
        std::fs::remove_file(&file).with_context(|| format!("remove {}", file.display()))?;
    }
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
        if manager_state.enabled
            && let Err(rollback) = runner.run_checked(
                "systemctl rollback enable previous unit",
                Command::new("systemctl").args(["--user", "enable", SYSTEMD_UNIT]),
            )
        {
            rollback_errors.push(format!("re-enable previous unit: {rollback:#}"));
        }
        return Err(combined_failure(error, rollback_errors));
    }
    println!("Service uninstalled ({})", file.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
                [user, show, load, unit_file, value, unit]
                    if user == "--user"
                        && show == "show"
                        && load == "--property=LoadState"
                        && unit_file == "--property=UnitFileState"
                        && value == "--value"
                        && unit == SYSTEMD_UNIT =>
                {
                    Ok(self
                        .unit_state_output
                        .clone()
                        .unwrap_or_else(|| "not-found\n\n".to_string()))
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
            exe: PathBuf::from("/usr/local/bin/ironclaw-reborn"),
            args: vec!["serve".to_string()],
            env: vec![(
                "IRONCLAW_REBORN_HOME".to_string(),
                "/home/op/.ironclaw/reborn".to_string(),
            )],
        }
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
        let unit = unit_content(&sample_invocation()).expect("valid unit");
        assert!(unit.contains("Type=simple"));
    }

    #[test]
    fn unit_content_includes_exec_start_tokens() {
        let unit = unit_content(&sample_invocation()).expect("valid unit");
        assert!(unit.contains(r#""/usr/local/bin/ironclaw-reborn""#));
        assert!(unit.contains(r#""serve""#));
    }

    #[test]
    fn unit_content_includes_environment_line() {
        let unit = unit_content(&sample_invocation()).expect("valid unit");
        assert!(unit.contains(r#"Environment="IRONCLAW_REBORN_HOME=/home/op/.ironclaw/reborn""#));
    }

    #[test]
    fn unit_content_includes_restart_policy_and_install_target() {
        let unit = unit_content(&sample_invocation()).expect("valid unit");
        assert!(unit.contains("Restart=always"));
        assert!(unit.contains("RestartSec=3"));
        assert!(unit.contains("WantedBy=default.target"));
    }

    #[test]
    fn unit_content_escapes_quotes_in_env_value() {
        let invocation = ServeInvocation {
            exe: PathBuf::from("/usr/local/bin/ironclaw-reborn"),
            args: vec!["serve".to_string()],
            env: vec![(
                "IRONCLAW_REBORN_PROFILE".to_string(),
                r#"has"quote"#.to_string(),
            )],
        };
        let unit = unit_content(&invocation).expect("valid unit");
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
        let unit = unit_content(&invocation).expect("escaped unit");

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
        let prior = std::env::var_os("HOME");
        // SAFETY: serialized by `lock_runtime_env`; restored before returning.
        unsafe { std::env::set_var("HOME", "/home/op") };
        let path = unit_path();
        // SAFETY: see above.
        unsafe {
            match prior {
                Some(v) => std::env::set_var("HOME", v),
                None => std::env::remove_var("HOME"),
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
        let prior = std::env::var_os("HOME");
        // SAFETY: serialized by `lock_runtime_env`; restored below.
        unsafe { std::env::set_var("HOME", tmp.path()) };
        let mut runner = RecordingRunner {
            fail_args: Some(vec!["--user", "daemon-reload"]),
            ..RecordingRunner::default()
        };
        let result = install_with_runner(&sample_invocation(), &mut runner);
        // SAFETY: serialized by `lock_runtime_env`.
        unsafe {
            match prior {
                Some(value) => std::env::set_var("HOME", value),
                None => std::env::remove_var("HOME"),
            }
        }

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
        let prior = std::env::var_os("HOME");
        // SAFETY: serialized by `lock_runtime_env`; restored below.
        unsafe { std::env::set_var("HOME", tmp.path()) };
        let mut runner = RecordingRunner {
            fail_nth_args: Some((vec!["--user", "enable", SYSTEMD_UNIT], 1)),
            ..RecordingRunner::default()
        };
        let result = install_with_runner(&sample_invocation(), &mut runner);
        // SAFETY: serialized by `lock_runtime_env`.
        unsafe {
            match prior {
                Some(value) => std::env::set_var("HOME", value),
                None => std::env::remove_var("HOME"),
            }
        }

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
        let prior = std::env::var_os("HOME");
        // SAFETY: serialized by `lock_runtime_env`; restored below.
        unsafe { std::env::set_var("HOME", tmp.path()) };
        let file = unit_path().expect("unit path");
        std::fs::create_dir_all(file.parent().expect("unit parent")).expect("create parent");
        std::fs::write(&file, "unit").expect("write unit");
        let mut runner = RecordingRunner {
            unit_state_output: Some("loaded\nenabled\n".to_string()),
            ..RecordingRunner::default()
        };
        let result = uninstall_with_runner(&mut runner);
        // SAFETY: serialized by `lock_runtime_env`.
        unsafe {
            match prior {
                Some(value) => std::env::set_var("HOME", value),
                None => std::env::remove_var("HOME"),
            }
        }

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
    fn uninstall_disable_failure_preserves_unit_file() {
        let _lock = crate::runtime::test_env::lock_runtime_env();
        let tmp = tempfile::tempdir().expect("tempdir");
        let prior = std::env::var_os("HOME");
        // SAFETY: serialized by `lock_runtime_env`; restored below.
        unsafe { std::env::set_var("HOME", tmp.path()) };
        let file = unit_path().expect("unit path");
        std::fs::create_dir_all(file.parent().expect("unit parent")).expect("create parent");
        std::fs::write(&file, "unit").expect("write unit");
        let mut runner = RecordingRunner {
            fail_args: Some(vec!["--user", "disable", "--now", SYSTEMD_UNIT]),
            unit_state_output: Some("loaded\nenabled\n".to_string()),
            ..RecordingRunner::default()
        };
        let result = uninstall_with_runner(&mut runner);
        // SAFETY: serialized by `lock_runtime_env`.
        unsafe {
            match prior {
                Some(value) => std::env::set_var("HOME", value),
                None => std::env::remove_var("HOME"),
            }
        }

        assert!(result.is_err());
        assert!(file.exists(), "failed disable must not remove the unit");
        assert_eq!(
            runner.labels,
            ["systemctl show unit state", "systemctl disable"]
        );
    }

    #[test]
    fn failed_reinstall_restores_previous_unit() {
        let _lock = crate::runtime::test_env::lock_runtime_env();
        let tmp = tempfile::tempdir().expect("tempdir");
        let prior = std::env::var_os("HOME");
        // SAFETY: serialized by `lock_runtime_env`; restored below.
        unsafe { std::env::set_var("HOME", tmp.path()) };
        let file = unit_path().expect("unit path");
        std::fs::create_dir_all(file.parent().expect("unit parent")).expect("create parent");
        std::fs::write(&file, "previous unit").expect("write previous unit");
        let mut runner = RecordingRunner {
            unit_state_output: Some("loaded\nenabled\n".to_string()),
            fail_nth_args: Some((vec!["--user", "enable", SYSTEMD_UNIT], 1)),
            ..RecordingRunner::default()
        };
        let result = install_with_runner(&sample_invocation(), &mut runner);
        // SAFETY: serialized by `lock_runtime_env`.
        unsafe {
            match prior {
                Some(value) => std::env::set_var("HOME", value),
                None => std::env::remove_var("HOME"),
            }
        }

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
                    "--value",
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
        let prior = std::env::var_os("HOME");
        // SAFETY: serialized by `lock_runtime_env`; restored below.
        unsafe { std::env::set_var("HOME", tmp.path()) };
        let file = unit_path().expect("unit path");
        std::fs::create_dir_all(file.parent().expect("unit parent")).expect("create parent");
        std::fs::write(&file, "previous unit").expect("write previous unit");
        let mut runner = RecordingRunner {
            unit_state_output: Some("loaded\ndisabled\n".to_string()),
            fail_nth_args: Some((vec!["--user", "enable", SYSTEMD_UNIT], 1)),
            ..RecordingRunner::default()
        };
        let result = install_with_runner(&sample_invocation(), &mut runner);
        // SAFETY: serialized by `lock_runtime_env`.
        unsafe {
            match prior {
                Some(value) => std::env::set_var("HOME", value),
                None => std::env::remove_var("HOME"),
            }
        }

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
                    "--value",
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
        let prior = std::env::var_os("HOME");
        // SAFETY: serialized by `lock_runtime_env`; restored below.
        unsafe { std::env::set_var("HOME", tmp.path()) };
        let mut runner = RecordingRunner::default();
        let result = uninstall_with_runner(&mut runner);
        // SAFETY: serialized by `lock_runtime_env`.
        unsafe {
            match prior {
                Some(value) => std::env::set_var("HOME", value),
                None => std::env::remove_var("HOME"),
            }
        }

        result.expect("absent uninstall succeeds");
        assert_eq!(runner.labels, ["systemctl show unit state"]);
    }

    #[test]
    fn absent_uninstall_disables_loaded_or_enabled_manager_state() {
        let _lock = crate::runtime::test_env::lock_runtime_env();
        let tmp = tempfile::tempdir().expect("tempdir");
        let prior = std::env::var_os("HOME");
        // SAFETY: serialized by `lock_runtime_env`; restored below.
        unsafe { std::env::set_var("HOME", tmp.path()) };
        let mut runner = RecordingRunner {
            unit_state_output: Some("loaded\nenabled\n".to_string()),
            ..RecordingRunner::default()
        };
        let result = uninstall_with_runner(&mut runner);
        // SAFETY: serialized by `lock_runtime_env`.
        unsafe {
            match prior {
                Some(value) => std::env::set_var("HOME", value),
                None => std::env::remove_var("HOME"),
            }
        }

        result.expect("orphan manager state uninstall succeeds");
        assert_eq!(
            runner.args,
            [
                vec![
                    "--user",
                    "show",
                    "--property=LoadState",
                    "--property=UnitFileState",
                    "--value",
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
        let prior = std::env::var_os("HOME");
        // SAFETY: serialized by `lock_runtime_env`; restored below.
        unsafe { std::env::set_var("HOME", tmp.path()) };
        let mut runner = RecordingRunner {
            fail_args: Some(vec!["--user", "enable", SYSTEMD_UNIT]),
            fail_nth_args: Some((vec!["--user", "daemon-reload"], 2)),
            ..RecordingRunner::default()
        };
        let error = install_with_runner(&sample_invocation(), &mut runner)
            .expect_err("enable and compensating reload failure must surface");
        // SAFETY: serialized by `lock_runtime_env`.
        unsafe {
            match prior {
                Some(value) => std::env::set_var("HOME", value),
                None => std::env::remove_var("HOME"),
            }
        }

        let rendered = error.to_string();
        assert!(rendered.contains("systemctl enable"), "{rendered}");
        assert!(rendered.contains("reload restored unit"), "{rendered}");
        assert!(rendered.contains("rollback failures"), "{rendered}");
    }

    #[test]
    fn uninstall_reload_failure_restores_unit_for_recovery() {
        let _lock = crate::runtime::test_env::lock_runtime_env();
        let tmp = tempfile::tempdir().expect("tempdir");
        let prior = std::env::var_os("HOME");
        // SAFETY: serialized by `lock_runtime_env`; restored below.
        unsafe { std::env::set_var("HOME", tmp.path()) };
        let file = unit_path().expect("unit path");
        std::fs::create_dir_all(file.parent().expect("unit parent")).expect("create parent");
        std::fs::write(&file, "previous unit").expect("write unit");
        let mut runner = RecordingRunner {
            unit_state_output: Some("loaded\nenabled\n".to_string()),
            fail_nth_args: Some((vec!["--user", "daemon-reload"], 1)),
            ..RecordingRunner::default()
        };
        let result = uninstall_with_runner(&mut runner);
        // SAFETY: serialized by `lock_runtime_env`.
        unsafe {
            match prior {
                Some(value) => std::env::set_var("HOME", value),
                None => std::env::remove_var("HOME"),
            }
        }

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
    fn stop_propagates_service_manager_failure_when_unit_exists() {
        let _lock = crate::runtime::test_env::lock_runtime_env();
        let tmp = tempfile::tempdir().expect("tempdir");
        let prior = std::env::var_os("HOME");
        // SAFETY: serialized by `lock_runtime_env`; restored below.
        unsafe { std::env::set_var("HOME", tmp.path()) };
        let file = unit_path().expect("unit path");
        std::fs::create_dir_all(file.parent().expect("unit parent")).expect("create parent");
        std::fs::write(file, "unit").expect("write unit");
        let mut runner = RecordingRunner {
            fail_args: Some(vec!["--user", "stop", SYSTEMD_UNIT]),
            ..RecordingRunner::default()
        };
        let result = stop_with_runner(&mut runner);
        // SAFETY: serialized by `lock_runtime_env`.
        unsafe {
            match prior {
                Some(value) => std::env::set_var("HOME", value),
                None => std::env::remove_var("HOME"),
            }
        }

        assert!(result.is_err());
        assert_eq!(runner.labels, ["systemctl stop"]);
    }

    #[test]
    fn status_reports_not_installed_and_skips_systemctl_query_when_unit_absent() {
        let _lock = crate::runtime::test_env::lock_runtime_env();
        let tmp = tempfile::tempdir().expect("tempdir");
        let prior = std::env::var_os("HOME");
        // SAFETY: serialized by `lock_runtime_env`; restored below.
        unsafe { std::env::set_var("HOME", tmp.path()) };
        let mut runner = RecordingRunner::default();
        let result = status_with_runner(&mut runner);
        // SAFETY: serialized by `lock_runtime_env`.
        unsafe {
            match prior {
                Some(value) => std::env::set_var("HOME", value),
                None => std::env::remove_var("HOME"),
            }
        }

        result.expect("status succeeds when not installed");
        assert!(
            runner.labels.is_empty(),
            "must not query systemctl when the unit file is absent"
        );
    }

    #[test]
    fn status_propagates_capture_failure() {
        let _lock = crate::runtime::test_env::lock_runtime_env();
        let tmp = tempfile::tempdir().expect("tempdir");
        let prior = std::env::var_os("HOME");
        // SAFETY: serialized by `lock_runtime_env`; restored below.
        unsafe { std::env::set_var("HOME", tmp.path()) };
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
        // SAFETY: serialized by `lock_runtime_env`.
        unsafe {
            match prior {
                Some(value) => std::env::set_var("HOME", value),
                None => std::env::remove_var("HOME"),
            }
        }

        assert!(result.is_err());
    }

    // ── restart ─────────────────────────────────────────────────

    #[test]
    fn restart_running_service_stops_then_starts() {
        let _lock = crate::runtime::test_env::lock_runtime_env();
        let tmp = tempfile::tempdir().expect("tempdir");
        let prior = std::env::var_os("HOME");
        // SAFETY: serialized by `lock_runtime_env`; restored below.
        unsafe { std::env::set_var("HOME", tmp.path()) };
        let file = unit_path().expect("unit path");
        std::fs::create_dir_all(file.parent().expect("unit parent")).expect("create parent");
        std::fs::write(&file, "unit").expect("write unit");
        let mut runner = RecordingRunner {
            active_state_output: Some("active\n".to_string()),
            ..RecordingRunner::default()
        };
        let result = restart_with_runner(&mut runner);
        // SAFETY: serialized by `lock_runtime_env`.
        unsafe {
            match prior {
                Some(value) => std::env::set_var("HOME", value),
                None => std::env::remove_var("HOME"),
            }
        }

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
        let prior = std::env::var_os("HOME");
        // SAFETY: serialized by `lock_runtime_env`; restored below.
        unsafe { std::env::set_var("HOME", tmp.path()) };
        let file = unit_path().expect("unit path");
        std::fs::create_dir_all(file.parent().expect("unit parent")).expect("create parent");
        std::fs::write(&file, "unit").expect("write unit");
        let mut runner = RecordingRunner {
            active_state_output: Some("inactive\n".to_string()),
            ..RecordingRunner::default()
        };
        let result = restart_with_runner(&mut runner);
        // SAFETY: serialized by `lock_runtime_env`.
        unsafe {
            match prior {
                Some(value) => std::env::set_var("HOME", value),
                None => std::env::remove_var("HOME"),
            }
        }

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
        let prior = std::env::var_os("HOME");
        // SAFETY: serialized by `lock_runtime_env`; restored below.
        unsafe { std::env::set_var("HOME", tmp.path()) };
        let mut runner = RecordingRunner::default();
        let result = restart_with_runner(&mut runner);
        // SAFETY: serialized by `lock_runtime_env`.
        unsafe {
            match prior {
                Some(value) => std::env::set_var("HOME", value),
                None => std::env::remove_var("HOME"),
            }
        }

        let error = result.expect_err("restart without an installed service must error");
        assert!(error.to_string().contains("service install"));
        assert!(runner.labels.is_empty(), "no commands should run");
    }

    #[test]
    fn restart_reports_stopped_when_start_fails_after_successful_stop() {
        let _lock = crate::runtime::test_env::lock_runtime_env();
        let tmp = tempfile::tempdir().expect("tempdir");
        let prior = std::env::var_os("HOME");
        // SAFETY: serialized by `lock_runtime_env`; restored below.
        unsafe { std::env::set_var("HOME", tmp.path()) };
        let file = unit_path().expect("unit path");
        std::fs::create_dir_all(file.parent().expect("unit parent")).expect("create parent");
        std::fs::write(&file, "unit").expect("write unit");
        let mut runner = RecordingRunner {
            active_state_output: Some("active\n".to_string()),
            fail_args: Some(vec!["--user", "start", SYSTEMD_UNIT]),
            ..RecordingRunner::default()
        };
        let result = restart_with_runner(&mut runner);
        // SAFETY: serialized by `lock_runtime_env`.
        unsafe {
            match prior {
                Some(value) => std::env::set_var("HOME", value),
                None => std::env::remove_var("HOME"),
            }
        }

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
        let prior = std::env::var_os("HOME");
        // SAFETY: serialized by `lock_runtime_env`; restored below.
        unsafe { std::env::set_var("HOME", tmp.path()) };
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
                [vec![
                    "--user",
                    "show",
                    "--property=ActiveState",
                    "--value",
                    SYSTEMD_UNIT,
                ]]
            );
        }
        // SAFETY: serialized by `lock_runtime_env`.
        unsafe {
            match prior {
                Some(value) => std::env::set_var("HOME", value),
                None => std::env::remove_var("HOME"),
            }
        }
    }
}
