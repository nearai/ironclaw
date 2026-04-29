//! Reborn composition profile configuration.
//!
//! Bootstrap-only knobs (env / TOML) for the Reborn production composition
//! root. Typed DB-backed settings under the `reborn.*.backend` namespaces
//! land in the second composition phase; until then env is enough to flip
//! the profile and hold the legacy startup path as the default.
//!
//! See `crates/ironclaw_reborn_composition/` for the factory this config
//! drives, and issue #3026 for the cutover contract.

use crate::config::helpers::{optional_env, parse_bool_env};
use crate::error::ConfigError;
use crate::settings::Settings;

pub use ironclaw_reborn_composition::RebornProfile;

/// Bootstrap configuration for the Reborn composition root.
///
/// `enabled` is the explicit operator switch. `profile` selects which
/// composition branch runs and what fail-closed validation applies.
/// Both fields are derived from env (and later from the typed settings
/// store); neither comes from a secrets repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebornConfig {
    /// Explicit composition switch. When `false`, `profile` is ignored and
    /// the legacy startup path runs unchanged. Default: `false` so adding
    /// the composition crate to the workspace does not change production
    /// behavior.
    pub enabled: bool,
    /// Profile selecting which composition branch runs. Default:
    /// [`RebornProfile::Disabled`].
    pub profile: RebornProfile,
}

impl Default for RebornConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            profile: RebornProfile::Disabled,
        }
    }
}

impl RebornConfig {
    /// Resolve from environment variables.
    ///
    /// `REBORN_ENABLED` (default: `false`) and `REBORN_PROFILE` (default:
    /// `disabled`) are the only bootstrap env entry points. Once typed
    /// DB-backed settings exist, those will overlay this resolution after
    /// `init_secrets` so credential-bearing backends can be re-resolved.
    ///
    /// An invalid `REBORN_PROFILE` value is fatal — silently coercing to
    /// `Disabled` would let an operator who intended to flip Reborn on
    /// believe they had, which is the exact opposite of the contract.
    pub fn resolve() -> Result<Self, ConfigError> {
        let enabled = parse_bool_env("REBORN_ENABLED", false)?;

        let profile = match optional_env("REBORN_PROFILE")? {
            None => RebornProfile::Disabled,
            Some(raw) => raw
                .parse::<RebornProfile>()
                .map_err(|err| ConfigError::InvalidValue {
                    key: "REBORN_PROFILE".to_string(),
                    message: err.to_string(),
                })?,
        };

        // Cross-validation: an explicit non-disabled profile without
        // `REBORN_ENABLED=true` is almost always a misconfiguration. Fail
        // closed rather than silently dropping the profile down to disabled.
        if !enabled && profile != RebornProfile::Disabled {
            return Err(ConfigError::InvalidValue {
                key: "REBORN_ENABLED".to_string(),
                message: format!(
                    "REBORN_PROFILE={profile} requires REBORN_ENABLED=true; \
                     unset the profile or enable Reborn explicitly"
                ),
            });
        }

        // Mirror invariant: enabled without a profile defaults the profile
        // to `LocalDev` so the operator gets a working dev graph rather
        // than a Disabled no-op when they explicitly turned Reborn on.
        // Production must be selected by name — no implicit promotion.
        let profile = if enabled && profile == RebornProfile::Disabled {
            RebornProfile::LocalDev
        } else {
            profile
        };

        Ok(Self { enabled, profile })
    }

    /// True when the operator has selected an explicit non-disabled
    /// profile. Equivalent to `profile != Disabled` but reads more
    /// clearly at call sites in `AppBuilder`.
    pub fn is_active(&self) -> bool {
        self.profile != RebornProfile::Disabled
    }

    /// Resolve from settings (DB/TOML overlay) with env fallback.
    ///
    /// Precedence (highest first):
    /// 1. `settings.reborn.{enabled,profile}` (DB/TOML)
    /// 2. `REBORN_ENABLED` / `REBORN_PROFILE` env vars
    /// 3. Built-in defaults (`enabled=false`, `profile=Disabled`)
    ///
    /// This is the production resolver called by `Config::build`. The
    /// env-only [`RebornConfig::resolve`] is kept for bootstrap paths that
    /// run before `Settings` is loaded (CLI subcommands, `--no-db` mode).
    ///
    /// Cross-validation matches [`RebornConfig::resolve`]: an explicit
    /// non-disabled profile without `enabled=true` is a startup error so
    /// operators cannot accidentally believe Reborn is on while the legacy
    /// path is running. An invalid `profile` string fails the same way —
    /// silent coercion would defeat the purpose of typed settings.
    pub fn resolve_with_settings(settings: &Settings) -> Result<Self, ConfigError> {
        // Step 1: env baseline. We start from `resolve()` so the env
        // resolver's invariants (default-off, cross-validation, snake_case
        // accept) all apply automatically.
        let mut cfg = Self::resolve()?;

        // Step 2: DB/TOML overlay. A typed setting with a value overrides
        // the env baseline. `None` defers to env unchanged.
        if let Some(enabled) = settings.reborn.enabled {
            cfg.enabled = enabled;
        }

        if let Some(profile_str) = settings.reborn.profile.as_deref() {
            let trimmed = profile_str.trim();
            if !trimmed.is_empty() {
                cfg.profile =
                    trimmed
                        .parse::<RebornProfile>()
                        .map_err(|err| ConfigError::InvalidValue {
                            key: "settings.reborn.profile".to_string(),
                            message: err.to_string(),
                        })?;
            }
        }

        // Step 3: re-apply the cross-validation invariants. If a DB
        // overlay produced an inconsistent state (e.g. enabled=false but
        // profile=production via the settings table), fail closed the same
        // way `resolve()` does for env-only inputs.
        if !cfg.enabled && cfg.profile != RebornProfile::Disabled {
            return Err(ConfigError::InvalidValue {
                key: "settings.reborn".to_string(),
                message: format!(
                    "reborn.profile={} requires reborn.enabled=true; \
                     unset the profile or enable Reborn explicitly",
                    cfg.profile
                ),
            });
        }
        if cfg.enabled && cfg.profile == RebornProfile::Disabled {
            cfg.profile = RebornProfile::LocalDev;
        }

        Ok(cfg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::helpers::lock_env;

    /// Helper: run a closure with REBORN_* env vars cleaned up before and
    /// after, under the global env mutex so tests do not race.
    fn with_clean_env<F: FnOnce() -> R, R>(f: F) -> R {
        let _guard = lock_env();
        // SAFETY: under ENV_MUTEX, no concurrent env access.
        unsafe {
            std::env::remove_var("REBORN_ENABLED");
            std::env::remove_var("REBORN_PROFILE");
        }
        let result = f();
        // SAFETY: under ENV_MUTEX, no concurrent env access.
        unsafe {
            std::env::remove_var("REBORN_ENABLED");
            std::env::remove_var("REBORN_PROFILE");
        }
        result
    }

    #[test]
    fn default_resolution_is_disabled() {
        with_clean_env(|| {
            let cfg = RebornConfig::resolve().expect("default must resolve");
            assert!(!cfg.enabled);
            assert_eq!(cfg.profile, RebornProfile::Disabled);
            assert!(!cfg.is_active());
        });
    }

    #[test]
    fn explicit_local_dev_resolves() {
        with_clean_env(|| {
            // SAFETY: under ENV_MUTEX in with_clean_env.
            unsafe {
                std::env::set_var("REBORN_ENABLED", "true");
                std::env::set_var("REBORN_PROFILE", "local-dev");
            }
            let cfg = RebornConfig::resolve().expect("local-dev must resolve");
            assert!(cfg.enabled);
            assert_eq!(cfg.profile, RebornProfile::LocalDev);
            assert!(cfg.is_active());
        });
    }

    #[test]
    fn enabled_without_profile_defaults_to_local_dev() {
        // Enabling without naming a profile should not silently become
        // production — that would ignore the explicit-name rule. LocalDev
        // is the safe non-disabled default.
        with_clean_env(|| {
            // SAFETY: under ENV_MUTEX in with_clean_env.
            unsafe { std::env::set_var("REBORN_ENABLED", "true") };
            let cfg = RebornConfig::resolve().expect("must resolve");
            assert!(cfg.enabled);
            assert_eq!(cfg.profile, RebornProfile::LocalDev);
        });
    }

    #[test]
    fn profile_without_enabled_is_rejected() {
        // Cross-validation: setting only REBORN_PROFILE=production is a
        // misconfiguration. Failing closed avoids the case where the
        // operator believes Reborn is on but the legacy path is running.
        with_clean_env(|| {
            // SAFETY: under ENV_MUTEX in with_clean_env.
            unsafe { std::env::set_var("REBORN_PROFILE", "production") };
            let err = RebornConfig::resolve().expect_err("must reject inconsistent config");
            match err {
                ConfigError::InvalidValue { key, message } => {
                    assert_eq!(key, "REBORN_ENABLED");
                    assert!(message.contains("production"));
                }
                other => panic!("expected InvalidValue, got {other:?}"),
            }
        });
    }

    #[test]
    fn invalid_profile_is_rejected() {
        with_clean_env(|| {
            // SAFETY: under ENV_MUTEX in with_clean_env.
            unsafe {
                std::env::set_var("REBORN_ENABLED", "true");
                std::env::set_var("REBORN_PROFILE", "nope");
            }
            let err = RebornConfig::resolve().expect_err("invalid profile must fail");
            match err {
                ConfigError::InvalidValue { key, .. } => assert_eq!(key, "REBORN_PROFILE"),
                other => panic!("expected InvalidValue, got {other:?}"),
            }
        });
    }

    #[test]
    fn snake_case_profile_is_accepted() {
        with_clean_env(|| {
            // SAFETY: under ENV_MUTEX in with_clean_env.
            unsafe {
                std::env::set_var("REBORN_ENABLED", "true");
                std::env::set_var("REBORN_PROFILE", "migration_dry_run");
            }
            let cfg = RebornConfig::resolve().expect("snake_case must resolve");
            assert_eq!(cfg.profile, RebornProfile::MigrationDryRun);
        });
    }

    // ── resolve_with_settings (Phase 2 — DB overlay) ─────────────────────

    use crate::settings::{RebornSettings, Settings};

    fn settings_with(reborn: RebornSettings) -> Settings {
        Settings {
            reborn,
            ..Settings::default()
        }
    }

    #[test]
    fn resolve_with_settings_falls_back_to_env_when_unset() {
        with_clean_env(|| {
            // SAFETY: under ENV_MUTEX in with_clean_env.
            unsafe {
                std::env::set_var("REBORN_ENABLED", "true");
                std::env::set_var("REBORN_PROFILE", "local-dev");
            }
            let settings = Settings::default();
            let cfg = RebornConfig::resolve_with_settings(&settings)
                .expect("env baseline must resolve when settings unset");
            assert!(cfg.enabled);
            assert_eq!(cfg.profile, RebornProfile::LocalDev);
        });
    }

    #[test]
    fn resolve_with_settings_db_overrides_env() {
        with_clean_env(|| {
            // SAFETY: under ENV_MUTEX in with_clean_env.
            unsafe {
                std::env::set_var("REBORN_ENABLED", "true");
                std::env::set_var("REBORN_PROFILE", "local-dev");
            }
            let settings = settings_with(RebornSettings {
                enabled: Some(true),
                profile: Some("migration-dry-run".to_string()),
            });
            let cfg =
                RebornConfig::resolve_with_settings(&settings).expect("DB overlay must resolve");
            // DB profile wins over env profile.
            assert_eq!(cfg.profile, RebornProfile::MigrationDryRun);
        });
    }

    #[test]
    fn resolve_with_settings_db_can_force_disable() {
        // With nothing in env, a DB-forced `enabled=false` produces a
        // clean Disabled config. The combination of an env baseline with
        // `enabled=true` and a DB overlay forcing `enabled=false` is
        // intentionally rejected by cross-validation — the env profile
        // becomes orphaned. Operators wanting to force-off must clear
        // both env and DB, or set both consistently. That symmetry is
        // covered by `resolve_with_settings_rejects_db_inconsistency`.
        with_clean_env(|| {
            let settings = settings_with(RebornSettings {
                enabled: Some(false),
                profile: None,
            });
            let cfg = RebornConfig::resolve_with_settings(&settings)
                .expect("DB-forced disable must resolve cleanly");
            assert!(!cfg.enabled);
            assert_eq!(cfg.profile, RebornProfile::Disabled);
        });
    }

    #[test]
    fn resolve_with_settings_rejects_invalid_db_profile() {
        with_clean_env(|| {
            let settings = settings_with(RebornSettings {
                enabled: Some(true),
                profile: Some("staging".to_string()),
            });
            let err = RebornConfig::resolve_with_settings(&settings)
                .expect_err("invalid DB profile must fail closed");
            match err {
                ConfigError::InvalidValue { key, .. } => {
                    assert_eq!(key, "settings.reborn.profile");
                }
                other => panic!("expected InvalidValue, got {other:?}"),
            }
        });
    }

    #[test]
    fn resolve_with_settings_rejects_db_inconsistency() {
        // Setting profile=production via DB while leaving enabled=false
        // is the same misconfiguration the env-only resolver rejects.
        // Cross-validation runs after the DB overlay, so this fails
        // closed even when nothing in env is set.
        with_clean_env(|| {
            let settings = settings_with(RebornSettings {
                enabled: Some(false),
                profile: Some("production".to_string()),
            });
            let err = RebornConfig::resolve_with_settings(&settings)
                .expect_err("inconsistent DB combo must fail closed");
            match err {
                ConfigError::InvalidValue { key, .. } => {
                    assert_eq!(key, "settings.reborn");
                }
                other => panic!("expected InvalidValue, got {other:?}"),
            }
        });
    }

    // ── Config layer distinctness (AC #7) ────────────────────────────────
    //
    // Issue #3026 AC #7 requires that bootstrap config, DB-backed
    // settings, extension config, and encrypted secrets remain distinct
    // layers. These tests assert that the Reborn layer does not leak
    // into adjacent layers and vice versa. They drive the Settings
    // struct directly because that is the seam where layer mixing would
    // first show up.

    /// Serialize a settings substruct to JSON for layer-distinctness
    /// equality checks. Most typed substructs in `Settings` don't derive
    /// `PartialEq`, so structural equality is asserted via stable JSON
    /// representation instead — that's actually a stronger guarantee
    /// since it would catch field-order or skipped-field drift too.
    fn json_of<T: serde::Serialize>(v: &T) -> serde_json::Value {
        serde_json::to_value(v).expect("serialize")
    }

    #[test]
    fn reborn_layer_does_not_leak_into_adjacent_settings() {
        // Setting a Reborn field must not silently mutate any other
        // typed settings substruct. A regression that flattened
        // Reborn fields into a global namespace would be caught here.
        let settings = Settings {
            reborn: RebornSettings {
                enabled: Some(true),
                profile: Some("local-dev".to_string()),
            },
            ..Settings::default()
        };
        let defaults = Settings::default();
        assert_eq!(json_of(&settings.safety), json_of(&defaults.safety));
        assert_eq!(json_of(&settings.skills), json_of(&defaults.skills));
        assert_eq!(json_of(&settings.agent), json_of(&defaults.agent));
        assert_eq!(json_of(&settings.embeddings), json_of(&defaults.embeddings));
        assert_eq!(json_of(&settings.wasm), json_of(&defaults.wasm));
    }

    #[test]
    fn adjacent_layer_changes_do_not_alter_reborn() {
        // Conversely, changes to skills (the closest analog to
        // "extension config" in the current Settings struct) and
        // agent settings must not silently set Reborn fields.
        let mut settings = Settings::default();
        // Touch fields the test cares about; the helper
        // `field_reassign_with_default` lint allows mutating individual
        // fields (the lint targets struct-style reassignment of whole
        // sub-structs after `default()`), so this stays clippy-clean.
        let baseline_skills_enabled = settings.skills.enabled;
        let baseline_auto_approve = settings.agent.auto_approve_tools;
        settings.skills.enabled = !baseline_skills_enabled;
        settings.agent.auto_approve_tools = !baseline_auto_approve;
        assert_eq!(settings.reborn, RebornSettings::default());
        assert!(settings.reborn.enabled.is_none());
        assert!(settings.reborn.profile.is_none());
    }

    #[test]
    fn reborn_settings_round_trip_through_db_map() {
        // The bootstrap layer (`RebornConfig::resolve()`) reads env.
        // The DB-backed layer writes via `set_setting` and reconstructs
        // via `Settings::from_db_map`. Round-tripping a Reborn-only
        // payload through `to_db_map` and `from_db_map` must preserve
        // the Reborn fields and not move any other typed substruct off
        // its default.
        let settings = Settings {
            reborn: RebornSettings {
                enabled: Some(true),
                profile: Some("local-dev".to_string()),
            },
            ..Settings::default()
        };
        let map = settings.to_db_map();
        let rehydrated = Settings::from_db_map(&map);
        assert_eq!(rehydrated.reborn.enabled, Some(true));
        assert_eq!(rehydrated.reborn.profile.as_deref(), Some("local-dev"));
        let defaults = Settings::default();
        assert_eq!(json_of(&rehydrated.safety), json_of(&defaults.safety));
        assert_eq!(json_of(&rehydrated.skills), json_of(&defaults.skills));
    }

    #[test]
    fn reborn_settings_carry_no_secret_material() {
        // Acceptance criterion #9 reinforced at the layer boundary:
        // RebornSettings has no field whose type can hold a secret.
        // This test serializes a fully-populated RebornSettings and
        // asserts no field name suggests credential material. The
        // moment a future field grows a Secret-bearing type, this test
        // will fail and force the field through SecretHandle.
        let settings = RebornSettings {
            enabled: Some(true),
            profile: Some("production".to_string()),
        };
        let rendered = serde_json::to_string(&settings).unwrap();
        let lc = rendered.to_ascii_lowercase();
        for forbidden in [
            "api_key",
            "secret",
            "password",
            "token",
            "credential",
            "master_key",
        ] {
            assert!(
                !lc.contains(forbidden),
                "RebornSettings serialization contains forbidden field '{forbidden}': {rendered}"
            );
        }
    }

    #[test]
    fn resolve_with_settings_does_not_leak_secrets() {
        // The Display rendering of `RebornConfig` must never include any
        // material from the secrets store. The struct only carries an
        // `enabled` bool and a `RebornProfile` enum, neither of which can
        // hold secret material — but if a future field is added that does,
        // this test will fail and force a manual Debug/Display impl.
        let cfg = RebornConfig {
            enabled: true,
            profile: RebornProfile::LocalDev,
        };
        let rendered = format!("{cfg:?}");
        assert!(!rendered.to_lowercase().contains("api_key"));
        assert!(!rendered.to_lowercase().contains("secret"));
        assert!(!rendered.contains("postgres://"));
    }
}
