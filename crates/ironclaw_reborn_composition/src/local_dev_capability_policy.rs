use std::{
    collections::{BTreeSet, HashSet},
    sync::OnceLock,
};

use ironclaw_approvals::LeaseApproval;
use ironclaw_host_api::{
    Action, CapabilityGrant, CapabilityGrantId, CapabilityId, CapabilitySet, EffectKind,
    ExtensionId, GrantConstraints, MountView, NetworkPolicy, NetworkTargetPattern, PackageId,
    Principal,
};
use serde::Deserialize;
use thiserror::Error;

use crate::runtime_profile_approval_policy::RuntimeProfileApprovalGateEffectSets;

const LOCAL_DEV_CAPABILITY_POLICY_TOML: &str = include_str!("local_dev_capability_policy.toml");

#[derive(Debug, Error)]
pub(crate) enum LocalDevCapabilityPolicyError {
    #[error("local-dev capability policy TOML is invalid: {0}")]
    InvalidToml(#[from] toml::de::Error),
    #[error("local-dev capability policy has no grants")]
    EmptyGrants,
    #[error("local-dev capability policy has duplicate grant for {capability}")]
    DuplicateGrant { capability: CapabilityId },
    #[error("local-dev capability policy is missing grant for {capability}")]
    MissingGrant { capability: CapabilityId },
    #[error("local-dev capability policy has empty effect set for {target}")]
    EmptyEffects { target: String },
    #[error("local-dev capability policy has duplicate effect {effect:?} for {target}")]
    DuplicateEffect { target: String, effect: EffectKind },
    #[error("local-dev capability policy provider id is invalid as an extension id: {0}")]
    InvalidProviderExtensionId(#[source] ironclaw_host_api::HostApiError),
    #[error("local-dev capability policy provider manifest path is empty")]
    EmptyProviderManifestPath,
    #[error("local-dev capability policy provider manifest path must be absolute")]
    NonAbsoluteProviderManifestPath,
    #[error("local-dev capability policy is invalid: {reason}")]
    CachedInvalid { reason: String },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LocalDevCapabilityPolicy {
    pub(crate) provider: LocalDevProviderPolicy,
    pub(crate) approval_gates: LocalDevApprovalGatePolicy,
    pub(crate) approval_defaults: LocalDevApprovalDefaultsPolicy,
    pub(crate) grants: Vec<LocalDevCapabilityGrantPolicy>,
}

impl LocalDevCapabilityPolicy {
    fn grant(
        &self,
        capability: &CapabilityId,
    ) -> Result<&LocalDevCapabilityGrantPolicy, LocalDevCapabilityPolicyError> {
        self.grants
            .iter()
            .find(|grant| grant.capability == *capability)
            .ok_or_else(|| LocalDevCapabilityPolicyError::MissingGrant {
                capability: capability.clone(),
            })
    }

    #[cfg(test)]
    pub(crate) fn capability_ids(&self) -> impl Iterator<Item = &CapabilityId> {
        self.grants.iter().map(|grant| &grant.capability)
    }

    pub(crate) fn skill_management_capability_ids(&self) -> impl Iterator<Item = &CapabilityId> {
        self.grants
            .iter()
            .filter(|grant| grant.mounts == LocalDevMountProfile::SkillManagement)
            .map(|grant| &grant.capability)
    }

    pub(crate) fn memory_capability_ids(&self) -> impl Iterator<Item = &CapabilityId> {
        self.grants
            .iter()
            .filter(|grant| grant.mounts == LocalDevMountProfile::Memory)
            .map(|grant| &grant.capability)
    }

    pub(crate) fn system_extensions_lifecycle_capability_ids(
        &self,
    ) -> impl Iterator<Item = &CapabilityId> {
        self.grants
            .iter()
            .filter(|grant| grant.mounts == LocalDevMountProfile::SystemExtensionsLifecycle)
            .map(|grant| &grant.capability)
    }

    pub(crate) fn builtin_grants(
        &self,
        grantee: &ExtensionId,
        workspace_mounts: &MountView,
        skill_mounts: &MountView,
        memory_mounts: &MountView,
        system_extensions_mounts: &MountView,
    ) -> CapabilitySet {
        let grants = self
            .grants
            .iter()
            .map(|grant| CapabilityGrant {
                id: CapabilityGrantId::new(),
                capability: grant.capability.clone(),
                grantee: Principal::Extension(grantee.clone()),
                issued_by: Principal::HostRuntime,
                constraints: constraint_terms(
                    grant,
                    workspace_mounts,
                    skill_mounts,
                    memory_mounts,
                    system_extensions_mounts,
                    None,
                ),
            })
            .collect();
        CapabilitySet { grants }
    }

    fn grant_constraints_for(
        &self,
        capability: &CapabilityId,
        workspace_mounts: &MountView,
        skill_mounts: &MountView,
        memory_mounts: &MountView,
        system_extensions_mounts: &MountView,
    ) -> Result<GrantConstraints, LocalDevCapabilityPolicyError> {
        let grant = self.grant(capability)?;
        Ok(constraint_terms(
            grant,
            workspace_mounts,
            skill_mounts,
            memory_mounts,
            system_extensions_mounts,
            None,
        ))
    }

    pub(crate) fn lease_approval_for(
        &self,
        action: LocalDevApprovalPolicyAction<'_>,
        workspace_mounts: &MountView,
        skill_mounts: &MountView,
        memory_mounts: &MountView,
        system_extensions_mounts: &MountView,
    ) -> Result<LeaseApproval, LocalDevCapabilityPolicyError> {
        let constraints = match action {
            LocalDevApprovalPolicyAction::Dispatch { capability } => self.grant_constraints_for(
                capability,
                workspace_mounts,
                skill_mounts,
                memory_mounts,
                system_extensions_mounts,
            )?,
            LocalDevApprovalPolicyAction::SpawnCapability { capability } => {
                match self.grant(capability) {
                    Ok(grant) => constraint_terms(
                        grant,
                        workspace_mounts,
                        skill_mounts,
                        memory_mounts,
                        system_extensions_mounts,
                        Some(EffectKind::SpawnProcess),
                    ),
                    Err(LocalDevCapabilityPolicyError::MissingGrant { .. }) => {
                        tracing::debug!(
                            %capability,
                            "local-dev spawn capability approval is using default lease terms"
                        );
                        constraint_terms(
                            &self.approval_defaults.spawn_capability,
                            workspace_mounts,
                            skill_mounts,
                            memory_mounts,
                            system_extensions_mounts,
                            None,
                        )
                    }
                    Err(error) => return Err(error),
                }
            }
        };
        Ok(local_dev_one_shot_lease_approval(constraints))
    }

    pub(crate) fn approval_gate_effects(&self) -> RuntimeProfileApprovalGateEffectSets {
        RuntimeProfileApprovalGateEffectSets::new(
            self.approval_gates.ask_writes.clone(),
            self.approval_gates.ask_destructive.clone(),
        )
    }

    pub(crate) fn approval_gate_exempt_capabilities(&self) -> Vec<CapabilityId> {
        self.approval_gates.exempt_capabilities.clone()
    }
}

pub(crate) fn local_dev_one_shot_lease_approval(constraints: GrantConstraints) -> LeaseApproval {
    LeaseApproval {
        issued_by: Principal::HostRuntime,
        allowed_effects: constraints.allowed_effects,
        mounts: constraints.mounts,
        network: constraints.network,
        secrets: constraints.secrets,
        resource_ceiling: constraints.resource_ceiling,
        // Local-dev leases are single-use (max_invocations = 1).
        // Wall-clock expiry is intentionally None: the policy file does
        // not configure an expires_at ceiling, and a short hard-coded
        // timeout would race against slow human approval flows. The
        // one-shot invocation count is the sole consumption bound.
        // If invocation-count enforcement ever regresses, this lease
        // becomes perpetual — see approval gate tests for the invariant.
        expires_at: constraints.expires_at,
        max_invocations: Some(1),
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LocalDevProviderPolicy {
    pub(crate) id: PackageId,
    pub(crate) manifest_path: String,
    pub(crate) authority_effects: Vec<EffectKind>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LocalDevApprovalGatePolicy {
    pub(crate) ask_writes: Vec<EffectKind>,
    pub(crate) ask_destructive: Vec<EffectKind>,
    #[serde(default)]
    pub(crate) exempt_capabilities: Vec<CapabilityId>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LocalDevApprovalDefaultsPolicy {
    pub(crate) spawn_capability: LocalDevConstraintPolicy,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LocalDevCapabilityGrantPolicy {
    pub(crate) capability: CapabilityId,
    pub(crate) effects: Vec<EffectKind>,
    pub(crate) mounts: LocalDevMountProfile,
    pub(crate) network: LocalDevNetworkProfile,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
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
    Memory,
    SystemExtensionsLifecycle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum LocalDevNetworkProfile {
    Default,
    LocalDevWildcard,
}

#[derive(Clone, Copy)]
pub(crate) enum LocalDevApprovalPolicyAction<'a> {
    Dispatch { capability: &'a CapabilityId },
    SpawnCapability { capability: &'a CapabilityId },
}

impl<'a> LocalDevApprovalPolicyAction<'a> {
    pub(crate) fn from_host_action(action: &'a Action) -> Option<Self> {
        match action {
            Action::Dispatch { capability, .. } => Some(Self::Dispatch { capability }),
            Action::SpawnCapability { capability, .. } => {
                Some(Self::SpawnCapability { capability })
            }
            _ => None,
        }
    }

    pub(crate) fn capability(&self) -> &CapabilityId {
        match self {
            Self::Dispatch { capability } | Self::SpawnCapability { capability } => capability,
        }
    }

    pub(crate) fn capability_id(&self) -> &CapabilityId {
        self.capability()
    }

    pub(crate) fn is_spawn_capability(&self) -> bool {
        matches!(self, Self::SpawnCapability { .. })
    }
}

trait LocalDevConstraintSource {
    fn effects(&self) -> &[EffectKind];
    fn mounts(&self) -> LocalDevMountProfile;
    fn network(&self) -> LocalDevNetworkProfile;
}

impl LocalDevConstraintSource for LocalDevCapabilityGrantPolicy {
    fn effects(&self) -> &[EffectKind] {
        &self.effects
    }

    fn mounts(&self) -> LocalDevMountProfile {
        self.mounts
    }

    fn network(&self) -> LocalDevNetworkProfile {
        self.network
    }
}

impl LocalDevConstraintSource for LocalDevConstraintPolicy {
    fn effects(&self) -> &[EffectKind] {
        &self.effects
    }

    fn mounts(&self) -> LocalDevMountProfile {
        self.mounts
    }

    fn network(&self) -> LocalDevNetworkProfile {
        self.network
    }
}

pub(crate) fn local_dev_capability_policy()
-> Result<LocalDevCapabilityPolicy, LocalDevCapabilityPolicyError> {
    static POLICY: OnceLock<Result<LocalDevCapabilityPolicy, String>> = OnceLock::new();
    POLICY
        .get_or_init(|| {
            parse_local_dev_capability_policy(LOCAL_DEV_CAPABILITY_POLICY_TOML)
                .map_err(|error| error.to_string())
        })
        .clone()
        .map_err(|reason| LocalDevCapabilityPolicyError::CachedInvalid { reason })
}

fn parse_local_dev_capability_policy(
    input: &str,
) -> Result<LocalDevCapabilityPolicy, LocalDevCapabilityPolicyError> {
    let policy: LocalDevCapabilityPolicy = toml::from_str(input)?;
    validate_policy(&policy)?;
    Ok(policy)
}

fn validate_policy(policy: &LocalDevCapabilityPolicy) -> Result<(), LocalDevCapabilityPolicyError> {
    ExtensionId::new(policy.provider.id.as_str())
        .map_err(LocalDevCapabilityPolicyError::InvalidProviderExtensionId)?;
    if policy.provider.manifest_path.trim().is_empty() {
        return Err(LocalDevCapabilityPolicyError::EmptyProviderManifestPath);
    }
    if !policy.provider.manifest_path.starts_with('/') {
        return Err(LocalDevCapabilityPolicyError::NonAbsoluteProviderManifestPath);
    }
    validate_effects(
        "provider authority_effects",
        &policy.provider.authority_effects,
    )?;
    validate_effects(
        "approval_gates.ask_writes",
        &policy.approval_gates.ask_writes,
    )?;
    validate_effects(
        "approval_gates.ask_destructive",
        &policy.approval_gates.ask_destructive,
    )?;
    validate_effects(
        "approval_defaults.spawn_capability effects",
        &policy.approval_defaults.spawn_capability.effects,
    )?;
    if policy.grants.is_empty() {
        return Err(LocalDevCapabilityPolicyError::EmptyGrants);
    }
    let mut seen = BTreeSet::new();
    for grant in &policy.grants {
        if !seen.insert(grant.capability.clone()) {
            return Err(LocalDevCapabilityPolicyError::DuplicateGrant {
                capability: grant.capability.clone(),
            });
        }
        validate_effects(
            &format!("grant {} effects", grant.capability),
            &grant.effects,
        )?;
    }
    Ok(())
}

fn validate_effects(
    target: &str,
    effects: &[EffectKind],
) -> Result<(), LocalDevCapabilityPolicyError> {
    if effects.is_empty() {
        return Err(LocalDevCapabilityPolicyError::EmptyEffects {
            target: target.to_string(),
        });
    }
    let mut seen = HashSet::new();
    for effect in effects {
        if !seen.insert(*effect) {
            return Err(LocalDevCapabilityPolicyError::DuplicateEffect {
                target: target.to_string(),
                effect: *effect,
            });
        }
    }
    Ok(())
}

fn constraint_terms(
    source: &impl LocalDevConstraintSource,
    workspace_mounts: &MountView,
    skill_mounts: &MountView,
    memory_mounts: &MountView,
    system_extensions_mounts: &MountView,
    required_effect: Option<EffectKind>,
) -> GrantConstraints {
    let mounts = match source.mounts() {
        LocalDevMountProfile::Workspace => workspace_mounts.clone(),
        LocalDevMountProfile::Ambient => MountView::default(),
        LocalDevMountProfile::SkillManagement => skill_mounts.clone(),
        LocalDevMountProfile::Memory => memory_mounts.clone(),
        LocalDevMountProfile::SystemExtensionsLifecycle => system_extensions_mounts.clone(),
    };
    let network = match source.network() {
        LocalDevNetworkProfile::Default => NetworkPolicy::default(),
        LocalDevNetworkProfile::LocalDevWildcard => local_dev_wildcard_network_policy(),
    };
    let mut allowed_effects = source.effects().to_vec();
    if let Some(effect) = required_effect
        && !allowed_effects.contains(&effect)
    {
        allowed_effects.push(effect);
    }
    GrantConstraints {
        allowed_effects,
        mounts,
        network,
        secrets: Vec::new(),
        resource_ceiling: None,
        expires_at: None,
        max_invocations: None,
    }
}

pub(crate) fn local_dev_wildcard_network_policy() -> NetworkPolicy {
    NetworkPolicy {
        allowed_targets: vec![NetworkTargetPattern {
            scheme: None,
            host_pattern: "*".to_string(),
            port: None,
        }],
        // Local-dev shell is intentionally broad for developer CLI workflows,
        // but it still uses the coarse host-local guard so cloud metadata,
        // link-local, multicast, loopback, and private IP targets remain
        // blocked by the shared network policy enforcer.
        deny_private_ip_ranges: true,
        max_egress_bytes: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_local_dev_capability_policy_parses() {
        let policy = local_dev_capability_policy().expect("policy parses");

        assert_eq!(policy.provider.id.as_str(), "builtin");
        assert_eq!(
            policy.provider.authority_effects,
            vec![
                EffectKind::DispatchCapability,
                EffectKind::ReadFilesystem,
                EffectKind::WriteFilesystem,
                EffectKind::DeleteFilesystem,
                EffectKind::SpawnProcess,
                EffectKind::ExecuteCode,
                EffectKind::Network,
                EffectKind::ExternalWrite,
            ]
        );
        let gate_effects = policy.approval_gate_effects();
        assert!(gate_effects.ask_writes.contains(&EffectKind::SpawnProcess));
        assert!(
            gate_effects
                .ask_destructive
                .contains(&EffectKind::SpawnProcess)
        );
        // onboard is exempt (it runs its own in-turn confirmed=true consent
        // before the network POST); trace_commons.profile_set is deliberately NOT
        // exempt — publishing a public community profile must hit the runtime
        // approval gate, with its model-controlled confirmed=true only as
        // defense-in-depth. builtin.profile_set IS exempt: private local write
        // only (no network/external_write), analogous to memory_write on a fixed path.
        assert!(
            policy
                .approval_gate_exempt_capabilities()
                .iter()
                .any(|capability| capability.as_str() == "builtin.trace_commons.onboard")
        );
        assert!(
            !policy
                .approval_gate_exempt_capabilities()
                .iter()
                .any(|capability| capability.as_str() == "builtin.trace_commons.profile_set")
        );
        assert!(
            policy
                .approval_gate_exempt_capabilities()
                .iter()
                .any(|capability| capability.as_str() == "builtin.profile_set"),
            "builtin.profile_set must be in the exempt list (private local write, no \
             network/external_write — analogous to memory_write on a fixed path)"
        );
        assert!(
            policy
                .approval_defaults
                .spawn_capability
                .effects
                .contains(&EffectKind::SpawnProcess)
        );
        assert_eq!(
            policy.approval_defaults.spawn_capability.mounts,
            LocalDevMountProfile::Workspace
        );
        assert_eq!(
            policy.approval_defaults.spawn_capability.network,
            LocalDevNetworkProfile::Default
        );
        assert!(
            policy
                .grant(&CapabilityId::new("builtin.shell").expect("capability id"))
                .is_ok()
        );
        assert!(
            policy
                .grant(&CapabilityId::new("builtin.apply_patch").expect("capability id"))
                .is_ok()
        );
        assert!(
            policy
                .grant(&CapabilityId::new("builtin.skill_install").expect("capability id"))
                .is_ok()
        );
        assert_trigger_grant(
            &policy,
            "builtin.trigger_create",
            &[EffectKind::DispatchCapability, EffectKind::ExternalWrite],
        );
        assert_trigger_grant(
            &policy,
            "builtin.trigger_list",
            &[EffectKind::DispatchCapability],
        );
        assert_trigger_grant(
            &policy,
            "builtin.trigger_remove",
            &[EffectKind::DispatchCapability, EffectKind::ExternalWrite],
        );

        // Trace Commons capabilities must be granted here or they vanish from
        // the model-visible tool surface in local-dev (REPL/serve) runs.
        let onboard = policy
            .grant(&CapabilityId::new("builtin.trace_commons.onboard").expect("capability id"))
            .expect("trace_commons.onboard grant");
        // onboard persists device-key material (Ed25519 keypair + policy.json),
        // so its grant carries the local filesystem read/write effects too.
        assert_eq!(
            onboard.effects,
            vec![
                EffectKind::DispatchCapability,
                EffectKind::ReadFilesystem,
                EffectKind::WriteFilesystem,
                EffectKind::Network,
                EffectKind::ExternalWrite,
            ]
        );
        assert_eq!(onboard.mounts, LocalDevMountProfile::Ambient);
        // Onboarding posts to an operator-chosen invite origin, so it needs the
        // wildcard egress profile (private/metadata IP ranges stay blocked).
        assert_eq!(onboard.network, LocalDevNetworkProfile::LocalDevWildcard);
        for capability in [
            "builtin.trace_commons.status",
            "builtin.trace_commons.credits",
        ] {
            let grant = policy
                .grant(&CapabilityId::new(capability).expect("capability id"))
                .expect("trace_commons read grant");
            assert_eq!(
                grant.effects,
                vec![EffectKind::DispatchCapability, EffectKind::ReadFilesystem]
            );
            assert_eq!(grant.mounts, LocalDevMountProfile::Ambient);
            assert_eq!(grant.network, LocalDevNetworkProfile::Default);
        }
        // builtin.profile_set writes context/profile.json under the memory mount.
        // It mirrors memory_write's effect set (read+write filesystem, memory mount,
        // default network) and must be present here or it is denied as MissingGrant.
        let builtin_profile_set = policy
            .grant(&CapabilityId::new("builtin.profile_set").expect("capability id"))
            .expect("builtin.profile_set grant must be present");
        assert_eq!(
            builtin_profile_set.effects,
            vec![
                EffectKind::DispatchCapability,
                EffectKind::ReadFilesystem,
                EffectKind::WriteFilesystem,
            ]
        );
        assert_eq!(builtin_profile_set.mounts, LocalDevMountProfile::Memory);
        assert_eq!(builtin_profile_set.network, LocalDevNetworkProfile::Default);

        // profile_token writes profile_token.jwt (0600), so its grant carries
        // WriteFilesystem; trace_commons.profile_set only reads policy + posts, so it does not.
        let profile_token = policy
            .grant(
                &CapabilityId::new("builtin.trace_commons.profile_token").expect("capability id"),
            )
            .expect("trace_commons.profile_token grant");
        assert_eq!(
            profile_token.effects,
            vec![
                EffectKind::DispatchCapability,
                EffectKind::ReadFilesystem,
                EffectKind::WriteFilesystem,
                EffectKind::Network,
                EffectKind::ExternalWrite,
            ]
        );
        assert_eq!(profile_token.mounts, LocalDevMountProfile::Ambient);
        assert_eq!(
            profile_token.network,
            LocalDevNetworkProfile::LocalDevWildcard
        );
        let profile_set = policy
            .grant(&CapabilityId::new("builtin.trace_commons.profile_set").expect("capability id"))
            .expect("trace_commons.profile_set grant");
        assert_eq!(
            profile_set.effects,
            vec![
                EffectKind::DispatchCapability,
                EffectKind::ReadFilesystem,
                EffectKind::Network,
                EffectKind::ExternalWrite,
            ]
        );
        assert_eq!(profile_set.mounts, LocalDevMountProfile::Ambient);
        assert_eq!(
            profile_set.network,
            LocalDevNetworkProfile::LocalDevWildcard
        );
    }

    #[test]
    fn network_effect_grants_use_non_empty_network_policy() {
        let policy = local_dev_capability_policy().expect("policy parses");

        for grant in &policy.grants {
            if grant.effects.contains(&EffectKind::Network) {
                assert_ne!(
                    grant.network,
                    LocalDevNetworkProfile::Default,
                    "{} declares network authority but would stage an empty network policy",
                    grant.capability
                );
            }
        }
    }

    fn assert_trigger_grant(
        policy: &LocalDevCapabilityPolicy,
        capability: &str,
        effects: &[EffectKind],
    ) {
        let grant = policy
            .grant(&CapabilityId::new(capability).expect("capability id"))
            .expect("trigger grant");
        assert_eq!(grant.effects, effects);
        assert_eq!(grant.mounts, LocalDevMountProfile::Ambient);
        assert_eq!(grant.network, LocalDevNetworkProfile::Default);
    }

    #[test]
    fn spawn_capability_approval_adds_required_spawn_effect_for_grants_without_it() {
        let policy = local_dev_capability_policy().expect("policy parses");
        let capability = CapabilityId::new("builtin.echo").expect("capability id");
        let approval = policy
            .lease_approval_for(
                LocalDevApprovalPolicyAction::SpawnCapability {
                    capability: &capability,
                },
                &MountView::default(),
                &MountView::default(),
                &MountView::default(),
                &MountView::default(),
            )
            .expect("lease approval");

        assert!(approval.allowed_effects.contains(&EffectKind::SpawnProcess));
        assert_eq!(
            approval
                .allowed_effects
                .iter()
                .filter(|effect| **effect == EffectKind::SpawnProcess)
                .count(),
            1
        );
    }

    #[test]
    fn spawn_capability_approval_does_not_duplicate_declared_spawn_effect() {
        let policy = local_dev_capability_policy().expect("policy parses");
        let capability = CapabilityId::new("builtin.shell").expect("capability id");
        let approval = policy
            .lease_approval_for(
                LocalDevApprovalPolicyAction::SpawnCapability {
                    capability: &capability,
                },
                &MountView::default(),
                &MountView::default(),
                &MountView::default(),
                &MountView::default(),
            )
            .expect("lease approval");

        assert_eq!(
            approval
                .allowed_effects
                .iter()
                .filter(|effect| **effect == EffectKind::SpawnProcess)
                .count(),
            1
        );
    }

    #[test]
    fn bundled_local_dev_capability_policy_rejects_unknown_fields() {
        let invalid = LOCAL_DEV_CAPABILITY_POLICY_TOML.replace(
            "manifest_path = \"/system/extensions/builtin/manifest.toml\"",
            "manifest_path = \"/system/extensions/builtin/manifest.toml\"\nunknown = true",
        );

        assert!(matches!(
            parse_local_dev_capability_policy(&invalid),
            Err(LocalDevCapabilityPolicyError::InvalidToml(_))
        ));
    }

    #[test]
    fn bundled_local_dev_capability_policy_rejects_invalid_capability_ids() {
        let invalid = LOCAL_DEV_CAPABILITY_POLICY_TOML
            .replace("capability = \"builtin.echo\"", "capability = \"echo\"");

        assert!(matches!(
            parse_local_dev_capability_policy(&invalid),
            Err(LocalDevCapabilityPolicyError::InvalidToml(_))
        ));
    }
}
