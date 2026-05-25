use std::path::PathBuf;

use ironclaw_host_api::runtime_policy::{DeploymentMode, RuntimeProfile};
use ironclaw_runtime_policy::{EffectiveRuntimePolicy as ResolvedRuntimePolicy, ResolveError};
use thiserror::Error;

use crate::{RebornBuildInput, RebornCompositionProfile};

#[derive(Debug, Error)]
pub enum RebornLocalRuntimeProfileError {
    #[error("profile={profile} is not a local Reborn runtime profile")]
    UnsupportedProfile { profile: RebornCompositionProfile },
    #[error("failed to resolve local runtime policy: {0}")]
    Policy(#[from] ResolveError),
}

/// Build the local runtime substrate input and its matching runtime policy from
/// one profile mapping, so yolo policy and process behavior cannot drift.
pub fn local_runtime_build_input(
    profile: RebornCompositionProfile,
    owner_id: impl Into<String>,
    root: PathBuf,
) -> Result<RebornBuildInput, RebornLocalRuntimeProfileError> {
    let policy = local_runtime_policy(profile)?;
    Ok(
        RebornBuildInput::local_dev_with_profile(profile, owner_id, root)
            .with_runtime_policy(policy),
    )
}

/// Resolved policy for the standalone local development runtime profile.
pub fn local_dev_runtime_policy() -> Result<ResolvedRuntimePolicy, ResolveError> {
    local_runtime_policy(RebornCompositionProfile::LocalDev).map_err(|error| match error {
        RebornLocalRuntimeProfileError::Policy(error) => error,
        RebornLocalRuntimeProfileError::UnsupportedProfile { .. } => {
            unreachable!("local-dev is a local runtime profile")
        }
    })
}

/// Resolved policy for trusted single-user local development with inherited
/// host environment access.
pub fn local_dev_yolo_runtime_policy() -> Result<ResolvedRuntimePolicy, ResolveError> {
    local_runtime_policy(RebornCompositionProfile::LocalDevYolo).map_err(|error| match error {
        RebornLocalRuntimeProfileError::Policy(error) => error,
        RebornLocalRuntimeProfileError::UnsupportedProfile { .. } => {
            unreachable!("local-dev-yolo is a local runtime profile")
        }
    })
}

fn local_runtime_policy(
    profile: RebornCompositionProfile,
) -> Result<ResolvedRuntimePolicy, RebornLocalRuntimeProfileError> {
    let (runtime_profile, yolo_disclosure_acknowledged) = match profile {
        RebornCompositionProfile::LocalDev => (RuntimeProfile::LocalDev, false),
        RebornCompositionProfile::LocalDevYolo => (RuntimeProfile::LocalYolo, true),
        RebornCompositionProfile::Disabled
        | RebornCompositionProfile::Production
        | RebornCompositionProfile::MigrationDryRun => {
            return Err(RebornLocalRuntimeProfileError::UnsupportedProfile { profile });
        }
    };
    let request = ironclaw_runtime_policy::ResolveRequest {
        yolo_disclosure_acknowledged,
        ..ironclaw_runtime_policy::ResolveRequest::new(
            DeploymentMode::LocalSingleUser,
            runtime_profile,
        )
    };
    Ok(ironclaw_runtime_policy::resolve(request)?)
}
