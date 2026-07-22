#![allow(dead_code)]

use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};

use std::sync::LazyLock;

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_authorization::*;
use ironclaw_capabilities::{CapabilityHost, CredentialPresence, HostPolicyFacts, PolicyAction};
use ironclaw_extensions::*;
use ironclaw_host_api::runtime_policy::{
    ApprovalPolicy, AuditMode, DeploymentMode, FilesystemBackendKind, NetworkMode,
    ProcessBackendKind, RuntimeProfile, SecretMode,
};
use ironclaw_host_api::*;
use ironclaw_trust::{
    AuthorityCeiling, EffectiveTrustClass, TrustDecision, TrustError, TrustPolicy,
    TrustPolicyInput, TrustProvenance,
};
use serde_json::json;

/// Trust policy double supplying the kernel's in-fold `evaluate_trust`
/// (§5.3.2/§9). It always returns the fixed `user_trusted` [`trust_decision`],
/// so kernel trust-eval succeeds for the test packages and existing test
/// outcomes are unchanged (the authorizer doubles ignore the decision anyway).
struct StaticTrustPolicy;

impl TrustPolicy for StaticTrustPolicy {
    fn evaluate(&self, _input: &TrustPolicyInput) -> Result<TrustDecision, TrustError> {
        Ok(trust_decision())
    }
}

static DEFAULT_TRUST_POLICY: LazyLock<StaticTrustPolicy> = LazyLock::new(|| StaticTrustPolicy);

/// Permissive runtime policy so the in-fold planner (`plan_capability`) never
/// denies the test capabilities. Field shape mirrors
/// `ironclaw_runtime_policy::planner` tests.
static DEFAULT_RUNTIME_POLICY: LazyLock<EffectiveRuntimePolicy> =
    LazyLock::new(|| EffectiveRuntimePolicy {
        deployment: DeploymentMode::LocalSingleUser,
        requested_profile: RuntimeProfile::LocalDev,
        resolved_profile: RuntimeProfile::LocalDev,
        filesystem_backend: FilesystemBackendKind::HostWorkspace,
        process_backend: ProcessBackendKind::LocalHost,
        network_mode: NetworkMode::DirectLogged,
        secret_mode: SecretMode::ScrubbedEnv,
        approval_policy: ApprovalPolicy::AskDestructive,
        audit_mode: AuditMode::LocalMinimal,
    });

/// Permissive `HostPolicyFacts` double: every credential is present and no
/// persistent grants exist, so the kernel's in-fold credential pre-flight
/// (§5.3.2/§9) never fires and existing test outcomes are unchanged.
pub struct PermissiveHostPolicyFacts;

#[async_trait]
impl HostPolicyFacts for PermissiveHostPolicyFacts {
    async fn credential_presence(
        &self,
        _capability_id: &CapabilityId,
        _scope: &ResourceScope,
    ) -> CredentialPresence {
        CredentialPresence::Satisfied
    }

    async fn persistent_grants(
        &self,
        _capability_id: &CapabilityId,
        _context: &ExecutionContext,
        _action: PolicyAction,
    ) -> Vec<CapabilityGrant> {
        Vec::new()
    }
}

static DEFAULT_POLICY_FACTS: LazyLock<PermissiveHostPolicyFacts> =
    LazyLock::new(|| PermissiveHostPolicyFacts);

/// `HostPolicyFacts` double whose credential pre-flight always reports a missing
/// credential (one required secret + one requirement). Drives the
/// credential-before-approval regression: the kernel must return
/// `AuthorizationRequiresAuth` before the authorizer's approval decision.
pub struct MissingCredentialPolicyFacts;

#[async_trait]
impl HostPolicyFacts for MissingCredentialPolicyFacts {
    async fn credential_presence(
        &self,
        _capability_id: &CapabilityId,
        _scope: &ResourceScope,
    ) -> CredentialPresence {
        CredentialPresence::Missing {
            required_secrets: vec![SecretHandle::new("test_missing_token").unwrap()],
            requirements: vec![RuntimeCredentialAuthRequirement {
                provider: VendorId::new("test_provider").unwrap(),
                setup: RuntimeCredentialAccountSetup::ManualToken,
                requester_extension: ExtensionId::new("caller").unwrap(),
                provider_scopes: Vec::new(),
            }],
        }
    }

    async fn persistent_grants(
        &self,
        _capability_id: &CapabilityId,
        _context: &ExecutionContext,
        _action: PolicyAction,
    ) -> Vec<CapabilityGrant> {
        Vec::new()
    }
}

/// `HostPolicyFacts` double whose `persistent_grants` returns exactly one
/// caller-supplied candidate grant (credential pre-flight satisfied). Drives the
/// persistent-approval regression: the kernel's `authorize()` fold must adopt the
/// grant that flips the authorizer to `Allow`, dispatching without an approval
/// gate.
pub struct PersistentGrantPolicyFacts {
    grant: CapabilityGrant,
}

impl PersistentGrantPolicyFacts {
    pub fn new(grant: CapabilityGrant) -> Self {
        Self { grant }
    }
}

#[async_trait]
impl HostPolicyFacts for PersistentGrantPolicyFacts {
    async fn credential_presence(
        &self,
        _capability_id: &CapabilityId,
        _scope: &ResourceScope,
    ) -> CredentialPresence {
        CredentialPresence::Satisfied
    }

    async fn persistent_grants(
        &self,
        _capability_id: &CapabilityId,
        _context: &ExecutionContext,
        _action: PolicyAction,
    ) -> Vec<CapabilityGrant> {
        vec![self.grant.clone()]
    }
}

/// Central constructor for `CapabilityHost` in tests.
///
/// Every test builds its host through this helper instead of calling
/// `CapabilityHost::new` inline, so a change to the kernel's construction
/// signature (e.g. the §5.3.2 milestone adding the trust-policy / runtime-policy
/// inputs) touches this one place rather than ~130 call sites. The trust-policy
/// and runtime-policy defaults are supplied here as `&'static` permissive values.
pub fn capability_host<'a, D>(
    registry: &'a ExtensionRegistry,
    dispatcher: &'a D,
    authorizer: &'a dyn TrustAwareCapabilityDispatchAuthorizer,
) -> CapabilityHost<'a, D>
where
    D: CapabilityDispatcher + ?Sized,
{
    CapabilityHost::new(
        registry,
        dispatcher,
        authorizer,
        &*DEFAULT_TRUST_POLICY,
        &DEFAULT_RUNTIME_POLICY,
        &*DEFAULT_POLICY_FACTS,
    )
}

/// `capability_host` variant that injects a caller-supplied [`HostPolicyFacts`]
/// double (trust and runtime policy stay the permissive defaults). Use when a
/// test needs the kernel's in-fold credential pre-flight to fire — e.g. the
/// credential-before-approval regression with [`MissingCredentialPolicyFacts`].
pub fn capability_host_with_policy_facts<'a, D>(
    registry: &'a ExtensionRegistry,
    dispatcher: &'a D,
    authorizer: &'a dyn TrustAwareCapabilityDispatchAuthorizer,
    policy_facts: &'a dyn HostPolicyFacts,
) -> CapabilityHost<'a, D>
where
    D: CapabilityDispatcher + ?Sized,
{
    CapabilityHost::new(
        registry,
        dispatcher,
        authorizer,
        &*DEFAULT_TRUST_POLICY,
        &DEFAULT_RUNTIME_POLICY,
        policy_facts,
    )
}

/// Trust policy double returning a caller-chosen fixed decision, for tests that
/// exercise the kernel's in-fold trust ceiling (§5.3.2/§9) — e.g. a decision
/// whose authority ceiling omits an effect, so a trust-aware authorizer denies.
pub struct FixedTrustPolicy {
    decision: TrustDecision,
}

impl FixedTrustPolicy {
    pub fn with_effects(allowed_effects: Vec<EffectKind>) -> Self {
        Self {
            decision: trust_decision_with_effects(allowed_effects),
        }
    }
}

impl TrustPolicy for FixedTrustPolicy {
    fn evaluate(&self, _input: &TrustPolicyInput) -> Result<TrustDecision, TrustError> {
        Ok(self.decision.clone())
    }
}

/// `capability_host` variant that injects a caller-supplied trust policy (the
/// runtime policy stays the permissive default). Use only when a test needs a
/// non-default kernel trust ceiling; otherwise use [`capability_host`].
pub fn capability_host_with_trust_policy<'a, D>(
    registry: &'a ExtensionRegistry,
    dispatcher: &'a D,
    authorizer: &'a dyn TrustAwareCapabilityDispatchAuthorizer,
    trust_policy: &'a dyn TrustPolicy,
) -> CapabilityHost<'a, D>
where
    D: CapabilityDispatcher + ?Sized,
{
    CapabilityHost::new(
        registry,
        dispatcher,
        authorizer,
        trust_policy,
        &DEFAULT_RUNTIME_POLICY,
        &*DEFAULT_POLICY_FACTS,
    )
}

#[derive(Default)]
pub struct RecordingDispatcher {
    request: Mutex<Option<ironclaw_host_api::dispatch_test_support::AuthorizedDispatchRecord>>,
    dispatch_count: AtomicUsize,
}

impl RecordingDispatcher {
    pub fn take_request(
        &self,
    ) -> ironclaw_host_api::dispatch_test_support::AuthorizedDispatchRecord {
        self.request
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
            .unwrap()
    }

    pub fn has_request(&self) -> bool {
        self.request
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_some()
    }

    pub fn dispatch_count(&self) -> usize {
        self.dispatch_count.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl CapabilityDispatcher for RecordingDispatcher {
    async fn dispatch_json(
        &self,
        authorized: Authorized,
    ) -> Result<CapabilityDispatchResult, DispatchError> {
        let deadline = authorized.deadline();
        let (invocation, lane, mounts, resource_reservation) = authorized
            .into_parts(chrono::Utc::now())
            .map_err(|authorized| {
                let capability = authorized.invocation().capability.clone();
                let _ = authorized.abort();
                DispatchError::AuthorizationExpired { capability }
            })?;
        let request = ironclaw_host_api::dispatch_test_support::AuthorizedDispatchRecord {
            authenticated_actor_user_id: invocation.actor.user_id().cloned(),
            run_id: match &invocation.origin {
                InvocationOrigin::LoopRun(run_id) if invocation.process_id.is_none() => {
                    Some(*run_id)
                }
                _ => None,
            },
            mounts,
            invocation,
            lane,
            resource_reservation,
            deadline,
        };
        self.dispatch_count.fetch_add(1, Ordering::SeqCst);
        *self
            .request
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(request.clone());
        Ok(CapabilityDispatchResult {
            capability_id: request.invocation.capability.clone(),
            provider: extension_id(),
            runtime: RuntimeKind::Wasm,
            output: json!({"ok": true}),
            display_preview: None,
            usage: ResourceUsage::default(),
            receipt: ResourceReceipt {
                id: ResourceReservationId::new(),
                scope: request.invocation.scope,
                status: ReservationStatus::Reconciled,
                estimate: request.invocation.estimate,
                actual: Some(ResourceUsage::default()),
            },
        })
    }
}

// The shared `CapabilityDispatcher` double. Re-exported so every test file that
// does `use support::*` gets it without importing the feature-gated module path.
pub use ironclaw_host_api::dispatch_test_support::TestDispatcher;

/// The standard success dispatch result, echoing the request's capability id,
/// scope, and estimate — the exact shape the retired hand-rolled
/// `RecordingDispatcher` returned.
pub fn ok_dispatch_result(
    request: &ironclaw_host_api::dispatch_test_support::AuthorizedDispatchRecord,
) -> CapabilityDispatchResult {
    dispatch_result_with_output(request, json!({"ok": true}))
}

/// Like [`ok_dispatch_result`] but with a caller-supplied output payload — the
/// replacement for the retired `OutputDispatcher`.
pub fn dispatch_result_with_output(
    request: &ironclaw_host_api::dispatch_test_support::AuthorizedDispatchRecord,
    output: serde_json::Value,
) -> CapabilityDispatchResult {
    CapabilityDispatchResult {
        capability_id: request.invocation.capability.clone(),
        provider: extension_id(),
        runtime: RuntimeKind::Wasm,
        output,
        display_preview: None,
        usage: ResourceUsage::default(),
        receipt: ResourceReceipt {
            id: ResourceReservationId::new(),
            scope: request.invocation.scope.clone(),
            status: ReservationStatus::Reconciled,
            estimate: request.invocation.estimate.clone(),
            actual: Some(ResourceUsage::default()),
        },
    }
}

/// Drop-in replacement for the retired `RecordingDispatcher`: records every
/// dispatched request and returns the standard success result. Assert on it via
/// `.recorded()` / `.call_count()` / `.last_request()`.
pub fn recording_dispatcher() -> TestDispatcher {
    TestDispatcher::responding(|request, _| Ok(ok_dispatch_result(request)))
}

pub struct ApprovalAuthorizer;

#[async_trait]
impl TrustAwareCapabilityDispatchAuthorizer for ApprovalAuthorizer {
    async fn authorize_dispatch_with_trust(
        &self,
        context: &ExecutionContext,
        descriptor: &CapabilityDescriptor,
        estimate: &ResourceEstimate,
        _trust_decision: &TrustDecision,
    ) -> Decision {
        Decision::RequireApproval {
            request: ApprovalRequest {
                id: ApprovalRequestId::new(),
                correlation_id: context.correlation_id,
                requested_by: Principal::Extension(context.extension_id.clone()),
                action: Box::new(Action::Dispatch {
                    capability: descriptor.id.clone(),
                    estimated_resources: estimate.clone(),
                }),
                invocation_fingerprint: None,
                reason: "approval required".to_string(),
                reusable_scope: None,
            },
        }
    }
}

pub struct MismatchedApprovalAuthorizer;

#[async_trait]
impl TrustAwareCapabilityDispatchAuthorizer for MismatchedApprovalAuthorizer {
    async fn authorize_dispatch_with_trust(
        &self,
        context: &ExecutionContext,
        _descriptor: &CapabilityDescriptor,
        estimate: &ResourceEstimate,
        _trust_decision: &TrustDecision,
    ) -> Decision {
        let wrong_scope =
            ResourceScope::local_default(context.user_id.clone(), InvocationId::new()).unwrap();
        Decision::RequireApproval {
            request: ApprovalRequest {
                id: ApprovalRequestId::new(),
                correlation_id: context.correlation_id,
                requested_by: Principal::Extension(context.extension_id.clone()),
                action: Box::new(Action::Dispatch {
                    capability: capability_id(),
                    estimated_resources: estimate.clone(),
                }),
                invocation_fingerprint: Some(
                    InvocationFingerprint::for_dispatch(
                        &wrong_scope,
                        &capability_id(),
                        estimate,
                        &json!({"message": "different"}),
                    )
                    .unwrap(),
                ),
                reason: "approval required".to_string(),
                reusable_scope: None,
            },
        }
    }
}

pub struct ObligatingAuthorizer;

#[async_trait]
impl TrustAwareCapabilityDispatchAuthorizer for ObligatingAuthorizer {
    async fn authorize_dispatch_with_trust(
        &self,
        _context: &ExecutionContext,
        _descriptor: &CapabilityDescriptor,
        _estimate: &ResourceEstimate,
        _trust_decision: &TrustDecision,
    ) -> Decision {
        Decision::Allow {
            obligations: Obligations::new(vec![Obligation::AuditBefore]).unwrap(),
        }
    }
}

pub fn registry_with_echo_capability() -> ExtensionRegistry {
    let manifest = ExtensionManifest::parse(
        ECHO_MANIFEST,
        ManifestSource::InstalledLocal,
        &HostPortCatalog::empty(),
        &capability_provider_contracts(),
    )
    .unwrap();
    let package = ExtensionPackage::from_manifest(
        manifest,
        VirtualPath::new("/system/extensions/echo").unwrap(),
    )
    .unwrap();
    let mut registry = ExtensionRegistry::new();
    registry.insert(package).unwrap();
    registry
}

pub fn registry_with_github_comment_capability() -> ExtensionRegistry {
    let manifest = ExtensionManifest::parse(
        GITHUB_COMMENT_MANIFEST,
        ManifestSource::InstalledLocal,
        &HostPortCatalog::empty(),
        &capability_provider_contracts(),
    )
    .unwrap();
    let package = ExtensionPackage::from_manifest(
        manifest,
        VirtualPath::new("/system/extensions/github").unwrap(),
    )
    .unwrap();
    let mut registry = ExtensionRegistry::new();
    registry.insert(package).unwrap();
    registry
}

pub fn execution_context(grants: CapabilitySet) -> ExecutionContext {
    let mut context = ExecutionContext::local_default(
        UserId::new("user").unwrap(),
        ExtensionId::new("caller").unwrap(),
        RuntimeKind::Wasm,
        TrustClass::UserTrusted,
        grants,
        MountView::default(),
    )
    .unwrap();
    context.origin = Some(InvocationOrigin::Product(
        ProductKind::new("tests").unwrap(),
    ));
    context
}

pub fn dispatch_grant() -> CapabilityGrant {
    capability_grant_with_effects(vec![EffectKind::DispatchCapability])
}

pub fn spawn_grant() -> CapabilityGrant {
    capability_grant_with_effects(vec![
        EffectKind::DispatchCapability,
        EffectKind::SpawnProcess,
    ])
}

pub fn capability_grant_with_effects(allowed_effects: Vec<EffectKind>) -> CapabilityGrant {
    CapabilityGrant {
        id: CapabilityGrantId::new(),
        capability: capability_id(),
        grantee: Principal::Extension(ExtensionId::new("caller").unwrap()),
        issued_by: Principal::HostRuntime,
        constraints: GrantConstraints {
            allowed_effects,
            mounts: MountView::default(),
            network: NetworkPolicy::default(),
            secrets: Vec::new(),
            resource_ceiling: None,
            expires_at: None,
            max_invocations: None,
        },
    }
}

pub fn trust_decision() -> TrustDecision {
    trust_decision_with_effects(vec![
        EffectKind::DispatchCapability,
        EffectKind::SpawnProcess,
    ])
}

pub fn trust_decision_with_effects(allowed_effects: Vec<EffectKind>) -> TrustDecision {
    TrustDecision {
        effective_trust: EffectiveTrustClass::user_trusted(),
        authority_ceiling: AuthorityCeiling {
            allowed_effects,
            max_resource_ceiling: None,
        },
        provenance: TrustProvenance::Default,
        evaluated_at: Utc::now(),
    }
}

pub fn capability_id() -> CapabilityId {
    CapabilityId::new("echo.say").unwrap()
}

/// A second, known-and-plannable echo-family capability. Used by the
/// capability-id-mismatch resume tests to trigger a mismatch with a capability
/// that EXISTS in the registry (and passes `plan_capability`), so the run-state
/// mismatch check fires — instead of an unknown id, which now short-circuits to
/// `UnknownCapability` in `resume_preflight` (existence-first, matching
/// host_runtime's deleted pre-authorization; see the resume-preflight change).
pub fn other_capability_id() -> CapabilityId {
    CapabilityId::new("echo.other").unwrap()
}

pub fn github_comment_capability_id() -> CapabilityId {
    CapabilityId::new("github.comment_issue").unwrap()
}

pub fn extension_id() -> ExtensionId {
    ExtensionId::new("echo").unwrap()
}

const ECHO_MANIFEST: &str = r#"
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

[[capability_provider.tools.capabilities]]
id = "echo.other"
description = "A second echo capability, distinct from echo.say"
effects = ["dispatch_capability"]
default_permission = "allow"
visibility = "host_internal"
input_schema_ref = "schemas/echo/say.input.v1.json"
output_schema_ref = "schemas/echo/say.output.v1.json"
"#;

const GITHUB_COMMENT_MANIFEST: &str = r#"
schema_version = "reborn.extension_manifest.v2"
id = "github"
name = "GitHub"
version = "0.2.5"
description = "GitHub issue comment test extension"
trust = "third_party"

[runtime]
kind = "wasm"
module = "wasm/github_tool.wasm"

[[host_api]]
id = "ironclaw.capability_provider/v1"
section = "capability_provider.tools"

[capability_provider.tools]

[[capability_provider.tools.capabilities]]
id = "github.comment_issue"
description = "Add a comment to a GitHub issue or pull request."
effects = ["dispatch_capability", "network", "use_secret", "external_write"]
default_permission = "ask"
visibility = "model"
input_schema_ref = "schemas/github/comment_issue.input.v1.json"
output_schema_ref = "schemas/github/comment_issue.output.v1.json"
prompt_doc_ref = "prompts/github/comment_issue.md"
"#;

fn capability_provider_contracts() -> ironclaw_extensions::HostApiContractRegistry {
    let mut contracts = ironclaw_extensions::HostApiContractRegistry::new();
    contracts
        .register(std::sync::Arc::new(
            ironclaw_extensions::CapabilityProviderHostApiContract::new()
                .expect("capability provider contract"),
        ))
        .expect("register capability provider contract");
    contracts
}
