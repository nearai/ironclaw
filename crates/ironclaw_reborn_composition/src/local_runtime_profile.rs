use std::path::PathBuf;

use ironclaw_host_api::runtime_policy::{DeploymentMode, RuntimeProfile};
use ironclaw_runtime_policy::{EffectiveRuntimePolicy as ResolvedRuntimePolicy, ResolveError};
use thiserror::Error;

use crate::{RebornBuildInput, RebornCompositionProfile};

#[derive(Debug, Error)]
pub enum RebornLocalRuntimeProfileError {
    #[error("profile={profile} is not a local Reborn runtime profile")]
    UnsupportedProfile { profile: RebornCompositionProfile },
    #[error(
        "profile=hosted-single-tenant-volume requires a binary built with the `libsql` feature"
    )]
    MissingLibsqlFeature,
    #[error("failed to resolve local runtime policy: {0}")]
    Policy(#[from] ResolveError),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RebornLocalRuntimeProfileOptions {
    pub confirm_host_access: bool,
}

/// Build the local runtime substrate input and its matching runtime policy from
/// one profile mapping, so yolo policy and process behavior cannot drift.
pub fn local_runtime_build_input(
    profile: RebornCompositionProfile,
    owner_id: impl Into<String>,
    root: PathBuf,
) -> Result<RebornBuildInput, RebornLocalRuntimeProfileError> {
    local_runtime_build_input_with_options(
        profile,
        owner_id,
        root,
        RebornLocalRuntimeProfileOptions::default(),
    )
}

/// Build the local runtime substrate input while applying local-only operator
/// confirmations such as trusted host access.
pub fn local_runtime_build_input_with_options(
    profile: RebornCompositionProfile,
    owner_id: impl Into<String>,
    root: PathBuf,
    options: RebornLocalRuntimeProfileOptions,
) -> Result<RebornBuildInput, RebornLocalRuntimeProfileError> {
    if profile == RebornCompositionProfile::HostedSingleTenantVolume {
        return hosted_single_tenant_volume_build_input(owner_id, root);
    }

    let policy = local_runtime_policy(profile, options)?;
    Ok(
        RebornBuildInput::local_dev_with_profile(profile, owner_id, root)
            .with_runtime_policy(policy),
    )
}

/// Build the hosted single-tenant volume substrate input with the matching
/// secure hosted runtime policy.
pub(crate) fn hosted_single_tenant_volume_build_input(
    owner_id: impl Into<String>,
    root: PathBuf,
) -> Result<RebornBuildInput, RebornLocalRuntimeProfileError> {
    #[cfg(not(feature = "libsql"))]
    {
        let _ = owner_id;
        let _ = root;
        Err(RebornLocalRuntimeProfileError::MissingLibsqlFeature)
    }

    #[cfg(feature = "libsql")]
    let policy = hosted_single_tenant_volume_runtime_policy()
        .map_err(RebornLocalRuntimeProfileError::Policy)?;
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
    let request = ironclaw_runtime_policy::ResolveRequest::new(
        DeploymentMode::HostedMultiTenant,
        RuntimeProfile::SecureDefault,
    );
    ironclaw_runtime_policy::resolve(request)
}

/// Resolved policy for trusted single-user local development with inherited
/// host environment access.
pub fn local_dev_yolo_runtime_policy(
    confirm_host_access: bool,
) -> Result<ResolvedRuntimePolicy, ResolveError> {
    local_runtime_policy(
        RebornCompositionProfile::LocalDevYolo,
        RebornLocalRuntimeProfileOptions {
            confirm_host_access,
        },
    )
    .map_err(|error| match error {
        RebornLocalRuntimeProfileError::Policy(error) => error,
        RebornLocalRuntimeProfileError::MissingLibsqlFeature => {
            unreachable!("local-dev-yolo is not the hosted volume profile")
        }
        RebornLocalRuntimeProfileError::UnsupportedProfile { .. } => {
            unreachable!("local-dev-yolo is a local runtime profile")
        }
    })
}

fn local_runtime_policy(
    profile: RebornCompositionProfile,
    options: RebornLocalRuntimeProfileOptions,
) -> Result<ResolvedRuntimePolicy, RebornLocalRuntimeProfileError> {
    let runtime_profile = match profile {
        RebornCompositionProfile::LocalDev => RuntimeProfile::LocalDev,
        RebornCompositionProfile::LocalDevYolo => RuntimeProfile::LocalYolo,
        RebornCompositionProfile::HostedSingleTenantVolume => {
            return hosted_single_tenant_volume_runtime_policy()
                .map_err(RebornLocalRuntimeProfileError::Policy);
        }
        RebornCompositionProfile::Disabled
        | RebornCompositionProfile::HostedSingleTenant
        | RebornCompositionProfile::Production
        | RebornCompositionProfile::MigrationDryRun => {
            return Err(RebornLocalRuntimeProfileError::UnsupportedProfile { profile });
        }
    };
    let request = ironclaw_runtime_policy::ResolveRequest {
        yolo_disclosure_acknowledged: options.confirm_host_access,
        ..ironclaw_runtime_policy::ResolveRequest::new(
            DeploymentMode::LocalSingleUser,
            runtime_profile,
        )
    };
    Ok(ironclaw_runtime_policy::resolve(request)?)
}

fn local_runtime_policy_for_local_dev_shape(
    profile_name: &'static str,
) -> Result<ResolvedRuntimePolicy, ResolveError> {
    local_runtime_policy(
        RebornCompositionProfile::LocalDev,
        RebornLocalRuntimeProfileOptions::default(),
    )
    .map_err(|error| match error {
        RebornLocalRuntimeProfileError::Policy(error) => error,
        RebornLocalRuntimeProfileError::MissingLibsqlFeature => {
            unreachable!("{profile_name} is not the hosted volume profile")
        }
        RebornLocalRuntimeProfileError::UnsupportedProfile { .. } => {
            unreachable!("{profile_name} uses the local-dev runtime policy shape")
        }
    })
}
