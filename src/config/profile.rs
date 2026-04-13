//! Deployment profiles — TOML-based configuration presets.
//!
//! A profile is a partial `Settings` TOML file that gets merged onto defaults
//! before config.toml and database overlays. This lets users (or ops teams)
//! select a deployment shape with a single env var:
//!
//! ```bash
//! IRONCLAW_PROFILE=server ironclaw
//! ```
//!
//! **Lookup order** for `IRONCLAW_PROFILE=foo`:
//! 1. `~/.ironclaw/profiles/foo.toml`  (user-defined, highest priority)
//! 2. Built-in profiles embedded in the binary
//!
//! Users can create any profile name they want — just drop a TOML file in the
//! profiles directory.
//!
//! **Built-in profiles:** `local`, `local-sandbox`, `server`, `server-multitenant`.

use std::path::PathBuf;

use crate::bootstrap::ironclaw_base_dir;
use crate::config::helpers::optional_env;
use crate::error::ConfigError;
use crate::settings::Settings;

// Built-in profile TOML files, embedded at compile time.
const BUILTIN_LOCAL: &str = include_str!("../../profiles/local.toml");
const BUILTIN_LOCAL_SANDBOX: &str = include_str!("../../profiles/local-sandbox.toml");
const BUILTIN_SERVER: &str = include_str!("../../profiles/server.toml");
const BUILTIN_SERVER_MULTITENANT: &str = include_str!("../../profiles/server-multitenant.toml");

/// Known built-in profile names and their embedded TOML content.
pub(crate) const BUILTIN_PROFILES: &[(&str, &str)] = &[
    ("local", BUILTIN_LOCAL),
    ("local-sandbox", BUILTIN_LOCAL_SANDBOX),
    ("server", BUILTIN_SERVER),
    ("server-multitenant", BUILTIN_SERVER_MULTITENANT),
];

/// Directory under the IronClaw base dir where user-defined profiles live.
const PROFILES_DIR: &str = "profiles";

/// Information about an available profile.
#[derive(Debug, Clone)]
pub struct ProfileInfo {
    /// Profile name (e.g. "server", "my-custom-profile").
    pub name: String,
    /// Whether this is a built-in profile.
    pub builtin: bool,
    /// Path to the TOML file (None for built-in profiles without a user override).
    pub path: Option<PathBuf>,
}

/// Read `IRONCLAW_PROFILE` and merge the profile onto `settings`.
///
/// If the env var is unset or empty, this is a no-op. If set to an unknown
/// name with no user-defined file, returns an error.
pub fn apply_profile(settings: &mut Settings) -> Result<(), ConfigError> {
    let name = match optional_env("IRONCLAW_PROFILE")? {
        Some(n) if !n.is_empty() => n,
        _ => return Ok(()),
    };

    let profile_settings = load_profile(&name)?;
    settings.merge_from(&profile_settings);

    tracing::debug!("Applied deployment profile: {name}");
    Ok(())
}

/// Load a profile by name, checking user directory first, then built-ins.
fn load_profile(name: &str) -> Result<Settings, ConfigError> {
    // Reject names that could escape the profiles directory.
    if name.contains('/') || name.contains('\\') || name.contains("..") || name.starts_with('.') {
        return Err(ConfigError::InvalidValue {
            key: "IRONCLAW_PROFILE".to_string(),
            message: format!(
                "invalid profile name '{name}': must not contain path separators or '..'"
            ),
        });
    }

    // Normalize to lowercase so built-in and user lookups are case-insensitive.
    let name = name.to_ascii_lowercase();

    // 1. Check user-defined profiles directory.
    let user_path = user_profiles_dir().join(format!("{name}.toml"));
    if user_path.is_file() {
        return Settings::load_toml(&user_path)
            .map_err(ConfigError::ParseError)?
            .ok_or(ConfigError::InvalidValue {
                key: "IRONCLAW_PROFILE".to_string(),
                message: format!(
                    "profile file exists but could not be loaded: {}",
                    user_path.display()
                ),
            });
    }

    // 2. Check built-in profiles.
    for &(builtin_name, toml_content) in BUILTIN_PROFILES {
        if builtin_name == name {
            let profile: Settings = toml::from_str(toml_content).map_err(|e| {
                ConfigError::ParseError(format!(
                    "failed to parse built-in profile '{builtin_name}': {e}"
                ))
            })?;
            return Ok(profile);
        }
    }

    // 3. Not found anywhere.
    let available: Vec<&str> = BUILTIN_PROFILES.iter().map(|(n, _)| *n).collect();
    Err(ConfigError::InvalidValue {
        key: "IRONCLAW_PROFILE".to_string(),
        message: format!(
            "unknown profile '{name}'. Built-in profiles: {}. \
             You can also create a custom profile at {}",
            available.join(", "),
            user_path.display(),
        ),
    })
}

/// List all available profiles (built-in + user-defined).
pub fn list_profiles() -> Vec<ProfileInfo> {
    let mut profiles = Vec::new();

    // Built-in profiles.
    for &(name, _) in BUILTIN_PROFILES {
        profiles.push(ProfileInfo {
            name: name.to_string(),
            builtin: true,
            path: None,
        });
    }

    // User-defined profiles.
    let user_dir = user_profiles_dir();
    if let Ok(entries) = std::fs::read_dir(&user_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "toml")
                && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
            {
                let name = stem.to_string();
                // If this overrides a built-in, update that entry instead of adding duplicate.
                if let Some(existing) = profiles.iter_mut().find(|p| p.name == name) {
                    existing.path = Some(path);
                } else {
                    profiles.push(ProfileInfo {
                        name,
                        builtin: false,
                        path: Some(path),
                    });
                }
            }
        }
    }

    profiles
}

/// Path to the user-defined profiles directory: `~/.ironclaw/profiles/`.
fn user_profiles_dir() -> PathBuf {
    ironclaw_base_dir().join(PROFILES_DIR)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::helpers::lock_env;

    #[test]
    fn no_profile_is_noop() {
        let _guard = lock_env();
        // Ensure IRONCLAW_PROFILE is not set.
        unsafe { std::env::remove_var("IRONCLAW_PROFILE") };

        let mut settings = Settings::default();
        let original = settings.clone();
        apply_profile(&mut settings).expect("no profile should be a no-op");

        // Settings should be unchanged.
        let orig_json = serde_json::to_value(&original).unwrap();
        let new_json = serde_json::to_value(&settings).unwrap();
        assert_eq!(orig_json, new_json);
    }

    #[test]
    fn unknown_profile_errors() {
        let _guard = lock_env();
        unsafe { std::env::set_var("IRONCLAW_PROFILE", "nonexistent-xyz-987") };

        let mut settings = Settings::default();
        let result = apply_profile(&mut settings);

        unsafe { std::env::remove_var("IRONCLAW_PROFILE") };

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("nonexistent-xyz-987"),
            "error should mention the profile name: {err}"
        );
        assert!(
            err.contains("local"),
            "error should list built-in profiles: {err}"
        );
    }

    #[test]
    fn path_traversal_rejected() {
        let bad_names = &[
            "../etc/passwd",
            "foo/bar",
            "foo\\bar",
            ".hidden",
            "..sneaky",
        ];
        for name in bad_names {
            let result = load_profile(name);
            assert!(result.is_err(), "profile name '{name}' should be rejected");
            let err = result.unwrap_err().to_string();
            assert!(
                err.contains("must not contain path separators"),
                "error for '{name}' should mention path separators: {err}"
            );
        }
    }

    #[test]
    fn local_profile_disables_gateway() {
        let _guard = lock_env();
        unsafe { std::env::set_var("IRONCLAW_PROFILE", "local") };

        let mut settings = Settings::default();
        apply_profile(&mut settings).expect("local profile should load");

        unsafe { std::env::remove_var("IRONCLAW_PROFILE") };

        assert!(!settings.channels.gateway_enabled);
        assert!(!settings.sandbox.enabled);
        assert!(!settings.heartbeat.enabled);
        assert!(!settings.routines.enabled);
        assert!(!settings.hygiene.enabled);
        assert_eq!(settings.channels.cli_mode.as_deref(), Some("tui"));
        assert_eq!(settings.database_backend.as_deref(), Some("libsql"));
    }

    #[test]
    fn server_profile_enables_postgres() {
        let _guard = lock_env();
        unsafe { std::env::set_var("IRONCLAW_PROFILE", "server") };

        let mut settings = Settings::default();
        apply_profile(&mut settings).expect("server profile should load");

        unsafe { std::env::remove_var("IRONCLAW_PROFILE") };

        assert_eq!(settings.database_backend.as_deref(), Some("postgres"));
        assert!(settings.sandbox.enabled);
        assert!(settings.heartbeat.enabled);
        assert!(settings.hygiene.enabled);
    }

    #[test]
    fn server_multitenant_sets_higher_parallelism() {
        let _guard = lock_env();
        unsafe { std::env::set_var("IRONCLAW_PROFILE", "server-multitenant") };

        let mut settings = Settings::default();
        apply_profile(&mut settings).expect("server-multitenant profile should load");

        unsafe { std::env::remove_var("IRONCLAW_PROFILE") };

        assert_eq!(settings.agent.max_parallel_jobs, 10);
        assert_eq!(settings.database_backend.as_deref(), Some("postgres"));
    }

    #[test]
    fn local_sandbox_enables_sandbox() {
        let _guard = lock_env();
        unsafe { std::env::set_var("IRONCLAW_PROFILE", "local-sandbox") };

        let mut settings = Settings::default();
        apply_profile(&mut settings).expect("local-sandbox profile should load");

        unsafe { std::env::remove_var("IRONCLAW_PROFILE") };

        assert!(settings.sandbox.enabled);
        assert_eq!(settings.sandbox.policy, "readonly");
        assert!(!settings.channels.gateway_enabled);
    }

    #[test]
    fn case_insensitive_profile_name() {
        let _guard = lock_env();
        unsafe { std::env::set_var("IRONCLAW_PROFILE", "SERVER") };

        let mut settings = Settings::default();
        apply_profile(&mut settings).expect("case-insensitive should work");

        unsafe { std::env::remove_var("IRONCLAW_PROFILE") };

        assert_eq!(settings.database_backend.as_deref(), Some("postgres"));
    }

    #[test]
    fn list_profiles_includes_builtins() {
        let profiles = list_profiles();
        let names: Vec<&str> = profiles.iter().map(|p| p.name.as_str()).collect();
        assert!(names.contains(&"local"));
        assert!(names.contains(&"local-sandbox"));
        assert!(names.contains(&"server"));
        assert!(names.contains(&"server-multitenant"));
    }

    #[test]
    fn user_profile_overrides_builtin() {
        // NOTE: We cannot redirect `user_profiles_dir()` in tests because
        // `ironclaw_base_dir()` is cached in a `LazyLock`. Instead we test the
        // override *logic*: load a user TOML that shadows a built-in profile
        // name and verify that `merge_from` makes the user values win.
        let _guard = lock_env();

        // 1. Load the built-in "local" profile to get its baseline values.
        let builtin = load_profile("local").expect("built-in 'local' should load");
        // Built-in local profile uses libsql.
        assert_eq!(builtin.database_backend.as_deref(), Some("libsql"));

        // 2. Create a user TOML that overrides database_backend to postgres.
        let tmp = tempfile::TempDir::new().unwrap();
        let profile_path = tmp.path().join("local.toml");
        std::fs::write(
            &profile_path,
            "database_backend = \"postgres\"\n\n[heartbeat]\nenabled = true\n",
        )
        .unwrap();

        let user_profile: Settings = Settings::load_toml(&profile_path).unwrap().unwrap();

        // 3. Start from defaults, apply built-in, then apply user override
        //    (same layering that load_profile would do if the user dir matched).
        let mut settings = Settings::default();
        settings.merge_from(&builtin);
        settings.merge_from(&user_profile);

        // User values win over built-in.
        assert_eq!(
            settings.database_backend.as_deref(),
            Some("postgres"),
            "user profile should override built-in database_backend"
        );
        assert!(
            settings.heartbeat.enabled,
            "user profile should override built-in heartbeat.enabled"
        );

        // Built-in values that the user didn't override are preserved.
        assert!(
            !settings.channels.gateway_enabled,
            "built-in gateway_enabled=false should survive user overlay"
        );
    }
}
