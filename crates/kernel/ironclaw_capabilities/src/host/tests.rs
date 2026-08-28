//! Unit tests for the capability host.
//!
//! Moved verbatim from the pre-split `host.rs`; the only change is the
//! de-indent that comes with becoming a file module.

use ironclaw_host_api::{
    capability::RuntimeCredentialAccountSetup,
    decision::{Decision, Obligation},
    dispatch::{CapabilityDispatchResult, DispatchAuthRequirement},
    ids::{CapabilityId, ExtensionId, SecretHandle, VendorId},
    invocation::Actor,
    lane::RuntimeLane,
};
use ironclaw_processes::{ProcessInvocationError, ProcessInvocationStart, ProcessInvocationStatus};
use ironclaw_trust::TrustDecision;

use super::authorize::{WITNESS_DEFAULT_TTL, witness_deadline};
use super::error_mapping::enrich_dispatch_error_credential_requirements;
use super::*;
use crate::ports::CredentialPresence;

fn auth_required_empty(cap: &str) -> DispatchError {
    DispatchError::AuthRequired {
        capability: CapabilityId::new(cap).unwrap(),
        requirement: Box::new(DispatchAuthRequirement {
            required_secrets: Vec::new(),
            credential_requirements: Vec::new(),
            model_visible_cause: None,
        }),
    }
}

fn auth_required_with_secrets(cap: &str) -> DispatchError {
    DispatchError::AuthRequired {
        capability: CapabilityId::new(cap).unwrap(),
        requirement: Box::new(DispatchAuthRequirement {
            required_secrets: vec![SecretHandle::new("raw_secret").unwrap()],
            credential_requirements: Vec::new(),
            model_visible_cause: None,
        }),
    }
}

fn auth_required_with_provider(cap: &str, provider: &str) -> DispatchError {
    use ironclaw_host_api::decision::RuntimeCredentialAuthRequirement;
    DispatchError::AuthRequired {
        capability: CapabilityId::new(cap).unwrap(),
        requirement: Box::new(DispatchAuthRequirement {
            required_secrets: Vec::new(),
            credential_requirements: vec![RuntimeCredentialAuthRequirement {
                provider: VendorId::new(provider).unwrap(),
                setup: RuntimeCredentialAccountSetup::ManualToken,
                requester_extension: ExtensionId::new(provider).unwrap(),
                provider_scopes: Vec::new(),
            }],
            model_visible_cause: None,
        }),
    }
}

fn inject_credential_obligation(provider: &str) -> Obligation {
    Obligation::InjectCredentialAccountOnce {
        handle: SecretHandle::new(format!("{provider}_pat")).unwrap(),
        provider: VendorId::new(provider).unwrap(),
        setup: RuntimeCredentialAccountSetup::ManualToken,
        provider_scopes: Vec::new(),
        requester_extension: ExtensionId::new(provider).unwrap(),
    }
}

// WASM case: both empty + exactly one obligation → enriched with that provider.
#[test]
fn enrich_fills_empty_from_single_credential_obligation() {
    let error = auth_required_empty("echo.say");
    let obligations = [inject_credential_obligation("github")];

    let result = enrich_dispatch_error_credential_requirements(error, &obligations);

    let DispatchError::AuthRequired { requirement, .. } = result else {
        panic!("expected AuthRequired");
    };
    assert_eq!(requirement.credential_requirements.len(), 1);
    assert_eq!(
        requirement.credential_requirements[0].provider,
        VendorId::new("github").unwrap()
    );
}

// required_secrets populated → returned unchanged (raw-secret gate must not become product-auth prompt).
#[test]
fn enrich_leaves_required_secrets_populated_unchanged() {
    let error = auth_required_with_secrets("echo.say");
    let obligations = [inject_credential_obligation("github")];

    let result = enrich_dispatch_error_credential_requirements(error, &obligations);

    let DispatchError::AuthRequired { requirement, .. } = result else {
        panic!("expected AuthRequired");
    };
    assert_eq!(
        requirement.required_secrets.len(),
        1,
        "required_secrets must be preserved"
    );
    assert!(
        requirement.credential_requirements.is_empty(),
        "credential_requirements must remain empty when required_secrets are present"
    );
}

// credential_requirements already populated → returned unchanged (e.g. MCP runtime already supplied requirements).
#[test]
fn enrich_leaves_non_empty_credential_requirements_unchanged() {
    let error = auth_required_with_provider("echo.say", "mcp_provider");
    let obligations = [inject_credential_obligation("github")];

    let result = enrich_dispatch_error_credential_requirements(error, &obligations);

    let DispatchError::AuthRequired { requirement, .. } = result else {
        panic!("expected AuthRequired");
    };
    assert_eq!(requirement.credential_requirements.len(), 1);
    assert_eq!(
        requirement.credential_requirements[0].provider,
        VendorId::new("mcp_provider").unwrap(),
        "original mcp_provider must be retained, not replaced by github"
    );
}

// ZERO credential obligations → unchanged (empty gate, not a guess). This is
// the preflight-shaped signal (no declared credential obligation on file at
// all), which must remain a stable auth gate rather than fall into the
// "ambiguous attribution" typed failure reserved for >1 obligations.
#[test]
fn enrich_leaves_unchanged_when_zero_credential_obligations() {
    let error = auth_required_empty("echo.say");
    let obligations: [Obligation; 0] = [];

    let result = enrich_dispatch_error_credential_requirements(error, &obligations);

    let DispatchError::AuthRequired { requirement, .. } = result else {
        panic!("expected AuthRequired");
    };
    assert!(
        requirement.credential_requirements.is_empty(),
        "zero obligations must leave credential_requirements empty"
    );
}

// TWO credential obligations → typed failure; the host does not guess.
#[test]
fn enrich_fails_without_gate_when_two_credential_obligations() {
    let error = auth_required_empty("echo.say");
    let obligations = [
        inject_credential_obligation("github"),
        inject_credential_obligation("gitlab"),
    ];

    let result = enrich_dispatch_error_credential_requirements(error, &obligations);

    assert!(matches!(
        result,
        DispatchError::Rejected {
            kind: ironclaw_host_api::dispatch::DispatchFailureKind::Runtime(
                ironclaw_host_api::dispatch::RuntimeDispatchErrorKind::SecretDenied
            ),
            ..
        }
    ));
}

// Non-AuthRequired variants returned unchanged.
#[test]
fn enrich_is_noop_for_non_auth_required_variants() {
    let error = DispatchError::UnknownCapability {
        capability: CapabilityId::new("echo.say").unwrap(),
    };
    let obligations = [inject_credential_obligation("github")];

    let result = enrich_dispatch_error_credential_requirements(error, &obligations);

    assert!(
        matches!(result, DispatchError::UnknownCapability { .. }),
        "non-AuthRequired variants must be returned unchanged"
    );
}

// --- Slice-C `authorize()` fold ---

// Unconditionally allows with no obligations, so the fold reaches the seal.
struct AllowAuthorizer;

#[async_trait::async_trait]
impl ironclaw_authorization::TrustAwareCapabilityDispatchAuthorizer for AllowAuthorizer {
    async fn authorize_dispatch_with_trust(
        &self,
        _context: &ExecutionContext,
        _descriptor: &CapabilityDescriptor,
        _estimate: &ResourceEstimate,
        _trust_decision: &TrustDecision,
    ) -> Decision {
        Decision::Allow {
            obligations: ironclaw_host_api::decision::Obligations::empty(),
        }
    }
}

// Permissive policy-facts double: credential pre-flight always satisfied and
// no persistent grants, so the in-fold credential check never fires.
struct SatisfiedPolicyFacts;

#[async_trait::async_trait]
impl HostPolicyFacts for SatisfiedPolicyFacts {
    async fn credential_presence(
        &self,
        _descriptor: &CapabilityDescriptor,
        _scope: &ResourceScope,
    ) -> CredentialPresence {
        CredentialPresence::Satisfied
    }

    async fn persistent_grants(
        &self,
        _capability_id: &CapabilityId,
        _context: &ExecutionContext,
        _action: crate::ports::PolicyAction,
    ) -> Vec<ironclaw_host_api::capability::CapabilityGrant> {
        Vec::new()
    }
}

// Returns a single persistent grant carrying `expiry`. With `AllowAuthorizer`
// the persistent-approval probe adopts it, so its `expires_at` becomes the
// witness's shortest-lived frozen fact.
struct GrantWithExpiryPolicyFacts {
    expiry: Timestamp,
}

#[async_trait::async_trait]
impl HostPolicyFacts for GrantWithExpiryPolicyFacts {
    async fn credential_presence(
        &self,
        _descriptor: &CapabilityDescriptor,
        _scope: &ResourceScope,
    ) -> CredentialPresence {
        CredentialPresence::Satisfied
    }

    async fn persistent_grants(
        &self,
        capability_id: &CapabilityId,
        context: &ExecutionContext,
        _action: crate::ports::PolicyAction,
    ) -> Vec<ironclaw_host_api::capability::CapabilityGrant> {
        use ironclaw_host_api::{
            action::NetworkPolicy,
            capability::{CapabilityGrant, GrantConstraints},
            ids::CapabilityGrantId,
            mount::MountView,
            scope::Principal,
        };
        vec![CapabilityGrant {
            id: CapabilityGrantId::new(),
            capability: capability_id.clone(),
            grantee: Principal::User(context.resource_scope.user_id.clone()),
            issued_by: Principal::HostRuntime,
            constraints: GrantConstraints {
                allowed_effects: Vec::new(),
                mounts: MountView::default(),
                network: NetworkPolicy::default(),
                secrets: Vec::new(),
                resource_ceiling: None,
                expires_at: Some(self.expiry),
                max_invocations: None,
            },
        }]
    }
}

// `authorize()` never dispatches; this satisfies the `CapabilityHost` type
// parameter without pulling in the integration-tier recording dispatcher.
const ECHO_MANIFEST_FIXTURE: &str = r#"
schema_version = "reborn.extension_manifest.v2"
id = "echo"
name = "Echo"
version = "0.1.0"
description = "Echo test extension"
trust = "third_party"

[runtime]
kind = "wasm"
module = "echo.wasm"

[[host_api]]
id = "ironclaw.capability_provider/v1"
section = "capability_provider.tools"

[capability_provider.tools]

[[capability_provider.tools.capabilities]]
id = "echo.say"
description = "Echoes input"
effects = ["dispatch_capability"]
default_permission = "allow"
visibility = "host_internal"
input_schema_ref = "schemas/echo/say.input.v1.json"
output_schema_ref = "schemas/echo/say.output.v1.json"
"#;

const ATLAS_MANIFEST_FIXTURE: &str = r#"
schema_version = "reborn.extension_manifest.v2"
id = "atlas"
name = "Atlas"
version = "0.1.0"
description = "Atlas credential fixture"
trust = "third_party"

[runtime]
kind = "wasm"
module = "atlas.wasm"

[[host_api]]
id = "ironclaw.capability_provider/v1"
section = "capability_provider.tools"

[capability_provider.tools]

[[capability_provider.tools.capabilities]]
id = "atlas.test"
description = "Atlas credential fixture"
effects = ["dispatch_capability", "use_secret"]
default_permission = "allow"
visibility = "host_internal"
input_schema_ref = "schemas/atlas/test.input.v1.json"
output_schema_ref = "schemas/atlas/test.output.v1.json"

[[capability_provider.tools.capabilities.runtime_credentials]]
handle = "atlas_runtime_token"
source = { type = "secret_handle" }
audience = { scheme = "https", host_pattern = "api.atlas.test" }
target = { type = "header", name = "authorization", prefix = "Bearer " }
placeholder_env = "ATLAS_TOKEN"
required = true

[[capability_provider.tools.capabilities.runtime_credentials]]
handle = "atlas_admin_token"
source = { type = "secret_handle" }
audience = { scheme = "https", host_pattern = "admin.atlas.test" }
target = { type = "header", name = "authorization", prefix = "Bearer " }
placeholder_env = "ATLAS_ADMIN_TOKEN"
required = true
"#;

const UNRELATED_AUTH_MANIFEST_FIXTURE: &str = r#"
schema_version = "reborn.extension_manifest.v2"
id = "unrelated"
name = "Unrelated"
version = "0.1.0"
description = "Unrelated product auth fixture"
trust = "third_party"

[runtime]
kind = "wasm"
module = "unrelated.wasm"

[[host_api]]
id = "ironclaw.capability_provider/v1"
section = "capability_provider.tools"

[capability_provider.tools]

[[capability_provider.tools.capabilities]]
id = "unrelated.test"
description = "Unrelated product auth fixture"
effects = ["dispatch_capability", "use_secret"]
default_permission = "allow"
visibility = "host_internal"
input_schema_ref = "schemas/unrelated/test.input.v1.json"
output_schema_ref = "schemas/unrelated/test.output.v1.json"

[[capability_provider.tools.capabilities.runtime_credentials]]
handle = "atlas_runtime_token"
source = { type = "product_auth_account", provider = "atlas" }
audience = { scheme = "https", host_pattern = "api.atlas.test" }
target = { type = "header", name = "authorization", prefix = "Bearer " }
required = true
"#;

fn package_from_manifest(
    manifest_toml: &str,
    root: &str,
) -> ironclaw_extension_registry::ExtensionPackage {
    use ironclaw_extension_registry::{
        CapabilityProviderHostApiContract, ExtensionManifest, ExtensionPackage,
        HostApiContractRegistry, ManifestSource,
    };
    use ironclaw_host_api::{host_port::HostPortCatalog, path::VirtualPath};
    let mut contracts = HostApiContractRegistry::new();
    contracts
        .register(std::sync::Arc::new(
            CapabilityProviderHostApiContract::new().expect("capability provider contract"),
        ))
        .expect("register capability provider contract");
    let manifest = ExtensionManifest::parse(
        manifest_toml,
        ManifestSource::InstalledLocal,
        &HostPortCatalog::empty(),
        &contracts,
    )
    .unwrap();
    ExtensionPackage::from_manifest(manifest, VirtualPath::new(root).unwrap()).unwrap()
}

fn registry_from_manifest(manifest_toml: &str, root: &str) -> ExtensionRegistry {
    let mut registry = ExtensionRegistry::new();
    registry
        .insert(package_from_manifest(manifest_toml, root))
        .unwrap();
    registry
}

fn echo_registry() -> ExtensionRegistry {
    registry_from_manifest(ECHO_MANIFEST_FIXTURE, "/system/extensions/echo")
}

fn allow_request() -> InvocationInput {
    use ironclaw_host_api::{
        capability::CapabilitySet,
        ids::UserId,
        mount::MountView,
        runtime::{RuntimeKind, TrustClass},
    };
    let mut context = ExecutionContext::local_default(
        UserId::new("user").unwrap(),
        ExtensionId::new("caller").unwrap(),
        RuntimeKind::Wasm,
        TrustClass::UserTrusted,
        CapabilitySet::default(),
        MountView::default(),
    )
    .unwrap();
    // A membrane-sealed actor and a real ingress origin are what make the
    // invocation seal-able. This models a direct product-surface action.
    context.authenticated_actor_user_id = Some(UserId::new("actor").unwrap());
    context.origin = Some(ironclaw_host_api::invocation::InvocationOrigin::Product(
        ironclaw_host_api::ids::ProductKind::new("settings").unwrap(),
    ));
    InvocationInput {
        context,
        capability_id: CapabilityId::new("echo.say").unwrap(),
        estimate: ResourceEstimate::default(),
        input: serde_json::json!({"message": "hi"}),
    }
}

/// Trust policy double for the in-fold `evaluate_trust` (§5.3.2/§9): always
/// classifies the echo package as `user_trusted` so the kernel trust-eval
/// succeeds and the `AllowAuthorizer` reaches the seal.
struct StaticTrustPolicy;

impl TrustPolicy for StaticTrustPolicy {
    fn evaluate(
        &self,
        _input: &ironclaw_host_api::trust::TrustPolicyInput,
    ) -> Result<TrustDecision, ironclaw_trust::TrustError> {
        use ironclaw_trust::{AuthorityCeiling, EffectiveTrustClass, TrustProvenance};
        Ok(TrustDecision {
            effective_trust: EffectiveTrustClass::user_trusted(),
            authority_ceiling: AuthorityCeiling {
                allowed_effects: Vec::new(),
                max_resource_ceiling: None,
            },
            provenance: TrustProvenance::Default,
            evaluated_at: chrono::Utc::now(),
        })
    }
}

fn privileged_local_manifest_policy(
    package_id: &str,
    manifest_path: &str,
) -> ironclaw_trust::HostTrustPolicy {
    use ironclaw_host_api::ids::PackageId;
    use ironclaw_trust::{AdminConfig, AdminEntry, HostTrustAssignment, HostTrustPolicy};

    HostTrustPolicy::new(vec![Box::new(AdminConfig::with_entries([
        AdminEntry::for_local_manifest(
            PackageId::new(package_id).unwrap(),
            manifest_path.to_string(),
            None,
            HostTrustAssignment::first_party(),
            vec![ironclaw_host_api::capability::EffectKind::UseSecret],
            None,
        ),
    ]))])
    .unwrap()
}

/// Permissive runtime policy so the in-fold planner never denies the echo
/// capability (echo declares only `dispatch_capability`, so no backend
/// constraint is even exercised).
fn permissive_runtime_policy() -> EffectiveRuntimePolicy {
    use ironclaw_host_api::runtime_policy::{
        ApprovalPolicy, AuditMode, DeploymentMode, FilesystemBackendKind, NetworkMode,
        ProcessBackendKind, RuntimeProfile, SecretMode,
    };
    EffectiveRuntimePolicy {
        deployment: DeploymentMode::LocalSingleUser,
        requested_profile: RuntimeProfile::LocalHost,
        resolved_profile: RuntimeProfile::LocalHost,
        filesystem_backend: FilesystemBackendKind::HostWorkspace,
        process_backend: ProcessBackendKind::LocalHost,
        network_mode: NetworkMode::DirectLogged,
        secret_mode: SecretMode::ScrubbedEnv,
        approval_policy: ApprovalPolicy::AskDestructive,
        audit_mode: AuditMode::LocalMinimal,
    }
}

#[tokio::test]
async fn sandbox_shell_enrichment_uses_explicit_manifest_credential_context() {
    use ironclaw_host_api::{
        capability::{EffectKind, RuntimeCredentialRequirementSource},
        runtime_policy::{ProcessBackendKind, RuntimeProfile},
    };

    let mut registry = registry_from_manifest(ATLAS_MANIFEST_FIXTURE, "/system/extensions/atlas");
    registry
        .insert(package_from_manifest(
            UNRELATED_AUTH_MANIFEST_FIXTURE,
            "/system/extensions/unrelated",
        ))
        .unwrap();
    let dispatcher =
        ironclaw_host_api::dispatch_test_support::TestDispatcher::responding(|request, _| {
            Err(DispatchError::UnknownCapability {
                capability: request.invocation.capability.clone(),
            })
        });
    let authorizer = AllowAuthorizer;
    let trust_policy =
        privileged_local_manifest_policy("atlas", "/system/extensions/atlas/manifest.toml");
    let mut runtime_policy = permissive_runtime_policy();
    runtime_policy.requested_profile = RuntimeProfile::HostedSafe;
    runtime_policy.resolved_profile = RuntimeProfile::HostedSafe;
    runtime_policy.process_backend = ProcessBackendKind::UserSandbox;
    let policy_facts = SatisfiedPolicyFacts;
    let host = CapabilityHost::new(
        &registry,
        &dispatcher,
        &authorizer,
        &trust_policy,
        &runtime_policy,
        &policy_facts,
    );
    let shell_id = CapabilityId::new("builtin.shell").unwrap();
    let mut descriptor = registry
        .get_capability(&CapabilityId::new("atlas.test").unwrap())
        .unwrap()
        .clone();
    descriptor.runtime_credentials.clear();

    let enriched = host
        .enrich_invocation_descriptor(
            &descriptor,
            &shell_id,
            &serde_json::json!({
                "command": "set -e; atlas resources list | jq '.items'; atlas-admin audit",
                "credential_contexts": ["atlas"]
            }),
        )
        .await
        .unwrap();
    assert!(enriched.effects.contains(&EffectKind::UseSecret));
    assert_eq!(
        enriched
            .runtime_credentials
            .iter()
            .map(|requirement| requirement.handle.as_str())
            .collect::<Vec<_>>(),
        ["atlas_runtime_token", "atlas_admin_token"]
    );
    assert!(
        enriched.runtime_credentials.iter().all(|requirement| {
            requirement.source == RuntimeCredentialRequirementSource::SecretHandle
        }),
        "credential authority must come only from the selected extension"
    );

    let unchanged = host
        .enrich_invocation_descriptor(
            &descriptor,
            &shell_id,
            &serde_json::json!({
                "command": "atlas resources list",
                "credential_contexts": []
            }),
        )
        .await
        .unwrap();
    assert_eq!(
        unchanged, descriptor,
        "command text alone must not acquire a credential context"
    );
}

#[tokio::test]
async fn sandbox_shell_enrichment_rejects_unknown_credential_context() {
    use ironclaw_host_api::runtime_policy::{ProcessBackendKind, RuntimeProfile};

    let registry = registry_from_manifest(ATLAS_MANIFEST_FIXTURE, "/system/extensions/atlas");
    let dispatcher =
        ironclaw_host_api::dispatch_test_support::TestDispatcher::responding(|request, _| {
            Err(DispatchError::UnknownCapability {
                capability: request.invocation.capability.clone(),
            })
        });
    let authorizer = AllowAuthorizer;
    let trust_policy = StaticTrustPolicy;
    let mut runtime_policy = permissive_runtime_policy();
    runtime_policy.requested_profile = RuntimeProfile::HostedSafe;
    runtime_policy.resolved_profile = RuntimeProfile::HostedSafe;
    runtime_policy.process_backend = ProcessBackendKind::UserSandbox;
    let policy_facts = SatisfiedPolicyFacts;
    let host = CapabilityHost::new(
        &registry,
        &dispatcher,
        &authorizer,
        &trust_policy,
        &runtime_policy,
        &policy_facts,
    );
    let shell_id = CapabilityId::new("builtin.shell").unwrap();
    let mut descriptor = registry
        .get_capability(&CapabilityId::new("atlas.test").unwrap())
        .unwrap()
        .clone();
    descriptor.runtime_credentials.clear();

    let error = host
        .enrich_invocation_descriptor(
            &descriptor,
            &shell_id,
            &serde_json::json!({
                "command": "echo safe",
                "credential_contexts": ["missing"]
            }),
        )
        .await
        .unwrap_err();
    let CapabilityInvocationError::AuthorizationDenied {
        reason: DenyReason::PolicyDenied,
        detail: Some(detail),
        ..
    } = error
    else {
        panic!("an unknown credential context must fail closed as a policy denial");
    };
    assert_eq!(
        detail,
        "shell credential context `missing` is not an active extension"
    );
}

// The Allow decision seals an `Authorized` whose lane is resolved from the
// descriptor (echo is a WASM extension) and whose invocation carries the
// exact capability/actor/input the request named. Echo declares no resource
// obligation and no persistent grant is adopted, so the witness carries no
// reservation (`None`, never a synthesized placeholder) and its deadline is
// the bounded default TTL (§5.3.2).
#[tokio::test]
async fn authorize_allow_path_seals_authorized_with_lane_and_invocation() {
    use ironclaw_host_api::ids::UserId;

    let registry = echo_registry();
    // Never dispatched on this authorize-only path; errors if it ever is.
    let dispatcher =
        ironclaw_host_api::dispatch_test_support::TestDispatcher::responding(|req, _| {
            Err(DispatchError::UnknownCapability {
                capability: req.invocation.capability.clone(),
            })
        });
    let authorizer = AllowAuthorizer;
    let trust_policy = StaticTrustPolicy;
    let runtime_policy = permissive_runtime_policy();
    let policy_facts = SatisfiedPolicyFacts;
    let host = CapabilityHost::new(
        &registry,
        &dispatcher,
        &authorizer,
        &trust_policy,
        &runtime_policy,
        &policy_facts,
    );

    let request = allow_request();
    let before = chrono::Utc::now();
    let fold = host.authorize(&request).await.unwrap();
    let after = chrono::Utc::now();

    let AuthorizeFold::Authorized(fold) = fold else {
        panic!("expected an allowed authorization");
    };
    let Some(AuthorizeResult::Authorized(authorized)) = &fold.result else {
        panic!("allow path with a sealed actor must mint an Authorized witness");
    };
    assert_eq!(authorized.lane(), RuntimeLane::Wasm);
    assert_eq!(
        authorized.descriptor(),
        Some(
            registry
                .get_capability(&CapabilityId::new("echo.say").unwrap())
                .unwrap()
        ),
        "the witness must freeze the exact descriptor that authorization evaluated"
    );
    let invocation = authorized.invocation();
    assert_eq!(
        invocation.capability,
        CapabilityId::new("echo.say").unwrap()
    );
    assert_eq!(
        invocation.actor,
        Actor::Sealed(UserId::new("actor").unwrap())
    );
    assert_eq!(
        invocation.origin,
        ironclaw_host_api::invocation::InvocationOrigin::Product(
            ironclaw_host_api::ids::ProductKind::new("settings").unwrap()
        )
    );
    assert_eq!(invocation.input, serde_json::json!({"message": "hi"}));
    // No resource obligation → no reservation on the witness.
    assert!(
        authorized.reservation().is_none(),
        "echo declares no resource obligation; the witness must carry no reservation"
    );
    // No frozen fact → the bounded default TTL from authorize-time.
    assert!(authorized.deadline() >= before + WITNESS_DEFAULT_TTL);
    assert!(authorized.deadline() <= after + WITNESS_DEFAULT_TTL);
}

// When a persistent grant carrying an `expires_at` is adopted in the fold, the
// witness deadline is that expiry (the shortest-lived frozen fact), not the
// default TTL.
#[tokio::test]
async fn authorize_seals_witness_deadline_from_adopted_grant_expiry() {
    let expiry = chrono::DateTime::from_timestamp(2_000_000_000, 0).unwrap();
    let registry = echo_registry();
    // Never dispatched on this authorize-only path; errors if it ever is.
    let dispatcher =
        ironclaw_host_api::dispatch_test_support::TestDispatcher::responding(|req, _| {
            Err(DispatchError::UnknownCapability {
                capability: req.invocation.capability.clone(),
            })
        });
    let authorizer = AllowAuthorizer;
    let trust_policy = StaticTrustPolicy;
    let runtime_policy = permissive_runtime_policy();
    let policy_facts = GrantWithExpiryPolicyFacts { expiry };
    let host = CapabilityHost::new(
        &registry,
        &dispatcher,
        &authorizer,
        &trust_policy,
        &runtime_policy,
        &policy_facts,
    );

    let request = allow_request();
    let fold = host.authorize(&request).await.unwrap();

    let AuthorizeFold::Authorized(fold) = fold else {
        panic!("expected an allowed authorization");
    };
    let Some(AuthorizeResult::Authorized(authorized)) = &fold.result else {
        panic!("allow path must mint an Authorized witness");
    };
    assert_eq!(
        authorized.deadline(),
        expiry,
        "adopted persistent-grant expiry is the shortest-lived frozen fact"
    );
}

#[tokio::test]
async fn authorize_seals_system_actor_and_real_origin_across_ingresses() {
    use ironclaw_host_api::{
        ids::{ProductKind, RoutineId, RunId, UserId},
        invocation::InvocationOrigin,
    };

    let registry = echo_registry();
    // Never dispatched on this authorize-only path; errors if it ever is.
    let dispatcher =
        ironclaw_host_api::dispatch_test_support::TestDispatcher::responding(|req, _| {
            Err(DispatchError::UnknownCapability {
                capability: req.invocation.capability.clone(),
            })
        });
    let authorizer = AllowAuthorizer;
    let trust_policy = StaticTrustPolicy;
    let runtime_policy = permissive_runtime_policy();
    let policy_facts = SatisfiedPolicyFacts;
    let host = CapabilityHost::new(
        &registry,
        &dispatcher,
        &authorizer,
        &trust_policy,
        &runtime_policy,
        &policy_facts,
    );

    struct Case {
        actor_override: Option<UserId>,
        origin: Option<InvocationOrigin>,
        run_id: Option<RunId>,
        expected_actor: Actor,
        expected_origin: InvocationOrigin,
    }

    let loop_run = RunId::new();
    let cases = vec![
        Case {
            actor_override: None,
            origin: Some(InvocationOrigin::Product(
                ProductKind::new("settings").unwrap(),
            )),
            run_id: None,
            expected_actor: Actor::System,
            expected_origin: InvocationOrigin::Product(ProductKind::new("settings").unwrap()),
        },
        Case {
            actor_override: Some(UserId::new("actor").unwrap()),
            origin: None,
            run_id: Some(loop_run),
            expected_actor: Actor::Sealed(UserId::new("actor").unwrap()),
            expected_origin: InvocationOrigin::LoopRun(loop_run),
        },
        Case {
            actor_override: None,
            origin: Some(InvocationOrigin::Automation(
                RoutineId::new("heartbeat").unwrap(),
            )),
            run_id: None,
            expected_actor: Actor::System,
            expected_origin: InvocationOrigin::Automation(RoutineId::new("heartbeat").unwrap()),
        },
    ];

    for Case {
        actor_override,
        origin,
        run_id,
        expected_actor,
        expected_origin,
    } in cases
    {
        let mut request = allow_request();
        request.context.authenticated_actor_user_id = actor_override;
        request.context.origin = origin;
        request.context.run_id = run_id;

        let fold = host.authorize(&request).await.unwrap();
        let AuthorizeFold::Authorized(fold) = fold else {
            panic!("expected an allowed authorization for {expected_origin:?}");
        };
        let Some(AuthorizeResult::Authorized(authorized)) = &fold.result else {
            panic!("every allowed invocation must mint a witness ({expected_origin:?})");
        };
        let invocation = authorized.invocation();
        assert_eq!(
            invocation.actor, expected_actor,
            "actor mismatch for {expected_origin:?}"
        );
        assert_eq!(
            invocation.origin, expected_origin,
            "origin mismatch for {expected_origin:?}"
        );
    }
}

#[test]
fn witness_deadline_takes_earliest_candidate_else_default_ttl() {
    let earlier = chrono::DateTime::from_timestamp(1_000, 0).unwrap();
    let later = chrono::DateTime::from_timestamp(2_000, 0).unwrap();
    // Shortest-lived candidate wins; `None` candidates are ignored.
    assert_eq!(
        witness_deadline([Some(later), None, Some(earlier)]),
        earlier
    );
    assert_eq!(witness_deadline([Some(earlier)]), earlier);
    // No frozen fact → bounded default TTL from now.
    let before = chrono::Utc::now();
    let fallback = witness_deadline([None, None]);
    let after = chrono::Utc::now();
    assert!(fallback >= before + WITNESS_DEFAULT_TTL);
    assert!(fallback <= after + WITNESS_DEFAULT_TTL);
}

// --- Resume-path witness deadline (`PendingClaim` lease expiry) ---

// Lease store double for the resume dispatch tail. The pending approval
// lease is claimed after authorization and consumed after successful
// dispatch; all other lease operations are unreachable for this test.
struct PendingClaimLeaseStore {
    lease: CapabilityLease,
}

#[async_trait::async_trait]
impl CapabilityLeaseStorePort for PendingClaimLeaseStore {
    async fn issue(
        &self,
        _lease: CapabilityLease,
    ) -> Result<CapabilityLease, ironclaw_authorization::CapabilityLeaseError> {
        unimplemented!("authorize_resumed does not issue leases")
    }

    async fn revoke(
        &self,
        _scope: &ResourceScope,
        _lease_id: CapabilityGrantId,
    ) -> Result<CapabilityLease, ironclaw_authorization::CapabilityLeaseError> {
        unimplemented!("authorize_resumed does not revoke leases")
    }

    async fn get(
        &self,
        _scope: &ResourceScope,
        _lease_id: CapabilityGrantId,
    ) -> Option<CapabilityLease> {
        unimplemented!("authorize_resumed does not read leases")
    }

    async fn claim(
        &self,
        scope: &ResourceScope,
        lease_id: CapabilityGrantId,
        _invocation_fingerprint: &InvocationFingerprint,
    ) -> Result<CapabilityLease, ironclaw_authorization::CapabilityLeaseError> {
        assert_eq!(scope, &self.lease.scope); // safety: test-only lease-store double validates caller scope.
        assert_eq!(lease_id, self.lease.grant.id); // safety: test-only lease-store double validates caller lease id.
        let mut lease = self.lease.clone();
        lease.status = ironclaw_authorization::CapabilityLeaseStatus::Claimed;
        Ok(lease)
    }

    async fn consume(
        &self,
        scope: &ResourceScope,
        lease_id: CapabilityGrantId,
    ) -> Result<CapabilityLease, ironclaw_authorization::CapabilityLeaseError> {
        assert_eq!(scope, &self.lease.scope); // safety: test-only lease-store double validates caller scope.
        assert_eq!(lease_id, self.lease.grant.id); // safety: test-only lease-store double validates caller lease id.
        let mut lease = self.lease.clone();
        lease.status = ironclaw_authorization::CapabilityLeaseStatus::Consumed;
        Ok(lease)
    }

    async fn begin_dispatch_claimed(
        &self,
        _scope: &ResourceScope,
        _lease_id: CapabilityGrantId,
        _invocation_fingerprint: &InvocationFingerprint,
    ) -> Result<CapabilityLease, ironclaw_authorization::CapabilityLeaseError> {
        unimplemented!("authorize_resumed does not transition leases")
    }

    async fn abort_dispatch_claimed(
        &self,
        _scope: &ResourceScope,
        _lease_id: CapabilityGrantId,
    ) -> Result<CapabilityLease, ironclaw_authorization::CapabilityLeaseError> {
        unimplemented!("authorize_resumed does not transition leases")
    }

    async fn leases_for_scope(&self, _scope: &ResourceScope) -> Vec<CapabilityLease> {
        unimplemented!("authorize_resumed does not enumerate leases")
    }

    async fn active_leases_for_context(&self, _context: &ExecutionContext) -> Vec<CapabilityLease> {
        unimplemented!("authorize_resumed does not enumerate leases")
    }
}

// Run-state double for the successful resume tail: only the post-dispatch
// completion transition is reachable.
struct CompletionInvocationStateStore;

#[async_trait::async_trait]
impl ProcessInvocationStatePort for CompletionInvocationStateStore {
    async fn start(
        &self,
        _start: ProcessInvocationStart,
    ) -> Result<ironclaw_processes::ProcessInvocationRecord, ProcessInvocationError> {
        unimplemented!("authorize_resumed Allow path does not mutate invocation state")
    }

    async fn block_approval(
        &self,
        _scope: &ResourceScope,
        _invocation_id: InvocationId,
        _approval: ironclaw_host_api::approval::ApprovalRequest,
    ) -> Result<ironclaw_processes::ProcessInvocationRecord, ProcessInvocationError> {
        unimplemented!("authorize_resumed Allow path does not mutate invocation state")
    }

    async fn block_auth(
        &self,
        _scope: &ResourceScope,
        _invocation_id: InvocationId,
        _error_kind: String,
    ) -> Result<ironclaw_processes::ProcessInvocationRecord, ProcessInvocationError> {
        unimplemented!("authorize_resumed Allow path does not mutate invocation state")
    }

    async fn complete(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
    ) -> Result<ironclaw_processes::ProcessInvocationRecord, ProcessInvocationError> {
        Ok(ironclaw_processes::ProcessInvocationRecord {
            invocation_id,
            capability_id: CapabilityId::new("echo.say").unwrap(),
            scope: scope.clone(),
            authenticated_actor_user_id: None,
            status: ProcessInvocationStatus::Completed,
            approval_request_id: None,
            error_kind: None,
        })
    }

    async fn fail(
        &self,
        _scope: &ResourceScope,
        _invocation_id: InvocationId,
        _error_kind: String,
    ) -> Result<ironclaw_processes::ProcessInvocationRecord, ProcessInvocationError> {
        unimplemented!("authorize_resumed Allow path does not mutate invocation state")
    }

    async fn get(
        &self,
        _scope: &ResourceScope,
        _invocation_id: InvocationId,
    ) -> Result<Option<ironclaw_processes::ProcessInvocationRecord>, ProcessInvocationError> {
        unimplemented!("authorize_resumed Allow path does not read invocation state")
    }

    async fn records_for_scope(
        &self,
        _scope: &ResourceScope,
    ) -> Result<Vec<ironclaw_processes::ProcessInvocationRecord>, ProcessInvocationError> {
        unimplemented!("authorize_resumed Allow path does not read invocation state")
    }
}

// A `resume_json` (`PendingClaim`) resume must seal the dispatch witness
// deadline bounded by the approval lease's expiry — threaded onto the
// pending-claim spec because the claim is deferred until after authorization
// — NOT the 5-minute default TTL, so a held witness can never outlive the
// approval that authorized it.
#[tokio::test]
async fn resumed_pending_claim_dispatch_seals_witness_deadline_from_lease_expiry() {
    // A lease expiry well inside the bounded 5-minute default window, so a
    // fallback to the default TTL would be observably wrong.
    let lease_expiry = chrono::Utc::now() + chrono::Duration::seconds(30);
    assert!(lease_expiry < chrono::Utc::now() + WITNESS_DEFAULT_TTL);

    let registry = echo_registry();
    let dispatcher =
        ironclaw_host_api::dispatch_test_support::TestDispatcher::responding(|request, _| {
            Ok(CapabilityDispatchResult {
                capability_id: request.invocation.capability.clone(),
                provider: ExtensionId::new("echo").unwrap(),
                runtime: RuntimeKind::Wasm,
                output: serde_json::json!({"ok": true}),
                display_preview: None,
                usage: ironclaw_host_api::resource::ResourceUsage::default(),
                receipt: ironclaw_host_api::resource::ResourceReceipt {
                    id: ironclaw_host_api::ids::ResourceReservationId::new(),
                    scope: request.invocation.scope.clone(),
                    status: ironclaw_host_api::resource::ReservationStatus::Reconciled,
                    estimate: request.invocation.estimate.clone(),
                    actual: Some(ironclaw_host_api::resource::ResourceUsage::default()),
                },
            })
        });
    let authorizer = AllowAuthorizer;
    let trust_policy = StaticTrustPolicy;
    let runtime_policy = permissive_runtime_policy();
    let policy_facts = SatisfiedPolicyFacts;
    let host = CapabilityHost::new(
        &registry,
        &dispatcher,
        &authorizer,
        &trust_policy,
        &runtime_policy,
        &policy_facts,
    );

    let request = allow_request();
    let capability_id = request.capability_id.clone();
    let estimate = request.estimate.clone();
    let input = request.input.clone();
    let context = request.context.clone();
    let scope = context.resource_scope.clone();
    let invocation_id = context.invocation_id;
    let descriptor = registry
        .get_capability(&capability_id)
        .expect("echo.say is registered");

    let grant_id = CapabilityGrantId::new();
    let fingerprint =
        InvocationFingerprint::for_dispatch(&scope, &capability_id, &estimate, &input).unwrap();
    let leases = PendingClaimLeaseStore {
        lease: CapabilityLease {
            scope: scope.clone(),
            grant: ironclaw_host_api::capability::CapabilityGrant {
                id: grant_id,
                capability: capability_id.clone(),
                grantee: ironclaw_host_api::scope::Principal::User(scope.user_id.clone()),
                issued_by: ironclaw_host_api::scope::Principal::HostRuntime,
                constraints: ironclaw_host_api::capability::GrantConstraints {
                    allowed_effects: Vec::new(),
                    mounts: ironclaw_host_api::mount::MountView::default(),
                    network: ironclaw_host_api::action::NetworkPolicy::default(),
                    secrets: Vec::new(),
                    resource_ceiling: None,
                    expires_at: Some(lease_expiry),
                    max_invocations: None,
                },
            },
            invocation_fingerprint: Some(fingerprint.clone()),
            status: ironclaw_authorization::CapabilityLeaseStatus::Active,
        },
    };
    let invocation_state = CompletionInvocationStateStore;

    let params = ResumedDispatchParams {
        invocation_state: &invocation_state,
        scope,
        invocation_id,
        capability_id,
        estimate,
        input,
        authorized_context: context,
        descriptor,
        lease_state: ResumedLeaseState::PendingClaim(PendingClaimAfterAuth {
            leases: &leases,
            grant_id,
            fingerprint,
            grant_expiry: Some(lease_expiry),
        }),
    };

    host.dispatch_resumed_capability(params).await.unwrap();
    let dispatched = dispatcher.last_request().unwrap();
    assert_eq!(
        dispatched.deadline, lease_expiry,
        "the sealed witness deadline must be bounded by the approval lease expiry, not the default TTL"
    );
}
