#![allow(dead_code)]

use std::sync::Mutex;

use async_trait::async_trait;
use ironclaw_authorization::*;
use ironclaw_extensions::*;
use ironclaw_host_api::*;
use serde_json::json;

#[derive(Default)]
pub struct RecordingDispatcher {
    request: Mutex<Option<CapabilityDispatchRequest>>,
}

impl RecordingDispatcher {
    pub fn take_request(&self) -> CapabilityDispatchRequest {
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
}

#[async_trait]
impl CapabilityDispatcher for RecordingDispatcher {
    async fn dispatch_json(
        &self,
        request: CapabilityDispatchRequest,
    ) -> Result<CapabilityDispatchResult, DispatchError> {
        *self
            .request
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(request.clone());
        Ok(CapabilityDispatchResult {
            capability_id: request.capability_id,
            provider: extension_id(),
            runtime: RuntimeKind::Wasm,
            output: json!({"ok": true}),
            usage: ResourceUsage::default(),
            receipt: ResourceReceipt {
                id: ResourceReservationId::new(),
                scope: request.scope,
                status: ReservationStatus::Reconciled,
                estimate: request.estimate,
                actual: Some(ResourceUsage::default()),
            },
        })
    }
}

pub struct ApprovalAuthorizer;

#[async_trait]
impl CapabilityDispatchAuthorizer for ApprovalAuthorizer {
    async fn authorize_dispatch(
        &self,
        context: &ExecutionContext,
        _descriptor: &CapabilityDescriptor,
        estimate: &ResourceEstimate,
    ) -> Decision {
        Decision::RequireApproval {
            request: ApprovalRequest {
                id: ApprovalRequestId::new(),
                correlation_id: context.correlation_id,
                requested_by: Principal::Extension(context.extension_id.clone()),
                action: Box::new(Action::Dispatch {
                    capability: capability_id(),
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
impl CapabilityDispatchAuthorizer for MismatchedApprovalAuthorizer {
    async fn authorize_dispatch(
        &self,
        context: &ExecutionContext,
        _descriptor: &CapabilityDescriptor,
        estimate: &ResourceEstimate,
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
impl CapabilityDispatchAuthorizer for ObligatingAuthorizer {
    async fn authorize_dispatch(
        &self,
        _context: &ExecutionContext,
        _descriptor: &CapabilityDescriptor,
        _estimate: &ResourceEstimate,
    ) -> Decision {
        Decision::Allow {
            obligations: Obligations::new(vec![Obligation::AuditBefore]).unwrap(),
        }
    }
}

pub fn registry_with_echo_capability() -> ExtensionRegistry {
    let manifest = ExtensionManifest::parse(ECHO_MANIFEST).unwrap();
    let package = ExtensionPackage::from_manifest(
        manifest,
        VirtualPath::new("/system/extensions/echo").unwrap(),
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
    CapabilityGrant {
        id: CapabilityGrantId::new(),
        capability: capability_id(),
        grantee: Principal::Extension(ExtensionId::new("caller").unwrap()),
        issued_by: Principal::HostRuntime,
        constraints: GrantConstraints {
            allowed_effects: vec![EffectKind::DispatchCapability],
            mounts: MountView::default(),
            network: NetworkPolicy::default(),
            secrets: Vec::new(),
            resource_ceiling: None,
            expires_at: None,
            max_invocations: None,
        },
    }
}

pub fn capability_id() -> CapabilityId {
    CapabilityId::new("echo.say").unwrap()
}

pub fn extension_id() -> ExtensionId {
    ExtensionId::new("echo").unwrap()
}

const ECHO_MANIFEST: &str = r#"
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
parameters_schema = {}
"#;
