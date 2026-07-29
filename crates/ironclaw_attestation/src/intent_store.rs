//! Durable record for a signed intent, plus its store contract
//! (attested-signing Phase B §B3, consumed by Phase C's review link).
//!
//! ## The record is a PROJECTION, never an authorization
//!
//! [`IntentState`] mirrors the gate outcome; it does not decide it. The sealed
//! one-shot grant CAS remains the only thing that authorizes advancing a turn,
//! and this record is written by the single facade that already claims that
//! grant. Nothing may read `IntentState::Approved` and conclude it is allowed
//! to sign or resume — that inversion is what the whole design avoids.
//!
//! ## The review token is stored hashed, never raw
//!
//! Phase C mints a 256-bit random token and puts it in a chat message; only its
//! SHA-256 hash reaches this record. A dump of this table therefore yields no
//! usable links. Lookup is by hash ([`IntentStore::find_by_token_hash`]), and a
//! miss is indistinguishable from an expired or terminal intent at the HTTP
//! layer (uniform 404) so tokens cannot be enumerated.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use ironclaw_signing_provider::{GateRef, TenantId};

use crate::intent::{IntentId, SignedIntent};

/// Length of a review-token hash (SHA-256).
pub const REVIEW_TOKEN_HASH_LEN: usize = 32;

/// SHA-256 of a review token. Only this ever reaches storage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ReviewTokenHash([u8; REVIEW_TOKEN_HASH_LEN]);

impl ReviewTokenHash {
    /// Hash a raw review token. The raw token is never retained by this type.
    pub fn of_token(raw_token: &str) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(raw_token.as_bytes());
        Self(hasher.finalize().into())
    }

    /// Wrap an already-computed hash (durable read-back).
    pub fn from_bytes(bytes: [u8; REVIEW_TOKEN_HASH_LEN]) -> Self {
        Self(bytes)
    }

    /// The raw hash bytes.
    pub fn as_bytes(&self) -> &[u8; REVIEW_TOKEN_HASH_LEN] {
        &self.0
    }
}

/// Deliberately opaque: a token hash is a lookup key an attacker would like to
/// harvest from logs, so it never renders.
impl std::fmt::Display for ReviewTokenHash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ReviewTokenHash(redacted)")
    }
}

/// Where an intent stands, as a projection of the gate outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntentState {
    /// Raised, awaiting the approver.
    Pending,
    /// The approver signed and the grant was claimed.
    Approved,
    /// The approver declined.
    Rejected,
    /// The window closed with no decision.
    Expired,
}

impl IntentState {
    /// Whether this state accepts no further transitions.
    pub fn is_terminal(&self) -> bool {
        !matches!(self, IntentState::Pending)
    }
}

/// The durable intent record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntentRecord {
    /// The sealed, verifiable intent.
    pub intent: SignedIntent,
    /// The gate this intent was raised alongside.
    ///
    /// This is the join the projection needs: the resolve path knows the gate
    /// it just claimed the grant for, and this is how it finds the intent to
    /// mark resolved. Set at raise time and immutable thereafter.
    pub gate_ref: GateRef,
    /// Hash of the review token that addresses it.
    pub review_token_hash: ReviewTokenHash,
    /// Lifecycle projection.
    pub state: IntentState,
}

impl IntentRecord {
    /// A freshly raised, pending record.
    pub fn pending(
        intent: SignedIntent,
        gate_ref: GateRef,
        review_token_hash: ReviewTokenHash,
    ) -> Self {
        Self {
            intent,
            gate_ref,
            review_token_hash,
            state: IntentState::Pending,
        }
    }

    /// The intent's tenant.
    pub fn tenant(&self) -> &TenantId {
        &self.intent.intent().tenant
    }

    /// The intent's id.
    pub fn intent_id(&self) -> &IntentId {
        &self.intent.intent().intent_id
    }
}

/// Why an intent-store operation failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum IntentStoreError {
    /// No intent matched.
    #[error("intent not found")]
    NotFound,

    /// An intent already exists under this id; writes are insert-only.
    #[error("intent already exists")]
    AlreadyExists,

    /// The requested transition is not legal from the current state.
    #[error("intent is already resolved")]
    AlreadyResolved,

    /// The backend failed.
    #[error("intent store error: {reason}")]
    Backend {
        /// Sanitized backend description.
        reason: String,
    },
}

/// Durable store for signed intents.
///
/// Every read is tenant-qualified. That is not merely defensive: Phase C looks
/// an intent up from an unauthenticated URL, so a lookup that could cross the
/// tenant boundary would hand an attacker another tenant's transaction detail.
#[async_trait]
pub trait IntentStore: Send + Sync {
    /// Persist a freshly raised intent. Insert-only: an intent is immutable
    /// once written except for its [`IntentState`] projection.
    async fn put(&self, record: IntentRecord) -> Result<(), IntentStoreError>;

    /// Load by `(tenant, intent_id)`.
    async fn get(
        &self,
        tenant: &TenantId,
        intent_id: &IntentId,
    ) -> Result<IntentRecord, IntentStoreError>;

    /// Resolve a review token hash to its intent.
    ///
    /// Deliberately NOT tenant-qualified — the token is presented before any
    /// session exists, so there is no tenant to qualify by yet. The token hash
    /// is a 256-bit-random-derived lookup key, and the caller MUST still run
    /// the approver/tenant authorization checks on the returned record before
    /// exposing anything from it.
    async fn find_by_token_hash(
        &self,
        token_hash: &ReviewTokenHash,
    ) -> Result<IntentRecord, IntentStoreError>;

    /// Find the intent raised alongside `gate_ref`.
    ///
    /// The projection's lookup: the resolve path holds a gate ref, not an
    /// intent id. Tenant-qualified like every other read.
    async fn find_by_gate_ref(
        &self,
        tenant: &TenantId,
        gate_ref: &GateRef,
    ) -> Result<IntentRecord, IntentStoreError>;

    /// Project the gate outcome onto the record.
    ///
    /// Legal only from [`IntentState::Pending`]; a second transition fails with
    /// [`IntentStoreError::AlreadyResolved`], mirroring the one-shot grant CAS
    /// this projects rather than establishing an independent one.
    async fn resolve(
        &self,
        tenant: &TenantId,
        intent_id: &IntentId,
        outcome: IntentState,
    ) -> Result<(), IntentStoreError>;
}

/// In-memory [`IntentStore`] for local-dev and tests.
#[derive(Default)]
pub struct InMemoryIntentStore {
    records: std::sync::Mutex<Vec<IntentRecord>>,
}

impl InMemoryIntentStore {
    /// Construct an empty store.
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Vec<IntentRecord>>, IntentStoreError> {
        self.records.lock().map_err(|_| IntentStoreError::Backend {
            reason: "intent store lock poisoned".to_string(),
        })
    }
}

#[async_trait]
impl IntentStore for InMemoryIntentStore {
    async fn put(&self, record: IntentRecord) -> Result<(), IntentStoreError> {
        let mut records = self.lock()?;
        let exists = records.iter().any(|existing| {
            existing.tenant().as_str() == record.tenant().as_str()
                && existing.intent_id() == record.intent_id()
        });
        if exists {
            return Err(IntentStoreError::AlreadyExists);
        }
        records.push(record);
        Ok(())
    }

    async fn get(
        &self,
        tenant: &TenantId,
        intent_id: &IntentId,
    ) -> Result<IntentRecord, IntentStoreError> {
        self.lock()?
            .iter()
            .find(|record| {
                record.tenant().as_str() == tenant.as_str() && record.intent_id() == intent_id
            })
            .cloned()
            .ok_or(IntentStoreError::NotFound)
    }

    async fn find_by_token_hash(
        &self,
        token_hash: &ReviewTokenHash,
    ) -> Result<IntentRecord, IntentStoreError> {
        self.lock()?
            .iter()
            .find(|record| &record.review_token_hash == token_hash)
            .cloned()
            .ok_or(IntentStoreError::NotFound)
    }

    async fn find_by_gate_ref(
        &self,
        tenant: &TenantId,
        gate_ref: &GateRef,
    ) -> Result<IntentRecord, IntentStoreError> {
        self.lock()?
            .iter()
            .find(|record| {
                record.tenant().as_str() == tenant.as_str()
                    && record.gate_ref.as_str() == gate_ref.as_str()
            })
            .cloned()
            .ok_or(IntentStoreError::NotFound)
    }

    async fn resolve(
        &self,
        tenant: &TenantId,
        intent_id: &IntentId,
        outcome: IntentState,
    ) -> Result<(), IntentStoreError> {
        let mut records = self.lock()?;
        let record = records
            .iter_mut()
            .find(|record| {
                record.tenant().as_str() == tenant.as_str() && record.intent_id() == intent_id
            })
            .ok_or(IntentStoreError::NotFound)?;
        if record.state.is_terminal() {
            return Err(IntentStoreError::AlreadyResolved);
        }
        record.state = outcome;
        Ok(())
    }
}

/// The canonical [`IntentStore`] behavioural contract.
///
/// Every implementation — the in-memory reference below and each durable
/// backend in `ironclaw_attested_store` — is driven through these exact cases
/// via [`intent_store_contract_cases!`]. That is the point: a durable backend
/// whose tenant qualification or one-shot projection drifts from the reference
/// is a disclosure or a rewritten outcome, and a suite that lived only beside
/// the in-memory impl would never have caught it.
#[cfg(any(test, feature = "contract-tests"))]
pub mod contract {
    // See the note in `grant::contract`: these are `pub` for out-of-crate
    // durable-backend crates, reachable only when `contract-tests` makes the
    // parent module public.
    #![cfg_attr(not(feature = "contract-tests"), allow(unreachable_pub))]
    use super::*;
    use crate::decoded_tx::{
        DecodedTransaction, EvmAddress, EvmTransaction, RenderingSchemaVersion,
    };
    use crate::intent::{AgentKeyId, INTENT_SIGNATURE_LEN, UnsignedIntent};
    use ironclaw_signing_provider::{ApprovedTxHash, ChainId, UserId};

    /// The tenant every single-tenant case uses.
    pub fn tenant_a() -> TenantId {
        TenantId::new("tenant-a")
    }

    /// A pending record for `(tenant, id)` whose review token is `token` and
    /// whose gate ref is derived from `id`.
    pub fn record_for(tenant: &str, id: &str, token: &str) -> IntentRecord {
        let intent = UnsignedIntent {
            intent_id: IntentId::from_string(id),
            tenant: TenantId::new(tenant),
            agent_key_id: AgentKeyId::new(TenantId::new(tenant), "agent-1", 1),
            approver: UserId::new("alice"),
            chain_id: ChainId::new("eip155:11155111"),
            approved_tx_hash: ApprovedTxHash::from_bytes([0xab; 32]),
            decoded_tx: DecodedTransaction::Evm(EvmTransaction {
                chain_id: 11155111,
                nonce: 1,
                tx_type: 2,
                to: Some(EvmAddress([0x11; 20])),
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
            created_at_ms: 1_000,
            expires_at_ms: 1_800_000,
            schema_version: RenderingSchemaVersion::CURRENT,
        };
        IntentRecord::pending(
            intent.into_signed([0u8; INTENT_SIGNATURE_LEN]),
            GateRef::new(format!("gate:attested-{id}")),
            ReviewTokenHash::of_token(token),
        )
    }

    /// A record must survive the round trip byte-for-byte — the review page
    /// renders the decoded transaction straight off it.
    pub async fn put_then_get_round_trips(store: impl IntentStore) {
        let record = record_for("tenant-a", "intent-1", "tok-1");
        store.put(record.clone()).await.expect("put");
        assert_eq!(
            store
                .get(&tenant_a(), &IntentId::from_string("intent-1"))
                .await
                .expect("get"),
            record
        );
    }

    /// Writes are insert-only.
    pub async fn writes_are_insert_only(store: impl IntentStore) {
        store
            .put(record_for("tenant-a", "intent-1", "tok-1"))
            .await
            .expect("put");
        assert_eq!(
            store.put(record_for("tenant-a", "intent-1", "tok-2")).await,
            Err(IntentStoreError::AlreadyExists),
            "an intent must never be silently replaced — a second write could swap the tx"
        );
    }

    /// Tenant isolation on the id lookup: Phase C serves transaction detail off
    /// this record, so a cross-tenant read would be a disclosure.
    pub async fn an_intent_does_not_resolve_under_another_tenant(store: impl IntentStore) {
        store
            .put(record_for("tenant-a", "intent-1", "tok-1"))
            .await
            .expect("put");
        assert_eq!(
            store
                .get(
                    &TenantId::new("tenant-b"),
                    &IntentId::from_string("intent-1")
                )
                .await,
            Err(IntentStoreError::NotFound)
        );
    }

    /// The same id under two tenants is two records, not a collision.
    pub async fn the_same_intent_id_is_distinct_per_tenant(store: impl IntentStore) {
        store
            .put(record_for("tenant-a", "shared-id", "tok-a"))
            .await
            .expect("put a");
        store
            .put(record_for("tenant-b", "shared-id", "tok-b"))
            .await
            .expect("put b — a different tenant is a different record");
        assert_eq!(
            store
                .get(&tenant_a(), &IntentId::from_string("shared-id"))
                .await
                .expect("get a")
                .tenant()
                .as_str(),
            "tenant-a"
        );
    }

    /// The token hash reaches exactly its own intent.
    pub async fn a_token_hash_resolves_its_intent_and_nothing_else(store: impl IntentStore) {
        store
            .put(record_for("tenant-a", "intent-1", "tok-1"))
            .await
            .expect("put");
        assert_eq!(
            store
                .find_by_token_hash(&ReviewTokenHash::of_token("tok-1"))
                .await
                .expect("found")
                .intent_id()
                .as_str(),
            "intent-1"
        );
        assert_eq!(
            store
                .find_by_token_hash(&ReviewTokenHash::of_token("wrong-token"))
                .await,
            Err(IntentStoreError::NotFound)
        );
    }

    /// The raw token must not be recoverable from a stored record, including
    /// after a durable round trip — a backend that persisted the token itself
    /// would turn a database read into a working approval credential.
    pub async fn the_raw_token_is_never_stored_or_rendered(store: impl IntentStore) {
        let record = record_for("tenant-a", "intent-1", "super-secret-token");
        store.put(record).await.expect("put");
        let stored = store
            .get(&tenant_a(), &IntentId::from_string("intent-1"))
            .await
            .expect("get");

        assert!(
            !format!("{stored:?}").contains("super-secret-token"),
            "the raw token must not survive into the persisted record"
        );
        assert_eq!(
            stored.review_token_hash.to_string(),
            "ReviewTokenHash(redacted)",
            "a token hash must not render into logs"
        );
        assert_eq!(
            stored.review_token_hash,
            ReviewTokenHash::of_token("super-secret-token"),
            "the stored value must still be the hash of the token"
        );
    }

    /// The projection is one-shot in the same sense the grant CAS is: a second
    /// resolve loses, so a late/duplicate outcome cannot rewrite history.
    pub async fn resolution_is_one_shot_and_terminal(store: impl IntentStore) {
        store
            .put(record_for("tenant-a", "intent-1", "tok-1"))
            .await
            .expect("put");
        let id = IntentId::from_string("intent-1");

        store
            .resolve(&tenant_a(), &id, IntentState::Approved)
            .await
            .expect("first resolve wins");
        assert_eq!(
            store.get(&tenant_a(), &id).await.expect("get").state,
            IntentState::Approved
        );

        for late in [
            IntentState::Rejected,
            IntentState::Expired,
            IntentState::Approved,
        ] {
            assert_eq!(
                store.resolve(&tenant_a(), &id, late).await,
                Err(IntentStoreError::AlreadyResolved),
                "a terminal intent must not be rewritten to {late:?}"
            );
        }
    }

    /// A resolve from another tenant must not touch the record.
    pub async fn a_cross_tenant_resolve_is_not_found_and_changes_nothing(store: impl IntentStore) {
        store
            .put(record_for("tenant-a", "intent-1", "tok-1"))
            .await
            .expect("put");
        let id = IntentId::from_string("intent-1");
        assert_eq!(
            store
                .resolve(&TenantId::new("tenant-b"), &id, IntentState::Rejected)
                .await,
            Err(IntentStoreError::NotFound)
        );
        assert_eq!(
            store.get(&tenant_a(), &id).await.expect("get").state,
            IntentState::Pending,
            "the foreign resolve must not have moved the record"
        );
    }

    /// The projection's join: the resolve path holds a gate ref and must reach
    /// exactly the intent raised with it — and never another tenant's.
    pub async fn an_intent_is_reachable_by_its_gate_ref(store: impl IntentStore) {
        store
            .put(record_for("tenant-a", "intent-1", "tok-1"))
            .await
            .expect("put");
        let gate = GateRef::new("gate:attested-intent-1");
        assert_eq!(
            store
                .find_by_gate_ref(&tenant_a(), &gate)
                .await
                .expect("found")
                .intent_id()
                .as_str(),
            "intent-1"
        );
        assert_eq!(
            store
                .find_by_gate_ref(&TenantId::new("tenant-b"), &gate)
                .await,
            Err(IntentStoreError::NotFound),
            "the gate-ref lookup is tenant-qualified like every other read"
        );
        assert_eq!(
            store
                .find_by_gate_ref(&tenant_a(), &GateRef::new("gate:attested-other"))
                .await,
            Err(IntentStoreError::NotFound)
        );
    }

    /// Expand the whole [`IntentStore`] contract against one factory.
    ///
    /// `$factory` is called fresh per case, so each gets an empty store.
    #[macro_export]
    macro_rules! intent_store_contract_cases {
        ($label:ident, $factory:expr) => {
            mod $label {
                #[tokio::test]
                async fn put_then_get_round_trips() {
                    $crate::intent_store::contract::put_then_get_round_trips($factory()).await;
                }
                #[tokio::test]
                async fn writes_are_insert_only() {
                    $crate::intent_store::contract::writes_are_insert_only($factory()).await;
                }
                #[tokio::test]
                async fn an_intent_does_not_resolve_under_another_tenant() {
                    $crate::intent_store::contract::an_intent_does_not_resolve_under_another_tenant(
                        $factory(),
                    )
                    .await;
                }
                #[tokio::test]
                async fn the_same_intent_id_is_distinct_per_tenant() {
                    $crate::intent_store::contract::the_same_intent_id_is_distinct_per_tenant(
                        $factory(),
                    )
                    .await;
                }
                #[tokio::test]
                async fn a_token_hash_resolves_its_intent_and_nothing_else() {
                    $crate::intent_store::contract::a_token_hash_resolves_its_intent_and_nothing_else(
                        $factory(),
                    )
                    .await;
                }
                #[tokio::test]
                async fn the_raw_token_is_never_stored_or_rendered() {
                    $crate::intent_store::contract::the_raw_token_is_never_stored_or_rendered(
                        $factory(),
                    )
                    .await;
                }
                #[tokio::test]
                async fn resolution_is_one_shot_and_terminal() {
                    $crate::intent_store::contract::resolution_is_one_shot_and_terminal($factory())
                        .await;
                }
                #[tokio::test]
                async fn a_cross_tenant_resolve_is_not_found_and_changes_nothing() {
                    $crate::intent_store::contract::a_cross_tenant_resolve_is_not_found_and_changes_nothing(
                        $factory(),
                    )
                    .await;
                }
                #[tokio::test]
                async fn an_intent_is_reachable_by_its_gate_ref() {
                    $crate::intent_store::contract::an_intent_is_reachable_by_its_gate_ref(
                        $factory(),
                    )
                    .await;
                }
            }
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The in-memory reference impl is held to the same contract every durable
    // backend is.
    // Fully qualified: the macro expands into a nested module, which does not
    // inherit this one's imports.
    crate::intent_store_contract_cases!(in_memory, crate::intent_store::InMemoryIntentStore::new);

    #[test]
    fn only_pending_is_non_terminal() {
        assert!(!IntentState::Pending.is_terminal());
        for terminal in [
            IntentState::Approved,
            IntentState::Rejected,
            IntentState::Expired,
        ] {
            assert!(terminal.is_terminal(), "{terminal:?} must be terminal");
        }
    }
}
