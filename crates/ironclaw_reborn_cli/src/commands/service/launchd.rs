//! macOS launchd generators, path resolution, and status matching for
//! `ironclaw-reborn service`. Verb bodies (install/start/stop/status/
//! uninstall) are appended once the shared shell-out helpers in
//! `super` exist.

use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::serve_invocation::ServeInvocation;

use super::{SERVICE_LABEL, home_dir};

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
    launchctl_list_output
        .lines()
        .any(|line| line.contains(SERVICE_LABEL))
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let plist = plist_content(&invocation, Path::new("/tmp/o.log"), Path::new("/tmp/e.log"));
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
        let output = "com.apple.something\t0\t0\ncom.ironclaw.reborn.daemon\t-\t0\n";
        assert!(service_running(output));
    }

    #[test]
    fn service_running_false_when_label_absent() {
        let output = "com.apple.something\t0\t0\n";
        assert!(!service_running(output));
    }
}
