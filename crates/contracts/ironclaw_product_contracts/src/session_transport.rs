//! Session-socket transport-auth vocabulary.
//!
//! The WebUI session WebSocket cannot carry a bearer header, so its upgrade
//! authenticates with a short-lived, single-use ticket minted over ordinary
//! bearer-authenticated HTTP. This module owns the ticket record and the
//! storage port; the WebUI host-auth layer owns minting/consumption policy
//! and its bounded in-memory adapter, while composition owns the shared
//! (multi-replica) adapter — assembly is the only layer that may name both
//! this port and the durable secret substrate, mirroring
//! [`crate::operator_secrets`].
//!
//! Tickets are transport-auth nonces, not product or conversation records:
//! they bind the exact authenticated caller for a few seconds and may be
//! removed after consumption or expiry.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use ironclaw_host_api::ids::{TenantId, UserId};

/// Lifetime of one session-socket ticket, from mint to expiry.
pub const SESSION_SOCKET_TICKET_TTL_MS: u64 = 15_000;

/// Hard bound on outstanding ticket nonces per store instance. Stores refuse
/// further mints (retryably) rather than growing without bound.
pub const MAX_OUTSTANDING_SESSION_SOCKET_TICKETS: usize = 1024;

/// One minted, not-yet-consumed session-socket ticket.
///
/// The record binds the exact authenticated caller observed at mint time.
/// The upgrade path reconstructs its caller identity from this record alone;
/// nothing in the WebSocket URL or headers carries authority.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionSocketTicket {
    pub tenant_id: TenantId,
    pub user_id: UserId,
    /// Whether the minting bearer carried operator WebUI configuration
    /// capability; replayed onto the socket's request-scoped capabilities.
    pub operator_config: bool,
    /// Unix epoch milliseconds after which the ticket must not authenticate.
    pub expires_at_unix_ms: u64,
}

/// Storage failures a ticket store may surface. The classification is the
/// whole contract: callers fail closed on either variant, retrying only the
/// retryable one.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SessionSocketTicketStoreError {
    /// The store is temporarily unable to mint or consume (backend
    /// unavailable, outstanding-ticket bound reached).
    #[error("session socket ticket store unavailable: {reason}")]
    Unavailable { reason: &'static str },
    /// The store rejected the operation permanently (malformed nonce,
    /// oversized record).
    #[error("session socket ticket rejected: {reason}")]
    Rejected { reason: &'static str },
}

/// Single-use ticket storage.
///
/// `consume` must be atomic across every replica sharing the store: for one
/// nonce, at most one caller ever receives the ticket, no matter how many
/// concurrent consumers race, and a consumed or expired nonce replays as
/// `Ok(None)`.
#[async_trait]
pub trait SessionSocketTicketStore: Send + Sync {
    /// Store a freshly minted single-use ticket under an opaque nonce.
    async fn mint(
        &self,
        nonce: &str,
        ticket: SessionSocketTicket,
    ) -> Result<(), SessionSocketTicketStoreError>;

    /// Atomically consume the nonce, returning its ticket to exactly one
    /// caller. Unknown, already-consumed, and expired nonces return
    /// `Ok(None)`.
    async fn consume(
        &self,
        nonce: &str,
    ) -> Result<Option<SessionSocketTicket>, SessionSocketTicketStoreError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ticket_record_round_trips_with_typed_identity() {
        let ticket = SessionSocketTicket {
            tenant_id: TenantId::new("tenant-alpha").expect("tenant"),
            user_id: UserId::new("user-alpha").expect("user"),
            operator_config: true,
            expires_at_unix_ms: 1_700_000_000_000,
        };
        let encoded = serde_json::to_value(&ticket).expect("ticket serializes");
        assert_eq!(encoded["tenant_id"], "tenant-alpha");
        assert_eq!(encoded["user_id"], "user-alpha");
        assert_eq!(encoded["operator_config"], true);
        let decoded: SessionSocketTicket =
            serde_json::from_value(encoded).expect("ticket deserializes");
        assert_eq!(decoded, ticket);
    }
}
