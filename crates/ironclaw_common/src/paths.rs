//! Shared filesystem path helpers.
//!
//! `ironclaw_base_dir()` resolves the IronClaw base directory used for env
//! files, session tokens, the libsql database, and other per-instance state.
//! Override with the `IRONCLAW_BASE_DIR` environment variable; defaults to
//! `~/.ironclaw`.

use std::path::PathBuf;
use std::sync::LazyLock;

const IRONCLAW_BASE_DIR_ENV: &str = "IRONCLAW_BASE_DIR";

static IRONCLAW_BASE_DIR: LazyLock<PathBuf> = LazyLock::new(compute_ironclaw_base_dir);

/// Compute the IronClaw base directory from the environment.
///
/// Bypasses the `LazyLock` cache. Use this in tests that mutate
/// `IRONCLAW_BASE_DIR`; production callers should use [`ironclaw_base_dir`].
pub fn compute_ironclaw_base_dir() -> PathBuf {
    std::env::var(IRONCLAW_BASE_DIR_ENV)
        .map(PathBuf::from)
        .map(|path| {
            if path.as_os_str().is_empty() {
                default_base_dir()
            } else if !path.is_absolute() {
                eprintln!(
                    "Warning: IRONCLAW_BASE_DIR is a relative path '{}', resolved against current directory",
                    path.display()
                );
                path
            } else {
                path
            }
        })
        .unwrap_or_else(|_| default_base_dir())
}

fn default_base_dir() -> PathBuf {
    if let Some(home) = dirs::home_dir() {
        home.join(".ironclaw")
    } else {
        eprintln!("Warning: Could not determine home directory, using current directory");
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("/tmp"))
            .join(".ironclaw")
    }
}

/// Get the IronClaw base directory.
///
/// Override with `IRONCLAW_BASE_DIR`. Defaults to `~/.ironclaw` (or
/// `./.ironclaw` if the home directory cannot be determined).
///
/// Thread-safe: the value is computed once and cached in a `LazyLock`.
pub fn ironclaw_base_dir() -> PathBuf {
    IRONCLAW_BASE_DIR.clone()
}
