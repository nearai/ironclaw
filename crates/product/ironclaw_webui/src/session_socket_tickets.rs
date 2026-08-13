//! Bounded in-memory session-socket ticket store.
//!
//! The standalone (single-process) deployment adapter for
//! [`SessionSocketTicketStore`]: a bounded map of outstanding nonces with
//! TTL pruning on insert and single-use `remove` semantics on consume. The
//! same eviction discipline as the OAuth pending-flow and login-ticket
//! stores in this crate: prune expired entries at the cap, then drop the
//! oldest if still full.
//!
//! Multi-replica deployments must NOT use this adapter — a ticket minted on
//! one replica would be unknown to the replica receiving the upgrade, and
//! replay protection would only hold per process. Composition wires the
//! shared one-shot adapter for those shapes; when neither is available the
//! session WebSocket capability is not advertised at all.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use ironclaw_product_contracts::session_transport::{
    MAX_OUTSTANDING_SESSION_SOCKET_TICKETS, SessionSocketTicket, SessionSocketTicketStore,
    SessionSocketTicketStoreError,
};

struct StoredTicket {
    ticket: SessionSocketTicket,
    minted_at: Instant,
    expires_at: Instant,
}

/// Process-local single-use ticket store for single-replica deployments.
pub struct InMemorySessionSocketTicketStore {
    ttl: Duration,
    max_outstanding: usize,
    inner: Mutex<HashMap<String, StoredTicket>>,
}

impl Default for InMemorySessionSocketTicketStore {
    fn default() -> Self {
        Self::new()
    }
}

impl InMemorySessionSocketTicketStore {
    pub fn new() -> Self {
        Self {
            ttl: Duration::from_millis(
                ironclaw_product_contracts::session_transport::SESSION_SOCKET_TICKET_TTL_MS,
            ),
            max_outstanding: MAX_OUTSTANDING_SESSION_SOCKET_TICKETS,
            inner: Mutex::new(HashMap::new()),
        }
    }

    #[cfg(test)]
    fn with_bounds(ttl: Duration, max_outstanding: usize) -> Self {
        Self {
            ttl,
            max_outstanding,
            inner: Mutex::new(HashMap::new()),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, StoredTicket>> {
        // Recover from poison: ticket state is small, self-healing (TTL), and
        // a panicked minting task must not wedge every future upgrade.
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[async_trait]
impl SessionSocketTicketStore for InMemorySessionSocketTicketStore {
    async fn mint(
        &self,
        nonce: &str,
        ticket: SessionSocketTicket,
    ) -> Result<(), SessionSocketTicketStoreError> {
        let now = Instant::now();
        let mut entries = self.lock();
        if entries.len() >= self.max_outstanding {
            entries.retain(|_, stored| stored.expires_at > now);
        }
        if entries.len() >= self.max_outstanding {
            // Still saturated after pruning: drop the oldest outstanding
            // nonce rather than growing without bound. Its holder simply
            // mints again.
            if let Some(oldest) = entries
                .iter()
                .min_by_key(|(_, stored)| stored.minted_at)
                .map(|(nonce, _)| nonce.clone())
            {
                entries.remove(&oldest);
            } else {
                return Err(SessionSocketTicketStoreError::Unavailable {
                    reason: "ticket store saturated",
                });
            }
        }
        entries.insert(
            nonce.to_string(),
            StoredTicket {
                ticket,
                minted_at: now,
                expires_at: now + self.ttl,
            },
        );
        Ok(())
    }

    async fn consume(
        &self,
        nonce: &str,
    ) -> Result<Option<SessionSocketTicket>, SessionSocketTicketStoreError> {
        let now = Instant::now();
        let mut entries = self.lock();
        let Some(stored) = entries.remove(nonce) else {
            return Ok(None);
        };
        if stored.expires_at <= now {
            return Ok(None);
        }
        Ok(Some(stored.ticket))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_host_api::ids::{TenantId, UserId};

    fn ticket(user: &str) -> SessionSocketTicket {
        SessionSocketTicket {
            tenant_id: TenantId::new("tenant-alpha").expect("tenant"),
            user_id: UserId::new(user).expect("user"),
            operator_config: false,
            expires_at_unix_ms: u64::MAX,
        }
    }

    #[tokio::test]
    async fn consume_returns_the_ticket_exactly_once() {
        let store = InMemorySessionSocketTicketStore::new();
        store.mint("nonce-1", ticket("user-a")).await.expect("mint");

        let first = store.consume("nonce-1").await.expect("consume");
        assert_eq!(
            first.as_ref().map(|ticket| ticket.user_id.as_str()),
            Some("user-a"),
            "the first consumer receives the bound caller",
        );
        assert_eq!(
            store.consume("nonce-1").await.expect("replay consume"),
            None,
            "a replayed nonce must fail closed",
        );
        assert_eq!(
            store.consume("never-minted").await.expect("unknown"),
            None,
            "an unknown nonce must fail closed",
        );
    }

    #[tokio::test]
    async fn expired_tickets_do_not_authenticate() {
        let store = InMemorySessionSocketTicketStore::with_bounds(Duration::from_millis(0), 1024);
        store.mint("nonce-1", ticket("user-a")).await.expect("mint");
        assert_eq!(
            store.consume("nonce-1").await.expect("consume"),
            None,
            "an expired ticket must not authenticate",
        );
    }

    #[tokio::test]
    async fn saturation_evicts_the_oldest_nonce_instead_of_growing() {
        let store = InMemorySessionSocketTicketStore::with_bounds(Duration::from_secs(60), 2);
        store.mint("nonce-1", ticket("user-a")).await.expect("mint");
        store.mint("nonce-2", ticket("user-b")).await.expect("mint");
        store.mint("nonce-3", ticket("user-c")).await.expect("mint");

        assert_eq!(
            store.consume("nonce-1").await.expect("consume"),
            None,
            "the oldest nonce is evicted at the cap",
        );
        assert!(store.consume("nonce-2").await.expect("consume").is_some());
        assert!(store.consume("nonce-3").await.expect("consume").is_some());
    }

    #[tokio::test]
    async fn concurrent_consumers_of_one_nonce_have_exactly_one_winner() {
        let store = std::sync::Arc::new(InMemorySessionSocketTicketStore::new());
        store.mint("nonce-1", ticket("user-a")).await.expect("mint");

        let mut winners = 0;
        let mut tasks = tokio::task::JoinSet::new();
        for _ in 0..16 {
            let store = std::sync::Arc::clone(&store);
            tasks.spawn(async move { store.consume("nonce-1").await.expect("consume").is_some() });
        }
        while let Some(result) = tasks.join_next().await {
            if result.expect("task joins") {
                winners += 1;
            }
        }
        assert_eq!(winners, 1, "exactly one consumer may win one nonce");
    }
}
