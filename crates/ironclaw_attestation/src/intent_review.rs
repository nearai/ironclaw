//! Authorization for the `/intent/{token}` review surface
//! (attested-signing Phase C §C1–§C3).
//!
//! ## The one sentence that governs everything
//!
//! **The link token is an addressing convenience, never an authorization.**
//! Authorization to *view* is an authenticated session whose user equals the
//! intent's bound approver; authorization to *sign* is the device ceremony;
//! authorization to *advance the turn* is the sealed one-shot grant CAS. The
//! token adds only routeability from a chat message, expiry, and
//! unguessability.
//!
//! ## Why every rejection is the same rejection
//!
//! Each check below collapses to [`ReviewRejection::NotFound`], which the HTTP
//! layer renders as a uniform 404. Unknown token, expired intent, already
//! resolved, wrong user, wrong tenant — all indistinguishable. A distinguishable
//! 403 would confirm to an attacker holding a leaked link that the intent
//! exists, who it belongs to, and whether it is still live; transaction detail
//! is exactly the reconnaissance an attacker wants. This is the #3995
//! cross-user IDOR lesson applied before the surface ships rather than after.
//!
//! ## GET is side-effect-free by construction
//!
//! These functions take `&IntentRecord` and return a decision; none of them can
//! mutate state. Chat platforms fetch link previews with bot user-agents, so a
//! state-changing or one-shot-consuming GET would be burned by the preview
//! fetch before the human ever clicked.

use ironclaw_signing_provider::{TenantId, UserId};

use crate::intent::IntentId;
use crate::intent_store::IntentRecord;

/// Why a review request was refused.
///
/// Deliberately single-variant: the type makes it hard to accidentally
/// introduce a distinguishable rejection later, since there is nothing else to
/// return. If a future surface genuinely needs to distinguish (it should not),
/// that becomes a visible API change rather than a quiet leak.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ReviewRejection {
    /// Uniform refusal: no such reviewable intent for this caller.
    #[error("no such intent")]
    NotFound,
}

/// The authenticated caller a review request arrives with.
///
/// Both axes are required — a session proves who, and the tenant scopes where.
/// Taking them together makes "authorized the user but forgot the tenant"
/// unrepresentable at this boundary.
#[derive(Debug, Clone, Copy)]
pub struct ReviewCaller<'a> {
    /// The authenticated session user.
    pub user: &'a UserId,
    /// The tenant the session is scoped to.
    pub tenant: &'a TenantId,
}

/// What an unauthenticated token presentation should do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenLanding {
    /// The token addresses a live intent: send the browser to the SPA route for
    /// `intent_id`. The redirect target is composed server-side; nothing from
    /// the request contributes to it.
    Redirect {
        /// The intent the SPA should load (after the caller authenticates).
        intent_id: IntentId,
    },
}

/// Resolve a presented token to its landing, WITHOUT authenticating.
///
/// This is the pre-session step: it proves only that the token addresses an
/// intent that is still worth sending someone to. It deliberately reveals
/// nothing about the transaction, the approver, or the tenant — the redirect
/// carries an intent id, and every data read behind it re-authorizes through
/// [`authorize_view`].
///
/// `now_ms` is supplied by the caller; this crate never reads the wall clock.
pub fn resolve_token_landing(
    record: &IntentRecord,
    now_ms: i64,
) -> Result<TokenLanding, ReviewRejection> {
    // Expired or already-decided intents are not worth a redirect, and saying
    // so distinguishably would leak their existence.
    if is_expired(record, now_ms) || record.state.is_terminal() {
        return Err(ReviewRejection::NotFound);
    }
    Ok(TokenLanding::Redirect {
        intent_id: record.intent_id().clone(),
    })
}

/// Authorize an authenticated caller to VIEW an intent's detail.
///
/// The check that matters: the session user must equal the intent's bound
/// approver, and the session tenant must equal the intent's tenant. A token
/// holder who is not the approver gets exactly what a stranger gets.
///
/// Ratified as Q4: viewing transaction details requires the bound approver —
/// the token alone shows nothing.
pub fn authorize_view<'a>(
    record: &'a IntentRecord,
    caller: ReviewCaller<'_>,
    now_ms: i64,
) -> Result<&'a IntentRecord, ReviewRejection> {
    // Tenant first: a cross-tenant caller must not even reach the user
    // comparison, so timing cannot separate "wrong tenant" from "wrong user".
    if record.tenant().as_str() != caller.tenant.as_str() {
        return Err(ReviewRejection::NotFound);
    }
    if record.intent.intent().approver.as_str() != caller.user.as_str() {
        return Err(ReviewRejection::NotFound);
    }
    if is_expired(record, now_ms) {
        return Err(ReviewRejection::NotFound);
    }
    Ok(record)
}

/// Authorize an authenticated caller to SUBMIT a signing proof.
///
/// Everything `authorize_view` requires, plus: the intent must still be
/// pending. A second submission after the grant was claimed is refused here,
/// and the sealed-grant CAS refuses it again underneath — this check is
/// convenience and clear errors, never the authority.
pub fn authorize_proof_submission<'a>(
    record: &'a IntentRecord,
    caller: ReviewCaller<'_>,
    now_ms: i64,
) -> Result<&'a IntentRecord, ReviewRejection> {
    let record = authorize_view(record, caller, now_ms)?;
    if record.state.is_terminal() {
        return Err(ReviewRejection::NotFound);
    }
    Ok(record)
}

/// Expiry is inclusive at the boundary, matching intent verification.
fn is_expired(record: &IntentRecord, now_ms: i64) -> bool {
    now_ms >= record.intent.intent().expires_at_ms
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decoded_tx::{
        DecodedTransaction, EvmAddress, EvmTransaction, RenderingSchemaVersion,
    };
    use crate::intent::{AgentKeyId, INTENT_SIGNATURE_LEN, UnsignedIntent};
    use crate::intent_store::{IntentState, ReviewTokenHash};
    use ironclaw_signing_provider::{ApprovedTxHash, ChainId};

    const CREATED: i64 = 1_000;
    const EXPIRES: i64 = 1_801_000; // CREATED + 30 min

    fn record(tenant: &str, approver: &str, state: IntentState) -> IntentRecord {
        let intent = UnsignedIntent {
            intent_id: IntentId::from_string("01J00000000000000000000REV"),
            tenant: TenantId::new(tenant),
            agent_key_id: AgentKeyId::new(TenantId::new(tenant), "agent-1", 1),
            approver: UserId::new(approver),
            chain_id: ChainId::new("eip155:11155111"),
            approved_tx_hash: ApprovedTxHash::from_bytes([0x11; 32]),
            decoded_tx: DecodedTransaction::Evm(EvmTransaction {
                chain_id: 11155111,
                nonce: 1,
                tx_type: 2,
                to: Some(EvmAddress([0x22; 20])),
                value: vec![],
                data: vec![],
                gas_limit: 21_000,
                gas_price: None,
                max_fee_per_gas: Some(vec![0x09]),
                max_priority_fee_per_gas: Some(vec![0x3b]),
                access_list: vec![],
                max_fee_per_blob_gas: None,
                blob_versioned_hashes: vec![],
            }),
            created_at_ms: CREATED,
            expires_at_ms: EXPIRES,
            schema_version: RenderingSchemaVersion::CURRENT,
        };
        let mut record = IntentRecord::pending(
            intent.into_signed([0u8; INTENT_SIGNATURE_LEN]),
            ironclaw_signing_provider::GateRef::new("gate:attested-review"),
            ReviewTokenHash::of_token("tok"),
        );
        record.state = state;
        record
    }

    fn approver_caller() -> (UserId, TenantId) {
        (UserId::new("alice"), TenantId::new("tenant-a"))
    }

    #[test]
    fn the_bound_approver_may_view_a_live_intent() {
        let record = record("tenant-a", "alice", IntentState::Pending);
        let (user, tenant) = approver_caller();
        assert!(
            authorize_view(
                &record,
                ReviewCaller {
                    user: &user,
                    tenant: &tenant
                },
                CREATED
            )
            .is_ok()
        );
    }

    /// The IDOR case (#3995 class): holding the link is not being the approver.
    /// A different user in the SAME tenant gets the stranger's answer.
    #[test]
    fn a_non_approver_in_the_same_tenant_is_refused_indistinguishably() {
        let record = record("tenant-a", "alice", IntentState::Pending);
        let mallory = UserId::new("mallory");
        let tenant = TenantId::new("tenant-a");
        assert_eq!(
            authorize_view(
                &record,
                ReviewCaller {
                    user: &mallory,
                    tenant: &tenant
                },
                CREATED
            )
            .err(),
            Some(ReviewRejection::NotFound)
        );
    }

    #[test]
    fn a_cross_tenant_caller_is_refused() {
        let record = record("tenant-a", "alice", IntentState::Pending);
        let user = UserId::new("alice");
        let other = TenantId::new("tenant-b");
        assert_eq!(
            authorize_view(
                &record,
                ReviewCaller {
                    user: &user,
                    tenant: &other
                },
                CREATED
            )
            .err(),
            Some(ReviewRejection::NotFound),
            "the same user name under another tenant is a different principal"
        );
    }

    /// Every refusal is the same refusal — the property that denies an attacker
    /// an existence oracle. If a future change introduced a distinguishable
    /// rejection, this test is where it should fail.
    #[test]
    fn every_refusal_is_indistinguishable() {
        let live = record("tenant-a", "alice", IntentState::Pending);
        let resolved = record("tenant-a", "alice", IntentState::Approved);
        let (alice, tenant_a) = approver_caller();
        let mallory = UserId::new("mallory");
        let tenant_b = TenantId::new("tenant-b");

        let refusals = [
            // wrong user
            authorize_view(
                &live,
                ReviewCaller {
                    user: &mallory,
                    tenant: &tenant_a,
                },
                CREATED,
            )
            .err(),
            // wrong tenant
            authorize_view(
                &live,
                ReviewCaller {
                    user: &alice,
                    tenant: &tenant_b,
                },
                CREATED,
            )
            .err(),
            // expired
            authorize_view(
                &live,
                ReviewCaller {
                    user: &alice,
                    tenant: &tenant_a,
                },
                EXPIRES,
            )
            .err(),
            // already resolved (proof submission)
            authorize_proof_submission(
                &resolved,
                ReviewCaller {
                    user: &alice,
                    tenant: &tenant_a,
                },
                CREATED,
            )
            .err(),
        ];
        for refusal in refusals {
            assert_eq!(
                refusal,
                Some(ReviewRejection::NotFound),
                "every refusal must be the same refusal — no existence oracle"
            );
        }
    }

    #[test]
    fn expiry_is_inclusive_at_the_boundary() {
        let record = record("tenant-a", "alice", IntentState::Pending);
        let (user, tenant) = approver_caller();
        let caller = ReviewCaller {
            user: &user,
            tenant: &tenant,
        };
        assert!(authorize_view(&record, caller, EXPIRES - 1).is_ok());
        assert_eq!(
            authorize_view(&record, caller, EXPIRES).err(),
            Some(ReviewRejection::NotFound)
        );
    }

    /// The token landing reveals nothing beyond "go here" — and refuses for
    /// dead intents so a stale link cannot even confirm the intent existed.
    #[test]
    fn a_live_token_redirects_and_a_dead_one_does_not() {
        let live = record("tenant-a", "alice", IntentState::Pending);
        assert_eq!(
            resolve_token_landing(&live, CREATED),
            Ok(TokenLanding::Redirect {
                intent_id: IntentId::from_string("01J00000000000000000000REV"),
            })
        );

        for dead in [
            record("tenant-a", "alice", IntentState::Approved),
            record("tenant-a", "alice", IntentState::Rejected),
            record("tenant-a", "alice", IntentState::Expired),
        ] {
            assert_eq!(
                resolve_token_landing(&dead, CREATED).err(),
                Some(ReviewRejection::NotFound)
            );
        }
        assert_eq!(
            resolve_token_landing(&live, EXPIRES).err(),
            Some(ReviewRejection::NotFound),
            "an expired intent must not redirect either"
        );
    }

    /// A preview bot (no session) reaching the token step must not be able to
    /// learn anything about the approver — the landing carries only an id.
    #[test]
    fn the_landing_carries_no_approver_or_tenant_detail() {
        let record = record("tenant-a", "alice", IntentState::Pending);
        let landing = resolve_token_landing(&record, CREATED).expect("live");
        let rendered = format!("{landing:?}");
        assert!(
            !rendered.contains("alice"),
            "the landing must not name the approver"
        );
        assert!(
            !rendered.contains("tenant-a"),
            "the landing must not name the tenant"
        );
    }

    /// Proof submission requires everything viewing requires, and then some:
    /// a resolved intent refuses even for the rightful approver (the grant CAS
    /// refuses underneath too — this is the clear error, not the authority).
    #[test]
    fn proof_submission_requires_a_pending_intent() {
        let (user, tenant) = approver_caller();
        let caller = ReviewCaller {
            user: &user,
            tenant: &tenant,
        };
        assert!(
            authorize_proof_submission(
                &record("tenant-a", "alice", IntentState::Pending),
                caller,
                CREATED
            )
            .is_ok()
        );
        assert_eq!(
            authorize_proof_submission(
                &record("tenant-a", "alice", IntentState::Approved),
                caller,
                CREATED
            )
            .err(),
            Some(ReviewRejection::NotFound)
        );
    }
}
