//! macOS launchd generators, path resolution, status matching, and verb
//! bodies for `ironclaw-reborn service`.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::context::RebornCliContext;
use crate::serve_invocation::ServeInvocation;

use super::{SERVICE_LABEL, ServiceCommandRunner, home_dir};

// ── Escaping ────────────────────────────────────────────────────

/// XML-escape a value for embedding in a `.plist` `<string>` element.
fn xml_escape(raw: &str) -> String {
    raw.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

// ── Plist generation ────────────────────────────────────────────

fn plist_content(
    invocation: &ServeInvocation,
    working_directory: &Path,
    stdout_log: &Path,
    stderr_log: &Path,
) -> String {
    let program_arguments: String = std::iter::once(invocation.exe.display().to_string())
        .chain(invocation.args.iter().cloned())
        .map(|value| format!("    <string>{}</string>", xml_escape(&value)))
        .collect::<Vec<_>>()
        .join("\n");

    let environment_variables: String = invocation
        .env
        .iter()
        .map(|(key, value)| {
            format!(
                "    <key>{}</key>\n    <string>{}</string>",
                xml_escape(key),
                xml_escape(value)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{label}</string>
  <key>ProgramArguments</key>
  <array>
{program_arguments}
  </array>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <true/>
  <key>WorkingDirectory</key>
  <string>{working_directory}</string>
  <key>EnvironmentVariables</key>
  <dict>
{environment_variables}
  </dict>
  <key>StandardOutPath</key>
  <string>{stdout}</string>
  <key>StandardErrorPath</key>
  <string>{stderr}</string>
</dict>
</plist>
"#,
        label = SERVICE_LABEL,
        working_directory = xml_escape(&working_directory.display().to_string()),
        stdout = xml_escape(&stdout_log.display().to_string()),
        stderr = xml_escape(&stderr_log.display().to_string()),
    )
}

// ── Path helpers ────────────────────────────────────────────────

fn plist_path() -> Result<PathBuf> {
    Ok(home_dir()?
        .join("Library")
        .join("LaunchAgents")
        .join(format!("{SERVICE_LABEL}.plist")))
}

// ── Status matching ─────────────────────────────────────────────

/// The three states `launchctl list` can report for a given label, read
/// from its three whitespace-separated columns (PID, last-exit-status,
/// label). Mirrors `launchd_status_from_line` in
/// `ironclaw_reborn_composition::observability::operator_service_lifecycle`
/// (copied shape, not imported — that module is a different crate and
/// this one intentionally does not depend on it for a few lines of
/// parsing).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LaunchdStatus {
    Running,
    Stopped,
    Failed,
}

/// Parse one `launchctl list` line into `(status, label)`. The PID
/// column is `-` when the job is loaded but not currently running (the
/// bug this fixes: the old `service_running` treated any 3-field line —
/// including a `-` PID — as running). A numeric PID means running; a
/// non-numeric PID with a nonzero last-exit-status means the job died
/// and hasn't been respawned yet (`Failed`); anything else is `Stopped`.
fn launchd_status_from_line(line: &str) -> Option<(LaunchdStatus, &str)> {
    let mut columns = line.split_whitespace();
    let pid = columns.next()?;
    let exit_status = columns.next()?;
    let label = columns.next()?;
    let status = if pid.parse::<i32>().is_ok() {
        LaunchdStatus::Running
    } else if exit_status.parse::<i32>().is_ok_and(|status| status != 0) {
        LaunchdStatus::Failed
    } else {
        LaunchdStatus::Stopped
    };
    Some((status, label))
}

/// The status of our label's line in `launchctl list`, if it appears at
/// all (i.e. the job is currently loaded, running or not).
fn find_label_status(launchctl_list_output: &str) -> Option<LaunchdStatus> {
    launchctl_list_output.lines().find_map(|line| {
        launchd_status_from_line(line)
            .and_then(|(status, label)| (label == SERVICE_LABEL).then_some(status))
    })
}

fn service_running(launchctl_list_output: &str) -> bool {
    find_label_status(launchctl_list_output) == Some(LaunchdStatus::Running)
}

/// Whether the label is registered with launchd at all, regardless of
/// whether it currently has a live PID. Used where a loaded-but-stopped
/// job still needs to be torn down / reloaded (uninstall, reinstall) —
/// unlike `service_running`, which only asks "does it have a PID right
/// now" (used for the user-facing running/stopped status line).
fn service_loaded(launchctl_list_output: &str) -> bool {
    find_label_status(launchctl_list_output).is_some()
}

/// Whether `service status` should report the service as installed:
/// either the plist file exists, or launchd still has the label loaded
/// (an orphan left behind after the plist was removed out-of-band).
fn resolve_installed(file_exists: bool, loaded: bool) -> bool {
    file_exists || loaded
}

/// Gate for the shared "keeps the OLD definition" advisory, launchd side.
/// The unload/load/start reload `install_with_runner` runs when the label
/// was already loaded makes the new definition live immediately, so the
/// note (which claims a running process is still on the old definition)
/// would be false in that case — suppress it. When the label was NOT
/// loaded no reload ran, so a replaced file leaves the advisory accurate.
fn launchd_install_advisory(replaced_existing: bool, was_loaded: bool) -> Option<&'static str> {
    super::replaced_existing_service_file_note(replaced_existing && !was_loaded)
}

// ── Verb bodies ─────────────────────────────────────────────────

/// Returns whether a pre-existing plist file at the target path was
/// replaced by this install (captured before the write). Exposed for
/// tests, and for `super::ServicePlatform::install_with_runner` (the
/// runner-injectable install path driven by the `service install`
/// preflight-warning integration test); production wraps this via
/// [`install`], which discards the bool once the advisory line has been
/// printed.
pub(super) fn install_with_runner(
    context: &RebornCliContext,
    invocation: &ServeInvocation,
    runner: &mut dyn ServiceCommandRunner,
) -> Result<bool> {
    let file = plist_path()?;
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let logs_dir = context.boot_config().home().path().join("logs");
    std::fs::create_dir_all(&logs_dir).with_context(|| format!("create {}", logs_dir.display()))?;
    // Unrotated: launchd appends to these paths forever across restarts.
    // This module wires no rotation (e.g. `newsyslog`) — operators must
    // add it externally if long-running installs need it.
    let stdout_log = logs_dir.join("serve.stdout.log");
    let stderr_log = logs_dir.join("serve.stderr.log");
    let list =
        runner.run_capture_checked("launchctl list", Command::new("launchctl").arg("list"))?;
    let was_loaded = service_loaded(&list);
    // Captured before the write: a pre-existing file at this path may
    // have been installed by this CLI's own prior run, or by the WebUI
    // operator facade (`RebornLocalServiceLifecycle`) — both surfaces
    // target the same label/path by design (see the module doc). Either
    // way the write below atomically replaces it.
    let replaced_existing = file.exists();
    // WorkingDirectory anchors cwd at `<reborn_home>/workspace`, not
    // launchd's default `/` and not the Reborn home itself — the home is
    // an ancestor of every default skill root, so it still trips
    // composition's `paths_overlap` check (see `service_working_directory`).
    let reborn_home = context.boot_config().home().path();
    let working_directory = super::ensure_service_working_directory(reborn_home)?;
    let plist = plist_content(invocation, &working_directory, &stdout_log, &stderr_log);
    super::write_atomic(&file, plist.as_bytes())?;
    if was_loaded {
        // A loaded job keeps running off its in-memory definition until
        // reloaded; overwriting the plist file alone does not make it
        // pick up the new ProgramArguments/EnvironmentVariables. Force a
        // reload with the same legacy verbs `start`/`stop` already use.
        runner.run_checked(
            "launchctl unload",
            Command::new("launchctl").arg("unload").arg("-w").arg(&file),
        )?;
        runner.run_checked(
            "launchctl load",
            Command::new("launchctl").arg("load").arg("-w").arg(&file),
        )?;
        runner.run_checked(
            "launchctl start",
            Command::new("launchctl").arg("start").arg(SERVICE_LABEL),
        )?;
    }
    println!("Installed launchd service: {}", file.display());
    if let Some(note) = launchd_install_advisory(replaced_existing, was_loaded) {
        println!("{note}");
    }
    println!("  Start with: ironclaw-reborn service start");
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
    let plist = plist_path()?;
    if !plist.exists() {
        bail!("Service not installed. Run `ironclaw-reborn service install` first.");
    }
    // A bare `launchctl load -w` fails when the label is already loaded
    // (loaded-but-stopped, i.e. a `-` PID) — the same condition `install`
    // already reloads around. Query first and skip straight to `start`
    // when the label is already registered with launchd; only a
    // not-loaded label needs the `load` step.
    let list =
        runner.run_capture_checked("launchctl list", Command::new("launchctl").arg("list"))?;
    if !service_loaded(&list) {
        runner.run_checked(
            "launchctl load",
            Command::new("launchctl").arg("load").arg("-w").arg(&plist),
        )?;
    }
    runner.run_checked(
        "launchctl start",
        Command::new("launchctl").arg("start").arg(SERVICE_LABEL),
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
    let plist = plist_path()?;
    if !plist.exists() {
        if verbose {
            println!("Service stopped");
        }
        return Ok(());
    }
    let list =
        runner.run_capture_checked("launchctl list", Command::new("launchctl").arg("list"))?;
    if !service_loaded(&list) {
        if verbose {
            println!("Service stopped");
        }
        return Ok(());
    }
    // A loaded job — running or not (a `-` PID) — stays registered with
    // launchd for respawn until unloaded; gating on `service_running` alone
    // (the finding-1 bug, already fixed for `status`/`uninstall`) left a
    // loaded-but-stopped KeepAlive job undisturbed.
    runner.run_checked(
        "launchctl stop",
        Command::new("launchctl").arg("stop").arg(SERVICE_LABEL),
    )?;
    runner.run_checked(
        "launchctl unload",
        Command::new("launchctl")
            .arg("unload")
            .arg("-w")
            .arg(&plist),
    )?;
    if verbose {
        println!("Service stopped");
    }
    Ok(())
}

/// Detects install/running state, then delegates the stop/start decision
/// tree to [`super::restart_generic`], which both platforms share.
///
/// Passes `service_loaded` (not `service_running`) as the "was active"
/// flag: a loaded-but-not-running (`-` PID) job must go through the same
/// stop-then-start reload as a running one, since launchd errors on a bare
/// `launchctl load` of an already-loaded label. `service_running` implies
/// `service_loaded`, so this subsumes the previously-running case without
/// changing its behavior.
pub(super) fn restart_with_runner(runner: &mut dyn ServiceCommandRunner) -> Result<()> {
    let plist = plist_path()?;
    let installed = plist.exists();
    let was_active = if installed {
        let list =
            runner.run_capture_checked("launchctl list", Command::new("launchctl").arg("list"))?;
        service_loaded(&list)
    } else {
        false
    };
    super::restart_generic(
        runner,
        installed,
        was_active,
        stop_with_runner_quiet,
        start_with_runner_quiet,
    )
}

/// Installed/running state shared by [`status_with_runner`] and
/// [`current_state_with_runner`] so the two don't drift on how "installed"
/// and "running" are derived from `launchctl list`.
struct LaunchdStatusInfo {
    installed: bool,
    running: bool,
}

fn resolve_status_info(runner: &mut dyn ServiceCommandRunner) -> Result<LaunchdStatusInfo> {
    let file_exists = plist_path()?.exists();
    // Query launchctl unconditionally — a plist that was removed
    // out-of-band while the job is still loaded is an orphan we must
    // still report as installed, not silently claim "not installed".
    let list =
        runner.run_capture_checked("launchctl list", Command::new("launchctl").arg("list"))?;
    let running = service_running(&list);
    let installed = resolve_installed(file_exists, service_loaded(&list));
    Ok(LaunchdStatusInfo { installed, running })
}

pub(super) fn status_with_runner(runner: &mut dyn ServiceCommandRunner) -> Result<()> {
    let plist = plist_path()?;
    let info = resolve_status_info(runner)?;
    println!(
        "Service: {}",
        super::status_label(info.installed, info.running)
    );
    println!("Unit: {}", plist.display());
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

pub(super) fn uninstall_with_runner(runner: &mut dyn ServiceCommandRunner) -> Result<()> {
    let file = plist_path()?;
    let list =
        runner.run_capture_checked("launchctl list", Command::new("launchctl").arg("list"))?;
    if service_loaded(&list) {
        // `stop` alone is insufficient for a KeepAlive agent: launchd
        // immediately restarts it. Target the registered label so a loaded
        // job left behind by an older broken uninstall can still be removed
        // even when its plist path is already absent.
        runner.run_checked(
            "launchctl stop",
            Command::new("launchctl").arg("stop").arg(SERVICE_LABEL),
        )?;
        let uid = runner
            .run_capture_checked("id -u", Command::new("id").arg("-u"))?
            .trim()
            .parse::<u32>()
            .context("parse numeric uid from `id -u`")?;
        runner.run_checked(
            "launchctl bootout",
            Command::new("launchctl")
                .arg("bootout")
                .arg(format!("gui/{uid}/{SERVICE_LABEL}")),
        )?;
    }
    if file.exists() {
        std::fs::remove_file(&file).with_context(|| format!("remove {}", file.display()))?;
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
        launchctl_list: String,
        fail_args: Option<Vec<&'static str>>,
        fail_capture_args: Option<Vec<&'static str>>,
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
            // Keep `launchctl_list` (read back by subsequent
            // `run_capture_checked("launchctl list", ...)` calls) in sync
            // with the label/unload/bootout operations this mock just
            // recorded, so a test spanning multiple `launchctl list`
            // queries (e.g. `restart` composing `stop` then `start`, both
            // of which query the label's loaded state) observes the same
            // state transitions the real `launchctl` daemon would report,
            // instead of a stale snapshot from before this call.
            match label {
                "launchctl unload" | "launchctl bootout" => self.launchctl_list.clear(),
                "launchctl load" => self.launchctl_list = format!("123\t0\t{SERVICE_LABEL}\n"),
                _ => {}
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
            let program = command.get_program().to_string_lossy();
            let args = self.args.last().cloned().unwrap_or_default();
            match (program.as_ref(), args.as_slice()) {
                ("launchctl", [arg]) if arg == "list" => Ok(self.launchctl_list.clone()),
                ("id", [arg]) if arg == "-u" => Ok("501\n".to_string()),
                _ => anyhow::bail!("unexpected capture argv: {program} {args:?}"),
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
    fn install_reloads_when_label_is_already_loaded() {
        let _lock = crate::runtime::test_env::lock_runtime_env();
        let home_tmp = tempfile::tempdir().expect("tempdir");
        let prior = std::env::var_os("HOME");
        // SAFETY: serialized by `lock_runtime_env`; restored below.
        unsafe { std::env::set_var("HOME", home_tmp.path()) };
        let (_ctx_tmp, context) = RebornCliContext::test_context();
        let mut runner = RecordingRunner {
            launchctl_list: format!("123\t0\t{SERVICE_LABEL}\n"),
            ..RecordingRunner::default()
        };
        let result = install_with_runner(&context, &sample_invocation(), &mut runner);
        // SAFETY: serialized by `lock_runtime_env`.
        unsafe {
            match prior {
                Some(value) => std::env::set_var("HOME", value),
                None => std::env::remove_var("HOME"),
            }
        }

        let replaced = result.expect("install must succeed");
        assert_eq!(
            runner.labels,
            [
                "launchctl list",
                "launchctl unload",
                "launchctl load",
                "launchctl start",
            ],
            "an already-loaded job must be reloaded so it picks up the new plist"
        );
        // The unload/load/start sequence above already makes the new
        // definition live, so the "keeps the OLD definition until
        // restart" advisory would be false here and must not print —
        // even though this is a fresh install with no prior plist file
        // (`replaced_existing` is false regardless).
        assert!(
            !replaced,
            "a fresh install (no prior plist) must not report a replacement"
        );
    }

    #[test]
    fn install_suppresses_stale_definition_note_when_reload_already_happened() {
        // Pins the launchd advisory fix: replacing an existing plist while
        // the label is already loaded triggers the unload/load/start
        // reload above, so the running process picks up the new
        // definition immediately. The "keeps the OLD definition until
        // `service restart`" note would be false in that case and must
        // not print. `install_with_runner` only returns `replaced_existing`
        // (not `was_loaded`), so this pins the actual gate
        // `launchd_install_advisory` applies, driven by the same
        // `replaced_existing = true` this scenario produces.
        let _lock = crate::runtime::test_env::lock_runtime_env();
        let home_tmp = tempfile::tempdir().expect("tempdir");
        let prior = std::env::var_os("HOME");
        // SAFETY: serialized by `lock_runtime_env`; restored below.
        unsafe { std::env::set_var("HOME", home_tmp.path()) };
        let (_ctx_tmp, context) = RebornCliContext::test_context();
        let file = plist_path().expect("plist path");
        std::fs::create_dir_all(file.parent().expect("plist parent")).expect("create parent");
        std::fs::write(&file, "pre-existing plist").expect("write pre-existing plist");
        let mut runner = RecordingRunner {
            launchctl_list: format!("123\t0\t{SERVICE_LABEL}\n"),
            ..RecordingRunner::default()
        };
        let result = install_with_runner(&context, &sample_invocation(), &mut runner);
        // SAFETY: serialized by `lock_runtime_env`.
        unsafe {
            match prior {
                Some(value) => std::env::set_var("HOME", value),
                None => std::env::remove_var("HOME"),
            }
        }

        let replaced = result.expect("install over a loaded label must succeed");
        assert!(
            replaced,
            "a pre-existing plist was in fact replaced on disk"
        );
        assert!(
            launchd_install_advisory(replaced, /* was_loaded */ true).is_none(),
            "the note must be suppressed once the label was already loaded and reloaded in place"
        );
    }

    #[test]
    fn launchd_install_advisory_prints_only_when_replaced_and_not_reloaded() {
        assert!(
            launchd_install_advisory(true, false).is_some(),
            "a replaced file with no in-place reload must keep the stale-definition advisory"
        );
        assert!(
            launchd_install_advisory(true, true).is_none(),
            "a replaced file that was reloaded in place must not claim a stale definition"
        );
        assert!(
            launchd_install_advisory(false, false).is_none(),
            "a fresh install has nothing to advise"
        );
        assert!(
            launchd_install_advisory(false, true).is_none(),
            "a fresh install has nothing to advise even if the label was already loaded"
        );
    }

    #[test]
    fn install_skips_reload_when_label_is_not_loaded() {
        let _lock = crate::runtime::test_env::lock_runtime_env();
        let home_tmp = tempfile::tempdir().expect("tempdir");
        let prior = std::env::var_os("HOME");
        // SAFETY: serialized by `lock_runtime_env`; restored below.
        unsafe { std::env::set_var("HOME", home_tmp.path()) };
        let (_ctx_tmp, context) = RebornCliContext::test_context();
        let mut runner = RecordingRunner::default();
        let result = install_with_runner(&context, &sample_invocation(), &mut runner);
        // SAFETY: serialized by `lock_runtime_env`.
        unsafe {
            match prior {
                Some(value) => std::env::set_var("HOME", value),
                None => std::env::remove_var("HOME"),
            }
        }

        let replaced = result.expect("install must succeed");
        assert_eq!(
            runner.labels,
            ["launchctl list"],
            "a fresh (not-loaded) install must not issue a reload sequence"
        );
        assert!(
            !replaced,
            "a fresh install (no prior plist) must not report a replacement"
        );
        assert!(
            super::super::replaced_existing_service_file_note(replaced).is_none(),
            "fresh install must not carry a replaced-file advisory"
        );
    }

    #[test]
    fn install_reports_replaced_existing_when_plist_already_present() {
        // Covers the shared-identity collision case (design doc: "adopt
        // identity"): a plist at this path may have been written by a
        // prior CLI install, or by the WebUI operator facade
        // (`RebornLocalServiceLifecycle`) — both target the same label/
        // path. Either way, `install` must report the replacement so the
        // operator knows a currently-running service (if any) keeps the
        // OLD definition until `service restart`.
        let _lock = crate::runtime::test_env::lock_runtime_env();
        let home_tmp = tempfile::tempdir().expect("tempdir");
        let prior = std::env::var_os("HOME");
        // SAFETY: serialized by `lock_runtime_env`; restored below.
        unsafe { std::env::set_var("HOME", home_tmp.path()) };
        let (_ctx_tmp, context) = RebornCliContext::test_context();
        let file = plist_path().expect("plist path");
        std::fs::create_dir_all(file.parent().expect("plist parent")).expect("create parent");
        // Simulate a pre-existing unit — e.g. one written by the WebUI
        // operator facade with a baked-in secret — that this install
        // must atomically overwrite.
        std::fs::write(&file, "pre-existing plist with a baked-in secret")
            .expect("write pre-existing plist");
        let mut runner = RecordingRunner::default();
        let result = install_with_runner(&context, &sample_invocation(), &mut runner);
        // SAFETY: serialized by `lock_runtime_env`.
        unsafe {
            match prior {
                Some(value) => std::env::set_var("HOME", value),
                None => std::env::remove_var("HOME"),
            }
        }

        let replaced = result.expect("install over an existing plist must succeed");
        assert!(
            replaced,
            "install must report that a pre-existing plist was replaced"
        );
        let note = super::super::replaced_existing_service_file_note(replaced)
            .expect("a replaced install must carry an advisory line");
        assert!(note.contains("Replaced an existing service definition"));
        assert!(note.contains("service restart"));
        let contents = std::fs::read_to_string(&file).expect("read plist after install");
        assert!(
            !contents.contains("baked-in secret"),
            "the pre-existing file's contents must be fully replaced"
        );
    }

    #[test]
    fn xml_escape_handles_reserved_chars() {
        let escaped = xml_escape("<&>\"' and text");
        assert_eq!(escaped, "&lt;&amp;&gt;&quot;&apos; and text");
    }

    #[test]
    fn xml_escape_passes_through_plain_text() {
        assert_eq!(xml_escape("hello world"), "hello world");
    }

    #[test]
    fn plist_content_includes_label() {
        let plist = plist_content(
            &sample_invocation(),
            Path::new("/home/op/.ironclaw/reborn"),
            Path::new("/home/op/.ironclaw/reborn/logs/serve.stdout.log"),
            Path::new("/home/op/.ironclaw/reborn/logs/serve.stderr.log"),
        );
        assert!(plist.contains("<key>Label</key>"));
        assert!(plist.contains(SERVICE_LABEL));
    }

    #[test]
    fn plist_content_includes_program_arguments() {
        let plist = plist_content(
            &sample_invocation(),
            Path::new("/home/op/.ironclaw/reborn"),
            Path::new("/tmp/o.log"),
            Path::new("/tmp/e.log"),
        );
        assert!(plist.contains("<string>/usr/local/bin/ironclaw-reborn</string>"));
        assert!(plist.contains("<string>serve</string>"));
    }

    #[test]
    fn plist_content_includes_environment_variables() {
        let plist = plist_content(
            &sample_invocation(),
            Path::new("/home/op/.ironclaw/reborn"),
            Path::new("/tmp/o.log"),
            Path::new("/tmp/e.log"),
        );
        assert!(plist.contains("<key>IRONCLAW_REBORN_HOME</key>"));
        assert!(plist.contains("<string>/home/op/.ironclaw/reborn</string>"));
    }

    #[test]
    fn plist_content_marks_run_at_load_and_keep_alive_true() {
        let plist = plist_content(
            &sample_invocation(),
            Path::new("/home/op/.ironclaw/reborn"),
            Path::new("/tmp/o.log"),
            Path::new("/tmp/e.log"),
        );
        assert!(plist.contains("<key>RunAtLoad</key>"));
        assert!(plist.contains("<key>KeepAlive</key>"));
        // Two `<true/>` markers: one for RunAtLoad, one for KeepAlive.
        assert_eq!(plist.matches("<true/>").count(), 2);
    }

    #[test]
    fn plist_content_includes_stdout_and_stderr_log_paths() {
        let plist = plist_content(
            &sample_invocation(),
            Path::new("/home/op/.ironclaw/reborn"),
            Path::new("/home/op/.ironclaw/reborn/logs/serve.stdout.log"),
            Path::new("/home/op/.ironclaw/reborn/logs/serve.stderr.log"),
        );
        assert!(plist.contains("<string>/home/op/.ironclaw/reborn/logs/serve.stdout.log</string>"));
        assert!(plist.contains("<string>/home/op/.ironclaw/reborn/logs/serve.stderr.log</string>"));
    }

    /// Pins the crash-loop fix: without WorkingDirectory, launchd runs with
    /// cwd=`/`, which overlaps a default skill root and composition refuses
    /// to boot. `plist_content` just writes the caller-supplied path
    /// faithfully — see `install_with_runner` /
    /// `ensure_service_working_directory` for the actual path choice.
    #[test]
    fn plist_content_includes_working_directory_line() {
        let plist = plist_content(
            &sample_invocation(),
            Path::new("/home/op/.ironclaw/reborn/workspace"),
            Path::new("/home/op/.ironclaw/reborn/logs/serve.stdout.log"),
            Path::new("/home/op/.ironclaw/reborn/logs/serve.stderr.log"),
        );
        assert!(plist.contains("<key>WorkingDirectory</key>"));
        assert!(plist.contains("<string>/home/op/.ironclaw/reborn/workspace</string>"));
        // WorkingDirectory precedes EnvironmentVariables, matching the doc
        // comment's placement.
        let working_dir_index = plist.find("<key>WorkingDirectory</key>").unwrap();
        let env_vars_index = plist.find("<key>EnvironmentVariables</key>").unwrap();
        assert!(working_dir_index < env_vars_index);
    }

    #[test]
    fn plist_content_escapes_xml_reserved_chars_in_env_value() {
        let invocation = ServeInvocation {
            exe: PathBuf::from("/usr/local/bin/ironclaw-reborn"),
            args: vec!["serve".to_string()],
            env: vec![(
                "IRONCLAW_REBORN_PROFILE".to_string(),
                "a&b<c>d\"e'f".to_string(),
            )],
        };
        let plist = plist_content(
            &invocation,
            Path::new("/home/op/.ironclaw/reborn"),
            Path::new("/tmp/o.log"),
            Path::new("/tmp/e.log"),
        );
        assert!(plist.contains("a&amp;b&lt;c&gt;d&quot;e&apos;f"));
        assert!(!plist.contains("a&b<c>d\"e'f"));
    }

    #[test]
    fn plist_path_ends_with_expected_suffix() {
        let _lock = crate::runtime::test_env::lock_runtime_env();
        let prior = std::env::var_os("HOME");
        // SAFETY: serialized by `lock_runtime_env`; restored before returning.
        unsafe { std::env::set_var("HOME", "/home/op") };
        let path = plist_path();
        // SAFETY: see above.
        unsafe {
            match prior {
                Some(v) => std::env::set_var("HOME", v),
                None => std::env::remove_var("HOME"),
            }
        }
        assert_eq!(
            path.expect("must resolve"),
            PathBuf::from("/home/op/Library/LaunchAgents/com.ironclaw.reborn.plist")
        );
    }

    #[test]
    fn service_running_matches_only_the_label_line() {
        let output = "-\t0\tcom.apple.something\n123\t0\tcom.ironclaw.reborn\n";
        assert!(service_running(output));
    }

    #[test]
    fn service_running_false_when_label_absent() {
        let output = "com.apple.something\t0\t0\n";
        assert!(!service_running(output));
        assert!(!service_running(&format!(
            "123\t0\t{SERVICE_LABEL}.helper\n"
        )));
    }

    #[test]
    fn uninstall_boots_out_loaded_keepalive_job_before_removing_plist() {
        let _lock = crate::runtime::test_env::lock_runtime_env();
        let tmp = tempfile::tempdir().expect("tempdir");
        let prior = std::env::var_os("HOME");
        // SAFETY: serialized by `lock_runtime_env`; restored below.
        unsafe { std::env::set_var("HOME", tmp.path()) };
        let file = plist_path().expect("plist path");
        std::fs::create_dir_all(file.parent().expect("plist parent")).expect("create parent");
        std::fs::write(&file, "plist").expect("write plist");
        let mut runner = RecordingRunner {
            launchctl_list: format!("-\t0\t{SERVICE_LABEL}\n"),
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
                "launchctl list",
                "launchctl stop",
                "id -u",
                "launchctl bootout"
            ]
        );
        assert_eq!(runner.args[1], ["stop", SERVICE_LABEL]);
        assert_eq!(runner.args[2], ["-u"]);
        assert_eq!(
            runner.args[3],
            [
                "bootout".to_string(),
                "gui/501/com.ironclaw.reborn".to_string()
            ]
        );
    }

    #[test]
    fn uninstall_bootout_failure_preserves_plist() {
        let _lock = crate::runtime::test_env::lock_runtime_env();
        let tmp = tempfile::tempdir().expect("tempdir");
        let prior = std::env::var_os("HOME");
        // SAFETY: serialized by `lock_runtime_env`; restored below.
        unsafe { std::env::set_var("HOME", tmp.path()) };
        let file = plist_path().expect("plist path");
        std::fs::create_dir_all(file.parent().expect("plist parent")).expect("create parent");
        std::fs::write(&file, "plist").expect("write plist");
        let mut runner = RecordingRunner {
            launchctl_list: format!("123\t0\t{SERVICE_LABEL}\n"),
            fail_args: Some(vec!["bootout", "gui/501/com.ironclaw.reborn"]),
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
        assert!(file.exists(), "failed bootout must not remove the plist");
    }

    #[test]
    fn uninstall_is_idempotent_when_plist_is_absent() {
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

        result.expect("repeated uninstall succeeds");
        assert_eq!(runner.labels, ["launchctl list"]);
    }

    #[test]
    fn uninstall_boots_out_loaded_label_when_plist_is_absent() {
        let _lock = crate::runtime::test_env::lock_runtime_env();
        let tmp = tempfile::tempdir().expect("tempdir");
        let prior = std::env::var_os("HOME");
        // SAFETY: serialized by `lock_runtime_env`; restored below.
        unsafe { std::env::set_var("HOME", tmp.path()) };
        let mut runner = RecordingRunner {
            launchctl_list: format!("123\t0\t{SERVICE_LABEL}\n"),
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

        result.expect("loaded orphan uninstall succeeds");
        assert_eq!(
            runner.args,
            [
                vec!["list"],
                vec!["stop", SERVICE_LABEL],
                vec!["-u"],
                vec!["bootout", "gui/501/com.ironclaw.reborn"]
            ]
        );
    }

    #[test]
    fn uninstall_preserves_plist_when_launchctl_list_fails() {
        let _lock = crate::runtime::test_env::lock_runtime_env();
        let tmp = tempfile::tempdir().expect("tempdir");
        let prior = std::env::var_os("HOME");
        // SAFETY: serialized by `lock_runtime_env`; restored below.
        unsafe { std::env::set_var("HOME", tmp.path()) };
        let file = plist_path().expect("plist path");
        std::fs::create_dir_all(file.parent().expect("plist parent")).expect("create parent");
        std::fs::write(&file, "plist").expect("write plist");
        let mut runner = RecordingRunner {
            fail_capture_args: Some(vec!["list"]),
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
        assert!(file.exists(), "failed list must not delete the plist");
        assert_eq!(runner.labels, ["launchctl list"]);
    }

    #[test]
    fn stop_unloads_loaded_job_with_dash_pid() {
        // Mirrors the finding-1 gap already fixed for `status`/`uninstall`:
        // a KeepAlive job with a `-` PID is loaded-but-not-running, and
        // stays registered for respawn until unloaded. The old
        // `stop_with_runner_impl` gated on `service_running` alone, so it
        // read this state as "already stopped" and returned early without
        // ever unloading the job.
        let _lock = crate::runtime::test_env::lock_runtime_env();
        let tmp = tempfile::tempdir().expect("tempdir");
        let prior = std::env::var_os("HOME");
        // SAFETY: serialized by `lock_runtime_env`; restored below.
        unsafe { std::env::set_var("HOME", tmp.path()) };
        let file = plist_path().expect("plist path");
        std::fs::create_dir_all(file.parent().expect("plist parent")).expect("create parent");
        std::fs::write(&file, "plist").expect("write plist");
        let mut runner = RecordingRunner {
            launchctl_list: format!("-\t0\t{SERVICE_LABEL}\n"),
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

        result.expect("stop of a loaded-not-running job succeeds");
        assert_eq!(
            runner.labels,
            ["launchctl list", "launchctl stop", "launchctl unload"],
            "a loaded-but-not-running job must still be stopped/unloaded, not treated as already stopped"
        );
    }

    #[test]
    fn stop_is_idempotent_when_plist_is_absent() {
        let _lock = crate::runtime::test_env::lock_runtime_env();
        let tmp = tempfile::tempdir().expect("tempdir");
        let prior = std::env::var_os("HOME");
        // SAFETY: serialized by `lock_runtime_env`; restored below.
        unsafe { std::env::set_var("HOME", tmp.path()) };
        let mut runner = RecordingRunner::default();
        let result = stop_with_runner(&mut runner);
        // SAFETY: serialized by `lock_runtime_env`.
        unsafe {
            match prior {
                Some(value) => std::env::set_var("HOME", value),
                None => std::env::remove_var("HOME"),
            }
        }

        result.expect("absent stop succeeds");
        assert!(runner.labels.is_empty());
    }

    #[test]
    fn status_propagates_launchctl_list_failure() {
        let _lock = crate::runtime::test_env::lock_runtime_env();
        let tmp = tempfile::tempdir().expect("tempdir");
        let prior = std::env::var_os("HOME");
        // SAFETY: serialized by `lock_runtime_env`; restored below.
        unsafe { std::env::set_var("HOME", tmp.path()) };
        let file = plist_path().expect("plist path");
        std::fs::create_dir_all(file.parent().expect("plist parent")).expect("create parent");
        std::fs::write(&file, "plist").expect("write plist");
        let mut runner = RecordingRunner {
            fail_capture_args: Some(vec!["list"]),
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

    #[test]
    fn status_queries_launchctl_unconditionally_and_reports_not_installed_when_absent_everywhere() {
        // Was: "skips launchctl query when plist absent". That skip was
        // itself the orphan-hiding bug (finding 5) — a plist removed
        // out-of-band while still loaded in launchd would silently read
        // as "not installed". The query must always run so an orphan can
        // be detected; only report "not installed" when the file AND the
        // manager both show nothing.
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
        assert_eq!(
            runner.labels,
            ["launchctl list"],
            "must query launchctl even when the plist is absent, to detect orphans"
        );
    }

    #[test]
    fn resolve_installed_true_when_file_absent_but_manager_still_shows_it_loaded() {
        assert!(!resolve_installed(false, false));
        assert!(resolve_installed(false, true), "orphaned unit still loaded");
        assert!(resolve_installed(true, false));
        assert!(resolve_installed(true, true));
    }

    #[test]
    fn service_running_false_when_loaded_but_stopped_with_dash_pid() {
        // The finding-1 bug: `launchctl list` reports `-` in the PID
        // column for a job that is loaded but not currently running. The
        // old `service_running` only checked that 3 fields were present,
        // so it misread this as running.
        let output = format!("-\t0\t{SERVICE_LABEL}\n");
        assert!(
            !service_running(&output),
            "dash PID must not read as running"
        );
        assert!(
            service_loaded(&output),
            "the label is still registered with launchd even though stopped"
        );
    }

    // ── start ───────────────────────────────────────────────────

    #[test]
    fn start_on_already_loaded_label_skips_bare_load_and_goes_straight_to_start() {
        // RED-then-green: a bare `launchctl load -w` errors on an
        // already-loaded label, while `install` over a loaded job
        // literally tells the operator to run `service start` next. This
        // pins that `start` queries `launchctl list` first and, when the
        // label is already loaded, issues only `launchctl start` — no
        // `load` call that would fail against launchd.
        let _lock = crate::runtime::test_env::lock_runtime_env();
        let tmp = tempfile::tempdir().expect("tempdir");
        let prior = std::env::var_os("HOME");
        // SAFETY: serialized by `lock_runtime_env`; restored below.
        unsafe { std::env::set_var("HOME", tmp.path()) };
        let file = plist_path().expect("plist path");
        std::fs::create_dir_all(file.parent().expect("plist parent")).expect("create parent");
        std::fs::write(&file, "plist").expect("write plist");
        let mut runner = RecordingRunner {
            launchctl_list: format!("-\t0\t{SERVICE_LABEL}\n"),
            ..RecordingRunner::default()
        };
        let result = start_with_runner(&mut runner);
        // SAFETY: serialized by `lock_runtime_env`.
        unsafe {
            match prior {
                Some(value) => std::env::set_var("HOME", value),
                None => std::env::remove_var("HOME"),
            }
        }

        result.expect("start on an already-loaded label must succeed");
        assert_eq!(
            runner.labels,
            ["launchctl list", "launchctl start"],
            "an already-loaded label must not be re-loaded with a bare `launchctl load`"
        );
    }

    #[test]
    fn start_on_not_loaded_label_loads_then_starts() {
        let _lock = crate::runtime::test_env::lock_runtime_env();
        let tmp = tempfile::tempdir().expect("tempdir");
        let prior = std::env::var_os("HOME");
        // SAFETY: serialized by `lock_runtime_env`; restored below.
        unsafe { std::env::set_var("HOME", tmp.path()) };
        let file = plist_path().expect("plist path");
        std::fs::create_dir_all(file.parent().expect("plist parent")).expect("create parent");
        std::fs::write(&file, "plist").expect("write plist");
        let mut runner = RecordingRunner::default();
        let result = start_with_runner(&mut runner);
        // SAFETY: serialized by `lock_runtime_env`.
        unsafe {
            match prior {
                Some(value) => std::env::set_var("HOME", value),
                None => std::env::remove_var("HOME"),
            }
        }

        result.expect("start on a not-loaded label must succeed");
        assert_eq!(
            runner.labels,
            ["launchctl list", "launchctl load", "launchctl start"],
            "a not-loaded label must still be loaded before starting"
        );
    }

    // ── restart ─────────────────────────────────────────────────

    #[test]
    fn restart_running_service_stops_then_starts() {
        let _lock = crate::runtime::test_env::lock_runtime_env();
        let tmp = tempfile::tempdir().expect("tempdir");
        let prior = std::env::var_os("HOME");
        // SAFETY: serialized by `lock_runtime_env`; restored below.
        unsafe { std::env::set_var("HOME", tmp.path()) };
        let file = plist_path().expect("plist path");
        std::fs::create_dir_all(file.parent().expect("plist parent")).expect("create parent");
        std::fs::write(&file, "plist").expect("write plist");
        let mut runner = RecordingRunner {
            launchctl_list: format!("123\t0\t{SERVICE_LABEL}\n"),
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
                "launchctl list",
                "launchctl list",
                "launchctl stop",
                "launchctl unload",
                // `start`'s own label-loaded check (the launchd
                // start-on-loaded guard): by this point `stop` has
                // unloaded the label, so this query correctly reports
                // not-loaded and `start` proceeds to `load` + `start`
                // rather than skipping straight to `start`.
                "launchctl list",
                "launchctl load",
                "launchctl start",
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
        let file = plist_path().expect("plist path");
        std::fs::create_dir_all(file.parent().expect("plist parent")).expect("create parent");
        std::fs::write(&file, "plist").expect("write plist");
        let mut runner = RecordingRunner::default();
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
                "launchctl list",
                // `start`'s own label-loaded check — see the comment in
                // `restart_running_service_stops_then_starts`.
                "launchctl list",
                "launchctl load",
                "launchctl start",
            ]
        );
    }

    #[test]
    fn restart_loaded_not_running_reloads_instead_of_bare_load() {
        // Mirrors the finding-1 gap fixed above for `stop`: a KeepAlive job
        // with a `-` PID is loaded but not running. The old
        // `restart_with_runner` derived `was_running` from `service_running`
        // alone, so this state skipped the stop step entirely and issued a
        // bare `launchctl load` on a label that's already loaded (which
        // launchd errors on). It must instead go through the same
        // stop-then-start reload sequence as the running case, so the
        // already-loaded label is unloaded before it's reloaded.
        let _lock = crate::runtime::test_env::lock_runtime_env();
        let tmp = tempfile::tempdir().expect("tempdir");
        let prior = std::env::var_os("HOME");
        // SAFETY: serialized by `lock_runtime_env`; restored below.
        unsafe { std::env::set_var("HOME", tmp.path()) };
        let file = plist_path().expect("plist path");
        std::fs::create_dir_all(file.parent().expect("plist parent")).expect("create parent");
        std::fs::write(&file, "plist").expect("write plist");
        let mut runner = RecordingRunner {
            launchctl_list: format!("-\t0\t{SERVICE_LABEL}\n"),
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

        result.expect("restart of a loaded-not-running job must succeed");
        assert_eq!(
            runner.labels,
            [
                "launchctl list",
                "launchctl list",
                "launchctl stop",
                "launchctl unload",
                // `start`'s own label-loaded check — see the comment in
                // `restart_running_service_stops_then_starts`.
                "launchctl list",
                "launchctl load",
                "launchctl start",
            ],
            "a loaded-but-not-running label must be unloaded before reload, not bare-loaded"
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
        let file = plist_path().expect("plist path");
        std::fs::create_dir_all(file.parent().expect("plist parent")).expect("create parent");
        std::fs::write(&file, "plist").expect("write plist");
        let mut runner = RecordingRunner {
            launchctl_list: format!("123\t0\t{SERVICE_LABEL}\n"),
            fail_args: Some(vec!["start", SERVICE_LABEL]),
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
                "launchctl list",
                "launchctl list",
                "launchctl stop",
                "launchctl unload",
                // `start`'s own label-loaded check — see the comment in
                // `restart_running_service_stops_then_starts`.
                "launchctl list",
                "launchctl load",
                "launchctl start",
            ]
        );
    }

    #[test]
    fn status_classifies_loaded_and_not_loaded_from_exact_list_command() {
        let _lock = crate::runtime::test_env::lock_runtime_env();
        let tmp = tempfile::tempdir().expect("tempdir");
        let prior = std::env::var_os("HOME");
        // SAFETY: serialized by `lock_runtime_env`; restored below.
        unsafe { std::env::set_var("HOME", tmp.path()) };
        let file = plist_path().expect("plist path");
        std::fs::create_dir_all(file.parent().expect("plist parent")).expect("create parent");
        std::fs::write(&file, "plist").expect("write plist");
        for output in [
            format!("123\t0\t{SERVICE_LABEL}\n"),
            "123\t0\tcom.apple.other\n".to_string(),
        ] {
            let mut runner = RecordingRunner {
                launchctl_list: output,
                ..RecordingRunner::default()
            };
            status_with_runner(&mut runner).expect("status succeeds");
            assert_eq!(runner.args, [vec!["list"]]);
        }
        // SAFETY: serialized by `lock_runtime_env`.
        unsafe {
            match prior {
                Some(value) => std::env::set_var("HOME", value),
                None => std::env::remove_var("HOME"),
            }
        }
    }

    // ── current_state ───────────────────────────────────────────

    #[test]
    fn current_state_reports_not_installed_when_absent_everywhere() {
        let _lock = crate::runtime::test_env::lock_runtime_env();
        let tmp = tempfile::tempdir().expect("tempdir");
        let prior = std::env::var_os("HOME");
        // SAFETY: serialized by `lock_runtime_env`; restored below.
        unsafe { std::env::set_var("HOME", tmp.path()) };
        let mut runner = RecordingRunner::default();
        let state = current_state_with_runner(&mut runner);
        // SAFETY: serialized by `lock_runtime_env`.
        unsafe {
            match prior {
                Some(value) => std::env::set_var("HOME", value),
                None => std::env::remove_var("HOME"),
            }
        }
        assert_eq!(
            state.expect("current_state must succeed"),
            super::super::ServiceState::NotInstalled
        );
    }

    #[test]
    fn current_state_reports_stopped_when_loaded_but_not_running() {
        let _lock = crate::runtime::test_env::lock_runtime_env();
        let tmp = tempfile::tempdir().expect("tempdir");
        let prior = std::env::var_os("HOME");
        // SAFETY: serialized by `lock_runtime_env`; restored below.
        unsafe { std::env::set_var("HOME", tmp.path()) };
        let file = plist_path().expect("plist path");
        std::fs::create_dir_all(file.parent().expect("plist parent")).expect("create parent");
        std::fs::write(&file, "plist").expect("write plist");
        let mut runner = RecordingRunner {
            launchctl_list: format!("-\t0\t{SERVICE_LABEL}\n"),
            ..RecordingRunner::default()
        };
        let state = current_state_with_runner(&mut runner);
        // SAFETY: serialized by `lock_runtime_env`.
        unsafe {
            match prior {
                Some(value) => std::env::set_var("HOME", value),
                None => std::env::remove_var("HOME"),
            }
        }
        assert_eq!(
            state.expect("current_state must succeed"),
            super::super::ServiceState::Stopped
        );
    }

    #[test]
    fn current_state_reports_running_when_pid_present() {
        let _lock = crate::runtime::test_env::lock_runtime_env();
        let tmp = tempfile::tempdir().expect("tempdir");
        let prior = std::env::var_os("HOME");
        // SAFETY: serialized by `lock_runtime_env`; restored below.
        unsafe { std::env::set_var("HOME", tmp.path()) };
        let file = plist_path().expect("plist path");
        std::fs::create_dir_all(file.parent().expect("plist parent")).expect("create parent");
        std::fs::write(&file, "plist").expect("write plist");
        let mut runner = RecordingRunner {
            launchctl_list: format!("123\t0\t{SERVICE_LABEL}\n"),
            ..RecordingRunner::default()
        };
        let state = current_state_with_runner(&mut runner);
        // SAFETY: serialized by `lock_runtime_env`.
        unsafe {
            match prior {
                Some(value) => std::env::set_var("HOME", value),
                None => std::env::remove_var("HOME"),
            }
        }
        assert_eq!(
            state.expect("current_state must succeed"),
            super::super::ServiceState::Running
        );
    }
}
