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
    CapabilityGrantId, CapabilityId, ExtensionId, MountView, NetworkPolicy, NetworkTargetPattern,
    Principal, ResourceCeiling, ResourceProfile, RuntimeCredentialAccountProviderId,
    RuntimeCredentialAuthRequirement, RuntimeCredentialTarget, RuntimeKind, SecretHandle,
    Timestamp, TrustClass,
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

impl EffectKind {
    pub fn is_write(self) -> bool {
        match self {
            Self::ReadFilesystem | Self::Network | Self::UseSecret | Self::DispatchCapability => {
                false
            }
            Self::WriteFilesystem
            | Self::DeleteFilesystem
            | Self::ExecuteCode
            | Self::SpawnProcess
            | Self::ModifyExtension
            | Self::ModifyApproval
            | Self::ModifyBudget
            | Self::ExternalWrite
            | Self::Financial => true,
        }
    }
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
    pub runtime_credentials: Vec<RuntimeCredentialRequirement>,
    /// Declared network egress allowlist for this capability, independent of any
    /// runtime credential. This lets a keyless-but-networked tool (one that
    /// declares the `Network` effect but injects no secret) populate its
    /// `ApplyNetworkPolicy` allowlist directly from the manifest. Credential
    /// `audience`s are folded in on top of these at grant issuance.
    #[serde(default)]
    pub network_targets: Vec<NetworkTargetPattern>,
    pub resource_profile: Option<ResourceProfile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeCredentialRequirement {
    pub handle: SecretHandle,
    #[serde(default)]
    pub source: RuntimeCredentialRequirementSource,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub provider_scopes: Vec<String>,
    pub audience: NetworkTargetPattern,
    pub target: RuntimeCredentialTarget,
    pub required: bool,
}

impl RuntimeCredentialRequirement {
    pub fn product_auth_requirement_for(
        &self,
        requester_extension: ExtensionId,
    ) -> Option<RuntimeCredentialAuthRequirement> {
        let RuntimeCredentialRequirementSource::ProductAuthAccount { provider, setup } =
            &self.source
        else {
            return None;
        };
        Some(RuntimeCredentialAuthRequirement {
            provider: provider.clone(),
            setup: setup.clone(),
            requester_extension,
            provider_scopes: self.provider_scopes.clone(),
        })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum RuntimeCredentialRequirementSource {
    #[default]
    SecretHandle,
    ProductAuthAccount {
        provider: RuntimeCredentialAccountProviderId,
        #[serde(default)]
        setup: RuntimeCredentialAccountSetup,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RuntimeCredentialAccountSetup {
    #[default]
    ManualToken,
    #[serde(rename = "oauth")]
    OAuth { scopes: Vec<String> },
    /// Channel pairing: the user links an external account by consuming a
    /// host-issued code on the external side (e.g. Telegram deep-link
    /// `/start <code>`). No credential account is minted — satisfaction is
    /// re-derived from the channel's binding store when the parked run
    /// re-checks its requirements. Unlike the retired Slack `channel_pairing`
    /// connect gate, this variant is host-issued-code, provider-keyed, and
    /// serviced by the standard auth-continuation fan-out.
    Pairing,
    /// Setup kinds this enum no longer models but persisted records may still
    /// carry — e.g. the pre-OAuth `channel_pairing` Slack connect gate removed
    /// by #5604, which was serialized inside `TurnRunRecord.credential_requirements`
    /// for runs parked on the connect gate. Turn-state snapshot decoding is
    /// all-or-nothing, so an unrecognized kind must fold here instead of
    /// making every thread's turn state unloadable. Carriers treat a retired
    /// setup as not-serviceable (no challenge can be produced for it).
    #[serde(other)]
    Retired,
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

#[cfg(test)]
mod credential_setup_wire_tests {
    use super::RuntimeCredentialAccountSetup;

    /// Persisted `TurnRunRecord.credential_requirements` may still carry setup
    /// kinds this enum no longer models (the pre-OAuth `channel_pairing` Slack
    /// connect gate, removed by #5604). Snapshot decoding is all-or-nothing,
    /// so an unrecognized kind must fold into [`RuntimeCredentialAccountSetup::Retired`]
    /// instead of failing the whole turn-state snapshot.
    #[test]
    fn legacy_channel_pairing_setup_still_deserializes() {
        let parsed: RuntimeCredentialAccountSetup =
            serde_json::from_str(r#"{"kind":"channel_pairing","channel":"slack"}"#)
                .expect("legacy persisted setup kind must stay loadable");
        assert_eq!(parsed, RuntimeCredentialAccountSetup::Retired);

        let parsed: RuntimeCredentialAccountSetup =
            serde_json::from_str(r#"{"kind":"some_future_kind"}"#)
                .expect("unknown setup kinds must stay loadable");
        assert_eq!(parsed, RuntimeCredentialAccountSetup::Retired);

        // Current kinds keep their exact wire shape.
        let parsed: RuntimeCredentialAccountSetup =
            serde_json::from_str(r#"{"kind":"oauth","scopes":["users:read"]}"#).expect("oauth");
        assert_eq!(
            parsed,
            RuntimeCredentialAccountSetup::OAuth {
                scopes: vec!["users:read".to_string()]
            }
        );

        let parsed: RuntimeCredentialAccountSetup =
            serde_json::from_str(r#"{"kind":"pairing"}"#).expect("pairing");
        assert_eq!(parsed, RuntimeCredentialAccountSetup::Pairing);
        assert_eq!(
            serde_json::to_value(RuntimeCredentialAccountSetup::Pairing).expect("serializes"),
            serde_json::json!({"kind": "pairing"}),
            "the pairing gate's persisted wire shape is locked"
        );
    }
}
