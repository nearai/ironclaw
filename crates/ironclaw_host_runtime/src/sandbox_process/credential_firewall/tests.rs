use super::*;
use ironclaw_host_api::NetworkMethod;
use ironclaw_secrets::CredentialPathPolicy;

const FAR_FUTURE: Duration = Duration::from_secs(3600);

fn tenant(value: &str) -> TenantId {
    TenantId::new(value).unwrap()
}

fn user(value: &str) -> UserId {
    UserId::new(value).unwrap()
}

fn handle(value: &str) -> SecretHandle {
    SecretHandle::new(value).unwrap()
}

fn allow_all_targets() -> Vec<CredentialTargetPolicy> {
    Vec::new()
}

fn far_future_deadline() -> Instant {
    Instant::now() + FAR_FUTURE
}

#[test]
fn staged_obligation_is_retrievable_by_its_tenant_and_user() {
    let firewall = Arc::new(SandboxCredentialFirewall::new());
    let tenant_a = tenant("tenant-a");
    let user_a = user("user-a");
    let _lease = firewall.stage(
        &tenant_a,
        &user_a,
        StagedCredentialObligation::new(handle("github-token"), allow_all_targets(), FAR_FUTURE),
    );

    let decision = firewall
        .authorize(Some((&tenant_a, &user_a)), far_future_deadline())
        .expect("attributed lookup within deadline must not error");

    match decision {
        SandboxCredentialDecision::Grant(obligations) => {
            assert_eq!(obligations.len(), 1);
            assert_eq!(obligations[0].secret_handle, handle("github-token"));
        }
        SandboxCredentialDecision::NoGrant => {
            panic!("expected a grant for the exact (tenant, user) that staged it")
        }
    }
}

/// GRANT-DENIAL: no obligation was ever staged for this (tenant, user).
/// Per D5, this must be a *decision* (safe to forward bare), never an
/// `Err` (which would tear the connection down) — a wrong-reason
/// failure here would be returning `Err` instead of `Ok(NoGrant)`, which
/// would incorrectly deny public/uncredentialed traffic outright.
#[test]
fn no_staged_obligation_is_grant_denial_not_connection_denial() {
    let firewall = Arc::new(SandboxCredentialFirewall::new());
    let tenant_a = tenant("tenant-a");
    let user_a = user("user-a");

    let decision = firewall
        .authorize(Some((&tenant_a, &user_a)), far_future_deadline())
        .expect("an unstaged lookup is a decision, not a firewall error");

    assert_eq!(decision, SandboxCredentialDecision::NoGrant);
}

/// CONNECTION-DENIAL: attribution itself failed (proxy passes `None`).
/// Must be a distinct `Err` variant from `LookupTimedOut` so a caller
/// cannot collapse both into one branch and lose the audit distinction.
#[test]
fn unattributed_connection_is_denied_not_forwarded() {
    let firewall = Arc::new(SandboxCredentialFirewall::new());

    let error = firewall
        .authorize(None, far_future_deadline())
        .expect_err("an unattributed connection must never resolve to a decision");

    assert_eq!(error, SandboxCredentialFirewallError::AttributionFailed);
}

/// CONNECTION-DENIAL: the lookup deadline already passed. Must deny,
/// never silently fall through to a `NoGrant` decision that a caller
/// might treat as "safe to forward" — a wrong-reason pass here would be
/// the timeout path returning `Ok(NoGrant)` instead of `Err`.
#[test]
fn expired_deadline_denies_the_connection_even_when_a_grant_is_staged() {
    let firewall = Arc::new(SandboxCredentialFirewall::new());
    let tenant_a = tenant("tenant-a");
    let user_a = user("user-a");
    let _lease = firewall.stage(
        &tenant_a,
        &user_a,
        StagedCredentialObligation::new(handle("github-token"), allow_all_targets(), FAR_FUTURE),
    );
    let already_passed = Instant::now()
        .checked_sub(Duration::from_secs(1))
        .expect("process has been running for at least 1 second by the time tests run");

    let error = firewall
        .authorize(Some((&tenant_a, &user_a)), already_passed)
        .expect_err("a deadline that already passed must deny even with a live grant staged");

    assert_eq!(error, SandboxCredentialFirewallError::LookupTimedOut);
}

/// A staged obligation past its own TTL is GRANT-DENIAL (safe to
/// forward bare), distinct from a timed-out *lookup* — expiry is a
/// property of the obligation, not of how long authorization took.
#[test]
fn expired_obligation_is_grant_denial() {
    let firewall = Arc::new(SandboxCredentialFirewall::new());
    let tenant_a = tenant("tenant-a");
    let user_a = user("user-a");
    let _lease = firewall.stage(
        &tenant_a,
        &user_a,
        StagedCredentialObligation::new(
            handle("github-token"),
            allow_all_targets(),
            Duration::ZERO,
        ),
    );
    // The obligation's `expires_at` is `Instant::now() + 0` at staging
    // time. Sleeping 2ms guarantees `authorize`'s own `now` (captured
    // when it runs, below) is strictly past `expires_at` regardless of
    // clock resolution, so the obligation is deterministically expired
    // by the time `authorize` checks it — independent of the `deadline`
    // argument, which is a far-future value precisely so this test
    // isolates obligation expiry from lookup-deadline expiry.
    std::thread::sleep(Duration::from_millis(2));

    let decision = firewall
        .authorize(Some((&tenant_a, &user_a)), far_future_deadline())
        .expect("expiry is a decision, not a firewall error");

    assert_eq!(decision, SandboxCredentialDecision::NoGrant);
}

/// Explicit revoke (D4's primary bound) removes a grant immediately,
/// without waiting for its TTL.
#[test]
fn revoke_removes_a_staged_obligation_before_its_ttl_expires() {
    let firewall = Arc::new(SandboxCredentialFirewall::new());
    let tenant_a = tenant("tenant-a");
    let user_a = user("user-a");
    let lease = firewall.stage(
        &tenant_a,
        &user_a,
        StagedCredentialObligation::new(handle("github-token"), allow_all_targets(), FAR_FUTURE),
    );

    lease.revoke();

    let decision = firewall
        .authorize(Some((&tenant_a, &user_a)), far_future_deadline())
        .expect("revoked lookup is a decision, not a firewall error");
    assert_eq!(decision, SandboxCredentialDecision::NoGrant);
}

/// Cross-user isolation: user B must never retrieve user A's staged
/// obligation, even under the same tenant. A wrong-reason pass here
/// would be keying staging by `tenant_id` alone and ignoring `user_id`.
#[test]
fn user_b_cannot_retrieve_user_a_staged_obligation() {
    let firewall = Arc::new(SandboxCredentialFirewall::new());
    let tenant_a = tenant("tenant-a");
    let user_a = user("user-a");
    let user_b = user("user-b");
    let _lease = firewall.stage(
        &tenant_a,
        &user_a,
        StagedCredentialObligation::new(handle("github-token"), allow_all_targets(), FAR_FUTURE),
    );

    let decision_for_b = firewall
        .authorize(Some((&tenant_a, &user_b)), far_future_deadline())
        .expect("an unstaged (tenant, user) is a decision, not a firewall error");
    assert_eq!(decision_for_b, SandboxCredentialDecision::NoGrant);

    // Sanity: user A's own lookup still resolves — proves the isolation
    // above is real key-scoping, not a bug that denies everyone.
    let decision_for_a = firewall
        .authorize(Some((&tenant_a, &user_a)), far_future_deadline())
        .expect("user a's own lookup must still succeed");
    assert!(matches!(
        decision_for_a,
        SandboxCredentialDecision::Grant(_)
    ));
}

/// An expired obligation must be reclaimed from the staging map on read,
/// not merely filtered on the way out. Without this, `staged` grows
/// monotonically with every distinct `(tenant_id, user_id)` the process
/// has ever seen — a caller that stages once and never returns leaves a
/// dead entry forever (unbounded-resource rule,
/// `.claude/rules/safety-and-sandbox.md`). The return value alone
/// (`NoGrant`) cannot distinguish "filtered on read" from "actually
/// removed" — `expired_obligation_is_grant_denial` above already pins the
/// return value; this test pins the map side effect.
#[test]
fn expired_obligation_is_removed_from_staging_map_on_read() {
    let firewall = Arc::new(SandboxCredentialFirewall::new());
    let tenant_a = tenant("tenant-a");
    let user_a = user("user-a");
    let _lease = firewall.stage(
        &tenant_a,
        &user_a,
        StagedCredentialObligation::new(
            handle("github-token"),
            allow_all_targets(),
            Duration::ZERO,
        ),
    );
    std::thread::sleep(Duration::from_millis(2));
    assert_eq!(
        firewall.staged_len(),
        1,
        "sanity: obligation must be staged before authorize reclaims it"
    );

    let decision = firewall
        .authorize(Some((&tenant_a, &user_a)), far_future_deadline())
        .expect("expiry is a decision, not a firewall error");

    assert_eq!(decision, SandboxCredentialDecision::NoGrant);
    assert_eq!(
        firewall.staged_len(),
        0,
        "expired obligation must be removed from the staging map on read, \
         not just filtered from the returned decision"
    );
}

/// Same user id under a different tenant must also be isolated — the
/// staging key is the full `(tenant_id, user_id)` pair, not user_id alone.
#[test]
fn same_user_id_under_a_different_tenant_is_isolated() {
    let firewall = Arc::new(SandboxCredentialFirewall::new());
    let tenant_a = tenant("tenant-a");
    let tenant_b = tenant("tenant-b");
    let shared_user = user("user-a");
    let _lease = firewall.stage(
        &tenant_a,
        &shared_user,
        StagedCredentialObligation::new(handle("github-token"), allow_all_targets(), FAR_FUTURE),
    );

    let decision = firewall
        .authorize(Some((&tenant_b, &shared_user)), far_future_deadline())
        .expect("an unstaged (tenant, user) is a decision, not a firewall error");

    assert_eq!(decision, SandboxCredentialDecision::NoGrant);
}

/// Staging twice for the same `(tenant_id, user_id)` key adds a second
/// live entry — it does NOT replace the first. The per-user sandbox
/// concurrency ceiling (`ironclaw_host_api::resource::SandboxQuota`) is >1, so two invocations
/// staging for the same principal at once is the normal path, and both
/// obligations must stay live until each is independently revoked.
///
/// This supersedes an earlier version of this test (single-slot design,
/// concurrency ceiling of 1) that asserted restaging *replaced* the
/// prior obligation — that assertion is no longer correct now that the
/// map holds a set per key rather than one slot.
#[test]
fn staging_the_same_key_twice_keeps_both_obligations_live() {
    let firewall = Arc::new(SandboxCredentialFirewall::new());
    let tenant_a = tenant("tenant-a");
    let user_a = user("user-a");
    let _lease_old = firewall.stage(
        &tenant_a,
        &user_a,
        StagedCredentialObligation::new(handle("old-token"), allow_all_targets(), FAR_FUTURE),
    );
    let _lease_new = firewall.stage(
        &tenant_a,
        &user_a,
        StagedCredentialObligation::new(handle("new-token"), allow_all_targets(), FAR_FUTURE),
    );

    assert_grant_contains(
        &firewall,
        &tenant_a,
        &user_a,
        &[handle("old-token"), handle("new-token")],
    );
}

/// HIGH-severity regression: a stale lease dropped *after* a newer
/// invocation has restaged the same `(tenant_id, user_id)` must not
/// revoke the newer invocation's still-live grant.
///
/// `staging_the_same_key_twice_keeps_both_obligations_live` above cannot
/// catch this: each lease already owns its own entry in the refcounted
/// set, so `_lease_old` and `_lease_new` falling out of scope in reverse
/// declaration order just removes each one's own entry — a harmless,
/// order-independent outcome that would pass whether or not per-entry
/// isolation actually holds. This test forces the actual hazard order:
/// drop the older lease *first*, while the newer lease (and its grant)
/// is still alive, to prove one lease's revoke never reaches into a
/// sibling's still-live entry.
#[test]
fn dropping_a_stale_lease_does_not_revoke_a_newer_grant_for_the_same_key() {
    let firewall = Arc::new(SandboxCredentialFirewall::new());
    let tenant_a = tenant("tenant-a");
    let user_a = user("user-a");

    let lease_old = firewall.stage(
        &tenant_a,
        &user_a,
        StagedCredentialObligation::new(handle("old-token"), allow_all_targets(), FAR_FUTURE),
    );
    let lease_new = firewall.stage(
        &tenant_a,
        &user_a,
        StagedCredentialObligation::new(handle("new-token"), allow_all_targets(), FAR_FUTURE),
    );

    // The hazard order: the older invocation finishes and drops its
    // lease first, while the newer invocation's lease — and grant — is
    // still alive.
    drop(lease_old);

    assert_grant_contains(&firewall, &tenant_a, &user_a, &[handle("new-token")]);

    // Overshoot guard: the fix must not go so far that dropping the
    // *newest* lease stops revoking. `lease_new` is still live, so
    // dropping it must still remove its own entry — and since it was
    // the last entry left for this key, the key itself must be gone.
    drop(lease_new);
    let decision_after_newest_drop = firewall
        .authorize(Some((&tenant_a, &user_a)), far_future_deadline())
        .expect("attributed lookup within deadline must not error");
    assert_eq!(
        decision_after_newest_drop,
        SandboxCredentialDecision::NoGrant,
        "dropping the current (newest) lease must still revoke its grant"
    );
}

/// The primary RAII guarantee: dropping the lease revokes the staged
/// obligation, mirroring `CredentialSessionLease`'s Drop-as-backstop
/// contract. This is the structural fix for the #6689-shaped leak class —
/// no caller discipline required.
#[test]
fn dropping_the_lease_revokes_the_staged_obligation() {
    let firewall = Arc::new(SandboxCredentialFirewall::new());
    let tenant_a = tenant("tenant-a");
    let user_a = user("user-a");
    let lease = firewall.stage(
        &tenant_a,
        &user_a,
        StagedCredentialObligation::new(handle("github-token"), allow_all_targets(), FAR_FUTURE),
    );

    drop(lease);

    let decision = firewall
        .authorize(Some((&tenant_a, &user_a)), far_future_deadline())
        .expect("post-drop lookup is a decision, not a firewall error");
    assert_eq!(decision, SandboxCredentialDecision::NoGrant);
}

/// Explicit `revoke(self)` is the fast path for success/error exits. It
/// must revoke immediately, and the `Drop` that still runs on the
/// moved-from value at scope end must not double-revoke or panic.
#[test]
fn explicit_revoke_does_not_double_revoke_or_panic_on_drop() {
    let firewall = Arc::new(SandboxCredentialFirewall::new());
    let tenant_a = tenant("tenant-a");
    let user_a = user("user-a");
    let lease = firewall.stage(
        &tenant_a,
        &user_a,
        StagedCredentialObligation::new(handle("github-token"), allow_all_targets(), FAR_FUTURE),
    );

    lease.revoke();

    let decision = firewall
        .authorize(Some((&tenant_a, &user_a)), far_future_deadline())
        .expect("post-revoke lookup is a decision, not a firewall error");
    assert_eq!(decision, SandboxCredentialDecision::NoGrant);
}

/// The backstop path: a panic mid-dispatch still unwinds through the
/// lease's `Drop`, so a standing grant cannot survive a panic the way an
/// explicit-only `revoke()` discipline would miss (the #6689 shape).
#[test]
fn lease_dropped_during_unwind_still_revokes() {
    let firewall = Arc::new(SandboxCredentialFirewall::new());
    let tenant_a = tenant("tenant-a");
    let user_a = user("user-a");
    let lease = firewall.stage(
        &tenant_a,
        &user_a,
        StagedCredentialObligation::new(handle("github-token"), allow_all_targets(), FAR_FUTURE),
    );

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _lease = lease;
        panic!("simulated mid-dispatch panic");
    }));
    assert!(
        result.is_err(),
        "the panic must actually unwind for this test to be meaningful"
    );

    let decision = firewall
        .authorize(Some((&tenant_a, &user_a)), far_future_deadline())
        .expect("post-unwind lookup is a decision, not a firewall error");
    assert_eq!(decision, SandboxCredentialDecision::NoGrant);
}

/// TTL clamp: `MAX_GRANT_TTL` is the outer backstop for a caller that
/// supplies an unreasonable TTL (or for a lease whose `Drop` never runs,
/// e.g. `std::mem::forget`) — a 24h request must not be honored verbatim.
#[test]
fn ttl_is_clamped_to_max_grant_ttl_backstop() {
    let firewall = Arc::new(SandboxCredentialFirewall::new());
    let tenant_a = tenant("tenant-a");
    let user_a = user("user-a");
    let lease = firewall.stage(
        &tenant_a,
        &user_a,
        StagedCredentialObligation::new(
            handle("github-token"),
            allow_all_targets(),
            Duration::from_secs(24 * 60 * 60),
        ),
    );
    // Captured *after* `stage()` returns, so the internal `Instant::now()`
    // that `StagedCredentialObligation::new` clamped against is guaranteed
    // to be at or before this point — an upper bound, not a racy lower one.
    let after_staging = Instant::now();

    let decision = firewall
        .authorize(Some((&tenant_a, &user_a)), far_future_deadline())
        .expect("attributed lookup within deadline must not error");
    let obligations = match decision {
        SandboxCredentialDecision::Grant(obligations) => obligations,
        SandboxCredentialDecision::NoGrant => panic!("expected a grant"),
    };
    assert_eq!(obligations.len(), 1);

    assert!(
        obligations[0].expires_at <= after_staging + MAX_GRANT_TTL,
        "a 24h TTL must be clamped to MAX_GRANT_TTL, not honored verbatim"
    );

    lease.revoke();
}

/// Helper: assert that `authorize` currently returns a live grant whose
/// obligation secret handles are exactly `expected` (order-independent —
/// the staging map is a `HashMap`, so iteration order over a key's set
/// is not guaranteed).
fn assert_grant_contains(
    firewall: &Arc<SandboxCredentialFirewall>,
    tenant_id: &TenantId,
    user_id: &UserId,
    expected: &[SecretHandle],
) {
    let decision = firewall
        .authorize(Some((tenant_id, user_id)), far_future_deadline())
        .expect("attributed lookup within deadline must not error");
    match decision {
        SandboxCredentialDecision::Grant(obligations) => {
            let actual: std::collections::HashSet<_> = obligations
                .iter()
                .map(|o| o.secret_handle.clone())
                .collect();
            let expected_set: std::collections::HashSet<_> = expected.iter().cloned().collect();
            assert_eq!(
                actual, expected_set,
                "live grant set did not match the expected staged obligations"
            );
        }
        SandboxCredentialDecision::NoGrant => {
            panic!("expected a live grant set containing {expected:?}, got NoGrant")
        }
    }
}

/// N-safety: with the per-user sandbox concurrency ceiling now >1
/// (`ironclaw_host_api::resource::SandboxQuota`), 4 concurrent invocations staging for the same
/// `(tenant_id, user_id)` is the normal case. Dropping their leases in a
/// SHUFFLED order (neither LIFO nor FIFO — the order most likely to
/// defeat an implementation that only happens to work for stack- or
/// queue-shaped drop patterns) must revoke only each dropped lease's own
/// entry; every remaining sibling grant must survive until its own
/// lease drops, and the key must disappear only once the last one does.
#[test]
fn dropping_four_concurrent_leases_in_shuffled_order_only_revokes_each_ones_own_entry() {
    let firewall = Arc::new(SandboxCredentialFirewall::new());
    let tenant_a = tenant("tenant-a");
    let user_a = user("user-a");

    let lease_a = firewall.stage(
        &tenant_a,
        &user_a,
        StagedCredentialObligation::new(handle("token-a"), allow_all_targets(), FAR_FUTURE),
    );
    let lease_b = firewall.stage(
        &tenant_a,
        &user_a,
        StagedCredentialObligation::new(handle("token-b"), allow_all_targets(), FAR_FUTURE),
    );
    let lease_c = firewall.stage(
        &tenant_a,
        &user_a,
        StagedCredentialObligation::new(handle("token-c"), allow_all_targets(), FAR_FUTURE),
    );
    let lease_d = firewall.stage(
        &tenant_a,
        &user_a,
        StagedCredentialObligation::new(handle("token-d"), allow_all_targets(), FAR_FUTURE),
    );

    assert_eq!(
        firewall.staged_len(),
        4,
        "sanity: all four concurrent obligations must be staged before any drop"
    );

    // Shuffled order: c, a, d, b.
    drop(lease_c);
    assert_eq!(firewall.staged_len(), 3);
    assert_grant_contains(
        &firewall,
        &tenant_a,
        &user_a,
        &[handle("token-a"), handle("token-b"), handle("token-d")],
    );

    drop(lease_a);
    assert_eq!(firewall.staged_len(), 2);
    assert_grant_contains(
        &firewall,
        &tenant_a,
        &user_a,
        &[handle("token-b"), handle("token-d")],
    );

    drop(lease_d);
    assert_eq!(firewall.staged_len(), 1);
    assert_grant_contains(&firewall, &tenant_a, &user_a, &[handle("token-b")]);

    drop(lease_b);
    assert_eq!(
        firewall.staged_len(),
        0,
        "the key's set must be gone once its last entry drops"
    );
    let decision = firewall
        .authorize(Some((&tenant_a, &user_a)), far_future_deadline())
        .expect("attributed lookup within deadline must not error");
    assert_eq!(
        decision,
        SandboxCredentialDecision::NoGrant,
        "no entries remain staged for this key after the last lease drops"
    );
}

/// Expiry reclamation on read must remove only the expired entry from a
/// key's set, not the whole key — live siblings staged under the same
/// `(tenant_id, user_id)` must survive the reclaim.
#[test]
fn expiry_reclaims_one_entry_without_disturbing_live_siblings() {
    let firewall = Arc::new(SandboxCredentialFirewall::new());
    let tenant_a = tenant("tenant-a");
    let user_a = user("user-a");

    let _short_lived = firewall.stage(
        &tenant_a,
        &user_a,
        StagedCredentialObligation::new(
            handle("expiring-token"),
            allow_all_targets(),
            Duration::ZERO,
        ),
    );
    let _long_lived = firewall.stage(
        &tenant_a,
        &user_a,
        StagedCredentialObligation::new(handle("live-token"), allow_all_targets(), FAR_FUTURE),
    );
    std::thread::sleep(Duration::from_millis(2));

    assert_eq!(
        firewall.staged_len(),
        2,
        "sanity: both obligations staged before the expired one is reclaimed"
    );

    assert_grant_contains(&firewall, &tenant_a, &user_a, &[handle("live-token")]);
    assert_eq!(
        firewall.staged_len(),
        1,
        "the expired sibling must be reclaimed, the live one must remain"
    );
}

/// Cross-user isolation must still hold when a key has multiple staged
/// entries: user B must never see any of user A's entries, even under
/// the same tenant.
#[test]
fn user_b_cannot_retrieve_any_of_user_a_multiple_staged_obligations() {
    let firewall = Arc::new(SandboxCredentialFirewall::new());
    let tenant_a = tenant("tenant-a");
    let user_a = user("user-a");
    let user_b = user("user-b");
    let _lease_1 = firewall.stage(
        &tenant_a,
        &user_a,
        StagedCredentialObligation::new(handle("token-1"), allow_all_targets(), FAR_FUTURE),
    );
    let _lease_2 = firewall.stage(
        &tenant_a,
        &user_a,
        StagedCredentialObligation::new(handle("token-2"), allow_all_targets(), FAR_FUTURE),
    );

    let decision_for_b = firewall
        .authorize(Some((&tenant_a, &user_b)), far_future_deadline())
        .expect("an unstaged (tenant, user) is a decision, not a firewall error");
    assert_eq!(decision_for_b, SandboxCredentialDecision::NoGrant);

    // Sanity: user A's own lookup still resolves both entries.
    assert_grant_contains(
        &firewall,
        &tenant_a,
        &user_a,
        &[handle("token-1"), handle("token-2")],
    );
}

/// The firewall stops at "here is everything currently entitled" and
/// leaves target matching to the future proxy caller (see
/// `SandboxCredentialDecision::Grant`'s doc) — but that only works if
/// non-empty `allowed_targets` actually survive `stage`/`authorize`
/// unchanged. Every other test in this module stages an empty policy
/// vector; this one pins that real policies round-trip intact.
#[test]
fn non_empty_target_policies_survive_stage_and_authorize_unchanged() {
    let firewall = Arc::new(SandboxCredentialFirewall::new());
    let tenant_a = tenant("tenant-a");
    let user_a = user("user-a");
    let policies = vec![
        CredentialTargetPolicy {
            scheme: "https".to_string(),
            host: "api.example.com".to_string(),
            port: Some(443),
            path: CredentialPathPolicy::Prefix("/v1".to_string()),
            methods: vec![NetworkMethod::Get],
        },
        CredentialTargetPolicy {
            scheme: "https".to_string(),
            host: "upload.example.com".to_string(),
            port: None,
            path: CredentialPathPolicy::Exact("/upload".to_string()),
            methods: vec![NetworkMethod::Post],
        },
    ];
    let _lease = firewall.stage(
        &tenant_a,
        &user_a,
        StagedCredentialObligation::new(handle("github-token"), policies.clone(), FAR_FUTURE),
    );

    let decision = firewall
        .authorize(Some((&tenant_a, &user_a)), far_future_deadline())
        .expect("attributed lookup within deadline must not error");

    match decision {
        SandboxCredentialDecision::Grant(obligations) => {
            assert_eq!(obligations.len(), 1);
            assert_eq!(
                obligations[0].allowed_targets, policies,
                "allowed_targets must round-trip through stage/authorize unchanged"
            );
        }
        SandboxCredentialDecision::NoGrant => panic!("expected a grant"),
    }
}

/// Real contention exercise (not this module's usual single-thread
/// pattern): several threads staging, authorizing, and revoking
/// concurrently for the same `(tenant_id, user_id)` must still leave
/// each entry's lifecycle isolated from its siblings' — the property
/// the shuffled-drop-order test above pins sequentially, exercised here
/// under real thread interleaving.
#[test]
fn concurrent_stage_authorize_and_revoke_preserve_per_entry_isolation() {
    let firewall = Arc::new(SandboxCredentialFirewall::new());
    let tenant_a = tenant("tenant-a");
    let user_a = user("user-a");

    let mut handles = Vec::new();
    for i in 0..8 {
        let firewall = Arc::clone(&firewall);
        let tenant_a = tenant_a.clone();
        let user_a = user_a.clone();
        handles.push(std::thread::spawn(move || {
            let lease = firewall.stage(
                &tenant_a,
                &user_a,
                StagedCredentialObligation::new(
                    handle(&format!("token-{i}")),
                    allow_all_targets(),
                    FAR_FUTURE,
                ),
            );
            // Concurrent reads while siblings are still staging/revoking.
            let _ = firewall.authorize(Some((&tenant_a, &user_a)), far_future_deadline());
            lease.revoke();
        }));
    }

    for handle in handles {
        handle.join().expect("worker thread must not panic");
    }

    // Every entry revoked its own lease; none should be left staged.
    let decision = firewall
        .authorize(Some((&tenant_a, &user_a)), far_future_deadline())
        .expect("attributed lookup within deadline must not error");
    assert_eq!(
        decision,
        SandboxCredentialDecision::NoGrant,
        "all 8 concurrently staged-and-revoked entries must be gone"
    );
}

#[test]
fn staged_obligation_debug_output_never_contains_secret_handle() {
    let obligation =
        StagedCredentialObligation::new(handle("github-token"), allow_all_targets(), FAR_FUTURE);

    let debug_output = format!("{obligation:?}");

    // Regression: `StagedCredentialObligation`'s manual `Debug` must omit
    // `secret_handle` — only `allowed_targets`/`expires_at` are safe to print.
    assert!(!debug_output.contains("github-token"));
}

#[test]
fn firewall_debug_output_never_contains_staged_identity_or_secret_handles() {
    let firewall = Arc::new(SandboxCredentialFirewall::new());
    let tenant_a = tenant("tenant-a");
    let user_a = user("user-a");
    let _lease = firewall.stage(
        &tenant_a,
        &user_a,
        StagedCredentialObligation::new(handle("github-token"), allow_all_targets(), FAR_FUTURE),
    );

    let debug_output = format!("{firewall:?}");

    // Regression: `SandboxCredentialFirewall`'s manual `Debug` must expose
    // only an aggregate staged-key count, never the staged tenant/user
    // identity or any secret handle nested inside it.
    assert!(!debug_output.contains("tenant-a"));
    assert!(!debug_output.contains("user-a"));
    assert!(!debug_output.contains("github-token"));
}
