//! Container-supervised flavor of `ironclaw service`.
//!
//! Hosted deployments run the Reborn binary as the container's foreground
//! process: the image entrypoint (`docker/reborn/entrypoint.sh`) `exec`s
//! `ironclaw serve`, so PID 1 is the container init (e.g. `docker-init`) and
//! the container runtime's restart policy — not systemd or launchd — is the
//! supervisor. There is no service manager to install into: `systemctl`
//! does not exist in the image, so before this module existed every
//! `ironclaw service` verb on such a host died with a confusing
//! "failed to spawn command for systemctl …" or "Service not installed"
//! error, while our own setup docs tell users to finish with
//! `ironclaw service restart`.
//!
//! [`super::ServicePlatform::detect`] resolves to `Container` on Linux when
//! systemd is not the running init (no `/run/systemd/system`) AND the
//! deployment has explicitly declared a managed restart policy via
//! `IRONCLAW_REBORN_CONTAINER_SUPERVISED` — systemd-absence alone is not
//! enough, and neither is merely being *a* container (WSL2, OpenRC distros,
//! a plain VM running `ironclaw onboard` directly, or a real Docker
//! container started with no restart policy at all, e.g. the documented
//! `docker run --rm ...` local-run command, all lack a guarantee that
//! anything relaunches `serve` after `restart` kills it; see
//! [`super::ServicePlatform::linux_platform`]'s doc). In `Container` mode:
//!
//! - `restart` terminates the running `serve` process. The container's main
//!   process exiting makes the container exit, and the restart policy
//!   relaunches it through the entrypoint — that *is* the restart primitive
//!   for this deployment shape.
//! - `status` reports whether a `serve` process is running.
//! - `install`/`uninstall`/`start`/`stop` fail with one actionable message:
//!   there is no service manager to act on, and a `stop` would be undone by
//!   the restart policy immediately.

use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};

use super::{ServiceCommandRunner, ServiceState};

/// The serve process is found by argv shape, not by pid-1 parentage: the
/// entrypoint `exec`s `ironclaw serve --host … --port …`, so argv0's
/// basename is `ironclaw` and argv1 is exactly `serve`.
const SERVE_SUBCOMMAND: &str = "serve";

/// Scan `<proc_root>/<pid>/cmdline` for running `ironclaw serve` processes,
/// excluding this process itself. `proc_root` is `/proc` in production and a
/// fake tree under a tempdir in tests (which lets the scan logic run on
/// macOS test hosts too, where the Container variant is otherwise
/// unreachable).
fn find_serve_pids(proc_root: &Path) -> Result<Vec<u32>> {
    let entries = std::fs::read_dir(proc_root)
        .with_context(|| format!("read process table at {}", proc_root.display()))?;
    let own_pid = std::process::id();
    let mut pids = Vec::new();
    for entry in entries {
        // silent-ok: a process may exit between readdir and stat; a vanished
        // entry is normal churn, not a scan failure.
        let Ok(entry) = entry else { continue };
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())
        else {
            continue;
        };
        if pid == own_pid {
            continue;
        }
        // silent-ok: same readdir/read race — the process can exit before we
        // read its cmdline; skip rather than fail the whole scan.
        let Ok(cmdline) = std::fs::read(entry.path().join("cmdline")) else {
            continue;
        };
        let mut argv = cmdline.split(|byte| *byte == 0);
        let argv0_is_ironclaw = argv
            .next()
            .map(String::from_utf8_lossy)
            .map(|argv0| {
                Path::new(argv0.as_ref())
                    .file_name()
                    .is_some_and(|name| name == "ironclaw")
            })
            .unwrap_or(false);
        let argv1_is_serve = argv
            .next()
            .map(String::from_utf8_lossy)
            .is_some_and(|argv1| argv1 == SERVE_SUBCOMMAND);
        if argv0_is_ironclaw && argv1_is_serve {
            pids.push(pid);
        }
    }
    pids.sort_unstable();
    Ok(pids)
}

/// Runner-injectable service-state query behind
/// [`super::ServicePlatform::current_state_with_runner`] — see that method's
/// doc. "Installed" doesn't apply to a container-supervised deployment (the
/// container itself is the installation), so state is purely running/stopped
/// based on whether a `serve` process is found.
pub(super) fn current_state(proc_root: &Path) -> Result<ServiceState> {
    Ok(if find_serve_pids(proc_root)?.is_empty() {
        ServiceState::Stopped
    } else {
        ServiceState::Running
    })
}

/// Runner-injectable `status` behind [`super::ServicePlatform::status_with_runner`]
/// — see that method's doc. Delegates message-building to [`status_message`]
/// (a pure fn) so the text is testable without capturing stdout.
pub(super) fn status(proc_root: &Path) -> Result<()> {
    let pids = find_serve_pids(proc_root)?;
    println!("{}", status_message(&pids));
    Ok(())
}

/// Builds the `service status` line for the given serve pids. Reuses
/// [`super::status_label`] (not a hand-rolled "running"/"stopped" string) so
/// the vocabulary can't drift from launchd/systemd's.
fn status_message(pids: &[u32]) -> String {
    let label = super::status_label(true, !pids.is_empty());
    if pids.is_empty() {
        format!("Service: {label} (container-supervised; no `ironclaw serve` process found)")
    } else {
        let pid_list = pids
            .iter()
            .map(|pid| pid.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        format!("Service: {label} (container-supervised, pid {pid_list})")
    }
}

/// Runner-injectable `restart` behind [`super::ServicePlatform::restart_with_runner`]
/// — see that method's doc. Unlike launchd/systemd's stop-then-start, this
/// signals the running `serve` process and relies on the container runtime's
/// restart policy to relaunch it — see the module doc for why that is the
/// correct restart primitive for this deployment shape.
pub(super) fn restart_with_runner(
    runner: &mut dyn ServiceCommandRunner,
    proc_root: &Path,
) -> Result<()> {
    let pids = find_serve_pids(proc_root)?;
    if pids.is_empty() {
        bail!(
            "no running `ironclaw serve` process found. This instance is supervised by its \
             container runtime (no systemd available); if serve is down, restart the container \
             from your hosting platform."
        );
    }
    let display_pids = pids
        .iter()
        .map(|pid| pid.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    // Print before signaling: terminating serve makes the container's main
    // process exit, which ends this session too — this line may be the last
    // output the operator sees before reconnecting.
    println!(
        "Terminating `ironclaw serve` (pid {display_pids}). The container will exit and be \
         relaunched by its restart policy; active sessions (including SSH) will disconnect. \
         Reconnect once the container is back up."
    );
    // Through `sh -c`, not a direct `Command::new("kill")`: unlike
    // `systemctl`/`launchctl`, `kill` is a shell BUILTIN on the shipped
    // image (debian:bookworm-slim has no `procps`, the only package
    // providing a standalone `/bin/kill`) — spawning "kill" directly fails
    // with ENOENT. `/bin/sh` is guaranteed present; it's what boots this
    // container (the image entrypoint is a `#!/bin/sh` script). Every
    // argument is a bare numeric pid, so no shell-injection surface.
    let shell_command = format!(
        "kill -TERM {}",
        pids.iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(" ")
    );
    runner.run_checked(
        "terminate serve process",
        Command::new("sh").arg("-c").arg(shell_command),
    )
}

/// Shared bail for the verbs that require a service manager. One message so
/// the guidance can't drift between verbs.
pub(super) fn unsupported_in_container(verb: &str) -> Result<()> {
    bail!(
        "`ironclaw service {verb}` is not available here: this instance runs as the container's \
         main process, supervised by the container runtime's restart policy (systemd is not \
         running). Use `ironclaw service restart` to restart serve, or manage the instance from \
         your hosting platform."
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build `<root>/<pid>/cmdline` with NUL-joined argv, mirroring the real
    /// procfs encoding.
    fn write_proc_entry(root: &Path, pid: u32, argv: &[&str]) {
        let dir = root.join(pid.to_string());
        std::fs::create_dir_all(&dir).expect("create fake proc pid dir");
        std::fs::write(dir.join("cmdline"), argv.join("\0").as_bytes() as &[u8])
            .expect("write fake cmdline");
    }

    fn fake_proc(entries: &[(u32, &[&str])]) -> tempfile::TempDir {
        let tmp = tempfile::tempdir().expect("tempdir");
        for (pid, argv) in entries {
            write_proc_entry(tmp.path(), *pid, argv);
        }
        tmp
    }

    #[derive(Default)]
    struct RecordingRunner {
        commands: Vec<(String, String)>,
        /// When set, `run_checked`/`run_capture_checked` still record the
        /// call, then return this error instead of `Ok` — used to prove
        /// `restart_with_runner` propagates a signal-send failure rather
        /// than reporting success.
        fail_with: Option<String>,
    }

    impl ServiceCommandRunner for RecordingRunner {
        fn run_checked(&mut self, label: &str, command: &mut Command) -> Result<()> {
            self.commands
                .push((label.to_string(), format!("{command:?}")));
            match &self.fail_with {
                Some(message) => bail!("{message}"),
                None => Ok(()),
            }
        }

        fn run_capture_checked(&mut self, label: &str, command: &mut Command) -> Result<String> {
            self.commands
                .push((label.to_string(), format!("{command:?}")));
            match &self.fail_with {
                Some(message) => bail!("{message}"),
                None => Ok(String::new()),
            }
        }
    }

    #[test]
    fn find_serve_pids_matches_only_ironclaw_serve_argv() {
        let proc = fake_proc(&[
            (
                1,
                &["/sbin/docker-init", "--", "ironclaw-reborn-entrypoint"],
            ),
            (
                71,
                &[
                    "/usr/local/bin/ironclaw",
                    "serve",
                    "--host",
                    "0.0.0.0",
                    "--port",
                    "3000",
                ],
            ),
            (87, &["sshd: /usr/sbin/sshd"]),
            // Same binary, different subcommand — must not match.
            (90, &["/usr/local/bin/ironclaw", "status"]),
            // `serve` argv1 under a different binary — must not match.
            (91, &["/usr/local/bin/other", "serve"]),
        ]);
        assert_eq!(
            find_serve_pids(proc.path()).expect("scan must succeed"),
            vec![71]
        );
    }

    #[test]
    fn find_serve_pids_skips_non_numeric_and_unreadable_entries() {
        let proc = fake_proc(&[(42, &["/usr/local/bin/ironclaw", "serve"])]);
        std::fs::create_dir_all(proc.path().join("self")).expect("non-numeric proc entry");
        // Numeric dir with no cmdline file: the readdir/read race shape.
        std::fs::create_dir_all(proc.path().join("99")).expect("raced proc entry");
        assert_eq!(
            find_serve_pids(proc.path()).expect("scan must succeed"),
            vec![42]
        );
    }

    #[test]
    fn find_serve_pids_skips_empty_and_argv0_only_cmdline() {
        // Real /proc races: a process caught mid-exec (or already exited)
        // can leave a 0-byte cmdline, or a cmdline with only argv0 and no
        // argv1 yet — neither is a serve match, and neither should error
        // the scan.
        let proc = fake_proc(&[(42, &["/usr/local/bin/ironclaw", "serve"])]);
        std::fs::create_dir_all(proc.path().join("50")).expect("create pid dir");
        std::fs::write(proc.path().join("50/cmdline"), b"").expect("empty cmdline");
        std::fs::create_dir_all(proc.path().join("51")).expect("create pid dir");
        std::fs::write(proc.path().join("51/cmdline"), b"/usr/local/bin/ironclaw\0")
            .expect("argv0-only cmdline");
        assert_eq!(
            find_serve_pids(proc.path()).expect("scan must succeed"),
            vec![42]
        );
    }

    #[test]
    fn find_serve_pids_returns_ascending_order_regardless_of_readdir_order() {
        // pids.sort_unstable() must actually run: entries are created with
        // pid 200 before 3 and 45, so a directory-order scan (readdir isn't
        // numerically ordered) would return them out of order without the
        // sort.
        let proc = fake_proc(&[
            (200, &["/usr/local/bin/ironclaw", "serve"]),
            (3, &["/usr/local/bin/ironclaw", "serve"]),
            (45, &["/usr/local/bin/ironclaw", "serve"]),
        ]);
        assert_eq!(
            find_serve_pids(proc.path()).expect("scan must succeed"),
            vec![3, 45, 200]
        );
    }

    #[test]
    fn find_serve_pids_propagates_missing_proc_root_error() {
        let missing = tempfile::tempdir()
            .expect("tempdir")
            .path()
            .join("does-not-exist");
        let err = find_serve_pids(&missing).expect_err("missing proc root must error");
        assert!(
            err.to_string()
                .contains(&format!("read process table at {}", missing.display())),
            "unexpected error message: {err}"
        );
    }

    #[test]
    fn status_message_reuses_the_shared_running_stopped_vocabulary() {
        assert_eq!(
            status_message(&[]),
            "Service: stopped (container-supervised; no `ironclaw serve` process found)"
        );
        assert_eq!(
            status_message(&[71, 72]),
            "Service: running (container-supervised, pid 71, 72)"
        );
    }

    #[test]
    fn current_state_maps_presence_to_running_and_absence_to_stopped() {
        let running = fake_proc(&[(7, &["/usr/local/bin/ironclaw", "serve"])]);
        assert_eq!(
            current_state(running.path()).expect("state"),
            ServiceState::Running
        );
        let stopped = fake_proc(&[(1, &["/sbin/docker-init"])]);
        assert_eq!(
            current_state(stopped.path()).expect("state"),
            ServiceState::Stopped
        );
    }

    #[test]
    fn restart_signals_every_serve_pid_through_the_runner() {
        let proc = fake_proc(&[
            (71, &["/usr/local/bin/ironclaw", "serve"]),
            (72, &["/usr/local/bin/ironclaw", "serve", "--port", "3001"]),
        ]);
        let mut runner = RecordingRunner::default();
        restart_with_runner(&mut runner, proc.path()).expect("restart must succeed");
        assert_eq!(runner.commands.len(), 1);
        let (label, command) = &runner.commands[0];
        assert_eq!(label, "terminate serve process");
        // Through `sh -c`, not a direct `Command::new("kill")` — `kill` is a
        // shell builtin on the shipped image (no `procps`), not a spawnable
        // binary; see the comment on `restart_with_runner`.
        assert!(
            command.starts_with("\"sh\" \"-c\""),
            "must invoke kill through a shell, since kill has no standalone binary on the \
             shipped image: {command}"
        );
        assert!(command.contains("kill -TERM 71 72"), "{command}");
    }

    #[test]
    fn restart_propagates_terminate_runner_error() {
        // The success-path test above only proves the happy path. A failure
        // to spawn or execute the signal command is the operational failure
        // this command exists to surface — it must not be swallowed.
        let proc = fake_proc(&[(71, &["/usr/local/bin/ironclaw", "serve"])]);
        let mut runner = RecordingRunner {
            fail_with: Some("spawn failed: No such file or directory".to_string()),
            ..Default::default()
        };
        let error = restart_with_runner(&mut runner, proc.path())
            .expect_err("a runner failure must propagate, not be reported as success");
        assert!(
            error.to_string().contains("spawn failed"),
            "the underlying runner error must survive: {error:#}"
        );
        assert_eq!(
            runner.commands.len(),
            1,
            "the attempt must still have been recorded"
        );
    }

    #[test]
    fn restart_without_a_serve_process_names_container_supervision() {
        let proc = fake_proc(&[(1, &["/sbin/docker-init"])]);
        let mut runner = RecordingRunner::default();
        let error = restart_with_runner(&mut runner, proc.path())
            .expect_err("restart without serve must fail");
        assert!(
            error.to_string().contains("container runtime"),
            "error must explain the supervision model: {error:#}"
        );
        assert!(
            runner.commands.is_empty(),
            "no signal may be sent when no serve process exists"
        );
    }

    #[test]
    fn unsupported_verbs_share_one_actionable_message() {
        for verb in ["install", "uninstall", "start", "stop"] {
            let error = unsupported_in_container(verb).expect_err("verb must be rejected");
            let text = error.to_string();
            assert!(
                text.contains(&format!("`ironclaw service {verb}`")),
                "{text}"
            );
            assert!(text.contains("container"), "{text}");
            assert!(text.contains("ironclaw service restart"), "{text}");
        }
    }
}
