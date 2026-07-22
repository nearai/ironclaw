//! Deployment configuration: a deployment mode is policy *data* resolved at
//! the composition edge, never a type the kernel or a substrate names.
//!
//! This is the Slice B artifact of
//! `docs/reborn/2026-07-17-architecture-simplification-dto-dyn-local.md`
//! (§4.4 "Eliminate `Local*`", §5.6 "The deployment interface — modes are
//! data"). Each deployment target is one [`DeploymentConfig`] value built by a
//! named constructor; the difference between local-dev, local-dev-yolo, and
//! the hosted volume preview is readable on this one page as data.
//!
//! Two deliberate boundaries:
//!
//! - The sanctioned resolver in `ironclaw_runtime_policy` stays the **only**
//!   producer of [`EffectiveRuntimePolicy`]; [`DeploymentConfig::resolve`] is
//!   a thin adapter over [`ResolveRequest`], not a second policy engine.
//! - Storage roots, workspace paths, and connection pools are runtime
//!   *handles*, not deployment policy — they continue to ride
//!   `RebornStorageInput`. This value carries only the policy request.

use ironclaw_host_api::runtime_policy::{DeploymentMode, RuntimeProfile};
use ironclaw_reborn_event_store::RebornProfile;
use ironclaw_runtime_policy::{
    EffectiveRuntimePolicy, OrgPolicyConstraints, ResolveError, ResolveRequest,
};

use std::path::PathBuf;

use thiserror::Error;

use crate::RebornCompositionProfile;
use crate::input::RebornBuildInput;
use crate::readiness::{RebornReadinessDiagnostic, RebornReadinessState};

/// Which runtime substrate a deployment assembles.
///
/// Replaces the `requires_production_shape` / `uses_local_runtime_substrate`
/// profile predicates as the value `build_reborn_services` and
/// `build_reborn_runtime` dispatch on: a deployment selects a substrate, it
/// does not *have a mode that implies one*.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeSubstrate {
    /// No runtime is assembled — the facades report disabled.
    None,
    /// The local runtime substrate (in-memory / on-disk / libSQL volume).
    Local,
    /// The production-shaped substrate (libSQL or PostgreSQL store graph).
    ProductionShaped,
}

/// Which storage handle shape a deployment is assembled from.
///
/// Replaces the `uses_local_dev_storage_input` predicate *and* the
/// `profile == HostedSingleTenant` pairing checks that guarded
/// `RebornStorageInput` variants: the question "does this deployment take a
/// filesystem root, a hosted single-tenant pool, or an operator-supplied
/// durable store" is an axis, not a mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageShape {
    /// No storage is assembled.
    None,
    /// A local filesystem root (`RebornStorageInput::LocalDev`).
    LocalDevRoot,
    /// A hosted single-tenant PostgreSQL pool plus a workspace root.
    HostedSingleTenantPool,
    /// An operator-supplied durable store (libSQL or PostgreSQL).
    OperatorSupplied,
}

/// Whether, and under what precondition, a deployment may carry live traffic.
///
/// Replaces the `starts_live_runtime` predicate plus every per-profile arm of
/// `enforce_runtime_cutover_gate`. The two gate conditions that used to be
/// spelled out per profile — which readiness state is required, and whether a
/// production-blocking diagnostic vetoes the start — are parameters here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrafficPolicy {
    /// Reborn is switched off; starting a runtime is an error.
    Disabled,
    /// Validates the assembled wiring but must never start live traffic.
    ValidateOnly,
    /// Serves live traffic once readiness reaches `required_readiness`.
    Serve {
        required_readiness: RebornReadinessState,
        /// When set, a readiness diagnostic with `blocks_production` also
        /// vetoes the start. Production-only today.
        veto_on_production_blocking_diagnostic: bool,
    },
}

impl TrafficPolicy {
    pub(crate) fn starts_live_runtime(self) -> bool {
        matches!(self, Self::Serve { .. })
    }

    /// The operator-facing reason this deployment refuses live traffic, or
    /// `None` when it serves.
    ///
    /// Shared by the pre-build check in `build_reborn_runtime` and the
    /// post-build cutover gate so the two cannot drift on wording or on which
    /// deployments are allowed to start.
    pub(crate) fn live_traffic_refusal(self, profile: RebornCompositionProfile) -> Option<String> {
        match self {
            Self::Disabled => Some(format!(
                "profile={profile} must not start live Reborn runtime traffic"
            )),
            Self::ValidateOnly => Some(format!(
                "profile={profile} validates production-shaped wiring but must not start live Reborn runtime traffic"
            )),
            Self::Serve { .. } => None,
        }
    }
}

/// The readiness contract a deployment reports, as data.
///
/// §4.4 Bucket 1: `readiness_contract_for_profile` used to `match` a
/// composition profile to build this pair. Each deployment constructor now
/// carries its own contract and the match is gone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadinessContract {
    pub state: RebornReadinessState,
    pub diagnostics: Vec<RebornReadinessDiagnostic>,
}

/// The runtime-policy request one deployment target makes, expressed as data.
///
/// Absent for deployments that assemble no local runtime policy: the disabled
/// profile and the production-shaped profiles, which carry an operator-supplied
/// policy on `RebornBuildInput` instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RuntimePolicyRequest {
    /// Where IronClaw is running and who owns the machine boundary.
    pub(crate) deployment: DeploymentMode,
    /// The operator-requested runtime preset for this deployment.
    pub(crate) requested_profile: RuntimeProfile,
    /// Operator acknowledgement required before a `*Yolo*` profile resolves.
    pub(crate) yolo_disclosure_acknowledged: bool,
    /// Tenant/org ceiling constraints applied by the resolver.
    pub(crate) org_policy: OrgPolicyConstraints,
}

/// One deployment target, expressed entirely as data.
///
/// This is the §5.6 "modes are data" value: every axis that used to be read by
/// `match`ing a [`RebornCompositionProfile`] — which substrate to assemble,
/// whether live traffic is allowed, what readiness reports, which event-store
/// profile and storage shape to use, and the runtime-policy request — is a
/// field here, set by one of the named constructors below. The whole
/// local/hosted/production diff is readable on this page.
///
/// Two deliberate boundaries are preserved:
///
/// - The sanctioned resolver in `ironclaw_runtime_policy` stays the **only**
///   producer of [`EffectiveRuntimePolicy`]; [`DeploymentConfig::resolve`] is
///   a thin adapter over [`ResolveRequest`], not a second policy engine.
/// - Storage roots, workspace paths, and connection pools are runtime
///   *handles*, not deployment policy — they continue to ride
///   `RebornStorageInput`. This value carries only the policy request and the
///   shape selections.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeploymentConfig {
    /// The profile name this config was built from. A **label** — carried for
    /// logging, telemetry, and the readiness diagnostics the operator reads.
    /// Nothing branches on it; that is what the other fields are for, and the
    /// `reborn_deployment_mode_branching_ratchet` architecture test holds the
    /// line.
    profile: RebornCompositionProfile,
    policy_request: Option<RuntimePolicyRequest>,
    substrate: RuntimeSubstrate,
    traffic: TrafficPolicy,
    readiness: ReadinessContract,
    event_store_profile: RebornProfile,
    /// Whether this deployment reads hosted extension installation state.
    hosted_extension_installation_state: bool,
    storage_shape: StorageShape,
}

impl DeploymentConfig {
    /// Reborn switched off: no substrate, no traffic, disabled readiness.
    pub fn disabled() -> Self {
        Self {
            profile: RebornCompositionProfile::Disabled,
            policy_request: None,
            substrate: RuntimeSubstrate::None,
            traffic: TrafficPolicy::Disabled,
            readiness: ReadinessContract {
                state: RebornReadinessState::Disabled,
                diagnostics: vec![RebornReadinessDiagnostic::disabled()],
            },
            event_store_profile: RebornProfile::LocalDev,
            hosted_extension_installation_state: false,
            storage_shape: StorageShape::None,
        }
    }

    /// Standalone local development on a single-user machine.
    pub fn local_dev() -> Self {
        Self {
            profile: RebornCompositionProfile::LocalDev,
            policy_request: Some(RuntimePolicyRequest {
                deployment: DeploymentMode::LocalSingleUser,
                requested_profile: RuntimeProfile::LocalDev,
                yolo_disclosure_acknowledged: false,
                org_policy: OrgPolicyConstraints::default(),
            }),
            substrate: RuntimeSubstrate::Local,
            traffic: TrafficPolicy::Serve {
                required_readiness: RebornReadinessState::DevOnly,
                veto_on_production_blocking_diagnostic: false,
            },
            readiness: ReadinessContract {
                state: RebornReadinessState::DevOnly,
                diagnostics: vec![RebornReadinessDiagnostic::local_dev()],
            },
            event_store_profile: RebornProfile::LocalDev,
            hosted_extension_installation_state: false,
            storage_shape: StorageShape::LocalDevRoot,
        }
    }

    /// Trusted-laptop local development with minimal approvals. Requires the
    /// operator's explicit host-access confirmation; without it the resolver
    /// fails closed with [`ResolveError::YoloRequiresDisclosure`].
    pub fn local_dev_yolo(confirm_host_access: bool) -> Self {
        Self {
            profile: RebornCompositionProfile::LocalDevYolo,
            policy_request: Some(RuntimePolicyRequest {
                deployment: DeploymentMode::LocalSingleUser,
                requested_profile: RuntimeProfile::LocalYolo,
                yolo_disclosure_acknowledged: confirm_host_access,
                org_policy: OrgPolicyConstraints::default(),
            }),
            readiness: ReadinessContract {
                state: RebornReadinessState::DevOnly,
                diagnostics: vec![RebornReadinessDiagnostic::local_dev_yolo()],
            },
            ..Self::local_dev()
        }
    }

    /// Hosted single-tenant product surface backed by the local runtime
    /// substrate and an operator-supplied store.
    pub fn hosted_single_tenant() -> Self {
        Self {
            profile: RebornCompositionProfile::HostedSingleTenant,
            policy_request: Some(RuntimePolicyRequest {
                deployment: DeploymentMode::LocalSingleUser,
                requested_profile: RuntimeProfile::LocalDev,
                yolo_disclosure_acknowledged: false,
                org_policy: OrgPolicyConstraints::default(),
            }),
            substrate: RuntimeSubstrate::Local,
            traffic: TrafficPolicy::Serve {
                required_readiness: RebornReadinessState::HostedSingleTenantValidated,
                veto_on_production_blocking_diagnostic: false,
            },
            readiness: ReadinessContract {
                state: RebornReadinessState::HostedSingleTenantValidated,
                diagnostics: vec![RebornReadinessDiagnostic::hosted_single_tenant()],
            },
            event_store_profile: RebornProfile::LocalDev,
            hosted_extension_installation_state: true,
            storage_shape: StorageShape::HostedSingleTenantPool,
        }
    }

    /// Hosted single-tenant preview backed by the local runtime substrate:
    /// process execution disabled, scoped virtual filesystem, brokered
    /// network/secrets, ask-always approvals (the resolver-owned secure
    /// default under a hosted deployment boundary).
    pub fn hosted_single_tenant_volume() -> Self {
        Self {
            profile: RebornCompositionProfile::HostedSingleTenantVolume,
            policy_request: Some(RuntimePolicyRequest {
                deployment: DeploymentMode::HostedMultiTenant,
                requested_profile: RuntimeProfile::SecureDefault,
                yolo_disclosure_acknowledged: false,
                org_policy: OrgPolicyConstraints::default(),
            }),
            traffic: TrafficPolicy::Serve {
                required_readiness: RebornReadinessState::HostedSingleTenantVolumePreviewValidated,
                veto_on_production_blocking_diagnostic: false,
            },
            readiness: ReadinessContract {
                state: RebornReadinessState::HostedSingleTenantVolumePreviewValidated,
                diagnostics: vec![RebornReadinessDiagnostic::hosted_single_tenant_volume()],
            },
            hosted_extension_installation_state: true,
            storage_shape: StorageShape::LocalDevRoot,
            ..Self::hosted_single_tenant()
        }
    }

    /// Production: the production-shaped substrate, serving live traffic only
    /// once readiness validates.
    pub fn production() -> Self {
        Self {
            profile: RebornCompositionProfile::Production,
            policy_request: None,
            substrate: RuntimeSubstrate::ProductionShaped,
            traffic: TrafficPolicy::Serve {
                required_readiness: RebornReadinessState::ProductionValidated,
                veto_on_production_blocking_diagnostic: true,
            },
            readiness: ReadinessContract {
                state: RebornReadinessState::ProductionValidated,
                diagnostics: Vec::new(),
            },
            event_store_profile: RebornProfile::Production,
            hosted_extension_installation_state: false,
            storage_shape: StorageShape::OperatorSupplied,
        }
    }

    /// Migration dry run: assembles production-shaped wiring to validate it,
    /// and must never start live traffic.
    pub fn migration_dry_run() -> Self {
        Self {
            profile: RebornCompositionProfile::MigrationDryRun,
            traffic: TrafficPolicy::ValidateOnly,
            readiness: ReadinessContract {
                state: RebornReadinessState::MigrationDryRunValidated,
                diagnostics: Vec::new(),
            },
            ..Self::production()
        }
    }

    /// Map a composition profile to its deployment config.
    ///
    /// This is the **one** place a profile name becomes deployment data
    /// (§4.4). `confirm_host_access` only affects the yolo policy request;
    /// every other axis is profile-determined, so this mapping is infallible
    /// and the profile predicates can delegate to it.
    pub fn for_profile(profile: RebornCompositionProfile, confirm_host_access: bool) -> Self {
        match profile {
            RebornCompositionProfile::Disabled => Self::disabled(),
            RebornCompositionProfile::LocalDev => Self::local_dev(),
            RebornCompositionProfile::LocalDevYolo => Self::local_dev_yolo(confirm_host_access),
            RebornCompositionProfile::HostedSingleTenant => Self::hosted_single_tenant(),
            RebornCompositionProfile::HostedSingleTenantVolume => {
                Self::hosted_single_tenant_volume()
            }
            RebornCompositionProfile::Production => Self::production(),
            RebornCompositionProfile::MigrationDryRun => Self::migration_dry_run(),
        }
    }

    /// The profile label this config was built from. Logging and telemetry
    /// only — never a branch (see the field doc).
    pub fn profile(&self) -> RebornCompositionProfile {
        self.profile
    }

    pub fn substrate(&self) -> RuntimeSubstrate {
        self.substrate
    }

    pub fn traffic(&self) -> TrafficPolicy {
        self.traffic
    }

    pub fn readiness(&self) -> &ReadinessContract {
        &self.readiness
    }

    pub(crate) fn event_store_profile(&self) -> RebornProfile {
        self.event_store_profile
    }

    pub(crate) fn uses_hosted_extension_installation_state(&self) -> bool {
        self.hosted_extension_installation_state
    }

    pub fn storage_shape(&self) -> StorageShape {
        self.storage_shape
    }

    pub(crate) fn uses_local_dev_storage_input(&self) -> bool {
        self.storage_shape == StorageShape::LocalDevRoot
    }

    /// Resolve this deployment's runtime-policy request through the sanctioned
    /// resolver.
    ///
    /// `Ok(None)` for deployments that make no policy request — disabled and
    /// the production-shaped profiles, which carry an operator-supplied policy
    /// on `RebornBuildInput` instead. Distinguishing "no request" from "a
    /// request that failed" keeps the fail-closed resolver error visible
    /// rather than collapsing both into an absent policy.
    pub(crate) fn resolve(&self) -> Result<Option<EffectiveRuntimePolicy>, ResolveError> {
        let Some(request) = self.policy_request.as_ref() else {
            return Ok(None);
        };
        ironclaw_runtime_policy::resolve(ResolveRequest {
            deployment: request.deployment,
            requested_profile: request.requested_profile,
            org_policy: request.org_policy.clone(),
            yolo_disclosure_acknowledged: request.yolo_disclosure_acknowledged,
        })
        .map(Some)
    }

    /// The deployment's capability-policy *data* (§4.4.1 category 1): the
    /// embedded `builtin_capability_policy.toml` — provider policy, approval
    /// gates/defaults, and capability grants — parsed once through the
    /// `OnceLock`-cached loader. Like [`DeploymentConfig::resolve`], this is a
    /// thin adapter over the owning module's loader, not a second policy
    /// source.
    // dead_code allow: config-scoped accessor added by the §4.4 relocation;
    // existing composition call sites still reach the module loader directly
    // and migrate here separately.
    #[allow(dead_code)]
    pub(crate) fn builtin_capability_policy(
        &self,
    ) -> Result<
        crate::builtin_capability_policy::BuiltinCapabilityPolicy,
        crate::builtin_capability_policy::BuiltinCapabilityPolicyError,
    > {
        crate::builtin_capability_policy::builtin_capability_policy()
    }
}

#[derive(Debug, Error)]
pub enum RebornRuntimeProfileError {
    #[error("profile={profile} is not a local Reborn runtime profile")]
    UnsupportedProfile { profile: RebornCompositionProfile },
    #[error("failed to resolve local runtime policy: {0}")]
    Policy(#[from] ResolveError),
    #[error("profile={profile} carries no runtime-policy request to resolve")]
    MissingPolicyRequest { profile: RebornCompositionProfile },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RebornRuntimeProfileOptions {
    pub confirm_host_access: bool,
}

/// Map a composition profile to its [`DeploymentConfig`] value — the one
/// place a profile name becomes deployment policy data (§4.4). Everything
/// past this edge consumes resolved policy values, never a mode.
pub(crate) fn deployment_config_for_profile(
    profile: RebornCompositionProfile,
    options: RebornRuntimeProfileOptions,
) -> Result<DeploymentConfig, RebornRuntimeProfileError> {
    let config = DeploymentConfig::for_profile(profile, options.confirm_host_access);
    // This module builds the *local-dev storage input* shape (a filesystem
    // root). Deployments that take an operator-supplied pool or assemble no
    // runtime are not its business — expressed as the config axis rather than
    // a second list of profile names.
    if !config.uses_local_dev_storage_input() {
        return Err(RebornRuntimeProfileError::UnsupportedProfile { profile });
    }
    Ok(config)
}

/// Build the local runtime substrate input and its matching runtime policy from
/// one profile mapping, so yolo policy and process behavior cannot drift.
pub fn local_runtime_build_input(
    profile: RebornCompositionProfile,
    owner_id: impl Into<String>,
    root: PathBuf,
) -> Result<RebornBuildInput, RebornRuntimeProfileError> {
    local_runtime_build_input_with_options(
        profile,
        owner_id,
        root,
        RebornRuntimeProfileOptions::default(),
    )
}

/// Build the local runtime substrate input while applying local-only operator
/// confirmations such as trusted host access.
pub fn local_runtime_build_input_with_options(
    profile: RebornCompositionProfile,
    owner_id: impl Into<String>,
    root: PathBuf,
    options: RebornRuntimeProfileOptions,
) -> Result<RebornBuildInput, RebornRuntimeProfileError> {
    if profile == RebornCompositionProfile::HostedSingleTenantVolume {
        return hosted_single_tenant_volume_build_input(owner_id, root);
    }

    // Build the deployment once, here, where the operator's host-access
    // confirmation is known, and carry it on the input rather than letting
    // downstream re-derive it from the profile name (§4.4).
    let deployment = deployment_config_for_profile(profile, options)?;
    let policy = deployment
        .resolve()?
        .ok_or(RebornRuntimeProfileError::MissingPolicyRequest { profile })?;
    Ok(
        RebornBuildInput::local_dev_from_deployment(deployment, owner_id, root)
            .with_runtime_policy(policy),
    )
}

/// Build the hosted single-tenant volume substrate input with the matching
/// secure hosted runtime policy.
pub(crate) fn hosted_single_tenant_volume_build_input(
    owner_id: impl Into<String>,
    root: PathBuf,
) -> Result<RebornBuildInput, RebornRuntimeProfileError> {
    let policy =
        hosted_single_tenant_volume_runtime_policy().map_err(RebornRuntimeProfileError::Policy)?;
    Ok(RebornBuildInput::local_dev_with_profile(
        RebornCompositionProfile::HostedSingleTenantVolume,
        owner_id,
        root,
    )
    .with_runtime_policy(policy))
}

/// Resolved policy for the standalone local development runtime profile.
pub fn local_dev_runtime_policy() -> Result<EffectiveRuntimePolicy, ResolveError> {
    local_runtime_policy_for_local_dev_shape("local-dev")
}

/// Resolved policy for the hosted single-tenant local product surface.
pub fn hosted_single_tenant_runtime_policy() -> Result<EffectiveRuntimePolicy, ResolveError> {
    local_runtime_policy_for_local_dev_shape("hosted-single-tenant")
}

/// Resolved policy for a hosted single-tenant preview backed by the local
/// runtime substrate. It keeps process execution disabled while preserving the
/// scoped virtual filesystem, brokered network, brokered secret handles, and
/// ask-always approval posture from the resolver-owned secure default.
pub fn hosted_single_tenant_volume_runtime_policy() -> Result<EffectiveRuntimePolicy, ResolveError>
{
    // The hosted volume preview always carries a policy request, so the
    // `None` arm is unreachable in practice; it maps to the resolver's own
    // fail-closed shape rather than being unwrapped.
    DeploymentConfig::hosted_single_tenant_volume()
        .resolve()
        .and_then(|policy| {
            policy.ok_or(ResolveError::IncompatibleDeployment {
                deployment: ironclaw_host_api::runtime_policy::DeploymentMode::HostedMultiTenant,
                profile: ironclaw_host_api::runtime_policy::RuntimeProfile::SecureDefault,
            })
        })
}

/// Resolved policy for trusted single-user local development with inherited
/// host environment access.
pub fn local_dev_yolo_runtime_policy(
    confirm_host_access: bool,
) -> Result<EffectiveRuntimePolicy, ResolveError> {
    local_runtime_policy(
        RebornCompositionProfile::LocalDevYolo,
        RebornRuntimeProfileOptions {
            confirm_host_access,
        },
    )
    .map_err(|error| match error {
        RebornRuntimeProfileError::Policy(error) => error,
        RebornRuntimeProfileError::UnsupportedProfile { .. } => {
            unreachable!("local-dev-yolo is a local runtime profile")
        }
        RebornRuntimeProfileError::MissingPolicyRequest { .. } => {
            unreachable!("local-dev-yolo carries a runtime-policy request")
        }
    })
}

fn local_runtime_policy(
    profile: RebornCompositionProfile,
    options: RebornRuntimeProfileOptions,
) -> Result<EffectiveRuntimePolicy, RebornRuntimeProfileError> {
    deployment_config_for_profile(profile, options)?
        .resolve()?
        .ok_or(RebornRuntimeProfileError::MissingPolicyRequest { profile })
}

fn local_runtime_policy_for_local_dev_shape(
    profile_name: &'static str,
) -> Result<EffectiveRuntimePolicy, ResolveError> {
    local_runtime_policy(
        RebornCompositionProfile::LocalDev,
        RebornRuntimeProfileOptions::default(),
    )
    .map_err(|error| match error {
        RebornRuntimeProfileError::Policy(error) => error,
        RebornRuntimeProfileError::UnsupportedProfile { .. } => {
            unreachable!("{profile_name} uses the local-dev runtime policy shape")
        }
        RebornRuntimeProfileError::MissingPolicyRequest { .. } => {
            unreachable!("{profile_name} carries a runtime-policy request")
        }
    })
}

#[cfg(test)]
mod tests {
    use ironclaw_host_api::runtime_policy::{ApprovalPolicy, ProcessBackendKind};

    use super::*;

    /// Resolve a config that is known to make a policy request.
    fn resolved(config: DeploymentConfig) -> EffectiveRuntimePolicy {
        config
            .resolve()
            .expect("resolves")
            .expect("config makes a policy request")
    }

    #[test]
    fn every_composition_profile_maps_to_a_deployment_config() {
        // The §4.4 pivot: `for_profile` is the one profile match, and it must
        // cover every variant so nothing downstream needs its own.
        for profile in [
            RebornCompositionProfile::Disabled,
            RebornCompositionProfile::LocalDev,
            RebornCompositionProfile::LocalDevYolo,
            RebornCompositionProfile::HostedSingleTenant,
            RebornCompositionProfile::HostedSingleTenantVolume,
            RebornCompositionProfile::Production,
            RebornCompositionProfile::MigrationDryRun,
        ] {
            let config = DeploymentConfig::for_profile(profile, true);
            assert_eq!(
                config.profile(),
                profile,
                "for_profile must round-trip the label it was built from"
            );
        }
    }

    #[test]
    fn substrate_and_traffic_axes_replace_the_profile_predicates() {
        // Locks the axis values the five former `match profile` sites read,
        // and pins the predicates on the profile enum as thin delegations —
        // they must agree with the config by construction.
        let cases = [
            (
                RebornCompositionProfile::Disabled,
                RuntimeSubstrate::None,
                false,
            ),
            (
                RebornCompositionProfile::LocalDev,
                RuntimeSubstrate::Local,
                true,
            ),
            (
                RebornCompositionProfile::LocalDevYolo,
                RuntimeSubstrate::Local,
                true,
            ),
            (
                RebornCompositionProfile::HostedSingleTenant,
                RuntimeSubstrate::Local,
                true,
            ),
            (
                RebornCompositionProfile::HostedSingleTenantVolume,
                RuntimeSubstrate::Local,
                true,
            ),
            (
                RebornCompositionProfile::Production,
                RuntimeSubstrate::ProductionShaped,
                true,
            ),
            (
                RebornCompositionProfile::MigrationDryRun,
                RuntimeSubstrate::ProductionShaped,
                false,
            ),
        ];
        for (profile, substrate, starts_live) in cases {
            let config = DeploymentConfig::for_profile(profile, true);
            assert_eq!(config.substrate(), substrate, "substrate for {profile}");
            assert_eq!(
                config.traffic().starts_live_runtime(),
                starts_live,
                "starts_live_runtime for {profile}"
            );
            assert_eq!(profile.starts_live_runtime(), starts_live);
            assert_eq!(
                profile.uses_local_dev_storage_input(),
                config.uses_local_dev_storage_input()
            );
            assert_eq!(
                profile.uses_hosted_extension_installation_state(),
                config.uses_hosted_extension_installation_state()
            );
            assert_eq!(
                profile.to_event_store_profile(),
                config.event_store_profile()
            );
            assert_eq!(
                profile.requires_production_shape(),
                substrate == RuntimeSubstrate::ProductionShaped
            );
            assert_eq!(
                profile.uses_local_runtime_substrate(),
                substrate == RuntimeSubstrate::Local
            );
        }
    }

    #[test]
    fn a_serving_deployment_requires_its_own_readiness_state() {
        // The cutover gate compares reported readiness against
        // `TrafficPolicy::Serve::required_readiness`. If a constructor ever set
        // the two independently, the deployment could never start — so the
        // invariant is pinned here rather than discovered at boot.
        for profile in [
            RebornCompositionProfile::LocalDev,
            RebornCompositionProfile::LocalDevYolo,
            RebornCompositionProfile::HostedSingleTenant,
            RebornCompositionProfile::HostedSingleTenantVolume,
            RebornCompositionProfile::Production,
        ] {
            let config = DeploymentConfig::for_profile(profile, true);
            let TrafficPolicy::Serve {
                required_readiness, ..
            } = config.traffic()
            else {
                panic!("{profile} must serve live traffic");
            };
            assert_eq!(
                required_readiness,
                config.readiness().state,
                "{profile} must require the readiness state it reports"
            );
        }
    }

    #[test]
    fn only_production_vetoes_on_a_production_blocking_diagnostic() {
        let production = DeploymentConfig::production();
        assert_eq!(
            production.traffic(),
            TrafficPolicy::Serve {
                required_readiness: RebornReadinessState::ProductionValidated,
                veto_on_production_blocking_diagnostic: true,
            }
        );
        for profile in [
            RebornCompositionProfile::LocalDev,
            RebornCompositionProfile::LocalDevYolo,
            RebornCompositionProfile::HostedSingleTenant,
            RebornCompositionProfile::HostedSingleTenantVolume,
        ] {
            let config = DeploymentConfig::for_profile(profile, true);
            assert!(
                matches!(
                    config.traffic(),
                    TrafficPolicy::Serve {
                        veto_on_production_blocking_diagnostic: false,
                        ..
                    }
                ),
                "{profile} must not inherit the production diagnostic veto"
            );
        }
        assert_eq!(
            DeploymentConfig::migration_dry_run().traffic(),
            TrafficPolicy::ValidateOnly
        );
        assert_eq!(
            DeploymentConfig::disabled().traffic(),
            TrafficPolicy::Disabled
        );
    }

    #[test]
    fn deployments_without_a_policy_request_resolve_to_none() {
        // Disabled and the production-shaped profiles carry an
        // operator-supplied policy on the build input instead. `Ok(None)` must
        // stay distinguishable from a resolver failure.
        for profile in [
            RebornCompositionProfile::Disabled,
            RebornCompositionProfile::Production,
            RebornCompositionProfile::MigrationDryRun,
        ] {
            let resolved = DeploymentConfig::for_profile(profile, false)
                .resolve()
                .expect("no request cannot fail resolution");
            assert!(resolved.is_none(), "{profile} makes no policy request");
        }
    }

    #[test]
    fn readiness_contract_travels_on_the_config() {
        let disabled = DeploymentConfig::disabled();
        assert_eq!(disabled.readiness().state, RebornReadinessState::Disabled);
        assert_eq!(disabled.readiness().diagnostics.len(), 1);

        assert_eq!(
            DeploymentConfig::production().readiness().state,
            RebornReadinessState::ProductionValidated
        );
        assert!(
            DeploymentConfig::production()
                .readiness()
                .diagnostics
                .is_empty()
        );
        assert_eq!(
            DeploymentConfig::migration_dry_run().readiness().state,
            RebornReadinessState::MigrationDryRunValidated
        );
    }

    #[test]
    fn local_dev_resolves_to_local_host_policy() {
        let policy = resolved(DeploymentConfig::local_dev());
        assert_eq!(policy.deployment, DeploymentMode::LocalSingleUser);
        assert_eq!(policy.resolved_profile, RuntimeProfile::LocalDev);
        assert_eq!(policy.process_backend, ProcessBackendKind::LocalHost);
        assert_eq!(policy.approval_policy, ApprovalPolicy::AskDestructive);
    }

    #[test]
    fn local_dev_yolo_without_disclosure_fails_closed() {
        let error = DeploymentConfig::local_dev_yolo(false)
            .resolve()
            .expect_err("yolo without disclosure must fail");
        assert!(matches!(error, ResolveError::YoloRequiresDisclosure { .. }));
    }

    #[test]
    fn local_dev_yolo_with_disclosure_resolves_minimal_approvals() {
        let policy = resolved(DeploymentConfig::local_dev_yolo(true));
        assert_eq!(policy.resolved_profile, RuntimeProfile::LocalYolo);
        assert_eq!(policy.approval_policy, ApprovalPolicy::Minimal);
    }

    #[test]
    fn hosted_single_tenant_volume_resolves_secure_default_without_processes() {
        let policy = resolved(DeploymentConfig::hosted_single_tenant_volume());
        assert_eq!(policy.deployment, DeploymentMode::HostedMultiTenant);
        assert_eq!(policy.resolved_profile, RuntimeProfile::SecureDefault);
        assert_eq!(policy.process_backend, ProcessBackendKind::None);
        assert_eq!(policy.approval_policy, ApprovalPolicy::AskAlways);
    }

    #[test]
    fn deployment_targets_differ_only_as_data() {
        // The whole local/hosted diff is field values on one struct — the
        // §4.4 claim this module exists to make true.
        assert_ne!(
            DeploymentConfig::local_dev(),
            DeploymentConfig::hosted_single_tenant_volume()
        );
        assert_eq!(DeploymentConfig::local_dev(), DeploymentConfig::local_dev());
    }
}

#[cfg(test)]
mod local_runtime_profile_tests {
    use ironclaw_host_api::runtime_policy::{ApprovalPolicy, RuntimeProfile};

    use super::*;

    #[test]
    fn yolo_disclosure_reaches_both_the_carried_deployment_and_the_resolved_policy() {
        // This module is the one place that holds the operator's host-access
        // confirmation, so it must be the place that builds the deployment.
        // The hazard being pinned: `RebornBuildInput::new` cannot know the
        // disclosure, so a config built there would carry
        // `yolo_disclosure_acknowledged: false` and resolve fail-closed. The
        // input must carry the config built *here* instead.
        let dir = std::env::temp_dir().join("reborn-yolo-disclosure-test");
        let input = local_runtime_build_input_with_options(
            RebornCompositionProfile::LocalDevYolo,
            "yolo-owner",
            dir,
            RebornRuntimeProfileOptions {
                confirm_host_access: true,
            },
        )
        .expect("confirmed local-dev-yolo builds");

        assert_eq!(
            input.profile(),
            RebornCompositionProfile::LocalDevYolo,
            "the carried deployment must keep the requested profile label"
        );
        let carried = input
            .deployment()
            .resolve()
            .expect("carried deployment resolves")
            .expect("local-dev-yolo makes a policy request");
        assert_eq!(
            carried.resolved_profile,
            RuntimeProfile::LocalYolo,
            "the carried deployment must have the disclosure, or it would fail closed"
        );
        assert_eq!(carried.approval_policy, ApprovalPolicy::Minimal);
    }

    #[test]
    fn unconfirmed_yolo_fails_closed_before_an_input_is_built() {
        let dir = std::env::temp_dir().join("reborn-yolo-unconfirmed-test");
        let error = local_runtime_build_input_with_options(
            RebornCompositionProfile::LocalDevYolo,
            "yolo-owner",
            dir,
            RebornRuntimeProfileOptions {
                confirm_host_access: false,
            },
        );
        let Err(error) = error else {
            panic!("unconfirmed yolo must not produce a build input");
        };
        assert!(matches!(
            error,
            RebornRuntimeProfileError::Policy(ResolveError::YoloRequiresDisclosure { .. })
        ));
    }

    #[test]
    fn deployments_without_the_local_dev_storage_shape_are_rejected() {
        // The helper builds the local-dev storage input shape; the rejection is
        // expressed as the storage-shape axis, not a list of profile names.
        for profile in [
            RebornCompositionProfile::Disabled,
            RebornCompositionProfile::HostedSingleTenant,
            RebornCompositionProfile::Production,
            RebornCompositionProfile::MigrationDryRun,
        ] {
            let error = deployment_config_for_profile(
                profile,
                RebornRuntimeProfileOptions {
                    confirm_host_access: true,
                },
            )
            .expect_err("non-local-dev-storage deployments are not this helper's business");
            assert!(matches!(
                error,
                RebornRuntimeProfileError::UnsupportedProfile { .. }
            ));
        }
    }
}
