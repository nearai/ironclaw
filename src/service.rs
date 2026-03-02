//! OS service management for running IronClaw as a daemon.
//!
//! Generates and manages platform-native service definitions:
//! - **macOS**: launchd plist at `~/Library/LaunchAgents/com.ironclaw.daemon.plist`
//! - **Linux**: systemd user unit at `~/.config/systemd/user/ironclaw.service`
//!
//! The installed service runs `ironclaw run` (the default agent mode) and is
//! configured to restart automatically on failure.

use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::bootstrap::ironclaw_base_dir;

const SERVICE_LABEL: &str = "com.ironclaw.daemon";
const SYSTEMD_UNIT: &str = "ironclaw.service";

// ── Public dispatch ─────────────────────────────────────────────

/// Route a service subcommand to the appropriate handler.
pub fn handle_command(command: &ServiceAction) -> Result<()> {
    match command {
        ServiceAction::Install => install(),
        ServiceAction::Start => start(),
        ServiceAction::Stop => stop(),
        ServiceAction::Status => status(),
        ServiceAction::Uninstall => uninstall(),
    }
}

/// The five service lifecycle actions.
#[derive(Debug, Clone)]
pub enum ServiceAction {
    Install,
    Start,
    Stop,
    Status,
    Uninstall,
}

// ── Install ─────────────────────────────────────────────────────

fn install() -> Result<()> {
    if cfg!(target_os = "macos") {
        install_macos()
    } else if cfg!(target_os = "linux") {
        install_linux()
    } else {
        bail!("Service management is only supported on macOS and Linux");
    }
}

fn install_macos() -> Result<()> {
    let file = macos_plist_path()?;
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let exe = std::env::current_exe().context("failed to resolve current executable")?;
    let logs_dir = ironclaw_logs_dir();
    std::fs::create_dir_all(&logs_dir)?;

    let stdout = logs_dir.join("daemon.stdout.log");
    let stderr = logs_dir.join("daemon.stderr.log");

    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{label}</string>
  <key>ProgramArguments</key>
  <array>
    <string>{exe}</string>
    <string>run</string>
  </array>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <true/>
  <key>StandardOutPath</key>
  <string>{stdout}</string>
  <key>StandardErrorPath</key>
  <string>{stderr}</string>
</dict>
</plist>
"#,
        label = SERVICE_LABEL,
        exe = xml_escape(&exe.display().to_string()),
        stdout = xml_escape(&stdout.display().to_string()),
        stderr = xml_escape(&stderr.display().to_string()),
    );

    std::fs::write(&file, plist)?;
    println!("Installed launchd service: {}", file.display());
    println!("  Start with: ironclaw service start");
    Ok(())
}

fn install_linux() -> Result<()> {
    let file = linux_unit_path()?;
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let exe = std::env::current_exe().context("failed to resolve current executable")?;
    let unit = format!(
        "[Unit]\n\
         Description=IronClaw daemon\n\
         After=network.target\n\
         \n\
         [Service]\n\
         Type=simple\n\
         ExecStart=\"{exe}\" run\n\
         Restart=always\n\
         RestartSec=3\n\
         \n\
         [Install]\n\
         WantedBy=default.target\n",
        exe = exe.display(),
    );

    std::fs::write(&file, unit)?;
    run_checked(Command::new("systemctl").args(["--user", "daemon-reload"])).ok();
    run_checked(Command::new("systemctl").args(["--user", "enable", SYSTEMD_UNIT])).ok();
    println!("Installed systemd user service: {}", file.display());
    println!("  Start with: ironclaw service start");
    Ok(())
}

// ── Start ───────────────────────────────────────────────────────

fn start() -> Result<()> {
    if cfg!(target_os = "macos") {
        let plist = macos_plist_path()?;
        if !plist.exists() {
            bail!("Service not installed. Run `ironclaw service install` first.");
        }
        run_checked(Command::new("launchctl").arg("load").arg("-w").arg(&plist))?;
        run_checked(Command::new("launchctl").arg("start").arg(SERVICE_LABEL))?;
        println!("Service started");
        Ok(())
    } else if cfg!(target_os = "linux") {
        run_checked(Command::new("systemctl").args(["--user", "daemon-reload"]))?;
        run_checked(Command::new("systemctl").args(["--user", "start", SYSTEMD_UNIT]))?;
        println!("Service started");
        Ok(())
    } else {
        bail!("Service management is only supported on macOS and Linux");
    }
}

// ── Stop ────────────────────────────────────────────────────────

fn stop() -> Result<()> {
    if cfg!(target_os = "macos") {
        let plist = macos_plist_path()?;
        run_checked(Command::new("launchctl").arg("stop").arg(SERVICE_LABEL)).ok();
        run_checked(
            Command::new("launchctl")
                .arg("unload")
                .arg("-w")
                .arg(&plist),
        )
        .ok();
        println!("Service stopped");
        Ok(())
    } else if cfg!(target_os = "linux") {
        run_checked(Command::new("systemctl").args(["--user", "stop", SYSTEMD_UNIT])).ok();
        println!("Service stopped");
        Ok(())
    } else {
        bail!("Service management is only supported on macOS and Linux");
    }
}

// ── Status ──────────────────────────────────────────────────────

fn status() -> Result<()> {
    if cfg!(target_os = "macos") {
        let out = run_capture(Command::new("launchctl").arg("list"))?;
        let running = out.lines().any(|line| line.contains(SERVICE_LABEL));
        println!(
            "Service: {}",
            if running {
                "running/loaded"
            } else {
                "not loaded"
            }
        );
        println!("Unit: {}", macos_plist_path()?.display());
        Ok(())
    } else if cfg!(target_os = "linux") {
        let state =
            run_capture(Command::new("systemctl").args(["--user", "is-active", SYSTEMD_UNIT]))
                .unwrap_or_else(|_| "unknown".into());
        println!("Service state: {}", state.trim());
        println!("Unit: {}", linux_unit_path()?.display());
        Ok(())
    } else {
        bail!("Service management is only supported on macOS and Linux");
    }
}

// ── Uninstall ───────────────────────────────────────────────────

fn uninstall() -> Result<()> {
    // Stop first (ignore errors, service might not be running)
    stop().ok();

    if cfg!(target_os = "macos") {
        let file = macos_plist_path()?;
        if file.exists() {
            std::fs::remove_file(&file)
                .with_context(|| format!("failed to remove {}", file.display()))?;
        }
        println!("Service uninstalled ({})", file.display());
        Ok(())
    } else if cfg!(target_os = "linux") {
        let file = linux_unit_path()?;
        if file.exists() {
            std::fs::remove_file(&file)
                .with_context(|| format!("failed to remove {}", file.display()))?;
        }
        run_checked(Command::new("systemctl").args(["--user", "daemon-reload"])).ok();
        println!("Service uninstalled ({})", file.display());
        Ok(())
    } else {
        bail!("Service management is only supported on macOS and Linux");
    }
}

// ── Path helpers ────────────────────────────────────────────────

fn macos_plist_path() -> Result<PathBuf> {
    let home = dirs::home_dir().context("could not find home directory")?;
    Ok(home
        .join("Library")
        .join("LaunchAgents")
        .join(format!("{SERVICE_LABEL}.plist")))
}

fn linux_unit_path() -> Result<PathBuf> {
    let home = dirs::home_dir().context("could not find home directory")?;
    Ok(home
        .join(".config")
        .join("systemd")
        .join("user")
        .join(SYSTEMD_UNIT))
}

fn ironclaw_logs_dir() -> PathBuf {
    ironclaw_base_dir().join("logs")
}

// ── Shell helpers ───────────────────────────────────────────────

fn run_checked(command: &mut Command) -> Result<()> {
    let output = command.output().context("failed to spawn command")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("command failed: {}", stderr.trim());
    }
    Ok(())
}

fn run_capture(command: &mut Command) -> Result<String> {
    let output = command.output().context("failed to spawn command")?;
    let mut text = String::from_utf8_lossy(&output.stdout).to_string();
    if text.trim().is_empty() {
        text = String::from_utf8_lossy(&output.stderr).to_string();
    }
    Ok(text)
}

fn xml_escape(raw: &str) -> String {
    raw.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

// ── Tests ───────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use crate::service::*;

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
    fn run_capture_reads_stdout() {
        let out = run_capture(Command::new("sh").args(["-c", "echo hello"]))
            .expect("stdout capture should succeed");
        assert_eq!(out.trim(), "hello");
    }

    #[test]
    fn run_capture_falls_back_to_stderr() {
        let out = run_capture(Command::new("sh").args(["-c", "echo warn 1>&2"]))
            .expect("stderr capture should succeed");
        assert_eq!(out.trim(), "warn");
    }

    #[test]
    fn run_checked_errors_on_non_zero_exit() {
        let err = run_checked(Command::new("sh").args(["-c", "exit 17"]))
            .expect_err("non-zero exit should error");
        assert!(err.to_string().contains("command failed"));
    }

    #[test]
    fn run_checked_succeeds_on_zero_exit() {
        assert!(run_checked(Command::new("sh").args(["-c", "exit 0"])).is_ok());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_plist_path_has_expected_suffix() {
        let path = macos_plist_path().unwrap();
        let s = path.to_string_lossy();
        assert!(
            s.ends_with("Library/LaunchAgents/com.ironclaw.daemon.plist"),
            "unexpected path: {s}"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_unit_path_has_expected_suffix() {
        let path = linux_unit_path().unwrap();
        let s = path.to_string_lossy();
        assert!(
            s.ends_with(".config/systemd/user/ironclaw.service"),
            "unexpected path: {s}"
        );
    }

    #[test]
    fn logs_dir_under_ironclaw() {
        let path = ironclaw_logs_dir();
        let s = path.to_string_lossy();
        assert!(s.ends_with(".ironclaw/logs"), "unexpected path: {s}");
    }
}
