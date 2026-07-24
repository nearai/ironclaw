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
    CapabilityGrantId, CapabilityId, ExtensionId, InvocationOrigin, MountView, NetworkPolicy,
    NetworkTargetPattern, Principal, ResourceCeiling, ResourceProfile,
    RuntimeCredentialAuthRequirement, RuntimeCredentialTarget, RuntimeKind, SecretHandle,
    Timestamp, TrustClass, VendorId,
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

/// Per-origin gate requirement (§5.2.1). Absence of a declaration for an
/// origin means `Forbidden` (deny-by-default).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum OriginGatePolicy {
    /// This origin may not invoke the capability at all.
    #[default]
    Forbidden,
    /// Every invocation gates; persistent grants are never honored (§5.2.7).
    AskAlways,
    /// Gates unless a scoped persistent/policy grant covers it (§5.2.7).
    GatedUnlessGranted,
    /// The origin's own gesture is the consent evidence (`Product` only).
    ConsentSufficient,
    /// No approval gate — for `LoopRun` requires a reviewed allowlist entry (§10).
    Ungated,
}

/// The per-origin gate matrix declared on a capability descriptor (§5.2.1).
/// Each origin defaults to [`OriginGatePolicy::Forbidden`] when the declaration
/// omits it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct OriginGateMatrix {
    #[serde(default)]
    pub loop_run: OriginGatePolicy,
    #[serde(default)]
    pub product: OriginGatePolicy,
    #[serde(default)]
    pub automation: OriginGatePolicy,
}

/// §5.2.1/§10 — capabilities the model (LoopRun) may invoke UNGATED.
/// Behavior-preserving seed (grandfathered: these are ungated under today's
/// `AskDestructive` effect gate, i.e. their effects are a subset of
/// `{read_filesystem, dispatch_capability}` or they are exempt from approval).
/// Additions require security review (S5 ratchet).
pub const UNGATED_LOOP_RUN_CAPABILITIES: &[&str] = &[
    "builtin.echo",
    "builtin.time",
    "builtin.json",
    "builtin.trace_commons.status",
    "builtin.trace_commons.credits",
    "builtin.trace_commons.onboard",
    "builtin.profile_set",
    "builtin.memory_search",
    "builtin.memory_read",
    "builtin.memory_tree",
    "builtin.read_file",
    "builtin.list_dir",
    "builtin.glob",
    "builtin.grep",
    "builtin.skill_list",
    "builtin.trigger_list",
    "builtin.extension_search",
];

impl OriginGateMatrix {
    /// The gate policy this matrix declares for the given invocation origin.
    /// Maps each [`InvocationOrigin`] variant to its matching matrix field;
    /// an omitted field is [`OriginGatePolicy::Forbidden`] by default.
    pub fn policy_for(&self, origin: &InvocationOrigin) -> OriginGatePolicy {
        match origin {
            InvocationOrigin::LoopRun(_) | InvocationOrigin::ScheduledLoopRun(_) => self.loop_run,
            InvocationOrigin::Product(_) => self.product,
            InvocationOrigin::Automation(_) => self.automation,
        }
    }

    /// Behavior-preserving per-capability matrix seed for a first-party builtin
    /// capability (§5.3 S3). `LoopRun` is [`OriginGatePolicy::Ungated`] exactly
    /// when `id` is in the reviewed [`UNGATED_LOOP_RUN_CAPABILITIES`] allowlist,
    /// otherwise [`OriginGatePolicy::GatedUnlessGranted`] (every non-allowlisted
    /// builtin is GATED under today's effect gate, so this mirrors current
    /// behavior). `Product` and `Automation` are deny-by-default
    /// ([`OriginGatePolicy::Forbidden`]) until a later reviewed ingress slice
    /// declares a live producer for them.
    pub fn builtin_loop_run_seed(id: &str) -> Self {
        let loop_run = if UNGATED_LOOP_RUN_CAPABILITIES.contains(&id) {
            OriginGatePolicy::Ungated
        } else {
            OriginGatePolicy::GatedUnlessGranted
        };
        Self {
            loop_run,
            product: OriginGatePolicy::Forbidden,
            automation: OriginGatePolicy::Forbidden,
        }
    }

    /// Product-origin-only matrix for first-party product API capabilities.
    pub fn product_consent_only() -> Self {
        Self {
            loop_run: OriginGatePolicy::Forbidden,
            product: OriginGatePolicy::ConsentSufficient,
            automation: OriginGatePolicy::Forbidden,
        }
    }
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
    /// Optional per-capability egress cap (bytes) applied to the minted
    /// `NetworkPolicy.max_egress_bytes`. Manifest-declared (v3 tool
    /// `max_egress_bytes`); `#[serde(default)]` so existing manifests/records
    /// parse to `None` (no cap). This lets a networked capability bound its
    /// egress from the manifest instead of a composition special-case.
    #[serde(default)]
    pub max_egress_bytes: Option<u64>,
    pub resource_profile: Option<ResourceProfile>,
    /// Per-origin gate matrix (§5.2.1). `None` = undeclared: treated as
    /// all-`Forbidden` (fail-closed) at authorization, and flagged by the
    /// §5 architecture ratchet (a later slice) which requires every descriptor
    /// to declare one. Populated per capability in a later slice.
    #[serde(default)]
    pub origin_gate_matrix: Option<OriginGateMatrix>,
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
        provider: VendorId,
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
    /// host-issued code on the external side (e.g. a messenger deep-link
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

#[cfg(test)]
mod origin_gate_wire_tests {
    use super::{OriginGateMatrix, OriginGatePolicy, UNGATED_LOOP_RUN_CAPABILITIES};
    use crate::{CapabilityId, InvocationOrigin, ProductKind, RoutineId, RunId};

    /// The checked-in Ungated-for-LoopRun allowlist seed (§5.2.1/§10) must be
    /// internally consistent: non-empty, free of duplicates, and every entry a
    /// well-formed capability id. The full "every descriptor matches the
    /// allowlist" ratchet is a later slice (S5); this locks the seed itself.
    #[test]
    fn ungated_loop_run_allowlist_is_internally_consistent() {
        assert!(
            !UNGATED_LOOP_RUN_CAPABILITIES.is_empty(),
            "the allowlist seed must not be empty"
        );
        let mut seen = std::collections::HashSet::new();
        for id in UNGATED_LOOP_RUN_CAPABILITIES {
            assert!(
                seen.insert(*id),
                "duplicate id in UNGATED_LOOP_RUN_CAPABILITIES: {id}"
            );
            CapabilityId::new(*id).unwrap_or_else(|_| {
                panic!("allowlist id {id} must be a well-formed capability id")
            });
        }
    }

    /// `builtin_loop_run_seed` mirrors today's effect gate: an allowlisted id is
    /// `Ungated` for `LoopRun`, everything else `GatedUnlessGranted`, and
    /// `Product`/`Automation` are always deny-by-default (`Forbidden`).
    #[test]
    fn builtin_loop_run_seed_mirrors_allowlist_membership() {
        for id in UNGATED_LOOP_RUN_CAPABILITIES {
            let matrix = OriginGateMatrix::builtin_loop_run_seed(id);
            assert_eq!(matrix.loop_run, OriginGatePolicy::Ungated, "{id}");
            assert_eq!(matrix.product, OriginGatePolicy::Forbidden, "{id}");
            assert_eq!(matrix.automation, OriginGatePolicy::Forbidden, "{id}");
        }
        let gated = OriginGateMatrix::builtin_loop_run_seed("builtin.write_file");
        assert_eq!(gated.loop_run, OriginGatePolicy::GatedUnlessGranted);
        assert_eq!(gated.product, OriginGatePolicy::Forbidden);
        assert_eq!(gated.automation, OriginGatePolicy::Forbidden);
    }

    /// `OriginGatePolicy` is a wire-stable enum: every variant must serialize to
    /// its snake_case tag and round-trip back. (§5.2.1)
    #[test]
    fn origin_gate_policy_is_snake_case_and_roundtrips() {
        for (policy, wire) in [
            (OriginGatePolicy::Forbidden, "forbidden"),
            (OriginGatePolicy::AskAlways, "ask_always"),
            (OriginGatePolicy::GatedUnlessGranted, "gated_unless_granted"),
            (OriginGatePolicy::ConsentSufficient, "consent_sufficient"),
            (OriginGatePolicy::Ungated, "ungated"),
        ] {
            let json = serde_json::to_value(policy).expect("serializes");
            assert_eq!(json, serde_json::json!(wire));
            let back: OriginGatePolicy = serde_json::from_value(json).expect("roundtrips");
            assert_eq!(back, policy);
        }
        // Deny-by-default: the enum's `Default` is `Forbidden`.
        assert_eq!(OriginGatePolicy::default(), OriginGatePolicy::Forbidden);
    }

    /// An omitted per-origin field defaults to `Forbidden` (deny-by-default),
    /// so a partial matrix is fully specified with the rest closed.
    #[test]
    fn origin_gate_matrix_omitted_field_defaults_to_forbidden() {
        // Only `loop_run` and `product` declared; `automation` omitted.
        let matrix: OriginGateMatrix = serde_json::from_value(serde_json::json!({
            "loop_run": "gated_unless_granted",
            "product": "consent_sufficient",
        }))
        .expect("partial matrix parses");
        assert_eq!(matrix.loop_run, OriginGatePolicy::GatedUnlessGranted);
        assert_eq!(matrix.product, OriginGatePolicy::ConsentSufficient);
        assert_eq!(
            matrix.automation,
            OriginGatePolicy::Forbidden,
            "an omitted origin is deny-by-default"
        );

        // A fully empty matrix is all-Forbidden.
        let empty: OriginGateMatrix =
            serde_json::from_value(serde_json::json!({})).expect("empty matrix parses");
        assert_eq!(empty, OriginGateMatrix::default());
    }

    /// `policy_for` selects the field matching the origin's variant.
    #[test]
    fn policy_for_maps_each_origin_variant_to_its_field() {
        let matrix = OriginGateMatrix {
            loop_run: OriginGatePolicy::AskAlways,
            product: OriginGatePolicy::ConsentSufficient,
            automation: OriginGatePolicy::GatedUnlessGranted,
        };
        assert_eq!(
            matrix.policy_for(&InvocationOrigin::LoopRun(RunId::new())),
            OriginGatePolicy::AskAlways
        );
        assert_eq!(
            matrix.policy_for(&InvocationOrigin::Product(
                ProductKind::new("settings").unwrap()
            )),
            OriginGatePolicy::ConsentSufficient
        );
        assert_eq!(
            matrix.policy_for(&InvocationOrigin::Automation(
                RoutineId::new("heartbeat").unwrap()
            )),
            OriginGatePolicy::GatedUnlessGranted
        );
    }

    #[test]
    fn product_consent_only_is_for_product_api_capabilities() {
        let matrix = OriginGateMatrix::product_consent_only();
        assert_eq!(matrix.loop_run, OriginGatePolicy::Forbidden);
        assert_eq!(matrix.product, OriginGatePolicy::ConsentSufficient);
        assert_eq!(matrix.automation, OriginGatePolicy::Forbidden);
    }
}
