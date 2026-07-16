//! macOS launchd generators, path resolution, status matching, and verb
//! bodies for `ironclaw-reborn service`.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::context::RebornCliContext;
use crate::serve_invocation::ServeInvocation;

use super::{OsServiceCommandRunner, SERVICE_LABEL, ServiceCommandRunner, home_dir};

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

fn plist_content(invocation: &ServeInvocation, stdout_log: &Path, stderr_log: &Path) -> String {
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

fn service_running(launchctl_list_output: &str) -> bool {
    launchctl_list_output.lines().any(|line| {
        let mut fields = line.split_whitespace();
        fields.next().is_some()
            && fields.next().is_some()
            && fields.next() == Some(SERVICE_LABEL)
            && fields.next().is_none()
    })
}

// ── Verb bodies ─────────────────────────────────────────────────

pub(super) fn install(context: &RebornCliContext, invocation: &ServeInvocation) -> Result<()> {
    let file = plist_path()?;
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let logs_dir = context.boot_config().home().path().join("logs");
    std::fs::create_dir_all(&logs_dir).with_context(|| format!("create {}", logs_dir.display()))?;
    let stdout_log = logs_dir.join("serve.stdout.log");
    let stderr_log = logs_dir.join("serve.stderr.log");
    let plist = plist_content(invocation, &stdout_log, &stderr_log);
    std::fs::write(&file, plist).with_context(|| format!("write {}", file.display()))?;
    println!("Installed launchd service: {}", file.display());
    println!("  Start with: ironclaw-reborn service start");
    Ok(())
}

pub(super) fn start() -> Result<()> {
    start_with_runner(&mut OsServiceCommandRunner)
}

fn start_with_runner(runner: &mut dyn ServiceCommandRunner) -> Result<()> {
    let plist = plist_path()?;
    if !plist.exists() {
        bail!("Service not installed. Run `ironclaw-reborn service install` first.");
    }
    runner.run_checked(
        "launchctl load",
        Command::new("launchctl").arg("load").arg("-w").arg(&plist),
    )?;
    runner.run_checked(
        "launchctl start",
        Command::new("launchctl").arg("start").arg(SERVICE_LABEL),
    )?;
    println!("Service started");
    Ok(())
}

pub(super) fn stop() -> Result<()> {
    stop_with_runner(&mut OsServiceCommandRunner)
}

fn stop_with_runner(runner: &mut dyn ServiceCommandRunner) -> Result<()> {
    let plist = plist_path()?;
    if !plist.exists() {
        println!("Service stopped");
        return Ok(());
    }
    let list =
        runner.run_capture_checked("launchctl list", Command::new("launchctl").arg("list"))?;
    if !service_running(&list) {
        println!("Service stopped");
        return Ok(());
    }
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
    println!("Service stopped");
    Ok(())
}

pub(super) fn status() -> Result<()> {
    status_with_runner(&mut OsServiceCommandRunner)
}

fn status_with_runner(runner: &mut dyn ServiceCommandRunner) -> Result<()> {
    let out =
        runner.run_capture_checked("launchctl list", Command::new("launchctl").arg("list"))?;
    println!(
        "Service: {}",
        if service_running(&out) {
            "running/loaded"
        } else {
            "not loaded"
        }
    );
    println!("Unit: {}", plist_path()?.display());
    Ok(())
}

pub(super) fn uninstall() -> Result<()> {
    uninstall_with_runner(&mut OsServiceCommandRunner)
}

pub(super) fn uninstall_with_runner(runner: &mut dyn ServiceCommandRunner) -> Result<()> {
    let file = plist_path()?;
    let list =
        runner.run_capture_checked("launchctl list", Command::new("launchctl").arg("list"))?;
    if service_running(&list) {
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
            Path::new("/home/op/.ironclaw/reborn/logs/serve.stdout.log"),
            Path::new("/home/op/.ironclaw/reborn/logs/serve.stderr.log"),
        );
        assert!(plist.contains("<string>/home/op/.ironclaw/reborn/logs/serve.stdout.log</string>"));
        assert!(plist.contains("<string>/home/op/.ironclaw/reborn/logs/serve.stderr.log</string>"));
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
            PathBuf::from("/home/op/Library/LaunchAgents/com.ironclaw.reborn.daemon.plist")
        );
    }

    #[test]
    fn service_running_matches_only_the_label_line() {
        let output = "-\t0\tcom.apple.something\n123\t0\tcom.ironclaw.reborn.daemon\n";
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
                "gui/501/com.ironclaw.reborn.daemon".to_string()
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
            fail_args: Some(vec!["bootout", "gui/501/com.ironclaw.reborn.daemon"]),
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
                vec!["bootout", "gui/501/com.ironclaw.reborn.daemon"]
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
        let prior = std::env::var_os("HOME");
        // SAFETY: serialized by `lock_runtime_env`; restored below.
        unsafe { std::env::set_var("HOME", "/home/op") };
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
    fn status_classifies_loaded_and_not_loaded_from_exact_list_command() {
        let _lock = crate::runtime::test_env::lock_runtime_env();
        let prior = std::env::var_os("HOME");
        // SAFETY: serialized by `lock_runtime_env`; restored below.
        unsafe { std::env::set_var("HOME", "/home/op") };
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
}
