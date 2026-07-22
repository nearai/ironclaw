mod support;

use support::legacy_capability_fixture_to_v2;

use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_approvals::{
    FilesystemPersistentApprovalPolicyStore, PersistentApprovalAction, PersistentApprovalPolicy,
    PersistentApprovalPolicyError, PersistentApprovalPolicyInput, PersistentApprovalPolicyKey,
    PersistentApprovalPolicyStore, test_support::in_memory_backed_persistent_approval_policy_store,
};
use ironclaw_authorization::{GrantAuthorizer, TrustAwareCapabilityDispatchAuthorizer};
use ironclaw_extensions::{ExtensionManifest, ExtensionPackage, ExtensionRegistry, ManifestSource};
use ironclaw_filesystem::{InMemoryBackend, ScopedFilesystem};
use ironclaw_host_api::dispatch_test_support::TestDispatcher;
use ironclaw_host_api::*;
use ironclaw_host_runtime::{
    CapabilitySurfaceVersion, DefaultHostRuntime, HostRuntime, RuntimeCapabilityAuthResumeRequest,
    RuntimeCapabilityRequest, RuntimeFailureKind,
};
use ironclaw_processes::{
    ProcessError, ProcessManager, ProcessRecord, ProcessStart, ProcessStatus,
};
use ironclaw_run_state::{RunStart, RunStateStore, RunStatus};
use ironclaw_trust::{AdminConfig, AdminEntry, HostTrustAssignment, HostTrustPolicy};
use serde_json::json;

#[tokio::test]
async fn default_runtime_uses_persistent_policy_as_dispatch_authority() {
    let registry = Arc::new(registry_with_echo_capability());
    let dispatcher = Arc::new(TestDispatcher::ok(dispatch_result()));
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> = Arc::new(GrantAuthorizer);
    let run_state = Arc::new(ironclaw_run_state::in_memory_backed_run_state_store());
    let approval_requests = Arc::new(ironclaw_run_state::in_memory_backed_approval_request_store());
    let policies = Arc::new(in_memory_backed_persistent_approval_policy_store());
    let context = execution_context_without_grants();
    policies
        .allow(PersistentApprovalPolicyInput {
            scope: context.resource_scope.clone(),
            action: PersistentApprovalAction::Dispatch,
            capability_id: capability_id(),
            grantee: Principal::Extension(context.extension_id.clone()),
            approved_by: Principal::User(context.user_id.clone()),
            constraints: GrantConstraints {
                allowed_effects: vec![EffectKind::DispatchCapability],
                mounts: MountView::default(),
                network: NetworkPolicy::default(),
                secrets: Vec::new(),
                resource_ceiling: None,
                expires_at: None,
                max_invocations: Some(1),
            },
            source_approval_request_id: None,
        })
        .await
        .expect("seed persistent policy");
    let policy_store: Arc<dyn PersistentApprovalPolicyStore> = policies;

    let runtime = DefaultHostRuntime::new(
        registry,
        dispatcher.clone(),
        authorizer,
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
        local_test_runtime_policy(),
    )
    .with_trust_policy(Arc::new(local_manifest_trust_policy()))
    .with_run_state(run_state)
    .with_approval_requests(approval_requests)
    .with_persistent_approval_policies(policy_store);

    let outcome = runtime
        .invoke_capability(RuntimeCapabilityRequest::new(
            context,
            capability_id(),
            ResourceEstimate::default(),
            json!({"message": "hello"}),
        ))
        .await
        .unwrap();

    match outcome {
        ironclaw_host_runtime::RuntimeCapabilityOutcome::Completed(completed) => {
            assert_eq!(completed.capability_id, capability_id());
            assert_eq!(completed.output, json!({"ok": true}));
        }
        other => panic!("expected Completed outcome, got {:?}", other),
    }
    assert!(dispatcher.call_count() > 0);
}

// Regression: an auth-resume (BlockedAuth → credential supplied → resume) for a
// capability authorized only by a persistent-approval grant must re-apply that
// grant on the resume preflight, exactly as the initial dispatch did. Before the
// fix, `auth_resume_capability` skipped `apply_persistent_approval_policy`, so the
// resume re-authorized a grant-less context and was denied — the credential gate
// resumed only to fail authorization (observed when connecting Gmail: OAuth
// completed, but the `extension_activate` auth-resume failed `authorization`,
// while a later fresh dispatch succeeded). With `approval_request_id = None`
// there is no approval lease to carry the grant, so the persistent policy is the
// only authority and must be re-applied.
#[tokio::test]
async fn default_runtime_uses_persistent_policy_as_auth_resume_authority() {
    let registry = Arc::new(registry_with_echo_capability());
    let dispatcher = Arc::new(TestDispatcher::ok(dispatch_result()));
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> = Arc::new(GrantAuthorizer);
    let run_state = Arc::new(ironclaw_run_state::in_memory_backed_run_state_store());
    let approval_requests = Arc::new(ironclaw_run_state::in_memory_backed_approval_request_store());
    let policies = Arc::new(in_memory_backed_persistent_approval_policy_store());
    let context = execution_context_without_grants();
    let scope = context.resource_scope.clone();
    let invocation_id = context.invocation_id;
    policies
        .allow(PersistentApprovalPolicyInput {
            scope: context.resource_scope.clone(),
            action: PersistentApprovalAction::Dispatch,
            capability_id: capability_id(),
            grantee: Principal::Extension(context.extension_id.clone()),
            approved_by: Principal::User(context.user_id.clone()),
            constraints: GrantConstraints {
                allowed_effects: vec![EffectKind::DispatchCapability],
                mounts: MountView::default(),
                network: NetworkPolicy::default(),
                secrets: Vec::new(),
                resource_ceiling: None,
                expires_at: None,
                max_invocations: Some(1),
            },
            source_approval_request_id: None,
        })
        .await
        .expect("seed persistent policy");
    let policy_store: Arc<dyn PersistentApprovalPolicyStore> = policies;

    // Park the invocation in BlockedAuth, mirroring the state after the initial
    // dispatch raised AuthRequired and opened the credential gate.
    run_state
        .start(RunStart {
            invocation_id,
            capability_id: capability_id(),
            scope: scope.clone(),
            authenticated_actor_user_id: None,
        })
        .await
        .expect("seed running invocation");
    run_state
        .block_auth(&scope, invocation_id, "AuthRequired".to_string())
        .await
        .expect("park invocation in BlockedAuth");

    let runtime = DefaultHostRuntime::new(
        registry,
        dispatcher.clone(),
        authorizer,
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
        local_test_runtime_policy(),
    )
    .with_trust_policy(Arc::new(local_manifest_trust_policy()))
    .with_run_state(run_state.clone())
    .with_approval_requests(approval_requests)
    .with_persistent_approval_policies(policy_store);

    // Auth-resume carries approval_request_id = None: there is no approval lease,
    // so the persistent-approval grant is the only authority for the re-dispatch.
    let outcome = runtime
        .auth_resume_capability(RuntimeCapabilityAuthResumeRequest::new(
            context,
            capability_id(),
            ResourceEstimate::default(),
            json!({"message": "hello"}),
            None,
        ))
        .await
        .unwrap();

    match outcome {
        ironclaw_host_runtime::RuntimeCapabilityOutcome::Completed(completed) => {
            assert_eq!(completed.capability_id, capability_id());
            assert_eq!(completed.output, json!({"ok": true}));
        }
        other => panic!(
            "expected Completed auth-resume outcome (persistent grant re-applied), got {:?}",
            other
        ),
    }
    assert!(dispatcher.call_count() > 0);

    let resumed = run_state.get(&scope, invocation_id).await.unwrap().unwrap();
    assert_eq!(
        resumed.status,
        RunStatus::Completed,
        "auth-resume authorized by the persistent grant must complete the run"
    );
}

#[tokio::test]
async fn default_runtime_uses_user_grantee_persistent_policy_as_dispatch_authority() {
    let registry = Arc::new(registry_with_echo_capability());
    let dispatcher = Arc::new(TestDispatcher::ok(dispatch_result()));
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> = Arc::new(GrantAuthorizer);
    let run_state = Arc::new(ironclaw_run_state::in_memory_backed_run_state_store());
    let approval_requests = Arc::new(ironclaw_run_state::in_memory_backed_approval_request_store());
    let policies = Arc::new(in_memory_backed_persistent_approval_policy_store());
    let context = execution_context_without_grants();
    policies
        .allow(PersistentApprovalPolicyInput {
            scope: context.resource_scope.clone(),
            action: PersistentApprovalAction::Dispatch,
            capability_id: capability_id(),
            grantee: Principal::User(context.user_id.clone()),
            approved_by: Principal::User(context.user_id.clone()),
            constraints: GrantConstraints {
                allowed_effects: vec![EffectKind::DispatchCapability],
                mounts: MountView::default(),
                network: NetworkPolicy::default(),
                secrets: Vec::new(),
                resource_ceiling: None,
                expires_at: None,
                max_invocations: Some(1),
            },
            source_approval_request_id: None,
        })
        .await
        .expect("seed user persistent policy");
    let policy_store: Arc<dyn PersistentApprovalPolicyStore> = policies;

    let runtime = DefaultHostRuntime::new(
        registry,
        dispatcher.clone(),
        authorizer,
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
        local_test_runtime_policy(),
    )
    .with_trust_policy(Arc::new(local_manifest_trust_policy()))
    .with_run_state(run_state)
    .with_approval_requests(approval_requests)
    .with_persistent_approval_policies(policy_store);

    let outcome = runtime
        .invoke_capability(RuntimeCapabilityRequest::new(
            context,
            capability_id(),
            ResourceEstimate::default(),
            json!({"message": "hello"}),
        ))
        .await
        .unwrap();

    match outcome {
        ironclaw_host_runtime::RuntimeCapabilityOutcome::Completed(completed) => {
            assert_eq!(completed.capability_id, capability_id());
            assert_eq!(completed.output, json!({"ok": true}));
        }
        other => panic!("expected Completed outcome, got {:?}", other),
    }
    assert!(dispatcher.call_count() > 0);
}

#[tokio::test]
async fn default_runtime_uses_threadless_filesystem_policy_after_thread_change() {
    let registry = Arc::new(registry_with_echo_capability());
    let dispatcher = Arc::new(TestDispatcher::ok(dispatch_result()));
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> = Arc::new(GrantAuthorizer);
    let run_state = Arc::new(ironclaw_run_state::in_memory_backed_run_state_store());
    let approval_requests = Arc::new(ironclaw_run_state::in_memory_backed_approval_request_store());
    let scoped = scoped_approval_fs();
    let policies = Arc::new(FilesystemPersistentApprovalPolicyStore::new(Arc::clone(
        &scoped,
    )));
    let mut context = execution_context_without_grants();
    let original_thread = ThreadId::new("thread-original").unwrap();
    let current_thread = ThreadId::new("thread-current").unwrap();
    context.project_id = None;
    context.thread_id = Some(current_thread.clone());
    context.resource_scope.project_id = None;
    context.resource_scope.thread_id = Some(current_thread);

    let mut original_scope = context.resource_scope.clone();
    original_scope.thread_id = Some(original_thread);
    policies
        .allow(PersistentApprovalPolicyInput {
            scope: original_scope,
            action: PersistentApprovalAction::Dispatch,
            capability_id: capability_id(),
            grantee: Principal::Extension(context.extension_id.clone()),
            approved_by: Principal::User(context.user_id.clone()),
            constraints: GrantConstraints {
                allowed_effects: vec![EffectKind::DispatchCapability],
                mounts: MountView::default(),
                network: NetworkPolicy::default(),
                secrets: Vec::new(),
                resource_ceiling: None,
                expires_at: None,
                max_invocations: None,
            },
            source_approval_request_id: None,
        })
        .await
        .expect("seed persistent policy");
    let policy_store: Arc<dyn PersistentApprovalPolicyStore> = policies;

    let runtime = DefaultHostRuntime::new(
        registry,
        dispatcher.clone(),
        authorizer,
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
        local_test_runtime_policy(),
    )
    .with_trust_policy(Arc::new(local_manifest_trust_policy()))
    .with_run_state(run_state)
    .with_approval_requests(approval_requests)
    .with_persistent_approval_policies(policy_store);

    let outcome = runtime
        .invoke_capability(RuntimeCapabilityRequest::new(
            context,
            capability_id(),
            ResourceEstimate::default(),
            json!({"message": "hello"}),
        ))
        .await
        .unwrap();

    match outcome {
        ironclaw_host_runtime::RuntimeCapabilityOutcome::Completed(completed) => {
            assert_eq!(completed.capability_id, capability_id());
            assert_eq!(completed.output, json!({"ok": true}));
        }
        other => panic!("expected Completed outcome, got {:?}", other),
    }
    assert!(dispatcher.call_count() > 0);
}

#[tokio::test]
async fn default_runtime_does_not_replay_tenant_grantee_persistent_policy() {
    let registry = Arc::new(registry_with_echo_capability());
    let dispatcher = Arc::new(TestDispatcher::ok(dispatch_result()));
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> = Arc::new(GrantAuthorizer);
    let run_state = Arc::new(ironclaw_run_state::in_memory_backed_run_state_store());
    let approval_requests = Arc::new(ironclaw_run_state::in_memory_backed_approval_request_store());
    let policies = Arc::new(in_memory_backed_persistent_approval_policy_store());
    let context = execution_context_without_grants();
    policies
        .allow(PersistentApprovalPolicyInput {
            scope: context.resource_scope.clone(),
            action: PersistentApprovalAction::Dispatch,
            capability_id: capability_id(),
            grantee: Principal::Tenant(context.tenant_id.clone()),
            approved_by: Principal::User(context.user_id.clone()),
            constraints: GrantConstraints {
                allowed_effects: vec![EffectKind::DispatchCapability],
                mounts: MountView::default(),
                network: NetworkPolicy::default(),
                secrets: Vec::new(),
                resource_ceiling: None,
                expires_at: None,
                max_invocations: Some(1),
            },
            source_approval_request_id: None,
        })
        .await
        .expect("seed tenant persistent policy");
    let policy_store: Arc<dyn PersistentApprovalPolicyStore> = policies;

    let runtime = DefaultHostRuntime::new(
        registry,
        dispatcher.clone(),
        authorizer,
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
        local_test_runtime_policy(),
    )
    .with_trust_policy(Arc::new(local_manifest_trust_policy()))
    .with_run_state(run_state)
    .with_approval_requests(approval_requests)
    .with_persistent_approval_policies(policy_store);

    let outcome = runtime
        .invoke_capability(RuntimeCapabilityRequest::new(
            context,
            capability_id(),
            ResourceEstimate::default(),
            json!({"message": "hello"}),
        ))
        .await
        .unwrap();

    match outcome {
        ironclaw_host_runtime::RuntimeCapabilityOutcome::Failed(failure) => {
            assert_eq!(failure.capability_id, capability_id());
            assert_eq!(failure.kind, RuntimeFailureKind::Authorization);
        }
        other => panic!("expected authorization failure, got {:?}", other),
    }
    assert_eq!(dispatcher.call_count(), 0);
}

#[tokio::test]
async fn default_runtime_skips_unusable_persistent_policy_for_later_match() {
    let registry = Arc::new(registry_with_echo_capability());
    let dispatcher = Arc::new(TestDispatcher::ok(dispatch_result()));
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> = Arc::new(GrantAuthorizer);
    let run_state = Arc::new(ironclaw_run_state::in_memory_backed_run_state_store());
    let approval_requests = Arc::new(ironclaw_run_state::in_memory_backed_approval_request_store());
    let policies = Arc::new(in_memory_backed_persistent_approval_policy_store());
    let context = execution_context_without_grants();
    policies
        .allow(PersistentApprovalPolicyInput {
            scope: context.resource_scope.clone(),
            action: PersistentApprovalAction::Dispatch,
            capability_id: capability_id(),
            grantee: Principal::Extension(context.extension_id.clone()),
            approved_by: Principal::User(context.user_id.clone()),
            constraints: GrantConstraints {
                allowed_effects: Vec::new(),
                mounts: MountView::default(),
                network: NetworkPolicy::default(),
                secrets: Vec::new(),
                resource_ceiling: None,
                expires_at: None,
                max_invocations: Some(1),
            },
            source_approval_request_id: None,
        })
        .await
        .expect("seed unusable extension persistent policy");
    policies
        .allow(PersistentApprovalPolicyInput {
            scope: context.resource_scope.clone(),
            action: PersistentApprovalAction::Dispatch,
            capability_id: capability_id(),
            grantee: Principal::User(context.user_id.clone()),
            approved_by: Principal::User(context.user_id.clone()),
            constraints: GrantConstraints {
                allowed_effects: vec![EffectKind::DispatchCapability],
                mounts: MountView::default(),
                network: NetworkPolicy::default(),
                secrets: Vec::new(),
                resource_ceiling: None,
                expires_at: None,
                max_invocations: Some(1),
            },
            source_approval_request_id: None,
        })
        .await
        .expect("seed usable user persistent policy");
    let policy_store: Arc<dyn PersistentApprovalPolicyStore> = policies;

    let runtime = DefaultHostRuntime::new(
        registry,
        dispatcher.clone(),
        authorizer,
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
        local_test_runtime_policy(),
    )
    .with_trust_policy(Arc::new(local_manifest_trust_policy()))
    .with_run_state(run_state)
    .with_approval_requests(approval_requests)
    .with_persistent_approval_policies(policy_store);

    let outcome = runtime
        .invoke_capability(RuntimeCapabilityRequest::new(
            context,
            capability_id(),
            ResourceEstimate::default(),
            json!({"message": "hello"}),
        ))
        .await
        .unwrap();

    match outcome {
        ironclaw_host_runtime::RuntimeCapabilityOutcome::Completed(completed) => {
            assert_eq!(completed.capability_id, capability_id());
            assert_eq!(completed.output, json!({"ok": true}));
        }
        other => panic!("expected Completed outcome, got {:?}", other),
    }
    assert!(dispatcher.call_count() > 0);
}

#[tokio::test]
async fn default_runtime_falls_back_when_persistent_policy_lookup_fails() {
    let registry = Arc::new(registry_with_echo_capability());
    let dispatcher = Arc::new(TestDispatcher::ok(dispatch_result()));
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> = Arc::new(GrantAuthorizer);
    let run_state = Arc::new(ironclaw_run_state::in_memory_backed_run_state_store());
    let approval_requests = Arc::new(ironclaw_run_state::in_memory_backed_approval_request_store());
    let policies: Arc<dyn PersistentApprovalPolicyStore> =
        Arc::new(FailingLookupPersistentApprovalPolicyStore);
    let context = execution_context_without_grants();

    let runtime = DefaultHostRuntime::new(
        registry,
        dispatcher.clone(),
        authorizer,
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
        local_test_runtime_policy(),
    )
    .with_trust_policy(Arc::new(local_manifest_trust_policy()))
    .with_run_state(run_state)
    .with_approval_requests(approval_requests)
    .with_persistent_approval_policies(policies);

    let outcome = runtime
        .invoke_capability(RuntimeCapabilityRequest::new(
            context,
            capability_id(),
            ResourceEstimate::default(),
            json!({"message": "hello"}),
        ))
        .await
        .unwrap();

    match outcome {
        ironclaw_host_runtime::RuntimeCapabilityOutcome::Failed(failure) => {
            assert_eq!(failure.capability_id, capability_id());
            assert_eq!(failure.kind, RuntimeFailureKind::Authorization);
        }
        other => panic!("expected authorization failure, got {:?}", other),
    }
    assert_eq!(dispatcher.call_count(), 0);
}

#[tokio::test]
async fn default_runtime_reuses_persistent_policy_for_manifest_ask() {
    let registry = Arc::new(registry_with_echo_capability_permission("ask"));
    let dispatcher = Arc::new(TestDispatcher::ok(dispatch_result()));
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> = Arc::new(GrantAuthorizer);
    let run_state = Arc::new(ironclaw_run_state::in_memory_backed_run_state_store());
    let approval_requests = Arc::new(ironclaw_run_state::in_memory_backed_approval_request_store());
    let policies = Arc::new(in_memory_backed_persistent_approval_policy_store());
    let context = execution_context_without_grants();
    policies
        .allow(PersistentApprovalPolicyInput {
            scope: context.resource_scope.clone(),
            action: PersistentApprovalAction::Dispatch,
            capability_id: capability_id(),
            grantee: Principal::Extension(context.extension_id.clone()),
            approved_by: Principal::User(context.user_id.clone()),
            constraints: GrantConstraints {
                allowed_effects: vec![EffectKind::DispatchCapability],
                mounts: MountView::default(),
                network: NetworkPolicy::default(),
                secrets: Vec::new(),
                resource_ceiling: None,
                expires_at: None,
                max_invocations: Some(1),
            },
            source_approval_request_id: None,
        })
        .await
        .expect("seed persistent policy");
    let policy_store: Arc<dyn PersistentApprovalPolicyStore> = policies;

    let runtime = DefaultHostRuntime::new(
        registry,
        dispatcher.clone(),
        authorizer,
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
        local_test_runtime_policy(),
    )
    .with_trust_policy(Arc::new(local_manifest_trust_policy()))
    .with_run_state(run_state)
    .with_approval_requests(approval_requests)
    .with_persistent_approval_policies(policy_store);

    let outcome = runtime
        .invoke_capability(RuntimeCapabilityRequest::new(
            context,
            capability_id(),
            ResourceEstimate::default(),
            json!({"message": "hello"}),
        ))
        .await
        .unwrap();

    match outcome {
        ironclaw_host_runtime::RuntimeCapabilityOutcome::Completed(completed) => {
            assert_eq!(completed.capability_id, capability_id());
            assert_eq!(completed.output, json!({"ok": true}));
        }
        other => panic!("expected Completed outcome, got {:?}", other),
    }
    assert!(dispatcher.call_count() > 0);
}

#[tokio::test]
async fn default_runtime_skips_expired_persistent_policy() {
    let registry = Arc::new(registry_with_echo_capability());
    let dispatcher = Arc::new(TestDispatcher::ok(dispatch_result()));
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> = Arc::new(GrantAuthorizer);
    let run_state = Arc::new(ironclaw_run_state::in_memory_backed_run_state_store());
    let approval_requests = Arc::new(ironclaw_run_state::in_memory_backed_approval_request_store());
    let policies = Arc::new(in_memory_backed_persistent_approval_policy_store());
    let context = execution_context_without_grants();
    policies
        .allow(PersistentApprovalPolicyInput {
            scope: context.resource_scope.clone(),
            action: PersistentApprovalAction::Dispatch,
            capability_id: capability_id(),
            grantee: Principal::Extension(context.extension_id.clone()),
            approved_by: Principal::User(context.user_id.clone()),
            constraints: GrantConstraints {
                allowed_effects: vec![EffectKind::DispatchCapability],
                mounts: MountView::default(),
                network: NetworkPolicy::default(),
                secrets: Vec::new(),
                resource_ceiling: None,
                expires_at: Some(Utc::now() - chrono::Duration::seconds(1)),
                max_invocations: None,
            },
            source_approval_request_id: None,
        })
        .await
        .expect("seed expired persistent policy");
    let policy_store: Arc<dyn PersistentApprovalPolicyStore> = policies;

    let runtime = DefaultHostRuntime::new(
        registry,
        dispatcher.clone(),
        authorizer,
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
        local_test_runtime_policy(),
    )
    .with_trust_policy(Arc::new(local_manifest_trust_policy()))
    .with_run_state(run_state)
    .with_approval_requests(approval_requests)
    .with_persistent_approval_policies(policy_store);

    let outcome = runtime
        .invoke_capability(RuntimeCapabilityRequest::new(
            context,
            capability_id(),
            ResourceEstimate::default(),
            json!({"message": "hello"}),
        ))
        .await
        .unwrap();

    match outcome {
        ironclaw_host_runtime::RuntimeCapabilityOutcome::Failed(failure) => {
            assert_eq!(failure.capability_id, capability_id());
            assert_eq!(failure.kind, RuntimeFailureKind::Authorization);
        }
        other => panic!("expected authorization failure, got {:?}", other),
    }
    assert_eq!(dispatcher.call_count(), 0);
}

#[tokio::test]
async fn default_runtime_uses_persistent_policy_for_no_project_no_thread_scope() {
    // A fully unscoped context (no project, no thread) now yields a valid
    // (tenant, user, agent) persistent approval scope: the lookup proceeds and a
    // seeded "always allow" policy authorizes dispatch without a gate.
    let registry = Arc::new(registry_with_echo_capability());
    let dispatcher = Arc::new(TestDispatcher::ok(dispatch_result()));
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> = Arc::new(GrantAuthorizer);
    let run_state = Arc::new(ironclaw_run_state::in_memory_backed_run_state_store());
    let approval_requests = Arc::new(ironclaw_run_state::in_memory_backed_approval_request_store());
    let policies = Arc::new(in_memory_backed_persistent_approval_policy_store());
    let mut context = execution_context_without_grants();
    context.project_id = None;
    context.thread_id = None;
    context.resource_scope.project_id = None;
    context.resource_scope.thread_id = None;

    policies
        .allow(PersistentApprovalPolicyInput {
            scope: context.resource_scope.clone(),
            action: PersistentApprovalAction::Dispatch,
            capability_id: capability_id(),
            grantee: Principal::Extension(context.extension_id.clone()),
            approved_by: Principal::User(context.user_id.clone()),
            constraints: GrantConstraints {
                allowed_effects: vec![EffectKind::DispatchCapability],
                mounts: MountView::default(),
                network: NetworkPolicy::default(),
                secrets: Vec::new(),
                resource_ceiling: None,
                expires_at: None,
                max_invocations: None,
            },
            source_approval_request_id: None,
        })
        .await
        .expect("seed persistent policy");
    let policy_store: Arc<dyn PersistentApprovalPolicyStore> = policies;

    let runtime = DefaultHostRuntime::new(
        registry,
        dispatcher.clone(),
        authorizer,
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
        local_test_runtime_policy(),
    )
    .with_trust_policy(Arc::new(local_manifest_trust_policy()))
    .with_run_state(run_state)
    .with_approval_requests(approval_requests)
    .with_persistent_approval_policies(policy_store);

    let outcome = runtime
        .invoke_capability(RuntimeCapabilityRequest::new(
            context,
            capability_id(),
            ResourceEstimate::default(),
            json!({"message": "hello"}),
        ))
        .await
        .unwrap();

    match outcome {
        ironclaw_host_runtime::RuntimeCapabilityOutcome::Completed(completed) => {
            assert_eq!(completed.capability_id, capability_id());
            assert_eq!(completed.output, json!({"ok": true}));
        }
        other => panic!("expected Completed outcome, got {:?}", other),
    }
    assert!(dispatcher.call_count() > 0);
}

#[tokio::test]
async fn default_runtime_uses_persistent_policy_as_spawn_capability_authority() {
    let registry = Arc::new(registry_with_echo_capability());
    let dispatcher = Arc::new(TestDispatcher::ok(dispatch_result()));
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> = Arc::new(GrantAuthorizer);
    let run_state = Arc::new(ironclaw_run_state::in_memory_backed_run_state_store());
    let approval_requests = Arc::new(ironclaw_run_state::in_memory_backed_approval_request_store());
    let process_manager = Arc::new(RecordingProcessManager);
    let policies = Arc::new(in_memory_backed_persistent_approval_policy_store());
    let context = execution_context_without_grants();
    policies
        .allow(PersistentApprovalPolicyInput {
            scope: context.resource_scope.clone(),
            action: PersistentApprovalAction::SpawnCapability,
            capability_id: capability_id(),
            grantee: Principal::Extension(context.extension_id.clone()),
            approved_by: Principal::User(context.user_id.clone()),
            constraints: GrantConstraints {
                allowed_effects: vec![EffectKind::DispatchCapability, EffectKind::SpawnProcess],
                mounts: MountView::default(),
                network: NetworkPolicy::default(),
                secrets: Vec::new(),
                resource_ceiling: None,
                expires_at: None,
                max_invocations: Some(1),
            },
            source_approval_request_id: None,
        })
        .await
        .expect("seed spawn persistent policy");
    let policy_store: Arc<dyn PersistentApprovalPolicyStore> = policies;

    let runtime = DefaultHostRuntime::new(
        registry,
        dispatcher,
        authorizer,
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
        local_test_runtime_policy(),
    )
    .with_trust_policy(Arc::new(local_manifest_trust_policy_with_effects(vec![
        EffectKind::DispatchCapability,
        EffectKind::SpawnProcess,
    ])))
    .with_run_state(run_state)
    .with_approval_requests(approval_requests)
    .with_process_manager(process_manager)
    .with_persistent_approval_policies(policy_store);

    let outcome = runtime
        .spawn_capability(RuntimeCapabilityRequest::new(
            context,
            capability_id(),
            ResourceEstimate::default(),
            json!({"message": "hello"}),
        ))
        .await
        .unwrap();

    match outcome {
        ironclaw_host_runtime::RuntimeCapabilityOutcome::SpawnedProcess(spawned) => {
            assert_eq!(spawned.capability_id, capability_id());
        }
        other => panic!("expected SpawnedProcess outcome, got {:?}", other),
    }
}

fn local_test_runtime_policy() -> ironclaw_host_api::runtime_policy::EffectiveRuntimePolicy {
    ironclaw_runtime_policy::resolve(ironclaw_runtime_policy::ResolveRequest::new(
        ironclaw_host_api::runtime_policy::DeploymentMode::LocalSingleUser,
        ironclaw_host_api::runtime_policy::RuntimeProfile::LocalDev,
    ))
    .unwrap()
}

fn dispatch_result() -> CapabilityDispatchResult {
    CapabilityDispatchResult {
        capability_id: capability_id(),
        provider: extension_id(),
        runtime: RuntimeKind::Wasm,
        output: json!({"ok": true}),
        display_preview: None,
        usage: ResourceUsage::default(),
        receipt: ResourceReceipt {
            id: ResourceReservationId::new(),
            scope: ResourceScope::system(),
            status: ReservationStatus::Reconciled,
            estimate: ResourceEstimate::default(),
            actual: Some(ResourceUsage::default()),
        },
    }
}

struct RecordingProcessManager;

#[async_trait]
impl ProcessManager for RecordingProcessManager {
    async fn spawn(&self, start: ProcessStart) -> Result<ProcessRecord, ProcessError> {
        Ok(ProcessRecord {
            process_id: start.process_id,
            parent_process_id: start.parent_process_id,
            invocation_id: start.invocation_id,
            scope: start.scope,
            authenticated_actor_user_id: start.authenticated_actor_user_id,
            extension_id: start.extension_id,
            capability_id: start.capability_id,
            runtime: start.runtime,
            status: ProcessStatus::Running,
            grants: start.grants,
            mounts: start.mounts,
            estimated_resources: start.estimated_resources,
            resource_reservation_id: start.resource_reservation_id,
            authorized_continuation: start.authorized_continuation,
            error_kind: None,
        })
    }
}

struct FailingLookupPersistentApprovalPolicyStore;

#[async_trait]
impl PersistentApprovalPolicyStore for FailingLookupPersistentApprovalPolicyStore {
    async fn allow(
        &self,
        _input: PersistentApprovalPolicyInput,
    ) -> Result<PersistentApprovalPolicy, PersistentApprovalPolicyError> {
        Err(PersistentApprovalPolicyError::Filesystem(
            "lookup-only test store".to_string(),
        ))
    }

    async fn lookup(
        &self,
        _key: &PersistentApprovalPolicyKey,
    ) -> Result<Option<PersistentApprovalPolicy>, PersistentApprovalPolicyError> {
        Err(PersistentApprovalPolicyError::Filesystem(
            "policy lookup failed".to_string(),
        ))
    }

    async fn revoke(
        &self,
        _key: &PersistentApprovalPolicyKey,
    ) -> Result<PersistentApprovalPolicy, PersistentApprovalPolicyError> {
        Err(PersistentApprovalPolicyError::UnknownPolicy)
    }

    async fn revoke_if_source_approval_request(
        &self,
        _key: &PersistentApprovalPolicyKey,
        _source_approval_request_id: ApprovalRequestId,
    ) -> Result<Option<PersistentApprovalPolicy>, PersistentApprovalPolicyError> {
        Ok(None)
    }
}

fn registry_with_echo_capability() -> ExtensionRegistry {
    registry_with_echo_capability_permission("allow")
}

fn registry_with_echo_capability_permission(permission: &str) -> ExtensionRegistry {
    let manifest = format!(
        r#"
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
default_permission = "{permission}"
parameters_schema = {{}}
"#
    );
    let manifest = parse_manifest(&manifest);
    let package = ExtensionPackage::from_manifest(
        manifest,
        VirtualPath::new("/system/extensions/echo").unwrap(),
    )
    .unwrap();
    let mut registry = ExtensionRegistry::new();
    registry.insert(package).unwrap();
    registry
}

fn parse_manifest(manifest: &str) -> ExtensionManifest {
    let manifest = legacy_capability_fixture_to_v2(manifest);
    ExtensionManifest::parse(
        &manifest,
        ManifestSource::InstalledLocal,
        &HostPortCatalog::empty(),
        &capability_provider_contracts(),
    )
    .unwrap()
}

fn execution_context_without_grants() -> ExecutionContext {
    let mut context = ExecutionContext::local_default(
        UserId::new("user").unwrap(),
        ExtensionId::new("caller").unwrap(),
        RuntimeKind::Wasm,
        TrustClass::UserTrusted,
        CapabilitySet::default(),
        MountView::default(),
    )
    .unwrap();
    context.run_id = Some(RunId::new());
    context
}

fn scoped_approval_fs() -> Arc<ScopedFilesystem<InMemoryBackend>> {
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/approvals").unwrap(),
        VirtualPath::new("/approvals").unwrap(),
        MountPermissions {
            read: true,
            write: true,
            delete: false,
            list: true,
            execute: false,
        },
    )])
    .expect("approval mount");
    Arc::new(ScopedFilesystem::with_fixed_view(
        Arc::new(InMemoryBackend::new()),
        mounts,
    ))
}

fn local_manifest_trust_policy() -> HostTrustPolicy {
    local_manifest_trust_policy_with_effects(vec![EffectKind::DispatchCapability])
}

fn local_manifest_trust_policy_with_effects(allowed_effects: Vec<EffectKind>) -> HostTrustPolicy {
    HostTrustPolicy::new(vec![Box::new(AdminConfig::with_entries(vec![
        AdminEntry::for_local_manifest(
            PackageId::new("echo").unwrap(),
            "/system/extensions/echo/manifest.toml".to_string(),
            None,
            HostTrustAssignment::user_trusted(),
            allowed_effects,
            None,
        ),
    ]))])
    .unwrap()
}

fn capability_id() -> CapabilityId {
    CapabilityId::new("echo.say").unwrap()
}

fn extension_id() -> ExtensionId {
    ExtensionId::new("echo").unwrap()
}

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
