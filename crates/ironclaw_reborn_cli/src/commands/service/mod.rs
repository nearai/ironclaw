//! `ironclaw-reborn service` — install/manage the standalone Reborn
//! binary as an OS-native service.
//!
//! - **macOS**: launchd user agent at
//!   `~/Library/LaunchAgents/com.ironclaw.reborn.daemon.plist`.
//! - **Linux**: systemd user unit at
//!   `~/.config/systemd/user/ironclaw-reborn.service`.
//!
//! The installed service runs `<current_exe> serve`, restarting
//! automatically on failure. Mirrors v1's `src/service.rs` shape with
//! Reborn's label, paths, and env-passthrough contract; no v1 code is
//! shared or reused.
//!
//! Platform dispatch happens once, in [`ServicePlatform::detect`] (added
//! alongside the clap surface once the `launchd`/`systemd` verb bodies
//! exist), called a single time from `ServiceCommand::execute`. The five
//! verbs are methods on `ServicePlatform` that each `match self` to
//! delegate into the OS-specific implementation, rather than every verb
//! re-checking `cfg!(target_os)`.

use std::path::PathBuf;

use anyhow::{Context, Result, bail};

mod launchd;
mod systemd;

const SERVICE_LABEL: &str = "com.ironclaw.reborn.daemon";
const SYSTEMD_UNIT: &str = "ironclaw-reborn.service";

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
