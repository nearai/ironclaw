//! Runtime profile selection and effective-policy resolution.
//!
//! This module is the *configuration* surface for #3045 — it captures what
//! the operator requested (deployment mode, runtime profile, yolo
//! disclosure, optional org-policy ceiling) and resolves it into the
//! [`EffectiveRuntimePolicy`] the host runtime planner consumes.
//!
//! Selection precedence (highest wins):
//! 1. CLI flags (`--deployment-mode`, `--runtime-profile`, `--yolo-disclosure`)
//! 2. Environment variables (`IRONCLAW_DEPLOYMENT_MODE`,
//!    `IRONCLAW_RUNTIME_PROFILE`, `IRONCLAW_YOLO_DISCLOSURE`)
//! 3. *(future)* DB-backed `Settings` — reserved for a follow-up; not wired
//!    in this PR so we don't conflate runtime-policy selection with
//!    settings-store migration.
//! 4. Defaults: `LocalSingleUser` + `SecureDefault`. Both are the safest
//!    choices and never grant provider-host authority.
//!
//! The resolver in `ironclaw_runtime_policy` is the only sanctioned producer
//! of [`EffectiveRuntimePolicy`]. This module's only job is to gather the
//! inputs from the configuration surface and call `resolve` once at startup.
//!
//! Legacy env vars (`ALLOW_LOCAL_TOOLS`, `SANDBOX_POLICY`,
//! `SANDBOX_ALLOW_FULL_ACCESS`) are intentionally untouched here — their
//! semantics overlap with this layer but live in `agent.rs`/`sandbox.rs`,
//! and the planner integration in PR 5 is the right place to reconcile them
//! against the resolved policy.

use ironclaw_host_api::runtime_policy::{DeploymentMode, RuntimeProfile};
use ironclaw_runtime_policy::{
    EffectiveRuntimePolicy, OrgPolicy, ResolveError, ResolveRequest, resolve,
};

use crate::error::ConfigError;

/// CLI-supplied runtime policy overrides. Each field is `None` when the CLI
/// did not specify the value; callers can then fall through to env vars and
/// defaults. The CLI layer is responsible for parsing user input into these
/// types — `DeploymentMode` and `RuntimeProfile` implement
/// `std::str::FromStr` against their snake_case wire names.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RuntimeConfigOverrides {
    pub deployment: Option<DeploymentMode>,
    pub profile: Option<RuntimeProfile>,
    pub yolo_disclosure_acknowledged: Option<bool>,
}

/// Resolved runtime configuration carried through the rest of the host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeConfig {
    /// What the operator requested before any deployment/policy narrowing.
    pub deployment: DeploymentMode,
    pub requested_profile: RuntimeProfile,
    pub yolo_disclosure_acknowledged: bool,
    /// The resolved policy actually enforced by the host runtime planner.
    pub effective_policy: EffectiveRuntimePolicy,
}

impl RuntimeConfig {
    /// Resolve the runtime configuration from CLI overrides + environment.
    ///
    /// Returns `ConfigError` when the resolver rejects the requested
    /// `(deployment, profile)` pair, when a yolo profile was requested
    /// without `IRONCLAW_YOLO_DISCLOSURE=true` / `--yolo-disclosure`, or
    /// when an env var fails to parse.
    pub fn resolve_from(overrides: &RuntimeConfigOverrides) -> Result<Self, ConfigError> {
        let deployment = match overrides.deployment {
            Some(value) => value,
            None => parse_optional_env::<DeploymentMode>("IRONCLAW_DEPLOYMENT_MODE")?
                .unwrap_or(DeploymentMode::LocalSingleUser),
        };
        let requested_profile = match overrides.profile {
            Some(value) => value,
            None => parse_optional_env::<RuntimeProfile>("IRONCLAW_RUNTIME_PROFILE")?
                .unwrap_or(RuntimeProfile::SecureDefault),
        };
        let yolo_disclosure_acknowledged = match overrides.yolo_disclosure_acknowledged {
            Some(value) => value,
            None => parse_bool_env_or_default("IRONCLAW_YOLO_DISCLOSURE", false)?,
        };

        let request = ResolveRequest {
            deployment,
            requested_profile,
            // Org policy is not yet sourced from the settings store.
            // Surfacing it here without a place to write it would be
            // confusing; reserve it for the follow-up that wires the
            // settings store layer.
            org_policy: OrgPolicy::default(),
            yolo_disclosure_acknowledged,
        };

        let effective_policy = resolve(request).map_err(resolver_error)?;

        Ok(Self {
            deployment,
            requested_profile,
            yolo_disclosure_acknowledged,
            effective_policy,
        })
    }

    /// Convenience for tests / `Config::for_testing`. Always succeeds with
    /// the safest defaults.
    pub fn safe_default() -> Self {
        let deployment = DeploymentMode::LocalSingleUser;
        let requested_profile = RuntimeProfile::SecureDefault;
        let effective_policy = resolve(ResolveRequest::new(deployment, requested_profile))
            .expect("LocalSingleUser + SecureDefault always resolves");
        Self {
            deployment,
            requested_profile,
            yolo_disclosure_acknowledged: false,
            effective_policy,
        }
    }
}

fn parse_optional_env<T>(name: &str) -> Result<Option<T>, ConfigError>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    match std::env::var(name) {
        Ok(value) if value.trim().is_empty() => Ok(None),
        Ok(value) => value
            .parse::<T>()
            .map(Some)
            .map_err(|error| ConfigError::InvalidValue {
                key: name.to_string(),
                message: format!("`{value}`: {error}"),
            }),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(error) => Err(ConfigError::InvalidValue {
            key: name.to_string(),
            message: error.to_string(),
        }),
    }
}

fn parse_bool_env_or_default(name: &str, default: bool) -> Result<bool, ConfigError> {
    match std::env::var(name) {
        Ok(value) if value.trim().is_empty() => Ok(default),
        Ok(value) => match value.to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" | "on" => Ok(true),
            "false" | "0" | "no" | "off" => Ok(false),
            _ => Err(ConfigError::InvalidValue {
                key: name.to_string(),
                message: format!("`{value}`: expected boolean (true/false/1/0/yes/no/on/off)"),
            }),
        },
        Err(std::env::VarError::NotPresent) => Ok(default),
        Err(error) => Err(ConfigError::InvalidValue {
            key: name.to_string(),
            message: error.to_string(),
        }),
    }
}

fn resolver_error(error: ResolveError) -> ConfigError {
    ConfigError::InvalidValue {
        key: "runtime.policy".to_string(),
        message: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_host_api::runtime_policy::{FilesystemBackendKind, ProcessBackendKind};

    /// Lock that serializes env-var-mutating tests in this module so they
    /// don't race when cargo runs them in parallel.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn clear_env() {
        for var in [
            "IRONCLAW_DEPLOYMENT_MODE",
            "IRONCLAW_RUNTIME_PROFILE",
            "IRONCLAW_YOLO_DISCLOSURE",
        ] {
            unsafe { std::env::remove_var(var) };
        }
    }

    #[test]
    fn defaults_to_local_single_user_secure_default_when_unset() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_env();
        let cfg = RuntimeConfig::resolve_from(&RuntimeConfigOverrides::default()).unwrap();
        assert_eq!(cfg.deployment, DeploymentMode::LocalSingleUser);
        assert_eq!(cfg.requested_profile, RuntimeProfile::SecureDefault);
        assert_eq!(
            cfg.effective_policy.resolved_profile,
            RuntimeProfile::SecureDefault
        );
        assert!(!cfg.yolo_disclosure_acknowledged);
        assert!(!cfg.effective_policy.was_reduced());
    }

    #[test]
    fn cli_overrides_take_precedence_over_env() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_env();
        unsafe {
            std::env::set_var("IRONCLAW_DEPLOYMENT_MODE", "hosted_multi_tenant");
            std::env::set_var("IRONCLAW_RUNTIME_PROFILE", "hosted_safe");
        }
        let cfg = RuntimeConfig::resolve_from(&RuntimeConfigOverrides {
            deployment: Some(DeploymentMode::LocalSingleUser),
            profile: Some(RuntimeProfile::LocalDev),
            yolo_disclosure_acknowledged: None,
        })
        .unwrap();
        clear_env();
        assert_eq!(cfg.deployment, DeploymentMode::LocalSingleUser);
        assert_eq!(cfg.requested_profile, RuntimeProfile::LocalDev);
    }

    #[test]
    fn env_vars_drive_resolution_when_no_cli_override() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_env();
        unsafe {
            std::env::set_var("IRONCLAW_DEPLOYMENT_MODE", "hosted_multi_tenant");
            std::env::set_var("IRONCLAW_RUNTIME_PROFILE", "hosted_dev");
        }
        let cfg = RuntimeConfig::resolve_from(&RuntimeConfigOverrides::default()).unwrap();
        clear_env();
        assert_eq!(cfg.deployment, DeploymentMode::HostedMultiTenant);
        assert_eq!(cfg.requested_profile, RuntimeProfile::HostedDev);
        assert_eq!(
            cfg.effective_policy.filesystem_backend,
            FilesystemBackendKind::TenantWorkspace
        );
        assert_eq!(
            cfg.effective_policy.process_backend,
            ProcessBackendKind::TenantSandbox
        );
    }

    #[test]
    fn yolo_profile_without_disclosure_fails_loud() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_env();
        let result = RuntimeConfig::resolve_from(&RuntimeConfigOverrides {
            deployment: Some(DeploymentMode::LocalSingleUser),
            profile: Some(RuntimeProfile::LocalYolo),
            yolo_disclosure_acknowledged: None,
        });
        assert!(matches!(result, Err(ConfigError::InvalidValue { .. })));
    }

    #[test]
    fn yolo_profile_with_disclosure_resolves() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_env();
        let cfg = RuntimeConfig::resolve_from(&RuntimeConfigOverrides {
            deployment: Some(DeploymentMode::LocalSingleUser),
            profile: Some(RuntimeProfile::LocalYolo),
            yolo_disclosure_acknowledged: Some(true),
        })
        .unwrap();
        assert_eq!(cfg.requested_profile, RuntimeProfile::LocalYolo);
        assert_eq!(
            cfg.effective_policy.resolved_profile,
            RuntimeProfile::LocalYolo
        );
    }

    #[test]
    fn hosted_multi_tenant_with_local_profile_fails_loud() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_env();
        let result = RuntimeConfig::resolve_from(&RuntimeConfigOverrides {
            deployment: Some(DeploymentMode::HostedMultiTenant),
            profile: Some(RuntimeProfile::LocalDev),
            yolo_disclosure_acknowledged: None,
        });
        assert!(matches!(result, Err(ConfigError::InvalidValue { .. })));
    }

    #[test]
    fn invalid_env_value_is_a_typed_config_error() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_env();
        unsafe { std::env::set_var("IRONCLAW_RUNTIME_PROFILE", "not_a_profile") };
        let result = RuntimeConfig::resolve_from(&RuntimeConfigOverrides::default());
        clear_env();
        assert!(matches!(result, Err(ConfigError::InvalidValue { .. })));
    }

    #[test]
    fn safe_default_constructs_without_env() {
        let cfg = RuntimeConfig::safe_default();
        assert_eq!(cfg.deployment, DeploymentMode::LocalSingleUser);
        assert_eq!(cfg.requested_profile, RuntimeProfile::SecureDefault);
        assert!(!cfg.yolo_disclosure_acknowledged);
    }
}
