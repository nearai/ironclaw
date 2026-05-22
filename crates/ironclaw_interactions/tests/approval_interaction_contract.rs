//! Contract tests for [`ApprovalInteractionService`].
//!
//! Covers the acceptance criteria from issue #3094 Slice 1:
//! - list pending approvals (scope-filtered)
//! - approve / deny route through the decision port
//! - missing approval, stale approval, cross-scope denial
//! - redaction sentinel (no-exposure for raw input, reason, fingerprint)

use std::sync::Mutex;

use async_trait::async_trait;
use ironclaw_approvals::{ApprovalResolutionError, DenyApproval, LeaseApproval};
use ironclaw_host_api::{
    Action, ApprovalRequest, ApprovalRequestId, CapabilityId, CorrelationId, EffectKind,
    ExtensionId, InvocationFingerprint, InvocationId, MountView, NetworkPolicy, Principal,
    ProjectId, ResourceEstimate, ResourceScope, SecretHandle, SecretUseMode, TenantId, UserId,
};
use ironclaw_interactions::approval::{
    ApprovalDecisionPort, ApprovalInteractionError, ApprovalInteractionService,
    PendingApprovalSummary,
};
use ironclaw_run_state::{ApprovalRequestStore, InMemoryApprovalRequestStore};

/// Records every approve/deny call so tests can assert the interaction
/// service routed through the port without coupling to lease internals.
#[derive(Default)]
struct RecordingDecisionPort {
    approves: Mutex<Vec<(ResourceScope, ApprovalRequestId, LeaseApproval)>>,
    denies: Mutex<Vec<(ResourceScope, ApprovalRequestId, DenyApproval)>>,
    fail_with: Mutex<Option<ApprovalResolutionError>>,
}

impl RecordingDecisionPort {
    fn approves(&self) -> Vec<(ResourceScope, ApprovalRequestId, LeaseApproval)> {
        self.approves.lock().unwrap().clone()
    }

    fn denies(&self) -> Vec<(ResourceScope, ApprovalRequestId, DenyApproval)> {
        self.denies.lock().unwrap().clone()
    }

    fn set_failure(&self, err: ApprovalResolutionError) {
        *self.fail_with.lock().unwrap() = Some(err);
    }
}

#[async_trait]
impl ApprovalDecisionPort for RecordingDecisionPort {
    async fn approve_dispatch(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
        approval: LeaseApproval,
    ) -> Result<(), ApprovalResolutionError> {
        if let Some(err) = self.fail_with.lock().unwrap().take() {
            return Err(err);
        }
        self.approves
            .lock()
            .unwrap()
            .push((scope.clone(), request_id, approval));
        Ok(())
    }

    async fn deny(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
        denial: DenyApproval,
    ) -> Result<(), ApprovalResolutionError> {
        if let Some(err) = self.fail_with.lock().unwrap().take() {
            return Err(err);
        }
        self.denies
            .lock()
            .unwrap()
            .push((scope.clone(), request_id, denial));
        Ok(())
    }
}

#[tokio::test]
async fn list_pending_returns_only_pending_records_for_scope() {
    let store = InMemoryApprovalRequestStore::new();
    let port = RecordingDecisionPort::default();
    let service = ApprovalInteractionService::new(&store, &port);

    let scope_a = scope("tenant1", "user1");
    let scope_b = scope("tenant1", "user2");

    let pending = save_pending_dispatch(&store, &scope_a, "echo.say").await;
    let approved = save_pending_dispatch(&store, &scope_a, "memory.read").await;
    store.approve(&scope_a, approved.request.id).await.unwrap();
    let _other_scope = save_pending_dispatch(&store, &scope_b, "echo.say").await;

    let listed = service.list_pending(&scope_a).await.unwrap();
    assert_eq!(
        listed.len(),
        1,
        "only the pending record in scope is listed"
    );
    assert_eq!(listed[0].request_id, pending.request.id);
    assert_eq!(listed[0].capability, capability("echo.say"));
}

#[tokio::test]
async fn list_pending_skips_records_whose_action_is_not_capability_shaped() {
    let store = InMemoryApprovalRequestStore::new();
    let port = RecordingDecisionPort::default();
    let service = ApprovalInteractionService::new(&store, &port);

    let scope = scope("tenant1", "user1");
    save_pending_dispatch(&store, &scope, "echo.say").await;
    save_pending_use_secret(&store, &scope).await;

    let listed = service.list_pending(&scope).await.unwrap();
    assert_eq!(listed.len(), 1, "non-capability action is excluded");
    assert_eq!(listed[0].capability, capability("echo.say"));
}

#[tokio::test]
async fn list_pending_redacts_raw_inputs_reasons_and_fingerprints() {
    // The PendingApprovalSummary DTO has a fixed shape — its fields are
    // `request_id`, `capability`, `requested_by`. Any future addition that
    // would leak input/reason/fingerprint/lease/path/secret/output would
    // require either changing this assertion or adding the field to the
    // struct; the contract docs both forbid. We pin the shape by checking
    // the field count via destructuring.
    let store = InMemoryApprovalRequestStore::new();
    let port = RecordingDecisionPort::default();
    let service = ApprovalInteractionService::new(&store, &port);
    let scope = scope("tenant1", "user1");

    // Plant a reason string with a sentinel that must not appear anywhere
    // in the returned summary's `Debug` output.
    let record =
        save_pending_with_reason(&store, &scope, "echo.say", "RAW_SENSITIVE_REASON_a3f7c9d4").await;

    let listed = service.list_pending(&scope).await.unwrap();
    let PendingApprovalSummary {
        request_id,
        capability,
        requested_by,
    } = &listed[0];
    assert_eq!(*request_id, record.request.id);
    assert_eq!(*capability, self::capability("echo.say"));
    assert!(matches!(requested_by, Principal::Extension(_)));

    let rendered = format!("{:?}", listed);
    assert!(
        !rendered.contains("RAW_SENSITIVE_REASON_a3f7c9d4"),
        "sentinel from reason leaked into summary debug output: {rendered}"
    );
}

#[tokio::test]
async fn approve_routes_through_decision_port() {
    let store = InMemoryApprovalRequestStore::new();
    let port = RecordingDecisionPort::default();
    let service = ApprovalInteractionService::new(&store, &port);
    let scope = scope("tenant1", "user1");
    let record = save_pending_dispatch(&store, &scope, "echo.say").await;

    service
        .approve(&scope, record.request.id, lease_approval(&scope))
        .await
        .unwrap();

    let calls = port.approves();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, scope);
    assert_eq!(calls[0].1, record.request.id);
    assert_eq!(port.denies().len(), 0);
}

#[tokio::test]
async fn deny_routes_through_decision_port() {
    let store = InMemoryApprovalRequestStore::new();
    let port = RecordingDecisionPort::default();
    let service = ApprovalInteractionService::new(&store, &port);
    let scope = scope("tenant1", "user1");
    let record = save_pending_dispatch(&store, &scope, "echo.say").await;

    service
        .deny(
            &scope,
            record.request.id,
            DenyApproval {
                denied_by: Principal::User(scope.user_id.clone()),
            },
        )
        .await
        .unwrap();

    let calls = port.denies();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, scope);
    assert_eq!(calls[0].1, record.request.id);
    assert_eq!(port.approves().len(), 0);
}

#[tokio::test]
async fn approve_returns_unknown_for_missing_request() {
    let store = InMemoryApprovalRequestStore::new();
    let port = RecordingDecisionPort::default();
    let service = ApprovalInteractionService::new(&store, &port);
    let scope = scope("tenant1", "user1");
    let bogus_id = ApprovalRequestId::new();

    let err = service
        .approve(&scope, bogus_id, lease_approval(&scope))
        .await
        .unwrap_err();
    assert!(matches!(err, ApprovalInteractionError::Unknown));
    assert_eq!(
        port.approves().len(),
        0,
        "decision port must not be called for an unknown request"
    );
}

#[tokio::test]
async fn approve_returns_unknown_for_cross_scope_request() {
    let store = InMemoryApprovalRequestStore::new();
    let port = RecordingDecisionPort::default();
    let service = ApprovalInteractionService::new(&store, &port);
    let scope_a = scope("tenant1", "user1");
    let scope_b = scope("tenant1", "user2");
    let record = save_pending_dispatch(&store, &scope_a, "echo.say").await;

    // Attacker is in scope_b and tries to approve scope_a's request.
    let err = service
        .approve(&scope_b, record.request.id, lease_approval(&scope_b))
        .await
        .unwrap_err();
    assert!(matches!(err, ApprovalInteractionError::Unknown));
    assert_eq!(port.approves().len(), 0);
}

#[tokio::test]
async fn approve_returns_not_pending_for_already_approved_request() {
    let store = InMemoryApprovalRequestStore::new();
    let port = RecordingDecisionPort::default();
    let service = ApprovalInteractionService::new(&store, &port);
    let scope = scope("tenant1", "user1");
    let record = save_pending_dispatch(&store, &scope, "echo.say").await;
    store.approve(&scope, record.request.id).await.unwrap();

    let err = service
        .approve(&scope, record.request.id, lease_approval(&scope))
        .await
        .unwrap_err();
    assert!(matches!(err, ApprovalInteractionError::NotPending));
    assert_eq!(port.approves().len(), 0);
}

#[tokio::test]
async fn approve_surfaces_resolver_failures_as_stable_errors() {
    let store = InMemoryApprovalRequestStore::new();
    let port = RecordingDecisionPort::default();
    let service = ApprovalInteractionService::new(&store, &port);
    let scope = scope("tenant1", "user1");
    let record = save_pending_dispatch(&store, &scope, "echo.say").await;

    port.set_failure(ApprovalResolutionError::MissingInvocationFingerprint);
    let err = service
        .approve(&scope, record.request.id, lease_approval(&scope))
        .await
        .unwrap_err();
    assert!(matches!(err, ApprovalInteractionError::Incomplete));
}

// --- Fixture helpers --------------------------------------------------------

mod approval_request_fixtures {
    use super::*;

    pub(super) async fn save_pending_dispatch(
        store: &InMemoryApprovalRequestStore,
        scope: &ResourceScope,
        capability_id: &str,
    ) -> ironclaw_run_state::ApprovalRecord {
        save_pending_with_reason(store, scope, capability_id, "approval reason").await
    }

    pub(super) async fn save_pending_with_reason(
        store: &InMemoryApprovalRequestStore,
        scope: &ResourceScope,
        capability_id: &str,
        reason: &str,
    ) -> ironclaw_run_state::ApprovalRecord {
        let cap = capability(capability_id);
        let request = ApprovalRequest {
            id: ApprovalRequestId::new(),
            correlation_id: CorrelationId::new(),
            requested_by: Principal::Extension(ExtensionId::new("caller").unwrap()),
            action: Box::new(Action::Dispatch {
                capability: cap.clone(),
                estimated_resources: ResourceEstimate::default(),
            }),
            reason: reason.to_string(),
            reusable_scope: None,
            invocation_fingerprint: Some(
                InvocationFingerprint::for_dispatch(
                    scope,
                    &cap,
                    &ResourceEstimate::default(),
                    &serde_json::json!({"sentinel": "RAW_SENSITIVE_INPUT_b1d6e8f2"}),
                )
                .unwrap(),
            ),
        };
        store.save_pending(scope.clone(), request).await.unwrap()
    }

    pub(super) async fn save_pending_use_secret(
        store: &InMemoryApprovalRequestStore,
        scope: &ResourceScope,
    ) -> ironclaw_run_state::ApprovalRecord {
        let request = ApprovalRequest {
            id: ApprovalRequestId::new(),
            correlation_id: CorrelationId::new(),
            requested_by: Principal::Extension(ExtensionId::new("caller").unwrap()),
            action: Box::new(Action::UseSecret {
                handle: SecretHandle::new("api-key").unwrap(),
                mode: SecretUseMode::InjectIntoRequest,
            }),
            reason: "secret access".to_string(),
            reusable_scope: None,
            invocation_fingerprint: None,
        };
        store.save_pending(scope.clone(), request).await.unwrap()
    }
}
use approval_request_fixtures::{
    save_pending_dispatch, save_pending_use_secret, save_pending_with_reason,
};

fn scope(tenant: &str, user: &str) -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new(tenant).unwrap(),
        user_id: UserId::new(user).unwrap(),
        agent_id: None,
        project_id: Some(ProjectId::new("project1").unwrap()),
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    }
}

fn capability(id: &str) -> CapabilityId {
    CapabilityId::new(id).unwrap()
}

fn lease_approval(scope: &ResourceScope) -> LeaseApproval {
    LeaseApproval {
        issued_by: Principal::User(scope.user_id.clone()),
        allowed_effects: vec![EffectKind::DispatchCapability],
        mounts: MountView::default(),
        network: NetworkPolicy::default(),
        secrets: Vec::new(),
        resource_ceiling: None,
        expires_at: None,
        max_invocations: Some(1),
    }
}
