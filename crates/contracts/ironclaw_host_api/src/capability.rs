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
    Timestamp,
    action::{NetworkPolicy, NetworkTargetPattern},
    decision::RuntimeCredentialAuthRequirement,
    http::RuntimeCredentialTarget,
    ids::{CapabilityGrantId, CapabilityId, ExtensionId, SecretHandle, VendorId},
    invocation::InvocationOrigin,
    messaging::StandardMessagingOp,
    mount::MountView,
    resource::{ResourceCeiling, ResourceProfile},
    runtime::{RuntimeKind, TrustClass},
    scope::Principal,
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

/// Provenance-backed policy for a capability's model-visible description.
///
/// This marker does not grant execution authority. It only records whether the
/// description was supplied by a signature-verified catalog path and may
/// therefore bypass vocabulary/path/credential-shape false-positive checks.
/// Structural prompt limits still apply on every variant.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityDescriptionTrust {
    /// Unknown, local, client-supplied, or otherwise unverified provenance.
    #[default]
    Untrusted,
    /// Description came from a registry package whose catalog manifest and
    /// artifacts were signature/digest verified before installation.
    VerifiedCatalog,
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
/// Capability id of the sandboxed-process lane.
///
/// Lives here rather than beside the lane's plan types because both the kernel
/// spawn path (`ironclaw_host_runtime`) and the loop tier
/// (`ironclaw_loop_host`) compare against it, and a `loops`-layer crate must
/// not take the lane's Docker/CA dependency cone for a string constant
/// (PROPOSAL §6.6.4, CHECKLIST WS10).
pub const PROCESS_SANDBOX_CAPABILITY_ID: &str = "system.process_sandbox.run";

/// Neutral identity of the read-only extension catalog search capability.
///
/// The lifecycle manager implements this capability, but the identity is a
/// host-facing contract because product-owned prepared contexts may narrow a
/// run to discovery without depending on that manager.
pub const EXTENSION_SEARCH_CAPABILITY_ID: &str = "builtin.extension_search";

/// Stable capability identity of the model-facing tool discovery bridge.
pub const TOOL_SEARCH_CAPABILITY_ID: &str = "ironclaw.tool_search";

/// Stable capability identity of the model-facing tool schema bridge.
pub const TOOL_DESCRIBE_CAPABILITY_ID: &str = "ironclaw.tool_describe";

/// Stable capability identity of the model-facing tool dispatch bridge.
pub const TOOL_CALL_CAPABILITY_ID: &str = "ironclaw.tool_call";

pub const UNGATED_LOOP_RUN_CAPABILITIES: &[&str] = &[
    "builtin.echo",
    "builtin.time",
    "builtin.json",
    "builtin.trace_commons.status",
    "builtin.trace_commons.credits",
    "builtin.trace_commons.onboard",
    // The bound memory provider's profile tool (formerly builtin-declared,
    // #3537 lifecycle rework): a private local write to the user's own agent
    // context, grandfathered with the same reviewed posture under its stable
    // provider-declared id.
    "ironclaw.memory.profile_set",
    // The memory tools moved from the builtin package (`builtin.memory_*`) to
    // the always-on `ironclaw.memory` package (#3537); same tools, same
    // reviewed read-only posture, renamed ids. `ironclaw.memory.write` stays
    // off this list (gated, arbitrary-path write).
    "ironclaw.memory.search",
    "ironclaw.memory.read",
    "ironclaw.memory.tree",
    "builtin.read_file",
    "builtin.list_dir",
    "builtin.glob",
    "builtin.grep",
    "builtin.skill_list",
    "builtin.trigger_list",
    EXTENSION_SEARCH_CAPABILITY_ID,
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

    /// Clamp a REQUESTED matrix for a memory-provider tool: `Ungated` is a
    /// reviewed host grant, not a manifest request. `loop_run` keeps `Ungated`
    /// only when `id` is in the reviewed
    /// [`UNGATED_LOOP_RUN_CAPABILITIES`] allowlist; any other `Ungated` cell —
    /// including `product`/`automation`, which have no reviewed Ungated
    /// allowlist at all — falls to [`OriginGatePolicy::GatedUnlessGranted`].
    /// Every non-`Ungated` policy passes through unchanged, so a provider can
    /// only ever request LESS gating than it gets, never less than the host
    /// grants.
    pub fn clamp_requested_for_memory_tool(mut self, id: &str) -> Self {
        if self.loop_run == OriginGatePolicy::Ungated
            && !UNGATED_LOOP_RUN_CAPABILITIES.contains(&id)
        {
            self.loop_run = OriginGatePolicy::GatedUnlessGranted;
        }
        if self.product == OriginGatePolicy::Ungated {
            self.product = OriginGatePolicy::GatedUnlessGranted;
        }
        if self.automation == OriginGatePolicy::Ungated {
            self.automation = OriginGatePolicy::GatedUnlessGranted;
        }
        self
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
    /// The standard messaging operation this descriptor is bound to (manifest
    /// v3 `standard_op`; mirrors `CapabilityDeclV2.standard_op`), or `None`
    /// for a bespoke capability. `#[serde(default)]` so descriptors persisted
    /// or serialized before this field existed rehydrate to `None`.
    #[serde(default)]
    pub standard_op: Option<StandardMessagingOp>,
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
    /// Linked device: the user authorizes a long-lived session on the vendor's
    /// own client (a login code delivered in-app, plus a second factor where
    /// the account has one), and the host takes custody of the resulting
    /// session material. Distinct from every neighbour here — there is no
    /// authorization server and no bearer token (`OAuth`), the user never
    /// hands over a pre-existing secret (`ManualToken`), and a credential
    /// account IS minted (`Pairing`). Satisfaction is the stored session, so
    /// revocation is host-side deletion plus a vendor-side session logout.
    DeviceLink,
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
mod capability_descriptor_runtime_kind_tests {
    use super::{CapabilityDescriptor, PermissionMode};
    use crate::{
        ids::{CapabilityId, ExtensionId},
        runtime::{RuntimeKind, TrustClass},
    };

    fn descriptor() -> CapabilityDescriptor {
        CapabilityDescriptor {
            id: CapabilityId::new("cap.example").expect("valid capability id"),
            provider: ExtensionId::new("acme").expect("valid extension id"),
            runtime: RuntimeKind::Wasm,
            trust_ceiling: TrustClass::Sandbox,
            description: "test".to_string(),
            parameters_schema: serde_json::json!({}),
            effects: vec![],
            default_permission: PermissionMode::Ask,
            standard_op: None,
            runtime_credentials: vec![],
            network_targets: vec![],
            max_egress_bytes: None,
            resource_profile: None,
            origin_gate_matrix: None,
        }
    }

    // This is the actual reachable attack path the module docs on
    // `RuntimeKind` (runtime.rs) warn about: a third-party manifest is parsed
    // into a `CapabilityDescriptor` with plain (untrusted) `Deserialize`, so a
    // manifest declaring `"runtime": "first_party" | "system" | "sandbox"`
    // must fail to parse rather than silently minting a privileged capability.
    #[test]
    fn manifest_deserialize_rejects_every_privileged_runtime_kind() {
        for privileged in ["first_party", "system", "sandbox"] {
            let mut wire = serde_json::to_value(descriptor()).expect("descriptor serializes");
            wire["runtime"] = serde_json::Value::String(privileged.to_string());
            assert!(
                serde_json::from_value::<CapabilityDescriptor>(wire).is_err(),
                "untrusted manifest must not be able to declare runtime = {privileged}"
            );
        }

        // Sanity: non-privileged runtime kinds are unaffected.
        let mut wire = serde_json::to_value(descriptor()).expect("descriptor serializes");
        wire["runtime"] = serde_json::Value::String("mcp".to_string());
        assert!(serde_json::from_value::<CapabilityDescriptor>(wire).is_ok());
    }
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

    /// The linked-device setup kind round-trips as `device_link` in both
    /// directions. It is persisted inside `TurnRunRecord.credential_requirements`
    /// for a run parked on a device-link gate, so the token is a durable wire
    /// contract, not an internal name.
    #[test]
    fn device_link_setup_round_trips_on_the_wire() {
        assert_eq!(
            serde_json::to_value(RuntimeCredentialAccountSetup::DeviceLink).expect("serializes"),
            serde_json::json!({"kind": "device_link"}),
            "the device-link setup's persisted wire shape is locked"
        );

        let parsed: RuntimeCredentialAccountSetup =
            serde_json::from_str(r#"{"kind":"device_link"}"#).expect("device_link");
        assert_eq!(parsed, RuntimeCredentialAccountSetup::DeviceLink);
    }

    /// Adding `DeviceLink` must not turn the `#[serde(other)]` catch-all into
    /// a match on known kinds: a kind a *future* binary introduces still has
    /// to fold to `Retired` rather than fail the whole turn-state snapshot.
    /// Pinned separately from the legacy case above because that one tests a
    /// kind we retired, and this one tests a kind we have not invented yet —
    /// the direction that breaks when someone "tightens" the enum.
    #[test]
    fn unknown_future_setup_kind_still_folds_to_retired() {
        for wire in [
            r#"{"kind":"device_link_v2"}"#,
            r#"{"kind":"passkey","challenge":"abc"}"#,
            r#"{"kind":"linked_device"}"#,
        ] {
            let parsed: RuntimeCredentialAccountSetup =
                serde_json::from_str(wire).expect("an unknown setup kind must stay loadable");
            assert_eq!(
                parsed,
                RuntimeCredentialAccountSetup::Retired,
                "{wire} must fold to Retired"
            );
        }
    }
}

#[cfg(test)]
mod origin_gate_wire_tests {
    use super::{OriginGateMatrix, OriginGatePolicy, UNGATED_LOOP_RUN_CAPABILITIES};
    use crate::{
        ids::{CapabilityId, ProductKind, RoutineId, RunId},
        invocation::InvocationOrigin,
    };

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

    /// A memory provider's requested matrix is clamped: off-allowlist
    /// `Ungated` (a write tool, or any Product/Automation cell) falls to
    /// `GatedUnlessGranted`; non-`Ungated` requests pass through unchanged.
    #[test]
    fn clamp_requested_for_memory_tool_downgrades_off_allowlist_ungated() {
        let clamped = OriginGateMatrix {
            loop_run: OriginGatePolicy::Ungated,
            product: OriginGatePolicy::Ungated,
            automation: OriginGatePolicy::Ungated,
        }
        .clamp_requested_for_memory_tool("ironclaw.memory.write");
        assert_eq!(clamped.loop_run, OriginGatePolicy::GatedUnlessGranted);
        assert_eq!(clamped.product, OriginGatePolicy::GatedUnlessGranted);
        assert_eq!(clamped.automation, OriginGatePolicy::GatedUnlessGranted);
    }

    /// The reviewed allowlist still grants `Ungated` loop_run to the read-only
    /// memory tools, and declared non-`Ungated` cells are never rewritten.
    #[test]
    fn clamp_requested_for_memory_tool_keeps_allowlisted_and_gated_cells() {
        let kept = OriginGateMatrix {
            loop_run: OriginGatePolicy::Ungated,
            product: OriginGatePolicy::Forbidden,
            automation: OriginGatePolicy::Forbidden,
        }
        .clamp_requested_for_memory_tool("ironclaw.memory.search");
        assert_eq!(kept.loop_run, OriginGatePolicy::Ungated);
        assert_eq!(kept.product, OriginGatePolicy::Forbidden);
        assert_eq!(kept.automation, OriginGatePolicy::Forbidden);

        let gated = OriginGateMatrix {
            loop_run: OriginGatePolicy::GatedUnlessGranted,
            product: OriginGatePolicy::Forbidden,
            automation: OriginGatePolicy::Forbidden,
        }
        .clamp_requested_for_memory_tool("ironclaw.memory.write");
        assert_eq!(gated.loop_run, OriginGatePolicy::GatedUnlessGranted);
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

#[cfg(test)]
mod process_sandbox_capability_id_tests {
    use super::PROCESS_SANDBOX_CAPABILITY_ID;
    use crate::ids::CapabilityId;

    /// The constant is compared as a `&str` on two gating paths — the kernel
    /// spawn check (`ironclaw_host_runtime::production`) and the process
    /// executor's routing check — where a malformed literal would not fail
    /// loudly: `capability_id.as_str() == LITERAL` would simply never match, so
    /// sandbox plans would silently stop being recognised. `CapabilityId::new`
    /// is fallible and cannot be evaluated in a `const`, so pin it here instead:
    /// this is the check that makes the literal a valid capability id.
    #[test]
    fn process_sandbox_capability_id_literal_is_a_valid_capability_id() {
        let parsed = CapabilityId::new(PROCESS_SANDBOX_CAPABILITY_ID)
            .expect("PROCESS_SANDBOX_CAPABILITY_ID must be a valid CapabilityId");
        assert_eq!(parsed.as_str(), PROCESS_SANDBOX_CAPABILITY_ID);
    }
}

#[cfg(test)]
mod tool_discovery_capability_id_tests {
    use super::{
        EXTENSION_SEARCH_CAPABILITY_ID, TOOL_CALL_CAPABILITY_ID, TOOL_DESCRIBE_CAPABILITY_ID,
        TOOL_SEARCH_CAPABILITY_ID,
    };
    use crate::ids::CapabilityId;

    #[test]
    fn tool_discovery_capability_id_literals_are_valid_and_stable() {
        for (id, expected) in [
            (EXTENSION_SEARCH_CAPABILITY_ID, "builtin.extension_search"),
            (TOOL_SEARCH_CAPABILITY_ID, "ironclaw.tool_search"),
            (TOOL_DESCRIBE_CAPABILITY_ID, "ironclaw.tool_describe"),
            (TOOL_CALL_CAPABILITY_ID, "ironclaw.tool_call"),
        ] {
            let parsed = CapabilityId::new(id)
                .expect("tool discovery capability ids must be valid CapabilityIds");
            assert_eq!(parsed.as_str(), expected);
        }
    }
}
