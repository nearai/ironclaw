//! Capability declaration and grant contracts.
//!
//! A [`CapabilityDescriptor`] says what an extension can provide; it does not
//! grant anyone authority to use it. Authority comes from active
//! [`CapabilityGrant`] values collected in a [`CapabilitySet`]. Grants carry
//! constraints for effects, mounts, network access, secrets, resources, expiry,
//! and invocation count so delegated authority can be attenuated across spawned
//! work.

use serde::{Deserialize, Serialize};

use crate::{
    CapabilityGrantId, CapabilityId, ExtensionId, MountView, NetworkPolicy, Principal,
    ResourceCeiling, ResourceProfile, RuntimeKind, SecretHandle, Timestamp, TrustClass,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectKind {
    ReadFilesystem,
    WriteFilesystem,
    DeleteFilesystem,
    Network,
    UseSecret,
    ExecuteCode,
    SpawnProcess,
    DispatchCapability,
    ModifyExtension,
    ModifyApproval,
    ModifyBudget,
    ExternalWrite,
    Financial,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionMode {
    Allow,
    Ask,
    Deny,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CapabilityDescriptor {
    pub id: CapabilityId,
    pub provider: ExtensionId,
    pub runtime: RuntimeKind,
    pub trust_ceiling: TrustClass,
    pub description: String,
    pub parameters_schema: serde_json::Value,
    pub effects: Vec<EffectKind>,
    pub default_permission: PermissionMode,
    pub resource_profile: Option<ResourceProfile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityGrant {
    pub id: CapabilityGrantId,
    pub capability: CapabilityId,
    pub grantee: Principal,
    pub issued_by: Principal,
    pub constraints: GrantConstraints,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilitySet {
    pub grants: Vec<CapabilityGrant>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GrantConstraints {
    pub allowed_effects: Vec<EffectKind>,
    pub mounts: MountView,
    pub network: NetworkPolicy,
    pub secrets: Vec<SecretHandle>,
    pub resource_ceiling: Option<ResourceCeiling>,
    pub expires_at: Option<Timestamp>,
    pub max_invocations: Option<u64>,
}
