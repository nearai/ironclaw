use std::path::PathBuf;

use crate::bootstrap::ironclaw_base_dir;
use crate::config::helpers::{
    db_first_bool, db_first_or_default, optional_env, parse_optional_env,
};
use crate::error::ConfigError;
use crate::settings::Settings;

/// Skills system configuration.
#[derive(Debug, Clone)]
pub struct SkillsConfig {
    /// Whether the skills system is enabled.
    pub enabled: bool,
    /// Directory containing user-placed skills (default: ~/.ironclaw/skills/).
    /// Skills here are loaded with `Trusted` trust level.
    pub local_dir: PathBuf,
    /// Directory containing registry-installed skills (default: ~/.ironclaw/installed_skills/).
    /// Skills here are loaded with `Installed` trust level and get read-only tool access.
    pub installed_dir: PathBuf,
    /// Maximum number of skills that can be active simultaneously.
    pub max_active_skills: usize,
    /// Maximum total context tokens allocated to skill prompts.
    pub max_context_tokens: usize,
    /// Whether regex activation criteria may auto-load skills.
    ///
    /// Keyword/tag activation and explicit `/skill`/`$skill` style mentions remain available
    /// when this is false.
    pub regex_activation_enabled: bool,
    /// Maximum recursion depth when scanning skill directories for bundle layouts.
    /// Subdirectories without `SKILL.md` are recursed into up to this depth.
    pub max_scan_depth: usize,
}

impl Default for SkillsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            local_dir: default_skills_dir(),
            installed_dir: default_installed_skills_dir(),
            max_active_skills: 3,
            // 6000 tokens accommodates one large persona setup (~3000)
            // plus one or two companion skills (~2000 each). With
            // max_active_skills=3 the slot count is the binding
            // constraint for setup bundles. Chain-loaded companions
            // are selected in requires.skills order, so put the most
            // critical companions first. After setup_marker exclusion
            // retires the setup skill, the full budget goes to
            // reactive skills (commitment-triage, decision-capture, etc.).
            max_context_tokens: 6000,
            regex_activation_enabled: true,
            max_scan_depth: 3,
        }
    }
}

/// Get the default user skills directory (~/.ironclaw/skills/).
fn default_skills_dir() -> PathBuf {
    ironclaw_base_dir().join("skills")
}

/// Get the default installed skills directory (~/.ironclaw/installed_skills/).
fn default_installed_skills_dir() -> PathBuf {
    ironclaw_base_dir().join("installed_skills")
}

impl SkillsConfig {
    pub(crate) fn resolve(settings: &Settings) -> Result<Self, ConfigError> {
        let defaults = crate::settings::SkillsSettings::default();
        let ss = &settings.skills;

        Ok(Self {
            enabled: db_first_bool(ss.enabled, defaults.enabled, "SKILLS_ENABLED")?,
            // local_dir and installed_dir are env-only (filesystem paths, no settings counterpart)
            local_dir: optional_env("SKILLS_DIR")?
                .map(PathBuf::from)
                .unwrap_or_else(default_skills_dir),
            installed_dir: optional_env("SKILLS_INSTALLED_DIR")?
                .map(PathBuf::from)
                .unwrap_or_else(default_installed_skills_dir),
            max_active_skills: db_first_or_default(
                &ss.max_active_skills,
                &defaults.max_active_skills,
                "SKILLS_MAX_ACTIVE",
            )?,
            max_context_tokens: db_first_or_default(
                &ss.max_context_tokens,
                &defaults.max_context_tokens,
                "SKILLS_MAX_CONTEXT_TOKENS",
            )?,
            regex_activation_enabled: db_first_bool(
                ss.regex_activation_enabled,
                defaults.regex_activation_enabled,
                "SKILLS_REGEX_ACTIVATION_ENABLED",
            )?,
            max_scan_depth: parse_optional_env("SKILLS_MAX_SCAN_DEPTH", 3)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Mutex that serialises tests in this module that mutate process-global
    /// env vars.  Mirrors the `ENV_LOCK` pattern used in `config::runtime`.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// `db_first_bool` consults the env var only when the settings value
    /// equals the default. SkillsSettings defaults `regex_activation_enabled`
    /// to `true`, so setting `SKILLS_REGEX_ACTIVATION_ENABLED=false` with a
    /// default settings struct must flip the resolved config to `false`.
    /// Regression guard for the env-var wiring added in #4144 — the
    /// `db_first_bool` path was previously untested.
    #[test]
    fn skills_config_reads_regex_activation_enabled_from_env() {
        // Hold the lock for the duration of the test to prevent concurrent
        // tests from observing a partial env-var mutation.
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let key = "SKILLS_REGEX_ACTIVATION_ENABLED";
        let prior = std::env::var(key).ok();

        // SAFETY (Rust 2024): no other threads are reading this env var
        // during the test — SkillsConfig::resolve is called synchronously
        // on the test thread, and the var is restored before return.
        unsafe { std::env::set_var(key, "false") };

        let settings = Settings::default();
        let cfg = SkillsConfig::resolve(&settings).expect("resolve skills config");
        assert!(
            !cfg.regex_activation_enabled,
            "SKILLS_REGEX_ACTIVATION_ENABLED=false must disable regex activation"
        );

        // Flip it back on and re-resolve to confirm the env value, not a
        // sticky default, drives the resolved field.
        unsafe { std::env::set_var(key, "true") };
        let cfg_on = SkillsConfig::resolve(&settings).expect("resolve skills config");
        assert!(cfg_on.regex_activation_enabled);

        // Restore.
        unsafe {
            match prior {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
    }

    /// Three-way precedence test for `regex_activation_enabled`:
    ///   1. DB/TOML `false` overrides the default `true` (DB wins over env)
    ///   2. Default `true` + no env var resolves to `true`
    ///   3. Default `true` + env=`false` resolves to `false` (env path covered
    ///      by `skills_config_reads_regex_activation_enabled_from_env`)
    ///
    /// The DB/TOML path requires NO env mutation, so no ENV_LOCK is needed.
    #[test]
    fn skills_config_regex_activation_enabled_precedence() {
        use crate::settings::SkillsSettings;

        // DB/TOML false — settings deviates from default so db_first_bool
        // returns the settings value without consulting the env var.
        let settings_false = Settings {
            skills: SkillsSettings {
                regex_activation_enabled: false,
                ..SkillsSettings::default()
            },
            ..Settings::default()
        };
        let cfg = SkillsConfig::resolve(&settings_false).expect("resolve with db false");
        assert!(
            !cfg.regex_activation_enabled,
            "DB/TOML regex_activation_enabled=false must take precedence"
        );

        // Default true + no env var: verify default is preserved.
        // Guard against process-level env pollution by holding the lock while
        // we ensure the key is absent.
        {
            let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let key = "SKILLS_REGEX_ACTIVATION_ENABLED";
            let prior = std::env::var(key).ok();
            // SAFETY (Rust 2024): single-threaded section guarded by ENV_LOCK.
            unsafe { std::env::remove_var(key) };
            let settings_default = Settings::default();
            let cfg_default =
                SkillsConfig::resolve(&settings_default).expect("resolve with default settings");
            assert!(
                cfg_default.regex_activation_enabled,
                "default must be true when env var is absent"
            );
            unsafe {
                match prior {
                    Some(value) => std::env::set_var(key, value),
                    None => std::env::remove_var(key),
                }
            }
        }
    }
}
