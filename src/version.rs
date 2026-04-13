//! Version tracking and downgrade detection.
//!
//! Persists the running IronClaw version in the database on every startup.
//! When the persisted version is newer than the current binary, a warning is
//! emitted so operators notice accidental downgrades (e.g., a Docker restart
//! pulling an older image tag).
//!
//! The "last known version" is stored in the settings table under the system
//! user (`"system"`) with key `"ironclaw.version"`. This is deliberately
//! separate from per-user settings.

use crate::db::SettingsStore;

/// Well-known settings key for the last-recorded binary version.
const VERSION_SETTING_KEY: &str = "ironclaw.version";

/// System-level user ID used for global (non-user) settings.
const SYSTEM_USER: &str = "system";

/// The current binary version, baked in at compile time.
pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Result of comparing the startup version against the previously recorded one.
#[derive(Debug, PartialEq, Eq)]
pub enum VersionTransition {
    /// First boot — no previous version was recorded.
    Fresh,
    /// Same version as last time.
    Unchanged,
    /// Upgraded from `previous` to the current version.
    Upgraded { previous: String },
    /// Downgraded from `previous` to the current version.
    Downgraded { previous: String },
}

/// Record the current version in the DB and detect version transitions.
///
/// Call this early during startup (after DB migrations succeed) so that
/// the warning is visible before any data-modifying operations run.
pub async fn check_and_record_version(db: &dyn SettingsStore) -> Result<VersionTransition, String> {
    check_and_record_version_with(db, CURRENT_VERSION).await
}

/// Same as [`check_and_record_version`], but with an injectable `current` version.
///
/// Exists so tests can drive the full transition matrix without needing to
/// rebuild the binary with a different `CARGO_PKG_VERSION`.
pub async fn check_and_record_version_with(
    db: &dyn SettingsStore,
    current: &str,
) -> Result<VersionTransition, String> {
    let previous = db
        .get_setting(SYSTEM_USER, VERSION_SETTING_KEY)
        .await
        .map_err(|e| format!("failed to read version setting: {e}"))?;

    let transition = match previous.and_then(|v| v.as_str().map(String::from)) {
        None => VersionTransition::Fresh,
        Some(ref prev) if prev == current => VersionTransition::Unchanged,
        Some(ref prev) => match compare_semver(prev, current) {
            std::cmp::Ordering::Greater => VersionTransition::Downgraded {
                previous: prev.clone(),
            },
            std::cmp::Ordering::Less => VersionTransition::Upgraded {
                previous: prev.clone(),
            },
            std::cmp::Ordering::Equal => VersionTransition::Unchanged,
        },
    };

    // Always persist the current version so next boot can compare.
    let value = serde_json::Value::String(current.to_string());
    db.set_setting(SYSTEM_USER, VERSION_SETTING_KEY, &value)
        .await
        .map_err(|e| format!("failed to persist version setting: {e}"))?;

    Ok(transition)
}

/// Read the previously-recorded version *without* modifying state.
///
/// Used by the web gateway to surface the "last known" version to reconnecting
/// browser clients, so they can detect version changes across restarts even if
/// their in-memory state was lost (e.g., page refresh during restart).
pub async fn read_previous_version(db: &dyn SettingsStore) -> Result<Option<String>, String> {
    let previous = db
        .get_setting(SYSTEM_USER, VERSION_SETTING_KEY)
        .await
        .map_err(|e| format!("failed to read version setting: {e}"))?;
    Ok(previous.and_then(|v| v.as_str().map(String::from)))
}

/// Full startup wiring for version tracking.
///
/// Runs `check_and_record_version`, matches on the result, and emits the
/// appropriate tracing log for each transition. This is the function that
/// `AppBuilder::init_database` calls — extracted here so tests can drive the
/// full wiring (not just the pure helper) per the "test through the caller"
/// rule in `.claude/rules/testing.md`.
///
/// Returns the `VersionTransition` so callers (including tests) can assert on it.
/// Never fails startup: a DB error is logged at `warn` and mapped to `Fresh`.
pub async fn run_startup_version_check(db: &dyn SettingsStore) -> VersionTransition {
    run_startup_version_check_with(db, CURRENT_VERSION).await
}

/// Same as [`run_startup_version_check`], with an injectable `current` version.
pub async fn run_startup_version_check_with(
    db: &dyn SettingsStore,
    current: &str,
) -> VersionTransition {
    match check_and_record_version_with(db, current).await {
        Ok(VersionTransition::Fresh) => {
            tracing::debug!("First startup — recorded version {}", current);
            VersionTransition::Fresh
        }
        Ok(VersionTransition::Unchanged) => VersionTransition::Unchanged,
        Ok(VersionTransition::Upgraded { previous }) => {
            tracing::warn!(
                "IronClaw upgraded from {} to {} — running migrations on newer schema",
                previous,
                current
            );
            VersionTransition::Upgraded { previous }
        }
        Ok(VersionTransition::Downgraded { previous }) => {
            tracing::error!(
                "VERSION DOWNGRADE DETECTED: previous version was {}, \
                 but this binary is {}. Data written by the newer version \
                 may be incompatible. If this is unintentional, stop the process \
                 and redeploy the correct version.",
                previous,
                current
            );
            VersionTransition::Downgraded { previous }
        }
        Err(e) => {
            tracing::warn!("Version check failed (non-fatal): {}", e);
            VersionTransition::Fresh
        }
    }
}

/// Compare two semver-style version strings (e.g., "0.24.0" vs "0.21.0").
///
/// Returns `Ordering::Greater` when `a` is newer than `b`.
/// Uses the `semver` crate for proper comparison including pre-release ordering.
/// Treats unparseable versions as equal (with a debug log) rather than falling
/// back to dangerous lexicographic comparison.
fn compare_semver(a: &str, b: &str) -> std::cmp::Ordering {
    let normalize = |s: &str| {
        let s = s.strip_prefix('v').unwrap_or(s);
        semver::Version::parse(s)
    };

    match (normalize(a), normalize(b)) {
        (Ok(va), Ok(vb)) => va.cmp(&vb),
        _ => {
            tracing::debug!(
                a = a,
                b = b,
                "Unparseable version string, treating as equal"
            );
            std::cmp::Ordering::Equal
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compare_semver_basic() {
        assert_eq!(
            compare_semver("0.24.0", "0.21.0"),
            std::cmp::Ordering::Greater
        );
        assert_eq!(compare_semver("0.21.0", "0.24.0"), std::cmp::Ordering::Less);
        assert_eq!(
            compare_semver("0.24.0", "0.24.0"),
            std::cmp::Ordering::Equal
        );
    }

    #[test]
    fn test_compare_semver_major_minor() {
        assert_eq!(
            compare_semver("1.0.0", "0.99.99"),
            std::cmp::Ordering::Greater
        );
        assert_eq!(
            compare_semver("0.2.0", "0.1.99"),
            std::cmp::Ordering::Greater
        );
    }

    #[test]
    fn test_compare_semver_with_v_prefix() {
        // v-prefix is stripped before parsing
        assert_eq!(
            compare_semver("v0.24.0", "0.21.0"),
            std::cmp::Ordering::Greater
        );
        assert_eq!(
            compare_semver("v1.0.0", "v0.99.0"),
            std::cmp::Ordering::Greater
        );
    }

    #[test]
    fn test_compare_semver_with_prerelease() {
        // Per semver spec, pre-release versions sort before the release
        assert_eq!(
            compare_semver("0.24.0-rc1", "0.24.0"),
            std::cmp::Ordering::Less
        );
        assert_eq!(
            compare_semver("0.25.0-beta", "0.24.0"),
            std::cmp::Ordering::Greater
        );
        // Pre-release ordering: alpha < beta < rc1 < rc2
        assert_eq!(
            compare_semver("0.24.0-alpha", "0.24.0-beta"),
            std::cmp::Ordering::Less
        );
        assert_eq!(
            compare_semver("0.24.0-rc.1", "0.24.0-rc.2"),
            std::cmp::Ordering::Less
        );
    }

    #[test]
    fn test_compare_semver_unparseable_treated_as_equal() {
        // Two-component version is not valid semver — treated as equal
        assert_eq!(compare_semver("0.24", "0.21.0"), std::cmp::Ordering::Equal);
        // Garbage strings treated as equal
        assert_eq!(
            compare_semver("not-a-version", "0.1.0"),
            std::cmp::Ordering::Equal
        );
    }

    #[test]
    fn test_compare_semver_no_lexicographic_fallback() {
        // The old code would say "9.0.0" > "10.0.0" via lexicographic fallback.
        // With semver crate, this is correctly handled.
        assert_eq!(compare_semver("9.0.0", "10.0.0"), std::cmp::Ordering::Less);
    }
}
