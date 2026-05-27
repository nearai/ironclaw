use std::collections::BTreeSet;

use ironclaw_host_api::EffectKind;
use serde::Deserialize;
use thiserror::Error;

const LOCAL_DEV_CAPABILITY_POLICY_TOML: &str = include_str!("local_dev_capability_policy.toml");

#[derive(Debug, Error)]
pub(crate) enum LocalDevCapabilityPolicyError {
    #[error("local-dev capability policy TOML is invalid: {0}")]
    InvalidToml(#[from] toml::de::Error),
    #[error("local-dev capability policy has no grants")]
    EmptyGrants,
    #[error("local-dev capability policy has duplicate grant for {capability}")]
    DuplicateGrant { capability: String },
    #[error("local-dev capability policy is missing grant for {capability}")]
    MissingGrant { capability: String },
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct LocalDevCapabilityPolicy {
    pub(crate) provider: LocalDevProviderPolicy,
    pub(crate) approval_defaults: LocalDevApprovalDefaultsPolicy,
    pub(crate) grants: Vec<LocalDevCapabilityGrantPolicy>,
}

impl LocalDevCapabilityPolicy {
    pub(crate) fn grant(
        &self,
        capability: &str,
    ) -> Result<&LocalDevCapabilityGrantPolicy, LocalDevCapabilityPolicyError> {
        self.grants
            .iter()
            .find(|grant| grant.capability == capability)
            .ok_or_else(|| LocalDevCapabilityPolicyError::MissingGrant {
                capability: capability.to_string(),
            })
    }
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct LocalDevProviderPolicy {
    pub(crate) id: String,
    pub(crate) manifest_path: String,
    pub(crate) authority_effects: Vec<EffectKind>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct LocalDevApprovalDefaultsPolicy {
    pub(crate) spawn_capability: LocalDevConstraintPolicy,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct LocalDevCapabilityGrantPolicy {
    pub(crate) capability: String,
    #[serde(flatten)]
    pub(crate) constraints: LocalDevConstraintPolicy,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct LocalDevConstraintPolicy {
    pub(crate) effects: Vec<EffectKind>,
    pub(crate) mounts: LocalDevMountProfile,
    pub(crate) network: LocalDevNetworkProfile,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum LocalDevMountProfile {
    Workspace,
    Ambient,
    SkillManagement,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum LocalDevNetworkProfile {
    Default,
    LocalDevWildcard,
}

pub(crate) fn local_dev_capability_policy()
-> Result<LocalDevCapabilityPolicy, LocalDevCapabilityPolicyError> {
    let policy: LocalDevCapabilityPolicy = toml::from_str(LOCAL_DEV_CAPABILITY_POLICY_TOML)?;
    validate_policy(&policy)?;
    Ok(policy)
}

fn validate_policy(policy: &LocalDevCapabilityPolicy) -> Result<(), LocalDevCapabilityPolicyError> {
    if policy.grants.is_empty() {
        return Err(LocalDevCapabilityPolicyError::EmptyGrants);
    }
    let mut seen = BTreeSet::new();
    for grant in &policy.grants {
        if !seen.insert(grant.capability.as_str()) {
            return Err(LocalDevCapabilityPolicyError::DuplicateGrant {
                capability: grant.capability.clone(),
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_local_dev_capability_policy_parses() {
        let policy = local_dev_capability_policy().expect("policy parses");

        assert_eq!(policy.provider.id, "builtin");
        assert_eq!(
            policy.provider.authority_effects,
            vec![
                EffectKind::DispatchCapability,
                EffectKind::ReadFilesystem,
                EffectKind::WriteFilesystem,
                EffectKind::SpawnProcess,
                EffectKind::ExecuteCode,
                EffectKind::Network,
            ]
        );
        assert!(
            policy
                .approval_defaults
                .spawn_capability
                .effects
                .contains(&EffectKind::SpawnProcess)
        );
        assert!(policy.grant("builtin.shell").is_ok());
        assert!(policy.grant("builtin.apply_patch").is_ok());
        assert!(policy.grant("builtin.skill_install").is_ok());
    }
}
