use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use ironclaw_host_api::{
    CapabilityId, ExtensionId, InvocationId, NetworkMethod, ProjectId, ResourceScope, SecretHandle,
    TenantId, UserId,
};

use crate::{
    CredentialAccount, CredentialAccountId, CredentialAccountStatus, CredentialBrokerError,
    CredentialPathPolicy, CredentialSessionRequest, CredentialTargetPolicy,
    InMemoryCredentialBroker, RedactedJson,
};

use super::{
    CREDENTIAL_PLACEHOLDER_SUFFIX_LEN, CredentialPlaceholderRegistry, CredentialPlaceholderToken,
    CredentialSessionLease,
};

fn sample_scope(tenant: &str, user: &str) -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new(tenant).unwrap(),
        user_id: UserId::new(user).unwrap(),
        agent_id: None,
        project_id: Some(ProjectId::new("project-a").unwrap()),
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    }
}

fn sample_account(
    scope: ResourceScope,
    id: CredentialAccountId,
    handle: SecretHandle,
) -> CredentialAccount {
    CredentialAccount {
        scope,
        id,
        provider_or_extension_id: ExtensionId::new("google").unwrap(),
        label: "Prod".to_string(),
        status: CredentialAccountStatus::Active,
        secret_handles: vec![handle],
        allowed_targets: vec![CredentialTargetPolicy {
            scheme: "https".to_string(),
            host: "api.example.com".to_string(),
            port: Some(443),
            path: CredentialPathPolicy::Prefix("/v1/".to_string()),
            methods: vec![NetworkMethod::Get],
        }],
        redacted_metadata: RedactedJson::new(serde_json::json!({})),
        updated_at: Utc::now(),
    }
}

fn session_request(
    scope: ResourceScope,
    account_id: CredentialAccountId,
    url: &str,
) -> CredentialSessionRequest {
    CredentialSessionRequest {
        invocation_id: scope.invocation_id,
        scope,
        capability_id: CapabilityId::new("google.drive").unwrap(),
        extension_id: ExtensionId::new("google").unwrap(),
        account_id,
        method: NetworkMethod::Get,
        url: url.to_string(),
        expires_at: None,
        max_uses: None,
    }
}

#[test]
fn placeholder_is_stable_across_repeated_lookups_and_recycle() {
    let registry = CredentialPlaceholderRegistry::new();
    let tenant = TenantId::new("tenant-a").unwrap();
    let user = UserId::new("user-a").unwrap();
    let provider = ExtensionId::new("google").unwrap();

    let first = registry.get_or_create(&tenant, &user, &provider).unwrap();
    let second = registry.get_or_create(&tenant, &user, &provider).unwrap();
    assert_eq!(first, second);

    // Simulate a container recycle: nothing about the registry is tied to
    // a container, so a "new container" asking for the same triple again
    // gets the identical token.
    drop(first);
    let after_recycle = registry.get_or_create(&tenant, &user, &provider).unwrap();
    assert_eq!(second, after_recycle);
}

#[test]
fn placeholders_are_distinct_and_isolated_per_user() {
    let registry = CredentialPlaceholderRegistry::new();
    let tenant = TenantId::new("tenant-a").unwrap();
    let provider = ExtensionId::new("google").unwrap();
    let user_a = UserId::new("user-a").unwrap();
    let user_b = UserId::new("user-b").unwrap();

    let token_a = registry.get_or_create(&tenant, &user_a, &provider).unwrap();
    let token_b = registry.get_or_create(&tenant, &user_b, &provider).unwrap();
    assert_ne!(token_a, token_b);

    let owner_a = registry.resolve(&token_a).unwrap().unwrap();
    let owner_b = registry.resolve(&token_b).unwrap().unwrap();
    assert_eq!(owner_a.user_id, user_a);
    assert_eq!(owner_b.user_id, user_b);
    assert_ne!(owner_a.user_id, owner_b.user_id);

    // User A's placeholder must never resolve to user B's owner triple.
    assert_ne!(
        registry.resolve(&token_a).unwrap(),
        registry.resolve(&token_b).unwrap()
    );
}

#[test]
fn placeholder_token_requires_fixed_prefix() {
    assert!(CredentialPlaceholderToken::parse("icsbx_0123456789abcdef0123456789abcdef").is_ok());
    let err = CredentialPlaceholderToken::parse("not_a_placeholder").unwrap_err();
    assert!(matches!(
        err,
        CredentialBrokerError::InvalidPlaceholderToken { .. }
    ));
}

#[test]
fn placeholder_token_parse_rejects_malformed_suffix() {
    // Empty string: no prefix at all.
    assert!(CredentialPlaceholderToken::parse("").is_err());
    // Bare prefix with no suffix carries no identifying material.
    assert!(CredentialPlaceholderToken::parse("icsbx_").is_err());
    // Suffix shorter than the registry ever produces.
    assert!(CredentialPlaceholderToken::parse("icsbx_ab").is_err());
    // Non-alphanumeric suffix characters (control chars / shell
    // metacharacters) must fail closed rather than being accepted and
    // potentially propagated into a log line or env var downstream.
    assert!(CredentialPlaceholderToken::parse("icsbx_0123456789abcdef\n; rm -rf /").is_err());
    // Suffix longer than the registry ever produces: an arbitrarily long
    // `icsbx_...` string from the sandbox must be rejected, not accepted
    // and driven through hashing/cloning/map-insertion downstream.
    assert!(
        CredentialPlaceholderToken::parse(format!(
            "icsbx_{}",
            "a".repeat(CREDENTIAL_PLACEHOLDER_SUFFIX_LEN + 1)
        ))
        .is_err()
    );
}

#[test]
fn placeholder_with_no_live_session_grants_nothing() {
    let broker = Arc::new(InMemoryCredentialBroker::new());
    let registry = CredentialPlaceholderRegistry::new();
    let tenant = TenantId::new("tenant-a").unwrap();
    let user = UserId::new("user-a").unwrap();
    let provider = ExtensionId::new("google").unwrap();
    let token = registry.get_or_create(&tenant, &user, &provider).unwrap();

    let found = broker
        .find_session_by_placeholder(&token, &sample_scope("tenant-a", "user-a"))
        .unwrap();
    assert!(found.is_none());
}

#[test]
fn jit_mint_binds_session_to_placeholder_and_enforces_target_policy() {
    let broker = Arc::new(InMemoryCredentialBroker::new());
    let registry = CredentialPlaceholderRegistry::new();
    let tenant = TenantId::new("tenant-a").unwrap();
    let user = UserId::new("user-a").unwrap();
    let provider = ExtensionId::new("google").unwrap();
    let token = registry.get_or_create(&tenant, &user, &provider).unwrap();

    let caller_scope = sample_scope("tenant-a", "user-a");
    let account_id = CredentialAccountId::new("google_prod").unwrap();
    broker
        .put_account(sample_account(
            caller_scope.clone(),
            account_id.clone(),
            SecretHandle::new("google_key").unwrap(),
        ))
        .unwrap();

    // Out-of-policy request (wrong path) must be rejected, not minted.
    let rejected = broker.mint_on_first_use(
        &token,
        session_request(
            caller_scope.clone(),
            account_id.clone(),
            "https://api.example.com/v2/x",
        ),
    );
    assert!(matches!(
        rejected,
        Err(CredentialBrokerError::CredentialPolicyMismatch { .. })
    ));
    assert!(
        broker
            .find_session_by_placeholder(&token, &caller_scope)
            .unwrap()
            .is_none()
    );

    // In-policy request mints and binds.
    let lease = broker
        .mint_on_first_use(
            &token,
            session_request(
                caller_scope.clone(),
                account_id,
                "https://api.example.com/v1/x",
            ),
        )
        .unwrap();
    let bound = broker
        .find_session_by_placeholder(&token, &caller_scope)
        .unwrap()
        .expect("session bound to placeholder after JIT mint");
    assert_eq!(bound.correlation_id(), lease.session_id());
    lease.revoke();
}

#[test]
fn placeholder_session_lookup_does_not_cross_user_scope() {
    let broker = Arc::new(InMemoryCredentialBroker::new());
    let registry = CredentialPlaceholderRegistry::new();
    let tenant = TenantId::new("tenant-a").unwrap();
    let provider = ExtensionId::new("google").unwrap();
    let user_a = UserId::new("user-a").unwrap();
    let token_a = registry.get_or_create(&tenant, &user_a, &provider).unwrap();

    let scope_a = sample_scope("tenant-a", "user-a");
    let account_id = CredentialAccountId::new("google_prod").unwrap();
    broker
        .put_account(sample_account(
            scope_a.clone(),
            account_id.clone(),
            SecretHandle::new("google_key").unwrap(),
        ))
        .unwrap();
    let lease = broker
        .mint_on_first_use(
            &token_a,
            session_request(scope_a, account_id, "https://api.example.com/v1/x"),
        )
        .unwrap();

    // User B's scope must never see user A's session through the placeholder.
    let other_scope = sample_scope("tenant-a", "user-b");
    let found = broker
        .find_session_by_placeholder(&token_a, &other_scope)
        .unwrap();
    assert!(found.is_none());
    lease.revoke();
}

#[test]
fn lease_revokes_on_explicit_call() {
    // Stands in for both the success-path and error-path explicit-revoke
    // call sites: both call the identical `CredentialSessionLease::revoke`
    // API and assert the identical postcondition (session gone). The only
    // difference between them was a narrative `if dispatch_result.is_err()`
    // wrapper around the call — no broker code path branches on it, so a
    // single test covers both without overlap. Cancellation-drop (timeout)
    // and unwind-drop (panic) exercise genuinely different Rust
    // mechanisms and stay as separate tests below.
    let broker = Arc::new(InMemoryCredentialBroker::new());
    let registry = CredentialPlaceholderRegistry::new();
    let (token, scope_a, account_id) = seeded(&broker, &registry, "user-explicit-revoke");

    let lease = broker
        .mint_on_first_use(
            &token,
            session_request(scope_a, account_id, "https://api.example.com/v1/x"),
        )
        .unwrap();
    let session_id = lease.session_id();
    lease.revoke(); // success and error paths both call this explicitly
    assert!(matches!(
        broker.validate_session(session_id, Utc::now()),
        Err(CredentialBrokerError::UnknownSession { .. })
    ));
}

#[tokio::test]
async fn lease_revokes_on_timeout_via_drop() {
    let broker = Arc::new(InMemoryCredentialBroker::new());
    let registry = CredentialPlaceholderRegistry::new();
    let (token, scope_a, account_id) = seeded(&broker, &registry, "user-timeout");

    let lease = broker
        .mint_on_first_use(
            &token,
            session_request(scope_a, account_id, "https://api.example.com/v1/x"),
        )
        .unwrap();
    let session_id = lease.session_id();

    let outcome = tokio::time::timeout(Duration::from_millis(5), async move {
        let _lease = lease; // held across the (never-completing) sleep
        tokio::time::sleep(Duration::from_secs(60)).await;
    })
    .await;

    assert!(outcome.is_err(), "expected the dispatch to time out");
    assert!(matches!(
        broker.validate_session(session_id, Utc::now()),
        Err(CredentialBrokerError::UnknownSession { .. })
    ));
}

#[test]
fn lease_revokes_on_panic_via_drop() {
    let broker = Arc::new(InMemoryCredentialBroker::new());
    let registry = CredentialPlaceholderRegistry::new();
    let (token, scope_a, account_id) = seeded(&broker, &registry, "user-panic");

    let lease = broker
        .mint_on_first_use(
            &token,
            session_request(scope_a, account_id, "https://api.example.com/v1/x"),
        )
        .unwrap();
    let session_id = lease.session_id();

    let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
        let _lease = lease; // dropped during unwind
        panic!("simulated panic mid-dispatch");
    }));

    assert!(result.is_err());
    assert!(matches!(
        broker.validate_session(session_id, Utc::now()),
        Err(CredentialBrokerError::UnknownSession { .. })
    ));
}

#[test]
fn reused_session_survives_the_first_of_two_leases_revoking() {
    // Two mint_on_first_use calls for the identical (invocation,
    // capability, account) binding — the documented cache-hit reuse path
    // — must hand out two independent leases over the *same* session.
    // Revoking the first-acquired lease must not revoke the session out
    // from under the second lease still holding it; only releasing both
    // should actually revoke it.
    let broker = Arc::new(InMemoryCredentialBroker::new());
    let registry = CredentialPlaceholderRegistry::new();
    let (token, scope_a, account_id) = seeded(&broker, &registry, "user-refcount");

    let first_lease = broker
        .mint_on_first_use(
            &token,
            session_request(
                scope_a.clone(),
                account_id.clone(),
                "https://api.example.com/v1/x",
            ),
        )
        .unwrap();
    let second_lease = broker
        .mint_on_first_use(
            &token,
            session_request(scope_a, account_id, "https://api.example.com/v1/x"),
        )
        .unwrap();
    assert_eq!(
        first_lease.session_id(),
        second_lease.session_id(),
        "cache-hit reuse must return the same underlying session"
    );
    let session_id = first_lease.session_id();

    first_lease.revoke();
    assert!(
        broker.validate_session(session_id, Utc::now()).is_ok(),
        "the second lease is still outstanding; revoking the first must not kill the shared session"
    );

    second_lease.revoke();
    assert!(matches!(
        broker.validate_session(session_id, Utc::now()),
        Err(CredentialBrokerError::UnknownSession { .. })
    ));
}

#[test]
fn revoke_prunes_secondary_indices_so_placeholder_no_longer_resolves() {
    // Revoking a session must not just make validate_session fail — it
    // must also stop the placeholder from resolving to it at all, and
    // must not leave stale bookkeeping (jit_minted / sessions_by_placeholder)
    // behind for the life of the process.
    let broker = Arc::new(InMemoryCredentialBroker::new());
    let registry = CredentialPlaceholderRegistry::new();
    let (token, scope_a, account_id) = seeded(&broker, &registry, "user-prune");

    let lease = broker
        .mint_on_first_use(
            &token,
            session_request(scope_a.clone(), account_id, "https://api.example.com/v1/x"),
        )
        .unwrap();
    assert!(
        broker
            .find_session_by_placeholder(&token, &scope_a)
            .unwrap()
            .is_some()
    );

    lease.revoke();

    assert!(
        broker
            .find_session_by_placeholder(&token, &scope_a)
            .unwrap()
            .is_none(),
        "placeholder must not resolve to a revoked session"
    );
}

#[test]
fn find_session_by_placeholder_iterates_multiple_distinct_sessions_for_same_placeholder() {
    // A placeholder can legitimately have more than one *distinct*
    // session bound to it at once (e.g. two different accounts under
    // one provider) — not just the same session reused, which is all
    // the refcount tests above exercise. This pins that the
    // scope-matching loop in `find_session_by_placeholder` actually
    // walks past a non-matching candidate to find the right one.
    let broker = Arc::new(InMemoryCredentialBroker::new());
    let registry = CredentialPlaceholderRegistry::new();
    let tenant = TenantId::new("tenant-a").unwrap();
    let provider = ExtensionId::new("google").unwrap();
    let user_a = UserId::new("user-multi-a").unwrap();
    let token = registry.get_or_create(&tenant, &user_a, &provider).unwrap();

    let scope_a = sample_scope("tenant-a", "user-multi-a");
    let scope_b = sample_scope("tenant-a", "user-multi-b");
    let account_a = CredentialAccountId::new("google_account_a").unwrap();
    let account_b = CredentialAccountId::new("google_account_b").unwrap();
    broker
        .put_account(sample_account(
            scope_a.clone(),
            account_a.clone(),
            SecretHandle::new("key_a").unwrap(),
        ))
        .unwrap();
    broker
        .put_account(sample_account(
            scope_b.clone(),
            account_b.clone(),
            SecretHandle::new("key_b").unwrap(),
        ))
        .unwrap();

    // Two distinct (invocation, capability, account) bindings mint two
    // distinct sessions, both bound to the same placeholder token.
    let lease_a = broker
        .mint_on_first_use(
            &token,
            session_request(scope_a.clone(), account_a, "https://api.example.com/v1/x"),
        )
        .unwrap();
    let lease_b = broker
        .mint_on_first_use(
            &token,
            session_request(scope_b.clone(), account_b, "https://api.example.com/v1/x"),
        )
        .unwrap();
    assert_ne!(
        lease_a.session_id(),
        lease_b.session_id(),
        "two different accounts must mint two distinct sessions"
    );

    // Regardless of HashSet iteration order, querying with a specific
    // scope must find that scope's session, not the other one bound to
    // the same placeholder.
    let found_b = broker
        .find_session_by_placeholder(&token, &scope_b)
        .unwrap()
        .expect("scope_b's session must be found among the placeholder's bound sessions");
    assert_eq!(found_b.correlation_id(), lease_b.session_id());

    let found_a = broker
        .find_session_by_placeholder(&token, &scope_a)
        .unwrap()
        .expect("scope_a's session must be found among the placeholder's bound sessions");
    assert_eq!(found_a.correlation_id(), lease_a.session_id());

    lease_a.revoke();
    lease_b.revoke();
}

#[test]
fn find_session_by_placeholder_skips_expired_and_exhausted_candidates_to_find_a_later_valid_one() {
    // Complements the test above: that one pins scope-mismatch skipping.
    // This pins that a *stale* candidate bound to the placeholder under
    // the *same* scope — expired, or use-exhausted — cannot prevent a
    // later, still-valid candidate bound to the identical placeholder
    // and scope from being found. `sessions_by_placeholder` is a
    // `HashSet`, so iteration order is unspecified; the loop in
    // `find_session_by_placeholder` must skip every non-matching
    // candidate regardless of where the valid one lands in that order.
    let broker = Arc::new(InMemoryCredentialBroker::new());
    let registry = CredentialPlaceholderRegistry::new();
    let tenant = TenantId::new("tenant-a").unwrap();
    let provider = ExtensionId::new("google").unwrap();
    let user = UserId::new("user-stale-candidates").unwrap();
    let token = registry.get_or_create(&tenant, &user, &provider).unwrap();
    let scope = sample_scope("tenant-a", "user-stale-candidates");

    let expired_account = CredentialAccountId::new("google_expired").unwrap();
    let exhausted_account = CredentialAccountId::new("google_exhausted").unwrap();
    let valid_account = CredentialAccountId::new("google_valid").unwrap();
    for (account_id, key) in [
        (&expired_account, "key_expired"),
        (&exhausted_account, "key_exhausted"),
        (&valid_account, "key_valid"),
    ] {
        broker
            .put_account(sample_account(
                scope.clone(),
                account_id.clone(),
                SecretHandle::new(key).unwrap(),
            ))
            .unwrap();
    }

    let mut expired_request = session_request(
        scope.clone(),
        expired_account,
        "https://api.example.com/v1/x",
    );
    expired_request.expires_at = Some(Utc::now() - chrono::Duration::seconds(1));
    let expired_lease = broker.mint_on_first_use(&token, expired_request).unwrap();

    let mut exhausted_request = session_request(
        scope.clone(),
        exhausted_account,
        "https://api.example.com/v1/x",
    );
    exhausted_request.max_uses = Some(1);
    let exhausted_lease = broker.mint_on_first_use(&token, exhausted_request).unwrap();
    broker
        .consume_session_use(exhausted_lease.session_id(), Utc::now())
        .unwrap();

    let valid_request =
        session_request(scope.clone(), valid_account, "https://api.example.com/v1/x");
    let valid_lease = broker.mint_on_first_use(&token, valid_request).unwrap();

    let found = broker
        .find_session_by_placeholder(&token, &scope)
        .unwrap()
        .expect(
            "a valid session must be found even with expired/exhausted candidates \
             also bound to the same placeholder and scope",
        );
    assert_eq!(found.correlation_id(), valid_lease.session_id());

    expired_lease.revoke();
    exhausted_lease.revoke();
    valid_lease.revoke();
}

#[test]
fn mint_on_first_use_remints_when_prior_jit_session_is_expired() {
    // The jit_minted cache remembers a session id per (invocation,
    // capability, account) key so re-use doesn't mint twice — but only
    // while that session is still live. This pins the fallthrough
    // branch: a cached key whose session has since expired must not be
    // handed back; a fresh session must be minted instead.
    let broker = Arc::new(InMemoryCredentialBroker::new());
    let registry = CredentialPlaceholderRegistry::new();
    let (token, scope_a, account_id) = seeded(&broker, &registry, "user-remint");

    let mut first_request = session_request(
        scope_a.clone(),
        account_id.clone(),
        "https://api.example.com/v1/x",
    );
    first_request.expires_at = Some(Utc::now() - chrono::Duration::seconds(1));
    let first_lease = broker.mint_on_first_use(&token, first_request).unwrap();
    let first_session_id = first_lease.session_id();
    assert!(
        broker
            .validate_session(first_session_id, Utc::now())
            .is_err(),
        "sanity: the first session must already be expired"
    );

    // The jit_minted cache entry still points at the now-expired
    // session; a second call for the identical key must not hand back a
    // lease for the dead session — it must remint.
    let second_request = session_request(scope_a, account_id, "https://api.example.com/v1/x");
    let second_lease = broker.mint_on_first_use(&token, second_request).unwrap();
    assert_ne!(
        first_session_id,
        second_lease.session_id(),
        "an expired jit-cached session must not be reused"
    );
    assert!(
        broker
            .validate_session(second_lease.session_id(), Utc::now())
            .is_ok(),
        "the reminted session must be live"
    );

    first_lease.revoke();
    second_lease.revoke();
}

#[test]
fn finish_lease_recovers_from_poisoned_placeholder_index_without_leaking_session() {
    // Regression test for the critical leak: `bind_placeholder_to_session`
    // used to be the one index write in this module that didn't use
    // `lock_or_recover`, so a poisoned `sessions_by_placeholder` mutex
    // made it return `Err` — and `finish_lease` used to construct the
    // `CredentialSessionLease` only *after* that fallible bind, so the
    // `?` propagated with the session already refcounted by the caller
    // but no lease ever constructed to drop and release that reference: a
    // standing grant nobody could revoke.
    //
    // Since the session-lifecycle collapse (`sessions`, `jit_minted`, and
    // `sessions_by_placeholder` now share one `session_state` mutex),
    // there is no longer a separate index write that can fail after the
    // session is already referenced: the whole mint-then-publish sequence
    // in `mint_on_first_use` runs under one `lock_or_recover` guard and
    // commits atomically. Poisoning that single mutex and confirming the
    // returned lease still mints, binds, and fully revokes proves the
    // same property this test always pinned, now trivially rather than
    // by careful ordering.
    let broker = Arc::new(InMemoryCredentialBroker::new());
    let registry = CredentialPlaceholderRegistry::new();
    let (token, scope_a, account_id) = seeded(&broker, &registry, "user-poison");

    // Poison `session_state` by panicking while holding its lock.
    let poison_broker = Arc::clone(&broker);
    let _ = std::panic::catch_unwind(AssertUnwindSafe(move || {
        let _guard = poison_broker.session_state.lock().unwrap();
        panic!("deliberately poison session_state for regression test");
    }));

    let lease = broker
        .mint_on_first_use(
            &token,
            session_request(scope_a.clone(), account_id, "https://api.example.com/v1/x"),
        )
        .expect(
            "mint_on_first_use must recover from a poisoned session_state lock, not fail \
             closed with an already-referenced, un-leased session",
        );
    let session_id = lease.session_id();

    assert!(
        broker
            .find_session_by_placeholder(&token, &scope_a)
            .unwrap()
            .is_some(),
        "recovery from the poisoned lock must still actually bind the session"
    );

    // Revoking the lease must fully release it: no standing grant left behind.
    lease.revoke();
    assert!(
        matches!(
            broker.validate_session(session_id, Utc::now()),
            Err(CredentialBrokerError::UnknownSession { .. })
        ),
        "the session must be genuinely revocable, not stranded past lease drop"
    );
    assert!(
        broker
            .find_session_by_placeholder(&token, &scope_a)
            .unwrap()
            .is_none(),
        "no dangling sessions_by_placeholder entry may survive revoke"
    );
}

#[test]
fn registry_get_or_create_token_always_resolves_immediately() {
    // Regression test: `get_or_create` used to publish into `by_owner`,
    // drop that guard, and only then lock `by_token` separately. A
    // concurrent call for the same triple in that window would take the
    // early-return `by_owner` hit and hand out a token `resolve()` still
    // answered `None` for — the registry's stated contract ("resolves a
    // placeholder back to its owner") was only *eventually* true. Both
    // maps now live behind one `Mutex<RegistryState>`, so every token
    // `get_or_create` returns already resolves by the time the call
    // returns, with no window at all.
    let registry = CredentialPlaceholderRegistry::new();
    let tenant = TenantId::new("tenant-a").unwrap();
    let user = UserId::new("user-resolve-atomic").unwrap();
    let provider = ExtensionId::new("google").unwrap();

    let token = registry.get_or_create(&tenant, &user, &provider).unwrap();
    let owner = registry
        .resolve(&token)
        .unwrap()
        .expect("a token returned by get_or_create must resolve immediately");
    assert_eq!(owner.tenant_id, tenant);
    assert_eq!(owner.user_id, user);
    assert_eq!(owner.provider_or_extension_id, provider);
}

#[test]
fn mint_on_first_use_is_race_free_under_concurrent_first_use() {
    // Regression test for a confirmed race: `jit_minted_session_id`
    // (read) and `record_jit_mint` (write) used to lock/unlock
    // `jit_minted` *separately*, so two threads racing on the identical
    // `JitMintKey` could both observe "nothing minted yet" and both mint
    // their own session — defeating the one-session-per-binding
    // guarantee this module's doc comment asserts, and multiplying a
    // `max_uses: Some(1)` budget. The fix holds one lock across the
    // whole lookup-or-mint sequence in `mint_on_first_use`.
    //
    // This is only probabilistic at *catching* a reintroduced race — a
    // `Barrier`-aligned start makes the race window likely to be hit,
    // not guaranteed. But it has zero false-fail risk once the fix is
    // in place: the lock enforces exclusion structurally, so a passing
    // run here is a real guarantee, not a lucky one.
    const THREAD_COUNT: usize = 32;

    let broker = Arc::new(InMemoryCredentialBroker::new());
    let registry = CredentialPlaceholderRegistry::new();
    let (token, scope_a, account_id) = seeded(&broker, &registry, "user-race");
    let barrier = Arc::new(std::sync::Barrier::new(THREAD_COUNT));

    let handles: Vec<_> = (0..THREAD_COUNT)
        .map(|_| {
            let broker = Arc::clone(&broker);
            let token = token.clone();
            let scope_a = scope_a.clone();
            let account_id = account_id.clone();
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                let request = session_request(scope_a, account_id, "https://api.example.com/v1/x");
                barrier.wait();
                broker
                    .mint_on_first_use(&token, request)
                    .expect("mint_on_first_use must succeed for every racing thread")
            })
        })
        .collect();

    let leases: Vec<CredentialSessionLease> = handles
        .into_iter()
        .map(|handle| {
            handle
                .join()
                .expect("mint_on_first_use thread must not panic")
        })
        .collect();

    let distinct_session_ids: std::collections::HashSet<_> =
        leases.iter().map(|lease| lease.session_id()).collect();
    assert_eq!(
        distinct_session_ids.len(),
        1,
        "every thread racing on the identical (invocation, capability, account) binding \
         must be handed a lease for the exact same session, not one each"
    );

    for lease in leases {
        lease.revoke();
    }
}

fn seeded(
    broker: &Arc<InMemoryCredentialBroker>,
    registry: &CredentialPlaceholderRegistry,
    user: &str,
) -> (
    CredentialPlaceholderToken,
    ResourceScope,
    CredentialAccountId,
) {
    let tenant = TenantId::new("tenant-a").unwrap();
    let user_id = UserId::new(user).unwrap();
    let provider = ExtensionId::new("google").unwrap();
    let token = registry
        .get_or_create(&tenant, &user_id, &provider)
        .unwrap();
    let caller_scope = sample_scope("tenant-a", user);
    let account_id = CredentialAccountId::new("google_prod").unwrap();
    broker
        .put_account(sample_account(
            caller_scope.clone(),
            account_id.clone(),
            SecretHandle::new("google_key").unwrap(),
        ))
        .unwrap();
    (token, caller_scope, account_id)
}
