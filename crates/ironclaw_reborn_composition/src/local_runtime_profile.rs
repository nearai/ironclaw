use std::path::PathBuf;

use ironclaw_runtime_policy::{EffectiveRuntimePolicy as ResolvedRuntimePolicy, ResolveError};
use thiserror::Error;

use crate::deployment::DeploymentConfig;
use crate::{RebornBuildInput, RebornCompositionProfile};

#[derive(Debug, Error)]
pub enum RebornRuntimeProfileError {
    #[error("profile={profile} is not a local Reborn runtime profile")]
    UnsupportedProfile { profile: RebornCompositionProfile },
    #[error(
        "profile=hosted-single-tenant-volume requires a binary built with the `libsql` feature"
    )]
    MissingLibsqlFeature,
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
    #[cfg(not(feature = "libsql"))]
    {
        let _ = owner_id;
        let _ = root;
        Err(RebornRuntimeProfileError::MissingLibsqlFeature)
    }

    #[cfg(feature = "libsql")]
    let policy =
        hosted_single_tenant_volume_runtime_policy().map_err(RebornRuntimeProfileError::Policy)?;
    #[cfg(feature = "libsql")]
    Ok(RebornBuildInput::local_dev_with_profile(
        RebornCompositionProfile::HostedSingleTenantVolume,
        owner_id,
        root,
    )
    .with_runtime_policy(policy))
}

/// Resolved policy for the standalone local development runtime profile.
pub fn local_dev_runtime_policy() -> Result<ResolvedRuntimePolicy, ResolveError> {
    local_runtime_policy_for_local_dev_shape("local-dev")
}

/// Resolved policy for the hosted single-tenant local product surface.
pub fn hosted_single_tenant_runtime_policy() -> Result<ResolvedRuntimePolicy, ResolveError> {
    local_runtime_policy_for_local_dev_shape("hosted-single-tenant")
}

/// Resolved policy for a hosted single-tenant preview backed by the local
/// runtime substrate. It keeps process execution disabled while preserving the
/// scoped virtual filesystem, brokered network, brokered secret handles, and
/// ask-always approval posture from the resolver-owned secure default.
pub fn hosted_single_tenant_volume_runtime_policy() -> Result<ResolvedRuntimePolicy, ResolveError> {
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
) -> Result<ResolvedRuntimePolicy, ResolveError> {
    local_runtime_policy(
        RebornCompositionProfile::LocalDevYolo,
        RebornRuntimeProfileOptions {
            confirm_host_access,
        },
    )
    .map_err(|error| match error {
        RebornRuntimeProfileError::Policy(error) => error,
        RebornRuntimeProfileError::MissingLibsqlFeature => {
            unreachable!("local-dev-yolo is not the hosted volume profile")
        }
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
) -> Result<ResolvedRuntimePolicy, RebornRuntimeProfileError> {
    deployment_config_for_profile(profile, options)?
        .resolve()?
        .ok_or(RebornRuntimeProfileError::MissingPolicyRequest { profile })
}

fn local_runtime_policy_for_local_dev_shape(
    profile_name: &'static str,
) -> Result<ResolvedRuntimePolicy, ResolveError> {
    local_runtime_policy(
        RebornCompositionProfile::LocalDev,
        RebornRuntimeProfileOptions::default(),
    )
    .map_err(|error| match error {
        RebornRuntimeProfileError::Policy(error) => error,
        RebornRuntimeProfileError::MissingLibsqlFeature => {
            unreachable!("{profile_name} is not the hosted volume profile")
        }
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
