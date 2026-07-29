//! The per-agent intent-signing key registry (attested-signing Phase B, §B4).
//!
//! One key per `(tenant, agent)`, versioned by a monotonic `generation`. This
//! store holds **public** keys and lifecycle state only — the sealed private
//! half lives behind the [`crate::IntentSigner`] port, in the layer that owns
//! `ironclaw_secrets` (a forbidden dependency here).
//!
//! ## The rotation state machine
//!
//! ```text
//!   Active  ──rotate──▶  Retiring  ──(overlap elapses / revoke)──▶  Revoked
//!      │                                                              ▲
//!      └──────────────────────── revoke ──────────────────────────────┘
//! ```
//!
//! * **Active** — the single generation new intents are signed with.
//! * **Retiring** — no longer signs, but still *verifies* for a bounded overlap
//!   so intents already in flight when the key rotated do not fail closed.
//! * **Revoked** — never signs, never verifies. Terminal.
//!
//! Revocation is deliberately immediate and total: a key is revoked because it
//! may be compromised, and an overlap for a compromised key is exactly the
//! window an attacker wants.
//!
//! ## The overlap window
//!
//! [`DEFAULT_ROTATION_OVERLAP_MS`] is **24 hours** (Q7, ratified 2026-07-25).
//! That is far longer than the 30-minute intent TTL strictly requires, and it
//! is chosen for operational headroom: a rotation can happen without anyone
//! timing it against in-flight work, and an operator does not have to reason
//! about clock skew, retries, or a queue backing up.
//!
//! What makes the long window acceptable is that it is **not** the lever for a
//! compromised key. `Revoked` is immediate and total (no overlap at all), so a
//! key believed compromised stops verifying the moment it is revoked. The
//! overlap only ever applies to a key retired in the normal course — one that
//! is being replaced, not contained.
//!
//! The floor is pinned by test: the window must always cover at least one full
//! intent lifetime, so an intent minted immediately before a rotation stays
//! verifiable for its whole life.
//!
//! Note this is defense in depth, not the security boundary: an intent
//! signature is attribution and tamper-evidence across the chat-channel hop,
//! never authorization (see [`crate::intent`]).

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use ironclaw_signing_provider::TenantId;

use crate::intent::{AGENT_PUBLIC_KEY_LEN, AgentKeyId};

/// How long a freshly minted intent stays valid (Q4: 30 minutes).
pub const DEFAULT_INTENT_TTL_MS: i64 = 30 * 60 * 1_000;

/// How long a `Retiring` key keeps verifying after it stopped signing.
///
/// 24 hours (Q7, ratified 2026-07-25) — operational headroom, so a rotation never
/// has to be timed against in-flight work. See the module header for why a
/// window this much longer than [`DEFAULT_INTENT_TTL_MS`] is safe: revocation
/// is the immediate, no-overlap lever for a key believed compromised, and this
/// window only governs keys retired in the normal course.
///
/// Callers pass the window in explicitly, so a deployment that wants a tighter
/// one changes its config rather than this default.
pub const DEFAULT_ROTATION_OVERLAP_MS: i64 = 24 * 60 * 60 * 1_000;

/// The floor on the overlap window, enforced at COMPILE time: whatever window
/// is configured, a retired key must keep verifying for at least one full
/// intent lifetime — otherwise an intent minted immediately before a rotation
/// could fail closed mid-flight. A future tightening of
/// [`DEFAULT_ROTATION_OVERLAP_MS`] that crossed this floor would fail the
/// build rather than a test.
// Evaluated during const-eval: a violation fails the BUILD, so it can never
// reach a running system.
const _: () = assert!(DEFAULT_ROTATION_OVERLAP_MS >= DEFAULT_INTENT_TTL_MS); // safety: compile-time invariant, not a runtime panic

/// Lifecycle state of one agent signing key generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentKeyState {
    /// Signs new intents and verifies.
    Active,
    /// Verifies within the overlap window; never signs.
    Retiring,
    /// Neither signs nor verifies. Terminal.
    Revoked,
}

/// A registered agent signing key: the public half plus its lifecycle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentSigningKey {
    /// `(tenant, agent, generation)`.
    pub key_id: AgentKeyId,
    /// ed25519 public key.
    pub public_key: [u8; AGENT_PUBLIC_KEY_LEN],
    /// Current lifecycle state.
    pub state: AgentKeyState,
    /// When the key was registered (unix millis).
    pub created_at_ms: i64,
    /// When the key entered [`AgentKeyState::Retiring`], if it has. The overlap
    /// window is measured from this instant, NOT from `created_at_ms` — a
    /// long-lived key must still get its full overlap when it finally rotates.
    pub retiring_since_ms: Option<i64>,
}

/// Why an agent-key lookup failed. Sanitized: never carries key material.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum AgentKeyError {
    /// No key is registered for the requested identity/generation.
    #[error("no such agent signing key")]
    NotFound,

    /// The key is revoked — it neither signs nor verifies.
    #[error("agent signing key is revoked")]
    Revoked,

    /// The key is retiring and its overlap window has elapsed.
    #[error("agent signing key is retired")]
    Retired,

    /// A key already exists for this `(tenant, agent, generation)`; registration
    /// is insert-only so a key can never be silently replaced.
    #[error("agent signing key already exists")]
    AlreadyExists,

    /// The backend failed.
    #[error("agent key store error: {reason}")]
    Backend {
        /// Sanitized backend description.
        reason: String,
    },
}

/// Registry of agent signing keys.
///
/// Reads are tenant-scoped by construction: every lookup goes through an
/// [`AgentKeyId`], whose first component is the tenant, so a key registered for
/// one tenant can never be resolved for another.
#[async_trait]
pub trait AgentSigningKeyStore: Send + Sync {
    /// Register a new key generation. Insert-only: re-registering an existing
    /// `(tenant, agent, generation)` fails with
    /// [`AgentKeyError::AlreadyExists`] rather than replacing the public key of
    /// a generation that intents may already reference.
    async fn register(&self, key: AgentSigningKey) -> Result<(), AgentKeyError>;

    /// The generation new intents should be signed with, if one is active.
    async fn active_key(
        &self,
        tenant: &TenantId,
        agent: &str,
    ) -> Result<AgentSigningKey, AgentKeyError>;

    /// The public key to VERIFY an intent that names `key_id`, enforcing the
    /// lifecycle rules against the caller-supplied clock.
    ///
    /// `now_ms` and `overlap_ms` are parameters, not ambient state: this crate
    /// never reads the wall clock, which keeps verification deterministic and
    /// the overlap window testable at its exact boundary.
    async fn verifying_key(
        &self,
        key_id: &AgentKeyId,
        now_ms: i64,
        overlap_ms: i64,
    ) -> Result<[u8; AGENT_PUBLIC_KEY_LEN], AgentKeyError>;

    /// Move a key to [`AgentKeyState::Retiring`] as of `now_ms`.
    async fn retire(&self, key_id: &AgentKeyId, now_ms: i64) -> Result<(), AgentKeyError>;

    /// Move a key to [`AgentKeyState::Revoked`]. Immediate and terminal.
    async fn revoke(&self, key_id: &AgentKeyId) -> Result<(), AgentKeyError>;
}

/// Decide whether `key` may verify at `now_ms`.
///
/// Shared by every backend so the lifecycle rule cannot drift between the
/// in-memory and durable implementations — the durable stores call this after
/// loading the row rather than re-expressing the comparison in SQL.
pub fn verification_admits(
    key: &AgentSigningKey,
    now_ms: i64,
    overlap_ms: i64,
) -> Result<(), AgentKeyError> {
    match key.state {
        AgentKeyState::Active => Ok(()),
        AgentKeyState::Revoked => Err(AgentKeyError::Revoked),
        AgentKeyState::Retiring => {
            // A retiring key with no recorded retirement instant is treated as
            // already retired: fail closed rather than granting an unbounded
            // overlap off a missing timestamp.
            let Some(since) = key.retiring_since_ms else {
                return Err(AgentKeyError::Retired);
            };
            // Inclusive at the boundary, matching intent expiry.
            if now_ms >= since.saturating_add(overlap_ms) {
                Err(AgentKeyError::Retired)
            } else {
                Ok(())
            }
        }
    }
}

/// In-memory [`AgentSigningKeyStore`] for local-dev and tests.
#[derive(Default)]
pub struct InMemoryAgentSigningKeyStore {
    keys: std::sync::Mutex<std::collections::HashMap<AgentKeyId, AgentSigningKey>>,
}

impl InMemoryAgentSigningKeyStore {
    /// Construct an empty store.
    pub fn new() -> Self {
        Self::default()
    }

    fn with_key<T>(
        &self,
        key_id: &AgentKeyId,
        f: impl FnOnce(&mut AgentSigningKey) -> Result<T, AgentKeyError>,
    ) -> Result<T, AgentKeyError> {
        let mut keys = self.keys.lock().map_err(|_| AgentKeyError::Backend {
            reason: "agent key store lock poisoned".to_string(),
        })?;
        let key = keys.get_mut(key_id).ok_or(AgentKeyError::NotFound)?;
        f(key)
    }
}

#[async_trait]
impl AgentSigningKeyStore for InMemoryAgentSigningKeyStore {
    async fn register(&self, key: AgentSigningKey) -> Result<(), AgentKeyError> {
        let mut keys = self.keys.lock().map_err(|_| AgentKeyError::Backend {
            reason: "agent key store lock poisoned".to_string(),
        })?;
        if keys.contains_key(&key.key_id) {
            return Err(AgentKeyError::AlreadyExists);
        }
        keys.insert(key.key_id.clone(), key);
        Ok(())
    }

    async fn active_key(
        &self,
        tenant: &TenantId,
        agent: &str,
    ) -> Result<AgentSigningKey, AgentKeyError> {
        let keys = self.keys.lock().map_err(|_| AgentKeyError::Backend {
            reason: "agent key store lock poisoned".to_string(),
        })?;
        keys.values()
            .filter(|key| {
                key.key_id.tenant.as_str() == tenant.as_str()
                    && key.key_id.agent == agent
                    && key.state == AgentKeyState::Active
            })
            // Highest generation wins if more than one is somehow active.
            .max_by_key(|key| key.key_id.generation)
            .cloned()
            .ok_or(AgentKeyError::NotFound)
    }

    async fn verifying_key(
        &self,
        key_id: &AgentKeyId,
        now_ms: i64,
        overlap_ms: i64,
    ) -> Result<[u8; AGENT_PUBLIC_KEY_LEN], AgentKeyError> {
        self.with_key(key_id, |key| {
            verification_admits(key, now_ms, overlap_ms)?;
            Ok(key.public_key)
        })
    }

    async fn retire(&self, key_id: &AgentKeyId, now_ms: i64) -> Result<(), AgentKeyError> {
        self.with_key(key_id, |key| {
            if key.state == AgentKeyState::Revoked {
                // Revoked is terminal — it must never walk back to retiring.
                return Err(AgentKeyError::Revoked);
            }
            key.state = AgentKeyState::Retiring;
            key.retiring_since_ms = Some(now_ms);
            Ok(())
        })
    }

    async fn revoke(&self, key_id: &AgentKeyId) -> Result<(), AgentKeyError> {
        self.with_key(key_id, |key| {
            key.state = AgentKeyState::Revoked;
            Ok(())
        })
    }
}

/// The canonical [`AgentSigningKeyStore`] behavioural contract.
///
/// Rotation and revocation are where a durable backend most plausibly drifts
/// from the reference: an overlap window computed off the wrong column, or a
/// revoked key that a `WHERE state <> 'revoked'` typo still hands out, is a
/// live signing key an operator believes is dead. Every implementation runs
/// these exact cases via [`agent_signing_key_store_contract_cases!`].
#[cfg(any(test, feature = "contract-tests"))]
pub mod contract {
    // See the note in `grant::contract`.
    #![cfg_attr(not(feature = "contract-tests"), allow(unreachable_pub))]
    use super::*;

    /// A fixed clock origin, so every window assertion is exact.
    pub const T0: i64 = 1_000_000;

    /// The tenant every single-tenant case uses.
    pub fn tenant() -> TenantId {
        TenantId::new("tenant-a")
    }

    /// `agent-1`'s key id at `generation`.
    pub fn key_id(generation: u32) -> AgentKeyId {
        AgentKeyId::new(tenant(), "agent-1", generation)
    }

    /// A key whose public bytes are `generation` repeated, so a case can tell
    /// generations apart by value alone.
    pub fn key(generation: u32, state: AgentKeyState) -> AgentSigningKey {
        AgentSigningKey {
            key_id: key_id(generation),
            public_key: [generation as u8; AGENT_PUBLIC_KEY_LEN],
            state,
            created_at_ms: T0,
            retiring_since_ms: None,
        }
    }

    async fn seed(store: &impl AgentSigningKeyStore, keys: Vec<AgentSigningKey>) {
        for k in keys {
            store.register(k).await.expect("register");
        }
    }

    /// The base case: an active key both signs and verifies.
    pub async fn an_active_key_signs_and_verifies(store: impl AgentSigningKeyStore) {
        seed(&store, vec![key(1, AgentKeyState::Active)]).await;
        assert_eq!(
            store
                .active_key(&tenant(), "agent-1")
                .await
                .expect("active")
                .key_id,
            key_id(1)
        );
        assert_eq!(
            store
                .verifying_key(&key_id(1), T0, DEFAULT_ROTATION_OVERLAP_MS)
                .await,
            Ok([1u8; AGENT_PUBLIC_KEY_LEN])
        );
    }

    /// Registration is insert-only.
    pub async fn registration_is_insert_only(store: impl AgentSigningKeyStore) {
        seed(&store, vec![key(1, AgentKeyState::Active)]).await;
        assert_eq!(
            store.register(key(1, AgentKeyState::Active)).await,
            Err(AgentKeyError::AlreadyExists),
            "re-registering a generation must not replace a public key intents may reference"
        );
    }

    /// The rotation contract: after rotating, the NEW generation signs while the
    /// old one still verifies — so an intent minted seconds before the rotation
    /// does not fail closed.
    pub async fn rotation_moves_signing_forward_while_the_old_key_still_verifies(
        store: impl AgentSigningKeyStore,
    ) {
        seed(&store, vec![key(1, AgentKeyState::Active)]).await;
        store.retire(&key_id(1), T0).await.expect("retire");
        store
            .register(key(2, AgentKeyState::Active))
            .await
            .expect("register gen 2");

        // New intents sign under generation 2...
        assert_eq!(
            store
                .active_key(&tenant(), "agent-1")
                .await
                .expect("active")
                .key_id,
            key_id(2)
        );
        // ...while an in-flight intent naming generation 1 still verifies.
        assert_eq!(
            store
                .verifying_key(&key_id(1), T0 + 60_000, DEFAULT_ROTATION_OVERLAP_MS)
                .await,
            Ok([1u8; AGENT_PUBLIC_KEY_LEN])
        );
    }

    /// The overlap window is measured from the retirement instant and is
    /// inclusive at its far edge, exactly like intent expiry.
    pub async fn the_overlap_window_closes_at_its_boundary(store: impl AgentSigningKeyStore) {
        seed(&store, vec![key(1, AgentKeyState::Active)]).await;
        store.retire(&key_id(1), T0).await.expect("retire");
        let overlap = DEFAULT_ROTATION_OVERLAP_MS;

        // One millisecond before the window closes: still accepted.
        assert!(
            store
                .verifying_key(&key_id(1), T0 + overlap - 1, overlap)
                .await
                .is_ok()
        );
        // At the boundary and after: retired.
        assert_eq!(
            store.verifying_key(&key_id(1), T0 + overlap, overlap).await,
            Err(AgentKeyError::Retired)
        );
        assert_eq!(
            store
                .verifying_key(&key_id(1), T0 + overlap + 86_400_000, overlap)
                .await,
            Err(AgentKeyError::Retired)
        );
    }

    /// Revocation is immediate and total — a revoked key gets NO overlap, since
    /// a key is revoked precisely because it may be compromised.
    pub async fn revocation_is_immediate_with_no_overlap(store: impl AgentSigningKeyStore) {
        seed(&store, vec![key(1, AgentKeyState::Active)]).await;
        store.revoke(&key_id(1)).await.expect("revoke");
        assert_eq!(
            store
                .verifying_key(&key_id(1), T0, DEFAULT_ROTATION_OVERLAP_MS)
                .await,
            Err(AgentKeyError::Revoked)
        );
        // Even at the very instant of revocation.
        assert_eq!(
            store.verifying_key(&key_id(1), T0, 0).await,
            Err(AgentKeyError::Revoked)
        );
        // And it can never be walked back to retiring.
        assert_eq!(
            store.retire(&key_id(1), T0).await,
            Err(AgentKeyError::Revoked)
        );
        // A revoked key is not offered for signing either.
        assert_eq!(
            store.active_key(&tenant(), "agent-1").await,
            Err(AgentKeyError::NotFound)
        );
    }

    /// A retiring key must never be handed out for SIGNING, only verification.
    pub async fn a_retiring_key_is_never_offered_for_signing(store: impl AgentSigningKeyStore) {
        seed(&store, vec![key(1, AgentKeyState::Active)]).await;
        store.retire(&key_id(1), T0).await.expect("retire");
        assert_eq!(
            store.active_key(&tenant(), "agent-1").await,
            Err(AgentKeyError::NotFound),
            "a retiring key verifies but must not sign new intents"
        );
    }

    /// Tenant isolation: the same agent name under another tenant is a
    /// different key and must not resolve.
    pub async fn keys_do_not_resolve_across_tenants(store: impl AgentSigningKeyStore) {
        seed(&store, vec![key(1, AgentKeyState::Active)]).await;
        assert_eq!(
            store
                .active_key(&TenantId::new("tenant-b"), "agent-1")
                .await,
            Err(AgentKeyError::NotFound)
        );
        let foreign = AgentKeyId::new(TenantId::new("tenant-b"), "agent-1", 1);
        assert_eq!(
            store
                .verifying_key(&foreign, T0, DEFAULT_ROTATION_OVERLAP_MS)
                .await,
            Err(AgentKeyError::NotFound)
        );
    }

    /// An unknown key is not found rather than admitted.
    pub async fn an_unknown_key_is_not_found(store: impl AgentSigningKeyStore) {
        assert_eq!(
            store
                .verifying_key(&key_id(9), T0, DEFAULT_ROTATION_OVERLAP_MS)
                .await,
            Err(AgentKeyError::NotFound)
        );
    }

    /// Expand the whole [`AgentSigningKeyStore`] contract against one factory.
    ///
    /// `$factory` is called fresh per case, so each gets an empty store.
    #[macro_export]
    macro_rules! agent_signing_key_store_contract_cases {
        ($label:ident, $factory:expr) => {
            mod $label {
                #[tokio::test]
                async fn an_active_key_signs_and_verifies() {
                    $crate::agent_key::contract::an_active_key_signs_and_verifies($factory()).await;
                }
                #[tokio::test]
                async fn registration_is_insert_only() {
                    $crate::agent_key::contract::registration_is_insert_only($factory()).await;
                }
                #[tokio::test]
                async fn rotation_moves_signing_forward_while_the_old_key_still_verifies() {
                    $crate::agent_key::contract::rotation_moves_signing_forward_while_the_old_key_still_verifies(
                        $factory(),
                    )
                    .await;
                }
                #[tokio::test]
                async fn the_overlap_window_closes_at_its_boundary() {
                    $crate::agent_key::contract::the_overlap_window_closes_at_its_boundary(
                        $factory(),
                    )
                    .await;
                }
                #[tokio::test]
                async fn revocation_is_immediate_with_no_overlap() {
                    $crate::agent_key::contract::revocation_is_immediate_with_no_overlap($factory())
                        .await;
                }
                #[tokio::test]
                async fn a_retiring_key_is_never_offered_for_signing() {
                    $crate::agent_key::contract::a_retiring_key_is_never_offered_for_signing(
                        $factory(),
                    )
                    .await;
                }
                #[tokio::test]
                async fn keys_do_not_resolve_across_tenants() {
                    $crate::agent_key::contract::keys_do_not_resolve_across_tenants($factory())
                        .await;
                }
                #[tokio::test]
                async fn an_unknown_key_is_not_found() {
                    $crate::agent_key::contract::an_unknown_key_is_not_found($factory()).await;
                }
            }
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use contract::{T0, key};

    // The in-memory reference impl is held to the same contract every durable
    // backend is. Fully qualified: the macro expands into a nested module.
    crate::agent_signing_key_store_contract_cases!(
        in_memory,
        crate::agent_key::InMemoryAgentSigningKeyStore::new
    );

    /// A `Retiring` row with no retirement instant is a corrupt/partial write;
    /// it must fail closed rather than grant an unbounded overlap. This is a
    /// pure-predicate case, so it lives outside the store contract.
    #[test]
    fn a_retiring_key_without_a_timestamp_fails_closed() {
        let mut corrupt = key(1, AgentKeyState::Retiring);
        corrupt.retiring_since_ms = None;
        assert_eq!(
            verification_admits(&corrupt, T0, DEFAULT_ROTATION_OVERLAP_MS),
            Err(AgentKeyError::Retired)
        );
    }
}
