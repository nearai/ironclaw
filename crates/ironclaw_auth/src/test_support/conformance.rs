//! OAuth-flow state-machine CONFORMANCE suite — shared, observable-behavior
//! assertions every [`AuthFlowManager`] implementation must satisfy.
//!
//! Two production-facing implementations exist: this crate's
//! [`InMemoryAuthProductServices`](crate::InMemoryAuthProductServices) fake
//! (what most consumer tests run against) and the durable
//! `FilesystemAuthProductServices` in `ironclaw_reborn_composition` (what
//! production runs). Their VALIDATION core is shared (`domain.rs`'s
//! `validate_callback_claim` / `prepare_callback_flow`), but each hand-rolls
//! its orchestration around it — terminal-idempotency sets, expiry
//! write-back, account minting — so the suites could drift apart with nothing
//! failing. Until this module existed their agreement was coincidence, not
//! contract (found while pinning the #6105 T4 replay arm).
//!
//! Call [`assert_auth_flow_callback_conformance`] from each implementation's
//! own test tier:
//! - fake: `tests/auth_product_contract/oauth_flow_contract.rs` (this crate);
//! - durable: root `tests/integration/oauth_connect.rs`, over the composed
//!   `OAuthProductAuthTestBundle`'s `flow_manager()`.
//!
//! Panics with a case-labeled message on the first violation, matching the
//! contract-test style of both call sites. Feature-gated test vocabulary
//! (`#[cfg(any(test, feature = "test-support"))]`, under [`crate::test_support`]):
//! this crate's charter is auth contracts *and* their test vocabulary, but the
//! panic-on-violation harness must not ship in production binaries.

use chrono::{Duration, Utc};

use crate::{
    AuthChallenge, AuthContinuationRef, AuthFlowId, AuthFlowKind, AuthFlowManager, AuthFlowRecord,
    AuthFlowStatus, AuthProductError, AuthProductScope, AuthProviderId, AuthorizationCodeHash,
    CredentialAccountLabel, NewAuthFlow, OAuthAuthorizationUrl, OAuthCallbackInput,
    OAuthProviderExchange, OpaqueStateHash, PkceVerifierHash, ProviderCallbackOutcome,
};
use ironclaw_host_api::SecretHandle;

/// Deterministic 64-hex digest for conformance hash newtypes.
fn digest(tag: &str) -> String {
    format!(
        "{:064x}",
        tag.bytes().fold(0_u64, |hash, byte| {
            hash.wrapping_mul(31).wrapping_add(u64::from(byte))
        })
    )
}

fn state_hash(tag: &str) -> OpaqueStateHash {
    OpaqueStateHash::new(digest(tag)).expect("conformance state hash is valid")
}

fn pkce_hash(tag: &str) -> PkceVerifierHash {
    PkceVerifierHash::new(digest(tag)).expect("conformance pkce hash is valid")
}

fn new_flow(
    scope: &AuthProductScope,
    provider: &AuthProviderId,
    tag: &str,
    expires_at: chrono::DateTime<Utc>,
) -> NewAuthFlow {
    NewAuthFlow {
        id: None,
        scope: scope.clone(),
        kind: AuthFlowKind::IntegrationCredential,
        provider: provider.clone(),
        challenge: AuthChallenge::OAuthUrl {
            authorization_url: OAuthAuthorizationUrl::new("https://provider.example/oauth")
                .expect("conformance authorization url is valid"),
            expires_at,
        },
        continuation: AuthContinuationRef::SetupOnly,
        update_binding: None,
        opaque_state_hash: Some(state_hash(tag)),
        pkce_verifier_hash: Some(pkce_hash(tag)),
        expires_at,
    }
}

fn authorized_outcome(provider: &AuthProviderId, tag: &str) -> ProviderCallbackOutcome {
    ProviderCallbackOutcome::Authorized {
        exchange: Box::new(OAuthProviderExchange {
            provider: provider.clone(),
            account_label: CredentialAccountLabel::new(format!("conformance {tag}"))
                .expect("conformance account label is valid"),
            authorization_code_hash: AuthorizationCodeHash::new(digest(&format!("code-{tag}")))
                .expect("conformance code hash is valid"),
            pkce_verifier_hash: pkce_hash(tag),
            access_secret: SecretHandle::new("conformance_access_secret")
                .expect("conformance secret handle is valid"),
            refresh_secret: None,
            scopes: Vec::new(),
            account_id: None,
            provider_identity: None,
        }),
    }
}

fn callback_input(
    flow_id: AuthFlowId,
    tag: &str,
    outcome: ProviderCallbackOutcome,
) -> OAuthCallbackInput {
    OAuthCallbackInput {
        flow_id,
        opaque_state_hash: state_hash(tag),
        outcome,
    }
}

async fn read_flow(
    flows: &dyn AuthFlowManager,
    scope: &AuthProductScope,
    flow_id: AuthFlowId,
    case: &str,
) -> AuthFlowRecord {
    flows
        .get_flow(scope, flow_id)
        .await
        .unwrap_or_else(|error| panic!("[{case}] get_flow must not error: {error:?}"))
        .unwrap_or_else(|| panic!("[{case}] flow record must remain readable"))
}

/// Run the OAuth-callback state-machine conformance cases against
/// `flows`. `scope`/`provider` come from the caller's tier (the fake uses a
/// plain local scope; the durable tier its composed test scope). Panics with
/// a case-labeled message on the first violation.
pub async fn assert_auth_flow_callback_conformance(
    flows: &dyn AuthFlowManager,
    scope: &AuthProductScope,
    provider: &AuthProviderId,
) {
    completed_flow_claim_idempotent_and_complete_rejects_replay(flows, scope, provider).await;
    expired_flow_rejects_and_marks_expired(flows, scope, provider).await;
    canceled_flow_rejects_completion_as_canceled(flows, scope, provider).await;
    unknown_flow_rejects_completion(flows, scope, provider).await;
    state_hash_mismatch_denies_without_burning_the_flow(flows, scope, provider).await;
}

/// Happy completion, then both replay arms — the exact split the hosted
/// callback route depends on: a replayed CLAIM is idempotent (returns the
/// completed record so a duplicated browser redirect can short-circuit to
/// success), while a replayed COMPLETE stays fail-closed
/// (`FlowAlreadyTerminal`; nothing re-mints or overwrites the grant).
async fn completed_flow_claim_idempotent_and_complete_rejects_replay(
    flows: &dyn AuthFlowManager,
    scope: &AuthProductScope,
    provider: &AuthProviderId,
) {
    const CASE: &str = "completed-flow replay arms";
    let tag = "conformance-completed";
    let flow = flows
        .create_flow(new_flow(
            scope,
            provider,
            tag,
            Utc::now() + Duration::minutes(5),
        ))
        .await
        .unwrap_or_else(|error| panic!("[{CASE}] create_flow: {error:?}"));

    let claimed = flows
        .claim_oauth_callback(
            scope,
            crate::OAuthCallbackClaimRequest {
                flow_id: flow.id,
                opaque_state_hash: state_hash(tag),
                provider: provider.clone(),
                pkce_verifier_hash: pkce_hash(tag),
            },
        )
        .await
        .unwrap_or_else(|error| panic!("[{CASE}] first claim: {error:?}"));
    assert_eq!(
        claimed.status,
        AuthFlowStatus::CallbackReceived,
        "[{CASE}] first claim moves the flow to CallbackReceived"
    );

    let completed = flows
        .complete_oauth_callback(
            scope,
            callback_input(flow.id, tag, authorized_outcome(provider, tag)),
        )
        .await
        .unwrap_or_else(|error| panic!("[{CASE}] complete: {error:?}"));
    assert_eq!(
        completed.status,
        AuthFlowStatus::Completed,
        "[{CASE}] authorized completion lands on Completed"
    );
    let account_id = completed
        .credential_account_id
        .unwrap_or_else(|| panic!("[{CASE}] completion must mint a credential account"));

    // Replayed CLAIM (duplicated redirect): idempotent, same terminal record.
    let reclaimed = flows
        .claim_oauth_callback(
            scope,
            crate::OAuthCallbackClaimRequest {
                flow_id: flow.id,
                opaque_state_hash: state_hash(tag),
                provider: provider.clone(),
                pkce_verifier_hash: pkce_hash(tag),
            },
        )
        .await
        .unwrap_or_else(|error| {
            panic!("[{CASE}] a replayed claim on a completed flow must be idempotent: {error:?}")
        });
    assert_eq!(
        reclaimed.status,
        AuthFlowStatus::Completed,
        "[{CASE}] replayed claim returns the completed record"
    );
    assert_eq!(
        reclaimed.credential_account_id,
        Some(account_id),
        "[{CASE}] replayed claim returns the ORIGINAL grant's account"
    );

    // Replayed COMPLETE: fail-closed, and the record is untouched.
    let replay = flows
        .complete_oauth_callback(
            scope,
            callback_input(flow.id, tag, ProviderCallbackOutcome::Denied),
        )
        .await
        .expect_err("a replayed complete on a terminal flow must be rejected");
    assert_eq!(
        replay,
        AuthProductError::FlowAlreadyTerminal,
        "[{CASE}] replayed complete rejects FlowAlreadyTerminal"
    );
    let record = read_flow(flows, scope, flow.id, CASE).await;
    assert_eq!(
        record.status,
        AuthFlowStatus::Completed,
        "[{CASE}] record stays Completed"
    );
    assert_eq!(
        record.credential_account_id,
        Some(account_id),
        "[{CASE}] the original grant survives the rejected replay"
    );
}

/// A callback for a lapsed flow is rejected `UnknownOrExpiredFlow` and the
/// record is durably marked terminal `Expired` — not left half-claimable.
async fn expired_flow_rejects_and_marks_expired(
    flows: &dyn AuthFlowManager,
    scope: &AuthProductScope,
    provider: &AuthProviderId,
) {
    const CASE: &str = "expired flow";
    let tag = "conformance-expired";
    let flow = flows
        .create_flow(new_flow(
            scope,
            provider,
            tag,
            // 10s (not 1s) so second-precision timestamp truncation on durable
            // backends still lands the flow firmly in the past.
            Utc::now() - Duration::seconds(10),
        ))
        .await
        .unwrap_or_else(|error| panic!("[{CASE}] create_flow: {error:?}"));

    let error = flows
        .complete_oauth_callback(
            scope,
            callback_input(flow.id, tag, authorized_outcome(provider, tag)),
        )
        .await
        .expect_err("completing a lapsed flow must be rejected");
    assert_eq!(
        error,
        AuthProductError::UnknownOrExpiredFlow,
        "[{CASE}] lapsed completion rejects UnknownOrExpiredFlow"
    );
    let record = read_flow(flows, scope, flow.id, CASE).await;
    assert_eq!(
        record.status,
        AuthFlowStatus::Expired,
        "[{CASE}] the record is marked terminal Expired (write-back), not left pending"
    );
}

/// A canceled flow rejects completion with the cancel-specific error, and
/// cancel itself is not silently repeatable.
async fn canceled_flow_rejects_completion_as_canceled(
    flows: &dyn AuthFlowManager,
    scope: &AuthProductScope,
    provider: &AuthProviderId,
) {
    const CASE: &str = "canceled flow";
    let tag = "conformance-canceled";
    let flow = flows
        .create_flow(new_flow(
            scope,
            provider,
            tag,
            Utc::now() + Duration::minutes(5),
        ))
        .await
        .unwrap_or_else(|error| panic!("[{CASE}] create_flow: {error:?}"));
    flows
        .cancel_flow(scope, flow.id)
        .await
        .unwrap_or_else(|error| panic!("[{CASE}] cancel_flow: {error:?}"));

    let error = flows
        .complete_oauth_callback(
            scope,
            callback_input(flow.id, tag, authorized_outcome(provider, tag)),
        )
        .await
        .expect_err("completing a canceled flow must be rejected");
    assert_eq!(
        error,
        AuthProductError::Canceled,
        "[{CASE}] completion rejects with the cancel-specific error, not a generic terminal one"
    );
    let recancel = flows
        .cancel_flow(scope, flow.id)
        .await
        .expect_err("re-canceling a canceled flow must be rejected");
    assert_eq!(
        recancel,
        AuthProductError::Canceled,
        "[{CASE}] re-cancel rejects as Canceled"
    );
}

/// A callback for a flow id that was never created is rejected without
/// leaking whether the id ever existed.
async fn unknown_flow_rejects_completion(
    flows: &dyn AuthFlowManager,
    scope: &AuthProductScope,
    provider: &AuthProviderId,
) {
    const CASE: &str = "unknown flow";
    let tag = "conformance-unknown";
    let error = flows
        .complete_oauth_callback(
            scope,
            callback_input(AuthFlowId::new(), tag, authorized_outcome(provider, tag)),
        )
        .await
        .expect_err("completing an unknown flow must be rejected");
    assert_eq!(
        error,
        AuthProductError::UnknownOrExpiredFlow,
        "[{CASE}] unknown flow rejects UnknownOrExpiredFlow"
    );
}

/// A state-hash mismatch is denied as cross-scope — and the flow is NOT
/// burned by the failed attempt: the genuine callback still completes.
async fn state_hash_mismatch_denies_without_burning_the_flow(
    flows: &dyn AuthFlowManager,
    scope: &AuthProductScope,
    provider: &AuthProviderId,
) {
    const CASE: &str = "state-hash mismatch";
    let tag = "conformance-mismatch";
    let flow = flows
        .create_flow(new_flow(
            scope,
            provider,
            tag,
            Utc::now() + Duration::minutes(5),
        ))
        .await
        .unwrap_or_else(|error| panic!("[{CASE}] create_flow: {error:?}"));

    let error = flows
        .complete_oauth_callback(
            scope,
            OAuthCallbackInput {
                flow_id: flow.id,
                opaque_state_hash: state_hash("conformance-mismatch-WRONG"),
                outcome: authorized_outcome(provider, tag),
            },
        )
        .await
        .expect_err("a mismatched state hash must be rejected");
    assert_eq!(
        error,
        AuthProductError::CrossScopeDenied,
        "[{CASE}] mismatch rejects CrossScopeDenied"
    );

    let completed = flows
        .complete_oauth_callback(
            scope,
            callback_input(flow.id, tag, authorized_outcome(provider, tag)),
        )
        .await
        .unwrap_or_else(|error| {
            panic!("[{CASE}] the genuine callback must still complete after a mismatch: {error:?}")
        });
    assert_eq!(
        completed.status,
        AuthFlowStatus::Completed,
        "[{CASE}] a rejected mismatch must not consume the flow"
    );
}
