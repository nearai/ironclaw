//! Linux systemd user-unit generators, path resolution for
//! `ironclaw-reborn service`. Verb bodies (install/start/stop/status/
//! uninstall) are appended once the shared shell-out helpers in
//! `super` exist.

use std::path::PathBuf;

use anyhow::Result;

use crate::serve_invocation::ServeInvocation;

use super::{SYSTEMD_UNIT, home_dir};

// ── Quoting ─────────────────────────────────────────────────────

fn unit_quote(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

// ── Unit generation ─────────────────────────────────────────────

fn unit_content(invocation: &ServeInvocation) -> String {
    let environment_lines: String = invocation
        .env
        .iter()
        .map(|(key, value)| format!("Environment={}\n", unit_quote(&format!("{key}={value}"))))
        .collect();

    let exec_start_args: String = std::iter::once(invocation.exe.display().to_string())
        .chain(invocation.args.iter().cloned())
        .map(|value| unit_quote(&value))
        .collect::<Vec<_>>()
        .join(" ");

    format!(
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
    )
}

// ── Path helpers ────────────────────────────────────────────────

fn unit_path() -> Result<PathBuf> {
    Ok(home_dir()?
        .join(".config")
        .join("systemd")
        .join("user")
        .join(SYSTEMD_UNIT))
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
    fn unit_quote_escapes_backslash_and_double_quote() {
        assert_eq!(unit_quote(r#"a"b\c"#), r#""a\"b\\c""#);
    }

    #[test]
    fn unit_content_includes_service_type() {
        let unit = unit_content(&sample_invocation());
        assert!(unit.contains("Type=simple"));
    }

    #[test]
    fn unit_content_includes_exec_start_tokens() {
        let unit = unit_content(&sample_invocation());
        assert!(unit.contains(r#""/usr/local/bin/ironclaw-reborn""#));
        assert!(unit.contains(r#""serve""#));
    }

    #[test]
    fn unit_content_includes_environment_line() {
        let unit = unit_content(&sample_invocation());
        assert!(unit.contains(r#"Environment="IRONCLAW_REBORN_HOME=/home/op/.ironclaw/reborn""#));
    }

    #[test]
    fn unit_content_includes_restart_policy_and_install_target() {
        let unit = unit_content(&sample_invocation());
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
        let unit = unit_content(&invocation);
        assert!(unit.contains(r#"IRONCLAW_REBORN_PROFILE=has\"quote"#));
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
}
