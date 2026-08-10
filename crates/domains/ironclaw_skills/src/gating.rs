//! Requirement gating for skill activation: does the environment have what a skill declares it
//! needs (`requires.bins` / `requires.env` / `requires.config`)?
//!
//! Restored after main deleted it as verified-dead (#6943). It WAS dead, and that is the bug:
//! `requires` was parsed and never consulted, so a skill declaring a missing binary activated
//! cleanly and failed later in the shell. `unmet_requirements_refusal` is the first real caller.

//! Requirements gating for skills.
//!
//! Checks that a skill's declared requirements (binaries, environment variables,
//! config files) are satisfied before the skill is loaded.

use crate::types::GatingRequirements;

/// Result of a gating check.
#[derive(Debug)]
pub struct GatingResult {
    /// Whether all requirements passed.
    pub passed: bool,
    /// Descriptions of failed requirements.
    pub failures: Vec<String>,
}

/// Async wrapper around [`check_requirements_sync`], offloading the blocking `which`/`where` calls
/// via `spawn_blocking`. Returns immediately when there is nothing to check, which is the common
/// case and avoids a subprocess per skill load.
pub async fn check_requirements(requirements: &GatingRequirements) -> GatingResult {
    if requirements.bins.is_empty() && requirements.env.is_empty() && requirements.config.is_empty()
    {
        return GatingResult {
            passed: true,
            failures: Vec::new(),
        };
    }

    let requirements = requirements.clone();
    tokio::task::spawn_blocking(move || check_requirements_sync(&requirements))
        .await
        .unwrap_or_else(|e| {
            let message = if e.is_panic() {
                format!("gating check panicked: {}", e)
            } else if e.is_cancelled() {
                format!("gating check task was cancelled: {}", e)
            } else {
                format!("gating check failed to join: {}", e)
            };
            tracing::error!("{}", message);
            GatingResult {
                passed: false,
                failures: vec![message],
            }
        })
}

/// Whether gating requirements are satisfied: `bins` findable via `which`, `env` set, `config`
/// paths present. Prefer the async [`check_requirements`] from an async context.
pub fn check_requirements_sync(requirements: &GatingRequirements) -> GatingResult {
    let mut failures = Vec::new();

    for bin in &requirements.bins {
        if !binary_exists(bin) {
            failures.push(format!("required binary not found: {}", bin));
        }
    }

    for var in &requirements.env {
        if std::env::var(var).is_err() {
            failures.push(format!("required env var not set: {}", var));
        }
    }

    for path in &requirements.config {
        if !std::path::Path::new(path).exists() {
            failures.push(format!("required config not found: {}", path));
        }
    }

    // Companion skill dependencies (`requirements.skills`) are intentionally
    // not checked here — the gating module has no access to the skill
    // registry. They are advisory metadata only.

    GatingResult {
        passed: failures.is_empty(),
        failures,
    }
}

/// Check if a binary exists on PATH using `std::process::Command`.
pub fn binary_exists(name: &str) -> bool {
    #[cfg(unix)]
    {
        std::process::Command::new("which")
            .arg(name)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    }
    #[cfg(windows)]
    {
        std::process::Command::new("where")
            .arg(name)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_requirements_pass() {
        let req = GatingRequirements::default();
        let result = check_requirements_sync(&req);
        assert!(result.passed);
        assert!(result.failures.is_empty());
    }

    #[test]
    fn test_missing_binary_fails() {
        let req = GatingRequirements {
            bins: vec!["__ironclaw_nonexistent_binary_xyz__".to_string()],
            ..Default::default()
        };
        let result = check_requirements_sync(&req);
        assert!(!result.passed);
        assert_eq!(result.failures.len(), 1);
        assert!(result.failures[0].contains("binary not found"));
    }

    #[test]
    fn test_missing_env_var_fails() {
        let req = GatingRequirements {
            env: vec!["__IRONCLAW_TEST_NONEXISTENT_VAR__".to_string()],
            ..Default::default()
        };
        let result = check_requirements_sync(&req);
        assert!(!result.passed);
        assert!(result.failures[0].contains("env var not set"));
    }

    #[test]
    fn test_present_env_var_passes() {
        let req = GatingRequirements {
            env: vec!["PATH".to_string()],
            ..Default::default()
        };
        let result = check_requirements_sync(&req);
        assert!(result.passed);
    }

    #[test]
    fn test_missing_config_fails() {
        let req = GatingRequirements {
            config: vec!["/nonexistent/path/ironclaw_test.conf".to_string()],
            ..Default::default()
        };
        let result = check_requirements_sync(&req);
        assert!(!result.passed);
        assert!(result.failures[0].contains("config not found"));
    }

    #[test]
    fn test_multiple_mixed_requirements() {
        let req = GatingRequirements {
            bins: vec!["__no_such_bin__".to_string()],
            env: vec!["__NO_SUCH_VAR__".to_string()],
            config: vec!["/no/such/file".to_string()],
            ..Default::default()
        };
        let result = check_requirements_sync(&req);
        assert!(!result.passed);
        assert_eq!(result.failures.len(), 3);
    }

    #[test]
    fn test_skill_dependencies_are_ignored_by_gating() {
        // Companion skill dependencies are advisory metadata only — gating
        // does not check them. Gating should pass even when `skills` is
        // populated.
        let req = GatingRequirements {
            skills: vec![
                "commitment-triage".to_string(),
                "commitment-digest".to_string(),
            ],
            ..Default::default()
        };
        let result = check_requirements_sync(&req);
        assert!(result.passed);
        assert!(result.failures.is_empty());
    }
}
