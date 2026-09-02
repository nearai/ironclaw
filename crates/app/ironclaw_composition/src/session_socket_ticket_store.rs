//! Shared session-socket ticket store over the durable secret plane.
//!
//! Multi-replica deployments need mint and consume to land on different
//! replicas with replay still failing closed. This adapter anchors the
//! single-use guarantee on the secret store's one-shot lease protocol —
//! [`SecretStorePort::lease_once`] / [`SecretStorePort::consume`] return
//! material to exactly one consumer per lease, CAS-backed on the shared
//! durable backend (the same machinery the durable OAuth PKCE-verifier flow
//! relies on for replica-safe one-shot reads). The browser-facing nonce IS
//! the lease id, so no second mapping can drift.
//!
//! Tickets are transport-auth nonces, not product or conversation records:
//! the underlying secret is deleted after consumption and carries a 15-second
//! expiry either way. Composition wires this adapter when the deployment's
//! storage shape shares a durable backend across replicas; single-process
//! standalone shapes use the WebUI's bounded in-memory adapter instead.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::ids::SecretHandle;
use ironclaw_host_api::resource::ResourceScope;
use ironclaw_product_contracts::session_transport::{
    SESSION_SOCKET_TICKET_TTL_MS, SessionSocketTicket, SessionSocketTicketStore,
    SessionSocketTicketStoreError,
};
use ironclaw_secrets::{SecretLeaseId, SecretMaterial, SecretStoreError, SecretStorePort};
use secrecy::ExposeSecret;

/// Purpose-separated handle prefix for socket-ticket secrets. Nothing else
/// writes under it, and the random suffix keeps handles collision-free.
const TICKET_HANDLE_PREFIX: &str = "session-socket-ticket";

/// The stored secret body: the ticket plus its own handle so the consumer
/// can delete the row after the one-shot read without a second lookup.
#[derive(serde::Serialize, serde::Deserialize)]
struct StoredSocketTicket {
    handle: String,
    ticket: SessionSocketTicket,
}

pub(crate) struct SecretStoreSessionSocketTicketStore {
    secrets: Arc<dyn SecretStorePort>,
    scope: ResourceScope,
}

impl SecretStoreSessionSocketTicketStore {
    pub(crate) fn new(secrets: Arc<dyn SecretStorePort>) -> Self {
        Self {
            secrets,
            // Tickets are host transport state, not user secrets: they live
            // under the system scope, and the caller identity they bind is
            // inside the ticket record itself.
            scope: ResourceScope::system(),
        }
    }
}

fn unavailable(reason: &'static str) -> SessionSocketTicketStoreError {
    SessionSocketTicketStoreError::Unavailable { reason }
}

#[async_trait]
impl SessionSocketTicketStore for SecretStoreSessionSocketTicketStore {
    async fn mint(
        &self,
        ticket: SessionSocketTicket,
    ) -> Result<String, SessionSocketTicketStoreError> {
        let suffix = {
            use rand::RngExt as _;
            let mut bytes = [0u8; 16];
            rand::rng().fill(&mut bytes);
            hex::encode(bytes)
        };
        let handle_name = format!("{TICKET_HANDLE_PREFIX}-{suffix}");
        let handle = SecretHandle::new(&handle_name).map_err(|error| {
            tracing::debug!(
                target: "ironclaw::reborn::session_tickets",
                error = %error,
                "session socket ticket handle rejected",
            );
            unavailable("ticket handle rejected")
        })?;
        let body = serde_json::to_string(&StoredSocketTicket {
            handle: handle_name,
            ticket,
        })
        .map_err(|error| {
            tracing::debug!(
                target: "ironclaw::reborn::session_tickets",
                error = %error,
                "session socket ticket serialization failed",
            );
            unavailable("ticket serialization failed")
        })?;
        let expires_at = chrono::Utc::now()
            + chrono::Duration::milliseconds(
                i64::try_from(SESSION_SOCKET_TICKET_TTL_MS).unwrap_or(15_000),
            );
        self.secrets
            .put(
                self.scope.clone(),
                handle.clone(),
                SecretMaterial::from(body),
                Some(expires_at),
            )
            .await
            .map_err(|error| {
                tracing::debug!(
                    target: "ironclaw::reborn::session_tickets",
                    error = %error,
                    "session socket ticket write failed",
                );
                unavailable("ticket write failed")
            })?;
        let lease = match self.secrets.lease_once(&self.scope, &handle).await {
            Ok(lease) => lease,
            Err(error) => {
                tracing::debug!(
                    target: "ironclaw::reborn::session_tickets",
                    error = %error,
                    "session socket ticket lease failed",
                );
                // A secret nobody can ever lease is dead weight: remove it
                // now rather than leaving it for expiry.
                if let Err(cleanup_error) = self.secrets.delete(&self.scope, &handle).await {
                    tracing::debug!(
                        target: "ironclaw::reborn::session_tickets",
                        error = %cleanup_error,
                        "unleasable session socket ticket cleanup failed",
                    );
                }
                return Err(unavailable("ticket lease failed"));
            }
        };
        Ok(lease.id.to_string())
    }

    async fn consume(
        &self,
        nonce: &str,
    ) -> Result<Option<SessionSocketTicket>, SessionSocketTicketStoreError> {
        // The nonce is the lease id (a UUID); anything else is unknown.
        let Ok(lease_id) = nonce.parse::<SecretLeaseId>() else {
            return Ok(None);
        };
        let material = match self.secrets.consume(&self.scope, lease_id).await {
            Ok(material) => material,
            // One-shot outcomes and expiry all replay as "not authenticated";
            // only genuine backend unavailability is retryable.
            Err(
                SecretStoreError::UnknownLease { .. }
                | SecretStoreError::LeaseConsumed { .. }
                | SecretStoreError::LeaseRevoked { .. }
                | SecretStoreError::LeaseExpired { .. }
                | SecretStoreError::SecretExpired
                | SecretStoreError::UnknownSecret { .. },
            ) => return Ok(None),
            Err(error) => {
                tracing::debug!(
                    target: "ironclaw::reborn::session_tickets",
                    error = %error,
                    "session socket ticket consume failed",
                );
                return Err(unavailable("ticket consume failed"));
            }
        };
        let stored: StoredSocketTicket = match serde_json::from_str(material.expose_secret()) {
            Ok(stored) => stored,
            Err(error) => {
                tracing::debug!(
                    target: "ironclaw::reborn::session_tickets",
                    error = %error,
                    "session socket ticket body malformed",
                );
                return Ok(None);
            }
        };
        // The lease was the single-use gate; both rows are now spent
        // transport state, and every socket connect mints one, so reclaim
        // them here rather than accumulate a permanent row per connect.
        // Best-effort: a failed cleanup leaves only an inert consumed lease
        // (the ticket cannot be replayed), and tickets that were minted but
        // never consumed are the residue a periodic sweep would reap.
        if let Ok(handle) = SecretHandle::new(&stored.handle)
            && let Err(error) = self.secrets.delete(&self.scope, &handle).await
        {
            tracing::debug!(
                target: "ironclaw::reborn::session_tickets",
                error = %error,
                "consumed session socket ticket cleanup failed",
            );
        }
        if let Err(error) = self.secrets.delete_lease(&self.scope, lease_id).await {
            tracing::debug!(
                target: "ironclaw::reborn::session_tickets",
                error = %error,
                "consumed session socket ticket lease cleanup failed",
            );
        }
        Ok(Some(stored.ticket))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_host_api::ids::{TenantId, UserId};
    use ironclaw_secrets::SecretStore;

    fn ticket(user: &str) -> SessionSocketTicket {
        SessionSocketTicket {
            tenant_id: TenantId::new("tenant-alpha").expect("tenant"),
            user_id: UserId::new(user).expect("user"),
            operator_config: false,
            expires_at_unix_ms: u64::MAX,
        }
    }

    fn store() -> SecretStoreSessionSocketTicketStore {
        SecretStoreSessionSocketTicketStore::new(Arc::new(SecretStore::ephemeral()))
    }

    #[tokio::test]
    async fn mint_then_consume_round_trips_the_bound_caller_once() {
        let store = store();
        let nonce = store.mint(ticket("user-a")).await.expect("mint");

        let first = store.consume(&nonce).await.expect("consume");
        assert_eq!(
            first.as_ref().map(|ticket| ticket.user_id.as_str()),
            Some("user-a"),
        );
        assert_eq!(
            store.consume(&nonce).await.expect("replay"),
            None,
            "a consumed lease must never authenticate again",
        );
        assert_eq!(store.consume("not-a-lease").await.expect("junk"), None);

        // Both rows are reclaimed: every socket connect mints one ticket,
        // so a consumed ticket must not leave a permanent secret or lease.
        assert!(
            store
                .secrets
                .metadata_for_scope(&store.scope)
                .await
                .expect("metadata")
                .is_empty(),
            "the consumed ticket secret is deleted"
        );
        assert!(
            store
                .secrets
                .leases_for_scope(&store.scope)
                .await
                .expect("leases")
                .is_empty(),
            "the consumed one-shot lease row is deleted"
        );
    }

    #[tokio::test]
    async fn concurrent_consumers_of_one_nonce_have_exactly_one_winner() {
        let store = Arc::new(store());
        let nonce = store.mint(ticket("user-a")).await.expect("mint");

        let mut winners = 0;
        let mut tasks = tokio::task::JoinSet::new();
        for _ in 0..16 {
            let store = Arc::clone(&store);
            let nonce = nonce.clone();
            tasks.spawn(async move { store.consume(&nonce).await.expect("consume").is_some() });
        }
        while let Some(result) = tasks.join_next().await {
            if result.expect("task joins") {
                winners += 1;
            }
        }
        assert_eq!(winners, 1, "exactly one consumer may win one lease");
    }
}
