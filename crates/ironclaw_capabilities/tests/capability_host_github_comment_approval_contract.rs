use async_trait::async_trait;
use ironclaw_approvals::*;
use ironclaw_authorization::*;
use ironclaw_capabilities::*;
use ironclaw_host_api::*;
use ironclaw_run_state::*;
use serde_json::json;

mod support;
use support::*;

#[tokio::test]
async fn capability_host_blocks_github_comment_issue_before_dispatch() {
    let fixture = blocked_github_comment_fixture().await;

    assert_eq!(fixture.dispatcher.dispatch_count(), 0);
    assert!(!fixture.dispatcher.has_request());
    let run = fixture
        .run_state
        .get(&fixture.scope, fixture.invocation_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(run.status, RunStatus::BlockedApproval);
    let approval = fixture
        .approval_requests
        .get(&fixture.scope, fixture.approval_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(approval.status, ApprovalStatus::Pending);
    assert_eq!(
        approval.request.invocation_fingerprint,
        Some(
            InvocationFingerprint::for_dispatch(
                &fixture.scope,
                &github_comment_capability_id(),
                &fixture.estimate,
                &fixture.input
            )
            .unwrap()
        )
    );
}

#[tokio::test]
async fn capability_host_resumes_approved_github_comment_issue_and_dispatches_once() {
    let fixture = approved_github_comment_fixture().await;
    let obligation_handler = AllowAllObligationHandler;
    let resume_authorizer = GrantAuthorizer::new();
    let resume_host =
        CapabilityHost::new(&fixture.registry, &fixture.dispatcher, &resume_authorizer)
            .with_run_state(&fixture.run_state)
            .with_approval_requests(&fixture.approval_requests)
            .with_capability_leases(&fixture.leases)
            .with_obligation_handler(&obligation_handler);

    let result = resume_host
        .resume_json(CapabilityResumeRequest {
            context: fixture.context.clone(),
            approval_request_id: fixture.approval_id,
            capability_id: github_comment_capability_id(),
            estimate: fixture.estimate.clone(),
            input: fixture.input.clone(),
            trust_decision: github_comment_trust_decision(),
        })
        .await
        .unwrap();

    assert_eq!(result.dispatch.output, json!({"ok": true}));
    assert_eq!(fixture.dispatcher.dispatch_count(), 1);
    let request = fixture.dispatcher.take_request();
    assert_eq!(request.capability_id, github_comment_capability_id());
    assert_eq!(request.input, fixture.input);
    let run = fixture
        .run_state
        .get(&fixture.scope, fixture.invocation_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(run.status, RunStatus::Completed);
    let consumed = fixture
        .leases
        .get(
            &fixture.scope,
            fixture.lease_id.expect("approved fixture has a lease"),
        )
        .await
        .unwrap();
    assert_eq!(consumed.status, CapabilityLeaseStatus::Consumed);
}

#[tokio::test]
async fn capability_host_rejects_mutated_github_comment_issue_replay_before_dispatch_or_claim() {
    let fixture = approved_github_comment_fixture().await;
    let obligation_handler = AllowAllObligationHandler;
    let resume_authorizer = GrantAuthorizer::new();
    let resume_host =
        CapabilityHost::new(&fixture.registry, &fixture.dispatcher, &resume_authorizer)
            .with_run_state(&fixture.run_state)
            .with_approval_requests(&fixture.approval_requests)
            .with_capability_leases(&fixture.leases)
            .with_obligation_handler(&obligation_handler);

    let err = resume_host
        .resume_json(CapabilityResumeRequest {
            context: fixture.context.clone(),
            approval_request_id: fixture.approval_id,
            capability_id: github_comment_capability_id(),
            estimate: fixture.estimate.clone(),
            input: json!({
                "owner": "nearai",
                "repo": "ironclaw",
                "issue_number": 3806,
                "body": "mutated approved comment"
            }),
            trust_decision: github_comment_trust_decision(),
        })
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        CapabilityInvocationError::ApprovalFingerprintMismatch { .. }
    ));
    assert_eq!(fixture.dispatcher.dispatch_count(), 0);
    assert!(!fixture.dispatcher.has_request());
    let active = fixture
        .leases
        .get(
            &fixture.scope,
            fixture.lease_id.expect("approved fixture has a lease"),
        )
        .await
        .unwrap();
    assert_eq!(active.status, CapabilityLeaseStatus::Active);
}

struct GitHubCommentApprovalFixture {
    registry: ironclaw_extensions::ExtensionRegistry,
    dispatcher: RecordingDispatcher,
    run_state: InMemoryRunStateStore,
    approval_requests: InMemoryApprovalRequestStore,
    leases: InMemoryCapabilityLeaseStore,
    context: ExecutionContext,
    scope: ResourceScope,
    invocation_id: InvocationId,
    estimate: ResourceEstimate,
    input: serde_json::Value,
    approval_id: ApprovalRequestId,
    lease_id: Option<CapabilityGrantId>,
}

async fn blocked_github_comment_fixture() -> GitHubCommentApprovalFixture {
    let registry = registry_with_github_comment_capability();
    let dispatcher = RecordingDispatcher::default();
    let run_state = InMemoryRunStateStore::new();
    let approval_requests = InMemoryApprovalRequestStore::new();
    let leases = InMemoryCapabilityLeaseStore::new();
    let block_host = CapabilityHost::new(&registry, &dispatcher, &ApprovalAuthorizer)
        .with_run_state(&run_state)
        .with_approval_requests(&approval_requests);
    let context = execution_context(CapabilitySet::default());
    let scope = context.resource_scope.clone();
    let invocation_id = context.invocation_id;
    let estimate = ResourceEstimate::default();
    let input = json!({
        "owner": "nearai",
        "repo": "ironclaw",
        "issue_number": 3806,
        "body": "approved comment"
    });

    let err = block_host
        .invoke_json(CapabilityInvocationRequest {
            context: context.clone(),
            capability_id: github_comment_capability_id(),
            estimate: estimate.clone(),
            input: input.clone(),
            trust_decision: github_comment_trust_decision(),
        })
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        CapabilityInvocationError::AuthorizationRequiresApproval { .. }
    ));
    let approval_id = run_state
        .get(&scope, invocation_id)
        .await
        .unwrap()
        .unwrap()
        .approval_request_id
        .unwrap();

    GitHubCommentApprovalFixture {
        registry,
        dispatcher,
        run_state,
        approval_requests,
        leases,
        context,
        scope,
        invocation_id,
        estimate,
        input,
        approval_id,
        lease_id: None,
    }
}

async fn approved_github_comment_fixture() -> GitHubCommentApprovalFixture {
    let mut fixture = blocked_github_comment_fixture().await;
    let lease = ApprovalResolver::new(&fixture.approval_requests, &fixture.leases)
        .approve_dispatch(
            &fixture.scope,
            fixture.approval_id,
            LeaseApproval {
                issued_by: Principal::HostRuntime,
                constraints: GrantConstraints {
                    allowed_effects: github_comment_effects(),
                    mounts: MountView::default(),
                    network: NetworkPolicy::default(),
                    secrets: vec![SecretHandle::new("github_token").unwrap()],
                    resource_ceiling: None,
                    expires_at: None,
                    max_invocations: Some(1),
                },
            },
        )
        .await
        .unwrap();
    fixture.lease_id = Some(lease.grant.id);
    fixture
}

fn github_comment_trust_decision() -> ironclaw_trust::TrustDecision {
    trust_decision_with_effects(github_comment_effects())
}

fn github_comment_effects() -> Vec<EffectKind> {
    vec![
        EffectKind::DispatchCapability,
        EffectKind::Network,
        EffectKind::UseSecret,
        EffectKind::ExternalWrite,
    ]
}

struct AllowAllObligationHandler;

#[async_trait]
impl CapabilityObligationHandler for AllowAllObligationHandler {
    async fn satisfy(
        &self,
        _request: CapabilityObligationRequest<'_>,
    ) -> Result<(), CapabilityObligationError> {
        Ok(())
    }
}
