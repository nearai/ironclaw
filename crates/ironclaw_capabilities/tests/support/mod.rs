#![allow(dead_code)]

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_authorization::*;
use ironclaw_extensions::*;
use ironclaw_host_api::*;
use ironclaw_trust::{AuthorityCeiling, EffectiveTrustClass, TrustDecision, TrustProvenance};
use serde_json::json;

// The shared `CapabilityDispatcher` double. Re-exported so every test file that
// does `use support::*` gets it without importing the feature-gated module path.
pub use ironclaw_host_api::dispatch_test_support::TestDispatcher;

/// The standard success dispatch result, echoing the request's capability id,
/// scope, and estimate — the exact shape the retired hand-rolled
/// `RecordingDispatcher` returned.
pub fn ok_dispatch_result(request: &CapabilityDispatchRequest) -> CapabilityDispatchResult {
    dispatch_result_with_output(request, json!({"ok": true}))
}

/// Like [`ok_dispatch_result`] but with a caller-supplied output payload — the
/// replacement for the retired `OutputDispatcher`.
pub fn dispatch_result_with_output(
    request: &CapabilityDispatchRequest,
    output: serde_json::Value,
) -> CapabilityDispatchResult {
    CapabilityDispatchResult {
        capability_id: request.capability_id.clone(),
        provider: extension_id(),
        runtime: RuntimeKind::Wasm,
        output,
        display_preview: None,
        usage: ResourceUsage::default(),
        receipt: ResourceReceipt {
            id: ResourceReservationId::new(),
            scope: request.scope.clone(),
            status: ReservationStatus::Reconciled,
            estimate: request.estimate.clone(),
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
    ExecutionContext::local_default(
        UserId::new("user").unwrap(),
        ExtensionId::new("caller").unwrap(),
        RuntimeKind::Wasm,
        TrustClass::UserTrusted,
        grants,
        MountView::default(),
    )
    .unwrap()
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

[[capabilities]]
id = "echo.say"
description = "Echoes input"
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

[[capabilities]]
id = "github.comment_issue"
description = "Add a comment to a GitHub issue or pull request."
effects = ["dispatch_capability", "network", "use_secret", "external_write"]
default_permission = "ask"
visibility = "model"
input_schema_ref = "schemas/github/comment_issue.input.v1.json"
output_schema_ref = "schemas/github/comment_issue.output.v1.json"
prompt_doc_ref = "prompts/github/comment_issue.md"
"#;
