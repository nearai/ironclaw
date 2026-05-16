use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_authorization::*;
use ironclaw_filesystem::{DirEntry, FileStat, FilesystemError, LocalFilesystem, RootFilesystem};
use ironclaw_host_api::*;
use ironclaw_trust::{AuthorityCeiling, EffectiveTrustClass, TrustDecision, TrustProvenance};

#[tokio::test]
async fn lease_authorizer_allows_matching_active_lease_without_context_grant() {
    let leases = InMemoryCapabilityLeaseStore::new();
    let context = execution_context(CapabilitySet::default());
    let descriptor = descriptor(CapabilityId::new("echo.say").unwrap());

    let lease = CapabilityLease::new(
        context.resource_scope.clone(),
        grant_for(
            descriptor.id.clone(),
            Principal::Extension(context.extension_id.clone()),
            vec![EffectKind::DispatchCapability],
        ),
    );
    leases.issue(lease.clone()).await.unwrap();

    let authorizer = LeaseBackedAuthorizer::new(&leases);
    let decision = authorizer
        .authorize_dispatch(&context, &descriptor, &ResourceEstimate::default())
        .await;

    assert!(matches!(decision, Decision::Allow { .. }));
    assert_eq!(
        leases
            .get(&context.resource_scope, lease.grant.id)
            .await
            .unwrap(),
        lease
    );
}

#[tokio::test]
async fn lease_backed_authorizer_with_trust_denies_when_authority_ceiling_excludes_effect() {
    let leases = InMemoryCapabilityLeaseStore::new();
    let context = execution_context(CapabilitySet::default());
    let capability = CapabilityId::new("echo.say").unwrap();
    let mut descriptor = descriptor(capability.clone());
    descriptor.effects = vec![EffectKind::DispatchCapability, EffectKind::Network];

    leases
        .issue(CapabilityLease::new(
            context.resource_scope.clone(),
            grant_for(
                capability,
                Principal::Extension(context.extension_id.clone()),
                vec![EffectKind::DispatchCapability, EffectKind::Network],
            ),
        ))
        .await
        .unwrap();
    let trust = trust_decision(vec![EffectKind::DispatchCapability], None);

    let decision = LeaseBackedAuthorizer::new(&leases)
        .authorize_dispatch_with_trust(&context, &descriptor, &ResourceEstimate::default(), &trust)
        .await;

    assert_eq!(
        decision,
        Decision::Deny {
            reason: DenyReason::PolicyDenied
        }
    );
}

#[tokio::test]
async fn fingerprinted_approval_lease_does_not_authorize_plain_dispatch() {
    let leases = InMemoryCapabilityLeaseStore::new();
    let context = execution_context(CapabilitySet::default());
    let descriptor = descriptor(CapabilityId::new("echo.say").unwrap());
    let fingerprint = InvocationFingerprint::for_dispatch(
        &context.resource_scope,
        &descriptor.id,
        &ResourceEstimate::default(),
        &serde_json::json!({"message": "approved"}),
    )
    .unwrap();
    let mut lease = CapabilityLease::new(
        context.resource_scope.clone(),
        grant_for(
            descriptor.id.clone(),
            Principal::Extension(context.extension_id.clone()),
            vec![EffectKind::DispatchCapability],
        ),
    );
    lease.invocation_fingerprint = Some(fingerprint);
    leases.issue(lease).await.unwrap();

    let authorizer = LeaseBackedAuthorizer::new(&leases);
    let decision = authorizer
        .authorize_dispatch(&context, &descriptor, &ResourceEstimate::default())
        .await;

    assert!(matches!(
        decision,
        Decision::Deny {
            reason: DenyReason::MissingGrant
        }
    ));
}

#[tokio::test]
async fn claim_marks_fingerprinted_lease_claimed_and_hides_it_from_authorizer() {
    let leases = InMemoryCapabilityLeaseStore::new();
    let context = execution_context(CapabilitySet::default());
    let descriptor = descriptor(CapabilityId::new("echo.say").unwrap());
    let fingerprint = InvocationFingerprint::for_dispatch(
        &context.resource_scope,
        &descriptor.id,
        &ResourceEstimate::default(),
        &serde_json::json!({"message": "approved"}),
    )
    .unwrap();
    let mut lease = CapabilityLease::new(
        context.resource_scope.clone(),
        grant_for(
            descriptor.id.clone(),
            Principal::Extension(context.extension_id.clone()),
            vec![EffectKind::DispatchCapability],
        ),
    );
    lease.invocation_fingerprint = Some(fingerprint.clone());
    let lease_id = lease.grant.id;
    leases.issue(lease).await.unwrap();

    let claimed = leases
        .claim(&context.resource_scope, lease_id, &fingerprint)
        .await
        .unwrap();

    assert_eq!(claimed.status, CapabilityLeaseStatus::Claimed);
    let authorizer = LeaseBackedAuthorizer::new(&leases);
    let decision = authorizer
        .authorize_dispatch(&context, &descriptor, &ResourceEstimate::default())
        .await;
    assert!(matches!(
        decision,
        Decision::Deny {
            reason: DenyReason::MissingGrant
        }
    ));
}

#[tokio::test]
async fn claim_rejects_fingerprint_mismatch_without_mutating_lease() {
    let leases = InMemoryCapabilityLeaseStore::new();
    let context = execution_context(CapabilitySet::default());
    let descriptor = descriptor(CapabilityId::new("echo.say").unwrap());
    let fingerprint = InvocationFingerprint::for_dispatch(
        &context.resource_scope,
        &descriptor.id,
        &ResourceEstimate::default(),
        &serde_json::json!({"message": "approved"}),
    )
    .unwrap();
    let other_fingerprint = InvocationFingerprint::for_dispatch(
        &context.resource_scope,
        &descriptor.id,
        &ResourceEstimate::default(),
        &serde_json::json!({"message": "tampered"}),
    )
    .unwrap();
    let mut lease = CapabilityLease::new(
        context.resource_scope.clone(),
        grant_for(
            descriptor.id.clone(),
            Principal::Extension(context.extension_id.clone()),
            vec![EffectKind::DispatchCapability],
        ),
    );
    lease.invocation_fingerprint = Some(fingerprint);
    let lease_id = lease.grant.id;
    leases.issue(lease).await.unwrap();

    let err = leases
        .claim(&context.resource_scope, lease_id, &other_fingerprint)
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        CapabilityLeaseError::FingerprintMismatch { lease_id: id } if id == lease_id
    ));
    assert_eq!(
        leases
            .get(&context.resource_scope, lease_id)
            .await
            .unwrap()
            .status,
        CapabilityLeaseStatus::Active
    );
}

#[tokio::test]
async fn lease_authorizer_hides_leases_across_tenant_scope() {
    let leases = InMemoryCapabilityLeaseStore::new();
    let context = execution_context(CapabilitySet::default());
    let other_scope = ResourceScope {
        tenant_id: TenantId::new("tenant2").unwrap(),
        user_id: context.resource_scope.user_id.clone(),
        agent_id: None,
        project_id: context.resource_scope.project_id.clone(),
        mission_id: None,
        thread_id: None,
        invocation_id: context.invocation_id,
    };
    let descriptor = descriptor(CapabilityId::new("echo.say").unwrap());

    leases
        .issue(CapabilityLease::new(
            other_scope,
            grant_for(
                descriptor.id.clone(),
                Principal::Extension(context.extension_id.clone()),
                vec![EffectKind::DispatchCapability],
            ),
        ))
        .await
        .unwrap();

    let authorizer = LeaseBackedAuthorizer::new(&leases);
    let decision = authorizer
        .authorize_dispatch(&context, &descriptor, &ResourceEstimate::default())
        .await;

    assert!(matches!(
        decision,
        Decision::Deny {
            reason: DenyReason::MissingGrant
        }
    ));
}

#[tokio::test]
async fn lease_store_hides_leases_across_agent_scope() {
    let leases = InMemoryCapabilityLeaseStore::new();
    let mut context = execution_context(CapabilitySet::default());
    context.agent_id = Some(AgentId::new("agent-a").unwrap());
    context.resource_scope.agent_id = context.agent_id.clone();
    let mut other_scope = context.resource_scope.clone();
    other_scope.agent_id = Some(AgentId::new("agent-b").unwrap());
    let descriptor = descriptor(CapabilityId::new("echo.say").unwrap());
    let lease = CapabilityLease::new(
        context.resource_scope.clone(),
        grant_for(
            descriptor.id.clone(),
            Principal::Extension(context.extension_id.clone()),
            vec![EffectKind::DispatchCapability],
        ),
    );
    let lease_id = lease.grant.id;
    leases.issue(lease).await.unwrap();

    assert!(leases.get(&other_scope, lease_id).await.is_none());
    assert_eq!(leases.leases_for_scope(&other_scope).await, Vec::new());
    assert!(matches!(
        leases.revoke(&other_scope, lease_id).await.unwrap_err(),
        CapabilityLeaseError::UnknownLease { .. }
    ));
}

#[tokio::test]
async fn lease_store_hides_leases_across_project_scope() {
    let leases = InMemoryCapabilityLeaseStore::new();
    let context = execution_context(CapabilitySet::default());
    let mut other_scope = context.resource_scope.clone();
    other_scope.project_id = Some(ProjectId::new("project2").unwrap());
    let descriptor = descriptor(CapabilityId::new("echo.say").unwrap());
    let lease = CapabilityLease::new(
        context.resource_scope.clone(),
        grant_for(
            descriptor.id.clone(),
            Principal::Extension(context.extension_id.clone()),
            vec![EffectKind::DispatchCapability],
        ),
    );
    let lease_id = lease.grant.id;
    leases.issue(lease).await.unwrap();

    assert!(leases.get(&other_scope, lease_id).await.is_none());
    assert_eq!(leases.leases_for_scope(&other_scope).await, Vec::new());
    assert!(matches!(
        leases.revoke(&other_scope, lease_id).await.unwrap_err(),
        CapabilityLeaseError::UnknownLease { .. }
    ));
}

#[tokio::test]
async fn lease_authorizer_denies_other_agent_context() {
    let leases = InMemoryCapabilityLeaseStore::new();
    let mut context = execution_context(CapabilitySet::default());
    context.agent_id = Some(AgentId::new("agent-a").unwrap());
    context.resource_scope.agent_id = context.agent_id.clone();
    let descriptor = descriptor(CapabilityId::new("echo.say").unwrap());
    leases
        .issue(CapabilityLease::new(
            context.resource_scope.clone(),
            grant_for(
                descriptor.id.clone(),
                Principal::Extension(context.extension_id.clone()),
                vec![EffectKind::DispatchCapability],
            ),
        ))
        .await
        .unwrap();

    let mut other_context = context.clone();
    other_context.agent_id = Some(AgentId::new("agent-b").unwrap());
    other_context.resource_scope.agent_id = other_context.agent_id.clone();

    let authorizer = LeaseBackedAuthorizer::new(&leases);
    let decision = authorizer
        .authorize_dispatch(&other_context, &descriptor, &ResourceEstimate::default())
        .await;

    assert!(matches!(
        decision,
        Decision::Deny {
            reason: DenyReason::MissingGrant
        }
    ));
}

#[tokio::test]
async fn revocation_is_scoped_to_tenant_and_user() {
    let leases = InMemoryCapabilityLeaseStore::new();
    let context = execution_context(CapabilitySet::default());
    let other_scope = ResourceScope {
        tenant_id: TenantId::new("tenant2").unwrap(),
        user_id: context.resource_scope.user_id.clone(),
        agent_id: None,
        project_id: context.resource_scope.project_id.clone(),
        mission_id: None,
        thread_id: None,
        invocation_id: context.invocation_id,
    };
    let descriptor = descriptor(CapabilityId::new("echo.say").unwrap());
    let lease = CapabilityLease::new(
        context.resource_scope.clone(),
        grant_for(
            descriptor.id.clone(),
            Principal::Extension(context.extension_id.clone()),
            vec![EffectKind::DispatchCapability],
        ),
    );
    let lease_id = lease.grant.id;
    leases.issue(lease.clone()).await.unwrap();

    let err = leases.revoke(&other_scope, lease_id).await.unwrap_err();

    assert!(matches!(
        err,
        CapabilityLeaseError::UnknownLease { lease_id: id } if id == lease_id
    ));
    assert_eq!(
        leases
            .get(&context.resource_scope, lease_id)
            .await
            .unwrap()
            .status,
        CapabilityLeaseStatus::Active
    );
}

#[tokio::test]
async fn lease_authorizer_denies_invalid_context_before_grant_match() {
    let leases = InMemoryCapabilityLeaseStore::new();
    let mut context = execution_context(CapabilitySet::default());
    context.tenant_id = TenantId::new("tenant2").unwrap();
    let descriptor = descriptor(CapabilityId::new("echo.say").unwrap());
    leases
        .issue(CapabilityLease::new(
            context.resource_scope.clone(),
            grant_for(
                descriptor.id.clone(),
                Principal::Extension(context.extension_id.clone()),
                vec![EffectKind::DispatchCapability],
            ),
        ))
        .await
        .unwrap();

    let authorizer = LeaseBackedAuthorizer::new(&leases);
    let decision = authorizer
        .authorize_dispatch(&context, &descriptor, &ResourceEstimate::default())
        .await;

    assert!(matches!(
        decision,
        Decision::Deny {
            reason: DenyReason::InternalInvariantViolation
        }
    ));
}

#[tokio::test]
async fn one_off_lease_does_not_authorize_different_invocation_in_same_tenant() {
    let leases = InMemoryCapabilityLeaseStore::new();
    let context = execution_context(CapabilitySet::default());
    let mut next_invocation_context = execution_context(CapabilitySet::default());
    next_invocation_context.tenant_id = context.tenant_id.clone();
    next_invocation_context.user_id = context.user_id.clone();
    next_invocation_context.project_id = context.project_id.clone();
    next_invocation_context.resource_scope.tenant_id = context.tenant_id.clone();
    next_invocation_context.resource_scope.user_id = context.user_id.clone();
    next_invocation_context.resource_scope.project_id = context.project_id.clone();
    let descriptor = descriptor(CapabilityId::new("echo.say").unwrap());
    let lease = CapabilityLease::new(
        context.resource_scope.clone(),
        grant_for(
            descriptor.id.clone(),
            Principal::Extension(context.extension_id.clone()),
            vec![EffectKind::DispatchCapability],
        ),
    );
    leases.issue(lease).await.unwrap();

    let authorizer = LeaseBackedAuthorizer::new(&leases);
    let decision = authorizer
        .authorize_dispatch(
            &next_invocation_context,
            &descriptor,
            &ResourceEstimate::default(),
        )
        .await;

    assert!(matches!(
        decision,
        Decision::Deny {
            reason: DenyReason::MissingGrant
        }
    ));
}

#[tokio::test]
async fn consume_decrements_remaining_invocations_and_consumes_one_shot_lease() {
    let leases = InMemoryCapabilityLeaseStore::new();
    let context = execution_context(CapabilitySet::default());
    let descriptor = descriptor(CapabilityId::new("echo.say").unwrap());
    let mut grant = grant_for(
        descriptor.id.clone(),
        Principal::Extension(context.extension_id.clone()),
        vec![EffectKind::DispatchCapability],
    );
    grant.constraints.max_invocations = Some(2);
    let lease = CapabilityLease::new(context.resource_scope.clone(), grant);
    let lease_id = lease.grant.id;
    leases.issue(lease).await.unwrap();

    let after_first = leases
        .consume(&context.resource_scope, lease_id)
        .await
        .unwrap();

    assert_eq!(after_first.status, CapabilityLeaseStatus::Active);
    assert_eq!(after_first.grant.constraints.max_invocations, Some(1));

    let after_second = leases
        .consume(&context.resource_scope, lease_id)
        .await
        .unwrap();

    assert_eq!(after_second.status, CapabilityLeaseStatus::Consumed);
    assert_eq!(after_second.grant.constraints.max_invocations, Some(0));
    assert!(matches!(
        leases.consume(&context.resource_scope, lease_id).await.unwrap_err(),
        CapabilityLeaseError::ExhaustedLease { lease_id: id } if id == lease_id
    ));
}

#[tokio::test]
async fn consumed_lease_no_longer_authorizes_dispatch() {
    let leases = InMemoryCapabilityLeaseStore::new();
    let context = execution_context(CapabilitySet::default());
    let descriptor = descriptor(CapabilityId::new("echo.say").unwrap());
    let mut grant = grant_for(
        descriptor.id.clone(),
        Principal::Extension(context.extension_id.clone()),
        vec![EffectKind::DispatchCapability],
    );
    grant.constraints.max_invocations = Some(1);
    let lease = CapabilityLease::new(context.resource_scope.clone(), grant);
    let lease_id = lease.grant.id;
    leases.issue(lease).await.unwrap();
    leases
        .consume(&context.resource_scope, lease_id)
        .await
        .unwrap();

    let authorizer = LeaseBackedAuthorizer::new(&leases);
    let decision = authorizer
        .authorize_dispatch(&context, &descriptor, &ResourceEstimate::default())
        .await;

    assert!(matches!(
        decision,
        Decision::Deny {
            reason: DenyReason::MissingGrant
        }
    ));
}

#[tokio::test]
async fn fingerprinted_lease_cannot_be_consumed_before_claim() {
    let leases = InMemoryCapabilityLeaseStore::new();
    let context = execution_context(CapabilitySet::default());
    let descriptor = descriptor(CapabilityId::new("echo.say").unwrap());
    let fingerprint = InvocationFingerprint::for_dispatch(
        &context.resource_scope,
        &descriptor.id,
        &ResourceEstimate::default(),
        &serde_json::json!({"message": "approved"}),
    )
    .unwrap();
    let mut lease = CapabilityLease::new(
        context.resource_scope.clone(),
        grant_for(
            descriptor.id.clone(),
            Principal::Extension(context.extension_id.clone()),
            vec![EffectKind::DispatchCapability],
        ),
    );
    lease.invocation_fingerprint = Some(fingerprint);
    let lease_id = lease.grant.id;
    leases.issue(lease).await.unwrap();

    let err = leases
        .consume(&context.resource_scope, lease_id)
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        CapabilityLeaseError::UnclaimedFingerprintLease { lease_id: id } if id == lease_id
    ));
    assert_eq!(
        leases
            .get(&context.resource_scope, lease_id)
            .await
            .unwrap()
            .status,
        CapabilityLeaseStatus::Active
    );
}

#[tokio::test]
async fn fingerprinted_lease_without_invocation_limit_is_consumed_after_one_use() {
    let leases = InMemoryCapabilityLeaseStore::new();
    let context = execution_context(CapabilitySet::default());
    let descriptor = descriptor(CapabilityId::new("echo.say").unwrap());
    let fingerprint = InvocationFingerprint::for_dispatch(
        &context.resource_scope,
        &descriptor.id,
        &ResourceEstimate::default(),
        &serde_json::json!({"message": "approved"}),
    )
    .unwrap();
    let mut lease = CapabilityLease::new(
        context.resource_scope.clone(),
        grant_for(
            descriptor.id.clone(),
            Principal::Extension(context.extension_id.clone()),
            vec![EffectKind::DispatchCapability],
        ),
    );
    lease.invocation_fingerprint = Some(fingerprint.clone());
    let lease_id = lease.grant.id;
    leases.issue(lease).await.unwrap();

    leases
        .claim(&context.resource_scope, lease_id, &fingerprint)
        .await
        .unwrap();
    let consumed = leases
        .consume(&context.resource_scope, lease_id)
        .await
        .unwrap();

    assert_eq!(consumed.status, CapabilityLeaseStatus::Consumed);
    assert!(matches!(
        leases
            .claim(&context.resource_scope, lease_id, &fingerprint)
            .await
            .unwrap_err(),
        CapabilityLeaseError::InactiveLease { lease_id: id, status: CapabilityLeaseStatus::Consumed }
            if id == lease_id
    ));
}

#[tokio::test]
async fn expired_lease_no_longer_authorizes_or_consumes() {
    let leases = InMemoryCapabilityLeaseStore::new();
    let context = execution_context(CapabilitySet::default());
    let descriptor = descriptor(CapabilityId::new("echo.say").unwrap());
    let mut grant = grant_for(
        descriptor.id.clone(),
        Principal::Extension(context.extension_id.clone()),
        vec![EffectKind::DispatchCapability],
    );
    grant.constraints.expires_at = Some(timestamp("2000-01-01T00:00:00Z"));
    let lease = CapabilityLease::new(context.resource_scope.clone(), grant);
    let lease_id = lease.grant.id;
    leases.issue(lease).await.unwrap();

    let authorizer = LeaseBackedAuthorizer::new(&leases);
    let decision = authorizer
        .authorize_dispatch(&context, &descriptor, &ResourceEstimate::default())
        .await;

    assert!(matches!(
        decision,
        Decision::Deny {
            reason: DenyReason::MissingGrant
        }
    ));
    assert!(matches!(
        leases.consume(&context.resource_scope, lease_id).await.unwrap_err(),
        CapabilityLeaseError::ExpiredLease { lease_id: id } if id == lease_id
    ));
}

#[tokio::test]
async fn consume_is_scoped_to_exact_invocation() {
    let leases = InMemoryCapabilityLeaseStore::new();
    let context = execution_context(CapabilitySet::default());
    let mut next_invocation_scope = execution_context(CapabilitySet::default()).resource_scope;
    next_invocation_scope.tenant_id = context.resource_scope.tenant_id.clone();
    next_invocation_scope.user_id = context.resource_scope.user_id.clone();
    next_invocation_scope.project_id = context.resource_scope.project_id.clone();
    let descriptor = descriptor(CapabilityId::new("echo.say").unwrap());
    let mut grant = grant_for(
        descriptor.id.clone(),
        Principal::Extension(context.extension_id.clone()),
        vec![EffectKind::DispatchCapability],
    );
    grant.constraints.max_invocations = Some(1);
    let lease = CapabilityLease::new(context.resource_scope.clone(), grant);
    let lease_id = lease.grant.id;
    leases.issue(lease.clone()).await.unwrap();

    let err = leases
        .consume(&next_invocation_scope, lease_id)
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        CapabilityLeaseError::UnknownLease { lease_id: id } if id == lease_id
    ));
    assert_eq!(
        leases
            .get(&context.resource_scope, lease_id)
            .await
            .unwrap()
            .status,
        CapabilityLeaseStatus::Active
    );
}

#[tokio::test]
async fn filesystem_lease_store_persists_and_reloads_issued_leases() {
    let fs = engine_filesystem();
    let context = execution_context(CapabilitySet::default());
    let descriptor = descriptor(CapabilityId::new("echo.say").unwrap());
    let mut grant = grant_for(
        descriptor.id.clone(),
        Principal::Extension(context.extension_id.clone()),
        vec![EffectKind::DispatchCapability],
    );
    grant.constraints.max_invocations = Some(3);
    let lease = CapabilityLease::new(context.resource_scope.clone(), grant);
    let lease_id = lease.grant.id;

    FilesystemCapabilityLeaseStore::new(&fs)
        .issue(lease.clone())
        .await
        .unwrap();
    let reloaded = FilesystemCapabilityLeaseStore::new(&fs);

    assert_eq!(
        reloaded.get(&context.resource_scope, lease_id).await,
        Some(lease)
    );
    assert_eq!(
        reloaded.leases_for_scope(&context.resource_scope).await,
        vec![
            reloaded
                .get(&context.resource_scope, lease_id)
                .await
                .unwrap()
        ]
    );
}

#[tokio::test]
async fn filesystem_lease_store_lists_from_owner_index_without_scanning_invocation_roots() {
    let fs = CountingFilesystem::new(engine_filesystem());
    let context = execution_context(CapabilitySet::default());
    let descriptor = descriptor(CapabilityId::new("echo.say").unwrap());
    let store = FilesystemCapabilityLeaseStore::new(&fs);
    let mut expected = Vec::new();

    for _ in 0..3 {
        let scope = execution_context(CapabilitySet::default()).resource_scope;
        let lease = CapabilityLease::new(
            scope,
            grant_for(
                descriptor.id.clone(),
                Principal::Extension(context.extension_id.clone()),
                vec![EffectKind::DispatchCapability],
            ),
        );
        expected.push(lease.grant.id);
        store.issue(lease).await.unwrap();
    }

    fs.reset_list_dir_calls();
    let leases = store.leases_for_scope(&context.resource_scope).await;

    let mut actual = leases
        .into_iter()
        .map(|lease| lease.grant.id)
        .collect::<Vec<_>>();
    actual.sort_by_key(|lease_id| lease_id.as_uuid());
    expected.sort_by_key(|lease_id| lease_id.as_uuid());
    assert_eq!(actual, expected);
    assert_eq!(
        fs.list_dir_calls(),
        0,
        "indexed lease listing should not scan every invocation directory"
    );
}

#[tokio::test]
async fn filesystem_lease_store_persists_revoke_claim_and_consume() {
    let fs = engine_filesystem();
    let context = execution_context(CapabilitySet::default());
    let descriptor = descriptor(CapabilityId::new("echo.say").unwrap());
    let fingerprint = InvocationFingerprint::for_dispatch(
        &context.resource_scope,
        &descriptor.id,
        &ResourceEstimate::default(),
        &serde_json::json!({"message": "approved"}),
    )
    .unwrap();
    let mut grant = grant_for(
        descriptor.id.clone(),
        Principal::Extension(context.extension_id.clone()),
        vec![EffectKind::DispatchCapability],
    );
    grant.constraints.max_invocations = Some(1);
    let mut lease = CapabilityLease::new(context.resource_scope.clone(), grant);
    lease.invocation_fingerprint = Some(fingerprint.clone());
    let lease_id = lease.grant.id;
    let store = FilesystemCapabilityLeaseStore::new(&fs);
    store.issue(lease).await.unwrap();

    let claimed = store
        .claim(&context.resource_scope, lease_id, &fingerprint)
        .await
        .unwrap();
    assert_eq!(claimed.status, CapabilityLeaseStatus::Claimed);
    assert_eq!(
        FilesystemCapabilityLeaseStore::new(&fs)
            .get(&context.resource_scope, lease_id)
            .await
            .unwrap()
            .status,
        CapabilityLeaseStatus::Claimed
    );

    let consumed = FilesystemCapabilityLeaseStore::new(&fs)
        .consume(&context.resource_scope, lease_id)
        .await
        .unwrap();
    assert_eq!(consumed.status, CapabilityLeaseStatus::Consumed);
    assert_eq!(consumed.grant.constraints.max_invocations, Some(0));

    let revoked = FilesystemCapabilityLeaseStore::new(&fs)
        .revoke(&context.resource_scope, lease_id)
        .await
        .unwrap();
    assert_eq!(revoked.status, CapabilityLeaseStatus::Revoked);
    assert_eq!(
        FilesystemCapabilityLeaseStore::new(&fs)
            .get(&context.resource_scope, lease_id)
            .await
            .unwrap()
            .status,
        CapabilityLeaseStatus::Revoked
    );
}

#[tokio::test]
async fn filesystem_fingerprinted_lease_cannot_be_consumed_before_claim() {
    let fs = engine_filesystem();
    let context = execution_context(CapabilitySet::default());
    let descriptor = descriptor(CapabilityId::new("echo.say").unwrap());
    let fingerprint = InvocationFingerprint::for_dispatch(
        &context.resource_scope,
        &descriptor.id,
        &ResourceEstimate::default(),
        &serde_json::json!({"message": "approved"}),
    )
    .unwrap();
    let mut lease = CapabilityLease::new(
        context.resource_scope.clone(),
        grant_for(
            descriptor.id.clone(),
            Principal::Extension(context.extension_id.clone()),
            vec![EffectKind::DispatchCapability],
        ),
    );
    lease.invocation_fingerprint = Some(fingerprint);
    let lease_id = lease.grant.id;
    let store = FilesystemCapabilityLeaseStore::new(&fs);
    store.issue(lease).await.unwrap();

    let err = store
        .consume(&context.resource_scope, lease_id)
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        CapabilityLeaseError::UnclaimedFingerprintLease { lease_id: id } if id == lease_id
    ));
    assert_eq!(
        FilesystemCapabilityLeaseStore::new(&fs)
            .get(&context.resource_scope, lease_id)
            .await
            .unwrap()
            .status,
        CapabilityLeaseStatus::Active
    );
}

#[tokio::test]
async fn filesystem_fingerprinted_lease_without_invocation_limit_is_consumed_after_one_use() {
    let fs = engine_filesystem();
    let context = execution_context(CapabilitySet::default());
    let descriptor = descriptor(CapabilityId::new("echo.say").unwrap());
    let fingerprint = InvocationFingerprint::for_dispatch(
        &context.resource_scope,
        &descriptor.id,
        &ResourceEstimate::default(),
        &serde_json::json!({"message": "approved"}),
    )
    .unwrap();
    let mut lease = CapabilityLease::new(
        context.resource_scope.clone(),
        grant_for(
            descriptor.id.clone(),
            Principal::Extension(context.extension_id.clone()),
            vec![EffectKind::DispatchCapability],
        ),
    );
    lease.invocation_fingerprint = Some(fingerprint.clone());
    let lease_id = lease.grant.id;
    let store = FilesystemCapabilityLeaseStore::new(&fs);
    store.issue(lease).await.unwrap();

    store
        .claim(&context.resource_scope, lease_id, &fingerprint)
        .await
        .unwrap();
    let consumed = FilesystemCapabilityLeaseStore::new(&fs)
        .consume(&context.resource_scope, lease_id)
        .await
        .unwrap();

    assert_eq!(consumed.status, CapabilityLeaseStatus::Consumed);
    assert!(matches!(
        FilesystemCapabilityLeaseStore::new(&fs)
            .claim(&context.resource_scope, lease_id, &fingerprint)
            .await
            .unwrap_err(),
        CapabilityLeaseError::InactiveLease { lease_id: id, status: CapabilityLeaseStatus::Consumed }
            if id == lease_id
    ));
}

#[tokio::test]
async fn filesystem_lease_store_is_tenant_user_invocation_scoped() {
    let fs = engine_filesystem();
    let context = execution_context(CapabilitySet::default());
    let mut other_invocation_scope = execution_context(CapabilitySet::default()).resource_scope;
    other_invocation_scope.tenant_id = context.resource_scope.tenant_id.clone();
    other_invocation_scope.user_id = context.resource_scope.user_id.clone();
    other_invocation_scope.project_id = context.resource_scope.project_id.clone();
    let descriptor = descriptor(CapabilityId::new("echo.say").unwrap());
    let lease = CapabilityLease::new(
        context.resource_scope.clone(),
        grant_for(
            descriptor.id.clone(),
            Principal::Extension(context.extension_id.clone()),
            vec![EffectKind::DispatchCapability],
        ),
    );
    let lease_id = lease.grant.id;
    let store = FilesystemCapabilityLeaseStore::new(&fs);
    store.issue(lease.clone()).await.unwrap();

    assert_eq!(store.get(&other_invocation_scope, lease_id).await, None);
    assert!(matches!(
        store.revoke(&other_invocation_scope, lease_id).await.unwrap_err(),
        CapabilityLeaseError::UnknownLease { lease_id: id } if id == lease_id
    ));
    assert_eq!(
        store.get(&context.resource_scope, lease_id).await,
        Some(lease)
    );
}

#[tokio::test]
async fn revoked_lease_no_longer_authorizes_dispatch() {
    let leases = InMemoryCapabilityLeaseStore::new();
    let context = execution_context(CapabilitySet::default());
    let descriptor = descriptor(CapabilityId::new("echo.say").unwrap());
    let lease = CapabilityLease::new(
        context.resource_scope.clone(),
        grant_for(
            descriptor.id.clone(),
            Principal::Extension(context.extension_id.clone()),
            vec![EffectKind::DispatchCapability],
        ),
    );
    let lease_id = lease.grant.id;
    leases.issue(lease).await.unwrap();
    leases
        .revoke(&context.resource_scope, lease_id)
        .await
        .unwrap();

    let authorizer = LeaseBackedAuthorizer::new(&leases);
    let decision = authorizer
        .authorize_dispatch(&context, &descriptor, &ResourceEstimate::default())
        .await;

    assert!(matches!(
        decision,
        Decision::Deny {
            reason: DenyReason::MissingGrant
        }
    ));
}

struct CountingFilesystem {
    inner: LocalFilesystem,
    list_dir_calls: Arc<AtomicUsize>,
}

impl CountingFilesystem {
    fn new(inner: LocalFilesystem) -> Self {
        Self {
            inner,
            list_dir_calls: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn reset_list_dir_calls(&self) {
        self.list_dir_calls.store(0, Ordering::SeqCst);
    }

    fn list_dir_calls(&self) -> usize {
        self.list_dir_calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl RootFilesystem for CountingFilesystem {
    async fn read_file(&self, path: &VirtualPath) -> Result<Vec<u8>, FilesystemError> {
        self.inner.read_file(path).await
    }

    async fn write_file(&self, path: &VirtualPath, bytes: &[u8]) -> Result<(), FilesystemError> {
        self.inner.write_file(path, bytes).await
    }

    async fn append_file(&self, path: &VirtualPath, bytes: &[u8]) -> Result<(), FilesystemError> {
        self.inner.append_file(path, bytes).await
    }

    async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
        self.list_dir_calls.fetch_add(1, Ordering::SeqCst);
        self.inner.list_dir(path).await
    }

    async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
        self.inner.stat(path).await
    }

    async fn delete(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        self.inner.delete(path).await
    }

    async fn create_dir_all(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        self.inner.create_dir_all(path).await
    }
}

fn trust_decision(
    allowed_effects: Vec<EffectKind>,
    max_resource_ceiling: Option<ResourceCeiling>,
) -> TrustDecision {
    TrustDecision {
        effective_trust: EffectiveTrustClass::sandbox(),
        authority_ceiling: AuthorityCeiling {
            allowed_effects,
            max_resource_ceiling,
        },
        provenance: TrustProvenance::Default,
        evaluated_at: Utc::now(),
    }
}

fn descriptor(id: CapabilityId) -> CapabilityDescriptor {
    CapabilityDescriptor {
        provider: ExtensionId::new(id.as_str().split('.').next().unwrap()).unwrap(),
        id,
        runtime: RuntimeKind::Wasm,
        trust_ceiling: TrustClass::Sandbox,
        description: "test".to_string(),
        parameters_schema: serde_json::json!({"type": "object"}),
        effects: vec![EffectKind::DispatchCapability],
        default_permission: PermissionMode::Deny,
        resource_profile: None,
    }
}

fn grant_for(
    capability: CapabilityId,
    grantee: Principal,
    allowed_effects: Vec<EffectKind>,
) -> CapabilityGrant {
    CapabilityGrant {
        id: CapabilityGrantId::new(),
        capability,
        grantee,
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

fn timestamp(value: &str) -> Timestamp {
    serde_json::from_value(serde_json::Value::String(value.to_string())).unwrap()
}

fn engine_filesystem() -> LocalFilesystem {
    let storage = tempfile::tempdir().unwrap().keep();
    let engine_root = storage.join("engine");
    std::fs::create_dir_all(&engine_root).unwrap();
    let mut fs = LocalFilesystem::new();
    fs.mount_local(
        VirtualPath::new("/engine").unwrap(),
        HostPath::from_path_buf(engine_root),
    )
    .unwrap();
    fs
}

fn execution_context(grants: CapabilitySet) -> ExecutionContext {
    let invocation_id = InvocationId::new();
    let resource_scope = ResourceScope {
        tenant_id: TenantId::new("tenant1").unwrap(),
        user_id: UserId::new("user1").unwrap(),
        agent_id: None,
        project_id: Some(ProjectId::new("project1").unwrap()),
        mission_id: None,
        thread_id: None,
        invocation_id,
    };
    ExecutionContext {
        invocation_id,
        correlation_id: CorrelationId::new(),
        process_id: None,
        parent_process_id: None,
        tenant_id: resource_scope.tenant_id.clone(),
        user_id: resource_scope.user_id.clone(),
        agent_id: resource_scope.agent_id.clone(),
        project_id: resource_scope.project_id.clone(),
        mission_id: resource_scope.mission_id.clone(),
        thread_id: resource_scope.thread_id.clone(),
        extension_id: ExtensionId::new("caller").unwrap(),
        runtime: RuntimeKind::Wasm,
        trust: TrustClass::Sandbox,
        grants,
        mounts: MountView::default(),
        resource_scope,
    }
}
